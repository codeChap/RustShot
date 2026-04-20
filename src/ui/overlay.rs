use crate::canvas::{render, Annotation, Bounds, Canvas, Pos, Style, ToolKind};
use crate::config::{self, Config};
use crate::export;
use crate::ui::{toolbar, UiResult};
use eframe::egui;
use image::{Rgba, RgbaImage};
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

#[derive(Default, PartialEq, Eq)]
enum Mode {
    #[default]
    SelectingRegion,
    Annotating,
}

#[derive(Debug, Clone)]
enum Draft {
    Pencil { points: Vec<Pos>, style: Style },
    Arrow { start: Pos, end: Pos, style: Style },
    Rect { start: Pos, end: Pos, style: Style },
    Ellipse { start: Pos, end: Pos, style: Style },
    Blur { start: Pos, end: Pos, sigma: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handle { N, S, E, W, NE, NW, SE, SW }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionEdit {
    None,
    Moving,
    Resizing(Handle),
}

fn handle_positions(rect: egui::Rect) -> [(Handle, egui::Pos2); 8] {
    let cx = rect.center().x;
    let cy = rect.center().y;
    let l = rect.left();
    let r = rect.right();
    let t = rect.top();
    let b = rect.bottom();
    [
        (Handle::NW, egui::pos2(l, t)),
        (Handle::N,  egui::pos2(cx, t)),
        (Handle::NE, egui::pos2(r, t)),
        (Handle::E,  egui::pos2(r, cy)),
        (Handle::SE, egui::pos2(r, b)),
        (Handle::S,  egui::pos2(cx, b)),
        (Handle::SW, egui::pos2(l, b)),
        (Handle::W,  egui::pos2(l, cy)),
    ]
}

fn handle_at(rect: egui::Rect, pos: egui::Pos2) -> Option<Handle> {
    const HIT: f32 = 12.0;
    for (h, p) in handle_positions(rect) {
        if (pos - p).length() < HIT {
            return Some(h);
        }
    }
    None
}

fn resize_rect(rect: egui::Rect, handle: Handle, delta: egui::Vec2) -> egui::Rect {
    let mut min = rect.min;
    let mut max = rect.max;
    match handle {
        Handle::N  => min.y += delta.y,
        Handle::S  => max.y += delta.y,
        Handle::E  => max.x += delta.x,
        Handle::W  => min.x += delta.x,
        Handle::NE => { min.y += delta.y; max.x += delta.x; }
        Handle::NW => { min.y += delta.y; min.x += delta.x; }
        Handle::SE => { max.x += delta.x; max.y += delta.y; }
        Handle::SW => { min.x += delta.x; max.y += delta.y; }
    }
    if min.x > max.x { std::mem::swap(&mut min.x, &mut max.x); }
    if min.y > max.y { std::mem::swap(&mut min.y, &mut max.y); }
    egui::Rect::from_min_max(min, max)
}

fn draw_handles(painter: &egui::Painter, sel: egui::Rect) {
    for (_, p) in handle_positions(sel) {
        let r = egui::Rect::from_center_size(p, egui::vec2(8.0, 8.0));
        painter.rect_filled(r, 1.0, egui::Color32::WHITE);
        painter.rect_stroke(r, 1.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 200, 0)));
    }
}

impl Draft {
    fn finalize(self) -> Option<Annotation> {
        match self {
            Draft::Pencil { points, style } if points.len() >= 2 => Some(Annotation::Pencil {
                points,
                color: style.color,
                width: style.width,
            }),
            Draft::Pencil { .. } => None,
            Draft::Arrow { start, end, style } => {
                if dist2(start, end) < 4.0 {
                    None
                } else {
                    Some(Annotation::Arrow {
                        start,
                        end,
                        color: style.color,
                        width: style.width,
                    })
                }
            }
            Draft::Rect { start, end, style } => {
                let r = Bounds::from_two(start, end);
                if r.w < 2.0 || r.h < 2.0 {
                    None
                } else {
                    Some(Annotation::Rect {
                        rect: r,
                        color: style.color,
                        width: style.width,
                    })
                }
            }
            Draft::Ellipse { start, end, style } => {
                let r = Bounds::from_two(start, end);
                if r.w < 2.0 || r.h < 2.0 {
                    None
                } else {
                    Some(Annotation::Ellipse {
                        rect: r,
                        color: style.color,
                        width: style.width,
                    })
                }
            }
            Draft::Blur { start, end, sigma } => {
                let r = Bounds::from_two(start, end);
                if r.w < 2.0 || r.h < 2.0 {
                    None
                } else {
                    Some(Annotation::Blur { rect: r, sigma })
                }
            }
        }
    }
}

