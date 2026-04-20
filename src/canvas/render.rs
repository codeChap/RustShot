use crate::canvas::{Annotation, Bounds, Pos};
use ab_glyph::{Font, FontRef};
use image::{Rgba, RgbaImage};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, PixmapMut, Rect, Stroke, Transform,
};

const FONT_BYTES: &[u8] = include_bytes!("../../assets/font.ttf");

fn font() -> &'static FontRef<'static> {
    static FONT: std::sync::OnceLock<FontRef<'static>> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        FontRef::try_from_slice(FONT_BYTES).expect("embedded font is valid TTF")
    })
}

pub fn rasterize(img: &mut RgbaImage, annotations: &[Annotation]) {
    if annotations.is_empty() {
        return;
    }

    // 1. Blur passes mutate img directly via crop+replace, before vector overlays.
    for a in annotations {
        if let Annotation::Blur { rect, sigma } = a {
            blur_region(img, *rect, *sigma);
        }
    }

    // 2. Vector primitives via tiny-skia, sharing the image's pixel buffer.
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
                Annotation::Blur { .. } => {}
                Annotation::Pencil { points, color, width } => {
                    draw_polyline(&mut pixmap, points, *color, *width);
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

    // 3. Counter numbers via imageproc::draw_text_mut.
    let font = font();
    for a in annotations {
        if let Annotation::Counter { center, number, color, radius } = a {
            draw_counter_text(img, *center, *number, *color, *radius, font);
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

fn blur_region(img: &mut RgbaImage, b: Bounds, sigma: f32) {
    let img_w = img.width();
    let img_h = img.height();
    let x = b.x.max(0.0) as u32;
    let y = b.y.max(0.0) as u32;
    let w = (b.w.max(0.0) as u32).min(img_w.saturating_sub(x));
    let h = (b.h.max(0.0) as u32).min(img_h.saturating_sub(y));
    if w == 0 || h == 0 {
        return;
    }
    let cropped = image::imageops::crop_imm(img, x, y, w, h).to_image();
    let blurred = imageproc::filter::gaussian_blur_f32(&cropped, sigma.max(0.5));
    image::imageops::replace(img, &blurred, x as i64, y as i64);
}
