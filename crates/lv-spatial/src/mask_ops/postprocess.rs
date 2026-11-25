//! Mask postprocessing matching Sammie-Roto-2 v2.5.0 / OpenCV 4.14:
//! `apply` runs holes, dots, border fix, and grow/shrink in that order.

use image::GrayImage;
use imageproc::contours::{find_contours, BorderType};
use imageproc::point::Point;

const FOREGROUND: u8 = 255;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PostprocessParams {
    pub holes: u32,
    pub dots: u32,
    pub border_fix: u32,
    pub grow: i32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PostprocessError {
    #[error("border_fix {border} does not fit in a {width}x{height} mask")]
    BorderTooLarge { border: u32, width: u32, height: u32 },
}

pub fn apply(mask: &GrayImage, params: &PostprocessParams) -> Result<GrayImage, PostprocessError> {
    let mut out = mask.clone();
    if params.holes > 0 {
        out = fill_small_holes(&out, params.holes);
    }
    if params.dots > 0 {
        out = remove_small_dots(&out, params.dots);
    }
    if params.border_fix > 0 {
        out = apply_border_fix(&out, params.border_fix)?;
    }
    if params.grow != 0 {
        out = grow_shrink(&out, params.grow);
    }
    Ok(out)
}

pub fn fill_small_holes(mask: &GrayImage, holes: u32) -> GrayImage {
    let max_area = f64::from(holes) * f64::from(holes);
    let mut filled = mask.clone();
    for contour in find_contours::<i32>(mask) {
        if contour.border_type == BorderType::Hole && polygon_area(&contour.points) <= max_area {
            fill_polygon(&mut filled, &contour.points);
        }
    }
    filled
}

pub fn remove_small_dots(mask: &GrayImage, dots: u32) -> GrayImage {
    let max_area = u64::from(dots) * u64::from(dots);
    let (width, height) = mask.dimensions();
    let (w, h) = (width as usize, height as usize);
    let raw = mask.as_raw();
    let mut visited = vec![false; w * h];
    let mut out = vec![0u8; w * h];
    let mut stack = Vec::new();
    let mut component = Vec::new();

    for start in 0..w * h {
        if raw[start] == 0 || visited[start] {
            continue;
        }
        visited[start] = true;
        stack.push(start);
        component.clear();
        while let Some(idx) = stack.pop() {
            component.push(idx);
            let (x, y) = (idx % w, idx / w);
            for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let n = ny * w + nx;
                    if raw[n] != 0 && !visited[n] {
                        visited[n] = true;
                        stack.push(n);
                    }
                }
            }
        }
        if component.len() as u64 > max_area {
            for &idx in &component {
                out[idx] = FOREGROUND;
            }
        }
    }
    GrayImage::from_raw(width, height, out).expect("buffer size matches dimensions")
}

pub fn apply_border_fix(mask: &GrayImage, border: u32) -> Result<GrayImage, PostprocessError> {
    if border == 0 {
        return Ok(mask.clone());
    }
    let (width, height) = mask.dimensions();
    if 2 * border >= width || 2 * border >= height {
        return Err(PostprocessError::BorderTooLarge { border, width, height });
    }
    let (b, w, h) = (border as usize, width as usize, height as usize);
    let raw = mask.as_raw();
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let sy = y.clamp(b, h - b - 1);
        for x in 0..w {
            let sx = x.clamp(b, w - b - 1);
            out[y * w + x] = raw[sy * w + sx];
        }
    }
    Ok(GrayImage::from_raw(width, height, out).expect("buffer size matches dimensions"))
}

pub fn grow_shrink(mask: &GrayImage, grow: i32) -> GrayImage {
    if grow == 0 {
        return mask.clone();
    }
    let size = grow.unsigned_abs() as i64 + 1;
    let anchor = size / 2;
    let dilate = grow > 0;
    let (width, height) = mask.dimensions();
    let (w, h) = (width as i64, height as i64);
    let raw = mask.as_raw();
    let mut out = vec![0u8; (w * h) as usize];

    for y in 0..h {
        for x in 0..w {
            let mut acc = if dilate { 0u8 } else { u8::MAX };
            for j in 0..size {
                let sy = y + j - anchor;
                if sy < 0 || sy >= h {
                    continue;
                }
                for i in 0..size {
                    let sx = x + i - anchor;
                    if sx < 0 || sx >= w {
                        continue;
                    }
                    let v = raw[(sy * w + sx) as usize];
                    acc = if dilate { acc.max(v) } else { acc.min(v) };
                }
            }
            out[(y * w + x) as usize] = acc;
        }
    }
    GrayImage::from_raw(width, height, out).expect("buffer size matches dimensions")
}

fn polygon_area(points: &[Point<i32>]) -> f64 {
    let n = points.len();
    if n < 3 {
        return 0.0;
    }
    let mut twice = 0i64;
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        twice += i64::from(a.x) * i64::from(b.y) - i64::from(b.x) * i64::from(a.y);
    }
    (twice.abs() as f64) / 2.0
}

fn fill_polygon(img: &mut GrayImage, points: &[Point<i32>]) {
    if points.len() < 3 {
        return;
    }
    let (width, height) = img.dimensions();
    let min_x = points.iter().map(|p| p.x).min().unwrap().max(0);
    let max_x = points.iter().map(|p| p.x).max().unwrap().min(width as i32 - 1);
    let min_y = points.iter().map(|p| p.y).min().unwrap().max(0);
    let max_y = points.iter().map(|p| p.y).max().unwrap().min(height as i32 - 1);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            if point_in_polygon(x, y, points) {
                img.put_pixel(x as u32, y as u32, image::Luma([FOREGROUND]));
            }
        }
    }
}

fn point_in_polygon(px: i32, py: i32, points: &[Point<i32>]) -> bool {
    let n = points.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (i64::from(points[i].x), i64::from(points[i].y));
        let (xj, yj) = (i64::from(points[j].x), i64::from(points[j].y));
        let (px, py) = (i64::from(px), i64::from(py));
        if (yi > py) != (yj > py) {
            let lhs = (px - xi) * (yj - yi);
            let rhs = (py - yi) * (xj - xi);
            let crosses = if yj - yi > 0 { lhs < rhs } else { lhs > rhs };
            if crosses {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}
