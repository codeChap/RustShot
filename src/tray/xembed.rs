//! Status-tray backend: legacy X11 XEmbed. Used when no SNI watcher is on
//! the session bus — covers stock i3bar with `tray_output primary` set.
//!
//! Protocol summary:
//!   1. Own / find `_NET_SYSTEM_TRAY_S<screen>` — the tray manager holds it.
//!   2. Create our tiny 22x22 window.
//!   3. Send a `_NET_SYSTEM_TRAY_OPCODE` ClientMessage with
//!      `SYSTEM_TRAY_REQUEST_DOCK` to the tray owner.
//!   4. Let the tray reparent the window; repaint on Expose; react to clicks.
//!
//! Re-dock: tray managers (i3bar, polybar, …) restart out from under us on
//! user-initiated reload/restart. The freedesktop spec says the new manager
//! MUST broadcast a `MANAGER` ClientMessage on the root window when it takes
//! the selection — so we subscribe to root's StructureNotify, watch for that
//! broadcast, and re-issue the dock without a daemon restart. We also catch
//! DestroyNotify on our own window (manager reaped it) and ReparentNotify
//! back to root (manager unmapped us).
//!
//! Rendering is intentionally simple: four FillRectangles form a yellow
//! selection-frame glyph on a dark background. No alpha, no XRender —
//! stays lightweight and matches the rest of RustShot's visual language.

use crate::capture::X11Capture;
use crate::config::Config;
use crate::ui::UiRequest;
use crossbeam_channel::Sender;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt, CreateGCAux,
    CreateWindowAux, EventMask, Gcontext, GrabMode, PropMode, Rectangle, Window, WindowClass,
    CLIENT_MESSAGE_EVENT,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

const ICON: u16 = 22;
const DARK_PX: u32 = 0x0020_2024;
const YELLOW_PX: u32 = 0x00FF_C800;

pub fn run(
    capture: Arc<X11Capture>,
    config: Arc<Config>,
    ui_tx: Sender<UiRequest>,
    gui_busy: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let (conn, screen_num) = RustConnection::connect(None)?;
    let root = conn.setup().roots[screen_num].root;

    let tray_sel = intern(&conn, &format!("_NET_SYSTEM_TRAY_S{screen_num}"))?;
    let xembed_info = intern(&conn, "_XEMBED_INFO")?;
    let tray_opcode = intern(&conn, "_NET_SYSTEM_TRAY_OPCODE")?;
    let manager_atom = intern(&conn, "MANAGER")?;

    // Hook root's StructureNotify so we receive the `MANAGER` ClientMessage
    // that a new tray manager broadcasts when it claims the selection.
    // Without this we'd never learn a new manager came online.
    conn.change_window_attributes(
        root,
        &ChangeWindowAttributesAux::default().event_mask(EventMask::STRUCTURE_NOTIFY),
    )?;

    let mut wid = create_tray_window(&conn, root, xembed_info)?;
    let (mut gc_bg, mut gc_frame) = make_gcs(&conn, wid)?;
    if !try_dock(&conn, wid, tray_sel, tray_opcode)? {
        tracing::info!("XEmbed tray: no manager yet; waiting for MANAGER broadcast");
    }
    conn.flush()?;

    let mut w = ICON;
    let mut h = ICON;
    loop {
        let event = conn.wait_for_event()?;
        match event {
            Event::Expose(e) if e.window == wid => {
                paint(&conn, wid, gc_bg, gc_frame, w, h)?;
                conn.flush()?;
            }
            Event::ConfigureNotify(e) if e.window == wid => {
                w = e.width;
                h = e.height;
            }
            Event::ButtonPress(e) if e.event == wid && e.detail == 1 => {
                super::spawn_capture(
                    capture.clone(),
                    config.clone(),
                    ui_tx.clone(),
                    gui_busy.clone(),
                );
            }
            Event::ButtonPress(e) if e.event == wid && e.detail == 3 => {
                if let Err(err) = show_quit_menu(&conn, root, e.root_x, e.root_y) {
                    tracing::warn!("quit menu failed: {err}");
                }
            }
            Event::ReparentNotify(e) if e.window == wid && e.parent == root => {
                // Manager unmapped / closed and pushed us back onto root.
                // Try to redock; if no owner now, the next MANAGER broadcast
                // will pick us up.
                tracing::info!("XEmbed tray: detached from tray; attempting redock");
                let _ = try_dock(&conn, wid, tray_sel, tray_opcode)?;
                conn.flush()?;
            }
            Event::DestroyNotify(e) if e.window == wid => {
                // Some managers reap the client window instead of reparenting
                // it. Build a fresh one and redock.
                tracing::info!("XEmbed tray: client window destroyed, recreating");
                wid = create_tray_window(&conn, root, xembed_info)?;
                let g = make_gcs(&conn, wid)?;
                gc_bg = g.0;
                gc_frame = g.1;
                w = ICON;
                h = ICON;
                let _ = try_dock(&conn, wid, tray_sel, tray_opcode)?;
                conn.flush()?;
            }
            Event::ClientMessage(e) if e.window == root && e.type_ == manager_atom => {
                // MANAGER broadcast payload: data[0]=timestamp,
                // data[1]=selection atom, data[2]=new owner.
                let data = e.data.as_data32();
                if data[1] == tray_sel {
                    tracing::info!("XEmbed tray: new manager (MANAGER broadcast); redocking");
                    let _ = try_dock(&conn, wid, tray_sel, tray_opcode)?;
                    conn.flush()?;
                }
            }
            _ => {}
        }
    }
}

