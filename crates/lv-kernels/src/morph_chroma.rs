//! Chromatic ink separation (`chroma >= 45`) and separable Chebyshev morphological dilation
//! (radius 2 / 3) shared by `lv-doc` and `lv-screen`.

/// Minimum RGB channel spread (`max(R,G,B) - min(R,G,B)`) treated as colored pen / stamp ink.
pub const CHROMA_MIN: u8 = 45;

/// Returns 1 where pixel is chromatic (`max(R,G,B) - min(R,G,B) >= min_chroma`), else 0.
/// `rgb` is tightly packed `R, G, B` bytes of length `w * h * 3`.
pub fn separate_chroma_rgb(rgb: &[u8], w: usize, h: usize, min_chroma: u8) -> Vec<u8> {
    assert_eq!(rgb.len(), w * h * 3, "rgb buffer length mismatch");
    rgb.chunks_exact(3)
        .map(|px| {
            let [r, g, b] = [px[0], px[1], px[2]];
            (r.max(g).max(b) - r.min(g).min(b) >= min_chroma) as u8
        })
        .collect()
}

/// Filters chromatic pixels by an existing foreground/ink mask (`ink_mask[i] != 0`).
pub fn separate_chroma_masked(rgb: &[u8], ink_mask: &[u8], w: usize, h: usize, min_chroma: u8) -> Vec<u8> {
    assert_eq!(rgb.len(), w * h * 3, "rgb buffer length mismatch");
    assert_eq!(ink_mask.len(), w * h, "ink_mask length mismatch");
    rgb.chunks_exact(3)
        .zip(ink_mask.iter().copied())
        .map(|(px, m)| {
            let [r, g, b] = [px[0], px[1], px[2]];
            (m != 0 && r.max(g).max(b) - r.min(g).min(b) >= min_chroma) as u8
        })
        .collect()
}

/// Separable O(W * H) square (Chebyshev L-inf ball) binary dilation with sliding window counts.
/// Preserves exact semantics of `blueprint-sandbox::siglip2::ink::InkPage::dilate(radius)`
/// (`radius = 2` for pen-stroke protection, `radius = 3` for monochrome master ink delta).
pub fn dilate_binary(mask: &[u8], w: usize, h: usize, radius: u32) -> Vec<u8> {
    assert_eq!(mask.len(), w * h, "mask length mismatch");
    let r = radius as usize;
    if r == 0 || w == 0 || h == 0 {
        return mask.iter().map(|&v| (v != 0) as u8).collect();
    }

    let spread = |line: &[u8], out: &mut [u8]| {
        let n = line.len();
        let mut count: usize = line[..r.min(n)].iter().map(|&v| (v != 0) as usize).sum();
        for i in 0..n {
            if i + r < n {
                count += (line[i + r] != 0) as usize;
            }
            if i > r {
                count -= (line[i - r - 1] != 0) as usize;
            }
            out[i] = (count > 0) as u8;
        }
    };

    let mut rows = vec![0u8; w * h];
    for y in 0..h {
        let row = &mask[y * w..(y + 1) * w];
        let dst = &mut rows[y * w..(y + 1) * w];
        spread(row, dst);
    }

    let mut out = vec![0u8; w * h];
    let (mut col, mut spread_col) = (vec![0u8; h], vec![0u8; h]);
    for x in 0..w {
        for y in 0..h {
            col[y] = rows[y * w + x];
        }
        spread(&col, &mut spread_col);
        for y in 0..h {
            out[y * w + x] = spread_col[y];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chroma_separation_distinguishes_red_pen_from_gray_scan() {
        // 3 pixels: gray halftone (chroma=12), red pen (chroma=190), exact threshold (45)
        let rgb = [120, 128, 116, 220, 30, 35, 100, 100, 145];
        let ink = [1, 1, 1];
        let out = separate_chroma_masked(&rgb, &ink, 3, 1, CHROMA_MIN);
        assert_eq!(out, vec![0, 1, 1]);
    }

    #[test]
    fn dilate_binary_supports_radius_2_and_3() {
        let mut mask = vec![0u8; 9 * 9];
        mask[4 * 9 + 4] = 1;

        let d2 = dilate_binary(&mask, 9, 9, 2);
        assert_eq!(d2.iter().map(|&v| v as usize).sum::<usize>(), 25);
        assert_eq!(d2[2 * 9 + 2], 1);
        assert_eq!(d2[1 * 9 + 4], 0);

        let d3 = dilate_binary(&mask, 9, 9, 3);
        assert_eq!(d3.iter().map(|&v| v as usize).sum::<usize>(), 49);
        assert_eq!(d3[1 * 9 + 1], 1);
        assert_eq!(d3[0 * 9 + 4], 0);
    }
}
