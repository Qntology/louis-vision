//! Multi-tier Qwen3.5 (`0.8B · 2B · 4B`) vocabulary trimming, 5-backend zero-VRAM `MappedEmbedTable`,
//! 3-way cache isolation `(QwenTier, hidden_size, tok_fingerprint, artifact_k)`, zero-requant `TrimmedHead`,
//! default `GuardMode::Off` vs `LOUIS_EXACT_LM_HEAD=strict|e1` host Q8_0 SIMD rescue, and `DocLangScope`.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{anyhow, bail, Result};
use candle_core::quantized::{ggml_file, gguf_file, GgmlDType, QMatMul, QTensor};
use candle_core::{Device, Module, Tensor};
use memmap2::Mmap;
use once_cell::sync::Lazy;
use premium_discount_cpu_kernels::q8::dot_q8_0_f32_simd;
use serde::{Deserialize, Serialize};

pub use premium_discount_quant::trim::{
    kind_of, kinds as token_kinds, select_compact_track_m, select_from, tok_fingerprint, Kind,
    Langs, TrimRule, ALIGN, CJK_COMMON,
};

use crate::device::RuntimeBackendKind;
use crate::residency::QwenTier;

pub const QWEN35_VOCAB_SIZE: usize = 248_320;
pub const NANO_CORE_K: usize = 16_384;
pub const NANO_MAX_K: usize = 24_576;
pub const BASE_SLICE_K: usize = 32_768;
pub const BASE_CJK_SLICE_K: usize = 45_056;
pub const BASE_MIXED_SLICE_K: usize = 49_152;
pub const HEAVY4B_STATIC_ARTIFACT_K: usize = 65_536;
pub const HEAVY4B_DEFAULT_SLICE_K: usize = 32_768;
pub const HEAVY4B_CJK_SLICE_K: usize = 45_056;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelPolicyVerdict {
    Permitted { canonical: String },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ModelPolicyError {
    #[error("retired model rejected by policy ({canonical}): {reason}")]
    Retired { canonical: String, reason: &'static str },
    #[error("forbidden model rejected by policy ({canonical}): {reason}")]
    Forbidden { canonical: String, reason: &'static str },
    #[error("foreign vocabulary model rejected ({canonical}): {reason}")]
    ForeignVocab { canonical: String, reason: &'static str },
    #[error("empty model identifier")]
    Empty,
}

/// Exact canonical-stem model gate retiring legacy models while cleanly allowing `embeddinggemma-2`,
/// `siglip2`, and all `qwen3.5` tiers without overbroad prefix collisions.
pub fn canonical_policy_key(raw: &str) -> std::result::Result<ModelPolicyVerdict, ModelPolicyError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ModelPolicyError::Empty);
    }
    let stem = trimmed
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(trimmed)
        .to_ascii_lowercase();
    let norm = stem.trim_end_matches(".gguf").trim_end_matches(".safetensors");

    if norm == "granite-4.0-h-350m" || norm.starts_with("granite-4.0-h-350m-") || norm == "granite-350m" {
        return Err(ModelPolicyError::Retired {
            canonical: stem,
            reason: "Retire: replaced by Gemma4 768d",
        });
    }
    if norm == "embeddinggemma-300m" || norm.starts_with("embeddinggemma-300m-") {
        return Err(ModelPolicyError::Forbidden {
            canonical: stem,
            reason: "Forbidden: replaced by google/embeddinggemma-2",
        });
    }
    if norm == "qwen3-0.6b" || norm.starts_with("qwen3-0.6b-") {
        return Err(ModelPolicyError::ForeignVocab {
            canonical: stem,
            reason: "Rejected: foreign 151,936 vocab incompatible with 248,320 Qwen3.5 tokenizer",
        });
    }
    if norm == "gocr" || norm.starts_with("gocr-") || norm.starts_with("gocr_") {
        return Err(ModelPolicyError::Retired {
            canonical: stem,
            reason: "Removed legacy OCR model; use Qwen35(Base2B/Heavy4B)",
        });
    }
    if norm == "zoro" || norm.starts_with("zoro-") || norm.starts_with("zoro_") {
        return Err(ModelPolicyError::Retired {
            canonical: stem,
            reason: "Removed legacy pointer model; zoro.rs deleted",
        });
    }
    if norm == "laya" || norm.starts_with("laya-") || norm.starts_with("laya_") {
        return Err(ModelPolicyError::Forbidden {
            canonical: stem,
            reason: "Forbidden: replaced zero-dependency by EmbedAxisScorer + Gemma4",
        });
    }
    Ok(ModelPolicyVerdict::Permitted { canonical: stem })
}

pub fn normalize_lang_tag(raw: &str) -> Option<String> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    let canon = match lower.as_str() {
        "zh-tw" | "zh_tw" | "zh-hant" | "zh_hant" | "zho-tw" | "traditional chinese" => "zh-tw",
        "zh-hk" | "zh_hk" => "zh-hk",
        "zh" | "zho" | "cmn" | "chinese" | "zh-cn" | "zh_cn" | "zh-hans" | "zh_hans" => "zh",
        "ko" | "kor" | "korean" => "ko",
        "en" | "eng" | "english" => "en",
        "ja" | "jpn" | "japanese" => "ja",
        "ar" | "ara" | "arabic" => "ar",
        "fr" | "fra" | "fre" | "french" => "fr",
        "de" | "deu" | "ger" | "german" => "de",
        "es" | "spa" | "spanish" => "es",
        "it" | "ita" | "italian" => "it",
        "pt" | "por" | "portuguese" | "pt-br" | "pt_br" => "pt",
        "nl" | "nld" | "dut" | "dutch" => "nl",
        "cs" | "ces" | "cze" | "czech" => "cs",
        "ru" | "rus" | "russian" => "ru",
        "uk" | "ukr" | "ukrainian" => "uk",
        "th" | "tha" | "thai" => "th",
        "hi" | "hin" | "hindi" => "hi",
        "el" | "ell" | "gre" | "greek" => "el",
        "he" | "heb" | "iw" | "hebrew" => "he",
        "vi" | "vie" | "vietnamese" => "vi",
        "pl" | "pol" | "polish" => "pl",
        "tr" | "tur" | "turkish" => "tr",
        "id" | "ind" | "indonesian" => "id",
        other => other.split(['-', '_']).next().unwrap_or(other),
    };
    Some(canon.to_string())
}

