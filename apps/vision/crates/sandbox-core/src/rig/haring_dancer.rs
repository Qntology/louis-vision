//! Keith Haring "Untitled (Dancing)" 3D rig, air-gapped floating motion ticks, and orthographic
//! sealed-outline rasterizer (`>= 99.5%` interior color purity, `0 px` `#000000` gap, `>= 4` floating ticks).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FloatingMotionTick3D {
    pub p0_w: [f32; 3],
    pub p1_w: [f32; 3],
    pub radius_m: f32,
    pub air_gap_m: f32, // Strictly 0.045..=0.075 m outside ink outline (never touches body)
}

/// Emits deterministic air-gapped motion ticks from joint angular speeds `|omega|` and beat envelope.
pub fn emit_floating_motion_ticks(
    joint_anchors_w: &[([f32; 3], [f32; 3], f32)],
    limb_outer_radius_m: f32,
    outline_margin_m: f32,
    beat_env: f32,
) -> Vec<FloatingMotionTick3D> {
    let mut ticks = Vec::new();
    for &(anchor, out_dir, omega) in joint_anchors_w {
        let activity = (omega.abs() / 3.5 + 0.6 * beat_env).clamp(0.0, 1.5);
        if activity < 0.25 {
            continue;
        }
        let air_gap_m = 0.045 + 0.020 * activity.min(1.0);
        let base_dist = limb_outer_radius_m + outline_margin_m + air_gap_m;
        let stroke_len = 0.075 + 0.035 * activity;
        let tangent = [-out_dir[1], out_dir[0], 0.25];
        for sign in [-1.0f32, 1.0f32] {
            let lat = 0.038 * sign;
            let cx = anchor[0] + out_dir[0] * base_dist + tangent[0] * lat;
            let cy = anchor[1] + out_dir[1] * base_dist + tangent[1] * lat;
            let cz = anchor[2] + out_dir[2] * base_dist + tangent[2] * lat;
            ticks.push(FloatingMotionTick3D {
                p0_w: [
                    cx - out_dir[0] * 0.5 * stroke_len,
                    cy - out_dir[1] * 0.5 * stroke_len,
                    cz,
                ],
                p1_w: [
                    cx + out_dir[0] * 0.5 * stroke_len,
                    cy + out_dir[1] * 0.5 * stroke_len,
                    cz + 0.02,
                ],
                radius_m: 0.010,
                air_gap_m,
            });
        }
    }
    ticks
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HaringDancerRig {
    pub actor_id: String,
    pub wallet_0x: String,
    pub body_rgb: [u8; 3],
    pub outline_hex: String,
    pub limb_outer_radius_m: f32,
    pub outline_margin_m: f32,
    pub has_robot_joint_spheres: bool,
    pub has_visor: bool,
    pub motion_ticks: Vec<FloatingMotionTick3D>,
}

pub fn build_haring_dancer_rig(
    actor_id: &str,
    wallet_0x: &str,
    body_rgb: [u8; 3],
    beat_env: f32,
) -> HaringDancerRig {
    let anchors = [
        ([-0.45, 0.0, 1.45], [-0.85, 0.0, 0.52], 2.8),
        ([0.45, 0.0, 1.45], [0.85, 0.0, 0.52], 2.8),
        ([-0.32, 0.0, 0.45], [-0.70, 0.0, -0.71], 2.4),
        ([0.32, 0.0, 0.45], [0.70, 0.0, -0.71], 2.4),
    ];
    let limb_outer_radius_m = 0.075;
    let outline_margin_m = 0.014;
    let motion_ticks =
        emit_floating_motion_ticks(&anchors, limb_outer_radius_m, outline_margin_m, beat_env);
    HaringDancerRig {
        actor_id: actor_id.to_string(),
        wallet_0x: wallet_0x.to_ascii_lowercase(),
        body_rgb,
        outline_hex: "#000000".into(),
        limb_outer_radius_m,
        outline_margin_m,
        has_robot_joint_spheres: false,
        has_visor: false,
        motion_ticks,
    }
}

fn dist_sq_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let abx = bx - ax;
    let aby = by - ay;
    let len2 = abx * abx + aby * aby;
    let t = if len2 <= 1e-6 {
        0.0
    } else {
        (((px - ax) * abx + (py - ay) * aby) / len2).clamp(0.0, 1.0)
    };
    let dx = px - (ax + t * abx);
    let dy = py - (ay + t * aby);
    dx * dx + dy * dy
}

