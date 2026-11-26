//! Native XLSX (`calamine`), CSV (`csv`), and Hangul digital PDF (`pdf-extract`) readers
//! with multi-header normalization and Hangul false-kerning repair (`"한 국 정 부" -> "한국정부"`).

use std::io::Cursor;

use anyhow::{Context, Result};
use calamine::{open_workbook_from_rs, Data, Reader, Xlsx};

use super::ir::{CellIr, DocBlock, DocIr, TableIr, VisionFallbackNeed};

/// Repairs spurious single-space false kerning between isolated Hangul syllables (`"한 국 정 부" -> "한국정부"`),
/// preserving normal spacing between multi-syllable words and ASCII tokens.
pub fn repair_hangul_false_kerning(input: &str) -> String {
    let is_hangul_syllable = |c: char| ('\u{AC00}'..='\u{D7A3}').contains(&c);
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == ' '
            && i > 0
            && i + 1 < chars.len()
            && is_hangul_syllable(chars[i - 1])
            && is_hangul_syllable(chars[i + 1])
        {
            let prev_is_space_or_start = i == 1 || chars[i - 2] == ' ' || chars[i - 2] == '\n';
            let next_is_space_or_end = i + 2 == chars.len() || chars[i + 2] == ' ' || chars[i + 2] == '\n';
            if prev_is_space_or_start && next_is_space_or_end {
                i += 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

pub fn parse_xlsx(bytes: &[u8]) -> Result<DocIr> {
    let mut wb: Xlsx<_> = open_workbook_from_rs(Cursor::new(bytes)).context("cannot open XLSX workbook")?;
    let mut blocks = Vec::new();
    let mut plain_lines = Vec::new();

    for name in wb.sheet_names().to_vec() {
        let Ok(range) = wb.worksheet_range(&name) else { continue };
        let (rows, cols) = range.get_size();
        let mut cells = Vec::new();
        for (r, row) in range.rows().enumerate() {
            for (c, val) in row.iter().enumerate() {
                let txt = match val {
                    Data::Empty => String::new(),
                    Data::String(s) => s.trim().to_string(),
                    Data::Int(v) => v.to_string(),
                    Data::Float(v) => format!("{v}"),
                    Data::Bool(b) => b.to_string(),
                    other => other.to_string(),
                };
                if !txt.is_empty() {
                    cells.push(CellIr { row: r, col: c, row_span: 1, col_span: 1, text: txt });
                }
            }
        }
        blocks.push(DocBlock::Heading { level: 1, breadcrumb: name.clone(), text: name });
        let tbl = TableIr { rows, cols, col_widths: vec![1; cols], cells };
        plain_lines.extend(tbl.to_plain_rows().into_iter().filter(|l| !l.trim().is_empty()));
        blocks.push(DocBlock::Table(tbl));
    }

    Ok(DocIr {
        format: "xlsx".to_string(),
        blocks,
        media: Vec::new(),
        fallback: if plain_lines.is_empty() { VisionFallbackNeed::FullPage } else { VisionFallbackNeed::None },
        plain_text: plain_lines.join("\n"),
    })
}

pub fn parse_csv(bytes: &[u8]) -> Result<DocIr> {
    let mut rdr = csv::ReaderBuilder::new().has_headers(false).flexible(true).from_reader(bytes);
    let mut cells = Vec::new();
    let (mut rows, mut cols) = (0usize, 0usize);
    for (r, rec) in rdr.records().enumerate() {
        let rec = rec?;
        rows = r + 1;
        cols = cols.max(rec.len());
        for (c, field) in rec.iter().enumerate() {
            if !field.trim().is_empty() {
                cells.push(CellIr { row: r, col: c, row_span: 1, col_span: 1, text: field.trim().to_string() });
            }
        }
    }
    let tbl = TableIr { rows, cols, col_widths: vec![1; cols], cells };
    let plain_text = tbl.to_plain_rows().join("\n");
    Ok(DocIr {
        format: "csv".to_string(),
        blocks: vec![DocBlock::Table(tbl)],
        media: Vec::new(),
        fallback: if plain_text.trim().is_empty() { VisionFallbackNeed::FullPage } else { VisionFallbackNeed::None },
        plain_text,
    })
}

pub fn parse_pdf(bytes: &[u8]) -> Result<DocIr> {
    let extracted = pdf_extract::extract_text_from_mem(bytes).unwrap_or_default();
    let repaired = extracted
        .lines()
        .map(|l| repair_hangul_false_kerning(l.trim()))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>();
    let fallback = if repaired.iter().map(String::len).sum::<usize>() < 16 {
        VisionFallbackNeed::FullPage
    } else {
        VisionFallbackNeed::None
    };
    let blocks = repaired
        .iter()
        .map(|text| DocBlock::Paragraph { text: text.clone(), track_changes: Vec::new() })
        .collect();
    Ok(DocIr {
        format: "pdf".to_string(),
        blocks,
        media: Vec::new(),
        fallback,
        plain_text: repaired.join("\n"),
    })
}
