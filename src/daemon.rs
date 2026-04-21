use crate::capture::X11Capture;
use crate::config::Config;
use crate::dbus::{Service, OBJECT_PATH, SERVICE_NAME};
use crate::ui;
use anyhow::Result;
use std::sync::Arc;

pub fn run() -> Result<()> {
    let config = Arc::new(Config::load_or_default());
    let capture = Arc::new(X11Capture::new()?);
    let (ui_tx, ui_rx) = crossbeam_channel::unbounded::<ui::UiRequest>();

    let capture_for_dbus = capture.clone();
    let config_for_dbus = config.clone();
    std::thread::Builder::new()
        .name("rustshot-dbus".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!("tokio runtime: {e}");
                    return;
                }
            };
            if let Err(e) = rt.block_on(dbus_main(capture_for_dbus, config_for_dbus, ui_tx)) {
                tracing::error!("dbus thread: {e}");
            }
        })?;

    ui::run_event_loop(ui_rx)?;
    Ok(())
}

async fn dbus_main(
    capture: Arc<X11Capture>,
    config: Arc<Config>,
    ui_tx: crossbeam_channel::Sender<ui::UiRequest>,
) -> Result<()> {
    let sni_tray = crate::tray::sni::Tray {
        capture: capture.clone(),
        config: config.clone(),
        ui_tx: ui_tx.clone(),
    };
    let tray_fallback = (capture.clone(), config.clone(), ui_tx.clone());
    let service = Service { capture, config, ui_tx };
    let built = zbus::connection::Builder::session()?
        .name(SERVICE_NAME)?
        .serve_at(OBJECT_PATH, service)?
        .build()
        .await;
    let _conn = match built {
        Ok(c) => c,
        Err(zbus::Error::NameTaken) => {
            eprintln!(
                "rustshot: another rustshot daemon already owns '{SERVICE_NAME}' on the session bus."
            );
            eprintln!("          Stop it first:");
            eprintln!("              systemctl --user stop rustshot.service");
            eprintln!("              pkill -x rustshot");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    tracing::info!(service = SERVICE_NAME, "rustshot daemon ready on DBus");

    // Status-tray icon. Try SNI first (KDE / polybar / i3status-rust).
    // If no SNI watcher exists on the bus — stock i3bar with tray_output
    // is the common case — fall back to an XEmbed client. Matches what Qt
    // does under the hood, which is why Flameshot works on i3bar tray.
    use ksni::TrayMethods;
    let _tray_handle = match sni_tray.spawn().await {
        Ok(h) => {
            tracing::info!("status-tray (SNI) registered");
            Some(h)
        }
        Err(e) => {
            tracing::info!("SNI unavailable ({e}); falling back to XEmbed");
            let (cap, cfg, tx) = tray_fallback;
            std::thread::Builder::new()
                .name("rustshot-xembed".into())
                .spawn(move || {
                    if let Err(e) = crate::tray::xembed::run(cap, cfg, tx) {
                        tracing::warn!("XEmbed tray unavailable: {e}");
                    }
                })?;
            None
        }
    };

    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");
    Ok(())
}
