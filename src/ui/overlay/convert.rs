//! Small conversion helpers between `eframe::egui` types and our own types.
//! Kept in one place so call sites stay terse.

use crate::canvas::Pos;
use eframe::egui;

pub(super) fn pos_from(p: egui::Pos2) -> Pos {
    Pos { x: p.x, y: p.y }
}

pub(super) fn egui_from(p: Pos) -> egui::Pos2 {
    egui::pos2(p.x, p.y)
}

pub(super) fn color_to_egui(c: image::Rgba<u8>) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], c.0[3])
}
