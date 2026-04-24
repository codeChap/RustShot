//! Overlay state — the data layer: image, selection, annotations, draft,
//! cached derived data. No X11, no painting.

use super::draft::Draft;
use super::selection::SelectionEdit;
use super::tool_buttons::Hit;
use crate::canvas::{render, Annotation, Bounds, Canvas, Pos};
use crate::config::Config;
use crate::export;
use crate::ui::UiResult;
use image::RgbaImage;
use std::sync::Arc;

/// Fingerprint of one committed Pixelate — used to detect changes.
pub(super) type PixelateKey = (u32, u32, u32, u32, u32);

pub(super) fn pixelate_key(b: Bounds, block: u32) -> PixelateKey {
    (
        b.x.to_bits(),
        b.y.to_bits(),
        b.w.to_bits(),
        b.h.to_bits(),
        block,
    )
}

#[derive(Default, PartialEq, Eq)]
pub(super) enum Mode {
    #[default]
    SelectingRegion,
    Annotating,
}

pub(super) struct OverlayState {
    /// Original captured image — never mutated.
    pub original: RgbaImage,
    /// Original + all committed Pixelates baked in. Reset into the display
    /// buffer at the start of every composite.
    pub base: RgbaImage,
    /// `base` with every RGB channel halved. Used as the whole-screen dim
    /// layer so we can memcpy once per frame instead of running tiny-skia
    /// over 3M+ pixels to fill dim rects. Rebuilt only when `base` changes.
    pub dim_base: RgbaImage,
    pub save_path: String,
    pub clipboard_pref: bool,
    pub counter_radius: f32,
    pub pixelate_block: u32,
    pub canvas: Canvas,
    pub mode: Mode,
    pub selection: Option<Bounds>,
    pub sel_drag_start: Option<Pos>,
    pub selection_edit: SelectionEdit,
    pub edit_drag_start: Option<Pos>,
    pub edit_rect_start: Option<Bounds>,
    pub draft: Option<Draft>,
    /// Live pixelate preview: (origin_x, origin_y, pixelated). Cached by
    /// (bounds, block) so we only re-run resize when the draft actually changes.
    pub draft_pixelate_cache: Option<(u32, u32, RgbaImage)>,
    pub draft_pixelate_sig: Option<PixelateKey>,
    /// Signature of committed pixelates baked into `base`. Drives the rebuild.
    pub committed_pixelate_sig: Vec<PixelateKey>,
    pub strip_hover: Option<Hit>,
    pub ctrl_down: bool,
}

impl OverlayState {
    pub fn new(image: RgbaImage, save_path: String, clipboard: bool, config: Arc<Config>) -> Self {
        let base = image.clone();
        let dim_base = build_dim(&base);
        Self {
            original: image,
            base,
            dim_base,
            save_path,
            clipboard_pref: clipboard,
            counter_radius: config.defaults.counter_radius.max(4.0),
            pixelate_block: config.defaults.pixelate_block.max(2),
            canvas: Canvas::default(),
            mode: Mode::default(),
            selection: None,
            sel_drag_start: None,
            selection_edit: SelectionEdit::None,
            edit_drag_start: None,
            edit_rect_start: None,
            draft: None,
            draft_pixelate_cache: None,
            draft_pixelate_sig: None,
            committed_pixelate_sig: Vec::new(),
            strip_hover: None,
            ctrl_down: false,
        }
    }

    /// Keep `base` in sync with committed Pixelate annotations. Fast-path: if
    /// the new signature is the old one plus one appended pixelate, only
    /// pixelate the new region into the existing `base`. Everything else does
    /// a full rebuild from `original`.
    pub fn refresh_base(&mut self) {
        let current: Vec<PixelateKey> = self
            .canvas
            .annotations
            .iter()
            .filter_map(|a| match a {
                Annotation::Pixelate { rect, block } => Some(pixelate_key(*rect, *block)),
                _ => None,
            })
            .collect();

        if self.committed_pixelate_sig == current {
            return;
        }

        let can_append = current.len() == self.committed_pixelate_sig.len() + 1
            && current.starts_with(&self.committed_pixelate_sig);

        if can_append {
            if let Some((rect, block)) = self
                .canvas
                .annotations
                .iter()
                .rev()
                .find_map(|a| match a {
                    Annotation::Pixelate { rect, block } => Some((*rect, *block)),
                    _ => None,
                })
            {
                if let Some((x, y, px)) = render::pixelate_crop(&self.base, rect, block) {
                    image::imageops::replace(&mut self.base, &px, x as i64, y as i64);
                }
                self.committed_pixelate_sig = current;
                self.dim_base = build_dim(&self.base);
                return;
            }
        }

        // Full rebuild from original.
        let mut base = self.original.clone();
        for a in &self.canvas.annotations {
            if let Annotation::Pixelate { rect, block } = a {
                if let Some((x, y, px)) = render::pixelate_crop(&base, *rect, *block) {
                    image::imageops::replace(&mut base, &px, x as i64, y as i64);
                }
            }
        }
        self.base = base;
        self.committed_pixelate_sig = current;
        self.dim_base = build_dim(&self.base);
    }

