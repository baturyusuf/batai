use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{broadcast, oneshot, Mutex as AsyncMutex},
};

use crate::runtime::{
    errors::{Result, RuntimeError},
    execution_provider::{
        ExecutionMode, ExecutionProvider, ProviderApprovalDecision, ProviderApprovalHandler,
        ProviderApprovalRequest, ProviderExecutionResult, ProviderExecutionStatus, ProviderFailure,
        ProviderSession, UsageSnapshot, UsageSource, UsageState,
    },
    types::{Agent, ResourceStatus, Task},
};

type PendingResponse = std::result::Result<Value, String>;
#[derive(Debug, Clone)]
struct ApprovalExecutionContext {
    agent_id: String,
    task_id: String,
    worktree: Option<String>,
    turn_id: Option<String>,
}

struct CodexClient {
    _child: Arc<AsyncMutex<Child>>,
    process_id: Option<u32>,
    stdin: Arc<AsyncMutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<PendingResponse>>>>,
    notifications: broadcast::Sender<Value>,
    next_id: AtomicU64,
}

impl CodexClient {
    async fn spawn(
        executable: &str,
        approval_handler: Option<Arc<dyn ProviderApprovalHandler>>,
        approval_contexts: Arc<Mutex<HashMap<String, ApprovalExecutionContext>>>,
    ) -> std::result::Result<Arc<Self>, ProviderFailure> {
        let mut child = Command::new(executable)
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    ProviderFailure::AppServerUnavailable("Codex CLI is not installed".into())
                }
                _ => ProviderFailure::Execution(e.to_string()),
            })?;
        let stdin = Arc::new(AsyncMutex::new(child.stdin.take().ok_or_else(|| {
            ProviderFailure::AppServerUnavailable("Codex stdin unavailable".into())
        })?));
        let stdout = child.stdout.take().ok_or_else(|| {
            ProviderFailure::AppServerUnavailable("Codex stdout unavailable".into())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ProviderFailure::AppServerUnavailable("Codex stderr unavailable".into())
        })?;
        let pending = Arc::new(Mutex::new(
            HashMap::<u64, oneshot::Sender<PendingResponse>>::new(),
        ));
        let (notifications, _) = broadcast::channel(1024);
        let process_id = child.id();
        let client = Arc::new(Self {
            _child: Arc::new(AsyncMutex::new(child)),
            process_id,
            stdin: stdin.clone(),
            pending: pending.clone(),
            notifications: notifications.clone(),
            next_id: AtomicU64::new(1),
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(_)) = lines.next_line().await {}
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if message.get("method").is_some() && message.get("id").is_some() {
                    let handler = approval_handler.clone();
                    let contexts = approval_contexts.clone();
                    let writer = stdin.clone();
                    let notification_sender = notifications.clone();
                    tokio::spawn(async move {
                        let (approval_id, decision, response) = resolve_server_request(
                            &message,
                            process_id,
                            &contexts,
                            handler.as_deref(),
                        )
                        .await;
                        let result = async {
                            let mut stdin = writer.lock().await;
                            stdin
                                .write_all(format!("{}\n", response).as_bytes())
                                .await?;
                            stdin.flush().await
                        }
                        .await
                        .map_err(|error| error.to_string());
                        if let (Some(handler), Some(approval_id)) = (handler, approval_id) {
                            handler.response_result(&approval_id, result.clone());
                        }
                        let _ = notification_sender.send(json!({
                            "method":"batai/providerApproval/responded",
                            "params":{"request":message,"decision":decision,"sent":result.is_ok()}
                        }));
                    });
                } else if let Some(id) = message.get("id").and_then(Value::as_u64) {
                    if let Ok(mut map) = pending.lock() {
                        if let Some(sender) = map.remove(&id) {
                            let result = message.get("result").cloned().ok_or_else(|| {
                                message
                                    .get("error")
                                    .cloned()
                                    .unwrap_or_else(|| json!({"message":"empty response"}))
                                    .to_string()
                            });
                            let _ = sender.send(result);
                        }
                    }
                } else {
                    let _ = notifications.send(message);
                }
            }
            if let Some(handler) = &approval_handler {
                let sessions = approval_contexts
                    .lock()
                    .map(|contexts| contexts.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                for session_id in sessions {
                    handler.cancel_session(&session_id);
                }
            }
            if let Ok(mut map) = pending.lock() {
                for (_, sender) in map.drain() {
                    let _ = sender.send(Err("Codex App Server exited".into()));
                }
            }
        });
        let init = client.request("initialize", json!({"clientInfo":{"name":"batai","title":"Batai","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":false}})).await?;
        if init.is_null() {
            return Err(ProviderFailure::UnsupportedVersion(
                "Codex initialize returned no capabilities".into(),
            ));
        }
        client.notify("initialized", json!({})).await?;
        Ok(client)
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
    ) -> std::result::Result<Value, ProviderFailure> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| ProviderFailure::Execution("Codex request lock failed".into()))?
            .insert(id, tx);
        let message = json!({"id":id,"method":method,"params":params});
        if let Err(error) = self.write(&message).await {
            if let Ok(mut map) = self.pending.lock() {
                map.remove(&id);
            }
            return Err(error);
        }
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(message))) => Err(classify_protocol_error(&message)),
            Ok(Err(_)) => Err(ProviderFailure::ProcessCrash {
                message: "Codex response channel closed".into(),
            }),
            Err(_) => {
                if let Ok(mut map) = self.pending.lock() {
                    map.remove(&id);
                }
                Err(ProviderFailure::Timeout)
            }
        }
    }
    async fn notify(
        &self,
        method: &str,
        params: Value,
    ) -> std::result::Result<(), ProviderFailure> {
        self.write(&json!({"method":method,"params":params})).await
    }
    async fn write(&self, value: &Value) -> std::result::Result<(), ProviderFailure> {
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(format!("{}\n", value).as_bytes())
            .await
            .map_err(|e| ProviderFailure::ProcessCrash {
                message: e.to_string(),
            })?;
        stdin
            .flush()
            .await
            .map_err(|e| ProviderFailure::ProcessCrash {
                message: e.to_string(),
            })
    }
    fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.notifications.subscribe()
    }
}

