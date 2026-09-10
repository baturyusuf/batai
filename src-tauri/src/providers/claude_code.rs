use async_trait::async_trait;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use super::process_supervisor::{CommandSpec, ProcessSupervisor};
use crate::runtime::{
    errors::{Result, RuntimeError},
    execution_provider::{
        ExecutionMode, ExecutionProvider, ProviderExecutionResult, ProviderExecutionStatus,
        ProviderFailure, ProviderSession, UsageSnapshot, UsageSource, UsageState,
    },
    types::{Agent, ResourceStatus, Task},
};

#[derive(Clone)]
pub struct ClaudeCodeProvider {
    executable: String,
    supervisor: ProcessSupervisor,
    known_sessions: Arc<Mutex<HashSet<String>>>,
    active: Arc<Mutex<HashMap<String, String>>>,
}

impl Default for ClaudeCodeProvider {
    fn default() -> Self {
        Self::new("claude")
    }
}
impl ClaudeCodeProvider {
    pub fn new(executable: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
            supervisor: Default::default(),
            known_sessions: Default::default(),
            active: Default::default(),
        }
    }
}

#[async_trait]
impl ExecutionProvider for ClaudeCodeProvider {
    fn name(&self) -> &str {
        "claude"
    }
    async fn create_session(&self, _agent: &Agent) -> Result<ProviderSession> {
        let id = uuid::Uuid::new_v4().to_string();
        Ok(ProviderSession {
            id,
            metadata: json!({"cli":"claude-code","resumable":true}),
        })
    }
    async fn resume_session(&self, session_id: &str, _agent: &Agent) -> Result<ProviderSession> {
        self.known_sessions
            .lock()
            .map_err(|_| RuntimeError::Lock("claude sessions"))?
            .insert(session_id.into());
        Ok(ProviderSession {
            id: session_id.into(),
            metadata: json!({"cli":"claude-code","resumable":true}),
        })
    }
    async fn send_task(
        &self,
        session_id: &str,
        agent: &Agent,
        task: &Task,
    ) -> std::result::Result<ProviderExecutionResult, ProviderFailure> {
        let started_at = chrono::Utc::now().to_rfc3339();
        let process_id = format!("claude:{}:{}", session_id, task.id);
        self.active
            .lock()
            .map_err(|_| ProviderFailure::Execution("claude active lock failed".into()))?
            .insert(session_id.into(), process_id.clone());
        let known = self
            .known_sessions
            .lock()
            .map_err(|_| ProviderFailure::Execution("claude session lock failed".into()))?
            .contains(session_id);
        let mut args = vec![
            "-p".into(),
            task.objective.clone(),
            "--output-format".into(),
            "json".into(),
            "--model".into(),
            agent.model.clone(),
        ];
        if known {
            args.extend(["--resume".into(), session_id.into()]);
        } else {
            args.extend(["--session-id".into(), session_id.into()]);
        }
        let output = self
            .supervisor
            .run(
                &process_id,
                CommandSpec {
                    executable: self.executable.clone(),
                    args,
                    cwd: agent.worktree.as_ref().map(PathBuf::from),
                    timeout: Duration::from_secs(60 * 60),
                    stdin: None,
                },
            )
            .await;
        self.active
            .lock()
            .map_err(|_| ProviderFailure::Execution("claude active lock failed".into()))?
            .remove(session_id);
        let output = output?;
        if output.exit_code != Some(0) {
            let text = if output.stderr.is_empty() {
                output.stdout
            } else {
                output.stderr
            };
            let lower = text.to_ascii_lowercase();
            if lower.contains("login") || lower.contains("auth") {
                return Err(ProviderFailure::AuthRequired);
            }
            if lower.contains("rate limit") || lower.contains("usage limit") {
                return Err(ProviderFailure::RateLimited { reset_at: None });
            }
            return Err(ProviderFailure::ProcessCrash {
                message: format!("Claude Code exited with {:?}: {}", output.exit_code, text),
            });
        }
        let output_meta = ClaudeOutputMeta {
            process_id: &output.id,
            process_pid: output.process_id,
            truncated: output.truncated,
        };
        let result = parse_result(
            &output.stdout,
            session_id,
            agent,
            task,
            started_at,
            output_meta,
        )?;
        self.known_sessions
            .lock()
            .map_err(|_| ProviderFailure::Execution("claude session lock failed".into()))?
            .insert(result.session_id.clone());
        Ok(result)
    }
    async fn cancel_turn(&self, session_id: &str) -> Result<bool> {
        let id = self
            .active
            .lock()
            .map_err(|_| RuntimeError::Lock("claude active"))?
            .get(session_id)
            .cloned();
        Ok(id.is_some_and(|id| self.supervisor.cancel(&id)))
    }
    async fn get_usage_state(&self, _agent: &Agent) -> Result<UsageState> {
        Ok(UsageState {
            status: ResourceStatus::Unknown,
            reset_at: None,
            details: json!({"source":"subscription","note":"Claude Code does not expose an authoritative quota endpoint"}),
        })
    }
}