fn create_tray_window(
    conn: &RustConnection,
    root: Window,
    xembed_info: u32,
) -> anyhow::Result<Window> {
    let wid = conn.generate_id()?;
    // override_redirect=true keeps the window manager (i3) out of this
    // window's business — without it, i3 sees a top-level mapped window
    // before the tray reparents it and floats / tiles it full-screen.
    let attrs = CreateWindowAux::default()
        .override_redirect(1)
        .event_mask(
            EventMask::EXPOSURE | EventMask::BUTTON_PRESS | EventMask::STRUCTURE_NOTIFY,
        )
        .background_pixel(DARK_PX);
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        wid,
        root,
        0,
        0,
        ICON,
        ICON,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &attrs,
    )?;

    conn.change_property8(
        PropMode::REPLACE,
        wid,
        u32::from(AtomEnum::WM_NAME),
        u32::from(AtomEnum::STRING),
        b"RustShot",
    )?;
    conn.change_property8(
        PropMode::REPLACE,
        wid,
        u32::from(AtomEnum::WM_CLASS),
        u32::from(AtomEnum::STRING),
        b"rustshot\0RustShot\0",
    )?;
    // _XEMBED_INFO: version=0, flags=XEMBED_MAPPED (bit 0)
    let xembed_data = [0u32, 1u32];
    conn.change_property32(
        PropMode::REPLACE,
        wid,
        xembed_info,
        xembed_info,
        &xembed_data,
    )?;
    Ok(wid)
}

fn make_gcs(conn: &RustConnection, wid: Window) -> anyhow::Result<(Gcontext, Gcontext)> {
    let gc_bg = conn.generate_id()?;
    conn.create_gc(gc_bg, wid, &CreateGCAux::default().foreground(DARK_PX))?;
    let gc_frame = conn.generate_id()?;
    conn.create_gc(gc_frame, wid, &CreateGCAux::default().foreground(YELLOW_PX))?;
    Ok((gc_bg, gc_frame))
}

/// Look up the current tray manager and send it a DOCK request for `wid`.
/// Returns `true` if a manager existed and the dock event was sent. `false`
/// just means "no manager right now" — wait for the MANAGER broadcast.
fn try_dock(
    conn: &RustConnection,
    wid: Window,
    tray_sel: u32,
    tray_opcode: u32,
) -> anyhow::Result<bool> {
    let owner = conn.get_selection_owner(tray_sel)?.reply()?.owner;
    if owner == x11rb::NONE {
        return Ok(false);
    }
    let mut payload = [0u32; 5];
    payload[0] = 0; // timestamp = CurrentTime
    payload[1] = 0; // SYSTEM_TRAY_REQUEST_DOCK
    payload[2] = wid;
    let dock = ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 0,
        window: owner,
        type_: tray_opcode,
        data: payload.into(),
    };
    conn.send_event(false, owner, EventMask::NO_EVENT, dock)?;
    conn.map_window(wid)?;
    Ok(true)
}

fn paint(
    conn: &RustConnection,
    wid: Window,
    gc_bg: Gcontext,
    gc_frame: Gcontext,
    w: u16,
    h: u16,
) -> anyhow::Result<()> {
    conn.poly_fill_rectangle(
        wid,
        gc_bg,
        &[Rectangle {
            x: 0,
            y: 0,
            width: w,
            height: h,
        }],
    )?;

    let min = w.min(h) as f32;
    let m = (min * 0.18).round() as i16;
    let t = ((min * 0.09).round() as u16).max(1);
    let iw = w as i16 - 2 * m;
    let ih = h as i16 - 2 * m;
    if iw > (2 * t as i16) && ih > (2 * t as i16) {
        let frame = [
            Rectangle { x: m, y: m, width: iw as u16, height: t },
            Rectangle {
                x: m,
                y: m + ih - t as i16,
                width: iw as u16,
                height: t,
            },
            Rectangle { x: m, y: m, width: t, height: ih as u16 },
            Rectangle {
                x: m + iw - t as i16,
                y: m,
                width: t,
                height: ih as u16,
            },
        ];
        conn.poly_fill_rectangle(wid, gc_frame, &frame)?;
    }
    Ok(())
}

