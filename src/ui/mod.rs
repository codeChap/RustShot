pub mod overlay;
pub mod toolbar;

use crate::config::Config;
use image::RgbaImage;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub enum UiRequest {
    ShowOverlay {
        image: RgbaImage,
        screen_origin: (i32, i32),
        save_path: String,
        clipboard: bool,
        config: Arc<Config>,
        result_tx: tokio::sync::oneshot::Sender<UiResult>,
        // Held for the lifetime of the overlay; drops release the gui-busy flag
        // so the next PrtSc can start. Dropped when the match arm exits below.
        _busy_guard: Option<BusyGuard>,
    },
}

#[derive(Debug, Clone)]
pub enum UiResult {
    Done,
    Cancelled,
}

/// RAII guard on the "an overlay is active" flag. One-at-a-time gate so
/// repeated PrtSc presses don't queue overlays behind the current one.
pub struct BusyGuard(Arc<AtomicBool>);

impl BusyGuard {
    /// Returns `Some(guard)` if the flag was previously clear (we own it now),
    /// `None` if another overlay is already active.
    pub fn acquire(flag: &Arc<AtomicBool>) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| BusyGuard(flag.clone()))
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub fn run_event_loop(rx: crossbeam_channel::Receiver<UiRequest>) -> anyhow::Result<()> {
    while let Ok(req) = rx.recv() {
        match req {
            UiRequest::ShowOverlay {
                image,
                screen_origin,
                save_path,
                clipboard,
                config,
                result_tx,
                _busy_guard,
            } => {
                overlay::show(image, screen_origin, save_path, clipboard, config, result_tx);
            }
        }
    }
    Ok(())
}