fn server_request_response(request: &Value, decision: ProviderApprovalDecision) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let protocol_decision = match decision {
        ProviderApprovalDecision::AllowOnce => "accept",
        ProviderApprovalDecision::Deny => "decline",
        ProviderApprovalDecision::Cancel => "cancel",
    };
    json!({"id":id,"result":{"decision":protocol_decision}})
}

#[cfg(test)]
fn safe_server_request_response(request: &Value) -> Value {
    server_request_response(request, ProviderApprovalDecision::Cancel)
}

async fn resolve_server_request(
    message: &Value,
    process_id: Option<u32>,
    contexts: &Arc<Mutex<HashMap<String, ApprovalExecutionContext>>>,
    handler: Option<&dyn ProviderApprovalHandler>,
) -> (Option<String>, ProviderApprovalDecision, Value) {
    let directive = match (
        handler,
        parse_approval_request(message, process_id, contexts),
    ) {
        (Some(handler), Ok(request)) => Some(handler.handle(request).await),
        _ => None,
    };
    let decision = directive
        .as_ref()
        .map_or(ProviderApprovalDecision::Deny, |value| value.decision);
    (
        directive.map(|value| value.approval_id),
        decision,
        server_request_response(message, decision),
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfficialApprovalParams {
    thread_id: String,
    turn_id: String,
    item_id: String,
    #[serde(default)]
    approval_id: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    grant_root: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

fn parse_approval_request(
    request: &Value,
    process_id: Option<u32>,
    contexts: &Arc<Mutex<HashMap<String, ApprovalExecutionContext>>>,
) -> std::result::Result<ProviderApprovalRequest, String> {
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| "approval method missing".to_owned())?;
    if !matches!(
        method,
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval"
    ) {
        return Err(format!("unsupported server request: {method}"));
    }
    let params: OfficialApprovalParams = serde_json::from_value(
        request
            .get("params")
            .cloned()
            .ok_or_else(|| "approval params missing".to_owned())?,
    )
    .map_err(|error| format!("invalid official approval params: {error}"))?;
    let context = contexts
        .lock()
        .map_err(|_| "approval context lock failed".to_owned())?
        .get(&params.thread_id)
        .cloned()
        .ok_or_else(|| "approval is not bound to an active Batai task".to_owned())?;
    if context
        .turn_id
        .as_deref()
        .is_some_and(|turn_id| turn_id != params.turn_id)
    {
        return Err("approval turn does not match the active turn".into());
    }
    let rpc_id = request
        .get("id")
        .ok_or_else(|| "approval JSON-RPC id missing".to_owned())?
        .to_string();
    let request_id = params.approval_id.as_ref().map_or_else(
        || format!("{rpc_id}:{}", params.item_id),
        |approval_id| format!("{rpc_id}:{approval_id}"),
    );
    let requested_target = params.grant_root.clone().or(params.cwd.clone());
    let requested_operation = params.command.clone().unwrap_or_else(|| method.to_owned());
    let risk = if method == "item/fileChange/requestApproval" {
        "FILESYSTEM_CHANGE"
    } else {
        "COMMAND_EXECUTION"
    };
    Ok(ProviderApprovalRequest {
        provider: "codex".into(),
        process_id,
        session_id: params.thread_id.clone(),
        thread_id: params.thread_id,
        turn_id: params.turn_id,
        request_id,
        agent_id: context.agent_id,
        task_id: context.task_id,
        worktree: context.worktree,
        requested_operation,
        requested_target,
        risk: risk.into(),
        detail: json!({
            "method":method,
            "itemId":params.item_id,
            "reason":params.reason,
            "command":params.command,
            "cwd":params.cwd,
            "grantRoot":params.grant_root,
        }),
    })
}

fn classify_protocol_error(message: &str) -> ProviderFailure {
    let lower = message.to_ascii_lowercase();
    if lower.contains("auth") || lower.contains("login") {
        ProviderFailure::AuthRequired
    } else if lower.contains("rate") && lower.contains("limit") {
        ProviderFailure::RateLimited { reset_at: None }
    } else {
        ProviderFailure::Execution(message.into())
    }
}

#[derive(Clone)]
pub struct CodexAppServerProvider {
    executable: String,
    client: Arc<AsyncMutex<Option<Arc<CodexClient>>>>,
    active_turns: Arc<Mutex<HashMap<String, String>>>,
    approval_handler: Option<Arc<dyn ProviderApprovalHandler>>,
    approval_contexts: Arc<Mutex<HashMap<String, ApprovalExecutionContext>>>,
}
impl Default for CodexAppServerProvider {
    fn default() -> Self {
        Self::new("codex")
    }
}
impl CodexAppServerProvider {
    pub fn new(executable: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
            client: Default::default(),
            active_turns: Default::default(),
            approval_handler: None,
            approval_contexts: Default::default(),
        }
    }
    pub fn with_approval_handler(mut self, handler: Arc<dyn ProviderApprovalHandler>) -> Self {
        self.approval_handler = Some(handler);
        self
    }
    async fn client(&self) -> std::result::Result<Arc<CodexClient>, ProviderFailure> {
        let mut guard = self.client.lock().await;
        if let Some(client) = guard.as_ref() {
            return Ok(client.clone());
        }
        let client = CodexClient::spawn(
            &self.executable,
            self.approval_handler.clone(),
            self.approval_contexts.clone(),
        )
        .await?;
        *guard = Some(client.clone());
        Ok(client)
    }
    fn cwd(agent: &Agent) -> Option<String> {
        agent
            .worktree
            .as_ref()
            .map(|path| PathBuf::from(path).to_string_lossy().into_owned())
    }
    fn model(agent: &Agent) -> Value {
        if agent.model.trim().is_empty() || agent.model.eq_ignore_ascii_case("auto") {
            Value::Null
        } else {
            Value::String(agent.model.clone())
        }
    }
    async fn invalidate_client_after(&self, error: &ProviderFailure) {
        if error.leaves_execution_unknown()
            || matches!(error, ProviderFailure::AppServerUnavailable(_))
        {
            *self.client.lock().await = None;
        }
    }

    fn approval_policy(&self) -> &'static str {
        if self.approval_handler.is_some() {
            "on-request"
        } else {
            "never"
        }
    }
}

