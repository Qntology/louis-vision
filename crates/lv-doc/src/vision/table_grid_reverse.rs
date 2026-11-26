//! 3-Way crop preprocessing:
//! 1. Single text crops: `dilate(2)` pen-stroke protection + `Luma = 215` background grid attenuation.
//! 2. Solid bordered tables: preserve original rules + 25% row overlap splitting.
//! 3. Borderless multi-column tables (`row_bands >= 2 && col_clusters >= 4`): inject 1px `Luma = 180` virtual gridlines in ink valleys.

use image::{DynamicImage, GrayImage, Luma, RgbImage};
use lv_kernels::morph_chroma::dilate_binary;

use super::ink::Bbox;

/// Background pipe/rule attenuation luminance for single-text blueprint/form crops.
pub const PIPE_ATTENUATE_LUMA: u8 = 215;
/// Virtual 1px gridline luminance for borderless multi-column tables.
pub const VIRTUAL_GRIDLINE_LUMA: u8 = 180;
/// Solid bordered table horizontal strip overlap fraction (25%).
pub const SOLID_TABLE_OVERLAP: f32 = 0.25;
pub const TABLE_BAND_MIN_COLS: usize = 4;
pub const ROW_TILE_MIN_BANDS: usize = 2;

const ROW_BAND_NOISE_SIGMA: f32 = 2.0;
const ROW_BAND_MERGE_GAP: u32 = 2;
const ROW_BAND_MIN_H: u32 = 4;
const COL_CLUSTER_GAP: u32 = 8;
const COL_CLUSTER_MIN_W: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropPreprocessMode {
    SingleText,
    SolidTable,
    BorderlessTable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowBand {
    pub y0: u32,
    pub y1: u32,
    pub col_clusters: usize,
    pub col_valleys: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlapTile {
    pub bbox: Bbox,
    pub index: usize,
    pub total: usize,
}

/// Detects horizontal text row bands and column clusters inside `bbox`.
pub fn text_row_bands(img: &DynamicImage, bbox: Bbox) -> Vec<RowBand> {
    let (x0, y0, x1, y1) = bbox;
    let w = x1.saturating_sub(x0).max(1);
    let h = y1.saturating_sub(y0).max(1);
    if h < ROW_BAND_MIN_H * 2 || img.width() == 0 || img.height() == 0 {
        return Vec::new();
    }
    let g = img.crop_imm(x0, y0, w.min(img.width() - x0), h.min(img.height() - y0)).to_luma8();
    let (gw, gh) = g.dimensions();
    let prof: Vec<f32> = (0..gh)
        .map(|y| (0..gw).map(|x| 255.0 - g.get_pixel(x, y)[0] as f32).sum::<f32>() / gw as f32)
        .collect();
    let mut sorted = prof.clone();
    sorted.sort_by(f32::total_cmp);
    let floor = sorted[sorted.len() / 10];
    let peak = sorted[sorted.len() * 9 / 10];
    if peak - floor < 1.0 {
        return Vec::new();
    }
    let span_gate = floor + (peak - floor) * 0.25;
    let half = &sorted[..(sorted.len() / 2).max(1)];
    let hm: f32 = half.iter().sum::<f32>() / half.len() as f32;
    let hsd: f32 = (half.iter().map(|v| (v - hm) * (v - hm)).sum::<f32>() / half.len() as f32).sqrt();
    let gate = (hm + hsd * ROW_BAND_NOISE_SIGMA).min(span_gate);

    let mut runs = Vec::new();
    let mut start = None;
    for y in 0..gh {
        if prof[y as usize] > gate {
            start.get_or_insert(y);
        } else if let Some(s) = start.take() {
            runs.push((s, y));
        }
    }
    if let Some(s) = start {
        runs.push((s, gh));
    }
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (bs, be) in runs {
        match merged.last_mut() {
            Some(last) if bs.saturating_sub(last.1) <= ROW_BAND_MERGE_GAP => last.1 = be,
            _ => merged.push((bs, be)),
        }
    }
    merged.retain(|(bs, be)| be.saturating_sub(*bs) >= ROW_BAND_MIN_H);

    merged
        .into_iter()
        .map(|(bs, be)| {
            let span = (be - bs).max(1) as f32;
            let colp: Vec<f32> = (0..gw)
                .map(|x| (bs..be).map(|y| 255.0 - g.get_pixel(x, y)[0] as f32).sum::<f32>() / span)
                .collect();
            let mut cs = colp.clone();
            cs.sort_by(f32::total_cmp);
            let cfloor = cs[cs.len() / 10];
            let cpeak = cs[cs.len() * 9 / 10];
            let cgate = cfloor + (cpeak - cfloor).max(1.0) * 0.30;
            let mut cruns = Vec::new();
            let mut cst = None;
            for x in 0..gw {
                if colp[x as usize] > cgate {
                    cst.get_or_insert(x);
                } else if let Some(s) = cst.take() {
                    cruns.push((s, x));
                }
            }
            if let Some(s) = cst {
                cruns.push((s, gw));
            }
            let mut cmerged: Vec<(u32, u32)> = Vec::new();
            for (s, e) in cruns {
                match cmerged.last_mut() {
                    Some(last) if s.saturating_sub(last.1) <= COL_CLUSTER_GAP => last.1 = e,
                    _ => cmerged.push((s, e)),
                }
            }
            let clusters = cmerged.iter().filter(|(s, e)| e.saturating_sub(*s) >= COL_CLUSTER_MIN_W).count();
            RowBand { y0: y0 + bs, y1: y0 + be, col_clusters: clusters, col_valleys: clusters.saturating_sub(1) }
        })
        .collect()
}

/// Computes borderless table evidence `(table_rows, total_rows, min_cols, is_borderless_table)`.
pub fn table_row_evidence(img: &DynamicImage, bbox: Bbox) -> (usize, usize, usize, bool) {
    let bands = text_row_bands(img, bbox);
    let table_rows = bands.iter().filter(|b| b.col_clusters >= TABLE_BAND_MIN_COLS).count();
    (table_rows, bands.len(), TABLE_BAND_MIN_COLS, table_rows >= ROW_TILE_MIN_BANDS)
}

/// 1. Single-text crop: protects pixels within `dilate(2)` of new pen strokes and attenuates light/background pipes to `Luma = 215`.
pub fn preprocess_single_text_crop(rgb: &RgbImage, new_pen_mask: &[u8]) -> GrayImage {
    let (w, h) = rgb.dimensions();
    assert_eq!(new_pen_mask.len(), (w * h) as usize);
    let protected = dilate_binary(new_pen_mask, w as usize, h as usize, 2);
    let mut out = image::imageops::grayscale(rgb);
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let p = out.get_pixel(x, y)[0];
            if protected[idx] == 0 && p >= 96 && p < PIPE_ATTENUATE_LUMA {
                out.put_pixel(x, y, Luma([PIPE_ATTENUATE_LUMA]));
            }
        }
    }
    out
}

