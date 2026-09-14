//! One-runtime-per-project daemon, authenticated local RPC, discovery and clients.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use axum::{
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, oneshot, Notify};

use crate::{
    application::{ApplicationMode, BataiApplication, CreateGithubIssueRequest, DecisionRequest},
    http::{self, HttpState, DEFAULT_PORT, MAX_REQUEST_BYTES},
    providers,
    runtime::{
        credentials::{CredentialStore, OsCredentialStore},
        governance::{Actor, AuthorityScope, ManualCapabilityEdit, MutationRequest, ReviewOutcome},
        meetings::CreateMeetingRequest,
        recovery::RecoveryAction,
    },
};

pub const IPC_PROTOCOL_VERSION: u32 = 1;
const RPC_APPLYING: &str = "APPLYING";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClientOrigin {
    Desktop,
    Mcp,
}

impl ClientOrigin {
    fn label(self) -> &'static str {
        match self {
            Self::Desktop => "DESKTOP",
            Self::Mcp => "MCP",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DaemonRuntimeState {
    Starting,
    Recovering,
    Ready,
    Stopping,
    Error,
}

impl DaemonRuntimeState {
    fn label(self) -> &'static str {
        match self {
            Self::Starting => "STARTING",
            Self::Recovering => "RECOVERING",
            Self::Ready => "READY",
            Self::Stopping => "STOPPING",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonDescriptor {
    pub protocol_version: u32,
    pub daemon_version: String,
    pub project_fingerprint: String,
    pub pid: u32,
    pub endpoint: String,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeResponse {
    pub compatible: bool,
    pub protocol_version: u32,
    pub daemon_version: String,
    pub project_fingerprint: String,
    pub runtime: DaemonRuntimeState,
    pub restart_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcRequest {
    pub protocol_version: u32,
    pub request_id: String,
    pub client_id: String,
    pub project_fingerprint: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcResponse {
    pub request_id: String,
    pub result: Option<Value>,
    pub error: Option<RpcError>,
    pub duplicate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub uncertain: bool,
}

#[derive(Clone)]
pub struct ControlService {
    application: Arc<BataiApplication>,
    events: broadcast::Sender<Value>,
    pulls: Arc<Mutex<HashMap<String, providers::ollama::PullCancellation>>>,
}

impl ControlService {
    pub fn new(application: Arc<BataiApplication>) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            application,
            events,
            pulls: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn application(&self) -> &Arc<BataiApplication> {
        &self.application
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }

    pub fn start_event_forwarder(&self) {
        let mut runtime_events = self.application.runtime().events.subscribe();
        let output = self.events.clone();
        tokio::spawn(async move {
            loop {
                match runtime_events.recv().await {
                    Ok(event) => {
                        let _ = output.send(serde_json::to_value(event).unwrap_or(Value::Null));
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let _ = output.send(json!({"type":"RESYNC_REQUIRED"}));
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    #[cfg(test)]
    pub fn publish_test_event(&self, value: Value) {
        let _ = self.events.send(value);
    }

    pub async fn invoke(
        &self,
        origin: ClientOrigin,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        let runtime = self.application.runtime();
        match method {
            "get_app_snapshot" => value(self.application.snapshot()),
            "batai_read_project_state" => self.application.external_snapshot().map_err(|e| e.to_string()),
            "shutdown_daemon" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                let force = params
                    .get("force")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let snapshot = self.application.snapshot().map_err(|e| e.to_string())?;
                let active_work = snapshot.project.running_tasks > 0
                    || snapshot.project.waiting_agents > 0
                    || snapshot.project.review_tasks > 0
                    || snapshot
                        .governance
                        .provider_approvals
                        .iter()
                        .any(|approval| !approval.status.terminal())
                    || snapshot.governance.recovery_requires_review > 0;
                let active_work = active_work
                    || snapshot.meetings.iter().any(|meeting| {
                        matches!(
                            meeting.status,
                            crate::runtime::meetings::MeetingStatus::Planned
                                | crate::runtime::meetings::MeetingStatus::Ready
                                | crate::runtime::meetings::MeetingStatus::Running
                        )
                    });
                if active_work && !force {
                    return Err("runtime has active or review-required work; retry with explicit force only after user confirmation".into());
                }
                Ok(json!({"accepted":true,"activeWork":active_work}))
            }
            "get_provider_connections" => value::<_, String>(Ok(providers::probe_all())),
            "get_connection_guide" => {
                let id = string_param(&params, "providerId", "provider_id")?;
                value::<_, String>(providers::connection_guide(&id).ok_or_else(|| format!("Unknown provider: {id}")))
            }
            "send_director_message" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(self.application.send_god_message(&string_param(&params, "content", "content")?))
            }
            "get_local_ai_state" => {
                let provider = providers::ollama::OllamaProvider::default();
                let state = provider.installation_state().await;
                let installed = provider.list_models().await.unwrap_or_default();
                let running = provider.running_models().await.unwrap_or_default();
                Ok(json!({"state":state,"installed":installed,"running":running}))
            }
            "get_ollama_model_details" => {
                let model = string_param(&params, "modelId", "model_id")?;
                provider_value(providers::ollama::OllamaProvider::default().show_model(&model).await)
            }
            "remove_ollama_model" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                let model = string_param(&params, "modelId", "model_id")?;
                providers::ollama::OllamaProvider::default().remove_model(&model).await
                    .map(|_| Value::Null).map_err(|error| format!("Ollama model removal failed: {error:?}"))
            }
            "pull_ollama_model" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                let model = string_param(&params, "modelId", "model_id")?;
                self.pull_model(model).await
            }
            "cancel_ollama_pull" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                let model = string_param(&params, "modelId", "model_id")?;
                let pulls = self.pulls.lock().map_err(|_| "download state unavailable".to_string())?;
                Ok(json!(pulls.get(&model).is_some_and(|cancel| { cancel.cancel(); true })))
            }
            "benchmark_local_model" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                self.benchmark_model(string_param(&params, "modelId", "model_id")?).await
            }
            "set_resource_credential" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                self.set_credential(
                    string_param(&params, "resourceId", "resource_id")?,
                    string_param(&params, "secret", "secret")?,
                )
            }
            "disconnect_resource" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                OsCredentialStore.delete(&string_param(&params, "resourceId", "resource_id")?)
                    .map(|_| Value::Null).map_err(|error| error.to_string())
            }
            "update_intelligence_resource" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                let profile = serde_json::from_value(params.get("profile").cloned().unwrap_or(params))
                    .map_err(|error| error.to_string())?;
                runtime.store.upsert_intelligence_resource(&profile).map(|_| Value::Null).map_err(|e| e.to_string())
            }
            "update_economic_policy" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                let policy = serde_json::from_value(params.get("policy").cloned().unwrap_or(params)).map_err(|e| e.to_string())?;
                runtime.store.set_economic_policy(&policy).map(|_| Value::Null).map_err(|e| e.to_string())
            }
            "update_capability_learning_policy" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                let policy = serde_json::from_value(params.get("policy").cloned().unwrap_or(params)).map_err(|e| e.to_string())?;
                runtime.store.set_capability_learning_policy(&policy).map(|_| Value::Null).map_err(|e| e.to_string())
            }
            "add_manual_capability_evidence" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(runtime.governance.record_manual_capability(ManualCapabilityEdit {
                    actor: god_actor(),
                    resource_id: string_param(&params, "resourceId", "resource_id")?,
                    model: optional_string(&params, "model", "model"),
                    dimension: string_param(&params, "dimension", "dimension")?,
                    score: number_param(&params, "score", "score")? as u8,
                    note: optional_string(&params, "note", "note"),
                }))
            }
            "acknowledge_resource_terms" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(runtime.governance.acknowledge_resource_terms(
                    &string_param(&params, "resourceId", "resource_id")?, god_actor()))
            }
            "replay_routing_decision" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                self.replay_route(params)
            }
            "test_resource_connection" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                self.test_connection(string_param(&params, "resourceId", "resource_id")?).await
            }
            "import_github_issue" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(runtime.delivery.import_issue(number_param(&params, "number", "number")?, "god"))
            }
            "create_github_issue_for_task" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                let labels: Vec<String> = serde_json::from_value(params.get("labels").cloned().unwrap_or_else(|| json!([]))).map_err(|e| e.to_string())?;
                value(runtime.delivery.create_issue_for_task(&string_param(&params,"taskId","task_id")?, god_actor(), &labels))
            }
            "prepare_task_delivery" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                let policy = serde_json::from_value(params.get("policy").cloned().ok_or("policy is required")?).map_err(|e| e.to_string())?;
                value(runtime.delivery.prepare_worktree(
                    &string_param(&params,"taskId","task_id")?,
                    &string_param(&params,"agentId","agent_id")?,
                    optional_string(&params,"base","base").as_deref(), policy))
            }
            "commit_task_delivery" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(runtime.delivery.commit(
                    &string_param(&params,"taskId","task_id")?,
                    &string_param(&params,"agentId","agent_id")?,
                    optional_string(&params,"summary","summary").as_deref()))
            }
            "push_task_delivery" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(runtime.delivery.push(&string_param(&params,"taskId","task_id")?, "god"))
            }
            "create_task_pull_request" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                let tests: Vec<String> = serde_json::from_value(params.get("tests").cloned().unwrap_or_else(|| json!([]))).map_err(|e| e.to_string())?;
                value(runtime.delivery.create_pull_request(&string_param(&params,"taskId","task_id")?, god_actor(), &tests))
            }
            "sync_task_github" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(runtime.delivery.sync(&string_param(&params,"taskId","task_id")?))
            }
            "request_task_github_review" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(runtime.delivery.request_github_review(&string_param(&params,"taskId","task_id")?, god_actor(), &string_param(&params,"reviewer","reviewer")?))
            }
            "request_task_merge_approval" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(runtime.delivery.request_merge_approval(&string_param(&params,"taskId","task_id")?, god_actor()))
            }
            "merge_task_pull_request" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(runtime.delivery.merge(&string_param(&params,"taskId","task_id")?, "god"))
            }
            "cleanup_task_worktree" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(runtime.delivery.cleanup_worktree(&string_param(&params,"taskId","task_id")?))
            }
            "close_task_github_issue" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(runtime.delivery.close_linked_issue(&string_param(&params,"taskId","task_id")?, god_actor()))
            }
            "cancel_task" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                runtime.tasks.cancel(&string_param(&params,"taskId","task_id")?, "user").await.map(|_| Value::Null).map_err(|e| e.to_string())
            }
            "mutate_organization" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                let mut request: MutationRequest = serde_json::from_value(params.get("request").cloned().unwrap_or(params)).map_err(|e| e.to_string())?;
                request.actor = god_actor();
                value(runtime.governance.mutate(request))
            }
            "resolve_god_decision" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(runtime.governance.resolve_decision(
                    &string_param(&params,"decisionId","decision_id")?,
                    bool_param(&params,"approve","approve")?, optional_string(&params,"note","note")))
            }
            "resolve_provider_approval" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(runtime.governance.resolve_provider_approval(
                    &string_param(&params,"approvalId","approval_id")?, bool_param(&params,"approve","approve")?))
            }
            "resolve_recovery_operation" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                let action: RecoveryAction = serde_json::from_value(params.get("action").cloned().ok_or("action is required")?).map_err(|e| e.to_string())?;
                value(runtime.governance.resolve_recovery_operation(&string_param(&params,"operationId","operation_id")?, action))
            }
            "record_review_outcome" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                let review: ReviewOutcome = serde_json::from_value(params.get("review").cloned().unwrap_or(params)).map_err(|e| e.to_string())?;
                if runtime.store.get_delivery_checkpoint(&review.task_id).map_err(|e| e.to_string())?.is_some() {
                    value(runtime.delivery.record_internal_review(review).map(|_| ()))
                } else {
                    value(runtime.governance.record_review(review))
                }
            }
            "create_meeting" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(self.application.create_meeting(god_actor(), parse::<CreateMeetingRequest>(params.get("request").cloned().unwrap_or(params))?))
            }
            "get_meeting" => value(self.application.get_meeting(&string_param(&params,"meetingId","meeting_id")?)),
            "list_meetings" => value(self.application.list_meetings()),
            "cancel_meeting" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                value(self.application.cancel_meeting(god_actor(), &string_param(&params,"meetingId","meeting_id")?).await)
            }
            "create_meeting_action_task" => {
                ensure_origin(origin, ClientOrigin::Desktop)?;
                let assigned_to: Vec<String> = serde_json::from_value(params.get("assignedTo").or_else(||params.get("assigned_to")).cloned().unwrap_or_else(||json!([]))).map_err(|e|e.to_string())?;
                value(self.application.create_meeting_action_task(god_actor(), &string_param(&params,"meetingId","meeting_id")?, &string_param(&params,"actionId","action_id")?, assigned_to).await)
            }

            // MCP Director surface. Origin is assigned by the authenticated transport.
            "batai_create_agent" => { ensure_origin(origin, ClientOrigin::Mcp)?; value(self.application.create_agent(parse(params)?)) }
            "batai_assign_task" => { ensure_origin(origin, ClientOrigin::Mcp)?; value(self.application.assign_task(parse(params)?).await) }
            "batai_approve_task" => { ensure_origin(origin, ClientOrigin::Mcp)?; value(self.application.approve_task(&string_param(&params,"task_id","taskId")?).await) }
            "batai_create_worktree" => { ensure_origin(origin, ClientOrigin::Mcp)?; value(self.application.create_worktree(&string_param(&params,"agent_id","agentId")?, &string_param(&params,"task_id","taskId")?, optional_string(&params,"base_ref","baseRef").as_deref())) }
            "batai_update_directives" => { ensure_origin(origin, ClientOrigin::Mcp)?; value(self.application.update_directives(&string_param(&params,"agent_id","agentId")?, &string_param(&params,"content","content")?)) }
            "batai_resume_agent" => { ensure_origin(origin, ClientOrigin::Mcp)?; value(self.application.resume_agent(&string_param(&params,"agent_id","agentId")?).await) }
            "batai_create_github_issue" => { ensure_origin(origin, ClientOrigin::Mcp)?; value(self.application.create_github_issue(parse::<CreateGithubIssueRequest>(params)?)) }
            "batai_read_director_inbox" => { ensure_origin(origin, ClientOrigin::Mcp)?; value(self.application.read_director_inbox(params.get("pending_only").and_then(Value::as_bool).unwrap_or(true))) }
            "batai_acknowledge_god_message" => { ensure_origin(origin, ClientOrigin::Mcp)?; value(self.application.acknowledge_god_message(&string_param(&params,"message_id","messageId")?)) }
            "batai_request_god_decision" => { ensure_origin(origin, ClientOrigin::Mcp)?; value(self.application.request_god_decision(parse::<DecisionRequest>(params)?)) }
            "batai_get_task" => value(self.application.get_task(&string_param(&params,"task_id","taskId")?)),
            "batai_get_resources" => value(runtime.store.list_intelligence_resources()),
            "batai_get_delivery" => value(self.application.get_delivery(&string_param(&params,"task_id","taskId")?)),
            "batai_get_decisions" => value(runtime.governance.snapshot().map(|s| json!({"decisions":s.decisions,"providerApprovals":s.provider_approvals}))),
            "batai_get_recovery_state" => value(runtime.governance.snapshot().map(|s| json!({"operations":s.recovery_operations,"requiresReview":s.recovery_requires_review}))),
            "batai_create_meeting" => { ensure_origin(origin, ClientOrigin::Mcp)?; value(self.application.create_meeting(director_actor(), parse::<CreateMeetingRequest>(params)?)) },
            "batai_get_meeting" => { ensure_origin(origin, ClientOrigin::Mcp)?; value(self.application.get_meeting(&string_param(&params,"meeting_id","meetingId")?)) },
            "batai_list_meetings" => { ensure_origin(origin, ClientOrigin::Mcp)?; value(self.application.list_meetings()) },
            "batai_cancel_meeting" => { ensure_origin(origin, ClientOrigin::Mcp)?; value(self.application.cancel_meeting(director_actor(), &string_param(&params,"meeting_id","meetingId")?).await) },
            other => Err(format!("unknown daemon method: {other}")),
        }
    }

    async fn pull_model(&self, model: String) -> Result<Value, String> {
        let entry = crate::runtime::benchmark::curated_catalog()
            .into_iter()
            .find(|entry| entry.id == model)
            .ok_or_else(|| "Only reviewed Batai catalog models can be downloaded".to_string())?;
        let hardware = crate::runtime::hardware::HardwareProfiler.detect();
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
        self.pulls
            .lock()
            .map_err(|_| "download state unavailable".to_string())?
            .insert(model.clone(), cancellation.clone());
        let events = self.events.clone();
        let result = providers::ollama::OllamaProvider::default().pull_model(&model, &cancellation, |progress| {
            let _ = events.send(json!({"type":"LOCAL_MODEL_PULL_PROGRESS","payload":{"modelId":model,"progress":progress}}));
        }).await.map(|_| Value::Null).map_err(|error| format!("Ollama download failed: {error:?}"));
        self.pulls
            .lock()
            .map_err(|_| "download state unavailable".to_string())?
            .remove(&model);
        result
    }

    async fn benchmark_model(&self, model: String) -> Result<Value, String> {
        let provider = providers::ollama::OllamaProvider::default();
        let installed = provider
            .list_models()
            .await
            .map_err(|e| format!("Ollama unavailable: {e:?}"))?;
        if !installed
            .iter()
            .any(|item| item.name == model || item.model.as_deref() == Some(&model))
        {
            return Err("Model is not installed".into());
        }
        let hardware = crate::runtime::hardware::HardwareProfiler.detect();
        let result =
            crate::runtime::benchmark::run_benchmark(&provider, &model, &hardware.fingerprint)
                .await;
        let store = &self.application.runtime().store;
        store.save_benchmark(&result).map_err(|e| e.to_string())?;
        let mut resource = store
            .list_intelligence_resources()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|resource| resource.id == "ollama-local")
            .unwrap_or_else(|| {
                crate::runtime::economic::native_resource_profile(
                    "ollama-local",
                    "ollama",
                    "Ollama Local",
                    crate::runtime::economic::BillingMode::Local,
                )
            });
        resource.status = "AVAILABLE".into();
        resource.supported_models = vec![model];
        resource.capabilities = result.capabilities.clone();
        resource.capability_evidence = result.evidence.clone();
        store
            .upsert_intelligence_resource(&resource)
            .map_err(|e| e.to_string())?;
        serde_json::to_value(result).map_err(|e| e.to_string())
    }

    fn set_credential(&self, resource_id: String, secret: String) -> Result<Value, String> {
        if !matches!(
            resource_id.as_str(),
            "kimi-personal-membership" | "zai-coding-plan" | "minimax-token-plan"
        ) {
            return Err("Unsupported credential resource".into());
        }
        OsCredentialStore
            .set(&resource_id, &secret)
            .map_err(|e| e.to_string())?;
        if let Some(mut profile) = self
            .application
            .runtime()
            .store
            .list_intelligence_resources()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|profile| profile.id == resource_id)
        {
            profile.status = "AVAILABLE".into();
            self.application
                .runtime()
                .store
                .upsert_intelligence_resource(&profile)
                .map_err(|e| e.to_string())?;
        }
        Ok(Value::Null)
    }

    fn replay_route(&self, params: Value) -> Result<Value, String> {
        let decision_id = string_param(&params, "decisionId", "decision_id")?;
        let policy =
            serde_json::from_value(params.get("policy").cloned().ok_or("policy is required")?)
                .map_err(|e| e.to_string())?;
        let store = &self.application.runtime().store;
        let original = store
            .get_routing_decision(&decision_id)
            .map_err(|e| e.to_string())?
            .ok_or("Routing decision not found")?;
        let resources = if original.resource_snapshot.is_empty() {
            store
                .list_intelligence_resources()
                .map_err(|e| e.to_string())?
        } else {
            original.resource_snapshot.clone()
        };
        let outcomes = store
            .list_task_outcomes(None, 10_000)
            .map_err(|e| e.to_string())?;
        let learning = store
            .capability_learning_policy()
            .map_err(|e| e.to_string())?;
        let replayed = crate::runtime::economic::QuotaAwareEconomicRouter.route_with_evidence(
            &original.task_id,
            &original.requirements,
            &resources,
            &policy,
            &outcomes,
            &learning,
            Utc::now(),
        );
        let actual = outcomes
            .iter()
            .find(|item| item.routing_decision_id.as_deref() == Some(&original.id));
        let billing = |id: Option<&String>| {
            id.and_then(|id| {
                resources
                    .iter()
                    .find(|r| &r.id == id)
                    .map(|r| r.billing_mode)
            })
        };
        value::<_, String>(Ok(crate::runtime::learning::replay_result(
            &original,
            &replayed,
            actual,
            billing(original.selected_resource_id.as_ref()),
            billing(replayed.selected_resource_id.as_ref()),
        )))
    }

    async fn test_connection(&self, resource_id: String) -> Result<Value, String> {
        let runtime = self.application.runtime();
        let profile = runtime
            .store
            .list_intelligence_resources()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|p| p.id == resource_id)
            .ok_or("Resource not found")?;
        if !matches!(
            resource_id.as_str(),
            "kimi-personal-membership" | "zai-coding-plan" | "minimax-token-plan"
        ) {
            return Err("Connection smoke is limited to hosted subscription adapters".into());
        }
        let agent = parse(
            json!({"id":"connection-smoke","name":"Connection smoke","role_template":"Agent","provider":profile.provider,"model":profile.supported_models.first().cloned().unwrap_or_default(),"status":"READY"}),
        )?;
        let task = parse(
            json!({"id":"connection-smoke","created_by":"god","objective":"Reply with exactly BATAI_CONNECTION_OK. Do not call tools or modify files.","assigned_to":["connection-smoke"],"status":"READY"}),
        )?;
        let provider = runtime
            .sessions
            .provider_for(&agent)
            .map_err(|e| e.to_string())?;
        let started = std::time::Instant::now();
        let session = provider
            .create_session(&agent)
            .await
            .map_err(|e| e.to_string())?;
        match provider.send_task(&session.id, &agent, &task).await {
            Ok(_) => Ok(
                json!({"status":"CONNECTED","detail":"Safe low-token inference completed; no capability evidence was created","latencyMs":started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64}),
            ),
            Err(error) => Ok(
                json!({"status":match error { crate::runtime::execution_provider::ProviderFailure::AuthRequired=>"AUTH_FAILED",crate::runtime::execution_provider::ProviderFailure::RateLimited{..}=>"RATE_LIMITED",_=>"ERROR"},"detail":providers::process_supervisor::redact_secrets(&format!("Connection test failed: {error:?}")),"latencyMs":null}),
            ),
        }
    }
}

