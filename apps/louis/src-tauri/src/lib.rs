pub mod router_host;
pub mod sql_guard;
pub mod ui_nn_defs;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use louis_docs::vault_lan::ensure_local_lan_addr;
use lv_agent::gates::{check_manual_amount, select_form_for_verdict, AgentVerdict, OutputForm};
use lv_core::device::rocm_exit_now_if_used;
use lv_doc::writer::guard::{N2sfGrade, ReverseCdrContext, WriterGuard};
use lv_router::vault::{DeviceRole, VaultError};
use router_host::{ActiveSurfaceOwner, HostRouterState, RouteDecision};
use serde::Serialize;
use sql_guard::{
    default_demo_arrow_views, execute_guarded_sql, validate_and_wrap_sql, SqlExecutionResult,
    SqlGuardError, SqlGuardGate, SqlRowMap, ValidatedSqlPlan, SQL_TIMEOUT_MS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StudioFormChoice {
    pub form: OutputForm,
    pub overridden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrivacyEpochAck {
    pub epoch: u64,
    pub locked: bool,
    #[serde(rename = "miniFeedBlurred")]
    pub mini_feed_blurred: bool,
    #[serde(rename = "stage3dScreenNonzeroBytes")]
    pub stage_3d_screen_nonzero_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LanPairCheckResult {
    pub allowed: bool,
    pub error: Option<String>,
}

pub fn check_field3d_role(role: DeviceRole) -> Result<(), String> {
    if role == DeviceRole::CounterKiosk {
        Err(VaultError::KioskForbidden.to_string())
    } else {
        Ok(())
    }
}

pub fn run_reverse_cdr_guard(
    text: &str,
    source_grade: N2sfGrade,
    target_grade: N2sfGrade,
    declassification_ref: Option<String>,
    stripped_covert_artifacts: usize,
    block_on_covert_artifacts: bool,
) -> Result<(), String> {
    let mut guard = WriterGuard::citizen_guard(Vec::new(), Vec::new());
    guard.reverse_cdr = Some(ReverseCdrContext {
        source_grade: Some(source_grade),
        target_grade: Some(target_grade),
        declassification_ref,
        stripped_covert_artifacts,
        block_on_covert_artifacts,
    });
    guard.verify_text(text).map_err(|e| e.to_string())
}

pub struct LouisAppState {
    pub router: Arc<Mutex<HostRouterState>>,
    pub views: Arc<Mutex<BTreeMap<String, Vec<SqlRowMap>>>>,
    pub vault_id: String,
    pub role: DeviceRole,
    pub max_n2sf_grade: u8,
}

impl Default for LouisAppState {
    fn default() -> Self {
        let vault_id = "vault-default".to_string();
        let views = default_demo_arrow_views(&vault_id);
        Self {
            router: Arc::new(Mutex::new(HostRouterState::default())),
            views: Arc::new(Mutex::new(views)),
            vault_id,
            role: DeviceRole::PersonalPhone,
            max_n2sf_grade: 1,
        }
    }
}

/// Executes a guarded SQL query against `LouisAppState`'s local Arrow views (X-06 / D-01).
pub fn run_sql_execute(sql: &str, state: &LouisAppState) -> SqlExecutionResult {
    let views = state.views.lock().unwrap_or_else(|p| p.into_inner());
    match execute_guarded_sql(sql, state.role, &state.vault_id, state.max_n2sf_grade, &views) {
        Ok(res) => res,
        Err(err) => {
            let (gate, msg) = match err {
                SqlGuardError::KioskForbidden => (
                    SqlGuardGate::G3VaultAndN2sfScope,
                    SqlGuardError::KioskForbidden.to_string(),
                ),
                SqlGuardError::Blocked { gate, reason } => (gate, reason),
            };
            SqlExecutionResult {
                ok: false,
                wrapped_sql: String::new(),
                table: String::new(),
                columns: Vec::new(),
                rows: Vec::new(),
                row_count: 0,
                timeout_ms: SQL_TIMEOUT_MS,
                gate: Some(gate),
                error: Some(msg),
            }
        }
    }
}

pub fn check_lan_pair_addr(addr: &str) -> LanPairCheckResult {
    match addr.parse::<SocketAddr>() {
        Ok(sa) => match ensure_local_lan_addr(sa) {
            Ok(_) => LanPairCheckResult {
                allowed: true,
                error: None,
            },
            Err(e) => LanPairCheckResult {
                allowed: false,
                error: Some(e.to_string()),
            },
        },
        Err(_) => LanPairCheckResult {
            allowed: false,
            error: Some("InvalidSocketAddr".into()),
        },
    }
}

#[tauri::command]
fn router_state(state: tauri::State<'_, LouisAppState>) -> HostRouterState {
    state.router.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

#[tauri::command]
fn router_route_input(
    text: String,
    files: Option<Vec<String>>,
    state: tauri::State<'_, LouisAppState>,
) -> RouteDecision {
    let owned = files.unwrap_or_default();
    let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
    router_host::route_intent(&text, &refs, state.role)
}

#[tauri::command]
fn sql_validate(sql: String, state: tauri::State<'_, LouisAppState>) -> Result<ValidatedSqlPlan, String> {
    validate_and_wrap_sql(&sql, state.role, &state.vault_id, state.max_n2sf_grade)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn sql_execute(sql: String, state: tauri::State<'_, LouisAppState>) -> SqlExecutionResult {
    run_sql_execute(&sql, &state)
}

#[tauri::command]
fn broadcast_privacy_epoch(
    locked: bool,
    reason: Option<String>,
    state: tauri::State<'_, LouisAppState>,
) -> PrivacyEpochAck {
    let mut g = state.router.lock().unwrap_or_else(|p| p.into_inner());
    let next_epoch = g.privacy_epoch + 1;
    let _ = g.apply_wire(router_host::LoopbackWireMsg::BroadcastPrivacyEpoch {
        epoch: next_epoch,
        locked,
        reason: reason.unwrap_or_else(|| "manual".into()),
    });
    PrivacyEpochAck {
        epoch: g.privacy_epoch,
        locked: g.privacy_locked,
        mini_feed_blurred: g.privacy_locked,
        stage_3d_screen_nonzero_bytes: 0,
    }
}

#[tauri::command]
fn lan_pair_check(addr: String) -> LanPairCheckResult {
    check_lan_pair_addr(&addr)
}

#[tauri::command]
fn set_surface_owner(owner: ActiveSurfaceOwner, state: tauri::State<'_, LouisAppState>) -> HostRouterState {
    let mut g = state.router.lock().unwrap_or_else(|p| p.into_inner());
    let seq = g.last_chat_seq;
    let _ = g.apply_wire(router_host::LoopbackWireMsg::HandoffStarWidget {
        to: owner,
        last_chat_seq: seq,
        active_tasks: 0,
    });
    g.clone()
}

#[tauri::command]
fn studio_select_form(
    requested: OutputForm,
    verdict: AgentVerdict,
    supplement_count: u32,
    simple_doc_missing: bool,
) -> StudioFormChoice {
    let c = select_form_for_verdict(requested, verdict, supplement_count, simple_doc_missing);
    StudioFormChoice {
        form: c.form,
        overridden: c.overridden,
    }
}

#[tauri::command]
fn studio_check_manual_amount(verified_sum: i64, requested: i64, evidence_sum: i64) -> Result<(), String> {
    check_manual_amount(verified_sum, requested, evidence_sum).map_err(|v| format!("{v:?}"))
}

#[tauri::command]
fn studio_reverse_cdr_check(
    text: String,
    source_grade: N2sfGrade,
    target_grade: N2sfGrade,
    declassification_ref: Option<String>,
    stripped_covert_artifacts: usize,
    block_on_covert_artifacts: bool,
) -> Result<(), String> {
    run_reverse_cdr_guard(
        &text,
        source_grade,
        target_grade,
        declassification_ref,
        stripped_covert_artifacts,
        block_on_covert_artifacts,
    )
}

#[tauri::command]
fn field3d_check_kiosk_role(role: Option<DeviceRole>, state: tauri::State<'_, LouisAppState>) -> Result<(), String> {
    check_field3d_role(role.unwrap_or(state.role))
}

pub fn run() {
    let state = LouisAppState::default();
    // X-06: Spawn Single-Anchor loopback router (`127.0.0.1:8787`) on desktop startup.
    let _ = router_host::spawn_loopback_router_listener(
        router_host::ROUTER_LOOPBACK_PORT,
        Arc::clone(&state.router),
    );
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            router_state,
            router_route_input,
            sql_validate,
            sql_execute,
            broadcast_privacy_epoch,
            lan_pair_check,
            set_surface_owner,
            studio_select_form,
            studio_check_manual_amount,
            studio_reverse_cdr_check,
            field3d_check_kiosk_role,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Louis Desktop application")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                rocm_exit_now_if_used(0);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn studio_and_field3d_tauri_commands_enforce_invariants() {
        let snapped = studio_select_form(
            OutputForm::ApprovalNotice,
            AgentVerdict::G2RejectContradictionLock,
            0,
            false,
        );
        assert_eq!(snapped.form, OutputForm::RejectNotice);
        assert!(snapped.overridden);

        let err = studio_check_manual_amount(80_000_000, 85_000_001, 5_000_000).unwrap_err();
        assert!(err.contains("G2BlockUnprovenEdit"));

        assert!(run_reverse_cdr_guard(
            "정상 문서",
            N2sfGrade::Classified,
            N2sfGrade::Open,
            Some("DECL-1".into()),
            0,
            true,
        )
        .is_err());
        assert!(run_reverse_cdr_guard(
            "순수 XML 재조립 완료",
            N2sfGrade::Sensitive,
            N2sfGrade::Open,
            Some("N2SF-2026-0812".into()),
            0,
            true,
        )
        .is_ok());

        assert!(check_field3d_role(DeviceRole::CounterKiosk)
            .unwrap_err()
            .to_ascii_lowercase()
            .contains("kiosk"));
        assert!(check_field3d_role(DeviceRole::PersonalPhone).is_ok());

        // X-06: sql_execute & lan_pair_check parity
        let st = LouisAppState::default();
        let ok = run_sql_execute("SELECT doc_id, amt_krw FROM docs_entities", &st);
        assert!(ok.ok);
        assert_eq!(ok.row_count, 2);
        assert!(ok.wrapped_sql.contains("vault_id = 'vault-default'"));

        let blocked = run_sql_execute("SELECT read_file('/etc/passwd'); DROP TABLE docs_entities", &st);
        assert!(!blocked.ok);
        assert_eq!(blocked.gate, Some(SqlGuardGate::G1SingleSelectOnly));

        assert!(check_lan_pair_addr("192.168.45.12:9999").allowed);
        assert!(!check_lan_pair_addr("8.8.8.8:9999").allowed);
    }
}
