//! Integral-image ink detection, morphological dilation (`lv-kernels`), text row bands, and skew estimation.

use image::{GrayImage, Rgb, RgbImage};
use lv_kernels::morph_chroma::{dilate_binary, separate_chroma_rgb, CHROMA_MIN};

/// Pixel bounding box `(x0, y0, x1, y1)`.
pub type Bbox = (u32, u32, u32, u32);

const SHADE_GAP: u8 = 20;
const BLOCK_GAP: u32 = 36;
const RUN_GAP: u32 = 10;
const MIN_GLYPH_PX: u32 = 4;
const MERGE_GAP_PX: u32 = 4;
const MIN_BAND_PX: u32 = 4;
const MAX_UPSCALE: f32 = 3.0;
const CROP_MAX_SIDE: u32 = 2016;
const SKEW_RANGE: f32 = 5.0;
const SKEW_STEP: f32 = 0.25;

/// Binary ink and chromatic masks with O(1) prefix-sum rectangle queries.
#[derive(Debug, Clone, PartialEq)]
pub struct InkPage {
    pub width: u32,
    pub height: u32,
    pub mask: Vec<u8>,
    pub color: Vec<u8>,
    integral: Vec<u32>,
}

/// Horizontal text run segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub bbox: Bbox,
    pub paragraph: bool,
}

impl InkPage {
    pub fn new(gray: &GrayImage) -> Self {
        let (width, height) = gray.dimensions();
        let w = width as usize;
        let h = height as usize;
        let cutoff = paper_level(gray).saturating_sub(SHADE_GAP);
        let raw = gray.as_raw();
        let mask: Vec<u8> = raw.iter().map(|&v| u8::from(v < cutoff)).collect();
        let mut integral = vec![0u32; (w + 1) * (h + 1)];
        let stride = w + 1;
        for y in 0..h {
            let mut row_acc = 0u32;
            for x in 0..w {
                row_acc += mask[y * w + x] as u32;
                integral[(y + 1) * stride + (x + 1)] = integral[y * stride + (x + 1)] + row_acc;
            }
        }
        Self { width, height, mask, color: Vec::new(), integral }
    }

    /// Attaches RGB chromatic pixel mask (`max(R,G,B) - min(R,G,B) >= CHROMA_MIN`).
    pub fn with_color(mut self, rgb: &RgbImage) -> Self {
        let (rw, rh) = rgb.dimensions();
        if rw == self.width && rh == self.height {
            self.color = separate_chroma_rgb(rgb.as_raw(), rw as usize, rh as usize, CHROMA_MIN);
        }
        self
    }

    pub fn color(&self) -> &[u8] {
        &self.color
    }

    pub fn dilate(&self, radius: u32) -> Vec<u8> {
        dilate_binary(&self.mask, self.width as usize, self.height as usize, radius)
    }

    pub fn ink_in(&self, b: Bbox) -> u32 {
        let (x0, y0, x1, y1) = (
            b.0.min(self.width) as usize,
            b.1.min(self.height) as usize,
            b.2.min(self.width) as usize,
            b.3.min(self.height) as usize,
        );
        if x1 <= x0 || y1 <= y0 {
            return 0;
        }
        let s = self.width as usize + 1;
        self.integral[y1 * s + x1] + self.integral[y0 * s + x0]
            - self.integral[y1 * s + x0]
            - self.integral[y0 * s + x1]
    }

    pub fn row_counts(&self, b: Bbox) -> Vec<u32> {
        let (x0, y0, x1, y1) = (
            b.0.min(self.width),
            b.1.min(self.height),
            b.2.min(self.width),
            b.3.min(self.height),
        );
        (y0..y1).map(|y| self.ink_in((x0, y, x1, y + 1))).collect()
    }

    pub fn col_counts(&self, b: Bbox) -> Vec<u32> {
        let (x0, y0, x1, y1) = (
            b.0.min(self.width),
            b.1.min(self.height),
            b.2.min(self.width),
            b.3.min(self.height),
        );
        (x0..x1).map(|x| self.ink_in((x, y0, x + 1, y1))).collect()
    }

