use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::runtime::{
    credentials::CredentialStore,
    errors::{Result, RuntimeError},
    execution_provider::{
        ExecutionMode, ExecutionProvider, ProviderExecutionResult, ProviderExecutionStatus,
        ProviderFailure, ProviderSession, UsageSnapshot, UsageSource, UsageState,
    },
    types::{Agent, ResourceStatus, Task},
};

#[derive(Debug, Clone)]
pub struct ProviderDefinition {
    pub name: &'static str,
    pub resource_id: &'static str,
    pub base_url: &'static str,
    pub allowed_models: &'static [&'static str],
    pub usage_source: UsageSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Clone)]
pub struct OpenAiCompatibleProvider {
    client: reqwest::Client,
    definition: ProviderDefinition,
    credentials: Arc<dyn CredentialStore>,
    conversations: Arc<Mutex<HashMap<String, Vec<Message>>>>,
}

impl OpenAiCompatibleProvider {
    pub fn new(definition: ProviderDefinition, credentials: Arc<dyn CredentialStore>) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(format!("Batai/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self {
            client,
            definition,
            credentials,
            conversations: Default::default(),
        }
    }

    fn credential(&self) -> std::result::Result<String, ProviderFailure> {
        self.credentials
            .get(self.definition.resource_id)
            .map_err(|_| ProviderFailure::AuthRequired)?
            .ok_or(ProviderFailure::AuthRequired)
    }

    fn messages(&self, session_id: &str) -> std::result::Result<Vec<Message>, ProviderFailure> {
        Ok(self
            .conversations
            .lock()
            .map_err(|_| ProviderFailure::Execution("conversation lock failed".into()))?
            .get(session_id)
            .cloned()
            .unwrap_or_default())
    }
}

#[async_trait]
impl ExecutionProvider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        self.definition.name
    }

    async fn create_session(&self, _agent: &Agent) -> Result<ProviderSession> {
        let id = format!(
            "{}-{}",
            self.definition.name.to_ascii_uppercase(),
            uuid::Uuid::new_v4()
        );
        self.conversations
            .lock()
            .map_err(|_| RuntimeError::Lock("compatible conversations"))?
            .insert(id.clone(), vec![]);
        Ok(ProviderSession {
            id,
            metadata: json!({"messages":[]}),
        })
    }

    async fn resume_session(&self, session_id: &str, _agent: &Agent) -> Result<ProviderSession> {
        self.conversations
            .lock()
            .map_err(|_| RuntimeError::Lock("compatible conversations"))?
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
            .map_err(|_| RuntimeError::Lock("compatible conversations"))?
            .insert(session_id.into(), messages);
        self.resume_session(session_id, agent).await
    }

    async fn send_task(
        &self,
        session_id: &str,
        agent: &Agent,
        task: &Task,
    ) -> std::result::Result<ProviderExecutionResult, ProviderFailure> {
        if !self
            .definition
            .allowed_models
            .contains(&agent.model.as_str())
        {
            return Err(ProviderFailure::Execution(format!(
                "model '{}' is not in the verified {} catalog",
                agent.model, self.definition.name
            )));
        }
        let key = self.credential()?;
        let mut messages = self.messages(session_id)?;
        messages.push(Message {
            role: "user".into(),
            content: task.objective.clone(),
        });
        let started_at = chrono::Utc::now().to_rfc3339();
        let response = self
            .client
            .post(format!(
                "{}/chat/completions",
                self.definition.base_url.trim_end_matches('/')
            ))
            .bearer_auth(key)
            .json(&json!({"model":agent.model,"messages":messages,"stream":false}))
            .send()
            .await
            .map_err(|_| ProviderFailure::Offline)?;
        let status = response.status();
        let reset_at = response
            .headers()
            .get("x-ratelimit-reset")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        if !status.is_success() {
            return Err(classify_status(status, reset_at));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|error| ProviderFailure::MalformedResponse(error.to_string()))?;
        let (content, usage, metadata) =
            parse_chat_response(&body, self.definition.usage_source.clone())?;
        messages.push(Message {
            role: "assistant".into(),
            content: content.clone(),
        });
        self.conversations
            .lock()
            .map_err(|_| ProviderFailure::Execution("conversation lock failed".into()))?
            .insert(session_id.into(), messages.clone());
        Ok(ProviderExecutionResult {
            provider: self.definition.name.into(),
            model: agent.model.clone(),
            agent_id: agent.id.clone(),
            task_id: task.id.clone(),
            session_id: session_id.into(),
            turn_id: body.get("id").and_then(Value::as_str).map(str::to_owned),
            status: ProviderExecutionStatus::Completed,
            summary: content,
            artifacts: vec![],
            changed_files: vec![],
            usage,
            started_at,
            completed_at: chrono::Utc::now().to_rfc3339(),
            execution_mode: ExecutionMode::ActionManifest,
            provider_metadata: metadata,
            session_metadata: json!({"messages":messages}),
        })
    }

    async fn get_usage_state(&self, _agent: &Agent) -> Result<UsageState> {
        Ok(UsageState {
            status: if self.credential().is_ok() {
                ResourceStatus::Available
            } else {
                ResourceStatus::AuthRequired
            },
            reset_at: None,
            details: json!({"source":"subscription","quota":"unknown","resourceId":self.definition.resource_id}),
        })
    }
}

