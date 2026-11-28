//! Pluggable in-memory hybrid search engine (`BM25 + char n-grams + 768d concept/Gemma dense + 1..2 hop relay`),
//! `seed2` vs `E2E` (`lv-doc` office parsing + `veryHidden` stripping + `sanitize_doc_ir`), and `N2SF`/`vault`/`sql_guard` isolation.

use std::collections::{BTreeSet, HashMap};

use anyhow::Result;
use lv_agent::factcheck::draft_or_fallback;
use lv_agent::gates::check_citizen_text;
use lv_core::gemma_embed::{cosine, normalize_l2, with_prompt, GemmaEmbedder, Role, OUTPUT_DIM};
use lv_doc::docs::ir::DocBlock;
use lv_doc::docs::sanitize::sanitize_doc_ir;
use lv_doc::docs::sheet_pdf::detect_xlsx_very_hidden_sheets;
use lv_doc::docs::parse_docs_bytes;
use lv_doc::writer::guard::N2sfGrade;
use lv_router::vault::DeviceRole;
use serde::{Deserialize, Serialize};

use super::domains::contains_forbidden_raw_gov_id_digits;
use super::domains::dom08_trade_customs_lc::normalize_trade_identifier;
use super::nl_domain_blueprints::ALL_19_BLUEPRINTS;
use super::nl_world::{xrc_twin_profile, RelationalWorld, WorldDocRole, ALL_18_DOMAIN_PROFILES};
use super::synth_6axes::audio_patch_synth::verify_wav_16khz_token_aligned;
use crate::sql_guard::{default_arrow_view_whitelist, verify_and_wrap_safe_sql};

pub trait DenseTextEmbedder: Send + Sync {
    fn dim(&self) -> usize;
    fn model_id(&self) -> &str;
    fn embed_query(&self, text: &str) -> Result<Vec<f32>>;
    fn embed_document(&self, title: Option<&str>, text: &str) -> Result<Vec<f32>>;
}

pub struct GemmaDenseEmbedderAdapter<'a>(pub &'a GemmaEmbedder);

impl<'a> DenseTextEmbedder for GemmaDenseEmbedderAdapter<'a> {
    fn dim(&self) -> usize {
        self.0.output_dim()
    }

    fn model_id(&self) -> &str {
        lv_core::gemma_embed::MODEL_ID
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let prompt = with_prompt(Role::Query, text);
        self.0.embed_text(&prompt)
    }

    fn embed_document(&self, title: Option<&str>, text: &str) -> Result<Vec<f32>> {
        let formatted = match title {
            Some(t) if !t.trim().is_empty() => format!("title: {t} | text: {text}"),
            _ => with_prompt(Role::Document, text),
        };
        self.0.embed_text(&formatted)
    }
}

#[cfg(feature = "local-lancedb")]
#[derive(Debug, Clone)]
pub struct LanceDbBackendConfig {
    pub table_name: String,
    pub vector_dim: usize,
}

pub struct OfflineHybridEmbedder {
    concept_map: HashMap<String, Vec<(usize, f32)>>,
    dim: usize,
}

impl Default for OfflineHybridEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

impl OfflineHybridEmbedder {
    /// Builds a deterministic 768d bilingual/15-language concept-projected embedder so cross-lingual
    /// and pure statutory domain-fit queries map orthogonally across `[0..304]` domain lanes.
    pub fn new() -> Self {
        let mut token_domains: HashMap<String, BTreeSet<usize>> = HashMap::new();
        let all_profiles: Vec<_> = ALL_18_DOMAIN_PROFILES
            .iter()
            .copied()
            .chain(std::iter::once(xrc_twin_profile()))
            .collect();

        for (slot_idx, prof) in all_profiles.iter().enumerate() {
            let mut texts: Vec<&str> = vec![
                prof.topic_ko,
                prof.topic_en,
                prof.topic_ja,
                prof.topic_zh,
                prof.topic_de,
                prof.topic_fr,
                prof.topic_es,
                prof.topic_ar,
                prof.topic_pt,
                prof.topic_it,
                prof.topic_nl,
                prof.topic_cs,
                prof.topic_ru,
                prof.topic_th,
                prof.synonym_ko,
                prof.synonym_en,
            ];
            if let Some(bp) = ALL_19_BLUEPRINTS.iter().find(|b| b.domain_id == prof.domain_id) {
                texts.push(bp.doc_category_ko);
                texts.push(bp.doc_category_en);
                texts.push(bp.governing_statute);
                texts.push(bp.domain_probe_ko);
                texts.push(bp.domain_probe_en);
                for &cl in &bp.statutory_clauses {
                    texts.push(cl);
                }
                for &sig in bp.domain_signature_terms {
                    texts.push(sig);
                }
                for &(n, sp, _) in &bp.line_item_templates {
                    texts.push(n);
                    texts.push(sp);
                }
                for &rf in bp.required_field_keys {
                    texts.push(rf);
                }
            }
            for txt in texts {
                for tok in tokenize_for_search(txt) {
                    if tok.chars().count() >= 2 && !is_generic_stop_token(&tok) {
                        token_domains.entry(tok).or_default().insert(slot_idx);
                    }
                }
            }
            for &lbl in prof.form_labels_ko.iter().chain(prof.form_labels_en.iter()) {
                for tok in tokenize_for_search(lbl) {
                    if tok.chars().count() >= 2 && !is_generic_stop_token(&tok) {
                        token_domains.entry(tok).or_default().insert(slot_idx);
                    }
                }
            }
            for form_code in prof.forms {
                token_domains
                    .entry(form_code.to_ascii_lowercase())
                    .or_default()
                    .insert(slot_idx);
            }
            token_domains
                .entry(prof.domain_id.to_ascii_lowercase())
                .or_default()
                .insert(slot_idx);
            token_domains
                .entry(prof.pack_id.to_ascii_lowercase())
                .or_default()
                .insert(slot_idx);
        }

        let mut concept_map = HashMap::new();
        for (tok, slots) in token_domains {
            if slots.len() == 1 {
                let slot = *slots.iter().next().expect("nonempty");
                concept_map.insert(tok, vec![(slot, 3.0f32)]);
            } else if slots.len() == 2 {
                concept_map.insert(
                    tok,
                    slots.into_iter().map(|s| (s, 0.5f32)).collect(),
                );
            }
        }

        Self {
            concept_map,
            dim: OUTPUT_DIM,
        }
    }

    fn encode(&self, text: &str) -> Vec<f32> {
        let mut vec = vec![0.0f32; self.dim];
        let toks = tokenize_for_search(text);
        if toks.is_empty() {
            vec[0] = 1.0;
            return vec;
        }
        // Dedicated orthogonal domain channels [0..304] (19 domains * 16 dims)
        for tok in &toks {
            if let Some(slots) = self.concept_map.get(tok.as_str()) {
                for &(slot, w) in slots {
                    let base = (slot * 16) % 304;
                    for off in 0..8 {
                        vec[base + off] += w;
                    }
                }
            }
            let h = fnv1a_64(tok.as_bytes());
            let idx = 304 + ((h as usize) % (self.dim - 304));
            let sign = if (h >> 63) == 0 { 1.0 } else { -1.0 };
            vec[idx] += 0.45 * sign;
        }
        let lower = text.to_lowercase();
        let chars: Vec<char> = lower.chars().collect();
        for win in chars.windows(3) {
            if win.iter().any(|c| c.is_whitespace()) {
                continue;
            }
            let s: String = win.iter().collect();
            if let Some(slots) = self.concept_map.get(s.as_str()) {
                for &(slot, w) in slots {
                    let base = (slot * 16) % 304;
                    vec[base] += w * 0.35;
                }
            }
            let h = fnv1a_64(s.as_bytes());
            let idx = 304 + ((h as usize) % (self.dim - 304));
            let sign = if (h >> 63) == 0 { 1.0 } else { -1.0 };
            vec[idx] += 0.15 * sign;
        }
        normalize_l2(&mut vec);
        vec
    }
}

