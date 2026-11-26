//! Layer 3 5-Gate workflow engine, automation policy guardrails, fact-locking,
//! lock-free staged inference (`Prepare -> Infer -> Commit`), and declarative safety interlocks.

pub mod embed_scorer;
pub mod factcheck;
pub mod gates;
pub mod interlock;
pub mod policy;
pub mod staged;

pub use embed_scorer::{
    AxisAnchorSet, DeterministicNanoTieBreaker, EmbedAxisScorer, MarginBreakdown,
    TieBreakEvidencePrompt, TieBreakPick, TieBreakSelector,
};
pub use gates::{
    check_citizen_text, check_manual_amount, judge_by_axes, officer_approve, select_form,
    select_form_for_verdict, AgentVerdict, Axis, Decided, FormChoice, Judgement, Justification,
    OfficerOutcome, OutputForm, StageRecord, StageStatus,
};
pub use interlock::{
    evaluate_interlock, record_interlock, AuditChain, InterlockCheck, InterlockError,
    InterlockSource, InterlockState, InterlockStepSpec, StepState, StepStatus,
};
pub use policy::{
    AutoAuthorization, AutomationPolicy, Disposition, DispositionNature, OperatingMode,
    PolicyError, AUTO_CONFIDENCE_FLOOR, POST_AUDIT_MOD,
};
pub use staged::{run_staged, Commit, EngineAccess, StagedEngine};
