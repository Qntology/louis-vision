//! Domain-agnostic dependency injection traits (`DocDomainBank`, `GroundingLexicon`),
//! Gumbel surprisal value grounding (`verify_claims_v2`), `unit="PCS"` exemption,
//! printed-label echo blocking, `BLANK_SURGE_SIGMA = 3.0`, and 15% padded neighbor rescue.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::legibility::{LegibilityMap, PatchLegibility};
use super::plan::{classify_protected_role, Baseline, ProtectedRole};
use super::DocPatchGrid;

/// Upper outlier threshold for blank patch surge (`mean + 3.0 * sigma`).
pub const BLANK_SURGE_SIGMA: f32 = 3.0;
/// Padded retry expansion ratio (15% on each side) when neighbor patches hold legible ink.
pub const PADDED_RETRY_FRAC: f32 = 0.15;
pub const NAME_RESIDUE_CHARS: [char; 9] = ['-', '\u{2013}', '\u{2014}', '\u{2212}', '_', '/', '\\', '|', '~'];

/// External doc taxonomy/anchor bank injected by downstream hosts (`lv-domain`, `logis-vision-doc`).
pub trait DocDomainBank: Send + Sync {
    fn group_anchors(&self) -> Vec<(String, String)>;
    fn field_anchors(&self, doc_type: &str, doc_lang: &str) -> Vec<(String, String, String)>;
    fn is_self_reference_field(&self, doc_type: &str, field: &str) -> bool {
        let _ = (doc_type, field);
        false
    }
}

/// External grounding policy lexicon injected to keep `lv-doc` zero-domain-hardcoded.
pub trait GroundingLexicon: Send + Sync {
    /// Exempts short/code-like unit tokens (e.g. `unit = "PCS"`) from visual surprisal drops.
    fn is_exempt(&self, field: &str, value: &str) -> bool;
    /// Blocks printed caption echoes mistakenly read as field values (e.g. `"AIRWAYBILL / BILL OF LADING"`).
    fn is_printed_label_echo(&self, field: &str, value: &str) -> bool;
}

/// Standard default lexicon enforcing `unit = "PCS"` / standard unit exemptions and `party_role` / `doc_type` exemption.
#[derive(Debug, Default)]
pub struct StandardDocLexicon {
    pub extra_echo_labels: Vec<String>,
}

impl GroundingLexicon for StandardDocLexicon {
    fn is_exempt(&self, field: &str, value: &str) -> bool {
        if matches!(field, "party_role" | "doc_type") {
            return true;
        }
        if field == "unit" || field.ends_with("_unit") {
            let up = value.trim().to_ascii_uppercase();
            return matches!(up.as_str(), "PCS" | "PC" | "EA" | "SET" | "BOX" | "PKG" | "CTN" | "KG" | "MT" | "LT");
        }
        false
    }

