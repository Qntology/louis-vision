//! Camera intrinsics, pose, ray unprojection, and vertical FOV / look-down pitch geometry.

use serde::{Deserialize, Serialize};
use crate::fit3::V3;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Intrinsics {
    pub width: u32,
    pub height: u32,
    pub fov_deg: f32,
    pub pitch_deg: f32,
    #[serde(default)]
    pub roll_deg: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
    pub eye_m: f32,
}

impl Pose {
    pub fn at_pos(x: f32, eye_m: f32, z: f32) -> Self {
        Self { x, z, yaw: 0.0, eye_m }
    }
}

fn basis(yaw: f32, pitch: f32, roll: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let (fwd, right, up) = ([cy * cp, sp, sy * cp], [-sy, 0.0, cy], [-cy * sp, cp, -sy * sp]);
    if roll == 0.0 {
        return (fwd, right, up);
    }
    let (sr, cr) = roll.sin_cos();
    let r = [right[0] * cr - up[0] * sr, right[1] * cr - up[1] * sr, right[2] * cr - up[2] * sr];
    let u = [up[0] * cr + right[0] * sr, up[1] * cr + right[1] * sr, up[2] * cr + right[2] * sr];
    (fwd, r, u)
}

pub fn wrap_angle(a: f32) -> f32 {
    let mut a = a % std::f32::consts::TAU;
    if a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    } else if a < -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}

impl Intrinsics {
    pub fn pinhole(width: u32, height: u32, fov_deg: f32) -> Self {
        Self {
            width,
            height,
            fov_deg,
            pitch_deg: 0.0,
            roll_deg: 0.0,
        }
    }

    fn screen(&self, px: f32, py: f32) -> (f32, f32) {
        let (w, h) = (self.width.max(1) as f32, self.height.max(1) as f32);
        let half_w = (self.fov_deg.to_radians() * 0.5).tan();
        let half_h = half_w * h / w;
        ((2.0 * px / w - 1.0) * half_w, (1.0 - 2.0 * py / h) * half_h)
    }

    fn axes(&self, yaw: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
        basis(yaw, self.pitch_deg.to_radians(), self.roll_deg.to_radians())
    }

    #[inline]
    pub fn focal_px(&self) -> f64 {
        0.5 * (self.height.max(1) as f64) / (0.5 * (self.vfov_deg() as f64).to_radians()).tan().max(1e-6)
    }

    #[inline]
    pub fn camera_axes(&self, yaw: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
        self.axes(yaw)
    }

    pub fn project_world_unclamped(&self, pose: &Pose, p: V3) -> Option<(f32, f32, f32)> {
        let (fwd, right, up) = self.axes(pose.yaw);
        let d = [
            p[0] - pose.x as f64,
            p[1] - pose.eye_m as f64,
            p[2] - pose.z as f64,
        ];
        let x_c = d[0] * right[0] as f64 + d[1] * right[1] as f64 + d[2] * right[2] as f64;
        let y_c = -(d[0] * up[0] as f64 + d[1] * up[1] as f64 + d[2] * up[2] as f64);
        let z_c = d[0] * fwd[0] as f64 + d[1] * fwd[1] as f64 + d[2] * fwd[2] as f64;
        if !z_c.is_finite() || z_c <= 1e-4 {
            return None;
        }
        let f_px = self.focal_px();
        let px = 0.5 * self.width.max(1) as f64 + f_px * (x_c / z_c);
        let py = 0.5 * self.height.max(1) as f64 + f_px * (y_c / z_c);
        Some((px as f32, py as f32, z_c as f32))
    }

    pub fn project_world(&self, pose: &Pose, p: V3) -> Option<(f32, f32, f32)> {
        let (px, py, z) = self.project_world_unclamped(pose, p)?;
        if px >= 0.0 && px < self.width as f32 && py >= 0.0 && py < self.height as f32 {
            Some((px, py, z))
        } else {
            None
        }
    }

    pub fn unproject_optical(&self, pose: &Pose, px: f32, py: f32, depth_m: f32) -> V3 {
        let (fwd, right, up) = self.axes(pose.yaw);
        let f_px = self.focal_px();
        let x_n = (px as f64 - 0.5 * self.width.max(1) as f64) / f_px;
        let y_n = (py as f64 - 0.5 * self.height.max(1) as f64) / f_px;
        let z = depth_m as f64;
        let x_c = x_n * z;
        let y_c = -y_n * z;
        [
            pose.x as f64 + x_c * right[0] as f64 + y_c * up[0] as f64 + z * fwd[0] as f64,
            pose.eye_m as f64 + x_c * right[1] as f64 + y_c * up[1] as f64 + z * fwd[1] as f64,
            pose.z as f64 + x_c * right[2] as f64 + y_c * up[2] as f64 + z * fwd[2] as f64,
        ]
    }

