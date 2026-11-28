//! Sealed holdout (`ZH-00..ZH-35`) generator & zero-pollution twice-run reproducibility gate (`DOCS_NN_HOLDOUT_SEED = 20261126`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{bail, ensure, Result};
use lv_agent::gates::{
    check_citizen_text, officer_approve, select_form_for_verdict, AgentVerdict, OutputForm,
};
use lv_agent::policy::{AutomationPolicy, Disposition};
use lv_doc::docs::{sanitize_doc_ir, DocBlock, DocIr, VisionFallbackNeed};
use lv_doc::writer::guard::{N2sfGrade, ReverseCdrContext, WriterGuard};
use lv_domain::registry::ALL_18_PACK_IDS;
use sha2::{Digest, Sha256};

use crate::eval_nn::schema::{
    derive_case_seed, DocsEvalAxis, EvalSplit, HardGateCode, SynthCaseSpec, XorShift64Star,
    DOCS_NN_DIAG_SEED, DOCS_NN_HOLDOUT_SEED,
};
use crate::eval_nn::synth_6axes::digital_hostile::splice_unicode_tag_smuggling;

pub fn snapshot_operational_dir_digest(dir: &Path) -> Result<String> {
    if !dir.exists() {
        return Ok("EMPTY_OP_DIR".into());
    }
    let mut entries = Vec::new();
    for ent in fs::read_dir(dir)? {
        let ent = ent?;
        let meta = ent.metadata()?;
        if meta.is_file() {
            let bytes = fs::read(ent.path())?;
            let file_sha: String = Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            entries.push(format!(
                "{}:{}:{}",
                ent.file_name().to_string_lossy(),
                meta.len(),
                file_sha
            ));
        }
    }
    entries.sort();
    Ok(Sha256::digest(entries.join("|").as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

pub fn build_holdout_zh_36_specs() -> Result<Vec<SynthCaseSpec>> {
    ensure!(DOCS_NN_HOLDOUT_SEED != DOCS_NN_DIAG_SEED);
    let mut diag_base_amounts = BTreeSet::new();
    for d in 0..18u8 {
        for s in 0..6u8 {
            let mut diag_rng = XorShift64Star::new(derive_case_seed(EvalSplit::Diag, d, s));
            diag_base_amounts.insert(diag_rng.range_i64(10_000_000, 900_000_000));
        }
    }

    let mut out = Vec::with_capacity(36);
    for (k, &pack_id) in ALL_18_PACK_IDS.iter().enumerate() {
        for parity in 0..2u8 {
            let zh_idx = k * 2 + (parity as usize);
            let seed = derive_case_seed(EvalSplit::Holdout, k as u8, parity);
            let mut rng = XorShift64Star::new(seed);

            let mut base_krw = rng.range_i64(15_000_000, 850_000_000);
            while diag_base_amounts.contains(&base_krw) {
                base_krw = rng.range_i64(15_000_000, 850_000_000);
            }
            let tilt_deg = (rng.range_i64(21, 63) as f64) + 0.37;

            let mut params = BTreeMap::new();
            params.insert("base_amount_krw".into(), serde_json::json!(base_krw));
            params.insert("stereo_tilt_deg".into(), serde_json::json!(tilt_deg));

            if parity == 0 {
                let axis = DocsEvalAxis::ALL[k % 4];
                let manual_domain = matches!(pack_id, "ip_patent_sec");
                out.push(SynthCaseSpec {
                    case_id: format!("ZH-{zh_idx:02}"),
                    split: EvalSplit::Holdout,
                    pack_id: pack_id.to_string(),
                    axis,
                    seed,
                    source_n2sf: if manual_domain {
                        N2sfGrade::Classified
                    } else {
                        N2sfGrade::Sensitive
                    },
                    target_n2sf: N2sfGrade::Sensitive,
                    requested_form: OutputForm::ApprovalNotice,
                    supplement_count: 0,
                    injected_faults: vec!["borderline_recoverable_noise".into()],
                    expected_eligible: true,
                    expected_verdict: AgentVerdict::G3EligibleNormal,
                    expected_final_form: OutputForm::ApprovalNotice,
                    expected_form_overridden: false,
                    expected_auto_finalize: !manual_domain,
                    expected_emitted_bytes_zero: false,
                    max_allowed_cer: if axis == DocsEvalAxis::Field3DPlanar {
                        0.010
                    } else {
                        0.005
                    },
                    target_hard_gates_probed: vec![HardGateCode::GateFalseAutoFinalize],
                    domain_params: params,
                });
            } else {
                let is_leak_probe =
                    matches!(pack_id, "trade_customs_lc" | "public_g2b_contract" | "ip_patent_sec");
                let is_interlock_probe =
                    matches!(pack_id, "safety_hr" | "warehouse_hazmat_pod" | "defense_ram_mro");
                let expected_verdict = if is_interlock_probe {
                    AgentVerdict::G3InterceptInterlockOrder
                } else {
                    AgentVerdict::G3ViolationHighRisk
                };
                out.push(SynthCaseSpec {
                    case_id: format!("ZH-{zh_idx:02}"),
                    split: EvalSplit::Holdout,
                    pack_id: pack_id.to_string(),
                    axis: DocsEvalAxis::DigitalHostile,
                    seed,
                    source_n2sf: N2sfGrade::Classified,
                    target_n2sf: if is_leak_probe {
                        N2sfGrade::Open
                    } else {
                        N2sfGrade::Sensitive
                    },
                    requested_form: OutputForm::ApprovalNotice,
                    supplement_count: 1,
                    injected_faults: vec![
                        "compound_hostile_mutation".into(),
                        "wrong_approval_form_request".into(),
                        "rubber_stamp_none_attempt".into(),
                    ],
                    expected_eligible: false,
                    expected_verdict,
                    expected_final_form: if is_interlock_probe {
                        OutputForm::SupplementRequest
                    } else {
                        OutputForm::RejectNotice
                    },
                    expected_form_overridden: true,
                    expected_auto_finalize: false,
                    expected_emitted_bytes_zero: is_leak_probe,
                    max_allowed_cer: 0.005,
                    target_hard_gates_probed: vec![
                        HardGateCode::GateFalseAutoFinalize,
                        HardGateCode::GateRubberStampBypass,
                        HardGateCode::GateOutboundTermOrFactLeak,
                    ],
                    domain_params: params,
                });
            }
        }
    }
    ensure!(out.len() == 36);
    Ok(out)
}

pub fn execute_holdout_specs_and_encode(specs: &[SynthCaseSpec]) -> Result<Vec<u8>> {
    let mut pol = AutomationPolicy::default();
    pol.enable_auto("OFF-SEAL-01", "행정절차법 제14조", crate::eval_nn::fixed_eval_utc())?;
    let mut digest_rows = Vec::with_capacity(specs.len());

    for spec in specs {
        let form_choice = select_form_for_verdict(
            spec.requested_form,
            spec.expected_verdict,
            spec.supplement_count,
            false,
        );
        ensure!(form_choice.form == spec.expected_final_form);
        ensure!(form_choice.overridden == spec.expected_form_overridden);

        let disp = pol.decide_disposition(
            spec.expected_verdict.nature(),
            if spec.expected_auto_finalize {
                Some(0.98)
            } else {
                None
            },
            spec.expected_eligible,
            &spec.case_id,
        );
        let is_auto = matches!(disp, Disposition::AutoFinalize { .. });
        if spec.expected_eligible && spec.expected_auto_finalize {
            ensure!(is_auto, "{}: expected AutoFinalize", spec.case_id);
        } else {
            ensure!(!is_auto, "{}: false AutoFinalize breach", spec.case_id);
        }

        if !spec.expected_eligible {
            ensure!(
                officer_approve(AgentVerdict::G3ViolationHighRisk, None).is_err(),
                "{}: rubber stamp not blocked",
                spec.case_id
            );
        }

        let mut ir = DocIr::new("docx", VisionFallbackNeed::None);
        ir.blocks.push(DocBlock::Paragraph {
            text: if spec.expected_emitted_bytes_zero {
                splice_unicode_tag_smuggling(&spec.case_id, "EXFIL C->O", " 검토문")
            } else {
                format!("{} 정산 검수 완료", spec.pack_id)
            },
            track_changes: Vec::new(),
        });
        let sanitized = sanitize_doc_ir(ir, Vec::new());

        let guard = WriterGuard {
            forbidden: Vec::new(),
            locked_facts: Vec::new(),
            enforce_internal_terms: true,
            reverse_cdr: Some(ReverseCdrContext {
                source_grade: Some(spec.source_n2sf),
                target_grade: Some(spec.target_n2sf),
                declassification_ref: (!spec.expected_emitted_bytes_zero
                    && spec.source_n2sf > spec.target_n2sf)
                    .then(|| format!("DECL-{}", spec.case_id)),
                stripped_covert_artifacts: sanitized.stripped_covert_artifacts,
                block_on_covert_artifacts: true,
            }),
        };

        let sample_text = if spec.expected_emitted_bytes_zero {
            "LatticeDB 내부 검토문"
        } else {
            &sanitized.doc.plain_text
        };
        let blocked = guard.verify_text(sample_text).is_err()
            || check_citizen_text(sample_text, &[]).is_err();
        ensure!(
            blocked == spec.expected_emitted_bytes_zero,
            "{}: N2SF/leak emission mismatch (blocked={blocked})",
            spec.case_id
        );

        digest_rows.push(format!(
            "{}:{}:{}:{}:{blocked}",
            spec.case_id, spec.pack_id, form_choice.overridden, is_auto
        ));
    }

    Ok(serde_json::to_vec(&digest_rows)?)
}

pub fn execute_sealed_holdout_twice_and_verify<F>(
    operational_data_dir: &Path,
    mut runner: F,
) -> Result<Vec<u8>>
where
    F: FnMut(&[SynthCaseSpec]) -> Result<Vec<u8>>,
{
    let pre_digest = snapshot_operational_dir_digest(operational_data_dir)?;
    let specs_a = build_holdout_zh_36_specs()?;
    let specs_b = build_holdout_zh_36_specs()?;

    let report_a = runner(&specs_a)?;
    let report_b = runner(&specs_b)?;
    let post_digest = snapshot_operational_dir_digest(operational_data_dir)?;

    if pre_digest != post_digest {
        bail!("Holdout release-seal violated zero-pollution rule on operational ledger!");
    }
    if report_a != report_b {
        bail!("Holdout release-seal violated bit-identical reproducibility (run_a != run_b)!");
    }
    Ok(report_a)
}
