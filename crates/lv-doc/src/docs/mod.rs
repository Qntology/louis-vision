//! Hybrid digital AST document dispatcher (`parse_docs_bytes` / `parse_docs_file`).

pub mod docx_native;
#[cfg(feature = "html-pug")]
pub mod html_native;
pub mod hwp_native;
pub mod ir;
pub mod sanitize;
pub mod sheet_pdf;

use std::path::Path;

use anyhow::{bail, Result};

pub use ir::{CellIr, DocBlock, DocIr, EmbeddedMedia, TableIr, TrackChange, VisionFallbackNeed};
pub use sanitize::{sanitize_doc_ir, CovertArtifactKind, SanitizedDocIr, UntrustedDocSpan};

pub fn parse_docs_bytes(ext: &str, bytes: &[u8]) -> Result<DocIr> {
    let clean_ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    match clean_ext.as_str() {
        "hwp" => hwp_native::parse_hwp5(bytes),
        "hwpx" => hwp_native::parse_hwpx(bytes),
        "docx" => docx_native::parse_docx(bytes),
        "xlsx" | "xlsm" => sheet_pdf::parse_xlsx(bytes),
        #[cfg(feature = "html-pug")]
        "html" | "htm" => html_native::parse_html_or_mhtml(bytes, "html"),
        #[cfg(feature = "html-pug")]
        "mhtml" | "mht" => html_native::parse_html_or_mhtml(bytes, "mhtml"),
        #[cfg(feature = "html-pug")]
        "xls" if html_native::is_disguised_html_or_mhtml(bytes) => {
            html_native::parse_html_or_mhtml(bytes, "html-xls")
        }
        "xls" => sheet_pdf::parse_xlsx(bytes),
        "csv" => sheet_pdf::parse_csv(bytes),
        "pdf" => sheet_pdf::parse_pdf(bytes),
        "xml" => {
            let s = String::from_utf8_lossy(bytes).into_owned();
            let mut ir = DocIr::new("xml", VisionFallbackNeed::None);
            ir.plain_text = s.clone();
            ir.blocks.push(DocBlock::Paragraph {
                text: s,
                track_changes: Vec::new(),
            });
            Ok(ir.finalize())
        }
        other => bail!("unsupported docs format: {other}"),
    }
}

pub fn parse_docs_file(path: &Path) -> Result<DocIr> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let bytes = std::fs::read(path)?;
    parse_docs_bytes(ext, &bytes)
}
