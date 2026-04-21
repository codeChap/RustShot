//! Fullscreen overlay: region selection, annotation, and export.
//!
//! Submodules: `selection` (handle editing), `draft` (in-progress shapes),
//! `preview` (live egui rendering), `convert` (tiny type conversions).

mod convert;
mod draft;
mod preview;
mod selection;
mod tool_buttons;

use crate::canvas::{render, Annotation, Bounds, Canvas, ToolKind};
use crate::config::{self, Config};
use crate::export;
use crate::ui::{toolbar, UiResult};
use convert::pos_from;
use draft::Draft;
use eframe::egui;
use image::{Rgba, RgbaImage};
use selection::{cursor_for_handle, draw_handles, handle_at, resize_rect, SelectionEdit};
use std::sync::{Arc, Mutex};

pub fn show(
    image: RgbaImage,
    screen_origin: (i32, i32),
    save_path: String,
    clipboard: bool,
    config: Arc<Config>,
) -> UiResult {
    let result = Arc::new(Mutex::new(UiResult::Cancelled));
    let result_for_app = result.clone();

    let (sx, sy) = screen_origin;
    let (img_w, img_h) = (image.width(), image.height());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("rustshot")
            .with_title("rustshot-overlay")
            .with_decorations(false)
            .with_resizable(false)
            .with_position([sx as f32, sy as f32])
            .with_inner_size([img_w as f32, img_h as f32])
            .with_window_level(egui::WindowLevel::AlwaysOnTop)
            .with_fullscreen(true),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "rustshot-overlay",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(OverlayApp::new(
                image,
                save_path,
                clipboard,
                config,
                result_for_app,
            )))
        }),
    );

    let lock = result.lock().unwrap();
    lock.clone()
}

/// Held across a drag so the handler doesn't flip mid-drag when `selection`
/// transitions from None → Some.
#[derive(Default, PartialEq, Eq)]
enum Mode {
    #[default]
    SelectingRegion,
    Annotating,
}

/// Fingerprint of a single committed Blur — used to detect changes without
/// implementing Hash/Eq on the annotation type. f32-bit-pattern comparison is
/// fine: the values come straight from drag state, no NaN/arith.
type BlurKey = (u32, u32, u32, u32, u32);

fn blur_key(b: Bounds, sigma: f32) -> BlurKey {
    (
        b.x.to_bits(),
        b.y.to_bits(),
        b.w.to_bits(),
        b.h.to_bits(),
        sigma.to_bits(),
    )
}

struct OverlayApp {
    image: RgbaImage,
    save_path: String,
    clipboard_pref: bool,
    palette: Vec<Rgba<u8>>,
    counter_radius: f32,
    blur_sigma: f32,
    /// `image` with all committed Blur annotations applied. Rebuilt only when
    /// the blur list changes.
    committed_base: Option<RgbaImage>,
    committed_blur_sig: Vec<BlurKey>,
    texture: Option<egui::TextureHandle>,
    /// Small texture for the in-progress Blur draft, updated every frame
    /// during drag so the user sees the real blur (not a placeholder square).
    draft_blur_tex: Option<egui::TextureHandle>,
    draft_blur_rect: Option<egui::Rect>,
    mode: Mode,
    selection: Option<egui::Rect>,
    sel_drag_start: Option<egui::Pos2>,
    selection_edit: SelectionEdit,
    edit_drag_start: Option<egui::Pos2>,
    edit_rect_start: Option<egui::Rect>,
    draft: Option<Draft>,
    canvas: Canvas,
    result: Arc<Mutex<UiResult>>,
}

