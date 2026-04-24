use crate::canvas::{Annotation, Bounds, Pos};
use ab_glyph::{Font, FontRef};
use image::{Rgba, RgbaImage};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, PixmapMut, Rect, Stroke, Transform,
};

const FONT_BYTES: &[u8] = include_bytes!("../../assets/font.ttf");

pub(crate) fn font() -> &'static FontRef<'static> {
    static FONT: std::sync::OnceLock<FontRef<'static>> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        FontRef::try_from_slice(FONT_BYTES).expect("embedded font is valid TTF")
    })
}

/// Pass 2 + 3: vector primitives via tiny-skia, then counter text via imageproc.
/// Pixelate annotations are skipped — caller is responsible for having baked
/// them in already (or wanting them omitted, e.g. when `img` is the cached
/// committed_base).
pub fn rasterize_overlays(img: &mut RgbaImage, annotations: &[Annotation]) {
    if annotations.is_empty() {
        return;
    }
    let w = img.width();
    let h = img.height();
    {
        let buf = img.as_mut();
        let mut pixmap = match PixmapMut::from_bytes(buf, w, h) {
            Some(p) => p,
            None => {
                tracing::error!("PixmapMut::from_bytes failed (w={w}, h={h})");
                return;
            }
        };
        for a in annotations {
            match a {
                // Pixelate is baked into `base` upstream; Stamp is text-only.
                Annotation::Pixelate { .. } | Annotation::Stamp { .. } => {}
                Annotation::Pencil { points, color, width } => {
                    draw_polyline(&mut pixmap, points, *color, *width);
                }
                Annotation::Line { start, end, color, width } => {
                    draw_line(&mut pixmap, *start, *end, *color, *width);
                }
                Annotation::Arrow { start, end, color, width } => {
                    draw_arrow(&mut pixmap, *start, *end, *color, *width);
                }
                Annotation::Rect { rect, color, width } => {
                    draw_rect_outline(&mut pixmap, *rect, *color, *width);
                }
                Annotation::Ellipse { rect, color, width } => {
                    draw_ellipse_outline(&mut pixmap, *rect, *color, *width);
                }
                Annotation::Counter { center, color, radius, .. } => {
                    draw_counter_circle(&mut pixmap, *center, *color, *radius);
                }
            }
        }
    }

    let font = font();
    for a in annotations {
        match a {
            Annotation::Counter { center, number, color, radius } => {
                draw_counter_text(img, *center, *number, *color, *radius, font);
            }
            Annotation::Stamp { center, ch, color, size } => {
                draw_stamp_text(img, *center, *ch, *color, *size, font);
            }
            _ => {}
        }
    }
}

fn paint_for(color: Rgba<u8>) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(color.0[0], color.0[1], color.0[2], color.0[3]));
    p.anti_alias = true;
    p
}

fn stroke_for(width: f32) -> Stroke {
    let mut s = Stroke::default();
    s.width = width.max(0.5);
    s.line_cap = LineCap::Round;
    s.line_join = LineJoin::Round;
    s
}

fn draw_polyline(pixmap: &mut PixmapMut, points: &[Pos], color: Rgba<u8>, width: f32) {
    if points.len() < 2 {
        return;
    }
    let mut pb = PathBuilder::new();
    pb.move_to(points[0].x, points[0].y);
    for p in &points[1..] {
        pb.line_to(p.x, p.y);
    }
    if let Some(path) = pb.finish() {
        pixmap.stroke_path(
            &path,
            &paint_for(color),
            &stroke_for(width),
            Transform::identity(),
            None,
        );
    }
}

fn draw_line(pixmap: &mut PixmapMut, start: Pos, end: Pos, color: Rgba<u8>, width: f32) {
    let mut pb = PathBuilder::new();
    pb.move_to(start.x, start.y);
    pb.line_to(end.x, end.y);
    if let Some(path) = pb.finish() {
        pixmap.stroke_path(
            &path,
            &paint_for(color),
            &stroke_for(width),
            Transform::identity(),
            None,
        );
    }
}