struct OverlayApp {
    image: RgbaImage,
    save_path: String,
    clipboard_pref: bool,
    palette: Vec<Rgba<u8>>,
    counter_radius: f32,
    blur_sigma: f32,
    texture: Option<egui::TextureHandle>,
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
        let palette: Vec<Rgba<u8>> = config
            .palette
            .colors
            .iter()
            .filter_map(|s| config::parse_color(s))
            .collect();
        let counter_radius = config.defaults.counter_radius.max(4.0);
        let blur_sigma = config.defaults.blur_sigma.max(0.5);
        Self {
            image,
            save_path,
            clipboard_pref: clipboard,
            palette,
            counter_radius,
            blur_sigma,
            texture: None,
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

    fn act_save(&mut self) -> UiResult {
        let img = self.compose();
        let mut acted = false;
        if !self.save_path.is_empty() {
            match export::file::save_png(&img, std::path::Path::new(&self.save_path)) {
                Ok(()) => {
                    tracing::info!(path = %self.save_path, w = img.width(), h = img.height(), "saved");
                    acted = true;
                }
                Err(e) => tracing::error!("save failed: {e}"),
            }
        }
        if self.clipboard_pref {
            match export::clipboard::copy(&img) {
                Ok(()) => {
                    tracing::info!(w = img.width(), h = img.height(), "copied to clipboard");
                    acted = true;
                }
                Err(e) => tracing::error!("clipboard copy failed: {e}"),
            }
        }
        if !acted {
            tracing::warn!("save action took no effect — no -p path and no -c flag");
        }
        UiResult::Done
    }

    fn act_copy(&mut self) -> UiResult {
        let img = self.compose();
        // Always save when copying so a file exists alongside the clipboard
        // — needed for tools that paste-by-path (Claude Code, etc.).
        // Skipped only when the daemon already stripped the path (--no-save).
        if !self.save_path.is_empty() {
            match export::file::save_png(&img, std::path::Path::new(&self.save_path)) {
                Ok(()) => tracing::info!(path = %self.save_path, "saved"),
                Err(e) => tracing::error!("save failed: {e}"),
            }
        }
        if let Err(e) = export::clipboard::copy(&img) {
            tracing::error!("clipboard copy failed: {e}");
        } else {
            tracing::info!(w = img.width(), h = img.height(), "copied to clipboard");
        }
        UiResult::Done
    }
}

impl eframe::App for OverlayApp {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.texture.is_none() {
            let size = [self.image.width() as usize, self.image.height() as usize];
            let img = egui::ColorImage::from_rgba_unmultiplied(size, self.image.as_raw());
            self.texture = Some(ctx.load_texture("base", img, egui::TextureOptions::LINEAR));
        }
        let texture_id = self.texture.as_ref().unwrap().id();

