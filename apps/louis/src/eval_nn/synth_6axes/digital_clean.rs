//! Axis 1 (`DigitalClean`): dependency-free in-memory `.xlsx`, HWP 5.0 OLE2 (`cfb`), and `.mhtml` builders.

use std::io::{Cursor, Write};

use anyhow::Result;
use cfb::CompoundFile;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

pub(crate) fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[derive(Debug, Clone)]
pub enum SynthXlsxCell {
    Empty,
    Text(String),
    Int(i64),
    Float(f64),
    Formula { expr: String, cached_val: f64 },
}

#[derive(Debug, Clone)]
pub struct SynthXlsxSheet {
    pub name: String,
    pub rows: Vec<Vec<SynthXlsxCell>>,
    pub very_hidden: bool,
}

fn col_letters(mut col_idx: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push((b'A' + (col_idx % 26) as u8) as char);
        if col_idx < 26 {
            break;
        }
        col_idx = (col_idx / 26) - 1;
    }
    out.into_iter().rev().collect()
}

pub fn synth_xlsx_bytes(sheets: &[SynthXlsxSheet]) -> Result<Vec<u8>> {
    let mut cur = Cursor::new(Vec::new());
    let opt = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut zip = ZipWriter::new(&mut cur);

    let mut overrides = String::new();
    let mut wb_sheets = String::new();
    let mut wb_rels = String::new();

    for (idx, sh) in sheets.iter().enumerate() {
        let n = idx + 1;
        overrides.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        ));
        let state_attr = if sh.very_hidden {
            r#" state="veryHidden""#
        } else {
            ""
        };
        wb_sheets.push_str(&format!(
            r#"<sheet name="{}" sheetId="{n}"{state_attr} r:id="rId{n}"/>"#,
            xml_escape(&sh.name)
        ));
        wb_rels.push_str(&format!(
            r#"<Relationship Id="rId{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{n}.xml"/>"#
        ));
    }

    zip.start_file("[Content_Types].xml", opt)?;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>{overrides}</Types>"#
    )?;

    zip.start_file("_rels/.rels", opt)?;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
    )?;

    zip.start_file("xl/workbook.xml", opt)?;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>{wb_sheets}</sheets></workbook>"#
    )?;

    zip.start_file("xl/_rels/workbook.xml.rels", opt)?;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{wb_rels}</Relationships>"#
    )?;

    for (idx, sh) in sheets.iter().enumerate() {
        let n = idx + 1;
        zip.start_file(format!("xl/worksheets/sheet{n}.xml"), opt)?;
        let mut sheet_xml = String::from(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
        );
        for (r_idx, row) in sh.rows.iter().enumerate() {
            let r_num = r_idx + 1;
            sheet_xml.push_str(&format!(r#"<row r="{r_num}">"#));
            for (c_idx, cell) in row.iter().enumerate() {
                let coord = format!("{}{r_num}", col_letters(c_idx));
                match cell {
                    SynthXlsxCell::Empty => {}
                    SynthXlsxCell::Text(s) => sheet_xml.push_str(&format!(
                        r#"<c r="{coord}" t="inlineStr"><is><t>{}</t></is></c>"#,
                        xml_escape(s)
                    )),
                    SynthXlsxCell::Int(v) => {
                        sheet_xml.push_str(&format!(r#"<c r="{coord}"><v>{v}</v></c>"#));
                    }
                    SynthXlsxCell::Float(v) => {
                        sheet_xml.push_str(&format!(r#"<c r="{coord}"><v>{v}</v></c>"#));
                    }
                    SynthXlsxCell::Formula { expr, cached_val } => sheet_xml.push_str(&format!(
                        r#"<c r="{coord}"><f>{}</f><v>{cached_val}</v></c>"#,
                        xml_escape(expr)
                    )),
                }
            }
            sheet_xml.push_str("</row>");
        }
        sheet_xml.push_str("</sheetData></worksheet>");
        zip.write_all(sheet_xml.as_bytes())?;
    }
    zip.finish()?;
    Ok(cur.into_inner())
}

pub fn synth_hwp5_ole_bytes(
    paragraphs: &[&str],
    compress: bool,
    attach_png: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let mut cur = Cursor::new(Vec::new());
    {
        let mut comp = CompoundFile::create(&mut cur)?;
        let mut file_hdr = vec![0u8; 256];
        let sig = b"HWP Document File";
        file_hdr[..sig.len()].copy_from_slice(sig);
        let flags: u32 = if compress { 1 } else { 0 };
        file_hdr[36..40].copy_from_slice(&flags.to_le_bytes());
        comp.create_stream("/FileHeader")?.write_all(&file_hdr)?;

        comp.create_storage("/BodyText")?;
        let mut raw_sec = Vec::new();
        for p in paragraphs {
            let mut rec_payload = vec![11u8, 0];
            rec_payload.extend_from_slice(&[0xAA; 14]);
            for u in p.encode_utf16() {
                rec_payload.extend_from_slice(&u.to_le_bytes());
            }
            let hdr: u32 = 67u32 | ((rec_payload.len() as u32) << 20);
            raw_sec.extend_from_slice(&hdr.to_le_bytes());
            raw_sec.extend_from_slice(&rec_payload);
        }
        let stream_bytes = if compress {
            let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
            enc.write_all(&raw_sec)?;
            enc.finish()?
        } else {
            raw_sec
        };
        comp.create_stream("/BodyText/Section0")?.write_all(&stream_bytes)?;

        if let Some(img) = attach_png {
            comp.create_storage("/BinData")?;
            comp.create_stream("/BinData/BIN0001.png")?.write_all(img)?;
        }
        comp.flush()?;
    }
    Ok(cur.into_inner())
}

pub fn synth_erp_mhtml_bytes(
    selected_status: &str,
    all_options: &[&str],
    table_html: &str,
    embedded_png: Option<&[u8]>,
) -> Vec<u8> {
    let boundary = "----=_LouisAirGapMhtmlBoundary_2026";
    let opts_html: String = all_options
        .iter()
        .map(|&o| {
            let sel = if o == selected_status {
                " selected"
            } else {
                ""
            };
            format!(r#"<option value="{o}"{sel}>{o}</option>"#)
        })
        .collect();
    let img_tag = if embedded_png.is_some() {
        r#"<img src="cid:evid_01"/>"#
    } else {
        ""
    };
    let mut out = format!(
        "MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"{boundary}\"\r\n\r\n\
         --{boundary}\r\nContent-Type: text/html; charset=\"utf-8\"\r\n\r\n\
         <!DOCTYPE html><html><body>\
         <header><nav>사내 SRM 포털 상단 메뉴 | 알림 | 로그아웃</nav></header>\
         <aside>좌측 고정 카테고리 트리</aside>\
         <main><h1>납품 판촉 정산 명세서</h1>\
         <select name=\"settle_state\">{opts_html}</select>\
         {table_html}{img_tag}</main>\
         </body></html>\r\n"
    )
    .into_bytes();

    if let Some(png) = embedded_png {
        out.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Type: image/png\r\nContent-ID: <evid_01>\r\n\
                 Content-Transfer-Encoding: binary\r\n\r\n"
            )
            .as_bytes(),
        );
        out.extend_from_slice(png);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    out
}
