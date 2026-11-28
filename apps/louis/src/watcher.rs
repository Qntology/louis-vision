//! Incremental folder watcher (`(mtime, len)` pre-filter -> SHA-256), lock-free
//! `lv-agent::staged` inference pipeline (`< 2 ms` engine lock), and `ReadOnly` residency routing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, UNIX_EPOCH};

use anyhow::Result;
use candle_core::Device;
use lv_agent::staged::{run_staged, Commit, StagedEngine, StagedSnapshot};
use lv_core::residency::{Group, Loaded, ModelId, Phase, Residency, ResidencyMode};
use lv_doc::docs::{parse_docs_bytes, sanitize_doc_ir};
use lv_router::residency_router::{CacheAccessMode, ResidencyRouter, RouterError};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChange {
    Added(PathBuf, String),
    Modified(PathBuf, String),
}

#[derive(Debug, Default)]
pub struct FolderWatcher {
    seen: BTreeMap<PathBuf, (u128, u64, String)>,
}

impl FolderWatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Scans `dir`, skipping unchanged `(mtime_ns, len)` files without hashing their contents.
    pub fn poll_dir(&mut self, dir: &Path) -> Result<(Vec<FileChange>, usize)> {
        let mut changes = Vec::new();
        let mut hashed_count = 0usize;
        if !dir.exists() {
            return Ok((changes, hashed_count));
        }
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok().map(|d| d.path()))
            .filter(|p| p.is_file())
            .collect();
        entries.sort();

        for path in entries {
            let meta = std::fs::metadata(&path)?;
            let len = meta.len();
            let mtime_ns = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());

            if let Some((prev_mt, prev_len, _)) = self.seen.get(&path) {
                if *prev_mt == mtime_ns && *prev_len == len {
                    continue;
                }
            }
            let bytes = std::fs::read(&path)?;
            hashed_count += 1;
            let digest: String = Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let was_present = match self.seen.insert(path.clone(), (mtime_ns, len, digest.clone())) {
                None => false,
                Some((_, _, old_hash)) if old_hash == digest => continue,
                Some(_) => true,
            };
            changes.push(if was_present {
                FileChange::Modified(path, digest)
            } else {
                FileChange::Added(path, digest)
            });
        }
        Ok((changes, hashed_count))
    }
}

#[derive(Debug, Clone, Default)]
pub struct IndexState {
    pub docs: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct IngestResult {
    pub rev: u64,
    pub block_count: usize,
    pub max_lock_hold_us: u128,
}

pub struct StagedLouisPipeline {
    engine: Mutex<StagedEngine<IndexState>>,
    router: ResidencyRouter,
}

impl StagedLouisPipeline {
    pub fn new(vram_budget_bytes: u64) -> Self {
        let shared = Residency::shared(
            Device::Cpu,
            vram_budget_bytes,
            ResidencyMode::Coexist { margin_bytes: 0 },
        );
        Self {
            engine: Mutex::new(StagedEngine::new(IndexState::default())),
            router: ResidencyRouter::new(shared),
        }
    }

    pub fn pin_slot(&self, model: ModelId, group: Group, bytes: u64) -> Result<()> {
        let handles = self.router.acquire(
            CacheAccessMode::ReadWrite,
            model,
            &Phase::new("pin", [group]),
            None,
            move |g| Ok(Arc::new(Loaded::empty(g, bytes))),
        )?;
        let _ = handles.len();
        Ok(())
    }

    pub fn lookup_readonly(&self, model: ModelId, phase: &Phase) -> std::result::Result<usize, RouterError> {
        self.router
            .acquire(CacheAccessMode::ReadOnly, model, phase, None, |_| {
                unreachable!("ReadOnly lookup must never invoke weight loader")
            })
            .map(|v| v.len())
    }

