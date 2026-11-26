//! Axis 2 (`DigitalHostile`): Unicode tag smuggling (`U+E0000`), `<w:del>`, `<w:strike>`, `0pt` white text, external beacons, and multi-page tables.

use std::io::{Cursor, Write};

use anyhow::Result;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::digital_clean::xml_escape;

/// 2026 AML.T0068 ASCII Smuggling: splices Plane 14 Unicode tags (`U+E0000..=U+E007F`).
pub fn splice_unicode_tag_smuggling(
    visible_prefix: &str,
    hidden_ascii_cmd: &str,
    visible_suffix: &str,
) -> String {
    let mut out = String::with_capacity(
        visible_prefix.len() + hidden_ascii_cmd.len() * 4 + visible_suffix.len() + 8,
    );
    out.push_str(visible_prefix);
    out.push('\u{E0001}');
    for b in hidden_ascii_cmd.bytes() {
        if (0x20..=0x7E).contains(&b) {
            if let Some(tag_ch) = char::from_u32(0xE0000 + b as u32) {
                out.push(tag_ch);
            }
        }
    }
    out.push('\u{E007F}');
    out.push_str(visible_suffix);
    out
}

/// Builds hostile OOXML `.docx` with `<w:del>` price changes, `0pt` white text, and external `.rels` beacons.
pub fn synth_hostile_docx_bytes(
    visible_body: &str,
    deleted_old_price: Option<&str>,
    inserted_new_price: Option<&str>,
    zero_pt_hidden_prompt: Option<&str>,
    external_beacon_url: Option<&str>,
) -> Result<Vec<u8>> {
    let mut cur = Cursor::new(Vec::new());
    let opt = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut zip = ZipWriter::new(&mut cur);

    let mut body_xml = format!(
        r#"<w:p><w:r><w:t>{}</w:t></w:r></w:p>"#,
        xml_escape(visible_body)
    );
    if let (Some(old_p), Some(new_p)) = (deleted_old_price, inserted_new_price) {
        body_xml.push_str(&format!(
            r#"<w:p><w:r><w:t>공급단가: </w:t></w:r><w:del w:id="1" w:author="vendor"><w:delText>{}</w:delText></w:del><w:ins w:id="2" w:author="vendor"><w:r><w:t>{}</w:t></w:r></w:ins></w:p>"#,
            xml_escape(old_p),
            xml_escape(new_p)
        ));
    }
    if let Some(prompt) = zero_pt_hidden_prompt {
        body_xml.push_str(&format!(
            r#"<w:p><w:r><w:rPr><w:vanish/><w:sz w:val="1"/><w:color w:val="FFFFFF"/></w:rPr><w:t>{}</w:t></w:r></w:p>"#,
            xml_escape(prompt)
        ));
    }
    if external_beacon_url.is_some() {
        body_xml.push_str(r#"<w:p><w:r><w:drawing><wp:docPr id="1" name="px" descr="covert_alt_prompt"/><a:blip xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:link="rId99"/></w:drawing></w:r></w:p>"#);
    }

    zip.start_file("word/document.xml", opt)?;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><w:body>{body_xml}</w:body></w:document>"#
    )?;

    if let Some(url) = external_beacon_url {
        zip.start_file("word/_rels/document.xml.rels", opt)?;
        write!(
            zip,
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId99" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="{}" TargetMode="External"/></Relationships>"#,
            xml_escape(url)
        )?;
    }
    zip.finish()?;
    Ok(cur.into_inner())
}

/// Builds OOXML `.docx` with `<w:strike>`, superscript/subscript runs, and multi-page unit-multiplier carry tables.
pub fn synth_semantic_multipage_docx_bytes(
    scale_header_note: &str,
    struck_old_line: &str,
    valid_p1_val: &str,
    valid_p2_val: &str,
    exponent_base: &str,
    exponent_sup: &str,
) -> Result<Vec<u8>> {
    let mut cur = Cursor::new(Vec::new());
    let opt = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut zip = ZipWriter::new(&mut cur);

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>{}</w:t></w:r></w:p>
    <w:p>
      <w:r><w:rPr><w:strike/></w:rPr><w:t>{}</w:t></w:r>
      <w:r><w:t> 기준 {exponent_base}</w:t></w:r>
      <w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:t>{exponent_sup}</w:t></w:r>
    </w:p>
    <w:tbl>
      <w:tr><w:tc><w:p><w:r><w:t>품명</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>금액</w:t></w:r></w:p></w:tc></w:tr>
      <w:tr><w:tc><w:p><w:r><w:t>1차분</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc></w:tr>
    </w:tbl>
    <w:p><w:r><w:br w:type="page"/></w:r></w:p>
    <w:tbl>
      <w:tr><w:tc><w:p><w:r><w:t>2차분</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc></w:tr>
    </w:tbl>
  </w:body>
</w:document>"#,
        xml_escape(scale_header_note),
        xml_escape(struck_old_line),
        xml_escape(valid_p1_val),
        xml_escape(valid_p2_val)
    );
    zip.start_file("word/document.xml", opt)?;
    zip.write_all(xml.as_bytes())?;
    zip.finish()?;
    Ok(cur.into_inner())
}
