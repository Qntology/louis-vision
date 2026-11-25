//! PyTorch / Candle exact tanh-approximation GELU kernels (`gelu_pytorch_tanh`).
//!
//! Used by SigLIP2 and Gemma4 vision MLP blocks (`lv-core`).

use rayon::prelude::*;

const PAR_MIN_ELEMS: usize = 16_384;
const PAR_CHUNK: usize = 4_096;
const SQRT_2_OVER_PI: f32 = 0.7978845608028654;
const GELU_COEFF: f32 = 0.044715;

/// Scalar PyTorch tanh-approximation GELU:
/// `0.5 * x * (1 + tanh(sqrt(2 / pi) * (x + 0.044715 * x^3)))`.
#[inline(always)]
pub fn gelu_tanh(x: f32) -> f32 {
    0.5 * x * (1.0 + (SQRT_2_OVER_PI * (x + GELU_COEFF * x * x * x)).tanh())
}

/// Applies [`gelu_tanh`] in place, switching to Rayon chunks above `PAR_MIN_ELEMS`.
pub fn gelu_tanh_inplace(xs: &mut [f32]) {
    if xs.len() < PAR_MIN_ELEMS {
        for v in xs.iter_mut() {
            *v = gelu_tanh(*v);
        }
    } else {
        xs.par_chunks_mut(PAR_CHUNK).for_each(|chunk| {
            for v in chunk.iter_mut() {
                *v = gelu_tanh(*v);
            }
        });
    }
}

/// Out-of-place [`gelu_tanh`] mapping `src` into `dst`.
pub fn gelu_tanh_slice(src: &[f32], dst: &mut [f32]) {
    assert_eq!(src.len(), dst.len(), "gelu_tanh_slice length mismatch");
    if src.len() < PAR_MIN_ELEMS {
        for (d, &s) in dst.iter_mut().zip(src.iter()) {
            *d = gelu_tanh(s);
        }
    } else {
        dst.par_chunks_mut(PAR_CHUNK)
            .zip(src.par_chunks(PAR_CHUNK))
            .for_each(|(dc, sc)| {
                for (d, &s) in dc.iter_mut().zip(sc.iter()) {
                    *d = gelu_tanh(s);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_and_slice_match_cpu_kernels_inplace() {
        let input: Vec<f32> = (-200..=200).map(|i| i as f32 * 0.03).collect();
        let mut expected = input.clone();
        premium_discount_cpu_kernels::gelu_tanh_inplace(&mut expected);

        let mut actual_inplace = input.clone();
        gelu_tanh_inplace(&mut actual_inplace);
        let mut actual_slice = vec![0.0f32; input.len()];
        gelu_tanh_slice(&input, &mut actual_slice);

        for i in 0..input.len() {
            assert_eq!(actual_inplace[i], expected[i], "inplace mismatch at {i}");
            assert_eq!(actual_slice[i], expected[i], "slice mismatch at {i}");
            assert_eq!(gelu_tanh(input[i]), expected[i], "scalar mismatch at {i}");
        }
    }
}
