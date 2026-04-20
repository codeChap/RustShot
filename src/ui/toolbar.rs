use crate::canvas::{Canvas, ToolKind};
use eframe::egui;
use image::Rgba;

pub enum Action {
    Save,
    Copy,
    Cancel,
    Undo,
    Redo,
}

pub fn show(ctx: &egui::Context, canvas: &mut Canvas, palette: &[Rgba<u8>]) -> Option<Action> {
    let mut action = None;
    egui::TopBottomPanel::bottom("rustshot-toolbar")
        .frame(
            egui::Frame::default()
                .fill(egui::Color32::from_rgb(28, 28, 32))
                .inner_margin(egui::Margin::symmetric(10.0, 8.0)),
        )
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, t) in ToolKind::ALL.iter().enumerate() {
                    let label = format!("{} ({})", t.label(), i + 1);
                    if ui.selectable_label(canvas.tool == *t, label).clicked() {
                        canvas.tool = *t;
                    }
                }

                ui.separator();

                for &c in palette.iter() {
                    let active = canvas.style.color == c;
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                    let painter = ui.painter();
                    painter.rect_filled(rect, 3.0, egui::Color32::from_rgb(c.0[0], c.0[1], c.0[2]));
                    if active {
                        painter.rect_stroke(rect, 3.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
                    }
                    if resp.clicked() {
                        canvas.style.color = c;
                    }
                }

                ui.separator();
                ui.label("Width");
                ui.add(egui::Slider::new(&mut canvas.style.width, 1.0..=20.0).show_value(false));

                ui.separator();
                if ui.button("Undo").on_hover_text("Ctrl+Z").clicked() {
                    action = Some(Action::Undo);
                }
                if ui.button("Redo").on_hover_text("Ctrl+Y").clicked() {
                    action = Some(Action::Redo);
                }

                ui.separator();
                if ui.button("Save (Enter)").clicked() {
                    action = Some(Action::Save);
                }
                if ui.button("Copy (Ctrl+C)").clicked() {
                    action = Some(Action::Copy);
                }
                if ui.button("Cancel (Esc)").clicked() {
                    action = Some(Action::Cancel);
                }
            });
        });
    action
}
