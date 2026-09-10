use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::runtime::{
    errors::{Result, RuntimeError},
    execution_provider::{
        ExecutionMode, ExecutionProvider, ProviderExecutionResult, ProviderExecutionStatus,
        ProviderFailure, ProviderSession, UsageSnapshot, UsageSource, UsageState,
    },
    types::{Agent, ResourceStatus, Task},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Clone)]
pub struct OllamaProvider {
    client: reqwest::Client,
    base_url: String,
    conversations: Arc<Mutex<HashMap<String, Vec<Message>>>>,
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new("http://127.0.0.1:11434/api")
    }
}
impl OllamaProvider {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').into(),
            conversations: Default::default(),
        }
    }
    fn messages(&self, id: &str) -> std::result::Result<Vec<Message>, ProviderFailure> {
        Ok(self
            .conversations
            .lock()
            .map_err(|_| ProviderFailure::Execution("ollama conversation lock failed".into()))?
            .get(id)
            .cloned()
            .unwrap_or_default())
    }
}

#[async_trait]
impl ExecutionProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn create_session(&self, _agent: &Agent) -> Result<ProviderSession> {
        let id = format!("OLLAMA-{}", uuid::Uuid::new_v4());
        self.conversations
            .lock()
            .map_err(|_| RuntimeError::Lock("ollama conversations"))?
            .insert(id.clone(), vec![]);
        Ok(ProviderSession {
            id,
            metadata: json!({"messages":[]}),
        })
    }

    async fn resume_session(&self, session_id: &str, _agent: &Agent) -> Result<ProviderSession> {
        self.conversations
            .lock()
            .map_err(|_| RuntimeError::Lock("ollama conversations"))?
            .entry(session_id.into())
            .or_default();
        Ok(ProviderSession {
            id: session_id.into(),
            metadata: json!({"messages":self.messages(session_id).unwrap_or_default()}),
        })
    }

    async fn restore_session(
        &self,
        session_id: &str,
        metadata: &Value,
        agent: &Agent,
    ) -> Result<ProviderSession> {
        let messages = serde_json::from_value(
            metadata
                .get("messages")
                .cloned()
                .unwrap_or_else(|| json!([])),
        )
        .unwrap_or_default();
        self.conversations
            .lock()
            .map_err(|_| RuntimeError::Lock("ollama conversations"))?
            .insert(session_id.into(), messages);
        self.resume_session(session_id, agent).await
    }

    async fn send_task(
        &self,
        session_id: &str,
        agent: &Agent,
        task: &Task,
    ) -> std::result::Result<ProviderExecutionResult, ProviderFailure> {
        let started_at = chrono::Utc::now().to_rfc3339();
        let mut messages = self.messages(session_id)?;
        messages.push(Message {
            role: "user".into(),
            content: task.objective.clone(),
        });
        let response = self
            .client
            .post(format!("{}/chat", self.base_url))
            .json(&json!({"model":agent.model,"messages":messages,"stream":false}))
            .send()
            .await
            .map_err(|_| ProviderFailure::Offline)?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ProviderFailure::AuthRequired);
        }
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ProviderFailure::RateLimited { reset_at: None });
        }
        if !response.status().is_success() {
            return Err(ProviderFailure::Execution(format!(
                "Ollama returned HTTP {}",
                response.status().as_u16()
            )));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|e| ProviderFailure::MalformedResponse(e.to_string()))?;
        let (content, usage, provider_metadata) = parse_response(&body)?;
        messages.push(Message {
            role: "assistant".into(),
            content: content.clone(),
        });
        self.conversations
            .lock()
            .map_err(|_| ProviderFailure::Execution("ollama conversation lock failed".into()))?
            .insert(session_id.into(), messages.clone());
        Ok(ProviderExecutionResult {
            provider: self.name().into(),
            model: agent.model.clone(),
            agent_id: agent.id.clone(),
            task_id: task.id.clone(),
            session_id: session_id.into(),
            turn_id: None,
            status: ProviderExecutionStatus::Completed,
            summary: content,
            artifacts: vec![],
            changed_files: vec![],
            usage,
            started_at,
            completed_at: chrono::Utc::now().to_rfc3339(),
            execution_mode: ExecutionMode::ActionManifest,
            provider_metadata,
            session_metadata: json!({"messages":messages}),
        })
    }

    async fn get_usage_state(&self, _agent: &Agent) -> Result<UsageState> {
        let result = self
            .client
            .get(format!("{}/tags", self.base_url))
            .send()
            .await;
        Ok(match result {
            Ok(response) if response.status().is_success() => UsageState {
                status: ResourceStatus::Available,
                reset_at: None,
                details: json!({"source":"local"}),
            },
            _ => UsageState {
                status: ResourceStatus::Offline,
                reset_at: None,
                details: json!({"source":"local"}),
            },
        })
    }
}

fn parse_response(
    body: &Value,
) -> std::result::Result<(String, UsageSnapshot, Value), ProviderFailure> {
    let content = body
        .pointer("/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderFailure::MalformedResponse("missing message.content".into()))?
        .to_owned();
    let usage = UsageSnapshot {
        source: UsageSource::Local,
        input_tokens: body.get("prompt_eval_count").and_then(Value::as_u64),
        cached_input_tokens: body.get("prompt_eval_cached_count").and_then(Value::as_u64),
        output_tokens: body.get("eval_count").and_then(Value::as_u64),
        details: json!({"total_duration":body.get("total_duration"),"load_duration":body.get("load_duration")}),
        ..UsageSnapshot::default()
    };
    Ok((
        content,
        usage,
        json!({"done_reason":body.get("done_reason")}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn session_metadata_round_trips_without_thinking_content() {
        let provider = OllamaProvider::default();
        let agent: Agent = serde_json::from_value(
            json!({"id":"a","name":"A","provider":"ollama","model":"qwen3"}),
        )
        .unwrap();
        provider
            .restore_session(
                "s",
                &json!({"messages":[{"role":"user","content":"hello"}]}),
                &agent,
            )
            .await
            .unwrap();
        let restored = provider.messages("s").unwrap();
        assert_eq!(restored.len(), 1);
        assert!(!serde_json::to_string(&restored)
            .unwrap()
            .contains("thinking"));
    }

    #[test]
    fn response_usage_is_normalized_and_thinking_is_discarded() {
        let body = json!({"message":{"content":"answer","thinking":"private"},"prompt_eval_count":7,"prompt_eval_cached_count":2,"eval_count":3,"done_reason":"stop"});
        let (content, usage, metadata) = parse_response(&body).unwrap();
        assert_eq!(content, "answer");
        assert_eq!(usage.input_tokens, Some(7));
        assert_eq!(usage.cached_input_tokens, Some(2));
        assert!(!metadata.to_string().contains("private"));
    }
}
