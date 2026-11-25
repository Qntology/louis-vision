//! Path-injected layout template cache (`LAYOUT_RECIPE`) preserving precision suffixes and dilated ink masks.

use std::path::PathBuf;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::legibility::LegibilityMap;
use super::{cosine, CategoryHeatmap, DocPatchGrid, DocTypeVerdict};

pub const LAYOUT_RECIPE: &str = "lv-doc-layout-v1";
const PROBE_WIN_MARGIN: f32 = 0.015;
const PROBE_MEDIAN_FLOOR: f32 = 0.72;
const PROBE_WIN_FLOOR: f32 = 0.65;
const PROBE_INK_FLOOR: f32 = 0.55;
const TRUST_AGREEMENTS: u32 = 3;
const AUDIT_EVERY: u32 = 8;
const AGREE_FLOOR: f32 = 0.65;
const RETIRE_DEMOTIONS: u32 = 3;
const PEAK_RADIUS: usize = 1;
const ANCHOR_MIN: usize = 4;
const ANCHOR_MAX: usize = 32;
const MAX_TEMPLATES: usize = 128;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct YieldStat {
    pub n: u64,
    pub mean: f64,
    pub m2: f64,
}

impl YieldStat {
    pub fn push(&mut self, x: f64) {
        self.n += 1;
        let delta = x - self.mean;
        self.mean += delta / self.n as f64;
        self.m2 += delta * (x - self.mean);
    }

