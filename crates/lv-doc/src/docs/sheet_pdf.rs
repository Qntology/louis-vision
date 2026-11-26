//! Native XLSX (`calamine`), CSV (`csv`), and Hangul digital PDF (`pdf-extract`) readers
//! with multi-header normalization and Hangul false-kerning repair (`"한 국 정 부" -> "한국정부"`).

use std::io::{Cursor, Read};

use anyhow::{Context, Result};
use calamine::{open_workbook_from_rs, Data, Reader as CalamineReader, Xlsx};
use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use zip::ZipArchive;

use super::ir::{CellIr, DocBlock, DocIr, TableIr, VisionFallbackNeed};

/// Inspects `xl/workbook.xml` inside `.xlsx`/`.xlsm` bytes and returns sheet names marked `state="veryHidden"`.
pub fn detect_xlsx_very_hidden_sheets(bytes: &[u8]) -> Result<Vec<String>> {
    let mut arc = ZipArchive::new(Cursor::new(bytes)).context("cannot open XLSX zip container")?;
    let mut xml = String::new();
    if let Ok(mut f) = arc.by_name("xl/workbook.xml") {
        f.read_to_string(&mut xml)?;
    } else {
        return Ok(Vec::new());
    }
    let mut reader = XmlReader::from_str(&xml);
    let mut buf = Vec::new();
    let mut hidden = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.local_name().as_ref() == b"sheet" => {
                let mut name: Option<String> = None;
                let mut state: Option<String> = None;
                for attr in e.attributes().flatten() {
                    match attr.key.local_name().as_ref() {
                        b"name" => name = std::str::from_utf8(&attr.value).ok().map(str::to_string),
                        b"state" => state = std::str::from_utf8(&attr.value).ok().map(str::to_string),
                        _ => {}
                    }
                }
                if state
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case("veryHidden"))
                {
                    if let Some(n) = name {
                        hidden.push(n);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(hidden)
}

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
        let tbl = TableIr {
            rows,
            cols,
            col_widths: vec![1; cols],
            cells,
            unit_multiplier: 1,
            carried_scale_from_prev: false,
            headers: Vec::new(),
        };
        plain_lines.extend(tbl.to_plain_rows().into_iter().filter(|l| !l.trim().is_empty()));
        blocks.push(DocBlock::Table(tbl));
    }

    let mut ir = DocIr::new(
        "xlsx",
        if plain_lines.is_empty() {
            VisionFallbackNeed::FullPage
        } else {
            VisionFallbackNeed::None
        },
    );
    ir.blocks = blocks;
    ir.plain_text = plain_lines.join("\n");
    Ok(ir.finalize())
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
    let tbl = TableIr {
        rows,
        cols,
        col_widths: vec![1; cols],
        cells,
        unit_multiplier: 1,
        carried_scale_from_prev: false,
        headers: Vec::new(),
    };
    let plain_text = tbl.to_plain_rows().join("\n");
    let mut ir = DocIr::new(
        "csv",
        if plain_text.trim().is_empty() {
            VisionFallbackNeed::FullPage
        } else {
            VisionFallbackNeed::None
        },
    );
    ir.blocks = vec![DocBlock::Table(tbl)];
    ir.plain_text = plain_text;
    Ok(ir.finalize())
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
    let mut ir = DocIr::new("pdf", fallback);
    ir.blocks = blocks;
    ir.plain_text = repaired.join("\n");
    Ok(ir.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    #[test]
    fn detects_very_hidden_sheets_in_workbook_xml() {
        let mut cur = Cursor::new(Vec::new());
        {
            let mut zw = ZipWriter::new(&mut cur);
            zw.start_file("xl/workbook.xml", SimpleFileOptions::default()).unwrap();
            zw.write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheets><sheet name="PublicSummary" sheetId="1" r:id="rId1"/><sheet name="SecretRecipe" sheetId="2" state="veryHidden" r:id="rId2"/></sheets></workbook>"#,
            )
            .unwrap();
            zw.finish().unwrap();
        }
        let found = detect_xlsx_very_hidden_sheets(&cur.into_inner()).unwrap();
        assert_eq!(found, vec!["SecretRecipe".to_string()]);
    }
}
