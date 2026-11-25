//! Native `.docx` generator supporting split `{{var}}` run merging and dynamic `<w:tbl>` row injection.

use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use anyhow::Result;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::guard::WriterGuard;

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Flattens fragmented XML text runs within a paragraph string (`</w:t></w:r><w:r><w:t>`),
/// substitutes `{{key}}` template placeholders cleanly, and validates against `guard`.
pub fn fill_docx_template_text(
    fragmented_runs: &[&str],
    vars: &BTreeMap<String, String>,
    table_rows: &[Vec<String>],
    guard: &WriterGuard,
) -> Result<Vec<u8>> {
    let merged = fragmented_runs.concat();
    let mut rendered = merged;
    for (k, v) in vars {
        rendered = rendered.replace(&format!("{{{{{k}}}}}"), v);
    }
    let mut all_text = rendered.clone();
    for r in table_rows {
        all_text.push('\n');
        all_text.push_str(&r.join(" "));
    }
    guard.verify_text(&all_text)?;
    let paragraphs: Vec<&str> = rendered.lines().collect();
    build_docx(&paragraphs, table_rows)
}

pub fn build_docx(paragraphs: &[&str], table_rows: &[Vec<String>]) -> Result<Vec<u8>> {
    let mut doc_xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
    );
    for p in paragraphs {
        doc_xml.push_str("<w:p><w:r><w:t>");
        doc_xml.push_str(&xml_escape(p));
        doc_xml.push_str("</w:t></w:r></w:p>");
    }
    if !table_rows.is_empty() {
        let cols = table_rows.iter().map(Vec::len).max().unwrap_or(1);
        doc_xml.push_str("<w:tbl><w:tblGrid>");
        for _ in 0..cols {
            doc_xml.push_str(r#"<w:gridCol w:w="2400"/>"#);
        }
        doc_xml.push_str("</w:tblGrid>");
        for row in table_rows {
            doc_xml.push_str("<w:tr>");
            for cell in row {
                doc_xml.push_str("<w:tc><w:p><w:r><w:t>");
                doc_xml.push_str(&xml_escape(cell));
                doc_xml.push_str("</w:t></w:r></w:p></w:tc>");
            }
            doc_xml.push_str("</w:tr>");
        }
        doc_xml.push_str("</w:tbl>");
    }
    doc_xml.push_str("</w:body></w:document>");

    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

    let mut cur = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cur);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("[Content_Types].xml", opts)?;
        zip.write_all(content_types.as_bytes())?;
        zip.start_file("_rels/.rels", opts)?;
        zip.write_all(rels.as_bytes())?;
        zip.start_file("word/document.xml", opts)?;
        zip.write_all(doc_xml.as_bytes())?;
        zip.finish()?;
    }
    Ok(cur.into_inner())
}
