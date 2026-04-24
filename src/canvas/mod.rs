pub mod geometry;
pub mod render;

pub use geometry::{Bounds, Pos};
use image::Rgba;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolKind {
    Pencil,
    Arrow,
    Rect,
    Ellipse,
    Blur,
    Counter,
}

impl ToolKind {
    pub const ALL: [ToolKind; 6] = [
        ToolKind::Pencil,
        ToolKind::Arrow,
        ToolKind::Rect,
        ToolKind::Ellipse,
        ToolKind::Blur,
        ToolKind::Counter,
    ];
}

#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub color: Rgba<u8>,
    pub width: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            color: Rgba([255, 50, 50, 255]),
            width: 4.0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Annotation {
    Pencil {
        points: Vec<Pos>,
        color: Rgba<u8>,
        width: f32,
    },
    Arrow {
        start: Pos,
        end: Pos,
        color: Rgba<u8>,
        width: f32,
    },
    Rect {
        rect: Bounds,
        color: Rgba<u8>,
        width: f32,
    },
    Ellipse {
        rect: Bounds,
        color: Rgba<u8>,
        width: f32,
    },
    Blur {
        rect: Bounds,
        sigma: f32,
    },
    Counter {
        center: Pos,
        number: u32,
        color: Rgba<u8>,
        radius: f32,
    },
}

#[derive(Debug)]
pub struct Canvas {
    pub annotations: Vec<Annotation>,
    pub redo: Vec<Annotation>,
    pub style: Style,
    /// `None` means no drawing tool is armed — inside-drag of the selection
    /// rectangle moves it instead of starting an annotation.
    pub tool: Option<ToolKind>,
    counter: u32,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            annotations: Vec::new(),
            redo: Vec::new(),
            style: Style::default(),
            tool: None,
            counter: 0,
        }
    }
}

impl Canvas {
    pub fn push(&mut self, a: Annotation) {
        self.annotations.push(a);
        self.redo.clear();
    }

    pub fn undo(&mut self) {
        if let Some(a) = self.annotations.pop() {
            if matches!(a, Annotation::Counter { .. }) {
                self.counter = self.counter.saturating_sub(1);
            }
            self.redo.push(a);
        }
    }

    pub fn redo(&mut self) {
        if let Some(a) = self.redo.pop() {
            if matches!(a, Annotation::Counter { .. }) {
                self.counter += 1;
            }
            self.annotations.push(a);
        }
    }

    pub fn next_counter(&mut self) -> u32 {
        self.counter += 1;
        self.counter
    }
}
