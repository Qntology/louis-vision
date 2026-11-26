//! 5x5 24-bit Census Transform, Hamming block cost volume, and Semi-Global (4/8-path) DP aggregation.
//!
//! Patent-clear single left cost-volume SGBM primitives shared with `lv-spatial::stereo` and
//! real-time Z-Keying (`StereoSandboxLive`): computes one cost volume over the left image with
//! illumination-invariant local rank bits and edge-adaptive P2 smoothness penalties.

use rayon::prelude::*;

pub const CENSUS_R: i64 = 2;
pub const CENSUS_BITS: u16 = 24;

/// 5x5 24-bit Census Transform (bit set per neighbor strictly darker than center pixel,
/// clamping at borders) and local 5x5 grey-level standard deviation for texture gating.
pub fn census_5x5_u32(grey: &[f32], w: usize, h: usize) -> (Vec<u32>, Vec<f32>) {
    assert_eq!(grey.len(), w * h, "grey image buffer length mismatch");
    if w == 0 || h == 0 {
        return (Vec::new(), Vec::new());
    }
    let mut code = vec![0u32; w * h];
    let mut sd = vec![0.0f32; w * h];
    let n = ((2 * CENSUS_R + 1) * (2 * CENSUS_R + 1)) as f32;
    let at = |x: i64, y: i64| -> f32 {
        grey[(y.clamp(0, h as i64 - 1) as usize) * w + x.clamp(0, w as i64 - 1) as usize]
    };

    code.par_chunks_mut(w)
        .zip(sd.par_chunks_mut(w))
        .enumerate()
        .for_each(|(yi, (code_row, sd_row))| {
            let y = yi as i64;
            for x in 0..w as i64 {
                let c = at(x, y);
                let (mut bits, mut s, mut ss) = (0u32, 0.0f32, 0.0f32);
                for dy in -CENSUS_R..=CENSUS_R {
                    for dx in -CENSUS_R..=CENSUS_R {
                        let v = at(x + dx, y + dy);
                        s += v;
                        ss += v * v;
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        bits = (bits << 1) | (v < c) as u32;
                    }
                }
                let xi = x as usize;
                code_row[xi] = bits;
                sd_row[xi] = (ss / n - (s / n).powi(2)).max(0.0).sqrt();
            }
        });

    (code, sd)
}

/// Hardware population-count Hamming distance between two 24-bit Census descriptors.
#[inline(always)]
pub fn hamming_u32(a: u32, b: u32) -> u16 {
    (a ^ b).count_ones() as u16
}

/// Computes block-summed Census Hamming cost volume `[h, w, nd]` over `(2 * block + 1)^2` windows.
pub fn census_cost_volume(cl: &[u32], cr: &[u32], w: usize, h: usize, nd: usize, block: usize) -> Vec<u16> {
    assert_eq!(cl.len(), w * h);
    assert_eq!(cr.len(), w * h);
    if w == 0 || h == 0 || nd == 0 {
        return Vec::new();
    }
    let row_stride = w * nd;
    let mut ham = vec![0u16; h * row_stride];
    ham.par_chunks_mut(row_stride).enumerate().for_each(|(y, dst_row)| {
        let row_off = y * w;
        for x in 0..w {
            let base = x * nd;
            let a = cl[row_off + x];
            let top = nd.min(x + 1);
            let mut least = CENSUS_BITS;
            for d in 0..top {
                let c = hamming_u32(a, cr[row_off + x - d]);
                dst_row[base + d] = c;
                least = least.min(c);
            }
            for d in top..nd {
                dst_row[base + d] = least;
            }
        }
    });

    if block == 0 {
        return ham;
    }

    let mut rows = vec![0u16; h * row_stride];
    rows.par_chunks_mut(row_stride).enumerate().for_each(|(y, dst_row)| {
        let src_row = &ham[y * row_stride..(y + 1) * row_stride];
        for x in 0..w {
            let out = &mut dst_row[x * nd..(x + 1) * nd];
            for k in 0..=2 * block {
                let xx = (x + k).saturating_sub(block).min(w - 1);
                let src = &src_row[xx * nd..(xx + 1) * nd];
                for d in 0..nd {
                    out[d] += src[d];
                }
            }
        }
    });

    let mut out = vec![0u16; h * row_stride];
    out.par_chunks_mut(row_stride).enumerate().for_each(|(y, dst_row)| {
        for k in 0..=2 * block {
            let yy = (y + k).saturating_sub(block).min(h - 1);
            let src_row = &rows[yy * row_stride..(yy + 1) * row_stride];
            for i in 0..row_stride {
                dst_row[i] += src_row[i];
            }
        }
    });

    out
}

