//! Pre-VLM hybrid delta engine (`ZZD1`), L-frame alignment, chromatic ink priority (`chroma >= 45`),
//! monochrome Chebyshev dilation (`dilate(3)`), `TitleBlock` invariant (`Baseline::Best`), and `-eval` A/B accounting.

use std::sync::Mutex;

use anyhow::{bail, Result};
use lv_kernels::morph_chroma::{dilate_binary, separate_chroma_masked, CHROMA_MIN};
use premium_discount_eval::gate_ab::{compare_gate_arms, ArmRunSummary, GateAbVerdict};

use super::ink::{Bbox, InkPage};
use super::plan::{classify_protected_role, Baseline, CropKind, PagePlan, ProtectedRole};
use super::DocPatchGrid;

const MAGIC: &[u8; 4] = b"ZZD1";
const VERSION: u32 = 1;
const CLIP_PAD: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMethod {
    Mean,
    Centered,
    Chamfer,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MasterPatches {
    pub rows: usize,
    pub cols: usize,
    pub dim: usize,
    pub scales: Vec<f32>,
    pub codes: Vec<i8>,
}

impl MasterPatches {
    pub fn from_grid(grid: &DocPatchGrid) -> Self {
        let rows = grid.grid_rows;
        let cols = grid.grid_cols;
        let dim = grid.dim();
        let n = rows * cols;
        let mut scales = vec![1.0f32; n];
        let mut codes = vec![0i8; n * dim];
        for (i, p) in grid.patches.iter().take(n).enumerate() {
            let max_abs = p.iter().fold(0.0f32, |m, &x| m.max(x.abs())).max(1e-8);
            scales[i] = max_abs / 127.0;
            for j in 0..dim {
                codes[i * dim + j] = (p[j] / scales[i]).round().clamp(-127.0, 127.0) as i8;
            }
        }
        Self { rows, cols, dim, scales, codes }
    }

    pub fn len(&self) -> usize {
        self.rows * self.cols
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn cosines(&self, scan: &DocPatchGrid) -> Option<Vec<f32>> {
        if scan.grid_rows != self.rows || scan.grid_cols != self.cols || scan.dim() != self.dim {
            return None;
        }
        let mut out = Vec::with_capacity(self.len());
        let mut buf = vec![0.0f32; self.dim];
        for i in 0..self.len() {
            let s = self.scales[i];
            for j in 0..self.dim {
                buf[j] = self.codes[i * self.dim + j] as f32 * s;
            }
            out.push(super::cosine(&buf, &scan.patches[i]));
        }
        Some(out)
    }
}

#[derive(Debug, Clone)]
pub struct Master {
    pub id: String,
    pub patches: MasterPatches,
    pub ink: InkPage,
    pub dilated_mask: Vec<u8>,
    pub ignore: Vec<Bbox>,
}

impl Master {
    pub fn new(id: impl Into<String>, grid: &DocPatchGrid, ink: InkPage, radius: u32) -> Self {
        let dilated_mask = ink.dilate(radius);
        Self {
            id: id.into(),
            patches: MasterPatches::from_grid(grid),
            ink,
            dilated_mask,
            ignore: Vec::new(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Debug, Default)]
pub struct MasterSet {
    pub pages: Vec<Master>,
    mean_cache: Mutex<Option<(usize, usize, Vec<f32>)>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Matched {
    pub index: usize,
    pub similarity: f32,
    pub margin: f32,
}

impl MasterSet {
    pub fn push(&mut self, m: Master) {
        self.pages.push(m);
        *self.mean_cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn get(&self, id: &str) -> Option<&Master> {
        self.pages.iter().find(|p| p.id() == id)
    }

    pub fn set_ignore(&mut self, id: &str, zones: &[[f32; 4]]) {
        if let Some(m) = self.pages.iter_mut().find(|p| p.id() == id) {
            let (w, h) = (m.ink.width as f32, m.ink.height as f32);
            m.ignore = zones
                .iter()
                .map(|z| ((z[0] * w) as u32, (z[1] * h) as u32, (z[2] * w).ceil() as u32, (z[3] * h).ceil() as u32))
                .collect();
        }
    }

    pub fn best_match(&self, scan: &DocPatchGrid) -> Option<Matched> {
        self.best_match_by(scan, MatchMethod::Centered)
    }

    pub fn best_match_by(&self, scan: &DocPatchGrid, method: MatchMethod) -> Option<Matched> {
        let dim = scan.dim();
        let center = match method {
            MatchMethod::Mean => None,
            MatchMethod::Centered | MatchMethod::Chamfer => Some(self.mean_patch(dim)?),
        };
        let mut scored: Vec<(usize, f32)> = self
            .pages
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                let s = match (&center, method) {
                    (None, _) => {
                        let c = m.patches.cosines(scan)?;
                        c.iter().sum::<f32>() / c.len().max(1) as f32
                    }
                    (Some(mu), MatchMethod::Chamfer) => block_similarity(&m.patches, scan, mu, 1)?,
                    (Some(mu), _) => block_similarity(&m.patches, scan, mu, 0)?,
                };
                Some((i, s))
            })
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        let (index, similarity) = *scored.first()?;
        let margin = scored.get(1).map(|s| similarity - s.1).unwrap_or(1.0);
        Some(Matched { index, similarity, margin })
    }

    fn mean_patch(&self, dim: usize) -> Option<Vec<f32>> {
        let key = (self.pages.len(), dim);
        if let Some((n_pages, d, ref cached)) = *self.mean_cache.lock().unwrap_or_else(|p| p.into_inner()) {
            if (n_pages, d) == key {
                return Some(cached.clone());
            }
        }
        let mut mu = vec![0f64; dim];
        let mut n = 0usize;
        for m in &self.pages {
            if m.patches.dim != dim {
                continue;
            }
            for i in 0..m.patches.len() {
                let scale = m.patches.scales[i] as f64;
                let codes = &m.patches.codes[i * dim..(i + 1) * dim];
                for (a, &q) in mu.iter_mut().zip(codes) {
                    *a += q as f64 * scale;
                }
                n += 1;
            }
        }
        let res = (n > 0).then(|| mu.into_iter().map(|v| (v / n as f64) as f32).collect::<Vec<f32>>())?;
        *self.mean_cache.lock().unwrap_or_else(|p| p.into_inner()) = Some((key.0, key.1, res.clone()));
        Some(res)
    }

    /// Serializes quantized master patch index (`ZZD1`).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.pages.len() as u32).to_le_bytes());
        for m in &self.pages {
            let name = m.id.as_bytes();
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(name);
            out.extend_from_slice(&(m.patches.rows as u32).to_le_bytes());
            out.extend_from_slice(&(m.patches.cols as u32).to_le_bytes());
            out.extend_from_slice(&(m.patches.dim as u32).to_le_bytes());
            for &s in &m.patches.scales {
                out.extend_from_slice(&s.to_le_bytes());
            }
            out.extend(m.patches.codes.iter().map(|&c| c as u8));
        }
        out
    }

    pub fn parse_header(bytes: &[u8]) -> Result<usize> {
        if bytes.len() < 12 || &bytes[0..4] != MAGIC {
            bail!("invalid ZZD1 header");
        }
        let count = u32::from_le_bytes(bytes[8..12].try_into()?) as usize;
        Ok(count)
    }
}

fn block_similarity(master: &MasterPatches, scan: &DocPatchGrid, mu: &[f32], reach: usize) -> Option<f32> {
    if scan.grid_rows != master.rows || scan.grid_cols != master.cols || scan.dim() != master.dim {
        return None;
    }
    let (rows, cols, dim) = (master.rows, master.cols, master.dim);
    let cells = rows * cols;
    let mut m_flat = vec![0f32; cells * dim];
    for i in 0..cells {
        let scale = master.scales[i];
        let codes = &master.codes[i * dim..(i + 1) * dim];
        let dst = &mut m_flat[i * dim..(i + 1) * dim];
        for j in 0..dim {
            dst[j] = codes[j] as f32 * scale - mu[j];
        }
        let n = lv_kernels::dot_f32_simd(dst, dst).sqrt();
        if n > 1e-8 {
            let inv = 1.0 / n;
            for x in dst.iter_mut() {
                *x *= inv;
            }
        }
    }
    let mut s_buf = vec![0f32; dim];
    let mut sum = 0f32;
    for r in 0..rows {
        for c in 0..cols {
            let sp = &scan.patches[r * cols + c];
            for j in 0..dim {
                s_buf[j] = sp[j] - mu[j];
            }
            let n = lv_kernels::dot_f32_simd(&s_buf, &s_buf).sqrt();
            if n > 1e-8 {
                let inv = 1.0 / n;
                for x in s_buf.iter_mut() {
                    *x *= inv;
                }
            }
            let mut best = f32::MIN;
            for rr in r.saturating_sub(reach)..(r + reach + 1).min(rows) {
                for cc in c.saturating_sub(reach)..(c + reach + 1).min(cols) {
                    let idx = (rr * cols + cc) * dim;
                    let dot = lv_kernels::dot_f32_simd(&s_buf, &m_flat[idx..idx + dim]);
                    best = best.max(dot);
                }
            }
            sum += best;
        }
    }
    Some(sum / cells.max(1) as f32)
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeltaParams {
    pub radius: u32,
    pub block: u32,
    pub ink_threshold: f32,
    pub edge: u32,
    pub align: bool,
    pub chroma: bool,
    pub baseline: Baseline,
}

impl Default for DeltaParams {
    fn default() -> Self {
        Self {
            radius: 3,
            block: 16,
            ink_threshold: 0.03,
            edge: 32,
            align: true,
            chroma: true,
            baseline: Baseline::Best,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDiff {
    pub block: u32,
    pub rows: u32,
    pub cols: u32,
    pub diff: Vec<f32>,
    pub offset: (i32, i32),
}

impl BlockDiff {
    pub fn changed(&self, threshold: f32) -> usize {
        self.diff.iter().filter(|d| **d > threshold).count()
    }

    pub fn changed_boxes(&self, threshold: f32) -> Vec<Bbox> {
        self.diff
            .iter()
            .enumerate()
            .filter(|(_, d)| **d > threshold)
            .map(|(i, _)| {
                let (r, c) = (i as u32 / self.cols, i as u32 % self.cols);
                (c * self.block, r * self.block, (c + 1) * self.block, (r + 1) * self.block)
            })
            .collect()
    }

    pub fn touches(&self, b: Bbox, threshold: f32) -> bool {
        self.changed_boxes(threshold)
            .into_iter()
            .any(|cb| cb.0 < b.2 && cb.2 > b.0 && cb.1 < b.3 && cb.3 > b.1)
    }
}

/// Computes outer drawing-frame feed offset `(dx, dy)` between master and scan.
pub fn frame_offset(master: &InkPage, scan: &InkPage) -> Option<(i32, i32)> {
    let mb = master.tighten((0, 0, master.width, master.height))?;
    let sb = scan.tighten((0, 0, scan.width, scan.height))?;
    Some((sb.0 as i32 - mb.0 as i32, sb.1 as i32 - mb.1 as i32))
}

pub fn moved(zones: &[Bbox], offset: (i32, i32)) -> Vec<Bbox> {
    zones
        .iter()
        .map(|z| {
            (
                (z.0 as i32 + offset.0).max(0) as u32,
                (z.1 as i32 + offset.1).max(0) as u32,
                (z.2 as i32 + offset.0).max(0) as u32,
                (z.3 as i32 + offset.1).max(0) as u32,
            )
        })
        .collect()
}

/// Pre-VLM hybrid delta: chromatic ink priority (`chroma >= 45`) + monochrome `dilate(radius=3)` fallback.
pub fn ink_changes(master: &Master, scan: &InkPage, p: &DeltaParams) -> Result<BlockDiff> {
    if master.ink.width != scan.width || master.ink.height != scan.height {
        bail!("size mismatch between master and scan");
    }
    let (w, h) = (scan.width as usize, scan.height as usize);
    let offset = if p.align {
        frame_offset(&master.ink, scan).unwrap_or((0, 0))
    } else {
        (0, 0)
    };
    let master_dil = if p.radius == 3 && !master.dilated_mask.is_empty() {
        master.dilated_mask.clone()
    } else {
        dilate_binary(&master.ink.mask, w, h, p.radius)
    };
    let scan_dil = dilate_binary(&scan.mask, w, h, p.radius);
    let bsz = p.block.max(1);
    let cols = scan.width.div_ceil(bsz);
    let rows = scan.height.div_ceil(bsz);
    let mut diff = vec![0.0f32; (rows * cols) as usize];

    for y in p.edge..(scan.height.saturating_sub(p.edge)) {
        for x in p.edge..(scan.width.saturating_sub(p.edge)) {
            let sx = x as usize;
            let sy = y as usize;
            let si = sy * w + sx;
            let mx = (x as i32 - offset.0).clamp(0, master.ink.width as i32 - 1) as usize;
            let my = (y as i32 - offset.1).clamp(0, master.ink.height as i32 - 1) as usize;
            let mi = my * w + mx;
            let chroma_hit = p.chroma && scan.color.get(si).copied().unwrap_or(0) != 0;
            let new_ink = scan.mask[si] != 0 && master_dil[mi] == 0;
            let erased_ink = master.ink.mask[mi] != 0 && scan_dil[si] == 0;
            if chroma_hit || new_ink || erased_ink {
                let r = y / bsz;
                let c = x / bsz;
                diff[(r * cols + c) as usize] += 1.0 / (bsz * bsz) as f32;
            }
        }
    }
    Ok(BlockDiff { block: bsz, rows, cols, diff, offset })
}

/// Trims ignore zones while strictly preserving `ProtectedRole::TitleBlock` when `Baseline::Best`.
pub fn trim_ignored_with_baseline(
    plan: &mut PagePlan,
    ink: &InkPage,
    ignore: &[Bbox],
    baseline: Baseline,
) -> usize {
    let mut trimmed = 0usize;
    let mut next_crops = Vec::with_capacity(plan.crops.len());
    for mut c in plan.crops.drain(..) {
        if c.kind == CropKind::Title
            || c.kind == CropKind::Sign
            || classify_protected_role(c.bbox, plan.width, plan.height, baseline) == ProtectedRole::TitleBlock
        {
            next_crops.push(c);
            continue;
        }
        let mut cur = Some(c.bbox);
        let mut modified = false;
        for &ig in ignore {
            let Some(b) = cur else { break };
            let iw = b.2.min(ig.2).saturating_sub(b.0.max(ig.0)) as u64;
            let ih = b.3.min(ig.3).saturating_sub(b.1.max(ig.1)) as u64;
            let area = (b.2 - b.0) as u64 * (b.3 - b.1) as u64;
            if iw * ih == 0 {
                continue;
            }
            if (iw * ih) as f32 >= area as f32 * 0.60 {
                cur = None;
                modified = true;
            } else if ig.0 <= b.0 + 40 && ig.2 < b.2 {
                cur = ink.tighten((ig.2, b.1, b.2, b.3));
                modified = true;
            }
        }
        if modified {
            trimmed += 1;
        }
        match cur {
            Some(b) => {
                c.bbox = b;
                next_crops.push(c);
            }
            None => plan.blank.push(c.bbox),
        }
    }
    plan.crops = next_crops;
    trimmed
}

/// Evaluates chromatic extraction helper for raw RGB scans (`separate_chroma_masked`).
pub fn chromatic_stroke_count(rgb: &[u8], ink: &InkPage) -> usize {
    separate_chroma_masked(rgb, &ink.mask, ink.width as usize, ink.height as usize, CHROMA_MIN)
        .into_iter()
        .filter(|&v| v != 0)
        .count()
}

/// Accounts VLM tile skip savings vs TitleBlock recall using `premium_discount_eval::gate_ab`.
pub fn evaluate_delta_gate_ab(delta_on: &ArmRunSummary, full_off: &ArmRunSummary) -> GateAbVerdict {
    compare_gate_arms(delta_on, full_off)
}

#[allow(dead_code)]
fn _pad_keep() -> u32 {
    CLIP_PAD
}
