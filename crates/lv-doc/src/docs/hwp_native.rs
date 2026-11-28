//! Native HWP 5.0 OLE2 (`cfb`) and OWPML `.hwpx` (`zip` + `quick-xml`) AST parsers.

use std::borrow::Cow;
use std::io::{Cursor, Read};

use anyhow::{bail, Context, Result};
use cfb::CompoundFile;
use flate2::read::DeflateDecoder;
use quick_xml::events::Event;
use quick_xml::Reader;
use zip::ZipArchive;

use super::ir::{
    CellIr, CellTrackChange, DocBlock, DocIr, EmbeddedMedia, TableIr, TrackChange,
    VisionFallbackNeed,
};

const HWPTAG_PARA_TEXT: u16 = 67;

/// Parses binary HWP 5.0 OLE2 compound document bytes into `DocIr`.
pub fn parse_hwp5(bytes: &[u8]) -> Result<DocIr> {
    let mut comp = CompoundFile::open(Cursor::new(bytes)).context("not a valid HWP 5.0 OLE container")?;
    let mut compressed = false;
    if comp.exists("FileHeader") {
        let mut hdr = Vec::new();
        comp.open_stream("FileHeader")?.read_to_end(&mut hdr)?;
        if hdr.len() >= 40 {
            let flags = u32::from_le_bytes([hdr[36], hdr[37], hdr[38], hdr[39]]);
            compressed = (flags & 1) != 0;
        }
    }

    let mut section_paths = Vec::new();
    let mut media = Vec::new();
    for entry in comp.walk() {
        let p = entry.path().to_string_lossy().replace('\\', "/");
        if p.starts_with("/BodyText/Section") && entry.is_stream() {
            section_paths.push(p);
        } else if p.starts_with("/BinData/") && entry.is_stream() {
            media.push(EmbeddedMedia { path: p, bytes_len: entry.len() as usize });
        }
    }
    section_paths.sort();

    let mut blocks = Vec::new();
    let mut lines = Vec::new();
    for spath in section_paths {
        let mut raw = Vec::new();
        comp.open_stream(&spath)?.read_to_end(&mut raw)?;
        let data = if compressed {
            let mut dec = DeflateDecoder::new(Cursor::new(&raw));
            let mut out = Vec::new();
            if dec.read_to_end(&mut out).is_ok() { out } else { raw }
        } else {
            raw
        };
        for para in decode_hwp5_section(&data) {
            lines.push(para.clone());
            blocks.push(DocBlock::Paragraph { text: para, track_changes: Vec::new() });
        }
    }

    let fallback = match (lines.is_empty(), media.is_empty()) {
        (true, false) => VisionFallbackNeed::FullPage,
        (false, false) => VisionFallbackNeed::EmbeddedOnly,
        (true, true) => VisionFallbackNeed::FullPage,
        (false, true) => VisionFallbackNeed::None,
    };
    let mut ir = DocIr::new("hwp5", fallback);
    ir.blocks = blocks;
    ir.media = media;
    ir.plain_text = lines.join("\n");
    Ok(ir.finalize())
}

fn decode_hwp5_section(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 4 <= data.len() {
        let hdr = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;
        let tag_id = (hdr & 0x3FF) as u16;
        let mut size = ((hdr >> 20) & 0xFFF) as usize;
        if size == 0xFFF {
            if pos + 4 > data.len() {
                break;
            }
            size = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
            pos += 4;
        }
        if pos + size > data.len() {
            break;
        }
        if tag_id == HWPTAG_PARA_TEXT {
            let text = decode_hwp5_para_text(&data[pos..pos + size]);
            if !text.trim().is_empty() {
                out.push(text);
            }
        }
        pos += size;
    }
    out
}

