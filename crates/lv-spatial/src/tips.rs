//! TIPSv2 ViT-B/14 DPT monocular depth + normals + ADE20K segmentation fallback
//! using ONLY `candle-core` + `candle-nn` (ZERO `candle-transformers`), reusing
//! `lv_core::siglip2::antialias_bilinear_weights` & `lv_kernels::stream_attention_f32`,
//! guarded by `MonocularDepthGuard` (`0 B` loaded when stereo `cameras >= 2`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use anyhow::{bail, Context, Result};
use candle_core::{DType, Device, Module, Tensor, D};
use candle_nn::{conv2d, conv2d_no_bias, ops::softmax_last_dim, Conv2d, Conv2dConfig, LayerNorm, Linear, VarBuilder};
use lv_core::siglip2::antialias_bilinear_weights;
use lv_kernels::{stream_attention_f32, CpuStreamAttnCfg};
use rayon::prelude::*;

use crate::depth::{DepthModel, DepthScene, SceneHeads};
use crate::frame::{DepthKind, DepthMap, NormalMap, Rgb, SegMap};
use crate::pyramid::{pyramid_compat_grid, PyramidCfg};

pub const MODE: &str = "tipsv2-b14-dpt";
pub const DEFAULT_DIR: &str = "models/tipsv2-b14-dpt";
pub const DIR_ENV: &str = "TIPS_DIR";
pub const HEADS_ENV: &str = "LV_TIPS_HEADS";
pub const WEIGHTS_FILE: &str = "model.safetensors";
pub const CONFIG_FILE: &str = "config.json";
pub const DEFAULT_SHORT_SIDE_PATCHES: usize = 24;

pub const ADE_CLASSES: usize = 150;
pub const ADE_SKY: usize = 2;
pub const ADE_TREE: usize = 4;
pub const ADE_ROCK: usize = 34;
pub const ADE_BOX: usize = 41;
pub const ADE_GROUND: [usize; 11] = [3, 6, 9, 11, 13, 29, 46, 52, 54, 91, 94];
pub const ADE_VEGETATION: [usize; 4] = [4, 17, 66, 72];
pub const ADE_FAR: [usize; 4] = [2, 16, 68, 48];
pub const NORMAL_RIGHT: usize = 0;

pub const ADE_NAMES: [&str; ADE_CLASSES] = [
    "wall", "building", "sky", "floor", "tree", "ceiling", "road", "bed", "windowpane", "grass",
    "cabinet", "sidewalk", "person", "earth", "door", "table", "mountain", "plant", "curtain", "chair",
    "car", "water", "painting", "sofa", "shelf", "house", "sea", "mirror", "rug", "field",
    "armchair", "seat", "fence", "desk", "rock", "wardrobe", "lamp", "bathtub", "railing", "cushion",
    "base", "box", "column", "signboard", "chest", "counter", "sand", "sink", "skyscraper", "fireplace",
    "refrigerator", "grandstand", "path", "stairs", "runway", "case", "pool", "pillow", "screen", "stairway",
    "river", "bridge", "bookcase", "blind", "coffee", "toilet", "flower", "book", "hill", "bench",
    "countertop", "stove", "palm", "kitchen", "computer", "swivel", "boat", "bar", "arcade", "hovel",
    "bus", "towel", "light", "truck", "tower", "chandelier", "awning", "streetlight", "booth", "television",
    "airplane", "dirt", "apparel", "pole", "land", "bannister", "escalator", "ottoman", "bottle", "buffet",
    "poster", "stage", "van", "ship", "fountain", "conveyer", "canopy", "washer", "plaything", "swimming",
    "stool", "barrel", "basket", "waterfall", "tent", "bag", "minibike", "cradle", "oven", "ball",
    "food", "step", "tank", "trade", "microwave", "pot", "animal", "bicycle", "lake", "dishwasher",
    "screen_door", "blanket", "sculpture", "hood", "sconce", "vase", "traffic", "tray", "ashcan", "fan",
    "pier", "crt", "plate", "monitor", "bulletin", "shower", "radiator", "glass", "clock", "flag",
];

pub fn ade_name(i: usize) -> &'static str {
    ADE_NAMES.get(i).copied().unwrap_or("?")
}

pub fn is_ade_ground(i: usize) -> bool {
    ADE_GROUND.contains(&i)
}

pub fn ade_colour(i: usize) -> [u8; 3] {
    match i {
        ADE_SKY => [120, 170, 230],
        ADE_TREE | 17 | 66 | 72 => [40, 140, 50],
        ADE_ROCK => [130, 130, 136],
        ADE_BOX => [230, 150, 60],
        9 | 29 => [110, 170, 70],
        13 | 46 | 52 | 91 | 94 => [150, 110, 70],
        3 | 6 | 11 | 54 => [170, 160, 150],
        16 | 68 => [90, 100, 120],
        21 | 26 | 60 | 128 => [40, 90, 170],
        _ => [((i * 53 + 31) % 200 + 30) as u8, ((i * 97 + 17) % 200 + 30) as u8, ((i * 151 + 73) % 200 + 30) as u8],
    }
}

