//! Evaluation calibration re-exported directly from `premium_discount_eval::calib`.

pub use premium_discount_eval::calib::{
    auroc, average_precision, ece, fit_label, platt_fit, platt_fit_or_prior, threshold_for_fpr,
    threshold_for_precision, CalibTable, LabelCalib, ECE_BINS,
};