fn draw_arrow(pixmap: &mut PixmapMut, start: Pos, end: Pos, color: Rgba<u8>, width: f32) {
    let mut pb = PathBuilder::new();
    pb.move_to(start.x, start.y);
    pb.line_to(end.x, end.y);
    if let Some(path) = pb.finish() {
        pixmap.stroke_path(
            &path,
            &paint_for(color),
            &stroke_for(width),
            Transform::identity(),
            None,
        );
    }

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
    let h1 = (
        end.x - head * (ux * cos_a - uy * sin_a),
        end.y - head * (uy * cos_a + ux * sin_a),
    );
    let h2 = (
        end.x - head * (ux * cos_a + uy * sin_a),
        end.y - head * (uy * cos_a - ux * sin_a),
    );
    let mut pb = PathBuilder::new();
    pb.move_to(h1.0, h1.1);
    pb.line_to(end.x, end.y);
    pb.line_to(h2.0, h2.1);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_for(color),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

fn draw_rect_outline(pixmap: &mut PixmapMut, b: Bounds, color: Rgba<u8>, width: f32) {
    if b.w <= 0.0 || b.h <= 0.0 {
        return;
    }
    let r = match Rect::from_xywh(b.x, b.y, b.w, b.h) {
        Some(r) => r,
        None => return,
    };
    let mut pb = PathBuilder::new();
    pb.push_rect(r);
    if let Some(path) = pb.finish() {
        pixmap.stroke_path(
            &path,
            &paint_for(color),
            &stroke_for(width),
            Transform::identity(),
            None,
        );
    }
}

fn draw_ellipse_outline(pixmap: &mut PixmapMut, b: Bounds, color: Rgba<u8>, width: f32) {
    if b.w <= 0.0 || b.h <= 0.0 {
        return;
    }
    let r = match Rect::from_xywh(b.x, b.y, b.w, b.h) {
        Some(r) => r,
        None => return,
    };
    let mut pb = PathBuilder::new();
    pb.push_oval(r);
    if let Some(path) = pb.finish() {
        pixmap.stroke_path(
            &path,
            &paint_for(color),
            &stroke_for(width),
            Transform::identity(),
            None,
        );
    }
}

fn draw_counter_circle(pixmap: &mut PixmapMut, center: Pos, color: Rgba<u8>, radius: f32) {
    let mut pb = PathBuilder::new();
    pb.push_circle(center.x, center.y, radius);
    let path = match pb.finish() {
        Some(p) => p,
        None => return,
    };
    let mut bg = Paint::default();
    bg.set_color(Color::WHITE);
    bg.anti_alias = true;
    pixmap.fill_path(&path, &bg, FillRule::Winding, Transform::identity(), None);
    pixmap.stroke_path(
        &path,
        &paint_for(color),
        &stroke_for(2.5),
        Transform::identity(),
        None,
    );
}

fn draw_counter_text(
    img: &mut RgbaImage,
    center: Pos,
    number: u32,
    color: Rgba<u8>,
    radius: f32,
    font: &impl Font,
) {
    let scale = ab_glyph::PxScale::from(radius * 1.2);
    let text = number.to_string();
    let (tw, th) = imageproc::drawing::text_size(scale, font, &text);
    let tx = center.x as i32 - tw as i32 / 2;
    let ty = center.y as i32 - th as i32 / 2;
    imageproc::drawing::draw_text_mut(img, color, tx, ty, scale, font, &text);
}

fn draw_stamp_text(
    img: &mut RgbaImage,
    center: Pos,
    ch: char,
    color: Rgba<u8>,
    size: f32,
    font: &impl Font,
) {
    let scale = ab_glyph::PxScale::from(size);
    let text = ch.to_string();
    let (tw, th) = imageproc::drawing::text_size(scale, font, &text);
    let tx = center.x as i32 - tw as i32 / 2;
    let ty = center.y as i32 - th as i32 / 2;
    imageproc::drawing::draw_text_mut(img, color, tx, ty, scale, font, &text);
}

/// Crop + pixelate a region via downscale→upscale (nearest). Returns the clamped
/// origin and the pixelated image so callers can paste it back into the base
/// (committed) or use it as a live preview (draft).
pub fn pixelate_crop(img: &RgbaImage, b: Bounds, block: u32) -> Option<(u32, u32, RgbaImage)> {
    let img_w = img.width();
    let img_h = img.height();
    let x = b.x.max(0.0) as u32;
    let y = b.y.max(0.0) as u32;
    let w = (b.w.max(0.0) as u32).min(img_w.saturating_sub(x));
    let h = (b.h.max(0.0) as u32).min(img_h.saturating_sub(y));
    if w == 0 || h == 0 {
        return None;
    }
    let block = block.max(2);
    let sw = (w / block).max(1);
    let sh = (h / block).max(1);
    let cropped = image::imageops::crop_imm(img, x, y, w, h).to_image();
    // Triangle downscale = area-averaged blocks; Nearest upscale keeps hard edges.
    let down = image::imageops::resize(&cropped, sw, sh, image::imageops::FilterType::Triangle);
    let up = image::imageops::resize(&down, w, h, image::imageops::FilterType::Nearest);
    Some((x, y, up))
}