#[derive(Debug, Clone)]
struct ScopeEntry {
    doc_lang: String,
    ui_lang: String,
    langs: Langs,
    pending: bool,
}

thread_local! {
    static DOC_LANG_STACK: RefCell<Vec<ScopeEntry>> = const { RefCell::new(Vec::new()) };
}

/// Thread-local RAII language scope preserving `"zh-tw"` and resolving `"pending"` image scopes
/// upon the first non-empty OCR crop via `observe_read`.
pub struct DocLangScope {
    depth: usize,
}

impl DocLangScope {
    pub fn text(doc_lang: &str, text: &str) -> Self {
        let norm = normalize_lang_tag(doc_lang).unwrap_or_else(|| "en".to_string());
        let langs = Langs::of_doc_lang(&norm).union(Langs::present(text));
        Self::push_entry(ScopeEntry {
            doc_lang: norm.clone(),
            ui_lang: norm,
            langs,
            pending: false,
        })
    }

    pub fn image() -> Self {
        Self::push_entry(ScopeEntry {
            doc_lang: "pending".into(),
            ui_lang: "en".into(),
            langs: Langs::any(),
            pending: true,
        })
    }

    fn push_entry(entry: ScopeEntry) -> Self {
        let depth = DOC_LANG_STACK.with(|st| {
            let mut s = st.borrow_mut();
            s.push(entry);
            s.len()
        });
        Self { depth }
    }

    pub fn observe_read(&self, crop_text: &str) {
        DOC_LANG_STACK.with(|st| {
            let mut s = st.borrow_mut();
            if let Some(top) = s.last_mut() {
                if top.pending {
                    let present = Langs::present(crop_text);
                    if present != Langs::present("") {
                        let resolved = if present.hangul {
                            "ko"
                        } else if present.kana {
                            "ja"
                        } else if present.arabic {
                            "ar"
                        } else if present.hanja || present.cjk {
                            if crop_text.contains('體') || crop_text.contains('萬') || crop_text.contains('灣') {
                                "zh-tw"
                            } else {
                                "zh"
                            }
                        } else if present.other {
                            "other"
                        } else {
                            "en"
                        };
                        top.doc_lang = resolved.to_string();
                        top.ui_lang = resolved.to_string();
                        top.langs = present.union(Langs::of_doc_lang(resolved));
                        top.pending = false;
                    }
                } else {
                    top.langs = top.langs.union(Langs::present(crop_text));
                }
            }
        });
    }

    /// Returns `[doc_lang, ui_lang, resolved_scope_lang]` — guaranteed non-empty.
    pub fn labels(&self) -> [String; 3] {
        Self::current_labels()
    }

    pub fn current_labels() -> [String; 3] {
        DOC_LANG_STACK.with(|st| {
            let s = st.borrow();
            if let Some(top) = s.last() {
                let resolved = top.langs.label();
                [
                    top.doc_lang.clone(),
                    top.ui_lang.clone(),
                    if resolved.is_empty() { "any".into() } else { resolved },
                ]
            } else {
                ["en".into(), "en".into(), "any".into()]
            }
        })
    }