    /// Maintain the small pixelated preview for an in-progress Pixelate draft.
    pub fn refresh_draft_pixelate(&mut self) {
        let Some(Draft::Pixelate { start, end, block }) = self.draft.as_ref() else {
            self.draft_pixelate_cache = None;
            self.draft_pixelate_sig = None;
            return;
        };
        let x0 = start.x.min(end.x);
        let y0 = start.y.min(end.y);
        let x1 = start.x.max(end.x);
        let y1 = start.y.max(end.y);
        if x1 - x0 < 2.0 || y1 - y0 < 2.0 {
            self.draft_pixelate_cache = None;
            self.draft_pixelate_sig = None;
            return;
        }
        let bounds = Bounds { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
        let key = pixelate_key(bounds, *block);
        if self.draft_pixelate_sig == Some(key) && self.draft_pixelate_cache.is_some() {
            return;
        }
        match render::pixelate_crop(&self.base, bounds, *block) {
            Some((x, y, px)) => {
                self.draft_pixelate_cache = Some((x, y, px));
                self.draft_pixelate_sig = Some(key);
            }
            None => {
                self.draft_pixelate_cache = None;
                self.draft_pixelate_sig = None;
            }
        }
    }

    /// Bake everything and export. Same logic as the eframe OverlayApp::act.
    pub fn act(&mut self, force_copy: bool) -> UiResult {
        let img = self.compose();
        if !self.save_path.is_empty() {
            match export::file::save_png(&img, std::path::Path::new(&self.save_path)) {
                Ok(()) => tracing::info!(
                    path = %self.save_path, w = img.width(), h = img.height(), "saved"
                ),
                Err(e) => tracing::error!("save failed: {e}"),
            }
        }
        if self.clipboard_pref || force_copy {
            if let Err(e) = export::clipboard::copy(&img) {
                tracing::error!("clipboard copy failed: {e}");
            } else {
                tracing::info!(w = img.width(), h = img.height(), "copied to clipboard");
            }
        }
        UiResult::Done
    }

    fn compose(&self) -> RgbaImage {
        let mut working = self.base.clone();
        render::rasterize_overlays(&mut working, &self.canvas.annotations);
        if let Some(sel) = self.selection {
            let (x, y, w, h) = clamp_to_image(sel, self.base.width(), self.base.height());
            if w > 0 && h > 0 {
                return image::imageops::crop_imm(&working, x, y, w, h).to_image();
            }
        }
        working
    }
}

/// Build the dim-overlay layer: `base` with RGB halved (alpha forced opaque).
/// In release with LLVM this vectorizes cleanly.
fn build_dim(base: &RgbaImage) -> RgbaImage {
    let mut dim = base.clone();
    for p in dim.as_mut().chunks_exact_mut(4) {
        p[0] >>= 1;
        p[1] >>= 1;
        p[2] >>= 1;
        p[3] = 0xff;
    }
    dim
}

fn clamp_to_image(sel: Bounds, max_w: u32, max_h: u32) -> (u32, u32, u32, u32) {
    let l = sel.x.max(0.0).min(max_w as f32) as u32;
    let t = sel.y.max(0.0).min(max_h as f32) as u32;
    let r = (sel.x + sel.w).max(0.0).min(max_w as f32) as u32;
    let b = (sel.y + sel.h).max(0.0).min(max_h as f32) as u32;
    (l, t, r.saturating_sub(l), b.saturating_sub(t))
}
