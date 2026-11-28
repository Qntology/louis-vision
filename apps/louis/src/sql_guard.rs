//! 5-Layer Local DataFusion SQL AST Safety Guard (`sql_guard.rs` — single canonical source for `louis-docs` and `louis-desktop`):
//! - G-SQL1: Single `SELECT` statement only (blocks DDL/DML/COPY/ATTACH/PRAGMA/INTO OUTFILE/file-scan functions).
//! - G-SQL2: 5 Arrow view + physical column whitelist.
//! - G-SQL3: Mandatory `vault_id` & `n2sf_grade_rank` inner-scan isolation (`CounterKiosk` -> `KioskForbidden`).
//! - G-SQL4: Single view / no CROSS JOIN / `LIMIT <= 200` / `1500 ms` timeout budget.
//! - G-SQL5: Serialized output cell inspection through Layer 0 `WriterGuard` (`CITIZEN_FORBIDDEN_TERMS` 16 terms + zero-digit Gov IDs).

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{ensure, Result};
use lv_doc::writer::guard::{N2sfGrade, WriterGuard};
use lv_router::vault::{DeviceRole, VaultError};
use serde::{Deserialize, Serialize};
use sqlparser::ast::{
    BinaryOperator, Expr, FunctionArg, FunctionArgExpr, FunctionArguments, GroupByExpr, Ident,
    JoinOperator, LimitClause, OrderByKind, Query, SelectItem, SetExpr, Statement, TableFactor,
    Value, ValueWithSpan,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

use crate::eval_nn::domains::contains_forbidden_raw_gov_id_digits;

pub const HARD_ROW_LIMIT: u64 = 200;
pub const SQL_MAX_LIMIT: u64 = HARD_ROW_LIMIT;
pub const SQL_TIMEOUT_MS: u64 = 1_500;
const RESERVED_SECURITY_COLS: &[&str] = &["vault_id", "n2sf_grade", "n2sf_grade_rank"];

pub const ALLOWED_ARROW_VIEWS: &[(&str, &[&str])] = &[
    (
        "docs_entities",
        &[
            "doc_id",
            "vault_id",
            "n2sf_grade_rank",
            "n2sf_grade",
            "pack_id",
            "vendor_id",
            "entity_key",
            "amt_krw",
            "currency",
            "status",
            "doc_date",
        ],
    ),
    (
        "gate_ledger",
        &[
            "entry_id",
            "vault_id",
            "n2sf_grade_rank",
            "n2sf_grade",
            "doc_id",
            "gate_code",
            "verdict",
            "overridden",
            "verified_krw",
            "evidence_krw",
            "risk_score",
            "created_at",
        ],
    ),
    (
        "media_events",
        &[
            "media_id",
            "vault_id",
            "n2sf_grade_rank",
            "n2sf_grade",
            "ts_ms",
            "bbox",
            "label",
            "scheme",
        ],
    ),
    (
        "sim_samples",
        &[
            "run_id",
            "vault_id",
            "n2sf_grade_rank",
            "n2sf_grade",
            "tick",
            "unit_id",
            "err_m",
            "provenance",
        ],
    ),
    (
        "stage_cues",
        &[
            "cue_id",
            "vault_id",
            "n2sf_grade_rank",
            "n2sf_grade",
            "smpte",
            "delta_m",
            "interlock_ok",
            "ts_ms",
        ],
    ),
];

pub const FORBIDDEN_FILE_FUNCTIONS: &[&str] = &[
    "read_file",
    "write_file",
    "csv_scan",
    "parquet_scan",
    "json_scan",
    "load_extension",
    "copy_to",
    "read_csv",
    "read_parquet",
    "read_json",
];

const FORBIDDEN_SQL_KEYWORDS: &[&str] = &[
    "COPY ",
    "ATTACH ",
    "DETACH ",
    "PRAGMA ",
    "INTO OUTFILE",
    "READ_PARQUET",
    "READ_CSV",
    "READ_JSON",
    "PARQUET_SCAN",
    "CSV_SCAN",
    "JSON_SCAN",
    "READ_FILE",
    "WRITE_FILE",
    "LOAD_EXTENSION",
    "INFORMATION_SCHEMA",
    "PG_CATALOG",
];

pub type SqlRowMap = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SqlGuardGate {
    G1SingleSelectOnly,
    G2TableColumnWhitelist,
    G3VaultAndN2sfScope,
    G4LimitAndJoinBudget,
    G5WriterOutputFilter,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SqlGuardError {
    #[error("KioskForbidden: CounterKiosk cannot execute DataFusion SQL")]
    KioskForbidden,
    #[error("[{gate:?}] blocked SQL query: {reason}")]
    Blocked { gate: SqlGuardGate, reason: String },
}

impl From<VaultError> for SqlGuardError {
    fn from(_: VaultError) -> Self {
        Self::KioskForbidden
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedSqlPlan {
    pub wrapped_sql: String,
    pub tables_referenced: Vec<String>,
    pub effective_limit: u64,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlExecutionResult {
    pub ok: bool,
    #[serde(rename = "wrappedSql")]
    pub wrapped_sql: String,
    pub table: String,
    pub columns: Vec<String>,
    pub rows: Vec<SqlRowMap>,
    pub row_count: usize,
    #[serde(rename = "timeoutMs")]
    pub timeout_ms: u64,
    pub gate: Option<SqlGuardGate>,
    pub error: Option<String>,
}

pub fn default_arrow_view_whitelist() -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    let mut map = BTreeMap::new();
    for &(view, cols) in ALLOWED_ARROW_VIEWS {
        map.insert(view, cols.iter().copied().collect());
    }
    map
}

pub fn n2sf_grade_rank(grade: N2sfGrade) -> u8 {
    match grade {
        N2sfGrade::Open => 0,
        N2sfGrade::Sensitive => 1,
        N2sfGrade::Classified => 2,
    }
}

/// Validates and rewrites a SQL query using the canonical 5-layer AST guard with inner-scan
/// `vault_id` and `n2sf_grade_rank` predicate pushdown.
pub fn validate_and_wrap_sql_with_whitelist(
    raw_sql: &str,
    role: DeviceRole,
    active_vault_id: &str,
    max_n2sf_grade: u8,
    whitelist: &BTreeMap<&str, BTreeSet<&str>>,
) -> std::result::Result<ValidatedSqlPlan, SqlGuardError> {
    if matches!(role, DeviceRole::CounterKiosk) {
        return Err(SqlGuardError::KioskForbidden);
    }
    let clean_vault: String = active_vault_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if clean_vault.is_empty() {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G3VaultAndN2sfScope,
            reason: "empty vault_id".into(),
        });
    }
    let trimmed = raw_sql.trim();
    if trimmed.is_empty() {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G1SingleSelectOnly,
            reason: "empty SQL string".into(),
        });
    }

    let upper = trimmed.to_ascii_uppercase();
    for banned in FORBIDDEN_SQL_KEYWORDS {
        if upper.contains(banned) {
            return Err(SqlGuardError::Blocked {
                gate: SqlGuardGate::G1SingleSelectOnly,
                reason: format!("forbidden SQL keyword or file-access function: {banned}"),
            });
        }
    }

    let mut stmts = Parser::parse_sql(&GenericDialect {}, trimmed).map_err(|e| SqlGuardError::Blocked {
        gate: SqlGuardGate::G1SingleSelectOnly,
        reason: format!("SQL parse error: {e}"),
    })?;
    if stmts.len() != 1 {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G1SingleSelectOnly,
            reason: format!("expected exactly 1 SELECT statement, found {}", stmts.len()),
        });
    }
    let Statement::Query(query_box) = &mut stmts[0] else {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G1SingleSelectOnly,
            reason: "non-SELECT statement forbidden".into(),
        });
    };

    let (table_name, effective_limit) = validate_query_node(query_box, whitelist)?;

    let clamped_grade = max_n2sf_grade.min(2);
    // D-01: Push mandatory vault & N2SF grade predicates directly into the inner SELECT WHERE scan
    // (eliminating outer-wrapper literal alias spoofing and enabling GROUP BY aggregations cleanly).
    let scope_pred = Expr::Nested(Box::new(Expr::BinaryOp {
        left: Box::new(Expr::BinaryOp {
            left: Box::new(Expr::Identifier(Ident::new("vault_id"))),
            op: BinaryOperator::Eq,
            right: Box::new(Expr::Value(Value::SingleQuotedString(clean_vault).into())),
        }),
        op: BinaryOperator::And,
        right: Box::new(Expr::BinaryOp {
            left: Box::new(Expr::Identifier(Ident::new("n2sf_grade_rank"))),
            op: BinaryOperator::LtEq,
            right: Box::new(Expr::Value(Value::Number(clamped_grade.to_string(), false).into())),
        }),
    }));

    if let SetExpr::Select(sel) = query_box.body.as_mut() {
        sel.selection = Some(match sel.selection.take() {
            Some(existing) => Expr::BinaryOp {
                left: Box::new(scope_pred),
                op: BinaryOperator::And,
                right: Box::new(Expr::Nested(Box::new(existing))),
            },
            None => scope_pred,
        });
    }
    query_box.limit_clause = Some(LimitClause::LimitOffset {
        limit: Some(Expr::Value(
            Value::Number(effective_limit.to_string(), false).into(),
        )),
        offset: None,
        limit_by: Vec::new(),
    });

    Ok(ValidatedSqlPlan {
        wrapped_sql: query_box.to_string(),
        tables_referenced: vec![table_name],
        effective_limit,
        timeout_ms: SQL_TIMEOUT_MS,
    })
}

