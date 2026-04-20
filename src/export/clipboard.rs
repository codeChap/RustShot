use crate::error::{Error, Result};
use image::RgbaImage;
use std::borrow::Cow;

pub fn copy(img: &RgbaImage) -> Result<()> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| Error::Other(format!("clipboard init: {e}")))?;
    let img_data = arboard::ImageData {
        width: img.width() as usize,
        height: img.height() as usize,
        bytes: Cow::Borrowed(img.as_raw().as_slice()),
    };
    clipboard
        .set_image(img_data)
        .map_err(|e| Error::Other(format!("clipboard set: {e}")))?;
    Ok(())
}
