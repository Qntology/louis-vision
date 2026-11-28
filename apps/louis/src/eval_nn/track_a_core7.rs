//! Track A (`DOM-01..DOM-07`): `D1..D6` and `D9` (multipage scale carry & superscripts) deterministic probes.

use std::collections::BTreeMap;

use anyhow::{ensure, Result};
use lv_agent::factcheck::draft_or_fallback;
use lv_agent::gates::{
    check_citizen_text, check_manual_amount, officer_approve, select_form_for_verdict,
    AgentVerdict, Axis, Justification, OfficerOutcome, OutputForm,
};
use lv_agent::interlock::{
    evaluate_interlock, record_interlock, InterlockCheck, InterlockError, InterlockEvidence,
    InterlockStepSpec,
};
use lv_agent::policy::{AutomationPolicy, Disposition};
use lv_doc::writer::guard::{N2sfGrade, ReverseCdrContext, WriterGuard};
use lv_doc::writer::{build_hwpx, NoticeSpec};
use lv_domain::{repair_code, CodeScheme, DocsDomainPack};
use lv_sds::CITIZEN_FORBIDDEN_TERMS;

use crate::eval_nn::synth_6axes::digital_hostile::{
    synth_hostile_docx_bytes, synth_semantic_multipage_docx_bytes,
};