pub fn validate_and_wrap_sql(
    raw_sql: &str,
    role: DeviceRole,
    active_vault_id: &str,
    max_n2sf_grade: u8,
) -> std::result::Result<ValidatedSqlPlan, SqlGuardError> {
    let wl = default_arrow_view_whitelist();
    validate_and_wrap_sql_with_whitelist(raw_sql, role, active_vault_id, max_n2sf_grade, &wl)
}

pub fn verify_and_wrap_safe_sql(
    caller_role: DeviceRole,
    active_vault_id: &str,
    caller_max_grade: N2sfGrade,
    raw_sql: &str,
    whitelist: &BTreeMap<&str, BTreeSet<&str>>,
) -> Result<String> {
    let plan = validate_and_wrap_sql_with_whitelist(
        raw_sql,
        caller_role,
        active_vault_id,
        n2sf_grade_rank(caller_max_grade),
        whitelist,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(plan.wrapped_sql)
}

fn validate_query_node(
    q: &Query,
    whitelist: &BTreeMap<&str, BTreeSet<&str>>,
) -> std::result::Result<(String, u64), SqlGuardError> {
    if q.with.is_some() {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G4LimitAndJoinBudget,
            reason: "CTE/WITH expression forbidden outside single whitelisted view".into(),
        });
    }
    let mut effective_limit = HARD_ROW_LIMIT;
    if let Some(LimitClause::LimitOffset { limit: Some(expr), .. }) = &q.limit_clause {
        if let Expr::Value(ValueWithSpan {
            value: Value::Number(s, _),
            ..
        }) = expr
        {
            if let Ok(n) = s.parse::<u64>() {
                effective_limit = n.min(HARD_ROW_LIMIT).max(1);
            }
        }
    }
    let SetExpr::Select(sel) = q.body.as_ref() else {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G1SingleSelectOnly,
            reason: "UNION/SetOperation or VALUES forbidden".into(),
        });
    };
    if sel.into.is_some() || sel.from.len() != 1 {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G4LimitAndJoinBudget,
            reason: "SELECT INTO or multi-FROM forbidden".into(),
        });
    }
    for join in &sel.from[0].joins {
        if matches!(
            join.join_operator,
            JoinOperator::CrossJoin(_) | JoinOperator::CrossApply
        ) {
            return Err(SqlGuardError::Blocked {
                gate: SqlGuardGate::G4LimitAndJoinBudget,
                reason: "Cartesian CROSS JOIN forbidden".into(),
            });
        }
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G4LimitAndJoinBudget,
            reason: "multi-table JOIN beyond single whitelisted view forbidden".into(),
        });
    }
    let TableFactor::Table { name, args, .. } = &sel.from[0].relation else {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G2TableColumnWhitelist,
            reason: "table function or lateral subquery forbidden".into(),
        });
    };
    if args.is_some() {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G1SingleSelectOnly,
            reason: format!("table-valued function forbidden: {name}"),
        });
    }
    let raw_tname = name.to_string().to_ascii_lowercase();
    let tname = raw_tname.rsplit('.').next().unwrap_or(&raw_tname).to_string();
    let Some(allowed_cols) = whitelist.get(tname.as_str()) else {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G2TableColumnWhitelist,
            reason: format!("unregistered view: {tname}"),
        });
    };
    let mut projection_aliases: BTreeSet<String> = BTreeSet::new();
    for item in &sel.projection {
        match item {
            SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _) => {
                return Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G2TableColumnWhitelist,
                    reason: "SELECT * wildcard forbidden; enumerate whitelisted columns explicitly".into(),
                });
            }
            SelectItem::UnnamedExpr(e) => {
                validate_expr_columns(e, allowed_cols, &projection_aliases)?;
            }
            SelectItem::ExprWithAlias { expr: e, alias } => {
                let alias_low = alias.value.to_ascii_lowercase();
                if RESERVED_SECURITY_COLS.contains(&alias_low.as_str()) {
                    return Err(SqlGuardError::Blocked {
                        gate: SqlGuardGate::G3VaultAndN2sfScope,
                        reason: format!("aliasing reserved security column '{alias_low}' forbidden"),
                    });
                }
                validate_expr_columns(e, allowed_cols, &projection_aliases)?;
                projection_aliases.insert(alias_low);
            }
        }
    }
    if let Some(cond) = &sel.selection {
        validate_expr_columns(cond, allowed_cols, &BTreeSet::new())?;
    }
    match &sel.group_by {
        GroupByExpr::All(_) => {
            return Err(SqlGuardError::Blocked {
                gate: SqlGuardGate::G2TableColumnWhitelist,
                reason: "GROUP BY ALL forbidden".into(),
            });
        }
        GroupByExpr::Expressions(exprs, _) => {
            for e in exprs {
                validate_expr_columns(e, allowed_cols, &projection_aliases)?;
            }
        }
    }
    if let Some(having) = &sel.having {
        validate_expr_columns(having, allowed_cols, &projection_aliases)?;
    }
    if let Some(order_by) = &q.order_by {
        match &order_by.kind {
            OrderByKind::All(_) => {
                return Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G2TableColumnWhitelist,
                    reason: "ORDER BY ALL forbidden".into(),
                });
            }
            OrderByKind::Expressions(exprs) => {
                for oe in exprs {
                    validate_expr_columns(&oe.expr, allowed_cols, &projection_aliases)?;
                }
            }
        }
    }
    Ok((tname, effective_limit))
}

