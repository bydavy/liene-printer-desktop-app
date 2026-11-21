//! Non-destructive photo edits shared by the GUI preview and the print
//! pipeline.
//!
//! Rotation is constrained to right angles. The crop rectangle is stored in
//! normalized coordinates (0..1) relative to the **rotated** image, so it is
//! independent of the preview resolution and of the final print canvas.

use image::{DynamicImage, GenericImageView};
use serde::{Deserialize, Serialize};

/// Smallest allowed crop edge, as a fraction of the image.
pub const MIN_CROP: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NormalizedRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Default for NormalizedRect {
    fn default() -> Self {
        Self::FULL
    }
}

impl NormalizedRect {
    pub const FULL: Self = Self {
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 1.0,
    };

    /// Clamp the rectangle into the image, enforcing a minimum edge length.
    pub fn clamped(self) -> Self {
        if !self.x.is_finite() || !self.y.is_finite() || !self.w.is_finite() || !self.h.is_finite()
        {
            return Self::FULL;
        }

        let w = self.w.clamp(MIN_CROP, 1.0);
        let h = self.h.clamp(MIN_CROP, 1.0);
        let x = self.x.clamp(0.0, 1.0 - w);
        let y = self.y.clamp(0.0, 1.0 - h);
        Self { x, y, w, h }
    }

    pub fn is_full(self) -> bool {
        (self.x.abs() < 1e-4)
            && (self.y.abs() < 1e-4)
            && (self.w - 1.0).abs() < 1e-4
            && (self.h - 1.0).abs() < 1e-4
    }

    /// Crop aspect ratio measured in **pixels** for an image of `width` x
    /// `height` (the rotated image dimensions).
    pub fn pixel_aspect(self, width: u32, height: u32) -> f32 {
        if height == 0 {
            return 1.0;
        }
        (self.w * width as f32) / (self.h * height as f32)
    }

    /// Largest centered crop with the given pixel aspect ratio.
    pub fn centered_aspect(width: u32, height: u32, aspect: f32) -> Self {
        if width == 0 || height == 0 || !aspect.is_finite() || aspect <= 0.0 {
            return Self::FULL;
        }
        let image_aspect = width as f32 / height as f32;
        if image_aspect >= aspect {
            let w = aspect / image_aspect;
            Self {
                x: (1.0 - w) * 0.5,
                y: 0.0,
                w,
                h: 1.0,
            }
        } else {
            let h = image_aspect / aspect;
            Self {
                x: 0.0,
                y: (1.0 - h) * 0.5,
                w: 1.0,
                h,
            }
        }
    }

    /// Transform the crop when the image is rotated 90 degrees clockwise.
    pub fn rotate_cw(self) -> Self {
        Self {
            x: 1.0 - self.y - self.h,
            y: self.x,
            w: self.h,
            h: self.w,
        }
        .clamped()
    }

    /// Transform the crop when the image is rotated 90 degrees counter-clockwise.
    pub fn rotate_ccw(self) -> Self {
        Self {
            x: self.y,
            y: 1.0 - self.x - self.w,
            w: self.h,
            h: self.w,
        }
        .clamped()
    }

    pub fn rotate_180(self) -> Self {
        Self {
            x: 1.0 - self.x - self.w,
            y: 1.0 - self.y - self.h,
            w: self.w,
            h: self.h,
        }
        .clamped()
    }
}

/// Rotation in degrees (clockwise) plus a normalized crop.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PhotoEdit {
    pub rotation: u16,
    pub crop: NormalizedRect,
}

impl PhotoEdit {
    /// Rotation rounded down to a multiple of 90 in `0..360`.
    pub fn normalized_rotation(&self) -> u16 {
        (self.rotation / 90 % 4) * 90
    }

    pub fn rotate_cw(&mut self) {
        self.crop = self.crop.rotate_cw();
        self.rotation = (self.normalized_rotation() + 90) % 360;
    }

    pub fn rotate_ccw(&mut self) {
        self.crop = self.crop.rotate_ccw();
        self.rotation = (self.normalized_rotation() + 270) % 360;
    }

