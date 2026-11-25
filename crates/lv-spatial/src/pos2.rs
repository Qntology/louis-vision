//! 2D ground-plane position estimator with Page-Hinkley CUSUM mode switching (`Static <-> Dynamic`),
//! 4-state constant-velocity Kalman filter, and chi2 Mahalanobis innovation gating.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

pub const CHI2_2DOF_99: f64 = 9.21034;
pub const POS2_RING_CAP: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pos2Model {
    #[default]
    Static,
    Dynamic,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pos2Obs {
    pub tick: u64,
    pub x: f64,
    pub z: f64,
    pub sigma_m: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pos2Estimate {
    pub mean: [f64; 2],
    pub velocity: [f64; 2],
    pub cov: [f64; 4],
    pub sigma_m: f64,
    pub model: Pos2Model,
    pub chi2_gate: f64,
}

impl Pos2Estimate {
    pub fn mahalanobis_sq(&self, x: f64, z: f64, sigma_m: f64) -> f64 {
        let dx = x - self.mean[0];
        let dz = z - self.mean[1];
        let sx = (self.cov[0] + sigma_m * sigma_m).max(1e-8);
        let sz = (self.cov[3] + sigma_m * sigma_m).max(1e-8);
        (dx * dx) / sx + (dz * dz) / sz
    }

    pub fn accepts(&self, x: f64, z: f64, sigma_m: f64) -> bool {
        self.mahalanobis_sq(x, z, sigma_m) <= self.chi2_gate
    }

    pub fn band_rect(&self, sigmas: f64, obs_sigma_m: f64) -> [f32; 4] {
        let sx = (self.cov[0].max(0.0) + obs_sigma_m * obs_sigma_m).sqrt() * sigmas;
        let sz = (self.cov[3].max(0.0) + obs_sigma_m * obs_sigma_m).sqrt() * sigmas;
        [
            (self.mean[0] - sx) as f32,
            (self.mean[0] + sx) as f32,
            (self.mean[1] - sz) as f32,
            (self.mean[1] + sz) as f32,
        ]
    }
}

#[derive(Clone, Debug, Default)]
struct Cusum {
    mean: f64,
    n: usize,
    pos: f64,
    neg: f64,
}

impl Cusum {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn push(&mut self, val: f64, sigma: f64) -> bool {
        self.n += 1;
        let n = self.n as f64;
        if self.n <= 2 {
            self.mean += (val - self.mean) / n;
            return false;
        }
        let ref_mean = self.mean;
        self.mean += (val - self.mean) / n;
        let s = sigma.max(0.05);
        let z = (val - ref_mean) / s;
        self.pos = (self.pos + z - 0.5).max(0.0);
        self.neg = (self.neg - z - 0.5).max(0.0);
        self.pos > 4.0 || self.neg > 4.0
    }
}

#[derive(Clone, Debug, Default)]
struct CvKalman {
    initialized: bool,
    last_tick: u64,
    x: [f64; 4], // [px, pz, vx, vz]
    p: [[f64; 4]; 4],
}

impl CvKalman {
    fn init(&mut self, tick: u64, px: f64, pz: f64, vx: f64, vz: f64, sigma: f64) {
        self.initialized = true;
        self.last_tick = tick;
        self.x = [px, pz, vx, vz];
        let s2 = sigma * sigma;
        self.p = [
            [s2, 0.0, 0.0, 0.0],
            [0.0, s2, 0.0, 0.0],
            [0.0, 0.0, 0.25, 0.0],
            [0.0, 0.0, 0.0, 0.25],
        ];
    }

    fn predict_at(&self, tick: u64) -> ([f64; 4], [[f64; 4]; 4]) {
        let dt = tick.saturating_sub(self.last_tick) as f64;
        let mut xp = self.x;
        xp[0] += xp[2] * dt;
        xp[1] += xp[3] * dt;
        let mut pp = self.p;
        let q_pos = 0.01 * dt;
        let q_vel = 0.02 * dt;
        pp[0][0] += dt * dt * self.p[2][2] + q_pos;
        pp[1][1] += dt * dt * self.p[3][3] + q_pos;
        pp[2][2] += q_vel;
        pp[3][3] += q_vel;
        (xp, pp)
    }

    fn update(&mut self, tick: u64, px: f64, pz: f64, sigma: f64) {
        if !self.initialized {
            self.init(tick, px, pz, 0.0, 0.0, sigma);
            return;
        }
        let dt = tick.saturating_sub(self.last_tick).max(1) as f64;
        let (mut xp, mut pp) = self.predict_at(tick);
        let r = (sigma * sigma).max(1e-6);
        let kx = pp[0][0] / (pp[0][0] + r);
        let kz = pp[1][1] / (pp[1][1] + r);
        let ix = px - xp[0];
        let iz = pz - xp[1];
        xp[0] += kx * ix;
        xp[1] += kz * iz;
        xp[2] += (kx / dt).min(0.8) * ix;
        xp[3] += (kz / dt).min(0.8) * iz;
        pp[0][0] *= 1.0 - kx;
        pp[1][1] *= 1.0 - kz;
        self.x = xp;
        self.p = pp;
        self.last_tick = tick;
    }
}

#[derive(Clone, Debug)]
pub struct Pos2Filter {
    obs: VecDeque<Pos2Obs>,
    model: Pos2Model,
    cusum_x: Cusum,
    cusum_z: Cusum,
    kf: CvKalman,
    pub chi2_gate: f64,
}

impl Default for Pos2Filter {
    fn default() -> Self {
        Self {
            obs: VecDeque::with_capacity(POS2_RING_CAP),
            model: Pos2Model::Static,
            cusum_x: Cusum::default(),
            cusum_z: Cusum::default(),
            kf: CvKalman::default(),
            chi2_gate: CHI2_2DOF_99,
        }
    }
}

impl Pos2Filter {
    pub fn clear(&mut self) {
        self.obs.clear();
        self.model = Pos2Model::Static;
        self.cusum_x.reset();
        self.cusum_z.reset();
        self.kf = CvKalman::default();
    }

    pub fn len(&self) -> usize {
        self.obs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.obs.is_empty()
    }

    pub fn model(&self) -> Pos2Model {
        self.model
    }

    pub fn observations(&self) -> &VecDeque<Pos2Obs> {
        &self.obs
    }

    pub fn estimate(&self, at_tick: Option<u64>) -> Option<Pos2Estimate> {
        if self.obs.is_empty() {
            return None;
        }
        if self.model == Pos2Model::Dynamic && self.kf.initialized {
            let tick = at_tick.unwrap_or(self.kf.last_tick);
            let (xp, pp) = self.kf.predict_at(tick);
            let sigma_m = pp[0][0].max(pp[1][1]).max(0.0).sqrt();
            return Some(Pos2Estimate {
                mean: [xp[0], xp[1]],
                velocity: [xp[2], xp[3]],
                cov: [pp[0][0], pp[0][1], pp[1][0], pp[1][1]],
                sigma_m,
                model: Pos2Model::Dynamic,
                chi2_gate: self.chi2_gate,
            });
        }

        let (mut wx, mut wz, mut wsum) = (0.0f64, 0.0f64, 0.0f64);
        for o in &self.obs {
            let w = 1.0 / (o.sigma_m.max(1e-4) * o.sigma_m.max(1e-4));
            wx += w * o.x;
            wz += w * o.z;
            wsum += w;
        }
        let inv = 1.0 / wsum.max(1e-12);
        let sigma_m = inv.sqrt();
        Some(Pos2Estimate {
            mean: [wx * inv, wz * inv],
            velocity: [0.0, 0.0],
            cov: [inv, 0.0, 0.0, inv],
            sigma_m,
            model: Pos2Model::Static,
            chi2_gate: self.chi2_gate,
        })
    }

    pub fn push(&mut self, obs: Pos2Obs) -> bool {
        if !obs.x.is_finite() || !obs.z.is_finite() || !obs.sigma_m.is_finite() {
            return false;
        }
        if self.obs.len() >= POS2_RING_CAP {
            self.obs.pop_front();
        }
        self.obs.push_back(obs);

        if self.model == Pos2Model::Static {
            let sx = self.cusum_x.push(obs.x, obs.sigma_m);
            let sz = self.cusum_z.push(obs.z, obs.sigma_m);
            if sx || sz {
                self.model = Pos2Model::Dynamic;
                let mut vx = 0.0;
                let mut vz = 0.0;
                if self.obs.len() >= 2 {
                    let prev = self.obs[self.obs.len() - 2];
                    let dt = obs.tick.saturating_sub(prev.tick).max(1) as f64;
                    vx = (obs.x - prev.x) / dt;
                    vz = (obs.z - prev.z) / dt;
                }
                self.kf.init(obs.tick, obs.x, obs.z, vx, vz, obs.sigma_m);
                return true;
            }
            false
        } else {
            self.kf.update(obs.tick, obs.x, obs.z, obs.sigma_m);
            false
        }
    }
}
