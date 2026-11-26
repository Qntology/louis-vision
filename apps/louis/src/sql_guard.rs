//! 5-Layer Local DataFusion SQL AST Safety Guard (`sql_guard.rs`):
//! - G-SQL1: Single `SELECT` statement only (blocks DDL/DML/COPY/ATTACH/PRAGMA/INTO OUTFILE/file-scan functions).
//! - G-SQL2: 5 Arrow view + physical column whitelist.
//! - G-SQL3: Mandatory `vault_id` & `n2sf_grade_rank` isolation (`CounterKiosk` -> `KioskForbidden`).
//! - G-SQL4: Single view / no CROSS JOIN / `LIMIT <= 200` / `1500 ms` timeout budget.
//! - G-SQL5: Serialized output cell inspection through Layer 0 `WriterGuard` (`CITIZEN_FORBIDDEN_TERMS` 16 terms + PII).

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, ensure, Result};
use lv_doc::writer::guard::{N2sfGrade, WriterGuard};
use lv_router::vault::{DeviceRole, VaultError};
use sqlparser::ast::{
    Expr, FunctionArg, FunctionArgExpr, FunctionArguments, JoinOperator, LimitClause, Query,
    SelectItem, SetExpr, Statement, TableFactor, Value, ValueWithSpan,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

use crate::eval_nn::domains::contains_forbidden_raw_gov_id_digits;

pub const HARD_ROW_LIMIT: u64 = 200;
pub const SQL_TIMEOUT_MS: u64 = 1_500;

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

pub fn default_arrow_view_whitelist() -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    let mut map = BTreeMap::new();
    for &(view, cols) in ALLOWED_ARROW_VIEWS {
        map.insert(view, cols.iter().copied().collect());
    }
    map
}

pub fn verify_and_wrap_safe_sql(
    caller_role: DeviceRole,
    active_vault_id: &str,
    caller_max_grade: N2sfGrade,
    raw_sql: &str,
    whitelist: &BTreeMap<&str, BTreeSet<&str>>,
) -> Result<String> {
    if matches!(caller_role, DeviceRole::CounterKiosk) {
        bail!("{}", VaultError::KioskForbidden);
    }
    let clean_vault: String = active_vault_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    ensure!(!clean_vault.is_empty(), "G-SQL3 blocked empty vault_id");

    let upper = raw_sql.to_ascii_uppercase();
    for banned in [
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
        "INFORMATION_SCHEMA",
        "PG_CATALOG",
    ] {
        ensure!(!upper.contains(banned), "G-SQL1 blocked forbidden keyword: {banned}");
    }

    let stmts = Parser::parse_sql(&GenericDialect {}, raw_sql)?;
    ensure!(stmts.len() == 1, "G-SQL1 blocked multi-statement or empty SQL");
    let Statement::Query(query_box) = &stmts[0] else {
        bail!("G-SQL1 blocked non-SELECT statement");
    };
    let effective_limit = validate_query_node(query_box, whitelist)?;

    let grade_rank = match caller_max_grade {
        N2sfGrade::Open => 0,
        N2sfGrade::Sensitive => 1,
        N2sfGrade::Classified => 2,
    };
    let trimmed = raw_sql.trim().trim_end_matches(';');
    Ok(format!(
        "SELECT * FROM ({trimmed}) AS _lv_sub \
         WHERE _lv_sub.vault_id = '{clean_vault}' AND _lv_sub.n2sf_grade_rank <= {grade_rank} \
         LIMIT {effective_limit}"
    ))
}

fn validate_query_node(
    q: &Query,
    whitelist: &BTreeMap<&str, BTreeSet<&str>>,
) -> Result<u64> {
    if q.with.is_some() {
        bail!("G-SQL4 blocked CTE/WITH expression outside whitelisted single view");
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
        bail!("G-SQL1/2 blocked UNION/SetOperation or VALUES");
    };
    ensure!(
        sel.into.is_none() && sel.from.len() == 1,
        "G-SQL4 blocked SELECT INTO or multi-FROM"
    );
    for join in &sel.from[0].joins {
        if matches!(
            join.join_operator,
            JoinOperator::CrossJoin(_) | JoinOperator::CrossApply
        ) {
            bail!("G-SQL4 blocked Cartesian CROSS JOIN");
        }
        bail!("G-SQL4 blocked multi-table JOIN beyond single whitelisted view");
    }
    let TableFactor::Table { name, args, .. } = &sel.from[0].relation else {
        bail!("G-SQL2 blocked table function or lateral subquery");
    };
    ensure!(args.is_none(), "G-SQL1 blocked table-valued function");
    let tname = name.to_string().to_ascii_lowercase();
    let Some(allowed_cols) = whitelist.get(tname.as_str()) else {
        bail!("G-SQL2 blocked unregistered view: {tname}");
    };
    for item in &sel.projection {
        if let SelectItem::UnnamedExpr(e) | SelectItem::ExprWithAlias { expr: e, .. } = item {
            validate_expr_columns(e, allowed_cols)?;
        }
    }
    if let Some(cond) = &sel.selection {
        validate_expr_columns(cond, allowed_cols)?;
    }
    if let Some(having) = &sel.having {
        validate_expr_columns(having, allowed_cols)?;
    }
    Ok(effective_limit)
}

