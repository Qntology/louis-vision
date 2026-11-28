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
use crate::gemma_audio::AudioTower;
use crate::gemma_vision::{
    RgbImage, VisionTower, IMAGE_MAX_SOFT_TOKENS, VIDEO_MAX_SOFT_TOKENS,
};
use crate::qcache::{cache_path, Fingerprint, QCache};
use crate::quant::{
    load_lin_with_encoder, Bits, Kernel, Lin, Proj, QuantEncoder, QuantRecipe, TowerQuant,
    TowerQuantPlan,
};

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
pub const BOA_ID: u32 = 256_000;
pub const EOA_ID: u32 = 258_883;
/// 장문 텍스트 천공 기준 토큰 수 (`LocalPerforated` 모드에서 초과 시 문단 고정 + 인접 확장 적용)
pub const LONG_DOC_PERFORATE_THRESHOLD: usize = 512;

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

/// 임베딩 실행 모드.
/// - `CanonicalFull` (기본): 색인·검색·정규 벡터와 100% 동일한 전체 경로.
/// - `LocalPerforated`: 장문(`> 512`토큰) 문단 고정+확장, 오디오 유성 연속 청크(`HALO_LEFT_TOKENS=16`) 적용.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbedMode {
    #[default]
    CanonicalFull,
    LocalPerforated,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GemmaLoadOptions {
    pub quant: QuantRecipe,
    pub kernel: Kernel,
    pub encoder: QuantEncoder,
    pub tower_bits: Option<Bits>,
    pub vision_quant: Option<TowerQuantPlan>,
    pub audio_quant: Option<TowerQuantPlan>,
    pub load_vision_on_init: bool,
    pub load_audio_on_init: bool,
}

