//! Unified SigLIP2 engine supporting four ecosystem compatibility profiles
//! (`LogisDocV1`, `StandardHf`, `RaonVe`, `NotoPyramidL1`), host-mmap zero-VRAM `MappedTable`,
//! sparse KV passes (`SparsePass`), and privacy-epoch block interruption (`EpochWatch`).

pub mod layout;
pub mod text;
pub mod tower;

use anyhow::Result;
use image::{imageops::FilterType, RgbImage};
use premium_discount_quant::safetensors::SafeTensors;
use zeroize::Zeroize;

pub use layout::{patch_columns_to_channel_major, Layout, Weights};
pub use text::{
    canonicalize, compute_anchors, mean_unit, SiglipTokenizer, TextTower, CONTEXT as TEXT_CONTEXT,
};
pub use tower::{
    antialias_bilinear_weights, bilinear_weights, cosine, unit, EpochWatch, LayerKv, SparsePass,
    VisionAbort, VisionConfig, VisionTower, WeightFormat,
};

pub const PATCH: usize = 16;
pub const DEFAULT_MAX_PATCHES: usize = 256;

/// Unified patch grid shared across `lv-core`, `lv-screen`, `lv-doc`, and `lv-spatial`.
/// Zeroized on `drop` and via `wipe()` for privacy protection.
#[derive(Clone, Debug, PartialEq)]
pub struct PatchGrid {
    pub grid_h: usize,
    pub grid_w: usize,
    pub patches: Vec<f32>,
}

impl PatchGrid {
    pub const DIM: usize = 3 * PATCH * PATCH;

    pub fn cells(&self) -> usize {
        self.grid_h * self.grid_w
    }

    pub fn patch(&self, cell: usize) -> &[f32] {
        &self.patches[cell * Self::DIM..(cell + 1) * Self::DIM]
    }

    pub fn wipe(&mut self) {
        self.patches.zeroize();
        self.grid_h = 0;
        self.grid_w = 0;
    }
}

impl Drop for PatchGrid {
    fn drop(&mut self) {
        self.patches.zeroize();
    }
}

/// Compatibility profile capturing downstream ecosystem numeric conventions on the unified SigLIP2 trunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiglipCompatProfile {
    /// Trade/doc 2D profile: non-alnum whitespace canonicalize + attn mask ON + clamped bilinear + Triangle `[0.5;3]` resize.
    LogisDocV1,
    /// Hugging Face `google/siglip2-so400m-patch16-naflex`: antialiased pos resize, mask OFF last-token pooling, `[0.5;3]`.
    StandardHf,
    /// KRAFTON Raon-VE timm profile: PIL bicubic + CLIP mean/std + open_clip canonicalize.
    RaonVe,
    /// Robotics `siglip2-base` 768d 1:2:4 pyramid L1 profile.
    NotoPyramidL1,
}

impl SiglipCompatProfile {
    pub fn layout(self) -> Layout {
        match self {
            Self::RaonVe => Layout::RaonVe,
            Self::LogisDocV1 | Self::StandardHf | Self::NotoPyramidL1 => Layout::Siglip2Hf,
        }
    }

    pub fn mean_std(self) -> ([f32; 3], [f32; 3]) {
        match self {
            Self::RaonVe => ([0.481_454_66, 0.457_827_5, 0.408_210_73], [0.268_629_5, 0.261_302_6, 0.275_777_1]),
            Self::LogisDocV1 | Self::StandardHf | Self::NotoPyramidL1 => ([0.5; 3], [0.5; 3]),
        }
    }

    pub fn pos_antialias(self) -> bool {
        matches!(self, Self::StandardHf)
    }

    pub fn use_attention_mask(self) -> bool {
        matches!(self, Self::LogisDocV1)
    }
}

/// Host mmap wrapper around token embedding weights (`0 B` GPU VRAM).
pub struct MappedTable<'a> {
    weights: Weights<'a>,
    tensor_name: &'static str,
}

impl<'a> MappedTable<'a> {
    pub fn new(st: &'a SafeTensors, layout: Layout, tensor_name: &'static str) -> Self {
        Self {
            weights: Weights { st, layout },
            tensor_name,
        }
    }

    pub fn gather_f32(&self, ids: &[u32]) -> Result<Vec<f32>> {
        let mut out = Vec::new();
        self.weights.gather_f32(self.tensor_name, ids, &mut out)?;
        Ok(out)
    }
}

