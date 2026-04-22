use crate::capture::X11Capture;
use crate::config::{self, Config};
use crate::export;
use crate::ui::{BusyGuard, UiRequest, UiResult};
use crossbeam_channel::Sender;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use zbus::interface;

pub const SERVICE_NAME: &str = "org.rustshot.RustShot";
pub const OBJECT_PATH: &str = "/";

pub struct Service {
    pub capture: Arc<X11Capture>,
    pub config: Arc<Config>,
    pub ui_tx: Sender<UiRequest>,
    pub gui_busy: Arc<AtomicBool>,
}

#[interface(name = "org.rustshot.RustShot")]
impl Service {
    /// Flameshot-CLI-compat: interactive region capture.
    /// Equivalent to `graphicCaptureFlags` with clipboard=false, no_save=false.
    #[zbus(name = "graphicCapture")]
    async fn graphic_capture(&self, path: String, delay: u32, id: String) -> zbus::fdo::Result<()> {
        self.graphic_capture_flags(path, delay, false, false, id).await
    }

    /// Extended: graphicCapture + clipboard + no_save flags.
    #[zbus(name = "graphicCaptureFlags")]
    async fn graphic_capture_flags(
        &self,
        path: String,
        delay: u32,
        clipboard: bool,
        no_save: bool,
        _id: String,
    ) -> zbus::fdo::Result<()> {
        // One overlay at a time. If a repeated PrtSc arrives while an overlay
        // is already up, drop it silently instead of letting captures queue.
        let Some(guard) = BusyGuard::acquire(&self.gui_busy) else {
            tracing::info!("capture ignored — overlay already active");
            return Ok(());
        };
        let resolved = resolve_save_path(path, no_save, &self.config);
        gui_capture(
            self.capture.clone(),
            self.config.clone(),
            self.ui_tx.clone(),
            resolved,
            clipboard,
            delay,
            guard,
        )
        .await
    }

    #[zbus(name = "fullScreen")]
    async fn full_screen(&self, path: String, delay: u32, id: String) -> zbus::fdo::Result<()> {
        self.full_screen_flags(path, delay, false, false, id).await
    }

    #[zbus(name = "fullScreenFlags")]
    async fn full_screen_flags(
        &self,
        path: String,
        delay: u32,
        clipboard: bool,
        no_save: bool,
        _id: String,
    ) -> zbus::fdo::Result<()> {
        let resolved = resolve_save_path(path, no_save, &self.config);
        do_capture(
            self.capture.clone(),
            self.config.clone(),
            CaptureKind::All,
            resolved,
            delay,
            clipboard,
        )
        .await
    }

    #[zbus(name = "captureScreen")]
    async fn capture_screen(
        &self,
        screen_number: i32,
        path: String,
        delay: u32,
        id: String,
    ) -> zbus::fdo::Result<()> {
        self.capture_screen_flags(screen_number, path, delay, false, false, id).await
    }

    #[zbus(name = "captureScreenFlags")]
    async fn capture_screen_flags(
        &self,
        screen_number: i32,
        path: String,
        delay: u32,
        clipboard: bool,
        no_save: bool,
        _id: String,
    ) -> zbus::fdo::Result<()> {
        let resolved = resolve_save_path(path, no_save, &self.config);
        do_capture(
            self.capture.clone(),
            self.config.clone(),
            kind_for_screen(screen_number),
            resolved,
            delay,
            clipboard,
        )
        .await
    }
}

async fn sleep_delay(delay_ms: u32) {
    if delay_ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms as u64)).await;
    }
}

/// Run blocking work on the tokio blocking pool, mapping both the join error
/// and the inner `anyhow::Result` into a `zbus::fdo::Result`.
async fn run_blocking<F, T>(f: F) -> zbus::fdo::Result<T>
where
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| zbus::fdo::Error::Failed(format!("join: {e}")))?
        .map_err(|e| zbus::fdo::Error::Failed(format!("{e}")))
}

fn kind_for_screen(n: i32) -> CaptureKind {
    if n < 0 {
        CaptureKind::CursorScreen
    } else {
        CaptureKind::Screen(n as usize)
    }
}

