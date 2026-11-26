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

/// Canonical trade relay pairs preventing alias self-loops (`INS<->IP`, `CA<->COA`, `PHYTO<->PC`, `BC<->BK`).
pub fn trading_relay_pair(a: &str, b: &str) -> Option<(&'static str, &'static str)> {
    let ua = a.trim().to_ascii_uppercase();
    let ub = b.trim().to_ascii_uppercase();
    if matches!(
        (ua.as_str(), ub.as_str()),
        ("INS", "IP")
            | ("IP", "INS")
            | ("CA", "COA")
            | ("COA", "CA")
            | ("PHYTO", "PC")
            | ("PC", "PHYTO")
            | ("BC", "BK")
            | ("BK", "BC")
    ) {
        return None;
    }
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