fn validate_expr_columns(
    expr: &Expr,
    allowed_cols: &BTreeSet<&str>,
    allowed_aliases: &BTreeSet<String>,
) -> std::result::Result<(), SqlGuardError> {
    match expr {
        Expr::Identifier(id) => {
            let col = id.value.to_ascii_lowercase();
            if col == "n2sf_grade_rank"
                || allowed_cols.contains(col.as_str())
                || allowed_aliases.contains(&col)
            {
                Ok(())
            } else {
                Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G2TableColumnWhitelist,
                    reason: format!("non-whitelisted column: {col}"),
                })
            }
        }
        Expr::CompoundIdentifier(parts) => {
            let col = parts
                .last()
                .map(|p| p.value.to_ascii_lowercase())
                .unwrap_or_default();
            if col == "n2sf_grade_rank" || allowed_cols.contains(col.as_str()) {
                Ok(())
            } else {
                Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G2TableColumnWhitelist,
                    reason: format!("non-whitelisted column: {col}"),
                })
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            validate_expr_columns(left, allowed_cols, allowed_aliases)?;
            validate_expr_columns(right, allowed_cols, allowed_aliases)
        }
        Expr::UnaryOp { expr: inner, .. }
        | Expr::Nested(inner)
        | Expr::Cast { expr: inner, .. } => validate_expr_columns(inner, allowed_cols, allowed_aliases),
        Expr::Function(f) => {
            let fname = f.name.to_string().to_ascii_uppercase();
            if FORBIDDEN_FILE_FUNCTIONS.iter().any(|b| b.eq_ignore_ascii_case(&fname)) {
                return Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G1SingleSelectOnly,
                    reason: format!("forbidden file-access function: {fname}"),
                });
            }
            if !matches!(
                fname.as_str(),
                "COUNT" | "SUM" | "AVG" | "MIN" | "MAX" | "COALESCE" | "ROUND"
            ) {
                return Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G2TableColumnWhitelist,
                    reason: format!("non-whitelisted SQL function: {fname}"),
                });
            }
            if let FunctionArguments::List(arg_list) = &f.args {
                for a in &arg_list.args {
                    match a {
                        FunctionArg::Unnamed(FunctionArgExpr::Expr(inner)) => {
                            validate_expr_columns(inner, allowed_cols, allowed_aliases)?;
                        }
                        FunctionArg::Unnamed(FunctionArgExpr::Wildcard) if fname == "COUNT" => {}
                        _ => {
                            return Err(SqlGuardError::Blocked {
                                gate: SqlGuardGate::G2TableColumnWhitelist,
                                reason: format!("unsupported function argument in {fname}"),
                            });
                        }
                    }
                }
            }
            Ok(())
        }
        Expr::Value(_) => Ok(()),
        Expr::Between {
            expr: inner,
            low,
            high,
            ..
        } => {
            validate_expr_columns(inner, allowed_cols, allowed_aliases)?;
            validate_expr_columns(low, allowed_cols, allowed_aliases)?;
            validate_expr_columns(high, allowed_cols, allowed_aliases)
        }
        Expr::InList {
            expr: inner, list, ..
        } => {
            validate_expr_columns(inner, allowed_cols, allowed_aliases)?;
            for item in list {
                validate_expr_columns(item, allowed_cols, allowed_aliases)?;
            }
            Ok(())
        }
        Expr::Case {
            operand,
            conditions,
            else_result,
            ..
        } => {
            if let Some(op) = operand {
                validate_expr_columns(op, allowed_cols, allowed_aliases)?;
            }
            for cw in conditions {
                validate_expr_columns(&cw.condition, allowed_cols, allowed_aliases)?;
                validate_expr_columns(&cw.result, allowed_cols, allowed_aliases)?;
            }
            if let Some(el) = else_result {
                validate_expr_columns(el, allowed_cols, allowed_aliases)?;
            }
            Ok(())
        }
        _ => Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G2TableColumnWhitelist,
            reason: "unsupported AST expression".into(),
        }),
    }
}