/// Computes NaFlex target grid `(rows, cols)` preserving aspect ratio with `rows * cols <= max_patches`.
pub fn naflex_grid(height: u32, width: u32, patch: usize, max_patches: usize) -> (usize, usize) {
    let p = patch.max(1) as f64;
    let mut lo = 1e-6f64;
    let mut hi = 100.0f64;
    let cells = |s: f64, sz: u32| ((s * sz as f64 / p).ceil() as usize).max(1);
    while hi - lo >= 1e-5 {
        let mid = 0.5 * (lo + hi);
        if cells(mid, height) * cells(mid, width) <= max_patches.max(1) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (cells(lo, height), cells(lo, width))
}

/// Preprocesses RGB8 pixels into `PatchGrid` for `LogisDocV1` / `StandardHf` / `NotoPyramidL1` (`FilterType::Triangle` + `[mean;std]`).
pub fn preprocess_for(profile: SiglipCompatProfile, rgb: &[u8], w: u32, h: u32, max_patches: usize) -> Result<PatchGrid> {
    let img = RgbImage::from_raw(w, h, rgb.to_vec())
        .ok_or_else(|| anyhow::anyhow!("invalid RGB buffer for {w}x{h}"))?;
    let (rows, cols) = naflex_grid(h, w, PATCH, max_patches);
    let (tw, th) = (cols * PATCH, rows * PATCH);
    let filter = match profile {
        SiglipCompatProfile::RaonVe => FilterType::CatmullRom,
        SiglipCompatProfile::LogisDocV1 | SiglipCompatProfile::StandardHf | SiglipCompatProfile::NotoPyramidL1 => {
            FilterType::Triangle
        }
    };
    let resized = image::imageops::resize(&img, tw as u32, th as u32, filter);
    let (mean, std) = profile.mean_std();
    let mut patches = vec![0.0f32; rows * cols * PatchGrid::DIM];
    let mut lut = [[0f32; 256]; 3];
    for c in 0..3 {
        for v in 0..256 {
            lut[c][v] = (v as f32 / 255.0 - mean[c]) / std[c];
        }
    }
    let raw = resized.as_raw();
    for py in 0..rows {
        for px in 0..cols {
            let base = (py * cols + px) * PatchGrid::DIM;
            for c in 0..3 {
                let table = &lut[c];
                let ch_base = base + c * PATCH * PATCH;
                for yy in 0..PATCH {
                    let row_off = ((py * PATCH + yy) * tw + px * PATCH) * 3 + c;
                    let dst_off = ch_base + yy * PATCH;
                    for xx in 0..PATCH {
                        patches[dst_off + xx] = table[raw[row_off + xx * 3] as usize];
                    }
                }
            }
        }
    }
    Ok(PatchGrid {
        grid_h: rows,
        grid_w: cols,
        patches,
    })
}

/// Combining-mark-preserving text canonicalizer (`LogisDocV1`).
pub fn canonicalize_preserving_marks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '\u{0640}' {
            continue;
        }
        let u = c as u32;
        let is_mark = matches!(
            u,
            0x0300..=0x036F
                | 0x0591..=0x05BD
                | 0x05BF
                | 0x05C1..=0x05C2
                | 0x05C4..=0x05C7
                | 0x0610..=0x061A
                | 0x064B..=0x065F
                | 0x0670
                | 0x06D6..=0x06ED
                | 0x0900..=0x0903
                | 0x093A..=0x094F
                | 0x0951..=0x0957
                | 0x0962..=0x0963
                | 0x0E31
                | 0x0E34..=0x0E3A
                | 0x0E47..=0x0E4E
        );
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
        } else if is_mark {
            out.push(c);
        } else {
            out.push(' ');
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compat_profiles_distinguish_layouts_and_resampling() {
        assert_eq!(SiglipCompatProfile::RaonVe.layout(), Layout::RaonVe);
        assert_eq!(SiglipCompatProfile::LogisDocV1.layout(), Layout::Siglip2Hf);
        assert!(SiglipCompatProfile::StandardHf.pos_antialias());
        assert!(!SiglipCompatProfile::RaonVe.pos_antialias());
        assert!(SiglipCompatProfile::LogisDocV1.use_attention_mask());
    }

    #[test]
    fn patch_grid_wipe_zeros_and_resets() {
        let mut g = PatchGrid {
            grid_h: 2,
            grid_w: 2,
            patches: vec![0.42; 4 * PatchGrid::DIM],
        };
        g.wipe();
        assert_eq!(g.cells(), 0);
        assert!(g.patches.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn preprocess_for_logis_doc_v1_produces_normalized_patches() {
        let rgb = vec![255u8; 64 * 64 * 3];
        let grid = preprocess_for(SiglipCompatProfile::LogisDocV1, &rgb, 64, 64, 256).unwrap();
        assert_eq!(grid.cells(), 256);
        assert!((grid.patch(0)[0] - 1.0).abs() < 1e-5);
    }
}