fn decode_hwp5_para_text(rec: &[u8]) -> String {
    let mut u16s = Vec::with_capacity(rec.len() / 2);
    let mut i = 0usize;
    while i + 1 < rec.len() {
        let code = u16::from_le_bytes([rec[i], rec[i + 1]]);
        i += 2;
        if matches!(code, 1..=3 | 11..=12 | 14..=18 | 21..=23) {
            i = (i + 14).min(rec.len());
            continue;
        }
        if code == 9 {
            u16s.push(b'\t' as u16);
        } else if code == 10 || code == 13 {
            u16s.push(b'\n' as u16);
        } else if code >= 32 {
            u16s.push(code);
        }
    }
    String::from_utf16_lossy(&u16s)
}

/// Parses `.hwpx` OWPML ZIP package into `DocIr`, restoring merged cells and flattening 1x1 fake tables.
pub fn parse_hwpx(bytes: &[u8]) -> Result<DocIr> {
    let mut arc = ZipArchive::new(Cursor::new(bytes)).context("not a valid HWPX ZIP container")?;
    let mut sections = Vec::new();
    let mut media = Vec::new();
    for i in 0..arc.len() {
        let f = arc.by_index(i)?;
        let name = f.name().to_string();
        if name.starts_with("Contents/section") && name.ends_with(".xml") {
            sections.push(name);
        } else if name.starts_with("BinData/") {
            media.push(EmbeddedMedia { path: name, bytes_len: f.size() as usize });
        }
    }
    sections.sort();
    if sections.is_empty() {
        bail!("HWPX contains no Contents/section*.xml");
    }

    let mut blocks = Vec::new();
    for s in sections {
        let mut xml = String::new();
        arc.by_name(&s)?.read_to_string(&mut xml)?;
        blocks.extend(parse_hwpx_section_xml(&xml)?);
    }

    let mut plain_lines = Vec::new();
    for b in &blocks {
        match b {
            DocBlock::Heading { text, .. } | DocBlock::Paragraph { text, .. } => {
                if !text.trim().is_empty() {
                    plain_lines.push(text.clone());
                }
            }
            DocBlock::Table(t) => {
                plain_lines.extend(t.to_plain_rows().into_iter().filter(|l| !l.trim().is_empty()));
            }
        }
    }

    let fallback = match (plain_lines.is_empty(), media.is_empty()) {
        (true, _) => VisionFallbackNeed::FullPage,
        (false, false) => VisionFallbackNeed::EmbeddedOnly,
        (false, true) => VisionFallbackNeed::None,
    };
    let mut ir = DocIr::new("hwpx", fallback);
    ir.blocks = blocks;
    ir.media = media;
    ir.plain_text = plain_lines.join("\n");
    Ok(ir.finalize())
}

