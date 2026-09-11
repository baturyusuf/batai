use super::{
    errors::{Result, RuntimeError},
    types::{Agent, ResourceStatus, Task},
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderSession {
    pub id: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    DirectMutation,
    Patch,
    ActionManifest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderExecutionStatus {
    Completed,
    Interrupted,
    Failed,
    UnknownAfterCrash,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    Subscription,
    Api,
    Local,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub source: UsageSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(default)]
    pub details: Value,
}
impl Default for UsageSnapshot {
    fn default() -> Self {
        Self {
            source: UsageSource::Unknown,
            input_tokens: None,
            cached_input_tokens: None,
            output_tokens: None,
            used_percent: None,
            reset_at: None,
            cost: None,
            currency: None,
            details: Value::Null,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderExecutionResult {
    pub provider: String,
    pub model: String,
    pub agent_id: String,
    pub task_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub status: ProviderExecutionStatus,
    pub summary: String,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub changed_files: Vec<String>,
    pub usage: UsageSnapshot,
    pub started_at: String,
    pub completed_at: String,
    pub execution_mode: ExecutionMode,
    #[serde(default)]
    pub provider_metadata: Value,
    #[serde(default)]
    pub session_metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderApprovalRequest {
    pub provider: String,
    pub process_id: Option<u32>,
    pub session_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub request_id: String,
    pub agent_id: String,
    pub task_id: String,
    pub worktree: Option<String>,
    pub requested_operation: String,
    pub requested_target: Option<String>,
    pub risk: String,
    pub detail: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProviderApprovalDecision {
    AllowOnce,
    Deny,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderApprovalDirective {
    pub approval_id: String,
    pub decision: ProviderApprovalDecision,
}

#[async_trait]
pub trait ProviderApprovalHandler: Send + Sync {
    async fn handle(&self, request: ProviderApprovalRequest) -> ProviderApprovalDirective;
    fn response_result(&self, approval_id: &str, result: std::result::Result<(), String>);
    fn cancel_session(&self, session_id: &str);
}
impl ProviderExecutionResult {
    pub fn into_value(self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageState {
    pub status: ResourceStatus,
    pub reset_at: Option<String>,
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderFailure {
    RateLimited { reset_at: Option<String> },
    AuthRequired,
    Offline,
    Cancelled,
    Timeout,
    ProcessCrash { message: String },
    AppServerUnavailable(String),
    UnsupportedVersion(String),
    MalformedResponse(String),
    Execution(String),
}
impl ProviderFailure {
    pub fn leaves_execution_unknown(&self) -> bool {
        matches!(self, Self::Timeout | Self::ProcessCrash { .. })
    }
}

#[async_trait]
pub trait ExecutionProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn create_session(&self, agent: &Agent) -> Result<ProviderSession>;
    async fn resume_session(&self, session_id: &str, agent: &Agent) -> Result<ProviderSession>;
    async fn restore_session(
        &self,
        session_id: &str,
        _metadata: &Value,
        agent: &Agent,
    ) -> Result<ProviderSession> {
        self.resume_session(session_id, agent).await
    }
    async fn send_task(
        &self,
        session_id: &str,
        agent: &Agent,
        task: &Task,
    ) -> std::result::Result<ProviderExecutionResult, ProviderFailure>;
    async fn cancel_turn(&self, _session_id: &str) -> Result<bool> {
        Ok(false)
    }
    async fn get_usage_state(&self, _agent: &Agent) -> Result<UsageState> {
        Ok(UsageState {
            status: ResourceStatus::Unknown,
            reset_at: None,
            details: Value::Null,
        })
    }
    async fn close_session(&self, _session_id: &str) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum MockOutcome {
    Success,
    Delay(Duration),
    Failure(String),
    RateLimited(Option<String>),
    AuthRequired,
    Crash(String),
    Cancelled,
}

#[derive(Clone, Default)]
pub struct MockProvider {
    outcomes: Arc<Mutex<HashMap<String, VecDeque<MockOutcome>>>>,
    usage: Arc<Mutex<HashMap<String, UsageState>>>,
    calls: Arc<Mutex<Vec<(String, String)>>>,
    resumed: Arc<Mutex<Vec<String>>>,
}
impl MockProvider {
    pub fn push_outcome(&self, agent: &str, outcome: MockOutcome) {
        self.outcomes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(agent.into())
            .or_default()
            .push_back(outcome);
    }
    pub fn set_usage(&self, agent: &str, state: UsageState) {
        self.usage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(agent.into(), state);
    }
    pub fn calls_for(&self, agent: &str) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|(id, _)| id == agent)
            .count()
    }
    pub fn total_calls(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
    pub fn resumed_count(&self) -> usize {
        self.resumed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

#[async_trait]
impl ExecutionProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }
    async fn create_session(&self, agent: &Agent) -> Result<ProviderSession> {
        Ok(ProviderSession {
            id: format!("SESSION-{}-{}", agent.id, uuid::Uuid::new_v4()),
            metadata: serde_json::json!({"mock":true}),
        })
    }
    async fn resume_session(&self, session_id: &str, _agent: &Agent) -> Result<ProviderSession> {
        self.resumed
            .lock()
            .map_err(|_| RuntimeError::Lock("mock resumed"))?
            .push(session_id.into());
        Ok(ProviderSession {
            id: session_id.into(),
            metadata: serde_json::json!({"resumed":true}),
        })
    }
    async fn send_task(
        &self,
        session_id: &str,
        agent: &Agent,
        task: &Task,
    ) -> std::result::Result<ProviderExecutionResult, ProviderFailure> {
        let started_at = chrono::Utc::now().to_rfc3339();
        self.calls
            .lock()
            .map_err(|_| ProviderFailure::Execution("mock lock".into()))?
            .push((agent.id.clone(), task.id.clone()));
        let outcome = self
            .outcomes
            .lock()
            .map_err(|_| ProviderFailure::Execution("mock lock".into()))?
            .get_mut(&agent.id)
            .and_then(VecDeque::pop_front)
            .unwrap_or(MockOutcome::Success);
        match outcome {
            MockOutcome::Success => {}
            MockOutcome::Delay(delay) => tokio::time::sleep(delay).await,
            MockOutcome::Failure(message) => return Err(ProviderFailure::Execution(message)),
            MockOutcome::RateLimited(reset_at) => {
                return Err(ProviderFailure::RateLimited { reset_at })
            }
            MockOutcome::AuthRequired => return Err(ProviderFailure::AuthRequired),
            MockOutcome::Crash(message) => return Err(ProviderFailure::ProcessCrash { message }),
            MockOutcome::Cancelled => return Err(ProviderFailure::Cancelled),
        }
        Ok(ProviderExecutionResult {
            provider: self.name().into(),
            model: agent.model.clone(),
            agent_id: agent.id.clone(),
            task_id: task.id.clone(),
            session_id: session_id.into(),
            turn_id: Some(format!("TURN-{}", uuid::Uuid::new_v4())),
            status: ProviderExecutionStatus::Completed,
            summary: format!("Mock agent {} completed: {}", agent.id, task.objective),
            artifacts: vec![],
            changed_files: vec![],
            usage: UsageSnapshot::default(),
            started_at,
            completed_at: chrono::Utc::now().to_rfc3339(),
            execution_mode: ExecutionMode::DirectMutation,
            provider_metadata: serde_json::json!({"mock":true}),
            session_metadata: serde_json::json!({"mock":true}),
        })
    }
    async fn get_usage_state(&self, agent: &Agent) -> Result<UsageState> {
        Ok(self
            .usage
            .lock()
            .map_err(|_| RuntimeError::Lock("mock usage"))?
            .get(&agent.id)
            .cloned()
            .unwrap_or(UsageState {
                status: ResourceStatus::Available,
                reset_at: None,
                details: Value::Null,
            }))
    }
}

pub fn provider_failure(error: ProviderFailure, agent_id: &str) -> RuntimeError {
    match error {
        ProviderFailure::RateLimited { reset_at } => RuntimeError::ResourceUnavailable {
            agent_id: agent_id.into(),
            status: "RATE_LIMITED".into(),
            reset_at,
        },
        ProviderFailure::AuthRequired => RuntimeError::ResourceUnavailable {
            agent_id: agent_id.into(),
            status: "AUTH_REQUIRED".into(),
            reset_at: None,
        },
        ProviderFailure::Offline => RuntimeError::ResourceUnavailable {
            agent_id: agent_id.into(),
            status: "OFFLINE".into(),
            reset_at: None,
        },
        ProviderFailure::Cancelled => RuntimeError::Provider("provider turn cancelled".into()),
        ProviderFailure::Timeout => RuntimeError::Provider("provider turn timed out".into()),
        ProviderFailure::ProcessCrash { message }
        | ProviderFailure::AppServerUnavailable(message)
        | ProviderFailure::UnsupportedVersion(message)
        | ProviderFailure::MalformedResponse(message)
        | ProviderFailure::Execution(message) => RuntimeError::Provider(message),
    }
}