#[derive(Clone)]
struct DaemonHttpState {
    service: Arc<ControlService>,
    tokens: Arc<TokenSet>,
    fingerprint: Arc<str>,
    runtime_state: Arc<RwLock<DaemonRuntimeState>>,
    clients: Arc<Mutex<HashMap<String, chrono::DateTime<Utc>>>>,
    subscriptions: Arc<tokio::sync::Mutex<HashMap<String, broadcast::Receiver<Value>>>>,
    shutdown: Arc<Notify>,
}

struct TokenSet {
    desktop: String,
    mcp: String,
}

impl DaemonHttpState {
    fn authenticate(&self, headers: &HeaderMap) -> Result<ClientOrigin, StatusCode> {
        let value = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(StatusCode::UNAUTHORIZED)?;
        if value == self.tokens.desktop {
            Ok(ClientOrigin::Desktop)
        } else if value == self.tokens.mcp {
            Ok(ClientOrigin::Mcp)
        } else {
            Err(StatusCode::UNAUTHORIZED)
        }
    }
    fn touch(&self, id: &str) {
        if let Ok(mut clients) = self.clients.lock() {
            clients.insert(id.to_owned(), Utc::now());
        }
    }
}

pub struct DaemonHost {
    descriptor: DaemonDescriptor,
    _service: Arc<ControlService>,
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl DaemonHost {
    pub async fn start(root: PathBuf, preferred_port: Option<u16>) -> Result<Self, String> {
        let root = canonical_root(&root)?;
        let fingerprint = project_fingerprint(&root);
        let application = BataiApplication::open(root.clone(), ApplicationMode::Daemon)
            .map_err(|e| e.to_string())?;
        // The project lock is acquired before credentials or discovery metadata
        // are touched, so concurrent startup losers cannot rotate the winner's
        // client credentials.
        let desktop = token_for(&fingerprint, ClientOrigin::Desktop)?;
        let mcp = token_for(&fingerprint, ClientOrigin::Mcp)?;
        let runtime_state = Arc::new(RwLock::new(DaemonRuntimeState::Recovering));
        let service = Arc::new(ControlService::new(application.clone()));
        service.start_event_forwarder();
        let listener = bind_daemon(preferred_port).await?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let descriptor = DaemonDescriptor {
            protocol_version: IPC_PROTOCOL_VERSION,
            daemon_version: env!("CARGO_PKG_VERSION").into(),
            project_fingerprint: fingerprint.clone(),
            pid: std::process::id(),
            endpoint: format!("http://{address}"),
            started_at: Utc::now().to_rfc3339(),
        };
        let shutdown_notify = Arc::new(Notify::new());
        let internal = DaemonHttpState {
            service: service.clone(),
            tokens: Arc::new(TokenSet { desktop, mcp }),
            fingerprint: fingerprint.clone().into(),
            runtime_state: runtime_state.clone(),
            clients: Arc::new(Mutex::new(HashMap::new())),
            subscriptions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            shutdown: shutdown_notify.clone(),
        };
        let http_token = std::env::var("BATAI_CONTROL_TOKEN").unwrap_or_else(|_| {
            token_for_label(&fingerprint, "http")
                .unwrap_or_else(|_| uuid::Uuid::new_v4().simple().to_string())
        });
        let legacy = std::env::var("BATAI_LEGACY_HTTP_MUTATIONS")
            .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        let health_state = runtime_state.clone();
        let http = HttpState::new(application.clone(), http_token, legacy)
            .with_runtime_status_provider(Arc::new(move || {
                health_state
                    .read()
                    .map(|state| state.label().to_owned())
                    .unwrap_or_else(|_| "ERROR".into())
            }));
        let router = http::router(http).merge(internal_router(internal));
        let (tx, rx) = oneshot::channel();
        let descriptor_path = descriptor_path(&root);
        let application_for_server = application.clone();
        let state_for_server = runtime_state.clone();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    tokio::select! {
                        _ = rx => {}
                        _ = shutdown_notify.notified() => {}
                    }
                })
                .await;
            if let Ok(mut state) = state_for_server.write() {
                *state = DaemonRuntimeState::Stopping;
            }
            let _ = application_for_server.shutdown().await;
            let _ = std::fs::remove_file(descriptor_path);
        });
        if let Err(error) = write_descriptor(&root, &descriptor) {
            let _ = tx.send(());
            let _ = task.await;
            return Err(error);
        }
        if let Err(error) = application.start().await {
            if let Ok(mut state) = runtime_state.write() {
                *state = DaemonRuntimeState::Error;
            }
            let _ = tx.send(());
            let _ = task.await;
            return Err(error.to_string());
        }
        if let Ok(mut state) = runtime_state.write() {
            *state = DaemonRuntimeState::Ready;
        }
        Ok(Self {
            descriptor,
            _service: service,
            shutdown: Some(tx),
            task,
        })
    }
    pub fn descriptor(&self) -> &DaemonDescriptor {
        &self.descriptor
    }
    pub async fn stop(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let _ = self.task.await;
    }
}

