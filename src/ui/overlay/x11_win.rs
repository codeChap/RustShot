//! X11 plumbing for the overlay: window, grabs, blit, cursors, keymap.
//! Everything that touches x11rb lives here so the paint/event-loop code
//! can stay in terms of our own types.

use crate::error::{Error, Result};
use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use x11rb::connection::Connection;
use x11rb::image::{BitsPerPixel, Image, ImageOrder, ScanlinePad};
use x11rb::protocol::shm::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    ChangeWindowAttributesAux, ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask,
    GrabMode, ImageFormat, Screen, WindowClass,
};
use x11rb::rust_connection::RustConnection;

/// Stock X11 cursor-font glyphs (see `/usr/include/X11/cursorfont.h`).
pub(super) const XC_CROSSHAIR: u16 = 34;
pub(super) const XC_FLEUR: u16 = 52;
pub(super) const XC_HAND1: u16 = 58;
pub(super) const XC_LEFT_PTR: u16 = 68;

// Keysyms we actually check. Stable X11 values from keysymdef.h.
pub(super) const KS_ESCAPE: u32 = 0xff1b;
pub(super) const KS_RETURN: u32 = 0xff0d;
pub(super) const KS_KP_ENTER: u32 = 0xff8d;
pub(super) const KS_C_LOWER: u32 = 0x0063;
pub(super) const KS_Z_LOWER: u32 = 0x007a;
pub(super) const KS_Y_LOWER: u32 = 0x0079;
pub(super) const KS_1: u32 = 0x0031;
pub(super) const KS_6: u32 = 0x0036;

pub(super) struct X11Win {
    pub conn: RustConnection,
    pub win: u32,
    pub gc: u32,
    pub width: u16,
    pub height: u16,
    pub depth: u8,
    cursor_font: u32,
    cursors: HashMap<u16, u32>,
    active_cursor_glyph: u16,
    /// MIT-SHM fast path: mapped directly into our process + attached to the
    /// X server. BGRA bytes written here are sent to the server with a tiny
    /// `shm_put_image` request instead of a ~33MB copy over the socket. `None`
    /// if MIT-SHM init failed (remote/SSH display, old server, etc) — we fall
    /// back to `blit_buf` + `Image::put`.
    shm: Option<ShmBuf>,
    blit_buf: Vec<u8>,
    /// keysym table: row-major, `keysyms_per_keycode` cols per keycode.
    keymap: Vec<u32>,
    min_keycode: u8,
    keysyms_per_keycode: u8,
}

struct ShmBuf {
    seg: shm::Seg,
    ptr: *mut u8,
    len: usize,
    /// Kept alive for the lifetime of the mapping. Dropping closes the fd,
    /// but the server-side segment is what references the pages after attach.
    _fd: OwnedFd,
}

// SAFETY: the pointer is into an anonymous mmap we own for the lifetime of
// the overlay. Access is single-threaded (everything is on the overlay's
// main thread). Send/Sync are declarative — we don't actually move across
// threads, but X11Win includes ShmBuf so they satisfy auto-trait bounds.
unsafe impl Send for ShmBuf {}
unsafe impl Sync for ShmBuf {}

impl X11Win {
    pub fn new(screen_origin: (i32, i32), width: u16, height: u16) -> Result<Self> {
        let (conn, screen_num) = x11rb::connect(None).map_err(Error::from)?;
        let setup = conn.setup();
        let screen: &Screen = &setup.roots[screen_num];
        let depth = screen.root_depth;
        let visual = screen.root_visual;
        let root = screen.root;

        let min_keycode = setup.min_keycode;
        let max_keycode = setup.max_keycode;

        let win = conn.generate_id()?;
        let gc = conn.generate_id()?;
        let cursor_font = conn.generate_id()?;

        let event_mask = EventMask::EXPOSURE
            | EventMask::KEY_PRESS
            | EventMask::KEY_RELEASE
            | EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION
            | EventMask::STRUCTURE_NOTIFY
            | EventMask::FOCUS_CHANGE;

        let win_aux = CreateWindowAux::new()
            .background_pixel(screen.black_pixel)
            .border_pixel(screen.black_pixel)
            .override_redirect(1)
            .event_mask(event_mask);

        conn.create_window(
            depth,
            win,
            root,
            screen_origin.0 as i16,
            screen_origin.1 as i16,
            width,
            height,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &win_aux,
        )?;

        conn.create_gc(gc, win, &CreateGCAux::new().graphics_exposures(0))?;
        conn.open_font(cursor_font, b"cursor")?;

        // Keysym map — queried once, small (usually ~6-8 syms/code × ~250 codes).
        let kb = conn
            .get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)?
            .reply()?;
        let keysyms_per_keycode = kb.keysyms_per_keycode;
        let keymap = kb.keysyms;

