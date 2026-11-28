//! `seed2` vs `E2E` relational N:N search runner, `premium-discount-eval` ranking scorer (`MRR`, `R@10`, `nDCG@10`, `Hit@1`),
//! JSONL corpus/query/result emitter, and hard security/hygiene gates.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{bail, Result};
use lv_core::gemma_embed::cosine;
use lv_router::vault::DeviceRole;
use premium_discount_eval::ranking::{mrr, ndcg_at_k, recall_at_k};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::domains::contains_forbidden_raw_gov_id_digits;
use super::nl_domain_blueprints::{blueprint_by_domain_id, ALL_19_BLUEPRINTS};
use super::nl_scenarios::{generate_domain_fit_probes, generate_nl_scenarios, NlSearchScenario};
use super::nl_search_engine::{
    index_world_e2e, index_world_seed2, DenseTextEmbedder, E2eIngestStats, InMemoryHybridIndex,
    OfflineHybridEmbedder, SearchIndexBackend, SearchRequest, SearchResponse,
};
use super::nl_world::{build_relational_world, WorldDocRecord, WorldDocRole};
use super::schema::{EvalSplit, DOCS_NN_DIAG_SEED, DOCS_NN_HOLDOUT_SEED};

#[derive(Debug, Clone)]
pub struct NlSearchEvalConfig {
    pub split: EvalSplit,
    pub release_seal: bool,
    pub cases_per_domain: usize,
    pub domains_filter: Option<Vec<String>>,
    pub include_xr_twin: bool,
    pub out_dir: Option<PathBuf>,
    pub dump_synth: bool,
    pub structured_layout: bool,
}

