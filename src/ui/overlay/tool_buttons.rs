//! Flameshot-style floating tool strip: circular icon buttons shown right
//! below (or above, if there's no room) the yellow selection frame. Only
//! rendered after a region has been selected.
//!
//! Layout: six tool buttons, a thin vertical separator, then the Save and
//! Copy action buttons. Returns `Action` when an action button is clicked.

use crate::canvas::ToolKind;
use eframe::egui;

const BUTTON_D: f32 = 34.0;
const GAP: f32 = 6.0;
const GROUP_GAP: f32 = 14.0;
const PAD: f32 = 6.0;
const MARGIN: f32 = 10.0;

pub(super) enum Action {
    Save,
    Copy,
}

enum Glyph {
    Tool(ToolKind),
    Save,
    Copy,
}

/// Compute the strip rect for the current selection. Prefer below the frame;
/// flip above if there's no room; last-resort fallback pins to the screen's
/// bottom edge.
pub(super) fn strip_rect(screen: egui::Rect, sel: egui::Rect) -> egui::Rect {
    let tool_n = ToolKind::ALL.len() as f32;
    let action_n = 2.0;
    let w = tool_n * BUTTON_D + (tool_n - 1.0) * GAP
        + GROUP_GAP
        + action_n * BUTTON_D + (action_n - 1.0) * GAP
        + PAD * 2.0;
    let h = BUTTON_D + PAD * 2.0;

    let y = if sel.bottom() + MARGIN + h <= screen.bottom() {
        sel.bottom() + MARGIN
    } else if sel.top() - MARGIN - h >= screen.top() {
        sel.top() - MARGIN - h
    } else {
        (screen.bottom() - h - 4.0).max(screen.top() + 4.0)
    };
    let x = (sel.center().x - w / 2.0)
        .max(screen.left() + 4.0)
        .min(screen.right() - w - 4.0);
    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
}

pub(super) fn show(
    ui: &mut egui::Ui,
    strip: egui::Rect,
    active: &mut ToolKind,
) -> Option<Action> {
    let painter = ui.painter();
    painter.rect_filled(
        strip,
        8.0,
        egui::Color32::from_rgba_premultiplied(28, 28, 32, 230),
    );
    painter.rect_stroke(
        strip,
        8.0,
        egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(90, 90, 108, 255)),
    );

    let mut x = strip.left() + PAD;
    let y = strip.top() + PAD;

    for &tool in ToolKind::ALL.iter() {
        let btn = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(BUTTON_D, BUTTON_D));
        let id = egui::Id::new(("rs-tool-btn", tool as u32));
        let resp = ui
            .interact(btn, id, egui::Sense::click())
            .on_hover_text(tool.label());
        let is_active = *active == tool;
        draw_button(ui.painter(), btn, Glyph::Tool(tool), is_active, resp.hovered());
        if resp.clicked() {
            *active = tool;
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        x += BUTTON_D + GAP;
    }

    let sep_x = x + (GROUP_GAP - GAP) / 2.0;
    ui.painter().line_segment(
        [
            egui::pos2(sep_x, strip.top() + 10.0),
            egui::pos2(sep_x, strip.bottom() - 10.0),
        ],
        egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(90, 90, 108, 255)),
    );
    x += GROUP_GAP;

    let mut action = None;
    if action_button(ui, x, y, "rs-save-btn", Glyph::Save, "Save (Enter)") {
        action = Some(Action::Save);
    }
    x += BUTTON_D + GAP;
    if action_button(ui, x, y, "rs-copy-btn", Glyph::Copy, "Copy (Ctrl+C)") {
        action = Some(Action::Copy);
    }
    action
}

fn action_button(
    ui: &mut egui::Ui,
    x: f32,
    y: f32,
    id: &'static str,
    glyph: Glyph,
    tip: &'static str,
) -> bool {
    let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(BUTTON_D, BUTTON_D));
    let resp = ui
        .interact(rect, egui::Id::new(id), egui::Sense::click())
        .on_hover_text(tip);
    draw_button(ui.painter(), rect, glyph, false, resp.hovered());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

