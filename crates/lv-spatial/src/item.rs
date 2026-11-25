//! 3D oriented bounding box (`ItemPose`: yaw + L/W/H dimensions) from stereo depth inside 2D box.

use serde::{Deserialize, Serialize};

use crate::camera::{Intrinsics, Pose};
use crate::frame::DepthMap;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemPose {
    pub yaw_deg: f32,
    pub rel_yaw_deg: f32,
    pub length_m: f32,
    pub width_m: f32,
    pub height_m: f32,
    pub centroid: [f32; 3],
    pub points: usize,
}

pub fn item_pose(
    depth: &DepthMap,
    intr: &Intrinsics,
    pose: &Pose,
    bbox: [f32; 4],
    ground_m: f32,
) -> Option<ItemPose> {
    let (w, h) = (intr.width.max(1) as f32, intr.height.max(1) as f32);
    let (sx, sy) = (depth.width as f32 / w, depth.height as f32 / h);
    let x0 = ((bbox[0] * w * sx).floor().max(0.0)) as u32;
    let x1 = ((bbox[2] * w * sx).ceil() as u32).min(depth.width.saturating_sub(1));
    let y0 = ((bbox[1] * h * sy).floor().max(0.0)) as u32;
    let y1 = ((bbox[3] * h * sy).ceil() as u32).min(depth.height.saturating_sub(1));

    let mut pts: Vec<[f32; 3]> = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = depth.at(x, y);
            if !d.is_finite() || d <= 0.0 || d > 25.0 {
                continue;
            }
            let px = (x as f32 + 0.5) / sx.max(1e-6);
            let py = (y as f32 + 0.5) / sy.max(1e-6);
            let r = intr.ray(pose.yaw, px, py);
            let wx = pose.x + d * r[0];
            let wy = pose.eye_m + d * r[1];
            let wz = pose.z + d * r[2];
            if wy > ground_m + 0.02 {
                pts.push([wx, wy, wz]);
            }
        }
    }
    if pts.len() < 8 {
        return None;
    }

    let n = pts.len() as f32;
    let mx = pts.iter().map(|p| p[0]).sum::<f32>() / n;
    let my = pts.iter().map(|p| p[1]).sum::<f32>() / n;
    let mz = pts.iter().map(|p| p[2]).sum::<f32>() / n;

    let (mut cxx, mut cxz, mut czz) = (0.0f64, 0.0f64, 0.0f64);
    for p in &pts {
        let dx = (p[0] - mx) as f64;
        let dz = (p[2] - mz) as f64;
        cxx += dx * dx;
        cxz += dx * dz;
        czz += dz * dz;
    }
    let angle = 0.5 * (2.0 * cxz).atan2(cxx - czz);
    let (ca, sa) = (angle.cos() as f32, angle.sin() as f32);

    let (mut u_min, mut u_max) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut v_min, mut v_max) = (f32::INFINITY, f32::NEG_INFINITY);
    let mut h_max = 0.0f32;
    for p in &pts {
        let dx = p[0] - mx;
        let dz = p[2] - mz;
        let u = dx * ca + dz * sa;
        let v = -dx * sa + dz * ca;
        u_min = u_min.min(u);
        u_max = u_max.max(u);
        v_min = v_min.min(v);
        v_max = v_max.max(v);
        h_max = h_max.max(p[1] - ground_m);
    }

    let su = (u_max - u_min).max(0.0);
    let sv = (v_max - v_min).max(0.0);
    let (yaw_rad, length_m, width_m) = if su >= sv {
        (angle as f32, su, sv)
    } else {
        (angle as f32 + std::f32::consts::FRAC_PI_2, sv, su)
    };
    let mut yaw_deg = yaw_rad.to_degrees() % 180.0;
    if yaw_deg > 90.0 {
        yaw_deg -= 180.0;
    } else if yaw_deg <= -90.0 {
        yaw_deg += 180.0;
    }
    let mut rel_yaw_deg = yaw_deg - pose.yaw.to_degrees();
    while rel_yaw_deg > 90.0 {
        rel_yaw_deg -= 180.0;
    }
    while rel_yaw_deg <= -90.0 {
        rel_yaw_deg += 180.0;
    }

    Some(ItemPose {
        yaw_deg,
        rel_yaw_deg,
        length_m,
        width_m,
        height_m: h_max,
        centroid: [mx, my, mz],
        points: pts.len(),
    })
}
