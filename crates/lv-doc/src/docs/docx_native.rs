//! Native `.docx` AST parser: `<w:tblGrid>`/`gridSpan`/`vMerge` 2D grid restoration,
//! `outlineLvl` heading breadcrumbs, `<w:ins>`/`<w:del>` track changes, and `word/media/*` detection.

use std::io::{Cursor, Read};

use anyhow::{bail, Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use zip::ZipArchive;

use super::ir::{CellIr, DocBlock, DocIr, EmbeddedMedia, TableIr, TrackChange, VisionFallbackNeed};

pub fn parse_docx(bytes: &[u8]) -> Result<DocIr> {
    let mut arc = ZipArchive::new(Cursor::new(bytes)).context("not a valid DOCX ZIP archive")?;
    let mut media = Vec::new();
    for i in 0..arc.len() {
        let f = arc.by_index(i)?;
        if f.name().starts_with("word/media/") {
            media.push(EmbeddedMedia { path: f.name().to_string(), bytes_len: f.size() as usize });
        }
    }
    let mut xml = String::new();
    arc.by_name("word/document.xml").context("missing word/document.xml")?.read_to_string(&mut xml)?;

    let blocks = parse_docx_xml(&xml)?;
    let mut plain_lines = Vec::new();
    for b in &blocks {
        match b {
            DocBlock::Heading { text, .. } | DocBlock::Paragraph { text, .. } => {
                if !text.trim().is_empty() {
                    plain_lines.push(text.clone());
                }
            }
            DocBlock::Table(t) => plain_lines.extend(t.to_plain_rows().into_iter().filter(|s| !s.trim().is_empty())),
        }
    }
    let fallback = match (plain_lines.is_empty(), media.is_empty()) {
        (true, _) => VisionFallbackNeed::FullPage,
        (false, false) => VisionFallbackNeed::EmbeddedOnly,
        (false, true) => VisionFallbackNeed::None,
    };
    Ok(DocIr { format: "docx".to_string(), blocks, media, fallback, plain_text: plain_lines.join("\n") })
}

fn parse_docx_xml(xml: &str) -> Result<Vec<DocBlock>> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut blocks = Vec::new();
    let mut heading_stack: Vec<(u8, String)> = Vec::new();

    let mut in_tbl = false;
    let mut col_widths: Vec<u32> = Vec::new();
    let mut raw_rows: Vec<Vec<(usize, Option<&'static str>, String)>> = Vec::new();
    let mut in_tr = false;
    let mut cur_row_cells: Vec<(usize, Option<&'static str>, String)> = Vec::new();
    let mut in_tc = false;
    let mut cur_cspan = 1usize;
    let mut cur_vmerge: Option<&'static str> = None;
    let mut tc_text = String::new();

    let mut in_p = false;
    let mut outline_lvl: Option<u8> = None;
    let mut in_ins = false;
    let mut in_del = false;
    let mut in_text_el = false;
    let mut ins_buf = String::new();
    let mut del_buf = String::new();
    let mut p_text = String::new();
    let mut p_changes: Vec<TrackChange> = Vec::new();

    let w_val = |e: &quick_xml::events::BytesStart<'_>| -> Option<String> {
        for a in e.attributes().flatten() {
            if a.key.local_name().as_ref() == b"val" {
                return std::str::from_utf8(&a.value).ok().map(str::to_string);
            }
        }
        None
    };

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                match e.local_name().as_ref() {
                    b"tbl" => {
                        in_tbl = true;
                        col_widths.clear();
                        raw_rows.clear();
                    }
                    b"gridCol" if in_tbl => {
                        for a in e.attributes().flatten() {
                            if a.key.local_name().as_ref() == b"w" {
                                if let Ok(v) = std::str::from_utf8(&a.value).unwrap_or("0").parse::<u32>() {
                                    col_widths.push(v);
                                }
                            }
                        }
                    }
                    b"tr" if in_tbl => {
                        in_tr = true;
                        cur_row_cells.clear();
                    }
                    b"tc" if in_tr => {
                        in_tc = true;
                        cur_cspan = 1;
                        cur_vmerge = None;
                        tc_text.clear();
                    }
                    b"gridSpan" if in_tc => {
                        if let Some(v) = w_val(&e).and_then(|s| s.parse::<usize>().ok()) {
                            cur_cspan = v.max(1);
                        }
                    }
                    b"vMerge" if in_tc => {
                        cur_vmerge = Some(match w_val(&e).as_deref() {
                            Some("restart") => "restart",
                            _ => "continue",
                        });
                    }
                    b"p" if !in_tc => {
                        in_p = true;
                        outline_lvl = None;
                        p_text.clear();
                        p_changes.clear();
                    }
                    b"outlineLvl" if in_p => {
                        if let Some(lvl) = w_val(&e).and_then(|s| s.parse::<u8>().ok()) {
                            outline_lvl = Some(lvl + 1);
                        }
                    }
                    b"ins" if in_p => {
                        in_ins = true;
                        ins_buf.clear();
                    }
                    b"del" if in_p => {
                        in_del = true;
                        del_buf.clear();
                    }
                    b"t" | b"delText" => in_text_el = true,
                    _ => {}
                }
            }
            Ok(Event::Text(e)) if in_text_el => {
                let s = String::from_utf8_lossy(e.as_ref());
                if in_tc {
                    if !in_del {
                        tc_text.push_str(&s);
                    }
                } else if in_del {
                    del_buf.push_str(&s);
                } else if in_ins {
                    ins_buf.push_str(&s);
                    p_text.push_str(&s);
                } else if in_p {
                    p_text.push_str(&s);
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"t" | b"delText" => in_text_el = false,
                b"ins" => {
                    in_ins = false;
                    if !ins_buf.trim().is_empty() {
                        p_changes.push(TrackChange::Inserted(ins_buf.trim().to_string()));
                    }
                }
                b"del" => {
                    in_del = false;
                    if !del_buf.trim().is_empty() {
                        p_changes.push(TrackChange::Deleted(del_buf.trim().to_string()));
                    }
                }
                b"p" => {
                    if in_tc {
                        if !tc_text.is_empty() && !tc_text.ends_with('\n') {
                            tc_text.push('\n');
                        }
                    } else if in_p {
                        in_p = false;
                        let txt = p_text.trim().to_string();
                        if !txt.is_empty() {
                            if let Some(lvl) = outline_lvl {
                                while heading_stack.last().is_some_and(|(l, _)| *l >= lvl) {
                                    heading_stack.pop();
                                }
                                heading_stack.push((lvl, txt.clone()));
                                let breadcrumb = heading_stack.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" > ");
                                blocks.push(DocBlock::Heading { level: lvl, breadcrumb, text: txt });
                            } else {
                                blocks.push(DocBlock::Paragraph {
                                    text: txt,
                                    track_changes: std::mem::take(&mut p_changes),
                                });
                            }
                        }
                    }
                }
                b"tc" if in_tc => {
                    cur_row_cells.push((cur_cspan, cur_vmerge, tc_text.trim().to_string()));
                    in_tc = false;
                }
                b"tr" if in_tr => {
                    raw_rows.push(std::mem::take(&mut cur_row_cells));
                    in_tr = false;
                }
                b"tbl" if in_tbl => {
                    in_tbl = false;
                    blocks.push(DocBlock::Table(normalize_docx_table(&col_widths, &raw_rows)));
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(err) => bail!("DOCX XML parse error: {err}"),
            _ => {}
        }
        buf.clear();
    }
    Ok(blocks)
}

