//! Domain-neutral prompt-injected PointerModel, 0-Generation `JevHead`, and 64-bit `key_tag` mmap anchor cache.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::ops::softmax_last_dim;
use memmap2::Mmap;

pub const HIDDEN: usize = 2560;
pub const QK_DIM: usize = 256;
pub const DEFAULT_SCALE: f64 = 1.0 / 32.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerFact {
    pub key: String,
    pub value: String,
}

impl PointerFact {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerTask {
    pub system_prompt: String,
    pub title: String,
    pub facts: Vec<PointerFact>,
    pub option_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerPrompt {
    pub text: String,
    pub query_byte: usize,
    pub option_bytes: Vec<usize>,
}

impl PointerPrompt {
    pub fn render(task: &PointerTask) -> Self {
        let json_str = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string());
        let mut text = String::new();
        text.push_str("<|im_start|>system\n");
        text.push_str(&task.system_prompt);
        text.push_str("<|im_end|>\n<|im_start|>user\n{\n  \"task\": ");
        text.push_str(&json_str(&task.title));
        text.push_str(",\n  \"facts\": {");
        for (i, fact) in task.facts.iter().enumerate() {
            if i > 0 {
                text.push_str(", ");
            }
            text.push_str(&json_str(&fact.key));
            text.push_str(": ");
            text.push_str(&json_str(&fact.value));
        }
        text.push_str("},\n  \"zoro_jev_head_options\": [");
        let mut option_bytes = Vec::with_capacity(task.option_codes.len());
        for (i, code) in task.option_codes.iter().enumerate() {
            if i > 0 {
                text.push_str(", ");
            }
            text.push('"');
            text.push_str(code);
            option_bytes.push(text.len());
            text.push('"');
        }
        text.push_str("]\n}<|im_end|>\n<|im_start|>assistant\n");
        let query_byte = text.len() - 1;
        Self {
            text,
            query_byte,
            option_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PointerHidden {
    pub query: Vec<f32>,
    pub options: Vec<f32>,
}

pub trait PointerBackbone: Send + Sync {
    fn encode(&self, prompt: &PointerPrompt) -> Result<PointerHidden>;
}

pub struct JevHead {
    wq_t: Tensor,
    bq: Tensor,
    wk_t: Tensor,
    bk: Tensor,
    scale: f64,
}

impl JevHead {
    pub fn from_safetensors(path: &Path) -> Result<Self> {
        let t = candle_core::safetensors::load(path, &Device::Cpu)
            .with_context(|| format!("failed to load jev_head from {}", path.display()))?;
        let get = |name: &str| -> Result<Tensor> {
            Ok(t.get(name)
                .with_context(|| format!("missing tensor '{name}'"))?
                .to_dtype(DType::F32)?)
        };
        Self::from_tensors(
            get("q.weight")?,
            get("q.bias")?,
            get("k.weight")?,
            get("k.bias")?,
        )
    }

    pub fn from_tensors(q_w: Tensor, q_b: Tensor, k_w: Tensor, k_b: Tensor) -> Result<Self> {
        if q_w.dims() != [QK_DIM, HIDDEN]
            || q_b.dims() != [QK_DIM]
            || k_w.dims() != [QK_DIM, HIDDEN]
            || k_b.dims() != [QK_DIM]
        {
            bail!("unexpected JevHead tensor dimensions");
        }
        Ok(Self {
            wq_t: q_w.to_dtype(DType::F32)?.t()?.contiguous()?,
            bq: q_b.to_dtype(DType::F32)?,
            wk_t: k_w.to_dtype(DType::F32)?.t()?.contiguous()?,
            bk: k_b.to_dtype(DType::F32)?,
            scale: DEFAULT_SCALE,
        })
    }

    pub fn with_scale(mut self, scale: f64) -> Result<Self> {
        if !(scale > 0.0 && scale.is_finite()) {
            bail!("scale must be positive and finite: {scale}");
        }
        self.scale = scale;
        Ok(self)
    }

    pub fn scores(&self, query: &[f32], options: &[f32]) -> Result<Vec<f32>> {
        self.dot(query, options)?
            .to_vec2::<f32>()?
            .into_iter()
            .next()
            .context("empty JevHead output")
    }

    pub fn probabilities(&self, query: &[f32], options: &[f32]) -> Result<Vec<f32>> {
        let scaled = self.dot(query, options)?.affine(self.scale, 0.0)?;
        let rows = softmax_last_dim(&scaled)?.to_vec2::<f32>()?;
        rows.into_iter().next().context("empty JevHead probabilities")
    }

    fn dot(&self, query: &[f32], options: &[f32]) -> Result<Tensor> {
        if query.len() != HIDDEN {
            bail!("query len {} != {HIDDEN}", query.len());
        }
        if options.is_empty() || !options.len().is_multiple_of(HIDDEN) {
            bail!("options len {} must be positive multiple of {HIDDEN}", options.len());
        }
        let n = options.len() / HIDDEN;
        let h = Tensor::from_slice(query, (1, HIDDEN), &Device::Cpu)?;
        let e = Tensor::from_slice(options, (n, HIDDEN), &Device::Cpu)?;
        let q = h.matmul(&self.wq_t)?.broadcast_add(&self.bq)?;
        let k = e.matmul(&self.wk_t)?.broadcast_add(&self.bk)?;
        Ok(q.matmul(&k.t()?)?)
    }
}

pub struct PointerModel {
    backbone: Box<dyn PointerBackbone>,
    head: JevHead,
}

impl PointerModel {
    pub fn new(backbone: Box<dyn PointerBackbone>, head: JevHead) -> Self {
        Self { backbone, head }
    }

    pub fn probabilities(&self, task: &PointerTask) -> Result<Vec<f32>> {
        if task.option_codes.is_empty() {
            bail!("option_codes cannot be empty");
        }
        let prompt = PointerPrompt::render(task);
        let hidden = self.backbone.encode(&prompt)?;
        if hidden.query.len() != HIDDEN || hidden.options.len() != task.option_codes.len() * HIDDEN {
            bail!("PointerBackbone returned unexpected hidden dimension");
        }
        self.head.probabilities(&hidden.query, &hidden.options)
    }
}

/// 64-bit `key_tag` mmap anchor cache storing projected key vectors on host disk (0 B GPU VRAM).
/// File layout: `[magic "JNAK" (4B), dim (u32 LE), count (u32 LE), reserved (4B)]`,
/// followed by sorted `(key_tag: u64 LE, vec: [f32; dim])` records.
const ANCHOR_MAGIC: &[u8; 4] = b"JNAK";

pub struct JevNgramAnchorCache {
    mmap: Mmap,
    dim: usize,
    count: usize,
}

impl JevNgramAnchorCache {
    pub fn key_tag(ngram: &str, scope: &str) -> u64 {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(scope.as_bytes());
        h.update([0xFF]);
        h.update(ngram.as_bytes());
        let d = h.finalize();
        u64::from_le_bytes(d[0..8].try_into().unwrap())
    }

    pub fn write(path: &Path, dim: usize, entries: &[(u64, Vec<f32>)]) -> Result<()> {
        let mut sorted: Vec<(u64, &[f32])> = entries.iter().map(|(k, v)| (*k, v.as_slice())).collect();
        sorted.sort_by_key(|(k, _)| *k);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = File::create(path)?;
        f.write_all(ANCHOR_MAGIC)?;
        f.write_all(&(dim as u32).to_le_bytes())?;
        f.write_all(&(sorted.len() as u32).to_le_bytes())?;
        f.write_all(&0u32.to_le_bytes())?;
        for (k, v) in sorted {
            if v.len() != dim {
                bail!("anchor vector len {} != dim {dim}", v.len());
            }
            f.write_all(&k.to_le_bytes())?;
            for &x in v {
                f.write_all(&x.to_le_bytes())?;
            }
        }
        Ok(())
    }

    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        if mmap.len() < 16 || &mmap[0..4] != ANCHOR_MAGIC {
            bail!("invalid JevNgramAnchorCache header");
        }
        let dim = u32::from_le_bytes(mmap[4..8].try_into().unwrap()) as usize;
        let count = u32::from_le_bytes(mmap[8..12].try_into().unwrap()) as usize;
        let stride = 8 + dim * 4;
        if mmap.len() < 16 + count * stride {
            bail!("truncated JevNgramAnchorCache file");
        }
        Ok(Self { mmap, dim, count })
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn lookup(&self, key_tag: u64) -> Option<Vec<f32>> {
        let stride = 8 + self.dim * 4;
        let mut lo = 0usize;
        let mut hi = self.count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let off = 16 + mid * stride;
            let k = u64::from_le_bytes(self.mmap[off..off + 8].try_into().unwrap());
            match k.cmp(&key_tag) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => {
                    let mut out = Vec::with_capacity(self.dim);
                    let base = off + 8;
                    for i in 0..self.dim {
                        let p = base + i * 4;
                        out.push(f32::from_le_bytes(self.mmap[p..p + 4].try_into().unwrap()));
                    }
                    return Some(out);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scripted {
        prefer: usize,
    }

    impl PointerBackbone for Scripted {
        fn encode(&self, prompt: &PointerPrompt) -> Result<PointerHidden> {
            let n = prompt.option_bytes.len();
            let mut query = vec![0f32; HIDDEN];
            query[3] = 8.0;
            let mut options = vec![0f32; n * HIDDEN];
            for i in 0..n {
                options[i * HIDDEN + if i == self.prefer { 3 } else { 10 }] = 8.0;
            }
            Ok(PointerHidden { query, options })
        }
    }

    #[test]
    fn prompt_and_jev_head_select_aligned_option() -> Result<()> {
        let mut w = vec![0f32; QK_DIM * HIDDEN];
        for i in 0..QK_DIM {
            w[i * HIDDEN + i] = 1.0;
        }
        let proj = Tensor::from_vec(w, (QK_DIM, HIDDEN), &Device::Cpu)?;
        let zeros = Tensor::zeros(QK_DIM, DType::F32, &Device::Cpu)?;
        let head = JevHead::from_tensors(proj.clone(), zeros.clone(), proj, zeros)?;
        let model = PointerModel::new(Box::new(Scripted { prefer: 1 }), head);
        let p = model.probabilities(&PointerTask {
            system_prompt: "Domain-neutral selector".into(),
            title: "Verify claim".into(),
            facts: vec![PointerFact::new("evidence", "verified")],
            option_codes: vec!["HOLD".into(), "PASS".into(), "REJECT".into()],
        })?;
        assert!(p[1] > p[0] && p[1] > p[2]);
        Ok(())
    }

    #[test]
    fn anchor_cache_mmap_lookup_roundtrips_zero_vram() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("lv_jev_anchor_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("anchors.jnak");
        let k1 = JevNgramAnchorCache::key_tag("commercial invoice", "trade");
        let k2 = JevNgramAnchorCache::key_tag("bill of lading", "trade");
        JevNgramAnchorCache::write(&path, 4, &[(k1, vec![1.0, 2.0, 3.0, 4.0]), (k2, vec![5.0, 6.0, 7.0, 8.0])])?;
        let cache = JevNgramAnchorCache::open(&path)?;
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.lookup(k1), Some(vec![1.0, 2.0, 3.0, 4.0]));
        assert_eq!(cache.lookup(k2), Some(vec![5.0, 6.0, 7.0, 8.0]));
        assert_eq!(cache.lookup(0), None);
        drop(cache);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