fn intern(conn: &RustConnection, name: &str) -> anyhow::Result<u32> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

/// Modal popup with a single "Quit" item, anchored at the right-click position.
/// Pointer-grabs so a click outside dismisses, a click inside exits the daemon.
/// Uses the X core "fixed" font (server-side bitmap) so we don't drag in any
/// client-side text-rendering deps for a 4-character label.
fn show_quit_menu(
    conn: &RustConnection,
    root: Window,
    root_x: i16,
    root_y: i16,
) -> anyhow::Result<()> {
    const W: u16 = 96;
    const H: u16 = 28;
    const POPUP_BG: u32 = 0x0028_2830;
    const POPUP_BORDER: u32 = 0x005A_5A6C;
    const POPUP_FG: u32 = 0x00FF_FFFF;
    // X core "fixed" font is a 6x13 bitmap.
    const GLYPH_W: i16 = 6;
    const ASCENT: i16 = 10;

    // Place near the cursor; clamp to the root window so the popup stays
    // on-screen no matter where the tray icon lives.
    let geom = conn.get_geometry(root)?.reply()?;
    let mut x = root_x;
    let mut y = root_y;
    if x + W as i16 > geom.width as i16 {
        x = geom.width as i16 - W as i16 - 2;
    }
    if y + H as i16 > geom.height as i16 {
        y = (root_y - H as i16).max(0);
    }
    if x < 0 {
        x = 0;
    }
    if y < 0 {
        y = 0;
    }

    let popup = conn.generate_id()?;
    let attrs = CreateWindowAux::default()
        .override_redirect(1)
        .event_mask(EventMask::EXPOSURE | EventMask::BUTTON_PRESS)
        .background_pixel(POPUP_BG);
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        popup,
        root,
        x,
        y,
        W,
        H,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &attrs,
    )?;

    let gc_bg = conn.generate_id()?;
    conn.create_gc(gc_bg, popup, &CreateGCAux::default().foreground(POPUP_BG))?;
    let gc_border = conn.generate_id()?;
    conn.create_gc(gc_border, popup, &CreateGCAux::default().foreground(POPUP_BORDER))?;
    let font = conn.generate_id()?;
    conn.open_font(font, b"fixed")?;
    let gc_text = conn.generate_id()?;
    conn.create_gc(
        gc_text,
        popup,
        &CreateGCAux::default()
            .foreground(POPUP_FG)
            .background(POPUP_BG)
            .font(font),
    )?;

    conn.map_window(popup)?;

    // Grab on the popup with owner_events=false so every click anywhere on
    // the screen lands here. We then check root coords against the popup's
    // rect to tell "click on Quit" from "click outside (dismiss)".
    let _ = conn
        .grab_pointer(
            false,
            popup,
            EventMask::BUTTON_PRESS,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            x11rb::NONE,
            x11rb::NONE,
            x11rb::CURRENT_TIME,
        )?
        .reply()?;
    conn.flush()?;

    let mut quit = false;
    loop {
        let ev = conn.wait_for_event()?;
        match ev {
            Event::Expose(e) if e.window == popup => {
                conn.poly_fill_rectangle(
                    popup,
                    gc_bg,
                    &[Rectangle { x: 0, y: 0, width: W, height: H }],
                )?;
                conn.poly_rectangle(
                    popup,
                    gc_border,
                    &[Rectangle {
                        x: 0,
                        y: 0,
                        width: W - 1,
                        height: H - 1,
                    }],
                )?;
                let label = b"Quit";
                let text_w = label.len() as i16 * GLYPH_W;
                let tx = (W as i16 - text_w) / 2;
                let ty = (H as i16 + ASCENT) / 2;
                conn.image_text8(popup, gc_text, tx, ty, label)?;
                conn.flush()?;
            }
            Event::ButtonPress(e) => {
                let inside = e.root_x >= x
                    && e.root_x < x + W as i16
                    && e.root_y >= y
                    && e.root_y < y + H as i16;
                if inside {
                    quit = true;
                }
                break;
            }
            _ => {}
        }
    }

    let _ = conn.ungrab_pointer(x11rb::CURRENT_TIME);
    let _ = conn.destroy_window(popup);
    let _ = conn.close_font(font);
    conn.flush()?;

    if quit {
        tracing::info!("quit requested via tray menu");
        std::process::exit(0);
    }
    Ok(())
}
