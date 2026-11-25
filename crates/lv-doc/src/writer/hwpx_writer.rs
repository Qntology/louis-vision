//! Native OWPML `.hwpx` builder (`mimetype`, `META-INF/container.xml`, `Contents/header.xml`, `Contents/section0.xml`).

use std::io::{Cursor, Write};

use anyhow::Result;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::docx_writer::xml_escape;
use super::guard::WriterGuard;

#[derive(Debug, Clone, Copy)]
pub struct NoticeSpec<'a> {
    pub title: &'a str,
    pub body: &'a str,
}

/// Builds a valid OWPML `.hwpx` archive with title, body paragraphs, optional tables, and Gate 4 validation.
pub fn build_hwpx(
    notice: &NoticeSpec<'_>,
    table_rows: &[Vec<String>],
    guard: &WriterGuard,
) -> Result<Vec<u8>> {
    let mut check_text = format!("{}\n{}", notice.title, notice.body);
    for row in table_rows {
        check_text.push('\n');
        check_text.push_str(&row.join(" "));
    }
    guard.verify_text(&check_text)?;

    let mut sec = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph">"#,
    );
    for line in std::iter::once(notice.title).chain(notice.body.lines()) {
        sec.push_str("<hp:p><hp:run><hp:t>");
        sec.push_str(&xml_escape(line));
        sec.push_str("</hp:t></hp:run></hp:p>");
    }
    if !table_rows.is_empty() {
        let cols = table_rows.iter().map(Vec::len).max().unwrap_or(1);
        sec.push_str(&format!(r#"<hp:tbl rowCnt="{}" colCnt="{}">"#, table_rows.len(), cols));
        for (r_idx, row) in table_rows.iter().enumerate() {
            sec.push_str("<hp:tr>");
            for (c_idx, cell) in row.iter().enumerate() {
                sec.push_str(&format!(
                    r#"<hp:tc><hp:cellAddr colAddr="{c_idx}" rowAddr="{r_idx}"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:p><hp:run><hp:t>{}</hp:t></hp:run></hp:p></hp:tc>"#,
                    xml_escape(cell)
                ));
            }
            sec.push_str("</hp:tr>");
        }
        sec.push_str("</hp:tbl>");
    }
    sec.push_str("</hs:sec>");

    let container = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="Contents/content.hpf" media-type="application/hwpml-package+xml"/></rootfiles></container>"#;
    let header = r#"<?xml version="1.0" encoding="UTF-8"?><hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" version="1.4"/>"#;
    let content_hpf = r#"<?xml version="1.0" encoding="UTF-8"?><opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:manifest><opf:item id="header" href="Contents/header.xml"/><opf:item id="section0" href="Contents/section0.xml"/></opf:manifest></opf:package>"#;

    let mut cur = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cur);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("mimetype", stored)?;
        zip.write_all(b"application/hwp+zip")?;
        zip.start_file("META-INF/container.xml", deflated)?;
        zip.write_all(container.as_bytes())?;
        zip.start_file("Contents/content.hpf", deflated)?;
        zip.write_all(content_hpf.as_bytes())?;
        zip.start_file("Contents/header.xml", deflated)?;
        zip.write_all(header.as_bytes())?;
        zip.start_file("Contents/section0.xml", deflated)?;
        zip.write_all(sec.as_bytes())?;
        zip.finish()?;
    }
    Ok(cur.into_inner())
}
