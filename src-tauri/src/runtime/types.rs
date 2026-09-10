use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_json::Value;

macro_rules! uppercase_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "SCREAMING_SNAKE_CASE")]
        pub enum $name { $($variant),+ }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                let value = serde_json::to_value(self).map_err(|_| fmt::Error)?;
                formatter.write_str(value.as_str().ok_or(fmt::Error)?)
            }
        }

        impl FromStr for $name {
            type Err = String;
            fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
                serde_json::from_value(Value::String(value.to_owned()))
                    .map_err(|_| format!("invalid {}: {value}", stringify!($name)))
            }
        }
    };
}

uppercase_enum!(AgentStatus {
    Created,
    Initializing,
    Ready,
    Running,
    Blocked,
    WaitingResource,
    Paused,
    Completed,
    Failed,
    Terminated
});
uppercase_enum!(TaskStatus {
    Pending,
    Ready,
    Running,
    Blocked,
    WaitingResource,
    Review,
    Completed,
    Failed,
    Cancelled
});
uppercase_enum!(ResourceStatus {
    Available,
    Low,
    Degraded,
    RateLimited,
    Unknown,
    AuthRequired,
    Offline
});
uppercase_enum!(SchedulerJobStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled
});
uppercase_enum!(TaskRunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    WaitingResource,
    Cancelled
});
uppercase_enum!(EventType {
    TaskCreated,
    TaskUpdated,
    TaskAssigned,
    TaskStarted,
    TaskBlocked,
    TaskCompleted,
    TaskFailed,
    TaskCancelled,
    TaskDispatched,
    TaskResultRecorded,
    ReviewRequired,
    AgentStatusChanged,
    AgentRateLimited,
    AgentResumed,
    ResourceStatusChanged
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Agent {
    pub id: String,
    pub name: String,
    #[serde(default = "default_role")]
    pub role_template: String,
    #[serde(default)]
    pub parent_agent_id: Option<String>,
    pub provider: String,
    pub model: String,
    #[serde(default = "default_reasoning")]
    pub reasoning_effort: String,
    #[serde(default = "default_auth_mode")]
    pub auth_mode: String,
    #[serde(default)]
    pub worktree: Option<String>,
    #[serde(default)]
    pub status: AgentStatus,
    #[serde(default)]
    pub current_task_id: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

fn default_role() -> String {
    "Agent".into()
}
fn default_reasoning() -> String {
    "medium".into()
}
fn default_auth_mode() -> String {
    "unknown".into()
}
#[allow(clippy::derivable_impls)]
impl Default for AgentStatus {
    fn default() -> Self {
        Self::Created
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TaskExecution {
    #[serde(default)]
    pub parallel: bool,
    #[serde(default)]
    pub requires_director_review: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TaskSuccessAction {
    #[serde(default)]
    pub start: Vec<String>,
    #[serde(default)]
    pub notify: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub created_by: String,
    pub objective: String,
    pub assigned_to: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default)]
    pub execution: TaskExecution,
    #[serde(default)]
    pub on_success: TaskSuccessAction,
    #[serde(default)]
    pub on_failure: TaskSuccessAction,
    #[serde(default = "default_weight")]
    pub weight: f64,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

fn default_weight() -> f64 {
    1.0
}
#[allow(clippy::derivable_impls)]
impl Default for TaskStatus {
    fn default() -> Self {
        Self::Pending
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub timestamp: String,
    pub source: String,
    pub target: Option<String>,
    pub task_id: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub agent_id: String,
    pub provider: String,
    pub provider_session_id: String,
    pub fingerprint: String,
    pub metadata: Value,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceState {
    pub agent_id: String,
    pub status: ResourceStatus,
    pub reset_at: Option<String>,
    pub details: Value,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskRun {
    pub task_id: String,
    pub agent_id: String,
    pub attempt: i64,
    pub status: TaskRunStatus,
    pub result: Option<Value>,
    pub provider: Option<String>,
    pub provider_session_id: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SchedulerJob {
    pub id: String,
    pub kind: String,
    pub agent_id: Option<String>,
    pub run_at: String,
    pub payload: Value,
    pub status: SchedulerJobStatus,
    pub attempts: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IngestDisposition {
    Created,
    Updated,
    Duplicate,
}
