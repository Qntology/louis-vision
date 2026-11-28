pub mod router_host;
pub mod sql_guard;
pub mod ui_nn_defs;

use std::sync::{Arc, Mutex};

use lv_agent::gates::{check_manual_amount, select_form_for_verdict, AgentVerdict, OutputForm};
use lv_core::device::rocm_exit_now_if_used;
use lv_doc::writer::guard::{N2sfGrade, ReverseCdrContext, WriterGuard};
use lv_router::vault::{DeviceRole, VaultError};
use router_host::{ActiveSurfaceOwner, HostRouterState, RouteDecision};
use serde::Serialize;
use sql_guard::{validate_and_wrap_sql, ValidatedSqlPlan};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StudioFormChoice {
    pub form: OutputForm,
    pub overridden: bool,
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
    pub vault_id: String,
    pub role: DeviceRole,
    pub max_n2sf_grade: u8,
}

impl Default for LouisAppState {
    fn default() -> Self {
        Self {
            router: Arc::new(Mutex::new(HostRouterState::default())),
            vault_id: "vault-default".to_string(),
            role: DeviceRole::PersonalPhone,
            max_n2sf_grade: 1,
        }
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
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            router_state,
            router_route_input,
            sql_validate,
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
    }
}