fn internal_router(state: DaemonHttpState) -> Router {
    Router::new()
        .route("/internal/handshake", get(handshake))
        .route("/internal/rpc", post(rpc))
        .route("/internal/events", get(next_event))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(state)
}

async fn handshake(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
) -> Result<Json<HandshakeResponse>, StatusCode> {
    let _ = state.authenticate(&headers)?;
    let client = headers
        .get("x-batai-client-id")
        .and_then(|value| value.to_str().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    state.touch(client);
    let mut subscriptions = state.subscriptions.lock().await;
    if !subscriptions.contains_key(client) {
        subscriptions.insert(client.to_owned(), state.service.subscribe());
    }
    drop(subscriptions);
    let requested = headers
        .get("x-batai-protocol")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    Ok(Json(HandshakeResponse {
        compatible: requested == IPC_PROTOCOL_VERSION,
        protocol_version: IPC_PROTOCOL_VERSION,
        daemon_version: env!("CARGO_PKG_VERSION").into(),
        project_fingerprint: state.fingerprint.to_string(),
        runtime: state
            .runtime_state
            .read()
            .map(|state| *state)
            .unwrap_or(DaemonRuntimeState::Error),
        restart_required: requested != IPC_PROTOCOL_VERSION,
    }))
}

async fn rpc(
    State(state): State<DaemonHttpState>,
    headers: HeaderMap,
    Json(request): Json<RpcRequest>,
) -> impl IntoResponse {
    let origin = match state.authenticate(&headers) {
        Ok(value) => value,
        Err(status) => {
            return (
                status,
                Json(error_response(
                    &request.request_id,
                    "UNAUTHORIZED",
                    "invalid daemon credential",
                    false,
                    false,
                )),
            )
                .into_response()
        }
    };
    state.touch(&request.client_id);
    if request.protocol_version != IPC_PROTOCOL_VERSION
        || request.project_fingerprint.as_str() != state.fingerprint.as_ref()
    {
        return (
            StatusCode::CONFLICT,
            Json(error_response(
                &request.request_id,
                "PROTOCOL_MISMATCH",
                "daemon protocol or project binding mismatch",
                false,
                false,
            )),
        )
            .into_response();
    }
    let mutation = is_mutation(&request.method);
    let shutdown_requested = request.method == "shutdown_daemon";
    if mutation
        && state
            .runtime_state
            .read()
            .map(|runtime| *runtime != DaemonRuntimeState::Ready)
            .unwrap_or(true)
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(error_response(
                &request.request_id,
                "RUNTIME_NOT_READY",
                "the project runtime is recovering; retry after it becomes ready",
                true,
                false,
            )),
        )
            .into_response();
    }
    let store = &state.service.application().runtime().store;
    if mutation {
        match store.rpc_mutation_result(&request.request_id) {
            Ok(Some(result)) if rpc_is_applying(&result) => {
                return (StatusCode::CONFLICT,Json(error_response(&request.request_id,"RESPONSE_UNCERTAIN","the original mutation may have completed; inspect current state before retrying with a new request",false,true))).into_response();
            }
            Ok(Some(result)) => {
                return (
                    StatusCode::OK,
                    Json(RpcResponse {
                        request_id: request.request_id,
                        result: Some(result),
                        error: None,
                        duplicate: true,
                    }),
                )
                    .into_response();
            }
            Ok(None) => {}
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_response(
                        &request.request_id,
                        "DEDUPE_READ_FAILED",
                        &error.to_string(),
                        true,
                        false,
                    )),
                )
                    .into_response();
            }
        }
        match store.begin_rpc_mutation(&request.request_id, origin.label(), &request.method) {
            Ok(true) => {}
            Ok(false) => {
                let existing = store
                    .rpc_mutation_result(&request.request_id)
                    .ok()
                    .flatten();
                return match existing {
                    Some(result) if !rpc_is_applying(&result) => (StatusCode::OK,Json(RpcResponse{request_id:request.request_id,result:Some(result),error:None,duplicate:true})).into_response(),
                    _ => (StatusCode::CONFLICT,Json(error_response(&request.request_id,"RESPONSE_UNCERTAIN","a mutation with this request id is already applying or its result is uncertain",true,true))).into_response(),
                };
            }
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_response(
                        &request.request_id,
                        "DEDUPE_CLAIM_FAILED",
                        &error.to_string(),
                        true,
                        false,
                    )),
                )
                    .into_response();
            }
        }
    }
    match state
        .service
        .invoke(origin, &request.method, request.params)
        .await
    {
        Ok(result) => {
            if mutation {
                if let Err(error) = state
                    .service
                    .application()
                    .runtime()
                    .store
                    .save_rpc_mutation_result(
                        &request.request_id,
                        origin.label(),
                        &request.method,
                        &result,
                    )
                {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(error_response(
                            &request.request_id,
                            "DEDUPE_PERSIST_FAILED",
                            &error.to_string(),
                            true,
                            true,
                        )),
                    )
                        .into_response();
                }
            }
            if shutdown_requested {
                let _ = state.service.events.send(json!({"type":"DAEMON_STOPPING"}));
                state.shutdown.notify_waiters();
            }
            (
                StatusCode::OK,
                Json(RpcResponse {
                    request_id: request.request_id,
                    result: Some(result),
                    error: None,
                    duplicate: false,
                }),
            )
                .into_response()
        }
        Err(message) => {
            if mutation {
                let _ = store.delete_rpc_mutation(&request.request_id);
            }
            (
                StatusCode::BAD_REQUEST,
                Json(error_response(
                    &request.request_id,
                    "RPC_FAILED",
                    &providers::process_supervisor::redact_secrets(&message),
                    false,
                    false,
                )),
            )
                .into_response()
        }
    }
}

