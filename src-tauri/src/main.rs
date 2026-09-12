#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod domain;
#[allow(dead_code)]
mod project;
mod providers;
#[allow(dead_code)]
mod runtime;

use std::{collections::HashMap, path::PathBuf, sync::Mutex};
use tauri::Emitter;

use domain::{AppSnapshot, ConnectionGuide, MessageReceipt, ProviderConnection};
use project::{discover_project_root, ProjectStore};
use runtime::governance::{
    Actor, AuthorityScope, ManualCapabilityEdit, MutationRequest, MutationResult, ProviderApproval,
    ReviewOutcome,
};
use runtime::recovery::{OperationJournal, RecoveryAction};

struct AppState {
    project: ProjectStore,
    runtime: std::sync::Arc<runtime::BataiRuntime>,
    ollama_pulls: Mutex<HashMap<String, providers::ollama::PullCancellation>>,
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
async fn get_local_ai_state() -> Result<serde_json::Value, String> {
    let provider = providers::ollama::OllamaProvider::default();
    let state = provider.installation_state().await;
    let installed = provider.list_models().await.unwrap_or_default();
    let running = provider.running_models().await.unwrap_or_default();
    Ok(serde_json::json!({"state":state,"installed":installed,"running":running}))
}

#[tauri::command]
async fn get_ollama_model_details(model_id: String) -> Result<serde_json::Value, String> {
    providers::ollama::OllamaProvider::default()
        .show_model(&model_id)
        .await
        .map_err(|error| format!("Ollama model details failed: {error:?}"))
}

#[tauri::command]
async fn remove_ollama_model(model_id: String) -> Result<(), String> {
    providers::ollama::OllamaProvider::default()
        .remove_model(&model_id)
        .await
        .map_err(|error| format!("Ollama model removal failed: {error:?}"))
}

#[tauri::command]
async fn pull_ollama_model(
    model_id: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let catalog = runtime::benchmark::curated_catalog();
    let entry = catalog
        .iter()
        .find(|entry| entry.id == model_id)
        .ok_or_else(|| "Only reviewed Batai catalog models can be downloaded".to_string())?;
    let hardware = runtime::hardware::HardwareProfiler.detect();
    let required = entry
        .approximate_disk_bytes
        .saturating_add(2 * 1024 * 1024 * 1024);
    if hardware
        .available_disk_bytes
        .is_some_and(|available| available < required)
    {
        return Err("Not enough free disk space with the required safety margin".into());
    }
    let cancellation = providers::ollama::PullCancellation::default();
    state
        .ollama_pulls
        .lock()
        .map_err(|_| "download state unavailable")?
        .insert(model_id.clone(), cancellation.clone());
    let result = providers::ollama::OllamaProvider::default()
        .pull_model(&model_id, &cancellation, |progress| {
            let payload = serde_json::json!({"modelId":model_id,"progress":progress});
            let _ = app.emit("batai://ollama-pull-progress", payload.clone());
            let _ = app.emit(
                "batai://runtime-event",
                serde_json::json!({"type":"LOCAL_MODEL_PULL_PROGRESS","payload":payload}),
            );
        })
        .await
        .map_err(|error| format!("Ollama download failed: {error:?}"));
    state
        .ollama_pulls
        .lock()
        .map_err(|_| "download state unavailable")?
        .remove(&model_id);
    result
}

#[tauri::command]
fn cancel_ollama_pull(model_id: String, state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let pulls = state
        .ollama_pulls
        .lock()
        .map_err(|_| "download state unavailable")?;
    if let Some(cancellation) = pulls.get(&model_id) {
        cancellation.cancel();
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
async fn benchmark_local_model(
    model_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<runtime::benchmark::BenchmarkResult, String> {
    let provider = providers::ollama::OllamaProvider::default();
    let installed = provider
        .list_models()
        .await
        .map_err(|error| format!("Ollama unavailable: {error:?}"))?;
    if !installed
        .iter()
        .any(|model| model.name == model_id || model.model.as_deref() == Some(&model_id))
    {
        return Err("Model is not installed".into());
    }
    let hardware = runtime::hardware::HardwareProfiler.detect();
    let result =
        runtime::benchmark::run_benchmark(&provider, &model_id, &hardware.fingerprint).await;
    state
        .runtime
        .store
        .save_benchmark(&result)
        .map_err(|error| error.to_string())?;
    let mut resource = state
        .runtime
        .store
        .list_intelligence_resources()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|resource| resource.id == "ollama-local")
        .unwrap_or_else(|| {
            runtime::economic::native_resource_profile(
                "ollama-local",
                "ollama",
                "Ollama Local",
                runtime::economic::BillingMode::Local,
            )
        });
    resource.status = "AVAILABLE".into();
    resource.supported_models = vec![model_id];
    resource.capabilities = result.capabilities.clone();
    resource.capability_evidence = result.evidence.clone();
    state
        .runtime
        .store
        .upsert_intelligence_resource(&resource)
        .map_err(|error| error.to_string())?;
    Ok(result)
}

#[tauri::command]
fn set_resource_credential(
    resource_id: String,
    secret: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    use runtime::credentials::CredentialStore;
    if !matches!(
        resource_id.as_str(),
        "kimi-personal-membership" | "zai-coding-plan" | "minimax-token-plan"
    ) {
        return Err("Unsupported credential resource".into());
    }
    runtime::credentials::OsCredentialStore
        .set(&resource_id, &secret)
        .map_err(|error| error.to_string())?;
    if let Some(mut profile) = state
        .runtime
        .store
        .list_intelligence_resources()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|profile| profile.id == resource_id)
    {
        profile.status = "AVAILABLE".into();
        state
            .runtime
            .store
            .upsert_intelligence_resource(&profile)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn disconnect_resource(resource_id: String) -> Result<(), String> {
    use runtime::credentials::CredentialStore;
    runtime::credentials::OsCredentialStore
        .delete(&resource_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn update_intelligence_resource(
    profile: runtime::economic::ResourceProfile,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state
        .runtime
        .store
        .upsert_intelligence_resource(&profile)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn update_economic_policy(
    policy: runtime::economic::EconomicPolicy,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state
        .runtime
        .store
        .set_economic_policy(&policy)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn update_capability_learning_policy(
    policy: runtime::learning::CapabilityLearningPolicy,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state
        .runtime
        .store
        .set_capability_learning_policy(&policy)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn add_manual_capability_evidence(
    resource_id: String,
    model: Option<String>,
    dimension: String,
    score: u8,
    note: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<runtime::economic::ResourceProfile, String> {
    state
        .runtime
        .governance
        .record_manual_capability(ManualCapabilityEdit {
            actor: Actor {
                id: "god".into(),
                scope: AuthorityScope::Project,
            },
            resource_id,
            model,
            dimension,
            score,
            note,
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn acknowledge_resource_terms(
    resource_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<runtime::economic::ResourceProfile, String> {
    state
        .runtime
        .governance
        .acknowledge_resource_terms(
            &resource_id,
            Actor {
                id: "god".into(),
                scope: AuthorityScope::Project,
            },
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn replay_routing_decision(
    decision_id: String,
    policy: runtime::economic::EconomicPolicy,
    state: tauri::State<'_, AppState>,
) -> Result<runtime::learning::RouterReplayResult, String> {
    let original = state
        .runtime
        .store
        .get_routing_decision(&decision_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Routing decision not found".to_string())?;
    let resources = if original.resource_snapshot.is_empty() {
        state
            .runtime
            .store
            .list_intelligence_resources()
            .map_err(|error| error.to_string())?
    } else {
        original.resource_snapshot.clone()
    };
    let outcomes = state
        .runtime
        .store
        .list_task_outcomes(None, 10_000)
        .map_err(|error| error.to_string())?;
    let learning_policy = state
        .runtime
        .store
        .capability_learning_policy()
        .map_err(|error| error.to_string())?;
    let replayed = runtime::economic::QuotaAwareEconomicRouter.route_with_evidence(
        &original.task_id,
        &original.requirements,
        &resources,
        &policy,
        &outcomes,
        &learning_policy,
        chrono::Utc::now(),
    );
    let actual = outcomes
        .iter()
        .find(|outcome| outcome.routing_decision_id.as_deref() == Some(&original.id));
    let billing = |id: Option<&String>| {
        id.and_then(|id| {
            resources
                .iter()
                .find(|resource| &resource.id == id)
                .map(|resource| resource.billing_mode)
        })
    };
    Ok(runtime::learning::replay_result(
        &original,
        &replayed,
        actual,
        billing(original.selected_resource_id.as_ref()),
        billing(replayed.selected_resource_id.as_ref()),
    ))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionTestResult {
    status: String,
    detail: String,
    latency_ms: Option<u64>,
}

#[tauri::command]
async fn test_resource_connection(
    resource_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ConnectionTestResult, String> {
    let profile = state
        .runtime
        .store
        .list_intelligence_resources()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|profile| profile.id == resource_id)
        .ok_or_else(|| "Resource not found".to_string())?;
    if !matches!(
        resource_id.as_str(),
        "kimi-personal-membership" | "zai-coding-plan" | "minimax-token-plan"
    ) {
        return Err("Connection smoke is limited to hosted subscription adapters".into());
    }
    let mut agent: runtime::types::Agent = serde_json::from_value(serde_json::json!({
        "id":"connection-smoke","name":"Connection smoke","role_template":"Agent",
        "provider":profile.provider,"model":profile.supported_models.first().cloned().unwrap_or_default(),
        "status":"READY"
    }))
    .map_err(|error| error.to_string())?;
    agent.worktree = None;
    let task: runtime::types::Task = serde_json::from_value(serde_json::json!({
        "id":"connection-smoke","created_by":"god",
        "objective":"Reply with exactly BATAI_CONNECTION_OK. Do not call tools or modify files.",
        "assigned_to":["connection-smoke"],"status":"READY"
    }))
    .map_err(|error| error.to_string())?;
    let provider = state
        .runtime
        .sessions
        .provider_for(&agent)
        .map_err(|error| error.to_string())?;
    let started = std::time::Instant::now();
    let session = provider
        .create_session(&agent)
        .await
        .map_err(|error| error.to_string())?;
    match provider.send_task(&session.id, &agent, &task).await {
        Ok(_) => Ok(ConnectionTestResult {
            status: "CONNECTED".into(),
            detail: "Safe low-token inference completed; no capability evidence was created".into(),
            latency_ms: Some(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64),
        }),
        Err(error) => {
            let status = match error {
                runtime::execution_provider::ProviderFailure::AuthRequired => "AUTH_FAILED",
                runtime::execution_provider::ProviderFailure::RateLimited { .. } => "RATE_LIMITED",
                _ => "ERROR",
            };
            Ok(ConnectionTestResult {
                status: status.into(),
                detail: providers::process_supervisor::redact_secrets(&format!(
                    "Connection test failed: {error:?}"
                )),
                latency_ms: None,
            })
        }
    }
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

#[tauri::command]
fn resolve_recovery_operation(
    operation_id: String,
    action: RecoveryAction,
    state: tauri::State<'_, AppState>,
) -> Result<OperationJournal, String> {
    state
        .runtime
        .governance
        .resolve_recovery_operation(&operation_id, action)
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
            ollama_pulls: Mutex::new(HashMap::new()),
        })
        .invoke_handler(tauri::generate_handler![
            get_app_snapshot,
            get_provider_connections,
            get_connection_guide,
            get_local_ai_state,
            get_ollama_model_details,
            remove_ollama_model,
            pull_ollama_model,
            cancel_ollama_pull,
            benchmark_local_model,
            set_resource_credential,
            disconnect_resource,
            update_intelligence_resource,
            update_economic_policy,
            update_capability_learning_policy,
            add_manual_capability_evidence,
            acknowledge_resource_terms,
            replay_routing_decision,
            test_resource_connection,
            send_director_message,
            cancel_task,
            mutate_organization,
            resolve_god_decision,
            resolve_provider_approval,
            resolve_recovery_operation,
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