        conn.flush()?;

        let _ = screen_num;
        let _ = root;

        // Try MIT-SHM; on any failure we fall back to socket PutImage.
        let shm = match try_init_shm(&conn, width, height) {
            Ok(s) => {
                tracing::info!("MIT-SHM attached ({}x{})", width, height);
                Some(s)
            }
            Err(e) => {
                tracing::info!("MIT-SHM unavailable ({e}); using PutImage fallback");
                None
            }
        };

        Ok(Self {
            conn,
            win,
            gc,
            width,
            height,
            depth,
            cursor_font,
            cursors: HashMap::new(),
            active_cursor_glyph: u16::MAX,
            shm,
            blit_buf: Vec::new(),
            keymap,
            min_keycode,
            keysyms_per_keycode,
        })
    }

    /// Map the window and grab keyboard+pointer. Retries grabs briefly — i3 or
    /// another tool can briefly hold a grab when PrtSc fires and the request
    /// races with theirs.
    pub fn map_and_grab(&mut self) -> Result<()> {
        self.conn.map_window(self.win)?;
        self.conn.flush()?;

        for _ in 0..10 {
            let k = self
                .conn
                .grab_keyboard(
                    true,
                    self.win,
                    x11rb::CURRENT_TIME,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )?
                .reply()?;
            let p = self
                .conn
                .grab_pointer(
                    true,
                    self.win,
                    EventMask::BUTTON_PRESS
                        | EventMask::BUTTON_RELEASE
                        | EventMask::POINTER_MOTION,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    self.win,
                    x11rb::NONE,
                    x11rb::CURRENT_TIME,
                )?
                .reply()?;
            if u8::from(k.status) == 0 && u8::from(p.status) == 0 {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        tracing::warn!("keyboard/pointer grab didn't succeed after retries — continuing anyway");
        Ok(())
    }

    pub fn set_cursor(&mut self, glyph_id: u16) -> Result<()> {
        if self.active_cursor_glyph == glyph_id {
            return Ok(());
        }
        let cur = match self.cursors.get(&glyph_id) {
            Some(&c) => c,
            None => {
                let c = self.conn.generate_id()?;
                self.conn.create_glyph_cursor(
                    c,
                    self.cursor_font,
                    self.cursor_font,
                    glyph_id,
                    glyph_id + 1,
                    0, 0, 0,
                    0xffff, 0xffff, 0xffff,
                )?;
                self.cursors.insert(glyph_id, c);
                c
            }
        };
        self.conn
            .change_window_attributes(self.win, &ChangeWindowAttributesAux::new().cursor(cur))?;
        self.conn.flush()?;
        self.active_cursor_glyph = glyph_id;
        Ok(())
    }

    /// Blit an RGBA buffer (matching window size) to the window. Uses the
    /// MIT-SHM fast path when available: BGRA swap goes straight into the
    /// shared-memory buffer, then one tiny `shm_put_image` request hands off
    /// to the X server. Falls back to `Image::put` if SHM wasn't initialized.
    pub fn blit_rgba(&mut self, rgba: &[u8]) -> Result<()> {
        let w = self.width as usize;
        let h = self.height as usize;
        let expected = w * h * 4;
        if rgba.len() != expected {
            return Err(Error::Other(format!(
                "blit: got {} bytes, expected {}",
                rgba.len(),
                expected
            )));
        }

        if let Some(shm) = &self.shm {
            // SAFETY: exclusive access via &mut self; ptr + len came from mmap
            // and are valid for the lifetime of `shm`.
            let dst = unsafe { std::slice::from_raw_parts_mut(shm.ptr, shm.len) };
            swap_rgba_to_bgra(rgba, dst);
            self.conn.shm_put_image(
                self.win,
                self.gc,
                self.width,
                self.height,
                0,
                0,
                self.width,
                self.height,
                0,
                0,
                self.depth,
                ImageFormat::Z_PIXMAP.into(),
                false,
                shm.seg,
                0,
            )?;
            self.conn.flush()?;
            return Ok(());
        }

        // Fallback: socket-based PutImage.
        if self.blit_buf.len() != expected {
            self.blit_buf.resize(expected, 0);
        }
        swap_rgba_to_bgra(rgba, &mut self.blit_buf);
        let img = Image::new(
            self.width,
            self.height,
            ScanlinePad::Pad32,
            self.depth,
            BitsPerPixel::B32,
            ImageOrder::LsbFirst,
            std::borrow::Cow::Borrowed(&self.blit_buf),
        )
        .map_err(|e| Error::Other(format!("Image::new: {e}")))?;
        img.put(&self.conn, self.win, self.gc, 0, 0)
            .map_err(|e| Error::Other(format!("Image::put: {e}")))?;
        self.conn.flush()?;
        Ok(())
    }

    /// Fallback direct `PutImage` — only used for the first frame before we
    /// have a full composite ready. Avoids a blank flash on slow repaints.
    #[allow(dead_code)]
    pub fn fill_black(&mut self) -> Result<()> {
        // Small solid-black put_image: 1x1 scaled by server fill. Cheaper
        // than allocating a full-screen buffer just to clear.
        self.conn.put_image(
            ImageFormat::Z_PIXMAP,
            self.win,
            self.gc,
            1,
            1,
            0,
            0,
            0,
            self.depth,
            &[0, 0, 0, 0],
        )?;
        self.conn.flush()?;
        Ok(())
    }

    /// Return the unshifted keysym for a keycode (col 0). Sufficient for our
    /// shortcut set — none of them vary with Shift.
    pub fn keysym(&self, keycode: u8) -> u32 {
        if keycode < self.min_keycode {
            return 0;
        }
        let row = (keycode - self.min_keycode) as usize;
        let cols = self.keysyms_per_keycode as usize;
        let idx = row * cols;
        self.keymap.get(idx).copied().unwrap_or(0)
    }

    pub fn teardown(&self) {
        if let Some(shm) = &self.shm {
            let _ = self.conn.shm_detach(shm.seg);
            // SAFETY: ptr/len came from mmap in try_init_shm; exactly one munmap.
            unsafe {
                libc::munmap(shm.ptr as *mut libc::c_void, shm.len);
            }
        }
        let _ = self.conn.ungrab_pointer(x11rb::CURRENT_TIME);
        let _ = self.conn.ungrab_keyboard(x11rb::CURRENT_TIME);
        let _ = self.conn.destroy_window(self.win);
        let _ = self.conn.close_font(self.cursor_font);
        for &c in self.cursors.values() {
            let _ = self.conn.free_cursor(c);
        }
        let _ = self.conn.free_gc(self.gc);
        let _ = self.conn.flush();
    }
}

impl Drop for X11Win {
    fn drop(&mut self) {
        self.teardown();
    }
}

/// In-place RGBA→BGRA (alpha forced opaque). LLVM autovectorizes this well
/// in release; the compiler sees the aligned 4-byte stride + no aliasing.
fn swap_rgba_to_bgra(src: &[u8], dst: &mut [u8]) {
    debug_assert_eq!(src.len(), dst.len());
    for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        d[0] = s[2];
        d[1] = s[1];
        d[2] = s[0];
        d[3] = 0xff;
    }
}

