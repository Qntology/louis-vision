//! Video / XR frame quality metrics: `psnr`, `ssim`, and temporal `motion_iou`.

use serde::{Deserialize, Serialize};

use crate::frame::Rgb;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionMetrics {
    pub motion_iou: f32,
    pub psnr: f32,
    pub ssim: f32,
    pub moving_ref: u32,
    pub moving_gen: u32,
}

fn luma(rgb: &Rgb) -> Vec<f32> {
    rgb.data
        .chunks_exact(3)
        .map(|p| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32)
        .collect()
}

pub fn psnr(a: &Rgb, b: &Rgb) -> f32 {
    if a.data.len() != b.data.len() || a.data.is_empty() {
        return 0.0;
    }
    let mse = a
        .data
        .iter()
        .zip(&b.data)
        .map(|(&x, &y)| {
            let d = x as f64 - y as f64;
            d * d
        })
        .sum::<f64>()
        / a.data.len() as f64;
    if mse < 1e-10 {
        99.0
    } else {
        (10.0 * (255.0 * 255.0 / mse).log10()) as f32
    }
}

pub fn ssim(a: &Rgb, b: &Rgb) -> f32 {
    if a.data.len() != b.data.len() || a.data.is_empty() {
        return 0.0;
    }
    let la = luma(a);
    let lb = luma(b);
    let n = la.len() as f64;
    let ma = la.iter().map(|&v| v as f64).sum::<f64>() / n;
    let mb = lb.iter().map(|&v| v as f64).sum::<f64>() / n;
    let (mut va, mut vb, mut cab) = (0.0f64, 0.0f64, 0.0f64);
    for (&x, &y) in la.iter().zip(&lb) {
        let da = x as f64 - ma;
        let db = y as f64 - mb;
        va += da * da;
        vb += db * db;
        cab += da * db;
    }
    va /= n;
    vb /= n;
    cab /= n;
    let c1 = (0.01 * 255.0) * (0.01 * 255.0);
    let c2 = (0.03 * 255.0) * (0.03 * 255.0);
    (((2.0 * ma * mb + c1) * (2.0 * cab + c2))
        / ((ma * ma + mb * mb + c1) * (va + vb + c2))) as f32
}

pub fn motion_metrics(reference: &[&Rgb], gen: &[&Rgb], diff_thresh: f32) -> Option<MotionMetrics> {
    if reference.len() < 2 || reference.len() != gen.len() {
        return None;
    }
    let (mut inter, mut uni, mut mr, mut mg) = (0u32, 0u32, 0u32, 0u32);
    for t in 1..reference.len() {
        let (r0, r1) = (luma(reference[t - 1]), luma(reference[t]));
        let (g0, g1) = (luma(gen[t - 1]), luma(gen[t]));
        for i in 0..r0.len().min(g0.len()) {
            let rm = (r1[i] - r0[i]).abs() >= diff_thresh;
            let gm = (g1[i] - g0[i]).abs() >= diff_thresh;
            mr += rm as u32;
            mg += gm as u32;
            inter += (rm && gm) as u32;
            uni += (rm || gm) as u32;
        }
    }
    let iou = if uni == 0 { 1.0 } else { inter as f32 / uni as f32 };
    let k = reference.len() as f32;
    let p = reference.iter().zip(gen).map(|(a, b)| psnr(a, b)).sum::<f32>() / k;
    let s = reference.iter().zip(gen).map(|(a, b)| ssim(a, b)).sum::<f32>() / k;
    Some(MotionMetrics {
        motion_iou: iou,
        psnr: p,
        ssim: s,
        moving_ref: mr,
        moving_gen: mg,
    })
}
