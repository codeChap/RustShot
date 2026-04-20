//! Live-preview rendering. Draws committed annotations and the in-progress
//! draft on the egui painter — what the user sees while the overlay is open.
//! (Final rasterization to pixels happens in `canvas::render`.)

use super::convert::{color_to_egui, egui_from};
use super::draft::Draft;
use crate::canvas::{Annotation, Pos};
use eframe::egui;

pub(super) fn draw_annotations(painter: &egui::Painter, annotations: &[Annotation]) {
    for a in annotations {
        draw_annotation(painter, a);
    }
}

fn draw_annotation(painter: &egui::Painter, a: &Annotation) {
    match a {
        Annotation::Pencil { points, color, width } => {
            draw_polyline(painter, points, *width, color_to_egui(*color));
        }
        Annotation::Arrow { start, end, color, width } => {
            let c = color_to_egui(*color);
            painter.line_segment([egui_from(*start), egui_from(*end)], egui::Stroke::new(*width, c));
            draw_arrowhead(painter, *start, *end, *width, c);
        }
        Annotation::Rect { rect, color, width } => {
            painter.rect_stroke(
                to_egui_rect(rect.x, rect.y, rect.w, rect.h),
                0.0,
                egui::Stroke::new(*width, color_to_egui(*color)),
            );
        }
        Annotation::Ellipse { rect, color, width } => {
            draw_ellipse(
                painter,
                rect.x + rect.w / 2.0,
                rect.y + rect.h / 2.0,
                rect.w / 2.0,
                rect.h / 2.0,
                egui::Stroke::new(*width, color_to_egui(*color)),
            );
        }
        Annotation::Blur { rect, .. } => {
            let r = to_egui_rect(rect.x, rect.y, rect.w, rect.h);
            painter.rect_filled(r, 0.0, egui::Color32::from_white_alpha(40));
            painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, egui::Color32::WHITE));
        }
        Annotation::Counter { center, number, color, radius } => {
            let c = color_to_egui(*color);
            let pos = egui_from(*center);
            painter.circle_filled(pos, *radius, egui::Color32::WHITE);
            painter.circle_stroke(pos, *radius, egui::Stroke::new(2.5, c));
            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                number.to_string(),
                egui::FontId::proportional(*radius * 1.1),
                c,
            );
        }
    }
}

pub(super) fn draw_draft(painter: &egui::Painter, draft: &Draft) {
    match draft {
        // Pencil's points get cloned by finalize(); draw directly to skip
        // the per-frame Vec clone in the hot path.
        Draft::Pencil { points, style } => {
            draw_polyline(painter, points, style.width, color_to_egui(style.color));
        }
        // Other drafts are tiny enums — finalize+draw is cheap.
        other => {
            if let Some(annotation) = other.clone().finalize() {
                draw_annotation(painter, &annotation);
            }
        }
    }
}

fn draw_polyline(painter: &egui::Painter, points: &[Pos], width: f32, color: egui::Color32) {
    if points.len() < 2 {
        return;
    }
    let stroke = egui::Stroke::new(width, color);
    for win in points.windows(2) {
        painter.line_segment([egui_from(win[0]), egui_from(win[1])], stroke);
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
    let (ux, uy) = (dx / len, dy / len);
    let angle = 28f32.to_radians();
    let (cos_a, sin_a) = (angle.cos(), angle.sin());
    let h1 = egui::pos2(
        end.x - head * (ux * cos_a - uy * sin_a),
        end.y - head * (uy * cos_a + ux * sin_a),
    );
    let h2 = egui::pos2(
        end.x - head * (ux * cos_a + uy * sin_a),
        end.y - head * (uy * cos_a - ux * sin_a),
    );
    let stroke = egui::Stroke::new(width, color);
    let tip = egui::pos2(end.x, end.y);
    painter.line_segment([tip, h1], stroke);
    painter.line_segment([tip, h2], stroke);
}

fn draw_ellipse(
    painter: &egui::Painter,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    stroke: egui::Stroke,
) {
    const SEGMENTS: usize = 64;
    let mut points = Vec::with_capacity(SEGMENTS + 1);
    for i in 0..=SEGMENTS {
        let t = (i as f32 / SEGMENTS as f32) * std::f32::consts::TAU;
        points.push(egui::pos2(cx + rx * t.cos(), cy + ry * t.sin()));
    }
    painter.add(egui::Shape::line(points, stroke));
}

fn to_egui_rect(x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
}
