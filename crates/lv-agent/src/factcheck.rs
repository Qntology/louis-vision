//! Korean numeral + Arabic number extraction and exact fact-locking (`factcheck`).

use std::collections::BTreeSet;

const SCALE: i64 = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactMention {
    pub text: String,
    pub values: Vec<i64>,
}

fn sino_digit(c: char) -> Option<i64> {
    "영일이삼사오육칠팔구".chars().position(|d| d == c).map(|i| i as i64)
}

fn small_unit(c: char) -> Option<i64> {
    match c {
        '십' => Some(10),
        '백' => Some(100),
        '천' => Some(1000),
        _ => None,
    }
}

fn big_unit(c: char) -> Option<i64> {
    match c {
        '만' => Some(10_000),
        '억' => Some(100_000_000),
        '조' => Some(1_000_000_000_000),
        _ => None,
    }
}

/// Extracts numeric mentions from mixed Arabic + Sino-Korean text (scaled by `1000` for fixed-point exactness).
pub fn mentions(text: &str) -> Vec<FactMention> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            let mut val: i64 = 0;
            while i < chars.len() {
                if chars[i].is_ascii_digit() {
                    val = val.saturating_mul(10).saturating_add((chars[i] as u8 - b'0') as i64);
                    i += 1;
                } else if chars[i] == ',' && i + 3 < chars.len() && chars[i + 1..=i + 3].iter().all(char::is_ascii_digit) {
                    i += 1;
                } else {
                    break;
                }
            }
            let mut scaled = val.saturating_mul(SCALE);
            if i < chars.len() && chars[i] == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
                i += 1;
                let mut mul = 100i64;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    if mul > 0 {
                        scaled = scaled.saturating_add(((chars[i] as u8 - b'0') as i64) * mul);
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
                    scaled = scaled.saturating_mul(m);
                    i += 1;
                }
            }
            let txt: String = chars[start..i].iter().collect();
            out.push(FactMention { text: txt, values: vec![scaled] });
        } else if sino_digit(chars[i]).is_some() || small_unit(chars[i]).is_some() {
            let start = i;
            let mut total = 0i64;
            let mut section = 0i64;
            let mut cur = 0i64;
            let mut count = 0usize;
            while i < chars.len() {
                if let Some(d) = sino_digit(chars[i]) {
                    cur = d;
                    count += 1;
                    i += 1;
                } else if let Some(u) = small_unit(chars[i]) {
                    section = section.saturating_add(cur.max(1).saturating_mul(u));
                    cur = 0;
                    count += 1;
                    i += 1;
                } else if let Some(bu) = big_unit(chars[i]) {
                    section = section.saturating_add(cur);
                    total = total.saturating_add(section.max(1).saturating_mul(bu));
                    section = 0;
                    cur = 0;
                    count += 1;
                    i += 1;
                } else {
                    break;
                }
            }
            section = section.saturating_add(cur);
            total = total.saturating_add(section);
            if count > 0 && (total >= 10 || count >= 2) {
                let txt: String = chars[start..i].iter().collect();
                out.push(FactMention { text: txt, values: vec![total.saturating_mul(SCALE)] });
            } else {
                i = start + 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

pub fn unsupported_numbers(draft: &str, source: &str) -> Vec<String> {
    let known: BTreeSet<i64> = mentions(source).into_iter().flat_map(|m| m.values).collect();
    mentions(draft)
        .into_iter()
        .filter(|m| !m.values.iter().any(|v| known.contains(v)))
        .map(|m| m.text)
        .collect()
}

/// Validates draft numbers against locked source facts; falls back to `template_fallback` on any mismatch.
pub fn draft_or_fallback(draft: &str, source: &str, template_fallback: &str) -> (String, bool, Vec<String>) {
    let bad = unsupported_numbers(draft, source);
    if bad.is_empty() {
        (draft.to_string(), true, bad)
    } else {
        (template_fallback.to_string(), false, bad)
    }
}
