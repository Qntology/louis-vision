//! Horn quaternion rigid (`s = 1.0`) and Umeyama similarity alignment (`MPJPE`, `PA-MPJPE`).

pub type V3 = [f64; 3];
pub type M3 = [[f64; 3]; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn norm(a: V3) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn top_eigenvector4(mut a: [[f64; 4]; 4]) -> [f64; 4] {
    let mut v = [[0.0; 4]; 4];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for _ in 0..64 {
        let off: f64 = (0..4)
            .flat_map(|i| (0..4).filter(move |j| *j != i).map(move |j| (i, j)))
            .map(|(i, j)| a[i][j] * a[i][j])
            .sum();
        if off < 1e-22 {
            break;
        }
        for p in 0..4 {
            for q in p + 1..4 {
                if a[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = if theta == 0.0 {
                    1.0
                } else {
                    theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt())
                };
                let (c, s) = (1.0 / (t * t + 1.0).sqrt(), t / (t * t + 1.0).sqrt());
                for k in 0..4 {
                    let (akp, akq) = (a[k][p], a[k][q]);
                    a[k][p] = c * akp - s * akq;
                    a[k][q] = s * akp + c * akq;
                }
                for k in 0..4 {
                    let (apk, aqk) = (a[p][k], a[q][k]);
                    a[p][k] = c * apk - s * aqk;
                    a[q][k] = s * apk + c * aqk;
                }
                for row in v.iter_mut() {
                    let (vp, vq) = (row[p], row[q]);
                    row[p] = c * vp - s * vq;
                    row[q] = s * vp + c * vq;
                }
            }
        }
    }
    let k = (0..4).max_by(|&i, &j| a[i][i].total_cmp(&a[j][j])).unwrap_or(0);
    [v[0][k], v[1][k], v[2][k], v[3][k]]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Similarity {
    pub s: f64,
    pub r: M3,
    pub t: V3,
}

impl Similarity {
    pub fn apply(&self, x: V3) -> V3 {
        [0, 1, 2].map(|i| self.s * (0..3).map(|j| self.r[i][j] * x[j]).sum::<f64>() + self.t[i])
    }
}

pub fn align_similarity(src: &[V3], dst: &[V3]) -> Option<Similarity> {
    let n = src.len();
    if n < 3 || dst.len() != n {
        return None;
    }
    let mean = |v: &[V3]| [0, 1, 2].map(|i| v.iter().map(|p| p[i]).sum::<f64>() / n as f64);
    let (mp, mq) = (mean(src), mean(dst));
    let mut s = [[0.0; 3]; 3];
    let mut var = 0.0;
    for (p, q) in src.iter().zip(dst) {
        let (a, b) = (sub(*p, mp), sub(*q, mq));
        var += a[0] * a[0] + a[1] * a[1] + a[2] * a[2];
        for i in 0..3 {
            for j in 0..3 {
                s[i][j] += a[i] * b[j];
            }
        }
    }
    if var < 1e-18 {
        return None;
    }
    let (sxx, sxy, sxz, syx, syy, syz, szx, szy, szz) =
        (s[0][0], s[0][1], s[0][2], s[1][0], s[1][1], s[1][2], s[2][0], s[2][1], s[2][2]);
    let nm = [
        [sxx + syy + szz, syz - szy, szx - sxz, sxy - syx],
        [syz - szy, sxx - syy - szz, sxy + syx, szx + sxz],
        [szx - sxz, sxy + syx, -sxx + syy - szz, syz + szy],
        [sxy - syx, szx + sxz, syz + szy, -sxx - syy + szz],
    ];
    let [w, x, y, z] = top_eigenvector4(nm);
    let qn = (w * w + x * x + y * y + z * z).sqrt().max(1e-300);
    let (w, x, y, z) = (w / qn, x / qn, y / qn, z / qn);
    let r = [
        [w * w + x * x - y * y - z * z, 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
        [2.0 * (x * y + w * z), w * w - x * x + y * y - z * z, 2.0 * (y * z - w * x)],
        [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), w * w - x * x - y * y + z * z],
    ];
    let mut num = 0.0;
    for (p, q) in src.iter().zip(dst) {
        let (a, b) = (sub(*p, mp), sub(*q, mq));
        let ra = [0, 1, 2].map(|i| (0..3).map(|j| r[i][j] * a[j]).sum::<f64>());
        num += ra[0] * b[0] + ra[1] * b[1] + ra[2] * b[2];
    }
    let scale = num / var;
    let rm = [0, 1, 2].map(|i| (0..3).map(|j| r[i][j] * mp[j]).sum::<f64>());
    Some(Similarity { s: scale, r, t: [0, 1, 2].map(|i| mq[i] - scale * rm[i]) })
}

pub fn align_rigid(src: &[V3], dst: &[V3]) -> Option<Similarity> {
    let sim = align_similarity(src, dst)?;
    let n = src.len() as f64;
    let mean = |v: &[V3]| [0, 1, 2].map(|i| v.iter().map(|p| p[i]).sum::<f64>() / n);
    let (mp, mq) = (mean(src), mean(dst));
    let rm = [0, 1, 2].map(|i| (0..3).map(|j| sim.r[i][j] * mp[j]).sum::<f64>());
    Some(Similarity { s: 1.0, r: sim.r, t: [0, 1, 2].map(|i| mq[i] - rm[i]) })
}

pub fn pa_mpjpe(pred: &[V3], reference: &[V3]) -> Option<f64> {
    let sim = align_similarity(pred, reference)?;
    Some(pred.iter().zip(reference).map(|(p, q)| norm(sub(sim.apply(*p), *q))).sum::<f64>() / pred.len() as f64)
}

pub fn mpjpe(pred: &[V3], reference: &[V3]) -> f64 {
    pred.iter().zip(reference).map(|(p, q)| norm(sub(*p, *q))).sum::<f64>() / pred.len().max(1) as f64
}
