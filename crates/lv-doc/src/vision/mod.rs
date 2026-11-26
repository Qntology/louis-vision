//! Domain-neutral 2D document and engineering blueprint vision pipeline.

pub mod arena;
pub mod delta;
pub mod grounding;
pub mod ink;
pub mod lattice_verify;
pub mod layout_cache;
pub mod legibility;
pub mod phrase_cache;
pub mod plan;
pub mod probe;
pub mod symbols;
pub mod table_grid_reverse;
pub mod tiling;

use lv_core::siglip2::PatchGrid;

/// Default capacity of the bank-neutral purge memo table.
pub const PURGE_MEMO_CAP: usize = 32;

/// Uniform patch descriptor compatible with SigLIP2 vision grids (`LogisDocV1` / `StandardHf`).
#[derive(Debug, Clone)]
pub struct DocPatchGrid {
    pub grid_rows: usize,
    pub grid_cols: usize,
    pub orig_width: u32,
    pub orig_height: u32,
    /// Unit-normalized or raw patch embeddings `[grid_rows * grid_cols][dim]`.
    pub patches: Vec<Vec<f32>>,
}

impl DocPatchGrid {
    pub fn dim(&self) -> usize {
        self.patches.first().map_or(0, Vec::len)
    }

    pub fn len(&self) -> usize {
        self.patches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }

    pub fn from_lv_patch_grid(raw: &PatchGrid, emb_dim: usize, orig_w: u32, orig_h: u32) -> Self {
        let cells = raw.cells();
        let stride = if cells > 0 { raw.patches.len() / cells } else { emb_dim };
        let patches = (0..cells)
            .map(|i| raw.patches[i * stride..(i + 1) * stride].to_vec())
            .collect();
        Self {
            grid_rows: raw.grid_h,
            grid_cols: raw.grid_w,
            orig_width: orig_w,
            orig_height: orig_h,
            patches,
        }
    }
}

/// Shared doc-type classification verdict consumed by layout cache and heatmap grounding.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DocTypeVerdict {
    pub route_trade: bool,
    pub group: String,
    pub doc_type: String,
    pub subtype: String,
    pub doc_lang: String,
    pub group_score: f32,
    pub group_margin: f32,
    pub type_score: f32,
    pub type_margin: f32,
}

/// Shared category heatmap computed over document patches.
#[derive(Debug, Clone)]
pub struct CategoryHeatmap {
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

/// Cosine similarity between two equal-length float vectors using `lv-kernels` SIMD dot product.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let dot = lv_kernels::dot_f32_simd(a, b);
    let na = lv_kernels::l2_norm_seq(a);
    let nb = lv_kernels::l2_norm_seq(b);
    if na <= 0.0 || nb <= 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}
