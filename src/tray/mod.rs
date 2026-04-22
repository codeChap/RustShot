//! Status-tray icon. Tries StatusNotifierItem first (KDE / polybar-with-SNI
//! / `i3status-rust`). If no SNI watcher is on the session bus, falls back
//! to XEmbed — same pattern Qt uses, which is why Flameshot "just works" on
//! stock i3bar.
//!
//! The `spawn_capture` helper below is the single entrypoint both backends
//! use when the icon is clicked. It funnels into `dbus::submit_overlay`, the
//! same function the `graphicCapture` DBus method uses — so a tray click and
//! a `dbus-send` invocation produce identical work and identical log lines.

pub mod sni;
pub mod xembed;

use crate::capture::X11Capture;
use crate::config::{self, Config};
use crate::ui::{BusyGuard, UiRequest};
use crossbeam_channel::Sender;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Fire a region capture on a new thread so the tray event loop (whichever
/// backend is active) stays responsive.
pub(super) fn spawn_capture(
    capture: Arc<X11Capture>,
    config: Arc<Config>,
    ui_tx: Sender<UiRequest>,
    gui_busy: Arc<AtomicBool>,
) {
    let Some(guard) = BusyGuard::acquire(&gui_busy) else {
        tracing::info!("capture ignored — overlay already active");
        return;
    };
    std::thread::Builder::new()
        .name("rustshot-tray-capture".into())
        .spawn(move || {
            let path = config::auto_save_path(
                &config.defaults.save_dir,
                &config.defaults.filename_pattern,
            )
            .to_string_lossy()
            .into_owned();
            // Fire-and-forget: drop the receiver, the tray doesn't care about
            // the overlay's UiResult.
            if let Err(e) =
                crate::dbus::submit_overlay(capture.as_ref(), config, &ui_tx, path, true, guard)
            {
                tracing::error!("tray capture: {e}");
            }
        })
        .ok();
}
