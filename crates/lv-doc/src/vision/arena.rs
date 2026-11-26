//! 4-round patch competition (`arena`) with `TitleBlock` / `Sign` protection.

pub const MAX_ROUNDS: usize = 4;
pub const MARGIN_RATIO: f32 = 0.5;
pub const MIN_CONTESTED: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellVerdict {
    pub owner: Option<usize>,
    pub margin: f32,
    pub contested: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arena {
    pub cells: Vec<CellVerdict>,
    pub alive: Vec<bool>,
    pub gate: f32,
    pub rounds: usize,
}

impl Arena {
    #[inline]
    pub fn owns(&self, kind: usize, cell: usize) -> bool {
        self.cells
            .get(cell)
            .is_some_and(|v| v.owner == Some(kind) && v.margin >= self.gate)
    }
}

fn compete(scores: &[&[f32]], alive: &[bool], active: &[bool]) -> Vec<CellVerdict> {
    (0..active.len())
        .map(|i| {
            let (mut best, mut second) = (None::<(usize, f32)>, None::<f32>);
            if active[i] {
                for (k, s) in scores.iter().enumerate() {
                    let v = s.get(i).copied().unwrap_or(0.0);
                    if !alive[k] || !v.is_finite() || v <= 0.0 {
                        continue;
                    }
                    match best {
                        Some((_, b)) if v <= b => {
                            second = Some(second.map_or(v, |x: f32| x.max(v)));
                        }
                        Some((_, b)) => {
                            second = Some(b);
                            best = Some((k, v));
                        }
                        None => best = Some((k, v)),
                    }
                }
            }
            match (best, second) {
                (Some((k, b)), Some(s)) => CellVerdict {
                    owner: Some(k),
                    margin: b - s,
                    contested: true,
                },
                (Some((k, b)), None) => CellVerdict {
                    owner: Some(k),
                    margin: b,
                    contested: false,
                },
                _ => CellVerdict {
                    owner: None,
                    margin: 0.0,
                    contested: false,
                },
            }
        })
        .collect()
}

fn median(mut v: Vec<f32>) -> f32 {
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// Runs iterative patch competition up to `MAX_ROUNDS = 4`. Protected kinds never get eliminated.
pub fn run_arena(scores: &[&[f32]], active: &[bool], protected: &[bool]) -> Arena {
    let k = scores.len();
    let mut alive = vec![true; k];
    let mut cells = compete(scores, &alive, active);
    let contested: Vec<f32> = cells.iter().filter(|c| c.contested).map(|c| c.margin).collect();
    let gate = if contested.len() < MIN_CONTESTED {
        0.0
    } else {
        median(contested) * MARGIN_RATIO
    };

    let mut rounds = 0usize;
    let mut dirty = false;
    for _ in 0..MAX_ROUNDS {
        rounds += 1;
        if dirty {
            cells = compete(scores, &alive, active);
            dirty = false;
        }
        let mut firm = vec![0usize; k];
        for c in &cells {
            if let Some(o) = c.owner.filter(|_| c.margin >= gate) {
                firm[o] += 1;
            }
        }
        let losers: Vec<usize> = (0..k)
            .filter(|&i| alive[i] && firm[i] == 0 && !protected.get(i).copied().unwrap_or(false))
            .collect();
        if losers.is_empty() {
            break;
        }
        for i in losers {
            alive[i] = false;
        }
        dirty = true;
    }
    if dirty {
        cells = compete(scores, &alive, active);
    }
    Arena {
        cells,
        alive,
        gate,
        rounds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elimination_and_protection_follow_4_round_contract() {
        let n = 8;
        let strong: Vec<f32> = (0..n).map(|i| if i < 4 { 0.9 } else { 0.1 }).collect();
        let rival: Vec<f32> = (0..n).map(|i| if i < 4 { 0.1 } else { 0.9 }).collect();
        let weak = vec![0.05f32; n];
        let got = run_arena(&[&strong, &rival, &weak], &vec![true; n], &[false, false, false]);
        assert_eq!(got.alive, vec![true, true, false]);
        assert!(got.rounds >= 2);
        assert!(got.owns(0, 0) && got.owns(1, 7));

        let prot = run_arena(&[&strong, &rival, &weak], &vec![true; n], &[false, false, true]);
        assert!(prot.alive[2], "protected kind survives without firm cells");
    }
}
