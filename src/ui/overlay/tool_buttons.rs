//! Floating tool strip under the selection rect. Only rendered once a region
//! has been selected. Six tool buttons, a separator, then Save + Copy.

use crate::canvas::{render, Bounds, Pos, ToolKind};
use ab_glyph::PxScale;
use image::{Rgba, RgbaImage};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, PixmapMut, Stroke, Transform,
};

const BUTTON_D: f32 = 34.0;
const GAP: f32 = 6.0;
const GROUP_GAP: f32 = 14.0;
const PAD: f32 = 6.0;
const MARGIN: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hit {
    Tool(ToolKind),
    Save,
    Copy,
}

/// Compute the strip rect for the current selection. Prefer below the frame;
/// flip above if there's no room; last-resort fallback pins to the screen's
/// bottom edge.
pub(super) fn strip_rect(screen_w: u32, screen_h: u32, sel: Bounds) -> Bounds {
    let tool_n = ToolKind::ALL.len() as f32;
    let action_n = 2.0;
    let w = tool_n * BUTTON_D + (tool_n - 1.0) * GAP
        + GROUP_GAP
        + action_n * BUTTON_D + (action_n - 1.0) * GAP
        + PAD * 2.0;
    let h = BUTTON_D + PAD * 2.0;
    let sw = screen_w as f32;
    let sh = screen_h as f32;
    let sb = sel.y + sel.h;
    let y = if sb + MARGIN + h <= sh {
        sb + MARGIN
    } else if sel.y - MARGIN - h >= 0.0 {
        sel.y - MARGIN - h
    } else {
        (sh - h - 4.0).max(4.0)
    };
    let sc = sel.x + sel.w * 0.5;
    let x = (sc - w * 0.5).max(4.0).min(sw - w - 4.0);
    Bounds { x, y, w, h }
}

pub(super) fn contains(strip: Bounds, p: Pos) -> bool {
    p.x >= strip.x && p.x < strip.x + strip.w && p.y >= strip.y && p.y < strip.y + strip.h
}

pub(super) fn hit(strip: Bounds, p: Pos) -> Option<Hit> {
    let mut x = strip.x + PAD;
    let y = strip.y + PAD;
    for &tool in ToolKind::ALL.iter() {
        if in_square(x, y, BUTTON_D, p) {
            return Some(Hit::Tool(tool));
        }
        x += BUTTON_D + GAP;
    }
    x += GROUP_GAP;
    if in_square(x, y, BUTTON_D, p) {
        return Some(Hit::Save);
    }
    x += BUTTON_D + GAP;
    if in_square(x, y, BUTTON_D, p) {
        return Some(Hit::Copy);
    }
    None
}

fn in_square(x: f32, y: f32, d: f32, p: Pos) -> bool {
    p.x >= x && p.x < x + d && p.y >= y && p.y < y + d
}

pub(super) fn paint(
    display: &mut RgbaImage,
    strip: Bounds,
    active: Option<ToolKind>,
    hover: Option<Hit>,
) {
    let w = display.width();
    let h = display.height();

    // Vector pass — strip bg + all buttons + all non-text glyph strokes.
    {
        let buf = display.as_mut();
        let mut pm = match PixmapMut::from_bytes(buf, w, h) {
            Some(p) => p,
            None => return,
        };

        // Strip background
        fill_rect(&mut pm, strip, color(28, 28, 32, 230));
        stroke_rect(&mut pm, strip, color(90, 90, 108, 255), 1.0);

        let mut x = strip.x + PAD;
        let y = strip.y + PAD;

        for &tool in ToolKind::ALL.iter() {
            let is_active = active == Some(tool);
            let is_hover = matches!(hover, Some(Hit::Tool(h)) if h == tool);
            draw_button(&mut pm, x, y, BUTTON_D, is_active, is_hover, |pm, cx, cy, d, fg, bg| {
                paint_tool_glyph(pm, cx, cy, d, tool, fg, bg);
            });
            x += BUTTON_D + GAP;
        }

        let sep_x = x + (GROUP_GAP - GAP) * 0.5;
        stroke_line(
            &mut pm,
            sep_x,
            strip.y + 10.0,
            sep_x,
            strip.y + strip.h - 10.0,
            color(90, 90, 108, 255),
            1.0,
        );
        x += GROUP_GAP;

        let is_save_hover = matches!(hover, Some(Hit::Save));
        draw_button(&mut pm, x, y, BUTTON_D, false, is_save_hover, |pm, cx, cy, d, fg, bg| {
            paint_glyph_save(pm, cx, cy, d, fg, bg);
        });
        x += BUTTON_D + GAP;

        let is_copy_hover = matches!(hover, Some(Hit::Copy));
        draw_button(&mut pm, x, y, BUTTON_D, false, is_copy_hover, |pm, cx, cy, d, fg, bg| {
            paint_glyph_copy(pm, cx, cy, d, fg, bg);
        });
    }

    // Text pass — counter "1" glyph. Only the Counter button has text.
    paint_counter_label(display, strip, active, hover);
}