fn classify_status(status: reqwest::StatusCode, reset_at: Option<String>) -> ProviderFailure {
    match status {
        reqwest::StatusCode::UNAUTHORIZED
        | reqwest::StatusCode::FORBIDDEN
        | reqwest::StatusCode::PAYMENT_REQUIRED => ProviderFailure::AuthRequired,
        reqwest::StatusCode::TOO_MANY_REQUESTS => ProviderFailure::RateLimited { reset_at },
        reqwest::StatusCode::REQUEST_TIMEOUT | reqwest::StatusCode::GATEWAY_TIMEOUT => {
            ProviderFailure::Timeout
        }
        _ => ProviderFailure::Execution(format!("provider returned HTTP {}", status.as_u16())),
    }
}

fn parse_chat_response(
    body: &Value,
    source: UsageSource,
) -> std::result::Result<(String, UsageSnapshot, Value), ProviderFailure> {
    let content = body
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ProviderFailure::MalformedResponse("missing choices[0].message.content".into())
        })?
        .to_owned();
    let usage = UsageSnapshot {
        source,
        input_tokens: body.pointer("/usage/prompt_tokens").and_then(Value::as_u64),
        cached_input_tokens: body
            .pointer("/usage/prompt_tokens_details/cached_tokens")
            .and_then(Value::as_u64),
        output_tokens: body
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64),
        details: json!({"total_tokens":body.pointer("/usage/total_tokens")}),
        ..UsageSnapshot::default()
    };
    Ok((
        content,
        usage,
        json!({"response_id":body.get("id"),"finish_reason":body.pointer("/choices/0/finish_reason")}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    struct TestCredentials(Option<String>);
    impl CredentialStore for TestCredentials {
        fn set(&self, _resource_id: &str, _secret: &str) -> Result<()> {
            Ok(())
        }
        fn get(&self, _resource_id: &str) -> Result<Option<String>> {
            Ok(self.0.clone())
        }
        fn delete(&self, _resource_id: &str) -> Result<()> {
            Ok(())
        }
    }
    fn fake_server(status: &str, body: &str) -> &'static str {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let body = body.to_owned();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 8192];
            let size = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..size]);
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer test-key"));
            let response=format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            stream.write_all(response.as_bytes()).unwrap();
        });
        Box::leak(format!("http://{address}/v1").into_boxed_str())
    }
    fn provider(base_url: &'static str, key: Option<String>) -> OpenAiCompatibleProvider {
        OpenAiCompatibleProvider::new(
            ProviderDefinition {
                name: "test",
                resource_id: "test-resource",
                base_url,
                allowed_models: &["model"],
                usage_source: UsageSource::Subscription,
            },
            Arc::new(TestCredentials(key)),
        )
    }
    fn agent(model: &str) -> Agent {
        serde_json::from_value(json!({"id":"a","name":"A","provider":"test","model":model}))
            .unwrap()
    }
    fn task() -> Task {
        serde_json::from_value(
            json!({"id":"t","created_by":"d","objective":"hello","assigned_to":["a"]}),
        )
        .unwrap()
    }
    #[test]
    fn usage_parses_and_hidden_reasoning_is_discarded() {
        let body = json!({"id":"x","choices":[{"message":{"content":"ok","reasoning_content":"secret"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":4,"total_tokens":14}});
        let (content, usage, metadata) =
            parse_chat_response(&body, UsageSource::Subscription).unwrap();
        assert_eq!(content, "ok");
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(4));
        assert!(!metadata.to_string().contains("secret"));
    }
    #[test]
    fn auth_rate_limit_timeout_and_malformed_are_classified() {
        assert_eq!(
            classify_status(reqwest::StatusCode::UNAUTHORIZED, None),
            ProviderFailure::AuthRequired
        );
        assert!(matches!(
            classify_status(reqwest::StatusCode::TOO_MANY_REQUESTS, Some("later".into())),
            ProviderFailure::RateLimited { .. }
        ));
        assert_eq!(
            classify_status(reqwest::StatusCode::GATEWAY_TIMEOUT, None),
            ProviderFailure::Timeout
        );
        assert!(parse_chat_response(&json!({}), UsageSource::Subscription).is_err());
    }
    #[tokio::test]
    async fn fake_http_success_parses_usage_without_secret_metadata() {
        let base = fake_server(
            "200 OK",
            r#"{"id":"turn","choices":[{"message":{"content":"done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#,
        );
        let provider = provider(base, Some("test-key".into()));
        let session = provider.create_session(&agent("model")).await.unwrap();
        let result = provider
            .send_task(&session.id, &agent("model"), &task())
            .await
            .unwrap();
        assert_eq!(result.usage.input_tokens, Some(3));
        assert!(!serde_json::to_string(&result).unwrap().contains("test-key"));
    }
    #[tokio::test]
    async fn fake_http_auth_rate_limit_malformed_and_invalid_model_fail_safely() {
        let missing = provider("http://127.0.0.1:1/v1", None);
        assert_eq!(
            missing
                .send_task("s", &agent("model"), &task())
                .await
                .unwrap_err(),
            ProviderFailure::AuthRequired
        );
        let rate = provider(
            fake_server("429 Too Many Requests", "{}"),
            Some("test-key".into()),
        );
        assert!(matches!(
            rate.send_task("s", &agent("model"), &task()).await,
            Err(ProviderFailure::RateLimited { .. })
        ));
        let malformed = provider(fake_server("200 OK", "{}"), Some("test-key".into()));
        assert!(matches!(
            malformed.send_task("s", &agent("model"), &task()).await,
            Err(ProviderFailure::MalformedResponse(_))
        ));
        let valid = provider("http://127.0.0.1:1/v1", Some("test-key".into()));
        assert!(matches!(
            valid.send_task("s", &agent("unknown"), &task()).await,
            Err(ProviderFailure::Execution(_))
        ));
    }
}
