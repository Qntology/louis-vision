//! Axis 1 (`DigitalClean`): dependency-free in-memory `.xlsx`, HWP 5.0 OLE2 (`cfb`), and `.mhtml` builders.

use std::collections::BTreeSet;
use std::io::{Cursor, Write};
use std::time::SystemTime;

use anyhow::Result;
use cfb::CompoundFile;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

fn fixed_zip_opt() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(
            DateTime::from_date_and_time(2026, 11, 26, 0, 0, 0).expect("valid fixed zip dt"),
        )
}

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
    let opt = fixed_zip_opt();
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
            let _ = comp.set_created_time("/BinData", SystemTime::UNIX_EPOCH);
            let _ = comp.set_modified_time("/BinData", SystemTime::UNIX_EPOCH);
        }
        let _ = comp.set_created_time("/BodyText", SystemTime::UNIX_EPOCH);
        let _ = comp.set_modified_time("/BodyText", SystemTime::UNIX_EPOCH);
        let _ = comp.set_created_time("/", SystemTime::UNIX_EPOCH);
        let _ = comp.set_modified_time("/", SystemTime::UNIX_EPOCH);
        comp.flush()?;
    }
    Ok(cur.into_inner())
}

pub fn synth_hwp5_ole2_bytes(
    paragraphs: &[&str],
    compress: bool,
    attach_png: Option<&[u8]>,
) -> Result<Vec<u8>> {
    synth_hwp5_ole_bytes(paragraphs, compress, attach_png)
}

pub fn synth_inline_xlsx_bytes(sheets: &[SynthXlsxSheet]) -> Result<Vec<u8>> {
    synth_xlsx_bytes(sheets)
}

pub fn synth_very_hidden_xlsx_bytes(
    public_sheet: &SynthXlsxSheet,
    hidden_sheet: &SynthXlsxSheet,
) -> Result<Vec<u8>> {
    let mut pub_s = public_sheet.clone();
    pub_s.very_hidden = false;
    let mut hid_s = hidden_sheet.clone();
    hid_s.very_hidden = true;
    synth_xlsx_bytes(&[pub_s, hid_s])
}