fn paint_counter_label(
    display: &mut RgbaImage,
    strip: Bounds,
    active: Option<ToolKind>,
    hover: Option<Hit>,
) {
    // Find the Counter button x position.
    let counter_idx = ToolKind::ALL
        .iter()
        .position(|&t| t == ToolKind::Counter)
        .unwrap_or(0) as f32;
    let bx = strip.x + PAD + counter_idx * (BUTTON_D + GAP);
    let by = strip.y + PAD;
    let is_active = active == Some(ToolKind::Counter);
    let fg = if is_active {
        Rgba([0, 0, 0, 255])
    } else {
        Rgba([255, 255, 255, 255])
    };
    let _ = hover; // hover doesn't change the glyph
    let scale = PxScale::from(BUTTON_D * 0.42);
    let (tw, th) = imageproc::drawing::text_size(scale, render::font(), "1");
    let tx = (bx + BUTTON_D * 0.5 - tw as f32 * 0.5) as i32;
    let ty = (by + BUTTON_D * 0.5 - th as f32 * 0.5) as i32;
    imageproc::drawing::draw_text_mut(display, fg, tx, ty, scale, render::font(), "1");
}

fn draw_button<F>(
    pm: &mut PixmapMut,
    x: f32,
    y: f32,
    d: f32,
    active: bool,
    hover: bool,
    paint_glyph: F,
) where
    F: FnOnce(&mut PixmapMut, f32, f32, f32, Color, Color),
{
    let cx = x + d * 0.5;
    let cy = y + d * 0.5;
    let r = d * 0.5;
    let (bg, ring, fg) = if active {
        (color(255, 200, 0, 255), color(255, 220, 60, 255), color(0, 0, 0, 255))
    } else if hover {
        (color(64, 64, 72, 255), color(200, 200, 220, 255), color(255, 255, 255, 255))
    } else {
        (color(48, 48, 56, 255), color(140, 140, 160, 255), color(255, 255, 255, 255))
    };
    fill_circle(pm, cx, cy, r, bg);
    stroke_circle(pm, cx, cy, r, ring, 1.5);
    paint_glyph(pm, cx, cy, d, fg, bg);
}

fn paint_tool_glyph(
    pm: &mut PixmapMut,
    cx: f32,
    cy: f32,
    d: f32,
    tool: ToolKind,
    fg: Color,
    _bg: Color,
) {
    let stroke_w = 2.0;
    match tool {
        ToolKind::Pencil => {
            let ax = cx - d * 0.24;
            let ay = cy + d * 0.24;
            let bx = cx + d * 0.24;
            let by = cy - d * 0.24;
            stroke_line(pm, ax, ay, bx, by, fg, stroke_w);
            fill_circle(pm, bx, by, 2.0, fg);
        }
        ToolKind::Arrow => {
            let ax = cx - d * 0.26;
            let ay = cy + d * 0.22;
            let bx = cx + d * 0.26;
            let by = cy - d * 0.22;
            stroke_line(pm, ax, ay, bx, by, fg, stroke_w);
            let h = d * 0.16;
            let dx = 1.0_f32;
            let dy = -1.0_f32;
            let len = (dx * dx + dy * dy).sqrt();
            let ux = dx / len;
            let uy = dy / len;
            let perp_x = -uy;
            let perp_y = ux;
            let e1x = bx - h * ux + perp_x * h * 0.6;
            let e1y = by - h * uy + perp_y * h * 0.6;
            let e2x = bx - h * ux - perp_x * h * 0.6;
            let e2y = by - h * uy - perp_y * h * 0.6;
            stroke_line(pm, bx, by, e1x, e1y, fg, stroke_w);
            stroke_line(pm, bx, by, e2x, e2y, fg, stroke_w);
        }
        ToolKind::Rect => {
            let w = d * 0.54;
            let h = d * 0.40;
            stroke_rect(
                pm,
                Bounds { x: cx - w * 0.5, y: cy - h * 0.5, w, h },
                fg,
                stroke_w,
            );
        }
        ToolKind::Ellipse => {
            stroke_ellipse(pm, cx, cy, d * 0.28, d * 0.20, fg, stroke_w);
        }
        ToolKind::Blur => {
            stroke_circle(pm, cx, cy, d * 0.10, fg, stroke_w);
            stroke_circle(pm, cx, cy, d * 0.20, fg, stroke_w);
            stroke_circle(pm, cx, cy, d * 0.30, fg, stroke_w);
        }
        ToolKind::Counter => {
            // Circle outline only — the "1" digit is drawn by paint_counter_label
            // in the separate text pass.
            stroke_circle(pm, cx, cy, d * 0.28, fg, stroke_w);
        }
    }
}

