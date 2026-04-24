//! X11 overlay — override-redirect window, tiny-skia composite, XPutImage blit.
//! Replaces the old eframe/egui overlay. Same `show(...)` signature so dbus
//! and tray callers are unchanged.
//!
//! Submodules: `state` (data), `paint` (render), `tool_buttons` (strip),
//! `selection` (handles), `draft` (in-progress shapes), `x11_win` (X11 plumbing).

mod draft;
mod paint;
mod selection;
mod state;
mod tool_buttons;
mod x11_win;

use crate::canvas::{Annotation, Bounds, Pos, ToolKind};
use crate::config::Config;
use crate::ui::UiResult;
use draft::Draft;
use image::RgbaImage;
use selection::{cursor_glyph_for_handle, handle_at, resize_rect, SelectionEdit};
use state::{Mode, OverlayState};
use std::sync::Arc;
use tokio::sync::oneshot;
use tool_buttons::Hit;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::KeyButMask;
use x11rb::protocol::Event;
use x11_win::{
    X11Win, KS_1, KS_8, KS_C_LOWER, KS_ESCAPE, KS_KP_ENTER, KS_RETURN, KS_Y_LOWER, KS_Z_LOWER,
    XC_CROSSHAIR, XC_FLEUR, XC_HAND1, XC_LEFT_PTR,
};

/// Click-vs-drag threshold in pixels squared. Motion below this on release
/// counts as a click (used for Counter placement + strip clicks).
const CLICK_SQ: f32 = 4.0 * 4.0;

pub fn show(
    image: RgbaImage,
    screen_origin: (i32, i32),
    save_path: String,
    clipboard: bool,
    config: Arc<Config>,
    result_tx: oneshot::Sender<UiResult>,
) {
    let t0 = std::time::Instant::now();
    let (w, h) = (image.width(), image.height());

    let mut win = match X11Win::new(screen_origin, w as u16, h as u16) {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("X11 overlay window creation failed: {e}");
            let _ = result_tx.send(UiResult::Cancelled);
            return;
        }
    };

    if let Err(e) = win.map_and_grab() {
        tracing::error!("X11 map_and_grab: {e}");
        let _ = result_tx.send(UiResult::Cancelled);
        return;
    }
    let _ = win.set_cursor(XC_CROSSHAIR);

    tracing::info!(
        setup_ms = t0.elapsed().as_millis() as u64,
        w, h,
        "overlay window ready"
    );

    let mut state = OverlayState::new(image, save_path, clipboard, config);
    let mut display = RgbaImage::new(w, h);
    let mut dragging = Dragging::None;
    let mut press_pos = Pos { x: 0.0, y: 0.0 };
    let mut last_cursor = XC_CROSSHAIR;

    // First paint.
    let mut dirty = true;
    let mut last_motion: Option<Pos> = None;
    let result = loop {
        if dirty {
            // If the last thing we saw was a motion, apply its effects now —
            // we deferred per-motion work while draining so repaint isn't 1:1
            // with event count.
            if let Some(p) = last_motion.take() {
                on_motion(&mut state, &mut dragging, p);
                let desired = pick_cursor(&state, &dragging, p);
                if desired != last_cursor {
                    let _ = win.set_cursor(desired);
                    last_cursor = desired;
                }
            }
            state.refresh_base();
            state.refresh_draft_pixelate();
            paint::composite(&mut display, &state);
            if let Err(e) = win.blit_rgba(display.as_raw()) {
                tracing::error!("blit failed: {e}");
                break UiResult::Cancelled;
            }
            dirty = false;
        }

        // Wait for the next event, then drain the queue before painting again.
        // This collapses a burst of MotionNotify into one repaint.
        let first = match win.conn.wait_for_event() {
            Ok(ev) => ev,
            Err(e) => {
                tracing::error!("X11 wait_for_event: {e}");
                break UiResult::Cancelled;
            }
        };
        let mut events: Vec<Event> = Vec::with_capacity(8);
        events.push(first);
        loop {
            match win.conn.poll_for_event() {
                Ok(Some(ev)) => events.push(ev),
                Ok(None) => break,
                Err(e) => {
                    tracing::error!("X11 poll_for_event: {e}");
                    break;
                }
            }
        }

        let mut finish: Option<UiResult> = None;
        for ev in events {
            match ev {
                Event::Expose(_) => dirty = true,
                Event::KeyPress(e) => {
                    if let Some(res) = handle_key(&mut state, &win, e.detail, u16::from(e.state)) {
                        finish = Some(res);
                        break;
                    }
                    state.ctrl_down = u16::from(e.state) & u16::from(KeyButMask::CONTROL) != 0;
                    dirty = true;
                }
                Event::KeyRelease(e) => {
                    state.ctrl_down = u16::from(e.state) & u16::from(KeyButMask::CONTROL) != 0;
                }
                Event::ButtonPress(e) if e.detail == 1 => {
                    // Apply any pending motion first so press-time state is correct.
                    if let Some(p) = last_motion.take() {
                        on_motion(&mut state, &mut dragging, p);
                    }
                    let p = Pos { x: e.event_x as f32, y: e.event_y as f32 };
                    press_pos = p;
                    state.ctrl_down = u16::from(e.state) & u16::from(KeyButMask::CONTROL) != 0;
                    dragging = on_press(&mut state, p);
                    dirty = true;
                }
                Event::MotionNotify(e) => {
                    let p = Pos { x: e.event_x as f32, y: e.event_y as f32 };
                    state.ctrl_down = u16::from(e.state) & u16::from(KeyButMask::CONTROL) != 0;
                    last_motion = Some(p);
                    dirty = true;
                }
                Event::ButtonRelease(e) if e.detail == 1 => {
                    if let Some(p) = last_motion.take() {
                        on_motion(&mut state, &mut dragging, p);
                    }
                    let p = Pos { x: e.event_x as f32, y: e.event_y as f32 };
                    state.ctrl_down = u16::from(e.state) & u16::from(KeyButMask::CONTROL) != 0;
                    if let Some(res) = on_release(&mut state, &mut dragging, p, press_pos) {
                        finish = Some(res);
                        break;
                    }
                    dirty = true;
                }
                _ => {}
            }
        }
        if let Some(r) = finish {
            break r;
        }
    };

    let _ = result_tx.send(result);
    tracing::info!(
        total_ms = t0.elapsed().as_millis() as u64,
        "overlay closed"
    );
    drop(win);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dragging {
    None,
    Region,
    Draft,
    EditResize,
    EditMove,
    Strip(Hit),
}