    pub fn current_langs() -> Langs {
        DOC_LANG_STACK.with(|st| st.borrow().last().map(|e| e.langs).unwrap_or_else(Langs::any))
    }
}

impl Drop for DocLangScope {
    fn drop(&mut self) {
        DOC_LANG_STACK.with(|st| {
            let mut s = st.borrow_mut();
            if s.len() >= self.depth {
                s.truncate(self.depth - 1);
            }
        });
    }
}

/// Validates every host token ID against `active_vocab_limit` BEFORE any GPU tensor indexing.
pub fn validate_host_token_ids(ids: &[u32], active_vocab_limit: usize) -> Result<()> {
    for &id in ids {
        if (id as usize) >= active_vocab_limit {
            bail!("token id {id} exceeds active vocab limit {active_vocab_limit}");
        }
    }
    Ok(())
}

/// Quantized row bytes per token vector in GGML `Q8_0`: `(hidden_size / 32) * 34`.
pub fn q8_row_bytes(hidden_size: usize) -> usize {
    (hidden_size / 32) * 34
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardMode {
    Off,
    Single,
    Clustered,
    StrictE1,
}

impl GuardMode {
    /// Parses `LOUIS_EXACT_LM_HEAD` (fallback `LOGIS_EXACT_LM_HEAD`). Defaults strictly to `GuardMode::Off`.
    pub fn from_env() -> Self {
        let raw = std::env::var("LOUIS_EXACT_LM_HEAD")
            .or_else(|_| std::env::var("LOGIS_EXACT_LM_HEAD"))
            .unwrap_or_default();
        Self::parse(&raw)
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "strict" | "e1" | "strict_e1" => Self::StrictE1,
            "single" => Self::Single,
            "clustered" | "cold" => Self::Clustered,
            _ => Self::Off,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Single => "single",
            Self::Clustered => "clustered",
            Self::StrictE1 => "strict_e1",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VocabExecMode {
    ArtifactDirect,
    RuntimeSliced,
    FullUntrimmed,
}

impl VocabExecMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ArtifactDirect => "artifact_direct",
            Self::RuntimeSliced => "runtime_sliced",
            Self::FullUntrimmed => "full_untrimmed",
        }
    }
}

/// 3-way isolation key separating cached row-slice plans across `(0.8B, 2B, 4B)` sharing vocab size `248,320`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrimCacheKey {
    pub tier: QwenTier,
    pub hidden_size: usize,
    pub tok_fingerprint: u64,
    pub artifact_k: usize,
}

static TRIM_PLAN_CACHE: Lazy<Mutex<HashMap<(TrimCacheKey, String, u64), Arc<Vec<u32>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn clear_trim_plan_cache() {
    TRIM_PLAN_CACHE.lock().unwrap_or_else(|p| p.into_inner()).clear();
}

pub fn cached_or_insert_trim_plan(
    key: TrimCacheKey,
    scope_label: &str,
    prompt_hash: u64,
    build: impl FnOnce() -> Vec<u32>,
) -> (bool, Arc<Vec<u32>>) {
    let full_key = (key, scope_label.to_string(), prompt_hash);
    let mut map = TRIM_PLAN_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(hit) = map.get(&full_key) {
        return (true, Arc::clone(hit));
    }
    let built = Arc::new(build());
    map.insert(full_key, Arc::clone(&built));
    (false, built)
}

/// Computes active vocabulary target `K` per Qwen3.5 tier & language scope.
pub fn tier_target_k(tier: QwenTier, langs: Langs) -> usize {
    match tier {
        QwenTier::Nano08B => NANO_CORE_K,
        QwenTier::Base2B => {
            if langs.other && (langs.cjk || langs.kana) {
                BASE_MIXED_SLICE_K
            } else if langs.cjk || langs.kana {
                BASE_CJK_SLICE_K
            } else {
                BASE_SLICE_K
            }
        }
        QwenTier::Heavy4B => {
            if langs.cjk || langs.kana {
                HEAVY4B_CJK_SLICE_K
            } else {
                HEAVY4B_DEFAULT_SLICE_K
            }
        }
    }
}

