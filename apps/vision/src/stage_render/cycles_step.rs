//! Adopted-camera-only tiled `FrameStepCycles` CPU raycaster baker with instant (`<= 16.6 ms`) abort.
//!
//! Uses `sandbox_world::render::{Camera, View}` pinhole projection with multi-sample Halton jitter
//! to raycast stage deck, lift podium, truss columns, LED wall, and floor checkerboard without
//! introducing any external GPU crates.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use sandbox_world::render::{Camera, View};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CyclesBakeJob {
    pub slice_idx: u32,
    pub adopted_cam_id: String,
    pub candidate_cams: Vec<String>,
    pub width: u32,
    pub height: u32,
    pub tile_size: u32,
    pub samples: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CyclesBakeReport {
    pub slice_idx: u32,
    pub baked_cam_id: String,
    pub skipped_cams: Vec<String>,
    pub tiles_completed: usize,
    pub tiles_total: usize,
    pub aborted: bool,
    pub elapsed_us: u64,
}

pub struct FrameStepCycles {
    abort_flag: Arc<AtomicBool>,
}

impl Default for FrameStepCycles {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
struct StageBox {
    min: [f32; 3],
    max: [f32; 3],
    albedo: [f32; 3],
    emissive: [f32; 3],
}

fn halton(mut index: u32, base: u32) -> f32 {
    let mut f = 1.0f32;
    let mut r = 0.0f32;
    let bf = base as f32;
    while index > 0 {
        f /= bf;
        r += f * (index % base) as f32;
        index /= base;
    }
    r
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm3(v: [f32; 3]) -> [f32; 3] {
    let l = dot3(v, v).sqrt().max(1e-6);
    [v[0] / l, v[1] / l, v[2] / l]
}

fn ray_aabb(ro: [f32; 3], rd: [f32; 3], b: &StageBox) -> Option<(f32, [f32; 3])> {
    let mut tmin = 0.001f32;
    let mut tmax = 120.0f32;
    let mut hit_axis = 0usize;
    let mut hit_sign = -1.0f32;

    for axis in 0..3 {
        let d = rd[axis];
        if d.abs() < 1e-7 {
            if ro[axis] < b.min[axis] || ro[axis] > b.max[axis] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / d;
        let mut t0 = (b.min[axis] - ro[axis]) * inv;
        let mut t1 = (b.max[axis] - ro[axis]) * inv;
        let mut s = -1.0f32;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
            s = 1.0;
        }
        if t0 > tmin {
            tmin = t0;
            hit_axis = axis;
            hit_sign = s;
        }
        tmax = tmax.min(t1);
        if tmax <= tmin {
            return None;
        }
    }
    let mut n = [0.0f32; 3];
    n[hit_axis] = hit_sign;
    Some((tmin, n))
}

fn view_for_adopted_cam(cam_id: &str, slice_idx: u32) -> View {
    let lower = cam_id.to_ascii_lowercase();
    let drift = (slice_idx as f32) * 0.08;
    if lower.contains("side") || lower.contains("stage_l") {
        View {
            origin: [-7.5, 3.2, 4.5 + drift],
            yaw: 0.92,
            pitch: -0.22,
            roll: 0.0,
        }
    } else if lower.contains("over") || lower.contains("top") {
        View {
            origin: [0.0, 11.5, 0.2 + drift],
            yaw: 0.0,
            pitch: -1.35,
            roll: 0.0,
        }
    } else if lower.contains("jib") {
        View {
            origin: [6.2, 4.4, 5.2 + drift],
            yaw: -0.78,
            pitch: -0.28,
            roll: 0.0,
        }
    } else {
        // FOH front-of-house hero camera looking at +Z stage deck
        View {
            origin: [0.0, 2.8, -7.8 + drift * 0.5],
            yaw: 0.0,
            pitch: -0.16,
            roll: 0.0,
        }
    }
}

fn default_stage_scene(slice_idx: u32) -> [StageBox; 6] {
    let lift_h = 0.8 + ((slice_idx % 4) as f32) * 0.25;
    [
        // Main stage deck slab
        StageBox {
            min: [-6.0, 0.0, -1.5],
            max: [6.0, 0.8, 5.0],
            albedo: [0.22, 0.24, 0.28],
            emissive: [0.0, 0.0, 0.0],
        },
        // Center hydraulic lift podium
        StageBox {
            min: [-1.4, 0.8, 0.4],
            max: [1.4, 0.8 + lift_h, 2.8],
            albedo: [0.35, 0.38, 0.44],
            emissive: [0.04, 0.14, 0.24],
        },
        // Center performer stand-in silhouette (vivid Haring yellow flat color)
        StageBox {
            min: [-0.35, 0.8 + lift_h, 1.25],
            max: [0.35, 2.55 + lift_h, 1.95],
            albedo: [0.98, 0.78, 0.05],
            emissive: [0.12, 0.09, 0.0],
        },
        // Backstage LED backdrop wall
        StageBox {
            min: [-5.6, 0.8, 4.7],
            max: [5.6, 5.6, 5.0],
            albedo: [0.12, 0.16, 0.26],
            emissive: [0.16, 0.34, 0.68],
        },
        // Left & right overhead truss towers
        StageBox {
            min: [-5.8, 0.0, -1.3],
            max: [-5.3, 6.2, -0.8],
            albedo: [0.55, 0.58, 0.62],
            emissive: [0.0, 0.0, 0.0],
        },
        StageBox {
            min: [5.3, 0.0, -1.3],
            max: [5.8, 6.2, -0.8],
            albedo: [0.55, 0.58, 0.62],
            emissive: [0.0, 0.0, 0.0],
        },
    ]
}

fn trace_stage_ray(ro: [f32; 3], rd: [f32; 3], scene: &[StageBox]) -> [f32; 3] {
    let mut best_t = f32::INFINITY;
    let mut hit_n = [0.0f32, 1.0, 0.0];
    let mut hit_albedo = [0.0f32; 3];
    let mut hit_emissive = [0.0f32; 3];
    let mut hit_any = false;

    // Intersect stage floor y = 0
    if rd[1] < -1e-5 {
        let t = -ro[1] / rd[1];
        if t > 0.001 && t < 80.0 {
            best_t = t;
            let px = ro[0] + rd[0] * t;
            let pz = ro[2] + rd[2] * t;
            let checker = ((px.floor() as i32 + pz.floor() as i32) & 1) == 0;
            hit_albedo = if checker { [0.16, 0.17, 0.20] } else { [0.10, 0.11, 0.13] };
            hit_emissive = [0.0, 0.0, 0.0];
            hit_n = [0.0, 1.0, 0.0];
            hit_any = true;
        }
    }

    for b in scene {
        if let Some((t, n)) = ray_aabb(ro, rd, b) {
            if t < best_t {
                best_t = t;
                hit_n = n;
                hit_albedo = b.albedo;
                hit_emissive = b.emissive;
                hit_any = true;
            }
        }
    }

    if !hit_any {
        // Dark arena sky gradient
        let up = (rd[1] * 0.5 + 0.5).clamp(0.0, 1.0);
        return [0.03 + 0.05 * up, 0.04 + 0.08 * up, 0.08 + 0.14 * up];
    }

    let hit_p = [
        ro[0] + rd[0] * best_t,
        ro[1] + rd[1] * best_t,
        ro[2] + rd[2] * best_t,
    ];
    let key_dir = norm3([0.45, 0.82, -0.36]);
    let rim_dir = norm3([-0.55, 0.42, 0.72]);

    // Cast hard shadow ray toward overhead DMX key light
    let shadow_origin = [
        hit_p[0] + hit_n[0] * 0.004,
        hit_p[1] + hit_n[1] * 0.004,
        hit_p[2] + hit_n[2] * 0.004,
    ];
    let mut in_shadow = false;
    for b in scene {
        if ray_aabb(shadow_origin, key_dir, b).is_some() {
            in_shadow = true;
            break;
        }
    }

    let ndl_key = dot3(hit_n, key_dir).max(0.0) * if in_shadow { 0.22 } else { 1.0 };
    let ndl_rim = dot3(hit_n, rim_dir).max(0.0) * 0.45;
    let ambient = 0.18;

    let key_col = [1.05, 0.96, 0.86];
    let rim_col = [0.28, 0.58, 1.0];
    [
        hit_emissive[0] + hit_albedo[0] * (ambient + ndl_key * key_col[0] + ndl_rim * rim_col[0]),
        hit_emissive[1] + hit_albedo[1] * (ambient + ndl_key * key_col[1] + ndl_rim * rim_col[1]),
        hit_emissive[2] + hit_albedo[2] * (ambient + ndl_key * key_col[2] + ndl_rim * rim_col[2]),
    ]
}

impl FrameStepCycles {
    pub fn new() -> Self {
        Self {
            abort_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn abort_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.abort_flag)
    }

    /// Triggers instant abort (`stage_render_abort`), returning signaling latency in microseconds.
    pub fn abort_now(&self) -> u64 {
        let t0 = Instant::now();
        self.abort_flag.store(true, Ordering::Release);
        t0.elapsed().as_micros().max(1) as u64
    }

    pub fn reset_abort(&self) {
        self.abort_flag.store(false, Ordering::Release);
    }

    /// Executes tiled jittered CPU raycasting STRICTLY for `job.adopted_cam_id`
    /// (skipping all non-adopted cameras) and checks `abort_flag` at every tile boundary
    /// for `<= 16.6 ms` cancellation responsiveness.
    pub fn bake_adopted_cam(&self, job: &CyclesBakeJob) -> (CyclesBakeReport, Vec<u8>) {
        let t0 = Instant::now();
        let tile = job.tile_size.max(16);
        let cols = job.width.div_ceil(tile) as usize;
        let rows = job.height.div_ceil(tile) as usize;
        let tiles_total = cols * rows;
        let skipped_cams: Vec<String> = job
            .candidate_cams
            .iter()
            .filter(|c| c.as_str() != job.adopted_cam_id.as_str())
            .cloned()
            .collect();

        let cam = Camera {
            width: job.width.max(1) as usize,
            height: job.height.max(1) as usize,
            fov_deg: 64.0,
            pitch_deg: -10.0,
            max_dist_m: 60.0,
            pan_deg: 0.0,
        };
        let view = view_for_adopted_cam(&job.adopted_cam_id, job.slice_idx);
        let scene = default_stage_scene(job.slice_idx);
        let spp = job.samples.clamp(1, 64);
        let inv_spp = 1.0 / (spp as f32);

        let mut rgba = vec![0u8; (job.width * job.height * 4) as usize];
        let mut tiles_completed = 0usize;
        let mut aborted = false;

        'outer: for ty in 0..rows {
            for tx in 0..cols {
                if self.abort_flag.load(Ordering::Acquire) {
                    aborted = true;
                    break 'outer;
                }
                let x0 = (tx as u32) * tile;
                let y0 = (ty as u32) * tile;
                let x1 = (x0 + tile).min(job.width);
                let y1 = (y0 + tile).min(job.height);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let mut acc = [0.0f32; 3];
                        for s in 0..spp {
                            let jx = halton(s + 1, 2);
                            let jy = halton(s + 1, 3);
                            let px = (x as f32) + jx;
                            let py = (y as f32) + jy;
                            let pt = cam
                                .world_point3_in(&view, px, py, 1.0)
                                .unwrap_or([view.origin[0], view.origin[1], view.origin[2] + 1.0]);
                            let rd = norm3([
                                pt[0] - view.origin[0],
                                pt[1] - view.origin[1],
                                pt[2] - view.origin[2],
                            ]);
                            let col = trace_stage_ray(view.origin, rd, &scene);
                            acc[0] += col[0];
                            acc[1] += col[1];
                            acc[2] += col[2];
                        }
                        let o = ((y * job.width + x) * 4) as usize;
                        for ch in 0..3 {
                            let lin = (acc[ch] * inv_spp).max(0.0);
                            // Reinhard tone-map + approximate sRGB gamma
                            let tm = lin / (1.0 + lin);
                            rgba[o + ch] = (tm.sqrt().clamp(0.0, 1.0) * 255.0).round() as u8;
                        }
                        rgba[o + 3] = 255;
                    }
                }
                tiles_completed += 1;
            }
        }

        let report = CyclesBakeReport {
            slice_idx: job.slice_idx,
            baked_cam_id: job.adopted_cam_id.clone(),
            skipped_cams,
            tiles_completed,
            tiles_total,
            aborted,
            elapsed_us: t0.elapsed().as_micros().max(1) as u64,
        };
        (report, rgba)
    }
}
