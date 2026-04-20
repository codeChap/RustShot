use crate::error::{Error, Result};
use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder, RgbaImage};
use std::io::Write;
use std::process::{Command, Stdio};

/// Put `img` on the X11 CLIPBOARD selection as `image/png`.
///
/// We shell out to `xclip` because it properly holds clipboard ownership until
/// another app replaces it — arboard's in-process approach loses the ownership
/// as soon as the `Clipboard` handle drops, so pasted data shows up as empty
/// on systems without a running clipboard manager.
///
/// The spawned xclip process lives on past this function; we spawn a reaper
/// thread so it doesn't accumulate as a zombie when the next copy replaces it.
pub fn copy(img: &RgbaImage) -> Result<()> {
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|e| Error::Other(format!("png encode: {e}")))?;

    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "image/png", "-i"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            Error::Other(format!(
                "spawn xclip: {e} — install it (apt install xclip) or switch to another tool"
            ))
        })?;

    // Write the PNG and close stdin so xclip starts advertising the clipboard.
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Other("xclip stdin unavailable".into()))?;
        stdin
            .write_all(&png)
            .map_err(|e| Error::Other(format!("write to xclip: {e}")))?;
    }

    // xclip stays alive holding the clipboard; reap it in the background when
    // it eventually exits (either replaced by another copy or on shutdown).
    std::thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(())
}
