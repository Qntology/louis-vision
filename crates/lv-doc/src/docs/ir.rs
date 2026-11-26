//! Hybrid AST intermediate representation for digital docs files (`HWP5`, `HWPX`, `DOCX`, `XLSX`, `CSV`, `PDF`).

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
    Struck(String),
}

fn default_unit_multiplier() -> i64 {
    1
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
    #[serde(default = "default_unit_multiplier")]
    pub unit_multiplier: i64,
    #[serde(default)]
    pub carried_scale_from_prev: bool,
    #[serde(default)]
    pub headers: Vec<String>,
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
    #[serde(default)]
    pub track_changes: Vec<TrackChange>,
    #[serde(default)]
    pub tables: Vec<TableIr>,
}

fn detect_unit_multiplier(text: &str) -> Option<i64> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("백만 원") || lower.contains("백만원") || lower.contains("1,000,000") {
        return Some(1_000_000);
    }
    if lower.contains("천 원")
        || lower.contains("천원")
        || lower.contains("usd 1,000")
        || lower.contains("krw 1,000")
        || lower.contains("unit: 1,000")
        || lower.contains("단위: 1,000")
    {
        return Some(1_000);
    }
    None
}

impl DocIr {
    pub fn new(format: impl Into<String>, fallback: VisionFallbackNeed) -> Self {
        Self {
            format: format.into(),
            blocks: Vec::new(),
            media: Vec::new(),
            fallback,
            plain_text: String::new(),
            track_changes: Vec::new(),
            tables: Vec::new(),
        }
    }

    /// Populates `track_changes`, `tables`, and multi-page `TableScaleCarry` unit inheritance.
    pub fn finalize(mut self) -> Self {
        let mut all_changes = Vec::new();
        let mut pending_scale: Option<i64> = None;
        let mut prev_cols: Option<usize> = None;
        let mut prev_scale: i64 = 1;
        let mut prev_headers: Vec<String> = Vec::new();

        for block in &mut self.blocks {
            match block {
                DocBlock::Heading { text, .. } => {
                    if let Some(m) = detect_unit_multiplier(text) {
                        pending_scale = Some(m);
                    }
                }
                DocBlock::Paragraph { text, track_changes } => {
                    all_changes.extend(track_changes.iter().cloned());
                    if let Some(m) = detect_unit_multiplier(text) {
                        pending_scale = Some(m);
                    }
                }
                DocBlock::Table(t) => {
                    if t.headers.is_empty() && t.rows > 0 {
                        t.headers = (0..t.cols)
                            .filter_map(|c| t.cell_at(0, c).map(|cell| cell.text.trim().to_string()))
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                    let explicit_cell_scale = t.cells.iter().find_map(|c| detect_unit_multiplier(&c.text));
                    if let Some(m) = explicit_cell_scale.or(pending_scale.take()) {
                        t.unit_multiplier = m;
                    } else if t.unit_multiplier == 1 && prev_cols == Some(t.cols) && prev_scale > 1 {
                        t.unit_multiplier = prev_scale;
                        t.carried_scale_from_prev = true;
                        if t.headers != prev_headers && !prev_headers.is_empty() && t.headers.len() == prev_headers.len() {
                            // Continuation slice retains inherited column semantics if header row was omitted.
                        }
                    }
                    prev_cols = Some(t.cols);
                    prev_scale = t.unit_multiplier;
                    if !t.headers.is_empty() {
                        prev_headers = t.headers.clone();
                    }
                }
            }
        }

        for existing in &self.track_changes {
            if !all_changes.contains(existing) {
                all_changes.push(existing.clone());
            }
        }
        self.track_changes = all_changes;
        self.tables = self
            .blocks
            .iter()
            .filter_map(|b| match b {
                DocBlock::Table(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        self
    }
}
