//! Weight-free geometric mask refinement & UI seeding (`holes`, `dots`, `border_fix`, `grow_shrink`,
//! `PointStore`, 21-color `PALETTE`), promoted from `sammie-core` with OpenCV 4.14 bit-exact parity.

pub mod palette;
pub mod points;
pub mod postprocess;

pub use palette::{hex, object_color, PALETTE};
pub use points::{DataPoint, PointStore, Prompts};
pub use postprocess::{
    apply, apply_border_fix, fill_small_holes, grow_shrink, remove_small_dots, PostprocessError,
    PostprocessParams,
};