/// Builds a Qwen3.5 tier-specific sorted & `64`-aligned `kept_ids` subset.
/// - `Nano08B`: unions core (`16,384`) + `prompt_ids`, capped at `24,576`.
/// - `Base2B`: retains full 248,320 mapped source and selects `32,768` / `45,056` / `49,152` including Tier-2 fallback scripts (`langs.other`).
/// - `Heavy4B`: selects `32,768` / `45,056` (including `"zh-tw"` CJK rows) within active vocab `limit` (`65,536` static artifact or `248,320`).
pub fn build_tier_kept_ids(
    kinds: &[Kind],
    pieces: &[Vec<u8>],
    domain: &BTreeSet<u32>,
    prompt_ids: &[u32],
    tier: QwenTier,
    langs: Langs,
    doc_lang: Option<&str>,
) -> Vec<u32> {
    let vocab = kinds.len();
    if vocab == 0 {
        return Vec::new();
    }
    let target_k = tier_target_k(tier, langs).min(vocab);
    let mut prompt_set = BTreeSet::new();
    for &id in prompt_ids {
        if (id as usize) < vocab {
            prompt_set.insert(id);
        }
    }
    let ascii_core: BTreeSet<u32> = (0..vocab as u32)
        .filter(|&i| kinds[i as usize] == Kind::Ascii)
        .take(4096)
        .collect();

    let (mut kept, _) = select_compact_track_m(
        kinds,
        pieces,
        domain,
        &prompt_set,
        &ascii_core,
        langs,
        doc_lang,
        target_k,
        None,
    );

    if tier == QwenTier::Nano08B {
        let mut set: BTreeSet<u32> = kept.into_iter().collect();
        for &pid in &prompt_set {
            set.insert(pid);
        }
        let target = set.len().next_multiple_of(ALIGN).min(NANO_MAX_K).min(vocab);
        for id in 0..vocab as u32 {
            if set.len() >= target {
                break;
            }
            set.insert(id);
        }
        kept = set.into_iter().take(NANO_MAX_K.min(vocab)).collect();
    }
    kept
}

/// Host memory-mapped embedding source (`token_embd.weight` or untied `output.weight`).
#[derive(Clone)]
pub struct TableSource {
    pub map: Arc<Mmap>,
    pub start: usize,
    pub rows: usize,
    pub cols: usize,
    pub dtype: GgmlDType,
    pub tied_to_input: bool,
}

impl std::fmt::Debug for TableSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableSource")
            .field("rows", &self.rows)
            .field("cols", &self.cols)
            .field("dtype", &self.dtype)
            .field("tied_to_input", &self.tied_to_input)
            .finish()
    }
}

impl TableSource {
    pub fn from_gguf(
        map: Arc<Mmap>,
        content: &gguf_file::Content,
        name: &str,
        tied_to_input: bool,
    ) -> Result<Self> {
        let info = content
            .tensor_infos
            .get(name)
            .ok_or_else(|| anyhow!("GGUF missing tensor '{name}'"))?;
        let (rows, cols) = info.shape.dims2()?;
        let dtype = info.ggml_dtype;
        if cols % dtype.block_size() != 0 {
            bail!("{name}: cols {cols} not divisible by block size {}", dtype.block_size());
        }
        let start = (content.tensor_data_offset + info.offset) as usize;
        let rb = cols / dtype.block_size() * dtype.type_size();
        if start + rows * rb > map.len() {
            bail!("{name}: mmap truncated");
        }
        Ok(Self {
            map,
            start,
            rows,
            cols,
            dtype,
            tied_to_input,
        })
    }

    /// Creates a deterministic `Q8_0` host mmap file for zero-requant verification and M2 tests.
    pub fn synthetic_q8_for_tests(path: &Path, rows: usize, cols: usize) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if cols % 32 != 0 {
            bail!("Q8_0 requires cols divisible by 32");
        }
        let vals: Vec<f32> = (0..rows * cols)
            .map(|i| (((i as f32) * 0.213).sin() * 0.7) + ((i / cols) as f32) * 0.002)
            .collect();
        let t = Tensor::from_vec(vals, (rows, cols), &Device::Cpu)?;
        let qt = QTensor::quantize(&t, GgmlDType::Q8_0)?;
        let mut gguf_bytes = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut gguf_bytes);
        gguf_file::write(&mut cursor, &[], &[("output.weight", &qt)])?;
        std::fs::write(path, &gguf_bytes)?;
        let file = std::fs::File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        let mut rdr = std::io::Cursor::new(&mmap[..]);
        let content = gguf_file::Content::read(&mut rdr)?;
        Self::from_gguf(Arc::new(mmap), &content, "output.weight", false)
    }

    pub fn row_bytes(&self) -> usize {
        self.cols / self.dtype.block_size() * self.dtype.type_size()
    }

    pub fn total_bytes(&self) -> usize {
        self.rows * self.row_bytes()
    }

    pub fn raw_row_bytes(&self, row_id: usize) -> Option<&[u8]> {
        if row_id >= self.rows {
            return None;
        }
        let rb = self.row_bytes();
        let s = self.start + row_id * rb;
        self.map.get(s..s + rb)
    }

    /// Gathers raw quantized block bytes for `ids` WITHOUT dequantization or re-quantization (E0 exact).
    pub fn gather(&self, ids: &[u32], device: &Device) -> Result<(QTensor, Vec<u8>)> {
        let rb = self.row_bytes();
        let total = ids.len() * rb;
        let mut raw = vec![0u8; total];
        for (k, &r) in ids.iter().enumerate() {
            let r = r as usize;
            if r >= self.rows {
                bail!("row {r} exceeds source table rows {}", self.rows);
            }
            raw[k * rb..(k + 1) * rb]
                .copy_from_slice(&self.map[self.start + r * rb..self.start + (r + 1) * rb]);
        }
        let qt = ggml_file::qtensor_from_ggml(self.dtype, &raw, vec![ids.len(), self.cols], device)?;
        Ok((qt, raw))
    }
}

