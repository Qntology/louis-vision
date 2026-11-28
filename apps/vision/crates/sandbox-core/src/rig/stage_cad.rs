//! Headless SolidCraft Stage Prop & Rigging CAD builder (`stage_cad_prompt`, v1.8).
//!
//! Pure Tier 1 geometry & URDF generator (Zero-Net / Zero-DB / Zero-Async) covering:
//! - `MovingHeadFixture` (Pan/Tilt revolute joints)
//! - `CurvedLedWall` (segmented arc + `EmissiveSurfaceSpec`)
//! - `KineticSplitLed` (left/right prismatic slider joints + auto `VenueHazardObb`)
//! - `KineticRunway` (elevatable runway prismatic joint + auto `VenueHazardObb`)
//! - `LogisticsRack` (multi-bay, multi-level warehouse rack for Sim/Stage parity)

use serde::{Deserialize, Serialize};

use super::urdf;
use super::venue_cad::VenueHazardObb;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmissiveSurfaceSpec {
    pub width_m: f32,
    pub height_m: f32,
    pub nits: f32,
    pub color_rgb: [u8; 3],
    pub video_slot: String,
}

impl Default for EmissiveSurfaceSpec {
    fn default() -> Self {
        Self {
            width_m: 6.0,
            height_m: 3.5,
            nits: 1200.0,
            color_rgb: [32, 168, 255],
            video_slot: "led_main_01".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MovingHeadFixture {
    pub id: String,
    pub mount_xyz: [f32; 3],
    pub pan_limits_rad: [f32; 2],
    pub tilt_limits_rad: [f32; 2],
    pub beam_angle_deg: f32,
}

impl MovingHeadFixture {
    pub fn standard(id: impl Into<String>, mount_xyz: [f32; 3]) -> Self {
        Self {
            id: id.into(),
            mount_xyz,
            pan_limits_rad: [-4.71239, 4.71239], // ±270 deg
            tilt_limits_rad: [-2.09440, 2.09440], // ±120 deg
            beam_angle_deg: 14.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvedLedWall {
    pub id: String,
    pub center_xyz: [f32; 3],
    pub radius_m: f32,
    pub arc_deg: f32,
    pub height_m: f32,
    pub segments: u32,
    pub emissive: EmissiveSurfaceSpec,
}

impl Default for CurvedLedWall {
    fn default() -> Self {
        Self {
            id: "curved_led_main".into(),
            center_xyz: [0.0, -4.5, 2.5],
            radius_m: 8.0,
            arc_deg: 90.0,
            height_m: 5.0,
            segments: 8,
            emissive: EmissiveSurfaceSpec {
                width_m: 12.5,
                height_m: 5.0,
                nits: 1500.0,
                color_rgb: [64, 180, 255],
                video_slot: "led_curved_01".into(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KineticSplitLed {
    pub id: String,
    pub center_xyz: [f32; 3],
    pub panel_width_m: f32,
    pub panel_height_m: f32,
    pub max_slide_m: f32,
    pub emissive: EmissiveSurfaceSpec,
}

impl Default for KineticSplitLed {
    fn default() -> Self {
        Self {
            id: "split_led".into(),
            center_xyz: [0.0, -2.0, 2.4],
            panel_width_m: 3.0,
            panel_height_m: 4.0,
            max_slide_m: 2.5,
            emissive: EmissiveSurfaceSpec {
                width_m: 3.0,
                height_m: 4.0,
                nits: 1400.0,
                color_rgb: [255, 120, 64],
                video_slot: "led_split_01".into(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KineticRunway {
    pub id: String,
    pub origin_xyz: [f32; 3],
    pub length_m: f32,
    pub width_m: f32,
    pub max_elev_m: f32,
}

impl Default for KineticRunway {
    fn default() -> Self {
        Self {
            id: "runway_center".into(),
            origin_xyz: [0.0, 3.0, 0.0],
            length_m: 8.0,
            width_m: 2.4,
            max_elev_m: 1.8,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogisticsRack {
    pub id: String,
    pub origin_xyz: [f32; 3],
    pub bays: u32,
    pub levels: u32,
    pub bay_width_m: f32,
    pub depth_m: f32,
    pub level_height_m: f32,
}

impl Default for LogisticsRack {
    fn default() -> Self {
        Self {
            id: "rack_a01".into(),
            origin_xyz: [5.0, 2.0, 0.0],
            bays: 3,
            levels: 4,
            bay_width_m: 1.4,
            depth_m: 1.0,
            level_height_m: 1.2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StagePropAssembly {
    pub name: String,
    pub moving_heads: Vec<MovingHeadFixture>,
    pub curved_walls: Vec<CurvedLedWall>,
    pub split_leds: Vec<KineticSplitLed>,
    pub runways: Vec<KineticRunway>,
    pub racks: Vec<LogisticsRack>,
}

impl Default for StagePropAssembly {
    fn default() -> Self {
        Self {
            name: "stage_props_v18".into(),
            moving_heads: vec![
                MovingHeadFixture::standard("mh_foh_01", [-4.0, -2.0, 10.5]),
                MovingHeadFixture::standard("mh_foh_02", [4.0, -2.0, 10.5]),
            ],
            curved_walls: vec![CurvedLedWall::default()],
            split_leds: vec![KineticSplitLed::default()],
            runways: vec![KineticRunway::default()],
            racks: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StagePropCadBuild {
    pub urdf_xml: String,
    pub hazard_obbs: Vec<VenueHazardObb>,
    pub emissive_surfaces: Vec<EmissiveSurfaceSpec>,
    pub joint_names: Vec<String>,
    pub roundtrip_max_err_m: f32,
}

fn sanitize_id(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    if cleaned.is_empty() {
        "prop".into()
    } else {
        cleaned
    }
}

/// Builds a headless SolidCraft stage-prop URDF, verifies `< 1e-4 m` round-trip fidelity via `rig::urdf::parse`,
/// and derives automatic `VenueHazardObb` envelopes for every kinetic moving assembly.
pub fn build_stage_prop_urdf(assembly: &StagePropAssembly) -> Result<StagePropCadBuild, String> {
    let mut xml = String::with_capacity(4096);
    let name = sanitize_id(&assembly.name);
    xml.push_str(&format!(
        r#"<?xml version="1.0"?>
<!-- Exported by SolidCraft Stage Prop CAD v1.8; units: m, kg, rad -->
<robot name="{name}">
  <link name="stage_prop_root">
    <inertial><origin xyz="0 0 0" rpy="0 0 0"/><mass value="100.0"/></inertial>
  </link>
"#
    ));

    let mut hazard_obbs = Vec::new();
    let mut emissive_surfaces = Vec::new();
    let mut expected_frames: Vec<(String, [f32; 3])> = Vec::new();
    let mut expected_limits: Vec<(String, f32)> = Vec::new();

    for mh in &assembly.moving_heads {
        if !mh.mount_xyz.iter().all(|v| v.is_finite()) {
            return Err(format!("moving head '{}' has non-finite mount_xyz", mh.id));
        }
        let id = sanitize_id(&mh.id);
        let base_link = format!("{id}_base");
        let yoke_link = format!("{id}_yoke");
        let head_link = format!("{id}_head");
        let pan_joint = format!("{id}_pan_revolute");
        let tilt_joint = format!("{id}_tilt_revolute");
        xml.push_str(&format!(
            r#"  <link name="{base_link}"><inertial><origin xyz="0 0 0" rpy="0 0 0"/><mass value="18.0"/></inertial></link>
  <link name="{yoke_link}"><inertial><origin xyz="0 0 -0.15" rpy="0 0 0"/><mass value="6.0"/></inertial></link>
  <link name="{head_link}"><inertial><origin xyz="0 0 -0.30" rpy="0 0 0"/><mass value="8.0"/></inertial></link>
  <joint name="{id}_mount_fixed" type="fixed">
    <parent link="stage_prop_root"/><child link="{base_link}"/>
    <origin xyz="{:.5} {:.5} {:.5}" rpy="0 0 0"/>
  </joint>
  <joint name="{pan_joint}" type="revolute">
    <parent link="{base_link}"/><child link="{yoke_link}"/>
    <origin xyz="0 0 0" rpy="0 0 0"/><axis xyz="0 0 1"/>
    <limit lower="{:.5}" upper="{:.5}" effort="120" velocity="4.5"/>
  </joint>
  <joint name="{tilt_joint}" type="revolute">
    <parent link="{yoke_link}"/><child link="{head_link}"/>
    <origin xyz="0 0 -0.20000" rpy="0 0 0"/><axis xyz="0 1 0"/>
    <limit lower="{:.5}" upper="{:.5}" effort="90" velocity="4.5"/>
  </joint>
"#,
            mh.mount_xyz[0],
            mh.mount_xyz[1],
            mh.mount_xyz[2],
            mh.pan_limits_rad[0],
            mh.pan_limits_rad[1],
            mh.tilt_limits_rad[0],
            mh.tilt_limits_rad[1],
        ));
        expected_frames.push((base_link, mh.mount_xyz));
        expected_limits.push((pan_joint, mh.pan_limits_rad[1]));
        expected_limits.push((tilt_joint, mh.tilt_limits_rad[1]));
    }

    for wall in &assembly.curved_walls {
        if !wall.radius_m.is_finite() || wall.radius_m <= 0.5 || !wall.height_m.is_finite() || wall.height_m <= 0.5 {
            return Err(format!("curved wall '{}' invalid radius/height", wall.id));
        }
        let id = sanitize_id(&wall.id);
        let link = format!("{id}_panel");
        xml.push_str(&format!(
            r#"  <link name="{link}"><inertial><origin xyz="0 0 0" rpy="0 0 0"/><mass value="650.0"/></inertial></link>
  <joint name="{id}_fixed" type="fixed">
    <parent link="stage_prop_root"/><child link="{link}"/>
    <origin xyz="{:.5} {:.5} {:.5}" rpy="0 0 0"/>
  </joint>
"#,
            wall.center_xyz[0], wall.center_xyz[1], wall.center_xyz[2]
        ));
        expected_frames.push((link, wall.center_xyz));
        emissive_surfaces.push(wall.emissive.clone());
    }

    for split in &assembly.split_leds {
        if !split.max_slide_m.is_finite() || split.max_slide_m <= 0.0 || split.panel_width_m <= 0.2 {
            return Err(format!("kinetic split LED '{}' invalid stroke/width", split.id));
        }
        let id = sanitize_id(&split.id);
        let l_link = format!("{id}_left_panel");
        let r_link = format!("{id}_right_panel");
        let l_joint = format!("{id}_left_prismatic");
        let r_joint = format!("{id}_right_prismatic");
        let half_w = split.panel_width_m * 0.5;
        xml.push_str(&format!(
            r#"  <link name="{l_link}"><inertial><origin xyz="0 0 0" rpy="0 0 0"/><mass value="220.0"/></inertial></link>
  <link name="{r_link}"><inertial><origin xyz="0 0 0" rpy="0 0 0"/><mass value="220.0"/></inertial></link>
  <joint name="{l_joint}" type="prismatic">
    <parent link="stage_prop_root"/><child link="{l_link}"/>
    <origin xyz="{:.5} {:.5} {:.5}" rpy="0 0 0"/><axis xyz="-1 0 0"/>
    <limit lower="0.00000" upper="{:.5}" effort="6000" velocity="0.6"/>
  </joint>
  <joint name="{r_joint}" type="prismatic">
    <parent link="stage_prop_root"/><child link="{r_link}"/>
    <origin xyz="{:.5} {:.5} {:.5}" rpy="0 0 0"/><axis xyz="1 0 0"/>
    <limit lower="0.00000" upper="{:.5}" effort="6000" velocity="0.6"/>
  </joint>
"#,
            split.center_xyz[0] - half_w,
            split.center_xyz[1],
            split.center_xyz[2],
            split.max_slide_m,
            split.center_xyz[0] + half_w,
            split.center_xyz[1],
            split.center_xyz[2],
            split.max_slide_m,
        ));
        expected_limits.push((l_joint, split.max_slide_m));
        expected_limits.push((r_joint, split.max_slide_m));
        emissive_surfaces.push(split.emissive.clone());
        let span_x = split.panel_width_m + split.max_slide_m + 0.35;
        let half_h = split.panel_height_m * 0.5;
        hazard_obbs.push(VenueHazardObb {
            name: format!("OBB-SplitLed-{id}"),
            min_xyz: [
                split.center_xyz[0] - span_x,
                split.center_xyz[1] - 0.50,
                (split.center_xyz[2] - half_h).max(0.0),
            ],
            max_xyz: [
                split.center_xyz[0] + span_x,
                split.center_xyz[1] + 0.50,
                split.center_xyz[2] + half_h,
            ],
        });
    }

    for rw in &assembly.runways {
        if !rw.max_elev_m.is_finite() || rw.max_elev_m <= 0.0 || rw.length_m <= 0.5 || rw.width_m <= 0.5 {
            return Err(format!("kinetic runway '{}' invalid geometry", rw.id));
        }
        let id = sanitize_id(&rw.id);
        let link = format!("{id}_deck");
        let joint = format!("{id}_elev_prismatic");
        xml.push_str(&format!(
            r#"  <link name="{link}"><inertial><origin xyz="0 0 0" rpy="0 0 0"/><mass value="900.0"/></inertial></link>
  <joint name="{joint}" type="prismatic">
    <parent link="stage_prop_root"/><child link="{link}"/>
    <origin xyz="{:.5} {:.5} {:.5}" rpy="0 0 0"/><axis xyz="0 0 1"/>
    <limit lower="0.00000" upper="{:.5}" effort="14000" velocity="0.5"/>
  </joint>
"#,
            rw.origin_xyz[0], rw.origin_xyz[1], rw.origin_xyz[2], rw.max_elev_m,
        ));
        expected_frames.push((link, rw.origin_xyz));
        expected_limits.push((joint, rw.max_elev_m));
        hazard_obbs.push(VenueHazardObb {
            name: format!("OBB-Runway-{id}"),
            min_xyz: [
                rw.origin_xyz[0] - rw.width_m * 0.5 - 0.25,
                rw.origin_xyz[1] - rw.length_m * 0.5 - 0.25,
                rw.origin_xyz[2],
            ],
            max_xyz: [
                rw.origin_xyz[0] + rw.width_m * 0.5 + 0.25,
                rw.origin_xyz[1] + rw.length_m * 0.5 + 0.25,
                rw.origin_xyz[2] + rw.max_elev_m,
            ],
        });
    }

    for rack in &assembly.racks {
        if rack.bays == 0 || rack.levels == 0 || rack.bay_width_m <= 0.2 || rack.level_height_m <= 0.2 {
            return Err(format!("logistics rack '{}' invalid dimensions", rack.id));
        }
        let id = sanitize_id(&rack.id);
        let link = format!("{id}_frame");
        xml.push_str(&format!(
            r#"  <link name="{link}"><inertial><origin xyz="0 0 0" rpy="0 0 0"/><mass value="{:.1}"/></inertial></link>
  <joint name="{id}_fixed" type="fixed">
    <parent link="stage_prop_root"/><child link="{link}"/>
    <origin xyz="{:.5} {:.5} {:.5}" rpy="0 0 0"/>
  </joint>
"#,
            (rack.bays * rack.levels) as f32 * 45.0,
            rack.origin_xyz[0],
            rack.origin_xyz[1],
            rack.origin_xyz[2],
        ));
        expected_frames.push((link, rack.origin_xyz));
        let w = rack.bays as f32 * rack.bay_width_m;
        let h = rack.levels as f32 * rack.level_height_m;
        hazard_obbs.push(VenueHazardObb {
            name: format!("OBB-Rack-{id}"),
            min_xyz: [
                rack.origin_xyz[0] - w * 0.5,
                rack.origin_xyz[1] - rack.depth_m * 0.5,
                rack.origin_xyz[2],
            ],
            max_xyz: [
                rack.origin_xyz[0] + w * 0.5,
                rack.origin_xyz[1] + rack.depth_m * 0.5,
                rack.origin_xyz[2] + h,
            ],
        });
    }

    xml.push_str("</robot>");

    let parsed = urdf::parse(&xml)?;
    let frames = parsed.world_frames();
    let mut max_err = 0.0f32;
    for (link, want) in &expected_frames {
        let got = frames
            .get(link)
            .ok_or_else(|| format!("roundtrip missing link '{link}'"))?
            .t;
        for ax in 0..3 {
            max_err = max_err.max((got[ax] - want[ax]).abs());
        }
    }
    for (jname, want_upper) in &expected_limits {
        let got_upper = parsed
            .joints
            .iter()
            .find(|j| &j.name == jname)
            .and_then(|j| j.limit)
            .ok_or_else(|| format!("roundtrip missing joint limit '{jname}'"))?[1];
        max_err = max_err.max((got_upper - want_upper).abs());
    }
    if max_err >= 1e-4 {
        return Err(format!("stage_cad URDF roundtrip error {max_err:.6} m >= 1e-4 m"));
    }

    let joint_names = parsed.joints.iter().map(|j| j.name.clone()).collect();
    Ok(StagePropCadBuild {
        urdf_xml: xml,
        hazard_obbs,
        emissive_surfaces,
        joint_names,
        roundtrip_max_err_m: max_err,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_props_urdf_roundtrips_under_one_tenth_mm_and_emits_hazard_obbs() {
        let mut assembly = StagePropAssembly::default();
        assembly.racks.push(LogisticsRack::default());
        let build = build_stage_prop_urdf(&assembly).unwrap();
        assert!(build.roundtrip_max_err_m < 1e-4, "err={}", build.roundtrip_max_err_m);
        assert!(build.joint_names.iter().any(|j| j == "mh_foh_01_pan_revolute"));
        assert!(build.joint_names.iter().any(|j| j == "mh_foh_01_tilt_revolute"));
        assert!(build.joint_names.iter().any(|j| j == "split_led_left_prismatic"));
        assert!(build.joint_names.iter().any(|j| j == "runway_center_elev_prismatic"));
        assert!(build.hazard_obbs.iter().any(|o| o.name == "OBB-SplitLed-split_led"));
        assert!(build.hazard_obbs.iter().any(|o| o.name == "OBB-Runway-runway_center"));
        assert!(build.hazard_obbs.iter().any(|o| o.name == "OBB-Rack-rack_a01"));
        assert_eq!(build.emissive_surfaces.len(), 2);
    }
}