/// Try to allocate a memfd, mmap it, and attach to the X server. Any failure
/// is non-fatal — caller treats this as "SHM path unavailable".
fn try_init_shm(conn: &RustConnection, width: u16, height: u16) -> Result<ShmBuf> {
    let version = conn
        .shm_query_version()
        .map_err(|e| Error::Other(format!("shm_query_version: {e}")))?
        .reply()
        .map_err(|e| Error::Other(format!("shm_query_version reply: {e}")))?;
    if !version.shared_pixmaps {
        // Even without shared_pixmaps, shm_put_image works. We just log it.
        tracing::debug!("MIT-SHM: shared_pixmaps=false (that's fine, we only need put_image)");
    }

    let len = (width as usize) * (height as usize) * 4;

    // memfd_create is available on Linux 3.17+. Flag CLOEXEC=1.
    let name = b"rustshot-shm\0";
    let fd_raw = unsafe { libc::memfd_create(name.as_ptr() as *const libc::c_char, 1) };
    if fd_raw < 0 {
        return Err(Error::Other(format!(
            "memfd_create: {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: fd_raw is a valid fresh fd; transfer ownership.
    let fd = unsafe { OwnedFd::from_raw_fd(fd_raw) };

    if unsafe { libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) } < 0 {
        return Err(Error::Other(format!(
            "ftruncate({len}): {}",
            std::io::Error::last_os_error()
        )));
    }

    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd.as_raw_fd(),
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(Error::Other(format!(
            "mmap({len}): {}",
            std::io::Error::last_os_error()
        )));
    }

    // Send a dup of the fd so we keep a handle of our own. x11rb takes
    // ownership of whatever we hand it — it'll close that copy after sending.
    let fd_dup = fd
        .try_clone()
        .map_err(|e| Error::Other(format!("fd dup: {e}")))?;

    let seg = conn
        .generate_id()
        .map_err(|e| Error::Other(format!("shm seg id: {e}")))?;
    conn.shm_attach_fd(seg, fd_dup, false)
        .map_err(|e| Error::Other(format!("shm_attach_fd: {e}")))?;
    conn.flush()?;

    Ok(ShmBuf {
        seg,
        ptr: ptr as *mut u8,
        len,
        _fd: fd,
    })
}
