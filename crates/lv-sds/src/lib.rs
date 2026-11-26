//! Layer 0 zero-network SDS facade over `premium-discount-sds` (`+audit`, `+geometry`, `+temporal`)
//! and optional Granite TTM-R3 Candle forecaster (`feature = "granite-ttm"`).

pub use premium_discount_sds::*;

/// Single Source of Truth (Layer 0) for internal risk, trace, and retired-model terms
/// that must never appear in citizen-facing deliverables or outward text.
pub const CITIZEN_FORBIDDEN_TERMS: &[&str] = &[
    "LatticeDB",
    "위장",
    "위장계약",
    "특별감사",
    "고위험",
    "G3_",
    "G4_",
    "G3Violation",
    "확실",
    "96.4%",
    "jev_head",
    "Zoro",
    "gocr",
    "laya",
    "qwen35_trim",
    "마스킹",
];

pub mod audit_chain;
pub mod blockies_haring;
pub mod seed_tree;
pub use audit_chain::{
    AuditChain, AuditEntry, ReflectionFlushGuard, ReflectionLogWriter, ReflectionSyncPolicy,
};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum N2sfGrade {
    #[default]
    Open,
    Sensitive,
    Classified,
}

#[cfg(feature = "granite-ttm")]
pub mod io;
#[cfg(feature = "granite-ttm")]
pub mod ttm;
#[cfg(feature = "granite-ttm")]
mod ttm_predictor;

#[cfg(feature = "granite-ttm")]
pub use ttm::{ttm_device_pref, ttm_parity_check, PadMode, TtmCallStats, TtmFingerprint, TtmForecaster, TtmInputPolicy, TtmOutput};
#[cfg(feature = "granite-ttm")]
pub use ttm_predictor::{CalibratedTtmPredictor, TtmPredictor};

#[cfg(feature = "granite-ttm")]
pub mod ttm_compat {
    use serde::{Deserialize, Serialize};

    pub const HOLDOUT: usize = crate::temporal::HOLDOUT;
    pub const MIN_POINTS: usize = crate::temporal::MIN_POINTS;
    pub const HISTORY_ONLY_MIN_N: u64 = crate::temporal::HISTORY_ONLY_MIN_N;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct TtmEligibility {
        pub real_points: usize,
        pub backtest_points: usize,
        pub padded: usize,
        pub dropped_by_align: usize,
        pub joins: bool,
        pub reason: Option<String>,
    }
}

#[cfg(feature = "granite-ttm")]
pub use ttm_compat::TtmEligibility;

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroize;

    #[test]
    fn reexports_geometry_zeroize_and_temporal_and_audit() {
        let mut bbox = geometry::BBox::new(10.0, 20.0, 30.0, 40.0);
        assert_eq!(bbox.width(), 20.0);
        bbox.zeroize();
        assert_eq!(bbox, geometry::BBox::new(0.0, 0.0, 0.0, 0.0));

        let cal = CalibrationStat::default();
        assert_eq!(cal.factor(), 1.0);

        let y = [0.2, 0.21, 0.19, 0.2, 0.2, 0.21, 0.2];
        let v = temporal::temporal_gate(&y, 0.20, None, &[], &[], &temporal::GateConfig::default());
        assert!(v.passes());
    }

    #[test]
    fn citizen_forbidden_terms_and_audit_chain_are_intact() {
        assert_eq!(CITIZEN_FORBIDDEN_TERMS.len(), 16);
        assert!(CITIZEN_FORBIDDEN_TERMS.contains(&"Zoro"));
        assert!(CITIZEN_FORBIDDEN_TERMS.contains(&"gocr"));
        assert!(CITIZEN_FORBIDDEN_TERMS.contains(&"laya"));
        assert!(CITIZEN_FORBIDDEN_TERMS.contains(&"qwen35_trim"));

        let mut chain = AuditChain::default();
        chain.append(100, "test", "first", None);
        chain.append(200, "test", "second", Some(serde_json::json!({"k": 1})));
        assert!(chain.verify());
        assert_eq!(chain.verify_broken_at(), Ok(()));

        chain.entries_mut()[1].line.msg = "tampered".into();
        assert_eq!(chain.verify_broken_at(), Err(2));

        let dir = std::env::temp_dir().join(format!("lv-sds-reflect-{}", std::process::id()));
        let log_path = dir.join("reflection.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        {
            let guard = ReflectionFlushGuard::open(&log_path, ReflectionSyncPolicy::OsFlush).unwrap();
            let mut chain2 = AuditChain::default();
            let e = chain2.append(300, "flush", "os_flush_ok", None).clone();
            guard.write_entry(&e).unwrap();
            // Even before guard drop, OsFlush persists immediately to OS cache/file.
            let txt = std::fs::read_to_string(&log_path).unwrap();
            assert!(txt.contains("os_flush_ok"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