    pub fn world_up_in_camera(&self) -> [f32; 3] {
        let (fwd, right, up) = self.axes(0.0);
        [right[1], up[1], -fwd[1]]
    }

    pub fn ray(&self, yaw: f32, px: f32, py: f32) -> [f32; 3] {
        let (fwd, right, up) = self.axes(yaw);
        let (sx, sy) = self.screen(px, py);
        [
            fwd[0] + right[0] * sx + up[0] * sy,
            fwd[1] + right[1] * sx + up[1] * sy,
            fwd[2] + right[2] * sx + up[2] * sy,
        ]
    }

    pub fn bearing(&self, pose: &Pose, px: f32, py: f32) -> f32 {
        let r = self.ray(pose.yaw, px, py);
        r[2].atan2(r[0])
    }

    pub fn relative_bearing(&self, pose: &Pose, px: f32, py: f32) -> f32 {
        wrap_angle(self.bearing(pose, px, py) - pose.yaw)
    }

    pub fn ground_range(&self, yaw: f32, px: f32, py: f32, depth_m: f32) -> f32 {
        let r = self.ray(yaw, px, py);
        depth_m * r[0].hypot(r[2])
    }

    pub fn with_pitch(&self, pitch_deg: f32) -> Self {
        Self { pitch_deg, ..*self }
    }

    pub fn vfov_deg(&self) -> f32 {
        let (w, h) = (self.width.max(1) as f32, self.height.max(1) as f32);
        let half_w = (self.fov_deg.to_radians() * 0.5).tan();
        2.0 * (half_w * h / w).atan().to_degrees()
    }

    pub fn look_down_pitch_deg(&self, eye_m: f32, range_m: f32, target_h_m: f32) -> f32 {
        let dz = (eye_m - target_h_m).max(0.05);
        let direct_deg = -dz.atan2(range_m.max(0.15)).to_degrees();
        let half_vfov = self.vfov_deg() * 0.5;
        let min_needed = direct_deg + half_vfov * 0.65;
        if self.pitch_deg > min_needed {
            direct_deg.clamp(-65.0, self.pitch_deg)
        } else {
            self.pitch_deg
        }
    }

    pub fn is_ground_target_visible(&self, eye_m: f32, range_m: f32, target_h_m: f32) -> bool {
        let dz = (eye_m - target_h_m).max(0.0);
        let target_pitch_deg = -dz.atan2(range_m.max(0.05)).to_degrees();
        let half_vfov = self.vfov_deg() * 0.5;
        let bottom_edge_deg = self.pitch_deg - half_vfov * 0.85;
        let top_edge_deg = self.pitch_deg + half_vfov * 0.85;
        target_pitch_deg >= bottom_edge_deg && target_pitch_deg <= top_edge_deg
    }

    pub fn depth_for_range(&self, yaw: f32, px: f32, py: f32, range_m: f32) -> f32 {
        let r = self.ray(yaw, px, py);
        range_m / r[0].hypot(r[2]).max(1e-6)
    }

    pub fn ground_point(&self, pose: &Pose, px: f32, py: f32, depth_m: f32) -> Option<(f32, f32)> {
        if !depth_m.is_finite() || depth_m <= 0.0 {
            return None;
        }
        let r = self.ray(pose.yaw, px, py);
        Some((pose.x + depth_m * r[0], pose.z + depth_m * r[2]))
    }

    pub fn project(&self, pose: &Pose, x: f32, y_rel: f32, z: f32) -> Option<(f32, f32, f32)> {
        let (w, h) = (self.width.max(1) as f32, self.height.max(1) as f32);
        let (fwd, right, up) = self.axes(pose.yaw);
        let rel = [x - pose.x, y_rel, z - pose.z];
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let depth = dot(rel, fwd);
        if depth <= 1e-4 {
            return None;
        }
        let half_w = (self.fov_deg.to_radians() * 0.5).tan();
        let half_h = half_w * h / w;
        let sx = dot(rel, right) / (depth * half_w);
        let sy = dot(rel, up) / (depth * half_h);
        let px = 0.5 * (sx + 1.0) * w;
        let py = 0.5 * (1.0 - sy) * h;
        Some((px, py, depth))
    }
}