/// Gate G-SQL5: blocks output cell values containing any of the 16 `CITIZEN_FORBIDDEN_TERMS`
/// or raw unredacted RRN / MyNumber / Aadhaar government IDs.
pub fn filter_output_rows(
    rows: &[Vec<String>],
    extra_forbidden: &[String],
) -> std::result::Result<(), SqlGuardError> {
    let guard = WriterGuard::citizen_guard(extra_forbidden.to_vec(), Vec::new());
    for row in rows {
        for cell in row {
            if let Err(e) = guard.verify_text(cell) {
                return Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G5WriterOutputFilter,
                    reason: e.to_string(),
                });
            }
            if contains_forbidden_raw_gov_id_digits(cell) {
                return Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G5WriterOutputFilter,
                    reason: "raw government ID digits detected in SQL output".into(),
                });
            }
        }
    }
    Ok(())
}

pub fn sanitize_sql_output_cells(rows: &[Vec<String>], guard: &WriterGuard) -> Result<()> {
    for row in rows {
        for cell in row {
            guard.verify_text(cell)?;
            ensure!(
                !contains_forbidden_raw_gov_id_digits(cell),
                "G-SQL5 blocked raw gov ID digits in SQL output"
            );
        }
    }
    Ok(())
}

/// Seeds deterministic in-memory Arrow view rows across the 5 whitelisted views, including
/// multi-vault and multi-N2SF-grade rows so scan-stage G-SQL3 isolation is strictly exercised.
pub fn default_demo_arrow_views(active_vault_id: &str) -> BTreeMap<String, Vec<SqlRowMap>> {
    let mk = |pairs: &[(&str, &str)]| -> SqlRowMap {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    };
    let mut views: BTreeMap<String, Vec<SqlRowMap>> = BTreeMap::new();
    views.insert(
        "docs_entities".into(),
        vec![
            mk(&[
                ("doc_id", "DOC-2026-001"),
                ("vault_id", active_vault_id),
                ("n2sf_grade_rank", "0"),
                ("n2sf_grade", "open"),
                ("pack_id", "DOM-01"),
                ("vendor_id", "VEND-KR-01"),
                ("entity_key", "contract_sum"),
                ("amt_krw", "45000000"),
                ("currency", "KRW"),
                ("status", "ok"),
                ("doc_date", "2026-10-15"),
            ]),
            mk(&[
                ("doc_id", "DOC-2026-002"),
                ("vault_id", active_vault_id),
                ("n2sf_grade_rank", "1"),
                ("n2sf_grade", "sensitive"),
                ("pack_id", "DOM-08"),
                ("vendor_id", "VEND-KR-01"),
                ("entity_key", "ci_amount"),
                ("amt_krw", "32500000"),
                ("currency", "KRW"),
                ("status", "ok"),
                ("doc_date", "2026-10-22"),
            ]),
            mk(&[
                ("doc_id", "DOC-2026-003"),
                ("vault_id", active_vault_id),
                ("n2sf_grade_rank", "2"),
                ("n2sf_grade", "classified"),
                ("pack_id", "DOM-14"),
                ("vendor_id", "VEND-SEC-99"),
                ("entity_key", "motie_approval_no"),
                ("amt_krw", "99000000"),
                ("currency", "KRW"),
                ("status", "restricted"),
                ("doc_date", "2026-11-01"),
            ]),
            mk(&[
                ("doc_id", "DOC-OTHER-VAULT"),
                ("vault_id", "vault-isolated-other"),
                ("n2sf_grade_rank", "0"),
                ("n2sf_grade", "open"),
                ("pack_id", "DOM-04"),
                ("vendor_id", "VEND-OTHER"),
                ("entity_key", "po_no"),
                ("amt_krw", "777000000"),
                ("currency", "KRW"),
                ("status", "ok"),
                ("doc_date", "2026-11-02"),
            ]),
        ],
    );
    views.insert(
        "gate_ledger".into(),
        vec![mk(&[
            ("entry_id", "GL-001"),
            ("vault_id", active_vault_id),
            ("n2sf_grade_rank", "1"),
            ("n2sf_grade", "sensitive"),
            ("doc_id", "DOC-2026-001"),
            ("gate_code", "G2"),
            ("verdict", "EligibleNormal"),
            ("overridden", "false"),
            ("verified_krw", "45000000"),
            ("evidence_krw", "45000000"),
            ("risk_score", "0.12"),
            ("created_at", "2026-10-15T09:00:00Z"),
        ])],
    );
    views.insert(
        "media_events".into(),
        vec![mk(&[
            ("media_id", "CAM-01"),
            ("vault_id", active_vault_id),
            ("n2sf_grade_rank", "0"),
            ("n2sf_grade", "open"),
            ("ts_ms", "12450"),
            ("bbox", "0.1,0.2,0.5,0.4"),
            ("label", "forklift_bay"),
            ("scheme", "day"),
        ])],
    );
    views.insert(
        "sim_samples".into(),
        vec![mk(&[
            ("run_id", "SIM-01"),
            ("vault_id", active_vault_id),
            ("n2sf_grade_rank", "0"),
            ("n2sf_grade", "open"),
            ("tick", "100"),
            ("unit_id", "unit_0"),
            ("err_m", "0.021"),
            ("provenance", "diag"),
        ])],
    );
    views.insert(
        "stage_cues".into(),
        vec![mk(&[
            ("cue_id", "Q-01"),
            ("vault_id", active_vault_id),
            ("n2sf_grade_rank", "0"),
            ("n2sf_grade", "open"),
            ("smpte", "01:00:00:00"),
            ("delta_m", "0.015"),
            ("interlock_ok", "true"),
            ("ts_ms", "60000"),
        ])],
    );
    views
}