/// Builds a KS X 6101 OWPML `.hwpx` package with 1x1 decorative framing box, optional superscript pair, and merged-header table.
pub fn synth_owpml_hwpx_bytes(
    title: &str,
    paragraphs: &[&str],
    merged_rows: &[(&str, &str, &str)],
    sup_pair: Option<(&str, &str)>,
) -> Result<Vec<u8>> {
    let mut sec = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph">"#,
    );
    sec.push_str(&format!(
        r#"<hp:p><hp:run><hp:tbl rowCnt="1" colCnt="1"><hp:tr><hp:tc><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:p><hp:run><hp:t>{}</hp:t></hp:run></hp:p></hp:tc></hp:tr></hp:tbl></hp:run></hp:p>"#,
        xml_escape(title)
    ));
    for p in paragraphs {
        sec.push_str(&format!(r#"<hp:p><hp:run><hp:t>{}</hp:t></hp:run></hp:p>"#, xml_escape(p)));
    }
    if let Some((base, exp)) = sup_pair {
        sec.push_str(&format!(
            r#"<hp:p><hp:run><hp:t>{}</hp:t></hp:run><hp:run><hp:supscript/><hp:t>{}</hp:t></hp:run><hp:run><hp:t> CFU/g</hp:t></hp:run></hp:p>"#,
            xml_escape(base),
            xml_escape(exp)
        ));
    }
    if !merged_rows.is_empty() {
        let row_cnt = merged_rows.len() + 1;
        sec.push_str(&format!(
            r#"<hp:p><hp:run><hp:tbl rowCnt="{row_cnt}" colCnt="3"><hp:tr><hp:tc><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="3" rowSpan="1"/><hp:p><hp:run><hp:t>명세 내역</hp:t></hp:run></hp:p></hp:tc></hp:tr>"#
        ));
        for (idx, &(c0, c1, c2)) in merged_rows.iter().enumerate() {
            let r = idx + 1;
            sec.push_str(&format!(
                r#"<hp:tr><hp:tc><hp:cellAddr colAddr="0" rowAddr="{r}"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:p><hp:run><hp:t>{}</hp:t></hp:run></hp:p></hp:tc><hp:tc><hp:cellAddr colAddr="1" rowAddr="{r}"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:p><hp:run><hp:t>{}</hp:t></hp:run></hp:p></hp:tc><hp:tc><hp:cellAddr colAddr="2" rowAddr="{r}"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:p><hp:run><hp:t>{}</hp:t></hp:run></hp:p></hp:tc></hp:tr>"#,
                xml_escape(c0),
                xml_escape(c1),
                xml_escape(c2)
            ));
        }
        sec.push_str("</hp:tbl></hp:run></hp:p>");
    }
    sec.push_str("</hs:sec>");

    let mut cur = Cursor::new(Vec::new());
    let opt = fixed_zip_opt();
    let mut zip = ZipWriter::new(&mut cur);
    zip.start_file("mimetype", opt.compression_method(CompressionMethod::Stored))?;
    zip.write_all(b"application/hwp+zip")?;
    zip.start_file("Contents/header.xml", opt)?;
    zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" secCnt="1"/>"#)?;
    zip.start_file("Contents/section0.xml", opt)?;
    zip.write_all(sec.as_bytes())?;
    zip.finish()?;
    Ok(cur.into_inner())
}

/// Builds EC376 OOXML `.docx` with 3-`<w:r>` split slot, `<w:del>`, `<w:strike/>`, and 2-page TableScaleCarry.
pub fn synth_ec376_docx_bytes(
    unit_header: &str,
    split_slot_parts: (&str, &str, &str),
    deleted_unit_price: &str,
    struck_old_price: &str,
    p1_row: (&str, &str),
    p2_row: (&str, &str),
) -> Result<Vec<u8>> {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>{}</w:t></w:r></w:p>
    <w:p><w:r><w:t>{}</w:t></w:r><w:r><w:t>{}</w:t></w:r><w:r><w:t>{}</w:t></w:r></w:p>
    <w:p><w:r><w:rPr><w:strike/></w:rPr><w:t>{}</w:t></w:r><w:del w:id="1" w:author="ec376"><w:delText>{}</w:delText></w:del></w:p>
    <w:tbl>
      <w:tr><w:tc><w:p><w:r><w:t>항목</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>금액</w:t></w:r></w:p></w:tc></w:tr>
      <w:tr><w:tc><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc></w:tr>
    </w:tbl>
    <w:p><w:r><w:br w:type="page"/></w:r></w:p>
    <w:tbl>
      <w:tr><w:tc><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc></w:tr>
    </w:tbl>
  </w:body>
</w:document>"#,
        xml_escape(unit_header),
        xml_escape(split_slot_parts.0),
        xml_escape(split_slot_parts.1),
        xml_escape(split_slot_parts.2),
        xml_escape(struck_old_price),
        xml_escape(deleted_unit_price),
        xml_escape(p1_row.0),
        xml_escape(p1_row.1),
        xml_escape(p2_row.0),
        xml_escape(p2_row.1),
    );
    let mut cur = Cursor::new(Vec::new());
    let opt = fixed_zip_opt();
    let mut zip = ZipWriter::new(&mut cur);
    zip.start_file("word/document.xml", opt)?;
    zip.write_all(xml.as_bytes())?;
    zip.finish()?;
    Ok(cur.into_inner())
}