pub enum GemmaPart<'a> {
    Text(&'a str),
    Image(&'a RgbImage),
    Video(&'a [RgbImage]),
    /// 16 kHz mono f32 PCM audio (40 ms per token, capped at 30 s = 480,000 samples).
    Audio(&'a [f32]),
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

/// Zero-allocation CPU F32 streaming bidirectional sliding-window attention (`|qi - ki| <= window`).
/// Eliminates `[t, t]` score/mask allocation (256 MB at `t = 8192`) while matching dense masked softmax.
fn stream_sliding_window_attention_f32(
    q: &[f32],
    k: &[f32],
    v: &[f32],
    seq_len: usize,
    heads: usize,
    kv_heads: usize,
    head_dim: usize,
    window: usize,
    out: &mut [f32],
) {
    let reps = (heads / kv_heads.max(1)).max(1);
    let q_stride = heads * head_dim;
    let kv_stride = kv_heads * head_dim;
    out.fill(0.0);

    for qi in 0..seq_len {
        let k_start = qi.saturating_sub(window);
        let k_end = (qi + window + 1).min(seq_len);
        for h in 0..heads {
            let kvh = h / reps;
            let q_off = qi * q_stride + h * head_dim;
            let q_row = &q[q_off..q_off + head_dim];
            let o_row = &mut out[q_off..q_off + head_dim];

            let mut m = f32::NEG_INFINITY;
            let mut l = 0.0f32;
            for ki in k_start..k_end {
                let kv_off = ki * kv_stride + kvh * head_dim;
                let s = premium_discount_cpu_kernels::dot_f32_simd(q_row, &k[kv_off..kv_off + head_dim]);
                let v_row = &v[kv_off..kv_off + head_dim];
                if s > m {
                    let corr = if m.is_finite() { (m - s).exp() } else { 0.0 };
                    l = l * corr + 1.0;
                    for d in 0..head_dim {
                        o_row[d] = o_row[d] * corr + v_row[d];
                    }
                    m = s;
                } else {
                    let w = (s - m).exp();
                    l += w;
                    for d in 0..head_dim {
                        o_row[d] += w * v_row[d];
                    }
                }
            }
            if l > 0.0 {
                let inv_l = 1.0 / l;
                for d in 0..head_dim {
                    o_row[d] *= inv_l;
                }
            }
        }
    }
}

impl SelfAttention {
    fn forward(
        &self,
        x: &Tensor,
        rope: &RopeCacheEntry,
        mask: Option<&Tensor>,
        sliding_window: Option<usize>,
    ) -> candle_core::Result<Tensor> {
        let (b, t, _) = x.dims3()?;
        let q = self.q_proj.forward(x)?.reshape((b, t, self.heads, self.head_dim))?;
        let q = apply_rope(&self.q_norm.forward(&q)?, rope)?;
        let k = self.k_proj.forward(x)?.reshape((b, t, self.kv_heads, self.head_dim))?;
        let k = apply_rope(&self.k_norm.forward(&k)?, rope)?;
        let v = self.v_proj.forward(x)?.reshape((b, t, self.kv_heads, self.head_dim))?;
        let v = self.v_norm.forward(&v)?;

        if b == 1 && x.device().is_cpu() && x.dtype() == DType::F32 {
            if mask.is_none() && sliding_window.is_none() {
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
            if let Some(win) = sliding_window {
                let q_vec = q.contiguous()?.flatten_all()?.to_vec1::<f32>()?;
                let k_vec = k.contiguous()?.flatten_all()?.to_vec1::<f32>()?;
                let v_vec = v.contiguous()?.flatten_all()?.to_vec1::<f32>()?;
                let mut out = vec![0.0f32; t * self.heads * self.head_dim];
                stream_sliding_window_attention_f32(
                    &q_vec,
                    &k_vec,
                    &v_vec,
                    t,
                    self.heads,
                    self.kv_heads,
                    self.head_dim,
                    win,
                    &mut out,
                );
                let out = Tensor::from_vec(out, (1, t, self.heads * self.head_dim), x.device())?;
                return self.o_proj.forward(&out);
            }
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
    fn forward(
        &self,
        h: &Tensor,
        ple: &Tensor,
        rope: &RopeCacheEntry,
        mask: Option<&Tensor>,
        sliding_window: Option<usize>,
    ) -> candle_core::Result<Tensor> {
        let x = self.input_norm.forward(h)?;
        let x = self.attn.forward(&x, rope, mask, sliding_window)?;
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
        cache: Option<&mut QCache>,
    ) -> candle_core::Result<Self> {
        Self::from_vb_with_config_and_encoder(
            cfg,
            vb,
            tokens,
            device,
            bidirectional,
            recipe,
            kernel,
            QuantEncoder::CandleFast,
            cache,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_vb_with_config_and_encoder(
        cfg: GemmaTextConfig,
        vb: &VarBuilder,
        tokens: GemmaTokenTable,
        device: &Device,
        bidirectional: bool,
        recipe: &QuantRecipe,
        kernel: Kernel,
        encoder: QuantEncoder,
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
                q_proj: load_lin_with_encoder(&a.pp("q_proj"), (q_dim, cfg.hidden_size), recipe.bits(i, Proj::Q), kernel, encoder, &mut cache, layer_key(i, "q_proj"))?,
                k_proj: load_lin_with_encoder(&a.pp("k_proj"), (kv_dim, cfg.hidden_size), recipe.bits(i, Proj::K), kernel, encoder, &mut cache, layer_key(i, "k_proj"))?,
                v_proj: load_lin_with_encoder(&a.pp("v_proj"), (kv_dim, cfg.hidden_size), recipe.bits(i, Proj::V), kernel, encoder, &mut cache, layer_key(i, "v_proj"))?,
                o_proj: load_lin_with_encoder(&a.pp("o_proj"), (cfg.hidden_size, q_dim), recipe.bits(i, Proj::O), kernel, encoder, &mut cache, layer_key(i, "o_proj"))?,
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
                gate: load_lin_with_encoder(&m.pp("gate_proj"), (cfg.intermediate_size, cfg.hidden_size), recipe.bits(i, Proj::Gate), kernel, encoder, &mut cache, layer_key(i, "gate_proj"))?,
                up: load_lin_with_encoder(&m.pp("up_proj"), (cfg.intermediate_size, cfg.hidden_size), recipe.bits(i, Proj::Up), kernel, encoder, &mut cache, layer_key(i, "up_proj"))?,
                down: load_lin_with_encoder(&m.pp("down_proj"), (cfg.hidden_size, cfg.intermediate_size), recipe.bits(i, Proj::Down), kernel, encoder, &mut cache, layer_key(i, "down_proj"))?,
                input_norm: hidden_norm("input_layernorm")?,
                post_attn_norm: hidden_norm("post_attention_layernorm")?,
                pre_ff_norm: hidden_norm("pre_feedforward_layernorm")?,
                post_ff_norm: hidden_norm("post_feedforward_layernorm")?,
                ple_gate: load_lin_with_encoder(
                    &p.pp("per_layer_input_gate"),
                    (cfg.ple_dim, cfg.hidden_size),
                    recipe.bits(i, Proj::PleGate),
                    kernel,
                    encoder,
                    &mut cache,
                    layer_key(i, "per_layer_input_gate"),
                )?,
                ple_proj: load_lin_with_encoder(
                    &p.pp("per_layer_projection"),
                    (cfg.hidden_size, cfg.ple_dim),
                    recipe.bits(i, Proj::PleProj),
                    kernel,
                    encoder,
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
        let use_cpu_streaming_window =
            self.bidirectional && t > BIDIRECTIONAL_SLIDING_WINDOW + 1 && dev.is_cpu();
        let sliding_window = use_cpu_streaming_window.then_some(BIDIRECTIONAL_SLIDING_WINDOW);
        let mask_sliding = if self.bidirectional && (t <= BIDIRECTIONAL_SLIDING_WINDOW + 1 || use_cpu_streaming_window) {
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
                GemmaAttention::Full => layer.forward(&h, &ple_i, &rope_f, mask_full.as_ref(), None)?,
                GemmaAttention::Sliding => layer.forward(&h, &ple_i, &rope_s, mask_sliding.as_ref(), sliding_window)?,
            };
        }
        self.norm.forward(&h)
    }

    pub fn embedding_projection_weight(&self) -> &Tensor {
        self.projection.weight()
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
        audio: Mutex<Option<Arc<AudioTower>>>,
        quant_cache: GemmaQuantCache,
        vision_plan: TowerQuantPlan,
        audio_plan: TowerQuantPlan,
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
        let mut model = GemmaTextModel::from_vb_with_config_and_encoder(
            GemmaTextConfig::default(),
            &vb,
            tokens,
            &device,
            true,
            &options.quant,
            options.kernel,
            options.encoder,
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
        let mut pin_ids = vec![BOS_ID, EOS_ID, BOI_ID, EOI_ID, BOA_ID, EOA_ID];
        for prompt in [PROMPT_QUERY, PROMPT_DOCUMENT, PROMPT_SIMILARITY] {
            if let Ok(enc) = tokenizer.encode(prompt, false) {
                pin_ids.extend_from_slice(enc.get_ids());
            }
        }
        pin_ids.sort_unstable();
        pin_ids.dedup();
        model.pin_tokens(&pin_ids)?;

        let vision_plan = options.vision_quant.clone().unwrap_or_else(|| TowerQuantPlan {
            default_bits: options.tower_bits,
            keep_f32_layers: TowerQuant::default_keep("vision"),
            group_overrides: Vec::new(),
            encoder: options.encoder,
        });
        let audio_plan = options.audio_quant.clone().unwrap_or_else(|| TowerQuantPlan {
            default_bits: options.tower_bits,
            keep_f32_layers: TowerQuant::default_keep("audio"),
            group_overrides: Vec::new(),
            encoder: options.encoder,
        });

        let vision_opt = if options.load_vision_on_init {
            Some(Arc::new(load_vision_tower(&weights, &device, &vision_plan, options.kernel)?))
        } else {
            None
        };
        let audio_opt = if options.load_audio_on_init {
            Some(Arc::new(load_audio_tower(&weights, &device, &audio_plan, options.kernel)?))
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
                audio: Mutex::new(audio_opt),
                quant_cache,
                vision_plan,
                audio_plan,
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

    pub fn embed_audio(&self, pcm_16khz: &[f32]) -> Result<Vec<f32>> {
        self.embed_parts(&[GemmaPart::Audio(pcm_16khz)])
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

    /// Canonical full-fidelity 768d embedding (default for index/query consistency).
    pub fn embed_parts(&self, parts: &[GemmaPart<'_>]) -> Result<Vec<f32>> {
        self.embed_parts_with_mode(parts, EmbedMode::CanonicalFull)
    }

    /// Embeds arbitrary combinations of text, image, video, and 16 kHz audio parts under the chosen [`EmbedMode`].
    pub fn embed_parts_with_mode(&self, parts: &[GemmaPart<'_>], mode: EmbedMode) -> Result<Vec<f32>> {
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
                audio,
                vision_plan,
                audio_plan,
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
                    let tower = Arc::new(load_vision_tower(weights, device, vision_plan, *kernel)?);
                    *guard = Some(Arc::clone(&tower));
                    Ok(tower)
                };
                let get_audio = || -> Result<Arc<AudioTower>> {
                    let mut guard = audio.lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(a) = guard.as_ref() {
                        return Ok(Arc::clone(a));
                    }
                    let tower = Arc::new(load_audio_tower(weights, device, audio_plan, *kernel)?);
                    *guard = Some(Arc::clone(&tower));
                    Ok(tower)
                };
                for part in parts {
                    match part {
                        GemmaPart::Text(text) => {
                            let enc_full = tokenizer
                                .encode(*text, false)
                                .map_err(|e| anyhow!("tokenization failed: {e}"))?;
                            if mode == EmbedMode::LocalPerforated
                                && enc_full.get_ids().len() > LONG_DOC_PERFORATE_THRESHOLD
                            {
                                let perf = select_perforated_document_text(text, 0.65);
                                let enc_perf = tokenizer
                                    .encode(perf.as_str(), false)
                                    .map_err(|e| anyhow!("tokenization failed: {e}"))?;
                                segments.push(model.token_embeddings(enc_perf.get_ids())?);
                            } else {
                                segments.push(model.token_embeddings(enc_full.get_ids())?);
                            }
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
                        GemmaPart::Audio(pcm) => {
                            let aud = get_audio()?;
                            let feat = match mode {
                                EmbedMode::CanonicalFull => aud.encode_pcm(pcm)?,
                                EmbedMode::LocalPerforated => aud.encode_pcm_voiced(pcm, 0.015)?.0,
                            };
                            let open = model.token_embeddings(&[BOA_ID])?;
                            let close = model.token_embeddings(&[EOA_ID])?;
                            segments.push(Tensor::cat(&[open, feat, close], 0)?);
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
    plan: &TowerQuantPlan,
    kernel: Kernel,
) -> Result<VisionTower> {
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[weights], DType::F32, device)? };
    let fp = plan.recipe_key("vision").and_then(|k| Fingerprint::of(weights, "vision", &k).ok());
    let mut cache = fp.as_ref().and_then(|f| QCache::load(&cache_path(weights, f), f, device));
    let cached = cache.as_ref().map_or(0, |c| c.len());
    let quant = TowerQuant::from_plan(plan, kernel, cache.as_mut());
    let tower = VisionTower::load_with(&vb, device, quant)?;
    if let Some(f) = &fp {
        let t = tower.quantized_tensors();
        if cached == 0 || cached != t.len() {
            let _ = QCache::save(&cache_path(weights, f), f, &t);
        }
    }
    Ok(tower)
}

fn load_audio_tower(
    weights: &Path,
    device: &Device,
    plan: &TowerQuantPlan,
    kernel: Kernel,
) -> Result<AudioTower> {
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[weights], DType::F32, device)? };
    let fp = plan.recipe_key("audio").and_then(|k| Fingerprint::of(weights, "audio", &k).ok());
    let mut cache = fp.as_ref().and_then(|f| QCache::load(&cache_path(weights, f), f, device));
    let cached = cache.as_ref().map_or(0, |c| c.len());
    let quant = TowerQuant::from_plan(plan, kernel, cache.as_mut());
    let tower = AudioTower::load_with(&vb, device, quant)?;
    if let Some(f) = &fp {
        let t = tower.quantized_tensors();
        if cached == 0 || cached != t.len() {
            let _ = QCache::save(&cache_path(weights, f), f, &t);
        }
    }
    Ok(tower)
}

/// 장문 문서(`> 512`토큰) 천공용 문단 고정 + `±1` 인접 확장.
/// 머리말(0번 문단) + 부정/예외/단서/조건/금액/식별자 문단을 필수로 고정하고,
/// 선택 비율이 `max_ratio`(기본 0.65)를 초과하면 원본 전문을 그대로 반환(FallbackFull)한다.
pub fn select_perforated_document_text(text: &str, max_ratio: f32) -> String {
    let paras: Vec<&str> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if paras.len() <= 3 {
        return text.to_string();
    }
    let mut pinned = vec![false; paras.len()];
    pinned[0] = true; // Header / title paragraph always pinned
    for (idx, p) in paras.iter().enumerate() {
        let low = p.to_ascii_lowercase();
        let has_negation_or_condition = [
            "단서", "예외", "제외", "불가", "금지", "아닌", "아니", "미만", "초과", "이하", "이상",
            "경우", "조건", "한하여", "unless", "except", "not ", "never", "prohibited", "provided that",
            "only if", "subject to", "excluded", "void",
        ]
        .iter()
        .any(|k| low.contains(k));
        let has_numeric_or_id = p.chars().filter(|c| c.is_ascii_digit()).count() >= 4
            || p.contains("원")
            || p.contains("KRW")
            || p.contains("USD");
        if has_negation_or_condition || has_numeric_or_id {
            pinned[idx] = true;
            if idx > 0 {
                pinned[idx - 1] = true;
            }
            if idx + 1 < paras.len() {
                pinned[idx + 1] = true;
            }
        }
    }
    let kept = pinned.iter().filter(|&&b| b).count();
    if (kept as f32 / paras.len() as f32) > max_ratio {
        return text.to_string();
    }
    paras
        .iter()
        .enumerate()
        .filter(|(i, _)| pinned[*i])
        .map(|(_, p)| *p)
        .collect::<Vec<_>>()
        .join("\n\n")
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
            GemmaPart::Audio(pcm) => accumulate_audio_signature(pcm, seed, &mut out),
        }
    }
    normalize_l2(&mut out);
    out
}

fn accumulate_audio_signature(pcm: &[f32], seed: u64, dst: &mut [f32]) {
    let step = (pcm.len() / 64).max(1);
    for (idx, chunk) in pcm.chunks(step).take(64).enumerate() {
        let rms = (chunk.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>()
            / chunk.len().max(1) as f64)
            .sqrt();
        let bucket = (rms * 16.0).round().clamp(0.0, 16.0) as u32;
        let key = format!("aud:{idx}:{bucket}");
        accumulate_char_ngrams(&key, seed, dst);
    }
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
        assert_eq!((BOA_ID, EOA_ID), (256_000, 258_883));

        // 3-modality deterministic embedder handles Audio cleanly in both CanonicalFull and LocalPerforated
        let det = GemmaEmbedder::deterministic_for_tests(42);
        let pcm: Vec<f32> = (0..16_000).map(|i| 0.2 * ((i as f32) * 0.05).sin()).collect();
        let e_aud = det.embed_audio(&pcm).unwrap();
        assert_eq!(e_aud.len(), OUTPUT_DIM);
        let e_multi = det
            .embed_parts_with_mode(&[GemmaPart::Text("sound check"), GemmaPart::Audio(&pcm)], EmbedMode::LocalPerforated)
            .unwrap();
        assert_eq!(e_multi.len(), OUTPUT_DIM);
    }

    #[test]
    fn streaming_sliding_window_matches_dense_masked_attention_above_258_tokens() {
        let t = 300usize; // > BIDIRECTIONAL_SLIDING_WINDOW + 1 (258)
        let heads = 2usize;
        let kv_heads = 1usize;
        let hd = 8usize;
        let win = 257usize;
        let q: Vec<f32> = (0..t * heads * hd).map(|i| ((i as f32) * 0.13).sin() * 0.4).collect();
        let k: Vec<f32> = (0..t * kv_heads * hd).map(|i| ((i as f32) * 0.17).cos() * 0.4).collect();
        let v: Vec<f32> = (0..t * kv_heads * hd).map(|i| ((i as f32) * 0.23).sin() * 0.5).collect();

        let mut streamed = vec![0.0f32; t * heads * hd];
        stream_sliding_window_attention_f32(&q, &k, &v, t, heads, kv_heads, hd, win, &mut streamed);

        // Reference via dense masked Candle matmul
        let dev = Device::Cpu;
        let qt = Tensor::from_vec(q, (1, t, heads, hd), &dev).unwrap().transpose(1, 2).unwrap().contiguous().unwrap();
        let kt = repeat_kv(Tensor::from_vec(k, (1, t, kv_heads, hd), &dev).unwrap().transpose(1, 2).unwrap().contiguous().unwrap(), heads / kv_heads).unwrap();
        let vt = repeat_kv(Tensor::from_vec(v, (1, t, kv_heads, hd), &dev).unwrap().transpose(1, 2).unwrap().contiguous().unwrap(), heads / kv_heads).unwrap();
        let mask = attention_mask(t, true, true, &dev).unwrap();
        let scores = qt.matmul(&kt.transpose(2, 3).unwrap().contiguous().unwrap()).unwrap().broadcast_add(&mask).unwrap();
        let ref_out = ops::softmax_last_dim(&scores).unwrap().matmul(&vt).unwrap().transpose(1, 2).unwrap().contiguous().unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();

        let max_err = streamed
            .iter()
            .zip(ref_out.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(max_err < 1e-5, "sliding window streaming attention drifted: max_err={max_err}");
    }

    #[test]
    fn long_doc_paragraph_pinning_preserves_negations_neighbors_and_respects_budget() {
        let mut paras = vec!["제목: 계약 총칙".to_string()];
        for i in 1..=10 {
            if i == 5 {
                paras.push("제5조 단서 예외: 본 항목은 지급 대상에서 제외되며 불가함".to_string());
            } else {
                paras.push(format!("일반 안내 문단 {i}호 설명 텍스트입니다."));
            }
        }
        let doc = paras.join("\n\n");
        let selected = select_perforated_document_text(&doc, 0.65);
        assert!(selected.contains("제목: 계약 총칙"), "header must be pinned");
        assert!(selected.contains("제5조 단서 예외"), "negation/exception paragraph must be pinned");
        assert!(selected.contains("일반 안내 문단 4호"), "-1 neighbor must be expanded");
        assert!(selected.contains("일반 안내 문단 6호"), "+1 neighbor must be expanded");
        assert!(!selected.contains("일반 안내 문단 9호"), "unpinned distant paragraph should be omitted");
    }
}
