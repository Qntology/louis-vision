//! Picture symbol dictionary (`ZZS1`) & connected-component matcher for non-text blueprint/form icons.

use anyhow::{bail, Result};
use image::{GrayImage, RgbImage};

use super::ink::{Bbox, InkPage};

const MAGIC: &[u8; 4] = b"ZZS1";
const VERSION: u32 = 1;

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

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 16 || &bytes[0..4] != MAGIC {
            bail!("invalid ZZS1 symbol dictionary");
        }
        let dim = u32::from_le_bytes(bytes[8..12].try_into()?) as usize;
        let count = u32::from_le_bytes(bytes[12..16].try_into()?) as usize;
        let mut off = 16usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let nlen = u32::from_le_bytes(bytes[off..off + 4].try_into()?) as usize;
            off += 4;
            let name = String::from_utf8(bytes[off..off + nlen].to_vec())?;
            off += nlen;
            let clen = u32::from_le_bytes(bytes[off..off + 4].try_into()?) as usize;
            off += 4;
            let category = String::from_utf8(bytes[off..off + clen].to_vec())?;
            off += clen;
            let scale = f32::from_le_bytes(bytes[off..off + 4].try_into()?);
            off += 4;
            let codes = bytes[off..off + dim].iter().map(|&b| b as i8).collect();
            off += dim;
            entries.push(SymbolEntry { name, category, scale, codes });
        }
        Ok(Self { dim, entries })
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