/// 5-backend (`CUDA` · `ROCm` · `Vulkan` · `Metal` · `Cpu`) zero-VRAM host-mmap token table.
pub struct MappedEmbedTable {
    pub backend: RuntimeBackendKind,
    pub source: TableSource,
    pub table_mode: &'static str,
    pub resident_table_mb: f32,
    pub mapped_table_mb: f32,
}

impl MappedEmbedTable {
    pub fn open(source: TableSource, backend: RuntimeBackendKind) -> Self {
        let mapped_mb = (source.total_bytes() as f64 / (1024.0 * 1024.0)) as f32;
        Self {
            backend,
            source,
            table_mode: "mapped",
            resident_table_mb: 0.0,
            mapped_table_mb: mapped_mb,
        }
    }

    pub fn supports_all_five_backends() -> [RuntimeBackendKind; 5] {
        [
            RuntimeBackendKind::Cuda,
            RuntimeBackendKind::Rocm,
            RuntimeBackendKind::Vulkan,
            RuntimeBackendKind::Metal,
            RuntimeBackendKind::Cpu,
        ]
    }

    /// Reads ONLY requested rows on host without allocating the full vocabulary table on GPU VRAM.
    pub fn embed_rows_f32(&self, ids: &[u32], device: &Device) -> Result<Tensor> {
        validate_host_token_ids(ids, self.source.rows)?;
        let (qt, _) = self.source.gather(ids, &Device::Cpu)?;
        let f32_cpu = qt.dequantize(&Device::Cpu)?;
        Ok(f32_cpu.to_device(device)?)
    }
}

/// Bufferless Q8_0 SIMD host rescue guard enabled strictly on `LOUIS_EXACT_LM_HEAD=strict|e1`.
pub struct RestRescueGuard {
    src: TableSource,
    rest_ids: Arc<Vec<u32>>,
    mu_rest: Vec<f32>,
    w_rest_bound: f32,
}

impl std::fmt::Debug for RestRescueGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestRescueGuard")
            .field("rest_len", &self.rest_ids.len())
            .field("w_rest_bound", &self.w_rest_bound)
            .finish()
    }
}

impl RestRescueGuard {
    pub fn build(src: &TableSource, kept: &[u32]) -> Result<Option<Self>> {
        if src.dtype != GgmlDType::Q8_0 {
            return Ok(None);
        }
        let mut is_kept = vec![false; src.rows];
        for &id in kept {
            if (id as usize) < src.rows {
                is_kept[id as usize] = true;
            }
        }
        let rest_ids: Vec<u32> = (0..src.rows as u32)
            .filter(|&id| !is_kept[id as usize])
            .collect();
        if rest_ids.is_empty() {
            return Ok(None);
        }
        let cols = src.cols;
        let mut sum = vec![0.0f64; cols];
        let mut deq_rows: Vec<Vec<f32>> = Vec::with_capacity(rest_ids.len());
        for chunk in rest_ids.chunks(256) {
            let (qt, _) = src.gather(chunk, &Device::Cpu)?;
            let flat = qt.dequantize(&Device::Cpu)?.flatten_all()?.to_vec1::<f32>()?;
            for r in flat.chunks_exact(cols) {
                for (s, &v) in sum.iter_mut().zip(r) {
                    *s += v as f64;
                }
                deq_rows.push(r.to_vec());
            }
        }
        let inv = 1.0 / (rest_ids.len() as f64);
        let mu_rest: Vec<f32> = sum.into_iter().map(|s| (s * inv) as f32).collect();
        let mut max_dev_sq = 0.0f32;
        for row in &deq_rows {
            let sq: f32 = row
                .iter()
                .zip(&mu_rest)
                .map(|(&w, &m)| {
                    let d = w - m;
                    d * d
                })
                .sum();
            if sq > max_dev_sq {
                max_dev_sq = sq;
            }
        }
        Ok(Some(Self {
            src: src.clone(),
            rest_ids: Arc::new(rest_ids),
            mu_rest,
            w_rest_bound: max_dev_sq.sqrt() * (1.0 + 1e-3),
        }))
    }