impl Default for NlSearchEvalConfig {
    fn default() -> Self {
        Self {
            split: EvalSplit::Diag,
            release_seal: false,
            cases_per_domain: 2,
            domains_filter: None,
            include_xr_twin: true,
            out_dir: None,
            dump_synth: false,
            structured_layout: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MetricSummary {
    pub positive_queries: usize,
    pub zero_hit_queries: usize,
    pub mrr: f64,
    pub recall_at_10: f64,
    pub ndcg_at_10: f64,
    pub hit_at_1: f64,
    pub zero_hit_accuracy: f64,
    pub relay_queries: usize,
    pub relay_recall: f64,
    pub security_leaks: usize,
    pub covert_leaks: usize,
    pub superseded_leaks: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CorpusSizeSummary {
    pub total_docs: usize,
    pub total_bytes: u64,
    pub min_doc_bytes: u64,
    pub median_doc_bytes: u64,
    pub avg_doc_bytes: f64,
    pub max_doc_bytes: u64,
    pub docs_under_500kb: usize,
    pub by_format_min_bytes: BTreeMap<String, u64>,
    pub by_format_avg_bytes: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnswerQualitySummary {
    pub evaluated_queries: usize,
    pub positive_answered: usize,
    pub abstention_queries: usize,
    pub fact_recall: f64,
    pub numeric_exact_rate: f64,
    pub citation_precision: f64,
    pub groundedness_rate: f64,
    pub citizen_guard_clean_rate: f64,
    pub abstention_accuracy: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DomainFitPerDomain {
    pub docs_checked: usize,
    pub structural_pass: usize,
    pub probe_top1_pass: usize,
    pub probe_total: usize,
    pub probe_top10_purity: f64,
    pub affinity_pass_rate: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DomainFitAuditSummary {
    pub structural_docs_checked: usize,
    pub structural_docs_passed: usize,
    pub domain_structural_pass_rate: f64,
    pub domain_probes_total: usize,
    pub domain_fit_top1_accuracy: f64,
    pub domain_fit_top10_purity: f64,
    pub domain_mismatch_top1_count: usize,
    pub doc_affinity_checked: usize,
    pub doc_affinity_passed: usize,
    pub doc_domain_affinity_accuracy: f64,
    pub per_domain_fit: BTreeMap<String, DomainFitPerDomain>,
    pub failure_samples: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DomainArmComparison {
    pub seed2: MetricSummary,
    pub e2e: MetricSummary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchRunRow {
    pub arm: String,
    pub query_id: String,
    pub domain_id: String,
    pub archetype: String,
    pub query_lang: String,
    pub is_cross_lingual: bool,
    pub sql_guard_passed: bool,
    pub ranked_doc_ids: Vec<String>,
    pub answer_text: String,
    pub citations: Vec<String>,
    pub evidence_snippets: Vec<String>,
    pub abstained: bool,
    pub factcheck_passed: bool,
    pub citizen_guard_clean: bool,
    pub fact_recall: Option<f64>,
    pub numeric_exact: Option<bool>,
    pub mrr: Option<f64>,
    pub recall_at_10: Option<f64>,
    pub ndcg_at_10: Option<f64>,
    pub hit_at_1: Option<f64>,
    pub security_leaks: usize,
    pub covert_leaks: usize,
    pub superseded_leaks: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NlSearchNnReport {
    pub split: EvalSplit,
    pub cases_per_domain: usize,
    pub domains_covered: usize,
    pub langs_covered: usize,
    pub total_docs: usize,
    pub total_queries: usize,
    pub corpus_size: CorpusSizeSummary,
    pub embedder_id: String,
    pub embedder_dim: usize,
    pub world_sha256_hex: String,
    pub queries_sha256_hex: String,
    pub e2e_ingest: E2eIngestStats,
    pub gov_id_digit_leaks: usize,
    pub sql_guard_checks_passed: usize,
    pub sql_guard_checks_total: usize,
    pub audio_span_matches: usize,
    pub audio_span_total: usize,
    pub seed2_overall: MetricSummary,
    pub e2e_overall: MetricSummary,
    pub seed2_answer_quality: AnswerQualitySummary,
    pub e2e_answer_quality: AnswerQualitySummary,
    pub domain_fit_audit: DomainFitAuditSummary,
    pub seed2_cross_lingual: MetricSummary,
    pub e2e_cross_lingual: MetricSummary,
    pub by_domain: BTreeMap<String, DomainArmComparison>,
    pub by_format: BTreeMap<String, DomainArmComparison>,
    pub by_archetype: BTreeMap<String, DomainArmComparison>,
    pub by_query_lang: BTreeMap<String, DomainArmComparison>,
    pub format_domain_matrix: BTreeMap<String, BTreeMap<String, usize>>,
    pub format_domain_cells_covered: usize,
    pub hard_gate_failures: Vec<String>,
    pub exit_code: i32,
}

#[derive(Default)]
struct Accum {
    pos_cnt: usize,
    zero_cnt: usize,
    zero_pass: usize,
    sum_mrr: f64,
    sum_r10: f64,
    sum_ndcg10: f64,
    sum_hit1: f64,
    relay_cnt: usize,
    sum_relay_recall: f64,
    security_leaks: usize,
    covert_leaks: usize,
    superseded_leaks: usize,
}

impl Accum {
    fn record(
        &mut self,
        sc: &NlSearchScenario,
        resp: &SearchResponse,
        world_doc_map: &BTreeMap<String, &WorldDocRecord>,
    ) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>, usize, usize, usize) {
        let ranked: Vec<&str> = resp.hits.iter().map(|h| h.doc_id.as_str()).collect();
        let top10: BTreeSet<&str> = ranked.iter().take(10).copied().collect();

        let forbidden_set: BTreeSet<&str> = sc.forbidden_doc_ids.iter().map(String::as_str).collect();
        let sup_set: BTreeSet<&str> = sc.superseded_doc_ids.iter().map(String::as_str).collect();

        let mut sec_leaks = 0usize;
        let mut cov_leaks = 0usize;
        let mut sup_leaks = 0usize;
        for &id in &ranked {
            if sup_set.contains(id) {
                sup_leaks += 1;
            }
            if forbidden_set.contains(id) {
                let is_covert = world_doc_map
                    .get(id)
                    .is_some_and(|d| d.covert_canary_token.is_some());
                if is_covert && sc.sub_variant == "covert_canary_zero_hit" {
                    cov_leaks += 1;
                } else {
                    sec_leaks += 1;
                }
            }
        }
        self.security_leaks += sec_leaks;
        self.covert_leaks += cov_leaks;
        self.superseded_leaks += sup_leaks;

        if sc.expect_zero_hits {
            self.zero_cnt += 1;
            if ranked.is_empty() {
                self.zero_pass += 1;
            }
            return (None, None, None, None, sec_leaks, cov_leaks, sup_leaks);
        }

        let rel: Vec<&str> = sc.relevant_doc_ids.iter().map(String::as_str).collect();
        let q_mrr = mrr(&ranked, &rel);
        let q_r10 = recall_at_k(&ranked, &rel, 10);
        let q_ndcg10 = ndcg_at_k(&ranked, &rel, 10);
        let q_hit1 = if ranked.first().is_some_and(|first| rel.contains(first)) {
            1.0
        } else {
            0.0
        };

        self.pos_cnt += 1;
        self.sum_mrr += q_mrr;
        self.sum_r10 += q_r10;
        self.sum_ndcg10 += q_ndcg10;
        self.sum_hit1 += q_hit1;

        if !sc.relay_target_ids.is_empty() {
            self.relay_cnt += 1;
            let hit_relay = sc
                .relay_target_ids
                .iter()
                .filter(|t| top10.contains(t.as_str()))
                .count();
            self.sum_relay_recall += hit_relay as f64 / sc.relay_target_ids.len() as f64;
        }

        (
            Some(q_mrr),
            Some(q_r10),
            Some(q_ndcg10),
            Some(q_hit1),
            sec_leaks,
            cov_leaks,
            sup_leaks,
        )
    }

    fn finish(&self) -> MetricSummary {
        let p = self.pos_cnt.max(1) as f64;
        let zero_acc = if self.zero_cnt == 0 {
            1.0
        } else {
            self.zero_pass as f64 / self.zero_cnt as f64
        };
        let rel_rec = if self.relay_cnt == 0 {
            1.0
        } else {
            self.sum_relay_recall / self.relay_cnt as f64
        };
        MetricSummary {
            positive_queries: self.pos_cnt,
            zero_hit_queries: self.zero_cnt,
            mrr: if self.pos_cnt == 0 { 0.0 } else { self.sum_mrr / p },
            recall_at_10: if self.pos_cnt == 0 { 0.0 } else { self.sum_r10 / p },
            ndcg_at_10: if self.pos_cnt == 0 { 0.0 } else { self.sum_ndcg10 / p },
            hit_at_1: if self.pos_cnt == 0 { 0.0 } else { self.sum_hit1 / p },
            zero_hit_accuracy: zero_acc,
            relay_queries: self.relay_cnt,
            relay_recall: rel_rec,
            security_leaks: self.security_leaks,
            covert_leaks: self.covert_leaks,
            superseded_leaks: self.superseded_leaks,
        }
    }
}

fn sha256_jsonl<T: Serialize>(items: &[T]) -> Result<String> {
    let mut buf = Vec::new();
    for item in items {
        serde_json::to_writer(&mut buf, item)?;
        buf.push(b'\n');
    }
    let digest = Sha256::digest(&buf);
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn write_jsonl<T: Serialize>(path: &std::path::Path, items: &[T]) -> Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    for item in items {
        serde_json::to_writer(&mut w, item)?;
        w.write_all(b"\n")?;
    }
    w.flush()?;
    Ok(())
}

#[derive(Default)]
struct AnswerAccum {
    evaluated: usize,
    pos_answered: usize,
    abs_queries: usize,
    abs_pass: usize,
    sum_fact_recall: f64,
    num_exact_pass: usize,
    sum_cit_prec: f64,
    grounded_pass: usize,
    citizen_clean_pass: usize,
}

impl AnswerAccum {
    fn record(
        &mut self,
        sc: &NlSearchScenario,
        resp: &SearchResponse,
        world_doc_map: &BTreeMap<String, &WorldDocRecord>,
    ) -> (Option<f64>, Option<bool>) {
        let ga = &resp.grounded_answer;
        self.evaluated += 1;
        if ga.factcheck_passed {
            self.grounded_pass += 1;
        }
        if ga.citizen_guard_clean && ga.gov_id_clean {
            self.citizen_clean_pass += 1;
        }
        if sc.expected_abstention {
            self.abs_queries += 1;
            if ga.abstained && ga.citations.is_empty() {
                self.abs_pass += 1;
            }
            return (None, None);
        }

        self.pos_answered += 1;
        let f_rec = if sc.expected_required_facts.is_empty() {
            1.0
        } else {
            let matched = sc
                .expected_required_facts
                .iter()
                .filter(|f| ga.answer_text.contains(f.as_str()))
                .count();
            matched as f64 / sc.expected_required_facts.len() as f64
        };
        self.sum_fact_recall += f_rec;

        let num_ok = !ga.abstained
            && sc.expected_amount_krw.is_none_or(|a| ga.cited_amount_krw == Some(a))
            && sc.expected_line_item_count.is_none_or(|c| ga.cited_line_item_count == Some(c));
        if num_ok {
            self.num_exact_pass += 1;
        }

        let cit_prec = if ga.citations.is_empty() {
            0.0
        } else {
            let forbidden_set: BTreeSet<&str> = sc.forbidden_doc_ids.iter().map(String::as_str).collect();
            let sup_set: BTreeSet<&str> = sc.superseded_doc_ids.iter().map(String::as_str).collect();
            let top1_rel = sc.relevant_doc_ids.contains(&ga.citations[0]);
            let valid_cnt = ga
                .citations
                .iter()
                .filter(|cid| {
                    if sc.relevant_doc_ids.contains(*cid) {
                        return true;
                    }
                    world_doc_map.get(cid.as_str()).is_some_and(|d| {
                        d.domain_id == sc.domain_id
                            && d.vault_id == sc.caller_vault_id
                            && d.role != WorldDocRole::CovertHostile
                            && !forbidden_set.contains(d.doc_id.as_str())
                            && !sup_set.contains(d.doc_id.as_str())
                    })
                })
                .count();
            let frac = valid_cnt as f64 / ga.citations.len() as f64;
            if top1_rel { frac } else { frac * 0.5 }
        };
        self.sum_cit_prec += cit_prec;
        (Some(f_rec), Some(num_ok))
    }

    fn finish(&self) -> AnswerQualitySummary {
        let tot = self.evaluated.max(1) as f64;
        let pos = self.pos_answered.max(1) as f64;
        AnswerQualitySummary {
            evaluated_queries: self.evaluated,
            positive_answered: self.pos_answered,
            abstention_queries: self.abs_queries,
            fact_recall: if self.pos_answered == 0 { 1.0 } else { self.sum_fact_recall / pos },
            numeric_exact_rate: if self.pos_answered == 0 { 1.0 } else { self.num_exact_pass as f64 / pos },
            citation_precision: if self.pos_answered == 0 { 1.0 } else { self.sum_cit_prec / pos },
            groundedness_rate: self.grounded_pass as f64 / tot,
            citizen_guard_clean_rate: self.citizen_clean_pass as f64 / tot,
            abstention_accuracy: if self.abs_queries == 0 { 1.0 } else { self.abs_pass as f64 / self.abs_queries as f64 },
        }
    }
}

struct ArmEvalOutput {
    overall: MetricSummary,
    answer_quality: AnswerQualitySummary,
    cross_lingual: MetricSummary,
    by_domain: BTreeMap<String, MetricSummary>,
    by_format: BTreeMap<String, MetricSummary>,
    by_archetype: BTreeMap<String, MetricSummary>,
    by_lang: BTreeMap<String, MetricSummary>,
    sql_passed: usize,
    sql_total: usize,
    audio_matches: usize,
    audio_total: usize,
    rows: Vec<SearchRunRow>,
}

fn evaluate_arm(
    arm_name: &str,
    index: &dyn SearchIndexBackend,
    embedder: &dyn DenseTextEmbedder,
    scenarios: &[NlSearchScenario],
    world_doc_map: &BTreeMap<String, &WorldDocRecord>,
) -> Result<ArmEvalOutput> {
    let mut overall = Accum::default();
    let mut ans_acc = AnswerAccum::default();
    let mut xling = Accum::default();
    let mut by_dom: BTreeMap<String, Accum> = BTreeMap::new();
    let mut by_fmt: BTreeMap<String, Accum> = BTreeMap::new();
    let mut by_arch: BTreeMap<String, Accum> = BTreeMap::new();
    let mut by_lang: BTreeMap<String, Accum> = BTreeMap::new();

    let mut sql_passed = 0usize;
    let mut sql_total = 0usize;
    let mut audio_matches = 0usize;
    let mut audio_total = 0usize;
    let mut rows = Vec::with_capacity(scenarios.len());

    for sc in scenarios {
        let req = SearchRequest {
            query_text: sc.query_text.clone(),
            caller_role: sc.caller_role,
            caller_vault_id: sc.caller_vault_id.clone(),
            caller_max_grade: sc.caller_max_grade,
            scope_forms: sc.scope_forms.clone(),
            min_amount_krw: sc.min_amount_krw,
            status_filter: sc.status_filter.clone(),
            require_relay_hops: sc.require_relay_hops,
            include_superseded: sc.include_superseded,
            sql_probe: sc.sql_probe.clone(),
            top_k: 10,
        };
        let resp = index.search(&req, embedder)?;

        if sc.sql_probe.is_some() {
            sql_total += 1;
            if resp.sql_guard_passed == sc.expect_sql_allowed {
                sql_passed += 1;
            }
        }
        if !sc.expected_audio_spans_ms.is_empty() {
            audio_total += 1;
            let matched = resp.hits.first().is_some_and(|top| {
                sc.relevant_doc_ids.contains(&top.doc_id)
                    && sc
                        .expected_audio_spans_ms
                        .iter()
                        .all(|sp| top.audio_spans_ms.contains(sp))
            });
            if matched {
                audio_matches += 1;
            }
        }

        let (qmrr, qr10, qndcg, qhit1, sec_l, cov_l, sup_l) =
            overall.record(sc, &resp, world_doc_map);
        let (f_rec, num_ok) = ans_acc.record(sc, &resp, world_doc_map);
        if sc.is_cross_lingual {
            xling.record(sc, &resp, world_doc_map);
        }
        by_dom
            .entry(sc.domain_id.clone())
            .or_default()
            .record(sc, &resp, world_doc_map);
        let fmt_key = sc
            .relevant_doc_ids
            .first()
            .or_else(|| sc.forbidden_doc_ids.first())
            .and_then(|id| world_doc_map.get(id.as_str()))
            .map_or_else(|| "unknown".to_string(), |d| d.file_format.clone());
        by_fmt
            .entry(fmt_key)
            .or_default()
            .record(sc, &resp, world_doc_map);
        by_arch
            .entry(sc.archetype.as_str().to_string())
            .or_default()
            .record(sc, &resp, world_doc_map);
        by_lang
            .entry(sc.query_lang.clone())
            .or_default()
            .record(sc, &resp, world_doc_map);

        let ga = &resp.grounded_answer;
        rows.push(SearchRunRow {
            arm: arm_name.into(),
            query_id: sc.query_id.clone(),
            domain_id: sc.domain_id.clone(),
            archetype: sc.archetype.as_str().into(),
            query_lang: sc.query_lang.clone(),
            is_cross_lingual: sc.is_cross_lingual,
            sql_guard_passed: resp.sql_guard_passed,
            ranked_doc_ids: resp.hits.iter().map(|h| h.doc_id.clone()).collect(),
            answer_text: ga.answer_text.clone(),
            citations: ga.citations.clone(),
            evidence_snippets: ga.evidence_snippets.clone(),
            abstained: ga.abstained,
            factcheck_passed: ga.factcheck_passed,
            citizen_guard_clean: ga.citizen_guard_clean && ga.gov_id_clean,
            fact_recall: f_rec,
            numeric_exact: num_ok,
            mrr: qmrr,
            recall_at_10: qr10,
            ndcg_at_10: qndcg,
            hit_at_1: qhit1,
            security_leaks: sec_l,
            covert_leaks: cov_l,
            superseded_leaks: sup_l,
        });
    }

    Ok(ArmEvalOutput {
        overall: overall.finish(),
        answer_quality: ans_acc.finish(),
        cross_lingual: xling.finish(),
        by_domain: by_dom.into_iter().map(|(k, v)| (k, v.finish())).collect(),
        by_format: by_fmt.into_iter().map(|(k, v)| (k, v.finish())).collect(),
        by_archetype: by_arch.into_iter().map(|(k, v)| (k, v.finish())).collect(),
        by_lang: by_lang.into_iter().map(|(k, v)| (k, v.finish())).collect(),
        sql_passed,
        sql_total,
        audio_matches,
        audio_total,
        rows,
    })
}

fn compute_corpus_size_summary(docs: &[WorldDocRecord]) -> CorpusSizeSummary {
    let mut lens: Vec<u64> = docs.iter().map(|d| d.rendered_byte_len).collect();
    lens.sort_unstable();
    let total_docs = lens.len();
    let total_bytes: u64 = lens.iter().sum();
    let min_doc_bytes = lens.first().copied().unwrap_or(0);
    let max_doc_bytes = lens.last().copied().unwrap_or(0);
    let median_doc_bytes = if lens.is_empty() { 0 } else { lens[lens.len() / 2] };
    let avg_doc_bytes = if total_docs == 0 { 0.0 } else { total_bytes as f64 / total_docs as f64 };
    let docs_under_500kb = lens.iter().filter(|&&b| b < 512_000).count();

    let mut fmt_bins: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for d in docs {
        fmt_bins.entry(d.file_format.clone()).or_default().push(d.rendered_byte_len);
    }
    let mut by_format_min_bytes = BTreeMap::new();
    let mut by_format_avg_bytes = BTreeMap::new();
    for (fmt, vals) in fmt_bins {
        let min_v = vals.iter().copied().min().unwrap_or(0);
        let sum_v: u64 = vals.iter().sum();
        let avg_v = sum_v as f64 / vals.len().max(1) as f64;
        by_format_min_bytes.insert(fmt.clone(), min_v);
        by_format_avg_bytes.insert(fmt, avg_v);
    }

    CorpusSizeSummary {
        total_docs,
        total_bytes,
        min_doc_bytes,
        median_doc_bytes,
        avg_doc_bytes,
        max_doc_bytes,
        docs_under_500kb,
        by_format_min_bytes,
        by_format_avg_bytes,
    }
}

fn run_domain_fit_audit(
    e2e_idx: &InMemoryHybridIndex,
    embedder: &dyn DenseTextEmbedder,
    world: &super::nl_world::RelationalWorld,
    world_doc_map: &BTreeMap<String, &WorldDocRecord>,
) -> Result<DomainFitAuditSummary> {
    let mut structural_docs_checked = 0usize;
    let mut structural_docs_passed = 0usize;
    let mut per_dom_docs_checked: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_dom_struct_pass: BTreeMap<String, usize> = BTreeMap::new();
    let mut failure_samples = Vec::new();

    // 1. Structural & Statutory Authenticity Audit on ALL E2E primary docs
    let mut primary_e2e_docs = Vec::new();
    for edoc in e2e_idx.docs() {
        let Some(wdoc) = world_doc_map.get(edoc.doc_id.as_str()) else { continue };
        if !matches!(wdoc.role, WorldDocRole::PrimaryForm { .. }) {
            continue;
        }
        primary_e2e_docs.push(edoc);
        structural_docs_checked += 1;
        *per_dom_docs_checked.entry(edoc.domain_id.clone()).or_insert(0) += 1;

        let bp = blueprint_by_domain_id(&edoc.domain_id)
            .ok_or_else(|| anyhow::anyhow!("missing blueprint for {}", edoc.domain_id))?;
        let req_ok = bp
            .required_field_keys
            .iter()
            .all(|k| edoc.text.contains(&format!("{k}=")));
        let sig_hits = bp
            .domain_signature_terms
            .iter()
            .filter(|&&t| edoc.text.contains(t) || edoc.title.contains(t))
            .count();
        let size_ok = wdoc.rendered_byte_len >= 512_000;
        let items_ok = edoc.line_item_count == 40 && edoc.line_item_sum_krw == wdoc.amount_krw;
        let media_ok = edoc.media_count >= 1;
        let gov_clean = !contains_forbidden_raw_gov_id_digits(&edoc.text);

        if req_ok && sig_hits >= 4 && size_ok && items_ok && media_ok && gov_clean {
            structural_docs_passed += 1;
            *per_dom_struct_pass.entry(edoc.domain_id.clone()).or_insert(0) += 1;
        } else if failure_samples.len() < 12 {
            failure_samples.push(format!(
                "STRUCT {} ({}): req={req_ok} sig={sig_hits}/{} size={size_ok}({}) items={items_ok}({}/{}) media={media_ok}({}) gov={gov_clean}",
                edoc.doc_id,
                wdoc.file_format,
                bp.domain_signature_terms.len(),
                wdoc.rendered_byte_len,
                edoc.line_item_count,
                edoc.line_item_sum_krw,
                edoc.media_count
            ));
        }
    }

    // 2. Pure NL Domain-Fit Search Probes (`ko` + `en` per domain)
    let probes = generate_domain_fit_probes(world)?;
    let mut domain_mismatch_top1_count = 0usize;
    let mut sum_top1_acc = 0.0f64;
    let mut sum_top10_purity = 0.0f64;
    let mut per_dom_probe_total: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_dom_probe_top1: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_dom_probe_purities: BTreeMap<String, Vec<f64>> = BTreeMap::new();

    for probe in &probes {
        let req = SearchRequest {
            query_text: probe.query_text.clone(),
            caller_role: DeviceRole::PersonalPhone,
            caller_vault_id: probe.caller_vault_id.clone(),
            caller_max_grade: probe.caller_max_grade,
            scope_forms: Vec::new(),
            min_amount_krw: None,
            status_filter: None,
            require_relay_hops: 0,
            include_superseded: false,
            sql_probe: None,
            top_k: 10,
        };
        let resp = e2e_idx.search(&req, embedder)?;
        *per_dom_probe_total.entry(probe.domain_id.clone()).or_insert(0) += 1;
        let top1_ok = resp.hits.first().is_some_and(|h| h.domain_id == probe.domain_id);
        if top1_ok {
            sum_top1_acc += 1.0;
            *per_dom_probe_top1.entry(probe.domain_id.clone()).or_insert(0) += 1;
        } else {
            domain_mismatch_top1_count += 1;
            if failure_samples.len() < 20 {
                let got = resp.hits.first().map_or("NONE", |h| h.domain_id.as_str());
                failure_samples.push(format!(
                    "PROBE_TOP1 {} ({}) expected={} got={got}",
                    probe.probe_id, probe.lang, probe.domain_id
                ));
            }
        }
        let purity = if resp.hits.is_empty() {
            0.0
        } else {
            let same = resp.hits.iter().filter(|h| h.domain_id == probe.domain_id).count();
            same as f64 / resp.hits.len() as f64
        };
        if purity < 1.0 && failure_samples.len() < 20 {
            let wrong: Vec<String> = resp
                .hits
                .iter()
                .filter(|h| h.domain_id != probe.domain_id)
                .map(|h| format!("{}:{}", h.doc_id, h.domain_id))
                .collect();
            failure_samples.push(format!(
                "PROBE_PURITY {} ({}) purity={purity:.3} off={wrong:?}",
                probe.probe_id, probe.lang
            ));
        }
        sum_top10_purity += purity;
        per_dom_probe_purities.entry(probe.domain_id.clone()).or_default().push(purity);
    }

    // 3. Reverse Doc-to-Domain Embedding Affinity across ALL E2E primary docs
    let mut probe_centroids: Vec<(&str, Vec<f32>)> = Vec::with_capacity(ALL_19_BLUEPRINTS.len());
    for bp in &ALL_19_BLUEPRINTS {
        let vk = embedder.embed_query(bp.domain_probe_ko)?;
        let ve = embedder.embed_query(bp.domain_probe_en)?;
        let mut avg: Vec<f32> = vk.iter().zip(ve.iter()).map(|(a, b)| a + b).collect();
        lv_core::gemma_embed::normalize_l2(&mut avg);
        probe_centroids.push((bp.domain_id, avg));
    }

    let mut doc_affinity_checked = 0usize;
    let mut doc_affinity_passed = 0usize;
    let mut per_dom_aff_tot: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_dom_aff_pass: BTreeMap<String, usize> = BTreeMap::new();

    for edoc in primary_e2e_docs {
        doc_affinity_checked += 1;
        *per_dom_aff_tot.entry(edoc.domain_id.clone()).or_insert(0) += 1;
        let mut best_dom = "";
        let mut best_sim = f32::NEG_INFINITY;
        for (dom_id, cent) in &probe_centroids {
            let sim = cosine(&edoc.dense_vec, cent);
            if sim > best_sim {
                best_sim = sim;
                best_dom = *dom_id;
            }
        }
        if best_dom == edoc.domain_id {
            doc_affinity_passed += 1;
            *per_dom_aff_pass.entry(edoc.domain_id.clone()).or_insert(0) += 1;
        } else if failure_samples.len() < 24 {
            failure_samples.push(format!(
                "AFFINITY {} ({}) expected={} best={best_dom} sim={best_sim:.3}",
                edoc.doc_id, edoc.doc_lang, edoc.domain_id
            ));
        }
    }

    let mut per_domain_fit = BTreeMap::new();
    for (dom_id, &docs_chk) in &per_dom_docs_checked {
        let p_tot = per_dom_probe_total.get(dom_id).copied().unwrap_or(0);
        let p_top1 = per_dom_probe_top1.get(dom_id).copied().unwrap_or(0);
        let pur_vec = per_dom_probe_purities.get(dom_id);
        let pur_avg = pur_vec.map_or(0.0, |v| v.iter().sum::<f64>() / v.len().max(1) as f64);
        let a_tot = per_dom_aff_tot.get(dom_id).copied().unwrap_or(1);
        let a_pass = per_dom_aff_pass.get(dom_id).copied().unwrap_or(0);
        per_domain_fit.insert(
            dom_id.clone(),
            DomainFitPerDomain {
                docs_checked: docs_chk,
                structural_pass: per_dom_struct_pass.get(dom_id).copied().unwrap_or(0),
                probe_top1_pass: p_top1,
                probe_total: p_tot,
                probe_top10_purity: pur_avg,
                affinity_pass_rate: a_pass as f64 / a_tot.max(1) as f64,
            },
        );
    }

    let p_len = probes.len().max(1) as f64;
    Ok(DomainFitAuditSummary {
        structural_docs_checked,
        structural_docs_passed,
        domain_structural_pass_rate: structural_docs_passed as f64 / structural_docs_checked.max(1) as f64,
        domain_probes_total: probes.len(),
        domain_fit_top1_accuracy: if probes.is_empty() { 0.0 } else { sum_top1_acc / p_len },
        domain_fit_top10_purity: if probes.is_empty() { 0.0 } else { sum_top10_purity / p_len },
        domain_mismatch_top1_count,
        doc_affinity_checked,
        doc_affinity_passed,
        doc_domain_affinity_accuracy: doc_affinity_passed as f64 / doc_affinity_checked.max(1) as f64,
        per_domain_fit,
        failure_samples,
    })
}

pub fn run_nl_search_nn_eval(cfg: &NlSearchEvalConfig) -> Result<NlSearchNnReport> {
    let embedder = OfflineHybridEmbedder::new();
    run_nl_search_nn_eval_with_embedder(cfg, &embedder)
}

pub fn run_nl_search_nn_eval_with_embedder(
    cfg: &NlSearchEvalConfig,
    embedder: &dyn DenseTextEmbedder,
) -> Result<NlSearchNnReport> {
    if cfg.split == EvalSplit::Holdout && !cfg.release_seal {
        bail!("--split holdout requires --release-seal flag (exit 2)");
    }

    let normalized_domains: Option<Vec<String>> = cfg.domains_filter.as_ref().and_then(|v| {
        if v.is_empty()
            || v.iter()
                .any(|s| s.eq_ignore_ascii_case("ALL") || s == "*")
        {
            None
        } else {
            Some(v.clone())
        }
    });
    let world = build_relational_world(
        cfg.split,
        cfg.cases_per_domain,
        normalized_domains.as_deref(),
        cfg.include_xr_twin,
    )?;
    let scenarios = generate_nl_scenarios(&world)?;

    let export_docs: Vec<WorldDocRecord> = if cfg.structured_layout {
        world
            .docs
            .iter()
            .map(|d| {
                let mut cloned = d.clone();
                cloned.rel_asset_path = d.structured_rel_asset_path();
                cloned.rel_audio_path = d.structured_rel_audio_path();
                cloned
            })
            .collect()
    } else {
        world.docs.clone()
    };
    let world_sha256_hex = sha256_jsonl(&export_docs)?;
    let queries_sha256_hex = sha256_jsonl(&scenarios)?;

    // Audit gov ID digits on raw docs and queries before indexing
    let mut gov_id_digit_leaks = 0usize;
    for d in &world.docs {
        let s = serde_json::to_string(d)?;
        if contains_forbidden_raw_gov_id_digits(&s) {
            gov_id_digit_leaks += 1;
        }
    }
    for sc in &scenarios {
        if contains_forbidden_raw_gov_id_digits(&sc.query_text) {
            gov_id_digit_leaks += 1;
        }
    }

    let seed2_idx = index_world_seed2(&world, embedder)?;
    let (e2e_idx, e2e_ingest) = index_world_e2e(&world, embedder)?;

    let world_doc_map: BTreeMap<String, &WorldDocRecord> = world
        .docs
        .iter()
        .map(|d| (d.doc_id.clone(), d))
        .collect();

    let seed2_eval = evaluate_arm("seed2", &seed2_idx, embedder, &scenarios, &world_doc_map)?;
    let e2e_eval = evaluate_arm("e2e", &e2e_idx, embedder, &scenarios, &world_doc_map)?;
    let corpus_size = compute_corpus_size_summary(&world.docs);
    let domain_fit_audit = run_domain_fit_audit(&e2e_idx, embedder, &world, &world_doc_map)?;

    let domains_covered = world
        .docs
        .iter()
        .map(|d| d.domain_id.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let mut langs = BTreeSet::new();
    for d in &world.docs {
        langs.insert(d.doc_lang.as_str());
    }
    for sc in &scenarios {
        langs.insert(sc.query_lang.as_str());
    }
    let langs_covered = langs.len();

    let mut by_domain = BTreeMap::new();
    for (k, s_m) in &seed2_eval.by_domain {
        by_domain.insert(
            k.clone(),
            DomainArmComparison {
                seed2: s_m.clone(),
                e2e: e2e_eval.by_domain.get(k).cloned().unwrap_or_default(),
            },
        );
    }
    let mut by_format = BTreeMap::new();
    for (k, s_m) in &seed2_eval.by_format {
        by_format.insert(
            k.clone(),
            DomainArmComparison {
                seed2: s_m.clone(),
                e2e: e2e_eval.by_format.get(k).cloned().unwrap_or_default(),
            },
        );
    }
    let mut by_archetype = BTreeMap::new();
    for (k, s_m) in &seed2_eval.by_archetype {
        by_archetype.insert(
            k.clone(),
            DomainArmComparison {
                seed2: s_m.clone(),
                e2e: e2e_eval.by_archetype.get(k).cloned().unwrap_or_default(),
            },
        );
    }
    let mut by_query_lang = BTreeMap::new();
    for (k, s_m) in &seed2_eval.by_lang {
        by_query_lang.insert(
            k.clone(),
            DomainArmComparison {
                seed2: s_m.clone(),
                e2e: e2e_eval.by_lang.get(k).cloned().unwrap_or_default(),
            },
        );
    }

    let mut format_domain_matrix: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for d in &world.docs {
        *format_domain_matrix
            .entry(d.domain_id.clone())
            .or_default()
            .entry(d.file_format.clone())
            .or_insert(0) += 1;
    }
    let format_domain_cells_covered: usize = format_domain_matrix
        .values()
        .map(|row| row.values().filter(|&&cnt| cnt >= 1).count())
        .sum();

    let mut hard_gate_failures = Vec::new();
    if corpus_size.min_doc_bytes < 512_000 {
        hard_gate_failures.push(format!("min_doc_bytes={} < 512000", corpus_size.min_doc_bytes));
    }
    if corpus_size.docs_under_500kb > 0 {
        hard_gate_failures.push(format!("docs_under_500kb={} > 0", corpus_size.docs_under_500kb));
    }
    if cfg.cases_per_domain >= 16 && cfg.domains_filter.is_none() && cfg.include_xr_twin && corpus_size.total_docs < 3000 {
        hard_gate_failures.push(format!("total_docs={} < 3000", corpus_size.total_docs));
    }
    if cfg.cases_per_domain >= 8 && cfg.domains_filter.is_none() && cfg.include_xr_twin {
        if format_domain_cells_covered < 152 {
            hard_gate_failures.push(format!(
                "format_domain_cells_covered={format_domain_cells_covered} < 152"
            ));
        }
        for (fmt, comp) in &by_format {
            if comp.e2e.positive_queries > 0 {
                if comp.e2e.recall_at_10 < 0.95 {
                    hard_gate_failures.push(format!(
                        "format {fmt} e2e r@10={:.4} < 0.95",
                        comp.e2e.recall_at_10
                    ));
                }
                if comp.e2e.mrr < 0.90 {
                    hard_gate_failures.push(format!(
                        "format {fmt} e2e mrr={:.4} < 0.90",
                        comp.e2e.mrr
                    ));
                }
            }
            let fmt_leaks = comp.e2e.security_leaks + comp.e2e.covert_leaks + comp.e2e.superseded_leaks;
            if fmt_leaks > 0 {
                hard_gate_failures.push(format!("format {fmt} e2e leaks={fmt_leaks}"));
            }
        }
    }
    if e2e_ingest.parse_failures > 0 {
        hard_gate_failures.push(format!("e2e parse_failures={}", e2e_ingest.parse_failures));
    }
    if e2e_ingest.wav_alignment_failures > 0 {
        hard_gate_failures.push(format!(
            "e2e wav_alignment_failures={}",
            e2e_ingest.wav_alignment_failures
        ));
    }
    if gov_id_digit_leaks > 0 {
        hard_gate_failures.push(format!("gov_id_digit_leaks={gov_id_digit_leaks}"));
    }
    let sql_total = e2e_eval.sql_total;
    let sql_passed = e2e_eval.sql_passed.min(seed2_eval.sql_passed);
    if sql_passed < sql_total {
        hard_gate_failures.push(format!("sql_guard_checks={sql_passed}/{sql_total}"));
    }
    let audio_total = e2e_eval.audio_total;
    let audio_matches = e2e_eval.audio_matches.min(seed2_eval.audio_matches);
    if audio_matches < audio_total {
        hard_gate_failures.push(format!("audio_span_matches={audio_matches}/{audio_total}"));
    }

    for (arm_label, overall, ans_q, xling, by_arch_map) in [
        (
            "seed2",
            &seed2_eval.overall,
            &seed2_eval.answer_quality,
            &seed2_eval.cross_lingual,
            &seed2_eval.by_archetype,
        ),
        (
            "e2e",
            &e2e_eval.overall,
            &e2e_eval.answer_quality,
            &e2e_eval.cross_lingual,
            &e2e_eval.by_archetype,
        ),
    ] {
        if overall.security_leaks > 0 {
            hard_gate_failures.push(format!("{arm_label} security_leaks={}", overall.security_leaks));
        }
        if overall.covert_leaks > 0 {
            hard_gate_failures.push(format!("{arm_label} covert_leaks={}", overall.covert_leaks));
        }
        if overall.superseded_leaks > 0 {
            hard_gate_failures.push(format!("{arm_label} superseded_leaks={}", overall.superseded_leaks));
        }
        if overall.mrr < 0.90 {
            hard_gate_failures.push(format!("{arm_label} mrr={:.4} < 0.90", overall.mrr));
        }
        if overall.recall_at_10 < 0.95 {
            hard_gate_failures.push(format!("{arm_label} recall@10={:.4} < 0.95", overall.recall_at_10));
        }
        if overall.ndcg_at_10 < 0.90 {
            hard_gate_failures.push(format!("{arm_label} ndcg@10={:.4} < 0.90", overall.ndcg_at_10));
        }
        if overall.hit_at_1 < 0.85 {
            hard_gate_failures.push(format!("{arm_label} hit@1={:.4} < 0.85", overall.hit_at_1));
        }
        if overall.zero_hit_accuracy < 1.0 {
            hard_gate_failures.push(format!(
                "{arm_label} zero_hit_accuracy={:.4} < 1.0",
                overall.zero_hit_accuracy
            ));
        }
        if xling.positive_queries > 0 && xling.mrr < 0.85 {
            hard_gate_failures.push(format!("{arm_label} cross_lingual mrr={:.4} < 0.85", xling.mrr));
        }
        if let Some(qa3) = by_arch_map.get("Q-A3") {
            if qa3.recall_at_10 < 0.95 || qa3.relay_recall < 0.95 {
                hard_gate_failures.push(format!(
                    "{arm_label} Q-A3 r@10={:.4} relay_recall={:.4} < 0.95",
                    qa3.recall_at_10, qa3.relay_recall
                ));
            }
        }
        if ans_q.fact_recall < 0.95 {
            hard_gate_failures.push(format!("{arm_label} fact_recall={:.4} < 0.95", ans_q.fact_recall));
        }
        if ans_q.numeric_exact_rate < 0.98 {
            hard_gate_failures.push(format!("{arm_label} numeric_exact_rate={:.4} < 0.98", ans_q.numeric_exact_rate));
        }
        if ans_q.citation_precision < 0.95 {
            hard_gate_failures.push(format!("{arm_label} citation_precision={:.4} < 0.95", ans_q.citation_precision));
        }
        if ans_q.groundedness_rate < 0.98 {
            hard_gate_failures.push(format!("{arm_label} groundedness_rate={:.4} < 0.98", ans_q.groundedness_rate));
        }
        if ans_q.citizen_guard_clean_rate < 1.0 {
            hard_gate_failures.push(format!("{arm_label} citizen_guard_clean_rate={:.4} < 1.0", ans_q.citizen_guard_clean_rate));
        }
        if ans_q.abstention_accuracy < 1.0 {
            hard_gate_failures.push(format!("{arm_label} abstention_accuracy={:.4} < 1.0", ans_q.abstention_accuracy));
        }
    }

    if domain_fit_audit.domain_structural_pass_rate < 1.0 {
        hard_gate_failures.push(format!(
            "domain_structural_pass_rate={:.4} < 1.0",
            domain_fit_audit.domain_structural_pass_rate
        ));
    }
    if domain_fit_audit.domain_fit_top1_accuracy < 1.0 {
        hard_gate_failures.push(format!(
            "domain_fit_top1_accuracy={:.4} < 1.0",
            domain_fit_audit.domain_fit_top1_accuracy
        ));
    }
    if domain_fit_audit.domain_fit_top10_purity < 0.99 {
        hard_gate_failures.push(format!(
            "domain_fit_top10_purity={:.4} < 0.99",
            domain_fit_audit.domain_fit_top10_purity
        ));
    }
    if domain_fit_audit.domain_mismatch_top1_count > 0 {
        hard_gate_failures.push(format!(
            "domain_mismatch_top1_count={} > 0",
            domain_fit_audit.domain_mismatch_top1_count
        ));
    }
    if domain_fit_audit.doc_domain_affinity_accuracy < 0.99 {
        hard_gate_failures.push(format!(
            "doc_domain_affinity_accuracy={:.4} < 0.99",
            domain_fit_audit.doc_domain_affinity_accuracy
        ));
    }

    let exit_code = if hard_gate_failures.is_empty() { 0 } else { 1 };
    let report = NlSearchNnReport {
        split: cfg.split,
        cases_per_domain: cfg.cases_per_domain,
        domains_covered,
        langs_covered,
        total_docs: world.docs.len(),
        total_queries: scenarios.len(),
        corpus_size,
        embedder_id: embedder.model_id().into(),
        embedder_dim: embedder.dim(),
        world_sha256_hex,
        queries_sha256_hex,
        e2e_ingest,
        gov_id_digit_leaks,
        sql_guard_checks_passed: sql_passed,
        sql_guard_checks_total: sql_total,
        audio_span_matches: audio_matches,
        audio_span_total: audio_total,
        seed2_overall: seed2_eval.overall,
        e2e_overall: e2e_eval.overall,
        seed2_answer_quality: seed2_eval.answer_quality,
        e2e_answer_quality: e2e_eval.answer_quality,
        domain_fit_audit,
        seed2_cross_lingual: seed2_eval.cross_lingual,
        e2e_cross_lingual: e2e_eval.cross_lingual,
        by_domain,
        by_format,
        by_archetype,
        by_query_lang,
        format_domain_matrix,
        format_domain_cells_covered,
        hard_gate_failures,
        exit_code,
    };

    if let Some(ref dir) = cfg.out_dir {
        fs::create_dir_all(dir)?;
        if cfg.structured_layout {
            let rel_dir = dir.join("relational");
            let eval_dir = dir.join("eval");
            fs::create_dir_all(&rel_dir)?;
            fs::create_dir_all(&eval_dir)?;

            write_jsonl(&rel_dir.join("world_docs.jsonl"), &export_docs)?;
            write_jsonl(&rel_dir.join("queries_nl.jsonl"), &scenarios)?;
            fs::write(
                rel_dir.join("format_domain_matrix.json"),
                serde_json::to_string_pretty(&report.format_domain_matrix)?,
            )?;
            write_jsonl(&eval_dir.join("search_results_seed2.jsonl"), &seed2_eval.rows)?;
            write_jsonl(&eval_dir.join("search_results_e2e.jsonl"), &e2e_eval.rows)?;
            fs::write(
                eval_dir.join("nl_search_nn_report.json"),
                serde_json::to_string_pretty(&report)?,
            )?;

            let mut docs_by_slug: BTreeMap<String, Vec<WorldDocRecord>> = BTreeMap::new();
            for d in &export_docs {
                docs_by_slug.entry(d.domain_dir_slug()).or_default().push(d.clone());
            }
            let mut qs_by_slug: BTreeMap<String, Vec<NlSearchScenario>> = BTreeMap::new();
            for q in &scenarios {
                let slug = format!("{}_{}", q.domain_id, q.pack_id);
                qs_by_slug.entry(slug).or_default().push(q.clone());
            }

            let mut domain_manifests = Vec::new();
            for (slug, dom_docs) in &docs_by_slug {
                let dom_dir = dir.join("domains").join(slug);
                fs::create_dir_all(&dom_dir)?;
                let dom_qs = qs_by_slug.get(slug).map(Vec::as_slice).unwrap_or(&[]);
                write_jsonl(&dom_dir.join("records.jsonl"), dom_docs)?;
                write_jsonl(&dom_dir.join("queries.jsonl"), dom_qs)?;
                let wav_cnt = dom_docs.iter().filter(|d| d.rel_audio_path.is_some()).count();
                let mut fmt_counts: BTreeMap<String, usize> = BTreeMap::new();
                for d in dom_docs {
                    *fmt_counts.entry(d.file_format.clone()).or_insert(0) += 1;
                }
                domain_manifests.push(serde_json::json!({
                    "domain_id": dom_docs[0].domain_id,
                    "pack_id": dom_docs[0].pack_id,
                    "dir": format!("domains/{slug}"),
                    "docs": dom_docs.len(),
                    "audio_wavs": wav_cnt,
                    "queries": dom_qs.len(),
                    "formats": fmt_counts,
                }));
            }

            if cfg.dump_synth {
                for doc in &export_docs {
                    if let Some(bytes) = world.rendered_bytes.get(&doc.doc_id) {
                        let full = dir.join(&doc.rel_asset_path);
                        if let Some(p) = full.parent() {
                            fs::create_dir_all(p)?;
                        }
                        fs::write(full, bytes)?;
                    }
                    if let (Some(rel_aud), Some(wav)) =
                        (&doc.rel_audio_path, world.audio_wav_bytes.get(&doc.doc_id))
                    {
                        let full = dir.join(rel_aud);
                        if let Some(p) = full.parent() {
                            fs::create_dir_all(p)?;
                        }
                        fs::write(full, wav)?;
                    }
                }
            }

            let seed_val = match cfg.split {
                EvalSplit::Diag => DOCS_NN_DIAG_SEED,
                EvalSplit::Holdout => DOCS_NN_HOLDOUT_SEED,
            };
            let total_audio_wavs = export_docs.iter().filter(|d| d.rel_audio_path.is_some()).count();
            let manifest = serde_json::json!({
                "split": cfg.split,
                "seed": seed_val,
                "cases_per_domain": cfg.cases_per_domain,
                "domains_covered": report.domains_covered,
                "formats_covered": report.by_format.len(),
                "format_domain_cells_covered": report.format_domain_cells_covered,
                "langs_covered": report.langs_covered,
                "total_docs": report.total_docs,
                "total_audio_wavs": total_audio_wavs,
                "total_queries": report.total_queries,
                "min_doc_bytes": report.corpus_size.min_doc_bytes,
                "median_doc_bytes": report.corpus_size.median_doc_bytes,
                "avg_doc_bytes": report.corpus_size.avg_doc_bytes,
                "max_doc_bytes": report.corpus_size.max_doc_bytes,
                "total_bytes": report.corpus_size.total_bytes,
                "docs_under_500kb": report.corpus_size.docs_under_500kb,
                "e2e_answer_quality": report.e2e_answer_quality,
                "domain_fit_audit": {
                    "domain_structural_pass_rate": report.domain_fit_audit.domain_structural_pass_rate,
                    "domain_fit_top1_accuracy": report.domain_fit_audit.domain_fit_top1_accuracy,
                    "domain_fit_top10_purity": report.domain_fit_audit.domain_fit_top10_purity,
                    "domain_mismatch_top1_count": report.domain_fit_audit.domain_mismatch_top1_count,
                    "doc_domain_affinity_accuracy": report.domain_fit_audit.doc_domain_affinity_accuracy,
                },
                "world_sha256_hex": report.world_sha256_hex,
                "queries_sha256_hex": report.queries_sha256_hex,
                "domains": domain_manifests,
            });
            fs::write(
                dir.join("manifest.json"),
                serde_json::to_string_pretty(&manifest)?,
            )?;
        } else {
            write_jsonl(&dir.join("world_docs.jsonl"), &export_docs)?;
            write_jsonl(&dir.join("queries_nl.jsonl"), &scenarios)?;
            write_jsonl(&dir.join("search_results_seed2.jsonl"), &seed2_eval.rows)?;
            write_jsonl(&dir.join("search_results_e2e.jsonl"), &e2e_eval.rows)?;
            fs::write(
                dir.join("nl_search_nn_report.json"),
                serde_json::to_string_pretty(&report)?,
            )?;
            if cfg.dump_synth {
                for doc in &export_docs {
                    if let Some(bytes) = world.rendered_bytes.get(&doc.doc_id) {
                        let full = dir.join(&doc.rel_asset_path);
                        if let Some(p) = full.parent() {
                            fs::create_dir_all(p)?;
                        }
                        fs::write(full, bytes)?;
                    }
                    if let (Some(rel_aud), Some(wav)) =
                        (&doc.rel_audio_path, world.audio_wav_bytes.get(&doc.doc_id))
                    {
                        let full = dir.join(rel_aud);
                        if let Some(p) = full.parent() {
                            fs::create_dir_all(p)?;
                        }
                        fs::write(full, wav)?;
                    }
                }
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holdout_without_release_seal_is_rejected() {
        let cfg = NlSearchEvalConfig {
            split: EvalSplit::Holdout,
            release_seal: false,
            ..Default::default()
        };
        assert!(run_nl_search_nn_eval(&cfg).is_err());
    }

    #[test]
    fn diag_all_18_domains_and_xrc_twin_nl_search_nn_passes_seed2_and_e2e_and_is_deterministic() {
        let base = std::env::temp_dir().join(format!("louis-nl-eval-{}", std::process::id()));
        let dir_a = base.join("a");
        let dir_b = base.join("b");
        let _ = fs::remove_dir_all(&base);

        let cfg_a = NlSearchEvalConfig {
            split: EvalSplit::Diag,
            release_seal: false,
            cases_per_domain: 2,
            domains_filter: None,
            include_xr_twin: true,
            out_dir: Some(dir_a.clone()),
            dump_synth: true,
            structured_layout: false,
        };
        let cfg_b = NlSearchEvalConfig {
            out_dir: Some(dir_b.clone()),
            dump_synth: false,
            ..cfg_a.clone()
        };

        let rep_a = run_nl_search_nn_eval(&cfg_a).unwrap();
        let rep_b = run_nl_search_nn_eval(&cfg_b).unwrap();

        assert_eq!(rep_a.world_sha256_hex, rep_b.world_sha256_hex);
        assert_eq!(rep_a.queries_sha256_hex, rep_b.queries_sha256_hex);
        assert_eq!(rep_a.domains_covered, 19);
        assert_eq!(rep_a.by_format.len(), 8);
        assert_eq!(rep_a.langs_covered, 15);
        assert_eq!(rep_a.e2e_ingest.parse_failures, 0);
        assert_eq!(rep_a.e2e_ingest.wav_alignment_failures, 0);
        assert!(rep_a.e2e_ingest.stripped_very_hidden_sheets > 0);
        assert!(rep_a.e2e_ingest.stripped_covert_artifacts > 0);
        assert_eq!(rep_a.gov_id_digit_leaks, 0);
        assert_eq!(rep_a.sql_guard_checks_passed, rep_a.sql_guard_checks_total);
        assert_eq!(rep_a.audio_span_matches, rep_a.audio_span_total);
        assert_eq!(
            rep_a.exit_code,
            0,
            "hard gates failed: {:?} | answer_qa_e2e={:?} | domain_fit_samples={:?}",
            rep_a.hard_gate_failures,
            rep_a.e2e_answer_quality,
            rep_a.domain_fit_audit.failure_samples
        );
        assert!(rep_a.seed2_overall.mrr >= 0.90);
        assert!(rep_a.e2e_overall.mrr >= 0.90);
        assert!(rep_a.e2e_overall.recall_at_10 >= 0.95);
        assert!(rep_a.e2e_overall.ndcg_at_10 >= 0.90);
        assert!(rep_a.e2e_overall.hit_at_1 >= 0.85);
        assert!(rep_a.e2e_overall.zero_hit_queries > 0);
        assert_eq!(rep_a.e2e_overall.zero_hit_accuracy, 1.0);
        assert!(rep_a.e2e_cross_lingual.mrr >= 0.85);
        assert!(rep_a.corpus_size.min_doc_bytes >= 512_000);
        assert_eq!(rep_a.corpus_size.docs_under_500kb, 0);
        assert!(rep_a.e2e_answer_quality.fact_recall >= 0.95);
        assert!(rep_a.e2e_answer_quality.numeric_exact_rate >= 0.98);
        assert!(rep_a.e2e_answer_quality.citation_precision >= 0.95);
        assert!(rep_a.e2e_answer_quality.groundedness_rate >= 0.98);
        assert_eq!(rep_a.e2e_answer_quality.citizen_guard_clean_rate, 1.0);
        assert_eq!(rep_a.e2e_answer_quality.abstention_accuracy, 1.0);
        assert_eq!(rep_a.domain_fit_audit.domain_structural_pass_rate, 1.0);
        assert_eq!(rep_a.domain_fit_audit.domain_fit_top1_accuracy, 1.0);
        assert!(rep_a.domain_fit_audit.domain_fit_top10_purity >= 0.99);
        assert_eq!(rep_a.domain_fit_audit.domain_mismatch_top1_count, 0);
        assert!(rep_a.domain_fit_audit.doc_domain_affinity_accuracy >= 0.99);
        assert!(dir_a.join("world_docs.jsonl").is_file());
        assert!(dir_a.join("queries_nl.jsonl").is_file());
        assert!(dir_a.join("nl_search_nn_report.json").is_file());

        let _ = fs::remove_dir_all(&base);
    }
}