#[async_trait]
impl ExecutionProvider for CodexAppServerProvider {
    fn name(&self) -> &str {
        "codex"
    }
    async fn create_session(&self, agent: &Agent) -> Result<ProviderSession> {
        let client = self
            .client()
            .await
            .map_err(|e| RuntimeError::Provider(format!("{e:?}")))?;
        let result = client.request("thread/start", json!({"model":Self::model(agent),"cwd":Self::cwd(agent),"approvalPolicy":self.approval_policy(),"sandbox":"workspace-write","serviceName":"batai"})).await
            .map_err(|e| RuntimeError::Provider(format!("{e:?}")))?;
        let id = result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                RuntimeError::Provider("Codex thread/start response omitted thread.id".into())
            })?
            .to_owned();
        Ok(ProviderSession {
            id,
            metadata: json!({"transport":"app-server","resumable":true}),
        })
    }
    async fn resume_session(&self, session_id: &str, agent: &Agent) -> Result<ProviderSession> {
        let client = self
            .client()
            .await
            .map_err(|e| RuntimeError::Provider(format!("{e:?}")))?;
        client.request("thread/resume", json!({"threadId":session_id,"model":Self::model(agent),"cwd":Self::cwd(agent),"approvalPolicy":self.approval_policy(),"sandbox":"workspace-write"})).await
            .map_err(|e| RuntimeError::Provider(format!("{e:?}")))?;
        Ok(ProviderSession {
            id: session_id.into(),
            metadata: json!({"transport":"app-server","resumed":true}),
        })
    }
    async fn send_task(
        &self,
        session_id: &str,
        agent: &Agent,
        task: &Task,
    ) -> std::result::Result<ProviderExecutionResult, ProviderFailure> {
        let started_at = chrono::Utc::now().to_rfc3339();
        let client = self.client().await?;
        let mut notifications = client.subscribe();
        self.approval_contexts
            .lock()
            .map_err(|_| ProviderFailure::Execution("Codex approval context lock failed".into()))?
            .insert(
                session_id.into(),
                ApprovalExecutionContext {
                    agent_id: agent.id.clone(),
                    task_id: task.id.clone(),
                    worktree: agent.worktree.clone(),
                    turn_id: None,
                },
            );
        let started = match client
            .request(
                "turn/start",
                json!({"threadId":session_id,"input":[{"type":"text","text":task.objective}]}),
            )
            .await
        {
            Ok(value) => value,
            Err(error) => {
                if let Ok(mut contexts) = self.approval_contexts.lock() {
                    contexts.remove(session_id);
                }
                self.invalidate_client_after(&error).await;
                return Err(error);
            }
        };
        let turn_id = match started.pointer("/turn/id").and_then(Value::as_str) {
            Some(turn_id) => turn_id.to_owned(),
            None => {
                if let Ok(mut contexts) = self.approval_contexts.lock() {
                    contexts.remove(session_id);
                }
                return Err(ProviderFailure::MalformedResponse(
                    "turn/start omitted turn.id".into(),
                ));
            }
        };
        if let Ok(mut contexts) = self.approval_contexts.lock() {
            if let Some(context) = contexts.get_mut(session_id) {
                context.turn_id = Some(turn_id.clone());
            }
        }
        self.active_turns
            .lock()
            .map_err(|_| ProviderFailure::Execution("Codex active turn lock failed".into()))?
            .insert(session_id.into(), turn_id.clone());
        let mut summary = String::new();
        let mut usage = UsageSnapshot {
            source: UsageSource::Subscription,
            ..UsageSnapshot::default()
        };
        let completed_result = tokio::time::timeout(Duration::from_secs(60 * 60), async {
            loop {
                let message =
                    notifications
                        .recv()
                        .await
                        .map_err(|e| ProviderFailure::ProcessCrash {
                            message: e.to_string(),
                        })?;
                let method = message
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let params = message.get("params").cloned().unwrap_or(Value::Null);
                if params
                    .get("threadId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id != session_id)
                {
                    continue;
                }
                if method == "item/agentMessage/delta" {
                    if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                        summary.push_str(delta);
                    }
                }
                if method == "item/completed"
                    && params.pointer("/item/type").and_then(Value::as_str) == Some("agentMessage")
                {
                    if let Some(text) = params.pointer("/item/text").and_then(Value::as_str) {
                        summary = text.into();
                    }
                }
                if method == "thread/tokenUsage/updated" {
                    usage = parse_token_usage(&params);
                }
                if method == "turn/completed"
                    && params.pointer("/turn/id").and_then(Value::as_str) == Some(turn_id.as_str())
                {
                    return Ok(params);
                }
            }
        })
        .await
        .map_err(|_| ProviderFailure::Timeout);
        self.active_turns
            .lock()
            .map_err(|_| ProviderFailure::Execution("Codex active turn lock failed".into()))?
            .remove(session_id);
        if let Ok(mut contexts) = self.approval_contexts.lock() {
            contexts.remove(session_id);
        }
        let completed = match completed_result {
            Ok(Ok(value)) => value,
            Ok(Err(error)) | Err(error) => {
                self.invalidate_client_after(&error).await;
                return Err(error);
            }
        };
        if let Ok(rate_limits) = client.request("account/rateLimits/read", json!({})).await {
            usage.used_percent = rate_limits
                .pointer("/rateLimits/primary/usedPercent")
                .and_then(Value::as_f64);
            usage.reset_at = rate_limits
                .pointer("/rateLimits/primary/resetsAt")
                .and_then(Value::as_i64)
                .and_then(unix_to_rfc3339);
            usage.details = json!({"tokens":usage.details,"rate_limits":rate_limits});
        }
        let status_text = completed
            .pointer("/turn/status")
            .and_then(Value::as_str)
            .unwrap_or("completed");
        let status = match status_text {
            "completed" => ProviderExecutionStatus::Completed,
            "interrupted" => ProviderExecutionStatus::Interrupted,
            _ => ProviderExecutionStatus::Failed,
        };
        if summary.is_empty() {
            summary = format!("Codex turn {status_text}");
        }
        Ok(ProviderExecutionResult {
            provider: self.name().into(),
            model: agent.model.clone(),
            agent_id: agent.id.clone(),
            task_id: task.id.clone(),
            session_id: session_id.into(),
            turn_id: Some(turn_id),
            status,
            summary,
            artifacts: vec![],
            changed_files: vec![],
            usage,
            started_at,
            completed_at: chrono::Utc::now().to_rfc3339(),
            execution_mode: ExecutionMode::DirectMutation,
            provider_metadata: json!({"turn":completed.get("turn"),"process_pid":client.process_id}),
            session_metadata: json!({"transport":"app-server","resumable":true}),
        })
    }
    async fn cancel_turn(&self, session_id: &str) -> Result<bool> {
        let turn_id = self
            .active_turns
            .lock()
            .map_err(|_| RuntimeError::Lock("Codex active turns"))?
            .get(session_id)
            .cloned();
        let Some(turn_id) = turn_id else {
            return Ok(false);
        };
        let client = self
            .client()
            .await
            .map_err(|e| RuntimeError::Provider(format!("{e:?}")))?;
        if let Some(handler) = &self.approval_handler {
            handler.cancel_session(session_id);
        }
        client
            .request(
                "turn/interrupt",
                json!({"threadId":session_id,"turnId":turn_id}),
            )
            .await
            .map_err(|e| RuntimeError::Provider(format!("{e:?}")))?;
        Ok(true)
    }
    async fn get_usage_state(&self, _agent: &Agent) -> Result<UsageState> {
        let client = self
            .client()
            .await
            .map_err(|e| RuntimeError::Provider(format!("{e:?}")))?;
        let value = client
            .request("account/rateLimits/read", json!({}))
            .await
            .map_err(|e| RuntimeError::Provider(format!("{e:?}")))?;
        let used = value
            .pointer("/rateLimits/primary/usedPercent")
            .and_then(Value::as_f64);
        let reset_at = value
            .pointer("/rateLimits/primary/resetsAt")
            .and_then(Value::as_i64)
            .and_then(unix_to_rfc3339);
        let status = if used.is_some_and(|v| v >= 100.0) {
            ResourceStatus::RateLimited
        } else if used.is_some_and(|v| v >= 90.0) {
            ResourceStatus::Low
        } else {
            ResourceStatus::Available
        };
        Ok(UsageState {
            status,
            reset_at,
            details: value,
        })
    }
}