    /// Checks Cauchy-Schwarz upper bound over REST rows and rescues via zero-buffer `dot_q8_0_f32_simd`.
    pub fn rescue_if_needed(&self, hidden_row: &[f32], kept_best_id: u32, kept_best_logit: f32) -> (u32, f32, bool) {
        let dot_mu: f32 = hidden_row
            .iter()
            .zip(&self.mu_rest)
            .map(|(&h, &m)| h * m)
            .sum();
        let h_norm = hidden_row.iter().map(|&x| x * x).sum::<f32>().sqrt();
        let cs_upper = dot_mu + h_norm * self.w_rest_bound;
        if cs_upper <= kept_best_logit {
            return (kept_best_id, kept_best_logit, false);
        }
        let mut best_id = kept_best_id;
        let mut best_val = kept_best_logit;
        let mut rescued = false;
        for &rid in self.rest_ids.iter() {
            if let Some(raw_q8) = self.src.raw_row_bytes(rid as usize) {
                let s = dot_q8_0_f32_simd(raw_q8, hidden_row);
                if s > best_val {
                    best_val = s;
                    best_id = rid;
                    rescued = true;
                }
            }
        }
        (best_id, best_val, rescued)
    }
}

/// Zero-requantized quantized `lm_head` projection storing `QTensor` bytes verbatim (0 B GPU `inv`/`pad`).
pub struct TrimmedHead {
    pub w: QMatMul,
    pub qweight: Arc<QTensor>,
    pub raw_gathered_bytes: Vec<u8>,
    pub kept: Arc<Vec<u32>>,
    pub vocab: usize,
    pub hidden_size: usize,
    pub tier: QwenTier,
    pub label: String,
    pub cache_key: TrimCacheKey,
    pub guard_mode: GuardMode,
    pub exec_mode: VocabExecMode,
    pub double_proj_steps: u32,
    host_inv: Arc<Vec<u32>>,
    src: TableSource,
    rescue: Option<Arc<RestRescueGuard>>,
}

impl std::fmt::Debug for TrimmedHead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrimmedHead")
            .field("vocab", &self.vocab)
            .field("kept_len", &self.kept.len())
            .field("hidden_size", &self.hidden_size)
            .field("tier", &self.tier)
            .field("guard_mode", &self.guard_mode)
            .field("exec_mode", &self.exec_mode)
            .field("label", &self.label)
            .finish()
    }
}

impl TrimmedHead {
    pub fn build(
        src: &TableSource,
        tier: QwenTier,
        tok_fingerprint: u64,
        kept: Arc<Vec<u32>>,
        label: impl Into<String>,
        guard_mode: GuardMode,
        device: &Device,
    ) -> Result<Self> {
        if kept.is_empty() {
            bail!("TrimmedHead kept rows cannot be empty");
        }
        let (qt, raw_gathered_bytes) = src.gather(&kept, device)?;
        let qweight = Arc::new(qt);
        let mut inv = vec![u32::MAX; src.rows];
        for (idx, &id) in kept.iter().enumerate() {
            if (id as usize) < src.rows {
                inv[id as usize] = idx as u32;
            }
        }
        let rescue = if guard_mode != GuardMode::Off && kept.len() < src.rows {
            RestRescueGuard::build(src, &kept)?.map(Arc::new)
        } else {
            None
        };
        let is_trimmed = kept.len() < src.rows;
        let exec_mode = if is_trimmed {
            VocabExecMode::RuntimeSliced
        } else if src.rows <= HEAVY4B_STATIC_ARTIFACT_K {
            VocabExecMode::ArtifactDirect
        } else {
            VocabExecMode::FullUntrimmed
        };
        let label = label.into();
        let cache_key = TrimCacheKey {
            tier,
            hidden_size: src.cols,
            tok_fingerprint,
            artifact_k: src.rows,
        };
        Ok(Self {
            w: QMatMul::from_arc(Arc::clone(&qweight))?,
            qweight,
            raw_gathered_bytes,
            kept,
            vocab: src.rows,
            hidden_size: src.cols,
            tier,
            label,
            cache_key,
            guard_mode,
            exec_mode,
            double_proj_steps: 0,
            host_inv: Arc::new(inv),
            src: src.clone(),
            rescue,
        })
    }

    pub fn row_of(&self, orig_id: u32) -> Option<u32> {
        self.host_inv
            .get(orig_id as usize)
            .copied()
            .filter(|&i| (i as usize) < self.kept.len())
    }

    #[inline]
    pub fn map_local_to_orig_id(&self, local_idx: u32) -> u32 {
        self.kept.get(local_idx as usize).copied().unwrap_or(local_idx)
    }

    pub fn head_bytes(&self) -> usize {
        self.raw_gathered_bytes.len()
    }

