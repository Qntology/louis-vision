//! 1-Hop Halo (3x3 max-pool) pruning index computation for vision token grids.
//!
//! Protects all 8 neighbors around any active merge-group (`hm x wm`) and retains a
//! representative blank group so dense-grid scatter reconstruction preserves spatial alignment.

/// Index plan produced for a pruned `[hm, wm]` merge grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HaloPlan {
    pub kept_patch_indices: Vec<u32>,
    pub kept_group_indices: Vec<u32>,
    pub rep_blank_group: u32,
    pub total_groups: usize,
}

/// Computes 3x3 max-pool (1-hop Chebyshev radius 1) boolean mask over an `[hm, wm]` grid.
pub fn halo_1hop_mask(active: &[bool], hm: usize, wm: usize) -> Vec<bool> {
    assert_eq!(active.len(), hm * wm, "active grid length mismatch");
    let mut keep = vec![false; hm * wm];
    for r in 0..hm {
        for c in 0..wm {
            if !active[r * wm + c] {
                continue;
            }
            let r0 = r.saturating_sub(1);
            let r1 = (r + 1).min(hm - 1);
            let c0 = c.saturating_sub(1);
            let c1 = (c + 1).min(wm - 1);
            for rr in r0..=r1 {
                for cc in c0..=c1 {
                    keep[rr * wm + cc] = true;
                }
            }
        }
    }
    keep
}

/// Computes `HaloPlan` over flattened patch tensor slice `patches`
/// (`(hm * merge_size) * (wm * merge_size) * patch_len` floats).
/// Returns `None` if the grid is too small (`<= 32` groups), completely blank, or too dense (`>= 85%` kept).
pub fn plan_1hop_halo_indices(
    patches: &[f32],
    grid_h: usize,
    grid_w: usize,
    merge_size: usize,
    patch_len: usize,
    ink_delta_thresh: f32,
) -> Option<HaloPlan> {
    let m = merge_size.max(1);
    let (hm, wm) = (grid_h / m, grid_w / m);
    let total_groups = hm * wm;
    if total_groups <= 32 {
        return None;
    }

    let mm = m * m;
    let group_elems = mm * patch_len;
    if patches.len() < total_groups * group_elems {
        return None;
    }

    let mut active = vec![false; total_groups];
    for g in 0..total_groups {
        let slice = &patches[g * group_elems..(g + 1) * group_elems];
        let (mut mn, mut mx) = (f32::INFINITY, f32::NEG_INFINITY);
        for &v in slice {
            if v < mn {
                mn = v;
            }
            if v > mx {
                mx = v;
            }
        }
        active[g] = (mx - mn) > ink_delta_thresh;
    }

    let mut keep = halo_1hop_mask(&active, hm, wm);
    let rep_blank = keep.iter().position(|&k| !k)?;
    keep[rep_blank] = true;

    let kept_count = keep.iter().filter(|&&k| k).count();
    if kept_count * 100 >= total_groups * 85 {
        return None;
    }

    let mut kept_group_indices = Vec::with_capacity(kept_count);
    let mut kept_patch_indices = Vec::with_capacity(kept_count * mm);
    for (g, &k) in keep.iter().enumerate() {
        if k {
            kept_group_indices.push(g as u32);
            let base = (g * mm) as u32;
            for j in 0..mm as u32 {
                kept_patch_indices.push(base + j);
            }
        }
    }

    Some(HaloPlan {
        kept_patch_indices,
        kept_group_indices,
        rep_blank_group: rep_blank as u32,
        total_groups,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halo_1hop_protects_all_8_neighbors_and_skips_dense_or_small() {
        let small = vec![0.0f32; 16 * 4 * 3];
        assert!(plan_1hop_halo_indices(&small, 8, 8, 2, 3, 0.05).is_none());

        let (hm, wm, merge, plen) = (10usize, 10usize, 2usize, 4usize);
        let group_elems = merge * merge * plen;
        let mut patches = vec![1.0f32; hm * wm * group_elems];
        let center = 5 * wm + 5;
        patches[center * group_elems] = -1.0;

        let plan = plan_1hop_halo_indices(&patches, hm * merge, wm * merge, merge, plen, 0.05)
            .expect("sparse center ink triggers halo pruning");
        assert_eq!(plan.kept_group_indices.len(), 10);
        for dr in 4..=6 {
            for dc in 4..=6 {
                let g = (dr * wm + dc) as u32;
                assert!(plan.kept_group_indices.contains(&g), "missing neighbor ({dr},{dc})");
            }
        }
    }
}