pub fn heads_from_env() -> SceneHeads {
    match std::env::var(HEADS_ENV).ok().as_deref() {
        Some("all") => SceneHeads::ALL,
        Some("normals") => SceneHeads { normals: true, segmentation: false },
        Some("seg" | "segmentation") => SceneHeads { normals: false, segmentation: true },
        _ => SceneHeads::NONE,
    }
}

/// Enforces the strict Stereo-First Monocular-Only Fallback policy:
/// when `camera_count >= 2`, monocular TIPSv2 loads `0` bytes (`None`) so Census-SGBM handles all depth.
pub fn maybe_load_monocular<F>(camera_count: usize, loader: F) -> Result<Option<TipsDepth>>
where
    F: FnOnce() -> Result<TipsDepth>,
{
    if camera_count >= 2 {
        Ok(None)
    } else {
        Ok(Some(loader()?))
    }
}

const DPT_LEVELS: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct TipsConfig {
    pub hidden: usize,
    pub layers: usize,
    pub heads: usize,
    pub mlp_hidden: usize,
    pub patch_size: usize,
    pub image_size: usize,
    pub register_tokens: usize,
    pub layer_norm_eps: f64,
    pub tap_blocks: [usize; DPT_LEVELS],
    pub post_process_channels: [usize; DPT_LEVELS],
    pub dpt_channels: usize,
    pub min_depth: f32,
    pub max_depth: f32,
    pub depth_bins: usize,
    pub seg_classes: usize,
}

impl Default for TipsConfig {
    fn default() -> Self {
        Self {
            hidden: 768,
            layers: 12,
            heads: 12,
            mlp_hidden: 3072,
            patch_size: 14,
            image_size: 448,
            register_tokens: 1,
            layer_norm_eps: 1e-6,
            tap_blocks: [2, 5, 8, 11],
            post_process_channels: [96, 192, 384, 768],
            dpt_channels: 256,
            min_depth: 0.001,
            max_depth: 10.0,
            depth_bins: 256,
            seg_classes: ADE_CLASSES,
        }
    }
}