    pub fn sd(&self) -> f64 {
        if self.n < 2 { 0.0 } else { (self.m2 / (self.n - 1) as f64).sqrt() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anchor {
    pub idx: usize,
    pub scale: f32,
    pub q: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedHeatmap {
    pub category: String,
    pub scores: Vec<f32>,
    pub top_field: String,
    pub top_score: f32,
    pub territory: usize,
    pub mean_margin: f32,
    pub top_rival: String,
    pub absent: bool,
    pub absent_reason: String,
    pub field_peaks: Vec<(String, usize, f32)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedVerdict {
    pub doc_type: String,
    pub subtype: String,
    pub doc_lang: String,
    pub group: String,
}

impl CachedVerdict {
    pub fn of(v: &DocTypeVerdict) -> Self {
        Self {
            doc_type: v.doc_type.clone(),
            subtype: v.subtype.clone(),
            doc_lang: v.doc_lang.clone(),
            group: v.group.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutTemplate {
    pub id: String,
    pub route_trade: bool,
    pub detected_type: String,
    pub grid_rows: usize,
    pub grid_cols: usize,
    pub aspect: f32,
    pub anchors: Vec<Anchor>,
    pub legible: Vec<bool>,
    pub verdict: Option<CachedVerdict>,
    pub heatmaps: Vec<CachedHeatmap>,
    pub observations: u32,
    pub agreements: u32,
    pub trusted: bool,
    pub reuse_count: u32,
    pub demotions: u32,
    pub retired: bool,
    pub yields: YieldStat,
    pub created_ms: i64,
    pub last_ms: i64,
}

impl LayoutTemplate {
    /// Projects master legible mask into `crop_bbox` local coordinates with `dilation_px` margin.
    pub fn trusted_dilated_crop_bounds(
        &self,
        crop_bbox: (u32, u32, u32, u32),
        orig_w: u32,
        orig_h: u32,
        dilation_px: u32,
    ) -> Option<(u32, u32, u32, u32)> {
        if !self.trusted || self.legible.is_empty() || self.grid_rows == 0 || self.grid_cols == 0 {
            return None;
        }
        let (cx0, cy0, cx1, cy1) = crop_bbox;
        let (cw, ch) = (cx1.saturating_sub(cx0), cy1.saturating_sub(cy0));
        if cw == 0 || ch == 0 || orig_w == 0 || orig_h == 0 {
            return None;
        }
        let (cell_w, cell_h) = (orig_w as f32 / self.grid_cols as f32, orig_h as f32 / self.grid_rows as f32);
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
        let mut found = false;
        for r in 0..self.grid_rows {
            let (py0, py1) = ((r as f32 * cell_h).floor() as u32, ((r + 1) as f32 * cell_h).ceil() as u32);
            if py1 <= cy0 || py0 >= cy1 {
                continue;
            }
            for c in 0..self.grid_cols {
                if !self.legible.get(r * self.grid_cols + c).copied().unwrap_or(false) {
                    continue;
                }
                let (px0, px1) = ((c as f32 * cell_w).floor() as u32, ((c + 1) as f32 * cell_w).ceil() as u32);
                if px1 <= cx0 || px0 >= cx1 {
                    continue;
                }
                let (lx0, ly0) = (px0.saturating_sub(cx0), py0.saturating_sub(cy0));
                let (lx1, ly1) = (px1.min(cx1).saturating_sub(cx0), py1.min(cy1).saturating_sub(cy0));
                if lx1 > lx0 && ly1 > ly0 {
                    min_x = min_x.min(lx0);
                    min_y = min_y.min(ly0);
                    max_x = max_x.max(lx1);
                    max_y = max_y.max(ly1);
                    found = true;
                }
            }
        }
        found.then_some((
            min_x.saturating_sub(dilation_px),
            min_y.saturating_sub(dilation_px),
            max_x.saturating_add(dilation_px).min(cw),
            max_y.saturating_add(dilation_px).min(ch),
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LayoutFile {
    recipe: String,
    templates: Vec<LayoutTemplate>,
}

#[derive(Debug, Clone)]
pub struct LayoutHit {
    pub template_id: String,
    pub trusted: bool,
    pub audit: bool,
    pub median: f32,
    pub win_rate: f32,
    pub ink_overlap: f32,
    pub route_trade: bool,
    pub detected_type: String,
    pub verdict: Option<CachedVerdict>,
    pub heatmaps: Vec<CachedHeatmap>,
    pub grid_rows: usize,
    pub grid_cols: usize,
    pub legible: Vec<bool>,
}

impl LayoutHit {
    pub fn reuse(&self) -> bool {
        self.trusted && !self.audit
    }
}

/// Path-injected template cache with optional precision fingerprint suffix (`|w=<tag>`).
pub struct LayoutCache {
    path: PathBuf,
    recipe: String,
    file: RwLock<Option<LayoutFile>>,
}

impl LayoutCache {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self::open_with_precision(path, None)
    }

    pub fn open_with_precision(path: impl Into<PathBuf>, precision_tag: Option<&str>) -> Self {
        let recipe = match precision_tag {
            Some(tag) if !tag.trim().is_empty() => format!("{LAYOUT_RECIPE}|w={}", tag.trim()),
            _ => LAYOUT_RECIPE.to_string(),
        };
        Self { path: path.into(), recipe, file: RwLock::new(None) }
    }

    pub fn recipe(&self) -> &str {
        &self.recipe
    }

    fn path_for(&self, recipe: &str) -> PathBuf {
        match recipe.split_once("|w=") {
            Some((_, suffix)) => self.path.with_file_name(format!("templates-{suffix}.json")),
            None => self.path.clone(),
        }
    }

    fn ensure_loaded(&self) {
        let current = |g: &Option<LayoutFile>| g.as_ref().is_some_and(|f| f.recipe == self.recipe);
        if self.file.read().map(|g| current(&*g)).unwrap_or(true) {
            return;
        }
        let Ok(mut g) = self.file.write() else { return };
        if current(&*g) {
            return;
        }
        let loaded = std::fs::read_to_string(self.path_for(&self.recipe))
            .ok()
            .and_then(|s| serde_json::from_str::<LayoutFile>(&s).ok());
        let file = match loaded {
            Some(f) if f.recipe == self.recipe => f,
            Some(_) => {
                let _ = std::fs::remove_file(self.path_for(&self.recipe));
                LayoutFile { recipe: self.recipe.clone(), templates: Vec::new() }
            }
            None => LayoutFile { recipe: self.recipe.clone(), templates: Vec::new() },
        };
        *g = Some(file);
    }

    fn persist(&self, file: &LayoutFile) {
        let Ok(text) = serde_json::to_string(file) else { return };
        let path = self.path_for(&file.recipe);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    pub fn probe(&self, grid: &DocPatchGrid, legibility: &LegibilityMap, allow_commerce: bool) -> Option<LayoutHit> {
        self.ensure_loaded();
        let mut g = self.file.write().ok()?;
        let file = g.as_mut()?;
        let n = grid.grid_rows * grid.grid_cols;
        let mask: Vec<bool> = (0..n).map(|i| legibility.is_legible(i)).collect();
        let aspect = grid.orig_width as f32 / grid.orig_height.max(1) as f32;
        let mut best: Option<(usize, f32, f32, f32)> = None;
        for (ti, t) in file.templates.iter().enumerate() {
            if t.retired
                || (!allow_commerce && !t.route_trade)
                || t.grid_rows != grid.grid_rows
                || t.grid_cols != grid.grid_cols
                || (t.aspect - aspect).abs() > 0.03 * aspect.max(0.1)
            {
                continue;
            }
            let (median, win) = align_score(t, grid);
            let ink = ink_overlap(&t.legible, &mask);
            if median >= PROBE_MEDIAN_FLOOR
                && win >= PROBE_WIN_FLOOR
                && ink >= PROBE_INK_FLOOR
                && best.is_none_or(|(_, m, _, _)| median > m)
            {
                best = Some((ti, median, win, ink));
            }
        }
        let (ti, median, win, ink) = best?;
        let t = &mut file.templates[ti];
        t.last_ms = now_ms();
        let audit = if t.trusted {
            t.reuse_count += 1;
            t.reuse_count % AUDIT_EVERY == 0
        } else {
            false
        };
        Some(LayoutHit {
            template_id: t.id.clone(),
            trusted: t.trusted,
            audit,
            median,
            win_rate: win,
            ink_overlap: ink,
            route_trade: t.route_trade,
            detected_type: t.detected_type.clone(),
            verdict: t.verdict.clone(),
            heatmaps: t.heatmaps.clone(),
            grid_rows: t.grid_rows,
            grid_cols: t.grid_cols,
            legible: t.legible.clone(),
        })
    }

    pub fn observe(&self, hit: &LayoutHit, detected_type: &str, fresh: &[CachedHeatmap]) {
        let (peaks, corr) = heatmap_agreement(&hit.heatmaps, fresh, hit.grid_cols);
        let agree = hit.detected_type == detected_type && peaks.min(corr) >= AGREE_FLOOR;
        self.ensure_loaded();
        let Ok(mut g) = self.file.write() else { return };
        let Some(file) = g.as_mut() else { return };
        let Some(t) = file.templates.iter_mut().find(|t| t.id == hit.template_id) else { return };
        t.observations += 1;
        if agree {
            t.agreements += 1;
            if !t.trusted && t.agreements >= TRUST_AGREEMENTS {
                t.trusted = true;
            }
        } else {
            t.agreements = 0;
            if t.trusted {
                t.trusted = false;
                t.demotions += 1;
                if t.demotions >= RETIRE_DEMOTIONS {
                    t.retired = true;
                }
            }
        }
        let snap = file.clone();
        drop(g);
        self.persist(&snap);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn commit(
        &self,
        hit: Option<&LayoutHit>,
        fresh: Option<&Vec<CachedHeatmap>>,
        grid: &DocPatchGrid,
        legibility: &LegibilityMap,
        route_trade: bool,
        detected_type: &str,
        verdict: Option<&DocTypeVerdict>,
        extracted: &Value,
    ) {
        let filled = count_filled(extracted);
        self.ensure_loaded();
        let Ok(mut g) = self.file.write() else { return };
        let Some(file) = g.as_mut() else { return };
        match hit {
            Some(h) => {
                let Some(t) = file.templates.iter_mut().find(|t| t.id == h.template_id) else { return };
                let (mean, sd, n) = (t.yields.mean, t.yields.sd(), t.yields.n);
                if h.reuse() {
                    let floor = if n >= 3 { mean - 2.0 * sd.max(1.0) } else { mean * 0.5 };
                    if filled == 0 || (n >= 1 && (filled as f64) < floor) {
                        t.trusted = false;
                        t.agreements = 0;
                        t.demotions += 1;
                        if t.demotions >= RETIRE_DEMOTIONS {
                            t.retired = true;
                        }
                    }
                }
                t.yields.push(filled as f64);
                t.last_ms = now_ms();
            }
            None => {
                let Some(fresh) = fresh.filter(|f| !f.is_empty() && filled > 0) else { return };
                let mut peaks: Vec<(usize, f32)> = Vec::new();
                for h in fresh {
                    for &(_, p, s) in &h.field_peaks {
                        if p < grid.patches.len() {
                            match peaks.iter_mut().find(|(q, _)| *q == p) {
                                Some(e) => e.1 = e.1.max(s),
                                None => peaks.push((p, s)),
                            }
                        }
                    }
                }
                peaks.sort_by(|a, b| b.1.total_cmp(&a.1));
                peaks.truncate(ANCHOR_MAX);
                if peaks.len() < ANCHOR_MIN {
                    return;
                }
                let now = now_ms();
                let template = LayoutTemplate {
                    id: format!("{}-{}x{}-{:x}", detected_type, grid.grid_rows, grid.grid_cols, now),
                    route_trade,
                    detected_type: detected_type.to_string(),
                    grid_rows: grid.grid_rows,
                    grid_cols: grid.grid_cols,
                    aspect: grid.orig_width as f32 / grid.orig_height.max(1) as f32,
                    anchors: peaks.iter().map(|(p, _)| quantize(*p, &grid.patches[*p])).collect(),
                    legible: (0..grid.grid_rows * grid.grid_cols).map(|i| legibility.is_legible(i)).collect(),
                    verdict: verdict.map(CachedVerdict::of),
                    heatmaps: fresh.clone(),
                    observations: 0,
                    agreements: 0,
                    trusted: false,
                    reuse_count: 0,
                    demotions: 0,
                    retired: false,
                    yields: {
                        let mut y = YieldStat::default();
                        y.push(filled as f64);
                        y
                    },
                    created_ms: now,
                    last_ms: now,
                };
                file.templates.push(template);
                if file.templates.len() > MAX_TEMPLATES {
                    file.templates.sort_by(|a, b| b.last_ms.cmp(&a.last_ms));
                    file.templates.truncate(MAX_TEMPLATES);
                }
            }
        }
        let snap = file.clone();
        drop(g);
        self.persist(&snap);
    }

    pub fn clear_all(&self) {
        let _ = std::fs::remove_file(self.path_for(&self.recipe));
        if let Ok(mut g) = self.file.write() {
            *g = Some(LayoutFile { recipe: self.recipe.clone(), templates: Vec::new() });
        }
    }
}

pub fn snapshot(heatmaps: &[CategoryHeatmap]) -> Vec<CachedHeatmap> {
    let finite = |x: f32| if x.is_finite() { x } else { f32::MIN };
    heatmaps
        .iter()
        .map(|h| CachedHeatmap {
            category: h.category.clone(),
            scores: h.scores.iter().map(|s| finite(*s)).collect(),
            top_field: h.top_field.clone(),
            top_score: finite(h.top_score),
            territory: h.territory,
            mean_margin: finite(h.mean_margin),
            top_rival: h.top_rival.clone(),
            absent: h.absent,
            absent_reason: h.absent_reason.clone(),
            field_peaks: h.field_peaks.iter().map(|(f, p, s)| (f.clone(), *p, finite(*s))).collect(),
        })
        .collect()
}

fn quantize(idx: usize, v: &[f32]) -> Anchor {
    let scale = v.iter().fold(0.0f32, |m, x| m.max(x.abs())).max(1e-8);
    let bytes: Vec<u8> = v.iter().map(|x| ((x / scale) * 127.0).round().clamp(-127.0, 127.0) as i8 as u8).collect();
    Anchor { idx, scale, q: B64.encode(bytes) }
}

fn dequant(a: &Anchor) -> Vec<f32> {
    B64.decode(a.q.as_bytes())
        .map(|b| b.into_iter().map(|x| (x as i8) as f32 / 127.0 * a.scale).collect())
        .unwrap_or_default()
}

fn ink_overlap(a: &[bool], b: &[bool]) -> f32 {
    let (mut inter, mut uni) = (0usize, 0usize);
    for i in 0..a.len().min(b.len()) {
        if a[i] || b[i] {
            uni += 1;
            if a[i] && b[i] {
                inter += 1;
            }
        }
    }
    if uni == 0 { 0.0 } else { inter as f32 / uni as f32 }
}

fn align_score(t: &LayoutTemplate, grid: &DocPatchGrid) -> (f32, f32) {
    let (rows, cols) = (grid.grid_rows as i64, grid.grid_cols as i64);
    let mut aligned = Vec::with_capacity(t.anchors.len());
    let mut wins = 0usize;
    for a in &t.anchors {
        if a.idx >= grid.patches.len() {
            continue;
        }
        let v = dequant(a);
        if v.len() != grid.patches[a.idx].len() {
            continue;
        }
        let s = cosine(&v, &grid.patches[a.idx]);
        let (r, c) = (a.idx as i64 / cols, a.idx as i64 % cols);
        let mut rival = f32::MIN;
        for (dr, dc) in [(-1i64, 0), (1, 0), (0, -1), (0, 1)] {
            let (rr, cc) = (r + dr, c + dc);
            if rr >= 0 && cc >= 0 && rr < rows && cc < cols {
                rival = rival.max(cosine(&v, &grid.patches[(rr * cols + cc) as usize]));
            }
        }
        if rival == f32::MIN || s > rival + PROBE_WIN_MARGIN {
            wins += 1;
        }
        aligned.push(s);
    }
    if aligned.is_empty() {
        return (0.0, 0.0);
    }
    aligned.sort_by(f32::total_cmp);
    (aligned[aligned.len() / 2], wins as f32 / aligned.len() as f32)
}

fn pearson(a: &[f32], b: &[f32]) -> Option<f32> {
    let pairs: Vec<(f32, f32)> = a.iter().zip(b).filter(|(x, y)| x.is_finite() && y.is_finite() && **x > -1e30 && **y > -1e30).map(|(x, y)| (*x, *y)).collect();
    if pairs.len() < 4 {
        return None;
    }
    let n = pairs.len() as f32;
    let ma = pairs.iter().map(|p| p.0).sum::<f32>() / n;
    let mb = pairs.iter().map(|p| p.1).sum::<f32>() / n;
    let (mut sab, mut saa, mut sbb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in pairs {
        sab += (x - ma) * (y - mb);
        saa += (x - ma) * (x - ma);
        sbb += (y - mb) * (y - mb);
    }
    (saa > 0.0 && sbb > 0.0).then_some(sab / (saa.sqrt() * sbb.sqrt()))
}

fn heatmap_agreement(template: &[CachedHeatmap], fresh: &[CachedHeatmap], cols: usize) -> (f32, f32) {
    let cols = cols.max(1);
    let cheb = |p: usize, q: usize| (p / cols).abs_diff(q / cols).max((p % cols).abs_diff(q % cols));
    let (mut peak_total, mut peak_hit, mut corr_sum, mut corr_n) = (0usize, 0usize, 0.0f32, 0usize);
    for ht in template {
        let Some(hf) = fresh.iter().find(|h| h.category == ht.category) else {
            peak_total += ht.field_peaks.len();
            continue;
        };
        for (f, p, _) in &ht.field_peaks {
            peak_total += 1;
            if hf.field_peaks.iter().any(|(g, q, _)| g == f && cheb(*p, *q) <= PEAK_RADIUS) {
                peak_hit += 1;
            }
        }
        if let Some(c) = pearson(&ht.scores, &hf.scores) {
            corr_sum += c.max(0.0);
            corr_n += 1;
        }
    }
    let peaks = if peak_total == 0 { 1.0 } else { peak_hit as f32 / peak_total as f32 };
    let corr = if corr_n == 0 { 0.0 } else { corr_sum / corr_n as f32 };
    (peaks, corr)
}

fn count_filled(v: &Value) -> usize {
    match v {
        Value::Null => 0,
        Value::Bool(_) | Value::Number(_) => 1,
        Value::String(s) => usize::from(!s.trim().is_empty()),
        Value::Array(a) => a.iter().map(count_filled).sum(),
        Value::Object(o) => o.values().map(count_filled).sum(),
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}
