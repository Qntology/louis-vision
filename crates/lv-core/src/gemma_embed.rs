//! Unified `google/embeddinggemma-2` 768d multimodal embedder promoted to Tier 1 (`lv-core`).
//!
//! Combines:
//! - `GemmaTokenTable`: host-mmaped BF16/F16/F32 input embedding table (`0 B` GPU VRAM for full table),
//!   pinning only special & prompt prefix rows on host RAM.
//! - `GemmaTextModel`: 24-layer bidirectional Gemma4 text backbone (`4` full attention layers `[5,11,17,23]`
//!   at `head_dim=512`, `20` sliding-window layers at `head_dim=256`, context PLE projection,
//!   F32 `embedding_projection` `512 -> 768` + F32 RMSNorms + `Q8_0` layer projections).
//! - `GemmaEmbedder`: unified `[BOS] ... [EOS]` 768d L2-normalized encoder for text (`&str`),
//!   images (`gemma_vision::RgbImage`), and video frames, enforcing cosine `>= 0.999` vs F32 reference.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Result};
use candle_core::quantized::QTensor;
use candle_core::safetensors::MmapedSafetensors;
use candle_core::{DType, Device, IndexOp, Tensor, D};
use candle_nn::{ops, Linear, Module, VarBuilder};
use safetensors::Dtype as SafeDtype;
use tokenizers::Tokenizer;

use crate::device::{accel_op_ok, device_from_env_or_cpu, AccelOp};
use crate::gemma_vision::{
    RgbImage, VisionTower, IMAGE_MAX_SOFT_TOKENS, VIDEO_MAX_SOFT_TOKENS,
};
use crate::qcache::{cache_path, Fingerprint, QCache};
use crate::quant::{load_lin, Bits, Kernel, Lin, Proj, QuantRecipe, TowerQuant};

pub const WEIGHTS_FILE: &str = "model.safetensors";
pub const TEXT_WEIGHTS_FILE: &str = "model-text.safetensors";
pub const TOKENIZER_FILE: &str = "tokenizer.json";
pub const EMBED_TOKENS: &str = "language_model.embed_tokens.weight";

pub const MODEL_ID: &str = "google/embeddinggemma-2";
pub const OUTPUT_DIM: usize = 768;
pub const MAX_INPUT_TOKENS: usize = 8192;
pub const IMAGE_TOKENS: usize = IMAGE_MAX_SOFT_TOKENS;
pub const VIDEO_FRAME_TOKENS: usize = VIDEO_MAX_SOFT_TOKENS;

pub const TEXT_HIDDEN_SIZE: usize = 512;
pub const TEXT_NUM_LAYERS: usize = 24;
pub const TEXT_NUM_HEADS: usize = 4;
pub const TEXT_NUM_KV_HEADS: usize = 2;
pub const TEXT_HEAD_DIM: usize = 256;
pub const TEXT_GLOBAL_HEAD_DIM: usize = 512;
pub const TEXT_GLOBAL_KV_HEADS: usize = 1;
pub const TEXT_VOCAB_SIZE: usize = 262_144;
pub const SLIDING_WINDOW: usize = 512;
pub const ROPE_THETA_FULL: f64 = 1_000_000.0;
pub const ROPE_THETA_SLIDING: f64 = 10_000.0;
pub const FULL_ATTENTION_LAYERS: [usize; 4] = [5, 11, 17, 23];

const INTERMEDIATE_SIZE: usize = 2048;
const PLE_DIM: usize = 512;
const RMS_EPS: f64 = 1e-6;
const MASKED: f32 = -1e9;
const BIDIRECTIONAL_SLIDING_WINDOW: usize = 257;

pub const BOS_ID: u32 = 2;
pub const EOS_ID: u32 = 1;
pub const BOI_ID: u32 = 255_999;
pub const EOI_ID: u32 = 258_882;

pub const PROMPT_QUERY: &str = "task: search result | query: ";
pub const PROMPT_DOCUMENT: &str = "title: none | text: ";
pub const PROMPT_SIMILARITY: &str = "task: sentence similarity | query: ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Query,
    Document,
    Similarity,
}

