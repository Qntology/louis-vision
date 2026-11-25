//! Layer 0 SIMD & vision primitives for `louis-vision`.
//!
//! Re-exports `premium_discount_cpu_kernels` (`PackedW` NR=32 FMA GEMM, streaming attention,
//! HWC u8 patchify, `q8` bufferless dot/prejudice kernels) without duplicating code, and adds
//! vision-specific primitives needed by `lv-core`, `lv-doc`, `lv-screen`, and `lv-spatial`.

pub mod census;
pub mod gelu_tanh;
pub mod halo;
pub mod morph_chroma;

pub use premium_discount_cpu_kernels::*;
