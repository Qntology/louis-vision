//! Integration gate running ONLY diagnostic N:N matrix (`EvalSplit::DiagAll18`, `DOCS_NN_DIAG_SEED=20260501`)
//! and suites `nn_01..nn_09`. Never executes sealed holdout during CI or `cargo test`.

use chrono::Utc;
use louis_docs::eval_nn::audit_probe::run_d8_shift_and_worm_probe;
use louis_docs::eval_nn::dom_d6_concert_twin::{derive_coupled_concert_twin, validate_pyro_sacn_universe};
use louis_docs::eval_nn::domains::{
    all_18_domain_catalog, contains_forbidden_raw_gov_id_digits,
    dom06_ra_qa_esg::{evaluate_dom06_ra_qa_esg, CbamSectorCategory, Dom06RaQaEsgInput},
    dom08_trade_customs_lc::trading_relay_pair_v2,
    dom09_warehouse_hazmat::compute_iso6346_check_digit,
    dom10_retail_srm_settle::{is_one_hop_friction_trap, ChoiceFunnelTelemetry},
    dom12_fin_kyc_loan::verify_biz_reg_no_kr10,
    dom13_tax_evid_audit::{reconcile_tax_monetary_axes, verify_etax_approval_no_24},
    dom14_ip_patent_sec::{evaluate_dom14_ip_patent_sec, Dom14IpPatentSecInput, TechFundingRoute},
    dom15_eng_ecn_bom::{evaluate_dom15_eng_ecn_bom, Dom15EngEcnBomInput, MocClassKind},
    dom16_defense_mro::{repair_and_verify_nsn13},
    dom17_blind_hr_recruit::compute_verified_career_days_union,
    dom18_audit_internal_gov::{compute_rule395_ytd_totals, verify_dom18_audit_override, QuoteMethod2026},
    SUPPORTED_15_LANGS,
};
use louis_docs::eval_nn::holdout_zh::build_holdout_zh_36_specs;
use louis_docs::eval_nn::mcp_bus::{bind_strict_loopback_mcp, ensure_strict_loopback};
use louis_docs::eval_nn::schema::{derive_case_seed, DocsNnSuiteReport, EvalSplit as NnEvalSplit, SuiteSplit};
use louis_docs::eval_nn::synth_6axes::audio_patch_synth::{
    compute_span_tiou, detect_calibrated_audio_spans, synth_calibrated_audio_clip_16k, SynthAudioEventKind,
};
use louis_docs::eval_nn::synth_6axes::burst_concurrent::run_burst_concurrent_d7_probe;
use louis_docs::eval_nn::synth_6axes::digital_clean::{
    synth_disguised_html_xls_bytes, synth_ec376_docx_bytes, synth_very_hidden_xlsx_bytes, SynthXlsxCell, SynthXlsxSheet,
};
use louis_docs::eval_nn::synth_6axes::digital_hostile::{audit_hostile_docx_covert_channels, synth_5covert_hostile_docx_bytes};
use louis_docs::eval_nn::synth_6axes::privacy_contaminated::run_privacy_contaminated_d5_probe;
use louis_docs::eval_nn::synth_6axes::scan_2d_degraded::run_scan_2d_degraded_probe;
use louis_docs::eval_nn::{run_eval_nn_suite, DOCS_NN_SCENARIOS_JSON};
use louis_docs::sql_guard::{default_arrow_view_whitelist, sanitize_sql_output_cells, verify_and_wrap_safe_sql};
use louis_docs::workflow::{evaluate_docs_nn_suite, EvalSplit};
use lv_agent::gates::{select_form_for_verdict, AgentVerdict, Justification, OutputForm};
use lv_agent::policy::{AutomationPolicy, Disposition};
use lv_doc::docs::{parse_docs_bytes, sheet_pdf::detect_xlsx_very_hidden_sheets};
use lv_doc::writer::guard::{N2sfGrade, WriterGuard};
use lv_domain::{DocsDomainPack, EngineMode};
use lv_router::cross_app_coord::{
    build_louis_doc_deeplink, build_vision_stage_deeplink, validate_mobile_skin_webrtc_ip, CrossAppCoordinator,
    MobileSkinKind,
};
use lv_router::vault::DeviceRole;
use sha2::{Digest, Sha256};

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

