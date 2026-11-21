use std::path::Path;

use anyhow::{bail, Context, Result};
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, ImageDecoder, ImageReader, Rgb, RgbImage};
use serde::{Deserialize, Serialize};

use crate::edit::PhotoEdit;

/// Upper bound used by the official app when compressing a photo
/// (`M.f(..., 1023)` compares against 1023 KiB).
const MAX_JPEG_BYTES: usize = 1023 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Scale up until the image fills the canvas, then crop the overflow.
    Cover,
    /// Scale down until the image fits, padding with white.
    Contain,
}

impl Fit {
    pub fn parse(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "cover" | "fill" | "crop" => Ok(Self::Cover),
            "contain" | "fit" | "letterbox" => Ok(Self::Contain),
            other => bail!("unknown fit mode '{other}' (use cover or contain)"),
        }
    }
}

/// Load an image, fit it onto a `width` x `height` canvas, and encode it as
/// a JPEG no larger than the app's ~1 MiB transfer target.
pub fn prepare_jpeg(path: &Path, width: u32, height: u32, fit: Fit) -> Result<Vec<u8>> {
    prepare_jpeg_with_edit(path, width, height, fit, &PhotoEdit::default())
}

/// Like [`prepare_jpeg`], but applies the user's rotation and crop first.
pub fn prepare_jpeg_with_edit(
    path: &Path,
    width: u32,
    height: u32,
    fit: Fit,
    edit: &PhotoEdit,
) -> Result<Vec<u8>> {
    if width == 0 || height == 0 {
        bail!("canvas dimensions must be non-zero");
    }

    let image = load_oriented(path)
        .with_context(|| format!("could not decode image {}", path.display()))?;
    let image = edit.apply(&image);
    let rgb = image.to_rgb8();

    let canvas = match fit {
        Fit::Cover => cover(&rgb, width, height),
        Fit::Contain => contain(&rgb, width, height),
    };

    encode_jpeg(&canvas, MAX_JPEG_BYTES)
        .with_context(|| format!("could not encode {}", path.display()))
}

/// Decode an image and apply its EXIF orientation, so phone photos are not
/// printed sideways.
pub fn load_oriented(path: &Path) -> Result<DynamicImage> {
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

fn cover(source: &RgbImage, width: u32, height: u32) -> RgbImage {
    let (sw, sh) = source.dimensions();
    let scale = f64::max(
        f64::from(width) / f64::from(sw),
        f64::from(height) / f64::from(sh),
    );
    let nw = (f64::from(sw) * scale).round().max(1.0) as u32;
    let nh = (f64::from(sh) * scale).round().max(1.0) as u32;
    let mut resized = image::imageops::resize(source, nw, nh, FilterType::Lanczos3);
    let x = (nw - width) / 2;
    let y = (nh - height) / 2;
    image::imageops::crop(&mut resized, x, y, width, height).to_image()
}

fn contain(source: &RgbImage, width: u32, height: u32) -> RgbImage {
    let (sw, sh) = source.dimensions();
    let scale = f64::min(
        f64::from(width) / f64::from(sw),
        f64::from(height) / f64::from(sh),
    );
    let nw = (f64::from(sw) * scale).round().max(1.0) as u32;
    let nh = (f64::from(sh) * scale).round().max(1.0) as u32;
    let resized = image::imageops::resize(source, nw, nh, FilterType::Lanczos3);

    let mut canvas = RgbImage::from_pixel(width, height, Rgb([255, 255, 255]));
    let x = (width - nw) / 2;
    let y = (height - nh) / 2;
    image::imageops::overlay(&mut canvas, &resized, i64::from(x), i64::from(y));
    canvas
}

fn encode_jpeg(canvas: &RgbImage, max_bytes: usize) -> Result<Vec<u8>> {
    let (width, height) = canvas.dimensions();

    let mut quality = 92u8;
    loop {
        let mut buf = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
        encoder.encode(canvas.as_raw(), width, height, ExtendedColorType::Rgb8)?;

        if buf.len() <= max_bytes || quality <= 40 {
            return Ok(buf);
        }
        quality = quality.saturating_sub(4);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_keeps_canvas_size() {
        let source = RgbImage::from_pixel(100, 100, Rgb([10, 20, 30]));
        let out = cover(&source, 600, 900);
        assert_eq!(out.dimensions(), (600, 900));
    }

    #[test]
    fn contain_pads_with_white() {
        let source = RgbImage::from_pixel(900, 600, Rgb([10, 20, 30]));
        let out = contain(&source, 600, 900);
        assert_eq!(out.dimensions(), (600, 900));
        assert_eq!(out.get_pixel(300, 5), &Rgb([255, 255, 255]));
    }
}