        let (esc, enter, ctrl_c, ctrl_z, ctrl_y, n1, n2, n3, n4, n5, n6) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Enter),
                i.modifiers.ctrl && i.key_pressed(egui::Key::C),
                i.modifiers.ctrl && i.key_pressed(egui::Key::Z),
                i.modifiers.ctrl && i.key_pressed(egui::Key::Y),
                i.key_pressed(egui::Key::Num1),
                i.key_pressed(egui::Key::Num2),
                i.key_pressed(egui::Key::Num3),
                i.key_pressed(egui::Key::Num4),
                i.key_pressed(egui::Key::Num5),
                i.key_pressed(egui::Key::Num6),
            )
        });

        if esc {
            self.finish(UiResult::Cancelled, ctx);
            return;
        }
        if ctrl_z {
            self.canvas.undo();
        }
        if ctrl_y {
            self.canvas.redo();
        }

        if self.mode == Mode::Annotating {
            if enter {
                let r = self.act_save();
                self.finish(r, ctx);
                return;
            }
            if ctrl_c {
                let r = self.act_copy();
                self.finish(r, ctx);
                return;
            }
            if n1 { self.canvas.tool = ToolKind::Pencil; }
            if n2 { self.canvas.tool = ToolKind::Arrow; }
            if n3 { self.canvas.tool = ToolKind::Rect; }
            if n4 { self.canvas.tool = ToolKind::Ellipse; }
            if n5 { self.canvas.tool = ToolKind::Blur; }
            if n6 { self.canvas.tool = ToolKind::Counter; }

            if let Some(action) = toolbar::show(ctx, &mut self.canvas, &self.palette) {
                match action {
                    toolbar::Action::Save => {
                        let r = self.act_save();
                        self.finish(r, ctx);
                        return;
                    }
                    toolbar::Action::Copy => {
                        let r = self.act_copy();
                        self.finish(r, ctx);
                        return;
                    }
                    toolbar::Action::Cancel => {
                        self.finish(UiResult::Cancelled, ctx);
                        return;
                    }
                    toolbar::Action::Undo => self.canvas.undo(),
                    toolbar::Action::Redo => self.canvas.redo(),
                }
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

                let painter = ui.painter();
                painter.image(
                    texture_id,
                    screen_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );

                let dim = egui::Color32::from_black_alpha(128);
                if let Some(sel) = self.selection {
                    paint_dim_around(painter, screen_rect, sel, dim);
                    painter.rect_stroke(
                        sel,
                        0.0,
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 0)),
                    );
                    if self.mode == Mode::Annotating {
                        draw_handles(painter, sel);
                    }
                } else {
                    painter.rect_filled(screen_rect, 0.0, dim);
                    painter.text(
                        screen_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "drag to select  -  Esc to cancel",
                        egui::FontId::proportional(18.0),
                        egui::Color32::from_white_alpha(220),
                    );
                }

                draw_annotations(painter, &self.canvas.annotations);
                if let Some(d) = &self.draft {
                    draw_draft(painter, d);
                }
            });

        // Continuous repaint only while live preview is changing.
        // egui auto-repaints on input events otherwise.
        if self.draft.is_some()
            || self.sel_drag_start.is_some()
            || self.selection_edit != SelectionEdit::None
        {
            ctx.request_repaint();
        }
    }
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
    let blur_sigma = app.blur_sigma;
    let counter_radius = app.counter_radius;
    let pointer = response.interact_pointer_pos();

    if response.drag_started() {
        // Decide: are we editing the selection (handle-drag or Ctrl-move), or drafting an annotation?
        app.selection_edit = SelectionEdit::None;
        if let (Some(sel), Some(p)) = (app.selection, pointer) {
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
            if let Some(p) = pointer {
                app.draft = match tool {
                    ToolKind::Pencil => Some(Draft::Pencil {
                        points: vec![pos_from(p)],
                        style,
                    }),
                    ToolKind::Arrow => Some(Draft::Arrow {
                        start: pos_from(p),
                        end: pos_from(p),
                        style,
                    }),
                    ToolKind::Rect => Some(Draft::Rect {
                        start: pos_from(p),
                        end: pos_from(p),
                        style,
                    }),
                    ToolKind::Ellipse => Some(Draft::Ellipse {
                        start: pos_from(p),
                        end: pos_from(p),
                        style,
                    }),
                    ToolKind::Blur => Some(Draft::Blur {
                        start: pos_from(p),
                        end: pos_from(p),
                        sigma: blur_sigma,
                    }),
                    ToolKind::Counter => None,
                };
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
                if let Some(p) = pointer {
                    if let Some(d) = &mut app.draft {
                        match d {
                            Draft::Pencil { points, .. } => points.push(pos_from(p)),
                            Draft::Arrow { end, .. }
                            | Draft::Rect { end, .. }
                            | Draft::Ellipse { end, .. }
                            | Draft::Blur { end, .. } => *end = pos_from(p),
                        }
                    }
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

    // Counter is click-only; don't fire while editing the selection.
    if app.selection_edit == SelectionEdit::None
        && tool == ToolKind::Counter
        && response.clicked()
    {
        if let Some(p) = pointer {
            let n = app.canvas.next_counter();
            app.canvas.push(Annotation::Counter {
                center: pos_from(p),
                number: n,
                color: style.color,
                radius: counter_radius,
            });
        }
    }
}

fn draw_annotations(painter: &egui::Painter, annotations: &[Annotation]) {
    for a in annotations {
        draw_annotation(painter, a);
    }
}

fn draw_annotation(painter: &egui::Painter, a: &Annotation) {
    match a {
        Annotation::Pencil { points, color, width } => {
            let stroke = egui::Stroke::new(*width, color_to_egui(*color));
            for w2 in points.windows(2) {
                painter.line_segment([egui_from(w2[0]), egui_from(w2[1])], stroke);
            }
        }
        Annotation::Arrow { start, end, color, width } => {
            let stroke = egui::Stroke::new(*width, color_to_egui(*color));
            painter.line_segment([egui_from(*start), egui_from(*end)], stroke);
            draw_arrowhead(painter, *start, *end, *width, color_to_egui(*color));
        }
        Annotation::Rect { rect, color, width } => {
            painter.rect_stroke(
                egui::Rect::from_min_size(egui::pos2(rect.x, rect.y), egui::vec2(rect.w, rect.h)),
                0.0,
                egui::Stroke::new(*width, color_to_egui(*color)),
            );
        }
        Annotation::Ellipse { rect, color, width } => {
            let cx = rect.x + rect.w / 2.0;
            let cy = rect.y + rect.h / 2.0;
            let rx = rect.w / 2.0;
            let ry = rect.h / 2.0;
            draw_ellipse_egui(
                painter,
                cx,
                cy,
                rx,
                ry,
                egui::Stroke::new(*width, color_to_egui(*color)),
            );
        }
        Annotation::Blur { rect, .. } => {
            let r = egui::Rect::from_min_size(
                egui::pos2(rect.x, rect.y),
                egui::vec2(rect.w, rect.h),
            );
            painter.rect_filled(r, 0.0, egui::Color32::from_white_alpha(40));
            painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, egui::Color32::WHITE));
        }
        Annotation::Counter { center, number, color, radius } => {
            let pos = egui_from(*center);
            painter.circle_filled(pos, *radius, egui::Color32::WHITE);
            painter.circle_stroke(pos, *radius, egui::Stroke::new(2.5, color_to_egui(*color)));
            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                number.to_string(),
                egui::FontId::proportional(*radius * 1.1),
                color_to_egui(*color),
            );
        }
    }
}

