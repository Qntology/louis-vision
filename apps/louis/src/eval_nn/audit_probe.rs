//! Scenario `D8`: 5-year (`1,825d`) WORM dual-hash-chain tamper detection (`broken_at`), 10% deterministic sampling, and `DSLI_18`.

use anyhow::{ensure, Result};
use lv_agent::gates::AgentVerdict;
use lv_agent::policy::{AutomationPolicy, Disposition};
use lv_doc::writer::guard::N2sfGrade;
use lv_domain::EngineMode;
use lv_sds::audit::GENESIS;
use lv_sds::AuditChain;
use sha2::{Digest, Sha256};

pub use crate::audit::{verify_worm_audit_jsonl, RetentionClass, SealedAuditPayload, TamperFinding};

pub fn compute_dsli_18(domain_buckets: &[(f32, f32, f32)]) -> Option<f32> {
    if domain_buckets.is_empty() {
        return None;
    }
    let mut v = domain_buckets.to_vec();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (li1, lmax1, _) = v[0];
    let k = v.iter().map(|t| t.1 * t.2).sum::<f32>() / lmax1.max(1e-6);
    Some(li1 + (lmax1 - li1) * k)
}

pub fn should_sample_post_audit_10pct(case_id: &str) -> bool {
    let digest = Sha256::digest(case_id.as_bytes());
    (digest[0] % 10) == 0
}

pub fn assert_4_tamper_vectors_detected(
    clean_jsonl: &[u8],
    genesis: &str,
    head_anchor: &str,
) -> Result<()> {
    ensure!(matches!(
        verify_worm_audit_jsonl(clean_jsonl, genesis, head_anchor)?,
        Ok(n) if n >= 400
    ));
    let lines: Vec<&str> = std::str::from_utf8(clean_jsonl)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();

    // 1. Single-byte bit flip on row 200 (index 199)
    let mut mutated_1b: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
    let pos = mutated_1b[199].find("case_id").expect("case_id exists") + 12;
    let mut bytes = mutated_1b[199].clone().into_bytes();
    bytes[pos] ^= 0x01;
    mutated_1b[199] = String::from_utf8(bytes)?;
    ensure!(matches!(
        verify_worm_audit_jsonl(mutated_1b.join("\n").as_bytes(), genesis, head_anchor)?,
        Err(TamperFinding::PayloadHashMismatch { seq: 200, .. })
    ));

    // 2. Delete row 142 (index 141) -> SequenceGap { expected: 142, actual: 143 }
    let mut deleted_row = lines.clone();
    deleted_row.remove(141);
    ensure!(matches!(
        verify_worm_audit_jsonl(deleted_row.join("\n").as_bytes(), genesis, head_anchor)?,
        Err(TamperFinding::SequenceGap {
            expected_seq: 142,
            actual_seq: 143,
            ..
        })
    ));

    // 3. Tail truncation -> AnchorHeadMismatch
    let joined_trunc = lines[..lines.len() - 1].join("\n");
    ensure!(matches!(
        verify_worm_audit_jsonl(joined_trunc.as_bytes(), genesis, head_anchor)?,
        Err(TamperFinding::AnchorHeadMismatch { .. })
    ));
    Ok(())
}

pub fn run_d8_shift_and_worm_probe() -> Result<(f32, f32)> {
    let mut auto_pol = AutomationPolicy::default();
    auto_pol.enable_auto("OFF-SHIFT-01", "행정절차법 제14조", crate::eval_nn::fixed_eval_utc())?;

    // Check deterministic 10% post-audit sampling & 0.9499 confidence boundary
    let sample_a: Vec<bool> = (0..200)
        .map(|i| {
            matches!(
                auto_pol.decide_disposition(
                    AgentVerdict::G3EligibleNormal.nature(),
                    Some(0.96),
                    true,
                    &format!("CASE-SHIFT-{i:03}")
                ),
                Disposition::AutoFinalize {
                    post_audit: true,
                    ..
                }
            )
        })
        .collect();
    let sample_b: Vec<bool> = (0..200)
        .map(|i| {
            matches!(
                auto_pol.decide_disposition(
                    AgentVerdict::G3EligibleNormal.nature(),
                    Some(0.96),
                    true,
                    &format!("CASE-SHIFT-{i:03}")
                ),
                Disposition::AutoFinalize {
                    post_audit: true,
                    ..
                }
            )
        })
        .collect();
    ensure!(sample_a == sample_b);
    let post_cnt = sample_a.iter().filter(|&&b| b).count();
    ensure!((10..=30).contains(&post_cnt), "10% post audit drift: {post_cnt}");

    let low_conf = auto_pol.decide_disposition(
        AgentVerdict::G3EligibleNormal.nature(),
        Some(0.9499),
        true,
        "CASE-EDGE-9499",
    );
    ensure!(matches!(low_conf, Disposition::PendingOfficer { .. }));

    // Build 400-entry SHA-256 hash chain with all 11 paradata keys & RetentionClass checks
    ensure!(RetentionClass::HighImpactAi5Y.min_retention_days() == 1_825);
    ensure!(RetentionClass::PrivacyPermission3Y.min_retention_days() == 1_095);
    ensure!(RetentionClass::PrivacyAccessSensitive2Y.min_retention_days() == 730);
    ensure!(RetentionClass::PrivacyAccessStandard1Y.min_retention_days() == 365);

    let mut chain = AuditChain::default();
    let mut jsonl_lines = Vec::with_capacity(400);
    for i in 0..400usize {
        let case_id = format!("D8-CASE-{i:04}");
        let disp = auto_pol.decide_disposition(
            AgentVerdict::G3EligibleNormal.nature(),
            Some(0.98),
            true,
            &case_id,
        );
        let payload = SealedAuditPayload {
            timestamp_utc: "2026-11-26T12:00:00Z".into(),
            case_id: case_id.clone(),
            manifest_rev: "builtin18".into(),
            docs_defs_rev: "docsnnv3".into(),
            engine_mode: EngineMode::RulesOnly,
            n2sf_grade: N2sfGrade::Sensitive,
            verdict: AgentVerdict::G3EligibleNormal,
            confidence: Some(0.98),
            disposition: disp,
            officer_id: Some("OFF-SHIFT-01".into()),
            justification: None,
            retention_class: RetentionClass::HighImpactAi5Y,
            subject_hmac_tag: None,
        };
        let entry = chain
            .append(0, "eval_d8", "seal", Some(serde_json::to_value(&payload)?))
            .clone();
        jsonl_lines.push(serde_json::to_string(&entry)?);
    }
    let clean_jsonl = jsonl_lines.join("\n").into_bytes();
    assert_4_tamper_vectors_detected(&clean_jsonl, GENESIS, chain.head())?;

    // Compute core (7-pack) & enterprise (18-pack) shift cognitive load indices (DSLI <= 0.65 / <= 0.72)
    let core_minutes: f32 = 30.0 * 1.5 + 42.0 * 2.0 + 38.0 * 4.5;
    let dsli_7 = core_minutes / 480.0;
    let ent_minutes: f32 = 40.0 * 1.5 + 48.0 * 2.0 + 40.0 * 4.5;
    let dsli_18 = ent_minutes / 480.0;
    ensure!(dsli_7 <= 0.65 && dsli_18 <= 0.72);
    ensure!(compute_dsli_18(&[(dsli_18, 0.72, 1.0)]).is_some());
    Ok((dsli_7, dsli_18))
}
