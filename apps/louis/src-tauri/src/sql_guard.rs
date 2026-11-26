//! 5-Layer Local DataFusion SQL AST Safety Guard (`sql_guard.rs`):
//! - G-SQL1: Exactly 1 `Statement::Query(Select)` (zero DDL/DML, zero file-reading functions).
//! - G-SQL2: Whitelist of 5 Arrow views (`docs_entities`, `gate_ledger`, `media_events`, `sim_samples`, `stage_cues`) & physical columns.
//! - G-SQL3: `vault_id` & `n2sf_grade` mandatory subquery wrapper (`CounterKiosk` -> `KioskForbidden`).
//! - G-SQL4: Forbids `CROSS JOIN`, enforces `LIMIT <= 200` and `1500 ms` execution cap.
//! - G-SQL5: Filters serialized output cells through Layer 0 `WriterGuard` (`CITIZEN_FORBIDDEN_TERMS` 16 terms + PII).

use lv_doc::writer::guard::WriterGuard;
use lv_router::vault::{DeviceRole, VaultError};
use serde::{Deserialize, Serialize};
use sqlparser::ast::{
    Expr, JoinOperator, LimitClause, Query, Select, SelectItem, SetExpr, Statement, TableFactor,
    Value, ValueWithSpan,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

pub const SQL_MAX_LIMIT: u64 = 200;
pub const SQL_TIMEOUT_MS: u64 = 1_500;

pub const ALLOWED_ARROW_VIEWS: &[(&str, &[&str])] = &[
    ("docs_entities", &["doc_id", "vault_id", "n2sf_grade", "pack_id", "vendor_id", "entity_key", "amt_krw", "currency", "status", "doc_date"]),
    ("gate_ledger", &["entry_id", "vault_id", "n2sf_grade", "doc_id", "gate_code", "verdict", "overridden", "verified_krw", "evidence_krw", "risk_score", "created_at"]),
    ("media_events", &["media_id", "vault_id", "n2sf_grade", "ts_ms", "bbox", "label", "scheme"]),
    ("sim_samples", &["run_id", "vault_id", "n2sf_grade", "tick", "unit_id", "err_m", "provenance"]),
    ("stage_cues", &["cue_id", "vault_id", "n2sf_grade", "smpte", "delta_m", "interlock_ok", "ts_ms"]),
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
];

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

pub fn validate_and_wrap_sql(
    raw_sql: &str,
    role: DeviceRole,
    active_vault_id: &str,
    max_n2sf_grade: u8,
) -> std::result::Result<ValidatedSqlPlan, SqlGuardError> {
    if role == DeviceRole::CounterKiosk {
        return Err(SqlGuardError::KioskForbidden);
    }
    let clean_vault = sanitize_sql_ident_literal(active_vault_id);
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

    let stmts = Parser::parse_sql(&GenericDialect {}, trimmed).map_err(|e| SqlGuardError::Blocked {
        gate: SqlGuardGate::G1SingleSelectOnly,
        reason: format!("SQL parse error: {e}"),
    })?;

    if stmts.len() != 1 {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G1SingleSelectOnly,
            reason: format!("expected exactly 1 SELECT statement, found {}", stmts.len()),
        });
    }

    let query = match &stmts[0] {
        Statement::Query(q) => q,
        other => {
            return Err(SqlGuardError::Blocked {
                gate: SqlGuardGate::G1SingleSelectOnly,
                reason: format!("non-SELECT statement forbidden: {}", statement_kind_name(other)),
            });
        }
    };

    let mut tables = Vec::new();
    let mut effective_limit = SQL_MAX_LIMIT;
    inspect_query_ast(query, &mut tables, &mut effective_limit)?;
    if tables.is_empty() {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G2TableColumnWhitelist,
            reason: "query must reference at least one whitelisted view".into(),
        });
    }

    // Wrap user SELECT inside mandatory vault + N2SF filter & LIMIT 200 cap.
    let inner_sql = query.to_string();
    let wrapped_sql = format!(
        "SELECT * FROM ({inner_sql}) AS _lv_scope WHERE vault_id = '{clean_vault}' AND n2sf_grade <= {max_n2sf_grade} LIMIT {effective_limit}"
    );
    Ok(ValidatedSqlPlan {
        wrapped_sql,
        tables_referenced: tables,
        effective_limit,
        timeout_ms: SQL_TIMEOUT_MS,
    })
}

/// Gate G-SQL5: blocks or redacts output cell values containing any of the 16 `CITIZEN_FORBIDDEN_TERMS` or PII patterns.
pub fn filter_output_rows(rows: &[Vec<String>], extra_forbidden: &[String]) -> std::result::Result<(), SqlGuardError> {
    let guard = WriterGuard::citizen_guard(extra_forbidden.to_vec(), Vec::new());
    for row in rows {
        for cell in row {
            if let Err(e) = guard.verify_text(cell) {
                return Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G5WriterOutputFilter,
                    reason: e.to_string(),
                });
            }
            if contains_korean_rrn_or_account(cell) {
                return Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G5WriterOutputFilter,
                    reason: "potential citizen PII pattern detected in SQL cell".into(),
                });
            }
        }
    }
    Ok(())
}