fn draw_draft(painter: &egui::Painter, draft: &Draft) {
    match draft {
        // Pencil's points get cloned by finalize(); draw directly to skip the per-frame Vec clone.
        Draft::Pencil { points, style } => {
            if points.len() < 2 {
                return;
            }
            let stroke = egui::Stroke::new(style.width, color_to_egui(style.color));
            for w2 in points.windows(2) {
                painter.line_segment([egui_from(w2[0]), egui_from(w2[1])], stroke);
            }
        }
        // Other drafts have no Vec; finalize+draw is a tiny enum copy.
        other => {
            if let Some(annotation) = other.clone().finalize() {
                draw_annotation(painter, &annotation);
            }
        }
    }
}

fn draw_arrowhead(painter: &egui::Painter, start: Pos, end: Pos, width: f32, color: egui::Color32) {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let head = (width * 4.0).max(12.0);
    let ux = dx / len;
    let uy = dy / len;
    let angle = 28f32.to_radians();
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    let h1 = egui::pos2(
        end.x - head * (ux * cos_a - uy * sin_a),
        end.y - head * (uy * cos_a + ux * sin_a),
    );
    let h2 = egui::pos2(
        end.x - head * (ux * cos_a + uy * sin_a),
        end.y - head * (uy * cos_a - ux * sin_a),
    );
    let stroke = egui::Stroke::new(width, color);
    painter.line_segment([egui::pos2(end.x, end.y), h1], stroke);
    painter.line_segment([egui::pos2(end.x, end.y), h2], stroke);
}

fn draw_ellipse_egui(painter: &egui::Painter, cx: f32, cy: f32, rx: f32, ry: f32, stroke: egui::Stroke) {
    let segments = 64;
    let mut points = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let t = (i as f32 / segments as f32) * std::f32::consts::TAU;
        points.push(egui::pos2(cx + rx * t.cos(), cy + ry * t.sin()));
    }
    painter.add(egui::Shape::line(points, stroke));
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

fn clamp_to_image(sel: egui::Rect, max_w: u32, max_h: u32) -> (u32, u32, u32, u32) {
    let l = sel.left().max(0.0).min(max_w as f32) as u32;
    let t = sel.top().max(0.0).min(max_h as f32) as u32;
    let r = sel.right().max(0.0).min(max_w as f32) as u32;
    let b = sel.bottom().max(0.0).min(max_h as f32) as u32;
    let w = r.saturating_sub(l);
    let h = b.saturating_sub(t);
    (l, t, w, h)
}

fn dist2(a: Pos, b: Pos) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

fn pos_from(p: egui::Pos2) -> Pos {
    Pos { x: p.x, y: p.y }
}

fn egui_from(p: Pos) -> egui::Pos2 {
    egui::pos2(p.x, p.y)
}

fn color_to_egui(c: image::Rgba<u8>) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], c.0[3])
}