/// Validates and executes a guarded local read-only SQL query over the in-memory Arrow views,
/// enforcing scan-stage `vault_id` + `n2sf_grade_rank` isolation, projection/aggregation,
/// `LIMIT <= 200`, and G-SQL5 output cell verification.
pub fn execute_guarded_sql(
    raw_sql: &str,
    role: DeviceRole,
    active_vault_id: &str,
    max_n2sf_grade: u8,
    views: &BTreeMap<String, Vec<SqlRowMap>>,
) -> std::result::Result<SqlExecutionResult, SqlGuardError> {
    let plan = validate_and_wrap_sql(raw_sql, role, active_vault_id, max_n2sf_grade)?;
    let table = plan
        .tables_referenced
        .first()
        .cloned()
        .unwrap_or_default();

    let stmts = Parser::parse_sql(&GenericDialect {}, &plan.wrapped_sql).map_err(|e| {
        SqlGuardError::Blocked {
            gate: SqlGuardGate::G1SingleSelectOnly,
            reason: format!("wrapped SQL re-parse error: {e}"),
        }
    })?;
    let Statement::Query(query_box) = &stmts[0] else {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G1SingleSelectOnly,
            reason: "wrapped statement is not SELECT".into(),
        });
    };
    let SetExpr::Select(sel) = query_box.body.as_ref() else {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G1SingleSelectOnly,
            reason: "wrapped query body is not SELECT".into(),
        });
    };

    let clean_vault: String = active_vault_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let clamped_grade = max_n2sf_grade.min(2);

    // Hard scan-stage G-SQL3 filter BEFORE evaluating any user WHERE expression.
    let source_rows = views.get(&table).map(Vec::as_slice).unwrap_or(&[]);
    let filtered: Vec<&SqlRowMap> = source_rows
        .iter()
        .filter(|row| {
            let v_ok = row.get("vault_id").map(String::as_str) == Some(clean_vault.as_str());
            let rank = row
                .get("n2sf_grade_rank")
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(u8::MAX);
            v_ok && rank <= clamped_grade
        })
        .filter(|row| {
            sel.selection
                .as_ref()
                .map(|expr| eval_predicate(expr, row))
                .unwrap_or(true)
        })
        .collect();

    let has_group_by = match &sel.group_by {
        GroupByExpr::Expressions(exprs, _) => !exprs.is_empty(),
        GroupByExpr::All(_) => false,
    };
    let has_agg = sel.projection.iter().any(select_item_has_aggregate);

    let mut columns = Vec::new();
    let mut out_rows: Vec<SqlRowMap> = Vec::new();

    if has_group_by || has_agg {
        let group_cols: Vec<String> = match &sel.group_by {
            GroupByExpr::Expressions(exprs, _) => exprs.iter().filter_map(expr_col_name).collect(),
            GroupByExpr::All(_) => Vec::new(),
        };
        let mut groups: BTreeMap<Vec<String>, Vec<&SqlRowMap>> = BTreeMap::new();
        if group_cols.is_empty() {
            groups.insert(Vec::new(), filtered);
        } else {
            for r in filtered {
                let key = group_cols
                    .iter()
                    .map(|c| r.get(c).cloned().unwrap_or_default())
                    .collect();
                groups.entry(key).or_default().push(r);
            }
        }
        for (_k, grp) in groups {
            let mut row_map = SqlRowMap::new();
            for (idx, item) in sel.projection.iter().enumerate() {
                let (col_name, val) = eval_select_item_agg(item, &grp, idx);
                if out_rows.is_empty() {
                    columns.push(col_name.clone());
                }
                row_map.insert(col_name, val);
            }
            out_rows.push(row_map);
        }
    } else {
        for r in filtered {
            let mut row_map = SqlRowMap::new();
            for (idx, item) in sel.projection.iter().enumerate() {
                let (col_name, val) = eval_select_item_row(item, r, idx);
                if out_rows.is_empty() {
                    columns.push(col_name.clone());
                }
                row_map.insert(col_name, val);
            }
            out_rows.push(row_map);
        }
    }

    if let Some(order_by) = &query_box.order_by {
        if let OrderByKind::Expressions(exprs) = &order_by.kind {
            if let Some(first) = exprs.first() {
                if let Some(col) = expr_col_name(&first.expr) {
                    let desc = first.options.asc == Some(false);
                    out_rows.sort_by(|a, b| {
                        let va = a.get(&col).map(String::as_str).unwrap_or("");
                        let vb = b.get(&col).map(String::as_str).unwrap_or("");
                        let ord = match (va.parse::<f64>(), vb.parse::<f64>()) {
                            (Ok(na), Ok(nb)) => na.total_cmp(&nb),
                            _ => va.cmp(vb),
                        };
                        if desc { ord.reverse() } else { ord }
                    });
                }
            }
        }
    }

    out_rows.truncate(plan.effective_limit as usize);

    // G-SQL5 output cell verification before returning any row to caller.
    let cell_matrix: Vec<Vec<String>> = out_rows
        .iter()
        .map(|r| r.values().cloned().collect())
        .collect();
    filter_output_rows(&cell_matrix, &[])?;

    let row_count = out_rows.len();
    Ok(SqlExecutionResult {
        ok: true,
        wrapped_sql: plan.wrapped_sql,
        table,
        columns,
        rows: out_rows,
        row_count,
        timeout_ms: plan.timeout_ms,
        gate: None,
        error: None,
    })
}