/// Builds legacy ERP `.xls` disguised HTML containing both a decoy bank `<select>` and `payout_hold_code` `<select>`.
pub fn synth_disguised_html_xls_bytes(
    bank_options: &[&str],
    payout_hold_selected: &str,
    payout_hold_options: &[&str],
    rows: &[(&str, &str, &str)],
) -> Vec<u8> {
    let bank_opts: String = bank_options
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            let sel = if i == 0 { " selected" } else { "" };
            format!(r#"<option value="{b}"{sel}>{b}</option>"#)
        })
        .collect();
    let hold_opts: String = payout_hold_options
        .iter()
        .map(|&o| {
            let sel = if o == payout_hold_selected { " selected" } else { "" };
            format!(r#"<option value="{o}"{sel}>{o}</option>"#)
        })
        .collect();
    let body_rows: String = rows
        .iter()
        .map(|&(c0, c1, c2)| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                xml_escape(c0),
                xml_escape(c1),
                xml_escape(c2)
            )
        })
        .collect();
    format!(
        r#"<!DOCTYPE html><html lang="ko"><body>
<header><nav>ERP Legacy SRM | 공지 | 로그아웃</nav></header>
<aside><ul><li>정산관리</li><li>대금지급</li></ul></aside>
<main>
  <select name="bank_catalog">{bank_opts}</select>
  <select name="payout_hold_code">{hold_opts}</select>
  <table id="legacy_xls_grid">
    <thead><tr><th>전표번호</th><th>구분</th><th>금액</th></tr></thead>
    <tbody>{body_rows}</tbody>
  </table>
</main></body></html>"#
    )
    .into_bytes()
}

