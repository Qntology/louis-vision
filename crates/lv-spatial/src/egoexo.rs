//! Multi-camera 3D anisotropic precision-matrix covariance fusion (`sigma_z = z^2 * sigma_d / (f*b)`).

use serde::{Deserialize, Serialize};

use crate::fit3::{M3, V3};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewObs {
    pub origin: V3,
    pub point: V3,
    pub sigma_z: f64,
    pub sigma_theta: f64,
    pub weight: f64,
}

#[inline]
pub fn stereo_sigma_z(z: f64, f_px: f64, b_m: f64, sigma_d_px: f64) -> f64 {
    z * z * sigma_d_px / (f_px * b_m).max(1e-12)
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn norm(a: V3) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

pub fn view_covariance(o: &ViewObs) -> M3 {
    let d = sub(o.point, o.origin);
    let z = norm(d).max(1e-9);
    let u = [d[0] / z, d[1] / z, d[2] / z];
    let (a, b) = (o.sigma_z * o.sigma_z, (z * o.sigma_theta).powi(2));
    let mut m = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let uu = u[i] * u[j];
            m[i][j] = a * uu + b * ((i == j) as u8 as f64 - uu);
        }
    }
    m
}

pub fn inv3(m: &M3) -> Option<M3> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-30 {
        return None;
    }
    let c = |r0: usize, c0: usize, r1: usize, c1: usize| {
        m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
    };
    Some([
        [c(1, 1, 2, 2) / det, -c(0, 1, 2, 2) / det, c(0, 1, 1, 2) / det],
        [-c(1, 0, 2, 2) / det, c(0, 0, 2, 2) / det, -c(0, 0, 1, 2) / det],
        [c(1, 0, 2, 1) / det, -c(0, 0, 2, 1) / det, c(0, 0, 1, 1) / det],
    ])
}

pub fn fuse(obs: &[ViewObs]) -> Option<(V3, M3)> {
    let mut info = [[0.0; 3]; 3];
    let mut acc = [0.0; 3];
    let mut any = false;
    for o in obs {
        if o.weight <= 0.0 || !o.weight.is_finite() {
            continue;
        }
        let cov = view_covariance(o);
        let Some(p) = inv3(&cov) else { continue };
        any = true;
        for i in 0..3 {
            for j in 0..3 {
                let w_p = p[i][j] * o.weight;
                info[i][j] += w_p;
                acc[i] += w_p * o.point[j];
            }
        }
    }
    if !any {
        return None;
    }
    let fused_cov = inv3(&info)?;
    let mut mean = [0.0; 3];
    for i in 0..3 {
        for j in 0..3 {
            mean[i] += fused_cov[i][j] * acc[j];
        }
    }
    Some((mean, fused_cov))
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fused3D {
    pub point: V3,
    pub cov: M3,
    pub sigma_m: f32,
}

/// Convenience wrapper returning fused 3D point, covariance matrix, and isotropic radial sigma (`sqrt(trace(cov) / 3)`).
pub fn fuse_3d(obs: &[ViewObs]) -> Option<Fused3D> {
    let (point, cov) = fuse(obs)?;
    let trace = (cov[0][0] + cov[1][1] + cov[2][2]).max(0.0);
    Some(Fused3D {
        point,
        cov,
        sigma_m: (trace / 3.0).sqrt() as f32,
    })
}