fn on_press(state: &mut OverlayState, p: Pos) -> Dragging {
    // Strip click takes priority if selection is shown.
    if let Some(sel) = state.selection {
        let strip = tool_buttons::strip_rect(state.base.width(), state.base.height(), sel);
        if tool_buttons::contains(strip, p) {
            if let Some(hit) = tool_buttons::hit(strip, p) {
                return Dragging::Strip(hit);
            }
            // Inside strip but between buttons — eat the click, no drag.
            return Dragging::None;
        }
    }

    match state.mode {
        Mode::SelectingRegion => {
            state.sel_drag_start = Some(p);
            state.selection = None;
            Dragging::Region
        }
        Mode::Annotating => {
            let Some(sel) = state.selection else {
                return Dragging::None;
            };

            // Handle hit?
            if let Some(h) = handle_at(sel, p) {
                state.selection_edit = SelectionEdit::Resizing(h);
                state.edit_drag_start = Some(p);
                state.edit_rect_start = Some(sel);
                return Dragging::EditResize;
            }
            // No tool armed, or Ctrl held: inside-drag moves the frame.
            if bounds_contains(sel, p) && (state.canvas.tool.is_none() || state.ctrl_down) {
                state.selection_edit = SelectionEdit::Moving;
                state.edit_drag_start = Some(p);
                state.edit_rect_start = Some(sel);
                return Dragging::EditMove;
            }
            // Counter: click-to-place, no drag.
            if state.canvas.tool == Some(ToolKind::Counter) && bounds_contains(sel, p) {
                let n = state.canvas.next_counter();
                state.canvas.push(Annotation::Counter {
                    center: p,
                    number: n,
                    color: state.canvas.style.color,
                    radius: state.counter_radius,
                });
                return Dragging::None;
            }
            // Start an annotation draft.
            if let Some(tool) = state.canvas.tool {
                if bounds_contains(sel, p) {
                    state.draft = Draft::new(tool, p, state.canvas.style, state.pixelate_block);
                    return Dragging::Draft;
                }
            }
            Dragging::None
        }
    }
}

fn on_motion(state: &mut OverlayState, dragging: &mut Dragging, p: Pos) {
    // Always update strip hover so buttons light up on hover, even outside a drag.
    state.strip_hover = match state.selection {
        Some(sel) => {
            let strip = tool_buttons::strip_rect(state.base.width(), state.base.height(), sel);
            if tool_buttons::contains(strip, p) {
                tool_buttons::hit(strip, p)
            } else {
                None
            }
        }
        None => None,
    };

    match *dragging {
        Dragging::Region => {
            if let Some(start) = state.sel_drag_start {
                state.selection = Some(Bounds::from_two(start, p));
            }
        }
        Dragging::Draft => {
            if let (Some(draft), Some(sel)) = (state.draft.as_mut(), state.selection) {
                draft.extend(clamp_to_bounds(p, sel));
            }
        }
        Dragging::EditResize => {
            if let (SelectionEdit::Resizing(h), Some(start), Some(rect)) =
                (state.selection_edit, state.edit_drag_start, state.edit_rect_start)
            {
                let dx = p.x - start.x;
                let dy = p.y - start.y;
                state.selection = Some(resize_rect(rect, h, dx, dy));
            }
        }
        Dragging::EditMove => {
            if let (Some(start), Some(rect)) = (state.edit_drag_start, state.edit_rect_start) {
                let dx = p.x - start.x;
                let dy = p.y - start.y;
                state.selection = Some(Bounds {
                    x: rect.x + dx,
                    y: rect.y + dy,
                    w: rect.w,
                    h: rect.h,
                });
            }
        }
        Dragging::None | Dragging::Strip(_) => {}
    }
}

