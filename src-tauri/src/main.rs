#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod domain;
mod project;
mod providers;

use std::path::PathBuf;

use domain::{AppSnapshot, ConnectionGuide, MessageReceipt, ProviderConnection};
use project::{discover_project_root, ProjectStore};

struct AppState {
    project: ProjectStore,
}

#[tauri::command]
fn get_app_snapshot(state: tauri::State<'_, AppState>) -> Result<AppSnapshot, String> {
    state.project.snapshot().map_err(|error| error.to_string())
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

fn main() {
    let project_root = std::env::var_os("BATAI_PROJECT_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok().map(discover_project_root))
        .unwrap_or_else(|| PathBuf::from("."));

    tauri::Builder::default()
        .manage(AppState {
            project: ProjectStore::new(project_root),
        })
        .invoke_handler(tauri::generate_handler![
            get_app_snapshot,
            get_provider_connections,
            get_connection_guide,
            send_director_message
        ])
        .run(tauri::generate_context!())
        .expect("error while running Batai desktop shell");
}
