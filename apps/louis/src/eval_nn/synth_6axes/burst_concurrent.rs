//! Axis 6 (`BurstConcurrent`): 50-doc 4-thread ingest (`max_lock_hold_us < 2,000`), `ReadOnly` 0 evictions, low-VRAM fallback, and `PurgeScope::TaskEnd`.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{ensure, Result};
use candle_core::Device;
use lv_core::device::PurgeScope;
use lv_core::residency::{Action, Group, Loaded, ModelId, Phase, QwenTier, Residency, ResidencyMode};
use lv_router::residency_router::RouterError;

use crate::watcher::{FolderWatcher, StagedLouisPipeline};

pub fn run_burst_concurrent_d7_probe() -> Result<()> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("louis-d7-burst-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;

    // Track 1: 50 files across folder watcher -> first scan 50, unchanged rescan 0
    for i in 0..50usize {
        std::fs::write(
            dir.join(format!("doc_{i:02}.xml")),
            format!("<doc id=\"{i}\">DOM-{:02} 정산 {i}</doc>", (i % 18) + 1),
        )?;
    }
    let mut watcher = FolderWatcher::new();
    let (ch1, h1) = watcher.poll_dir(&dir)?;
    ensure!(ch1.len() == 50 && h1 == 50);
    let (ch2, h2) = watcher.poll_dir(&dir)?;
    ensure!(ch2.is_empty() && h2 == 0);
    let _ = std::fs::remove_dir_all(&dir);

    // Track 1+2: 4-thread concurrent ingest (< 2,000 us lock hold) + ReadOnly lookup (0 evictions)
    let pipe = Arc::new(StagedLouisPipeline::new(4096));
    pipe.pin_slot(ModelId::Siglip2, Group::Vision, 512)?;
    let xml_sample = b"<doc><p>DOM-01 clean burst</p></doc>".to_vec();

    let handles: Vec<_> = (0..4)
        .map(|tid| {
            let p = Arc::clone(&pipe);
            let bytes = xml_sample.clone();
            std::thread::spawn(move || -> Result<u128> {
                let mut worst = 0u128;
                for j in 0..12 {
                    let res = p.ingest_document(&format!("b_{tid}_{j}"), "xml", &bytes)?;
                    worst = worst.max(res.max_lock_hold_us);
                    let hits = p.lookup_readonly(
                        ModelId::Siglip2,
                        &Phase::new("ov", [Group::Vision]),
                    )?;
                    ensure!(hits == 1);
                    let miss = p.lookup_readonly(
                        ModelId::Qwen35(QwenTier::Heavy4B),
                        &Phase::new("heavy", [Group::Text]),
                    );
                    ensure!(matches!(miss, Err(RouterError::ReadOnlyMiss { .. })));
                }
                Ok(worst)
            })
        })
        .collect();

    for h in handles {
        let worst = h
            .join()
            .map_err(|_| anyhow::anyhow!("burst worker panicked"))??;
        ensure!(worst < 2_000, "mutex hold {worst} us >= 2000 us");
    }

    // Track 3+4: StrictSwap transition Release-before-Load + TaskEnd purge to 0 resident bytes
    let mut res = Residency::new(Device::Cpu, 8_192, ResidencyMode::StrictSwap);
    res.enter_phase(
        ModelId::Gemma4,
        &Phase::new("gemma", [Group::Embedding]),
        Some(8_192),
        |g| Ok(Arc::new(Loaded::empty(g, 1_024))),
    )?;
    res.enter_phase(
        ModelId::Qwen35(QwenTier::Base2B),
        &Phase::new("base2b", [Group::Text]),
        Some(8_192),
        |g| Ok(Arc::new(Loaded::empty(g, 2_048))),
    )?;
    let trans = res.transitions();
    let idx_rel = trans.iter().position(|t| t.action == Action::Release);
    let idx_load2 = trans
        .iter()
        .rposition(|t| t.action == Action::Load && t.model == ModelId::Qwen35(QwenTier::Base2B));
    ensure!(idx_rel.is_some() && idx_load2.is_some() && idx_rel < idx_load2);

    res.purge(PurgeScope::TaskEnd)?;
    ensure!(res.resident_bytes() == 0);
    Ok(())
}
