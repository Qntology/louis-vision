//! Guarded office writers (`WriterGuard` + `.docx` + native OWPML `.hwpx`).

pub mod docx_writer;
pub mod guard;
pub mod hwpx_writer;

pub use docx_writer::{build_docx, fill_docx_template_text};
pub use guard::{WriterGuard, FIELD_FORBIDDEN};
pub use hwpx_writer::{build_hwpx, NoticeSpec};
