//! Trade, customs LC (UCP 600 / ISBP 821), bonded warehouse, and hazmat schemas (`DOM-08` / `DOM-09`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogisticsCodeScheme {
    Iso6346Container,
    HsCode10,
    UnHazmat4,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeDiscrepancyInput {
    pub po_amount: i64,
    pub ci_amount: i64,
    pub lc_limit: i64,
    pub bl_ship_ymd: u32,
    pub lc_latest_ship_ymd: u32,
    pub pl_packages: u32,
    pub bl_packages: u32,
    pub certificate_of_origin_missing: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WarehouseCheckInput {
    pub container_code: String,
    pub bonded_release_verified: bool,
    pub un_hazmat_code: Option<String>,
    pub msds_matched: bool,
    pub vent_loto_complete: bool,
}

/// Maps a canonical trade document code to its reference field name (D-13 single source of truth).
pub fn trade_reference_field_of(doc_type: &str) -> Option<&'static str> {
    match doc_type.trim().to_ascii_uppercase().as_str() {
        "PO" => Some("reference_po"),
        "LC" => Some("reference_lc"),
        "CI" => Some("reference_invoice"),
        "PL" => Some("reference_packing"),
        "BL" => Some("reference_bl"),
        "BC" | "BK" => Some("reference_booking"),
        "CO" => Some("reference_origin"),
        "CNM" => Some("reference_non_manipulation"),
        "CA" | "COA" => Some("reference_analysis"),
        "PHYTO" | "PC" => Some("reference_phyto"),
        "INS" | "IP" => Some("reference_policy"),
        _ => None,
    }
}

/// Unified `(my_field, peer_field)` trade relay pair preventing alias self-loops
/// (`INS<->IP`, `CA<->COA`, `PHYTO<->PC`, `BC<->BK`) where `mine == foreign` (D-13).
pub fn trading_relay_pair_fields(from_type: &str, to_type: &str) -> Option<(&'static str, &'static str)> {
    if from_type.trim().eq_ignore_ascii_case(to_type.trim()) {
        return None;
    }
    let mine = trade_reference_field_of(to_type)?;
    let foreign = trade_reference_field_of(from_type)?;
    (mine != foreign).then_some((mine, foreign))
}

/// Canonical trade relay pairs preventing alias self-loops (`INS<->IP`, `CA<->COA`, `PHYTO<->PC`, `BC<->BK`).
pub fn trading_relay_pair(a: &str, b: &str) -> Option<(&'static str, &'static str)> {
    let _ = trading_relay_pair_fields(a, b)?;
    let ua = a.trim().to_ascii_uppercase();
    let ub = b.trim().to_ascii_uppercase();
    match (ua.as_str(), ub.as_str()) {
        ("PO", "CI") | ("CI", "PO") => Some(("PO", "CI")),
        ("CI", "BL") | ("BL", "CI") => Some(("CI", "BL")),
        ("BL", "LC") | ("LC", "BL") => Some(("BL", "LC")),
        ("LC", "PO") | ("PO", "LC") => Some(("LC", "PO")),
        ("BL", "PL") | ("PL", "BL") => Some(("BL", "PL")),
        ("CI", "PL") | ("PL", "CI") => Some(("CI", "PL")),
        _ => None,
    }
}