fn unix_to_rfc3339(timestamp: i64) -> Option<String> {
    use chrono::TimeZone;
    chrono::Utc
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|v| v.to_rfc3339())
}
fn parse_token_usage(value: &Value) -> UsageSnapshot {
    let total = value
        .get("tokenUsage")
        .and_then(|v| v.get("total"))
        .unwrap_or(value);
    UsageSnapshot {
        source: UsageSource::Subscription,
        input_tokens: total.get("inputTokens").and_then(Value::as_u64),
        cached_input_tokens: total.get("cachedInputTokens").and_then(Value::as_u64),
        output_tokens: total.get("outputTokens").and_then(Value::as_u64),
        details: value.clone(),
        ..UsageSnapshot::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    struct FakeApprovalHandler {
        receiver: AsyncMutex<Option<oneshot::Receiver<ProviderApprovalDecision>>>,
    }

    #[async_trait]
    impl ProviderApprovalHandler for FakeApprovalHandler {
        async fn handle(
            &self,
            _request: ProviderApprovalRequest,
        ) -> crate::runtime::execution_provider::ProviderApprovalDirective {
            let receiver = self.receiver.lock().await.take().expect("one request");
            crate::runtime::execution_provider::ProviderApprovalDirective {
                approval_id: "APR-FAKE".into(),
                decision: receiver.await.expect("decision"),
            }
        }

        fn response_result(&self, _approval_id: &str, _result: std::result::Result<(), String>) {}
        fn cancel_session(&self, _session_id: &str) {}
    }
    #[test]
    fn approval_requests_are_never_auto_accepted() {
        let response = safe_server_request_response(
            &json!({"id":4,"method":"item/commandExecution/requestApproval"}),
        );
        assert_eq!(
            response.pointer("/result/decision").and_then(Value::as_str),
            Some("cancel")
        );
        assert!(!response.to_string().contains("accept"));
    }

    #[tokio::test]
    async fn official_approval_request_pauses_and_resumes_the_same_json_rpc_request() {
        let contexts = Arc::new(Mutex::new(HashMap::from([(
            "thread-1".into(),
            ApprovalExecutionContext {
                agent_id: "agent-a".into(),
                task_id: "TASK-1".into(),
                worktree: Some("C:\\worktree".into()),
                turn_id: Some("turn-1".into()),
            },
        )])));
        let (send, receive) = oneshot::channel();
        let handler = Arc::new(FakeApprovalHandler {
            receiver: AsyncMutex::new(Some(receive)),
        });
        let message = json!({
            "id":99,
            "method":"item/commandExecution/requestApproval",
            "params":{
                "threadId":"thread-1","turnId":"turn-1","itemId":"item-1",
                "startedAtMs":1,"command":"cargo test","cwd":"C:\\worktree"
            }
        });
        let waiting = tokio::spawn({
            let contexts = contexts.clone();
            let handler = handler.clone();
            async move {
                resolve_server_request(&message, Some(7), &contexts, Some(handler.as_ref())).await
            }
        });
        tokio::task::yield_now().await;
        assert!(
            !waiting.is_finished(),
            "request must remain paused without a decision"
        );
        send.send(ProviderApprovalDecision::AllowOnce)
            .expect("allow");
        let (approval_id, decision, response) = waiting.await.expect("response");
        assert_eq!(approval_id.as_deref(), Some("APR-FAKE"));
        assert_eq!(decision, ProviderApprovalDecision::AllowOnce);
        assert_eq!(response["id"], 99);
        assert_eq!(response["result"]["decision"], "accept");
    }
    #[test]
    fn token_usage_is_normalized() {
        let usage = parse_token_usage(
            &json!({"tokenUsage":{"total":{"inputTokens":10,"cachedInputTokens":3,"outputTokens":4}}}),
        );
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.cached_input_tokens, Some(3));
        assert_eq!(usage.output_tokens, Some(4));
    }

    #[tokio::test]
    async fn authenticated_app_server_smoke_is_explicitly_opt_in() {
        if std::env::var("BATAI_REAL_PROVIDER_SMOKE").as_deref() != Ok("codex") {
            return;
        }
        let provider = CodexAppServerProvider::default();
        let agent: Agent = serde_json::from_value(json!({
            "id":"smoke","name":"Smoke","provider":"codex","model":"auto",
            "worktree":std::env::current_dir().expect("current directory")
        }))
        .expect("agent");
        let session = provider
            .create_session(&agent)
            .await
            .expect("authenticated thread/start");
        assert!(!session.id.is_empty());
        let usage = provider
            .get_usage_state(&agent)
            .await
            .expect("rate limit snapshot");
        assert!(matches!(
            usage.status,
            ResourceStatus::Available | ResourceStatus::Low | ResourceStatus::RateLimited
        ));
    }
}