struct ClaudeOutputMeta<'a> {
    process_id: &'a str,
    process_pid: Option<u32>,
    truncated: bool,
}

fn parse_result(
    output: &str,
    expected_session: &str,
    agent: &Agent,
    task: &Task,
    started_at: String,
    output_meta: ClaudeOutputMeta<'_>,
) -> std::result::Result<ProviderExecutionResult, ProviderFailure> {
    let value: Value = serde_json::from_str(output.trim())
        .map_err(|e| ProviderFailure::MalformedResponse(e.to_string()))?;
    if value
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(ProviderFailure::Execution(
            value
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or("Claude Code failed")
                .into(),
        ));
    }
    let summary = value
        .get("result")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/message/content").and_then(Value::as_str))
        .ok_or_else(|| ProviderFailure::MalformedResponse("missing result".into()))?
        .to_owned();
    let session_id = value
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or(expected_session)
        .to_owned();
    let usage_value = value.get("usage").cloned().unwrap_or(Value::Null);
    let usage = UsageSnapshot {
        source: UsageSource::Subscription,
        input_tokens: usage_value.get("input_tokens").and_then(Value::as_u64),
        cached_input_tokens: usage_value
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64),
        output_tokens: usage_value.get("output_tokens").and_then(Value::as_u64),
        cost: value.get("total_cost_usd").and_then(Value::as_f64),
        currency: value
            .get("total_cost_usd")
            .and_then(Value::as_f64)
            .map(|_| "USD".into()),
        details: json!({"duration_ms":value.get("duration_ms"),"num_turns":value.get("num_turns")}),
        ..UsageSnapshot::default()
    };
    Ok(ProviderExecutionResult {
        provider: "claude".into(),
        model: agent.model.clone(),
        agent_id: agent.id.clone(),
        task_id: task.id.clone(),
        session_id,
        turn_id: None,
        status: ProviderExecutionStatus::Completed,
        summary,
        artifacts: vec![],
        changed_files: vec![],
        usage,
        started_at,
        completed_at: chrono::Utc::now().to_rfc3339(),
        execution_mode: ExecutionMode::DirectMutation,
        provider_metadata: json!({"subtype":value.get("subtype"),"process_id":output_meta.process_id,"process_pid":output_meta.process_pid,"output_truncated":output_meta.truncated}),
        session_metadata: json!({"cli":"claude-code","resumable":true}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_structured_claude_output_and_usage() {
        let agent: Agent = serde_json::from_value(
            json!({"id":"a","name":"A","provider":"claude","model":"sonnet"}),
        )
        .unwrap();
        let task: Task = serde_json::from_value(
            json!({"id":"t","created_by":"d","objective":"Do it","assigned_to":["a"]}),
        )
        .unwrap();
        let result = parse_result(r#"{"result":"done","session_id":"real","usage":{"input_tokens":4,"output_tokens":2},"total_cost_usd":0.02}"#, "expected", &agent, &task, "now".into(), ClaudeOutputMeta { process_id: "fixture", process_pid: Some(42), truncated: false }).unwrap();
        assert_eq!(result.session_id, "real");
        assert_eq!(result.usage.input_tokens, Some(4));
        assert_eq!(result.usage.cost, Some(0.02));
    }
}
