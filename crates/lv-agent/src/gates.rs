//! 5-Gate state machine: G2 amount edit protection, G3 high-risk approval control & `judge_by_axes`,
//! and G4 internal-term / real-name outbound leak interception.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::policy::DispositionNature;

pub const FIELD_FORBIDDEN: &[&str] = &[
    "LatticeDB",
    "위장계약",
    "특별감사",
    "고위험",
    "G3Violation",
    "확실",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentVerdict {
    G1ExpiredMandatoryReject,
    G1SignInvalidRetry,
    G1InterceptForceHold,
    G2RejectContradictionLock,
    G2RequireRepairReceipt,
    G2BlockUnprovenEdit,
    G3EligibleNormal,
    G3EligibleHomonymClear,
    G3HoldSimpleReceiptMissing,
    G3ViolationHighRisk,
    G3ViolationRelatedVendorSplit,
    G3InterceptRubberStamp,
    G3InterceptInterlockOrder,
    G4BlockInternalLeak,
}

impl AgentVerdict {
    pub fn nature(self) -> DispositionNature {
        match self {
            Self::G3EligibleNormal | Self::G3EligibleHomonymClear => DispositionNature::Favorable,
            Self::G1ExpiredMandatoryReject
            | Self::G2RejectContradictionLock
            | Self::G3ViolationHighRisk
            | Self::G3ViolationRelatedVendorSplit => DispositionNature::Adverse,
            Self::G1SignInvalidRetry
            | Self::G1InterceptForceHold
            | Self::G2RequireRepairReceipt
            | Self::G2BlockUnprovenEdit
            | Self::G3HoldSimpleReceiptMissing
            | Self::G3InterceptRubberStamp
            | Self::G3InterceptInterlockOrder
            | Self::G4BlockInternalLeak => DispositionNature::Procedural,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Judgement {
    pub verdict: AgentVerdict,
    pub confidence: Option<f32>,
}

impl Judgement {
    pub fn by_rule(verdict: AgentVerdict) -> Self {
        Self { verdict, confidence: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Axis {
    pub name: String,
    pub facts: Vec<(String, String)>,
}

pub trait AxisScorer: Send + Sync {
    fn score_pair(&self, title: &str, axis_facts: &[(String, String)], rule: AgentVerdict, counter: AgentVerdict) -> Result<[f32; 2]>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decided {
    pub judgement: Judgement,
    pub axes: Vec<(String, f32)>,
}

/// Evaluates independent verification axes and takes the minimum probability as overall confidence;
/// if any axis prefers the counter verdict (`p_rule <= p_counter`), confidence collapses to `None`.
pub fn judge_by_axes(
    rule: AgentVerdict,
    counter: AgentVerdict,
    axes: &[Axis],
    model: Option<&dyn AxisScorer>,
) -> Result<Decided> {
    let Some(m) = model else {
        return Ok(Decided { judgement: Judgement::by_rule(rule), axes: Vec::new() });
    };
    let mut probs = Vec::with_capacity(axes.len());
    let mut agree = true;
    for ax in axes {
        let p = m.score_pair(&ax.name, &ax.facts, rule, counter)?;
        probs.push((ax.name.clone(), p[0]));
        if p[0] <= p[1] {
            agree = false;
        }
    }
    let lowest = probs.iter().map(|(_, v)| *v).fold(1.0f32, f32::min);
    Ok(Decided {
        judgement: Judgement {
            verdict: rule,
            confidence: (agree && !probs.is_empty()).then_some(lowest),
        },
        axes: probs,
    })
}

/// Gate 2 guard: manual reduction is allowed, upward edits require supporting evidence headroom.
pub fn check_manual_amount(verified_sum: i64, requested: i64, evidence_sum: i64) -> Result<(), AgentVerdict> {
    if requested <= verified_sum {
        return Ok(());
    }
    let headroom = verified_sum
        .checked_add(evidence_sum)
        .ok_or(AgentVerdict::G2BlockUnprovenEdit)?;
    if requested <= headroom {
        Ok(())
    } else {
        Err(AgentVerdict::G2BlockUnprovenEdit)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Justification {
    CancelMistake,
    ProveFalsePositive { evidence_scans: u32 },
    ApplyLawException { team_lead_cosign: bool, rule_ref: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfficerOutcome {
    Reverted,
    Approved { special_audit_flag: bool },
}

/// Gate 3 guard: high-risk cases cannot be rubber-stamped without proof or cosigned law exception.
pub fn officer_approve(
    verdict: AgentVerdict,
    justification: Option<&Justification>,
) -> Result<OfficerOutcome, AgentVerdict> {
    if !matches!(
        verdict,
        AgentVerdict::G3ViolationHighRisk | AgentVerdict::G3ViolationRelatedVendorSplit
    ) {
        return Ok(OfficerOutcome::Approved { special_audit_flag: false });
    }
    match justification {
        None => Err(AgentVerdict::G3InterceptRubberStamp),
        Some(Justification::CancelMistake) => Ok(OfficerOutcome::Reverted),
        Some(Justification::ProveFalsePositive { evidence_scans }) if *evidence_scans > 0 => {
            Ok(OfficerOutcome::Approved { special_audit_flag: false })
        }
        Some(Justification::ProveFalsePositive { .. }) => Err(AgentVerdict::G3InterceptRubberStamp),
        Some(Justification::ApplyLawException { team_lead_cosign: true, rule_ref })
            if !rule_ref.trim().is_empty() =>
        {
            Ok(OfficerOutcome::Approved { special_audit_flag: true })
        }
        Some(Justification::ApplyLawException { .. }) => Err(AgentVerdict::G3InterceptRubberStamp),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputForm {
    SupplementRequest,
    RejectNotice,
    ApprovalNotice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormChoice {
    pub form: OutputForm,
    pub overridden: bool,
}

pub fn select_form(requested: OutputForm, supplement_count: u32, simple_doc_missing: bool) -> FormChoice {
    if simple_doc_missing && supplement_count == 0 && requested == OutputForm::RejectNotice {
        return FormChoice { form: OutputForm::SupplementRequest, overridden: true };
    }
    FormChoice { form: requested, overridden: false }
}

/// Gate 4 outbound leak check: blocks internal terminology or third-party real names.
pub fn check_citizen_text(text: &str, extra_terms: &[String]) -> Result<(), (AgentVerdict, Vec<String>)> {
    let mut leaks: Vec<String> = FIELD_FORBIDDEN
        .iter()
        .filter(|t| text.contains(**t))
        .map(|s| (*s).to_string())
        .collect();
    for term in extra_terms {
        if !term.trim().is_empty() && text.contains(term.as_str()) {
            leaks.push(term.clone());
        }
    }
    if leaks.is_empty() {
        Ok(())
    } else {
        Err((AgentVerdict::G4BlockInternalLeak, leaks))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StageStatus {
    Done,
    Skipped,
    Corrected,
    Alert,
    Waiting,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageRecord {
    pub stage: u8,
    pub name: String,
    pub actor: String,
    pub ms: f64,
    pub status: StageStatus,
    pub summary: String,
}
