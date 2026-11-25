//! Leading-space preserving `SmallVec<[u8; 16]>` cursor and Shadow Top-3 visual evidence
//! intersection gate (`>= 0.08`) blocking ungrounded ledger hallucinations.

use smallvec::SmallVec;

pub const SHADOW_EVIDENCE_MIN: f32 = 0.08;

#[derive(Debug, Clone)]
pub struct ShadowToken {
    pub bytes: SmallVec<[u8; 16]>,
    pub prob: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LatticeVerifyDecision {
    Verified { matched_token: String },
    RejectedUngrounded { reason: &'static str },
}

/// Preserves exact leading ASCII whitespace into a stack-allocated `SmallVec<[u8; 16]>` cursor.
pub fn leading_space_cursor(text: &str) -> SmallVec<[u8; 16]> {
    let mut cursor = SmallVec::<[u8; 16]>::new();
    for &b in text.as_bytes() {
        if b == b' ' || b == b'\t' {
            cursor.push(b);
        } else {
            break;
        }
    }
    cursor
}

/// Compares `proposed` bytes against visual `shadow_top3` candidates without heap allocation,
/// requiring at least one intersecting token/prefix with probability >= `SHADOW_EVIDENCE_MIN` (`0.08`).
pub fn verify_ledger_with_shadow_top3(
    raw_ocr: &str,
    shadow_top3: &[ShadowToken],
    proposed_ledger: &str,
) -> LatticeVerifyDecision {
    let _cursor = leading_space_cursor(raw_ocr);
    let prop_trimmed = proposed_ledger.trim();
    if prop_trimmed.is_empty() {
        return LatticeVerifyDecision::RejectedUngrounded { reason: "empty ledger proposal" };
    }
    let prop_bytes = prop_trimmed.as_bytes();
    let mut best_token: Option<&ShadowToken> = None;

    for cand in shadow_top3.iter().take(3) {
        if cand.prob < SHADOW_EVIDENCE_MIN {
            continue;
        }
        let c_str = String::from_utf8_lossy(&cand.bytes);
        let c_trim = c_str.trim();
        if c_trim.is_empty() {
            continue;
        }
        let cb = c_trim.as_bytes();
        let intersects = prop_bytes == cb
            || prop_bytes.starts_with(cb)
            || cb.starts_with(prop_bytes)
            || prop_trimmed.eq_ignore_ascii_case(c_trim);
        if intersects && best_token.is_none_or(|b| cand.prob > b.prob) {
            best_token = Some(cand);
        }
    }

    match best_token {
        Some(t) => LatticeVerifyDecision::Verified {
            matched_token: String::from_utf8_lossy(&t.bytes).trim().to_string(),
        },
        None => LatticeVerifyDecision::RejectedUngrounded {
            reason: "no Shadow Top-3 visual evidence >= 0.08",
        },
    }
}