impl OverlayApp {
    fn new(
        image: RgbaImage,
        save_path: String,
        clipboard: bool,
        config: Arc<Config>,
        result: Arc<Mutex<UiResult>>,
    ) -> Self {
        let mut canvas = Canvas::default();
        if let Some(c) = config::parse_color(&config.defaults.color) {
            canvas.style.color = c;
        }
        canvas.style.width = config.defaults.width.max(1.0);
        if let Some(t) = config::parse_tool(&config.defaults.initial_tool) {
            canvas.tool = t;
        }
        let palette = config
            .palette
            .colors
            .iter()
            .filter_map(|s| config::parse_color(s))
            .collect();
        Self {
            image,
            save_path,
            clipboard_pref: clipboard,
            palette,
            counter_radius: config.defaults.counter_radius.max(4.0),
            blur_sigma: config.defaults.blur_sigma.max(0.5),
            committed_base: None,
            committed_blur_sig: Vec::new(),
            texture: None,
            draft_blur_tex: None,
            draft_blur_rect: None,
            mode: Mode::default(),
            selection: None,
            sel_drag_start: None,
            selection_edit: SelectionEdit::None,
            edit_drag_start: None,
            edit_rect_start: None,
            draft: None,
            canvas,
            result,
        }
    }

    fn finish(&mut self, value: UiResult, ctx: &egui::Context) {
        *self.result.lock().unwrap() = value;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Apply annotations onto a working copy, crop to selection if present.
    fn compose(&self) -> RgbaImage {
        let mut working = self.image.clone();
        render::rasterize(&mut working, &self.canvas.annotations);
        if let Some(sel) = self.selection {
            let (x, y, w, h) = clamp_to_image(sel, self.image.width(), self.image.height());
            if w > 0 && h > 0 {
                return image::imageops::crop_imm(&working, x, y, w, h).to_image();
            }
        }
        working
    }

    /// Save to disk (if a path is set) and/or copy to clipboard.
    /// `force_copy` = triggered by Copy action; otherwise we copy only
    /// when the user passed `-c`.
    fn act(&mut self, force_copy: bool) -> UiResult {
        let img = self.compose();
        if !self.save_path.is_empty() {
            match export::file::save_png(&img, std::path::Path::new(&self.save_path)) {
                Ok(()) => tracing::info!(
                    path = %self.save_path, w = img.width(), h = img.height(), "saved"
                ),
                Err(e) => tracing::error!("save failed: {e}"),
            }
        }
        if self.clipboard_pref || force_copy {
            if let Err(e) = export::clipboard::copy(&img) {
                tracing::error!("clipboard copy failed: {e}");
            } else {
                tracing::info!(w = img.width(), h = img.height(), "copied to clipboard");
            }
        }
        UiResult::Done
    }
}

impl eframe::App for OverlayApp {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        refresh_base_texture(self, ctx);
        refresh_draft_blur(self, ctx);
        let texture_id = self.texture.as_ref().unwrap().id();
        let mut early_finish: Option<UiResult> = None;

        let k = Keys::read(ctx);

        if k.esc {
            self.finish(UiResult::Cancelled, ctx);
            return;
        }
        if k.ctrl_z { self.canvas.undo(); }
        if k.ctrl_y { self.canvas.redo(); }
        if k.enter {
            let r = self.act(false);
            self.finish(r, ctx);
            return;
        }
        if k.ctrl_c {
            let r = self.act(true);
            self.finish(r, ctx);
            return;
        }
        if let Some(t) = k.tool_swap {
            self.canvas.tool = t;
        }

