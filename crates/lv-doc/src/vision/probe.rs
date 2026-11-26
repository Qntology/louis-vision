//! Visual crop anchor bank (`ZZP1`) mapping patch features to `[form, sign, table, title, guide]`.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::plan::Scores;
use super::DocPatchGrid;

pub const ZZP1_MAGIC: &[u8; 4] = b"ZZP1";

#[derive(Debug, Clone, PartialEq)]
pub struct ProbeBank {
    pub names: Vec<String>,
    pub dim: usize,
    mean: Vec<f32>,
    weights: Vec<f32>,
    intercept: Vec<f32>,
}

struct Reader<'a> {
    raw: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.raw.len());
        let Some(end) = end else {
            bail!("probe bank truncated at byte {}", self.at);
        };
        let out = &self.raw[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u32(&mut self) -> Result<usize> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    }

    fn f32s(&mut self, n: usize) -> Result<Vec<f32>> {
        let bytes = self.take(n * 4)?;
        Ok(bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect())
    }
}

impl ProbeBank {
    pub fn new(
        names: Vec<String>,
        dim: usize,
        mean: Vec<f32>,
        weights: Vec<f32>,
        intercept: Vec<f32>,
    ) -> Result<Self> {
        if dim == 0 || mean.len() != dim || intercept.len() != names.len() || weights.len() != names.len() * dim {
            bail!("invalid ProbeBank dimensions");
        }
        Ok(Self { names, dim, mean, weights, intercept })
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let raw = std::fs::read(path)
            .with_context(|| format!("failed to read probe bank: {}", path.display()))?;
        Self::from_bytes(&raw)
    }

    pub fn from_bytes(raw: &[u8]) -> Result<Self> {
        let mut r = Reader { raw, at: 0 };
        if r.take(4)? != ZZP1_MAGIC {
            bail!("invalid ZZP1 probe bank magic");
        }
        let (k, dim) = (r.u32()?, r.u32()?);
        let mut names = Vec::with_capacity(k);
        for _ in 0..k {
            let n = r.u32()?;
            names.push(String::from_utf8(r.take(n)?.to_vec())?);
        }
        let mean = r.f32s(dim)?;
        let weights = r.f32s(k * dim)?;
        let intercept = r.f32s(k)?;
        Self::new(names, dim, mean, weights, intercept)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(ZZP1_MAGIC);
        out.extend_from_slice(&(self.names.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.dim as u32).to_le_bytes());
        for n in &self.names {
            out.extend_from_slice(&(n.len() as u32).to_le_bytes());
            out.extend_from_slice(n.as_bytes());
        }
        for v in self.mean.iter().chain(&self.weights).chain(&self.intercept) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }

    /// Computes per-patch logits `[kind][patch]` using SIMD dot products over centred features.
    pub fn score_grid(&self, grid: &DocPatchGrid) -> Result<Vec<Vec<f32>>> {
        if grid.dim() != self.dim {
            bail!("probe bank dim {} != patch grid dim {}", self.dim, grid.dim());
        }
        let n = grid.len();
        let mut out = vec![vec![0f32; n]; self.names.len()];
        let mut centred = vec![0f32; self.dim];
        for (i, patch) in grid.patches.iter().take(n).enumerate() {
            for ((c, &x), &m) in centred.iter_mut().zip(patch.iter()).zip(self.mean.iter()) {
                *c = x - m;
            }
            for (k, row) in out.iter_mut().enumerate() {
                let w = &self.weights[k * self.dim..(k + 1) * self.dim];
                row[i] = self.intercept[k] + lv_kernels::dot_f32_simd(w, &centred);
            }
        }
        Ok(out)
    }
}

/// Smooths a patch score map via 3x3 available-neighbor averaging.
pub fn smooth(scores: &[f32], rows: usize, cols: usize) -> Vec<f32> {
    let mut out = vec![0f32; rows * cols];
    for r in 0..rows {
        for c in 0..cols {
            let (mut sum, mut n) = (0f32, 0f32);
            for rr in r.saturating_sub(1)..(r + 2).min(rows) {
                for cc in c.saturating_sub(1)..(c + 2).min(cols) {
                    sum += scores[rr * cols + cc];
                    n += 1.0;
                }
            }
            out[r * cols + c] = if n > 0.0 { sum / n } else { 0.0 };
        }
    }
    out
}

/// Converts raw `ProbeBank` scores into smoothed region channels while keeping `sign` unsmoothed.
pub fn build_scores(bank: &ProbeBank, grid: &DocPatchGrid) -> Result<Scores> {
    let raw = bank.score_grid(grid)?;
    let n = grid.len();
    let chan = |name: &str, do_smooth: bool| -> Vec<f32> {
        let Some(idx) = bank.index_of(name) else {
            return vec![0f32; n];
        };
        if do_smooth {
            smooth(&raw[idx], grid.grid_rows, grid.grid_cols)
        } else {
            raw[idx].clone()
        }
    };
    Ok(Scores {
        rows: grid.grid_rows,
        cols: grid.grid_cols,
        form: chan("form", true),
        sign: chan("sign", false),
        table: chan("table", true),
        title: chan("title", true),
        guide: chan("guide", true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zzp1_probe_scores_round_trips_and_smooths() -> Result<()> {
        let bank = ProbeBank::new(
            vec!["form".into(), "sign".into(), "table".into(), "title".into(), "guide".into()],
            2,
            vec![0.5, 0.0],
            vec![1.0, 0.0, 0.0, 2.0, 0.5, 0.5, 0.0, -1.0, -1.0, 1.0],
            vec![0.1, -0.1, 0.0, 0.0, 0.2],
        )?;
        let raw = bank.to_bytes();
        assert_eq!(ProbeBank::from_bytes(&raw)?, bank);
        assert!(ProbeBank::from_bytes(b"ZZS1....").is_err());

        let grid = DocPatchGrid {
            grid_rows: 1,
            grid_cols: 3,
            orig_width: 48,
            orig_height: 16,
            patches: vec![vec![1.0, 0.0], vec![0.5, 1.0], vec![0.0, 0.0]],
        };
        let sc = build_scores(&bank, &grid)?;
        assert_eq!(sc.sign.len(), 3);
        let s = smooth(&[3.0, 0.0, 0.0], 1, 3);
        assert!((s[0] - 1.5).abs() < 1e-6 && s[2] == 0.0);
        Ok(())
    }
}
