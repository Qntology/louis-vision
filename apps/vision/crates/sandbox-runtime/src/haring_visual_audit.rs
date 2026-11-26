//! `Gate UI-HARING` pixel & 8-connected CCL visual invariant verifier across 0/45/90/Top views:
//! - >= 99.5% 2px-eroded flat interior color purity (zero robot joint spheres/visors/shading),
//! - 0 px `#000000` outline leakage between body silhouette and background,
//! - >= 4 non-touching floating motion tick components (`cnt >= 8`) in 0/45/Top views.

use std::collections::VecDeque;

use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HaringVisualAuditReport {
    pub angle_deg: u16,
    pub body_color_purity_ratio: f32,
    pub outline_gap_pixels: usize,
    pub floating_tick_components: usize,
}

#[inline]
fn is_pure_black_ink(rgb: [u8; 3]) -> bool {
    rgb[0] <= 12 && rgb[1] <= 12 && rgb[2] <= 12
}

#[inline]
fn color_dist_l1(a: [u8; 3], b: [u8; 3]) -> u16 {
    (a[0].abs_diff(b[0]) as u16) + (a[1].abs_diff(b[1]) as u16) + (a[2].abs_diff(b[2]) as u16)
}

pub fn verify_haring_screenshot_invariants(
    rgb: &[u8],
    width: usize,
    height: usize,
    angle_deg: u16,
    expected_body_rgb: [u8; 3],
    bg_rgb: [u8; 3],
    expect_dancing_ticks: bool,
) -> Result<HaringVisualAuditReport> {
    ensure!(
        rgb.len() == width * height * 3 && width >= 128 && height >= 128,
        "invalid Haring audit image buffer"
    );
    let px_at = |x: usize, y: usize| -> [u8; 3] {
        let i = (y * width + x) * 3;
        [rgb[i], rgb[i + 1], rgb[i + 2]]
    };

    let mut cls = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let c = px_at(x, y);
            cls[y * width + x] = if is_pure_black_ink(c) {
                1
            } else if color_dist_l1(c, bg_rgb) <= 10 {
                0
            } else {
                2
            };
        }
    }

    // 1. 2px-eroded interior purity >= 99.5%
    let (mut eroded_total, mut eroded_pure) = (0usize, 0usize);
    for y in 2..(height - 2) {
        for x in 2..(width - 2) {
            let inside = (-2isize..=2).all(|dy| {
                (-2isize..=2).all(|dx| {
                    cls[(y as isize + dy) as usize * width + (x as isize + dx) as usize] == 2
                })
            });
            if inside {
                eroded_total += 1;
                if color_dist_l1(px_at(x, y), expected_body_rgb) <= 6 {
                    eroded_pure += 1;
                }
            }
        }
    }
    ensure!(eroded_total > 50, "Haring silhouette interior too small ({eroded_total})");
    let purity = eroded_pure as f32 / eroded_total as f32;
    ensure!(purity >= 0.995, "[Gate UI-HARING] interior purity {purity:.4} < 0.995");

    // 2. #000000 ink outline seal (zero 4-neighbor contact between body class 2 and background class 0)
    let mut outline_gap_pixels = 0usize;
    for y in 1..(height - 1) {
        for x in 1..(width - 1) {
            if cls[y * width + x] == 2 {
                for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                    if cls[ny * width + nx] == 0 {
                        outline_gap_pixels += 1;
                    }
                }
            }
        }
    }
    ensure!(
        outline_gap_pixels == 0,
        "[Gate UI-HARING] broken black ink outline ({outline_gap_pixels} px)"
    );

    // 3. 8-connected CCL: verify >= 4 air-gapped floating ticks AND 0 interior black ink blotches
    let mut visited = vec![false; width * height];
    let mut floating_ticks = 0usize;
    let mut interior_ink_components = 0usize;
    let mut q = VecDeque::new();
    for y in 0..height {
        for x in 0..width {
            let root = y * width + x;
            if cls[root] != 1 || visited[root] {
                continue;
            }
            visited[root] = true;
            q.push_back((x, y));
            let (mut cnt, mut touches_body, mut touches_bg) = (0usize, false, false);
            while let Some((cx, cy)) = q.pop_front() {
                cnt += 1;
                for dy in -1isize..=1 {
                    for dx in -1isize..=1 {
                        let (nx, ny) = (cx as isize + dx, cy as isize + dy);
                        if nx < 0 || ny < 0 || nx >= width as isize || ny >= height as isize {
                            continue;
                        }
                        let ni = (ny as usize) * width + (nx as usize);
                        if cls[ni] == 2 {
                            touches_body = true;
                        } else if cls[ni] == 0 {
                            touches_bg = true;
                        } else if cls[ni] == 1 && !visited[ni] {
                            visited[ni] = true;
                            q.push_back((nx as usize, ny as usize));
                        }
                    }
                }
            }
            if touches_body && !touches_bg {
                interior_ink_components += 1;
            }
            if cnt >= 8 && !touches_body {
                floating_ticks += 1;
            }
        }
    }
    ensure!(
        interior_ink_components == 0,
        "[Gate UI-HARING] interior black visor/joint ink components ({interior_ink_components})"
    );
    if expect_dancing_ticks && angle_deg != 90 {
        ensure!(
            floating_ticks >= 4,
            "[Gate UI-HARING] floating motion ticks {floating_ticks} < 4"
        );
    }

    Ok(HaringVisualAuditReport {
        angle_deg,
        body_color_purity_ratio: purity,
        outline_gap_pixels,
        floating_tick_components: floating_ticks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_core::rig::haring_dancer::{build_haring_dancer_rig, render_haring_orthographic_rgb};

    #[test]
    fn haring_4view_passes_and_3_counterexamples_fail() {
        let body = [255u8, 196, 0];
        let bg = [242u8, 244, 247];
        let rig = build_haring_dancer_rig("HARING-01", "0x742d35cc6634c0532925a3b844bc454e4438f44e", body, 1.0);

        for angle in [0u16, 45, 90, 180] {
            let buf = render_haring_orthographic_rgb(&rig, 256, 256, angle, bg);
            let rep = verify_haring_screenshot_invariants(&buf, 256, 256, angle, body, bg, true)
                .unwrap_or_else(|e| panic!("angle {angle} failed: {e}"));
            assert!(rep.body_color_purity_ratio >= 0.995);
            assert_eq!(rep.outline_gap_pixels, 0);
            if angle != 90 {
                assert!(rep.floating_tick_components >= 4);
            }
        }

        // Counterexample 1a: black interior visor/joint blotch inside body
        let mut dirty_visor = render_haring_orthographic_rgb(&rig, 256, 256, 0, bg);
        for y in 70..86 {
            for x in 120..136 {
                let o = (y * 256 + x) * 3;
                dirty_visor[o..o + 3].copy_from_slice(&[2, 2, 2]);
            }
        }
        assert!(verify_haring_screenshot_invariants(&dirty_visor, 256, 256, 0, body, bg, true).is_err());

        // Counterexample 1b: shaded gray joint sphere blotch pollutes interior purity (< 0.995)
        let mut shaded_joint = render_haring_orthographic_rgb(&rig, 256, 256, 0, bg);
        for y in 70..86 {
            for x in 120..136 {
                let o = (y * 256 + x) * 3;
                shaded_joint[o..o + 3].copy_from_slice(&[120, 120, 120]);
            }
        }
        assert!(verify_haring_screenshot_invariants(&shaded_joint, 256, 256, 0, body, bg, true).is_err());

        // Counterexample 2: 2px outline breach exposes body directly to bg
        let mut broken_outline = render_haring_orthographic_rgb(&rig, 256, 256, 0, bg);
        'outer: for y in 1..255 {
            for x in 1..255 {
                let o = (y * 256 + x) * 3;
                if broken_outline[o..o + 3] == body && broken_outline[((y - 4) * 256 + x) * 3..((y - 4) * 256 + x) * 3 + 3] == bg {
                    for dy in 1..=3 {
                        let lo = ((y - dy) * 256 + x) * 3;
                        broken_outline[lo..lo + 3].copy_from_slice(&bg);
                    }
                    break 'outer;
                }
            }
        }
        assert!(verify_haring_screenshot_invariants(&broken_outline, 256, 256, 0, body, bg, true).is_err());

        // Counterexample 3: missing floating motion ticks (< 4)
        let mut no_ticks = rig.clone();
        no_ticks.motion_ticks.clear();
        let flat_buf = render_haring_orthographic_rgb(&no_ticks, 256, 256, 0, bg);
        assert!(verify_haring_screenshot_invariants(&flat_buf, 256, 256, 0, body, bg, true).is_err());
    }
}
