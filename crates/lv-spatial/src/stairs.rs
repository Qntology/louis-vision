//! Forward corridor step/curb detection using a sliding-window median profile.

use serde::{Deserialize, Serialize};

use crate::camera::{Intrinsics, Pose};
use crate::frame::DepthMap;

fn default_min_points() -> usize {
    3
}

fn default_window() -> usize {
    4
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepCfg {
    pub near_m: f32,
    pub far_m: f32,
    pub half_width_m: f32,
    pub bin_m: f32,
    pub min_step_m: f32,
    #[serde(default = "default_min_points")]
    pub min_points: usize,
    #[serde(default = "default_window")]
    pub window: usize,
}

impl Default for StepCfg {
    fn default() -> Self {
        Self {
            near_m: 0.55,
            far_m: 1.7,
            half_width_m: 0.28,
            bin_m: 0.05,
            min_step_m: 0.06,
            min_points: default_min_points(),
            window: default_window(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepRead {
    pub dist_m: f32,
    pub height_m: f32,
    pub coverage: f32,
}

fn median(v: &mut [f32]) -> f32 {
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// Detects the first height step along the camera-forward corridor ahead of the agent.
pub fn read_step(
    depth: &DepthMap,
    intr: &Intrinsics,
    pose: &Pose,
    cfg: &StepCfg,
) -> Option<StepRead> {
    let n = ((cfg.far_m - cfg.near_m) / cfg.bin_m).ceil().max(1.0) as usize;
    let window = cfg.window.max(1);
    if n < 2 * window {
        return None;
    }
    let mut bins: Vec<Vec<f32>> = vec![Vec::new(); n];
    let sx = intr.width as f32 / depth.width.max(1) as f32;
    let sy = intr.height as f32 / depth.height.max(1) as f32;

    for y in 0..depth.height {
        for x in 0..depth.width {
            let d = depth.at(x, y);
            if !d.is_finite() || d <= 0.0 {
                continue;
            }
            // Camera optical axis aligns with heading; avoid double rotation by pose.yaw.
            let r = intr.ray(0.0, (x as f32 + 0.5) * sx, (y as f32 + 0.5) * sy);
            let (along, across, up) = (r[0] * d, r[2] * d, r[1] * d);
            if across.abs() > cfg.half_width_m || along < cfg.near_m || along >= cfg.far_m {
                continue;
            }
            let idx = ((along - cfg.near_m) / cfg.bin_m) as usize;
            if idx < n {
                bins[idx].push(pose.eye_m + up);
            }
        }
    }

    let profile: Vec<Option<f32>> = bins
        .iter_mut()
        .map(|b| (b.len() >= cfg.min_points).then(|| median(b)))
        .collect();
    let read = profile.iter().flatten().count();
    if read < 2 * window {
        return None;
    }
    let coverage = read as f32 / n as f32;

    let side = |r: std::ops::Range<usize>| {
        let mut v: Vec<f32> = profile[r].iter().flatten().copied().collect();
        (v.len() * 2 > window).then(|| median(&mut v))
    };

    let mut peak: Option<(usize, f32)> = None;
    for i in window..=n - window {
        let (Some(a), Some(b)) = (side(i - window..i), side(i..i + window)) else {
            continue;
        };
        let jump = b - a;
        match peak {
            None if jump.abs() >= cfg.min_step_m => peak = Some((i, jump)),
            Some((_, p)) if jump * p > 0.0 && jump.abs() > p.abs() => peak = Some((i, jump)),
            Some((_, p)) if jump.abs() < 0.5 * p.abs() => break,
            _ => {}
        }
    }

    peak.map(|(i, jump)| StepRead {
        dist_m: cfg.near_m + i as f32 * cfg.bin_m,
        height_m: jump,
        coverage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::DepthKind;

    fn scene(eye: f32, pitch_deg: f32, edge: f32, h: f32, noise: f32) -> (DepthMap, Intrinsics, Pose) {
        let intr = Intrinsics { width: 192, height: 128, fov_deg: 70.0, pitch_deg, roll_deg: 0.0 };
        let mut data = Vec::new();
        for y in 0..intr.height {
            for x in 0..intr.width {
                let r = intr.ray(0.0, x as f32 + 0.5, y as f32 + 0.5);
                let mut d = f32::NAN;
                if r[1] < -1e-4 {
                    let top = -(eye - h) / r[1];
                    let low = -eye / r[1];
                    d = if h >= 0.0 {
                        if r[0] * top >= edge {
                            top
                        } else if r[0] * low >= edge {
                            edge / r[0]
                        } else {
                            low
                        }
                    } else if r[0] * top < edge {
                        low.min(edge / r[0]).max(0.0).min(top).max(low.min(top))
                    } else {
                        top
                    };
                    let u = ((x * 73 + y * 37) % 101) as f32 / 100.0 - 0.5;
                    d *= 1.0 + noise * u;
                }
                data.push(d);
            }
        }
        (
            DepthMap { width: intr.width, height: intr.height, kind: DepthKind::Metric, data },
            intr,
            Pose { x: 0.0, z: 0.0, yaw: 0.0, eye_m: eye },
        )
    }

    #[test]
    fn level_ground_is_no_step() {
        let (d, i, p) = scene(1.5, -40.0, 9.0, 0.0, 0.01);
        assert_eq!(read_step(&d, &i, &p, &StepCfg::default()), None);
    }

    #[test]
    fn a_step_up_is_read_for_its_height_and_where_it_starts() {
        for (edge, h) in [(1.2f32, 0.15f32), (0.95, 0.08), (1.3, 0.18)] {
            let (d, i, p) = scene(1.5, -40.0, edge, h, 0.005);
            let r = read_step(&d, &i, &p, &StepCfg::default()).expect("a step");
            assert!((r.height_m - h).abs() < 0.03, "{r:?} for {h}");
            assert!((r.dist_m - edge).abs() < 0.15, "{r:?} for {edge}");
        }
    }

    #[test]
    fn a_depth_off_in_scale_reads_the_same_step() {
        let (mut d, i, p) = scene(1.5, -40.0, 1.2, 0.15, 0.0);
        for v in d.data.iter_mut() {
            *v *= 0.85;
        }
        let r = read_step(&d, &i, &p, &StepCfg::default()).expect("a step");
        assert!((r.height_m - 0.15).abs() < 0.04, "{r:?}");
    }

    #[test]
    fn missing_pixels_do_not_make_a_step() {
        let (mut d, i, p) = scene(1.5, -40.0, 9.0, 0.0, 0.0);
        for (k, v) in d.data.iter_mut().enumerate() {
            if k % 3 == 0 {
                *v = f32::NAN;
            }
        }
        assert_eq!(read_step(&d, &i, &p, &StepCfg::default()), None);
    }

    #[test]
    fn nonzero_pose_yaw_does_not_double_rotate() {
        let (d, i, p0) = scene(1.5, -40.0, 1.2, 0.15, 0.005);
        let p_yaw = Pose { yaw: 1.2, ..p0 };
        let r0 = read_step(&d, &i, &p0, &StepCfg::default()).expect("yaw 0 step");
        let r1 = read_step(&d, &i, &p_yaw, &StepCfg::default()).expect("yaw 1.2 step");
        assert!((r0.height_m - r1.height_m).abs() < 1e-4);
        assert!((r0.dist_m - r1.dist_m).abs() < 1e-4);
    }
}