    /// Dimensions of the image after rotation.
    pub fn rotated_dimensions(&self, width: u32, height: u32) -> (u32, u32) {
        if matches!(self.normalized_rotation(), 90 | 270) {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// Rotate only; used for the editing preview (the crop is drawn as an
    /// overlay rather than baked into the texture).
    pub fn apply_rotation(&self, image: &DynamicImage) -> DynamicImage {
        match self.normalized_rotation() {
            90 => image.rotate90(),
            180 => image.rotate180(),
            270 => image.rotate270(),
            _ => image.clone(),
        }
    }

    /// Apply rotation and crop; used for the full-resolution print path.
    pub fn apply(&self, image: &DynamicImage) -> DynamicImage {
        let rotated = self.apply_rotation(image);
        let crop = self.crop.clamped();
        if crop.is_full() {
            return rotated;
        }

        let (width, height) = rotated.dimensions();
        if width == 0 || height == 0 {
            return rotated;
        }

        let x = ((crop.x * width as f32).round() as u32).min(width - 1);
        let y = ((crop.y * height as f32).round() as u32).min(height - 1);
        let w = ((crop.w * width as f32).round() as u32)
            .max(1)
            .min(width - x);
        let h = ((crop.h * height as f32).round() as u32)
            .max(1)
            .min(height - y);
        rotated.crop_imm(x, y, w, h)
    }

    /// Set the crop to the largest centered 2:3 rectangle for the rotated
    /// image dimensions.
    pub fn set_print_aspect_crop(&mut self, width: u32, height: u32) {
        self.crop = NormalizedRect::centered_aspect(width, height, 2.0 / 3.0);
    }

    /// Current framing zoom relative to the largest possible 2:3 crop.
    /// `1.0` is the full 2:3 frame; larger values zoom in.
    pub fn zoom(&self, width: u32, height: u32) -> f32 {
        let max = NormalizedRect::centered_aspect(width, height, 2.0 / 3.0);
        if self.crop.w > 0.0 {
            (max.w / self.crop.w).clamp(1.0, 64.0)
        } else {
            1.0
        }
    }

    /// Set a 2:3 crop at `zoom`, optionally keeping the current crop center.
    pub fn set_zoom(&mut self, width: u32, height: u32, zoom: f32, preserve_center: bool) {
        let max = NormalizedRect::centered_aspect(width, height, 2.0 / 3.0);
        let center = if preserve_center {
            (
                self.crop.x + self.crop.w * 0.5,
                self.crop.y + self.crop.h * 0.5,
            )
        } else {
            (max.x + max.w * 0.5, max.y + max.h * 0.5)
        };

        let zoom = if zoom.is_finite() {
            zoom.clamp(1.0, 64.0)
        } else {
            1.0
        };

        let mut w = (max.w / zoom).clamp(MIN_CROP, max.w);
        let mut h = w * (max.h / max.w);
        if h > max.h {
            h = max.h;
            w = h * (max.w / max.h);
        }

        let x = (center.0 - w * 0.5).clamp(0.0, 1.0 - w);
        let y = (center.1 - h * 0.5).clamp(0.0, 1.0 - h);
        self.crop = NormalizedRect { x, y, w, h };
    }

    /// Snap the crop to 2:3 while keeping its current center.
    pub fn snap_to_print_aspect(&mut self, width: u32, height: u32) {
        let zoom = self.zoom(width, height);
        self.set_zoom(width, height, zoom, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn test_image() -> DynamicImage {
        // 4x2 image with a distinct color in each quadrant.
        let mut image = RgbImage::new(4, 2);
        for y in 0..2 {
            for x in 0..4 {
                let color = match (x < 2, y == 0) {
                    (true, true) => Rgb([255, 0, 0]),
                    (false, true) => Rgb([0, 255, 0]),
                    (true, false) => Rgb([0, 0, 255]),
                    (false, false) => Rgb([255, 255, 0]),
                };
                image.put_pixel(x, y, color);
            }
        }
        DynamicImage::ImageRgb8(image)
    }

    #[test]
    fn clamps_into_image() {
        let rect = NormalizedRect {
            x: 0.9,
            y: -0.2,
            w: 0.5,
            h: 0.01,
        }
        .clamped();
        assert!((rect.x - 0.5).abs() < 1e-6);
        assert_eq!(rect.y, 0.0);
        assert!((rect.w - 0.5).abs() < 1e-6);
        assert!((rect.h - MIN_CROP).abs() < 1e-6);
    }

    #[test]
    fn rotates_crop_with_image() {
        let crop = NormalizedRect {
            x: 0.1,
            y: 0.2,
            w: 0.3,
            h: 0.4,
        };
        let cw = crop.rotate_cw();
        assert!((cw.x - 0.4).abs() < 1e-6);
        assert!((cw.y - 0.1).abs() < 1e-6);
        assert!((cw.w - 0.4).abs() < 1e-6);
        assert!((cw.h - 0.3).abs() < 1e-6);
        // Rotating back restores the original rectangle.
        let back = cw.rotate_ccw();
        for (a, b) in [
            (back.x, crop.x),
            (back.y, crop.y),
            (back.w, crop.w),
            (back.h, crop.h),
        ] {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn apply_rotates_and_crops() {
        let image = test_image();
        let mut edit = PhotoEdit {
            rotation: 90,
            crop: NormalizedRect::FULL,
        };
        let rotated = edit.apply(&image);
        assert_eq!(rotated.dimensions(), (2, 4));
        // Top-left pixel of the rotated image comes from the bottom-left.
        assert_eq!(rotated.to_rgb8().get_pixel(0, 0), &Rgb([0, 0, 255]));

        edit.crop = NormalizedRect {
            x: 0.0,
            y: 0.0,
            w: 0.5,
            h: 0.5,
        };
        let cropped = edit.apply(&image);
        assert_eq!(cropped.dimensions(), (1, 2));
    }

    #[test]
    fn centered_aspect_fits_image() {
        // Already 2:3.
        let crop = NormalizedRect::centered_aspect(600, 900, 2.0 / 3.0);
        assert!(crop.is_full());

        // Square image: crop a centered 2:3 column.
        let crop = NormalizedRect::centered_aspect(900, 900, 2.0 / 3.0);
        assert!((crop.w - 2.0 / 3.0).abs() < 1e-6);
        assert!((crop.x - 1.0 / 6.0).abs() < 1e-6);
        assert_eq!(crop.h, 1.0);

        // Tall image: crop a centered 2:3 band.
        let crop = NormalizedRect::centered_aspect(600, 1200, 2.0 / 3.0);
        assert_eq!(crop.w, 1.0);
        assert!((crop.h - 0.75).abs() < 1e-6);
        assert!((crop.y - 0.125).abs() < 1e-6);
    }

    #[test]
    fn zoom_keeps_aspect_and_center() {
        let mut edit = PhotoEdit::default();
        edit.set_zoom(600, 900, 1.0, false);
        assert!(edit.crop.is_full());

        edit.set_zoom(600, 900, 2.0, false);
        assert!((edit.crop.w - 0.5).abs() < 1e-6);
        assert!((edit.crop.h - 0.5).abs() < 1e-6);
        assert!(((edit.crop.x + edit.crop.w * 0.5) - 0.5).abs() < 1e-6);

        // Zooming around an off-center framing keeps that center when possible.
        edit.crop = NormalizedRect {
            x: 0.5,
            y: 0.0,
            w: 0.25,
            h: 0.25,
        };
        let center = (
            edit.crop.x + edit.crop.w * 0.5,
            edit.crop.y + edit.crop.h * 0.5,
        );
        edit.set_zoom(600, 900, 4.0, true);
        assert!((edit.crop.w - 0.25).abs() < 1e-6);
        assert!((edit.crop.x - (center.0 - 0.125)).abs() < 1e-6);
    }

    #[test]
    fn zoom_snaps_landscape_to_2_3() {
        let mut edit = PhotoEdit::default();
        edit.set_zoom(900, 600, 1.0, false);
        assert!((edit.crop.w - 4.0 / 9.0).abs() < 1e-6);
        assert!((edit.crop.h - 1.0).abs() < 1e-6);
        assert!((edit.crop.x - 5.0 / 18.0).abs() < 1e-6);
    }

    #[test]
    fn rotated_dimensions_swap() {
        let edit = PhotoEdit {
            rotation: 90,
            ..Default::default()
        };
        assert_eq!(edit.rotated_dimensions(600, 900), (900, 600));
        assert_eq!(edit.normalized_rotation(), 90);
    }
}
