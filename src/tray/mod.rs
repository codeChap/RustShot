//! Status-tray icon. Tries StatusNotifierItem first (KDE / polybar-with-SNI
//! / `i3status-rust`). If no SNI watcher is on the session bus, falls back
//! to XEmbed — same pattern Qt uses, which is why Flameshot "just works" on
//! stock i3bar.
//!
//! The `spawn_capture` helper below is the single entrypoint both backends
//! use when the icon is clicked.

pub mod sni;
pub mod xembed;

use crate::capture::X11Capture;
use crate::config::{self, Config};
use crate::ui::UiRequest;
use crossbeam_channel::Sender;
use std::sync::Arc;

/// Fire a region capture on a new thread so the tray event loop (whichever
/// backend is active) stays responsive.
pub(super) fn spawn_capture(
    capture: Arc<X11Capture>,
    config: Arc<Config>,
    ui_tx: Sender<UiRequest>,
) {
    std::thread::Builder::new()
        .name("rustshot-tray-capture".into())
        .spawn(move || {
            if let Err(e) = do_capture(capture, config, ui_tx) {
                tracing::error!("tray capture: {e}");
            }
        })
        .ok();
}

fn do_capture(
    capture: Arc<X11Capture>,
    config: Arc<Config>,
    ui_tx: Sender<UiRequest>,
) -> anyhow::Result<()> {
    let include_cursor = config.capture.include_cursor;
    let screen = capture.cursor_screen()?;
    let image = capture.capture_screen_with_cursor(&screen, include_cursor)?;
    let save_path =
        config::auto_save_path(&config.defaults.save_dir, &config.defaults.filename_pattern)
            .to_string_lossy()
            .into_owned();
    // Fire-and-forget: the tray doesn't care about the overlay's UiResult.
    let (result_tx, _rx) = tokio::sync::oneshot::channel();
    ui_tx.send(UiRequest::ShowOverlay {
        image,
        screen_origin: (screen.x, screen.y),
        save_path,
        clipboard: true,
        config,
        result_tx,
    })?;
    Ok(())
}
