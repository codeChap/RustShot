//! In-progress annotations. A Draft is alive during a drag and becomes an
//! `Annotation` on release (via `finalize`).

use crate::canvas::{Annotation, Bounds, Pos, Style, ToolKind};

#[derive(Debug, Clone)]
pub(super) enum Draft {
    Pencil { points: Vec<Pos>, style: Style },
    Arrow { start: Pos, end: Pos, style: Style },
    Rect { start: Pos, end: Pos, style: Style },
    Ellipse { start: Pos, end: Pos, style: Style },
    Blur { start: Pos, end: Pos, sigma: f32 },
}

impl Draft {
    /// Build a draft for `tool` starting at `pos`. Returns `None` for tools that
    /// don't use drag (e.g. Counter fires on click, not drag).
    pub(super) fn new(tool: ToolKind, pos: Pos, style: Style, blur_sigma: f32) -> Option<Self> {
        Some(match tool {
            ToolKind::Pencil => Draft::Pencil { points: vec![pos], style },
            ToolKind::Arrow => Draft::Arrow { start: pos, end: pos, style },
            ToolKind::Rect => Draft::Rect { start: pos, end: pos, style },
            ToolKind::Ellipse => Draft::Ellipse { start: pos, end: pos, style },
            ToolKind::Blur => Draft::Blur { start: pos, end: pos, sigma: blur_sigma },
            ToolKind::Counter => return None,
        })
    }

    /// Called every frame during a drag to update the in-progress shape.
    pub(super) fn extend(&mut self, p: Pos) {
        match self {
            Draft::Pencil { points, .. } => points.push(p),
            Draft::Arrow { end, .. }
            | Draft::Rect { end, .. }
            | Draft::Ellipse { end, .. }
            | Draft::Blur { end, .. } => *end = p,
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
            Draft::Blur { start, end, sigma } => drawable(start, end).map(|rect| {
                Annotation::Blur { rect, sigma }
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
