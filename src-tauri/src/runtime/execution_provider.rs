use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use serde_json::Value;

use super::{
    errors::{Result, RuntimeError},
    types::{Agent, ResourceStatus, Task},
};

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderSession {
    pub id: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageState {
    pub status: ResourceStatus,
    pub reset_at: Option<String>,
    pub details: Value,
}

#[derive(Debug, Clone)]
pub enum ProviderFailure {
    RateLimited { reset_at: Option<String> },
    AuthRequired,
    Offline,
    Execution(String),
}

#[async_trait]
pub trait ExecutionProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn create_session(&self, agent: &Agent) -> Result<ProviderSession>;
    async fn resume_session(&self, session_id: &str, agent: &Agent) -> Result<ProviderSession>;
    async fn send_task(
        &self,
        session_id: &str,
        agent: &Agent,
        task: &Task,
    ) -> std::result::Result<Value, ProviderFailure>;
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
    ) -> std::result::Result<Value, ProviderFailure> {
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
        }
        Ok(
            serde_json::json!({"agent_id":agent.id,"task_id":task.id,"session_id":session_id,"summary":format!("Mock agent {} completed: {}",agent.id,task.objective)}),
        )
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
        ProviderFailure::Execution(message) => RuntimeError::Provider(message),
    }
}
