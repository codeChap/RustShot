//! Selection rectangle editing — handle hit-testing, resize math, and the
//! little yellow-outlined squares drawn at corners + edge midpoints.

use crate::canvas::{Bounds, Pos};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Handle { N, S, E, W, NE, NW, SE, SW }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SelectionEdit {
    None,
    Moving,
    Resizing(Handle),
}

pub(super) const CORNER_HIT: f32 = 14.0;
pub(super) const EDGE_HIT: f32 = 8.0;
pub(super) const HANDLE_VISUAL: f32 = 10.0;

/// Returns which handle (if any) the pointer is on. Corners take priority over
/// edges, and clicking anywhere along an edge line counts as a grab — not just
/// the midpoint dot.
pub(super) fn handle_at(rect: Bounds, p: Pos) -> Option<Handle> {
    let l = rect.x;
    let r = rect.x + rect.w;
    let t = rect.y;
    let b = rect.y + rect.h;

    let in_corner = |cx: f32, cy: f32| {
        (p.x - cx).abs() <= CORNER_HIT && (p.y - cy).abs() <= CORNER_HIT
    };
    if in_corner(l, t) { return Some(Handle::NW); }
    if in_corner(r, t) { return Some(Handle::NE); }
    if in_corner(l, b) { return Some(Handle::SW); }
    if in_corner(r, b) { return Some(Handle::SE); }

    let in_x = p.x >= l - EDGE_HIT && p.x <= r + EDGE_HIT;
    let in_y = p.y >= t - EDGE_HIT && p.y <= b + EDGE_HIT;
    if (p.y - t).abs() <= EDGE_HIT && in_x { return Some(Handle::N); }
    if (p.y - b).abs() <= EDGE_HIT && in_x { return Some(Handle::S); }
    if (p.x - l).abs() <= EDGE_HIT && in_y { return Some(Handle::W); }
    if (p.x - r).abs() <= EDGE_HIT && in_y { return Some(Handle::E); }
    None
}

/// Apply a drag delta to the rect for the given handle. Normalizes min/max
/// so dragging past the opposite edge flips cleanly.
pub(super) fn resize_rect(rect: Bounds, handle: Handle, dx: f32, dy: f32) -> Bounds {
    let mut l = rect.x;
    let mut t = rect.y;
    let mut r = rect.x + rect.w;
    let mut b = rect.y + rect.h;
    match handle {
        Handle::N  => t += dy,
        Handle::S  => b += dy,
        Handle::E  => r += dx,
        Handle::W  => l += dx,
        Handle::NE => { t += dy; r += dx; }
        Handle::NW => { t += dy; l += dx; }
        Handle::SE => { r += dx; b += dy; }
        Handle::SW => { l += dx; b += dy; }
    }
    if l > r { std::mem::swap(&mut l, &mut r); }
    if t > b { std::mem::swap(&mut t, &mut b); }
    Bounds { x: l, y: t, w: r - l, h: b - t }
}

pub(super) fn handle_corner_positions(rect: Bounds) -> [(Handle, f32, f32); 8] {
    let cx = rect.x + rect.w * 0.5;
    let cy = rect.y + rect.h * 0.5;
    let l = rect.x;
    let r = rect.x + rect.w;
    let t = rect.y;
    let b = rect.y + rect.h;
    [
        (Handle::NW, l, t),
        (Handle::N,  cx, t),
        (Handle::NE, r, t),
        (Handle::E,  r, cy),
        (Handle::SE, r, b),
        (Handle::S,  cx, b),
        (Handle::SW, l, b),
        (Handle::W,  l, cy),
    ]
}

/// Stock X11 cursor-font glyph ID for each resize direction.
/// See `/usr/include/X11/cursorfont.h`.
pub(super) fn cursor_glyph_for_handle(h: Handle) -> u16 {
    match h {
        Handle::N  => 138, // XC_top_side
        Handle::S  => 16,  // XC_bottom_side
        Handle::E  => 96,  // XC_right_side
        Handle::W  => 70,  // XC_left_side
        Handle::NE => 136, // XC_top_right_corner
        Handle::NW => 134, // XC_top_left_corner
        Handle::SE => 14,  // XC_bottom_right_corner
        Handle::SW => 12,  // XC_bottom_left_corner
    }
}
