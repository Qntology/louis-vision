//! Bearing-only triangulation (`triangulate`) and parallax-planning (`plan_step`).

use serde::{Deserialize, Serialize};

use crate::camera::{wrap_angle, Pose};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bearing {
    pub x: f32,
    pub z: f32,
    pub heading: f32,
    pub sigma: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fix {
    pub x: f32,
    pub z: f32,
    pub range_m: f32,
    pub parallax_deg: f32,
    pub baseline_m: f32,
    pub sigma_m: f32,
    pub residual_m: f32,
    pub rays: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum TriError {
    TooFewRays,
    LowParallax { parallax_deg: f32, need_deg: f32 },
    Behind,
}

impl TriError {
    pub fn code(&self) -> &'static str {
        match self {
            TriError::TooFewRays => "too_few_rays",
            TriError::LowParallax { .. } => "low_parallax",
            TriError::Behind => "behind",
        }
    }
}

pub fn triangulate(rays: &[Bearing], min_parallax_deg: f32) -> Result<Fix, TriError> {
    if rays.len() < 2 {
        return Err(TriError::TooFewRays);
    }
    let mut parallax = 0.0f32;
    let mut baseline = 0.0f32;
    for i in 0..rays.len() {
        for j in i + 1..rays.len() {
            parallax = parallax.max(wrap_angle(rays[i].heading - rays[j].heading).abs());
            baseline = baseline.max((rays[i].x - rays[j].x).hypot(rays[i].z - rays[j].z));
        }
    }
    let parallax_deg = parallax.to_degrees();
    if parallax_deg < min_parallax_deg {
        return Err(TriError::LowParallax {
            parallax_deg,
            need_deg: min_parallax_deg,
        });
    }

    // Weighted linear least-squares intersection of 2D lines with normal (-sin h, cos h).
    let (mut h00, mut h01, mut h11, mut b0, mut b1) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
    for r in rays {
        let s = r.heading.sin() as f64;
        let c = r.heading.cos() as f64;
        let (nx, nz) = (-s, c);
        let d = nx * r.x as f64 + nz * r.z as f64;
        let w = 1.0 / (r.sigma.max(1e-4) as f64).powi(2);
        h00 += w * nx * nx;
        h01 += w * nx * nz;
        h11 += w * nz * nz;
        b0 += w * nx * d;
        b1 += w * nz * d;
    }
    let det = h00 * h11 - h01 * h01;
    if det.abs() <= 1e-18 {
        return Err(TriError::LowParallax {
            parallax_deg,
            need_deg: min_parallax_deg,
        });
    }
    let x = ((h11 * b0 - h01 * b1) / det) as f32;
    let z = ((-h01 * b0 + h00 * b1) / det) as f32;

    let mut sq_err = 0.0f64;
    for r in rays {
        let dx = (x - r.x) as f64;
        let dz = (z - r.z) as f64;
        let fwd = dx * (r.heading.cos() as f64) + dz * (r.heading.sin() as f64);
        if fwd <= 0.0 {
            return Err(TriError::Behind);
        }
        let perp = -dx * (r.heading.sin() as f64) + dz * (r.heading.cos() as f64);
        sq_err += perp * perp;
    }
    let residual_m = (sq_err / rays.len() as f64).sqrt() as f32;
    let last = rays.last().unwrap();
    let range_m = (x - last.x).hypot(z - last.z);
    let sigma_m = (h00 + h11).recip().sqrt() as f32 * 2.0 + residual_m * 0.5;

    Ok(Fix {
        x,
        z,
        range_m,
        parallax_deg,
        baseline_m: baseline,
        sigma_m,
        residual_m,
        rays: rays.len(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TriCfg {
    pub min_parallax_deg: f32,
    pub plan_margin: f32,
    pub min_step_m: f32,
    pub max_step_m: f32,
    pub angled_offset_deg: f32,
}

impl Default for TriCfg {
    fn default() -> Self {
        Self {
            min_parallax_deg: 6.0,
            plan_margin: 1.4,
            min_step_m: 0.5,
            max_step_m: 2.5,
            angled_offset_deg: 55.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TriStep {
    pub kind: String,
    pub to: (f32, f32),
    pub step_m: f32,
    pub baseline_perp_m: f32,
    pub expected_parallax_deg: f32,
    pub face: (f32, f32),
}

pub fn plan_step(
    pose: &Pose,
    heading: f32,
    range_guess_m: f32,
    holonomic: bool,
    side: f32,
    cfg: &TriCfg,
) -> TriStep {
    let range = range_guess_m.clamp(1.0, 30.0);
    let need = (cfg.min_parallax_deg * cfg.plan_margin).to_radians();
    let perp = range * need.tan();
    let s = if side >= 0.0 { 1.0 } else { -1.0 };
    let (kind, dir_h, raw_step) = if holonomic {
        ("strafe", heading + s * std::f32::consts::FRAC_PI_2, perp)
    } else {
        let off = cfg.angled_offset_deg.to_radians();
        ("angled", heading + s * off, perp / off.sin())
    };
    let step_m = raw_step.clamp(cfg.min_step_m, cfg.max_step_m);
    let to = (pose.x + step_m * dir_h.cos(), pose.z + step_m * dir_h.sin());
    let face = (
        pose.x + range * heading.cos(),
        pose.z + range * heading.sin(),
    );
    let baseline_perp_m = step_m * (dir_h - heading).sin().abs();
    let expected_parallax_deg = (baseline_perp_m / range).atan().to_degrees();
    TriStep {
        kind: kind.to_string(),
        to,
        step_m,
        baseline_perp_m,
        expected_parallax_deg,
        face,
    }
}
