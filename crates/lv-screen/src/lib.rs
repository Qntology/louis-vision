//! Layer 2 zero-network screen vision & virtual event camera (`lv-screen`).
//!
//! Absorbs `premium-discount-sensor` (`event_diff.wgsl`, `LogLumSensor`, `RoiTracker`,
//! `naflex::redact_rgb_for_naflex`, `LumaThumb`) and `premium-discount-vision` (`perforate`,
//! `AnchorBank`, `PassGuard`, `ScreenEncoder`), delegating transformer execution to `lv-core::siglip2`,
//! geometry to `lv-sds::geometry`, and gate/calibration evaluation to `premium-discount-eval`.

pub mod bank;
pub mod kind;
pub mod naflex;
pub mod perforate;
pub mod screen;
pub mod sensor;
pub mod thumb;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use candle_core::Device;
use premium_discount_quant::safetensors::SafeTensors;

pub use bank::{Acceptance, AnchorBank, VisionDomainSpec, VisionKind, VisionScore};
pub use lv_core::siglip2::tower::{
    EpochWatch, LayerKv, SparsePass, VisionAbort, VisionTower, WeightFormat,
};
pub use lv_core::siglip2::{layout, text, tower, Layout, PatchGrid};
pub use lv_sds::geometry::{
    classify_flow, BBox, FlowDirection, FlowThresholds, Polar, ScreenGeometry,
};
pub use naflex::{
    patchify_frame, patchify_frame_with, redact_rgb_for_naflex, NaFlexInput, Preprocess, Resample,
};
pub use perforate::{ExpandCfg, PerfPattern, PerfReport, PerforatedCfg};
pub use screen::{PassGuard, ScreenConfig, ScreenEncoder, ScreenReading};
pub use sensor::{
    drop_rois_in_zones, extract_rois, EventDiffParams, EventDiffState, LogLumSensor,
    PurgableSensor, Roi, RoiTracker, TileStats, Track, EVENT_DIFF_PURGE_BINDINGS,
    EVENT_DIFF_WGSL, REPRIME_THRESHOLD_MILLI,
};
pub use thumb::LumaThumb;

pub mod eval_bridge {
    use premium_discount_eval::calib::platt_fit_or_prior;
    use premium_discount_eval::gate_ab::{compare_gate_arms, ArmRunSummary, GateAbVerdict};

    /// Evaluates perforated pass A/B efficiency (`GateAbVerdict`) against full-grid arm using `premium-discount-eval`.
    pub fn evaluate_perforation_ab(perforated_on: &ArmRunSummary, full_off: &ArmRunSummary) -> GateAbVerdict {
        compare_gate_arms(perforated_on, full_off)
    }

    /// Calibrates VisionKind cosine score thresholds safely with small-sample Laplace prior fallback.
    pub fn calibrate_vision_scores(scores: &[f32], hits: &[bool], min_pos: usize) -> Option<(f32, f32)> {
        platt_fit_or_prior(scores, hits, min_pos)
    }
}

pub fn checkpoint(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join("model.safetensors")
    } else {
        path.to_path_buf()
    }
}

pub fn layout_of(path: &Path) -> Layout {
    let f = checkpoint(path);
    SafeTensors::open(std::slice::from_ref(&f))
        .map(|st| Layout::detect(&st))
        .unwrap_or(Layout::RaonVe)
}

pub fn fingerprint(path: &Path) -> String {
    let f = checkpoint(path);
    let size = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
    format!("{}:{size}", layout_of(path).tag())
}

pub fn preprocess_for(path: &Path) -> Preprocess {
    let layout_default = match layout_of(path) {
        Layout::Siglip2Hf => Preprocess::SIGLIP2,
        Layout::RaonVe => Preprocess::RAON_VE,
    };
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    };
    let Ok(text) = std::fs::read_to_string(dir.join("preprocessor_config.json")) else {
        return layout_default;
    };
    let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&text) else {
        return layout_default;
    };
    let triple = |key: &str| -> Option<[f32; 3]> {
        let a = cfg.get(key)?.as_array()?;
        if a.len() != 3 {
            return None;
        }
        Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32])
    };
    Preprocess {
        mean: triple("image_mean").unwrap_or(layout_default.mean),
        std: triple("image_std").unwrap_or(layout_default.std),
        resample: cfg
            .get("resample")
            .and_then(|r| r.as_u64())
            .and_then(Resample::from_pil)
            .unwrap_or(layout_default.resample),
    }
}

pub fn load_vision(path: &Path, device: &Device, format: WeightFormat) -> Result<(VisionTower, f32, f32)> {
    let f = checkpoint(path);
    let st = SafeTensors::open(std::slice::from_ref(&f)).with_context(|| f.display().to_string())?;
    let tower = VisionTower::load(&st, device, format)?;
    let w = layout::Weights::new(&st);
    let scale = w.all_f32("logit_scale")?[0].exp();
    let bias = w.all_f32("logit_bias")?[0];
    Ok((tower, scale, bias))
}

#[cfg(test)]
mod tests {
    use super::eval_bridge::{calibrate_vision_scores, evaluate_perforation_ab};
    use premium_discount_eval::gate_ab::{ArmRunSummary, DetectionCounters};

    #[test]
    fn eval_bridge_measures_perforation_ab_and_calibrates_scores() {
        let on = ArmRunSummary {
            runs: 10,
            done: 10,
            wasted: 2,
            ticks: 100,
            counters: DetectionCounters { tp: 9, fp: 1, fn_: 0, tn: 10 },
        };
        let off = ArmRunSummary {
            runs: 10,
            done: 10,
            wasted: 6,
            ticks: 180,
            counters: DetectionCounters { tp: 9, fp: 1, fn_: 0, tn: 10 },
        };
        let verdict = evaluate_perforation_ab(&on, &off);
        assert!(verdict.passed, "expected gate pass: {:?}", verdict.reasons);
        assert!(verdict.waste_reduction > 0.6);

        let scores = [0.1, 0.2, 0.85, 0.92, 0.15, 0.9];
        let hits = [false, false, true, true, false, true];
        let (a, _b) = calibrate_vision_scores(&scores, &hits, 2).expect("calibrated");
        assert!(a > 0.0, "expected positive slope, got {a}");
    }
}