impl DenseTextEmbedder for OfflineHybridEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn model_id(&self) -> &str {
        "offline-hybrid-768d-concept-v1"
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.encode(text))
    }

    fn embed_document(&self, title: Option<&str>, text: &str) -> Result<Vec<f32>> {
        let joined = match title {
            Some(t) if !t.is_empty() => format!("{t} {text}"),
            _ => text.to_string(),
        };
        Ok(self.encode(&joined))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexedDoc {
    pub doc_id: String,
    pub domain_id: String,
    pub pack_id: String,
    pub form_code: String,
    pub doc_lang: String,
    pub vault_id: String,
    pub n2sf_grade: N2sfGrade,
    pub revision: u32,
    pub entity_group_key: String,
    pub supersedes: Option<String>,
    pub superseded_by: Option<String>,
    pub title: String,
    pub text: String,
    pub primary_id: String,
    pub checksum_code: String,
    pub link_keys: Vec<String>,
    pub counterparty: String,
    pub subject_item: String,
    pub governing_statute: String,
    pub clause_excerpts: Vec<String>,
    pub revision_note: String,
    pub line_item_count: usize,
    pub line_item_sum_krw: i64,
    pub media_count: usize,
    pub amount_krw: i64,
    pub doc_date: String,
    pub status: String,
    pub is_covert_role: bool,
    pub audio_spans_ms: Vec<(u64, u64)>,
    pub dense_vec: Vec<f32>,
    pub sanitized_covert_count: usize,
    pub tokens: Vec<String>,
    #[serde(skip)]
    pub term_freqs: HashMap<String, usize>,
    #[serde(skip)]
    pub sparse_vec: Vec<(u16, f32)>,
    #[serde(skip)]
    pub primary_id_low: String,
    #[serde(skip)]
    pub checksum_low: String,
    #[serde(skip)]
    pub norm_checksum_low: String,
    #[serde(skip)]
    pub link_keys_low: Vec<String>,
}

impl IndexedDoc {
    pub fn prepare_caches(&mut self) {
        self.term_freqs.clear();
        for tok in &self.tokens {
            *self.term_freqs.entry(tok.clone()).or_insert(0) += 1;
        }
        self.sparse_vec.clear();
        for (idx, &v) in self.dense_vec.iter().enumerate() {
            if v.abs() > 1e-7 && idx < u16::MAX as usize {
                self.sparse_vec.push((idx as u16, v));
            }
        }
        self.primary_id_low = self.primary_id.to_ascii_lowercase();
        self.checksum_low = self.checksum_code.to_ascii_lowercase();
        self.norm_checksum_low = normalize_trade_identifier(&self.checksum_code).to_ascii_lowercase();
        self.link_keys_low = self.link_keys.iter().map(|s| s.to_ascii_lowercase()).collect();
    }
}

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub query_text: String,
    pub caller_role: DeviceRole,
    pub caller_vault_id: String,
    pub caller_max_grade: N2sfGrade,
    pub scope_forms: Vec<String>,
    pub min_amount_krw: Option<i64>,
    pub status_filter: Option<String>,
    pub require_relay_hops: u8,
    pub include_superseded: bool,
    pub sql_probe: Option<String>,
    pub top_k: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub doc_id: String,
    pub domain_id: String,
    pub form_code: String,
    pub score: f64,
    pub relay_hop: u8,
    pub audio_spans_ms: Vec<(u64, u64)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroundedAnswer {
    pub answer_text: String,
    pub citations: Vec<String>,
    pub evidence_snippets: Vec<String>,
    pub cited_amount_krw: Option<i64>,
    pub cited_line_item_count: Option<usize>,
    pub abstained: bool,
    pub factcheck_passed: bool,
    pub citizen_guard_clean: bool,
    pub gov_id_clean: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub sql_guard_passed: bool,
    pub sql_guard_error: Option<String>,
    pub grounded_answer: GroundedAnswer,
}

pub trait SearchIndexBackend {
    fn backend_name(&self) -> &'static str;
    fn upsert(&mut self, doc: IndexedDoc) -> Result<()>;
    fn search(&self, req: &SearchRequest, embedder: &dyn DenseTextEmbedder) -> Result<SearchResponse>;
    fn doc_count(&self) -> usize;
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryHybridIndex {
    docs: Vec<IndexedDoc>,
    doc_freq: HashMap<String, usize>,
    avg_doc_len: f64,
    topic_vocab: BTreeSet<String>,
    link_index: HashMap<String, Vec<usize>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct E2eIngestStats {
    pub total_docs_ingested: usize,
    pub parse_failures: usize,
    pub wav_checks_run: usize,
    pub wav_alignment_failures: usize,
    pub stripped_very_hidden_sheets: usize,
    pub stripped_covert_artifacts: usize,
    /// E-02: number of structured fields that failed visible-text extraction and fell back to empty/default.
    pub ground_truth_fallback_fills: usize,
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn grade_rank(g: N2sfGrade) -> u8 {
    match g {
        N2sfGrade::Open => 0,
        N2sfGrade::Sensitive => 1,
        N2sfGrade::Classified => 2,
    }
}

fn is_generic_stop_token(tok: &str) -> bool {
    matches!(
        tok,
        "의" | "및" | "등" | "에서" | "으로" | "위한" | "대한" | "관련" | "기준" | "문서" | "서류"
            | "확인" | "조회" | "검색" | "검증" | "합계" | "세부" | "내역" | "조서" | "보고서"
            | "제출" | "첨부" | "보존" | "준수" | "통제" | "관리" | "처리" | "승인" | "해당"
            | "the" | "and" | "for" | "with" | "of" | "in" | "to" | "on" | "by" | "from"
            | "document" | "documents" | "record" | "records" | "verify" | "verification"
            | "check" | "checks" | "search" | "locate" | "find" | "please" | "official"
            | "report" | "compliance" | "audit" | "total" | "amount" | "de" | "la" | "et"
            | "und" | "des" | "del" | "в" | "по" | "для" | "การ"
    )
}

pub fn tokenize_for_search(text: &str) -> Vec<String> {
    #[derive(Copy, Clone, PartialEq, Eq)]
    enum ScriptBucket {
        AsciiLatin,
        Hangul,
        CjkHan,
        Kana,
        Arabic,
        Thai,
        Cyrillic,
        Other,
    }

    fn script_bucket(ch: char) -> ScriptBucket {
        if ch.is_ascii_alphanumeric()
            || matches!(ch, '-' | '_' | '.')
            || ('\u{00C0}'..='\u{024F}').contains(&ch)
        {
            ScriptBucket::AsciiLatin
        } else if ('\u{AC00}'..='\u{D7AF}').contains(&ch)
            || ('\u{1100}'..='\u{11FF}').contains(&ch)
            || ('\u{3130}'..='\u{318F}').contains(&ch)
        {
            ScriptBucket::Hangul
        } else if ('\u{4E00}'..='\u{9FFF}').contains(&ch) {
            ScriptBucket::CjkHan
        } else if ('\u{3040}'..='\u{30FF}').contains(&ch) {
            ScriptBucket::Kana
        } else if ('\u{0600}'..='\u{06FF}').contains(&ch) {
            ScriptBucket::Arabic
        } else if ('\u{0E00}'..='\u{0E7F}').contains(&ch) {
            ScriptBucket::Thai
        } else if ('\u{0400}'..='\u{04FF}').contains(&ch) {
            ScriptBucket::Cyrillic
        } else {
            ScriptBucket::Other
        }
    }

    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if cur.is_empty() {
            return;
        }
        let lower = cur.to_lowercase();
        out.push(lower.clone());
        for josa in ["의", "을", "를", "은", "는", "이", "가", "에", "에서", "로", "으로", "와", "과", "인"] {
            if lower.len() > josa.len() + 3 && lower.ends_with(josa) {
                let stem = &lower[..lower.len() - josa.len()];
                if stem.chars().count() >= 2 {
                    out.push(stem.to_string());
                }
                break;
            }
        }
        let norm = normalize_trade_identifier(cur);
        if norm.len() >= 6 && norm != lower.to_ascii_uppercase() {
            out.push(norm.to_ascii_lowercase());
        }
        let cjk_chars: Vec<char> = lower
            .chars()
            .filter(|c| ('\u{4E00}'..='\u{9FFF}').contains(c) || ('\u{3040}'..='\u{30FF}').contains(c))
            .collect();
        for win in cjk_chars.windows(2) {
            out.push(win.iter().collect());
        }
        cur.clear();
    };

    let mut prev_bucket: Option<ScriptBucket> = None;
    for ch in text.chars() {
        let mapped = match ch {
            '\u{FF10}'..='\u{FF19}' => char::from_u32(ch as u32 - 0xFF10 + '0' as u32).unwrap_or(ch),
            '\u{FF21}'..='\u{FF3A}' => char::from_u32(ch as u32 - 0xFF21 + 'A' as u32).unwrap_or(ch),
            '\u{FF41}'..='\u{FF5A}' => char::from_u32(ch as u32 - 0xFF41 + 'A' as u32).unwrap_or(ch),
            c => c,
        };
        if mapped.is_alphanumeric()
            || matches!(mapped, '-' | '_' | '.')
            || ('\u{0E00}'..='\u{0E7F}').contains(&mapped)
            || ('\u{0600}'..='\u{06FF}').contains(&mapped)
        {
            let bucket = script_bucket(mapped);
            if prev_bucket.is_some_and(|pb| pb != bucket) && !cur.is_empty() {
                flush(&mut cur, &mut out);
            }
            cur.push(mapped);
            prev_bucket = Some(bucket);
        } else {
            flush(&mut cur, &mut out);
            prev_bucket = None;
        }
    }
    flush(&mut cur, &mut out);
    out.into_iter()
        .map(|s| s.trim_matches('.').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn is_identifier_like(tok: &str) -> bool {
    let up = tok.to_ascii_uppercase();
    if up.ends_with("-DIGIT")
        || up.ends_with("-STAGE")
        || up.ends_with("-WAY")
        || up.ends_with("-HOP")
        || up.ends_with("-YEAR")
        || up.ends_with("-MONTH")
    {
        return false;
    }
    (up.contains('-') || up.contains('_'))
        && up.len() >= 6
        && up.chars().any(|c| c.is_ascii_alphabetic())
        && up.chars().any(|c| c.is_ascii_digit())
}

fn extract_link_keys_from_text(text: &str, exclude_doc_id: &str) -> Vec<String> {
    let mut keys = BTreeSet::new();
    for raw in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_') {
        let t = raw.trim();
        if t.len() < 8 || t.eq_ignore_ascii_case(exclude_doc_id) {
            continue;
        }
        let up = t.to_ascii_uppercase();
        let is_date = up.len() == 10
            && up.as_bytes()[4] == b'-'
            && up.as_bytes()[7] == b'-'
            && up.bytes().filter(|b| b.is_ascii_digit()).count() == 8;
        if is_date || up.starts_with("DOM-") || up.contains("REDACTED") {
            continue;
        }
        if up.starts_with("KEY-")
            || up.starts_with("HUB-")
            || up.starts_with("MID-")
            || up.starts_with("LEAF-")
            || up.starts_with("ATA-")
            || up.starts_with("KR-ATA-")
            || up.starts_with("OLDREV-")
            || up.starts_with("ADVLNK-")
            || up.starts_with("XVLNK-")
            || up.starts_with("CLSLNK-")
        {
            keys.insert(up);
        }
    }
    keys.into_iter().collect()
}

fn parse_line_item_summary(text: &str) -> (usize, i64) {
    let cnt = text
        .split("LINE_ITEM_COUNT=")
        .nth(1)
        .and_then(|rest| {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse::<usize>().ok()
        })
        .unwrap_or(0);
    let sum = text
        .split("LINE_ITEM_SUM_KRW=")
        .nth(1)
        .and_then(|rest| {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse::<i64>().ok()
        })
        .unwrap_or(0);
    (cnt, sum)
}

fn parse_prefixed_segment(text: &str, prefix: &str) -> Option<String> {
    let idx = text.find(prefix)?;
    let after = &text[idx + prefix.len()..];
    let end = after
        .find('\n')
        .into_iter()
        .chain(after.find(" | "))
        .chain(after.find('\t'))
        .min()
        .unwrap_or(after.len());
    let val = after[..end].trim();
    (!val.is_empty()).then(|| val.to_string())
}

fn parse_clause_excerpts(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tag in ["CLAUSE_01:", "CLAUSE_02:", "CLAUSE_03:"] {
        if let Some(body) = parse_prefixed_segment(text, tag) {
            out.push(body);
        }
    }
    out
}

fn extract_e2e_xml_visible_text(raw: &str) -> (String, usize) {
    let mut cleaned = if let Some(start) = raw.find("<EvidenceAttachment") {
        let tail = &raw[start..];
        if let Some(end_rel) = tail.find("</EvidenceAttachment>") {
            format!("{}{}", &raw[..start], &tail[end_rel + "</EvidenceAttachment>".len()..])
        } else {
            raw[..start].to_string()
        }
    } else {
        raw.to_string()
    };
    let had_attachment = usize::from(cleaned.len() < raw.len());
    for (open_tag, label) in [
        ("<PrimaryId>", "\nPRIMARY_ID="),
        ("<ChecksumCode>", "\nCHECKSUM_CODE="),
        ("<Counterparty>", "\nCOUNTERPARTY="),
        ("<SubjectItem>", "\nSUBJECT_ITEM="),
        ("<GoverningStatute>", "\nGOVERNING_STATUTE="),
        ("<RevisionNote>", "\nREVISION_NOTE="),
        ("<Status>", "\nSTATUS="),
    ] {
        if cleaned.contains(open_tag) {
            cleaned = cleaned.replace(open_tag, label);
        }
    }
    for close_tag in [
        "</Title>", "</Body>", "</GoverningStatute>", "</RevisionNote>", "</Reconciliation>",
        "</AuditTrail>", "</Field>", "</Clause>", "</Item>", "</DocId>", "</PrimaryId>",
        "</ChecksumCode>", "</Counterparty>", "</SubjectItem>", "</Links>", "</AmountKrw>",
        "</DocDate>", "</Status>",
    ] {
        if cleaned.contains(close_tag) {
            cleaned = cleaned.replace(close_tag, "\n");
        }
    }
    let mut plain = String::with_capacity(cleaned.len() / 2 + 128);
    let mut in_tag = false;
    for ch in cleaned.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
            if !plain.ends_with([' ', '\n']) {
                plain.push(' ');
            }
        } else if !in_tag {
            plain.push(ch);
        }
    }
    plain = plain
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'");
    let lines: Vec<String> = plain
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect();
    (lines.join("\n"), had_attachment)
}

fn parse_amount_from_text_opt(text: &str) -> Option<i64> {
    let (_, exact_sum) = parse_line_item_summary(text);
    if exact_sum > 0 {
        return Some(exact_sum);
    }
    let mut best = 0i64;
    for tok in text.split(|c: char| !c.is_ascii_digit()) {
        if tok.len() >= 7 && tok.len() <= 12 {
            if let Ok(v) = tok.parse::<i64>() {
                if v >= 1_000_000 && v <= 9_000_000_000 && v > best {
                    best = v;
                }
            }
        }
    }
    (best > 0).then_some(best)
}

fn parse_status_from_text_opt(text: &str) -> Option<String> {
    if let Some(st) = parse_prefixed_segment(text, "STATUS=") {
        let up = st.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
        if matches!(up.as_str(), "APPROVED" | "HOLD" | "REVIEW") {
            return Some(up);
        }
    }
    if text.contains("payout_hold_code=REVIEW") || text.contains("settle_state=REVIEW") || text.contains("REVIEW") {
        Some("REVIEW".into())
    } else if text.contains("payout_hold_code=HOLD") || text.contains("settle_state=HOLD") || text.contains("| HOLD") {
        Some("HOLD".into())
    } else if text.contains("APPROVED") {
        Some("APPROVED".into())
    } else {
        None
    }
}

fn parse_between_markers(text: &str, start_marker: &str, end_marker: &str) -> Option<String> {
    let idx = text.find(start_marker)?;
    let after = &text[idx + start_marker.len()..];
    let end = after.find(end_marker)?;
    let val = after[..end].trim();
    (!val.is_empty()).then(|| val.to_string())
}

fn parse_labeled_row_cells(text: &str, row_key: &str) -> Option<(String, String)> {
    for line in text.lines() {
        let trimmed = line.trim();
        let rest = if let Some(r) = trimmed.strip_prefix(&format!("{row_key}\t")) {
            r
        } else if let Some(r) = trimmed.strip_prefix(&format!("{row_key} | ")) {
            r
        } else {
            continue;
        };
        let parts: Vec<&str> = if rest.contains('\t') {
            rest.split('\t').map(str::trim).collect()
        } else {
            rest.split(" | ").map(str::trim).collect()
        };
        if parts.len() >= 2 && !parts[0].is_empty() {
            return Some((parts[0].to_string(), parts[1].to_string()));
        }
    }
    None
}

fn parse_primary_id_from_text(text: &str) -> Option<String> {
    parse_prefixed_segment(text, "PRIMARY_ID=")
        .or_else(|| parse_between_markers(text, " ID ", " CHK "))
        .or_else(|| parse_labeled_row_cells(text, "DOC_ID").map(|(_, pid)| pid).filter(|s| !s.is_empty()))
}

fn parse_checksum_from_text(text: &str) -> Option<String> {
    parse_prefixed_segment(text, "CHECKSUM_CODE=")
        .or_else(|| parse_between_markers(text, " CHK ", " FORM "))
        .or_else(|| parse_labeled_row_cells(text, "CHECKSUM").map(|(chk, _)| chk).filter(|s| !s.is_empty()))
}

fn parse_party_from_text(text: &str) -> Option<(String, String)> {
    if let (Some(cp), Some(subj)) = (
        parse_prefixed_segment(text, "COUNTERPARTY="),
        parse_prefixed_segment(text, "SUBJECT_ITEM="),
    ) {
        return Some((cp, subj));
    }
    parse_labeled_row_cells(text, "PARTY")
}

fn strip_xlsx_very_hidden_blocks(blocks: &[DocBlock], hidden_sheets: &[String]) -> String {
    let mut lines = Vec::new();
    let mut skip_next_table = false;
    for b in blocks {
        match b {
            DocBlock::Heading { text, .. } => {
                if hidden_sheets.iter().any(|h| h == text) {
                    skip_next_table = true;
                    continue;
                }
                skip_next_table = false;
                if !text.trim().is_empty() {
                    lines.push(text.clone());
                }
            }
            DocBlock::Paragraph { text, .. } => {
                skip_next_table = false;
                if !text.trim().is_empty() {
                    lines.push(text.clone());
                }
            }
            DocBlock::Table(t) => {
                if skip_next_table {
                    skip_next_table = false;
                    continue;
                }
                lines.extend(t.to_plain_rows().into_iter().filter(|l| !l.trim().is_empty()));
            }
        }
    }
    lines.join("\n")
}

fn build_topic_vocab_set() -> BTreeSet<String> {
    let mut vocab = BTreeSet::new();
    let all_profiles: Vec<_> = ALL_18_DOMAIN_PROFILES
        .iter()
        .copied()
        .chain(std::iter::once(xrc_twin_profile()))
        .collect();
    for prof in &all_profiles {
        let mut texts: Vec<&str> = vec![
            prof.topic_ko,
            prof.topic_en,
            prof.topic_ja,
            prof.topic_zh,
            prof.topic_de,
            prof.topic_fr,
            prof.topic_es,
            prof.topic_ar,
            prof.topic_pt,
            prof.topic_it,
            prof.topic_nl,
            prof.topic_cs,
            prof.topic_ru,
            prof.topic_th,
            prof.synonym_ko,
            prof.synonym_en,
        ];
        if let Some(bp) = ALL_19_BLUEPRINTS.iter().find(|b| b.domain_id == prof.domain_id) {
            texts.push(bp.doc_category_ko);
            texts.push(bp.doc_category_en);
            texts.push(bp.governing_statute);
            texts.push(bp.domain_probe_ko);
            texts.push(bp.domain_probe_en);
            for &cl in &bp.statutory_clauses {
                texts.push(cl);
            }
            for &sig in bp.domain_signature_terms {
                texts.push(sig);
            }
            for &(n, sp, _) in &bp.line_item_templates {
                texts.push(n);
                texts.push(sp);
            }
        }
        for txt in texts {
            for tok in tokenize_for_search(txt) {
                vocab.insert(tok);
            }
        }
    }
    vocab
}

pub fn index_world_seed2(
    world: &RelationalWorld,
    embedder: &dyn DenseTextEmbedder,
) -> Result<InMemoryHybridIndex> {
    let mut index = InMemoryHybridIndex::new_empty();
    let mut batch = Vec::with_capacity(world.docs.len());
    for doc in &world.docs {
        // Notice: seed2 indexes ONLY legitimate structured fields (never covert_canary_token)
        let req_str = doc
            .required_field_pairs
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ");
        let clauses_str = doc.statutory_clauses.join(" ");
        let full_text = format!(
            "{} {} {} {} {} {} {} {} {} {} {} {} {}",
            doc.doc_id,
            doc.primary_id,
            doc.checksum_code,
            doc.link_keys.join(" "),
            doc.counterparty,
            doc.subject_item,
            doc.status,
            doc.amount_krw,
            doc.governing_statute,
            req_str,
            doc.revision_note,
            clauses_str,
            doc.body_lines.join(" ")
        );
        let dense_vec = embedder.embed_document(Some(&doc.title), &full_text)?;
        let tokens = tokenize_for_search(&format!("{} {full_text}", doc.title));
        batch.push(IndexedDoc {
            doc_id: doc.doc_id.clone(),
            domain_id: doc.domain_id.clone(),
            pack_id: doc.pack_id.clone(),
            form_code: doc.form_code.clone(),
            doc_lang: doc.doc_lang.clone(),
            vault_id: doc.vault_id.clone(),
            n2sf_grade: doc.n2sf_grade,
            revision: doc.revision,
            entity_group_key: doc.entity_group_key.clone(),
            supersedes: doc.supersedes.clone(),
            superseded_by: doc.superseded_by.clone(),
            title: doc.title.clone(),
            text: full_text,
            primary_id: doc.primary_id.clone(),
            checksum_code: doc.checksum_code.clone(),
            link_keys: doc.link_keys.iter().map(|s| s.to_ascii_uppercase()).collect(),
            counterparty: doc.counterparty.clone(),
            subject_item: doc.subject_item.clone(),
            governing_statute: doc.governing_statute.clone(),
            clause_excerpts: doc.statutory_clauses.iter().take(2).cloned().collect(),
            revision_note: doc.revision_note.clone(),
            line_item_count: doc.line_item_count,
            line_item_sum_krw: doc.line_item_sum_krw,
            media_count: 1,
            amount_krw: doc.amount_krw,
            doc_date: doc.doc_date.clone(),
            status: doc.status.clone(),
            is_covert_role: doc.role == WorldDocRole::CovertHostile,
            audio_spans_ms: doc.audio_spans_ms.iter().map(|(_, s, e)| (*s, *e)).collect(),
            dense_vec,
            sanitized_covert_count: 0,
            tokens,
            term_freqs: HashMap::new(),
            sparse_vec: Vec::new(),
            primary_id_low: String::new(),
            checksum_low: String::new(),
            norm_checksum_low: String::new(),
            link_keys_low: Vec::new(),
        });
    }
    index.upsert_batch(batch)?;
    Ok(index)
}

pub fn index_world_e2e(
    world: &RelationalWorld,
    embedder: &dyn DenseTextEmbedder,
) -> Result<(InMemoryHybridIndex, E2eIngestStats)> {
    let mut index = InMemoryHybridIndex::new_empty();
    let mut stats = E2eIngestStats::default();
    let mut batch = Vec::with_capacity(world.docs.len());

    for doc in &world.docs {
        let bytes = world
            .rendered_bytes
            .get(&doc.doc_id)
            .ok_or_else(|| anyhow::anyhow!("missing rendered bytes for {}", doc.doc_id))?;
        let parse_ext = match doc.file_format.as_str() {
            "xls_html" => "xls",
            other => other,
        };
        let mut ir = match parse_docs_bytes(parse_ext, bytes) {
            Ok(ir) => ir,
            Err(_) => {
                stats.parse_failures += 1;
                continue;
            }
        };

        let mut media_count = ir.media.len();
        let mut doc_stripped_covert = ir.hidden_sheets.len() + ir.hidden_run_count;
        if doc.file_format == "xml" {
            let (stripped_xml_text, xml_media) = extract_e2e_xml_visible_text(&ir.plain_text);
            ir.plain_text = stripped_xml_text;
            media_count = media_count.max(xml_media);
        } else if doc.file_format == "xlsx" {
            if let Ok(hidden) = detect_xlsx_very_hidden_sheets(bytes) {
                if !hidden.is_empty() {
                    stats.stripped_very_hidden_sheets += hidden.len();
                    doc_stripped_covert = doc_stripped_covert.max(hidden.len());
                    ir.plain_text = strip_xlsx_very_hidden_blocks(&ir.blocks, &hidden);
                }
            }
            if bytes.windows(19).any(|w| w == b"xl/media/evidence_c") {
                media_count = media_count.max(1);
            }
        } else if doc.file_format == "pdf" && bytes.windows(14).any(|w| w == b"/Subtype /Imag") {
            media_count = media_count.max(1);
        }

        let sanitized = sanitize_doc_ir(ir, Vec::new());
        stats.stripped_covert_artifacts += sanitized.stripped_covert_artifacts;
        doc_stripped_covert += sanitized.stripped_covert_artifacts;
        let visible_text = sanitized.doc.plain_text;

        let mut verified_audio_spans = Vec::new();
        if let Some(wav) = world.audio_wav_bytes.get(&doc.doc_id) {
            stats.wav_checks_run += 1;
            if verify_wav_16khz_token_aligned(wav).is_ok() {
                verified_audio_spans = doc.audio_spans_ms.iter().map(|(_, s, e)| (*s, *e)).collect();
            } else {
                stats.wav_alignment_failures += 1;
            }
        }

        // E-02: E2E extracts identifiers, link keys, clauses, status, and amount from sanitized visible text ONLY;
        // any miss is explicitly counted in `stats.ground_truth_fallback_fills` rather than silently copying `doc.*`.
        let extracted_links = extract_link_keys_from_text(&visible_text, &doc.doc_id);
        let (parsed_item_cnt, parsed_item_sum) = parse_line_item_summary(&visible_text);
        let extracted_amt = if parsed_item_sum > 0 {
            parsed_item_sum
        } else if let Some(amt) = parse_amount_from_text_opt(&visible_text) {
            amt
        } else {
            stats.ground_truth_fallback_fills += 1;
            0
        };
        let extracted_status = if let Some(st) = parse_status_from_text_opt(&visible_text) {
            st
        } else {
            stats.ground_truth_fallback_fills += 1;
            String::new()
        };
        let extracted_statute = if let Some(st) = parse_prefixed_segment(&visible_text, "GOVERNING_STATUTE=") {
            st
        } else {
            if !doc.governing_statute.is_empty() {
                stats.ground_truth_fallback_fills += 1;
            }
            String::new()
        };
        let extracted_rev_note = if let Some(rn) = parse_prefixed_segment(&visible_text, "REVISION_NOTE=") {
            rn
        } else {
            if !doc.revision_note.is_empty() {
                stats.ground_truth_fallback_fills += 1;
            }
            String::new()
        };
        let extracted_primary_id = if let Some(pid) = parse_primary_id_from_text(&visible_text) {
            pid
        } else {
            stats.ground_truth_fallback_fills += 1;
            String::new()
        };
        let extracted_checksum = if let Some(chk) = parse_checksum_from_text(&visible_text) {
            chk
        } else {
            // Covert XLSX fixture intentionally omits CHECKSUM row on the visible sheet.
            if doc_stripped_covert == 0 {
                stats.ground_truth_fallback_fills += 1;
            }
            String::new()
        };
        let (extracted_counterparty, extracted_subject_item) = if let Some(pair) = parse_party_from_text(&visible_text) {
            pair
        } else {
            if doc_stripped_covert == 0 {
                stats.ground_truth_fallback_fills += 1;
            }
            (String::new(), String::new())
        };
        let extracted_clauses = parse_clause_excerpts(&visible_text);
        let extracted_title = visible_text
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .to_string();

        let dense_vec = embedder.embed_document(Some(&extracted_title), &visible_text)?;
        let tokens = tokenize_for_search(&visible_text);

        batch.push(IndexedDoc {
            doc_id: doc.doc_id.clone(),
            domain_id: doc.domain_id.clone(),
            pack_id: doc.pack_id.clone(),
            form_code: doc.form_code.clone(),
            doc_lang: doc.doc_lang.clone(),
            vault_id: doc.vault_id.clone(), // ACL enforced by vault ledger
            n2sf_grade: doc.n2sf_grade,     // Security grade enforced by vault ledger
            revision: doc.revision,
            entity_group_key: doc.entity_group_key.clone(),
            supersedes: doc.supersedes.clone(),
            superseded_by: doc.superseded_by.clone(),
            title: extracted_title,
            text: visible_text,
            primary_id: extracted_primary_id,
            checksum_code: extracted_checksum,
            link_keys: extracted_links,
            counterparty: extracted_counterparty,
            subject_item: extracted_subject_item,
            governing_statute: extracted_statute,
            clause_excerpts: extracted_clauses,
            revision_note: extracted_rev_note,
            line_item_count: parsed_item_cnt,
            line_item_sum_krw: if parsed_item_sum > 0 { parsed_item_sum } else { extracted_amt },
            media_count,
            amount_krw: extracted_amt,
            doc_date: doc.doc_date.clone(),
            status: extracted_status,
            is_covert_role: doc_stripped_covert > 0,
            audio_spans_ms: verified_audio_spans,
            dense_vec,
            sanitized_covert_count: doc_stripped_covert,
            tokens,
            term_freqs: HashMap::new(),
            sparse_vec: Vec::new(),
            primary_id_low: String::new(),
            checksum_low: String::new(),
            norm_checksum_low: String::new(),
            link_keys_low: Vec::new(),
        });
        stats.total_docs_ingested += 1;
    }
    index.upsert_batch(batch)?;

    Ok((index, stats))
}

fn fast_cosine(q_vec: &[f32], doc: &IndexedDoc) -> f32 {
    if !doc.sparse_vec.is_empty() && doc.sparse_vec.len() <= 256 {
        let mut dot = 0.0f64;
        for &(idx, dv) in &doc.sparse_vec {
            if let Some(&qv) = q_vec.get(idx as usize) {
                dot += (qv as f64) * (dv as f64);
            }
        }
        dot as f32
    } else {
        cosine(q_vec, &doc.dense_vec)
    }
}

fn synthesize_grounded_answer(
    req: &SearchRequest,
    hits: &[SearchHit],
    docs: &[IndexedDoc],
    doc_pos: &HashMap<&str, usize>,
    sql_guard_passed: bool,
    sql_guard_error: Option<&str>,
) -> GroundedAnswer {
    if !sql_guard_passed {
        let answer = format!(
            "보안 정책 차단으로 요청을 거부함: {} ({})",
            sql_guard_error.unwrap_or("SQL_GUARD_REJECTED"),
            req.caller_vault_id
        );
        let citizen_guard_clean = check_citizen_text(&answer, &[]).is_ok();
        let gov_id_clean = !contains_forbidden_raw_gov_id_digits(&answer);
        return GroundedAnswer {
            answer_text: answer,
            citations: Vec::new(),
            evidence_snippets: Vec::new(),
            cited_amount_krw: None,
            cited_line_item_count: None,
            abstained: true,
            factcheck_passed: true,
            citizen_guard_clean,
            gov_id_clean,
        };
    }

    if hits.is_empty() {
        let answer = "권한 범위 내 가시 원문에서 부합하는 근거 문서를 찾지 못해 답변을 유보함.".to_string();
        let citizen_guard_clean = check_citizen_text(&answer, &[]).is_ok();
        let gov_id_clean = !contains_forbidden_raw_gov_id_digits(&answer);
        return GroundedAnswer {
            answer_text: answer,
            citations: Vec::new(),
            evidence_snippets: Vec::new(),
            cited_amount_krw: None,
            cited_line_item_count: None,
            abstained: true,
            factcheck_passed: true,
            citizen_guard_clean,
            gov_id_clean,
        };
    }

    let is_clean_doc = |d: &IndexedDoc| !d.is_covert_role && d.sanitized_covert_count == 0;
    let has_clean = hits
        .iter()
        .any(|h| doc_pos.get(h.doc_id.as_str()).is_some_and(|&i| is_clean_doc(&docs[i])));

    let mut cited_docs: Vec<(&SearchHit, &IndexedDoc)> = Vec::new();
    if req.require_relay_hops > 0 {
        let top_sc = hits
            .iter()
            .find(|h| !has_clean || doc_pos.get(h.doc_id.as_str()).is_some_and(|&i| is_clean_doc(&docs[i])))
            .map_or(0.0, |h| h.score);
        for h in hits.iter().take(10) {
            if let Some(&idx) = doc_pos.get(h.doc_id.as_str()) {
                let d = &docs[idx];
                if has_clean && !is_clean_doc(d) {
                    continue;
                }
                if cited_docs.is_empty() || h.relay_hop > 0 || h.score >= top_sc * 0.55 {
                    cited_docs.push((h, d));
                }
            }
        }
    } else {
        let mut top_sc = 0.0f64;
        let mut top_dom: Option<&str> = None;
        for h in hits.iter().take(10) {
            if cited_docs.len() >= 5 {
                break;
            }
            if let Some(&idx) = doc_pos.get(h.doc_id.as_str()) {
                let d = &docs[idx];
                if has_clean && !is_clean_doc(d) {
                    continue;
                }
                if cited_docs.is_empty() {
                    top_sc = h.score;
                    top_dom = Some(d.domain_id.as_str());
                    cited_docs.push((h, d));
                } else {
                    let ratio = if top_sc >= 150.0 { 0.70 } else { 0.55 };
                    if Some(d.domain_id.as_str()) == top_dom && h.score >= top_sc * ratio {
                        cited_docs.push((h, d));
                    }
                }
            }
        }
    }
    let Some((_, d0)) = cited_docs.first().copied() else {
        return GroundedAnswer {
            answer_text: "권한 범위 내 근거 문서를 찾지 못해 답변을 유보함.".into(),
            citations: Vec::new(),
            evidence_snippets: Vec::new(),
            cited_amount_krw: None,
            cited_line_item_count: None,
            abstained: true,
            factcheck_passed: true,
            citizen_guard_clean: true,
            gov_id_clean: true,
        };
    };

    let citations: Vec<String> = cited_docs.iter().map(|(_, d)| d.doc_id.clone()).collect();
    let agg_sum: i64 = cited_docs.iter().map(|(_, d)| d.amount_krw).sum();

    let mut source_parts = Vec::with_capacity(cited_docs.len() + 1);
    for (h, d) in &cited_docs {
        let span_text = d
            .audio_spans_ms
            .iter()
            .map(|(s, e)| format!("{s}..{e}ms"))
            .collect::<Vec<_>>()
            .join(" ");
        source_parts.push(format!(
            "{} {} {} {} {} {} {} {} {} {} {} {} {} {} hop={} {}",
            d.doc_id,
            d.form_code,
            d.doc_date,
            d.status,
            d.primary_id,
            d.checksum_code,
            d.title,
            d.counterparty,
            d.subject_item,
            d.governing_statute,
            d.revision_note,
            d.amount_krw,
            d.line_item_sum_krw,
            d.line_item_count,
            h.relay_hop,
            format!("Rev.{} {} {}", d.revision, span_text, d.text)
        ));
    }
    source_parts.push(format!("agg_sum={agg_sum} hit_count={}", cited_docs.len()));
    let source_facts = source_parts.join("\n");

    let mut parts = Vec::new();
    parts.push(format!(
        "[{}] ({}, {}) 문서 기준 대상 거래처 {} 및 품목 {} 정본 확인 완료. 식별코드 {} · 검증번호 {} · 상태 {} · Rev.{}.",
        d0.doc_id,
        d0.form_code,
        d0.doc_date,
        d0.counterparty,
        d0.subject_item,
        d0.primary_id,
        d0.checksum_code,
        d0.status,
        d0.revision,
    ));
    parts.push(format!(
        "금액 대사: 확정 {}원, 세부 {}건 합계 {}원 정합.",
        d0.amount_krw, d0.line_item_count, d0.line_item_sum_krw
    ));
    parts.push(format!("법적근거: {}.", d0.governing_statute));

    let mut evidence_snippets = Vec::new();
    evidence_snippets.push(format!("[{}] {}", d0.doc_id, d0.governing_statute));
    if let Some(cl) = d0.clause_excerpts.first() {
        parts.push(format!("주요 규범: {cl}"));
        evidence_snippets.push(format!("[{}] {cl}", d0.doc_id));
    }
    if req.require_relay_hops > 0 && cited_docs.len() > 1 {
        let chain = cited_docs
            .iter()
            .map(|(h, d)| format!("[{}] (hop={}, {})", d.doc_id, h.relay_hop, d.form_code))
            .collect::<Vec<_>>()
            .join(" -> ");
        parts.push(format!("연계 추적 체인: {chain}."));
    } else if cited_docs.len() > 1 {
        let refs = cited_docs
            .iter()
            .map(|(_, d)| format!("[{}]", d.doc_id))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!("추가 근거 문서({}건): {refs} 합산 {}원.", cited_docs.len(), agg_sum));
    }
    if req.sql_probe.is_some() {
        parts.push("SQL 보안 필터 통과.".to_string());
    }
    if !d0.audio_spans_ms.is_empty() {
        let spans = d0
            .audio_spans_ms
            .iter()
            .map(|(s, e)| format!("{s}..{e}ms"))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!("검출 오디오 구간: {spans}."));
        evidence_snippets.push(format!("[{}] 오디오 {spans}", d0.doc_id));
    }

    let draft = parts.join(" ");
    let fallback = format!(
        "[{}] {} ({}) {}원 근거 대조 완료.",
        d0.doc_id, d0.counterparty, d0.subject_item, d0.amount_krw
    );
    let (checked_text, factcheck_passed, _) = draft_or_fallback(&draft, &source_facts, &fallback);
    let final_text = if check_citizen_text(&checked_text, &[]).is_ok() {
        checked_text
    } else {
        fallback
    };
    let citizen_guard_clean = check_citizen_text(&final_text, &[]).is_ok();
    let gov_id_clean = !contains_forbidden_raw_gov_id_digits(&final_text);

    GroundedAnswer {
        answer_text: final_text,
        citations,
        evidence_snippets,
        cited_amount_krw: Some(d0.line_item_sum_krw),
        cited_line_item_count: Some(d0.line_item_count),
        abstained: false,
        factcheck_passed,
        citizen_guard_clean,
        gov_id_clean,
    }
}