        if let Some(action) = toolbar::show(ctx, &mut self.canvas, &self.palette) {
            match action {
                toolbar::Action::Cancel => { self.finish(UiResult::Cancelled, ctx); return; }
                toolbar::Action::Undo => self.canvas.undo(),
                toolbar::Action::Redo => self.canvas.redo(),
            }
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(egui::Color32::BLACK))
            .show(ctx, |ui| {
                let screen_rect = ui.max_rect();
                let response = ui.interact(
                    screen_rect,
                    egui::Id::new("rustshot-overlay-canvas"),
                    egui::Sense::click_and_drag(),
                );

                match self.mode {
                    Mode::SelectingRegion => handle_region_drag(self, &response),
                    Mode::Annotating => handle_tool_input(self, &response, ctx),
                }

                ctx.set_cursor_icon(pick_cursor(self, &response, ctx));

                let painter = ui.painter();
                painter.image(
                    texture_id,
                    screen_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                if let (Some(tex), Some(rect)) =
                    (self.draft_blur_tex.as_ref(), self.draft_blur_rect)
                {
                    painter.image(
                        tex.id(),
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }

                let dim = egui::Color32::from_black_alpha(128);
                if let Some(sel) = self.selection {
                    paint_dim_around(painter, screen_rect, sel, dim);
                    painter.rect_stroke(
                        sel,
                        0.0,
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 0)),
                    );
                    draw_handles(painter, sel);
                } else {
                    painter.rect_filled(screen_rect, 0.0, dim);
                    painter.text(
                        screen_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "drag to select a region  -  Enter saves full screen  -  Esc cancels",
                        egui::FontId::proportional(18.0),
                        egui::Color32::from_white_alpha(220),
                    );
                }

                preview::draw_annotations(painter, &self.canvas.annotations);
                if let Some(d) = &self.draft {
                    preview::draw_draft(painter, d);
                }

                if let Some(sel) = self.selection {
                    let strip = tool_buttons::strip_rect(screen_rect, sel);
                    if let Some(a) = tool_buttons::show(ui, strip, &mut self.canvas.tool) {
                        match a {
                            tool_buttons::Action::Save => {
                                early_finish = Some(self.act(false));
                            }
                            tool_buttons::Action::Copy => {
                                early_finish = Some(self.act(true));
                            }
                        }
                    }
                }
            });

        if let Some(r) = early_finish {
            self.finish(r, ctx);
            return;
        }

        // Only keep repainting while something is animating.
        if self.draft.is_some()
            || self.sel_drag_start.is_some()
            || self.selection_edit != SelectionEdit::None
        {
            ctx.request_repaint();
        }
    }
}

/// Per-frame keyboard snapshot. Reading the input once keeps the borrow on
/// `ctx` small and the dispatch tidy.
struct Keys {
    esc: bool,
    enter: bool,
    ctrl_c: bool,
    ctrl_z: bool,
    ctrl_y: bool,
    tool_swap: Option<ToolKind>,
}

impl Keys {
    fn read(ctx: &egui::Context) -> Self {
        ctx.input(|i| Self {
            esc: i.key_pressed(egui::Key::Escape),
            enter: i.key_pressed(egui::Key::Enter),
            ctrl_c: i.modifiers.ctrl && i.key_pressed(egui::Key::C),
            ctrl_z: i.modifiers.ctrl && i.key_pressed(egui::Key::Z),
            ctrl_y: i.modifiers.ctrl && i.key_pressed(egui::Key::Y),
            tool_swap: tool_from_numkey(i),
        })
    }
}

fn tool_from_numkey(i: &egui::InputState) -> Option<ToolKind> {
    if i.key_pressed(egui::Key::Num1) { return Some(ToolKind::Pencil); }
    if i.key_pressed(egui::Key::Num2) { return Some(ToolKind::Arrow); }
    if i.key_pressed(egui::Key::Num3) { return Some(ToolKind::Rect); }
    if i.key_pressed(egui::Key::Num4) { return Some(ToolKind::Ellipse); }
    if i.key_pressed(egui::Key::Num5) { return Some(ToolKind::Blur); }
    if i.key_pressed(egui::Key::Num6) { return Some(ToolKind::Counter); }
    None
}

fn handle_region_drag(app: &mut OverlayApp, response: &egui::Response) {
    if response.drag_started() {
        app.sel_drag_start = response.interact_pointer_pos();
        app.selection = None;
    }
    if response.dragged() {
        if let (Some(start), Some(now)) = (app.sel_drag_start, response.interact_pointer_pos()) {
            app.selection = Some(egui::Rect::from_two_pos(start, now));
        }
    }
    if response.drag_stopped() {
        if let Some(sel) = app.selection {
            if sel.width() >= 4.0 && sel.height() >= 4.0 {
                app.mode = Mode::Annotating;
            } else {
                app.selection = None;
            }
        }
    }
}

