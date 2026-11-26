//! Multi-clue equipment identification (`L1..L3`, 7 clues, abstention when `gap < 1.0`) and 2-hop Off-Page Connector tracer.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

pub const MIN_SCORE: f32 = 2.5;
pub const MIN_GAP: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DamageLevel {
    L1CodeLost,
    L2CodeAndNameLost,
    L3AllTextLost,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityClues {
    pub symbol_class: Option<String>,
    pub residual_tag: Option<String>,
    pub neighbor_upstream: Option<String>,
    pub neighbor_downstream: Option<String>,
    pub line_spec: Option<String>,
    pub embed_sim_by_candidate: HashMap<String, f32>,
    pub nominal_zone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityCandidate {
    pub id: String,
    pub symbol_class: String,
    pub canonical_tag: String,
    pub upstream: Option<String>,
    pub downstream: Option<String>,
    pub line_spec: Option<String>,
    pub zone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IdentifyOutcome {
    Identified { id: String, score: f32, margin: f32 },
    PendingOfficer { top: Option<String>, score: f32, margin: f32, reason: &'static str },
}

/// Fuses up to 7 multimodal/topological clues and abstains (`PendingOfficer`) when `score < 2.5` or `gap < 1.0`.
pub fn identify_entity(clues: &EntityClues, pool: &[EntityCandidate]) -> IdentifyOutcome {
    if pool.is_empty() {
        return IdentifyOutcome::PendingOfficer {
            top: None,
            score: 0.0,
            margin: 0.0,
            reason: "empty candidate pool",
        };
    }

    let mut scored: Vec<(&str, f32)> = pool
        .iter()
        .map(|cand| {
            let mut s = 0.0f32;
            if let Some(ref sym) = clues.symbol_class {
                if sym == &cand.symbol_class {
                    s += 1.4;
                }
            }
            if let Some(ref tag) = clues.residual_tag {
                if cand.canonical_tag.contains(tag.as_str()) {
                    s += 1.5;
                }
            }
            if clues.neighbor_upstream.is_some() && clues.neighbor_upstream == cand.upstream {
                s += 1.0;
            }
            if clues.neighbor_downstream.is_some() && clues.neighbor_downstream == cand.downstream {
                s += 1.0;
            }
            if clues.line_spec.is_some() && clues.line_spec == cand.line_spec {
                s += 0.6;
            }
            if let Some(&sim) = clues.embed_sim_by_candidate.get(&cand.id) {
                if sim > 0.0 {
                    s += sim.clamp(0.0, 1.0) * 1.2;
                }
            }
            if clues.nominal_zone.is_some() && clues.nominal_zone == cand.zone {
                s += 0.5;
            }
            (cand.id.as_str(), s)
        })
        .collect();

    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let (top_id, top_score) = scored[0];
    let second_score = scored.get(1).map_or(0.0, |x| x.1);
    let margin = top_score - second_score;

    if top_score < MIN_SCORE {
        return IdentifyOutcome::PendingOfficer {
            top: Some(top_id.to_string()),
            score: top_score,
            margin,
            reason: "score_below_min_2_5",
        };
    }
    if scored.len() > 1 && margin < MIN_GAP {
        return IdentifyOutcome::PendingOfficer {
            top: Some(top_id.to_string()),
            score: top_score,
            margin,
            reason: "ambiguous_gap_below_1_0",
        };
    }

    IdentifyOutcome::Identified {
        id: top_id.to_string(),
        score: top_score,
        margin,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorEdge {
    pub from_node: String,
    pub to_node: String,
    pub connector_id: String,
}

/// Traverses upstream Off-Page Connectors (`max_hops`, typically 2) to locate root upstream equipment nodes.
pub fn trace_upstream_connectors(
    edges: &[ConnectorEdge],
    start_node: &str,
    max_hops: usize,
) -> Vec<(String, usize, Vec<String>)> {
    let mut visited = std::collections::HashSet::new();
    visited.insert(start_node.to_string());
    let mut q = VecDeque::new();
    q.push_back((start_node.to_string(), 0usize, Vec::<String>::new()));
    let mut reached = Vec::new();

    while let Some((cur, depth, trail)) = q.pop_front() {
        if depth > 0 {
            reached.push((cur.clone(), depth, trail.clone()));
        }
        if depth >= max_hops {
            continue;
        }
        for e in edges {
            if e.to_node == cur && visited.insert(e.from_node.clone()) {
                let mut next_trail = trail.clone();
                next_trail.push(e.connector_id.clone());
                q.push_back((e.from_node.clone(), depth + 1, next_trail));
            }
        }
    }
    reached
}
