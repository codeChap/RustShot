use crate::error::Result;
use image::RgbaImage;
use std::path::Path;

pub fn save_png(img: &RgbaImage, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    img.save_with_format(path, image::ImageFormat::Png)?;
    Ok(())
}
