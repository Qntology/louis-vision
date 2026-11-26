//! Model residency manager enforcing single active phase / Coexist (SigLIP2 vision >= 512 MiB headroom)
//! / Pinned (60fps XR swap-free) invariants, quantized `TrimmedHead` handles, and lease safety.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Result};
use candle_core::{Device, Tensor};
use serde::{Deserialize, Serialize};

use crate::device::{trim_gpu_pool_scoped, PurgeScope};
use crate::vocab_trim::TrimmedHead;

/// Handle tensor/slot name for vocabulary-trimmed output projection (`token_embd.weight` / `output.weight`).
pub const HEAD_VIEW: &str = "head.view";
/// Host-only token embedding table name (never uploaded wholesale to GPU VRAM).
pub const TEXT_TOKEN_TABLE: &str = "text_model.embeddings.token_embedding.weight";
pub const DEFAULT_COEXIST_MARGIN_BYTES: u64 = 512 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QwenTier {
    /// Qwen/Qwen3.5-0.8B (24L: 18 GDN + 6 Full, H=1024, V=248,320)
    Nano08B,
    /// Qwen/Qwen3.5-2B (24L: 18 GDN + 6 Full, H=2048, V=248,320, 27L VLM)
    Base2B,
    /// Qwen/Qwen3.5-4B (32L: 24 GDN + 8 Full, H=2560, K_static=65,536, 27L VLM)
    Heavy4B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelId {
    Siglip2,
    Gemma4,
    Qwen35(QwenTier),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Group {
    Embedding,
    Blocks { start: u32, end: u32 },
    Output,
    Vision,
    Head,
    Text,
}

impl Group {
    pub fn blocks(range: Range<usize>) -> Self {
        Self::Blocks {
            start: range.start as u32,
            end: range.end as u32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidencyMode {
    /// Load current phase bundles, releasing all previous bundles before loading next phase.
    StrictSwap,
    /// Keep approved bundle (e.g., SigLIP2 Vision) resident alongside next model when free >= headroom + margin.
    Coexist { margin_bytes: u64 },
    /// Lock approved bundle permanently on device without eviction (for 60fps XR live mode).
    Pinned,
}

#[derive(Debug, Clone)]
pub struct Phase {
    pub name: String,
    pub groups: Vec<Group>,
}

impl Phase {
    pub fn new(name: impl Into<String>, groups: impl IntoIterator<Item = Group>) -> Self {
        Self {
            name: name.into(),
            groups: groups.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Weight {
    Dense(Tensor),
    TrimmedHead(Arc<TrimmedHead>),
}

#[derive(Debug)]
pub struct Loaded {
    pub group: Group,
    pub device_bytes: u64,
    weights: HashMap<String, Weight>,
}

impl Loaded {
    pub fn empty(group: Group, device_bytes: u64) -> Self {
        Self {
            group,
            device_bytes,
            weights: HashMap::new(),
        }
    }

    pub fn from_tensors(group: Group, device_bytes: u64, tensors: HashMap<String, Weight>) -> Self {
        Self {
            group,
            device_bytes,
            weights: tensors,
        }
    }

    /// Attaches a zero-requantized quantized `TrimmedHead` handle into `HEAD_VIEW` without F32 expansion.
    pub fn insert_trimmed_head(&mut self, head: Arc<TrimmedHead>) {
        self.weights.insert(HEAD_VIEW.to_string(), Weight::TrimmedHead(head));
    }

    pub fn get(&self, name: &str) -> Result<&Weight> {
        self.weights
            .get(name)
            .ok_or_else(|| anyhow!("weight '{name}' is not loaded in group {:?}", self.group))
    }

    pub fn dense(&self, name: &str) -> Result<&Tensor> {
        match self.get(name)? {
            Weight::Dense(t) => Ok(t),
            Weight::TrimmedHead(_) => bail!("weight '{name}' is a quantized TrimmedHead handle, not a dense tensor"),
        }
    }

    pub fn trimmed_head(&self) -> Result<&Arc<TrimmedHead>> {
        match self.get(HEAD_VIEW)? {
            Weight::TrimmedHead(h) => Ok(h),
            Weight::Dense(_) => bail!("{HEAD_VIEW} is not a TrimmedHead handle"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Load,
    Release,
    CoexistHit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub action: Action,
    pub model: ModelId,
    pub group: Group,
    pub bytes: u64,
}

/// Thread-safe shared residency handle across `Siglip2`, `Gemma4`, and `Qwen35`.
pub type SharedResidency = Arc<Mutex<Residency>>;

pub struct Residency {
    device: Device,
    budget_bytes: u64,
    mode: ResidencyMode,
    current_model: Option<ModelId>,
    current: BTreeMap<Group, Arc<Loaded>>,
    pinned: BTreeMap<(ModelId, Group), Arc<Loaded>>,
    transitions: Vec<Transition>,
}

impl Residency {
    pub fn new(device: Device, budget_bytes: u64, mode: ResidencyMode) -> Self {
        Self {
            device,
            budget_bytes,
            mode,
            current_model: None,
            current: BTreeMap::new(),
            pinned: BTreeMap::new(),
            transitions: Vec::new(),
        }
    }

    pub fn shared(device: Device, budget_bytes: u64, mode: ResidencyMode) -> SharedResidency {
        Arc::new(Mutex::new(Self::new(device, budget_bytes, mode)))
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn mode(&self) -> ResidencyMode {
        self.mode
    }

    pub fn resident_bytes(&self) -> u64 {
        self.current.values().map(|l| l.device_bytes).sum::<u64>()
            + self.pinned.values().map(|l| l.device_bytes).sum::<u64>()
    }

    pub fn transitions(&self) -> &[Transition] {
        &self.transitions
    }

    /// Pins a resident bundle for Coexist or XR Pinned mode.
    pub fn pin(&mut self, model: ModelId, loaded: Arc<Loaded>) {
        self.pinned.insert((model, loaded.group), loaded);
    }

    /// Unloads all non-pinned groups not in `phase`, failing if an active `Arc<Loaded>` lease is still held.
    pub fn enter_phase<F>(
        &mut self,
        model: ModelId,
        phase: &Phase,
        free_bytes_override: Option<u64>,
        mut loader: F,
    ) -> Result<Vec<Arc<Loaded>>>
    where
        F: FnMut(Group) -> Result<Arc<Loaded>>,
    {
        // Fast path: all groups served from pinned Coexist / Pinned cache.
        if phase
            .groups
            .iter()
            .all(|g| self.pinned.contains_key(&(model, *g)))
        {
            let mut out = Vec::with_capacity(phase.groups.len());
            for g in &phase.groups {
                let h = Arc::clone(&self.pinned[&(model, *g)]);
                self.transitions.push(Transition {
                    action: Action::CoexistHit,
                    model,
                    group: *g,
                    bytes: h.device_bytes,
                });
                out.push(h);
            }
            return Ok(out);
        }

        // Switching models releases previous model groups before loading new ones.
        if self.current_model != Some(model) {
            let old_model = self.current_model;
            let keys: Vec<Group> = self.current.keys().copied().collect();
            for g in keys {
                self.release_group(old_model.unwrap_or(model), g)?;
            }
            self.current_model = Some(model);
        }

        // Release stale groups not needed by the incoming phase BEFORE loading anything new.
        let stale: Vec<Group> = self
            .current
            .keys()
            .copied()
            .filter(|g| !phase.groups.contains(g))
            .collect();
        for g in stale {
            self.release_group(model, g)?;
        }

        // Check Coexist headroom constraint against free VRAM if provided.
        if let (ResidencyMode::Coexist { margin_bytes }, Some(free)) = (self.mode, free_bytes_override) {
            if !self.pinned.is_empty() && free < margin_bytes {
                self.evict_pinned()?;
            }
        }

        for g in &phase.groups {
            if !self.current.contains_key(g) && !self.pinned.contains_key(&(model, *g)) {
                let loaded = loader(*g)?;
                if self.budget_bytes > 0
                    && self.resident_bytes().saturating_add(loaded.device_bytes) > self.budget_bytes
                {
                    if matches!(self.mode, ResidencyMode::Coexist { .. }) && !self.pinned.is_empty() {
                        self.evict_pinned()?;
                    }
                }
                if self.budget_bytes > 0
                    && self.resident_bytes().saturating_add(loaded.device_bytes) > self.budget_bytes
                {
                    bail!(
                        "loading {:?} ({} bytes) would exceed budget {} bytes",
                        g,
                        loaded.device_bytes,
                        self.budget_bytes
                    );
                }
                self.transitions.push(Transition {
                    action: Action::Load,
                    model,
                    group: *g,
                    bytes: loaded.device_bytes,
                });
                self.current.insert(*g, loaded);
            }
        }

        Ok(phase
            .groups
            .iter()
            .map(|g| {
                self.pinned
                    .get(&(model, *g))
                    .or_else(|| self.current.get(g))
                    .cloned()
                    .expect("group loaded")
            })
            .collect())
    }

    /// Non-mutating lookup for `CacheAccessMode::ReadOnly`: resolves purely from already-resident
    /// `pinned` or `current` slots without switching `current_model` or releasing any group.
    /// Records `Action::CoexistHit` telemetry only when every group in `phase` is hit in `pinned`.
    pub fn lookup_resident(&mut self, model: ModelId, phase: &Phase) -> Option<Vec<Arc<Loaded>>> {
        let mut resolved = Vec::with_capacity(phase.groups.len());
        for g in &phase.groups {
            if let Some(h) = self.pinned.get(&(model, *g)) {
                resolved.push((true, *g, Arc::clone(h)));
            } else if self.current_model == Some(model) {
                let h = self.current.get(g)?;
                resolved.push((false, *g, Arc::clone(h)));
            } else {
                return None;
            }
        }
        for (is_pinned, g, h) in &resolved {
            if *is_pinned {
                self.transitions.push(Transition {
                    action: Action::CoexistHit,
                    model,
                    group: *g,
                    bytes: h.device_bytes,
                });
            }
        }
        Some(resolved.into_iter().map(|(_, _, h)| h).collect())
    }

    fn release_group(&mut self, model: ModelId, group: Group) -> Result<()> {
        let Some(existing) = self.current.get(&group) else {
            return Ok(());
        };
        let holders = Arc::strong_count(existing) - 1;
        if holders > 0 {
            bail!(
                "cannot unload {:?}/{:?}: {} active lease(s) still held",
                model,
                group,
                holders
            );
        }
        let dropped = self.current.remove(&group).unwrap();
        self.transitions.push(Transition {
            action: Action::Release,
            model,
            group,
            bytes: dropped.device_bytes,
        });
        drop(dropped);
        trim_gpu_pool_scoped(&self.device, PurgeScope::Phase);
        Ok(())
    }

    pub fn evict_pinned(&mut self) -> Result<()> {
        if self.mode == ResidencyMode::Pinned {
            bail!("cannot evict bundles in Pinned XR mode");
        }
        for ((m, g), arc) in &self.pinned {
            if Arc::strong_count(arc) > 1 {
                bail!("cannot evict pinned {:?}/{:?} while lease held", m, g);
            }
        }
        let old = std::mem::take(&mut self.pinned);
        for ((model, group), loaded) in old {
            self.transitions.push(Transition {
                action: Action::Release,
                model,
                group,
                bytes: loaded.device_bytes,
            });
        }
        trim_gpu_pool_scoped(&self.device, PurgeScope::Phase);
        Ok(())
    }

    pub fn purge(&mut self, scope: PurgeScope) -> Result<()> {
        if let Some(m) = self.current_model {
            let keys: Vec<Group> = self.current.keys().copied().collect();
            for g in keys {
                self.release_group(m, g)?;
            }
        }
        if scope == PurgeScope::TaskEnd && self.mode != ResidencyMode::Pinned {
            self.evict_pinned()?;
        }
        trim_gpu_pool_scoped(&self.device, scope);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releases_stale_groups_before_loading_new_phase_and_guards_active_leases() {
        let mut res = Residency::new(Device::Cpu, 1024, ResidencyMode::StrictSwap);
        let vision_p = Phase::new("qwen35:vision", [Group::Vision]);
        let lang_p = Phase::new("qwen35:lang", [Group::blocks(0..4), Group::Output]);

        let handles = res
            .enter_phase(ModelId::Qwen35(QwenTier::Base2B), &vision_p, None, |g| Ok(Arc::new(Loaded::empty(g, 400))))
            .unwrap();
        // Attempting to unload while handle is alive must fail cleanly.
        assert!(res
            .enter_phase(ModelId::Qwen35(QwenTier::Base2B), &lang_p, None, |g| Ok(Arc::new(Loaded::empty(g, 300))))
            .is_err());

        drop(handles);
        let lang_h = res
            .enter_phase(ModelId::Qwen35(QwenTier::Base2B), &lang_p, None, |g| Ok(Arc::new(Loaded::empty(g, 300))))
            .unwrap();
        assert_eq!(lang_h.len(), 2);
        // Check Release happened strictly before new Loads
        let actions: Vec<&Action> = res.transitions().iter().map(|t| &t.action).collect();
        assert_eq!(
            actions,
            vec![&Action::Load, &Action::Release, &Action::Load, &Action::Load]
        );
        drop(lang_h);
        res.purge(PurgeScope::TaskEnd).unwrap();
        assert_eq!(res.resident_bytes(), 0);
    }

    #[test]
    fn coexist_serves_siglip2_when_margin_holds_and_evicts_when_tight() {
        let mut res = Residency::new(
            Device::Cpu,
            2048,
            ResidencyMode::Coexist {
                margin_bytes: DEFAULT_COEXIST_MARGIN_BYTES,
            },
        );
        res.pin(ModelId::Siglip2, Arc::new(Loaded::empty(Group::Vision, 500)));

        let s_phase = Phase::new("siglip2", [Group::Vision]);
        let h = res.enter_phase(ModelId::Siglip2, &s_phase, Some(900 << 20), |_| unreachable!()).unwrap();
        assert_eq!(h[0].device_bytes, 500);
        drop(h);

        // Tight free VRAM triggers eviction of Coexist pinned bundle.
        let q_phase = Phase::new("qwen35_4b", [Group::blocks(0..2)]);
        let _qh = res
            .enter_phase(ModelId::Qwen35(QwenTier::Heavy4B), &q_phase, Some(100 << 20), |g| Ok(Arc::new(Loaded::empty(g, 400))))
            .unwrap();
        assert!(res.pinned.is_empty());
    }

    #[test]
    fn trimmed_head_attaches_zero_requant_handle_to_loaded_group() {
        use crate::vocab_trim::{GuardMode, TableSource, TrimmedHead};
        use candle_core::quantized::GgmlDType;

        let dir = std::env::temp_dir().join(format!("lv-res-head-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = TableSource::synthetic_q8_for_tests(&dir.join("head.bin"), 16, 64).unwrap();
        let kept = Arc::new(vec![1u32, 3, 5, 7]);
        let head = Arc::new(
            TrimmedHead::build(
                &src,
                QwenTier::Nano08B,
                0x1234,
                kept,
                "test_head",
                GuardMode::Off,
                &Device::Cpu,
            )
            .unwrap(),
        );
        let mut loaded = Loaded::empty(Group::Output, head.head_bytes() as u64);
        loaded.insert_trimmed_head(Arc::clone(&head));
        let got = loaded.trimmed_head().unwrap();
        assert_eq!(got.qweight.dtype(), GgmlDType::Q8_0);
        assert!(loaded.dense(HEAD_VIEW).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