#[test]
fn nn_01_defs_rev_and_108_cell_coverage() {
    assert_eq!(DocsDomainPack::ALL.len(), 7);
    assert_eq!(all_18_domain_catalog().len(), 18);
    assert!(!DOCS_NN_SCENARIOS_JSON.is_empty());
    let rep: DocsNnSuiteReport = run_eval_nn_suite(EngineMode::RulesOnly, SuiteSplit::DiagAll18, false).unwrap();
    assert_eq!(rep.defs_rev.len(), 8);
    assert_eq!(rep.total_cells, 108);
    assert_eq!(rep.passed_cells, 108);
    assert_eq!(rep.hard_gate_violations, 0);
}

#[test]
fn nn_02_phase_d1_d4_d6_d9_format_and_law() {
    let mp = synth_ec376_docx_bytes(
        "(단위: 천 원)",
        ("{{cla", "imed_", "amt}}"),
        "19,500",
        "18,000",
        ("1차", "50,000"),
        ("2차", "30,000"),
    )
    .unwrap();
    let ir = parse_docs_bytes("docx", &mp).unwrap();
    assert_eq!(ir.tables.len(), 2);
    assert!(ir.tables[1].carried_scale_from_prev && ir.tables[1].unit_multiplier == 1_000);

    let xls = synth_disguised_html_xls_bytes(&["국민은행"], "반품공제보류", &["정상지급", "반품공제보류"], &[("SRM-1", "수수료", "10,000")]);
    let ir_xls = parse_docs_bytes("xls", &xls).unwrap();
    assert!(ir_xls.plain_text.contains("payout_hold_code=반품공제보류"));

    let (v, fc) = evaluate_dom06_ra_qa_esg(
        &Dom06RaQaEsgInput {
            sector: CbamSectorCategory::SteelAluminumCementFertilizer,
            annual_import_net_mass_kg: 49_500,
            verifier_report_attached: false,
            coa_microbial_cfu_text: "1.2x10^4 CFU/g".into(),
            calib_sig_valid: true,
            catalyst_grade: N2sfGrade::Sensitive,
            target_zone: N2sfGrade::Sensitive,
            audio_ndt_defect_ping: false,
            supplement_count: 0,
        },
        OutputForm::ApprovalNotice,
    );
    assert_eq!(v, AgentVerdict::G2RejectContradictionLock);
    assert!(fc.overridden && fc.form == OutputForm::RejectNotice);
}

#[test]
fn nn_03_sequence_d2_d3_d11_split_3d_modality() {
    assert_eq!(compute_iso6346_check_digit("MSCU113466"), Some('0'));
    assert!(trading_relay_pair_v2("INS", "IP").is_none());
    assert!(is_one_hop_friction_trap(&ChoiceFunnelTelemetry {
        z_choice: 2.4,
        latency_ms: 4_000,
        rage_click_or_backtrack: true,
        abandon_rate_pct: 55.0,
        is_fast_track_complete: false,
        is_compare_tab_close: false,
    }));
    assert!(!is_one_hop_friction_trap(&ChoiceFunnelTelemetry {
        z_choice: 2.6,
        latency_ms: 2_000,
        rage_click_or_backtrack: false,
        abandon_rate_pct: 5.0,
        is_fast_track_complete: true,
        is_compare_tab_close: false,
    }));
    let exc = Justification::ApplyLawException {
        team_lead_cosign: true,
        rule_ref: "행안부예규 제395호".into(),
    };
    assert_eq!(
        verify_dom18_audit_override(AgentVerdict::G3ViolationRelatedVendorSplit, Some(&exc), false),
        Err(AgentVerdict::G3InterceptRubberStamp)
    );
    assert_eq!(compute_rule395_ytd_totals(&[], 2026_1110, QuoteMethod2026::SingleQuoteSmallSum, 5_000_000), (0, 5_000_000));
    assert_eq!(verify_biz_reg_no_kr10("11O-8I-41272").as_deref(), Some("1108141272"));
    assert_eq!(repair_and_verify_nsn13("284O-37-I04829I").as_deref(), Some("2840-37-1048291"));
    assert!(verify_etax_approval_no_24("20261031-41000012-a9b8c7d6").is_some());
    assert_eq!(
        reconcile_tax_monetary_axes(Some(1108141272), Some(10_000), Some(110_000), Some(100_000), &["110-81-41272"]),
        (Some(100_000), Some(10_000), true)
    );
    assert_eq!(compute_verified_career_days_union(&[(0, 731, 1.0), (100, 282, 1.0)]), 731);
}