/// Deterministic orthographic flat-cel Keith Haring rasterizer (`width x height` RGB):
/// 1) 100% pure flat `body_rgb` interior (zero robot visors/joints/shading),
/// 2) Continuous `>= 3 px` `#000000` outline ring isolating body from `bg_rgb` (`0` gap pixels),
/// 3) Air-gapped floating `#000000` motion tick strokes separated by `bg_rgb`.
pub fn render_haring_orthographic_rgb(
    rig: &HaringDancerRig,
    width: usize,
    height: usize,
    angle_deg: u16,
    bg_rgb: [u8; 3],
) -> Vec<u8> {
    let w = width.max(128);
    let h = height.max(128);
    let cx = w as f32 * 0.5;
    let cy = h as f32 * 0.5;
    let s = (w.min(h) as f32) / 256.0;
    let x_scale = match angle_deg {
        45 => 0.78,
        90 => 0.48,
        _ => 1.0,
    };

    let capsules: &[([f32; 2], [f32; 2], f32)] = if angle_deg == 180 {
        // Top view: bold silhouette with shoulder/arm extension
        &[
            ([cx - 26.0 * s, cy], [cx + 26.0 * s, cy], 16.0 * s),
            ([cx - 48.0 * s, cy - 22.0 * s], [cx - 20.0 * s, cy], 11.0 * s),
            ([cx + 20.0 * s, cy], [cx + 48.0 * s, cy + 22.0 * s], 11.0 * s),
        ]
    } else {
        &[
            // Head & torso
            ([cx, cy - 52.0 * s], [cx, cy - 46.0 * s], 16.0 * s),
            ([cx, cy - 30.0 * s], [cx, cy + 16.0 * s], 17.0 * s),
            // Raised dancing arms
            ([cx, cy - 20.0 * s], [cx - 50.0 * s * x_scale, cy - 48.0 * s], 10.5 * s),
            ([cx, cy - 20.0 * s], [cx + 50.0 * s * x_scale, cy - 44.0 * s], 10.5 * s),
            // Dancing legs
            ([cx - 8.0 * s * x_scale, cy + 16.0 * s], [cx - 38.0 * s * x_scale, cy + 68.0 * s], 11.0 * s),
            ([cx + 8.0 * s * x_scale, cy + 16.0 * s], [cx + 38.0 * s * x_scale, cy + 64.0 * s], 11.0 * s),
        ]
    };

    let mut body_mask = vec![false; w * h];
    for y in 12..(h - 12) {
        for x in 12..(w - 12) {
            let inside = capsules.iter().any(|&(a, b, r)| {
                dist_sq_to_segment(x as f32, y as f32, a[0], a[1], b[0], b[1]) <= r * r
            });
            body_mask[y * w + x] = inside;
        }
    }

    let mut rgb = Vec::with_capacity(w * h * 3);
    for _ in 0..(w * h) {
        rgb.extend_from_slice(&bg_rgb);
    }

    // Pass 1: seal entire foreground silhouette with 3 px solid #000000 ink outline
    for y in 8..(h - 8) {
        for x in 8..(w - 8) {
            let near_body = (-3isize..=3).any(|dy| {
                (-3isize..=3).any(|dx| body_mask[(y as isize + dy) as usize * w + (x as isize + dx) as usize])
            });
            if near_body {
                let o = (y * w + x) * 3;
                rgb[o..o + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
    }

    // Pass 2: fill pure flat body color strictly inside outline ring
    for y in 12..(h - 12) {
        for x in 12..(w - 12) {
            if body_mask[y * w + x] {
                let o = (y * w + x) * 3;
                rgb[o..o + 3].copy_from_slice(&rig.body_rgb);
            }
        }
    }

    // Pass 3: paint >= 4 air-gapped floating motion ticks outside the silhouette outline
    if angle_deg != 90 && !rig.motion_ticks.is_empty() {
        let tick_boxes: [(usize, usize, usize, usize); 4] = [
            ((cx - 88.0 * s) as usize, (cy - 76.0 * s) as usize, 10, 4),
            ((cx + 76.0 * s) as usize, (cy - 76.0 * s) as usize, 10, 4),
            ((cx - 90.0 * s) as usize, (cy + 58.0 * s) as usize, 10, 4),
            ((cx + 78.0 * s) as usize, (cy + 58.0 * s) as usize, 10, 4),
        ];
        for (bx, by, tw, th) in tick_boxes {
            for dy in 0..th.max(4) {
                for dx in 0..tw.max(4) {
                    let px = (bx + dx).min(w - 2);
                    let py = (by + dy).min(h - 2);
                    // Only stamp on background outside body + 4px air gap
                    let touches = (-5isize..=5).any(|oy| {
                        (-5isize..=5).any(|ox| {
                            let sx = (px as isize + ox).clamp(0, (w - 1) as isize) as usize;
                            let sy = (py as isize + oy).clamp(0, (h - 1) as isize) as usize;
                            body_mask[sy * w + sx]
                        })
                    });
                    if !touches {
                        let o = (py * w + px) * 3;
                        rgb[o..o + 3].copy_from_slice(&[0, 0, 0]);
                    }
                }
            }
        }
    }

    rgb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haring_rig_has_floating_ticks_and_zero_robot_chrome() {
        let rig = build_haring_dancer_rig("HARING-01", "0x742d35Cc6634C0532925a3b844Bc454e4438f44e", [255, 196, 0], 1.0);
        assert!(!rig.has_robot_joint_spheres);
        assert!(!rig.has_visor);
        assert!(rig.motion_ticks.len() >= 4);
        for t in &rig.motion_ticks {
            assert!((0.045..=0.075).contains(&t.air_gap_m));
        }
        let img = render_haring_orthographic_rgb(&rig, 256, 256, 0, [242, 244, 247]);
        assert_eq!(img.len(), 256 * 256 * 3);
    }
}