    pub fn tighten(&self, b: Bbox) -> Option<Bbox> {
        let (x0, y0, x1, y1) = (
            b.0.min(self.width),
            b.1.min(self.height),
            b.2.min(self.width),
            b.3.min(self.height),
        );
        if x1 <= x0 || y1 <= y0 || self.ink_in((x0, y0, x1, y1)) == 0 {
            return None;
        }
        let rows = self.row_counts((x0, y0, x1, y1));
        let cols = self.col_counts((x0, y0, x1, y1));
        let ry0 = rows.iter().position(|&c| c > 0)? as u32 + y0;
        let ry1 = rows.iter().rposition(|&c| c > 0)? as u32 + y0 + 1;
        let rx0 = cols.iter().position(|&c| c > 0)? as u32 + x0;
        let rx1 = cols.iter().rposition(|&c| c > 0)? as u32 + x0 + 1;
        Some((rx0, ry0, rx1, ry1))
    }

    /// Strips sparse outer border frames and returns inner glyph bounds if any exist.
    pub fn text_box(&self, b: Bbox) -> Option<Bbox> {
        let t = self.tighten(b)?;
        let rows = self.row_counts(t);
        let cols = self.col_counts(t);
        let w = (t.2 - t.0).max(1) as f32;
        let h = (t.3 - t.1).max(1) as f32;
        let r0 = rows.iter().position(|&c| (c as f32) < w * 0.75).unwrap_or(0) as u32;
        let r1 = rows.iter().rposition(|&c| (c as f32) < w * 0.75).unwrap_or(rows.len() - 1) as u32 + 1;
        let c0 = cols.iter().position(|&c| (c as f32) < h * 0.75).unwrap_or(0) as u32;
        let c1 = cols.iter().rposition(|&c| (c as f32) < h * 0.75).unwrap_or(cols.len() - 1) as u32 + 1;
        self.tighten((t.0 + c0, t.1 + r0, t.0 + c1, t.1 + r1))
    }

    pub fn glyphs(&self, b: Bbox, limit: usize) -> usize {
        let mut count = 0usize;
        for (ry0, ry1) in row_bands(self, b) {
            let cols = self.col_counts((b.0, ry0, b.2, ry1));
            let mut in_glyph = false;
            let mut width = 0u32;
            for &c in &cols {
                if c > 0 {
                    in_glyph = true;
                    width += 1;
                } else if in_glyph {
                    if width >= MIN_GLYPH_PX && width <= (ry1 - ry0) * 3 {
                        count += 1;
                        if count >= limit {
                            return count;
                        }
                    }
                    in_glyph = false;
                    width = 0;
                }
            }
            if in_glyph && width >= MIN_GLYPH_PX {
                count += 1;
            }
        }
        count
    }
}

pub fn paper_level(gray: &GrayImage) -> u8 {
    let mut hist = [0u32; 256];
    let step = ((gray.width() * gray.height()) / 16_384).max(1) as usize;
    for &p in gray.as_raw().iter().step_by(step) {
        hist[p as usize] += 1;
    }
    let total: u32 = hist.iter().sum();
    let target = (total as f32 * 0.85) as u32;
    let mut acc = 0u32;
    for (v, &c) in hist.iter().enumerate() {
        acc += c;
        if acc >= target {
            return (v as u8).max(128);
        }
    }
    250
}

pub fn blocks(ink: &InkPage) -> Vec<Bbox> {
    let mut out = Vec::new();
    let full = (0, 0, ink.width, ink.height);
    let r_counts = ink.row_counts(full);
    let mut ry0 = None;
    let mut gap = 0u32;
    for (y, &c) in r_counts.iter().enumerate() {
        if c > 0 {
            if ry0.is_none() {
                ry0 = Some(y as u32);
            }
            gap = 0;
        } else if let Some(s) = ry0 {
            gap += 1;
            if gap > BLOCK_GAP {
                if let Some(t) = ink.tighten((0, s, ink.width, y as u32 - gap + 1)) {
                    out.push(t);
                }
                ry0 = None;
            }
        }
    }
    if let Some(s) = ry0 {
        if let Some(t) = ink.tighten((0, s, ink.width, ink.height)) {
            out.push(t);
        }
    }
    out
}