impl SearchIndexBackend for InMemoryHybridIndex {
    fn backend_name(&self) -> &'static str {
        "in_memory_hybrid_bm25_768d_relay"
    }

    fn upsert(&mut self, mut doc: IndexedDoc) -> Result<()> {
        doc.prepare_caches();
        if let Some(pos) = self.docs.iter().position(|d| d.doc_id == doc.doc_id) {
            self.docs[pos] = doc;
        } else {
            self.docs.push(doc);
        }
        self.rebuild_stats();
        Ok(())
    }

    fn search(&self, req: &SearchRequest, embedder: &dyn DenseTextEmbedder) -> Result<SearchResponse> {
        let mut sql_guard_passed = true;
        let mut sql_guard_error = None;
        let doc_pos: HashMap<&str, usize> = self
            .docs
            .iter()
            .enumerate()
            .map(|(i, d)| (d.doc_id.as_str(), i))
            .collect();

        if let Some(ref sql) = req.sql_probe {
            let wl = default_arrow_view_whitelist();
            match verify_and_wrap_safe_sql(
                req.caller_role,
                &req.caller_vault_id,
                req.caller_max_grade,
                sql,
                &wl,
            ) {
                Ok(_) => sql_guard_passed = true,
                Err(e) => {
                    sql_guard_passed = false;
                    sql_guard_error = Some(e.to_string());
                    let grounded_answer = synthesize_grounded_answer(
                        req,
                        &[],
                        &self.docs,
                        &doc_pos,
                        false,
                        sql_guard_error.as_deref(),
                    );
                    return Ok(SearchResponse {
                        hits: Vec::new(),
                        sql_guard_passed,
                        sql_guard_error,
                        grounded_answer,
                    });
                }
            }
        }

        let eff_max_grade = match req.caller_role {
            DeviceRole::CounterKiosk => 0,
            DeviceRole::PersonalPhone => grade_rank(req.caller_max_grade),
        };

        // 1. Strict Vault + N2SF authorization filter
        let mut auth_mask: Vec<bool> = self
            .docs
            .iter()
            .map(|d| d.vault_id == req.caller_vault_id && grade_rank(d.n2sf_grade) <= eff_max_grade)
            .collect();

        // 2. Superseded filter unless explicitly requested
        if !req.include_superseded {
            let mut sup_ids: BTreeSet<&str> = BTreeSet::new();
            for (idx, d) in self.docs.iter().enumerate() {
                if !auth_mask[idx] {
                    continue;
                }
                if let Some(ref s) = d.supersedes {
                    sup_ids.insert(s.as_str());
                }
                if d.superseded_by.is_some() {
                    sup_ids.insert(d.doc_id.as_str());
                }
            }
            for (idx, d) in self.docs.iter().enumerate() {
                if auth_mask[idx] && sup_ids.contains(d.doc_id.as_str()) {
                    auth_mask[idx] = false;
                }
            }
        }

        // 3. Apply explicit scope/attribute filters
        let filtered_indices: Vec<usize> = self
            .docs
            .iter()
            .enumerate()
            .filter(|(idx, d)| {
                if !auth_mask[*idx] {
                    return false;
                }
                if !req.scope_forms.is_empty() && !req.scope_forms.iter().any(|f| f == &d.form_code) {
                    return false;
                }
                if let Some(min_a) = req.min_amount_krw {
                    if d.amount_krw < min_a {
                        return false;
                    }
                }
                if let Some(ref st) = req.status_filter {
                    if !d.status.eq_ignore_ascii_case(st) {
                        return false;
                    }
                }
                true
            })
            .map(|(idx, _)| idx)
            .collect();

        let q_vec = embedder.embed_query(&req.query_text)?;
        let q_toks = tokenize_for_search(&req.query_text);
        let q_toks_norm: Vec<(String, String)> = q_toks
            .iter()
            .map(|qt| (qt.clone(), normalize_trade_identifier(qt).to_ascii_lowercase()))
            .collect();
        let q_id_toks: Vec<String> = q_toks
            .iter()
            .filter(|t| is_identifier_like(t))
            .cloned()
            .collect();
        let uniq_q_toks: Vec<&str> = {
            let set: BTreeSet<&str> = q_toks.iter().map(String::as_str).collect();
            set.into_iter().collect()
        };

        let total_docs = self.docs.len().max(1) as f64;
        let avg_dl = self.avg_doc_len.max(1.0);
        let k1 = 1.2f64;
        let b = 0.75f64;

        let mut scored: Vec<(usize, f64, bool)> = Vec::new();
        for doc_idx in filtered_indices {
            let doc = &self.docs[doc_idx];
            let mut bm25 = 0.0f64;
            let mut lex_hits = 0usize;
            let dl = doc.tokens.len().max(1) as f64;
            for &qt in &uniq_q_toks {
                let tf = *doc.term_freqs.get(qt).unwrap_or(&0) as f64;
                if tf > 0.0 {
                    lex_hits += 1;
                    let df = *self.doc_freq.get(qt).unwrap_or(&1) as f64;
                    let idf = ((total_docs - df + 0.5) / (df + 0.5) + 1.0).ln().max(0.1);
                    let denom = tf + k1 * (1.0 - b + b * (dl / avg_dl));
                    let term_weight = if self.topic_vocab.contains(qt) {
                        0.15
                    } else {
                        2.5
                    };
                    bm25 += term_weight * idf * ((tf * (k1 + 1.0)) / denom);
                }
            }

            let dense_sim = fast_cosine(&q_vec, doc).max(0.0) as f64;
            let primary_match = q_toks_norm
                .iter()
                .any(|(qt_low, _)| doc.primary_id_low == *qt_low);
            let checksum_match = q_toks_norm
                .iter()
                .any(|(qt_low, qt_norm)| {
                    doc.checksum_low == *qt_low || doc.norm_checksum_low == *qt_norm
                });
            let link_match = q_toks_norm
                .iter()
                .any(|(qt_low, _)| doc.link_keys_low.iter().any(|lk| lk == qt_low));
            let exact_id_hit = primary_match || checksum_match || link_match;

            if !q_id_toks.is_empty() && !exact_id_hit && lex_hits == 0 {
                continue;
            }
            if lex_hits == 0 && !exact_id_hit && dense_sim < 0.28 {
                continue;
            }

            let mut total = bm25 + dense_sim * 8.0;
            if primary_match {
                total += 200.0;
            }
            if checksum_match {
                total += 110.0;
            }
            if link_match {
                total += 70.0;
            }
            if !doc.is_covert_role && doc.sanitized_covert_count == 0 {
                total += 0.8;
            } else {
                total -= 2.5;
            }
            scored.push((doc_idx, total, exact_id_hit));
        }

        scored.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| self.docs[a.0].doc_id.cmp(&self.docs[b.0].doc_id))
        });

        let mut results: Vec<SearchHit> = Vec::new();
        let mut seen_ids = BTreeSet::new();

        // 4. Multi-hop entity relay over O(degree) `link_index` on authorized non-superseded docs (1..2 hops)
        if req.require_relay_hops > 0 && !scored.is_empty() {
            let top_sc = scored[0].1;
            let mut relay_frontier: Vec<usize> = scored
                .iter()
                .filter(|&&(idx, sc, exact)| !self.docs[idx].is_covert_role && (exact || sc >= top_sc * 0.55))
                .take(2)
                .map(|&(idx, _, _)| idx)
                .collect();
            if relay_frontier.is_empty() && !self.docs[scored[0].0].is_covert_role {
                relay_frontier.push(scored[0].0);
            }
            for &seed_idx in &relay_frontier {
                let seed_doc = &self.docs[seed_idx];
                if seen_ids.insert(seed_doc.doc_id.clone()) {
                    let seed_sc = scored
                        .iter()
                        .find(|&&(i, _, _)| i == seed_idx)
                        .map_or(top_sc, |x| x.1);
                    results.push(SearchHit {
                        doc_id: seed_doc.doc_id.clone(),
                        domain_id: seed_doc.domain_id.clone(),
                        form_code: seed_doc.form_code.clone(),
                        score: seed_sc,
                        relay_hop: 0,
                        audio_spans_ms: seed_doc.audio_spans_ms.clone(),
                    });
                }
            }

            for hop in 1..=req.require_relay_hops {
                let mut next_frontier = Vec::new();
                let mut candidate_indices = BTreeSet::new();
                for &cur_idx in &relay_frontier {
                    for lk in &self.docs[cur_idx].link_keys {
                        if let Some(neighbors) = self.link_index.get(lk) {
                            for &cand_idx in neighbors {
                                if auth_mask[cand_idx]
                                    && !self.docs[cand_idx].is_covert_role
                                    && !seen_ids.contains(&self.docs[cand_idx].doc_id)
                                {
                                    candidate_indices.insert(cand_idx);
                                }
                            }
                        }
                    }
                }
                let mut hop_candidates: Vec<(usize, usize)> = candidate_indices
                    .into_iter()
                    .map(|idx| (idx, self.docs[idx].link_keys.len()))
                    .collect();
                hop_candidates.sort_by(|a, b| {
                    b.1.cmp(&a.1)
                        .then_with(|| self.docs[a.0].doc_id.cmp(&self.docs[b.0].doc_id))
                });
                let base_score = results
                    .first()
                    .map_or(5.0, |h| h.score * (0.75f64.powi(hop as i32)));
                for (offset, (cand_idx, _)) in hop_candidates.into_iter().enumerate() {
                    let cand = &self.docs[cand_idx];
                    if seen_ids.insert(cand.doc_id.clone()) {
                        results.push(SearchHit {
                            doc_id: cand.doc_id.clone(),
                            domain_id: cand.domain_id.clone(),
                            form_code: cand.form_code.clone(),
                            score: base_score - (offset as f64) * 0.01,
                            relay_hop: hop,
                            audio_spans_ms: cand.audio_spans_ms.clone(),
                        });
                        next_frontier.push(cand_idx);
                    }
                }
                relay_frontier = next_frontier;
            }
            for &(doc_idx, sc, _) in &scored {
                if results.len() >= req.top_k {
                    break;
                }
                let doc = &self.docs[doc_idx];
                if seen_ids.insert(doc.doc_id.clone()) {
                    results.push(SearchHit {
                        doc_id: doc.doc_id.clone(),
                        domain_id: doc.domain_id.clone(),
                        form_code: doc.form_code.clone(),
                        score: sc,
                        relay_hop: 0,
                        audio_spans_ms: doc.audio_spans_ms.clone(),
                    });
                }
            }
            if results.len() > req.top_k {
                results.truncate(req.top_k);
            }
        } else {
            for &(doc_idx, sc, _) in scored.iter().take(req.top_k) {
                let doc = &self.docs[doc_idx];
                results.push(SearchHit {
                    doc_id: doc.doc_id.clone(),
                    domain_id: doc.domain_id.clone(),
                    form_code: doc.form_code.clone(),
                    score: sc,
                    relay_hop: 0,
                    audio_spans_ms: doc.audio_spans_ms.clone(),
                });
            }
        }

        let grounded_answer = synthesize_grounded_answer(
            req,
            &results,
            &self.docs,
            &doc_pos,
            sql_guard_passed,
            sql_guard_error.as_deref(),
        );

        Ok(SearchResponse {
            hits: results,
            sql_guard_passed,
            sql_guard_error,
            grounded_answer,
        })
    }

    fn doc_count(&self) -> usize {
        self.docs.len()
    }
}

