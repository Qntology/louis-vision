//! Model residency manager enforcing single active phase / Coexist (SigLIP2 vision >= 512 MiB headroom)
//! / Pinned (60fps XR swap-free) invariants, `HEAD_VIEW` row-sliced projection weights, and lease safety.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Result};
use candle_core::{DType, Device, Tensor};

use crate::device::{trim_gpu_pool_scoped, PurgeScope};

/// Row-view tensor name for vocabulary-trimmed output projection (`token_embd.weight` / `output.weight`).
pub const HEAD_VIEW: &str = "head.view";
/// Host-only token embedding table name (never uploaded wholesale to GPU VRAM).
pub const TEXT_TOKEN_TABLE: &str = "text_model.embeddings.token_embedding.weight";
pub const DEFAULT_COEXIST_MARGIN_BYTES: u64 = 512 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModelId {
    Zoro,
    Gocr,
    Siglip2,
    Gemma4,
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
    RowView { base: String, rows: Arc<Vec<u32>>, tensor: Tensor },
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

    pub fn insert_row_view(&mut self, base: impl Into<String>, rows: Vec<u32>, tensor: Tensor) {
        self.weights.insert(
            HEAD_VIEW.to_string(),
            Weight::RowView {
                base: base.into(),
                rows: Arc::new(rows),
                tensor,
            },
        );
    }

    pub fn get(&self, name: &str) -> Result<&Weight> {
        self.weights
            .get(name)
            .ok_or_else(|| anyhow!("weight '{name}' is not loaded in group {:?}", self.group))
    }

    pub fn dense(&self, name: &str) -> Result<&Tensor> {
        match self.get(name)? {
            Weight::Dense(t) | Weight::RowView { tensor: t, .. } => Ok(t),
        }
    }

    /// Slices selected row indices from a 2D tensor into `HEAD_VIEW`.
    pub fn slice_head_view(table: &Tensor, rows: &[u32], device: &Device) -> Result<Tensor> {
        if rows.is_empty() {
            bail!("HEAD_VIEW rows cannot be empty");
        }
        let idx = Tensor::from_slice(rows, rows.len(), device)?;
        Ok(table.index_select(&idx, 0)?.to_dtype(DType::F32)?)
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

/// Thread-safe shared residency handle across SigLIP2 / VLM / Zoro.
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
        let vision_p = Phase::new("gocr:vision", [Group::Vision]);
        let lang_p = Phase::new("gocr:lang", [Group::blocks(0..4), Group::Output]);

        let handles = res
            .enter_phase(ModelId::Gocr, &vision_p, None, |g| Ok(Arc::new(Loaded::empty(g, 400))))
            .unwrap();
        // Attempting to unload while handle is alive must fail cleanly.
        assert!(res
            .enter_phase(ModelId::Gocr, &lang_p, None, |g| Ok(Arc::new(Loaded::empty(g, 300))))
            .is_err());

        drop(handles);
        let lang_h = res
            .enter_phase(ModelId::Gocr, &lang_p, None, |g| Ok(Arc::new(Loaded::empty(g, 300))))
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
        let z_phase = Phase::new("zoro", [Group::blocks(0..2)]);
        let _zh = res
            .enter_phase(ModelId::Zoro, &z_phase, Some(100 << 20), |g| Ok(Arc::new(Loaded::empty(g, 400))))
            .unwrap();
        assert!(res.pinned.is_empty());
    }

    #[test]
    fn head_view_slices_selected_rows_without_full_projection() {
        let table = Tensor::arange(0u32, 30, &Device::Cpu)
            .unwrap()
            .to_dtype(DType::F32)
            .unwrap()
            .reshape((6, 5))
            .unwrap();
        let sliced = Loaded::slice_head_view(&table, &[1, 4], &Device::Cpu).unwrap();
        assert_eq!(sliced.dims(), &[2, 5]);
        assert_eq!(sliced.to_vec2::<f32>().unwrap(), vec![vec![5.0, 6.0, 7.0, 8.0, 9.0], vec![20.0, 21.0, 22.0, 23.0, 24.0]]);
    }
}
