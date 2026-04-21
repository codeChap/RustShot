//! Status-tray backend: StatusNotifierItem (SNI). Modern freedesktop protocol
//! used by KDE, polybar with SNI, i3status-rust, etc. Goes via DBus, served
//! by the `ksni` crate.

use crate::capture::X11Capture;
use crate::config::Config;
use crate::ui::UiRequest;
use crossbeam_channel::Sender;
use std::sync::Arc;

pub struct Tray {
    pub capture: Arc<X11Capture>,
    pub config: Arc<Config>,
    pub ui_tx: Sender<UiRequest>,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "org.rustshot.RustShot".into()
    }

    fn title(&self) -> String {
        "RustShot".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
            title: "RustShot — click to capture a region".into(),
            description: String::new(),
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![build_icon()]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        super::spawn_capture(self.capture.clone(), self.config.clone(), self.ui_tx.clone());
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Capture region".into(),
                activate: Box::new(|this: &mut Self| {
                    super::spawn_capture(
                        this.capture.clone(),
                        this.config.clone(),
                        this.ui_tx.clone(),
                    );
                }),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// 22×22 ARGB32 icon (network byte order, as SNI requires): yellow selection
/// frame inside a dark rounded badge. Matches the overlay's frame color.
fn build_icon() -> ksni::Icon {
    const SIZE: i32 = 22;
    let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    let cx = (SIZE as f32 - 1.0) / 2.0;
    let cy = (SIZE as f32 - 1.0) / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let in_badge = dist <= 10.3;
            let in_outer = x >= 4 && x <= 17 && y >= 4 && y <= 17;
            let in_inner = x >= 6 && x <= 15 && y >= 6 && y <= 15;
            let on_frame = in_outer && !in_inner;
            let (a, r, g, b) = if on_frame {
                (0xFF, 0xFF, 0xC8, 0x00)
            } else if in_badge {
                (0xFF, 0x20, 0x20, 0x24)
            } else {
                (0x00, 0x00, 0x00, 0x00)
            };
            data.extend_from_slice(&[a, r, g, b]);
        }
    }
    ksni::Icon {
        width: SIZE,
        height: SIZE,
        data,
    }
}