#[test]
fn nn_04_security_d10_reverse_cdr_and_covert() {
    let hostile = synth_5covert_hostile_docx_bytes(
        "기술보안 문서",
        "FORCE_OPEN",
        "0pt hidden prompt",
        "100",
        "200",
        "https://exfil.invalid/px.png",
        "ACCT-01",
    )
    .unwrap();
    assert!(audit_hostile_docx_covert_channels(&hostile).unwrap().all_five_present());
    let vh_xlsx = synth_very_hidden_xlsx_bytes(
        &SynthXlsxSheet { name: "Pub".into(), rows: vec![vec![SynthXlsxCell::Text("A".into())]], very_hidden: false },
        &SynthXlsxSheet { name: "RecipeVH".into(), rows: vec![vec![SynthXlsxCell::Text("B".into())]], very_hidden: true },
    )
    .unwrap();
    assert_eq!(detect_xlsx_very_hidden_sheets(&vh_xlsx).unwrap(), vec!["RecipeVH".to_string()]);
    let (v, _, out) = evaluate_dom14_ip_patent_sec(
        &Dom14IpPatentSecInput {
            is_nct_79: true,
            days_since_nct_determination: 10,
            registered_with_motie: true,
            funding_route: TechFundingRoute::SelfFundedRequiresPreFilingReceipt,
            motie_approval_or_receipt_present: true,
            has_reservation_succession_rule: true,
            days_since_invention_completion: 30,
            explicit_non_succession_notice: false,
            is_defense_patent_art41: false,
            foreign_pct_without_clearance: false,
            xlsx_has_very_hidden_sheet: true,
            nominal_doc_grade: N2sfGrade::Sensitive,
            target_zone_grade: N2sfGrade::Open,
            declassification_ref: Some("DECL-1".into()),
            ciso_0x_cosigned: true,
            covert_artifact_count: 0,
            external_beacon_present: false,
            audio_gas_flow_leak_detected: false,
            body_text: "[RRN_REDACTED]".into(),
            supplement_count: 0,
        },
        OutputForm::ApprovalNotice,
    );
    assert_eq!(v, AgentVerdict::G3ViolationHighRisk);
    assert_eq!(out.len(), 0);
}

#[test]
fn nn_05_blueprint_d12_a0_zzs1_shadow_twin() {
    let scan2d = run_scan_2d_degraded_probe().unwrap();
    assert!(scan2d.virtual_luma_ok && scan2d.titleblock_protected && scan2d.legend_recall_ok && scan2d.legend_none_ok);
    let (v_twin, _) = evaluate_dom15_eng_ecn_bom(
        &Dom15EngEcnBomInput {
            moc_kind: MocClassKind::ReplacementInKind,
            moc_committee_approved: false,
            pssr_completed: false,
            drawing_is_master_rev: true,
            signer_sector_authorized: true,
            title_block_5_of_5: true,
            zzd1_redline_pressure_bar: 3.5,
            design_limit_bar: 4.2,
            zzs1_hoist_count: 14,
            active_bom_lines: vec![("HOIST".into(), 14, false, false), ("OLD".into(), 2, true, false)],
            twin_identify_gap: 0.5,
            audio_unauthorized_load_order: false,
            supplement_count: 0,
        },
        OutputForm::ApprovalNotice,
    );
    assert_eq!(v_twin, AgentVerdict::G1InterceptForceHold);
}

#[test]
fn nn_06_crew_d5_kiosk_jail_and_epoch_zeroize() {
    run_privacy_contaminated_d5_probe().unwrap();
    let mut coord = CrossAppCoordinator::default();
    let mut b2 = vec![99u8; 32];
    let mut b3 = vec![77u8; 32];
    assert_eq!(coord.privacy.trigger_lockdown_and_zeroize(100, &mut b2, &mut b3), 1);
    assert!(b2.iter().all(|&x| x == 0) && b3.iter().all(|&x| x == 0));
    assert!(validate_mobile_skin_webrtc_ip(MobileSkinKind::CounterKiosk, "192.168.1.5").is_err());
    assert!(validate_mobile_skin_webrtc_ip(MobileSkinKind::WirelessCameraRig, "8.8.8.8").is_err());
    assert!(validate_mobile_skin_webrtc_ip(MobileSkinKind::WirelessCameraRig, "192.168.1.5").is_ok());
    assert_eq!(build_vision_stage_deeplink("STG-1", 20260501, 2, "OBB-Lift-C"), "vision://stage/STG-1?seed=20260501&cue=2&obb=OBB-Lift-C");
    assert_eq!(build_louis_doc_deeplink("B1", "D1", [1, 2, 3, 4]), "louis://doc/B1/D1?bbox=1,2,3,4");
}