fn handle_tool_input(app: &mut OverlayApp, response: &egui::Response, ctx: &egui::Context) {
    let style = app.canvas.style;
    let tool = app.canvas.tool;
    let pointer = response.interact_pointer_pos();

    if response.drag_started() {
        app.selection_edit = SelectionEdit::None;
        // egui's drag threshold means `pointer` has already drifted a few px from
        // the press point — use the actual press origin for handle hit-testing so
        // edge grabs (EDGE_HIT = 8px) aren't lost to that drift.
        let press = ctx.input(|i| i.pointer.press_origin()).or(pointer);
        if let (Some(sel), Some(p)) = (app.selection, press) {
            if let Some(h) = handle_at(sel, p) {
                app.selection_edit = SelectionEdit::Resizing(h);
                app.edit_drag_start = Some(p);
                app.edit_rect_start = Some(sel);
            } else if ctx.input(|i| i.modifiers.ctrl) && sel.contains(p) {
                app.selection_edit = SelectionEdit::Moving;
                app.edit_drag_start = Some(p);
                app.edit_rect_start = Some(sel);
            }
        }

        if app.selection_edit == SelectionEdit::None {
            if let (Some(p), Some(sel)) = (press, app.selection) {
                if sel.contains(p) {
                    app.draft = Draft::new(tool, pos_from(p), style, app.blur_sigma);
                }
            }
        }
    }

    if response.dragged() {
        match app.selection_edit {
            SelectionEdit::Resizing(h) => {
                if let (Some(p), Some(start), Some(rect)) =
                    (pointer, app.edit_drag_start, app.edit_rect_start)
                {
                    app.selection = Some(resize_rect(rect, h, p - start));
                }
            }
            SelectionEdit::Moving => {
                if let (Some(p), Some(start), Some(rect)) =
                    (pointer, app.edit_drag_start, app.edit_rect_start)
                {
                    app.selection = Some(rect.translate(p - start));
                }
            }
            SelectionEdit::None => {
                if let (Some(p), Some(d), Some(sel)) =
                    (pointer, app.draft.as_mut(), app.selection)
                {
                    d.extend(pos_from(sel.clamp(p)));
                }
            }
        }
    }

    if response.drag_stopped() {
        match app.selection_edit {
            SelectionEdit::None => {
                if let Some(d) = app.draft.take() {
                    if let Some(a) = d.finalize() {
                        app.canvas.push(a);
                    }
                }
            }
            _ => {
                app.selection_edit = SelectionEdit::None;
                app.edit_drag_start = None;
                app.edit_rect_start = None;
            }
        }
    }

    if app.selection_edit == SelectionEdit::None
        && tool == ToolKind::Counter
        && response.clicked()
    {
        if let (Some(p), Some(sel)) = (pointer, app.selection) {
            if sel.contains(p) {
                let n = app.canvas.next_counter();
                app.canvas.push(Annotation::Counter {
                    center: pos_from(p),
                    number: n,
                    color: style.color,
                    radius: app.counter_radius,
                });
            }
        }
    }
}

fn pick_cursor(
    app: &OverlayApp,
    response: &egui::Response,
    ctx: &egui::Context,
) -> egui::CursorIcon {
    match app.selection_edit {
        SelectionEdit::Resizing(h) => return cursor_for_handle(h),
        SelectionEdit::Moving => return egui::CursorIcon::Grabbing,
        SelectionEdit::None => {}
    }
    let Some(hover) = response.hover_pos() else {
        return egui::CursorIcon::Crosshair;
    };
    let Some(sel) = app.selection else {
        return egui::CursorIcon::Crosshair;
    };
    if let Some(h) = handle_at(sel, hover) {
        return cursor_for_handle(h);
    }
    if sel.contains(hover) {
        if ctx.input(|i| i.modifiers.ctrl) {
            egui::CursorIcon::Grab
        } else {
            egui::CursorIcon::Crosshair
        }
    } else {
        egui::CursorIcon::Default
    }
}

fn paint_dim_around(
    painter: &egui::Painter,
    screen: egui::Rect,
    sel: egui::Rect,
    dim: egui::Color32,
) {
    let sel = sel.intersect(screen);
    let top = egui::Rect::from_min_max(screen.left_top(), egui::pos2(screen.right(), sel.top()));
    let bottom = egui::Rect::from_min_max(
        egui::pos2(screen.left(), sel.bottom()),
        screen.right_bottom(),
    );
    let left = egui::Rect::from_min_max(
        egui::pos2(screen.left(), sel.top()),
        egui::pos2(sel.left(), sel.bottom()),
    );
    let right = egui::Rect::from_min_max(
        egui::pos2(sel.right(), sel.top()),
        egui::pos2(screen.right(), sel.bottom()),
    );
    for r in [top, bottom, left, right] {
        if r.width() > 0.0 && r.height() > 0.0 {
            painter.rect_filled(r, 0.0, dim);
        }
    }
}