/// Edge-adaptive P2 lookup table indexed by `|grey(p) - grey(q)|.min(255)`.
pub fn p2_by_step(p1: u16, p2: u16, p2_edge: f32) -> [u16; 256] {
    let p2 = p2.max(p1 + 1);
    let mut lut = [p2; 256];
    if p2_edge > 0.0 {
        for (g, v) in lut.iter_mut().enumerate() {
            *v = ((p2 as f32 * p2_edge / (p2_edge + g as f32)).round() as u16).max(p1 + 1);
        }
    }
    lut
}

/// One Semi-Global DP step along an aggregation ray:
/// `L(p, d) = C(p, d) + min(L(p-r, d), L(p-r, d±1) + P1, min L(p-r) + P2) - min L(p-r)`.
#[inline]
pub fn path_step(c: &[u16], prev: Option<(&[u16], u16)>, p1: u16, p2: u16, out: &mut [u16]) -> u16 {
    let nd = c.len();
    match prev {
        None => {
            out.copy_from_slice(c);
            c.iter().copied().min().unwrap_or(0)
        }
        Some((lp, mp)) => {
            let jump = mp.saturating_add(p2);
            if nd == 0 {
                return u16::MAX;
            }
            if nd == 1 {
                let v = c[0] + lp[0].min(jump) - mp;
                out[0] = v;
                return v;
            }
            let b0 = lp[0].min(jump).min(lp[1] + p1);
            let v0 = c[0] + b0 - mp;
            out[0] = v0;
            let mut m = v0;
            for d in 1..nd - 1 {
                let best = lp[d].min(jump).min(lp[d - 1] + p1).min(lp[d + 1] + p1);
                let v = c[d] + best - mp;
                out[d] = v;
                m = m.min(v);
            }
            let last = nd - 1;
            let blast = lp[last].min(jump).min(lp[last - 1] + p1);
            let vlast = c[last] + blast - mp;
            out[last] = vlast;
            m.min(vlast)
        }
    }
}

