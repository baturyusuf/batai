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
            cancel_task
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