async fn next_event(State(state): State<DaemonHttpState>, headers: HeaderMap) -> impl IntoResponse {
    if state.authenticate(&headers).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let client = headers
        .get("x-batai-client-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    state.touch(client);
    let mut receiver = state
        .subscriptions
        .lock()
        .await
        .remove(client)
        .unwrap_or_else(|| state.service.subscribe());
    let response = match tokio::time::timeout(Duration::from_secs(20), receiver.recv()).await {
        Ok(Ok(event)) => Json(json!({"event":event,"resync":false})).into_response(),
        Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
            Json(json!({"event":null,"resync":true})).into_response()
        }
        _ => Json(json!({"event":null,"resync":false})).into_response(),
    };
    state
        .subscriptions
        .lock()
        .await
        .insert(client.to_owned(), receiver);
    response
}

#[derive(Clone)]
pub struct DaemonClient {
    root: Arc<PathBuf>,
    descriptor: DaemonDescriptor,
    token: Arc<str>,
    origin: ClientOrigin,
    client_id: Arc<str>,
    client: reqwest::Client,
}

impl DaemonClient {
    pub async fn connect(root: &Path, origin: ClientOrigin) -> Result<Self, String> {
        let root = canonical_root(root)?;
        let descriptor = read_descriptor(&root)?;
        let fingerprint = project_fingerprint(&root);
        if descriptor.project_fingerprint != fingerprint {
            return Err("daemon descriptor belongs to another project".into());
        }
        let token = token_for(&fingerprint, origin)?;
        let client = Self {
            root: Arc::new(root),
            descriptor,
            token: token.into(),
            origin,
            client_id: format!(
                "{}-{}",
                origin.label().to_ascii_lowercase(),
                uuid::Uuid::new_v4()
            )
            .into(),
            client: reqwest::Client::new(),
        };
        let handshake = client.handshake().await?;
        if !handshake.compatible {
            return Err("Batai daemon restart required: protocol version mismatch".into());
        }
        Ok(client)
    }
    pub async fn connect_or_spawn(root: &Path, origin: ClientOrigin) -> Result<Self, String> {
        if let Ok(client) = Self::connect(root, origin).await {
            return Ok(client);
        }
        spawn_daemon(root)?;
        let mut delay = Duration::from_millis(40);
        for _ in 0..30 {
            tokio::time::sleep(delay).await;
            if let Ok(client) = Self::connect(root, origin).await {
                return Ok(client);
            }
            delay = (delay * 2).min(Duration::from_millis(500));
        }
        Err("Batai daemon did not become ready".into())
    }
    pub async fn handshake(&self) -> Result<HandshakeResponse, String> {
        let response = self
            .client
            .get(format!("{}/internal/handshake", self.descriptor.endpoint))
            .bearer_auth(self.token.as_ref())
            .header("x-batai-protocol", IPC_PROTOCOL_VERSION)
            .header("x-batai-client-id", self.client_id.as_ref())
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("daemon handshake failed: {}", response.status()));
        }
        response.json().await.map_err(|e| e.to_string())
    }
    pub async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, String> {
        let value = self.call_value(method, params).await?;
        serde_json::from_value(value).map_err(|e| e.to_string())
    }
    pub async fn call_value(&self, method: &str, params: Value) -> Result<Value, String> {
        self.call_with_request_id(&format!("RPC-{}", uuid::Uuid::new_v4()), method, params)
            .await
    }
    pub async fn call_with_request_id(
        &self,
        request_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        let request = RpcRequest {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id: request_id.into(),
            client_id: self.client_id.to_string(),
            project_fingerprint: self.descriptor.project_fingerprint.clone(),
            method: method.into(),
            params,
        };
        let response = self
            .client
            .post(format!("{}/internal/rpc", self.descriptor.endpoint))
            .bearer_auth(self.token.as_ref())
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                format!(
                    "daemon disconnected before response; mutation outcome may be uncertain: {e}"
                )
            })?;
        let result: RpcResponse = response.json().await.map_err(|e| e.to_string())?;
        match (result.result, result.error) {
            (Some(value), None) => Ok(value),
            (_, Some(error)) => Err(format!(
                "{}: {} [retryable={}, uncertain={}]",
                error.code, error.message, error.retryable, error.uncertain
            )),
            _ => Err("empty daemon response".into()),
        }
    }
    pub async fn next_event(&self) -> Result<(Option<Value>, bool), String> {
        let value: Value = self
            .client
            .get(format!("{}/internal/events", self.descriptor.endpoint))
            .bearer_auth(self.token.as_ref())
            .header("x-batai-client-id", self.client_id.as_ref())
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        Ok((
            value.get("event").filter(|v| !v.is_null()).cloned(),
            value
                .get("resync")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ))
    }
    pub fn endpoint(&self) -> &str {
        &self.descriptor.endpoint
    }
    pub fn origin(&self) -> ClientOrigin {
        self.origin
    }
    pub async fn reconnect(&self) -> Result<Self, String> {
        Self::connect_or_spawn(self.root.as_ref(), self.origin).await
    }
}

