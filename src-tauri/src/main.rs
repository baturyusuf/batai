#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{path::PathBuf, sync::Arc, time::Duration};

use batai::{
    daemon::{self, ClientOrigin, DaemonClient, HandshakeResponse},
    domain::{AppSnapshot, ConnectionGuide, MessageReceipt, ProviderConnection},
    project::discover_project_root,
    runtime::{
        benchmark::BenchmarkResult,
        delivery::{DeliveryCheckpoint, DeliveryPolicy, ExternalLink},
        economic::{EconomicPolicy, ResourceProfile},
        governance::{MutationRequest, MutationResult, ProviderApproval, ReviewOutcome},
        learning::{CapabilityLearningPolicy, RouterReplayResult},
        meetings::{CreateMeetingRequest, Meeting},
        recovery::{OperationJournal, RecoveryAction},
        types::Task,
    },
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tauri::Emitter;
use tokio::sync::RwLock;

type SharedClient = Arc<RwLock<DaemonClient>>;

struct AppState {
    client: SharedClient,
}

async fn rpc<T: DeserializeOwned>(
    state: &tauri::State<'_, AppState>,
    method: &str,
    params: Value,
) -> Result<T, String> {
    let client = state.client.read().await.clone();
    client.call(method, params).await
}

#[tauri::command]
async fn get_daemon_status(state: tauri::State<'_, AppState>) -> Result<HandshakeResponse, String> {
    let client = state.client.read().await.clone();
    client.handshake().await
}

#[tauri::command]
async fn get_app_snapshot(state: tauri::State<'_, AppState>) -> Result<AppSnapshot, String> {
    rpc(&state, "get_app_snapshot", json!({})).await
}
#[tauri::command]
async fn get_provider_connections(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ProviderConnection>, String> {
    rpc(&state, "get_provider_connections", json!({})).await
}
#[tauri::command]
async fn get_connection_guide(
    provider_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ConnectionGuide, String> {
    rpc(
        &state,
        "get_connection_guide",
        json!({"providerId":provider_id}),
    )
    .await
}
#[tauri::command]
async fn get_local_ai_state(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    rpc(&state, "get_local_ai_state", json!({})).await
}
#[tauri::command]
async fn get_ollama_model_details(
    model_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Value, String> {
    rpc(
        &state,
        "get_ollama_model_details",
        json!({"modelId":model_id}),
    )
    .await
}
#[tauri::command]
async fn remove_ollama_model(
    model_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    rpc(&state, "remove_ollama_model", json!({"modelId":model_id})).await
}
#[tauri::command]
async fn pull_ollama_model(
    model_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    rpc(&state, "pull_ollama_model", json!({"modelId":model_id})).await
}
#[tauri::command]
async fn cancel_ollama_pull(
    model_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    rpc(&state, "cancel_ollama_pull", json!({"modelId":model_id})).await
}
#[tauri::command]
async fn benchmark_local_model(
    model_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<BenchmarkResult, String> {
    rpc(&state, "benchmark_local_model", json!({"modelId":model_id})).await
}
#[tauri::command]
async fn set_resource_credential(
    resource_id: String,
    secret: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    rpc(
        &state,
        "set_resource_credential",
        json!({"resourceId":resource_id,"secret":secret}),
    )
    .await
}
#[tauri::command]
async fn disconnect_resource(
    resource_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    rpc(
        &state,
        "disconnect_resource",
        json!({"resourceId":resource_id}),
    )
    .await
}
#[tauri::command]
async fn update_intelligence_resource(
    profile: ResourceProfile,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    rpc(
        &state,
        "update_intelligence_resource",
        json!({"profile":profile}),
    )
    .await
}
#[tauri::command]
async fn update_economic_policy(
    policy: EconomicPolicy,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    rpc(&state, "update_economic_policy", json!({"policy":policy})).await
}
#[tauri::command]
async fn update_capability_learning_policy(
    policy: CapabilityLearningPolicy,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    rpc(
        &state,
        "update_capability_learning_policy",
        json!({"policy":policy}),
    )
    .await
}
#[tauri::command]
async fn add_manual_capability_evidence(
    resource_id: String,
    model: Option<String>,
    dimension: String,
    score: u8,
    note: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<ResourceProfile, String> {
    rpc(&state,"add_manual_capability_evidence",json!({"resourceId":resource_id,"model":model,"dimension":dimension,"score":score,"note":note})).await
}
#[tauri::command]
async fn acknowledge_resource_terms(
    resource_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ResourceProfile, String> {
    rpc(
        &state,
        "acknowledge_resource_terms",
        json!({"resourceId":resource_id}),
    )
    .await
}
#[tauri::command]
async fn replay_routing_decision(
    decision_id: String,
    policy: EconomicPolicy,
    state: tauri::State<'_, AppState>,
) -> Result<RouterReplayResult, String> {
    rpc(
        &state,
        "replay_routing_decision",
        json!({"decisionId":decision_id,"policy":policy}),
    )
    .await
}
#[tauri::command]
async fn test_resource_connection(
    resource_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Value, String> {
    rpc(
        &state,
        "test_resource_connection",
        json!({"resourceId":resource_id}),
    )
    .await
}
#[tauri::command]
async fn send_director_message(
    content: String,
    state: tauri::State<'_, AppState>,
) -> Result<MessageReceipt, String> {
    rpc(&state, "send_director_message", json!({"content":content})).await
}
#[tauri::command]
async fn import_github_issue(
    number: u64,
    state: tauri::State<'_, AppState>,
) -> Result<Task, String> {
    rpc(&state, "import_github_issue", json!({"number":number})).await
}
#[tauri::command]
async fn create_github_issue_for_task(
    task_id: String,
    labels: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<ExternalLink, String> {
    rpc(
        &state,
        "create_github_issue_for_task",
        json!({"taskId":task_id,"labels":labels}),
    )
    .await
}
#[tauri::command]
async fn prepare_task_delivery(
    task_id: String,
    agent_id: String,
    base: Option<String>,
    policy: DeliveryPolicy,
    state: tauri::State<'_, AppState>,
) -> Result<DeliveryCheckpoint, String> {
    rpc(
        &state,
        "prepare_task_delivery",
        json!({"taskId":task_id,"agentId":agent_id,"base":base,"policy":policy}),
    )
    .await
}
#[tauri::command]
async fn commit_task_delivery(
    task_id: String,
    agent_id: String,
    summary: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<DeliveryCheckpoint, String> {
    rpc(
        &state,
        "commit_task_delivery",
        json!({"taskId":task_id,"agentId":agent_id,"summary":summary}),
    )
    .await
}
#[tauri::command]
async fn push_task_delivery(
    task_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<DeliveryCheckpoint, String> {
    rpc(&state, "push_task_delivery", json!({"taskId":task_id})).await
}
#[tauri::command]
async fn create_task_pull_request(
    task_id: String,
    tests: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<DeliveryCheckpoint, String> {
    rpc(
        &state,
        "create_task_pull_request",
        json!({"taskId":task_id,"tests":tests}),
    )
    .await
}
#[tauri::command]
async fn sync_task_github(
    task_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<DeliveryCheckpoint, String> {
    rpc(&state, "sync_task_github", json!({"taskId":task_id})).await
}
#[tauri::command]
async fn request_task_github_review(
    task_id: String,
    reviewer: String,
    state: tauri::State<'_, AppState>,
) -> Result<DeliveryCheckpoint, String> {
    rpc(
        &state,
        "request_task_github_review",
        json!({"taskId":task_id,"reviewer":reviewer}),
    )
    .await
}
#[tauri::command]
async fn request_task_merge_approval(
    task_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<DeliveryCheckpoint, String> {
    rpc(
        &state,
        "request_task_merge_approval",
        json!({"taskId":task_id}),
    )
    .await
}
#[tauri::command]
async fn merge_task_pull_request(
    task_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<DeliveryCheckpoint, String> {
    rpc(&state, "merge_task_pull_request", json!({"taskId":task_id})).await
}
#[tauri::command]
async fn cleanup_task_worktree(
    task_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<DeliveryCheckpoint, String> {
    rpc(&state, "cleanup_task_worktree", json!({"taskId":task_id})).await
}
#[tauri::command]
async fn close_task_github_issue(
    task_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ExternalLink, String> {
    rpc(&state, "close_task_github_issue", json!({"taskId":task_id})).await
}
#[tauri::command]
async fn cancel_task(task_id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    rpc(&state, "cancel_task", json!({"taskId":task_id})).await
}
#[tauri::command]
async fn mutate_organization(
    request: MutationRequest,
    state: tauri::State<'_, AppState>,
) -> Result<MutationResult, String> {
    rpc(&state, "mutate_organization", json!({"request":request})).await
}
#[tauri::command]
async fn resolve_god_decision(
    decision_id: String,
    approve: bool,
    note: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<MutationResult, String> {
    rpc(
        &state,
        "resolve_god_decision",
        json!({"decisionId":decision_id,"approve":approve,"note":note}),
    )
    .await
}
#[tauri::command]
async fn resolve_provider_approval(
    approval_id: String,
    approve: bool,
    state: tauri::State<'_, AppState>,
) -> Result<ProviderApproval, String> {
    rpc(
        &state,
        "resolve_provider_approval",
        json!({"approvalId":approval_id,"approve":approve}),
    )
    .await
}
#[tauri::command]
async fn resolve_recovery_operation(
    operation_id: String,
    action: RecoveryAction,
    state: tauri::State<'_, AppState>,
) -> Result<OperationJournal, String> {
    rpc(
        &state,
        "resolve_recovery_operation",
        json!({"operationId":operation_id,"action":action}),
    )
    .await
}
#[tauri::command]
async fn record_review_outcome(
    review: ReviewOutcome,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    rpc(&state, "record_review_outcome", json!({"review":review})).await
}

#[tauri::command]
async fn create_meeting(
    request: CreateMeetingRequest,
    state: tauri::State<'_, AppState>,
) -> Result<Meeting, String> {
    rpc(&state, "create_meeting", json!({"request": request})).await
}

#[tauri::command]
async fn get_meeting(
    meeting_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<Meeting>, String> {
    rpc(&state, "get_meeting", json!({"meetingId": meeting_id})).await
}

#[tauri::command]
async fn list_meetings(state: tauri::State<'_, AppState>) -> Result<Vec<Meeting>, String> {
    rpc(&state, "list_meetings", json!({})).await
}

#[tauri::command]
async fn cancel_meeting(
    meeting_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Meeting, String> {
    rpc(&state, "cancel_meeting", json!({"meetingId": meeting_id})).await
}

#[tauri::command]
async fn create_meeting_action_task(
    meeting_id: String,
    action_id: String,
    assigned_to: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Task, String> {
    rpc(
        &state,
        "create_meeting_action_task",
        json!({"meetingId": meeting_id, "actionId": action_id, "assignedTo": assigned_to}),
    )
    .await
}

fn project_root() -> PathBuf {
    std::env::var_os("BATAI_PROJECT_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok().map(discover_project_root))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn main() {
    let root = project_root();
    if std::env::args().any(|arg| arg == "--daemon-child") {
        let runtime = tokio::runtime::Runtime::new().expect("daemon runtime");
        runtime
            .block_on(daemon::run_daemon(root, None))
            .expect("Batai daemon failed");
        return;
    }
    let client = tauri::async_runtime::block_on(DaemonClient::connect_or_spawn(
        &root,
        ClientOrigin::Desktop,
    ))
    .expect("failed to connect Batai daemon");
    let shared = Arc::new(RwLock::new(client));
    let app = tauri::Builder::default()
        .manage(AppState {
            client: shared.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            get_daemon_status,
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
            import_github_issue,
            create_github_issue_for_task,
            prepare_task_delivery,
            commit_task_delivery,
            push_task_delivery,
            create_task_pull_request,
            sync_task_github,
            request_task_github_review,
            request_task_merge_approval,
            merge_task_pull_request,
            cleanup_task_worktree,
            close_task_github_issue,
            cancel_task,
            mutate_organization,
            resolve_god_decision,
            resolve_provider_approval,
            resolve_recovery_operation,
            record_review_outcome,
            create_meeting,
            get_meeting,
            list_meetings,
            cancel_meeting,
            create_meeting_action_task
        ])
        .build(tauri::generate_context!())
        .expect("error while building Batai desktop shell");
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let mut backoff = Duration::from_millis(150);
        loop {
            let current = shared.read().await.clone();
            match current.next_event().await {
                Ok((Some(event), _)) => {
                    backoff = Duration::from_millis(150);
                    let _ = handle.emit("batai://runtime-event", event.clone());
                    if event.get("type").and_then(Value::as_str) == Some("DAEMON_STOPPING") {
                        let _ = handle.emit("batai://daemon-status", json!({"state":"STOPPING"}));
                        break;
                    }
                    if event.get("type").and_then(Value::as_str)
                        == Some("LOCAL_MODEL_PULL_PROGRESS")
                    {
                        let _ = handle.emit(
                            "batai://ollama-pull-progress",
                            event.get("payload").cloned().unwrap_or(Value::Null),
                        );
                    }
                }
                Ok((None, true)) => {
                    let _ = handle.emit("batai://runtime-event", json!({"type":"RESYNC_REQUIRED"}));
                }
                Ok((None, false)) => {}
                Err(_) => {
                    let _ = handle.emit("batai://daemon-status", json!({"state":"DISCONNECTED"}));
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(5));
                    if let Ok(reconnected) =
                        DaemonClient::connect_or_spawn(&root, ClientOrigin::Desktop).await
                    {
                        *shared.write().await = reconnected;
                        let _ = handle.emit("batai://daemon-status", json!({"state":"CONNECTED"}));
                        let _ =
                            handle.emit("batai://runtime-event", json!({"type":"RESYNC_REQUIRED"}));
                    }
                }
            }
        }
    });
    app.run(|_, _| {});
}