fn on_release(
    state: &mut OverlayState,
    dragging: &mut Dragging,
    p: Pos,
    press: Pos,
) -> Option<UiResult> {
    let d = *dragging;
    *dragging = Dragging::None;

    match d {
        Dragging::Region => {
            if let Some(sel) = state.selection {
                if sel.w >= 4.0 && sel.h >= 4.0 {
                    state.mode = Mode::Annotating;
                } else {
                    state.selection = None;
                }
            }
            state.sel_drag_start = None;
        }
        Dragging::Draft => {
            if let Some(draft) = state.draft.take() {
                if let Some(a) = draft.finalize() {
                    state.canvas.push(a);
                }
            }
        }
        Dragging::EditResize | Dragging::EditMove => {
            state.selection_edit = SelectionEdit::None;
            state.edit_drag_start = None;
            state.edit_rect_start = None;
        }
        Dragging::Strip(hit) => {
            // Only trigger if release is still on the same button AND travel < CLICK_SQ.
            let dx = p.x - press.x;
            let dy = p.y - press.y;
            if dx * dx + dy * dy < CLICK_SQ {
                if let Some(sel) = state.selection {
                    let strip =
                        tool_buttons::strip_rect(state.base.width(), state.base.height(), sel);
                    if tool_buttons::hit(strip, p) == Some(hit) {
                        return apply_hit(state, hit);
                    }
                }
            }
        }
        Dragging::None => {}
    }
    None
}

fn apply_hit(state: &mut OverlayState, hit: Hit) -> Option<UiResult> {
    match hit {
        Hit::Tool(t) => {
            // Click the active tool to disarm (back to move-the-frame mode).
            state.canvas.tool = if state.canvas.tool == Some(t) { None } else { Some(t) };
            None
        }
        Hit::Save => Some(state.act(false)),
        Hit::Copy => Some(state.act(true)),
    }
}

fn handle_key(
    state: &mut OverlayState,
    win: &X11Win,
    keycode: u8,
    state_mask: u16,
) -> Option<UiResult> {
    let ks = win.keysym(keycode);
    let ctrl = state_mask & u16::from(KeyButMask::CONTROL) != 0;

    match ks {
        KS_ESCAPE => return Some(UiResult::Cancelled),
        KS_RETURN | KS_KP_ENTER => return Some(state.act(false)),
        _ => {}
    }

    if ctrl {
        match ks {
            KS_C_LOWER => return Some(state.act(true)),
            KS_Z_LOWER => state.canvas.undo(),
            KS_Y_LOWER => state.canvas.redo(),
            _ => {}
        }
        return None;
    }

    if (KS_1..=KS_8).contains(&ks) {
        let idx = (ks - KS_1) as usize;
        if let Some(&t) = ToolKind::ALL.get(idx) {
            state.canvas.tool = Some(t);
        }
    }
    None
}

fn pick_cursor(state: &OverlayState, dragging: &Dragging, p: Pos) -> u16 {
    // Active drag: resize cursor sticks through the drag.
    if let Dragging::EditResize = dragging {
        if let SelectionEdit::Resizing(h) = state.selection_edit {
            return cursor_glyph_for_handle(h);
        }
    }
    if let Dragging::EditMove = dragging {
        return XC_FLEUR;
    }
    // Strip hover: pointing hand.
    if state.strip_hover.is_some() {
        return XC_HAND1;
    }
    match state.selection {
        Some(sel) => {
            if let Some(h) = handle_at(sel, p) {
                cursor_glyph_for_handle(h)
            } else if bounds_contains(sel, p) {
                if state.ctrl_down {
                    XC_FLEUR
                } else {
                    XC_CROSSHAIR
                }
            } else {
                XC_LEFT_PTR
            }
        }
        None => XC_CROSSHAIR,
    }
}

fn bounds_contains(b: Bounds, p: Pos) -> bool {
    p.x >= b.x && p.x <= b.x + b.w && p.y >= b.y && p.y <= b.y + b.h
}

fn clamp_to_bounds(p: Pos, b: Bounds) -> Pos {
    Pos {
        x: p.x.max(b.x).min(b.x + b.w),
        y: p.y.max(b.y).min(b.y + b.h),
    }
}

