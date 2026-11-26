//! Layer 2 zero-network 2D document/blueprint vision engine and native bidirectional docs AST I/O.
//!
//! - `vision`: domain-agnostic 2D pipeline (`delta`, `plan`, `layout_cache`, `phrase_cache`,
//!   `legibility`, `grounding`, `table_grid_reverse`, `lattice_verify`, `symbols`). Compiles with
//!   `--no-default-features` for vision-only hosts (`logis-vision-doc`).
//! - `docs` (`feature = "docs-io"`): hybrid AST reader producing `DocIr` + `VisionFallbackNeed`
//!   for `.hwp` (OLE2), `.hwpx` (OWPML), `.docx`, `.xlsx`/`.csv`, and Hangul `.pdf`.
//! - `writer` (`feature = "docs-io"`): `WriterGuard` + `.docx` and native `.hwpx` builders.

pub mod vision;

#[cfg(feature = "docs-io")]
pub mod docs;
#[cfg(feature = "docs-io")]
pub mod writer;