pub fn runs(ink: &InkPage, block: Bbox) -> Vec<Run> {
    let rb = row_bands(ink, block);
    if rb.is_empty() {
        return vec![Run { bbox: block, paragraph: false }];
    }
    let mut groups: Vec<Vec<(u32, u32)>> = vec![vec![rb[0]]];
    for &band in &rb[1..] {
        let prev = groups.last().unwrap().last().unwrap().1;
        if band.0.saturating_sub(prev) <= RUN_GAP {
            groups.last_mut().unwrap().push(band);
        } else {
            groups.push(vec![band]);
        }
    }
    groups
        .into_iter()
        .filter_map(|g| {
            let bbox = ink.tighten((block.0, g.first()?.0, block.2, g.last()?.1))?;
            Some(Run { bbox, paragraph: g.len() >= 2 })
        })
        .collect()
}

pub fn tiles(_ink: &InkPage, bbox: Bbox, scale: f32, max_patches: u32) -> Vec<Bbox> {
    let (x0, y0, x1, y1) = bbox;
    let w = (x1.saturating_sub(x0)) as f32 * scale;
    let h = (y1.saturating_sub(y0)) as f32 * scale;
    let patches = (w * h) / (14.0 * 14.0);
    if patches <= max_patches.max(1) as f32 {
        return vec![bbox];
    }
    let n = (patches / max_patches.max(1) as f32).ceil() as u32;
    let step = ((y1 - y0) / n.max(1)).max(16);
    let mut out = Vec::new();
    let mut y = y0;
    while y < y1 {
        let ye = (y + step + 8).min(y1);
        out.push((x0, y, x1, ye));
        if ye == y1 {
            break;
        }
        y = ye.saturating_sub(8);
    }
    out
}

fn bands(counts: &[u32], offset: u32) -> Vec<(u32, u32)> {
    let mut runs = Vec::new();
    let mut start = None;
    for (i, &c) in counts.iter().enumerate() {
        if c > 0 {
            start.get_or_insert(i as u32);
        } else if let Some(s) = start.take() {
            runs.push((s, i as u32));
        }
    }
    if let Some(s) = start {
        runs.push((s, counts.len() as u32));
    }
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (s, e) in runs {
        match merged.last_mut() {
            Some(last) if s.saturating_sub(last.1) <= MERGE_GAP_PX => last.1 = e,
            _ => merged.push((s, e)),
        }
    }
    merged.retain(|(s, e)| e - s >= MIN_BAND_PX);
    merged.into_iter().map(|(s, e)| (offset + s, offset + e)).collect()
}

pub fn row_bands(ink: &InkPage, bbox: Bbox) -> Vec<(u32, u32)> {
    if bbox.2 <= bbox.0 || bbox.3 <= bbox.1 {
        return Vec::new();
    }
    bands(&ink.row_counts(bbox), bbox.1)
}

pub fn text_height(ink: &InkPage, bbox: Bbox) -> Option<f32> {
    let mut h: Vec<f32> = row_bands(ink, bbox).iter().map(|(s, e)| (e - s) as f32).collect();
    if h.is_empty() {
        return None;
    }
    h.sort_by(f32::total_cmp);
    Some(if h.len() >= 4 { h[h.len() / 4] } else { h[0] })
}

pub fn upscale_factor(text_h: Option<f32>, target: f32, width: u32, height: u32) -> f32 {
    let want = text_h.map_or(1.0, |h| (target / h.max(1.0)).clamp(1.0, MAX_UPSCALE));
    let cap = CROP_MAX_SIDE as f32 / width.max(height).max(1) as f32;
    want.min(cap.max(1.0))
}

