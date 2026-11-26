//! Integration gate running ONLY diagnostic N:N matrix (`EvalSplit::DiagAll18`, `DOCS_NN_DIAG_SEED=20260501`).
//! Never executes sealed holdout during CI or `cargo test`.

use louis_docs::workflow::{evaluate_docs_nn_suite, EvalSplit};
use lv_domain::EngineMode;

#[test]
fn docs_nn_diag_all_18_packs_and_holdout_unsealed_guard() {
    // 1. Attempting holdout without --release-seal MUST fail
    assert!(evaluate_docs_nn_suite(EngineMode::RulesOnly, EvalSplit::HoldoutSeal, false).is_err());

    // 2. Run 18-pack x 6-axis diagnostic suite + all D1..D12 procedural probes
    let report = evaluate_docs_nn_suite(EngineMode::RulesOnly, EvalSplit::DiagAll18, false)
        .expect("diag suite passes");
    assert_eq!(report.exit_code, 0);
    assert_eq!(report.total_cells, 108);
    assert_eq!(report.passed_cells, 108);
    assert_eq!(report.d1_d12_probes_passed, 12);
    assert_eq!(report.hard_gate_violations, 0);
    assert!(report.max_cer <= 0.005);
    assert!(report.dsli_7 <= 0.65);
    assert!(report.dsli_18 <= 0.72);
}