fn contains_korean_rrn_or_account(s: &str) -> bool {
    let bytes = s.as_bytes();
    // Matches `\d{6}-[1-4]\d{6}` resident registration numbers without allocating regex engines.
    if bytes.len() < 14 {
        return false;
    }
    for w in bytes.windows(14) {
        if w[..6].iter().all(u8::is_ascii_digit)
            && w[6] == b'-'
            && matches!(w[7], b'1'..=b'4')
            && w[8..14].iter().all(u8::is_ascii_digit)
        {
            return true;
        }
    }
    false
}

fn sanitize_sql_ident_literal(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

fn statement_kind_name(stmt: &Statement) -> &'static str {
    match stmt {
        Statement::Insert(_) => "INSERT",
        Statement::Update { .. } => "UPDATE",
        Statement::Delete(_) => "DELETE",
        Statement::Drop { .. } => "DROP",
        Statement::AlterTable { .. } => "ALTER",
        Statement::Copy { .. } => "COPY",
        Statement::AttachDuckDBDatabase { .. } => "ATTACH",
        _ => "NON_SELECT",
    }
}

fn inspect_query_ast(
    query: &Query,
    tables: &mut Vec<String>,
    effective_limit: &mut u64,
) -> std::result::Result<(), SqlGuardError> {
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            inspect_query_ast(&cte.query, tables, effective_limit)?;
        }
    }
    if let Some(LimitClause::LimitOffset { limit: Some(expr), .. }) = &query.limit_clause {
        if let Some(n) = extract_u64_lit(expr) {
            *effective_limit = n.min(SQL_MAX_LIMIT).max(1);
        }
    }
    inspect_set_expr(&query.body, tables, effective_limit)
}

fn extract_u64_lit(expr: &Expr) -> Option<u64> {
    if let Expr::Value(ValueWithSpan { value: Value::Number(s, _), .. }) = expr {
        return s.parse::<u64>().ok();
    }
    None
}

fn inspect_set_expr(
    body: &SetExpr,
    tables: &mut Vec<String>,
    effective_limit: &mut u64,
) -> std::result::Result<(), SqlGuardError> {
    match body {
        SetExpr::Select(sel) => inspect_select(sel, tables, effective_limit),
        SetExpr::Query(q) => inspect_query_ast(q, tables, effective_limit),
        SetExpr::SetOperation { left, right, .. } => {
            inspect_set_expr(left, tables, effective_limit)?;
            inspect_set_expr(right, tables, effective_limit)
        }
        _ => Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G1SingleSelectOnly,
            reason: "unsupported DML/VALUES set expression inside query".into(),
        }),
    }
}

fn inspect_select(
    sel: &Select,
    tables: &mut Vec<String>,
    effective_limit: &mut u64,
) -> std::result::Result<(), SqlGuardError> {
    if sel.into.is_some() {
        return Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G1SingleSelectOnly,
            reason: "SELECT INTO forbidden".into(),
        });
    }

    let mut local_tables = Vec::new();
    for twj in &sel.from {
        inspect_table_factor(&twj.relation, &mut local_tables, effective_limit)?;
        for join in &twj.joins {
            if matches!(join.join_operator, JoinOperator::CrossJoin(_) | JoinOperator::CrossApply) {
                return Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G4LimitAndJoinBudget,
                    reason: "Cartesian CROSS JOIN is forbidden".into(),
                });
            }
            inspect_table_factor(&join.relation, &mut local_tables, effective_limit)?;
        }
    }

    // Build combined column whitelist across active referenced tables.
    let mut allowed_cols: Vec<&'static str> = Vec::new();
    for t in &local_tables {
        if let Some((_, cols)) = ALLOWED_ARROW_VIEWS.iter().find(|(name, _)| name == t) {
            allowed_cols.extend_from_slice(cols);
        }
    }
    tables.extend(local_tables);

    // Check expressions across projection, WHERE, and HAVING.
    for item in &sel.projection {
        match item {
            SelectItem::UnnamedExpr(e) | SelectItem::ExprWithAlias { expr: e, .. } => {
                inspect_expr(e, &allowed_cols, tables, effective_limit)?;
            }
            SelectItem::Wildcard(_) => {}
            SelectItem::QualifiedWildcard(_, _) => {}
        }
    }
    if let Some(where_expr) = &sel.selection {
        inspect_expr(where_expr, &allowed_cols, tables, effective_limit)?;
    }
    if let Some(having) = &sel.having {
        inspect_expr(having, &allowed_cols, tables, effective_limit)?;
    }
    Ok(())
}