impl TipsConfig {
    pub fn from_file(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_json(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn from_json(raw: &str) -> Result<Self> {
        let v: serde_json::Value = serde_json::from_str(raw)?;
        if let Some(f) = v["vision_fn"].as_str() {
            if f != "vit_base" {
                bail!("only ViT-B/14 vision_fn='vit_base' is supported, got {f:?}");
            }
        }
        if let Some(f) = v["ffn_layer"].as_str() {
            if f != "mlp" {
                bail!("only GELU MLP ffn_layer='mlp' is supported, got {f:?}");
            }
        }
        let mut cfg = Self::default();
        if let Some(n) = v["hidden_size"].as_u64() { cfg.hidden = n as usize; }
        if let Some(n) = v["num_hidden_layers"].as_u64() { cfg.layers = n as usize; }
        if let Some(n) = v["num_attention_heads"].as_u64() { cfg.heads = n as usize; }
        if let Some(n) = v["intermediate_size"].as_u64() { cfg.mlp_hidden = n as usize; }
        if let Some(n) = v["patch_size"].as_u64() { cfg.patch_size = n as usize; }
        if let Some(n) = v["image_size"].as_u64() { cfg.image_size = n as usize; }
        if let Some(n) = v["min_depth"].as_f64() { cfg.min_depth = n as f32; }
        if let Some(n) = v["max_depth"].as_f64() { cfg.max_depth = n as f32; }
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn position_side(&self) -> usize {
        self.image_size / self.patch_size
    }

    fn validate(&self) -> Result<()> {
        if self.heads == 0 || self.hidden % self.heads != 0 {
            bail!("hidden {} not divisible by heads {}", self.hidden, self.heads);
        }
        if self.patch_size == 0 || self.image_size % self.patch_size != 0 {
            bail!("image_size {} not divisible by patch {}", self.image_size, self.patch_size);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputGrid {
    ShortSide(usize),
    Fixed { cols: usize, rows: usize },
    Pyramid { l1_short_side: usize, l1_budget: usize, ratio: usize },
}

impl Default for InputGrid {
    fn default() -> Self {
        Self::ShortSide(DEFAULT_SHORT_SIDE_PATCHES)
    }
}

impl InputGrid {
    pub fn pyramid(cfg: &PyramidCfg) -> Self {
        Self::Pyramid {
            l1_short_side: cfg.l1_short,
            l1_budget: cfg.l1_budget,
            ratio: crate::pyramid::L2_RATIO,
        }
    }

    pub fn grid_for(self, width: u32, height: u32) -> (usize, usize) {
        match self {
            InputGrid::Fixed { cols, rows } => (cols.max(1), rows.max(1)),
            InputGrid::Pyramid { l1_short_side, l1_budget, ratio } => {
                let p = pyramid_compat_grid(width, height, l1_short_side, l1_budget);
                (p.0 * ratio.max(1), p.1 * ratio.max(1))
            }
            InputGrid::ShortSide(n) => {
                let n = n.max(1) as f64;
                let (w, h) = (width.max(1) as f64, height.max(1) as f64);
                let long = |a: f64, b: f64| ((n * a / b).round() as usize).max(1);
                if width <= height { (n as usize, long(h, w)) } else { (long(w, h), n as usize) }
            }
        }
    }
}

struct Attention {
    qkv: Linear,
    proj: Linear,
    heads: usize,
    head_dim: usize,
}

impl Attention {
    fn new(cfg: &TipsConfig, vb: VarBuilder) -> candle_core::Result<Self> {
        let d = cfg.hidden;
        let qkv = candle_nn::linear(d, 3 * d, vb.pp("qkv"))?;
        let proj = candle_nn::linear(d, d, vb.pp("proj"))?;
        Ok(Self { qkv, proj, heads: cfg.heads, head_dim: d / cfg.heads })
    }

    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let (b, n, d) = x.dims3()?;
        let scale = (self.head_dim as f64).powf(-0.5);
        let qkv = self.qkv.forward(x)?.reshape((b, n, 3, d))?;
        let q = qkv.narrow(2, 0, 1)?.squeeze(2)?.contiguous()?;
        let k = qkv.narrow(2, 1, 1)?.squeeze(2)?.contiguous()?;
        let v = qkv.narrow(2, 2, 1)?.squeeze(2)?.contiguous()?;

        if x.device().is_cpu() && x.dtype() == DType::F32 {
            let (qs, ql) = q.storage_and_layout();
            let (ks, kl) = k.storage_and_layout();
            let (vs, vl) = v.storage_and_layout();
            if let (
                candle_core::Storage::Cpu(candle_core::CpuStorage::F32(qv)),
                candle_core::Storage::Cpu(candle_core::CpuStorage::F32(kv)),
                candle_core::Storage::Cpu(candle_core::CpuStorage::F32(vv)),
            ) = (&*qs, &*ks, &*vs)
            {
                let cfg = CpuStreamAttnCfg {
                    num_heads: self.heads,
                    num_kv_heads: self.heads,
                    head_dim: self.head_dim,
                    scale: scale as f32,
                    causal: false,
                    past_offset: 0,
                };
                let mut ctx = vec![0.0f32; b * n * d];
                let (q0, k0, v0) = (ql.start_offset(), kl.start_offset(), vl.start_offset());
                for bi in 0..b {
                    stream_attention_f32(
                        &qv[q0 + bi * n * d..q0 + (bi + 1) * n * d],
                        &kv[k0 + bi * n * d..k0 + (bi + 1) * n * d],
                        &vv[v0 + bi * n * d..v0 + (bi + 1) * n * d],
                        n,
                        n,
                        &cfg,
                        &mut ctx[bi * n * d..(bi + 1) * n * d],
                    );
                }
                let ctx_t = Tensor::from_vec(ctx, (b, n, d), x.device())?;
                return self.proj.forward(&ctx_t);
            }
        }

        let split = |t: Tensor| -> candle_core::Result<Tensor> {
            t.reshape((b, n, self.heads, self.head_dim))?.transpose(1, 2)?.contiguous()
        };
        let (qh, kh, vh) = (split(q)?, split(k)?, split(v)?);
        let scores = qh.matmul(&kh.transpose(2, 3)?)?.affine(scale, 0.0)?;
        let probs = softmax_last_dim(&scores)?;
        let ctx = probs.matmul(&vh)?.transpose(1, 2)?.contiguous()?.reshape((b, n, d))?;
        self.proj.forward(&ctx)
    }
}

struct Block {
    norm1: LayerNorm,
    attn: Attention,
    ls1: Tensor,
    norm2: LayerNorm,
    fc1: Linear,
    fc2: Linear,
    ls2: Tensor,
}

impl Block {
    fn new(cfg: &TipsConfig, vb: VarBuilder) -> candle_core::Result<Self> {
        let d = cfg.hidden;
        Ok(Self {
            norm1: candle_nn::layer_norm(d, cfg.layer_norm_eps, vb.pp("norm1"))?,
            attn: Attention::new(cfg, vb.pp("attn"))?,
            ls1: vb.get(d, "ls1.gamma")?,
            norm2: candle_nn::layer_norm(d, cfg.layer_norm_eps, vb.pp("norm2"))?,
            fc1: candle_nn::linear(d, cfg.mlp_hidden, vb.pp("mlp.fc1"))?,
            fc2: candle_nn::linear(cfg.mlp_hidden, d, vb.pp("mlp.fc2"))?,
            ls2: vb.get(d, "ls2.gamma")?,
        })
    }

    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let a = self.attn.forward(&self.norm1.forward(x)?)?.broadcast_mul(&self.ls1)?;
        let x = (x + a)?;
        let h = self.fc1.forward(&self.norm2.forward(&x)?)?.gelu()?;
        let m = self.fc2.forward(&h)?.broadcast_mul(&self.ls2)?;
        x + m
    }
}

struct Encoder {
    patch_proj: Linear,
    cls_token: Tensor,
    register_tokens: Option<Tensor>,
    pos_embed: Tensor,
    pos_cache: Mutex<HashMap<(usize, usize), Tensor>>,
    blocks: Vec<Block>,
    norm: LayerNorm,
    side: usize,
    hidden: usize,
    patch: usize,
}

struct Tap {
    cls: Tensor,
    patches: Tensor,
    rows: usize,
    cols: usize,
}

impl Encoder {
    fn new(cfg: &TipsConfig, vb: VarBuilder) -> candle_core::Result<Self> {
        let d = cfg.hidden;
        let p = cfg.patch_size;
        let pw = vb.get((d, 3, p, p), "patch_embed.proj.weight")?.reshape((d, 3 * p * p))?;
        let pb = vb.get(d, "patch_embed.proj.bias")?;
        let side = cfg.position_side();
        let blocks = (0..cfg.layers)
            .map(|i| Block::new(cfg, vb.pp(format!("blocks.{i}"))))
            .collect::<candle_core::Result<Vec<_>>>()?;
        Ok(Self {
            patch_proj: Linear::new(pw, Some(pb)),
            cls_token: vb.get((1, 1, d), "cls_token")?,
            register_tokens: if cfg.register_tokens > 0 {
                Some(vb.get((1, cfg.register_tokens, d), "register_tokens")?)
            } else {
                None
            },
            pos_embed: vb.get((1, 1 + side * side, d), "pos_embed")?,
            pos_cache: Mutex::new(HashMap::new()),
            blocks,
            norm: candle_nn::layer_norm(d, cfg.layer_norm_eps, vb.pp("norm"))?,
            side,
            hidden: d,
            patch: p,
        })
    }

    fn positions(&self, rows: usize, cols: usize, device: &Device) -> candle_core::Result<Tensor> {
        if let Some(cached) = self.pos_cache.lock().unwrap_or_else(|p| p.into_inner()).get(&(rows, cols)) {
            return Ok(cached.clone());
        }
        let s = self.side;
        let d = self.hidden;
        let cls_pos = self.pos_embed.narrow(1, 0, 1)?;
        let grid_pos = self.pos_embed.narrow(1, 1, s * s)?.reshape((s, s, d))?;
        let resampled = if (rows, cols) == (s, s) {
            grid_pos.reshape((1, s * s, d))?
        } else {
            let wy = Tensor::from_vec(antialias_bilinear_weights(s, rows), (rows, s), device)?;
            let wx = Tensor::from_vec(antialias_bilinear_weights(s, cols), (cols, s), device)?;
            let by_rows = wy.matmul(&grid_pos.reshape((s, s * d))?)?.reshape((rows, s, d))?;
            wx.broadcast_left(rows)?.contiguous()?.matmul(&by_rows)?.reshape((1, rows * cols, d))?
        };
        let out = Tensor::cat(&[&cls_pos, &resampled], 1)?;
        self.pos_cache.lock().unwrap_or_else(|p| p.into_inner()).insert((rows, cols), out.clone());
        Ok(out)
    }

    fn forward_taps(&self, chw: &Tensor, rows: usize, cols: usize, tap_blocks: &[usize; DPT_LEVELS]) -> candle_core::Result<Vec<Tap>> {
        let (b, _, h, w) = chw.dims4()?;
        let p = self.patch;
        // Extract non-overlapping p x p patches directly into (b, rows * cols, 3 * p * p) for GEMM patch embedding.
        let flat = chw
            .reshape((b, 3, rows, p, cols, p))?
            .permute((0, 2, 4, 1, 3, 5))?
            .contiguous()?
            .reshape((b, rows * cols, 3 * p * p))?;
        let _ = (h, w);
        let pat_emb = self.patch_proj.forward(&flat)?;
        let cls = self.cls_token.broadcast_as((b, 1, self.hidden))?;
        let mut x = Tensor::cat(&[&cls, &pat_emb], 1)?;
        let pos = self.positions(rows, cols, x.device())?;
        x = x.broadcast_add(&pos)?;

        let reg_n = if let Some(reg) = &self.register_tokens {
            let r = reg.broadcast_as((b, reg.dim(1)?, self.hidden))?;
            let x_cls = x.narrow(1, 0, 1)?;
            let x_pat = x.narrow(1, 1, rows * cols)?;
            x = Tensor::cat(&[&x_cls, &r, &x_pat], 1)?;
            reg.dim(1)?
        } else {
            0
        };

        let mut taps = Vec::with_capacity(DPT_LEVELS);
        for (idx, blk) in self.blocks.iter().enumerate() {
            x = blk.forward(&x)?;
            if tap_blocks.contains(&idx) {
                let xn = self.norm.forward(&x)?;
                taps.push(Tap {
                    cls: xn.narrow(1, 0, 1)?,
                    patches: xn.narrow(1, 1 + reg_n, rows * cols)?,
                    rows,
                    cols,
                });
            }
        }
        Ok(taps)
    }
}

struct PixelShuffleUp {
    proj: Linear,
    bias: Tensor,
    ch: usize,
    scale: usize,
}

impl PixelShuffleUp {
    fn new(ch: usize, scale: usize, vb: VarBuilder) -> candle_core::Result<Self> {
        let w = vb.get((ch, ch, scale, scale), "weight")?;
        let bias = vb.get(ch, "bias")?;
        // Transpose [c_in, c_out, s, s] -> [c_out * s * s, c_in] for non-overlapping PixelShuffle up-projection.
        let w_mat = w.permute((1, 2, 3, 0))?.contiguous()?.reshape((ch * scale * scale, ch))?;
        Ok(Self { proj: Linear::new(w_mat, None), bias, ch, scale })
    }

    fn forward_nchw(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let (b, c, h, w) = x.dims4()?;
        let s = self.scale;
        let nhwc = x.permute((0, 2, 3, 1))?.contiguous()?.reshape((b * h * w, c))?;
        let y = self
            .proj
            .forward(&nhwc)?
            .reshape((b, h, w, self.ch, s, s))?
            .permute((0, 3, 1, 4, 2, 5))?
            .contiguous()?
            .reshape((b, self.ch, h * s, w * s))?;
        y.broadcast_add(&self.bias.reshape((1, self.ch, 1, 1))?)
    }
}

enum Resize {
    Up(PixelShuffleUp),
    Keep,
    Down(Conv2d),
}

struct ReassembleLevel {
    readout: Linear,
    out_proj: Linear,
    out_channels: usize,
    resize: Resize,
}

impl ReassembleLevel {
    fn new(level: usize, cfg: &TipsConfig, vb: VarBuilder) -> candle_core::Result<Self> {
        let d = cfg.hidden;
        let out_c = cfg.post_process_channels[level];
        let readout = candle_nn::linear(2 * d, d, vb.pp(format!("readout_projects.{level}")))?;
        let op_w = vb
            .get((out_c, d, 1, 1), &format!("out_projections.{level}.weight"))?
            .reshape((out_c, d))?;
        let op_b = vb.get(out_c, &format!("out_projections.{level}.bias"))?;
        let out_proj = Linear::new(op_w, Some(op_b));
        let resize = match level {
            0 => Resize::Up(PixelShuffleUp::new(out_c, 4, vb.pp("resize_layers.0"))?),
            1 => Resize::Up(PixelShuffleUp::new(out_c, 2, vb.pp("resize_layers.1"))?),
            2 => Resize::Keep,
            _ => Resize::Down(conv2d(
                out_c,
                out_c,
                3,
                Conv2dConfig { stride: 2, padding: 1, ..Default::default() },
                vb.pp("resize_layers.3"),
            )?),
        };
        Ok(Self { readout, out_proj, out_channels: out_c, resize })
    }

    fn forward(&self, tap: &Tap) -> candle_core::Result<Tensor> {
        let (b, n, _) = tap.patches.dims3()?;
        let cls = tap.cls.broadcast_as((b, n, tap.cls.dim(2)?))?;
        let cat = Tensor::cat(&[&tap.patches, &cls], 2)?;
        let r = self.readout.forward(&cat)?.gelu()?;
        let p = self.out_proj.forward(&r)?;
        let nchw = p.transpose(1, 2)?.contiguous()?.reshape((b, self.out_channels, tap.rows, tap.cols))?;
        match &self.resize {
            Resize::Up(u) => u.forward_nchw(&nchw),
            Resize::Keep => Ok(nchw),
            Resize::Down(c) => c.forward(&nchw),
        }
    }
}

struct ResidualUnit {
    conv1: Conv2d,
    conv2: Conv2d,
}

impl ResidualUnit {
    fn new(ch: usize, vb: VarBuilder) -> candle_core::Result<Self> {
        let cfg = Conv2dConfig { padding: 1, ..Default::default() };
        Ok(Self {
            conv1: conv2d_no_bias(ch, ch, 3, cfg, vb.pp("conv1"))?,
            conv2: conv2d_no_bias(ch, ch, 3, cfg, vb.pp("conv2"))?,
        })
    }

    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let h = self.conv1.forward(&x.relu()?)?;
        let h = self.conv2.forward(&h.relu()?)?;
        x + h
    }
}

struct FusionBlock {
    residual_unit: Option<ResidualUnit>,
    main_unit: ResidualUnit,
    out_w: Tensor,
    out_b: Tensor,
}

impl FusionBlock {
    fn new(ch: usize, has_skip: bool, vb: VarBuilder) -> candle_core::Result<Self> {
        Ok(Self {
            residual_unit: if has_skip {
                Some(ResidualUnit::new(ch, vb.pp("residual_unit"))?)
            } else {
                None
            },
            main_unit: ResidualUnit::new(ch, vb.pp("main_unit"))?,
            out_w: vb.get((ch, ch, 1, 1), "out_conv.weight")?.reshape((ch, ch))?,
            out_b: vb.get(ch, "out_conv.bias")?,
        })
    }

    fn forward(&self, x: &Tensor, skip: Option<&Tensor>, target_hw: (usize, usize)) -> candle_core::Result<Tensor> {
        let mut h = x.clone();
        if let (Some(ru), Some(s)) = (&self.residual_unit, skip) {
            h = (h + ru.forward(s)?)?;
        }
        h = self.main_unit.forward(&h)?;
        h = h.upsample_bilinear2d(target_hw.0, target_hw.1, false)?;
        let (b, ch, hh, ww) = h.dims4()?;
        let flat = h.reshape((b, ch, hh * ww))?;
        let proj = self.out_w.broadcast_left(b)?.matmul(&flat)?;
        proj.reshape((b, ch, hh, ww))?
            .broadcast_add(&self.out_b.reshape((1, ch, 1, 1))?)
    }
}

struct Decoder {
    reassemble: Vec<ReassembleLevel>,
    convs: Vec<Conv2d>,
    fusion: Vec<FusionBlock>,
    project: Conv2d,
}

impl Decoder {
    fn new(cfg: &TipsConfig, vb: VarBuilder) -> candle_core::Result<Self> {
        let ch = cfg.dpt_channels;
        let reassemble = (0..DPT_LEVELS)
            .map(|i| ReassembleLevel::new(i, cfg, vb.pp("reassemble")))
            .collect::<candle_core::Result<Vec<_>>>()?;
        let convs = (0..DPT_LEVELS)
            .map(|i| {
                conv2d_no_bias(
                    cfg.post_process_channels[i],
                    ch,
                    3,
                    Conv2dConfig { padding: 1, ..Default::default() },
                    vb.pp(format!("convs.{i}")),
                )
            })
            .collect::<candle_core::Result<Vec<_>>>()?;
        let fusion = (0..DPT_LEVELS)
            .map(|i| FusionBlock::new(ch, i > 0, vb.pp(format!("fusion_blocks.{i}"))))
            .collect::<candle_core::Result<Vec<_>>>()?;
        let project = conv2d(
            ch,
            ch,
            3,
            Conv2dConfig { padding: 1, ..Default::default() },
            vb.pp("project"),
        )?;
        Ok(Self { reassemble, convs, fusion, project })
    }

    fn forward(&self, taps: &[Tap]) -> candle_core::Result<Tensor> {
        let mut feats = Vec::with_capacity(DPT_LEVELS);
        for (i, t) in taps.iter().enumerate() {
            let r = self.reassemble[i].forward(t)?;
            feats.push(self.convs[i].forward(&r)?);
        }
        // Reverse coarse-to-fine DPT refinement path (`3 -> 2 -> 1 -> 0` -> final 2x).
        let (h2, w2) = (feats[2].dim(2)?, feats[2].dim(3)?);
        let mut path = self.fusion[0].forward(&feats[3], None, (h2, w2))?;
        let (h1, w1) = (feats[1].dim(2)?, feats[1].dim(3)?);
        path = self.fusion[1].forward(&path, Some(&feats[2]), (h1, w1))?;
        let (h0, w0) = (feats[0].dim(2)?, feats[0].dim(3)?);
        path = self.fusion[2].forward(&path, Some(&feats[1]), (h0, w0))?;
        path = self.fusion[3].forward(&path, Some(&feats[0]), (h0 * 2, w0 * 2))?;
        self.project.forward(&path)?.relu()
    }
}

pub fn bin_centers(min: f32, max: f32, bins: usize, device: &Device) -> candle_core::Result<Tensor> {
    let step = (max as f64 - min as f64) / (bins.max(2) - 1) as f64;
    let centers: Vec<f32> = (0..bins).map(|i| (min as f64 + step * i as f64) as f32).collect();
    Tensor::from_vec(centers, bins, device)
}

pub fn decode_bins(logits: &Tensor, centers_col: &Tensor, min_depth: f64) -> candle_core::Result<Tensor> {
    let p = (logits.relu()? + min_depth)?;
    let p = p.broadcast_div(&p.sum_keepdim(D::Minus1)?)?;
    let col = if centers_col.rank() == 1 {
        centers_col.reshape((centers_col.dim(0)?, 1))?
    } else {
        centers_col.clone()
    };
    p.broadcast_matmul(&col)?.squeeze(D::Minus1)
}

struct DepthHead {
    dec: Decoder,
    head: Linear,
    centers_col: Tensor,
    min_depth: f64,
}

impl DepthHead {
    fn new(cfg: &TipsConfig, vb: VarBuilder, device: &Device) -> candle_core::Result<Self> {
        let dh = vb.pp("depth_head");
        let ch = cfg.dpt_channels;
        Ok(Self {
            dec: Decoder::new(cfg, dh.clone())?,
            head: candle_nn::linear(ch, cfg.depth_bins, dh.pp("depth_head"))?,
            centers_col: bin_centers(cfg.min_depth, cfg.max_depth, cfg.depth_bins, device)?,
            min_depth: cfg.min_depth as f64,
        })
    }

    fn forward(&self, taps: &[Tap], target_hw: (usize, usize)) -> candle_core::Result<Tensor> {
        let feat = self.dec.forward(taps)?.upsample_bilinear2d(target_hw.0, target_hw.1, false)?;
        let (b, c, h, w) = feat.dims4()?;
        let nhwc = feat.permute((0, 2, 3, 1))?.contiguous()?;
        let logits = self.head.forward(&nhwc.reshape((b * h * w, c))?)?;
        decode_bins(&logits, &self.centers_col, self.min_depth)?.reshape((b, h, w))
    }
}

struct NormalsHead {
    dec: Decoder,
    out_w: Tensor,
    out_b: Tensor,
}

impl NormalsHead {
    fn new(cfg: &TipsConfig, vb: VarBuilder) -> candle_core::Result<Self> {
        let nh = vb.pp("normals_head");
        Ok(Self {
            dec: Decoder::new(cfg, nh.clone())?,
            out_w: nh.get((3, cfg.dpt_channels), "normals_head.weight")?,
            out_b: nh.get(3, "normals_head.bias")?,
        })
    }

    fn forward(&self, taps: &[Tap]) -> candle_core::Result<Tensor> {
        let feat = self.dec.forward(taps)?;
        let (b, c, h, w) = feat.dims4()?;
        let flat = feat.reshape((b, c, h * w))?;
        let raw = self.out_w.broadcast_left(b)?.matmul(&flat)?.reshape((b, 3, h, w))?
            .broadcast_add(&self.out_b.reshape((1, 3, 1, 1))?)?;
        let n = (raw.sqr()?.sum_keepdim(1)?.sqrt()? + 1e-12)?;
        raw.broadcast_div(&n)
    }
}

struct SegHead {
    dec: Decoder,
    out_w: Tensor,
    out_b: Tensor,
    classes: usize,
}

impl SegHead {
    fn new(cfg: &TipsConfig, vb: VarBuilder) -> candle_core::Result<Self> {
        let sh = vb.pp("segmentation_head");
        Ok(Self {
            dec: Decoder::new(cfg, sh.clone())?,
            out_w: sh.get((cfg.seg_classes, cfg.dpt_channels), "segmentation_head.weight")?,
            out_b: sh.get(cfg.seg_classes, "segmentation_head.bias")?,
            classes: cfg.seg_classes,
        })
    }

    fn forward(&self, taps: &[Tap]) -> candle_core::Result<Tensor> {
        let feat = self.dec.forward(taps)?;
        let (b, c, h, w) = feat.dims4()?;
        let flat = feat.reshape((b, c, h * w))?;
        self.out_w
            .broadcast_left(b)?
            .matmul(&flat)?
            .reshape((b, self.classes, h, w))?
            .broadcast_add(&self.out_b.reshape((1, self.classes, 1, 1))?)
    }
}

pub struct TipsDepth {
    cfg: TipsConfig,
    enc: Encoder,
    depth_head: DepthHead,
    normals_head: Option<NormalsHead>,
    seg_head: Option<SegHead>,
    pub input_grid: InputGrid,
    pub flip_tta: bool,
    device: Device,
    heads: SceneHeads,
    mode: String,
}

fn u8_to_unit_lut() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut t = [0.0f32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            *slot = i as f32 / 255.0;
        }
        t
    })
}

