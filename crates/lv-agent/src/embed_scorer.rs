//! Zero-`laya` bipolar embedding margin axis scorer (`EmbedAxisScorer`) powered by
//! 768d `lv_core::gemma_embed::GemmaEmbedder`, double-centered against pool mean/std + Gumbel expectation,
//! with optional 1-token `Qwen35(Nano08B)` `[VECTOR EVIDENCE]` tie-break delegation.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use lv_core::gemma_embed::{cosine, GemmaEmbedder};

use crate::gates::{AgentVerdict, AxisScorer};

pub const DEFAULT_MARGIN_HIGH: f32 = 0.35;
pub const DEFAULT_PREJ_WEIGHT: f32 = 0.65;
pub const CALIBRATION_GAMMA: f32 = 8.5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxisAnchorSet {
    pub axis_name: String,
    /// Positive anchors supporting `rule` verdict (768d L2-normalized vectors).
    pub rule_bias: Vec<Vec<f32>>,
    /// Prejudice / predicate-negation (`PolarityFlip`) / exclusionary condition anchors against `rule`.
    pub rule_prejudice: Vec<Vec<f32>>,
    /// Positive anchors supporting `counter` verdict.
    pub counter_bias: Vec<Vec<f32>>,
    /// Prejudice / exclusionary anchors against `counter`.
    pub counter_prejudice: Vec<Vec<f32>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TieBreakPick {
    RuleA,
    CounterB,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TieBreakEvidencePrompt {
    pub axis_name: String,
    pub prompt: String,
    pub delta_s: f32,
    pub s_rule: f32,
    pub s_counter: f32,
}

/// Optional 1-token selector (`"A"` / `"B"`) invoked strictly when `|ΔS| < margin_high`.
pub trait TieBreakSelector: Send + Sync {
    fn select_tie_break(&self, ev: &TieBreakEvidencePrompt) -> Result<(TieBreakPick, f32)>;
}

/// Deterministic offline 1-token `[VECTOR EVIDENCE]` selector modeling `Qwen35(Nano08B)` behavior
/// when polarity / sign / fact-level tie-breaking is needed without an external model file.
pub struct DeterministicNanoTieBreaker;

impl TieBreakSelector for DeterministicNanoTieBreaker {
    fn select_tie_break(&self, ev: &TieBreakEvidencePrompt) -> Result<(TieBreakPick, f32)> {
        let pick = if ev.delta_s >= 0.0 && ev.s_rule > ev.s_counter {
            TieBreakPick::RuleA
        } else {
            TieBreakPick::CounterB
        };
        let conf = 0.55 + (ev.delta_s.abs().min(DEFAULT_MARGIN_HIGH) * 0.40);
        Ok((pick, conf.clamp(0.51, 0.90)))
    }
}

/// Bipolar Gumbel double-centered axis scorer replacing retired `laya`.
pub struct EmbedAxisScorer {
    pub gemma: Arc<GemmaEmbedder>,
    pub banks: HashMap<String, AxisAnchorSet>,
    pub tie_breaker: Option<Arc<dyn TieBreakSelector>>,
    pub margin_high: f32,
    pub prej_weight: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarginBreakdown {
    pub s_rule: f32,
    pub s_counter: f32,
    pub delta_s: f32,
    pub used_llm_tie_break: bool,
    pub probs: [f32; 2],
}

impl EmbedAxisScorer {
    pub fn new(
        gemma: Arc<GemmaEmbedder>,
        banks: HashMap<String, AxisAnchorSet>,
        tie_breaker: Option<Arc<dyn TieBreakSelector>>,
    ) -> Self {
        Self {
            gemma,
            banks,
            tie_breaker,
            margin_high: DEFAULT_MARGIN_HIGH,
            prej_weight: DEFAULT_PREJ_WEIGHT,
        }
    }

    pub fn gumbel_expected_max(n: usize) -> f32 {
        if n <= 1 {
            return 0.0;
        }
        // Standard expected max of n standard normals approximation via sqrt(2 ln n) - Euler/ln factor.
        let ln_n = (n as f32).ln();
        (2.0 * ln_n).sqrt() - 0.5772 / (n as f32).max(2.0)
    }

    /// Evaluates `[p_rule, p_counter]` given an embedded query vector `q` and anchor set `set`.
    pub fn score_vector(&self, query_vec: &[f32], set: &AxisAnchorSet, context_summary: &str) -> Result<MarginBreakdown> {
        if set.rule_bias.is_empty() && set.counter_bias.is_empty() {
            bail!("anchor set '{}' has no bias anchors", set.axis_name);
        }
        let mut all_sims = Vec::new();
        let rule_pos_sims: Vec<f32> = set.rule_bias.iter().map(|a| cosine(query_vec, a)).collect();
        let rule_neg_sims: Vec<f32> = set.rule_prejudice.iter().map(|a| cosine(query_vec, a).max(0.0)).collect();
        let cnt_pos_sims: Vec<f32> = set.counter_bias.iter().map(|a| cosine(query_vec, a)).collect();
        let cnt_neg_sims: Vec<f32> = set.counter_prejudice.iter().map(|a| cosine(query_vec, a).max(0.0)).collect();

        all_sims.extend_from_slice(&rule_pos_sims);
        all_sims.extend_from_slice(&rule_neg_sims);
        all_sims.extend_from_slice(&cnt_pos_sims);
        all_sims.extend_from_slice(&cnt_neg_sims);

        let mean = if all_sims.is_empty() {
            0.0
        } else {
            all_sims.iter().sum::<f32>() / (all_sims.len() as f32)
        };
        let var = if all_sims.len() <= 1 {
            1e-4
        } else {
            all_sims
                .iter()
                .map(|&s| (s - mean) * (s - mean))
                .sum::<f32>()
                / (all_sims.len() as f32)
        };
        let sigma = var.sqrt().max(1e-3);

        let max_or_zero = |xs: &[f32]| xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let compute_net = |pos: &[f32], neg: &[f32]| -> f32 {
            let p_max = if pos.is_empty() { mean } else { max_or_zero(pos) };
            let s_pos = ((p_max - mean) / sigma) - Self::gumbel_expected_max(pos.len());
            let s_neg_raw = if neg.is_empty() { 0.0 } else { max_or_zero(neg).max(0.0) };
            let s_neg = ((s_neg_raw - mean) / sigma).max(0.0);
            s_pos - self.prej_weight * s_neg
        };

        let s_rule = compute_net(&rule_pos_sims, &rule_neg_sims);
        let s_counter = compute_net(&cnt_pos_sims, &cnt_neg_sims);
        let delta_s = s_rule - s_counter;

        // 3-zone decision rule (Step 0 zero-LLM vs Step 1 Nano08B tie-breaker):
        if delta_s >= self.margin_high && s_rule > 0.0 {
            let raw_prob = 1.0 / (1.0 + (-CALIBRATION_GAMMA * delta_s).exp());
            let p_rule = raw_prob.max(0.95);
            return Ok(MarginBreakdown {
                s_rule,
                s_counter,
                delta_s,
                used_llm_tie_break: false,
                probs: [p_rule, (1.0 - p_rule).max(0.0)],
            });
        }
        if delta_s <= -self.margin_high || s_rule <= 0.0 {
            let p_rule = (1.0 / (1.0 + (-CALIBRATION_GAMMA * delta_s).exp())).min(0.45);
            return Ok(MarginBreakdown {
                s_rule,
                s_counter,
                delta_s,
                used_llm_tie_break: false,
                probs: [p_rule, (1.0 - p_rule).max(0.55)],
            });
        }

        // Marginal band (|delta_s| < margin_high): Step 1 Nano08B `[VECTOR EVIDENCE]` or rules-only hold.
        if let Some(tb) = &self.tie_breaker {
            let ev = TieBreakEvidencePrompt {
                axis_name: set.axis_name.clone(),
                prompt: format!(
                    "[VECTOR EVIDENCE]\naxis={}\ndelta_s={delta_s:.3} s_rule={s_rule:.3} s_counter={s_counter:.3}\ncontext={context_summary}\nChoose 1 token: A (rule) or B (counter)",
                    set.axis_name
                ),
                delta_s,
                s_rule,
                s_counter,
            };
            let (pick, conf) = tb.select_tie_break(&ev)?;
            let probs = match pick {
                TieBreakPick::RuleA => [conf, (1.0 - conf).max(0.0)],
                TieBreakPick::CounterB => [(1.0 - conf).min(0.49), conf.max(0.51)],
            };
            Ok(MarginBreakdown {
                s_rule,
                s_counter,
                delta_s,
                used_llm_tie_break: true,
                probs,
            })
        } else {
            // RulesOnly: keep p_rule <= p_counter so judge_by_axes cleanly yields confidence = None (PendingOfficer).
            Ok(MarginBreakdown {
                s_rule,
                s_counter,
                delta_s,
                used_llm_tie_break: false,
                probs: [0.49, 0.51],
            })
        }
    }

    pub fn build_anchor_set_from_phrases(
        gemma: &GemmaEmbedder,
        axis_name: impl Into<String>,
        rule_phrases: &[&str],
        rule_prej_phrases: &[&str],
        counter_phrases: &[&str],
        counter_prej_phrases: &[&str],
    ) -> Result<AxisAnchorSet> {
        let enc = |xs: &[&str]| -> Result<Vec<Vec<f32>>> {
            xs.iter().map(|s| gemma.embed_text(s)).collect()
        };
        Ok(AxisAnchorSet {
            axis_name: axis_name.into(),
            rule_bias: enc(rule_phrases)?,
            rule_prejudice: enc(rule_prej_phrases)?,
            counter_bias: enc(counter_phrases)?,
            counter_prejudice: enc(counter_prej_phrases)?,
        })
    }
}

impl AxisScorer for EmbedAxisScorer {
    fn score_pair(
        &self,
        title: &str,
        axis_facts: &[(String, String)],
        rule: AgentVerdict,
        counter: AgentVerdict,
    ) -> Result<[f32; 2]> {
        let Some(set) = self.banks.get(title) else {
            // Missing anchor bank holds cleanly below 0.5 so PendingOfficer catches unanchored axes.
            return Ok([0.45, 0.55]);
        };
        let mut joined = format!("axis={title} rule={rule:?} counter={counter:?}");
        for (k, v) in axis_facts {
            joined.push(' ');
            joined.push_str(k);
            joined.push(':');
            joined.push_str(v);
        }
        let q = self.gemma.embed_text(&joined)?;
        let br = self.score_vector(&q, set, &joined)?;
        Ok(br.probs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lv_core::gemma_embed::normalize_l2;

    fn unit_basis(idx: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; 768];
        v[idx] = 1.0;
        v
    }

    #[test]
    fn high_margin_exits_at_step0_with_zero_llm_calls_and_polarity_flip_blocks() {
        let gemma = Arc::new(GemmaEmbedder::deterministic_for_tests(42));
        let mut banks = HashMap::new();
        banks.insert(
            "homonym".into(),
            AxisAnchorSet {
                axis_name: "homonym".into(),
                rule_bias: vec![unit_basis(0)],
                rule_prejudice: vec![unit_basis(2)],
                counter_bias: vec![unit_basis(1)],
                counter_prejudice: vec![],
            },
        );
        let scorer = EmbedAxisScorer::new(gemma, banks, Some(Arc::new(DeterministicNanoTieBreaker)));
        let set = scorer.banks.get("homonym").unwrap();

        // Strong positive margin -> zero LLM calls, confidence >= 0.95
        let q_pass = unit_basis(0);
        let br_pass = scorer.score_vector(&q_pass, set, "distinct homonym").unwrap();
        assert!(!br_pass.used_llm_tie_break);
        assert!(br_pass.probs[0] >= 0.95);

        // PolarityFlip prejudice active -> confidence collapses below counter
        let mut q_flip = vec![0.0f32; 768];
        q_flip[0] = 0.4;
        q_flip[2] = 0.9;
        normalize_l2(&mut q_flip);
        let br_flip = scorer.score_vector(&q_flip, set, "negated condition").unwrap();
        assert!(!br_flip.used_llm_tie_break);
        assert!(br_flip.probs[0] <= br_flip.probs[1]);

        // Marginal band triggers Nano08B `[VECTOR EVIDENCE]` tie-break
        let mut q_mid = vec![0.0f32; 768];
        q_mid[0] = 0.72;
        q_mid[1] = 0.69;
        normalize_l2(&mut q_mid);
        let br_mid = scorer.score_vector(&q_mid, set, "close boundary").unwrap();
        assert!(br_mid.used_llm_tie_break);
    }
}
