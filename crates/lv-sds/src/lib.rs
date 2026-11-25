//! Layer 0 zero-network SDS facade over `premium-discount-sds` (`+audit`, `+geometry`, `+temporal`)
//! and optional Granite TTM-R3 Candle forecaster (`feature = "granite-ttm"`).

pub use premium_discount_sds::*;

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
}