#[test]
fn nn_07_search_12track_audio_and_sql_5guard() {
    assert_eq!(SUPPORTED_15_LANGS.len(), 15);
    assert!(SUPPORTED_15_LANGS.contains(&"zh-tw"));
    assert!(SUPPORTED_15_LANGS.contains(&"ru") && SUPPORTED_15_LANGS.contains(&"th"));

    let (pcm, truths) = synth_calibrated_audio_clip_16k(
        NnEvalSplit::Diag,
        "NN07-AUD",
        160,
        &[
            (SynthAudioEventKind::L0GasDetectorBeepAlarm, 8, 8),
            (SynthAudioEventKind::L2McSpeechUnaccompanied, 30, 96),
            (SynthAudioEventKind::NegSungBalladVocalWithPad, 128, 30),
        ],
    );
    for tr in truths.iter().filter(|t| t.is_positive_target) {
        let det = detect_calibrated_audio_spans(&pcm, tr.event_kind);
        let tiou = det.iter().map(|&d| compute_span_tiou(d, (tr.start_ms, tr.end_ms))).fold(0.0, f64::max);
        assert!(tiou >= 0.70);
    }
    assert!(detect_calibrated_audio_spans(&pcm, SynthAudioEventKind::NegSungBalladVocalWithPad).is_empty());

    let wl = default_arrow_view_whitelist();
    assert!(verify_and_wrap_safe_sql(DeviceRole::PersonalPhone, "v1", N2sfGrade::Sensitive, "SELECT doc_id FROM docs_entities", &wl).is_ok());
    assert!(verify_and_wrap_safe_sql(DeviceRole::PersonalPhone, "v1", N2sfGrade::Sensitive, "DROP TABLE gate_ledger", &wl).is_err());
    assert!(verify_and_wrap_safe_sql(DeviceRole::CounterKiosk, "v1", N2sfGrade::Open, "SELECT doc_id FROM docs_entities", &wl).is_err());
    let g = WriterGuard::citizen_guard(Vec::new(), Vec::new());
    assert!(sanitize_sql_output_cells(&[vec!["123".into()]], &g).is_ok());
    assert!(sanitize_sql_output_cells(&[vec!["900101-1234567".into()]], &g).is_err());

    let twin = derive_coupled_concert_twin(NnEvalSplit::Diag, 0).unwrap();
    assert!(validate_pyro_sacn_universe(64, true, &twin.b06_sacn_smpte_cue.show_lan_ip));
    assert!(!validate_pyro_sacn_universe(63, true, &twin.b06_sacn_smpte_cue.show_lan_ip));
}

#[test]
fn nn_08_burst_d7_and_shift_aggregate_d8() {
    run_burst_concurrent_d7_probe().unwrap();
    let (dsli_7, dsli_18) = run_d8_shift_and_worm_probe().unwrap();
    assert!(dsli_7 <= 0.65 && dsli_18 <= 0.72);
    ensure_strict_loopback(&"127.0.0.1:8787".parse().unwrap()).unwrap();
    let (l, addr) = bind_strict_loopback_mcp(0).unwrap();
    assert!(addr.ip().is_loopback());
    drop(l);
}

#[test]
fn nn_09_holdout_zh36_sealed_guard_and_hygiene() {
    assert!(run_eval_nn_suite(EngineMode::RulesOnly, SuiteSplit::HoldoutSeal, false).is_err());
    let zh = build_holdout_zh_36_specs().unwrap();
    assert_eq!(zh.len(), 36);
    for s in &zh {
        let diag_seed = derive_case_seed(NnEvalSplit::Diag, 0, 0);
        assert_ne!(s.seed, diag_seed);
        let j = serde_json::to_string(s).unwrap();
        assert!(!contains_forbidden_raw_gov_id_digits(&j));
        let fc = select_form_for_verdict(s.requested_form, s.expected_verdict, s.supplement_count, false);
        assert_eq!(fc.form, s.expected_final_form);
        let mut pol = AutomationPolicy::default();
        pol.enable_auto("OFF-TEST", "법령", Utc::now()).unwrap();
        if !s.expected_auto_finalize {
            let conf = if s.expected_eligible { None } else { Some(1.0) };
            assert!(matches!(
                pol.decide_disposition(s.expected_verdict.nature(), conf, s.expected_eligible, &s.case_id),
                Disposition::PendingOfficer { .. }
            ));
        }
    }
    let hash_check = Sha256::digest(DOCS_NN_SCENARIOS_JSON.as_bytes());
    assert_eq!(hash_check.len(), 32);
}