pub fn with_prompt(role: Role, text: &str) -> String {
    let prefix = match role {
        Role::Query => PROMPT_QUERY,
        Role::Document => PROMPT_DOCUMENT,
        Role::Similarity => PROMPT_SIMILARITY,
    };
    format!("{prefix}{text}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GemmaAttention {
    Sliding,
    Full,
}

pub fn attention_kind(layer: usize) -> Option<GemmaAttention> {
    if layer >= TEXT_NUM_LAYERS {
        return None;
    }
    Some(if FULL_ATTENTION_LAYERS.contains(&layer) {
        GemmaAttention::Full
    } else {
        GemmaAttention::Sliding
    })
}

pub fn weights_file(dir: &Path) -> Option<PathBuf> {
    [WEIGHTS_FILE, TEXT_WEIGHTS_FILE]
        .iter()
        .map(|f| dir.join(f))
        .find(|p| p.is_file())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GemmaQuantCache {
    Off,
    Hit,
    Built,
    Unwritable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GemmaLoadOptions {
    pub quant: QuantRecipe,
    pub kernel: Kernel,
    pub tower_bits: Option<Bits>,
    pub load_vision_on_init: bool,
}

pub enum GemmaPart<'a> {
    Text(&'a str),
    Image(&'a RgbImage),
    Video(&'a [RgbImage]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowType {
    Bf16,
    F16,
    F32,
}

impl RowType {
    fn width(self) -> usize {
        match self {
            Self::Bf16 | Self::F16 => 2,
            Self::F32 => 4,
        }
    }
}

/// Host-mmaped token embedding table (`0 B` GPU VRAM for the full matrix).
pub struct GemmaTokenTable {
    file: MmapedSafetensors,
    name: String,
    rows: usize,
    cols: usize,
    kind: RowType,
    pinned: HashMap<u32, Box<[f32]>>,
    device: Device,
}

impl GemmaTokenTable {
    pub fn open(model_file: &Path, rows: usize, cols: usize, device: &Device) -> candle_core::Result<Self> {
        Self::open_named(model_file, EMBED_TOKENS, rows, cols, device)
    }

    pub fn open_named(
        model_file: &Path,
        name: &str,
        rows: usize,
        cols: usize,
        device: &Device,
    ) -> candle_core::Result<Self> {
        let file = unsafe { MmapedSafetensors::new(model_file)? };
        let (kind, shape) = {
            let view = file.get(name)?;
            let kind = match view.dtype() {
                SafeDtype::BF16 => RowType::Bf16,
                SafeDtype::F16 => RowType::F16,
                SafeDtype::F32 => RowType::F32,
                other => {
                    return Err(candle_core::Error::Msg(format!(
                        "{name}: unsupported token dtype {other:?}"
                    )));
                }
            };
            (kind, view.shape().to_vec())
        };
        if shape != [rows, cols] {
            return Err(candle_core::Error::Msg(format!(
                "{name}: shape mismatch {shape:?}, expected [{rows}, {cols}]"
            )));
        }
        Ok(Self {
            file,
            name: name.to_string(),
            rows,
            cols,
            kind,
            pinned: HashMap::new(),
            device: device.clone(),
        })
    }

    pub fn pin(&mut self, ids: &[u32]) -> candle_core::Result<()> {
        let view = self.file.get(&self.name)?;
        let data = view.data();
        for &id in ids {
            if self.pinned.contains_key(&id) {
                continue;
            }
            let mut row = vec![0f32; self.cols];
            self.read_row(data, id, &mut row)?;
            self.pinned.insert(id, row.into_boxed_slice());
        }
        Ok(())
    }

    pub fn pinned_len(&self) -> usize {
        self.pinned.len()
    }

    pub fn table_mode(&self) -> &'static str {
        "mapped"
    }

    pub fn resident_table_bytes(&self) -> usize {
        self.pinned.len() * self.cols * std::mem::size_of::<f32>()
    }

    pub fn rows(&self, ids: &[u32]) -> candle_core::Result<Tensor> {
        let mut out = vec![0f32; ids.len() * self.cols];
        let mut data: Option<&[u8]> = None;
        let view;
        if ids.iter().any(|id| !self.pinned.contains_key(id)) {
            view = self.file.get(&self.name)?;
            data = Some(view.data());
        }
        for (i, &id) in ids.iter().enumerate() {
            let dst = &mut out[i * self.cols..(i + 1) * self.cols];
            match self.pinned.get(&id) {
                Some(row) => dst.copy_from_slice(row),
                None => self.read_row(data.expect("mmap slice initialized"), id, dst)?,
            }
        }
        Tensor::from_vec(out, (ids.len(), self.cols), &self.device)
    }

    fn read_row(&self, data: &[u8], id: u32, dst: &mut [f32]) -> candle_core::Result<()> {
        let id = id as usize;
        if id >= self.rows {
            return Err(candle_core::Error::Msg(format!(
                "token id {id} out of range {}",
                self.rows
            )));
        }
        let w = self.kind.width();
        let start = id * self.cols * w;
        let bytes = data.get(start..start + self.cols * w).ok_or_else(|| {
            candle_core::Error::Msg(format!("{}: truncated file at row {id}", self.name))
        })?;
        match self.kind {
            RowType::Bf16 => {
                for (d, b) in dst.iter_mut().zip(bytes.chunks_exact(2)) {
                    *d = f32::from_bits((u16::from_le_bytes([b[0], b[1]]) as u32) << 16);
                }
            }
            RowType::F16 => {
                for (d, b) in dst.iter_mut().zip(bytes.chunks_exact(2)) {
                    *d = half::f16::from_bits(u16::from_le_bytes([b[0], b[1]])).to_f32();
                }
            }
            RowType::F32 => {
                for (d, b) in dst.iter_mut().zip(bytes.chunks_exact(4)) {
                    *d = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                }
            }
        }
        Ok(())
    }
}

struct RmsNorm {
    weight: Option<Tensor>,
}

impl RmsNorm {
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        if let Some(w) = &self.weight {
            if !x.device().is_cpu() && accel_op_ok(x.device(), AccelOp::DecoderOps) {
                if let Ok(y) = candle_core::decoder_ops::rmsnorm_fused(x, w, RMS_EPS as f32, false) {
                    return Ok(y);
                }
            }
        }
        let mean_sq = x.sqr()?.mean_keepdim(D::Minus1)?;
        let inv = mean_sq.affine(1.0, RMS_EPS)?.powf(-0.5)?;
        let y = x.broadcast_mul(&inv)?;
        match &self.weight {
            Some(w) => y.broadcast_mul(w),
            None => Ok(y),
        }
    }
}

struct RopeCacheEntry {
    cos: Tensor,
    sin: Tensor,
    cos_half: Tensor,
    sin_half: Tensor,
}

fn rope_tables(theta: f64, head_dim: usize, seq: usize, dev: &Device) -> candle_core::Result<RopeCacheEntry> {
    let half = head_dim / 2;
    let inv: Vec<f32> = (0..half)
        .map(|i| (1.0 / theta.powf((2 * i) as f64 / head_dim as f64)) as f32)
        .collect();
    let mut cos = vec![0f32; seq * head_dim];
    let mut sin = vec![0f32; seq * head_dim];
    let mut cos_h = vec![0f32; seq * half];
    let mut sin_h = vec![0f32; seq * half];
    for t in 0..seq {
        let base = t * head_dim;
        let hbase = t * half;
        let tf = t as f32;
        for (i, &f) in inv.iter().enumerate() {
            let (s, c) = (tf * f).sin_cos();
            cos[base + i] = c;
            cos[base + half + i] = c;
            sin[base + i] = s;
            sin[base + half + i] = s;
            cos_h[hbase + i] = c;
            sin_h[hbase + i] = s;
        }
    }
    Ok(RopeCacheEntry {
        cos: Tensor::from_vec(cos, (seq, head_dim), dev)?,
        sin: Tensor::from_vec(sin, (seq, head_dim), dev)?,
        cos_half: Tensor::from_vec(cos_h, (seq, half), dev)?,
        sin_half: Tensor::from_vec(sin_h, (seq, half), dev)?,
    })
}

fn apply_rope(x: &Tensor, rope: &RopeCacheEntry) -> candle_core::Result<Tensor> {
    if !x.device().is_cpu() && accel_op_ok(x.device(), AccelOp::Rope) {
        let bhtd = x.transpose(1, 2)?.contiguous()?;
        if let Ok(y) = candle_nn::rotary_emb::rope(&bhtd, &rope.cos_half, &rope.sin_half) {
            return y.transpose(1, 2);
        }
    }
    let d = x.dim(D::Minus1)?;
    let x1 = x.narrow(D::Minus1, 0, d / 2)?;
    let x2 = x.narrow(D::Minus1, d / 2, d / 2)?;
    let rotated = Tensor::cat(&[&x2.neg()?, &x1], D::Minus1)?;
    let cos = rope.cos.unsqueeze(0)?.unsqueeze(2)?;
    let sin = rope.sin.unsqueeze(0)?.unsqueeze(2)?;
    x.broadcast_mul(&cos)?.broadcast_add(&rotated.broadcast_mul(&sin)?)
}

fn repeat_kv(x: Tensor, reps: usize) -> candle_core::Result<Tensor> {
    if reps == 1 {
        return Ok(x);
    }
    let (b, kv, t, d) = x.dims4()?;
    x.unsqueeze(2)?
        .expand((b, kv, reps, t, d))?
        .contiguous()?
        .reshape((b, kv * reps, t, d))
}

fn attention_mask(t: usize, sliding: bool, bidirectional: bool, dev: &Device) -> candle_core::Result<Tensor> {
    let mut v = vec![0f32; t * t];
    for q in 0..t {
        for k in 0..t {
            let causal_ok = bidirectional || k <= q;
            let window_ok = !sliding
                || if bidirectional {
                    q.abs_diff(k) <= BIDIRECTIONAL_SLIDING_WINDOW
                } else {
                    k + SLIDING_WINDOW > q
                };
            if !(causal_ok && window_ok) {
                v[q * t + k] = MASKED;
            }
        }
    }
    Tensor::from_vec(v, (t, t), dev)
}

struct SelfAttention {
    q_proj: Lin,
    k_proj: Lin,
    v_proj: Lin,
    o_proj: Lin,
    q_norm: RmsNorm,
    k_norm: RmsNorm,
    v_norm: RmsNorm,
    heads: usize,
    kv_heads: usize,
    head_dim: usize,
}

impl SelfAttention {
    fn forward(&self, x: &Tensor, rope: &RopeCacheEntry, mask: Option<&Tensor>) -> candle_core::Result<Tensor> {
        let (b, t, _) = x.dims3()?;
        let q = self.q_proj.forward(x)?.reshape((b, t, self.heads, self.head_dim))?;
        let q = apply_rope(&self.q_norm.forward(&q)?, rope)?;
        let k = self.k_proj.forward(x)?.reshape((b, t, self.kv_heads, self.head_dim))?;
        let k = apply_rope(&self.k_norm.forward(&k)?, rope)?;
        let v = self.v_proj.forward(x)?.reshape((b, t, self.kv_heads, self.head_dim))?;
        let v = self.v_norm.forward(&v)?;

        if mask.is_none() && b == 1 && x.device().is_cpu() && x.dtype() == DType::F32 {
            let q_vec = q.contiguous()?.flatten_all()?.to_vec1::<f32>()?;
            let k_vec = k.contiguous()?.flatten_all()?.to_vec1::<f32>()?;
            let v_vec = v.contiguous()?.flatten_all()?.to_vec1::<f32>()?;
            let mut out = vec![0.0f32; t * self.heads * self.head_dim];
            let cfg = premium_discount_cpu_kernels::CpuStreamAttnCfg {
                num_heads: self.heads,
                num_kv_heads: self.kv_heads,
                head_dim: self.head_dim,
                scale: 1.0,
                causal: false,
                past_offset: 0,
            };
            premium_discount_cpu_kernels::stream_attention_f32(&q_vec, &k_vec, &v_vec, t, t, &cfg, &mut out);
            let out = Tensor::from_vec(out, (1, t, self.heads * self.head_dim), x.device())?;
            return self.o_proj.forward(&out);
        }

        let q = q.transpose(1, 2)?.contiguous()?;
        let k = k.transpose(1, 2)?.contiguous()?;
        let v = v.transpose(1, 2)?.contiguous()?;

        if mask.is_none()
            && self.head_dim <= 256
            && !x.device().is_cpu()
            && accel_op_ok(x.device(), AccelOp::FlashAttn)
        {
            let cfg = candle_nn::flash_attn::FlashAttnConfig::new(1.0);
            if let Ok(out) = candle_nn::flash_attn::flash_attn_bhsd(&q, &k, &v, &cfg) {
                let out = out.transpose(1, 2)?.reshape((b, t, self.heads * self.head_dim))?;
                return self.o_proj.forward(&out);
            }
        }

        let reps = self.heads / self.kv_heads;
        let k = repeat_kv(k, reps)?;
        let v = repeat_kv(v, reps)?;
        let scores = q.matmul(&k.transpose(2, 3)?.contiguous()?)?;
        let scores = match mask {
            Some(m) => scores.broadcast_add(m)?,
            None => scores,
        };
        let probs = ops::softmax_last_dim(&scores)?;
        let out = probs.matmul(&v)?;
        let out = out.transpose(1, 2)?.reshape((b, t, self.heads * self.head_dim))?;
        self.o_proj.forward(&out)
    }
}

struct GemmaLayer {
    kind: GemmaAttention,
    attn: SelfAttention,
    gate: Lin,
    up: Lin,
    down: Lin,
    input_norm: RmsNorm,
    post_attn_norm: RmsNorm,
    pre_ff_norm: RmsNorm,
    post_ff_norm: RmsNorm,
    ple_gate: Lin,
    ple_proj: Lin,
    post_ple_norm: RmsNorm,
    layer_scalar: Tensor,
}

impl GemmaLayer {
    fn forward(&self, h: &Tensor, ple: &Tensor, rope: &RopeCacheEntry, mask: Option<&Tensor>) -> candle_core::Result<Tensor> {
        let x = self.input_norm.forward(h)?;
        let x = self.attn.forward(&x, rope, mask)?;
        let x = self.post_attn_norm.forward(&x)?;
        let (h, x) = if let Some(ref w) = self.pre_ff_norm.weight {
            if !h.device().is_cpu() && accel_op_ok(h.device(), AccelOp::DecoderOps) {
                match candle_core::decoder_ops::residual_rmsnorm_inplace(
                    h.clone(),
                    Some(&x),
                    w,
                    RMS_EPS as f32,
                    false,
                ) {
                    Ok((res, normed)) => (res, normed),
                    Err(_) => {
                        let h = h.broadcast_add(&x)?;
                        let x = self.pre_ff_norm.forward(&h)?;
                        (h, x)
                    }
                }
            } else {
                let h = h.broadcast_add(&x)?;
                let x = self.pre_ff_norm.forward(&h)?;
                (h, x)
            }
        } else {
            let h = h.broadcast_add(&x)?;
            let x = self.pre_ff_norm.forward(&h)?;
            (h, x)
        };

        let x = self.gate.forward(&x)?.gelu()?.broadcast_mul(&self.up.forward(&x)?)?;
        let x = self.down.forward(&x)?;
        let x = self.post_ff_norm.forward(&x)?;
        let h = h.broadcast_add(&x)?;

        let g = self.ple_gate.forward(&h)?.gelu()?;
        let g = g.broadcast_mul(ple)?;
        let p = self.post_ple_norm.forward(&self.ple_proj.forward(&g)?)?;
        let h = h.broadcast_add(&p)?;
        h.broadcast_mul(&self.layer_scalar)
    }
}

fn layer_key(layer: usize, name: &str) -> String {
    format!("layers.{layer}.{name}")
}

#[derive(Debug, Clone, Copy)]
pub struct GemmaTextConfig {
    pub hidden_size: usize,
    pub intermediate_size: usize,
    pub num_layers: usize,
    pub num_heads: usize,
    pub num_kv_heads: usize,
    pub head_dim: usize,
    pub global_head_dim: usize,
    pub global_kv_heads: usize,
    pub vocab_size: usize,
    pub ple_dim: usize,
    pub output_dim: usize,
}

impl Default for GemmaTextConfig {
    fn default() -> Self {
        Self {
            hidden_size: TEXT_HIDDEN_SIZE,
            intermediate_size: INTERMEDIATE_SIZE,
            num_layers: TEXT_NUM_LAYERS,
            num_heads: TEXT_NUM_HEADS,
            num_kv_heads: TEXT_NUM_KV_HEADS,
            head_dim: TEXT_HEAD_DIM,
            global_head_dim: TEXT_GLOBAL_HEAD_DIM,
            global_kv_heads: TEXT_GLOBAL_KV_HEADS,
            vocab_size: TEXT_VOCAB_SIZE,
            ple_dim: PLE_DIM,
            output_dim: OUTPUT_DIM,
        }
    }
}

/// 24-layer Gemma4 text backbone projecting 512d tokens into 768d L2-normalized embeddings.
pub struct GemmaTextModel {
    cfg: GemmaTextConfig,
    tokens: GemmaTokenTable,
    ple_proj: Linear,
    ple_norm: RmsNorm,
    layers: Vec<GemmaLayer>,
    norm: RmsNorm,
    projection: Linear,
    device: Device,
    bidirectional: bool,
    rope_cache: Mutex<HashMap<usize, (Arc<RopeCacheEntry>, Arc<RopeCacheEntry>)>>,
}

impl GemmaTextModel {
    pub fn load(model_file: &Path, device: &Device, bidirectional: bool) -> candle_core::Result<Self> {
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[model_file], DType::F32, device)? };
        let tokens = GemmaTokenTable::open(model_file, TEXT_VOCAB_SIZE, TEXT_HIDDEN_SIZE, device)?;
        Self::from_vb(&vb, tokens, device, bidirectional, &QuantRecipe::Off, Kernel::default(), None)
    }

    pub fn from_vb(
        vb: &VarBuilder,
        tokens: GemmaTokenTable,
        device: &Device,
        bidirectional: bool,
        recipe: &QuantRecipe,
        kernel: Kernel,
        cache: Option<&mut QCache>,
    ) -> candle_core::Result<Self> {
        Self::from_vb_with_config(
            GemmaTextConfig::default(),
            vb,
            tokens,
            device,
            bidirectional,
            recipe,
            kernel,
            cache,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_vb_with_config(
        cfg: GemmaTextConfig,
        vb: &VarBuilder,
        tokens: GemmaTokenTable,
        device: &Device,
        bidirectional: bool,
        recipe: &QuantRecipe,
        kernel: Kernel,
        mut cache: Option<&mut QCache>,
    ) -> candle_core::Result<Self> {
        let lm = vb.pp("language_model");
        let ple = lm.pp("ple");
        let ple_proj = Linear::new(
            ple.pp("per_layer_model_projection")
                .get((cfg.num_layers * cfg.ple_dim, cfg.hidden_size), "weight")?,
            None,
        );
        let ple_norm = RmsNorm {
            weight: Some(ple.pp("per_layer_projection_norm").get(cfg.ple_dim, "weight")?),
        };
        let norm = RmsNorm {
            weight: Some(lm.pp("norm").get(cfg.hidden_size, "weight")?),
        };
        let projection = Linear::new(
            vb.get((cfg.output_dim, cfg.hidden_size), "language_model.embedding_projection.weight")?,
            None,
        );

        let mut layers = Vec::with_capacity(cfg.num_layers);
        for i in 0..cfg.num_layers {
            let l = lm.pp("layers").pp(i.to_string());
            let kind = if FULL_ATTENTION_LAYERS.contains(&i) || (cfg.num_layers < TEXT_NUM_LAYERS && i + 1 == cfg.num_layers) {
                GemmaAttention::Full
            } else {
                GemmaAttention::Sliding
            };
            let (head_dim, kv_heads) = match kind {
                GemmaAttention::Full => (cfg.global_head_dim, cfg.global_kv_heads),
                GemmaAttention::Sliding => (cfg.head_dim, cfg.num_kv_heads),
            };
            let a = l.pp("self_attn");
            let q_dim = cfg.num_heads * head_dim;
            let kv_dim = kv_heads * head_dim;
            let attn = SelfAttention {
                q_proj: load_lin(&a.pp("q_proj"), (q_dim, cfg.hidden_size), recipe.bits(i, Proj::Q), kernel, &mut cache, layer_key(i, "q_proj"))?,
                k_proj: load_lin(&a.pp("k_proj"), (kv_dim, cfg.hidden_size), recipe.bits(i, Proj::K), kernel, &mut cache, layer_key(i, "k_proj"))?,
                v_proj: load_lin(&a.pp("v_proj"), (kv_dim, cfg.hidden_size), recipe.bits(i, Proj::V), kernel, &mut cache, layer_key(i, "v_proj"))?,
                o_proj: load_lin(&a.pp("o_proj"), (cfg.hidden_size, q_dim), recipe.bits(i, Proj::O), kernel, &mut cache, layer_key(i, "o_proj"))?,
                q_norm: RmsNorm {
                    weight: Some(a.pp("q_norm").get(head_dim, "weight")?),
                },
                k_norm: RmsNorm {
                    weight: Some(a.pp("k_norm").get(head_dim, "weight")?),
                },
                v_norm: RmsNorm { weight: None },
                heads: cfg.num_heads,
                kv_heads,
                head_dim,
            };
            let m = l.pp("mlp");
            let hidden_norm = |name: &str| -> candle_core::Result<RmsNorm> {
                Ok(RmsNorm {
                    weight: Some(l.pp(name).get(cfg.hidden_size, "weight")?),
                })
            };
            let p = l.pp("ple_block");
            layers.push(GemmaLayer {
                kind,
                attn,
                gate: load_lin(&m.pp("gate_proj"), (cfg.intermediate_size, cfg.hidden_size), recipe.bits(i, Proj::Gate), kernel, &mut cache, layer_key(i, "gate_proj"))?,
                up: load_lin(&m.pp("up_proj"), (cfg.intermediate_size, cfg.hidden_size), recipe.bits(i, Proj::Up), kernel, &mut cache, layer_key(i, "up_proj"))?,
                down: load_lin(&m.pp("down_proj"), (cfg.hidden_size, cfg.intermediate_size), recipe.bits(i, Proj::Down), kernel, &mut cache, layer_key(i, "down_proj"))?,
                input_norm: hidden_norm("input_layernorm")?,
                post_attn_norm: hidden_norm("post_attention_layernorm")?,
                pre_ff_norm: hidden_norm("pre_feedforward_layernorm")?,
                post_ff_norm: hidden_norm("post_feedforward_layernorm")?,
                ple_gate: load_lin(
                    &p.pp("per_layer_input_gate"),
                    (cfg.ple_dim, cfg.hidden_size),
                    recipe.bits(i, Proj::PleGate),
                    kernel,
                    &mut cache,
                    layer_key(i, "per_layer_input_gate"),
                )?,
                ple_proj: load_lin(
                    &p.pp("per_layer_projection"),
                    (cfg.hidden_size, cfg.ple_dim),
                    recipe.bits(i, Proj::PleProj),
                    kernel,
                    &mut cache,
                    layer_key(i, "per_layer_projection"),
                )?,
                post_ple_norm: RmsNorm {
                    weight: Some(p.pp("post_per_layer_input_norm").get(cfg.hidden_size, "weight")?),
                },
                layer_scalar: l.get(1, "layer_scalar")?,
            });
        }

        Ok(Self {
            cfg,
            tokens,
            ple_proj,
            ple_norm,
            layers,
            norm,
            projection,
            device: device.clone(),
            bidirectional,
            rope_cache: Mutex::new(HashMap::new()),
        })
    }

    fn cached_ropes(&self, t: usize) -> candle_core::Result<(Arc<RopeCacheEntry>, Arc<RopeCacheEntry>)> {
        if let Some(pair) = self.rope_cache.lock().unwrap_or_else(|p| p.into_inner()).get(&t) {
            return Ok(pair.clone());
        }
        let sliding = Arc::new(rope_tables(ROPE_THETA_SLIDING, self.cfg.head_dim, t, &self.device)?);
        let full = Arc::new(rope_tables(ROPE_THETA_FULL, self.cfg.global_head_dim, t, &self.device)?);
        let mut cache = self.rope_cache.lock().unwrap_or_else(|p| p.into_inner());
        if cache.len() >= 64 {
            cache.clear();
        }
        cache.insert(t, (sliding.clone(), full.clone()));
        Ok((sliding, full))
    }

    pub fn token_embeddings(&self, ids: &[u32]) -> candle_core::Result<Tensor> {
        self.tokens
            .rows(ids)?
            .affine((self.cfg.hidden_size as f64).sqrt(), 0.0)
    }

    pub fn quantized_tensors(&self) -> Vec<(String, &QTensor)> {
        let mut out = Vec::new();
        for (i, l) in self.layers.iter().enumerate() {
            let named: [(&str, &Lin); 9] = [
                ("q_proj", &l.attn.q_proj),
                ("k_proj", &l.attn.k_proj),
                ("v_proj", &l.attn.v_proj),
                ("o_proj", &l.attn.o_proj),
                ("gate_proj", &l.gate),
                ("up_proj", &l.up),
                ("down_proj", &l.down),
                ("per_layer_input_gate", &l.ple_gate),
                ("per_layer_projection", &l.ple_proj),
            ];
            for (name, lin) in named {
                if let Some(q) = lin.qtensor() {
                    out.push((layer_key(i, name), q));
                }
            }
        }
        out
    }

    pub fn layer_weight_bytes(&self) -> usize {
        self.layers
            .iter()
            .map(|l| {
                [
                    &l.attn.q_proj,
                    &l.attn.k_proj,
                    &l.attn.v_proj,
                    &l.attn.o_proj,
                    &l.gate,
                    &l.up,
                    &l.down,
                    &l.ple_gate,
                    &l.ple_proj,
                ]
                .iter()
                .map(|x| x.weight_bytes())
                .sum::<usize>()
            })
            .sum()
    }

    pub fn pin_tokens(&mut self, ids: &[u32]) -> candle_core::Result<()> {
        self.tokens.pin(ids)
    }

    pub fn token_table_mode(&self) -> &'static str {
        self.tokens.table_mode()
    }

    pub fn forward(&self, ids: &[u32]) -> candle_core::Result<Tensor> {
        let h = self.token_embeddings(ids)?.unsqueeze(0)?;
        self.forward_embeds(&h)
    }

    pub fn forward_embeds(&self, input: &Tensor) -> candle_core::Result<Tensor> {
        let t = input.dim(1)?;
        let dev = &self.device;
        let mut h = input.clone();

        let ple = self
            .ple_proj
            .forward(&h)?
            .affine(1.0 / (self.cfg.hidden_size as f64).sqrt(), 0.0)?
            .reshape((1, t, self.cfg.num_layers, self.cfg.ple_dim))?;
        let ple = self.ple_norm.forward(&ple)?;

        let (rope_s, rope_f) = self.cached_ropes(t)?;
        let mask_sliding = if self.bidirectional && t <= BIDIRECTIONAL_SLIDING_WINDOW + 1 {
            None
        } else {
            Some(attention_mask(t, true, self.bidirectional, dev)?)
        };
        let mask_full = if self.bidirectional {
            None
        } else {
            Some(attention_mask(t, false, false, dev)?)
        };

        for (i, layer) in self.layers.iter().enumerate() {
            let ple_i = ple.i((.., .., i, ..))?;
            h = match layer.kind {
                GemmaAttention::Full => layer.forward(&h, &ple_i, &rope_f, mask_full.as_ref())?,
                GemmaAttention::Sliding => layer.forward(&h, &ple_i, &rope_s, mask_sliding.as_ref())?,
            };
        }
        self.norm.forward(&h)
    }

    pub fn embed(&self, ids: &[u32]) -> candle_core::Result<Vec<f32>> {
        self.embed_hidden(&self.forward(ids)?)
    }

    pub fn embed_hidden(&self, hidden: &Tensor) -> candle_core::Result<Vec<f32>> {
        let pooled_hidden = hidden.mean(1)?;
        let pooled = self.projection.forward(&pooled_hidden)?;
        let norm = pooled.sqr()?.sum_keepdim(D::Minus1)?.sqrt()?;
        pooled.broadcast_div(&norm)?.squeeze(0)?.to_vec1::<f32>()
    }
}

enum EmbedderBackend {
    Model {
        tokenizer: Tokenizer,
        model: GemmaTextModel,
        weights: PathBuf,
        device: Device,
        vision: Mutex<Option<Arc<VisionTower>>>,
        quant_cache: GemmaQuantCache,
        tower_bits: Option<Bits>,
        kernel: Kernel,
    },
    /// Explicit deterministic 768d unit-vector projection backend for zero-checkpoint Tier 1 & diag unit tests.
    Deterministic768 { seed: u64 },
}

/// Unified 768d multimodal embedder shared across `louis-vision`, `premium-discount`, and `logis-center`.
pub struct GemmaEmbedder {
    backend: EmbedderBackend,
}

impl GemmaEmbedder {
    pub fn load(dir: &Path) -> Result<Self> {
        Self::load_with(dir, GemmaLoadOptions::default())
    }

    pub fn load_with(dir: &Path, options: GemmaLoadOptions) -> Result<Self> {
        let device = device_from_env_or_cpu();
        Self::load_with_on(dir, options, device)
    }

    pub fn load_with_on(dir: &Path, options: GemmaLoadOptions, device: Device) -> Result<Self> {
        let tokenizer = Tokenizer::from_file(dir.join(TOKENIZER_FILE))
            .map_err(|e| anyhow!("failed to load embeddinggemma tokenizer: {e}"))?;
        let weights = weights_file(dir)
            .ok_or_else(|| anyhow!("missing {WEIGHTS_FILE} or {TEXT_WEIGHTS_FILE} in {}", dir.display()))?;
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[&weights], DType::F32, &device)? };
        let tokens = GemmaTokenTable::open(&weights, TEXT_VOCAB_SIZE, TEXT_HIDDEN_SIZE, &device)?;
        let fp = options.quant.key("text").and_then(|k| Fingerprint::of(&weights, "text", &k).ok());
        let mut cache = fp.as_ref().and_then(|f| QCache::load(&cache_path(&weights, f), f, &device));
        let cached = cache.as_ref().map_or(0, |c| c.len());
        let mut model = GemmaTextModel::from_vb(
            &vb,
            tokens,
            &device,
            true,
            &options.quant,
            options.kernel,
            cache.as_mut(),
        )?;
        let quant_cache = match &fp {
            None => GemmaQuantCache::Off,
            Some(f) => {
                let tensors = model.quantized_tensors();
                if cached > 0 && cached == tensors.len() {
                    GemmaQuantCache::Hit
                } else if QCache::save(&cache_path(&weights, f), f, &tensors).is_ok() {
                    GemmaQuantCache::Built
                } else {
                    GemmaQuantCache::Unwritable
                }
            }
        };
        let mut pin_ids = vec![BOS_ID, EOS_ID, BOI_ID, EOI_ID];
        for prompt in [PROMPT_QUERY, PROMPT_DOCUMENT, PROMPT_SIMILARITY] {
            if let Ok(enc) = tokenizer.encode(prompt, false) {
                pin_ids.extend_from_slice(enc.get_ids());
            }
        }
        pin_ids.sort_unstable();
        pin_ids.dedup();
        model.pin_tokens(&pin_ids)?;

        let vision_opt = if options.load_vision_on_init {
            Some(Arc::new(load_vision_tower(
                &weights,
                &device,
                options.tower_bits,
                options.kernel,
            )?))
        } else {
            None
        };

        Ok(Self {
            backend: EmbedderBackend::Model {
                tokenizer,
                model,
                weights,
                device,
                vision: Mutex::new(vision_opt),
                quant_cache,
                tower_bits: options.tower_bits,
                kernel: options.kernel,
            },
        })
    }

    /// Explicit deterministic 768d unit-vector embedder for zero-checkpoint unit & harness tests.
    pub fn deterministic_for_tests(seed: u64) -> Self {
        Self {
            backend: EmbedderBackend::Deterministic768 { seed },
        }
    }

    pub fn output_dim(&self) -> usize {
        OUTPUT_DIM
    }

    pub fn quant_cache(&self) -> GemmaQuantCache {
        match &self.backend {
            EmbedderBackend::Model { quant_cache, .. } => *quant_cache,
            EmbedderBackend::Deterministic768 { .. } => GemmaQuantCache::Off,
        }
    }

    pub fn token_table_mode(&self) -> &'static str {
        match &self.backend {
            EmbedderBackend::Model { model, .. } => model.token_table_mode(),
            EmbedderBackend::Deterministic768 { .. } => "mapped",
        }
    }

    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        self.embed_parts(&[GemmaPart::Text(text)])
    }

    pub fn embed_image(&self, img: &RgbImage) -> Result<Vec<f32>> {
        self.embed_parts(&[GemmaPart::Image(img)])
    }

    pub fn token_count(&self, text: &str) -> Result<usize> {
        match &self.backend {
            EmbedderBackend::Model { tokenizer, .. } => {
                let enc = tokenizer
                    .encode(text, false)
                    .map_err(|e| anyhow!("tokenization failed: {e}"))?;
                Ok(enc.get_ids().len() + 2)
            }
            EmbedderBackend::Deterministic768 { .. } => Ok(text.split_whitespace().count() + 2),
        }
    }

    pub fn embed_parts(&self, parts: &[GemmaPart<'_>]) -> Result<Vec<f32>> {
        if parts.is_empty() {
            bail!("embed_parts requires at least one part");
        }
        match &self.backend {
            EmbedderBackend::Deterministic768 { seed } => Ok(synthetic_embed_parts(parts, *seed)),
            EmbedderBackend::Model {
                tokenizer,
                model,
                weights,
                device,
                vision,
                tower_bits,
                kernel,
                ..
            } => {
                let mut segments = Vec::with_capacity(parts.len() + 2);
                segments.push(model.token_embeddings(&[BOS_ID])?);
                let get_vision = || -> Result<Arc<VisionTower>> {
                    let mut guard = vision.lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(v) = guard.as_ref() {
                        return Ok(Arc::clone(v));
                    }
                    let tower = Arc::new(load_vision_tower(weights, device, *tower_bits, *kernel)?);
                    *guard = Some(Arc::clone(&tower));
                    Ok(tower)
                };
                for part in parts {
                    match part {
                        GemmaPart::Text(text) => {
                            let enc = tokenizer
                                .encode(*text, false)
                                .map_err(|e| anyhow!("tokenization failed: {e}"))?;
                            segments.push(model.token_embeddings(enc.get_ids())?);
                        }
                        GemmaPart::Image(image) => {
                            let feat = get_vision()?.encode_image(image, IMAGE_TOKENS)?;
                            let open = model.token_embeddings(&[BOI_ID])?;
                            let close = model.token_embeddings(&[EOI_ID])?;
                            segments.push(Tensor::cat(&[open, feat, close], 0)?);
                        }
                        GemmaPart::Video(frames) => {
                            let vis = get_vision()?;
                            for frame in frames.iter() {
                                let feat = vis.encode_image(frame, VIDEO_FRAME_TOKENS)?;
                                let open = model.token_embeddings(&[BOI_ID])?;
                                let close = model.token_embeddings(&[EOI_ID])?;
                                segments.push(Tensor::cat(&[open, feat, close], 0)?);
                            }
                        }
                    }
                }
                segments.push(model.token_embeddings(&[EOS_ID])?);
                let total: usize = segments.iter().map(|s| s.dim(0).unwrap_or(0)).sum();
                if total > MAX_INPUT_TOKENS {
                    bail!("input uses {total} tokens, limit is {MAX_INPUT_TOKENS}");
                }
                let input = Tensor::cat(&segments, 0)?.unsqueeze(0)?;
                let hidden = model.forward_embeds(&input)?;
                Ok(model.embed_hidden(&hidden)?)
            }
        }
    }
}

