use crate::capture::X11Capture;
use crate::config::{self, Config};
use crate::export;
use crate::ui::{UiRequest, UiResult};
use crossbeam_channel::Sender;
use std::path::Path;
use std::sync::Arc;
use zbus::interface;

pub const SERVICE_NAME: &str = "org.rustshot.RustShot";
pub const OBJECT_PATH: &str = "/";

pub struct Service {
    pub capture: Arc<X11Capture>,
    pub config: Arc<Config>,
    pub ui_tx: Sender<UiRequest>,
}

#[interface(name = "org.rustshot.RustShot")]
impl Service {
    /// Flameshot-compatible: interactive region-select then save.
    /// Empty `path` triggers auto-save to the configured `save_dir`/`filename_pattern`.
    #[zbus(name = "graphicCapture")]
    async fn graphic_capture(
        &self,
        path: String,
        delay: u32,
        _id: String,
    ) -> zbus::fdo::Result<()> {
        if delay > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay as u64)).await;
        }
        let resolved = resolve_save_path(path, false, &self.config);
        gui_capture(self.capture.clone(), self.config.clone(), self.ui_tx.clone(), resolved, false).await
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
        if delay > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay as u64)).await;
        }
        let resolved = resolve_save_path(path, no_save, &self.config);
        gui_capture(self.capture.clone(), self.config.clone(), self.ui_tx.clone(), resolved, clipboard).await
    }

    #[zbus(name = "fullScreen")]
    async fn full_screen(
        &self,
        path: String,
        delay: u32,
        _id: String,
    ) -> zbus::fdo::Result<()> {
        let resolved = resolve_save_path(path, false, &self.config);
        do_capture(self.capture.clone(), self.config.clone(), CaptureKind::All, resolved, delay, false).await
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
        do_capture(self.capture.clone(), self.config.clone(), CaptureKind::All, resolved, delay, clipboard).await
    }

    #[zbus(name = "captureScreen")]
    async fn capture_screen(
        &self,
        screen_number: i32,
        path: String,
        delay: u32,
        _id: String,
    ) -> zbus::fdo::Result<()> {
        let resolved = resolve_save_path(path, false, &self.config);
        do_capture(self.capture.clone(), self.config.clone(), kind_for_screen(screen_number), resolved, delay, false).await
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
        do_capture(self.capture.clone(), self.config.clone(), kind_for_screen(screen_number), resolved, delay, clipboard).await
    }
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
) -> zbus::fdo::Result<()> {
    let include_cursor = config.capture.include_cursor;
    let cap = capture.clone();
    let (image, screen_origin) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let screen = cap.cursor_screen()?;
        let img = cap.capture_screen_with_cursor(&screen, include_cursor)?;
        Ok((img, (screen.x, screen.y)))
    })
    .await
    .map_err(|e| zbus::fdo::Error::Failed(format!("join: {e}")))?
    .map_err(|e| zbus::fdo::Error::Failed(format!("capture: {e}")))?;

    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
    ui_tx
        .send(UiRequest::ShowOverlay {
            image,
            screen_origin,
            save_path: path,
            clipboard,
            config,
            result_tx: resp_tx,
        })
        .map_err(|e| zbus::fdo::Error::Failed(format!("ui send: {e}")))?;
    let result = resp_rx
        .await
        .map_err(|e| zbus::fdo::Error::Failed(format!("ui recv: {e}")))?;

    match result {
        UiResult::Cancelled => tracing::info!("overlay cancelled"),
        UiResult::Done => tracing::info!("overlay done"),
    }
    Ok(())
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
    if delay > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(delay as u64)).await;
    }
    let include_cursor = config.capture.include_cursor;
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
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
    .map_err(|e| zbus::fdo::Error::Failed(format!("join: {e}")))?
    .map_err(|e| zbus::fdo::Error::Failed(format!("{e}")))?;
    Ok(())
}
