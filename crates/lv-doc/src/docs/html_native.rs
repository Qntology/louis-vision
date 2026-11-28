//! Offline HTML / disguised `.xls` / MHTML AST parser (`feature = "html-pug"`):
//! `HtmlStructuralLayout` (`List` vs `Detail`), `HtmlSelectState`, strike isolation (`TrackChange::Struck`),
//! superscript/subscript normalization (`10^3`, `CO_2`), and MHTML `cid:`/`data:` extraction.

use anyhow::Result;
use scraper::{Html, Node, Selector};
use serde::{Deserialize, Serialize};

use super::ir::{
    CellIr, CellTrackChange, DocBlock, DocIr, EmbeddedMedia, TableIr, TrackChange,
    VisionFallbackNeed,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HtmlStructuralLayout {
    List,
    Detail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlSelectState {
    pub name: String,
    pub selected: Vec<String>,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlListCensus {
    pub layout: HtmlStructuralLayout,
    pub table_count: usize,
    pub max_rows: usize,
    pub max_cols: usize,
    pub selects: Vec<HtmlSelectState>,
}

/// Sniffs whether raw bytes are actually HTML or MHTML disguised under another extension (e.g. legacy ERP `.xls`).
pub fn is_disguised_html_or_mhtml(bytes: &[u8]) -> bool {
    let head = std::str::from_utf8(&bytes[..bytes.len().min(2048)])
        .unwrap_or("")
        .trim_start()
        .to_ascii_lowercase();
    head.starts_with("<!doctype")
        || head.starts_with("<html")
        || head.starts_with("<table")
        || head.starts_with("<?xml")
        || (head.contains("mime-version:") && head.contains("multipart/related"))
        || head.contains("<html")
}

/// Counts structural list-vs-detail cues across parsed tables and extracts selected `<select>` states.
pub fn list_census(html: &str) -> HtmlListCensus {
    let doc = Html::parse_document(html);
    let sel_table = Selector::parse("table").expect("valid selector");
    let sel_tr = Selector::parse("tr").expect("valid selector");
    let sel_cell = Selector::parse("th, td").expect("valid selector");
    let sel_select = Selector::parse("select").expect("valid selector");
    let sel_opt = Selector::parse("option").expect("valid selector");

    let mut table_count = 0usize;
    let mut max_rows = 0usize;
    let mut max_cols = 0usize;
    for tbl in doc.select(&sel_table) {
        table_count += 1;
        let rows: Vec<_> = tbl.select(&sel_tr).collect();
        max_rows = max_rows.max(rows.len());
        for tr in rows {
            max_cols = max_cols.max(tr.select(&sel_cell).count());
        }
    }

    let mut selects = Vec::new();
    for sel in doc.select(&sel_select) {
        let name = sel.value().attr("name").or_else(|| sel.value().attr("id")).unwrap_or("select").to_string();
        let mut selected = Vec::new();
        let mut options = Vec::new();
        for opt in sel.select(&sel_opt) {
            let txt = opt.text().collect::<Vec<_>>().join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
            let val = opt.value().attr("value").unwrap_or(&txt).to_string();
            let label = if txt.is_empty() { val.clone() } else { txt };
            options.push(label.clone());
            if opt.value().attrs().any(|(k, _)| k.eq_ignore_ascii_case("selected")) {
                selected.push(if val.is_empty() { label } else { val });
            }
        }
        if selected.is_empty() {
            if let Some(first) = options.first() {
                selected.push(first.clone());
            }
        }
        selects.push(HtmlSelectState { name, selected, options });
    }

    let layout = if max_rows >= 2 && max_cols >= 3 {
        HtmlStructuralLayout::List
    } else {
        HtmlStructuralLayout::Detail
    };

    HtmlListCensus {
        layout,
        table_count,
        max_rows,
        max_cols,
        selects,
    }
}

fn collect_inline_text(
    node: ego_tree::NodeRef<'_, Node>,
    out: &mut String,
    struck: &mut Vec<TrackChange>,
    in_strike: bool,
) {
    match node.value() {
        Node::Text(t) => {
            let clean = t.split_whitespace().collect::<Vec<_>>().join(" ");
            if clean.is_empty() {
                return;
            }
            if in_strike {
                struck.push(TrackChange::Struck(clean));
            } else {
                if !out.is_empty() && !out.ends_with([' ', '\n', '^', '_']) {
                    out.push(' ');
                }
                out.push_str(&clean);
            }
        }
        Node::Element(el) => {
            let tag = el.name().to_ascii_lowercase();
            if matches!(tag.as_str(), "script" | "style" | "noscript" | "svg") {
                return;
            }
            if tag == "br" {
                out.push(' ');
                return;
            }
            if tag == "sup" {
                let mut inner = String::new();
                for ch in node.children() {
                    collect_inline_text(ch, &mut inner, struck, in_strike);
                }
                let t = inner.trim();
                if !t.is_empty() {
                    out.push('^');
                    out.push_str(t);
                }
                return;
            }
            if tag == "sub" {
                let mut inner = String::new();
                for ch in node.children() {
                    collect_inline_text(ch, &mut inner, struck, in_strike);
                }
                let t = inner.trim();
                if !t.is_empty() {
                    out.push('_');
                    out.push_str(t);
                }
                return;
            }
            let next_strike = in_strike || matches!(tag.as_str(), "s" | "del" | "strike");
            for ch in node.children() {
                collect_inline_text(ch, out, struck, next_strike);
            }
        }
        _ => {
            for ch in node.children() {
                collect_inline_text(ch, out, struck, in_strike);
            }
        }
    }
}

/// Extracts primary HTML document and embedded MIME binary parts (`cid:`, images) from MHTML bytes.
fn unwrap_mhtml(bytes: &[u8]) -> (String, Vec<EmbeddedMedia>) {
    let raw = String::from_utf8_lossy(bytes);
    let lower = raw.to_ascii_lowercase();
    if !lower.contains("mime-version:") && !lower.contains("multipart/related") {
        return (raw.into_owned(), Vec::new());
    }

    let boundary = raw
        .lines()
        .find_map(|line| {
            let ll = line.to_ascii_lowercase();
            if let Some(pos) = ll.find("boundary=") {
                let rest = line[pos + "boundary=".len()..].trim();
                let unquoted = rest.trim_matches('"').split(';').next().unwrap_or("").trim();
                if !unquoted.is_empty() {
                    return Some(unquoted.to_string());
                }
            }
            None
        });

    let Some(bnd) = boundary else {
        return (raw.into_owned(), Vec::new());
    };
    let marker = format!("--{bnd}");
    let mut html_body = String::new();
    let mut media = Vec::new();
    for part in raw.split(&marker) {
        let trimmed = part.trim_start_matches(['\r', '\n']);
        if trimmed.is_empty() || trimmed.starts_with("--") {
            continue;
        }
        let split_at = trimmed
            .find("\r\n\r\n")
            .map(|p| (p, 4usize))
            .or_else(|| trimmed.find("\n\n").map(|p| (p, 2usize)));
        let Some((hdr_end, sep_len)) = split_at else {
            continue;
        };
        let hdr = &trimmed[..hdr_end];
        let body = &trimmed[hdr_end + sep_len..];
        let hdr_low = hdr.to_ascii_lowercase();
        if hdr_low.contains("text/html") && html_body.is_empty() {
            html_body = body.to_string();
        } else if hdr_low.contains("image/") {
            let cid = hdr
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("content-id:"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().trim_matches(['<', '>']).to_string())
                .unwrap_or_else(|| format!("part_{}", media.len()));
            media.push(EmbeddedMedia {
                path: format!("cid:{cid}"),
                bytes_len: body.len(),
            });
        }
    }
    if html_body.is_empty() {
        html_body = raw.into_owned();
    }
    (html_body, media)
}

pub fn parse_html_or_mhtml(bytes: &[u8], format_label: &str) -> Result<DocIr> {
    let (html_str, mut media) = unwrap_mhtml(bytes);
    let census = list_census(&html_str);
    let doc = Html::parse_document(&html_str);

    let sel_img = Selector::parse("img").expect("valid selector");
    for img in doc.select(&sel_img) {
        if let Some(src) = img.value().attr("src") {
            if src.starts_with("cid:") || src.starts_with("data:") {
                if !media.iter().any(|m| m.path == src) {
                    media.push(EmbeddedMedia {
                        path: src.chars().take(64).collect(),
                        bytes_len: src.len(),
                    });
                }
            }
        }
    }

    let mut blocks = Vec::new();
    let mut plain_lines = Vec::new();

    for sel_state in &census.selects {
        if !sel_state.selected.is_empty() {
            let line = format!("{}={}", sel_state.name, sel_state.selected.join(","));
            plain_lines.push(line.clone());
            blocks.push(DocBlock::Paragraph {
                text: line,
                track_changes: Vec::new(),
            });
        }
    }

    // D-06 & D-08: Emit headings, paragraphs, and tables strictly in document order (so unit headers
    // precede their corresponding table) and preserve per-cell `(row, col)` strike/deletion traces.
    let sel_flow = Selector::parse("h1, h2, h3, p, table").expect("valid selector");
    let sel_tr = Selector::parse("tr").expect("valid selector");
    let sel_td = Selector::parse("th, td").expect("valid selector");
    for el in doc.select(&sel_flow) {
        let name = el.value().name();
        if name != "table"
            && el.ancestors().any(|anc| {
                matches!(anc.value(), Node::Element(ae) if ae.name().eq_ignore_ascii_case("table"))
            })
        {
            continue;
        }
        if name == "table" {
            let mut cells = Vec::new();
            let mut rows = 0usize;
            let mut cols = 0usize;
            let mut cell_changes: Vec<CellTrackChange> = Vec::new();
            for (r_idx, tr) in el.select(&sel_tr).enumerate() {
                let mut c_idx = 0usize;
                for td in tr.select(&sel_td) {
                    let cspan = td.value().attr("colspan").and_then(|v| v.parse().ok()).unwrap_or(1).max(1);
                    let rspan = td.value().attr("rowspan").and_then(|v| v.parse().ok()).unwrap_or(1).max(1);
                    let mut cell_txt = String::new();
                    let mut local_struck = Vec::new();
                    collect_inline_text(*td, &mut cell_txt, &mut local_struck, false);
                    for ch in local_struck {
                        cell_changes.push(CellTrackChange {
                            row: r_idx,
                            col: c_idx,
                            change: ch,
                        });
                    }
                    cells.push(CellIr {
                        row: r_idx,
                        col: c_idx,
                        row_span: rspan,
                        col_span: cspan,
                        text: cell_txt.trim().to_string(),
                    });
                    c_idx += cspan;
                }
                cols = cols.max(c_idx);
                rows = rows.max(r_idx + 1);
            }
            if rows > 0 && cols > 0 {
                let t = TableIr {
                    rows,
                    cols,
                    col_widths: vec![1; cols],
                    cells,
                    unit_multiplier: 1,
                    carried_scale_from_prev: false,
                    headers: Vec::new(),
                    cell_changes,
                };
                plain_lines.extend(t.to_plain_rows().into_iter().filter(|l| !l.trim().is_empty()));
                blocks.push(DocBlock::Table(t));
            }
        } else {
            let mut txt = String::new();
            let mut changes = Vec::new();
            collect_inline_text(*el, &mut txt, &mut changes, false);
            let txt = txt.trim().to_string();
            if txt.is_empty() && changes.is_empty() {
                continue;
            }
            if !txt.is_empty() {
                plain_lines.push(txt.clone());
            }
            if name.starts_with('h') {
                let lvl = name[1..].parse::<u8>().unwrap_or(1);
                blocks.push(DocBlock::Heading {
                    level: lvl,
                    breadcrumb: txt.clone(),
                    text: txt,
                });
            } else {
                blocks.push(DocBlock::Paragraph {
                    text: txt,
                    track_changes: changes,
                });
            }
        }
    }

    let fallback = match (plain_lines.is_empty(), media.is_empty()) {
        (true, _) => VisionFallbackNeed::FullPage,
        (false, false) => VisionFallbackNeed::EmbeddedOnly,
        (false, true) => VisionFallbackNeed::None,
    };
    let mut ir = DocIr::new(format_label, fallback);
    ir.blocks = blocks;
    ir.media = media;
    ir.plain_text = plain_lines.join("\n");
    Ok(ir.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_disguised_xls_and_extracts_selected_state_and_strike() {
        let html = br#"<!DOCTYPE html><html><body>
            <select name="state"><option value="hold" selected>hold</option><option value="ok">ok</option></select>
            <p><s>old 18,000</s> new 12,000 CO<sub>2</sub> 10<sup>3</sup></p>
            <table><tr><th>Item</th><th>Qty</th><th>Amt</th></tr><tr><td>A</td><td>10</td><td>120,000</td></tr><tr><td>B</td><td>20</td><td>240,000</td></tr></table>
        </body></html>"#;
        assert!(is_disguised_html_or_mhtml(html));
        let census = list_census(std::str::from_utf8(html).unwrap());
        assert_eq!(census.layout, HtmlStructuralLayout::List);
        assert_eq!(census.selects[0].selected, vec!["hold"]);
        let ir = parse_html_or_mhtml(html, "html-xls").unwrap();
        assert_eq!(ir.tables.len(), 1);
        assert!(ir.plain_text.contains("CO_2"));
        assert!(ir.plain_text.contains("10^3"));
        assert!(ir.track_changes.contains(&TrackChange::Struck("old 18,000".into())));
    }
}
