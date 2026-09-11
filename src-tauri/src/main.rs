#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod domain;
#[allow(dead_code)]
mod project;
mod providers;
#[allow(dead_code)]
mod runtime;

use std::path::PathBuf;
use tauri::Emitter;

use domain::{AppSnapshot, ConnectionGuide, MessageReceipt, ProviderConnection};
use project::{discover_project_root, ProjectStore};
use runtime::governance::{
    AuthorityScope, MutationRequest, MutationResult, ProviderApproval, ReviewOutcome,
};

struct AppState {
    project: ProjectStore,
    runtime: std::sync::Arc<runtime::BataiRuntime>,
}

#[tauri::command]
fn get_app_snapshot(state: tauri::State<'_, AppState>) -> Result<AppSnapshot, String> {
    state.runtime.snapshot().map_err(|error| error.to_string())
}

#[tauri::command]
fn get_provider_connections() -> Vec<ProviderConnection> {
    providers::probe_all()
}

#[tauri::command]
fn get_connection_guide(provider_id: String) -> Result<ConnectionGuide, String> {
    providers::connection_guide(&provider_id)
        .ok_or_else(|| format!("Unknown provider: {provider_id}"))
}

#[tauri::command]
fn send_director_message(
    content: String,
    state: tauri::State<'_, AppState>,
) -> Result<MessageReceipt, String> {
    state
        .project
        .send_director_message(&content)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn cancel_task(task_id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state
        .runtime
        .tasks
        .cancel(&task_id, "user")
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn mutate_organization(
    mut request: MutationRequest,
    state: tauri::State<'_, AppState>,
) -> Result<MutationResult, String> {
    // Desktop organization editing is an explicit human/GOD interaction. Agent-originated
    // mutations enter through the runtime API and cannot claim this identity.
    request.actor.id = "god".into();
    request.actor.scope = AuthorityScope::Project;
    state
        .runtime
        .governance
        .mutate(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn resolve_god_decision(
    decision_id: String,
    approve: bool,
    note: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<MutationResult, String> {
    state
        .runtime
        .governance
        .resolve_decision(&decision_id, approve, note)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn resolve_provider_approval(
    approval_id: String,
    approve: bool,
    state: tauri::State<'_, AppState>,
) -> Result<ProviderApproval, String> {
    state
        .runtime
        .governance
        .resolve_provider_approval(&approval_id, approve)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn record_review_outcome(
    review: ReviewOutcome,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state
        .runtime
        .governance
        .record_review(review)
        .map_err(|error| error.to_string())
}

fn main() {
    let project_root = std::env::var_os("BATAI_PROJECT_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok().map(discover_project_root))
        .unwrap_or_else(|| PathBuf::from("."));

    let runtime =
        runtime::BataiRuntime::open(project_root.clone()).expect("failed to open Batai runtime");
    tauri::async_runtime::block_on(runtime.start()).expect("failed to start Batai runtime");

    let app = tauri::Builder::default()
        .manage(AppState {
            project: ProjectStore::new(project_root),
            runtime: runtime.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            get_app_snapshot,
            get_provider_connections,
            get_connection_guide,
            send_director_message,
            cancel_task,
            mutate_organization,
            resolve_god_decision,
            resolve_provider_approval,
            record_review_outcome
        ])
        .build(tauri::generate_context!())
        .expect("error while building Batai desktop shell");
    let app_handle = app.handle().clone();
    let mut runtime_events = runtime.events.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match runtime_events.recv().await {
                Ok(event) => {
                    let _ = app_handle.emit("batai://runtime-event", event);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    app.run(|_, _| {});
    tauri::async_runtime::block_on(runtime.shutdown()).expect("failed to stop Batai runtime");
}
