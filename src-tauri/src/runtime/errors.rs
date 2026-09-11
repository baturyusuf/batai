use std::path::PathBuf;

use crate::runtime::types::{AgentStatus, TaskStatus};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("watcher error: {0}")]
    Watcher(#[from] notify::Error),
    #[error("lock poisoned: {0}")]
    Lock(&'static str),
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("task not found: {0}")]
    TaskNotFound(String),
    #[error("provider not registered: {0}")]
    ProviderNotRegistered(String),
    #[error("invalid agent transition: {from} -> {to}")]
    InvalidAgentTransition { from: AgentStatus, to: AgentStatus },
    #[error("invalid task transition: {from} -> {to}")]
    InvalidTaskTransition { from: TaskStatus, to: TaskStatus },
    #[error("invalid task {task_id}: {message}")]
    InvalidTask { task_id: String, message: String },
    #[error("task dependency cycle: {0}")]
    DependencyCycle(String),
    #[error("provider execution failed: {0}")]
    Provider(String),
    #[error("resource unavailable for {agent_id}: {status}")]
    ResourceUnavailable {
        agent_id: String,
        status: String,
        reset_at: Option<String>,
    },
    #[error("invalid task file {path}: {message}")]
    InvalidTaskFile { path: PathBuf, message: String },
    #[error("runtime is shutting down")]
    ShuttingDown,
    #[error("task cancelled: {0}")]
    Cancelled(String),
    #[error("provider stopped after possible repository mutation: {0}")]
    UnknownAfterCrash(String),
    #[error("governance error: {0}")]
    Governance(String),
    #[error("organization revision conflict: expected {expected}, current {current}")]
    OrganizationConflict { expected: u64, current: u64 },
}

pub type Result<T> = std::result::Result<T, RuntimeError>;
