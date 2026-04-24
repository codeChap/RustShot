//! Composite pipeline: start from `base` (captured image + committed pixelates),
//! apply live annotations + UI chrome (dim, selection frame, handles, strip),
//! blit to the X11 window.
//!
//! Display buffer is an `RgbaImage` throughout; tiny-skia temporarily wraps it
//! via `PixmapMut::from_bytes` for vector work, imageproc takes it directly
//! for text. Keeps the pipeline single-buffer.

use super::draft::Draft;
use super::selection::{handle_corner_positions, HANDLE_VISUAL};
use super::state::OverlayState;
use super::tool_buttons;
use crate::canvas::{render, Annotation, Bounds};
use ab_glyph::PxScale;
use image::{Rgba, RgbaImage};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, PixmapMut, Stroke, Transform};

pub(super) fn composite(display: &mut RgbaImage, state: &OverlayState) {
    // 1. Start from dim_base (whole screen pre-dimmed). One memcpy.
    display.as_mut().copy_from_slice(state.dim_base.as_raw());

    // 2. If a selection is active, undim its interior by copying rows of
    //    `base` on top. Zero per-pixel math — just row-aligned memcpy.
    if let Some(sel) = state.selection {
        overlay_rect_from_base(display, &state.base, sel);
    }

    // 3. Rasterize committed non-pixelate annotations.
    // (Committed pixelate is already baked into `base` and `dim_base`.)
    let anns: Vec<Annotation> = state
        .canvas
        .annotations
        .iter()
        .filter(|a| !matches!(a, Annotation::Pixelate { .. }))
        .cloned()
        .collect();
    render::rasterize_overlays(display, &anns);

    // 4. Draft: live preview of the in-progress shape.
    if let Some(draft) = &state.draft {
        paint_draft(display, draft, state);
    }

    // 5. UI chrome: frame, handles, strip, or the initial hint.
    if let Some(sel) = state.selection {
        paint_selection_frame(display, sel);
        paint_handles(display, sel);
        let strip = tool_buttons::strip_rect(display.width(), display.height(), sel);
        tool_buttons::paint(display, strip, state.canvas.tool, state.strip_hover);
    } else {
        paint_hint_text(display);
    }
}

fn paint_draft(display: &mut RgbaImage, draft: &Draft, state: &OverlayState) {
    match draft {
        Draft::Pixelate { .. } => {
            if let Some((x, y, ref img)) = state.draft_pixelate_cache {
                image::imageops::replace(display, img, x as i64, y as i64);
            }
        }
        other => {
            if let Some(a) = other.clone().finalize() {
                render::rasterize_overlays(display, &[a]);
            }
        }
    }
}

/// Copy the selection region out of `base` and into `display`, row by row.
/// Clamps to image bounds so an off-screen drag doesn't underflow.
fn overlay_rect_from_base(display: &mut RgbaImage, base: &RgbaImage, sel: Bounds) {
    let w = display.width() as i32;
    let h = display.height() as i32;
    let l = (sel.x as i32).max(0);
    let t = (sel.y as i32).max(0);
    let r = ((sel.x + sel.w) as i32).min(w);
    let b = ((sel.y + sel.h) as i32).min(h);
    if r <= l || b <= t {
        return;
    }
    let stride = (w as usize) * 4;
    let row_bytes = ((r - l) as usize) * 4;
    let start_x = (l as usize) * 4;
    let src = base.as_raw();
    let dst = display.as_mut();
    for y in t..b {
        let off = (y as usize) * stride + start_x;
        dst[off..off + row_bytes].copy_from_slice(&src[off..off + row_bytes]);
    }
}

fn paint_selection_frame(display: &mut RgbaImage, sel: Bounds) {
    let w = display.width();
    let h = display.height();
    let buf = display.as_mut();
    let Some(mut pm) = PixmapMut::from_bytes(buf, w, h) else { return; };
    let yellow = Color::from_rgba8(255, 200, 0, 255);
    stroke_rect_px(&mut pm, sel.x, sel.y, sel.w, sel.h, yellow, 2.0);
}

fn paint_handles(display: &mut RgbaImage, sel: Bounds) {
    let w = display.width();
    let h = display.height();
    let buf = display.as_mut();
    let Some(mut pm) = PixmapMut::from_bytes(buf, w, h) else { return; };
    let fill = Color::WHITE;
    let stroke = Color::from_rgba8(255, 200, 0, 255);
    let s = HANDLE_VISUAL;
    for (_, hx, hy) in handle_corner_positions(sel) {
        let rx = hx - s * 0.5;
        let ry = hy - s * 0.5;
        fill_rect_px(&mut pm, rx, ry, s, s, fill);
        stroke_rect_px(&mut pm, rx, ry, s, s, stroke, 2.0);
    }
}

fn paint_hint_text(display: &mut RgbaImage) {
    let w = display.width() as f32;
    let h = display.height() as f32;
    let font = render::font();
    let title_scale = PxScale::from(34.0);
    let body_scale = PxScale::from(18.0);
    let title_h = 52.0_f32;
    let line_h = 26.0_f32;

    let body: &[&str] = &[
        "Drag to select a region",
        "Enter saves the full screen    Esc cancels",
        "",
        "After selecting a region:",
        "1 Pencil    2 Highlighter    3 Line    4 Arrow",
        "5 Rect    6 Ellipse    7 Pixelate    8 Counter",
        "Drag inside to move the frame    Drag a handle to resize",
        "Ctrl+Z undo    Ctrl+Y redo    Ctrl+C copy    Enter save",
    ];

    let block_h = title_h + line_h * body.len() as f32;
    let top = (h * 0.5 - block_h * 0.5) as i32;

    let title = "RUST SHOT";
    let title_color = Rgba([255u8, 255, 255, 255]);
    let body_color = Rgba([210u8, 210, 210, 255]);
    let (tw, _) = imageproc::drawing::text_size(title_scale, font, title);
    let tx = (w * 0.5 - tw as f32 * 0.5) as i32;
    imageproc::drawing::draw_text_mut(display, title_color, tx, top, title_scale, font, title);

    let body_top = top + title_h as i32;
    for (i, line) in body.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let (tw, _) = imageproc::drawing::text_size(body_scale, font, line);
        let tx = (w * 0.5 - tw as f32 * 0.5) as i32;
        let ty = body_top + (i as f32 * line_h) as i32;
        imageproc::drawing::draw_text_mut(display, body_color, tx, ty, body_scale, font, line);
    }
}

// --- tiny-skia helpers -------------------------------------------------------

fn fill_rect_px(pm: &mut PixmapMut, x: f32, y: f32, w: f32, h: f32, c: Color) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let Some(r) = tiny_skia::Rect::from_xywh(x, y, w, h) else { return; };
    let mut pb = PathBuilder::new();
    pb.push_rect(r);
    if let Some(path) = pb.finish() {
        let mut p = Paint::default();
        p.set_color(c);
        p.anti_alias = false;
        pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
    }
}

fn stroke_rect_px(pm: &mut PixmapMut, x: f32, y: f32, w: f32, h: f32, c: Color, width: f32) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let Some(r) = tiny_skia::Rect::from_xywh(x, y, w, h) else { return; };
    let mut pb = PathBuilder::new();
    pb.push_rect(r);
    if let Some(path) = pb.finish() {
        let mut p = Paint::default();
        p.set_color(c);
        p.anti_alias = true;
        let mut s = Stroke::default();
        s.width = width;
        pm.stroke_path(&path, &p, &s, Transform::identity(), None);
    }
}
