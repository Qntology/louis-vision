//! Gate 4 output guard blocking forbidden internal terms, N2SF downgrades (`ReverseCdrContext`), external beacons, and locked-fact mutations.

use anyhow::{bail, Result};
pub use lv_sds::N2sfGrade;

/// Internal audit / risk terms never permitted in citizen-facing deliverables
/// (single source of truth anchored in Layer 0 `lv-sds`).
pub use lv_sds::CITIZEN_FORBIDDEN_TERMS as FIELD_FORBIDDEN;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReverseCdrContext {
    pub source_grade: Option<N2sfGrade>,
    pub target_grade: Option<N2sfGrade>,
    pub declassification_ref: Option<String>,
    pub stripped_covert_artifacts: usize,
    pub block_on_covert_artifacts: bool,
}

#[derive(Debug, Clone, Default)]
pub struct WriterGuard {
    pub forbidden: Vec<String>,
    pub locked_facts: Vec<String>,
    pub enforce_internal_terms: bool,
    pub reverse_cdr: Option<ReverseCdrContext>,
}

impl WriterGuard {
    pub fn citizen_guard(extra_forbidden: Vec<String>, locked_facts: Vec<String>) -> Self {
        Self {
            forbidden: extra_forbidden,
            locked_facts,
            enforce_internal_terms: true,
            reverse_cdr: None,
        }
    }

    pub fn verify_text(&self, text: &str) -> Result<()> {
        if let Some(ref cdr) = self.reverse_cdr {
            if let (Some(src), Some(dst)) = (cdr.source_grade, cdr.target_grade) {
                if src == N2sfGrade::Classified && dst == N2sfGrade::Open {
                    bail!("WriterGuard blocked document: N2SF Classified -> Open direct export forbidden");
                }
                if src > dst {
                    if cdr.block_on_covert_artifacts && cdr.stripped_covert_artifacts > 0 {
                        bail!("WriterGuard blocked document: covert artifacts remain before N2SF downgrade");
                    }
                    let has_ref = cdr
                        .declassification_ref
                        .as_deref()
                        .is_some_and(|r| !r.trim().is_empty());
                    if !has_ref {
                        bail!("WriterGuard blocked document: N2SF downgrade requires declassification_ref");
                    }
                }
            }
        }
        if (self.enforce_internal_terms || self.reverse_cdr.is_some())
            && (text.contains("http://") || text.contains("https://"))
        {
            bail!("WriterGuard blocked document: external beacon URL present");
        }
        let mut leak_count = 0usize;
        if self.enforce_internal_terms {
            leak_count += FIELD_FORBIDDEN.iter().filter(|&&t| text.contains(t)).count();
        }
        leak_count += self.forbidden.iter().filter(|t| !t.is_empty() && text.contains(t.as_str())).count();
        if leak_count > 0 {
            // Never quote the leaked strings in error messages.
            bail!("WriterGuard blocked document: {leak_count} forbidden term(s) present");
        }
        let missing = self.locked_facts.iter().filter(|f| !f.is_empty() && !text.contains(f.as_str())).count();
        if missing > 0 {
            bail!("WriterGuard blocked document: {missing} locked fact(s) missing or altered");
        }
        Ok(())
    }
}
