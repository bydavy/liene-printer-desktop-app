//! Collage layouts and rendering for 2–4 photos on a 2×3 canvas.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use image::imageops::FilterType;
use image::{Rgb, RgbImage};

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

#[derive(Debug, Clone)]
pub struct CollageLayout {
    pub id: &'static str,
    pub name: &'static str,
    pub cells: Vec<CollageCell>,
}

pub fn layouts_for(count: usize) -> Vec<CollageLayout> {
    match count {
        2 => vec![
            layout("v2", "Vertical", &[1.0, 1.0], &[1.0]),
            layout("h2", "Horizontal", &[1.0], &[1.0, 1.0]),
            layout("top2", "Top focus", &[0.62, 0.38], &[1.0]),
            layout("left2", "Left focus", &[1.0], &[0.62, 0.38]),
        ],
        3 => vec![
            layout("v3", "Vertical", &[1.0, 1.0, 1.0], &[1.0]),
            layout("h3", "Horizontal", &[1.0], &[1.0, 1.0, 1.0]),
            hero("left3", "Left hero", true),
            hero("top3", "Top hero", false),
        ],
        4 => vec![
            layout("grid4", "2×2", &[1.0, 1.0], &[1.0, 1.0]),
            layout("v4", "4 down", &[1.0, 1.0, 1.0, 1.0], &[1.0]),
            layout("h4", "4 across", &[1.0], &[1.0, 1.0, 1.0, 1.0]),
            hero_grid("left4", "Left hero", 3),
        ],
        _ => Vec::new(),
    }
}

fn layout(id: &'static str, name: &'static str, rows: &[f32], cols: &[f32]) -> CollageLayout {
    CollageLayout {
        id,
        name,
        cells: grid(rows, cols),
    }
}

fn grid(rows: &[f32], cols: &[f32]) -> Vec<CollageCell> {
    let content_w = 1.0 - 2.0 * MARGIN - GAP * (cols.len() as f32 - 1.0);
    let content_h = 1.0 - 2.0 * MARGIN - GAP * (rows.len() as f32 - 1.0);
    let row_sum: f32 = rows.iter().sum();
    let col_sum: f32 = cols.iter().sum();
    let heights: Vec<f32> = rows.iter().map(|r| content_h * r / row_sum).collect();
    let widths: Vec<f32> = cols.iter().map(|c| content_w * c / col_sum).collect();

    let mut cells = Vec::with_capacity(rows.len() * cols.len());
    let mut y = MARGIN;
    for height in heights {
        let mut x = MARGIN;
        for width in &widths {
            cells.push(CollageCell {
                x,
                y,
                w: *width,
                h: height,
            });
            x += width + GAP;
        }
        y += height + GAP;
    }
    cells
}

fn hero(id: &'static str, name: &'static str, left: bool) -> CollageLayout {
    let content_w = 1.0 - 2.0 * MARGIN;
    let content_h = 1.0 - 2.0 * MARGIN;
    let cells = if left {
        let split = MARGIN + (content_w - GAP) * 0.62;
        let small_h = (content_h - GAP) / 2.0;
        vec![
            CollageCell {
                x: MARGIN,
                y: MARGIN,
                w: split - MARGIN,
                h: content_h,
            },
            CollageCell {
                x: split + GAP,
                y: MARGIN,
                w: 1.0 - MARGIN - split - GAP,
                h: small_h,
            },
            CollageCell {
                x: split + GAP,
                y: MARGIN + small_h + GAP,
                w: 1.0 - MARGIN - split - GAP,
                h: small_h,
            },
        ]
    } else {
        let split = MARGIN + (content_h - GAP) * 0.62;
        let small_w = (content_w - GAP) / 2.0;
        vec![
            CollageCell {
                x: MARGIN,
                y: MARGIN,
                w: content_w,
                h: split - MARGIN,
            },
            CollageCell {
                x: MARGIN,
                y: split + GAP,
                w: small_w,
                h: 1.0 - MARGIN - split - GAP,
            },
            CollageCell {
                x: MARGIN + small_w + GAP,
                y: split + GAP,
                w: small_w,
                h: 1.0 - MARGIN - split - GAP,
            },
        ]
    };
    CollageLayout { id, name, cells }
}

fn hero_grid(id: &'static str, name: &'static str, rows: usize) -> CollageLayout {
    let content_w = 1.0 - 2.0 * MARGIN;
    let content_h = 1.0 - 2.0 * MARGIN;
    let split = MARGIN + (content_w - GAP) / 2.0;
    let mut cells = vec![CollageCell {
        x: MARGIN,
        y: MARGIN,
        w: split - MARGIN,
        h: content_h,
    }];
    let small_h = (content_h - GAP * (rows as f32 - 1.0)) / rows as f32;
    for index in 0..rows {
        let y = MARGIN + index as f32 * (small_h + GAP);
        cells.push(CollageCell {
            x: split + GAP,
            y,
            w: 1.0 - MARGIN - split - GAP,
            h: small_h,
        });
    }
    CollageLayout { id, name, cells }
}

/// Render photos into a 2×3 JPEG, cropping each photo to fill its cell.
pub fn render_collage_jpeg(
    paths: &[PathBuf],
    layout_id: &str,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    if !(2..=4).contains(&paths.len()) {
        bail!("a collage needs 2–4 photos, got {}", paths.len());
    }
    if width == 0 || height == 0 {
        bail!("collage dimensions must be non-zero");
    }
    let layouts = layouts_for(paths.len());
    let layout = layouts
        .iter()
        .find(|layout| layout.id == layout_id)
        .with_context(|| format!("unknown collage layout '{layout_id}'"))?;

    let mut canvas = RgbImage::from_pixel(width, height, Rgb([255, 255, 255]));
    for (path, cell) in paths.iter().zip(&layout.cells) {
        let source = image_prep::load_oriented(path)
            .with_context(|| format!("could not load {}", path.display()))?
            .to_rgb8();
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

        let scale = f64::max(
            f64::from(cell_width) / f64::from(source_width),
            f64::from(cell_height) / f64::from(source_height),
        );
        let resized_width = (f64::from(source_width) * scale).round().max(1.0) as u32;
        let resized_height = (f64::from(source_height) * scale).round().max(1.0) as u32;
        let mut resized =
            image::imageops::resize(&source, resized_width, resized_height, FilterType::Lanczos3);
        let crop_x = (resized_width - cell_width) / 2;
        let crop_y = (resized_height - cell_height) / 2;
        let cropped =
            image::imageops::crop(&mut resized, crop_x, crop_y, cell_width, cell_height).to_image();
        image::imageops::overlay(&mut canvas, &cropped, x, y);
    }

    image_prep::encode_jpeg(&canvas)
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

        let jpeg = render_collage_jpeg(&paths, "h3", 1200, 1800).unwrap();
        assert!(jpeg.len() <= 1023 * 1024);
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(decoded.width(), 1200);
        assert_eq!(decoded.height(), 1800);

        std::fs::remove_dir_all(&dir).ok();
    }
}