/// 2. Solid-bordered table: splits vertically with 25% overlap preserving full original border rules.
pub fn plan_solid_table_overlap(bbox: Bbox, tile_count: usize) -> Vec<OverlapTile> {
    let (x0, y0, x1, y1) = bbox;
    if tile_count <= 1 || y1 <= y0 {
        return vec![OverlapTile { bbox, index: 0, total: 1 }];
    }
    let h = (y1 - y0) as f32;
    let n = tile_count as f32;
    let denom = n - (n - 1.0) * SOLID_TABLE_OVERLAP;
    if denom <= 0.0 {
        return vec![OverlapTile { bbox, index: 0, total: 1 }];
    }
    let tile_h = h / denom;
    let step = tile_h * (1.0 - SOLID_TABLE_OVERLAP);
    let mut out = Vec::with_capacity(tile_count);
    for i in 0..tile_count {
        let ty0 = y0 as f32 + step * i as f32;
        let ty1 = (ty0 + tile_h).min(y1 as f32);
        if ty1 > ty0 + 1.0 {
            out.push(OverlapTile { bbox: (x0, ty0 as u32, x1, ty1 as u32), index: i, total: tile_count });
        }
    }
    let total = out.len().max(1);
    for o in &mut out {
        o.total = total;
    }
    out
}

/// Finds column whitespace valleys inside a grayscale borderless table crop.
pub fn detect_column_valleys(gray: &GrayImage) -> Vec<u32> {
    let (w, h) = gray.dimensions();
    if w < 16 || h < 8 {
        return Vec::new();
    }
    let col_ink: Vec<u32> = (0..w)
        .map(|x| (0..h).filter(|&y| gray.get_pixel(x, y)[0] < 200).count() as u32)
        .collect();
    let mut valleys = Vec::new();
    let mut gap_start: Option<u32> = None;
    for (x, &ink) in col_ink.iter().enumerate() {
        if ink == 0 {
            gap_start.get_or_insert(x as u32);
        } else if let Some(s) = gap_start.take() {
            if s > 2 && (x as u32) < w - 2 && x as u32 - s >= 3 {
                valleys.push((s + x as u32) / 2);
            }
        }
    }
    valleys
}

/// 3. Borderless multi-column table (`row_bands >= 2 && col_clusters >= 4`):
/// injects 1px horizontal and vertical virtual gridlines at `Luma = 180` inside whitespace valleys.
pub fn inject_virtual_gridlines(gray: &GrayImage, min_row_bands: usize) -> Option<(GrayImage, Vec<u32>, Vec<u32>)> {
    let (w, h) = gray.dimensions();
    let dyn_img = DynamicImage::ImageLuma8(gray.clone());
    let (table_rows, _, _, is_table) = table_row_evidence(&dyn_img, (0, 0, w, h));
    if !is_table || table_rows < min_row_bands.max(ROW_TILE_MIN_BANDS) {
        return None;
    }
    let valleys_x = detect_column_valleys(gray);
    let bands = text_row_bands(&dyn_img, (0, 0, w, h));
    let mut valleys_y = Vec::new();
    for pair in bands.windows(2) {
        let mid_y = (pair[0].y1 + pair[1].y0) / 2;
        if mid_y < h {
            valleys_y.push(mid_y);
        }
    }
    let mut out = gray.clone();
    for &vx in &valleys_x {
        for y in 0..h {
            out.put_pixel(vx, y, Luma([VIRTUAL_GRIDLINE_LUMA]));
        }
    }
    for &vy in &valleys_y {
        for x in 0..w {
            out.put_pixel(x, vy, Luma([VIRTUAL_GRIDLINE_LUMA]));
        }
    }
    Some((out, valleys_x, valleys_y))
}

/// Runs `straight_ink` deskew alignment on an RGB scan prior to injecting `Luma = 180` virtual gridlines.
pub fn straight_borderless_grid(
    rgb: &image::RgbImage,
    min_row_bands: usize,
) -> Option<(GrayImage, Vec<u32>, Vec<u32>, f32)> {
    let (aligned_rgb, _ink, deg) = super::ink::straight_ink(rgb);
    let gray = image::imageops::grayscale(&aligned_rgb);
    let (out, vx, vy) = inject_virtual_gridlines(&gray, min_row_bands)?;
    Some((out, vx, vy, deg))
}
