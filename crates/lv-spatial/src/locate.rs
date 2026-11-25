//! Semantic heat-map localization powered by `lv_core::siglip2` (`VisionTower` + `TextTower`)
//! and `premium_discount_sds::matrix::double_center` (zero duplicate SigLIP2 implementation).

use anyhow::Result;
use serde::{Deserialize, Serialize};

use lv_core::device::{trim_gpu_pool_scoped, PurgeScope};
use lv_core::siglip2::{
    cosine, preprocess_for, PatchGrid, SiglipCompatProfile, SiglipTokenizer, TextTower,
    VisionTower,
};
use premium_discount_sds::matrix::double_center;

use crate::frame::Rgb;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LabelPrompt {
    pub label: String,
    pub phrases: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Heat {
    pub cols: u32,
    pub rows: u32,
    pub values: Vec<f32>,
}

impl Heat {
    pub fn max(&self) -> f32 {
        self.values.iter().copied().filter(|v| v.is_finite()).fold(f32::NEG_INFINITY, f32::max)
    }

    pub fn argmax(&self) -> Option<(u32, u32, f32)> {
        let (idx, &val) = self
            .values
            .iter()
            .enumerate()
            .filter(|(_, v)| v.is_finite())
            .max_by(|a, b| a.1.total_cmp(b.1))?;
        Some((idx as u32 % self.cols.max(1), idx as u32 / self.cols.max(1), val))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticPoint {
    pub label: String,
    pub px: f32,
    pub py: f32,
    pub score: f32,
    pub presence: f32,
    pub heat: Heat,
}

pub trait SemanticLocator: Send {
    fn locate(&mut self, rgb: &Rgb, prompts: &[LabelPrompt]) -> Result<Vec<SemanticPoint>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LocateScoring {
    Cosine,
    #[default]
    Contrast,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocateCfg {
    pub scoring: LocateScoring,
    pub max_patches: usize,
    pub double_center: bool,
    pub template: String,
    pub logit_scale: f32,
    pub logit_bias: f32,
}

impl Default for LocateCfg {
    fn default() -> Self {
        Self {
            scoring: LocateScoring::Contrast,
            max_patches: 256,
            double_center: true,
            template: "a photo of {}".into(),
            logit_scale: 10.0,
            logit_bias: -1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabelTexts {
    pub labels: Vec<(String, Vec<f32>, Option<Vec<f32>>)>,
    pub background: Vec<Vec<f32>>,
}

pub fn apply_template(template: &str, phrase: &str) -> String {
    if template.contains("{}") {
        template.replace("{}", phrase)
    } else {
        format!("{template} {phrase}")
    }
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Computes `(label, Heat, presence)` over dense patch features `[cols * rows][d]`,
/// optionally doubly centering the `[label x cell]` score matrix via `premium_discount_sds::matrix`.
pub fn label_heats(
    dense: &[Vec<f32>],
    cols: u32,
    rows: u32,
    texts: &LabelTexts,
    cfg: &LocateCfg,
    logit_scale: f32,
    logit_bias: f32,
) -> Vec<(String, Heat, f32)> {
    let n_cells = dense.len();
    let n_labels = texts.labels.len();
    if n_cells == 0 || n_labels == 0 {
        return Vec::new();
    }

    let mut raw_scores = vec![vec![0.0f32; n_cells]; n_labels];
    for (li, (_, anchor, alt)) in texts.labels.iter().enumerate() {
        for (ci, feat) in dense.iter().enumerate() {
            let mut best_cos = cosine(feat, anchor);
            if let Some(a2) = alt {
                best_cos = best_cos.max(cosine(feat, a2));
            }
            raw_scores[li][ci] = match cfg.scoring {
                LocateScoring::Cosine => best_cos,
                LocateScoring::Contrast => {
                    let bg_max = texts
                        .background
                        .iter()
                        .map(|b| cosine(feat, b))
                        .fold(f32::NEG_INFINITY, f32::max);
                    if bg_max.is_finite() {
                        sigmoid(logit_scale * (best_cos - bg_max) + logit_bias)
                    } else {
                        sigmoid(logit_scale * best_cos + logit_bias)
                    }
                }
            };
        }
    }

    if cfg.double_center && n_labels > 1 && n_cells > 1 {
        raw_scores = double_center(&raw_scores);
    }

    texts
        .labels
        .iter()
        .zip(raw_scores)
        .map(|((label, _, _), values)| {
            let presence = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            (label.clone(), Heat { cols, rows, values }, presence)
        })
        .collect()
}

pub fn points_from_heat(width: u32, height: u32, heats: Vec<(String, Heat)>) -> Vec<SemanticPoint> {
    let with_p = heats
        .into_iter()
        .map(|(l, h)| {
            let p = h.max();
            (l, h, p)
        })
        .collect();
    points_with_presence(width, height, with_p)
}

pub fn points_with_presence(
    width: u32,
    height: u32,
    heats: Vec<(String, Heat, f32)>,
) -> Vec<SemanticPoint> {
    heats
        .into_iter()
        .filter_map(|(label, heat, presence)| {
            let (cx, cy, score) = heat.argmax()?;
            let px = (cx as f32 + 0.5) / heat.cols.max(1) as f32 * width as f32;
            let py = (cy as f32 + 0.5) / heat.rows.max(1) as f32 * height as f32;
            Some(SemanticPoint { label, px, py, score, presence, heat })
        })
        .collect()
}

pub struct SiglipLocator {
    pub vision: VisionTower,
    pub text: Option<TextTower>,
    pub tokenizer: Option<SiglipTokenizer>,
    pub profile: SiglipCompatProfile,
    pub cfg: LocateCfg,
    pub background: Vec<Vec<f32>>,
}

impl SiglipLocator {
    pub fn new(
        vision: VisionTower,
        text: Option<TextTower>,
        tokenizer: Option<SiglipTokenizer>,
        cfg: LocateCfg,
    ) -> Self {
        let profile = match vision.layout {
            lv_core::siglip2::Layout::RaonVe => SiglipCompatProfile::RaonVe,
            lv_core::siglip2::Layout::Siglip2Hf => SiglipCompatProfile::NotoPyramidL1,
        };
        Self {
            vision,
            text,
            tokenizer,
            profile,
            cfg,
            background: Vec::new(),
        }
    }

    pub fn grid_for(&self, rgb: &Rgb) -> Result<PatchGrid> {
        preprocess_for(self.profile, &rgb.data, rgb.width, rgb.height, self.cfg.max_patches)
    }

    pub fn encode_texts(&self, prompts: &[LabelPrompt]) -> Result<LabelTexts> {
        let Some(text) = &self.text else {
            anyhow::bail!("SiglipLocator text tower is not attached");
        };
        let Some(tok) = &self.tokenizer else {
            anyhow::bail!("SiglipLocator tokenizer is not attached");
        };
        let mut labels = Vec::with_capacity(prompts.len());
        for p in prompts {
            let phrases: Vec<String> = if p.phrases.is_empty() {
                vec![apply_template(&self.cfg.template, &p.label)]
            } else {
                p.phrases.iter().map(|s| apply_template(&self.cfg.template, s)).collect()
            };
            let embs = text.embed(tok, &phrases, phrases.len().max(1))?;
            let emb = lv_core::siglip2::mean_unit(&embs)
                .ok_or_else(|| anyhow::anyhow!("empty text embeddings for '{}'", p.label))?;
            labels.push((p.label.clone(), emb, None));
        }
        Ok(LabelTexts { labels, background: self.background.clone() })
    }
}

impl SemanticLocator for SiglipLocator {
    fn locate(&mut self, rgb: &Rgb, prompts: &[LabelPrompt]) -> Result<Vec<SemanticPoint>> {
        let grid = self.grid_for(rgb)?;
        let pass = self.vision.forward_full(&grid)?;
        let dense = self.vision.dense(&pass)?;
        let texts = self.encode_texts(prompts)?;
        let heats = label_heats(
            &dense,
            grid.grid_w as u32,
            grid.grid_h as u32,
            &texts,
            &self.cfg,
            self.cfg.logit_scale,
            self.cfg.logit_bias,
        );
        trim_gpu_pool_scoped(self.vision.device(), PurgeScope::Phase);
        Ok(points_with_presence(rgb.width, rgb.height, heats))
    }
}