pub fn run_track_a_d1_to_d6_probes() -> Result<()> {
    // [D1] Hostile <w:del> unit-price hike detection + +1 KRW manual edit block (G2BlockUnprovenEdit)
    let hostile_iv = synth_hostile_docx_bytes(
        "구매발주 PO-2026-401 정산 송장",
        Some("10,000,000"),
        Some("15,000,000"),
        None,
        None,
    )?;
    let parsed_iv = lv_doc::docs::parse_docs_bytes("docx", &hostile_iv)?;
    ensure!(!parsed_iv.track_changes.is_empty());
    ensure!(check_manual_amount(80_000_000, 85_000_000, 5_000_000).is_ok());
    ensure!(
        check_manual_amount(80_000_000, 85_000_001, 5_000_000)
            == Err(AgentVerdict::G2BlockUnprovenEdit)
    );

    // [D2] Related-vendor split rubber-stamp block vs team-lead law exception cosign & homonym check
    ensure!(
        officer_approve(AgentVerdict::G3ViolationRelatedVendorSplit, None)
            == Err(AgentVerdict::G3InterceptRubberStamp)
    );
    let _homonym_axis = Axis {
        name: "biz_reg_distinct".into(),
        facts: vec![("biz_reg".into(), "410-86-99999".into())],
    };
    let cosigned = officer_approve(
        AgentVerdict::G3ViolationRelatedVendorSplit,
        Some(&Justification::ApplyLawException {
            team_lead_cosign: true,
            rule_ref: "하도급법 제13조제1항 단서 확인".into(),
        }),
    )
    .map_err(|e| anyhow::anyhow!("unexpected interlock/approval verdict {e:?}"))?;
    ensure!(cosigned == OfficerOutcome::Approved { special_audit_flag: true });

    // [D3-Interlock] KKS repair + Occupational Safety Act Art. 92 LOTO 3-step sequential interlock
    ensure!(repair_code("1O-laa-10-aa-123", CodeScheme::Kks) == "10LAA10AA123");
    let mut loto_state = evaluate_interlock(
        "DOM07-PTW-BOILER-FEED",
        &[
            InterlockStepSpec {
                no: 1,
                name: "주증기 배관 감압 확인".into(),
                check: InterlockCheck::Below {
                    tag: "steam_pressure_bar".into(),
                    value: 0.5,
                },
            },
            InterlockStepSpec {
                no: 2,
                name: "밀폐공간 산소농도 측정".into(),
                check: InterlockCheck::AtLeast {
                    tag: "purge_o2_pct".into(),
                    value: 19.5,
                },
            },
            InterlockStepSpec {
                no: 3,
                name: "주전원 차단기 LOTO 체결".into(),
                check: InterlockCheck::Loto,
            },
        ],
        &BTreeMap::new(),
    );
    ensure!(
        record_interlock(
            &mut loto_state,
            3,
            InterlockEvidence::RadioLoto {
                supervisor: "SUP-09".into(),
            }
        ) == Err(InterlockError::OutOfOrder {
            step: 3,
            pending: 1
        })
    );
    record_interlock(
        &mut loto_state,
        1,
        InterlockEvidence::Value {
            value: 0.2,
            how: "gauge".into(),
        },
    )?;
    record_interlock(
        &mut loto_state,
        2,
        InterlockEvidence::Value {
            value: 20.8,
            how: "sensor".into(),
        },
    )?;
    record_interlock(
        &mut loto_state,
        3,
        InterlockEvidence::RadioLoto {
            supervisor: "SUP-09".into(),
        },
    )?;
    ensure!(loto_state.complete());

    // [D4] Korean/Arabic numeral equivalence, hallucination fallback, 16-term leak block (0 B)
    let (_, fact_ok, bad_nums) = draft_or_fallback(
        "기성 정산 승인액 20만 원(200,000원) 지급 예정",
        "원장 확인 금액 이십만 원",
        "원본 대조 요망",
    );
    ensure!(fact_ok && bad_nums.is_empty());

    let (_fallback_txt, hall_ok, hall_bad) = draft_or_fallback(
        "지급액 9,900,000원 확정",
        "원장 금액 1,200,000원",
        "확인 금액 기준 재안내",
    );
    ensure!(!hall_ok && !hall_bad.is_empty());

    for &forbidden_term in CITIZEN_FORBIDDEN_TERMS {
        let guard = WriterGuard {
            forbidden: vec!["박민수".into()],
            locked_facts: vec!["200,000원".into()],
            enforce_internal_terms: true,
            reverse_cdr: Some(ReverseCdrContext {
                source_grade: Some(N2sfGrade::Sensitive),
                target_grade: Some(N2sfGrade::Open),
                declassification_ref: Some("N2SF-2026-DECL-01".into()),
                stripped_covert_artifacts: 0,
                block_on_covert_artifacts: true,
            }),
        };
        let leaked_body = format!("정산액 200,000원 검토 결과 ({forbidden_term}) 안내");
        ensure!(check_citizen_text(&leaked_body, &[]).is_err());
        ensure!(
            build_hwpx(
                &NoticeSpec {
                    title: "통지서",
                    body: &leaked_body,
                },
                &[],
                &guard,
            )
            .is_err()
        );
    }

    // [D6] Mandatory form snap + AutoFinalize permanent block on Adverse & Procedural
    let mut auto_pol = AutomationPolicy::default();
    auto_pol.enable_auto("OFF-ADMIN-01", "행정절차 자동화 고시 제2026-4호", crate::eval_nn::fixed_eval_utc())?;

    for (verdict, req_form, supp_cnt, simple_missing, want_form) in [
        (AgentVerdict::G1ExpiredMandatoryReject, OutputForm::ApprovalNotice, 0, false, OutputForm::RejectNotice),
        (AgentVerdict::G1SignInvalidRetry, OutputForm::ApprovalNotice, 0, false, OutputForm::SupplementRequest),
        (AgentVerdict::G3HoldSimpleReceiptMissing, OutputForm::RejectNotice, 0, true, OutputForm::SupplementRequest),
        (AgentVerdict::G2RejectContradictionLock, OutputForm::ApprovalNotice, 1, false, OutputForm::RejectNotice),
        (AgentVerdict::G3InterceptInterlockOrder, OutputForm::ApprovalNotice, 1, false, OutputForm::SupplementRequest),
    ] {
        let choice = select_form_for_verdict(req_form, verdict, supp_cnt, simple_missing);
        ensure!(choice.form == want_form && choice.overridden);
        let disp = auto_pol.decide_disposition(verdict.nature(), Some(1.0), true, "CASE-D6-PROBE");
        ensure!(matches!(disp, Disposition::PendingOfficer { .. }));
    }

    // [D9 Semantic Format & TableScaleCarry] Multipage scale inheritance + <w:strike> + superscript 10^3
    let mp_docx = synth_semantic_multipage_docx_bytes(
        "(단위: 천 원)",
        "단가 18,000원",
        "30,000",
        "20,000",
        "10",
        "3",
    )?;
    let mp_ir = lv_doc::docs::parse_docs_bytes("docx", &mp_docx)?;
    ensure!(mp_ir.tables.len() >= 2);
    ensure!(mp_ir.tables[0].unit_multiplier == 1_000);
    ensure!(mp_ir.tables[1].unit_multiplier == 1_000 && mp_ir.tables[1].carried_scale_from_prev);
    ensure!(mp_ir.plain_text.contains("10^3"));
    ensure!(mp_ir
        .track_changes
        .iter()
        .any(|tc| matches!(tc, lv_doc::docs::TrackChange::Struck(s) if s.contains("18,000"))));

    ensure!(DocsDomainPack::ALL.len() == 7);
    Ok(())
}
