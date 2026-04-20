pub mod x11;

pub use self::x11::X11Capture;

#[derive(Debug, Clone, Copy)]
pub struct Screen {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