/// Builds a valid minimal PDF 1.4 with Type0 Identity-H font + exact `beginbfchar` ToUnicode CMap
/// so Hangul false-kerning (`"한 국 정 부"`) and multilingual lines roundtrip through `pdf-extract`.
pub fn synth_false_kerning_pdf_bytes(lines: &[&str]) -> Vec<u8> {
    let mut unique_u16 = BTreeSet::new();
    for line in lines {
        for cu in line.encode_utf16() {
            unique_u16.insert(cu);
        }
    }
    if unique_u16.is_empty() {
        unique_u16.insert(b' ' as u16);
    }

    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n\
         /CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let u16_list: Vec<u16> = unique_u16.into_iter().collect();
    for chunk in u16_list.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for &cu in chunk {
            cmap.push_str(&format!("<{cu:04X}> <{cu:04X}>\n"));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");

    let mut content = String::from("BT\n/F1 12 Tf\n50 740 Td\n");
    for (idx, line) in lines.iter().enumerate() {
        if idx > 0 {
            content.push_str("0 -18 Td\n");
        }
        content.push('<');
        for cu in line.encode_utf16() {
            content.push_str(&format!("{cu:04X}"));
        }
        content.push_str("> Tj\n");
    }
    content.push_str("ET\n");

    let objs: Vec<String> = vec![
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".into(),
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".into(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>\nendobj\n".into(),
        format!("4 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n", content.len(), content),
        "5 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /SynthGothic /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 8 0 R >>\nendobj\n".into(),
        "6 0 obj\n<< /Type /Font /Subtype /CIDFontType2 /BaseFont /SynthGothic /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> /FontDescriptor 7 0 R /DW 1000 >>\nendobj\n".into(),
        "7 0 obj\n<< /Type /FontDescriptor /FontName /SynthGothic /Flags 4 /FontBBox [0 -200 1000 800] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 >>\nendobj\n".into(),
        format!("8 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n", cmap.len(), cmap),
    ];

    let mut pdf = Vec::from(&b"%PDF-1.4\n"[..]);
    let mut offsets = Vec::with_capacity(objs.len() + 1);
    offsets.push(0usize);
    for obj in &objs {
        offsets.push(pdf.len());
        pdf.extend_from_slice(obj.as_bytes());
    }
    let xref_offset = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for &off in &offsets[1..] {
        pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

pub fn synth_standard_xml_bytes(
    root_tag: &str,
    approval_no_24: &str,
    fields: &[(&str, &str)],
) -> Vec<u8> {
    let mut xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><{root}><ApprovalNo>{}</ApprovalNo>"#,
        xml_escape(approval_no_24),
        root = root_tag
    );
    for &(k, v) in fields {
        xml.push_str(&format!("<{k}>{}</{k}>", xml_escape(v)));
    }
    xml.push_str(&format!("</{root_tag}>"));
    xml.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lv_doc::docs::{parse_docs_bytes, sheet_pdf::detect_xlsx_very_hidden_sheets};

    #[test]
    fn all_eight_clean_formats_are_bit_deterministic_and_parse_cleanly() {
        let hwpx1 = synth_owpml_hwpx_bytes("제목", &["본문 A"], &[("H1", "V1", "100")], Some(("10", "4"))).unwrap();
        let hwpx2 = synth_owpml_hwpx_bytes("제목", &["본문 A"], &[("H1", "V1", "100")], Some(("10", "4"))).unwrap();
        assert_eq!(hwpx1, hwpx2);
        let ir_hwpx = parse_docs_bytes("hwpx", &hwpx1).unwrap();
        assert!(ir_hwpx.plain_text.contains("10^4"));

        let hwp1 = synth_hwp5_ole2_bytes(&["한글5 결정론 본문"], true, None).unwrap();
        let hwp2 = synth_hwp5_ole2_bytes(&["한글5 결정론 본문"], true, None).unwrap();
        assert_eq!(hwp1, hwp2);
        assert!(parse_docs_bytes("hwp", &hwp1).unwrap().plain_text.contains("한글5 결정론 본문"));

        let docx1 = synth_ec376_docx_bytes(
            "(단위: 천 원)",
            ("{{claim", "ed_amou", "nt}}"),
            "99,000",
            "88,000",
            ("P1", "100"),
            ("P2", "200"),
        )
        .unwrap();
        let docx2 = synth_ec376_docx_bytes(
            "(단위: 천 원)",
            ("{{claim", "ed_amou", "nt}}"),
            "99,000",
            "88,000",
            ("P1", "100"),
            ("P2", "200"),
        )
        .unwrap();
        assert_eq!(docx1, docx2);
        let ir_docx = parse_docs_bytes("docx", &docx1).unwrap();
        assert_eq!(ir_docx.tables.len(), 2);
        assert!(ir_docx.tables[1].carried_scale_from_prev);

        let xlsx = synth_very_hidden_xlsx_bytes(
            &SynthXlsxSheet {
                name: "OpenSheet".into(),
                rows: vec![vec![SynthXlsxCell::Text("공개".into())]],
                very_hidden: false,
            },
            &SynthXlsxSheet {
                name: "SecretProcess".into(),
                rows: vec![vec![SynthXlsxCell::Text("비닉공정".into())]],
                very_hidden: true,
            },
        )
        .unwrap();
        assert!(parse_docs_bytes("xlsx", &xlsx).unwrap().plain_text.contains("공개"));
        assert_eq!(detect_xlsx_very_hidden_sheets(&xlsx).unwrap(), vec!["SecretProcess".to_string()]);

        let xls = synth_disguised_html_xls_bytes(&["국민은행", "신한은행"], "반품공제보류", &["정상지급", "반품공제보류"], &[("INV-1", "정산", "50,000")]);
        let ir_xls = parse_docs_bytes("xls", &xls).unwrap();
        assert!(ir_xls.plain_text.contains("payout_hold_code=반품공제보류"));

        let pdf = synth_false_kerning_pdf_bytes(&["한 국 정 부 공문서 OK 123"]);
        let ir_pdf = parse_docs_bytes("pdf", &pdf).unwrap();
        assert!(ir_pdf.plain_text.contains("한국정부"), "pdf got: {}", ir_pdf.plain_text);

        let xml = synth_standard_xml_bytes("TaxInvoice", "2026103141000012a9b8c7d6", &[("Amount", "100000")]);
        assert!(parse_docs_bytes("xml", &xml).unwrap().plain_text.contains("2026103141000012a9b8c7d6"));

        let mhtml = synth_erp_mhtml_bytes("반품공제보류", &["정상지급", "반품공제보류"], "<table><tr><td>1</td></tr></table>", None);
        assert!(parse_docs_bytes("mhtml", &mhtml).unwrap().plain_text.contains("반품공제보류"));
    }
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
