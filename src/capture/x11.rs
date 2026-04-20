use super::Screen;
use crate::error::{Error, Result};
use image::RgbaImage;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xfixes::ConnectionExt as _;
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat};
use x11rb::rust_connection::RustConnection;

pub struct X11Capture {
    conn: RustConnection,
    root: u32,
    root_width: u16,
    root_height: u16,
    xfixes_ok: bool,
}

impl X11Capture {
    pub fn new() -> Result<Self> {
        let (conn, screen_num) = x11rb::connect(None)?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let root_width = screen.width_in_pixels;
        let root_height = screen.height_in_pixels;
        let xfixes_ok = match conn.xfixes_query_version(5, 0) {
            Ok(cookie) => cookie.reply().is_ok(),
            Err(e) => {
                tracing::warn!("xfixes query failed: {e}; cursor compositing disabled");
                false
            }
        };
        Ok(Self {
            conn,
            root,
            root_width,
            root_height,
            xfixes_ok,
        })
    }

    pub fn screens(&self) -> Result<Vec<Screen>> {
        let resources = self.conn.randr_get_screen_resources(self.root)?.reply()?;
        let mut screens = Vec::new();
        for &output in &resources.outputs {
            let info = self
                .conn
                .randr_get_output_info(output, resources.config_timestamp)?
                .reply()?;
            if info.crtc == 0 {
                continue;
            }
            let crtc = self
                .conn
                .randr_get_crtc_info(info.crtc, resources.config_timestamp)?
                .reply()?;
            if crtc.width == 0 || crtc.height == 0 {
                continue;
            }
            screens.push(Screen {
                x: crtc.x as i32,
                y: crtc.y as i32,
                width: crtc.width as u32,
                height: crtc.height as u32,
            });
        }
        Ok(screens)
    }

    pub fn cursor_position(&self) -> Result<(i32, i32)> {
        let reply = self.conn.query_pointer(self.root)?.reply()?;
        Ok((reply.root_x as i32, reply.root_y as i32))
    }

    pub fn cursor_screen(&self) -> Result<Screen> {
        let (x, y) = self.cursor_position()?;
        let screens = self.screens()?;
        screens
            .into_iter()
            .find(|s| {
                x >= s.x
                    && x < s.x + s.width as i32
                    && y >= s.y
                    && y < s.y + s.height as i32
            })
            .ok_or_else(|| Error::Other(format!("cursor at ({x},{y}) not on any screen")))
    }

    pub fn capture_screen(&self, screen: &Screen) -> Result<RgbaImage> {
        self.get_image(
            screen.x as i16,
            screen.y as i16,
            screen.width as u16,
            screen.height as u16,
        )
    }

    pub fn capture_all(&self) -> Result<RgbaImage> {
        self.get_image(0, 0, self.root_width, self.root_height)
    }

    /// Capture `screen` and (optionally) composite the X11 cursor on top via XFixes.
    pub fn capture_screen_with_cursor(
        &self,
        screen: &Screen,
        include_cursor: bool,
    ) -> Result<RgbaImage> {
        let mut img = self.capture_screen(screen)?;
        if include_cursor && self.xfixes_ok {
            if let Err(e) = self.composite_cursor(&mut img, screen.x, screen.y) {
                tracing::warn!("cursor composite failed: {e}");
            }
        }
        Ok(img)
    }

    fn get_image(&self, x: i16, y: i16, w: u16, h: u16) -> Result<RgbaImage> {
        let reply = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, self.root, x, y, w, h, !0u32)?
            .reply()?;
        bgrx_to_rgba(&reply.data, w as u32, h as u32)
    }

    /// XFixes cursor image is ARGB premultiplied (one u32 per pixel).
    /// Composite over the captured image at (cursor.x - hot - screen_origin).
    fn composite_cursor(&self, img: &mut RgbaImage, screen_x: i32, screen_y: i32) -> Result<()> {
        let cursor = self.conn.xfixes_get_cursor_image()?.reply()?;
        let cw = cursor.width as i32;
        let ch = cursor.height as i32;
        let cx = cursor.x as i32 - cursor.xhot as i32 - screen_x;
        let cy = cursor.y as i32 - cursor.yhot as i32 - screen_y;
        let img_w = img.width() as i32;
        let img_h = img.height() as i32;
        for j in 0..ch {
            let dy = cy + j;
            if dy < 0 || dy >= img_h {
                continue;
            }
            for i in 0..cw {
                let dx = cx + i;
                if dx < 0 || dx >= img_w {
                    continue;
                }
                let idx = (j * cw + i) as usize;
                let src = match cursor.cursor_image.get(idx) {
                    Some(&v) => v,
                    None => continue,
                };
                let a = ((src >> 24) & 0xff) as u32;
                if a == 0 {
                    continue;
                }
                let pr = ((src >> 16) & 0xff) as u32;
                let pg = ((src >> 8) & 0xff) as u32;
                let pb = (src & 0xff) as u32;
                let inv = 255 - a;
                let dst = img.get_pixel_mut(dx as u32, dy as u32);
                dst.0[0] = (pr + (dst.0[0] as u32 * inv) / 255).min(255) as u8;
                dst.0[1] = (pg + (dst.0[1] as u32 * inv) / 255).min(255) as u8;
                dst.0[2] = (pb + (dst.0[2] as u32 * inv) / 255).min(255) as u8;
            }
        }
        Ok(())
    }
}

/// X11 Z_PIXMAP on a 24-bit TrueColor visual returns BGRX bytes (4 per pixel).
/// Convert to RGBA. Assumes little-endian server (standard on x86_64).
fn bgrx_to_rgba(data: &[u8], w: u32, h: u32) -> Result<RgbaImage> {
    let pixel_count = (w as usize)
        .checked_mul(h as usize)
        .ok_or_else(|| Error::Other("image dimensions overflow".into()))?;
    let needed = pixel_count * 4;
    if data.len() < needed {
        return Err(Error::Other(format!(
            "X11 returned {} bytes; expected at least {needed}",
            data.len()
        )));
    }
    let mut out = Vec::with_capacity(needed);
    for chunk in data.chunks_exact(4).take(pixel_count) {
        out.extend_from_slice(&[chunk[2], chunk[1], chunk[0], 0xff]);
    }
    RgbaImage::from_raw(w, h, out)
        .ok_or_else(|| Error::Other("failed to construct RgbaImage".into()))
}