/// Empty path + !no_save → generate from config. no_save → empty (skip save). Else → as-is.
fn resolve_save_path(path: String, no_save: bool, config: &Config) -> String {
    if no_save {
        return String::new();
    }
    if !path.is_empty() {
        return path;
    }
    config::auto_save_path(&config.defaults.save_dir, &config.defaults.filename_pattern)
        .to_string_lossy()
        .into_owned()
}

async fn gui_capture(
    capture: Arc<X11Capture>,
    config: Arc<Config>,
    ui_tx: Sender<UiRequest>,
    path: String,
    clipboard: bool,
    delay: u32,
    busy_guard: BusyGuard,
) -> zbus::fdo::Result<()> {
    let t0 = std::time::Instant::now();
    sleep_delay(delay).await;
    let resp_rx = run_blocking(move || {
        submit_overlay(capture.as_ref(), config, &ui_tx, path, clipboard, busy_guard)
    })
    .await?;
    let result = resp_rx
        .await
        .map_err(|e| zbus::fdo::Error::Failed(format!("ui recv: {e}")))?;
    tracing::info!(
        total_ms = t0.elapsed().as_millis() as u64,
        "gui_capture finished"
    );
    match result {
        UiResult::Cancelled => tracing::info!("overlay cancelled"),
        UiResult::Done => tracing::info!("overlay done"),
    }
    Ok(())
}

/// Capture the cursor's monitor and submit a `ShowOverlay` request to the UI
/// loop. The single shared entrypoint for both the DBus `graphicCapture*`
/// methods and the tray click — same code path, same log line, same gating.
///
/// Sync (it does the blocking X11 capture and a non-blocking channel send).
/// Returns the receiver for the overlay's eventual `UiResult`. DBus awaits it
/// to know when the method call is "done"; the tray drops it (fire-and-forget).
pub fn submit_overlay(
    capture: &X11Capture,
    config: Arc<Config>,
    ui_tx: &Sender<UiRequest>,
    save_path: String,
    clipboard: bool,
    busy_guard: BusyGuard,
) -> anyhow::Result<tokio::sync::oneshot::Receiver<UiResult>> {
    let t0 = std::time::Instant::now();
    let include_cursor = config.capture.include_cursor;
    let screen = capture.cursor_screen()?;
    let t_screen = std::time::Instant::now();
    let image = capture.capture_screen_with_cursor(&screen, include_cursor)?;
    let t_capture = std::time::Instant::now();

    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
    ui_tx.send(UiRequest::ShowOverlay {
        image,
        screen_origin: (screen.x, screen.y),
        save_path,
        clipboard,
        config,
        result_tx: resp_tx,
        _busy_guard: Some(busy_guard),
    })?;
    let t_send = std::time::Instant::now();
    tracing::info!(
        cursor_screen_ms = (t_screen - t0).as_millis() as u64,
        capture_ms = (t_capture - t_screen).as_millis() as u64,
        send_ms = (t_send - t_capture).as_millis() as u64,
        "overlay submitted — eframe startup follows"
    );
    Ok(resp_rx)
}

enum CaptureKind {
    CursorScreen,
    All,
    Screen(usize),
}

async fn do_capture(
    capture: Arc<X11Capture>,
    config: Arc<Config>,
    kind: CaptureKind,
    path: String,
    delay: u32,
    clipboard: bool,
) -> zbus::fdo::Result<()> {
    sleep_delay(delay).await;
    let include_cursor = config.capture.include_cursor;
    run_blocking(move || -> anyhow::Result<()> {
        let img = match kind {
            CaptureKind::CursorScreen => {
                let screen = capture.cursor_screen()?;
                capture.capture_screen_with_cursor(&screen, include_cursor)?
            }
            CaptureKind::All => capture.capture_all()?,
            CaptureKind::Screen(n) => {
                let screens = capture.screens()?;
                let screen = screens
                    .get(n)
                    .ok_or_else(|| anyhow::anyhow!("screen {n} out of range"))?;
                capture.capture_screen_with_cursor(screen, include_cursor)?
            }
        };
        let mut acted = false;
        if !path.is_empty() {
            export::file::save_png(&img, Path::new(&path))?;
            tracing::info!(path = %path, "saved screenshot");
            acted = true;
        }
        if clipboard {
            export::clipboard::copy(&img)?;
            tracing::info!("copied to clipboard");
            acted = true;
        }
        if !acted {
            tracing::warn!("capture produced no output (--no-save and no -c)");
        }
        Ok(())
    })
    .await
}