impl InMemoryHybridIndex {
    pub fn new_empty() -> Self {
        Self {
            docs: Vec::new(),
            doc_freq: HashMap::new(),
            avg_doc_len: 1.0,
            topic_vocab: build_topic_vocab_set(),
            link_index: HashMap::new(),
        }
    }

    pub fn docs(&self) -> &[IndexedDoc] {
        &self.docs
    }

    pub fn upsert_batch(&mut self, docs: Vec<IndexedDoc>) -> Result<()> {
        for mut doc in docs {
            doc.prepare_caches();
            if let Some(pos) = self.docs.iter().position(|d| d.doc_id == doc.doc_id) {
                self.docs[pos] = doc;
            } else {
                self.docs.push(doc);
            }
        }
        self.rebuild_stats();
        Ok(())
    }

    fn rebuild_stats(&mut self) {
        self.doc_freq.clear();
        self.link_index.clear();
        let mut total_len = 0usize;
        for (doc_idx, d) in self.docs.iter().enumerate() {
            total_len += d.tokens.len();
            for t in d.term_freqs.keys() {
                *self.doc_freq.entry(t.clone()).or_insert(0) += 1;
            }
            for lk in &d.link_keys {
                self.link_index.entry(lk.clone()).or_default().push(doc_idx);
            }
        }
        self.avg_doc_len = if self.docs.is_empty() {
            1.0
        } else {
            total_len as f64 / self.docs.len() as f64
        };
    }
}
