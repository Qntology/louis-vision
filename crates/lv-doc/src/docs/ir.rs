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
pub struct CellTrackChange {
    pub row: usize,
    pub col: usize,
    pub change: TrackChange,
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
    /// Cell-scoped track changes (`<w:del>`, `<w:strike>`, HWPX strikeout, HTML `<s>`/`<del>`) with `(row, col)` (D-06).
    #[serde(default)]
    pub cell_changes: Vec<CellTrackChange>,
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
    /// Hidden / veryHidden sheet names excluded from visible text (D-05).
    #[serde(default)]
    pub hidden_sheets: Vec<String>,
    /// Count of hidden / vanish / `<= 1pt` runs excluded during parsing (D-05).
    #[serde(default)]
    pub hidden_run_count: usize,
    /// Per-page vision fallback classification for multi-page formats such as PDF (D-09).
    #[serde(default)]
    pub page_fallbacks: Vec<VisionFallbackNeed>,
}

/// Contextual unit-scale parser (D-07):
/// - Recognizes scale multipliers ONLY in explicit unit declarations (`단위:`, `unit:`, parenthetical
///   unit cues, or currency+unit phrases) so a raw monetary cell like `"1,000,000"` never corrupts scale.
/// - Explicitly detects reset declarations (`단위: 원`, `(원)`, `unit: krw`, `단위: 1원`) -> `Some(1)`.
fn detect_unit_multiplier(text: &str) -> Option<i64> {
    let lower = text.to_ascii_lowercase();
    let has_unit_ctx = lower.contains("단위")
        || lower.contains("unit:")
        || lower.contains("unit :")
        || (lower.contains('(') && lower.contains(')'))
        || (lower.contains('[') && lower.contains(']'));

    if lower.contains("백만 원")
        || lower.contains("백만원")
        || (has_unit_ctx && (lower.contains("1,000,000") || lower.contains("million")))
    {
        return Some(1_000_000);
    }
    if lower.contains("천 원")
        || lower.contains("천원")
        || lower.contains("usd 1,000")
        || lower.contains("krw 1,000")
        || lower.contains("unit: 1,000")
        || lower.contains("단위: 1,000")
        || lower.contains("단위 : 1,000")
        || (has_unit_ctx && lower.contains("thousand"))
    {
        return Some(1_000);
    }
    // Explicit return to base unit 1 ("단위: 원", "단위 : 원", "(원)", "(단위: 원)", "unit: krw")
    if (lower.contains("단위") && (lower.contains("원") || lower.contains("krw")) && !lower.contains("천") && !lower.contains("백만"))
        || lower.contains("(원)")
        || lower.contains("(단위:원)")
        || lower.contains("unit: krw")
        || lower.contains("unit: usd")
    {
        return Some(1);
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
            hidden_sheets: Vec::new(),
            hidden_run_count: 0,
            page_fallbacks: Vec::new(),
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
                    // D-07: Reset scale carry across sheet/section heading boundaries unless heading specifies a unit.
                    prev_cols = None;
                    prev_scale = 1;
                    prev_headers.clear();
                    pending_scale = detect_unit_multiplier(text);
                }
                DocBlock::Paragraph { text, track_changes } => {
                    all_changes.extend(track_changes.iter().cloned());
                    if let Some(m) = detect_unit_multiplier(text) {
                        pending_scale = Some(m);
                        if m == 1 {
                            prev_scale = 1;
                        }
                    }
                }
                DocBlock::Table(t) => {
                    for cc in &t.cell_changes {
                        all_changes.push(cc.change.clone());
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d07_scale_multiplier_ignores_plain_numbers_and_resets_at_headings_and_won() {
        let mut ir = DocIr::new("test", VisionFallbackNeed::None);
        ir.blocks.push(DocBlock::Paragraph {
            text: "(단위: 천 원)".into(),
            track_changes: Vec::new(),
        });
        ir.blocks.push(DocBlock::Table(TableIr {
            rows: 1,
            cols: 2,
            col_widths: vec![1, 1],
            cells: vec![
                CellIr { row: 0, col: 0, row_span: 1, col_span: 1, text: "항목".into() },
                CellIr { row: 0, col: 1, row_span: 1, col_span: 1, text: "500".into() },
            ],
            unit_multiplier: 1,
            carried_scale_from_prev: false,
            headers: Vec::new(),
            cell_changes: Vec::new(),
        }));
        // Continuation table inherits 1_000
        ir.blocks.push(DocBlock::Table(TableIr {
            rows: 1,
            cols: 2,
            col_widths: vec![1, 1],
            cells: vec![
                CellIr { row: 0, col: 0, row_span: 1, col_span: 1, text: "항목2".into() },
                CellIr { row: 0, col: 1, row_span: 1, col_span: 1, text: "300".into() },
            ],
            unit_multiplier: 1,
            carried_scale_from_prev: false,
            headers: Vec::new(),
            cell_changes: Vec::new(),
        }));
        // Explicit "단위: 원" resets to 1; raw "1,000,000" cell must NOT become 1_000_000 scale!
        ir.blocks.push(DocBlock::Paragraph {
            text: "단위: 원".into(),
            track_changes: Vec::new(),
        });
        ir.blocks.push(DocBlock::Table(TableIr {
            rows: 1,
            cols: 2,
            col_widths: vec![1, 1],
            cells: vec![
                CellIr { row: 0, col: 0, row_span: 1, col_span: 1, text: "금액".into() },
                CellIr { row: 0, col: 1, row_span: 1, col_span: 1, text: "1,000,000".into() },
            ],
            unit_multiplier: 1,
            carried_scale_from_prev: false,
            headers: Vec::new(),
            cell_changes: Vec::new(),
        }));
        // Heading boundary resets carry for next sheet
        ir.blocks.push(DocBlock::Paragraph {
            text: "(단위: 천 원)".into(),
            track_changes: Vec::new(),
        });
        ir.blocks.push(DocBlock::Heading {
            level: 1,
            breadcrumb: "Sheet2".into(),
            text: "Sheet2".into(),
        });
        ir.blocks.push(DocBlock::Table(TableIr {
            rows: 1,
            cols: 2,
            col_widths: vec![1, 1],
            cells: vec![
                CellIr { row: 0, col: 0, row_span: 1, col_span: 1, text: "수량".into() },
                CellIr { row: 0, col: 1, row_span: 1, col_span: 1, text: "1,000,000".into() },
            ],
            unit_multiplier: 1,
            carried_scale_from_prev: false,
            headers: Vec::new(),
            cell_changes: Vec::new(),
        }));
        let out = ir.finalize();
        assert_eq!(out.tables[0].unit_multiplier, 1_000);
        assert_eq!(out.tables[1].unit_multiplier, 1_000);
        assert!(out.tables[1].carried_scale_from_prev);
        assert_eq!(out.tables[2].unit_multiplier, 1, "단위: 원 must reset and raw 1,000,000 must stay 1");
        assert_eq!(out.tables[3].unit_multiplier, 1, "heading boundary must stop carry");
    }
}
