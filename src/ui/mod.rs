pub mod overlay;
pub mod toolbar;

use crate::config::Config;
use image::RgbaImage;
use std::sync::Arc;

pub enum UiRequest {
    ShowOverlay {
        image: RgbaImage,
        screen_origin: (i32, i32),
        save_path: String,
        clipboard: bool,
        config: Arc<Config>,
        result_tx: tokio::sync::oneshot::Sender<UiResult>,
    },
}

#[derive(Debug, Clone)]
pub enum UiResult {
    Done,
    Cancelled,
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
            } => {
                let result = overlay::show(image, screen_origin, save_path, clipboard, config);
                let _ = result_tx.send(result);
            }
        }
    }
    Ok(())
}