fn draw_button(
    painter: &egui::Painter,
    rect: egui::Rect,
    glyph: Glyph,
    active: bool,
    hovered: bool,
) {
    let c = rect.center();
    let r = rect.width() / 2.0;
    let (bg, ring, fg) = if active {
        (
            egui::Color32::from_rgb(255, 200, 0),
            egui::Color32::from_rgb(255, 220, 60),
            egui::Color32::BLACK,
        )
    } else if hovered {
        (
            egui::Color32::from_rgb(64, 64, 72),
            egui::Color32::from_rgb(200, 200, 220),
            egui::Color32::WHITE,
        )
    } else {
        (
            egui::Color32::from_rgb(48, 48, 56),
            egui::Color32::from_rgb(140, 140, 160),
            egui::Color32::WHITE,
        )
    };
    painter.circle_filled(c, r, bg);
    painter.circle_stroke(c, r, egui::Stroke::new(1.5, ring));
    paint_glyph(painter, c, rect.width(), glyph, fg, bg);
}

fn paint_glyph(
    painter: &egui::Painter,
    c: egui::Pos2,
    d: f32,
    glyph: Glyph,
    fg: egui::Color32,
    bg: egui::Color32,
) {
    let stroke = egui::Stroke::new(2.0, fg);
    match glyph {
        Glyph::Tool(ToolKind::Pencil) => {
            let a = c + egui::vec2(-d * 0.24, d * 0.24);
            let b = c + egui::vec2(d * 0.24, -d * 0.24);
            painter.line_segment([a, b], stroke);
            painter.circle_filled(b, 2.0, fg);
        }
        Glyph::Tool(ToolKind::Arrow) => {
            let a = c + egui::vec2(-d * 0.26, d * 0.22);
            let b = c + egui::vec2(d * 0.26, -d * 0.22);
            painter.line_segment([a, b], stroke);
            let h = d * 0.16;
            let dir = egui::vec2(1.0, -1.0).normalized();
            let perp = egui::vec2(-dir.y, dir.x);
            painter.line_segment([b, b - dir * h + perp * h * 0.6], stroke);
            painter.line_segment([b, b - dir * h - perp * h * 0.6], stroke);
        }
        Glyph::Tool(ToolKind::Rect) => {
            let w = d * 0.54;
            let h = d * 0.40;
            let r = egui::Rect::from_center_size(c, egui::vec2(w, h));
            painter.rect_stroke(r, 1.0, stroke);
        }
        Glyph::Tool(ToolKind::Ellipse) => {
            draw_ellipse(painter, c.x, c.y, d * 0.28, d * 0.20, stroke);
        }
        Glyph::Tool(ToolKind::Blur) => {
            painter.circle_stroke(c, d * 0.10, stroke);
            painter.circle_stroke(c, d * 0.20, stroke);
            painter.circle_stroke(c, d * 0.30, stroke);
        }
        Glyph::Tool(ToolKind::Counter) => {
            painter.circle_stroke(c, d * 0.28, stroke);
            painter.text(
                c,
                egui::Align2::CENTER_CENTER,
                "1",
                egui::FontId::proportional(d * 0.42),
                fg,
            );
        }
        Glyph::Save => {
            let w = d * 0.52;
            let h = d * 0.52;
            let outer = egui::Rect::from_center_size(c, egui::vec2(w, h));
            painter.rect_stroke(outer, 2.0, stroke);
            let lbl = egui::Rect::from_center_size(
                c + egui::vec2(0.0, h * 0.18),
                egui::vec2(w * 0.66, h * 0.44),
            );
            painter.rect_filled(lbl, 1.0, fg);
        }
        Glyph::Copy => {
            let side = d * 0.34;
            let off = d * 0.08;
            let back = egui::Rect::from_min_size(
                c + egui::vec2(-off * 2.0, -off * 2.0),
                egui::vec2(side, side),
            );
            painter.rect_stroke(back, 1.5, stroke);
            let front = egui::Rect::from_min_size(c, egui::vec2(side, side));
            painter.rect_filled(front, 1.5, bg);
            painter.rect_stroke(front, 1.5, stroke);
        }
    }
}

fn draw_ellipse(painter: &egui::Painter, cx: f32, cy: f32, rx: f32, ry: f32, stroke: egui::Stroke) {
    const SEG: usize = 48;
    let mut pts = Vec::with_capacity(SEG + 1);
    for i in 0..=SEG {
        let t = (i as f32 / SEG as f32) * std::f32::consts::TAU;
        pts.push(egui::pos2(cx + rx * t.cos(), cy + ry * t.sin()));
    }
    painter.add(egui::Shape::line(pts, stroke));
}