    /// Parses `bytes` outside the mutex via `run_staged` and commits if revision matches.
    pub fn ingest_document(&self, key: &str, ext: &str, bytes: &[u8]) -> Result<IngestResult> {
        let max_lock_us = std::cell::Cell::new(0u128);
        let doc_key = key.to_string();
        let ext_s = ext.to_string();
        let raw = bytes.to_vec();

        let (rev, block_count) = run_staged(
            &self.engine,
            |e| {
                let t0 = Instant::now();
                let snap = StagedSnapshot { rev: e.rev, input: (doc_key.clone(), raw.clone()) };
                max_lock_us.set(max_lock_us.get().max(t0.elapsed().as_micros()));
                Ok(snap)
            },
            |(_k, data)| {
                // D-05: Always sanitize parsed IR in real watcher ingestion (strip invisible/covert artifacts).
                let raw_ir = parse_docs_bytes(&ext_s, data)?;
                let sanitized = sanitize_doc_ir(raw_ir, Vec::new());
                let lines: Vec<String> = sanitized.doc.blocks.iter().map(|b| format!("{b:?}")).collect();
                Ok(lines)
            },
            |e, _snap_rev, lines| {
                let t0 = Instant::now();
                let count = lines.len();
                e.state.docs.insert(doc_key.clone(), lines);
                let out = Ok(Commit::Done((e.rev + 1, count)));
                max_lock_us.set(max_lock_us.get().max(t0.elapsed().as_micros()));
                out
            },
        )?;

        Ok(IngestResult { rev, block_count, max_lock_hold_us: max_lock_us.get() })
    }

    pub fn ingest_field_records(
        &self,
        surface_key: &str,
        raw_lines: Vec<String>,
    ) -> Result<IngestResult> {
        let max_lock_us = std::cell::Cell::new(0u128);
        let key_owned = surface_key.to_string();

        run_staged(
            &self.engine,
            |eng| {
                let t0 = Instant::now();
                let existing = eng.state.docs.get(&key_owned).cloned().unwrap_or_default();
                let snap = StagedSnapshot {
                    rev: eng.rev,
                    input: (existing, raw_lines.clone()),
                };
                max_lock_us.set(max_lock_us.get().max(t0.elapsed().as_micros()));
                Ok(snap)
            },
            |(existing, incoming)| {
                let mut merged = existing.clone();
                for line in incoming {
                    let trimmed = line.trim().to_string();
                    if !trimmed.is_empty() && !merged.contains(&trimmed) {
                        merged.push(trimmed);
                    }
                }
                Ok(merged)
            },
            |eng, _snap_rev, merged| {
                let t0 = Instant::now();
                let block_count = merged.len();
                eng.state.docs.insert(key_owned.clone(), merged.clone());
                let commit_rev = eng.rev + 1;
                max_lock_us.set(max_lock_us.get().max(t0.elapsed().as_micros()));
                Ok(Commit::Done(IngestResult {
                    rev: commit_rev,
                    block_count,
                    max_lock_hold_us: max_lock_us.get(),
                }))
            },
        )
    }

    pub fn doc_count(&self) -> usize {
        self.engine.lock().map(|g| g.state.docs.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lv_doc::writer::{build_hwpx, NoticeSpec, WriterGuard};

    #[test]
    fn watcher_prefilter_staged_lock_hold_and_readonly_routing() {
        let tmp = std::env::temp_dir().join(format!("louis-docs-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let guard = WriterGuard::citizen_guard(vec![], vec!["1000".into()]);
        let hwpx = build_hwpx(
            &NoticeSpec { title: "검수 알림", body: "청구 금액 1000원 확인 완료" },
            &[],
            &guard,
        )
        .unwrap();
        let p = tmp.join("notice.hwpx");
        std::fs::write(&p, &hwpx).unwrap();

        let mut watcher = FolderWatcher::new();
        let (ch1, h1) = watcher.poll_dir(&tmp).unwrap();
        assert_eq!(ch1.len(), 1);
        assert_eq!(h1, 1);

        // Second poll sees unchanged mtime+len -> zero SHA-256 hashes computed.
        let (ch2, h2) = watcher.poll_dir(&tmp).unwrap();
        assert!(ch2.is_empty());
        assert_eq!(h2, 0);

        // Staged ingest parses .hwpx outside lock with sub-2ms mutex hold.
        let pipe = StagedLouisPipeline::new(4096);
        pipe.pin_slot(ModelId::Siglip2, Group::Vision, 512).unwrap();
        let ingested = pipe.ingest_document("notice.hwpx", "hwpx", &hwpx).unwrap();
        assert!(ingested.block_count >= 2);
        assert!(ingested.max_lock_hold_us < 2_000, "lock hold {} us", ingested.max_lock_hold_us);
        assert_eq!(pipe.doc_count(), 1);

        // ReadOnly overlay lookup succeeds on resident slot and rejects uncached phase without eviction.
        assert_eq!(pipe.lookup_readonly(ModelId::Siglip2, &Phase::new("overlay", [Group::Vision])).unwrap(), 1);
        assert!(matches!(
            pipe.lookup_readonly(ModelId::Gemma4, &Phase::new("cold", [Group::Vision])),
            Err(RouterError::ReadOnlyMiss { .. })
        ));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