pub fn estimate_skew(gray: &GrayImage) -> f32 {
    const COL_STEP: usize = 3;
    const BIN: f32 = 2.0;
    let ink = InkPage::new(gray);
    let (w, h) = (ink.width as usize, ink.height as usize);
    let mut points = Vec::new();
    for y in 0..h {
        for x in (0..w).step_by(COL_STEP) {
            if ink.mask[y * w + x] != 0 {
                points.push((x as f32 - w as f32 / 2.0, y as f32 - h as f32 / 2.0));
            }
        }
    }
    if points.len() < 64 {
        return 0.0;
    }
    let rows = (h + w / 8) / BIN as usize + 2;
    let contrast = |deg: f32| -> f32 {
        let tan = deg.to_radians().tan();
        let mut hist = vec![0f32; rows];
        for &(x, y) in &points {
            let row = (y - x * tan) / BIN + rows as f32 / 2.0;
            if row >= 0.0 && (row as usize) < rows {
                hist[row as usize] += 1.0;
            }
        }
        hist.iter().map(|v| v * v).sum()
    };
    let n = (SKEW_RANGE / SKEW_STEP).round() as i32;
    let (mut best, mut at) = (f32::MIN, 0.0);
    for i in -n..=n {
        let deg = i as f32 * SKEW_STEP;
        let c = contrast(deg);
        if c > best {
            best = c;
            at = deg;
        }
    }
    at
}

pub fn deskew(img: &RgbImage, deg: f32) -> RgbImage {
    if deg == 0.0 {
        return img.clone();
    }
    let (w, h) = img.dimensions();
    let (sin, cos) = deg.to_radians().sin_cos();
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let sample = |x: f32, y: f32| -> Rgb<u8> {
        if x < 0.0 || y < 0.0 || x >= (w - 1) as f32 || y >= (h - 1) as f32 {
            return Rgb([255, 255, 255]);
        }
        let (x0, y0) = (x as u32, y as u32);
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let px = |xx: u32, yy: u32, c: usize| img.get_pixel(xx, yy)[c] as f32;
        let mut out = [0u8; 3];
        for (c, o) in out.iter_mut().enumerate() {
            let top = px(x0, y0, c) * (1.0 - fx) + px(x0 + 1, y0, c) * fx;
            let bot = px(x0, y0 + 1, c) * (1.0 - fx) + px(x0 + 1, y0 + 1, c) * fx;
            *o = (top * (1.0 - fy) + bot * fy + 0.5) as u8;
        }
        Rgb(out)
    };
    RgbImage::from_fn(w, h, |x, y| {
        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        sample(cx + dx * cos - dy * sin - 0.5, cy + dx * sin + dy * cos - 0.5)
    })
}

pub fn extract_ink_page(gray: &image::GrayImage) -> InkPage {
    InkPage::new(gray)
}

/// Estimates skew angle (`±5.0°` in `0.25°` steps), deskews `rgb`, and returns aligned RGB, `InkPage` (with chroma), and skew angle.
pub fn straight_ink(rgb: &RgbImage) -> (RgbImage, InkPage, f32) {
    let gray = image::imageops::grayscale(rgb);
    let deg = estimate_skew(&gray);
    let aligned = if deg.abs() > 0.05 { deskew(rgb, deg) } else { rgb.clone() };
    let aligned_gray = image::imageops::grayscale(&aligned);
    let ink = InkPage::new(&aligned_gray).with_color(&aligned);
    (aligned, ink, deg)
}

/// Maps a bounding box from deskewed pixel coordinates back to the original image frame (`deg` inverted).
pub fn unrotate_bbox(b: Bbox, w: u32, h: u32, deg: f32) -> Bbox {
    if deg.abs() <= 0.05 || w == 0 || h == 0 {
        return b;
    }
    let rad = (-deg).to_radians();
    let (s, c) = rad.sin_cos();
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let pts = [
        (b.0 as f32, b.1 as f32),
        (b.2 as f32, b.1 as f32),
        (b.2 as f32, b.3 as f32),
        (b.0 as f32, b.3 as f32),
    ];
    let (mut x0, mut y0, mut x1, mut y1) = (w as f32, h as f32, 0.0f32, 0.0f32);
    for (px, py) in pts {
        let dx = px - cx;
        let dy = py - cy;
        let ox = (c * dx - s * dy + cx).clamp(0.0, w as f32);
        let oy = (s * dx + c * dy + cy).clamp(0.0, h as f32);
        x0 = x0.min(ox);
        y0 = y0.min(oy);
        x1 = x1.max(ox);
        y1 = y1.max(oy);
    }
    (x0.floor() as u32, y0.floor() as u32, (x1.ceil() as u32).min(w), (y1.ceil() as u32).min(h))
}