fn parse_hwpx_section_xml(xml: &str) -> Result<Vec<DocBlock>> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut blocks = Vec::new();

    let mut in_table = false;
    let mut tbl_rows = 0usize;
    let mut tbl_cols = 0usize;
    let mut cells: Vec<CellIr> = Vec::new();

    let mut in_tc = false;
    let mut cur_row = 0usize;
    let mut cur_col = 0usize;
    let mut cur_rspan = 1usize;
    let mut cur_cspan = 1usize;
    let mut cell_text = String::new();

    let mut in_t = false;
    let mut in_strike = false;
    let mut run_vert: u8 = 0; // 0 = normal, 1 = superscript (^), 2 = subscript (_)
    let mut para_text = String::new();
    let mut para_struck: Vec<TrackChange> = Vec::new();
    let mut cell_struck: Vec<CellTrackChange> = Vec::new();

    let attr_usize = |e: &quick_xml::events::BytesStart<'_>, keys: &[&[u8]]| -> Option<usize> {
        for a in e.attributes().flatten() {
            let local = a.key.local_name();
            if keys.iter().any(|k| *k == local.as_ref()) {
                if let Ok(s) = std::str::from_utf8(&a.value) {
                    if let Ok(v) = s.trim().parse::<usize>() {
                        return Some(v);
                    }
                }
            }
        }
        None
    };

    let attr_str = |e: &quick_xml::events::BytesStart<'_>, key: &[u8]| -> Option<String> {
        for a in e.attributes().flatten() {
            if a.key.local_name().as_ref() == key {
                return std::str::from_utf8(&a.value).ok().map(str::to_string);
            }
        }
        None
    };

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let ln = e.local_name();
                match ln.as_ref() {
                    b"tbl" => {
                        in_table = true;
                        tbl_rows = attr_usize(&e, &[b"rowCnt"]).unwrap_or(0);
                        tbl_cols = attr_usize(&e, &[b"colCnt"]).unwrap_or(0);
                        cells.clear();
                        cell_struck.clear();
                    }
                    b"tc" if in_table => {
                        in_tc = true;
                        cur_row = 0;
                        cur_col = 0;
                        cur_rspan = 1;
                        cur_cspan = 1;
                        cell_text.clear();
                    }
                    b"cellAddr" if in_tc => {
                        if let Some(c) = attr_usize(&e, &[b"colAddr"]) {
                            cur_col = c;
                        }
                        if let Some(r) = attr_usize(&e, &[b"rowAddr"]) {
                            cur_row = r;
                        }
                    }
                    b"cellSpan" if in_tc => {
                        cur_cspan = attr_usize(&e, &[b"colSpan"]).unwrap_or(1).max(1);
                        cur_rspan = attr_usize(&e, &[b"rowSpan"]).unwrap_or(1).max(1);
                    }
                    b"p" if !in_tc => {
                        para_text.clear();
                        para_struck.clear();
                    }
                    b"run" => {
                        in_strike = false;
                        run_vert = 0;
                    }
                    b"strikeout" | b"strike" => in_strike = true,
                    b"supscript" => run_vert = 1,
                    b"subscript" => run_vert = 2,
                    b"vertAlign" => {
                        if let Some(val) = attr_str(&e, b"val") {
                            let up = val.trim().to_ascii_uppercase();
                            run_vert = match up.as_str() {
                                "SUPERSCRIPT" => 1,
                                "SUBSCRIPT" => 2,
                                _ => 0,
                            };
                        }
                    }
                    b"t" => in_t = true,
                    _ => {}
                }
            }
            Ok(Event::Text(e)) if in_t => {
                let raw = String::from_utf8_lossy(e.as_ref());
                let s: Cow<'_, str> = match run_vert {
                    1 if !raw.trim().is_empty() => Cow::Owned(format!("^{}", raw.trim())),
                    2 if !raw.trim().is_empty() => Cow::Owned(format!("_{}", raw.trim())),
                    _ => raw,
                };
                if in_strike {
                    let t = s.trim();
                    if !t.is_empty() {
                        if in_tc {
                            cell_struck.push(CellTrackChange {
                                row: cur_row,
                                col: cur_col,
                                change: TrackChange::Struck(t.to_string()),
                            });
                        } else {
                            para_struck.push(TrackChange::Struck(t.to_string()));
                        }
                    }
                } else if in_tc {
                    cell_text.push_str(&s);
                } else {
                    para_text.push_str(&s);
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"t" => in_t = false,
                b"strikeout" | b"strike" => in_strike = false,
                b"supscript" | b"subscript" | b"charPr" => run_vert = 0,
                b"run" => {
                    in_strike = false;
                    run_vert = 0;
                }
                b"p" => {
                    if in_tc {
                        if !cell_text.ends_with('\n') && !cell_text.is_empty() {
                            cell_text.push('\n');
                        }
                    } else if !para_text.trim().is_empty() || !para_struck.is_empty() {
                        blocks.push(DocBlock::Paragraph {
                            text: para_text.trim().to_string(),
                            track_changes: std::mem::take(&mut para_struck),
                        });
                        para_text.clear();
                    }
                }
                b"tc" if in_table => {
                    tbl_rows = tbl_rows.max(cur_row + cur_rspan);
                    tbl_cols = tbl_cols.max(cur_col + cur_cspan);
                    cells.push(CellIr {
                        row: cur_row,
                        col: cur_col,
                        row_span: cur_rspan,
                        col_span: cur_cspan,
                        text: cell_text.trim().to_string(),
                    });
                    in_tc = false;
                }
                b"tbl" => {
                    in_table = false;
                    // Flatten 1x1 decorative framing box into regular paragraphs.
                    if tbl_rows == 1 && tbl_cols == 1 && cells.len() == 1 {
                        let flat_changes: Vec<TrackChange> =
                            std::mem::take(&mut cell_struck).into_iter().map(|c| c.change).collect();
                        let lines: Vec<String> = cells[0]
                            .text
                            .lines()
                            .filter(|l| !l.trim().is_empty())
                            .map(|l| l.trim().to_string())
                            .collect();
                        if lines.is_empty() && !flat_changes.is_empty() {
                            blocks.push(DocBlock::Paragraph {
                                text: String::new(),
                                track_changes: flat_changes,
                            });
                        } else {
                            for (idx, line) in lines.into_iter().enumerate() {
                                blocks.push(DocBlock::Paragraph {
                                    text: line,
                                    track_changes: if idx == 0 { flat_changes.clone() } else { Vec::new() },
                                });
                            }
                        }
                    } else {
                        blocks.push(DocBlock::Table(TableIr {
                            rows: tbl_rows,
                            cols: tbl_cols,
                            col_widths: vec![1; tbl_cols],
                            cells: std::mem::take(&mut cells),
                            unit_multiplier: 1,
                            carried_scale_from_prev: false,
                            headers: Vec::new(),
                            cell_changes: std::mem::take(&mut cell_struck),
                        }));
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(err) => bail!("HWPX XML parse error: {err}"),
            _ => {}
        }
        buf.clear();
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    #[test]
    fn parses_hwpx_flattens_1x1_preserves_merged_cells_and_superscript() {
        let sec = r#"<?xml version="1.0" encoding="UTF-8"?>
<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph">
  <hp:p><hp:run><hp:tbl rowCnt="1" colCnt="1"><hp:tr><hp:tc><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:p><hp:run><hp:t>장식표 제목</hp:t></hp:run></hp:p></hp:tc></hp:tr></hp:tbl></hp:run></hp:p>
  <hp:p><hp:run><hp:t>농도 1.2 x 10</hp:t></hp:run><hp:run><hp:supscript/><hp:t>4</hp:t></hp:run><hp:run><hp:t> CFU/g</hp:t></hp:run></hp:p>
  <hp:p><hp:run><hp:tbl rowCnt="2" colCnt="2">
    <hp:tr><hp:tc><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="2" rowSpan="1"/><hp:p><hp:run><hp:t>합계</hp:t></hp:run></hp:p></hp:tc></hp:tr>
    <hp:tr><hp:tc><hp:cellAddr colAddr="0" rowAddr="1"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:p><hp:run><hp:t>A</hp:t></hp:run></hp:p></hp:tc><hp:tc><hp:cellAddr colAddr="1" rowAddr="1"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:p><hp:run><hp:t>500</hp:t></hp:run></hp:p></hp:tc></hp:tr>
  </hp:tbl></hp:run></hp:p>
</hs:sec>"#;
        let mut cur = Cursor::new(Vec::new());
        {
            let mut zw = ZipWriter::new(&mut cur);
            zw.start_file("Contents/section0.xml", SimpleFileOptions::default()).unwrap();
            zw.write_all(sec.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        let ir = parse_hwpx(&cur.into_inner()).unwrap();
        assert!(ir.plain_text.contains("장식표 제목"));
        assert!(ir.plain_text.contains("10^4"));
        assert_eq!(ir.tables.len(), 1);
        assert_eq!(ir.tables[0].cells[0].col_span, 2);
    }
}
