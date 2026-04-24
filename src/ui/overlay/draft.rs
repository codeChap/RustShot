//! In-progress annotations. A Draft is alive during a drag and becomes an
//! `Annotation` on release (via `finalize`).

use crate::canvas::{Annotation, Bounds, Pos, Style, ToolKind};
use image::Rgba;

/// Highlighter uses a fixed semi-transparent yellow + wide stroke, like a
/// physical marker — ignores `canvas.style`.
const HIGHLIGHTER_STYLE: Style = Style {
    color: Rgba([255, 230, 0, 110]),
    width: 20.0,
};

#[derive(Debug, Clone)]
pub(super) enum Draft {
    Pencil { points: Vec<Pos>, style: Style },
    Line { start: Pos, end: Pos, style: Style },
    Arrow { start: Pos, end: Pos, style: Style },
    Rect { start: Pos, end: Pos, style: Style },
    Ellipse { start: Pos, end: Pos, style: Style },
    Pixelate { start: Pos, end: Pos, block: u32 },
}

impl Draft {
    /// Build a draft for `tool` starting at `pos`. Returns `None` for tools that
    /// don't use drag (e.g. Counter fires on click, not drag).
    pub(super) fn new(tool: ToolKind, pos: Pos, style: Style, pixelate_block: u32) -> Option<Self> {
        Some(match tool {
            ToolKind::Pencil => Draft::Pencil { points: vec![pos], style },
            ToolKind::Highlighter => Draft::Pencil { points: vec![pos], style: HIGHLIGHTER_STYLE },
            ToolKind::Line => Draft::Line { start: pos, end: pos, style },
            ToolKind::Arrow => Draft::Arrow { start: pos, end: pos, style },
            ToolKind::Rect => Draft::Rect { start: pos, end: pos, style },
            ToolKind::Ellipse => Draft::Ellipse { start: pos, end: pos, style },
            ToolKind::Pixelate => Draft::Pixelate { start: pos, end: pos, block: pixelate_block },
            ToolKind::Counter => return None,
        })
    }

    /// Called every frame during a drag to update the in-progress shape.
    pub(super) fn extend(&mut self, p: Pos) {
        match self {
            Draft::Pencil { points, .. } => points.push(p),
            Draft::Line { end, .. }
            | Draft::Arrow { end, .. }
            | Draft::Rect { end, .. }
            | Draft::Ellipse { end, .. }
            | Draft::Pixelate { end, .. } => *end = p,
        }
    }

    /// Convert a completed draft into a committed `Annotation`. Returns `None`
    /// for zero-area drags (so a click-without-drag doesn't create a garbage shape).
    pub(super) fn finalize(self) -> Option<Annotation> {
        match self {
            Draft::Pencil { points, style } if points.len() >= 2 => Some(Annotation::Pencil {
                points,
                color: style.color,
                width: style.width,
            }),
            Draft::Pencil { .. } => None,
            Draft::Line { start, end, style } => (dist2(start, end) >= 4.0).then_some(
                Annotation::Line {
                    start,
                    end,
                    color: style.color,
                    width: style.width,
                },
            ),
            Draft::Arrow { start, end, style } => (dist2(start, end) >= 4.0).then_some(
                Annotation::Arrow {
                    start,
                    end,
                    color: style.color,
                    width: style.width,
                },
            ),
            Draft::Rect { start, end, style } => drawable(start, end).map(|rect| {
                Annotation::Rect {
                    rect,
                    color: style.color,
                    width: style.width,
                }
            }),
            Draft::Ellipse { start, end, style } => drawable(start, end).map(|rect| {
                Annotation::Ellipse {
                    rect,
                    color: style.color,
                    width: style.width,
                }
            }),
            Draft::Pixelate { start, end, block } => drawable(start, end).map(|rect| {
                Annotation::Pixelate { rect, block }
            }),
        }
    }
}

/// Returns a `Bounds` only if the rect has enough area to be visible.
fn drawable(a: Pos, b: Pos) -> Option<Bounds> {
    let r = Bounds::from_two(a, b);
    (r.w >= 2.0 && r.h >= 2.0).then_some(r)
}

fn dist2(a: Pos, b: Pos) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}
