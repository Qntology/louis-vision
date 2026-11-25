//! Perforated encoder passes (`Checker`, `Fovea`, `All`) with deterministic minesweeper opening
//! around label peaks via `lv_core::siglip2::VisionTower::extend_sparse`.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use lv_core::siglip2::{PatchGrid, VisionTower};

use crate::frame::Mask;
use crate::locate::{label_heats, LabelTexts, LocateCfg};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerfPattern {
    Checker,
    Fovea,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExpandCfg {
    pub seed_min: f32,
    pub grow_min: f32,
    pub radius: u32,
    pub seeds_per_label: usize,
    pub rounds: u32,
}

impl Default for ExpandCfg {
    fn default() -> Self {
        Self {
            seed_min: 0.15,
            grow_min: 0.30,
            radius: 2,
            seeds_per_label: 2,
            rounds: 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PerforatedCfg {
    pub pattern: PerfPattern,
    #[serde(default)]
    pub expand: Option<ExpandCfg>,
}

impl Default for PerforatedCfg {
    fn default() -> Self {
        Self {
            pattern: PerfPattern::Fovea,
            expand: Some(ExpandCfg::default()),
        }
    }
}

impl PerforatedCfg {
    pub fn parse(s: &str) -> Option<Self> {
        if s == "perf" {
            return Some(Self::default());
        }
        let (p, x) = match s.strip_suffix("-x") {
            Some(p) => (p, true),
            None => (s, false),
        };
        let pattern = match p {
            "checker" | "perf-checker" => PerfPattern::Checker,
            "fovea" | "perf-fovea" => PerfPattern::Fovea,
            "all" | "perf-all" => PerfPattern::All,
            _ => return None,
        };
        Some(Self {
            pattern,
            expand: x.then(ExpandCfg::default),
        })
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        if tag == "perf" {
            return Some(Self::default());
        }
        let rest = tag.strip_prefix("perf-")?;
        Self::parse(rest)
    }

    pub fn tag(&self) -> String {
        if *self == Self::default() {
            return "perf".to_string();
        }
        let p = match self.pattern {
            PerfPattern::Checker => "checker",
            PerfPattern::Fovea => "fovea",
            PerfPattern::All => "all",
        };
        if self.expand.is_some() {
            format!("perf-{p}-x")
        } else {
            format!("perf-{p}")
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerforatedReport {
    pub cells: u32,
    pub first: u32,
    pub opened: u32,
    pub tokens_run: u32,
    pub rounds_run: u32,
}

pub fn first_cells(cols: u32, rows: u32, pattern: PerfPattern) -> Vec<u32> {
    let mut out = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let checker = (c + r) % 2 == 0;
            let keep = match pattern {
                PerfPattern::Checker => checker,
                PerfPattern::All => true,
                PerfPattern::Fovea => {
                    let x = (c as f32 + 0.5) / cols.max(1) as f32;
                    let y = (r as f32 + 0.5) / rows.max(1) as f32;
                    if !(1.0 / 12.0..=11.0 / 12.0).contains(&y) {
                        c % 2 == 0 && r % 2 == 0
                    } else if (1.0 / 6.0..5.0 / 6.0).contains(&x)
                        && (1.0 / 6.0..5.0 / 6.0).contains(&y)
                    {
                        true
                    } else {
                        checker
                    }
                }
            };
            if keep {
                out.push(r * cols + c);
            }
        }
    }
    out
}

fn neighbours(c: u32, r: u32, cols: u32, rows: u32) -> impl Iterator<Item = (u32, u32)> {
    (-1..=1)
        .flat_map(move |dr| (-1..=1).map(move |dc| (dc, dr)))
        .filter_map(move |(dc, dr)| {
            let (cc, rr) = (c as i32 + dc, r as i32 + dr);
            ((dc, dr) != (0, 0) && cc >= 0 && rr >= 0 && cc < cols as i32 && rr < rows as i32)
                .then_some((cc as u32, rr as u32))
        })
}

pub fn seeds(heats: &[Vec<f32>], cols: u32, rows: u32, cfg: &ExpandCfg) -> Vec<u32> {
    let mut out = Vec::new();
    for h in heats {
        let mut local: Vec<(u32, f32)> = (0..rows)
            .flat_map(|r| (0..cols).map(move |c| (c, r)))
            .filter_map(|(c, r)| {
                let v = h[(r * cols + c) as usize];
                let peak = v.is_finite()
                    && v >= cfg.seed_min
                    && neighbours(c, r, cols, rows).all(|(cc, rr)| {
                        h[(rr * cols + cc) as usize].partial_cmp(&v)
                            != Some(std::cmp::Ordering::Greater)
                    });
                peak.then_some((r * cols + c, v))
            })
            .collect();
        local.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        out.extend(local.into_iter().take(cfg.seeds_per_label).map(|(i, _)| i));
    }
    out.sort_unstable();
    out.dedup();
    out
}

pub fn cells_to_open(
    heats: &[Vec<f32>],
    open: &[bool],
    seed_cells: &[u32],
    cols: u32,
    rows: u32,
    round: u32,
    cfg: &ExpandCfg,
) -> Vec<u32> {
    let mut want = vec![false; open.len()];
    let near_seed = |c: u32, r: u32| {
        seed_cells.iter().any(|&s| {
            (c as i32 - (s % cols) as i32)
                .abs()
                .max((r as i32 - (s / cols) as i32).abs())
                <= cfg.radius as i32
        })
    };
    if round == 0 {
        for &s in seed_cells {
            for (cc, rr) in neighbours(s % cols, s / cols, cols, rows) {
                want[(rr * cols + cc) as usize] = true;
            }
        }
    } else {
        for r in 0..rows {
            for c in 0..cols {
                let i = (r * cols + c) as usize;
                if !open[i] || !near_seed(c, r) || !heats.iter().any(|h| h[i] >= cfg.grow_min) {
                    continue;
                }
                for (cc, rr) in neighbours(c, r, cols, rows) {
                    if near_seed(cc, rr) {
                        want[(rr * cols + cc) as usize] = true;
                    }
                }
            }
        }
    }
    (0..open.len())
        .filter(|&i| want[i] && !open[i])
        .map(|i| i as u32)
        .collect()
}

pub fn fill_holes(values: &mut [f32], open: &[bool], cols: u32, rows: u32) {
    let floor = values
        .iter()
        .zip(open)
        .filter(|(_, o)| **o)
        .map(|(v, _)| *v)
        .fold(f32::INFINITY, f32::min);
    let floor = if floor.is_finite() { floor } else { 0.0 };
    let snap = values.to_vec();
    for r in 0..rows {
        for c in 0..cols {
            let i = (r * cols + c) as usize;
            if open[i] {
                continue;
            }
            let min_nbr = neighbours(c, r, cols, rows)
                .filter(|&(cc, rr)| open[(rr * cols + cc) as usize])
                .map(|(cc, rr)| snap[(rr * cols + cc) as usize])
                .fold(f32::INFINITY, f32::min);
            values[i] = if min_nbr.is_finite() { min_nbr } else { floor };
        }
    }
}

pub type LabelHeatsOut = (Vec<(String, crate::locate::Heat, f32)>, PerforatedReport);

pub fn perforated_heats(
    tower: &VisionTower,
    grid: &PatchGrid,
    texts: &LabelTexts,
    locate: &LocateCfg,
    cfg: &PerforatedCfg,
    seed_mask: Option<&Mask>,
    logit_scale: f32,
    logit_bias: f32,
) -> Result<LabelHeatsOut> {
    let _ = seed_mask;
    let cols = grid.grid_w as u32;
    let rows = grid.grid_h as u32;
    let total = (cols * rows) as usize;
    let first = first_cells(cols, rows, cfg.pattern);
    let mut pass = tower.forward_sparse(grid, &first)?;
    let mut open = vec![false; total];
    for &c in &first {
        open[c as usize] = true;
    }
    let mut tokens_run = first.len() as u32;
    let mut opened = 0u32;
    let mut rounds_run = 0u32;

    let mut dense = vec![vec![0.0f32; tower.cfg.hidden]; total];
    for (&cell, feat) in pass.cells.iter().zip(tower.dense(&pass)?) {
        dense[cell as usize] = feat;
    }

    let mut heats_res = label_heats(&dense, cols, rows, texts, locate, logit_scale, logit_bias);
    if let Some(exp) = cfg.expand {
        let mut heat_vecs: Vec<Vec<f32>> = heats_res.iter().map(|(_, h, _)| h.values.clone()).collect();
        for (hv, _) in heat_vecs.iter_mut().zip(0..) {
            for (i, &o) in open.iter().enumerate() {
                if !o {
                    hv[i] = f32::NAN;
                }
            }
        }
        let seed_cells = seeds(&heat_vecs, cols, rows, &exp);
        for round in 0..exp.rounds {
            let todo = cells_to_open(&heat_vecs, &open, &seed_cells, cols, rows, round, &exp);
            if todo.is_empty() {
                break;
            }
            let ext = tower.extend_sparse(grid, &pass, &todo)?;
            tokens_run += todo.len() as u32;
            opened += todo.len() as u32;
            rounds_run += 1;
            for (&c, f) in ext.cells.iter().zip(tower.dense(&ext)?) {
                open[c as usize] = true;
                dense[c as usize] = f;
            }
            pass = tower.merge_sparse(&pass, &ext)?;
            heats_res = label_heats(&dense, cols, rows, texts, locate, logit_scale, logit_bias);
            heat_vecs = heats_res
                .iter()
                .map(|(_, h, _)| {
                    h.values
                        .iter()
                        .zip(&open)
                        .map(|(&v, &o)| if o { v } else { f32::NAN })
                        .collect()
                })
                .collect();
        }
    }

    for (_, h, _) in &mut heats_res {
        fill_holes(&mut h.values, &open, cols, rows);
    }

    Ok((
        heats_res,
        PerforatedReport {
            cells: total as u32,
            first: first.len() as u32,
            opened,
            tokens_run,
            rounds_run,
        },
    ))
}