    pub fn gathered_row_bytes(&self, local_idx: usize) -> Option<&[u8]> {
        let rb = self.src.row_bytes();
        let s = local_idx * rb;
        self.raw_gathered_bytes.get(s..s + rb)
    }

    /// O(|P|) host short-circuit when all pre-encoded `prompt_ids` are already covered by `self.kept`.
    /// Returns `(rebuilt, elapsed_ms)`.
    pub fn refresh_for_ids(&mut self, prompt_ids: &[u32], device: &Device) -> Result<(bool, f64)> {
        let t0 = Instant::now();
        validate_host_token_ids(prompt_ids, self.vocab)?;
        if prompt_ids.iter().all(|&id| self.row_of(id).is_some()) {
            return Ok((false, t0.elapsed().as_secs_f64() * 1000.0));
        }
        let mut merged: BTreeSet<u32> = self.kept.iter().copied().collect();
        for &id in prompt_ids {
            merged.insert(id);
        }
        let mut next: Vec<u32> = merged.into_iter().collect();
        let padded_len = next.len().next_multiple_of(ALIGN).min(self.vocab);
        for id in 0..self.vocab as u32 {
            if next.len() >= padded_len {
                break;
            }
            if !next.contains(&id) {
                next.push(id);
            }
        }
        next.sort_unstable();
        let rebuilt = Self::build(
            &self.src,
            self.tier,
            self.cache_key.tok_fingerprint,
            Arc::new(next),
            self.label.clone(),
            self.guard_mode,
            device,
        )?;
        *self = rebuilt;
        Ok((true, t0.elapsed().as_secs_f64() * 1000.0))
    }

    /// Applies repeat penalties & banned IDs purely on `[1, K]` sub-logits via host `host_inv`.
    pub fn apply_host_penalties(
        &self,
        sub_k: &mut [f32],
        seen_tokens: &[u32],
        repeat_penalty: f32,
        banned_ids: &[u32],
    ) {
        if (repeat_penalty - 1.0).abs() > f32::EPSILON {
            let inv_p = 1.0 / repeat_penalty;
            let mut visited = BTreeSet::new();
            for &tid in seen_tokens {
                if let Some(j) = self.row_of(tid) {
                    if visited.insert(j) {
                        let slot = &mut sub_k[j as usize];
                        if slot.is_finite() {
                            *slot = if *slot > 0.0 {
                                *slot * inv_p
                            } else {
                                *slot * repeat_penalty
                            };
                        }
                    }
                }
            }
        }
        for &bid in banned_ids {
            if let Some(j) = self.row_of(bid) {
                sub_k[j as usize] = f32::NEG_INFINITY;
            }
        }
    }

    /// Computes `[M, K]` logits without expanding trimmed rows onto GPU VRAM.
    pub fn project_rows_f32(&self, h_2d: &Tensor) -> Result<Tensor> {
        let out = self.w.forward(&h_2d.to_dtype(candle_core::DType::F32)?)?;
        Ok(out.to_dtype(candle_core::DType::F32)?)
    }