fn paint_glyph_save(pm: &mut PixmapMut, cx: f32, cy: f32, d: f32, fg: Color, _bg: Color) {
    let w = d * 0.52;
    let h = d * 0.52;
    stroke_rect(
        pm,
        Bounds { x: cx - w * 0.5, y: cy - h * 0.5, w, h },
        fg,
        2.0,
    );
    let lw = w * 0.66;
    let lh = h * 0.44;
    fill_rect(
        pm,
        Bounds {
            x: cx - lw * 0.5,
            y: cy + h * 0.18 - lh * 0.5,
            w: lw,
            h: lh,
        },
        fg,
    );
}

fn paint_glyph_copy(pm: &mut PixmapMut, cx: f32, cy: f32, d: f32, fg: Color, bg: Color) {
    let side = d * 0.34;
    let off = d * 0.08;
    // Composite bbox sits lower-right of `c`; nudge the whole glyph up
    // so it reads as vertically centered inside the circular button.
    let shift_x = -3.0;
    let shift_y = -3.0;
    stroke_rect(
        pm,
        Bounds {
            x: cx - off * 2.0 + shift_x,
            y: cy - off * 2.0 + shift_y,
            w: side,
            h: side,
        },
        fg,
        1.5,
    );
    // Front square: fill with button bg to cover the back-square stroke,
    // then stroke the front outline.
    fill_rect(
        pm,
        Bounds {
            x: cx + shift_x,
            y: cy + shift_y,
            w: side,
            h: side,
        },
        bg,
    );
    stroke_rect(
        pm,
        Bounds {
            x: cx + shift_x,
            y: cy + shift_y,
            w: side,
            h: side,
        },
        fg,
        1.5,
    );
}

// --- tiny-skia helpers -------------------------------------------------------

fn color(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

fn paint_of(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

fn stroke_of(width: f32) -> Stroke {
    let mut s = Stroke::default();
    s.width = width.max(0.5);
    s.line_cap = LineCap::Round;
    s.line_join = LineJoin::Round;
    s
}

pub(super) fn fill_rect(pm: &mut PixmapMut, b: Bounds, c: Color) {
    if b.w <= 0.0 || b.h <= 0.0 {
        return;
    }
    let Some(r) = tiny_skia::Rect::from_xywh(b.x, b.y, b.w, b.h) else { return; };
    let mut pb = PathBuilder::new();
    pb.push_rect(r);
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, &paint_of(c), FillRule::Winding, Transform::identity(), None);
    }
}

pub(super) fn stroke_rect(pm: &mut PixmapMut, b: Bounds, c: Color, width: f32) {
    if b.w <= 0.0 || b.h <= 0.0 {
        return;
    }
    let Some(r) = tiny_skia::Rect::from_xywh(b.x, b.y, b.w, b.h) else { return; };
    let mut pb = PathBuilder::new();
    pb.push_rect(r);
    if let Some(path) = pb.finish() {
        pm.stroke_path(&path, &paint_of(c), &stroke_of(width), Transform::identity(), None);
    }
}

pub(super) fn stroke_line(pm: &mut PixmapMut, x0: f32, y0: f32, x1: f32, y1: f32, c: Color, w: f32) {
    let mut pb = PathBuilder::new();
    pb.move_to(x0, y0);
    pb.line_to(x1, y1);
    if let Some(path) = pb.finish() {
        pm.stroke_path(&path, &paint_of(c), &stroke_of(w), Transform::identity(), None);
    }
}

pub(super) fn fill_circle(pm: &mut PixmapMut, cx: f32, cy: f32, r: f32, c: Color) {
    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, r);
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, &paint_of(c), FillRule::Winding, Transform::identity(), None);
    }
}

pub(super) fn stroke_circle(pm: &mut PixmapMut, cx: f32, cy: f32, r: f32, c: Color, w: f32) {
    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, r);
    if let Some(path) = pb.finish() {
        pm.stroke_path(&path, &paint_of(c), &stroke_of(w), Transform::identity(), None);
    }
}

pub(super) fn stroke_ellipse(
    pm: &mut PixmapMut,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    c: Color,
    w: f32,
) {
    if rx <= 0.0 || ry <= 0.0 {
        return;
    }
    let Some(r) = tiny_skia::Rect::from_xywh(cx - rx, cy - ry, rx * 2.0, ry * 2.0) else { return; };
    let mut pb = PathBuilder::new();
    pb.push_oval(r);
    if let Some(path) = pb.finish() {
        pm.stroke_path(&path, &paint_of(c), &stroke_of(w), Transform::identity(), None);
    }
}
