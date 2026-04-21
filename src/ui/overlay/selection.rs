//! Selection rectangle editing — handle hit-testing, resize math, and the
//! little yellow-outlined squares drawn at corners + edge midpoints.

use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Handle { N, S, E, W, NE, NW, SE, SW }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SelectionEdit {
    None,
    Moving,
    Resizing(Handle),
}

const CORNER_HIT: f32 = 14.0;  // half-size, so 28×28 px square
const EDGE_HIT: f32 = 8.0;     // px distance from edge line
const HANDLE_VISUAL: f32 = 10.0;

fn handle_positions(rect: egui::Rect) -> [(Handle, egui::Pos2); 8] {
    let cx = rect.center().x;
    let cy = rect.center().y;
    let l = rect.left();
    let r = rect.right();
    let t = rect.top();
    let b = rect.bottom();
    [
        (Handle::NW, egui::pos2(l, t)),
        (Handle::N,  egui::pos2(cx, t)),
        (Handle::NE, egui::pos2(r, t)),
        (Handle::E,  egui::pos2(r, cy)),
        (Handle::SE, egui::pos2(r, b)),
        (Handle::S,  egui::pos2(cx, b)),
        (Handle::SW, egui::pos2(l, b)),
        (Handle::W,  egui::pos2(l, cy)),
    ]
}

/// Returns which handle (if any) the pointer is on. Corners take priority over
/// edges, and clicking anywhere along an edge line counts as a grab — not just
/// the midpoint dot.
pub(super) fn handle_at(rect: egui::Rect, pos: egui::Pos2) -> Option<Handle> {
    let corners = [
        (Handle::NW, egui::pos2(rect.left(), rect.top())),
        (Handle::NE, egui::pos2(rect.right(), rect.top())),
        (Handle::SW, egui::pos2(rect.left(), rect.bottom())),
        (Handle::SE, egui::pos2(rect.right(), rect.bottom())),
    ];
    for (h, p) in corners {
        if egui::Rect::from_center_size(p, egui::vec2(CORNER_HIT * 2.0, CORNER_HIT * 2.0))
            .contains(pos)
        {
            return Some(h);
        }
    }

    let in_x = pos.x >= rect.left() - EDGE_HIT && pos.x <= rect.right() + EDGE_HIT;
    let in_y = pos.y >= rect.top() - EDGE_HIT && pos.y <= rect.bottom() + EDGE_HIT;
    if (pos.y - rect.top()).abs() <= EDGE_HIT && in_x {
        return Some(Handle::N);
    }
    if (pos.y - rect.bottom()).abs() <= EDGE_HIT && in_x {
        return Some(Handle::S);
    }
    if (pos.x - rect.left()).abs() <= EDGE_HIT && in_y {
        return Some(Handle::W);
    }
    if (pos.x - rect.right()).abs() <= EDGE_HIT && in_y {
        return Some(Handle::E);
    }
    None
}

/// Apply a drag delta to the rect for the given handle. Normalizes min/max
/// so dragging past the opposite edge flips cleanly.
pub(super) fn resize_rect(rect: egui::Rect, handle: Handle, delta: egui::Vec2) -> egui::Rect {
    let mut min = rect.min;
    let mut max = rect.max;
    match handle {
        Handle::N  => min.y += delta.y,
        Handle::S  => max.y += delta.y,
        Handle::E  => max.x += delta.x,
        Handle::W  => min.x += delta.x,
        Handle::NE => { min.y += delta.y; max.x += delta.x; }
        Handle::NW => { min.y += delta.y; min.x += delta.x; }
        Handle::SE => { max.x += delta.x; max.y += delta.y; }
        Handle::SW => { min.x += delta.x; max.y += delta.y; }
    }
    if min.x > max.x { std::mem::swap(&mut min.x, &mut max.x); }
    if min.y > max.y { std::mem::swap(&mut min.y, &mut max.y); }
    egui::Rect::from_min_max(min, max)
}

pub(super) fn cursor_for_handle(h: Handle) -> egui::CursorIcon {
    match h {
        Handle::N  => egui::CursorIcon::ResizeNorth,
        Handle::S  => egui::CursorIcon::ResizeSouth,
        Handle::E  => egui::CursorIcon::ResizeEast,
        Handle::W  => egui::CursorIcon::ResizeWest,
        Handle::NE => egui::CursorIcon::ResizeNorthEast,
        Handle::NW => egui::CursorIcon::ResizeNorthWest,
        Handle::SE => egui::CursorIcon::ResizeSouthEast,
        Handle::SW => egui::CursorIcon::ResizeSouthWest,
    }
}

pub(super) fn draw_handles(painter: &egui::Painter, sel: egui::Rect) {
    let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 200, 0));
    for (_, p) in handle_positions(sel) {
        let r = egui::Rect::from_center_size(p, egui::vec2(HANDLE_VISUAL, HANDLE_VISUAL));
        painter.rect_filled(r, 2.0, egui::Color32::WHITE);
        painter.rect_stroke(r, 2.0, stroke);
    }
}
