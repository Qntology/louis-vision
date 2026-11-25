//! Declarative safety & workflow interlocks + SHA-256 hash-chained `AuditChain` backed by `lv_sds`.

use std::collections::BTreeMap;

use lv_sds::audit::{Level, LogLine, GENESIS};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Passed,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterlockSource {
    Telemetry,
    Field,
    Supervisor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterlockCheck {
    Below { tag: String, value: f64 },
    AtLeast { tag: String, value: f64 },
    Loto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterlockStepSpec {
    pub no: u32,
    pub name: String,
    pub check: InterlockCheck,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepState {
    pub no: u32,
    pub name: String,
    pub check: InterlockCheck,
    pub status: StepStatus,
    pub observed: Option<f64>,
    pub source: Option<InterlockSource>,
    pub evidence: Option<String>,
}

impl StepState {
    pub fn passed(&self) -> bool {
        self.status == StepStatus::Passed
    }

    fn holds(&self, v: f64) -> bool {
        match &self.check {
            InterlockCheck::Below { value, .. } => v <= *value,
            InterlockCheck::AtLeast { value, .. } => v >= *value,
            InterlockCheck::Loto => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterlockState {
    pub name: String,
    pub steps: Vec<StepState>,
}

impl InterlockState {
    pub fn complete(&self) -> bool {
        self.steps.iter().all(StepState::passed)
    }

    pub fn next_pending(&self) -> Option<&StepState> {
        self.steps.iter().find(|s| !s.passed())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InterlockError {
    #[error("unknown interlock step {0}")]
    UnknownStep(u32),
    #[error("G3_INTERCEPT_INTERLOCK_ORDER: must complete step {pending} before step {step}")]
    OutOfOrder { step: u32, pending: u32 },
    #[error("step {step} threshold not met (observed {observed}, required)")]
    NotMet { step: u32, observed: String },
    #[error("wrong evidence kind for step {0}")]
    WrongEvidence(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum InterlockEvidence {
    Value { value: f64, how: String },
    RadioLoto { supervisor: String },
}

pub fn evaluate_interlock(
    name: impl Into<String>,
    specs: &[InterlockStepSpec],
    telemetry: &BTreeMap<String, f64>,
) -> InterlockState {
    let steps = specs
        .iter()
        .map(|sp| {
            let mut st = StepState {
                no: sp.no,
                name: sp.name.clone(),
                check: sp.check.clone(),
                status: StepStatus::Pending,
                observed: None,
                source: None,
                evidence: None,
            };
            match &sp.check {
                InterlockCheck::Below { tag, .. } | InterlockCheck::AtLeast { tag, .. } => {
                    if let Some(&v) = telemetry.get(tag) {
                        st.observed = Some(v);
                        if st.holds(v) {
                            st.status = StepStatus::Passed;
                            st.source = Some(InterlockSource::Telemetry);
                            st.evidence = Some(format!("{tag}={v}"));
                        }
                    }
                }
                InterlockCheck::Loto => {}
            }
            st
        })
        .collect();
    InterlockState { name: name.into(), steps }
}

pub fn record_interlock(
    state: &mut InterlockState,
    no: u32,
    evidence: InterlockEvidence,
) -> Result<&StepState, InterlockError> {
    let idx = state
        .steps
        .iter()
        .position(|s| s.no == no)
        .ok_or(InterlockError::UnknownStep(no))?;
    if let Some(pending) = state.steps[..idx].iter().find(|s| !s.passed()) {
        return Err(InterlockError::OutOfOrder { step: no, pending: pending.no });
    }
    let st = &mut state.steps[idx];
    match (&st.check, evidence) {
        (InterlockCheck::Below { .. } | InterlockCheck::AtLeast { .. }, InterlockEvidence::Value { value, how }) => {
            st.observed = Some(value);
            if !st.holds(value) {
                return Err(InterlockError::NotMet { step: no, observed: format!("{value}") });
            }
            st.status = StepStatus::Passed;
            st.source = Some(InterlockSource::Supervisor);
            st.evidence = Some(format!("{how}:{value}"));
        }
        (InterlockCheck::Loto, InterlockEvidence::RadioLoto { supervisor }) if !supervisor.trim().is_empty() => {
            st.status = StepStatus::Passed;
            st.source = Some(InterlockSource::Supervisor);
            st.evidence = Some(format!("LOTO:{supervisor}"));
        }
        _ => return Err(InterlockError::WrongEvidence(no)),
    }
    Ok(&state.steps[idx])
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEntry {
    pub line: LogLine,
    pub prev: String,
    pub hash: String,
}

/// Append-only SHA-256 hash-chained audit trail backed by `lv_sds::audit::LogLine`.
#[derive(Debug, Clone)]
pub struct AuditChain {
    head: String,
    entries: Vec<AuditEntry>,
}

impl Default for AuditChain {
    fn default() -> Self {
        Self { head: GENESIS.to_string(), entries: Vec::new() }
    }
}

impl AuditChain {
    pub fn append(&mut self, ts: i64, cat: &str, msg: &str, data: Option<serde_json::Value>) -> &AuditEntry {
        let seq = self.entries.len() as u64 + 1;
        let line = LogLine {
            seq,
            ts,
            tick: None,
            cat: cat.to_string(),
            level: Level::Info,
            msg: msg.to_string(),
            data,
        };
        let body = serde_json::to_string(&line).unwrap_or_default();
        let mut h = Sha256::new();
        h.update(self.head.as_bytes());
        h.update(b":");
        h.update(body.as_bytes());
        let hash = h.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>();
        let entry = AuditEntry { line, prev: self.head.clone(), hash: hash.clone() };
        self.head = hash;
        self.entries.push(entry);
        self.entries.last().expect("pushed entry")
    }

    pub fn verify(&self) -> bool {
        let mut prev = GENESIS.to_string();
        for e in &self.entries {
            if e.prev != prev {
                return false;
            }
            let body = serde_json::to_string(&e.line).unwrap_or_default();
            let mut h = Sha256::new();
            h.update(prev.as_bytes());
            h.update(b":");
            h.update(body.as_bytes());
            let want = h.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>();
            if e.hash != want {
                return false;
            }
            prev = want;
        }
        true
    }

    pub fn entries(&self) -> &[AuditEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factcheck::{draft_or_fallback, unsupported_numbers};
    use crate::gates::{check_citizen_text, check_manual_amount, judge_by_axes, officer_approve, select_form, AgentVerdict, Axis, AxisScorer, Justification, OfficerOutcome, OutputForm};
    use crate::policy::{AutomationPolicy, Disposition, DispositionNature, OperatingMode, PolicyError};
    use crate::staged::{run_staged, Commit, StagedEngine, StagedSnapshot};
    use chrono::Utc;
    use std::sync::Mutex;

    #[test]
    fn automation_policy_blocks_adverse_and_procedural_even_at_confidence_one() {
        let mut pol = AutomationPolicy::default();
        assert_eq!(pol.mode(), OperatingMode::Assist);
        assert!(matches!(pol.enable_auto(" ", "rule-1", Utc::now()), Err(PolicyError::MissingApprover)));
        pol.enable_auto("sup-01", "rule-sec-5", Utc::now()).unwrap();
        assert_eq!(pol.mode(), OperatingMode::Auto);
        assert!(matches!(
            pol.decide_disposition(DispositionNature::Adverse, Some(1.0), true, "c-1"),
            Disposition::PendingOfficer { .. }
        ));
        assert!(matches!(
            pol.decide_disposition(DispositionNature::Procedural, Some(1.0), true, "c-2"),
            Disposition::PendingOfficer { .. }
        ));
        assert!(matches!(
            pol.decide_disposition(DispositionNature::Favorable, Some(0.96), true, "c-3"),
            Disposition::AutoFinalize { .. }
        ));
    }

    #[test]
    fn gates_factcheck_staged_and_interlock_invariants() {
        assert_eq!(check_manual_amount(500_000, 600_000, 50_000), Err(AgentVerdict::G2BlockUnprovenEdit));
        assert_eq!(officer_approve(AgentVerdict::G3ViolationHighRisk, None), Err(AgentVerdict::G3InterceptRubberStamp));
        assert_eq!(
            officer_approve(AgentVerdict::G3ViolationHighRisk, Some(&Justification::ApplyLawException { team_lead_cosign: true, rule_ref: "Art-2".into() })),
            Ok(OfficerOutcome::Approved { special_audit_flag: true })
        );
        assert!(select_form(OutputForm::RejectNotice, 0, true).overridden);
        assert_eq!(check_citizen_text("LatticeDB 위장계약 박영희", &["박영희".into()]).unwrap_err().0, AgentVerdict::G4BlockInternalLeak);

        struct MockScorer;
        impl AxisScorer for MockScorer {
            fn score_pair(&self, name: &str, _: &[(String, String)], _: AgentVerdict, _: AgentVerdict) -> anyhow::Result<[f32; 2]> {
                Ok(if name == "time" { [0.97, 0.03] } else { [0.95, 0.05] })
            }
        }
        let d = judge_by_axes(
            AgentVerdict::G3EligibleNormal,
            AgentVerdict::G3ViolationHighRisk,
            &[Axis { name: "time".into(), facts: vec![] }, Axis { name: "phys".into(), facts: vec![] }],
            Some(&MockScorer),
        ).unwrap();
        assert_eq!(d.judgement.confidence, Some(0.95));

        assert!(unsupported_numbers("20만 원 지급", "원금 200,000원").is_empty());
        let (txt, ok, _) = draft_or_fallback("999원 지급", "200,000원", "200,000원 정산");
        assert!(!ok && txt == "200,000원 정산");

        // Staged lock-free CAS conflict retry
        let eng = Mutex::new(StagedEngine::new(0u32));
        let val = run_staged(
            &eng,
            |e| Ok(StagedSnapshot { rev: e.rev, input: e.state + 1 }),
            |inp| Ok(*inp * 2),
            |e, _, out| { e.state = out; Ok(Commit::Done(out)) },
        ).unwrap();
        assert_eq!(val, 2);

        // Interlock ordering + SHA-256 chain
        let specs = vec![
            InterlockStepSpec { no: 1, name: "Upstream".into(), check: InterlockCheck::Below { tag: "p".into(), value: 30.0 } },
            InterlockStepSpec { no: 2, name: "Bypass".into(), check: InterlockCheck::AtLeast { tag: "v".into(), value: 50.0 } },
            InterlockStepSpec { no: 3, name: "LOTO".into(), check: InterlockCheck::Loto },
        ];
        let mut tel = BTreeMap::new();
        tel.insert("p".into(), 28.0);
        let mut st = evaluate_interlock("AOP-103", &specs, &tel);
        assert_eq!(
            record_interlock(&mut st, 3, InterlockEvidence::RadioLoto { supervisor: "S1".into() }),
            Err(InterlockError::OutOfOrder { step: 3, pending: 2 })
        );
        record_interlock(&mut st, 2, InterlockEvidence::Value { value: 65.0, how: "radio".into() }).unwrap();
        record_interlock(&mut st, 3, InterlockEvidence::RadioLoto { supervisor: "S1".into() }).unwrap();
        assert!(st.complete());

        let mut chain = AuditChain::default();
        chain.append(1000, "interlock", "passed", None);
        assert!(chain.verify());
    }
}