fn select_item_has_aggregate(item: &SelectItem) -> bool {
    let expr = match item {
        SelectItem::UnnamedExpr(e) | SelectItem::ExprWithAlias { expr: e, .. } => e,
        _ => return false,
    };
    expr_has_aggregate(expr)
}

fn expr_has_aggregate(expr: &Expr) -> bool {
    match expr {
        Expr::Function(f) => {
            let n = f.name.to_string().to_ascii_uppercase();
            matches!(n.as_str(), "COUNT" | "SUM" | "AVG" | "MIN" | "MAX")
        }
        Expr::BinaryOp { left, right, .. } => expr_has_aggregate(left) || expr_has_aggregate(right),
        Expr::Nested(e) | Expr::UnaryOp { expr: e, .. } | Expr::Cast { expr: e, .. } => {
            expr_has_aggregate(e)
        }
        _ => false,
    }
}

fn expr_col_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Identifier(id) => Some(id.value.to_ascii_lowercase()),
        Expr::CompoundIdentifier(parts) => parts.last().map(|p| p.value.to_ascii_lowercase()),
        Expr::Nested(inner) => expr_col_name(inner),
        _ => None,
    }
}

fn eval_scalar(expr: &Expr, row: &SqlRowMap) -> String {
    match expr {
        Expr::Identifier(id) => row
            .get(&id.value.to_ascii_lowercase())
            .cloned()
            .unwrap_or_default(),
        Expr::CompoundIdentifier(parts) => parts
            .last()
            .and_then(|p| row.get(&p.value.to_ascii_lowercase()))
            .cloned()
            .unwrap_or_default(),
        Expr::Value(vw) => match &vw.value {
            Value::SingleQuotedString(s) | Value::DoubleQuotedString(s) => s.clone(),
            Value::Number(n, _) => n.clone(),
            Value::Boolean(b) => b.to_string(),
            Value::Null => String::new(),
            other => other.to_string(),
        },
        Expr::Nested(inner) | Expr::Cast { expr: inner, .. } => eval_scalar(inner, row),
        Expr::Function(f) => {
            let fname = f.name.to_string().to_ascii_uppercase();
            if fname == "COALESCE" {
                if let FunctionArguments::List(args) = &f.args {
                    for a in &args.args {
                        if let FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) = a {
                            let v = eval_scalar(e, row);
                            if !v.is_empty() {
                                return v;
                            }
                        }
                    }
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

fn eval_predicate(expr: &Expr, row: &SqlRowMap) -> bool {
    match expr {
        Expr::Nested(inner) => eval_predicate(inner, row),
        Expr::BinaryOp { left, op, right } => match op {
            BinaryOperator::And => eval_predicate(left, row) && eval_predicate(right, row),
            BinaryOperator::Or => eval_predicate(left, row) || eval_predicate(right, row),
            BinaryOperator::Eq
            | BinaryOperator::NotEq
            | BinaryOperator::Lt
            | BinaryOperator::LtEq
            | BinaryOperator::Gt
            | BinaryOperator::GtEq => {
                let lv = eval_scalar(left, row);
                let rv = eval_scalar(right, row);
                let ord = match (lv.parse::<f64>(), rv.parse::<f64>()) {
                    (Ok(a), Ok(b)) => a.total_cmp(&b),
                    _ => lv.cmp(&rv),
                };
                match op {
                    BinaryOperator::Eq => ord.is_eq(),
                    BinaryOperator::NotEq => !ord.is_eq(),
                    BinaryOperator::Lt => ord.is_lt(),
                    BinaryOperator::LtEq => ord.is_le(),
                    BinaryOperator::Gt => ord.is_gt(),
                    BinaryOperator::GtEq => ord.is_ge(),
                    _ => false,
                }
            }
            _ => true,
        },
        Expr::InList {
            expr: inner,
            list,
            negated,
        } => {
            let v = eval_scalar(inner, row);
            let found = list.iter().any(|item| eval_scalar(item, row) == v);
            if *negated { !found } else { found }
        }
        Expr::Between {
            expr: inner,
            low,
            high,
            negated,
        } => {
            let v = eval_scalar(inner, row);
            let lo = eval_scalar(low, row);
            let hi = eval_scalar(high, row);
            let inside = match (v.parse::<f64>(), lo.parse::<f64>(), hi.parse::<f64>()) {
                (Ok(vn), Ok(ln), Ok(hn)) => vn >= ln && vn <= hn,
                _ => v >= lo && v <= hi,
            };
            if *negated { !inside } else { inside }
        }
        _ => true,
    }
}

fn eval_select_item_row(item: &SelectItem, row: &SqlRowMap, idx: usize) -> (String, String) {
    match item {
        SelectItem::UnnamedExpr(e) => {
            let col = expr_col_name(e).unwrap_or_else(|| format!("col_{idx}"));
            (col, eval_scalar(e, row))
        }
        SelectItem::ExprWithAlias { expr, alias } => {
            (alias.value.to_ascii_lowercase(), eval_scalar(expr, row))
        }
        _ => (format!("col_{idx}"), String::new()),
    }
}

fn eval_select_item_agg(item: &SelectItem, rows: &[&SqlRowMap], idx: usize) -> (String, String) {
    let (expr, explicit_alias) = match item {
        SelectItem::UnnamedExpr(e) => (e, None),
        SelectItem::ExprWithAlias { expr, alias } => (expr, Some(alias.value.to_ascii_lowercase())),
        _ => return (format!("col_{idx}"), String::new()),
    };
    let col_name = explicit_alias
        .or_else(|| expr_col_name(expr))
        .unwrap_or_else(|| format!("agg_{idx}"));

    if let Expr::Function(f) = expr {
        let fname = f.name.to_string().to_ascii_uppercase();
        let arg_expr = match &f.args {
            FunctionArguments::List(l) => l.args.first().and_then(|a| match a {
                FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => Some(e),
                _ => None,
            }),
            _ => None,
        };
        let val = match fname.as_str() {
            "COUNT" => rows.len().to_string(),
            "SUM" | "AVG" | "MIN" | "MAX" => {
                let nums: Vec<f64> = rows
                    .iter()
                    .filter_map(|r| arg_expr.and_then(|e| eval_scalar(e, r).parse::<f64>().ok()))
                    .collect();
                if nums.is_empty() {
                    "0".to_string()
                } else {
                    let res = match fname.as_str() {
                        "SUM" => nums.iter().sum::<f64>(),
                        "AVG" => nums.iter().sum::<f64>() / nums.len() as f64,
                        "MIN" => nums.iter().copied().fold(f64::INFINITY, f64::min),
                        "MAX" => nums.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                        _ => 0.0,
                    };
                    if (res.fract()).abs() < 1e-9 {
                        format!("{}", res as i64)
                    } else {
                        format!("{res:.4}")
                    }
                }
            }
            _ => rows
                .first()
                .map(|r| eval_scalar(expr, r))
                .unwrap_or_default(),
        };
        return (col_name, val);
    }

    let val = rows
        .first()
        .map(|r| eval_scalar(expr, r))
        .unwrap_or_default();
    (col_name, val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_layer_sql_guard_blocks_hostile_queries_and_wraps_valid_select() {
        let wl = default_arrow_view_whitelist();
        let attacks = [
            "DROP TABLE gate_ledger",
            "DELETE FROM docs_entities WHERE 1=1",
            "INSERT INTO gate_ledger (entry_id) VALUES ('x')",
            "UPDATE docs_entities SET amt_krw = 0",
            "ALTER TABLE docs_entities ADD COLUMN pwn INT",
            "TRUNCATE TABLE gate_ledger",
            "COPY docs_entities TO '/tmp/leak.csv'",
            "ATTACH '/tmp/pwn.db' AS pwn",
            "SELECT doc_id FROM docs_entities; DROP TABLE gate_ledger",
            "SELECT * FROM secret_users",
            "SELECT read_file('/etc/passwd') FROM docs_entities",
            "SELECT parquet_scan('/etc/passwd') FROM docs_entities",
            "SELECT hallucinated_column FROM docs_entities",
            "SELECT amt_krw FROM docs_entities WHERE secret_col = 1",
            "SELECT * FROM docs_entities CROSS JOIN gate_ledger",
            "SELECT * FROM docs_entities INTO leaked_tbl",
            "SELECT * FROM csv_scan('/tmp/x.csv')",
            "PRAGMA table_info('docs_entities')",
            "VALUES (1, 2)",
            "",
        ];
        assert_eq!(attacks.len(), 20);
        for bad in attacks {
            assert!(
                verify_and_wrap_safe_sql(
                    DeviceRole::PersonalPhone,
                    "vault-01",
                    N2sfGrade::Sensitive,
                    bad,
                    &wl
                )
                .is_err(),
                "escaped SQL guard: {bad}"
            );
            assert!(
                validate_and_wrap_sql(bad, DeviceRole::PersonalPhone, "vault-01", 1).is_err(),
                "escaped validate_and_wrap_sql: {bad}"
            );
        }

        assert_eq!(
            validate_and_wrap_sql(
                "SELECT doc_id FROM docs_entities",
                DeviceRole::CounterKiosk,
                "vault-open",
                0
            ),
            Err(SqlGuardError::KioskForbidden)
        );

        let wrapped = verify_and_wrap_safe_sql(
            DeviceRole::PersonalPhone,
            "vault-fin-04",
            N2sfGrade::Sensitive,
            "SELECT doc_id, SUM(amt_krw) AS tot FROM docs_entities WHERE status = 'ok' LIMIT 900",
            &wl,
        )
        .unwrap();
        assert!(wrapped.contains("vault_id = 'vault-fin-04'"));
        assert!(wrapped.contains("n2sf_grade_rank <= 1"));
        assert!(wrapped.ends_with("LIMIT 200"));

        // D-01: Reserved alias spoofing, SELECT *, IN (SELECT ...), JOIN ON 1=1, and bad GROUP BY/ORDER BY must be blocked.
        for spoof in [
            "SELECT 'vault-fin-04' AS vault_id, 0 AS n2sf_grade_rank, doc_id FROM docs_entities WHERE vault_id = 'vault-other'",
            "SELECT * FROM docs_entities",
            "SELECT doc_id FROM docs_entities JOIN gate_ledger ON 1=1",
            "SELECT doc_id FROM docs_entities WHERE doc_id IN (SELECT doc_id FROM gate_ledger)",
            "SELECT doc_id FROM docs_entities GROUP BY secret_col",
            "SELECT doc_id FROM docs_entities ORDER BY secret_col",
            "SELECT CAST(secret_col AS INT) FROM docs_entities",
        ] {
            assert!(
                verify_and_wrap_safe_sql(
                    DeviceRole::PersonalPhone,
                    "vault-fin-04",
                    N2sfGrade::Sensitive,
                    spoof,
                    &wl
                )
                .is_err(),
                "D-01 bypass escaped SQL guard: {spoof}"
            );
        }

        // D-01: GROUP BY aggregation works cleanly with inner-scan WHERE pushdown.
        let agg = verify_and_wrap_safe_sql(
            DeviceRole::PersonalPhone,
            "vault-fin-04",
            N2sfGrade::Sensitive,
            "SELECT vendor_id, SUM(amt_krw) AS tot FROM docs_entities GROUP BY vendor_id ORDER BY tot DESC",
            &wl,
        )
        .unwrap();
        assert!(agg.contains("WHERE (vault_id = 'vault-fin-04' AND n2sf_grade_rank <= 1)"));
        assert!(agg.contains("GROUP BY vendor_id"));
        assert!(agg.ends_with("LIMIT 200"));

        // G-SQL5 blocks forbidden citizen terms & raw Gov IDs.
        let g = WriterGuard::citizen_guard(Vec::new(), Vec::new());
        assert!(sanitize_sql_output_cells(&[vec!["[RRN_REDACTED]".into(), "150000".into()]], &g).is_ok());
        assert!(sanitize_sql_output_cells(&[vec!["위장계약".into()]], &g).is_err());
        assert!(sanitize_sql_output_cells(&[vec!["900101-1234567".into()]], &g).is_err());
        assert!(filter_output_rows(&[vec!["G3Violation 발생".into()]], &[]).is_err());
    }

    #[test]
    fn execute_guarded_sql_enforces_vault_n2sf_and_aggregates() {
        let views = default_demo_arrow_views("vault-default");
        // Max grade 1 (Sensitive) sees DOC-2026-001 (rank 0) and DOC-2026-002 (rank 1),
        // excluding DOC-2026-003 (rank 2 Classified) and DOC-OTHER-VAULT (other vault).
        let res = execute_guarded_sql(
            "SELECT vendor_id, COUNT(doc_id) AS cnt, SUM(amt_krw) AS total_krw FROM docs_entities GROUP BY vendor_id",
            DeviceRole::PersonalPhone,
            "vault-default",
            1,
            &views,
        )
        .unwrap();
        assert!(res.ok);
        assert_eq!(res.row_count, 1);
        assert_eq!(res.rows[0].get("vendor_id").map(String::as_str), Some("VEND-KR-01"));
        assert_eq!(res.rows[0].get("cnt").map(String::as_str), Some("2"));
        assert_eq!(res.rows[0].get("total_krw").map(String::as_str), Some("77500000"));

        // G-SQL5 blocks contaminated view cell containing raw RRN digits.
        let mut poisoned = views.clone();
        poisoned.get_mut("docs_entities").unwrap()[0].insert("status".into(), "900101-1234567".into());
        let err = execute_guarded_sql(
            "SELECT doc_id, status FROM docs_entities",
            DeviceRole::PersonalPhone,
            "vault-default",
            1,
            &poisoned,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            SqlGuardError::Blocked {
                gate: SqlGuardGate::G5WriterOutputFilter,
                ..
            }
        ));
    }
}
