//! Page crop planning, TitleBlock protection invariant (`Baseline::Best` vs `Baseline::Each`), and crop NMS.

use super::ink::{blocks, runs, text_height, tiles, upscale_factor, Bbox, InkPage};

pub const MIN_GLYPHS: usize = 3;
const PAD: u32 = 8;
const CONTAINED: f32 = 0.85;
const TITLE_REGION: f32 = 0.35;
const MIN_CELL_MARGIN: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CropKind {
    Form,
    Table,
    Title,
    Sign,
}

/// Protection class separating engineering title blocks & signature corners from droppable body notices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectedRole {
    TitleBlock,
    SignatureCorner,
    Standard,
}

/// Evaluation mode: single-sheet scan (`Best`) enables full `TitleBlock` immunity;
/// A0 tiled scan (`Each`) disables per-tile protection and routes tiles via fast `LedgerBox`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Baseline {
    Best,
    Each,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CropPlan {
    pub kind: CropKind,
    pub bbox: Bbox,
    pub upscale: f32,
    pub guide_margin: Option<f32>,
}

impl CropPlan {
    pub fn patches(&self) -> u32 {
        let (w, h) = ((self.bbox.2 - self.bbox.0) as f32, (self.bbox.3 - self.bbox.1) as f32);
        (w * h * self.upscale * self.upscale / (14.0 * 14.0)) as u32
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PagePlan {
    pub width: u32,
    pub height: u32,
    pub crops: Vec<CropPlan>,
    pub dropped: Vec<Bbox>,
    pub blank: Vec<Bbox>,
    pub unfiltered: bool,
}

impl PagePlan {
    pub fn kept_area(&self) -> f32 {
        let total: u64 = self
            .crops
            .iter()
            .filter(|c| c.kind != CropKind::Sign)
            .map(|c| (c.bbox.2 - c.bbox.0) as u64 * (c.bbox.3 - c.bbox.1) as u64)
            .sum();
        total as f32 / (self.width as u64 * self.height as u64).max(1) as f32
    }
}

#[derive(Debug, Clone)]
pub struct PlanParams {
    pub text_px: f32,
    pub max_tile_patches: u32,
    pub drop_margin: f32,
    pub drop_share: f32,
    pub sign_gate: f32,
    pub use_arena: bool,
    pub protect: bool,
    pub baseline: Baseline,
}

impl Default for PlanParams {
    fn default() -> Self {
        Self {
            text_px: 28.0,
            max_tile_patches: 256,
            drop_margin: 0.08,
            drop_share: 0.60,
            sign_gate: 0.35,
            use_arena: true,
            protect: true,
            baseline: Baseline::Best,
        }
    }
}

/// Per-cell scores smoothed over 3x3 neighborhoods.
#[derive(Debug, Clone)]
pub struct Scores {
    pub rows: usize,
    pub cols: usize,
    pub form: Vec<f32>,
    pub sign: Vec<f32>,
    pub table: Vec<f32>,
    pub title: Vec<f32>,
    pub guide: Vec<f32>,
}

impl Scores {
    pub fn cells(&self, ink: &InkPage, (x0, y0, x1, y1): Bbox) -> Vec<usize> {
        let (w, h) = (ink.width as usize, ink.height as usize);
        if w == 0 || h == 0 || x1 <= x0 || y1 <= y0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for r in (y0 as usize * self.rows / h)..=((y1 as usize - 1) * self.rows / h).min(self.rows - 1) {
            for c in (x0 as usize * self.cols / w)..=((x1 as usize - 1) * self.cols / w).min(self.cols - 1) {
                let cell = (
                    (c * w / self.cols) as u32,
                    (r * h / self.rows) as u32,
                    ((c + 1) * w / self.cols) as u32,
                    ((r + 1) * h / self.rows) as u32,
                );
                let clipped = (cell.0.max(x0), cell.1.max(y0), cell.2.min(x1), cell.3.min(y1));
                if clipped.2 > clipped.0 && clipped.3 > clipped.1 && ink.ink_in(clipped) > 0 {
                    out.push(r * self.cols + c);
                }
            }
        }
        out
    }

    pub fn mean(values: &[f32], cells: &[usize]) -> f32 {
        cells.iter().map(|&i| values[i]).sum::<f32>() / cells.len().max(1) as f32
    }
}

fn overlap_ratio(b: Bbox, zone: Bbox) -> f32 {
    let area = (b.2.saturating_sub(b.0)) as u64 * (b.3.saturating_sub(b.1)) as u64;
    if area == 0 {
        return 0.0;
    }
    let iw = b.2.min(zone.2).saturating_sub(b.0.max(zone.0)) as u64;
    let ih = b.3.min(zone.3).saturating_sub(b.1.max(zone.1)) as u64;
    (iw * ih) as f32 / area as f32
}

/// Returns the structural role of `b`:
/// - `TitleBlock` when in top banner (`y in 0.02..0.095 H` overlap >= 20%) or bottom-right (`x0 >= 0.60 && y0 >= 0.75`)
/// - `SignatureCorner` when bottom-right corner (`x >= 0.72 W && y >= 0.86 H` overlap >= 20%)
pub fn classify_protected_role(b: Bbox, w: u32, h: u32, baseline: Baseline) -> ProtectedRole {
    if baseline == Baseline::Each || w == 0 || h == 0 {
        return ProtectedRole::Standard;
    }
    let (wf, hf) = (w as f32, h as f32);
    let top_band = (0, (hf * 0.02) as u32, w, (hf * 0.095).ceil() as u32);
    if overlap_ratio(b, top_band) >= 0.20 {
        return ProtectedRole::TitleBlock;
    }
    let x0_norm = b.0 as f32 / wf;
    let y0_norm = b.1 as f32 / hf;
    if x0_norm >= 0.60 && y0_norm >= 0.75 {
        return ProtectedRole::TitleBlock;
    }
    let sig_corner = ((wf * 0.72) as u32, (hf * 0.86) as u32, w, h);
    if overlap_ratio(b, sig_corner) >= 0.20 {
        return ProtectedRole::SignatureCorner;
    }
    ProtectedRole::Standard
}

/// True when `b` belongs to a protected region (TitleBlock band or signature corner) under `Baseline::Best`.
pub fn protected(b: Bbox, w: u32, h: u32) -> bool {
    classify_protected_role(b, w, h, Baseline::Best) != ProtectedRole::Standard
}

pub fn suppress_contained(crops: &mut Vec<CropPlan>) {
    let area = |b: Bbox| (b.2 - b.0) as u64 * (b.3 - b.1) as u64;
    let inside = |a: Bbox, b: Bbox| -> u64 {
        let (w, h) = (a.2.min(b.2).saturating_sub(a.0.max(b.0)), a.3.min(b.3).saturating_sub(a.1.max(b.1)));
        w as u64 * h as u64
    };
    let snapshot: Vec<(CropKind, Bbox)> = crops.iter().map(|c| (c.kind, c.bbox)).collect();
    let mut keep = vec![true; crops.len()];
    for (i, (kind, b)) in snapshot.iter().enumerate() {
        if *kind == CropKind::Sign || *kind == CropKind::Title {
            continue;
        }
        let swallowed = snapshot.iter().enumerate().any(|(j, (other, o))| {
            j != i
                && keep[j]
                && *other != CropKind::Sign
                && (area(*o) > area(*b) || (area(*o) == area(*b) && j < i))
                && inside(*b, *o) as f32 >= area(*b) as f32 * CONTAINED
        });
        keep[i] = !swallowed;
    }
    let mut flags = keep.into_iter();
    crops.retain(|_| flags.next().unwrap_or(true));
}

fn pad(ink: &InkPage, (x0, y0, x1, y1): Bbox) -> Bbox {
    (
        x0.saturating_sub(PAD),
        y0.saturating_sub(PAD),
        (x1 + PAD).min(ink.width),
        (y1 + PAD).min(ink.height),
    )
}

fn crop_of(ink: &InkPage, p: &PlanParams, kind: CropKind, bbox: Bbox, guide_margin: Option<f32>) -> CropPlan {
    let bbox = pad(ink, bbox);
    let upscale = upscale_factor(text_height(ink, bbox), p.text_px, bbox.2 - bbox.0, bbox.3 - bbox.1);
    CropPlan { kind, bbox, upscale, guide_margin }
}

fn sign_regions(s: &Scores, ink: &InkPage, gate: f32) -> Vec<Bbox> {
    let (rows, cols) = (s.rows, s.cols);
    let mut seen = vec![false; rows * cols];
    let mut out = Vec::new();
    let (w, h) = (ink.width as usize, ink.height as usize);
    for start in 0..rows * cols {
        if seen[start] || s.sign[start] < gate {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![start];
        let (mut r0, mut r1, mut c0, mut c1) = (rows, 0, cols, 0);
        while let Some(i) = stack.pop() {
            let (r, c) = (i / cols, i % cols);
            (r0, r1, c0, c1) = (r0.min(r), r1.max(r), c0.min(c), c1.max(c));
            for rr in r.saturating_sub(1)..(r + 2).min(rows) {
                for cc in c.saturating_sub(1)..(c + 2).min(cols) {
                    let j = rr * cols + cc;
                    if !seen[j] && s.sign[j] >= gate {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        let bbox = ((c0 * w / cols) as u32, (r0 * h / rows) as u32, ((c1 + 1) * w / cols) as u32, ((r1 + 1) * h / rows) as u32);
        if let Some(tight) = ink.tighten(bbox) {
            out.push(tight);
        }
    }
    out
}

pub fn plan_unfiltered(ink: &InkPage, p: &PlanParams) -> PagePlan {
    let mut crops = Vec::new();
    let mut blank = Vec::new();
    for block in blocks(ink) {
        let scale = upscale_factor(text_height(ink, block), p.text_px, block.2 - block.0, block.3 - block.1);
        for t in tiles(ink, block, scale, p.max_tile_patches) {
            let Some(t) = ink.text_box(t).or_else(|| ink.tighten(t)) else { continue };
            if ink.glyphs(t, MIN_GLYPHS) < MIN_GLYPHS {
                blank.push(t);
                continue;
            }
            crops.push(crop_of(ink, p, CropKind::Form, t, None));
        }
    }
    crops.sort_by_key(|c| (c.bbox.1 / 16, c.bbox.0));
    PagePlan { width: ink.width, height: ink.height, crops, dropped: Vec::new(), blank, unfiltered: true }
}

pub fn plan_page(ink: &InkPage, s: &Scores, p: &PlanParams) -> PagePlan {
    let mut crops = Vec::new();
    let mut dropped = Vec::new();
    let mut blank = Vec::new();
    let mut title: Option<(usize, f32)> = None;

    for run in blocks(ink).into_iter().flat_map(|b| runs(ink, b)) {
        let Some(bbox) = ink.tighten(run.bbox) else { continue };
        let role = classify_protected_role(bbox, ink.width, ink.height, p.baseline);
        if run.paragraph {
            let cells = s.cells(ink, bbox);
            let guide_wins = if p.use_arena && !cells.is_empty() {
                let firm = cells
                    .iter()
                    .filter(|&&i| {
                        let best_other = s.form[i].max(s.table[i]).max(s.title[i]);
                        s.guide[i] - best_other >= MIN_CELL_MARGIN
                    })
                    .count();
                firm as f32 >= cells.len() as f32 * p.drop_share
            } else {
                let gm = Scores::mean(&s.guide, &cells);
                let bm = Scores::mean(&s.form, &cells)
                    .max(Scores::mean(&s.table, &cells))
                    .max(Scores::mean(&s.title, &cells));
                gm - bm >= p.drop_margin
            };
            if guide_wins && !(p.protect && role != ProtectedRole::Standard) {
                dropped.push(bbox);
                continue;
            }
        }
        let scale = upscale_factor(text_height(ink, bbox), p.text_px, bbox.2 - bbox.0, bbox.3 - bbox.1);
        for t in tiles(ink, bbox, scale, p.max_tile_patches) {
            let Some(t) = ink.text_box(t) else { continue };
            if ink.glyphs(t, MIN_GLYPHS) < MIN_GLYPHS {
                blank.push(t);
                continue;
            }
            let cells = s.cells(ink, t);
            let f_m = Scores::mean(&s.form, &cells);
            let t_m = Scores::mean(&s.table, &cells);
            let ti_m = Scores::mean(&s.title, &cells);
            let g_m = Scores::mean(&s.guide, &cells);
            let kind = if t_m > f_m { CropKind::Table } else { CropKind::Form };
            if (t.1 as f32) < ink.height as f32 * TITLE_REGION && title.is_none_or(|(_, best)| ti_m > best) {
                title = Some((crops.len(), ti_m));
            }
            crops.push(crop_of(ink, p, kind, t, Some(g_m - f_m.max(t_m).max(ti_m))));
        }
    }
    if let Some((i, _)) = title {
        crops[i].kind = CropKind::Title;
    }
    suppress_contained(&mut crops);
    for r in sign_regions(s, ink, p.sign_gate) {
        crops.push(crop_of(ink, p, CropKind::Sign, r, None));
    }
    crops.sort_by_key(|c| (c.bbox.1 / 16, c.bbox.0));
    PagePlan { width: ink.width, height: ink.height, crops, dropped, blank, unfiltered: false }
}