fn load_vision_tower(
    weights: &Path,
    device: &Device,
    tower_bits: Option<Bits>,
    kernel: Kernel,
) -> Result<VisionTower> {
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[weights], DType::F32, device)? };
    let fp = tower_bits.and_then(|b| Fingerprint::of(weights, "vision", &format!("eg2_vision_{}_v1", b.name())).ok());
    let mut cache = fp.as_ref().and_then(|f| QCache::load(&cache_path(weights, f), f, device));
    let cached = cache.as_ref().map_or(0, |c| c.len());
    let quant = TowerQuant {
        bits: tower_bits,
        kernel,
        cache: cache.as_mut(),
        keep_f32: TowerQuant::default_keep("vision"),
    };
    let tower = VisionTower::load_with(&vb, device, quant)?;
    if let Some(f) = &fp {
        let t = tower.quantized_tensors();
        if cached == 0 || cached != t.len() {
            let _ = QCache::save(&cache_path(weights, f), f, &t);
        }
    }
    Ok(tower)
}

pub fn check_total_tokens(total: usize) -> Result<usize> {
    if total > MAX_INPUT_TOKENS {
        bail!("input uses {total} tokens, limit is {MAX_INPUT_TOKENS}");
    }
    Ok(total)
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for i in 0..n {
        let x = a[i] as f64;
        let y = b[i] as f64;
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= 1e-12 {
        0.0
    } else {
        (dot / denom) as f32
    }
}

pub fn normalize_l2(v: &mut [f32]) {
    let norm = v.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>().sqrt();
    if norm > 1e-12 {
        let inv = (1.0 / norm) as f32;
        for x in v.iter_mut() {
            *x *= inv;
        }
    }
}

fn synthetic_embed_parts(parts: &[GemmaPart<'_>], seed: u64) -> Vec<f32> {
    let mut out = vec![0.0f32; OUTPUT_DIM];
    for part in parts {
        match part {
            GemmaPart::Text(s) => accumulate_char_ngrams(s, seed, &mut out),
            GemmaPart::Image(img) => accumulate_image_signature(img, seed, &mut out),
            GemmaPart::Video(frames) => {
                for f in frames.iter() {
                    accumulate_image_signature(f, seed, &mut out);
                }
            }
        }
    }
    normalize_l2(&mut out);
    out
}

fn accumulate_char_ngrams(text: &str, seed: u64, dst: &mut [f32]) {
    let lower = text.to_ascii_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    if chars.is_empty() {
        dst[0] += 1.0;
        return;
    }
    for w in 1..=3 {
        if chars.len() < w {
            continue;
        }
        for win in chars.windows(w) {
            let mut h = seed ^ 0x9E37_79B9_7F4A_7C15 ^ (w as u64);
            for &ch in win {
                h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9) ^ (ch as u64);
                h ^= h >> 27;
            }
            let idx = (h as usize) % dst.len();
            let sign = if (h >> 63) == 0 { 1.0 } else { -1.0 };
            dst[idx] += sign;
        }
    }
}

fn accumulate_image_signature(img: &RgbImage, seed: u64, dst: &mut [f32]) {
    // Project image spatial color & structural histogram into shared 768d space so attached photo
    // consistency checks reflect cross-modal alignment cleanly in deterministic fixtures.
    let step_x = (img.width / 8).max(1);
    let step_y = (img.height / 8).max(1);
    for y in (0..img.height).step_by(step_y) {
        for x in (0..img.width).step_by(step_x) {
            let p = (y * img.width + x) * 3;
            let (r, g, b) = (img.pixels[p], img.pixels[p + 1], img.pixels[p + 2]);
            let key = format!("px:{}:{}:{}", r / 32, g / 32, b / 32);
            accumulate_char_ngrams(&key, seed, dst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_synthetic_gemma_safetensors(path: &Path, cfg: &GemmaTextConfig) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut tensors = HashMap::<String, Tensor>::new();
        let det = |rows: usize, cols: usize, seed: u64| -> Tensor {
            let v: Vec<f32> = (0..rows * cols)
                .map(|i| (((i as u64 + seed) as f32) * 0.137).sin() * 0.08)
                .collect();
            Tensor::from_vec(v, (rows, cols), &Device::Cpu).unwrap()
        };
        let ones = |n: usize| Tensor::ones((n,), DType::F32, &Device::Cpu).unwrap();

        tensors.insert(EMBED_TOKENS.into(), det(cfg.vocab_size, cfg.hidden_size, 1).to_dtype(DType::BF16).unwrap());
        tensors.insert("language_model.ple.per_layer_model_projection.weight".into(), det(cfg.num_layers * cfg.ple_dim, cfg.hidden_size, 2));
        tensors.insert("language_model.ple.per_layer_projection_norm.weight".into(), ones(cfg.ple_dim));
        tensors.insert("language_model.norm.weight".into(), ones(cfg.hidden_size));
        tensors.insert("language_model.embedding_projection.weight".into(), det(cfg.output_dim, cfg.hidden_size, 3));

        for i in 0..cfg.num_layers {
            let is_full = i + 1 == cfg.num_layers;
            let (hd, kvh) = if is_full { (cfg.global_head_dim, cfg.global_kv_heads) } else { (cfg.head_dim, cfg.num_kv_heads) };
            let qd = cfg.num_heads * hd;
            let kvd = kvh * hd;
            let p = format!("language_model.layers.{i}");
            tensors.insert(format!("{p}.self_attn.q_proj.weight"), det(qd, cfg.hidden_size, 10 + i as u64));
            tensors.insert(format!("{p}.self_attn.k_proj.weight"), det(kvd, cfg.hidden_size, 20 + i as u64));
            tensors.insert(format!("{p}.self_attn.v_proj.weight"), det(kvd, cfg.hidden_size, 30 + i as u64));
            tensors.insert(format!("{p}.self_attn.o_proj.weight"), det(cfg.hidden_size, qd, 40 + i as u64));
            tensors.insert(format!("{p}.self_attn.q_norm.weight"), ones(hd));
            tensors.insert(format!("{p}.self_attn.k_norm.weight"), ones(hd));
            tensors.insert(format!("{p}.mlp.gate_proj.weight"), det(cfg.intermediate_size, cfg.hidden_size, 50 + i as u64));
            tensors.insert(format!("{p}.mlp.up_proj.weight"), det(cfg.intermediate_size, cfg.hidden_size, 60 + i as u64));
            tensors.insert(format!("{p}.mlp.down_proj.weight"), det(cfg.hidden_size, cfg.intermediate_size, 70 + i as u64));
            tensors.insert(format!("{p}.input_layernorm.weight"), ones(cfg.hidden_size));
            tensors.insert(format!("{p}.post_attention_layernorm.weight"), ones(cfg.hidden_size));
            tensors.insert(format!("{p}.pre_feedforward_layernorm.weight"), ones(cfg.hidden_size));
            tensors.insert(format!("{p}.post_feedforward_layernorm.weight"), ones(cfg.hidden_size));
            tensors.insert(format!("{p}.ple_block.per_layer_input_gate.weight"), det(cfg.ple_dim, cfg.hidden_size, 80 + i as u64));
            tensors.insert(format!("{p}.ple_block.per_layer_projection.weight"), det(cfg.hidden_size, cfg.ple_dim, 90 + i as u64));
            tensors.insert(format!("{p}.ple_block.post_per_layer_input_norm.weight"), ones(cfg.hidden_size));
            tensors.insert(format!("{p}.layer_scalar"), Tensor::ones((1,), DType::F32, &Device::Cpu).unwrap());
        }
        candle_core::safetensors::save(&tensors, path).unwrap();
    }

    #[test]
    fn f32_vs_q8_text_backbone_maintains_high_cosine_and_host_mmap() {
        let dir = std::env::temp_dir().join(format!("lv-gemma-embed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let weights = dir.join("model.safetensors");
        let cfg = GemmaTextConfig {
            hidden_size: 64,
            intermediate_size: 128,
            num_layers: 2,
            num_heads: 2,
            num_kv_heads: 1,
            head_dim: 32,
            global_head_dim: 64,
            global_kv_heads: 1,
            vocab_size: 128,
            ple_dim: 64,
            output_dim: OUTPUT_DIM,
        };
        write_synthetic_gemma_safetensors(&weights, &cfg);

        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[&weights], DType::F32, &Device::Cpu).unwrap() };
        let tok_f32 = GemmaTokenTable::open(&weights, cfg.vocab_size, cfg.hidden_size, &Device::Cpu).unwrap();
        let m_f32 = GemmaTextModel::from_vb_with_config(cfg, &vb, tok_f32, &Device::Cpu, true, &QuantRecipe::Off, Kernel::Dequant, None).unwrap();

        let tok_q8 = GemmaTokenTable::open(&weights, cfg.vocab_size, cfg.hidden_size, &Device::Cpu).unwrap();
        let q8_recipe = QuantRecipe::Uniform {
            groups: crate::quant::ProjGroup::ALL.to_vec(),
            bits: Bits::Q8_0,
        };
        let m_q8 = GemmaTextModel::from_vb_with_config(cfg, &vb, tok_q8, &Device::Cpu, true, &q8_recipe, Kernel::Dequant, None).unwrap();

        assert_eq!(m_q8.token_table_mode(), "mapped");
        assert!(m_q8.layer_weight_bytes() < m_f32.layer_weight_bytes() / 2);

        let e_f32 = m_f32.embed(&[2, 5, 19, 44, 1]).unwrap();
        let e_q8 = m_q8.embed(&[2, 5, 19, 44, 1]).unwrap();
        assert_eq!(e_f32.len(), OUTPUT_DIM);
        assert_eq!(e_q8.len(), OUTPUT_DIM);
        let cos = cosine(&e_f32, &e_q8);
        assert!(cos >= 0.999, "expected F32 vs Q8_0 cosine >= 0.999, got {cos}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_dir_fails_cleanly_and_layer_counts_match_config() {
        assert!(GemmaEmbedder::load(Path::new("nonexistent-gemma-dir")).is_err());
        let full = (0..TEXT_NUM_LAYERS).filter(|&i| attention_kind(i) == Some(GemmaAttention::Full)).count();
        assert_eq!(full, 4);
        assert_eq!(with_prompt(Role::Query, "abc"), "task: search result | query: abc");
    }
}
