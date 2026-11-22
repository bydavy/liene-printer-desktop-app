//! Collage layouts and rendering for 2–4 photos on a 2×3 canvas.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use image::imageops::FilterType;
use image::{Rgb, RgbImage};
use serde::{Deserialize, Serialize};

use crate::image_prep;

const MARGIN: f32 = 0.035;
const GAP: f32 = 0.018;

#[derive(Debug, Clone, Copy)]
pub struct CollageCell {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct PhotoFrame {
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
    pub rotation: u32,
}

impl Default for PhotoFrame {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            rotation: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CollageLayout {
    pub id: &'static str,
    pub name: &'static str,
    pub cells: Vec<CollageCell>,
}

pub fn layouts_for(count: usize) -> Vec<CollageLayout> {
    layouts_for_borders(count, true)
}

pub fn layouts_for_borders(count: usize, borders: bool) -> Vec<CollageLayout> {
    let (margin, gap) = if borders { (MARGIN, GAP) } else { (0.0, 0.0) };
    match count {
        2 => vec![
            layout("v2", "Vertical", &[1.0, 1.0], &[1.0], margin, gap),
            layout("h2", "Horizontal", &[1.0], &[1.0, 1.0], margin, gap),
            layout("top2", "Top focus", &[0.62, 0.38], &[1.0], margin, gap),
            layout("left2", "Left focus", &[1.0], &[0.62, 0.38], margin, gap),
        ],
        3 => vec![
            layout("v3", "Vertical", &[1.0, 1.0, 1.0], &[1.0], margin, gap),
            layout("h3", "Horizontal", &[1.0], &[1.0, 1.0, 1.0], margin, gap),
            hero("left3", "Left hero", true, margin, gap),
            hero("top3", "Top hero", false, margin, gap),
        ],
        4 => vec![
            layout("grid4", "2×2", &[1.0, 1.0], &[1.0, 1.0], margin, gap),
            layout("v4", "4 down", &[1.0, 1.0, 1.0, 1.0], &[1.0], margin, gap),
            layout("h4", "4 across", &[1.0], &[1.0, 1.0, 1.0, 1.0], margin, gap),
            hero_grid("left4", "Left hero", 3, margin, gap),
        ],
        _ => Vec::new(),
    }
}

fn layout(
    id: &'static str,
    name: &'static str,
    rows: &[f32],
    cols: &[f32],
    margin: f32,
    gap: f32,
) -> CollageLayout {
    CollageLayout {
        id,
        name,
        cells: grid(rows, cols, margin, gap),
    }
}

fn grid(rows: &[f32], cols: &[f32], margin: f32, gap: f32) -> Vec<CollageCell> {
    let content_w = 1.0 - 2.0 * margin - gap * (cols.len() as f32 - 1.0);
    let content_h = 1.0 - 2.0 * margin - gap * (rows.len() as f32 - 1.0);
    let row_sum: f32 = rows.iter().sum();
    let col_sum: f32 = cols.iter().sum();
    let heights: Vec<f32> = rows.iter().map(|r| content_h * r / row_sum).collect();
    let widths: Vec<f32> = cols.iter().map(|c| content_w * c / col_sum).collect();

    let mut cells = Vec::with_capacity(rows.len() * cols.len());
    let mut y = margin;
    for height in heights {
        let mut x = margin;
        for width in &widths {
            cells.push(CollageCell {
                x,
                y,
                w: *width,
                h: height,
            });
            x += width + gap;
        }
        y += height + gap;
    }
    cells
}

fn hero(id: &'static str, name: &'static str, left: bool, margin: f32, gap: f32) -> CollageLayout {
    let content_w = 1.0 - 2.0 * margin;
    let content_h = 1.0 - 2.0 * margin;
    let cells = if left {
        let split = margin + (content_w - gap) * 0.62;
        let small_h = (content_h - gap) / 2.0;
        vec![
            CollageCell {
                x: margin,
                y: margin,
                w: split - margin,
                h: content_h,
            },
            CollageCell {
                x: split + gap,
                y: margin,
                w: 1.0 - margin - split - gap,
                h: small_h,
            },
            CollageCell {
                x: split + gap,
                y: margin + small_h + gap,
                w: 1.0 - margin - split - gap,
                h: small_h,
            },
        ]
    } else {
        let split = margin + (content_h - gap) * 0.62;
        let small_w = (content_w - gap) / 2.0;
        vec![
            CollageCell {
                x: margin,
                y: margin,
                w: content_w,
                h: split - margin,
            },
            CollageCell {
                x: margin,
                y: split + gap,
                w: small_w,
                h: 1.0 - margin - split - gap,
            },
            CollageCell {
                x: margin + small_w + gap,
                y: split + gap,
                w: small_w,
                h: 1.0 - margin - split - gap,
            },
        ]
    };
    CollageLayout { id, name, cells }
}

fn hero_grid(
    id: &'static str,
    name: &'static str,
    rows: usize,
    margin: f32,
    gap: f32,
) -> CollageLayout {
    let content_w = 1.0 - 2.0 * margin;
    let content_h = 1.0 - 2.0 * margin;
    let split = margin + (content_w - gap) / 2.0;
    let mut cells = vec![CollageCell {
        x: margin,
        y: margin,
        w: split - margin,
        h: content_h,
    }];
    let small_h = (content_h - gap * (rows as f32 - 1.0)) / rows as f32;
    for index in 0..rows {
        let y = margin + index as f32 * (small_h + gap);
        cells.push(CollageCell {
            x: split + gap,
            y,
            w: 1.0 - margin - split - gap,
            h: small_h,
        });
    }
    CollageLayout { id, name, cells }
}

/// Render photos into a 2×3 JPEG, cropping each photo to fill its cell.
pub fn render_collage_jpeg(
    paths: &[PathBuf],
    layout_id: &str,
    borders: bool,
    frames: &[PhotoFrame],
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    if !(2..=4).contains(&paths.len()) {
        bail!("a collage needs 2–4 photos, got {}", paths.len());
    }
    if width == 0 || height == 0 {
        bail!("collage dimensions must be non-zero");
    }
    let layouts = layouts_for_borders(paths.len(), borders);
    let layout = layouts
        .iter()
        .find(|layout| layout.id == layout_id)
        .with_context(|| format!("unknown collage layout '{layout_id}'"))?;

    let mut canvas = RgbImage::from_pixel(width, height, Rgb([255, 255, 255]));
    for (index, (path, cell)) in paths.iter().zip(&layout.cells).enumerate() {
        let source = image_prep::load_oriented(path)
            .with_context(|| format!("could not load {}", path.display()))?
            .to_rgb8();
        let frame = frames.get(index).copied().unwrap_or_default();
        let mut source = match frame.rotation % 360 {
            90 => image::imageops::rotate90(&source),
            180 => image::imageops::rotate180(&source),
            270 => image::imageops::rotate270(&source),
            _ => source,
        };
        let (source_width, source_height) = source.dimensions();
        let x = (cell.x * width as f32).round() as i64;
        let y = (cell.y * height as f32).round() as i64;
        let cell_width = ((cell.w * width as f32).round() as u32).max(1);
        let cell_height = ((cell.h * height as f32).round() as u32).max(1);
        let cell_width = cell_width.min(width.saturating_sub(x.max(0) as u32));
        let cell_height = cell_height.min(height.saturating_sub(y.max(0) as u32));
        if cell_width == 0 || cell_height == 0 {
            continue;
        }

        let (crop_x, crop_y, crop_width, crop_height) =
            source_crop(source_width, source_height, cell_width, cell_height, frame);
        let cropped =
            image::imageops::crop(&mut source, crop_x, crop_y, crop_width, crop_height).to_image();
        let resized =
            image::imageops::resize(&cropped, cell_width, cell_height, FilterType::Lanczos3);
        image::imageops::overlay(&mut canvas, &resized, x, y);
    }

    image_prep::encode_jpeg(&canvas)
}

fn source_crop(
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
    frame: PhotoFrame,
) -> (u32, u32, u32, u32) {
    let target_width_f = target_width as f32;
    let target_height_f = target_height as f32;
    let base_scale = f32::max(
        target_width_f / source_width as f32,
        target_height_f / source_height as f32,
    );
    let scale = base_scale * frame.zoom.clamp(1.0, 8.0);
    let draw_width = source_width as f32 * scale;
    let draw_height = source_height as f32 * scale;
    let slack_x = (draw_width - target_width_f).max(0.0);
    let slack_y = (draw_height - target_height_f).max(0.0);
    let left = (target_width_f - draw_width) / 2.0 + frame.pan_x.clamp(-1.0, 1.0) * slack_x / 2.0;
    let top = (target_height_f - draw_height) / 2.0 + frame.pan_y.clamp(-1.0, 1.0) * slack_y / 2.0;
    let crop_width = (target_width_f / scale)
        .clamp(1.0, source_width as f32)
        .round() as u32;
    let crop_height = (target_height_f / scale)
        .clamp(1.0, source_height as f32)
        .round() as u32;
    let crop_width = crop_width.min(source_width);
    let crop_height = crop_height.min(source_height);
    let x = ((-left / scale).max(0.0) as u32).min(source_width - crop_width);
    let y = ((-top / scale).max(0.0) as u32).min(source_height - crop_height);
    (x, y, crop_width, crop_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_photo_counts() {
        for count in 2..=4 {
            let layouts = layouts_for(count);
            assert!(!layouts.is_empty());
            for layout in layouts {
                assert_eq!(layout.cells.len(), count);
            }
        }
    }

    #[test]
    fn cells_stay_inside_the_canvas() {
        for count in 2..=4 {
            for layout in layouts_for(count) {
                for cell in layout.cells {
                    assert!(cell.x >= 0.0 && cell.y >= 0.0);
                    assert!(cell.x + cell.w <= 1.0 + f32::EPSILON);
                    assert!(cell.y + cell.h <= 1.0 + f32::EPSILON);
                }
            }
        }
    }

    #[test]
    fn borderless_layouts_fill_the_paper() {
        let cells = layouts_for_borders(2, false).first().unwrap().cells.clone();
        assert!((cells[0].x - 0.0).abs() < f32::EPSILON);
        assert!((cells[0].y - 0.0).abs() < f32::EPSILON);
        assert!((cells[0].w - 1.0).abs() < f32::EPSILON);
        assert!((cells[0].h - 0.5).abs() < f32::EPSILON);
        assert!((cells[1].y - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn renders_a_print_sized_jpeg() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("liene-collage-test-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let colors = [[220, 40, 40], [40, 200, 80], [40, 80, 220]];
        let mut paths = Vec::new();
        for (index, color) in colors.iter().enumerate() {
            let path = dir.join(format!("{index}.png"));
            RgbImage::from_pixel(240, 240, Rgb(*color))
                .save(&path)
                .unwrap();
            paths.push(path);
        }

        let jpeg = render_collage_jpeg(&paths, "h3", true, &[], 1200, 1800).unwrap();
        assert!(jpeg.len() <= 1023 * 1024);
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(decoded.width(), 1200);
        assert_eq!(decoded.height(), 1800);

        std::fs::remove_dir_all(&dir).ok();
    }
}