fn rgb_to_chw_f32_into(rgb: &Rgb, width: u32, height: u32, dst: &mut [f32]) -> Result<()> {
    let resized;
    let data: &[u8] = if (rgb.width, rgb.height) == (width, height) {
        &rgb.data
    } else {
        let src = image::ImageBuffer::<image::Rgb<u8>, &[u8]>::from_raw(
            rgb.width,
            rgb.height,
            rgb.data.as_slice(),
        )
        .context("RGB buffer does not match dimensions")?;
        resized = image::imageops::resize(&src, width, height, image::imageops::FilterType::Triangle);
        resized.as_raw()
    };
    let lut = u8_to_unit_lut();
    let plane = (width * height) as usize;
    let (r_p, rest) = dst.split_at_mut(plane);
    let (g_p, b_p) = rest.split_at_mut(plane);
    for (i, px) in data.chunks_exact(3).enumerate() {
        r_p[i] = lut[px[0] as usize];
        g_p[i] = lut[px[1] as usize];
        b_p[i] = lut[px[2] as usize];
    }
    Ok(())
}

impl TipsDepth {
    pub fn from_var_builder(cfg: TipsConfig, vb: VarBuilder, device: &Device) -> Result<Self> {
        Self::from_var_builder_with(cfg, vb, device, SceneHeads::NONE)
    }

