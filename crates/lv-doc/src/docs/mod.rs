//! Hybrid digital AST document dispatcher (`parse_docs_bytes` / `parse_docs_file`).

pub mod docx_native;
pub mod hwp_native;
pub mod ir;
pub mod sheet_pdf;

use std::path::Path;

use anyhow::{bail, Result};

pub use ir::{CellIr, DocBlock, DocIr, EmbeddedMedia, TableIr, TrackChange, VisionFallbackNeed};

pub fn parse_docs_bytes(ext: &str, bytes: &[u8]) -> Result<DocIr> {
    match ext.trim().trim_start_matches('.').to_ascii_lowercase().as_str() {
        "hwp" => hwp_native::parse_hwp5(bytes),
        "hwpx" => hwp_native::parse_hwpx(bytes),
        "docx" => docx_native::parse_docx(bytes),
        "xlsx" | "xlsm" => sheet_pdf::parse_xlsx(bytes),
        "csv" => sheet_pdf::parse_csv(bytes),
        "pdf" => sheet_pdf::parse_pdf(bytes),
        other => bail!("unsupported docs format: {other}"),
    }
}

pub fn parse_docs_file(path: &Path) -> Result<DocIr> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let bytes = std::fs::read(path)?;
    parse_docs_bytes(ext, &bytes)
}
