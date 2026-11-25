//! Target navigation & fixation decision state machine backed by `Pos2Filter` and triangulation.

use serde::{Deserialize, Serialize};

use crate::camera::{wrap_angle, Intrinsics, Pose};
use crate::pos2::{Pos2Filter, Pos2Obs};
use crate::triangulate::{plan_step, triangulate, Bearing, Fix, TriCfg, TriStep};

pub const LANDMARK_CENTRE_FRAC: f32 = 0.2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecideCfg {
    pub reach_m: f32,
    pub hop_max_m: f32,
    pub search_turn_deg: f32,
    pub max_sigma_frac: f32,
    pub max_rays: usize,
    pub band_widen_sigma: f32,
    pub default_range_m: f32,
    pub close_range_m: f32,
    pub tri: TriCfg,
}

impl Default for DecideCfg {
    fn default() -> Self {
        Self {
            reach_m: 0.75,
            hop_max_m: 1.8,
            search_turn_deg: 28.0,
            max_sigma_frac: 0.25,
            max_rays: 16,
            band_widen_sigma: 2.0,
            default_range_m: 4.0,
            close_range_m: 1.5,
            tri: TriCfg::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sighting {
    pub keyframe: u32,
    pub pose: Pose,
    pub heading: f32,
    pub sigma: f32,
    #[serde(default)]
    pub extent: f32,
    pub depth_range_m: Option<f32>,
    pub track_id: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Search { turn_deg: f32 },
    LookDown { pitch_deg: f32, turn_deg: f32 },
    Triangulate(TriStep),
    Approach { to: (f32, f32), hop_m: f32, face: (f32, f32), look_pitch_deg: f32 },
    Arrive { at: (f32, f32), look_pitch_deg: f32 },
    Hold { reason: String },
    Explore { turned_deg: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prediction {
    pub bearing_rel_deg: f32,
    pub range_m: f32,
    pub pitch_deg: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    pub x: f32,
    pub z: f32,
    pub range_m: f32,
    pub sigma_m: f32,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionReport {
    pub decision: Decision,
    pub estimate: Option<Estimate>,
    pub fix: Option<Fix>,
    pub fix_error: Option<String>,
    pub depth_scale: Option<f32>,
    pub residual_deg: Option<f32>,
    pub prediction: Option<Prediction>,
    pub forecast_method: String,
    pub band: Option<[f32; 4]>,
}

pub fn scale_usable(fix: &Fix, s: &Sighting) -> bool {
    let centre = s.sigma.hypot(LANDMARK_CENTRE_FRAC * s.extent) / s.sigma.max(1e-6);
    let near = s
        .depth_range_m
        .is_some_and(|d| d >= 0.5 && (0.5..=2.0).contains(&(fix.range_m / d)));
    fix.rays >= 3
        && fix.residual_m <= 0.15
        && fix.sigma_m * centre <= 0.15 * fix.range_m
        && (1.5..=9.0).contains(&fix.range_m)
        && near
}

pub fn centroid_sigma(width: u32, fov_deg: f32, px_err: f32) -> f32 {
    let f = width.max(1) as f32 * 0.5 / (fov_deg.to_radians() * 0.5).tan();
    (px_err / f).max(1e-4)
}

pub struct Decider {
    pub cfg: DecideCfg,
    rays: Vec<Sighting>,
    pos2: Pos2Filter,
    last_fix: Option<Fix>,
    side: f32,
    lost: u32,
    turned_deg: f32,
}

impl Decider {
    pub fn new(cfg: DecideCfg) -> Self {
        Self {
            cfg,
            rays: Vec::new(),
            pos2: Pos2Filter::default(),
            last_fix: None,
            side: 1.0,
            lost: 0,
            turned_deg: 0.0,
        }
    }

    pub fn step(
        &mut self,
        pose: &Pose,
        intr: &Intrinsics,
        sighting: Option<Sighting>,
        holonomic: bool,
    ) -> DecisionReport {
        let Some(s) = sighting else {
            self.lost += 1;
            let turn = self.cfg.search_turn_deg * self.side;
            self.turned_deg += turn.abs();
            if self.turned_deg >= 360.0 {
                return DecisionReport {
                    decision: Decision::Hold { reason: "target_lost".into() },
                    estimate: None,
                    fix: self.last_fix,
                    fix_error: None,
                    depth_scale: None,
                    residual_deg: None,
                    prediction: None,
                    forecast_method: "none".into(),
                    band: self.pos2.estimate(None).map(|e| e.band_rect(self.cfg.band_widen_sigma as f64, 0.1)),
                };
            }
            return DecisionReport {
                decision: Decision::Search { turn_deg: turn },
                estimate: None,
                fix: self.last_fix,
                fix_error: None,
                depth_scale: None,
                residual_deg: None,
                prediction: None,
                forecast_method: "search".into(),
                band: None,
            };
        };

        self.lost = 0;
        self.turned_deg = 0.0;
        if self.rays.len() >= self.cfg.max_rays {
            self.rays.remove(0);
        }
        self.rays.push(s);

        let bearings: Vec<Bearing> = self
            .rays
            .iter()
            .map(|r| Bearing { x: r.pose.x, z: r.pose.z, heading: r.heading, sigma: r.sigma })
            .collect();

        let mut fix_err = None;
        let mut depth_scale = None;
        match triangulate(&bearings, self.cfg.tri.min_parallax_deg) {
            Ok(f) => {
                self.last_fix = Some(f);
                if scale_usable(&f, &s) {
                    if let Some(d) = s.depth_range_m {
                        depth_scale = Some(f.range_m / d);
                    }
                }
                self.pos2.push(Pos2Obs {
                    tick: s.keyframe as u64,
                    x: f.x as f64,
                    z: f.z as f64,
                    sigma_m: f.sigma_m.max(0.05) as f64,
                });
            }
            Err(e) => {
                fix_err = Some(e.code().to_string());
                if let Some(dr) = s.depth_range_m {
                    let gx = pose.x + dr * s.heading.cos();
                    let gz = pose.z + dr * s.heading.sin();
                    self.pos2.push(Pos2Obs {
                        tick: s.keyframe as u64,
                        x: gx as f64,
                        z: gz as f64,
                        sigma_m: (0.2 * dr).max(0.1) as f64,
                    });
                }
            }
        }

        let est = self.pos2.estimate(Some(s.keyframe as u64));
        let (tx, tz, range_m, sigma_m, source) = if let Some(f) = self.last_fix {
            (f.x, f.z, (f.x - pose.x).hypot(f.z - pose.z), f.sigma_m, "tri".to_string())
        } else if let Some(dr) = s.depth_range_m {
            (
                pose.x + dr * s.heading.cos(),
                pose.z + dr * s.heading.sin(),
                dr,
                0.2 * dr,
                "depth".to_string(),
            )
        } else {
            let step = plan_step(pose, s.heading, self.cfg.default_range_m, holonomic, self.side, &self.cfg.tri);
            self.side = -self.side;
            return DecisionReport {
                decision: Decision::Triangulate(step),
                estimate: None,
                fix: self.last_fix,
                fix_error: fix_err,
                depth_scale,
                residual_deg: None,
                prediction: None,
                forecast_method: "triangulate".into(),
                band: est.map(|e| e.band_rect(self.cfg.band_widen_sigma as f64, 0.2)),
            };
        };

        let look_pitch_deg = intr.look_down_pitch_deg(pose.eye_m, range_m, 0.0);
        let rel_deg = wrap_angle(s.heading - pose.yaw).to_degrees();
        let pred = Some(Prediction { bearing_rel_deg: rel_deg, range_m, pitch_deg: look_pitch_deg });

        let decision = if range_m <= self.cfg.reach_m {
            Decision::Arrive { at: (tx, tz), look_pitch_deg }
        } else if sigma_m > self.cfg.max_sigma_frac * range_m && self.last_fix.is_none() {
            let step = plan_step(pose, s.heading, range_m, holonomic, self.side, &self.cfg.tri);
            self.side = -self.side;
            Decision::Triangulate(step)
        } else {
            let hop_m = (range_m - self.cfg.reach_m * 0.5).clamp(0.1, self.cfg.hop_max_m);
            let dir = (tz - pose.z).atan2(tx - pose.x);
            let to = (pose.x + hop_m * dir.cos(), pose.z + hop_m * dir.sin());
            Decision::Approach { to, hop_m, face: (tx, tz), look_pitch_deg }
        };

        DecisionReport {
            decision,
            estimate: Some(Estimate { x: tx, z: tz, range_m, sigma_m, source }),
            fix: self.last_fix,
            fix_error: fix_err,
            depth_scale,
            residual_deg: self.last_fix.map(|f| (f.residual_m / f.range_m.max(0.1)).to_degrees()),
            prediction: pred,
            forecast_method: "pos2_kalman".into(),
            band: est.map(|e| e.band_rect(self.cfg.band_widen_sigma as f64, sigma_m as f64)),
        }
    }
}