    pub fn from_var_builder_with(
        cfg: TipsConfig,
        vb: VarBuilder,
        device: &Device,
        heads: SceneHeads,
    ) -> Result<Self> {
        cfg.validate()?;
        let enc = Encoder::new(&cfg, vb.pp("vision_encoder"))?;
        let depth_head = DepthHead::new(&cfg, vb.clone(), device)?;
        let normals_head = heads.normals.then(|| NormalsHead::new(&cfg, vb.clone())).transpose()?;
        let seg_head = heads.segmentation.then(|| SegHead::new(&cfg, vb)).transpose()?;
        Ok(Self {
            cfg,
            enc,
            depth_head,
            normals_head,
            seg_head,
            input_grid: InputGrid::default(),
            flip_tta: false,
            device: device.clone(),
            heads,
            mode: MODE.to_string(),
        })
    }

    pub fn config(&self) -> &TipsConfig {
        &self.cfg
    }

    pub fn heads(&self) -> SceneHeads {
        self.heads
    }

    pub fn predict(&self, rgb: &Rgb) -> Result<DepthMap> {
        Ok(self.predict_batch(&[rgb])?.pop().expect("single depth"))
    }

    pub fn predict_batch(&self, frames: &[&Rgb]) -> Result<Vec<DepthMap>> {
        Ok(self.scene_batch(frames, SceneHeads::NONE)?.into_iter().map(|s| s.depth).collect())
    }