/// Rebuild `texture` (and `committed_base`) whenever the committed Blur list
/// changes. Non-blur annotations stay as egui primitives drawn in preview.
fn refresh_base_texture(app: &mut OverlayApp, ctx: &egui::Context) {
    let current: Vec<BlurKey> = app
        .canvas
        .annotations
        .iter()
        .filter_map(|a| match a {
            Annotation::Blur { rect, sigma } => Some(blur_key(*rect, *sigma)),
            _ => None,
        })
        .collect();

    let needs_rebuild = app.texture.is_none() || app.committed_blur_sig != current;
    if !needs_rebuild {
        return;
    }

    let mut base = app.image.clone();
    for a in &app.canvas.annotations {
        if let Annotation::Blur { rect, sigma } = a {
            if let Some((x, y, blurred)) = render::blur_crop(&base, *rect, *sigma) {
                image::imageops::replace(&mut base, &blurred, x as i64, y as i64);
            }
        }
    }

    let size = [base.width() as usize, base.height() as usize];
    let img = egui::ColorImage::from_rgba_unmultiplied(size, base.as_raw());
    match app.texture.as_mut() {
        Some(h) => h.set(img, egui::TextureOptions::LINEAR),
        None => {
            app.texture = Some(ctx.load_texture("base", img, egui::TextureOptions::LINEAR));
        }
    }
    app.committed_base = Some(base);
    app.committed_blur_sig = current;
}

/// Update the small draft-blur texture so the in-progress Blur shows the
/// real blur effect (not a placeholder). Blurs off `committed_base` so a
/// draft over an existing blur composites correctly.
fn refresh_draft_blur(app: &mut OverlayApp, ctx: &egui::Context) {
    let Some(Draft::Blur { start, end, sigma }) = app.draft.as_ref() else {
        app.draft_blur_tex = None;
        app.draft_blur_rect = None;
        return;
    };

    let x0 = start.x.min(end.x);
    let y0 = start.y.min(end.y);
    let x1 = start.x.max(end.x);
    let y1 = start.y.max(end.y);
    if x1 - x0 < 2.0 || y1 - y0 < 2.0 {
        app.draft_blur_tex = None;
        app.draft_blur_rect = None;
        return;
    }

    let src = app.committed_base.as_ref().unwrap_or(&app.image);
    let bounds = Bounds {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    };
    let Some((x, y, blurred)) = render::blur_crop(src, bounds, *sigma) else {
        app.draft_blur_tex = None;
        app.draft_blur_rect = None;
        return;
    };

    let size = [blurred.width() as usize, blurred.height() as usize];
    let cimg = egui::ColorImage::from_rgba_unmultiplied(size, blurred.as_raw());
    match app.draft_blur_tex.as_mut() {
        Some(h) => h.set(cimg, egui::TextureOptions::LINEAR),
        None => {
            app.draft_blur_tex =
                Some(ctx.load_texture("rs-draft-blur", cimg, egui::TextureOptions::LINEAR));
        }
    }
    app.draft_blur_rect = Some(egui::Rect::from_min_max(
        egui::pos2(x as f32, y as f32),
        egui::pos2((x + blurred.width()) as f32, (y + blurred.height()) as f32),
    ));
}

fn clamp_to_image(sel: egui::Rect, max_w: u32, max_h: u32) -> (u32, u32, u32, u32) {
    let l = sel.left().max(0.0).min(max_w as f32) as u32;
    let t = sel.top().max(0.0).min(max_h as f32) as u32;
    let r = sel.right().max(0.0).min(max_w as f32) as u32;
    let b = sel.bottom().max(0.0).min(max_h as f32) as u32;
    (l, t, r.saturating_sub(l), b.saturating_sub(t))
}
