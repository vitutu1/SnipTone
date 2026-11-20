use crate::frame::CapturedFrame;
use crate::tonemap::bgra8_to_rgb8;
use anyhow::{Context, Result};
use chrono::Local;
use image::{ImageBuffer, Rgb, Rgb32FImage};
use std::fs;
use std::path::PathBuf;

pub fn default_output_dir() -> PathBuf {
    let base = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Pictures").join("SnipTone")
}

pub fn save_sdr_png(
    width: u32,
    height: u32,
    bgra: &[u8],
    operator_tag: &str,
) -> Result<PathBuf> {
    let dir = default_output_dir();
    fs::create_dir_all(&dir).with_context(|| format!("Failed to create {:?}", dir))?;

    let stamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    let safe_tag: String = operator_tag
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let path = dir.join(format!("SnipTone_{}_{}.png", stamp, safe_tag));

    let rgb = bgra8_to_rgb8(bgra);
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width, height, rgb).context("Failed to construct RGB8 image")?;
    img.save(&path)
        .with_context(|| format!("Failed to save PNG to {:?}", path))?;

    Ok(path)
}

/// Saves the raw linear scRGB HDR frame as a lossless 32-bit float OpenEXR file,
/// preserving values > 1.0 and negative wide-gamut values for offline tonemapping experiments.
pub fn save_hdr_exr(frame: &CapturedFrame) -> Result<PathBuf> {
    let dir = default_output_dir();
    fs::create_dir_all(&dir).with_context(|| format!("Failed to create {:?}", dir))?;

    let stamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    let path = dir.join(format!("SnipTone_{}_raw_scrgb.exr", stamp));

    let mut flat = Vec::with_capacity(frame.rgb.len() * 3);
    for &[r, g, b] in &frame.rgb {
        flat.push(r);
        flat.push(g);
        flat.push(b);
    }

    let img: Rgb32FImage = ImageBuffer::from_raw(frame.width, frame.height, flat)
        .context("Failed to construct Rgb32FImage")?;
    img.save(&path)
        .with_context(|| format!("Failed to save EXR to {:?}", path))?;

    Ok(path)
}
