//! Two-phase VLM OcrEngine (encode all crops -> drop vision -> decode on CPU/GPU)
//! and 1-Hop Halo pruning over merged vision patch groups (`lv-kernels::halo`).

use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Result};
use candle_core::{Device, Tensor};

use crate::residency::{Group, Loaded, ModelId, Phase, SharedResidency};

pub const MAX_BATCH_PATCHES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneMode {
    Off,
    Halo1HopScatter,
    Halo1HopDrop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrunedGrid {
    pub kept_patch_indices: Vec<u32>,
    pub kept_group_indices: Vec<u32>,
    pub rep_blank_group: u32,
    pub total_groups: usize,
}

/// Computes a 1-Hop Halo pruning plan delegating pixel ink variance scan & 3x3 max-pool to `lv-kernels::halo`.
pub fn plan_1hop_halo(
    patches: &[f32],
    grid_h: usize,
    grid_w: usize,
    merge_size: usize,
    patch_len: usize,
    ink_delta_thresh: f32,
) -> Option<PrunedGrid> {
    let plan = lv_kernels::halo::plan_1hop_halo_indices(
        patches,
        grid_h,
        grid_w,
        merge_size,
        patch_len,
        ink_delta_thresh,
    )?;
    Some(PrunedGrid {
        kept_patch_indices: plan.kept_patch_indices,
        kept_group_indices: plan.kept_group_indices,
        rep_blank_group: plan.rep_blank_group,
        total_groups: plan.total_groups,
    })
}

/// Scatters pruned group features `[kept_groups, D]` back onto `[total_groups, D]`, filling pruned blanks with `rep_blank_group`.
pub fn scatter_groups(kept_out: &Tensor, plan: &PrunedGrid, device: &Device) -> candle_core::Result<Tensor> {
    let rep_pos = plan
        .kept_group_indices
        .binary_search(&plan.rep_blank_group)
        .expect("rep_blank_group must be present in kept_group_indices") as u32;
    let mut gather_idx = vec![rep_pos; plan.total_groups];
    for (pos, &orig_g) in plan.kept_group_indices.iter().enumerate() {
        gather_idx[orig_g as usize] = pos as u32;
    }
    let idx = Tensor::from_vec(gather_idx, (plan.total_groups,), device)?;
    kept_out.index_select(&idx, 0)
}

#[derive(Debug, Clone)]
pub struct OcrVisionFeatures {
    pub tokens: Tensor,
    pub rows: usize,
    pub cols: usize,
    pub prompt: String,
    pub vision_ms: f64,
}

pub trait VisionCropEncoder: Send + Sync {
    type Input;
    fn encode_batch(&self, vision: &Loaded, inputs: &[Self::Input]) -> Result<Vec<OcrVisionFeatures>>;
}

pub trait LanguageCropDecoder: Send + Sync {
    type Output;
    fn decode(&self, blocks: &Loaded, output: &Loaded, features: OcrVisionFeatures) -> Result<Self::Output>;
}

/// Enforces non-overlapping 2-phase Vision -> Language residency transition across arbitrary batch sizes.
pub struct OcrEngine<E: VisionCropEncoder, D: LanguageCropDecoder> {
    residency: SharedResidency,
    model_id: ModelId,
    blocks: usize,
    encoder: E,
    decoder: D,
}

impl<E: VisionCropEncoder, D: LanguageCropDecoder> OcrEngine<E, D> {
    pub fn new(residency: SharedResidency, model_id: ModelId, blocks: usize, encoder: E, decoder: D) -> Self {
        Self {
            residency,
            model_id,
            blocks,
            encoder,
            decoder,
        }
    }

    pub fn recognize<FL>(
        &self,
        inputs: &[E::Input],
        mut load_group: FL,
    ) -> Result<Vec<D::Output>>
    where
        FL: FnMut(Group) -> Result<Arc<Loaded>>,
    {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let mut res = self.residency.lock().unwrap_or_else(|p| p.into_inner());
        let t0 = Instant::now();
        let vision_phase = Phase::new("vlm:vision", [Group::Vision]);
        let features = {
            let handles = res.enter_phase(self.model_id, &vision_phase, None, &mut load_group)?;
            if handles.len() != 1 {
                bail!("expected single vision group handle");
            }
            self.encoder.encode_batch(&handles[0], inputs)?
        };
        let _vision_elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let lang_phase = Phase::new("vlm:lang", [Group::blocks(0..self.blocks), Group::Output]);
        let handles = res.enter_phase(self.model_id, &lang_phase, None, &mut load_group)?;
        let (blk_h, out_h) = (&handles[0], &handles[1]);

        let mut results = Vec::with_capacity(features.len());
        for feat in features {
            results.push(self.decoder.decode(blk_h, out_h, feat)?);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::residency::{Action, Residency, ResidencyMode};

    struct DummyEnc;
    impl VisionCropEncoder for DummyEnc {
        type Input = Vec<f32>;
        fn encode_batch(&self, vision: &Loaded, inputs: &[Self::Input]) -> Result<Vec<OcrVisionFeatures>> {
            assert_eq!(vision.group, Group::Vision);
            inputs
                .iter()
                .map(|v| {
                    Ok(OcrVisionFeatures {
                        tokens: Tensor::from_slice(v, v.len(), &Device::Cpu)?,
                        rows: 1,
                        cols: 1,
                        prompt: "Text Recognition:".into(),
                        vision_ms: 1.0,
                    })
                })
                .collect()
        }
    }

    struct DummyDec;
    impl LanguageCropDecoder for DummyDec {
        type Output = f32;
        fn decode(&self, blocks: &Loaded, output: &Loaded, feat: OcrVisionFeatures) -> Result<f32> {
            assert!(matches!(blocks.group, Group::Blocks { .. }));
            assert_eq!(output.group, Group::Output);
            Ok(feat.tokens.to_vec1::<f32>()?.iter().sum())
        }
    }

    #[test]
    fn two_phase_ocr_engine_unloads_vision_before_loading_language_weights() {
        let res = Residency::shared(Device::Cpu, 2048, ResidencyMode::StrictSwap);
        let engine = OcrEngine::new(Arc::clone(&res), ModelId::Gocr, 2, DummyEnc, DummyDec);
        let outs = engine
            .recognize(&[vec![1.0, 2.0], vec![3.0, 4.0]], |g| Ok(Arc::new(Loaded::empty(g, 500))))
            .unwrap();
        assert_eq!(outs, vec![3.0, 7.0]);
        let r = res.lock().unwrap();
        let actions: Vec<&Action> = r.transitions().iter().map(|t| &t.action).collect();
        assert_eq!(
            actions,
            vec![&Action::Load, &Action::Release, &Action::Load, &Action::Load]
        );
    }

    #[test]
    fn scatter_groups_reconstructs_full_tensor_grid() {
        let (hm, wm, merge, plen) = (10usize, 10usize, 2usize, 4usize);
        let group_elems = merge * merge * plen;
        let mut patches = vec![1.0f32; hm * wm * group_elems];
        patches[(5 * wm + 5) * group_elems] = -1.0;
        let plan = plan_1hop_halo(&patches, hm * merge, wm * merge, merge, plen, 0.05).unwrap();
        let kept = Tensor::arange(0u32, plan.kept_group_indices.len() as u32, &Device::Cpu)
            .unwrap()
            .to_dtype(candle_core::DType::F32)
            .unwrap()
            .reshape((plan.kept_group_indices.len(), 1))
            .unwrap();
        let full = scatter_groups(&kept, &plan, &Device::Cpu).unwrap();
        assert_eq!(full.dims(), &[100, 1]);
    }
}