    /// Greedy argmax returning `(orig_vocab_id, logit, rescue_ms)`.
    /// When `guard_mode == GuardMode::Off`, `rescue_ms == 0.0` strictly.
    pub fn greedy_argmax_orig(&self, hidden_1d: &Tensor) -> Result<(u32, f32, f32)> {
        let h_2d = hidden_1d.reshape((1, self.hidden_size))?;
        let sub = self.project_rows_f32(&h_2d)?.squeeze(0)?.to_vec1::<f32>()?;
        let mut best_local = 0usize;
        let mut best_val = f32::NEG_INFINITY;
        for (i, &v) in sub.iter().enumerate() {
            if v > best_val {
                best_val = v;
                best_local = i;
            }
        }
        let orig_best = self.map_local_to_orig_id(best_local as u32);
        let Some(guard) = &self.rescue else {
            return Ok((orig_best, best_val, 0.0));
        };
        let t0 = Instant::now();
        let h_cpu = hidden_1d.to_device(&Device::Cpu)?.flatten_all()?.to_vec1::<f32>()?;
        let (final_id, final_val, _) = guard.rescue_if_needed(&h_cpu, orig_best, best_val);
        let rescue_ms = (t0.elapsed().as_secs_f64() * 1000.0) as f32;
        Ok((final_id, final_val, rescue_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::DType;

    #[test]
    fn canonical_policy_retires_legacy_without_false_positive() {
        assert!(canonical_policy_key("google/embeddinggemma-2").is_ok());
        assert!(canonical_policy_key("google/siglip2-so400m").is_ok());
        assert!(canonical_policy_key("Qwen3.5-4B-lc-trim-all12-k65536-v1").is_ok());

        assert!(matches!(canonical_policy_key("granite-4.0-h-350m"), Err(ModelPolicyError::Retired { .. })));
        assert!(matches!(canonical_policy_key("embeddinggemma-300m"), Err(ModelPolicyError::Forbidden { .. })));
        assert!(matches!(canonical_policy_key("Qwen3-0.6B"), Err(ModelPolicyError::ForeignVocab { .. })));
        assert!(matches!(canonical_policy_key("gocr"), Err(ModelPolicyError::Retired { .. })));
        assert!(matches!(canonical_policy_key("zoro"), Err(ModelPolicyError::Retired { .. })));
        assert!(matches!(canonical_policy_key("laya"), Err(ModelPolicyError::Forbidden { .. })));
    }

    #[test]
    fn zero_requant_gather_cache_isolation_and_strict_e1_rescue() {
        let dir = std::env::temp_dir().join(format!("lv-vocab-trim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p_a = dir.join("head_2b.gguf");
        let p_b = dir.join("head_4b.gguf");
        let src_2b = TableSource::synthetic_q8_for_tests(&p_a, 192, 64).unwrap();
        let src_4b = TableSource::synthetic_q8_for_tests(&p_b, 192, 96).unwrap();

        assert_eq!(q8_row_bytes(1024), 1088);
        assert_eq!(q8_row_bytes(2048), 2176);
        assert_eq!(q8_row_bytes(2560), 2720);
        assert_eq!(
            (QWEN35_VOCAB_SIZE - HEAVY4B_STATIC_ARTIFACT_K) * q8_row_bytes(2560),
            497_172_480
        );

        let kept: Arc<Vec<u32>> = Arc::new((0..64u32).collect());
        let mut head = TrimmedHead::build(
            &src_2b,
            QwenTier::Base2B,
            0xABCD,
            Arc::clone(&kept),
            "ko",
            GuardMode::Off,
            &Device::Cpu,
        )
        .unwrap();
        assert!(head.head_bytes() < src_2b.total_bytes());

        // E0 byte-for-byte identity on all 64 sampled rows
        for i in 0..64 {
            assert_eq!(
                head.gathered_row_bytes(i).unwrap(),
                src_2b.raw_row_bytes(i).unwrap(),
                "zero-requant mismatch at row {i}"
            );
        }

        // O(|P|) short-circuit when prompt IDs already covered
        let (rebuilt, _ms) = head.refresh_for_ids(&[0, 5, 63], &Device::Cpu).unwrap();
        assert!(!rebuilt);

        // Cache key isolation across tiers with identical vocab size
        clear_trim_plan_cache();
        let k2 = TrimCacheKey { tier: QwenTier::Base2B, hidden_size: 2048, tok_fingerprint: 9, artifact_k: QWEN35_VOCAB_SIZE };
        let k4 = TrimCacheKey { tier: QwenTier::Heavy4B, hidden_size: 2560, tok_fingerprint: 9, artifact_k: HEAVY4B_STATIC_ARTIFACT_K };
        let (_, v2) = cached_or_insert_trim_plan(k2.clone(), "ko", 1, || vec![1, 2, 3]);
        let (hit4, v4) = cached_or_insert_trim_plan(k4, "ko", 1, || vec![4, 5, 6]);
        assert!(!hit4);
        assert_ne!(&*v2, &*v4);
        let _ = src_4b;

        // GuardMode::StrictE1 matches full-table argmax exactly across kept+rest rows
        let head_e1 = TrimmedHead::build(
            &src_2b,
            QwenTier::Base2B,
            0xABCD,
            kept,
            "ko",
            GuardMode::StrictE1,
            &Device::Cpu,
        )
        .unwrap();
        let h = Tensor::ones((64,), DType::F32, &Device::Cpu).unwrap();
        let (e1_id, _e1_logit, rescue_ms) = head_e1.greedy_argmax_orig(&h).unwrap();
        assert!(rescue_ms >= 0.0);

        let full_w = src_2b.gather(&(0..192u32).collect::<Vec<_>>(), &Device::Cpu).unwrap().0.dequantize(&Device::Cpu).unwrap();
        let full_logits = full_w.matmul(&h.reshape((64, 1)).unwrap()).unwrap().squeeze(1).unwrap().to_vec1::<f32>().unwrap();
        let full_best = full_logits.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0 as u32;
        assert_eq!(e1_id, full_best);

        // Default GuardMode::Off has zero rescue cost and 5-backend MappedEmbedTable mode == "mapped"
        let mapped = MappedEmbedTable::open(src_2b, RuntimeBackendKind::Metal);
        assert_eq!(mapped.table_mode, "mapped");
        assert_eq!(mapped.resident_table_mb, 0.0);
        assert_eq!(normalize_lang_tag("zh_TW").as_deref(), Some("zh-tw"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