fn validate_expr_columns(expr: &Expr, allowed_cols: &BTreeSet<&str>) -> Result<()> {
    match expr {
        Expr::Identifier(id) => {
            let col = id.value.to_ascii_lowercase();
            ensure!(
                col == "n2sf_grade_rank" || allowed_cols.contains(col.as_str()),
                "G-SQL2 blocked column: {col}"
            );
            Ok(())
        }
        Expr::CompoundIdentifier(parts) => {
            let col = parts
                .last()
                .map(|p| p.value.to_ascii_lowercase())
                .unwrap_or_default();
            ensure!(
                col == "n2sf_grade_rank" || allowed_cols.contains(col.as_str()),
                "G-SQL2 blocked column: {col}"
            );
            Ok(())
        }
        Expr::BinaryOp { left, right, .. } => {
            validate_expr_columns(left, allowed_cols)?;
            validate_expr_columns(right, allowed_cols)
        }
        Expr::UnaryOp { expr: inner, .. }
        | Expr::Nested(inner)
        | Expr::Cast { expr: inner, .. } => validate_expr_columns(inner, allowed_cols),
        Expr::Function(f) => {
            let fname = f.name.to_string().to_ascii_uppercase();
            ensure!(
                matches!(
                    fname.as_str(),
                    "COUNT" | "SUM" | "AVG" | "MIN" | "MAX" | "COALESCE" | "ROUND"
                ),
                "G-SQL2 blocked function: {fname}"
            );
            if let FunctionArguments::List(arg_list) = &f.args {
                for a in &arg_list.args {
                    if let FunctionArg::Unnamed(FunctionArgExpr::Expr(inner)) = a {
                        validate_expr_columns(inner, allowed_cols)?;
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
            validate_expr_columns(inner, allowed_cols)?;
            validate_expr_columns(low, allowed_cols)?;
            validate_expr_columns(high, allowed_cols)
        }
        Expr::InList {
            expr: inner, list, ..
        } => {
            validate_expr_columns(inner, allowed_cols)?;
            for item in list {
                validate_expr_columns(item, allowed_cols)?;
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
                validate_expr_columns(op, allowed_cols)?;
            }
            for cw in conditions {
                validate_expr_columns(&cw.condition, allowed_cols)?;
                validate_expr_columns(&cw.result, allowed_cols)?;
            }
            if let Some(el) = else_result {
                validate_expr_columns(el, allowed_cols)?;
            }
            Ok(())
        }
        _ => bail!("G-SQL2 blocked unsupported AST expression"),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_layer_sql_guard_blocks_hostile_queries_and_wraps_valid_select() {
        let wl = default_arrow_view_whitelist();
        for bad in [
            "DROP TABLE gate_ledger",
            "DELETE FROM docs_entities WHERE 1=1",
            "SELECT doc_id FROM docs_entities UNION SELECT entry_id FROM gate_ledger",
            "SELECT * FROM secret_users",
            "SELECT read_file('/etc/passwd') FROM docs_entities",
            "SELECT hallucinated_col FROM docs_entities",
            "SELECT * FROM docs_entities CROSS JOIN gate_ledger",
            "SELECT doc_id FROM docs_entities; DROP TABLE gate_ledger",
            "ATTACH '/tmp/x.db' AS x",
            "VALUES (1, 2)",
        ] {
            assert!(
                verify_and_wrap_safe_sql(DeviceRole::PersonalPhone, "vault-01", N2sfGrade::Sensitive, bad, &wl).is_err(),
                "escaped SQL guard: {bad}"
            );
        }
        assert!(verify_and_wrap_safe_sql(
            DeviceRole::CounterKiosk,
            "vault-open",
            N2sfGrade::Open,
            "SELECT doc_id FROM docs_entities",
            &wl
        )
        .is_err());

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

        let g = WriterGuard::citizen_guard(Vec::new(), Vec::new());
        assert!(sanitize_sql_output_cells(&[vec!["[RRN_REDACTED]".into(), "150000".into()]], &g).is_ok());
        assert!(sanitize_sql_output_cells(&[vec!["위장계약".into()]], &g).is_err());
        assert!(sanitize_sql_output_cells(&[vec!["900101-1234567".into()]], &g).is_err());
    }
}
