//! Hybrid AST intermediate representation for digital office files (`HWP5`, `HWPX`, `DOCX`, `XLSX`, `CSV`, `PDF`).

use serde::{Deserialize, Serialize};

/// Per-document or per-page selective vision fallback classification (avoids whole-document rasterization).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VisionFallbackNeed {
    /// Clean digital AST extracted; zero vision model calls required.
    None,
    /// Digital body extracted cleanly, only embedded media/seals (`BinData/*`, `word/media/*`) need `lv-core` vision.
    EmbeddedOnly,
    /// Scanned or image-only document requiring full-page rasterization.
    FullPage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackChange {
    Inserted(String),
    Deleted(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellIr {
    pub row: usize,
    pub col: usize,
    pub row_span: usize,
    pub col_span: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableIr {
    pub rows: usize,
    pub cols: usize,
    pub col_widths: Vec<u32>,
    pub cells: Vec<CellIr>,
}

impl TableIr {
    pub fn cell_at(&self, row: usize, col: usize) -> Option<&CellIr> {
        self.cells.iter().find(|c| row >= c.row && row < c.row + c.row_span && col >= c.col && col < c.col + c.col_span)
    }

    pub fn to_plain_rows(&self) -> Vec<String> {
        (0..self.rows)
            .map(|r| {
                (0..self.cols)
                    .filter_map(|c| {
                        let cell = self.cell_at(r, c)?;
                        (cell.row == r && cell.col == c).then_some(cell.text.trim().to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocBlock {
    Heading {
        level: u8,
        breadcrumb: String,
        text: String,
    },
    Paragraph {
        text: String,
        track_changes: Vec<TrackChange>,
    },
    Table(TableIr),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedMedia {
    pub path: String,
    pub bytes_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocIr {
    pub format: String,
    pub blocks: Vec<DocBlock>,
    pub media: Vec<EmbeddedMedia>,
    pub fallback: VisionFallbackNeed,
    pub plain_text: String,
}
