//! Status-tray backend: legacy X11 XEmbed. Used when no SNI watcher is on
//! the session bus — covers stock i3bar with `tray_output primary` set.
//!
//! Protocol summary:
//!   1. Own / find `_NET_SYSTEM_TRAY_S<screen>` — the tray manager holds it.
//!   2. Create our tiny 22x22 window.
//!   3. Send a `_NET_SYSTEM_TRAY_OPCODE` ClientMessage with
//!      `SYSTEM_TRAY_REQUEST_DOCK` to the tray owner.
//!   4. Let the tray reparent the window; repaint on Expose; react to clicks.
//!
//! Rendering is intentionally simple: four FillRectangles form a yellow
//! selection-frame glyph on a dark background. No alpha, no XRender —
//! stays lightweight and matches the rest of RustShot's visual language.

use crate::capture::X11Capture;
use crate::config::Config;
use crate::ui::UiRequest;
use crossbeam_channel::Sender;
use std::sync::Arc;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, Gcontext,
    PropMode, Rectangle, Window, WindowClass, CLIENT_MESSAGE_EVENT,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

const ICON: u16 = 22;
const DARK_PX: u32 = 0x0020_2024;
const YELLOW_PX: u32 = 0x00FF_C800;

/// Blocks on the X event loop for the tray window. Returns `Ok(())` on clean
/// exit (e.g. tray restart), `Err` if we can't even find a tray manager.
pub fn run(
    capture: Arc<X11Capture>,
    config: Arc<Config>,
    ui_tx: Sender<UiRequest>,
) -> anyhow::Result<()> {
    let (conn, screen_num) = RustConnection::connect(None)?;
    let screen = &conn.setup().roots[screen_num];

    let tray_sel = intern(&conn, &format!("_NET_SYSTEM_TRAY_S{screen_num}"))?;
    let xembed_info = intern(&conn, "_XEMBED_INFO")?;
    let tray_opcode = intern(&conn, "_NET_SYSTEM_TRAY_OPCODE")?;

    let tray_owner = conn.get_selection_owner(tray_sel)?.reply()?.owner;
    if tray_owner == x11rb::NONE {
        anyhow::bail!(
            "no XEmbed tray manager (selection `_NET_SYSTEM_TRAY_S{screen_num}` has no owner — \
             set `tray_output primary` in your i3bar config)"
        );
    }

    let wid = conn.generate_id()?;
    // override_redirect=true keeps the window manager (i3) out of this
    // window's business — without it, i3 sees a top-level mapped window
    // before the tray reparents it and floats / tiles it full-screen.
    let attrs = CreateWindowAux::default()
        .override_redirect(1)
        .event_mask(
            EventMask::EXPOSURE | EventMask::BUTTON_PRESS | EventMask::STRUCTURE_NOTIFY,
        )
        .background_pixel(DARK_PX);
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        wid,
        screen.root,
        0,
        0,
        ICON,
        ICON,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &attrs,
    )?;

    conn.change_property8(
        PropMode::REPLACE,
        wid,
        u32::from(AtomEnum::WM_NAME),
        u32::from(AtomEnum::STRING),
        b"RustShot",
    )?;
    conn.change_property8(
        PropMode::REPLACE,
        wid,
        u32::from(AtomEnum::WM_CLASS),
        u32::from(AtomEnum::STRING),
        b"rustshot\0RustShot\0",
    )?;

    // _XEMBED_INFO: version=0, flags=XEMBED_MAPPED (bit 0)
    let xembed_data = [0u32, 1u32];
    conn.change_property32(
        PropMode::REPLACE,
        wid,
        xembed_info,
        xembed_info,
        &xembed_data,
    )?;

    // SYSTEM_TRAY_REQUEST_DOCK(wid) → tray_owner
    let mut payload = [0u32; 5];
    payload[0] = 0; // timestamp = CurrentTime
    payload[1] = 0; // SYSTEM_TRAY_REQUEST_DOCK
    payload[2] = wid;
    let dock = ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 0,
        window: tray_owner,
        type_: tray_opcode,
        data: payload.into(),
    };
    conn.send_event(false, tray_owner, EventMask::NO_EVENT, dock)?;
    conn.map_window(wid)?;

    let gc_bg = conn.generate_id()?;
    conn.create_gc(gc_bg, wid, &CreateGCAux::default().foreground(DARK_PX))?;
    let gc_frame = conn.generate_id()?;
    conn.create_gc(gc_frame, wid, &CreateGCAux::default().foreground(YELLOW_PX))?;

    conn.flush()?;

    let mut w = ICON;
    let mut h = ICON;
    loop {
        let event = conn.wait_for_event()?;
        match event {
            Event::Expose(e) if e.window == wid => {
                paint(&conn, wid, gc_bg, gc_frame, w, h)?;
                conn.flush()?;
            }
            Event::ConfigureNotify(e) if e.window == wid => {
                w = e.width;
                h = e.height;
            }
            Event::ButtonPress(e) if e.event == wid && e.detail == 1 => {
                super::spawn_capture(capture.clone(), config.clone(), ui_tx.clone());
            }
            Event::DestroyNotify(e) if e.window == wid => {
                tracing::info!("XEmbed tray window destroyed (tray restart?)");
                break;
            }
            _ => {}
        }
    }
    Ok(())
}

fn paint(
    conn: &RustConnection,
    wid: Window,
    gc_bg: Gcontext,
    gc_frame: Gcontext,
    w: u16,
    h: u16,
) -> anyhow::Result<()> {
    conn.poly_fill_rectangle(
        wid,
        gc_bg,
        &[Rectangle {
            x: 0,
            y: 0,
            width: w,
            height: h,
        }],
    )?;

    let min = w.min(h) as f32;
    let m = (min * 0.18).round() as i16;
    let t = ((min * 0.09).round() as u16).max(1);
    let iw = w as i16 - 2 * m;
    let ih = h as i16 - 2 * m;
    if iw > (2 * t as i16) && ih > (2 * t as i16) {
        let frame = [
            Rectangle { x: m, y: m, width: iw as u16, height: t },
            Rectangle {
                x: m,
                y: m + ih - t as i16,
                width: iw as u16,
                height: t,
            },
            Rectangle { x: m, y: m, width: t, height: ih as u16 },
            Rectangle {
                x: m + iw - t as i16,
                y: m,
                width: t,
                height: ih as u16,
            },
        ];
        conn.poly_fill_rectangle(wid, gc_frame, &frame)?;
    }
    Ok(())
}

fn intern(conn: &RustConnection, name: &str) -> anyhow::Result<u32> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}