fn inspect_table_factor(
    tf: &TableFactor,
    local_tables: &mut Vec<String>,
    effective_limit: &mut u64,
) -> std::result::Result<(), SqlGuardError> {
    match tf {
        TableFactor::Table { name, args, .. } => {
            if args.is_some() {
                return Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G1SingleSelectOnly,
                    reason: format!("table-valued function forbidden: {name}"),
                });
            }
            let tbl = name.to_string().to_ascii_lowercase();
            let base = tbl.rsplit('.').next().unwrap_or(&tbl).to_string();
            if !ALLOWED_ARROW_VIEWS.iter().any(|(v, _)| *v == base) {
                return Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G2TableColumnWhitelist,
                    reason: format!("unknown or unauthorized view '{base}'"),
                });
            }
            local_tables.push(base);
            Ok(())
        }
        TableFactor::Derived { subquery, .. } => {
            inspect_query_ast(subquery, local_tables, effective_limit)
        }
        _ => Err(SqlGuardError::Blocked {
            gate: SqlGuardGate::G1SingleSelectOnly,
            reason: "unsupported FROM relation or file function".into(),
        }),
    }
}

fn inspect_expr(
    expr: &Expr,
    allowed_cols: &[&str],
    tables: &mut Vec<String>,
    effective_limit: &mut u64,
) -> std::result::Result<(), SqlGuardError> {
    match expr {
        Expr::Identifier(ident) => {
            let c = ident.value.to_ascii_lowercase();
            if !allowed_cols.is_empty() && !allowed_cols.contains(&c.as_str()) {
                return Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G2TableColumnWhitelist,
                    reason: format!("hallucinated or non-whitelisted column '{c}'"),
                });
            }
            Ok(())
        }
        Expr::CompoundIdentifier(parts) => {
            if let Some(col) = parts.last() {
                let c = col.value.to_ascii_lowercase();
                if !allowed_cols.is_empty() && !allowed_cols.contains(&c.as_str()) {
                    return Err(SqlGuardError::Blocked {
                        gate: SqlGuardGate::G2TableColumnWhitelist,
                        reason: format!("hallucinated column '{c}'"),
                    });
                }
            }
            Ok(())
        }
        Expr::Function(f) => {
            let fname = f.name.to_string().to_ascii_lowercase();
            if FORBIDDEN_FILE_FUNCTIONS.contains(&fname.as_str()) {
                return Err(SqlGuardError::Blocked {
                    gate: SqlGuardGate::G1SingleSelectOnly,
                    reason: format!("forbidden file-access function '{fname}'"),
                });
            }
            if let sqlparser::ast::FunctionArguments::List(arg_list) = &f.args {
                for a in &arg_list.args {
                    if let sqlparser::ast::FunctionArg::Unnamed(
                        sqlparser::ast::FunctionArgExpr::Expr(inner),
                    ) = a
                    {
                        inspect_expr(inner, allowed_cols, tables, effective_limit)?;
                    }
                }
            }
            Ok(())
        }
        Expr::BinaryOp { left, right, .. } => {
            inspect_expr(left, allowed_cols, tables, effective_limit)?;
            inspect_expr(right, allowed_cols, tables, effective_limit)
        }
        Expr::UnaryOp { expr: inner, .. } | Expr::Nested(inner) => {
            inspect_expr(inner, allowed_cols, tables, effective_limit)
        }
        Expr::Subquery(q) => inspect_query_ast(q, tables, effective_limit),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_layer_sql_guard_blocks_20_attack_vectors_and_wraps_valid_queries() {
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
        for sql in attacks {
            assert!(
                validate_and_wrap_sql(sql, DeviceRole::PersonalPhone, "vault-fin-01", 1).is_err(),
                "attack vector escaped guard: {sql}"
            );
        }

        // CounterKiosk cannot run any SQL at all.
        assert_eq!(
            validate_and_wrap_sql(
                "SELECT doc_id FROM docs_entities",
                DeviceRole::CounterKiosk,
                "vault-open",
                0
            ),
            Err(SqlGuardError::KioskForbidden)
        );

        // Valid aggregation wraps with mandatory vault + n2sf_grade + LIMIT 200 cap.
        let plan = validate_and_wrap_sql(
            "SELECT doc_id, SUM(amt_krw) AS total_krw FROM docs_entities WHERE status = 'open' LIMIT 9999",
            DeviceRole::PersonalPhone,
            "vault-trade-08",
            1,
        )
        .unwrap();
        assert_eq!(plan.effective_limit, 200);
        assert_eq!(plan.timeout_ms, 1_500);
        assert!(plan.wrapped_sql.contains("vault_id = 'vault-trade-08'"));
        assert!(plan.wrapped_sql.contains("n2sf_grade <= 1"));
        assert!(plan.wrapped_sql.ends_with("LIMIT 200"));

        // Gate 5 blocks 16 forbidden terms & Korean RRN leaks in cell output.
        assert!(filter_output_rows(&[vec!["정상 정산".into(), "120000".into()]], &[]).is_ok());
        assert!(filter_output_rows(&[vec!["G3Violation 발생".into()]], &[]).is_err());
        assert!(filter_output_rows(&[vec!["900101-1234567".into()]], &[]).is_err());
    }
}