    fn is_printed_label_echo(&self, _field: &str, value: &str) -> bool {
        let norm = value.trim().to_ascii_uppercase();
        if matches!(
            norm.as_str(),
            "AIRWAYBILL / BILL OF LADING"
                | "BUYER (IF NOT CONSIGNEE)"
                | "INVOICE NUMBER"
                | "DATE OF ISSUE"
                | "PORT OF LOADING"
                | "DESCRIPTION OF GOODS"
        ) {
            return true;
        }
        self.extra_echo_labels.iter().any(|l| l.trim().eq_ignore_ascii_case(norm.as_str()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerdictGate {
    Accepted,
    Format,
    Held,
}

#[derive(Debug, Clone)]
pub struct GroundingClaim {
    pub category: String,
    pub field: String,
    pub value: String,
    pub bbox: (u32, u32, u32, u32),
}

#[derive(Debug, Clone)]
pub struct GroundingVerdict {
    pub category: String,
    pub field: String,
    pub value: String,
    pub surprisal_in: f32,
    pub surprisal_out: f32,
    pub top_patch: usize,
    pub top_legible: bool,
    pub accepted: bool,
    pub gate: VerdictGate,
    pub reason: String,
}

pub fn gumbel_expected_z(k: usize) -> f32 {
    if k <= 1 {
        return 0.0;
    }
    (2.0 * (k as f32).ln()).sqrt() * 0.65
}

pub fn patches_in_bbox(
    bbox: (u32, u32, u32, u32),
    grid_rows: usize,
    grid_cols: usize,
    orig_w: u32,
    orig_h: u32,
) -> Vec<usize> {
    let mut out = Vec::new();
    if grid_rows == 0 || grid_cols == 0 || orig_w == 0 || orig_h == 0 {
        return out;
    }
    let cw = orig_w as f32 / grid_cols as f32;
    let ch = orig_h as f32 / grid_rows as f32;
    for r in 0..grid_rows {
        for c in 0..grid_cols {
            let cx = (c as f32 + 0.5) * cw;
            let cy = (r as f32 + 0.5) * ch;
            if cx >= bbox.0 as f32 && cx <= bbox.2 as f32 && cy >= bbox.1 as f32 && cy <= bbox.3 as f32 {
                out.push(r * grid_cols + c);
            }
        }
    }
    out
}

pub fn trim_name_residue(field: &str, value: &str) -> Option<String> {
    if !field.ends_with("_name") {
        return None;
    }
    let t = value.trim();
    let cut = t.trim_matches(|c: char| c.is_whitespace() || NAME_RESIDUE_CHARS.contains(&c));
    if cut == t || !cut.chars().any(char::is_alphanumeric) {
        return None;
    }
    Some(cut.to_string())
}

pub fn strip_name_residue(claims: &mut [GroundingClaim], data: &mut Map<String, Value>) -> usize {
    fn fix(node: &mut Value, field: &str, old: &str, new: &str) {
        match node {
            Value::Object(o) => {
                if let Some(Value::String(s)) = o.get_mut(field) {
                    if s.trim() == old {
                        *s = new.to_string();
                    }
                }
            }
            Value::Array(rows) => rows.iter_mut().for_each(|r| fix(r, field, old, new)),
            _ => {}
        }
    }
    let mut trimmed = 0usize;
    for c in claims.iter_mut() {
        let Some(cut) = trim_name_residue(&c.field, &c.value) else { continue };
        let old = c.value.trim().to_string();
        if let Some(Value::String(s)) = data.get_mut(&c.field) {
            if s.trim() == old {
                *s = cut.clone();
            }
        }
        for node in data.values_mut() {
            fix(node, &c.field, &old, &cut);
        }
        c.value = cut;
        trimmed += 1;
    }
    trimmed
}

/// Expands `bbox` by 15% on each side when an adjacent patch contains `Legible` ink.
pub fn padded_retry_bbox(
    bbox: (u32, u32, u32, u32),
    legibility: &LegibilityMap,
    orig_w: u32,
    orig_h: u32,
) -> Option<(u32, u32, u32, u32)> {
    if !legibility.has_legible_neighbor(bbox, orig_w, orig_h) {
        return None;
    }
    let w = bbox.2.saturating_sub(bbox.0) as f32;
    let h = bbox.3.saturating_sub(bbox.1) as f32;
    let px = (w * PADDED_RETRY_FRAC).ceil().max(4.0) as u32;
    let py = (h * PADDED_RETRY_FRAC).ceil().max(4.0) as u32;
    Some((
        bbox.0.saturating_sub(px),
        bbox.1.saturating_sub(py),
        bbox.2.saturating_add(px).min(orig_w),
        bbox.3.saturating_add(py).min(orig_h),
    ))
}

/// Verifies extracted claim values against visual patch embeddings, legibility map, lexicon, and blank-surge upper bound.
pub fn verify_claims_v2(
    grid: &DocPatchGrid,
    legibility: &LegibilityMap,
    claims: &[GroundingClaim],
    value_embs: &[Option<Vec<f32>>],
    lexicon: &dyn GroundingLexicon,
    blank_surge_upper_bound: Option<f32>,
) -> Vec<GroundingVerdict> {
    let n = grid.patches.len();
    let mut out = Vec::with_capacity(claims.len());
    for (idx, claim) in claims.iter().enumerate() {
        let v = claim.value.trim();
        if v.is_empty() {
            continue;
        }
        if lexicon.is_printed_label_echo(&claim.field, v) {
            out.push(GroundingVerdict {
                category: claim.category.clone(),
                field: claim.field.clone(),
                value: v.to_string(),
                surprisal_in: 0.0,
                surprisal_out: 0.0,
                top_patch: 0,
                top_legible: true,
                accepted: false,
                gate: VerdictGate::Format,
                reason: "printed label echo blocked".to_string(),
            });
            continue;
        }
        if lexicon.is_exempt(&claim.field, v) {
            out.push(GroundingVerdict {
                category: claim.category.clone(),
                field: claim.field.clone(),
                value: v.to_string(),
                surprisal_in: 1.0,
                surprisal_out: 0.0,
                top_patch: 0,
                top_legible: true,
                accepted: true,
                gate: VerdictGate::Accepted,
                reason: "lexicon exemption".to_string(),
            });
            continue;
        }
        let emb_opt = value_embs.get(idx).and_then(|o| o.as_ref());
        let Some(emb) = emb_opt else {
            out.push(GroundingVerdict {
                category: claim.category.clone(),
                field: claim.field.clone(),
                value: v.to_string(),
                surprisal_in: 0.0,
                surprisal_out: 0.0,
                top_patch: 0,
                top_legible: true,
                accepted: true,
                gate: VerdictGate::Held,
                reason: "missing phrase embedding — held".to_string(),
            });
            continue;
        };
        let sims: Vec<f32> = grid.patches.iter().map(|p| super::cosine(emb, p)).collect();
        let mean = sims.iter().sum::<f32>() / n.max(1) as f32;
        let std = (sims.iter().map(|s| (s - mean) * (s - mean)).sum::<f32>() / n.max(1) as f32).sqrt().max(1e-6);
        let inside = patches_in_bbox(claim.bbox, grid.grid_rows, grid.grid_cols, grid.orig_width, grid.orig_height);
        if inside.is_empty() {
            out.push(GroundingVerdict {
                category: claim.category.clone(),
                field: claim.field.clone(),
                value: v.to_string(),
                surprisal_in: 0.0,
                surprisal_out: 0.0,
                top_patch: 0,
                top_legible: true,
                accepted: true,
                gate: VerdictGate::Held,
                reason: "no patches in bbox — held".to_string(),
            });
            continue;
        }
        let mut max_in = f32::MIN;
        let mut top_patch = inside[0];
        for &i in &inside {
            if sims[i] > max_in {
                max_in = sims[i];
                top_patch = i;
            }
        }
        let mut max_out = f32::MIN;
        let mut n_out = 0usize;
        for (i, &s) in sims.iter().enumerate() {
            if !inside.contains(&i) {
                n_out += 1;
                max_out = max_out.max(s);
            }
        }
        let s_in = (max_in - mean) / std - gumbel_expected_z(inside.len());
        let s_out = if n_out == 0 { f32::MIN } else { (max_out - mean) / std - gumbel_expected_z(n_out) };
        let top_state = legibility.verdict.get(top_patch).copied().unwrap_or(PatchLegibility::Legible);
        let top_legible = top_state == PatchLegibility::Legible;

        let (lg, il, bl) = legibility.count_in_bbox(claim.bbox, grid.orig_width, grid.orig_height);
        let total_cells = (lg + il + bl).max(1) as f32;
        let blank_surge = blank_surge_upper_bound.is_some_and(|b| bl as f32 / total_cells > b && lg < il + bl);

        if s_in <= 0.0 || !top_legible || blank_surge {
            out.push(GroundingVerdict {
                category: claim.category.clone(),
                field: claim.field.clone(),
                value: v.to_string(),
                surprisal_in: s_in,
                surprisal_out: s_out,
                top_patch,
                top_legible,
                accepted: false,
                gate: VerdictGate::Format,
                reason: if blank_surge {
                    "source blank surge outlier".to_string()
                } else if !top_legible {
                    "illegible source patch".to_string()
                } else {
                    "ungrounded in crop".to_string()
                },
            });
        } else {
            out.push(GroundingVerdict {
                category: claim.category.clone(),
                field: claim.field.clone(),
                value: v.to_string(),
                surprisal_in: s_in,
                surprisal_out: s_out,
                top_patch,
                top_legible,
                accepted: true,
                gate: VerdictGate::Accepted,
                reason: "verified".to_string(),
            });
        }
    }
    out
}

/// Applies `lv_sds::matrix::double_center` while giving `ProtectedRole::TitleBlock` rows immunity on `Baseline::Best`.
pub fn bank_neutral_with_titleblock_immunity(
    raw: &[Vec<f32>],
    row_bboxes: &[(u32, u32, u32, u32)],
    page_w: u32,
    page_h: u32,
    baseline: Baseline,
) -> Vec<Vec<f32>> {
    let mut centered = lv_sds::matrix::double_center(raw);
    for (r, &b) in row_bboxes.iter().enumerate() {
        if r < centered.len()
            && classify_protected_role(b, page_w, page_h, baseline) == ProtectedRole::TitleBlock
        {
            centered[r] = raw[r].clone();
        }
    }
    centered
}
