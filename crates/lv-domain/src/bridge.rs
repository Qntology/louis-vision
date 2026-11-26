//! Backwards-compatible adapter from legacy 7-pack `PackCheckInput` to 18-pack `DomainEvalRequest`.

use crate::registry::{DomainEvalRequest, PackSignals};
use crate::PackCheckInput;

impl From<&PackCheckInput> for DomainEvalRequest {
    fn from(legacy: &PackCheckInput) -> Self {
        Self {
            pack_id: legacy.pack.id().to_string(),
            claimed_total: legacy.claimed_total,
            evidence_total: legacy.evidence_total,
            fields_present: legacy.fields_present.clone(),
            ocr_predicted: legacy.ocr_predicted.clone(),
            ocr_reference: legacy.ocr_reference.clone(),
            signals: PackSignals::default(),
        }
    }
}