/// 4- or 8-path Semi-Global cost aggregation `S(p, d) = sum_r L_r(p, d)` over a single left cost volume.
pub fn sgbm_aggregate(
    cost: &[u16],
    grey: &[f32],
    w: usize,
    h: usize,
    nd: usize,
    paths: u8,
    p1: u16,
    p2: u16,
    p2_edge: f32,
) -> Vec<u16> {
    assert_eq!(cost.len(), w * h * nd);
    assert_eq!(grey.len(), w * h);
    if w == 0 || h == 0 || nd == 0 {
        return Vec::new();
    }
    let lut = p2_by_step(p1, p2, p2_edge);
    let diagonals = paths >= 8;

    let run_sweep = |forward: bool| -> Vec<u16> {
        let step = |a: usize, b: usize| lut[(grey[a] - grey[b]).abs().min(255.0) as usize];
        let mut sum = vec![0u16; w * h * nd];
        let mut tmp = vec![0u16; nd];
        let mut prev: [Vec<u16>; 3] = [vec![0u16; w * nd], vec![0u16; w * nd], vec![0u16; w * nd]];
        let mut prev_min: [Vec<u16>; 3] = [vec![0u16; w], vec![0u16; w], vec![0u16; w]];
        let mut cur: [Vec<u16>; 3] = [vec![0u16; w * nd], vec![0u16; w * nd], vec![0u16; w * nd]];
        let mut cur_min: [Vec<u16>; 3] = [vec![0u16; w], vec![0u16; w], vec![0u16; w]];
        let mut along = vec![0u16; nd];
        let mut along_min = 0u16;

        for yi in 0..h {
            let y = if forward { yi } else { h - 1 - yi };
            for xi in 0..w {
                let x = if forward { xi } else { w - 1 - xi };
                let p = (y * w + x) * nd;
                let c = &cost[p..p + nd];
                along_min = if xi == 0 {
                    path_step(c, None, p1, 0, &mut tmp)
                } else {
                    let q = if forward { x.wrapping_sub(1) } else { x + 1 };
                    path_step(c, Some((&along, along_min)), p1, step(y * w + x, y * w + q), &mut tmp)
                };
                along.copy_from_slice(&tmp);
                let dst = &mut sum[p..p + nd];
                for d in 0..nd {
                    dst[d] += tmp[d];
                }
                for k in 0..3 {
                    if k != 1 && !diagonals {
                        continue;
                    }
                    let from: Option<usize> = if yi == 0 {
                        None
                    } else {
                        let s: i64 = if forward { 1 } else { -1 };
                        let fx = x as i64 + (k as i64 - 1) * s;
                        (0..w as i64).contains(&fx).then_some(fx as usize)
                    };
                    let m = match from {
                        Some(fx) => {
                            let lp = &prev[k][fx * nd..fx * nd + nd];
                            let fy = if forward { y - 1 } else { y + 1 };
                            path_step(c, Some((lp, prev_min[k][fx])), p1, step(y * w + x, fy * w + fx), &mut tmp)
                        }
                        None => path_step(c, None, p1, 0, &mut tmp),
                    };
                    cur[k][x * nd..x * nd + nd].copy_from_slice(&tmp);
                    cur_min[k][x] = m;
                    let dst = &mut sum[p..p + nd];
                    for d in 0..nd {
                        dst[d] += tmp[d];
                    }
                }
            }
            std::mem::swap(&mut prev, &mut cur);
            std::mem::swap(&mut prev_min, &mut cur_min);
        }
        sum
    };

    let (mut fwd, bwd) = rayon::join(|| run_sweep(true), || run_sweep(false));
    fwd.par_chunks_mut(w * nd)
        .zip(bwd.par_chunks(w * nd))
        .for_each(|(a, b)| {
            for (x, &y) in a.iter_mut().zip(b.iter()) {
                *x += y;
            }
        });
    fwd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn census_is_invariant_to_monotonic_gain_and_bias() {
        let (w, h) = (16usize, 12usize);
        let base: Vec<f32> = (0..w * h)
            .map(|i| (((i * 37 + (i / w) * 19) % 97) as f32) + 10.0)
            .collect();
        let darker_or_shifted: Vec<f32> = base.iter().map(|&v| v * 1.6 + 22.0).collect();

        let (c1, sd1) = census_5x5_u32(&base, w, h);
        let (c2, _sd2) = census_5x5_u32(&darker_or_shifted, w, h);
        assert_eq!(c1, c2, "strict inequality ordering must remain gain/bias invariant");
        assert!(sd1[5 * w + 5] > 10.0);
    }

    #[test]
    fn sgbm_recovers_constant_disparity_under_brightness_shift() {
        let (w, h, nd, true_d) = (40usize, 16usize, 16usize, 5usize);
        let tex = |x: i32, y: i32| -> f32 {
            let v = ((x * 29 + y * 13 + (x ^ y) * 7) % 100) as f32;
            40.0 + v
        };
        let mut left = vec![0.0f32; w * h];
        let mut right = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                left[y * w + x] = tex(x as i32, y as i32);
                right[y * w + x] = tex(x as i32 + true_d as i32, y as i32) * 0.75 + 15.0;
            }
        }
        let (cl, _) = census_5x5_u32(&left, w, h);
        let (cr, _) = census_5x5_u32(&right, w, h);
        let cost = census_cost_volume(&cl, &cr, w, h, nd, 1);
        let agg = sgbm_aggregate(&cost, &left, w, h, nd, 8, 10, 120, 12.0);
        for y in 4..h - 4 {
            for x in 8..w - 4 {
                let s = &agg[(y * w + x) * nd..(y * w + x + 1) * nd];
                let best_d = s.iter().enumerate().min_by_key(|&(_, v)| v).unwrap().0;
                assert_eq!(best_d, true_d, "wrong disparity at ({x},{y})");
            }
        }
    }
}
