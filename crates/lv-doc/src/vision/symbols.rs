//! Picture symbol dictionary (`ZZS1` v1/v2), A0 run-length DSU candidate extractor (`NONE_CLASS`), and legend few-shot builder.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use image::{GrayImage, Rgb, RgbImage};
use serde::Serialize;

use super::ink::{Bbox, InkPage};

pub const DICT_FILE: &str = "symbol_dict.bin";
pub const NONE_CLASS: &str = "none";
pub const NONE: &str = NONE_CLASS;
pub const SYMBOL_MAX_PATCHES: usize = 64;
const MAGIC: &[u8; 4] = b"ZZS1";
const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SymbolFeature {
    Pooled,
    MeanPatch,
}

pub type Feature = SymbolFeature;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CandidateParams {
    pub min_side: u32,
    pub max_side: u32,
    pub min_aspect: f32,
    pub min_fill: f32,
    pub max_fill: f32,
    pub dark: u8,
}

impl Default for CandidateParams {
    fn default() -> Self {
        Self {
            min_side: 34,
            max_side: 120,
            min_aspect: 0.35,
            min_fill: 0.04,
            max_fill: 0.62,
            dark: 120,
        }
    }
}

impl CandidateParams {
    pub fn scaled(scale: f32) -> Self {
        let d = Self::default();
        Self {
            min_side: (d.min_side as f32 * scale).round() as u32,
            max_side: (d.max_side as f32 * scale).round() as u32,
            ..d
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SymbolCandidate {
    pub bbox: [u32; 4],
    pub ink: u32,
}

pub type Candidate = SymbolCandidate;

impl SymbolCandidate {
    pub fn width(&self) -> u32 {
        self.bbox[2].saturating_sub(self.bbox[0])
    }

    pub fn height(&self) -> u32 {
        self.bbox[3].saturating_sub(self.bbox[1])
    }

    pub fn fill(&self) -> f32 {
        self.ink as f32 / (self.width() * self.height()).max(1) as f32
    }
}

#[inline]
fn is_black_ink(p: &Rgb<u8>, dark: u8) -> bool {
    let [r, g, b] = p.0;
    let lum = (r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8;
    let chroma = r.max(g).max(b) - r.min(g).min(b);
    lum < dark as u32 && chroma < 60
}

struct Dsu(Vec<u32>);

impl Dsu {
    fn find(&mut self, mut x: u32) -> u32 {
        while self.0[x as usize] != x {
            let up = self.0[self.0[x as usize] as usize];
            self.0[x as usize] = up;
            x = up;
        }
        x
    }

    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra.max(rb) as usize] = ra.min(rb);
        }
    }
}

/// A0 memory-free 8-connected black-ink run-length connected components (zero full-page visited table).
pub fn components(img: &RgbImage, dark: u8) -> Vec<SymbolCandidate> {
    let (w, h) = img.dimensions();
    let mut runs: Vec<(u32, u32, u32)> = Vec::new();
    let mut dsu = Dsu(Vec::new());
    let mut prev = 0..0;
    for y in 0..h {
        let start = runs.len();
        let mut x = 0;
        while x < w {
            if !is_black_ink(img.get_pixel(x, y), dark) {
                x += 1;
                continue;
            }
            let x0 = x;
            while x < w && is_black_ink(img.get_pixel(x, y), dark) {
                x += 1;
            }
            dsu.0.push(runs.len() as u32);
            runs.push((y, x0, x - 1));
        }
        let mut j = prev.start;
        for i in start..runs.len() {
            let (_, a0, a1) = runs[i];
            while j < prev.end && runs[j].2 + 1 < a0 {
                j += 1;
            }
            let mut k = j;
            while k < prev.end && runs[k].1 <= a1 + 1 {
                dsu.union(i as u32, k as u32);
                k += 1;
            }
        }
        prev = start..runs.len();
    }
    let mut index: HashMap<u32, usize> = HashMap::new();
    let mut out: Vec<SymbolCandidate> = Vec::new();
    for (i, &(y, x0, x1)) in runs.iter().enumerate() {
        let root = dsu.find(i as u32);
        let n = *index.entry(root).or_insert_with(|| {
            out.push(SymbolCandidate { bbox: [x0, y, x1 + 1, y + 1], ink: 0 });
            out.len() - 1
        });
        let c = &mut out[n];
        c.bbox = [c.bbox[0].min(x0), c.bbox[1].min(y), c.bbox[2].max(x1 + 1), c.bbox[3].max(y + 1)];
        c.ink += x1 - x0 + 1;
    }
    out
}

/// Extracts non-text symbol candidates in `34..=120 px`, aspect >= 0.35, fill 0.04..=0.62, merging interior pieces (+-2 px).
pub fn candidates(img: &RgbImage, p: &CandidateParams) -> Vec<SymbolCandidate> {
    let (w, h) = img.dimensions();
    let small: Vec<SymbolCandidate> = components(img, p.dark)
        .into_iter()
        .filter(|c| c.width() <= p.max_side && c.height() <= p.max_side)
        .collect();
    let mut seeds: Vec<SymbolCandidate> = small
        .iter()
        .filter(|c| c.width().max(c.height()) >= p.min_side)
        .filter(|c| c.bbox[0] > 0 && c.bbox[1] > 0 && c.bbox[2] < w && c.bbox[3] < h)
        .copied()
        .collect();
    for c in small.iter().filter(|c| c.width().max(c.height()) < p.min_side) {
        if let Some(s) = seeds.iter_mut().find(|s| {
            s.bbox[0] <= c.bbox[0] + 2
                && s.bbox[1] <= c.bbox[1] + 2
                && c.bbox[2] <= s.bbox[2] + 2
                && c.bbox[3] <= s.bbox[3] + 2
        }) {
            s.ink += c.ink;
        }
    }
    seeds.retain(|s| {
        let aspect = s.width().min(s.height()) as f32 / s.width().max(s.height()).max(1) as f32;
        aspect >= p.min_aspect && (p.min_fill..=p.max_fill).contains(&s.fill())
    });
    seeds.sort_by_key(|s| (s.bbox[1], s.bbox[0]));
    seeds
}

/// Crops a 1.5x square centered on `bbox`, padding outside-page pixels with white `[255, 255, 255]`.
pub fn crop_square(img: &RgbImage, bbox: [u32; 4]) -> RgbImage {
    let side = ((bbox[2] - bbox[0]).max(bbox[3] - bbox[1]) as f32 * 1.5).round().max(8.0) as i64;
    let cx = (bbox[0] + bbox[2]) as i64 / 2;
    let cy = (bbox[1] + bbox[3]) as i64 / 2;
    let (x0, y0) = (cx - side / 2, cy - side / 2);
    let (w, h) = (img.width() as i64, img.height() as i64);
    RgbImage::from_fn(side as u32, side as u32, |x, y| {
        let (sx, sy) = (x0 + x as i64, y0 + y as i64);
        if (0..w).contains(&sx) && (0..h).contains(&sy) {
            *img.get_pixel(sx as u32, sy as u32)
        } else {
            Rgb([255, 255, 255])
        }
    })
}

pub fn ink_extent(img: &RgbImage, bbox: [u32; 4], margin: u32, dark: u8) -> Option<[u32; 4]> {
    let (x0, y0) = (bbox[0] + margin, bbox[1] + margin);
    let (x1, y1) = (bbox[2].saturating_sub(margin).min(img.width()), bbox[3].saturating_sub(margin).min(img.height()));
    let mut out: Option<[u32; 4]> = None;
    for y in y0..y1 {
        for x in x0..x1 {
            if is_black_ink(img.get_pixel(x, y), dark) {
                out = Some(out.map_or([x, y, x + 1, y + 1], |o| [o[0].min(x), o[1].min(y), o[2].max(x + 1), o[3].max(y + 1)]));
            }
        }
    }
    out
}

pub fn normalize_vec(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        for x in v.iter_mut() {
            *x /= n;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SymbolMatch {
    pub class: Option<String>,
    pub nearest: String,
    pub sim: f32,
    pub margin: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolEntry {
    pub name: String,
    pub category: String,
    pub scale: f32,
    pub codes: Vec<i8>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SymbolDict {
    pub dim: usize,
    pub entries: Vec<SymbolEntry>,
}

impl SymbolDict {
    /// Builds an in-memory symbol dictionary from few-shot legend samples without retraining weights.
    pub fn from_legend_samples(
        feature: SymbolFeature,
        classes: Vec<String>,
        samples: &[(usize, Vec<f32>)],
    ) -> Result<Self> {
        let dim = samples.first().map(|s| s.1.len()).context("no legend symbol samples provided")?;
        if dim == 0 {
            bail!("legend sample vector dimension cannot be 0");
        }
        let cat = match feature {
            SymbolFeature::Pooled => "legend:pooled",
            SymbolFeature::MeanPatch => "legend:mean_patch",
        };
        let mut entries = Vec::with_capacity(classes.len());
        for (c, cname) in classes.into_iter().enumerate() {
            let mut sum = vec![0.0f32; dim];
            let mut n = 0usize;
            for (k, v) in samples.iter().filter(|s| s.0 == c) {
                debug_assert_eq!(*k, c);
                for (a, b) in sum.iter_mut().zip(v.iter()) {
                    *a += *b;
                }
                n += 1;
            }
            if n == 0 {
                bail!("legend class '{cname}' has no samples");
            }
            normalize_vec(&mut sum);
            let max_abs = sum.iter().fold(0.0f32, |m, &x| m.max(x.abs())).max(1e-8);
            let scale = max_abs / 127.0;
            let codes = sum.iter().map(|&x| (x / scale).round().clamp(-127.0, 127.0) as i8).collect();
            entries.push(SymbolEntry {
                name: cname,
                category: cat.to_string(),
                scale,
                codes,
            });
        }
        Ok(Self { dim, entries })
    }

    /// Classifies `query` with explicit rejection gates (`NONE_CLASS`, `min_sim`, `min_margin`).
    pub fn classify_with(&self, query: &[f32], min_sim: f32, min_margin: f32) -> SymbolMatch {
        if query.len() != self.dim || self.entries.is_empty() {
            return SymbolMatch {
                class: None,
                nearest: NONE_CLASS.to_string(),
                sim: 0.0,
                margin: 0.0,
            };
        }
        let mut scored = Vec::with_capacity(self.entries.len());
        let mut buf = vec![0.0f32; self.dim];
        for e in &self.entries {
            for (d, &c) in buf.iter_mut().zip(&e.codes) {
                *d = c as f32 * e.scale;
            }
            scored.push((e.name.as_str(), super::cosine(query, &buf)));
        }
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        let (nearest, sim) = (scored[0].0.to_string(), scored[0].1);
        let mut best_by_class: Vec<(&str, f32)> = Vec::new();
        for (name, s) in &scored {
            if let Some(existing) = best_by_class.iter_mut().find(|(n, _)| n == name) {
                existing.1 = existing.1.max(*s);
            } else {
                best_by_class.push((*name, *s));
            }
        }
        let margin = best_by_class.get(1).map_or(sim, |(_, s2)| sim - *s2);
        let accepted = nearest != NONE_CLASS && sim >= min_sim && margin >= min_margin;
        SymbolMatch {
            class: accepted.then(|| nearest.clone()),
            nearest,
            sim,
            margin,
        }
    }

    pub fn classify(&self, query: &[f32]) -> SymbolMatch {
        self.classify_with(query, 0.0, 0.0)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.dim as u32).to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for e in &self.entries {
            let nb = e.name.as_bytes();
            let cb = e.category.as_bytes();
            out.extend_from_slice(&(nb.len() as u32).to_le_bytes());
            out.extend_from_slice(nb);
            out.extend_from_slice(&(cb.len() as u32).to_le_bytes());
            out.extend_from_slice(cb);
            out.extend_from_slice(&e.scale.to_le_bytes());
            out.extend(e.codes.iter().map(|&c| c as u8));
        }
        out
    }

    /// Serializes dictionary using ZZS1 v2 feature/class/prototype layout.
    pub fn to_bytes_v2(&self, feature: SymbolFeature, min_sim: f32, min_margin: f32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(match feature {
            SymbolFeature::Pooled => 0,
            SymbolFeature::MeanPatch => 1,
        });
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for e in &self.entries {
            out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            out.extend_from_slice(e.name.as_bytes());
        }
        out.extend_from_slice(&(self.dim as u32).to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (c, e) in self.entries.iter().enumerate() {
            out.extend_from_slice(&(c as u32).to_le_bytes());
            for &code in &e.codes {
                let v = code as f32 * e.scale;
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out.extend_from_slice(&min_sim.to_le_bytes());
        out.extend_from_slice(&min_margin.to_le_bytes());
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 9 || &bytes[0..4] != MAGIC {
            bail!("invalid ZZS1 symbol dictionary");
        }
        // Raw v2 blueprint-sandbox format starts with 1-byte feature id (0 or 1), while v1 starts with u32 VERSION == 1.
        if bytes.len() >= 16 && u32::from_le_bytes(bytes[4..8].try_into()?) == VERSION && bytes.get(4..8) != Some(&[1, 1, 0, 0]) {
            // Check whether v1 bounds fit cleanly; fall through to v2 if malformed for v1.
            if let Ok(dict) = Self::parse_v1(bytes) {
                return Ok(dict);
            }
        }
        Self::parse_v2(bytes)
    }

    fn parse_v1(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 16 {
            bail!("truncated ZZS1 v1 header");
        }
        let dim = u32::from_le_bytes(bytes[8..12].try_into()?) as usize;
        let count = u32::from_le_bytes(bytes[12..16].try_into()?) as usize;
        let mut off = 16usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let nlen = u32::from_le_bytes(bytes.get(off..off + 4).context("eof")?.try_into()?) as usize;
            off += 4;
            let name = String::from_utf8(bytes.get(off..off + nlen).context("eof")?.to_vec())?;
            off += nlen;
            let clen = u32::from_le_bytes(bytes.get(off..off + 4).context("eof")?.try_into()?) as usize;
            off += 4;
            let category = String::from_utf8(bytes.get(off..off + clen).context("eof")?.to_vec())?;
            off += clen;
            let scale = f32::from_le_bytes(bytes.get(off..off + 4).context("eof")?.try_into()?);
            off += 4;
            let codes = bytes.get(off..off + dim).context("eof")?.iter().map(|&b| b as i8).collect();
            off += dim;
            entries.push(SymbolEntry { name, category, scale, codes });
        }
        if off != bytes.len() {
            bail!("trailing bytes in ZZS1 v1 stream");
        }
        Ok(Self { dim, entries })
    }

    fn parse_v2(bytes: &[u8]) -> Result<Self> {
        let mut at = 4usize;
        let take = |at: &mut usize, n: usize| -> Result<&[u8]> {
            let s = bytes.get(*at..*at + n).context("truncated ZZS1 v2 symbol dictionary")?;
            *at += n;
            Ok(s)
        };
        let feat_byte = take(&mut at, 1)?[0];
        let cat = match feat_byte {
            0 => "legend:pooled",
            1 => "legend:mean_patch",
            other => bail!("unknown ZZS1 v2 feature kind {other}"),
        };
        let u32_at = |s: &[u8]| u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
        let k = u32_at(take(&mut at, 4)?) as usize;
        let mut classes = Vec::with_capacity(k);
        for _ in 0..k {
            let n = u16::from_le_bytes(take(&mut at, 2)?.try_into()?) as usize;
            classes.push(String::from_utf8(take(&mut at, n)?.to_vec())?);
        }
        let dim = u32_at(take(&mut at, 4)?) as usize;
        let n = u32_at(take(&mut at, 4)?) as usize;
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            let c = u32_at(take(&mut at, 4)?) as usize;
            if c >= classes.len() {
                bail!("ZZS1 v2 class index out of range");
            }
            let raw_f32: Vec<f32> = take(&mut at, dim * 4)?
                .chunks_exact(4)
                .map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]]))
                .collect();
            let max_abs = raw_f32.iter().fold(0.0f32, |m, &x| m.max(x.abs())).max(1e-8);
            let scale = max_abs / 127.0;
            let codes = raw_f32.iter().map(|&x| (x / scale).round().clamp(-127.0, 127.0) as i8).collect();
            entries.push(SymbolEntry {
                name: classes[c].clone(),
                category: cat.to_string(),
                scale,
                codes,
            });
        }
        let _min_sim = f32::from_le_bytes(take(&mut at, 4)?.try_into()?);
        let _min_margin = f32::from_le_bytes(take(&mut at, 4)?.try_into()?);
        Ok(Self { dim, entries })
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        Self::from_bytes(&std::fs::read(path).with_context(|| format!("failed to read symbol dict: {}", path.display()))?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_bytes()).with_context(|| format!("failed to save symbol dict: {}", path.display()))
    }

    pub fn match_vec(&self, query: &[f32], top_k: usize) -> Vec<(&SymbolEntry, f32)> {
        if query.len() != self.dim || self.entries.is_empty() {
            return Vec::new();
        }
        let mut buf = vec![0.0f32; self.dim];
        let mut scored: Vec<(&SymbolEntry, f32)> = self
            .entries
            .iter()
            .map(|e| {
                for (d, &c) in buf.iter_mut().zip(&e.codes) {
                    *d = c as f32 * e.scale;
                }
                (e, super::cosine(query, &buf))
            })
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(top_k);
        scored
    }

    /// Scans grayscale image candidates without allocating a page-wide visited bitmap and returns accepted symbol names.
    pub fn match_in_gray(&self, gray: &GrayImage) -> Vec<String> {
        if self.dim == 0 || self.entries.is_empty() {
            return Vec::new();
        }
        let rgb = RgbImage::from_fn(gray.width(), gray.height(), |x, y| {
            let v = gray.get_pixel(x, y)[0];
            Rgb([v, v, v])
        });
        let cands = candidates(&rgb, &CandidateParams::default());
        let mut hits = Vec::new();
        for c in cands {
            let sq = crop_square(&rgb, c.bbox);
            let desc = candidate_descriptor(&sq, self.dim);
            let m = self.classify_with(&desc, 0.72, 0.05);
            if let Some(cls) = m.class {
                hits.push(cls);
            }
        }
        hits
    }
}

fn candidate_descriptor(crop: &RgbImage, dim: usize) -> Vec<f32> {
    let mut vec = vec![0.0f32; dim];
    let (w, h) = crop.dimensions();
    if w == 0 || h == 0 || dim == 0 {
        return vec;
    }
    for y in 0..h {
        for x in 0..w {
            let p = crop.get_pixel(x, y).0;
            let lum = (p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114) / 255.0;
            let ink = (1.0 - lum).max(0.0);
            let idx = (((y * dim as u32 / h) * 7 + (x * dim as u32 / w) * 13) as usize) % dim;
            vec[idx] += ink;
        }
    }
    normalize_vec(&mut vec);
    vec
}

pub fn find_components(ink: &InkPage, min_area: u32, max_area: u32) -> Vec<Bbox> {
    let (w, h) = (ink.width as usize, ink.height as usize);
    let mut visited = vec![false; w * h];
    let mut out = Vec::new();
    for start in 0..w * h {
        if visited[start] || ink.mask[start] == 0 {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![start];
        let (mut x0, mut y0, mut x1, mut y1) = (w as u32, h as u32, 0u32, 0u32);
        let mut pixels = 0u32;
        while let Some(i) = stack.pop() {
            let (x, y) = ((i % w) as u32, (i / w) as u32);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            pixels += 1;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        let j = ny as usize * w + nx as usize;
                        if !visited[j] && ink.mask[j] != 0 {
                            visited[j] = true;
                            stack.push(j);
                        }
                    }
                }
            }
        }
        if pixels >= min_area && (x1 - x0) * (y1 - y0) <= max_area {
            out.push((x0, y0, x1, y1));
        }
    }
    out
}

pub fn component_luma_crop(rgb: &RgbImage, bbox: Bbox) -> GrayImage {
    let cropped = image::imageops::crop_imm(rgb, bbox.0, bbox.1, bbox.2 - bbox.0, bbox.3 - bbox.1).to_image();
    image::imageops::grayscale(&cropped)
}