pub async fn run_daemon(root: PathBuf, port: Option<u16>) -> Result<(), String> {
    let mut host = DaemonHost::start(root, port).await?;
    eprintln!("Batai daemon ready at {}", host.descriptor().endpoint);
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            if let Some(shutdown) = host.shutdown.take() {
                let _ = shutdown.send(());
            }
            let _ = (&mut host.task).await;
        }
        _ = &mut host.task => {
            host.shutdown.take();
        }
    }
    Ok(())
}

async fn bind_daemon(preferred: Option<u16>) -> Result<tokio::net::TcpListener, String> {
    let port = preferred.unwrap_or(DEFAULT_PORT);
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => Ok(listener),
        Err(_) if preferred.is_none() => tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| e.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

fn spawn_daemon(root: &Path) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let control = executable
        .file_stem()
        .and_then(|v| v.to_str())
        .is_some_and(|v| v.contains("batai-control"));
    let mut command = std::process::Command::new(executable);
    if control {
        command.arg("daemon");
    } else {
        command.arg("--daemon-child");
    }
    command
        .env("BATAI_PROJECT_ROOT", root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to start Batai daemon: {e}"))
}

pub fn project_fingerprint(root: &Path) -> String {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let normalized = if cfg!(windows) {
        canonical.to_string_lossy().to_ascii_lowercase()
    } else {
        canonical.to_string_lossy().into_owned()
    };
    format!("{:x}", Sha256::digest(normalized.as_bytes()))
}
fn canonical_root(root: &Path) -> Result<PathBuf, String> {
    root.canonicalize()
        .map_err(|e| format!("project root unavailable: {e}"))
}
fn descriptor_path(root: &Path) -> PathBuf {
    root.join(".runtime/daemon.json")
}
fn write_descriptor(root: &Path, value: &DaemonDescriptor) -> Result<(), String> {
    let path = descriptor_path(root);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(
        &temp,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}
fn read_descriptor(root: &Path) -> Result<DaemonDescriptor, String> {
    serde_json::from_slice(&std::fs::read(descriptor_path(root)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn token_for(fingerprint: &str, origin: ClientOrigin) -> Result<String, String> {
    token_for_label(fingerprint, &origin.label().to_ascii_lowercase())
}
#[cfg(not(test))]
fn token_for_label(fingerprint: &str, label: &str) -> Result<String, String> {
    let id = format!("daemon-{fingerprint}-{label}");
    match OsCredentialStore.get(&id).map_err(|e| e.to_string())? {
        Some(value) => Ok(value),
        None => {
            let value = uuid::Uuid::new_v4().simple().to_string();
            OsCredentialStore
                .set(&id, &value)
                .map_err(|e| e.to_string())?;
            Ok(value)
        }
    }
}
#[cfg(test)]
fn token_for_label(fingerprint: &str, label: &str) -> Result<String, String> {
    use std::sync::OnceLock;
    static TOKENS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let id = format!("daemon-{fingerprint}-{label}");
    let mut tokens = TOKENS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "test credential store unavailable".to_string())?;
    Ok(tokens
        .entry(id)
        .or_insert_with(|| uuid::Uuid::new_v4().simple().to_string())
        .clone())
}
fn error_response(
    id: &str,
    code: &str,
    message: &str,
    retryable: bool,
    uncertain: bool,
) -> RpcResponse {
    RpcResponse {
        request_id: id.into(),
        result: None,
        error: Some(RpcError {
            code: code.into(),
            message: message.into(),
            retryable,
            uncertain,
        }),
        duplicate: false,
    }
}
fn rpc_is_applying(value: &Value) -> bool {
    value.get("__rpcState").and_then(Value::as_str) == Some(RPC_APPLYING)
}
fn is_mutation(method: &str) -> bool {
    !matches!(
        method,
        "get_app_snapshot"
            | "get_provider_connections"
            | "get_connection_guide"
            | "get_local_ai_state"
            | "get_ollama_model_details"
            | "batai_read_project_state"
            | "batai_read_director_inbox"
            | "batai_get_task"
            | "batai_get_resources"
            | "batai_get_delivery"
            | "batai_get_decisions"
            | "batai_get_recovery_state"
            | "batai_get_meeting"
            | "batai_list_meetings"
            | "get_meeting"
            | "list_meetings"
            | "replay_routing_decision"
    )
}
fn ensure_origin(actual: ClientOrigin, expected: ClientOrigin) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{} transport cannot invoke this operation",
            actual.label()
        ))
    }
}
fn god_actor() -> Actor {
    Actor {
        id: "god".into(),
        scope: AuthorityScope::Project,
    }
}
fn director_actor() -> Actor {
    Actor {
        id: "director".into(),
        scope: AuthorityScope::Project,
    }
}
fn value<T: Serialize, E: std::fmt::Display>(result: Result<T, E>) -> Result<Value, String> {
    serde_json::to_value(result.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
fn provider_value<T: Serialize, E: std::fmt::Debug>(result: Result<T, E>) -> Result<Value, String> {
    value(result.map_err(|e| format!("provider operation failed: {e:?}")))
}
fn parse<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| e.to_string())
}
fn field<'a>(value: &'a Value, a: &str, b: &str) -> Option<&'a Value> {
    value.get(a).or_else(|| value.get(b))
}
fn string_param(value: &Value, a: &str, b: &str) -> Result<String, String> {
    field(value, a, b)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{a} is required"))
}
fn optional_string(value: &Value, a: &str, b: &str) -> Option<String> {
    field(value, a, b)
        .and_then(Value::as_str)
        .map(str::to_owned)
}
fn number_param(value: &Value, a: &str, b: &str) -> Result<u64, String> {
    field(value, a, b)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{a} is required"))
}
fn bool_param(value: &Value, a: &str, b: &str) -> Result<bool, String> {
    field(value, a, b)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{a} is required"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    fn project() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join(".batai/tasks")).unwrap();
        temp
    }

    #[tokio::test]
    async fn recovering_daemon_rejects_mutations_but_answers_handshake() {
        let temp = project();
        let application =
            BataiApplication::open(temp.path().to_path_buf(), ApplicationMode::Daemon).unwrap();
        let service = Arc::new(ControlService::new(application));
        let fingerprint = project_fingerprint(temp.path());
        let state = DaemonHttpState {
            service,
            tokens: Arc::new(TokenSet {
                desktop: "desktop-token".into(),
                mcp: "mcp-token".into(),
            }),
            fingerprint: fingerprint.clone().into(),
            runtime_state: Arc::new(RwLock::new(DaemonRuntimeState::Recovering)),
            clients: Arc::new(Mutex::new(HashMap::new())),
            subscriptions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            shutdown: Arc::new(Notify::new()),
        };
        let request = RpcRequest {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id: "RPC-during-recovery".into(),
            client_id: "desktop-test".into(),
            project_fingerprint: fingerprint,
            method: "send_director_message".into(),
            params: json!({"content":"not yet"}),
        };
        let response = internal_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/rpc")
                    .header(header::AUTHORIZATION, "Bearer desktop-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn daemon_descriptor_handshake_snapshot_and_clean_stop() {
        let temp = project();
        let host = DaemonHost::start(temp.path().to_path_buf(), Some(0))
            .await
            .unwrap();
        let client = DaemonClient::connect(temp.path(), ClientOrigin::Desktop)
            .await
            .unwrap();
        assert!(client.handshake().await.unwrap().compatible);
        let snapshot: Value = client.call("get_app_snapshot", json!({})).await.unwrap();
        assert!(snapshot.get("project").is_some());
        let path = descriptor_path(temp.path());
        assert!(path.exists());
        host.stop().await;
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn two_clients_receive_one_event_without_blocking_each_other() {
        let temp = project();
        let host = DaemonHost::start(temp.path().to_path_buf(), Some(0))
            .await
            .unwrap();
        let a = DaemonClient::connect(temp.path(), ClientOrigin::Desktop)
            .await
            .unwrap();
        let b = DaemonClient::connect(temp.path(), ClientOrigin::Mcp)
            .await
            .unwrap();
        let fa = tokio::spawn(async move { a.next_event().await.unwrap().0 });
        let fb = tokio::spawn(async move { b.next_event().await.unwrap().0 });
        tokio::time::sleep(Duration::from_millis(30)).await;
        host._service
            .publish_test_event(json!({"type":"TEST_FANOUT"}));
        assert!(fa.await.unwrap().is_some());
        assert!(fb.await.unwrap().is_some());
        host.stop().await;
    }

    #[tokio::test]
    async fn lagging_client_gets_resync_without_blocking_event_publisher() {
        let temp = project();
        let host = DaemonHost::start(temp.path().to_path_buf(), Some(0))
            .await
            .unwrap();
        let client = DaemonClient::connect(temp.path(), ClientOrigin::Desktop)
            .await
            .unwrap();
        let waiting = {
            let client = client.clone();
            tokio::spawn(async move { client.next_event().await.unwrap() })
        };
        tokio::time::sleep(Duration::from_millis(30)).await;
        host._service.publish_test_event(json!({"sequence":0}));
        assert!(waiting.await.unwrap().0.is_some());
        tokio::time::sleep(Duration::from_millis(10)).await;
        for sequence in 1..400 {
            host._service
                .publish_test_event(json!({"sequence":sequence}));
        }
        let (_, resync) = client.next_event().await.unwrap();
        assert!(resync);
        host.stop().await;
    }

    #[tokio::test]
    async fn actor_isolation_and_persistent_request_deduplication() {
        let temp = project();
        let host = DaemonHost::start(temp.path().to_path_buf(), Some(0))
            .await
            .unwrap();
        let mcp = DaemonClient::connect(temp.path(), ClientOrigin::Mcp)
            .await
            .unwrap();
        assert!(mcp
            .call_value("send_director_message", json!({"content":"spoof"}))
            .await
            .is_err());
        assert!(mcp
            .call_value("shutdown_daemon", json!({"force":true}))
            .await
            .is_err());
        let desktop = DaemonClient::connect(temp.path(), ClientOrigin::Desktop)
            .await
            .unwrap();
        let id = "RPC-fixed";
        let first = desktop
            .call_with_request_id(id, "send_director_message", json!({"content":"once"}))
            .await
            .unwrap();
        let second = desktop
            .call_with_request_id(id, "send_director_message", json!({"content":"once"}))
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(host.descriptor.protocol_version, IPC_PROTOCOL_VERSION);
        host.stop().await;
    }

    #[tokio::test]
    async fn desktop_can_request_graceful_idle_shutdown() {
        let temp = project();
        let host = DaemonHost::start(temp.path().to_path_buf(), Some(0))
            .await
            .unwrap();
        let client = DaemonClient::connect(temp.path(), ClientOrigin::Desktop)
            .await
            .unwrap();
        let result = client
            .call_value("shutdown_daemon", json!({"force":false}))
            .await
            .unwrap();
        assert_eq!(result["accepted"], true);
        for _ in 0..20 {
            if !descriptor_path(temp.path()).exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(!descriptor_path(temp.path()).exists());
        host.stop().await;
    }

    #[tokio::test]
    async fn version_and_project_mismatch_fail_closed() {
        let temp = project();
        let host = DaemonHost::start(temp.path().to_path_buf(), Some(0))
            .await
            .unwrap();
        let client = DaemonClient::connect(temp.path(), ClientOrigin::Desktop)
            .await
            .unwrap();
        let mut bad = client.clone();
        bad.descriptor.project_fingerprint = "other".into();
        assert!(bad.call_value("get_app_snapshot", json!({})).await.is_err());
        host.stop().await;
    }

    #[tokio::test]
    async fn desktop_mcp_and_http_share_one_runtime_owner() {
        let temp = project();
        let host = DaemonHost::start(temp.path().to_path_buf(), Some(0))
            .await
            .unwrap();
        let desktop = DaemonClient::connect(temp.path(), ClientOrigin::Desktop)
            .await
            .unwrap();
        let mcp = DaemonClient::connect(temp.path(), ClientOrigin::Mcp)
            .await
            .unwrap();

        let desktop_state = desktop
            .call_value("get_app_snapshot", json!({}))
            .await
            .unwrap();
        let mcp_state = mcp
            .call_value("batai_read_project_state", json!({}))
            .await
            .unwrap();
        let http_state: Value = reqwest::get(format!("{}/api/state", host.descriptor.endpoint))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let health: Value = reqwest::get(format!("{}/api/health", host.descriptor.endpoint))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            desktop_state["project"]["name"],
            mcp_state["project"]["name"]
        );
        assert_eq!(
            desktop_state["project"]["name"],
            http_state["project"]["name"]
        );
        assert_eq!(health["runtime"], "READY");
        assert_eq!(health["owner"], "daemon");
        assert!(
            BataiApplication::open(temp.path().to_path_buf(), ApplicationMode::Desktop).is_err()
        );
        host.stop().await;
    }

    #[tokio::test]
    async fn client_disconnect_does_not_stop_project_runtime() {
        let temp = project();
        let host = DaemonHost::start(temp.path().to_path_buf(), Some(0))
            .await
            .unwrap();
        {
            let desktop = DaemonClient::connect(temp.path(), ClientOrigin::Desktop)
                .await
                .unwrap();
            let mcp = DaemonClient::connect(temp.path(), ClientOrigin::Mcp)
                .await
                .unwrap();
            assert!(desktop.handshake().await.unwrap().compatible);
            assert!(mcp.handshake().await.unwrap().compatible);
        }
        let reconnected = DaemonClient::connect(temp.path(), ClientOrigin::Desktop)
            .await
            .unwrap();
        let snapshot = reconnected
            .call_value("get_app_snapshot", json!({}))
            .await
            .unwrap();
        assert!(snapshot.get("project").is_some());
        host.stop().await;
    }

    #[tokio::test]
    async fn concurrent_startup_has_exactly_one_lock_winner() {
        let temp = project();
        let root = temp.path().to_path_buf();
        let attempts = (0..10)
            .map(|_| {
                let root = root.clone();
                tokio::spawn(async move { DaemonHost::start(root, Some(0)).await })
            })
            .collect::<Vec<_>>();
        let mut hosts = Vec::new();
        for attempt in attempts {
            if let Ok(host) = attempt.await.unwrap() {
                hosts.push(host);
            }
        }
        assert_eq!(hosts.len(), 1);
        let client = DaemonClient::connect(temp.path(), ClientOrigin::Desktop)
            .await
            .unwrap();
        assert!(client.handshake().await.unwrap().compatible);
        hosts.pop().unwrap().stop().await;
    }

    #[tokio::test]
    async fn applying_marker_prevents_uncertain_mutation_replay() {
        let temp = project();
        let host = DaemonHost::start(temp.path().to_path_buf(), Some(0))
            .await
            .unwrap();
        host._service
            .application()
            .runtime()
            .store
            .begin_rpc_mutation("RPC-uncertain", "DESKTOP", "send_director_message")
            .unwrap();
        let client = DaemonClient::connect(temp.path(), ClientOrigin::Desktop)
            .await
            .unwrap();
        let error = client
            .call_with_request_id(
                "RPC-uncertain",
                "send_director_message",
                json!({"content":"must not be duplicated"}),
            )
            .await
            .unwrap_err();
        assert!(error.contains("may have completed"));
        assert!(host
            ._service
            .application()
            .read_director_inbox(false)
            .unwrap()
            .is_empty());
        host.stop().await;
    }

    #[tokio::test]
    async fn completed_request_is_deduplicated_after_daemon_restart() {
        let temp = project();
        let host = DaemonHost::start(temp.path().to_path_buf(), Some(0))
            .await
            .unwrap();
        let client = DaemonClient::connect(temp.path(), ClientOrigin::Desktop)
            .await
            .unwrap();
        let first = client
            .call_with_request_id(
                "RPC-across-restart",
                "send_director_message",
                json!({"content":"persist once"}),
            )
            .await
            .unwrap();
        host.stop().await;

        let restarted = DaemonHost::start(temp.path().to_path_buf(), Some(0))
            .await
            .unwrap();
        let client = DaemonClient::connect(temp.path(), ClientOrigin::Desktop)
            .await
            .unwrap();
        let second = client
            .call_with_request_id(
                "RPC-across-restart",
                "send_director_message",
                json!({"content":"persist once"}),
            )
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(
            restarted
                ._service
                .application()
                .read_director_inbox(false)
                .unwrap()
                .len(),
            1
        );
        restarted.stop().await;
    }

    #[tokio::test]
    async fn stale_descriptor_is_replaced_only_by_lock_owner() {
        let temp = project();
        std::fs::create_dir_all(temp.path().join(".runtime")).unwrap();
        std::fs::write(descriptor_path(temp.path()), b"stale descriptor").unwrap();
        let host = DaemonHost::start(temp.path().to_path_buf(), Some(0))
            .await
            .unwrap();
        let descriptor = read_descriptor(temp.path()).unwrap();
        assert_eq!(
            descriptor.project_fingerprint,
            project_fingerprint(temp.path())
        );
        host.stop().await;
    }
}
