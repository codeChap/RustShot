use crate::canvas::Canvas;
use eframe::egui;
use image::Rgba;

pub enum Action {
    Cancel,
    Undo,
    Redo,
}

pub fn show(ctx: &egui::Context, canvas: &mut Canvas, palette: &[Rgba<u8>]) -> Option<Action> {
    let mut action = None;
    // Floating Area (not a bottom panel) so it doesn't shrink the central
    // canvas — a shrunk canvas forces egui to scale the image texture, which
    // throws off the 1:1 mapping between pointer coords and image pixel
    // coords (committed blurs and other annotations would snap upward).
    egui::Area::new(egui::Id::new("rustshot-toolbar"))
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -14.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::default()
                .fill(egui::Color32::from_rgba_premultiplied(28, 28, 32, 235))
                .stroke(egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgba_premultiplied(90, 90, 108, 255),
                ))
                .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                .rounding(10.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for &c in palette.iter() {
                            let active = canvas.style.color == c;
                            let (rect, resp) = ui
                                .allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                            let painter = ui.painter();
                            painter.rect_filled(
                                rect,
                                3.0,
                                egui::Color32::from_rgb(c.0[0], c.0[1], c.0[2]),
                            );
                            if active {
                                painter.rect_stroke(
                                    rect,
                                    3.0,
                                    egui::Stroke::new(2.0, egui::Color32::WHITE),
                                );
                            }
                            if resp.clicked() {
                                canvas.style.color = c;
                            }
                        }

                        ui.separator();
                        ui.label("Width");
                        ui.add(
                            egui::Slider::new(&mut canvas.style.width, 1.0..=20.0)
                                .show_value(false),
                        );

                        ui.separator();
                        if ui.button("Undo").on_hover_text("Ctrl+Z").clicked() {
                            action = Some(Action::Undo);
                        }
                        if ui.button("Redo").on_hover_text("Ctrl+Y").clicked() {
                            action = Some(Action::Redo);
                        }

                        ui.separator();
                        if ui.button("Cancel (Esc)").clicked() {
                            action = Some(Action::Cancel);
                        }
                    });
                });
        });
    action
}
