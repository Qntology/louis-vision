//! Procedural SolidCraft Concert Stage & Rigging URDF builder (`venue_build`).

use serde::{Deserialize, Serialize};

use super::urdf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StageVenueSpec {
    pub venue_name: String,
    pub max_lift_stroke_m: f32,
    pub max_lift_payload_kg: u32,
    pub mothergrid_elev_m: f32,
    pub servo_hoist_count: u32,
    pub deck_width_m: f32,
    pub deck_depth_m: f32,
    pub turntable_radius_m: f32,
}

impl Default for StageVenueSpec {
    fn default() -> Self {
        Self {
            venue_name: "xr_arena_stage_2026".into(),
            max_lift_stroke_m: 2.50,
            max_lift_payload_kg: 500,
            mothergrid_elev_m: 14.0,
            servo_hoist_count: 16,
            deck_width_m: 16.0,
            deck_depth_m: 12.0,
            turntable_radius_m: 3.5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VenueHazardObb {
    pub name: String,
    pub min_xyz: [f32; 3],
    pub max_xyz: [f32; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VenueCadBuild {
    pub urdf_xml: String,
    pub obb_lift_c: VenueHazardObb,
    pub mothergrid_hoists: Vec<[f32; 3]>,
    pub roundtrip_max_err_m: f32,
}

pub fn build_stage_venue_urdf(spec: &StageVenueSpec) -> Result<VenueCadBuild, String> {
    if !spec.max_lift_stroke_m.is_finite() || spec.max_lift_stroke_m <= 0.0 {
        return Err("max_lift_stroke_m must be finite and > 0".into());
    }
    if spec.max_lift_payload_kg == 0 {
        return Err("max_lift_payload_kg must be > 0".into());
    }
    if !spec.mothergrid_elev_m.is_finite() || spec.mothergrid_elev_m <= spec.max_lift_stroke_m {
        return Err("mothergrid_elev_m must exceed max_lift_stroke_m".into());
    }
    if spec.servo_hoist_count < 4 {
        return Err("servo_hoist_count must be >= 4".into());
    }

    let urdf_xml = format!(
        r#"<?xml version="1.0"?>
<!-- Exported by SolidCraft Stage CAD; units: m, kg, rad -->
<robot name="{name}">
  <link name="base_link">
    <inertial><origin xyz="0 0 0" rpy="0 0 0"/><mass value="2500.0"/></inertial>
  </link>
  <link name="stage_deck">
    <inertial><origin xyz="0 0 0.5" rpy="0 0 0"/><mass value="1800.0"/></inertial>
  </link>
  <link name="center_spiral_lift">
    <inertial><origin xyz="0 0 0" rpy="0 0 0"/><mass value="{payload:.1}"/></inertial>
  </link>
  <link name="center_turntable">
    <inertial><origin xyz="0 0 0.1" rpy="0 0 0"/><mass value="450.0"/></inertial>
  </link>
  <link name="mothergrid_truss">
    <inertial><origin xyz="0 0 0" rpy="0 0 0"/><mass value="3200.0"/></inertial>
  </link>
  <joint name="deck_fixed" type="fixed">
    <parent link="base_link"/>
    <child link="stage_deck"/>
    <origin xyz="0 0 0" rpy="0 0 0"/>
  </joint>
  <joint name="sl01_lift_prismatic" type="prismatic">
    <parent link="stage_deck"/>
    <child link="center_spiral_lift"/>
    <origin xyz="0 0 0" rpy="0 0 0"/>
    <axis xyz="0 0 1"/>
    <limit lower="0.00000" upper="{stroke:.5}" effort="15000" velocity="0.8"/>
  </joint>
  <joint name="stage_turntable_revolute" type="revolute">
    <parent link="stage_deck"/>
    <child link="center_turntable"/>
    <origin xyz="0 0 1.00000" rpy="0 0 0"/>
    <axis xyz="0 0 1"/>
    <limit lower="-3.14159" upper="3.14159" effort="8000" velocity="1.2"/>
  </joint>
  <joint name="mothergrid_fixed" type="fixed">
    <parent link="base_link"/>
    <child link="mothergrid_truss"/>
    <origin xyz="0 0 {elev:.5}" rpy="0 0 0"/>
  </joint>
</robot>"#,
        name = spec.venue_name,
        payload = spec.max_lift_payload_kg as f32,
        stroke = spec.max_lift_stroke_m,
        elev = spec.mothergrid_elev_m,
    );

    let parsed = urdf::parse(&urdf_xml)?;
    let frames = parsed.world_frames();
    let grid_z = frames
        .get("mothergrid_truss")
        .ok_or_else(|| "missing mothergrid_truss frame".to_string())?
        .t[2];
    let lift_upper = parsed
        .joints
        .iter()
        .find(|j| j.name == "sl01_lift_prismatic")
        .and_then(|j| j.limit)
        .ok_or_else(|| "missing sl01_lift_prismatic limit".to_string())?[1];
    let roundtrip_max_err_m = (grid_z - spec.mothergrid_elev_m)
        .abs()
        .max((lift_upper - spec.max_lift_stroke_m).abs());

    let mut mothergrid_hoists = Vec::with_capacity(spec.servo_hoist_count as usize);
    let cols = 4usize.max((spec.servo_hoist_count as usize) / 4);
    for i in 0..spec.servo_hoist_count as usize {
        let r = i / cols;
        let c = i % cols;
        let x = -0.4 * spec.deck_width_m + (c as f32 / (cols.max(2) - 1) as f32) * 0.8 * spec.deck_width_m;
        let rows_tot = ((spec.servo_hoist_count as usize) + cols - 1) / cols;
        let y = -0.4 * spec.deck_depth_m + (r as f32 / (rows_tot.max(2) - 1) as f32) * 0.8 * spec.deck_depth_m;
        mothergrid_hoists.push([x, y, spec.mothergrid_elev_m]);
    }

    let obb_lift_c = VenueHazardObb {
        name: "OBB-Lift-C".into(),
        min_xyz: [-1.20, -1.20, 0.0],
        max_xyz: [1.20, 1.20, spec.max_lift_stroke_m],
    };

    Ok(VenueCadBuild {
        urdf_xml,
        obb_lift_c,
        mothergrid_hoists,
        roundtrip_max_err_m,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_venue_urdf_roundtrips_under_one_tenth_mm() {
        let spec = StageVenueSpec::default();
        let build = build_stage_venue_urdf(&spec).unwrap();
        assert!(build.roundtrip_max_err_m < 1e-4, "err={}", build.roundtrip_max_err_m);
        assert_eq!(build.mothergrid_hoists.len(), 16);
        assert_eq!(build.obb_lift_c.name, "OBB-Lift-C");
        assert!((build.obb_lift_c.max_xyz[2] - 2.50).abs() < 1e-5);
    }
}