    pub fn scene_batch(&self, frames: &[&Rgb], want: SceneHeads) -> Result<Vec<DepthScene>> {
        if frames.is_empty() {
            return Ok(Vec::new());
        }
        let want = want.and(self.heads);
        let f0 = frames[0];
        let (cols, rows) = self.input_grid.grid_for(f0.width, f0.height);
        let in_w = (cols * self.cfg.patch_size) as u32;
        let in_h = (rows * self.cfg.patch_size) as u32;
        let plane = 3 * (in_w * in_h) as usize;
        let mut flat = vec![0.0f32; frames.len() * plane];
        flat.par_chunks_mut(plane)
            .zip(frames.par_iter())
            .try_for_each(|(dst, frame)| rgb_to_chw_f32_into(frame, in_w, in_h, dst))?;
        let chw = Tensor::from_vec(flat, (frames.len(), 3, in_h as usize, in_w as usize), &self.device)?;

        let taps = self.enc.forward_taps(&chw, rows, cols, &self.cfg.tap_blocks)?;
        let d_tensor = self
            .depth_head
            .forward(&taps, (f0.height as usize, f0.width as usize))?;
        let d_vecs = d_tensor.to_vec3::<f32>()?;

        let n_tensor = match (want.normals, &self.normals_head) {
            (true, Some(nh)) => Some(nh.forward(&taps)?),
            _ => None,
        };
        let s_tensor = match (want.segmentation, &self.seg_head) {
            (true, Some(sh)) => Some(sh.forward(&taps)?),
            _ => None,
        };

        let mut out = Vec::with_capacity(frames.len());
        for (bi, frame) in frames.iter().enumerate() {
            let depth = DepthMap {
                width: f0.width,
                height: f0.height,
                kind: DepthKind::Metric,
                data: d_vecs[bi].iter().flatten().copied().collect(),
            };
            let depth = if (frame.width, frame.height) == (f0.width, f0.height) {
                depth
            } else {
                DepthMap {
                    width: frame.width,
                    height: frame.height,
                    kind: DepthKind::Metric,
                    data: vec![depth.data[0]; (frame.width * frame.height) as usize],
                }
            };

            let normals = if let Some(nt) = &n_tensor {
                let b_n = nt.narrow(0, bi, 1)?.squeeze(0)?;
                let (_, nh, nw) = b_n.dims3()?;
                let raw = b_n.permute((1, 2, 0))?.contiguous()?.flatten_all()?.to_vec1::<f32>()?;
                let data = raw.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
                Some(NormalMap { width: nw as u32, height: nh as u32, data })
            } else {
                None
            };

            let segmentation = if let Some(st) = &s_tensor {
                let b_s = st.narrow(0, bi, 1)?.squeeze(0)?;
                let (c_n, sh, sw) = b_s.dims3()?;
                let flat_c = b_s.permute((1, 2, 0))?.contiguous()?.reshape((sh * sw, c_n))?;
                let probs = softmax_last_dim(&flat_c)?;
                let labels_u32 = flat_c.argmax(D::Minus1)?.to_vec1::<u32>()?;
                let sky_col = probs.narrow(1, ADE_SKY, 1)?.squeeze(1)?.to_vec1::<f32>()?;
                Some(SegMap {
                    width: sw as u32,
                    height: sh as u32,
                    labels: labels_u32.into_iter().map(|v| v as u8).collect(),
                    sky: sky_col,
                })
            } else {
                None
            };

            let mut scene = DepthScene { depth, normals, segmentation };
            scene.mask_sky();
            out.push(scene);
        }
        Ok(out)
    }
}

impl DepthModel for TipsDepth {
    fn mode(&self) -> &str {
        &self.mode
    }

    fn device(&self) -> String {
        lv_core::device::device_label(&self.device)
    }

    fn max_depth_m(&self) -> Option<f32> {
        Some(self.cfg.max_depth)
    }

    fn infer(&mut self, rgb: &Rgb) -> Result<DepthMap> {
        self.predict(rgb)
    }

    fn infer_batch(&mut self, frames: &[&Rgb]) -> Result<Vec<DepthMap>> {
        self.predict_batch(frames)
    }

    fn scene_heads(&self) -> SceneHeads {
        self.heads
    }

    fn infer_scene_batch(&mut self, frames: &[&Rgb], want: SceneHeads) -> Result<Vec<DepthScene>> {
        self.scene_batch(frames, want)
    }

    fn set_pyramid(&mut self, cfg: Option<&PyramidCfg>) {
        self.input_grid = cfg.map(InputGrid::pyramid).unwrap_or_default();
    }
}