fn normalize_docx_table(col_widths: &[u32], raw_rows: &[Vec<(usize, Option<&'static str>, String)>]) -> TableIr {
    let rows = raw_rows.len();
    let cols = col_widths.len().max(
        raw_rows
            .iter()
            .map(|r| r.iter().map(|(cs, _, _)| *cs).sum::<usize>())
            .max()
            .unwrap_or(0),
    );
    let mut cells: Vec<CellIr> = Vec::new();
    for (r, raw) in raw_rows.iter().enumerate() {
        let mut c = 0usize;
        for (cspan, vmerge, text) in raw {
            if *vmerge == Some("continue") {
                if let Some(prev) = cells.iter_mut().rev().find(|prev| prev.col == c && prev.row + prev.row_span == r) {
                    prev.row_span += 1;
                    if !text.is_empty() && !prev.text.is_empty() {
                        prev.text.push('\n');
                    }
                    prev.text.push_str(text);
                }
            } else {
                cells.push(CellIr { row: r, col: c, row_span: 1, col_span: *cspan, text: text.clone() });
            }
            c += *cspan;
        }
    }
    TableIr {
        rows,
        cols,
        col_widths: if col_widths.len() == cols { col_widths.to_vec() } else { vec![1; cols] },
        cells,
    }
}
