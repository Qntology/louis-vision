//! Rule-6-compliant bridge from `JellyActorSpec` -> `HaringDancerRig` + visual invariant audit.
//!
//! Strictly preserves Rule 6 (`CLAUDE.md`):
//! - Flat vivid interior purity `>= 99.5%`
//! - `0 px` `#000000` outline leakage between body interior and background
//! - `>= 4` floating air-gapped motion ticks (`air_gap_m in 0.045..=0.075 m`)

use anyhow::{ensure, Result};
use lv_sds::blockies_haring::derive_blockies_haring_spec;
use sandbox_core::rig::haring_dancer::{
    build_haring_dancer_rig, render_haring_orthographic_rgb, HaringDancerRig,
};
use serde::{Deserialize, Serialize};

use super::motion::{BpmMotif, HeadPropKind};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JellyActorSpec {
    pub actor_id: String,
    pub wallet_0x: String,
    pub origin_xz: [f32; 2],
    pub bpm: f32,
    pub motif: BpmMotif,
    pub head_prop: HeadPropKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HaringRigAudit {
    pub actor_id: String,
    pub interior_purity_pct: f32,
    pub outline_leak_px: usize,
    pub floating_tick_count: usize,
    pub passed: bool,
}

fn hsl_to_rgb(h_deg: u16, s_pct: f64, l_pct: f64) -> [u8; 3] {
    let h = (h_deg as f64) % 360.0;
    let s = (s_pct / 100.0).clamp(0.0, 1.0);
    let l = (l_pct / 100.0).clamp(0.0, 1.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r1, g1, b1) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    [
        ((r1 + m) * 255.0).round().clamp(16.0, 240.0) as u8,
        ((g1 + m) * 255.0).round().clamp(16.0, 240.0) as u8,
        ((b1 + m) * 255.0).round().clamp(16.0, 240.0) as u8,
    ]
}

/// Converts a `JellyActorSpec` into a deterministic Rule-6-compliant `HaringDancerRig`
/// whose flat vivid interior color is derived from EIP-55 case-insensitive `wallet_0x`.
pub fn build_rig_from_jelly_actor(spec: &JellyActorSpec, beat_env: f32) -> HaringDancerRig {
    let bh = derive_blockies_haring_spec(&spec.wallet_0x);
    let body_rgb = hsl_to_rgb(
        bh.haring_body_hsl.h_deg,
        bh.haring_body_hsl.s_pct,
        bh.haring_body_hsl.l_pct,
    );
    build_haring_dancer_rig(&spec.actor_id, &spec.wallet_0x, body_rgb, beat_env)
}

/// Verifies Rule 6 invariants on the orthographic render of `rig`:
/// `>= 99.5%` interior purity, `0 px` `#000000` outline leakage, and `>= 4` air-gapped floating ticks.
pub fn audit_haring_rig_invariants(rig: &HaringDancerRig) -> Result<HaringRigAudit> {
    let w = 192usize;
    let h = 192usize;
    let bg = [242u8, 244, 247];
    let rgb = render_haring_orthographic_rgb(rig, w, h, 0, bg);

    let mut body_px = 0usize;
    let mut pure_body_px = 0usize;
    let mut leak_px = 0usize;

    let px_at = |x: usize, y: usize| -> [u8; 3] {
        let o = (y * w + x) * 3;
        [rgb[o], rgb[o + 1], rgb[o + 2]]
    };

    for y in 1..(h - 1) {
        for x in 1..(w - 1) {
            let p = px_at(x, y);
            if p != bg && p != [0, 0, 0] {
                body_px += 1;
                if p == rig.body_rgb {
                    pure_body_px += 1;
                }
                // Check 4-neighbors: body interior must NEVER directly touch bg without #000000 outline
                for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                    if px_at(nx, ny) == bg {
                        leak_px += 1;
                        break;
                    }
                }
            }
        }
    }

    let purity_pct = if body_px == 0 {
        0.0
    } else {
        (pure_body_px as f32 / body_px as f32) * 100.0
    };
    let ticks_valid = rig.motion_ticks.len() >= 4
        && rig
            .motion_ticks
            .iter()
            .all(|t| (0.045..=0.075).contains(&t.air_gap_m));
    let passed = purity_pct >= 99.5 && leak_px == 0 && ticks_valid;

    ensure!(
        passed,
        "Rule 6 Haring invariant failed for {}: purity={purity_pct:.2}% leak_px={leak_px} ticks={}",
        rig.actor_id,
        rig.motion_ticks.len()
    );

    Ok(HaringRigAudit {
        actor_id: rig.actor_id.clone(),
        interior_purity_pct: purity_pct,
        outline_leak_px: leak_px,
        floating_tick_count: rig.motion_ticks.len(),
        passed,
    })
}
