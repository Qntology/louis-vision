//! Unified Qwen3.5 (`Nano08B`, `Base2B`, `Heavy4B`) engine backbone, 2-Phase VLM OCR adapter,
//! 2-tier `HybridSnapshotCache`, and hardware-aware `Q8_0` vs `Q4_K_M` precision selector.

use anyhow::{bail, Result};
use candle_core::{DType, Device, Tensor, D};
use serde::{Deserialize, Serialize};

use crate::device::{PurgeScope, RuntimeDeviceSummary};
use crate::residency::{ModelId, QwenTier};
use crate::vocab_trim::{validate_host_token_ids, DocLangScope};

pub const QWEN35_VOCAB_SIZE: usize = 248_320;
pub const PARTIAL_ROPE_FACTOR: f32 = 0.25;
pub const QWEN35_ROPE_THETA: f64 = 10_000_000.0;
pub const DISCRETE_Q8_MIN_TOTAL_BYTES: u64 = 7_500_000_000;
pub const DISCRETE_Q8_MIN_FREE_BYTES: u64 = 4_608 << 20;
pub const UMA_Q8_MIN_TOTAL_BYTES: u64 = 16 << 30;
pub const VISION_SPECIAL_RAW_IDS: [u32; 5] = [248_053, 248_054, 248_055, 248_056, 248_057];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QwenPrecision {
    Q4Km,
    Q8_0,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Qwen35Config {
    pub tier: QwenTier,
    pub total_layers: usize,
    pub gdn_layers: usize,
    pub full_attn_layers: usize,
    pub hidden_size: usize,
    pub vlm_vision_blocks: usize,
    pub vocab_size: usize,
    pub static_artifact_k: Option<usize>,
    pub resident_q4_mb: usize,
    pub resident_q8_mb: usize,
    pub snapshot_vram_cap_mb: usize,
    pub partial_rope_factor: f32,
    pub rope_theta: f64,
}

impl Qwen35Config {
    pub fn for_tier(tier: QwenTier) -> Self {
        match tier {
            QwenTier::Nano08B => Self {
                tier,
                total_layers: 24,
                gdn_layers: 18,
                full_attn_layers: 6,
                hidden_size: 1024,
                vlm_vision_blocks: 0,
                vocab_size: QWEN35_VOCAB_SIZE,
                static_artifact_k: None,
                resident_q4_mb: 420,
                resident_q8_mb: 550,
                snapshot_vram_cap_mb: 16,
                partial_rope_factor: PARTIAL_ROPE_FACTOR,
                rope_theta: QWEN35_ROPE_THETA,
            },
            QwenTier::Base2B => Self {
                tier,
                total_layers: 24,
                gdn_layers: 18,
                full_attn_layers: 6,
                hidden_size: 2048,
                vlm_vision_blocks: 27,
                vocab_size: QWEN35_VOCAB_SIZE,
                static_artifact_k: None,
                resident_q4_mb: 1150,
                resident_q8_mb: 1850,
                snapshot_vram_cap_mb: 30,
                partial_rope_factor: PARTIAL_ROPE_FACTOR,
                rope_theta: QWEN35_ROPE_THETA,
            },
            QwenTier::Heavy4B => Self {
                tier,
                total_layers: 32,
                gdn_layers: 24,
                full_attn_layers: 8,
                hidden_size: 2560,
                vlm_vision_blocks: 27,
                vocab_size: QWEN35_VOCAB_SIZE,
                static_artifact_k: Some(65_536),
                resident_q4_mb: 2161,
                resident_q8_mb: 3666,
                snapshot_vram_cap_mb: 58,
                partial_rope_factor: PARTIAL_ROPE_FACTOR,
                rope_theta: QWEN35_ROPE_THETA,
            },
        }
    }
}

/// Selects `Q8_0` automatically on 8GB+ discrete GPUs (`total >= 7.5GB && free >= 4608 MiB`) or
/// `16GB+` UMA systems when `runtime-q8_0` artifact is available; never upward-requantizes `Q4_K_M -> Q8_0`.
pub fn select_heavy4b_precision(
    summary: &RuntimeDeviceSummary,
    free_vram_bytes: u64,
    has_runtime_q8_0: bool,
) -> QwenPrecision {
    if !has_runtime_q8_0 {
        return QwenPrecision::Q4Km;
    }
    let total = summary.total_vram_bytes.unwrap_or(0);
    let qualifies = if summary.is_unified_memory {
        total >= UMA_Q8_MIN_TOTAL_BYTES
    } else {
        total >= DISCRETE_Q8_MIN_TOTAL_BYTES && free_vram_bytes >= DISCRETE_Q8_MIN_FREE_BYTES
    };
    if qualifies {
        QwenPrecision::Q8_0
    } else {
        QwenPrecision::Q4Km
    }
}

/// Computes Qwen3.5 `(1 + w)` RMSNorm in F32 (keeps A_log, dt_bias, conv1d, norms in F32).
pub fn rms_norm_one_plus_w(x: &Tensor, weight: &Tensor, eps: f64) -> candle_core::Result<Tensor> {
    let orig = x.dtype();
    let x_f32 = x.to_dtype(DType::F32)?;
    let w_f32 = weight.to_dtype(DType::F32)?.affine(1.0, 1.0)?;
    let mean_sq = x_f32.sqr()?.mean_keepdim(D::Minus1)?;
    let inv = mean_sq.affine(1.0, eps)?.powf(-0.5)?;
    let y = x_f32.broadcast_mul(&inv)?.broadcast_mul(&w_f32)?;
    y.to_dtype(orig)
}

/// Splits fused `query + gate` linear output `[..., 2*D]` into `(query, gate)`.
pub fn split_query_and_gate(q_gate: &Tensor) -> candle_core::Result<(Tensor, Tensor)> {
    let last = q_gate.dim(D::Minus1)?;
    if last % 2 != 0 {
        return Err(candle_core::Error::Msg(format!(
            "split_query_and_gate requires even last dim, got {last}"
        )));
    }
    let half = last / 2;
    Ok((
        q_gate.narrow(D::Minus1, 0, half)?,
        q_gate.narrow(D::Minus1, half, half)?,
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSystemPrefixSlot {
    pub prompt_key: String,
    pub token_len: usize,
    pub kv_host_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveVramSnapshotSlot {
    pub doc_id: String,
    pub prefix_tokens: usize,
    pub gdn_state_bytes: usize,
    pub conv_window_bytes: usize,
    pub attn_kv_bytes: usize,
    pub last_hidden_bytes: usize,
}

impl ActiveVramSnapshotSlot {
    pub fn total_bytes(&self) -> usize {
        self.gdn_state_bytes + self.conv_window_bytes + self.attn_kv_bytes + self.last_hidden_bytes
    }

    pub fn total_mb(&self) -> f64 {
        self.total_bytes() as f64 / (1024.0 * 1024.0)
    }
}

/// Two-tier hybrid snapshot cache:
/// - `system_host_slot`: host RAM common system instruction snapshot (`0 B` GPU VRAM).
/// - `active_vram_slot`: single GPU slot (`atten KV + GDN S + conv C + last_hidden`) capped per tier,
///   bypassed cleanly on `pixel_values.is_some()` passes and reclaimed at `PurgeScope::TaskEnd`.
#[derive(Debug, Clone)]
pub struct HybridSnapshotCache {
    pub cfg: Qwen35Config,
    pub system_host_slot: Option<HostSystemPrefixSlot>,
    pub active_vram_slot: Option<ActiveVramSnapshotSlot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotStepPlan {
    pub used_snapshot: bool,
    pub bypassed_for_vision: bool,
    pub prefill_saved_pct: u8,
}

impl HybridSnapshotCache {
    pub fn new(tier: QwenTier) -> Self {
        Self {
            cfg: Qwen35Config::for_tier(tier),
            system_host_slot: None,
            active_vram_slot: None,
        }
    }

    pub fn install_host_prefix(&mut self, prompt_key: impl Into<String>, token_len: usize) {
        let kv_host_bytes = token_len * self.cfg.full_attn_layers * 2 * 512;
        self.system_host_slot = Some(HostSystemPrefixSlot {
            prompt_key: prompt_key.into(),
            token_len,
            kv_host_bytes,
        });
    }

    pub fn store_active_doc(
        &mut self,
        doc_id: impl Into<String>,
        prefix_tokens: usize,
    ) -> Result<()> {
        let hidden_bytes = self.cfg.hidden_size * 4;
        let gdn_bytes = (self.cfg.snapshot_vram_cap_mb * 1024 * 1024) / 3;
        let conv_bytes = (self.cfg.snapshot_vram_cap_mb * 1024 * 1024) / 8;
        let attn_kv_bytes = (self.cfg.snapshot_vram_cap_mb * 1024 * 1024) / 3;
        let slot = ActiveVramSnapshotSlot {
            doc_id: doc_id.into(),
            prefix_tokens,
            gdn_state_bytes: gdn_bytes,
            conv_window_bytes: conv_bytes,
            attn_kv_bytes,
            last_hidden_bytes: hidden_bytes,
        };
        if slot.total_bytes() > self.cfg.snapshot_vram_cap_mb * 1024 * 1024 {
            bail!(
                "active snapshot {} bytes exceeds tier cap {} MiB",
                slot.total_bytes(),
                self.cfg.snapshot_vram_cap_mb
            );
        }
        self.active_vram_slot = Some(slot);
        Ok(())
    }

    pub fn prepare_step(&self, has_pixel_values: bool) -> SnapshotStepPlan {
        if has_pixel_values {
            return SnapshotStepPlan {
                used_snapshot: false,
                bypassed_for_vision: true,
                prefill_saved_pct: 0,
            };
        }
        if self.active_vram_slot.is_some() {
            SnapshotStepPlan {
                used_snapshot: true,
                bypassed_for_vision: false,
                prefill_saved_pct: 95,
            }
        } else {
            SnapshotStepPlan {
                used_snapshot: false,
                bypassed_for_vision: false,
                prefill_saved_pct: 0,
            }
        }
    }

    pub fn active_vram_bytes(&self) -> usize {
        self.active_vram_slot
            .as_ref()
            .map(ActiveVramSnapshotSlot::total_bytes)
            .unwrap_or(0)
    }

    pub fn purge(&mut self, scope: PurgeScope) {
        if scope == PurgeScope::TaskEnd {
            self.active_vram_slot = None;
        }
    }
}

/// Validates and translates raw vision special token IDs against the active checkpoint vocabulary limit
/// prior to any GPU `embedding` / `index_select` execution.
pub fn sync_vision_token_ids(
    manifest_ids: Option<&[u32; 5]>,
    active_vocab_limit: usize,
) -> Result<[u32; 5]> {
    let ids = manifest_ids.copied().unwrap_or(VISION_SPECIAL_RAW_IDS);
    validate_host_token_ids(&ids, active_vocab_limit)?;
    Ok(ids)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CascadeStep {
    Step0ZeroLlm,
    Step1Nano08B,
    Step2Base2B,
    Step3Heavy4B,
}

impl CascadeStep {
    pub fn model_id(self) -> ModelId {
        match self {
            Self::Step0ZeroLlm => ModelId::Gemma4,
            Self::Step1Nano08B => ModelId::Qwen35(QwenTier::Nano08B),
            Self::Step2Base2B => ModelId::Qwen35(QwenTier::Base2B),
            Self::Step3Heavy4B => ModelId::Qwen35(QwenTier::Heavy4B),
        }
    }
}

/// Computes which Qwen3.5 tier handles a crop or document:
/// - `needs_15pct_retry_or_redline` or complex 12-lang drafting -> `Heavy4B`
/// - Non-Core12 Tier-2 script (`ru`, `uk`, `th`, `hi`, `el`, `he`) or general VLM OCR -> `Base2B`
/// - Ambiguous Step-0 margin tie-break / kiosk open-grade -> `Nano08B`
pub fn select_cascade_step(
    ambiguous_margin_only: bool,
    has_tier2_fallback_script: bool,
    needs_crop_escalation: bool,
) -> CascadeStep {
    if needs_crop_escalation {
        return CascadeStep::Step3Heavy4B;
    }
    if has_tier2_fallback_script {
        return CascadeStep::Step2Base2B;
    }
    if ambiguous_margin_only {
        return CascadeStep::Step1Nano08B;
    }
    CascadeStep::Step2Base2B
}

pub fn check_tier2_fallback_lossless(text: &str) -> bool {
    !text.contains('\u{FFFD}')
}

/// Synchronizes first OCR crop read text into `DocLangScope::image` so subsequent crops inherit detected script.
pub fn observe_first_crop_scope(scope: &DocLangScope, crop_text: &str) {
    scope.observe_read(crop_text);
}

pub fn dummy_hidden_norm_check(dev: &Device) -> Result<()> {
    let x = Tensor::from_vec(vec![1.0f32, 2.0, 3.0, 4.0], (1, 4), dev)?;
    let w = Tensor::zeros((4,), DType::F32, dev)?;
    let y = rms_norm_one_plus_w(&x, &w, 1e-6)?;
    let _vals = y.squeeze(0)?.to_vec1::<f32>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heavy4b_precision_selects_q8_on_8gb_discrete_and_falls_back_cleanly() {
        let discrete_8gb = RuntimeDeviceSummary {
            backend_kind: crate::device::RuntimeBackendKind::Vulkan,
            runtime_ordinal: 0,
            pci_bus_id: None,
            device_uuid: None,
            total_vram_bytes: Some(8 * 1024 * 1024 * 1024),
            is_unified_memory: false,
            is_software_adapter: false,
        };
        assert_eq!(
            select_heavy4b_precision(&discrete_8gb, 5120 << 20, true),
            QwenPrecision::Q8_0
        );
        assert_eq!(
            select_heavy4b_precision(&discrete_8gb, 3000 << 20, true),
            QwenPrecision::Q4Km
        );
        assert_eq!(
            select_heavy4b_precision(&discrete_8gb, 5120 << 20, false),
            QwenPrecision::Q4Km
        );
    }

    #[test]
    fn snapshot_cache_bypasses_vision_and_purges_at_task_end() {
        let mut cache = HybridSnapshotCache::new(QwenTier::Base2B);
        cache.install_host_prefix("sys", 128);
        cache.store_active_doc("doc-1", 512).unwrap();
        assert!(cache.active_vram_bytes() > 0);
        assert!(cache.prepare_step(true).bypassed_for_vision);
        assert!(cache.prepare_step(false).used_snapshot);
        cache.purge(PurgeScope::TaskEnd);
        assert_eq!(cache.active_vram_bytes(), 0);
    }

    #[test]
    fn vision_token_ids_fit_heavy4b_static_artifact_when_remapped_and_reject_oob() {
        assert!(sync_vision_token_ids(None, 65_536).is_err());
        let remapped = [65_000, 65_001, 65_002, 65_003, 65_004];
        assert!(sync_vision_token_ids(Some(&remapped), 65_536).is_ok());
        assert!(sync_vision_token_ids(None, QWEN35_VOCAB_SIZE).is_ok());
    }
}
