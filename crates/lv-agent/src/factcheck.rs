//! Korean numeral + Arabic number extraction and exact fact-locking (`factcheck`, D-04).
//!
//! Avoids saturating-integer collisions on huge numbers / long numeric codes by tracking exact
//! `FactValue` (`Scaled(i128)` or `RawDigits(String)`) and provides `(label, value)` pair locking
//! (`unsupported_labeled_facts`) so swapped amounts (e.g. claimed vs approved) are caught.

use std::collections::{BTreeMap, BTreeSet};

const SCALE: i128 = 1000;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum FactValue {
    Scaled(i128),
    RawDigits(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactMention {
    pub text: String,
    pub values: Vec<i64>,
    pub exact: Vec<FactValue>,
}

fn sino_digit(c: char) -> Option<i128> {
    "영일이삼사오육칠팔구".chars().position(|d| d == c).map(|i| i as i128)
}

fn small_unit(c: char) -> Option<i128> {
    match c {
        '십' => Some(10),
        '백' => Some(100),
        '천' => Some(1000),
        _ => None,
    }
}

fn big_unit(c: char) -> Option<i128> {
    match c {
        '만' => Some(10_000),
        '억' => Some(100_000_000),
        '조' => Some(1_000_000_000_000),
        _ => None,
    }
}

/// Extracts numeric mentions from mixed Arabic + Sino-Korean text with checked `i128` arithmetic
/// and non-colliding `RawDigits` fallback for huge numbers / long codes (D-04).
pub fn mentions(text: &str) -> Vec<FactMention> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            let mut val: Option<i128> = Some(0);
            let mut raw_digits = String::new();
            while i < chars.len() {
                if chars[i].is_ascii_digit() {
                    let d = (chars[i] as u8 - b'0') as i128;
                    raw_digits.push(chars[i]);
                    val = val.and_then(|v| v.checked_mul(10)?.checked_add(d));
                    i += 1;
                } else if chars[i] == ',' && i + 3 < chars.len() && chars[i + 1..=i + 3].iter().all(char::is_ascii_digit) {
                    i += 1;
                } else {
                    break;
                }
            }
            let mut scaled = val.and_then(|v| v.checked_mul(SCALE));
            if i < chars.len() && chars[i] == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
                raw_digits.push('.');
                i += 1;
                let mut mul = 100i128;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    raw_digits.push(chars[i]);
                    if mul > 0 {
                        let d = (chars[i] as u8 - b'0') as i128;
                        scaled = scaled.and_then(|s| s.checked_add(d * mul));
                        mul /= 10;
                    }
                    i += 1;
                }
            }
            // Check optional Korean multiplier right after Arabic digits (`20만`, `3억`)
            while i < chars.len() && chars[i].is_whitespace() {
                if big_unit(chars.get(i + 1).copied().unwrap_or(' ')).is_some() {
                    i += 1;
                }
                break;
            }
            if let Some(&ch) = chars.get(i) {
                if let Some(m) = big_unit(ch) {
                    raw_digits.push(ch);
                    scaled = scaled.and_then(|s| s.checked_mul(m));
                    i += 1;
                }
            }
            let txt: String = chars[start..i].iter().collect();
            let (values, exact) = match scaled {
                Some(v) => {
                    let legacy = i64::try_from(v).ok().into_iter().collect();
                    (legacy, vec![FactValue::Scaled(v)])
                }
                None => (Vec::new(), vec![FactValue::RawDigits(raw_digits)]),
            };
            out.push(FactMention { text: txt, values, exact });
        } else if sino_digit(chars[i]).is_some() || small_unit(chars[i]).is_some() {
            let start = i;
            let mut total: Option<i128> = Some(0);
            let mut section: Option<i128> = Some(0);
            let mut cur: i128 = 0;
            let mut count = 0usize;
            while i < chars.len() {
                if let Some(d) = sino_digit(chars[i]) {
                    cur = d;
                    count += 1;
                    i += 1;
                } else if let Some(u) = small_unit(chars[i]) {
                    section = section.and_then(|s| s.checked_add(cur.max(1).checked_mul(u)?));
                    cur = 0;
                    count += 1;
                    i += 1;
                } else if let Some(bu) = big_unit(chars[i]) {
                    section = section.and_then(|s| s.checked_add(cur));
                    total = total.and_then(|t| t.checked_add(section?.max(1).checked_mul(bu)?));
                    section = Some(0);
                    cur = 0;
                    count += 1;
                    i += 1;
                } else {
                    break;
                }
            }
            section = section.and_then(|s| s.checked_add(cur));
            total = total.and_then(|t| t.checked_add(section?));
            if count > 0 && (total.is_none_or(|t| t >= 10) || count >= 2) {
                let txt: String = chars[start..i].iter().collect();
                let scaled = total.and_then(|t| t.checked_mul(SCALE));
                let (values, exact) = match scaled {
                    Some(v) => {
                        let legacy = i64::try_from(v).ok().into_iter().collect();
                        (legacy, vec![FactValue::Scaled(v)])
                    }
                    None => (Vec::new(), vec![FactValue::RawDigits(txt.clone())]),
                };
                out.push(FactMention { text: txt, values, exact });
            } else {
                i = start + 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Extracts `(normalized_label, FactValue)` bindings where a label precedes a numeric mention
/// (e.g. `"청구액: 100,000원"` -> `("청구액", Scaled(100_000_000))`).
pub fn labeled_facts(text: &str) -> BTreeMap<String, BTreeSet<FactValue>> {
    let mut map: BTreeMap<String, BTreeSet<FactValue>> = BTreeMap::new();
    for segment in text.split(|c: char| matches!(c, '\n' | ';' | ',' | '|')) {
        let seg = segment.trim();
        if seg.is_empty() {
            continue;
        }
        let seg_mentions = mentions(seg);
        let Some(first) = seg_mentions.first() else {
            continue;
        };
        if let Some(pos) = seg.find(&first.text) {
            let prefix = seg[..pos]
                .trim()
                .trim_end_matches(|c: char| matches!(c, ':' | '=' | '-' | '·' | ' '));
            let label = prefix
                .split_whitespace()
                .last()
                .unwrap_or("")
                .trim_matches(|c: char| matches!(c, '[' | ']' | '(' | ')' | '"' | '\''))
                .to_ascii_lowercase();
            if label.chars().count() >= 2 {
                let entry = map.entry(label).or_default();
                for fv in &first.exact {
                    entry.insert(fv.clone());
                }
            }
        }
    }
    map
}

/// Checks both exact numeric values (without i64 saturation collisions) and `(label, value)` bindings
/// so swapping values between two labels (e.g. claimed vs approved) is rejected (D-04).
pub fn unsupported_labeled_facts(draft: &str, source: &str) -> Vec<String> {
    let mut bad = unsupported_numbers(draft, source);
    let src_labels = labeled_facts(source);
    if src_labels.is_empty() {
        return bad;
    }
    let dst_labels = labeled_facts(draft);
    for (label, vals) in dst_labels {
        if let Some(allowed) = src_labels.get(&label) {
            for v in vals {
                if !allowed.contains(&v) {
                    bad.push(format!("{label}:{v:?}"));
                }
            }
        }
    }
    bad
}

pub fn unsupported_numbers(draft: &str, source: &str) -> Vec<String> {
    let known: BTreeSet<FactValue> = mentions(source).into_iter().flat_map(|m| m.exact).collect();
    mentions(draft)
        .into_iter()
        .filter(|m| !m.exact.iter().any(|v| known.contains(v)))
        .map(|m| m.text)
        .collect()
}

/// Validates draft numbers & labeled fact pairs against locked source facts; falls back to
/// `template_fallback` on any mismatch.
pub fn draft_or_fallback(draft: &str, source: &str, template_fallback: &str) -> (String, bool, Vec<String>) {
    let bad = unsupported_labeled_facts(draft, source);
    if bad.is_empty() {
        (draft.to_string(), true, bad)
    } else {
        (template_fallback.to_string(), false, bad)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d04_factcheck_blocks_swapped_labels_and_huge_code_collisions() {
        let src = "청구액: 100,000원, 인정액: 80,000원";
        let ok_draft = "청구액: 100,000원, 인정액: 80,000원";
        let swapped = "청구액: 80,000원, 인정액: 100,000원";
        assert!(unsupported_labeled_facts(ok_draft, src).is_empty());
        assert!(!unsupported_labeled_facts(swapped, src).is_empty(), "swapped values must be caught");

        // Huge numbers exceeding i64::MAX must never saturate to the same value
        let big_src = "승인코드 9999999999999999999999999999999999999991";
        let big_bad = "승인코드 9999999999999999999999999999999999999992";
        assert!(!unsupported_numbers(big_bad, big_src).is_empty(), "distinct huge codes must not collide");
    }
}
