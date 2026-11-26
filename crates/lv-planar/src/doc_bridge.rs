//! Direct bridge from rectified 2D patches into `lv-doc::vision` (`Luma=180` gridlines, `TitleBlock` 5/5 protection, `ZZS1`, `ShadowToken`).

use image::{DynamicImage, GrayImage};
use lv_doc::vision::{
    ink::{extract_ink_page, InkPage},
    lattice_verify::{verify_ledger_with_shadow_top3, LatticeVerifyDecision, ShadowToken},
    legibility::{build_legibility_map, LegibilityMap},
    plan::{Baseline, ProtectedRole},
    symbols::SymbolDict,
    table_grid_reverse::inject_virtual_gridlines,
};

#[cfg(feature = "domain")]
use lv_domain::{repair_code, CodeScheme};

use crate::homography::RectifiedPlanePatch;

pub struct PreparedRectifiedDoc {
    pub enhanced_gray: GrayImage,
    pub legibility: LegibilityMap,
    pub ink_page: InkPage,
    pub virtual_cols: usize,
    pub virtual_rows: usize,
    pub has_protected_titleblock: bool,
    pub matched_symbols: Vec<String>,
}

pub fn prepare_rectified_document(
    rect: &RectifiedPlanePatch,
    baseline: Baseline,
    symbols: Option<&SymbolDict>,
) -> Option<PreparedRectifiedDoc> {
    let rgb_img = rect.to_rgb_image()?;
    let dyn_img = DynamicImage::ImageRgb8(rgb_img);
    let raw_gray = dyn_img.to_luma8();

    let grid_rows = (rect.height / 16).clamp(4, 128) as usize;
    let grid_cols = (rect.width / 16).clamp(4, 128) as usize;
    let legibility = build_legibility_map(&dyn_img, grid_rows, grid_cols);

    let (enhanced_gray, virtual_cols, virtual_rows) = match inject_virtual_gridlines(&raw_gray, 2) {
        Some((grid_img, vx, vy)) => (grid_img, vx.len(), vy.len()),
        None => (raw_gray.clone(), 0, 0),
    };

    let ink_page = extract_ink_page(&enhanced_gray);
    let has_protected_titleblock = matches!(baseline, Baseline::Best)
        && rect.width >= 32
        && rect.height >= 32
        && has_ink_in_titleblock_zone(&raw_gray, ProtectedRole::TitleBlock);

    let matched_symbols = symbols
        .map(|dict| dict.match_in_gray(&raw_gray))
        .unwrap_or_default();

    Some(PreparedRectifiedDoc {
        enhanced_gray,
        legibility,
        ink_page,
        virtual_cols,
        virtual_rows,
        has_protected_titleblock,
        matched_symbols,
    })
}

#[inline]
pub fn verify_rectified_ledger_shadow(
    decoded_value: &str,
    shadow_top3: &[[ShadowToken; 3]],
    ledger_candidates: &[&str],
) -> LatticeVerifyDecision {
    let flat_tokens: Vec<ShadowToken> = shadow_top3.iter().flat_map(|g| g.iter().cloned()).collect();
    for &cand in ledger_candidates {
        if let res @ LatticeVerifyDecision::Verified { .. } =
            verify_ledger_with_shadow_top3(decoded_value, &flat_tokens, cand)
        {
            return res;
        }
    }
    LatticeVerifyDecision::RejectedUngrounded {
        reason: "no Shadow Top-3 visual evidence >= 0.08",
    }
}

#[cfg(feature = "domain")]
#[inline]
pub fn repair_surface_identifier(raw: &str, scheme: CodeScheme) -> String {
    repair_code(raw, scheme)
}

fn has_ink_in_titleblock_zone(gray: &GrayImage, _role: ProtectedRole) -> bool {
    let (w, h) = gray.dimensions();
    let x0 = ((w as f32 * 0.60).floor() as u32).min(w.saturating_sub(1));
    let y0 = ((h as f32 * 0.75).floor() as u32).min(h.saturating_sub(1));
    let mut dark_px = 0u32;
    for y in y0..h {
        for x in x0..w {
            if gray.get_pixel(x, y)[0] < 210 {
                dark_px += 1;
                if dark_px >= 8 {
                    return true;
                }
            }
        }
    }
    false
}
