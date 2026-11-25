//! Gate 4 output guard blocking forbidden internal terms and enforcing locked-fact 1:1 preservation.

use anyhow::{bail, Result};

/// Internal audit / risk terms never permitted in citizen-facing deliverables.
pub const FIELD_FORBIDDEN: &[&str] = &["LatticeDB", "위장", "96.4%", "G3_", "jev_head", "Zoro", "마스킹"];

#[derive(Debug, Clone, Default)]
pub struct WriterGuard {
    pub forbidden: Vec<String>,
    pub locked_facts: Vec<String>,
    pub enforce_internal_terms: bool,
}

impl WriterGuard {
    pub fn citizen_guard(extra_forbidden: Vec<String>, locked_facts: Vec<String>) -> Self {
        Self { forbidden: extra_forbidden, locked_facts, enforce_internal_terms: true }
    }

    pub fn verify_text(&self, text: &str) -> Result<()> {
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
