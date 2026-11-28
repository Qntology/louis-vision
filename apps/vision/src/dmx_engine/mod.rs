//! 6-Layer DMX / sACN Priority Merger (`vision-sandbox v1.8`).
//!
//! Priority order (highest to lowest):
//! - `L1SafetyPark`: Interlock Halt (velocity forced to `0.0`) & Universe 64 non-dimmable pyro isolation
//! - `L2Blackout`: Emergency Blackout (`dimmer = 0`, `rgb = [0,0,0]`, `laser = 0%`, never dims Univ 64 non-dimmable)
//! - `L3LaserBam`: Audience / camera frustum Laser BAM `0%` blanking zone
//! - `L4Inspector`: Live operator manual inspector override
//! - `L5PhaserRoboSpot`: Automated BPM phaser / RoboSpot performer follow-spot
//! - `L6CueSlice`: 10-second slice cue baseline

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub const PYRO_ISOLATED_UNIVERSE: u16 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DmxLayer {
    L1SafetyPark = 1,
    L2Blackout = 2,
    L3LaserBam = 3,
    L4Inspector = 4,
    L5PhaserRoboSpot = 5,
    L6CueSlice = 6,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DmxFixturePatch {
    pub fixture_id: String,
    pub universe: u16,
    pub dmx_addr: u16,
    pub non_dimmable: bool,
    pub is_pyro: bool,
    pub is_laser: bool,
    pub in_camera_frustum: bool,
}

impl DmxFixturePatch {
    pub fn validate(&self) -> Result<()> {
        if self.dmx_addr == 0 || self.dmx_addr > 512 {
            bail!("fixture '{}' invalid DMX address {}", self.fixture_id, self.dmx_addr);
        }
        if self.is_pyro && (self.universe != PYRO_ISOLATED_UNIVERSE || !self.non_dimmable) {
            bail!(
                "pyro fixture '{}' must be patched exclusively to non-dimmable Universe {PYRO_ISOLATED_UNIVERSE} (got univ={} non_dimmable={})",
                self.fixture_id,
                self.universe,
                self.non_dimmable
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DmxChannelState {
    pub dimmer_16bit: u16,
    pub pan_16bit: u16,
    pub tilt_16bit: u16,
    pub color_rgb: [u8; 3],
    pub velocity_mps: f32,
    pub laser_power_pct: f32,
}

impl Default for DmxChannelState {
    fn default() -> Self {
        Self {
            dimmer_16bit: 0,
            pan_16bit: 32_768,
            tilt_16bit: 32_768,
            color_rgb: [255, 255, 255],
            velocity_mps: 0.0,
            laser_power_pct: 0.0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DmxMergeInput {
    pub safety_park_active: bool,
    pub interlock_halt: bool,
    pub blackout_active: bool,
    pub inspector_override: Option<DmxChannelState>,
    pub phaser_robospot: Option<DmxChannelState>,
    pub cue_slice: DmxChannelState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DmxMergedOutput {
    pub fixture_id: String,
    pub universe: u16,
    pub winning_layer: DmxLayer,
    pub state: DmxChannelState,
    pub non_dimmable_enforced: bool,
    pub laser_bam_clamped: bool,
}

/// Evaluates the deterministic 6-layer DMX priority merger (`L1 > L2 > L3 > L4 > L5 > L6`).
pub fn merge_dmx_six_layer(patch: &DmxFixturePatch, input: &DmxMergeInput) -> Result<DmxMergedOutput> {
    patch.validate()?;

    // Start from lowest layers (L6 -> L5 -> L4)
    let (mut winning, mut state) = if let Some(insp) = input.inspector_override {
        (DmxLayer::L4Inspector, insp)
    } else if let Some(ph) = input.phaser_robospot {
        (DmxLayer::L5PhaserRoboSpot, ph)
    } else {
        (DmxLayer::L6CueSlice, input.cue_slice)
    };

    state.laser_power_pct = state.laser_power_pct.clamp(0.0, 100.0);
    let mut laser_bam_clamped = false;

    // L3: Laser BAM 0% blanking whenever laser intersects audience/camera frustum
    if patch.is_laser && patch.in_camera_frustum {
        state.laser_power_pct = 0.0;
        laser_bam_clamped = true;
        winning = DmxLayer::L3LaserBam;
    }

    // L2: Emergency Blackout zeroes optical output (never dims non-dimmable Universe 64 safety power rail)
    if input.blackout_active {
        if !patch.non_dimmable {
            state.dimmer_16bit = 0;
            state.color_rgb = [0, 0, 0];
        }
        state.laser_power_pct = 0.0;
        winning = DmxLayer::L2Blackout;
    }

    // L1: Safety Park / Interlock Halt unconditionally stops kinetic velocity and safes pyro/laser
    if input.safety_park_active || input.interlock_halt {
        state.velocity_mps = 0.0;
        state.laser_power_pct = 0.0;
        if patch.is_pyro {
            state.dimmer_16bit = 0;
        }
        winning = DmxLayer::L1SafetyPark;
    }

    Ok(DmxMergedOutput {
        fixture_id: patch.fixture_id.clone(),
        universe: patch.universe,
        winning_layer: winning,
        state,
        non_dimmable_enforced: patch.non_dimmable,
        laser_bam_clamped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moving_head_patch() -> DmxFixturePatch {
        DmxFixturePatch {
            fixture_id: "MH-01".into(),
            universe: 1,
            dmx_addr: 1,
            non_dimmable: false,
            is_pyro: false,
            is_laser: false,
            in_camera_frustum: false,
        }
    }

    #[test]
    fn enforces_strict_six_layer_priority_and_universe_64_pyro_isolation() {
        // Reject pyro outside Universe 64 or dimmable pyro
        let bad_pyro = DmxFixturePatch {
            fixture_id: "PYRO-1".into(),
            universe: 63,
            dmx_addr: 1,
            non_dimmable: true,
            is_pyro: true,
            is_laser: false,
            in_camera_frustum: false,
        };
        assert!(merge_dmx_six_layer(&bad_pyro, &DmxMergeInput::default()).is_err());

        let cue = DmxChannelState { dimmer_16bit: 20_000, velocity_mps: 0.6, laser_power_pct: 80.0, ..Default::default() };
        let phaser = DmxChannelState { dimmer_16bit: 40_000, velocity_mps: 0.7, laser_power_pct: 90.0, ..Default::default() };
        let insp = DmxChannelState { dimmer_16bit: 65_535, velocity_mps: 0.9, laser_power_pct: 100.0, ..Default::default() };

        // L6 < L5 < L4
        let out6 = merge_dmx_six_layer(&moving_head_patch(), &DmxMergeInput { cue_slice: cue, ..Default::default() }).unwrap();
        assert_eq!(out6.winning_layer, DmxLayer::L6CueSlice);
        assert_eq!(out6.state.dimmer_16bit, 20_000);

        let out5 = merge_dmx_six_layer(&moving_head_patch(), &DmxMergeInput { phaser_robospot: Some(phaser), cue_slice: cue, ..Default::default() }).unwrap();
        assert_eq!(out5.winning_layer, DmxLayer::L5PhaserRoboSpot);
        assert_eq!(out5.state.dimmer_16bit, 40_000);

        let out4 = merge_dmx_six_layer(&moving_head_patch(), &DmxMergeInput { inspector_override: Some(insp), phaser_robospot: Some(phaser), cue_slice: cue, ..Default::default() }).unwrap();
        assert_eq!(out4.winning_layer, DmxLayer::L4Inspector);
        assert_eq!(out4.state.dimmer_16bit, 65_535);

        // L3 Laser BAM overrides L4 when in camera frustum -> 0.0%
        let laser = DmxFixturePatch {
            fixture_id: "LASER-1".into(),
            universe: 2,
            dmx_addr: 1,
            non_dimmable: false,
            is_pyro: false,
            is_laser: true,
            in_camera_frustum: true,
        };
        let out3 = merge_dmx_six_layer(&laser, &DmxMergeInput { inspector_override: Some(insp), cue_slice: cue, ..Default::default() }).unwrap();
        assert_eq!(out3.winning_layer, DmxLayer::L3LaserBam);
        assert_eq!(out3.state.laser_power_pct, 0.0);
        assert!(out3.laser_bam_clamped);

        // L2 Blackout overrides L4/L3 -> dimmer=0, rgb=[0,0,0]
        let out2 = merge_dmx_six_layer(&moving_head_patch(), &DmxMergeInput { blackout_active: true, inspector_override: Some(insp), cue_slice: cue, ..Default::default() }).unwrap();
        assert_eq!(out2.winning_layer, DmxLayer::L2Blackout);
        assert_eq!(out2.state.dimmer_16bit, 0);
        assert_eq!(out2.state.color_rgb, [0, 0, 0]);

        // L1 Safety Park / Interlock Halt overrides everything -> velocity_mps = 0.0
        let out1 = merge_dmx_six_layer(&moving_head_patch(), &DmxMergeInput { interlock_halt: true, blackout_active: true, inspector_override: Some(insp), cue_slice: cue, ..Default::default() }).unwrap();
        assert_eq!(out1.winning_layer, DmxLayer::L1SafetyPark);
        assert_eq!(out1.state.velocity_mps, 0.0);
    }
}
