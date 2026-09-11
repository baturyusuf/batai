use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use crate::runtime::{
    benchmark::{BenchmarkCaseResult, BenchmarkCategory, BenchmarkExecutor},
    errors::{Result, RuntimeError},
    execution_provider::{
        ExecutionMode, ExecutionProvider, ProviderExecutionResult, ProviderExecutionStatus,
        ProviderFailure, ProviderSession, UsageSnapshot, UsageSource, UsageState,
    },
    types::{Agent, ResourceStatus, Task},
};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[async_trait]
impl BenchmarkExecutor for OllamaProvider {
    async fn execute(&self, model: &str, category: BenchmarkCategory) -> BenchmarkCaseResult {
        let (prompt, expected) = benchmark_fixture(category);
        let started = Instant::now();
        let response = self
            .client
            .post(format!("{}/generate", self.base_url))
            .json(
                &json!({"model":model,"prompt":prompt,"stream":false,"options":{"temperature":0}}),
            )
            .send()
            .await;
        let latency_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let Ok(response) = response else {
            return benchmark_failure(category, latency_ms, "offline");
        };
        if !response.status().is_success() {
            return benchmark_failure(category, latency_ms, &format!("HTTP {}", response.status()));
        }
        let Ok(body) = response.json::<Value>().await else {
            return benchmark_failure(category, latency_ms, "malformed response");
        };
        let output = body
            .get("response")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let normalized = output
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        let correctness = expected
            .iter()
            .filter(|token| normalized.contains(&token.to_ascii_lowercase()))
            .count() as f64
            / expected.len() as f64;
        let total_duration = body.get("total_duration").and_then(Value::as_u64);
        let eval_duration = body.get("eval_duration").and_then(Value::as_u64);
        let eval_count = body.get("eval_count").and_then(Value::as_u64);
        BenchmarkCaseResult {
            category,
            correctness,
            tests_passed: Some(correctness >= 0.99),
            structured_output_valid: serde_json::from_str::<Value>(output).is_ok(),
            retry_count: 0,
            latency_ms,
            time_to_first_token_ms: body
                .get("prompt_eval_duration")
                .and_then(Value::as_u64)
                .map(|ns| ns / 1_000_000),
            tokens_per_second: eval_count
                .zip(eval_duration)
                .filter(|(_, duration)| *duration > 0)
                .map(|(count, duration)| count as f64 / (duration as f64 / 1_000_000_000.0)),
            total_tokens: body
                .get("prompt_eval_count")
                .and_then(Value::as_u64)
                .zip(eval_count)
                .map(|(prompt, output)| prompt + output),
            peak_memory_bytes: None,
            failure: None.or_else(|| {
                total_duration
                    .is_none()
                    .then(|| "timing unavailable".into())
            }),
        }
    }
}

fn benchmark_failure(
    category: BenchmarkCategory,
    latency_ms: u64,
    reason: &str,
) -> BenchmarkCaseResult {
    BenchmarkCaseResult {
        category,
        correctness: 0.0,
        tests_passed: Some(false),
        structured_output_valid: false,
        retry_count: 0,
        latency_ms,
        time_to_first_token_ms: None,
        tokens_per_second: None,
        total_tokens: None,
        peak_memory_bytes: None,
        failure: Some(reason.into()),
    }
}

fn benchmark_fixture(category: BenchmarkCategory) -> (&'static str, &'static [&'static str]) {
    match category {
        BenchmarkCategory::Coding => ("Return only JSON. Implement add(a,b) as a JavaScript expression. Schema: {\"answer\":\"...\"}", &["\"answer\"", "a+b"]),
        BenchmarkCategory::Debugging => ("Return only JSON. Fix: function even(n){return n%2===1}. Schema {\"answer\":\"correct expression\"}", &["\"answer\"", "n%2===0"]),
        BenchmarkCategory::Script => ("Return only JSON for CSV a,1 newline b,2 transformed to object. Schema {\"a\":1,\"b\":2}", &["\"a\":1", "\"b\":2"]),
        BenchmarkCategory::TestGeneration => ("Return only JSON with boundary test inputs for absolute value. Schema {\"tests\":[-1,0,1]}", &["-1", "0", "1"]),
        BenchmarkCategory::Planning => ("Return only JSON with exactly three ordered steps: inspect, implement, test. Schema {\"steps\":[...]}", &["inspect", "implement", "test"]),
        BenchmarkCategory::InstructionFollowing => ("Return exactly this JSON and nothing else: {\"batai\":true,\"count\":3}", &["\"batai\":true", "\"count\":3"]),
        BenchmarkCategory::ToolPatch => ("Return only JSON action manifest to replace x with y in hello.txt. Schema {\"action\":\"replace\",\"path\":\"hello.txt\",\"from\":\"x\",\"to\":\"y\"}", &["\"action\":\"replace\"", "\"path\":\"hello.txt\"", "\"from\":\"x\"", "\"to\":\"y\""]),
    }
}

#[derive(Clone)]
pub struct OllamaProvider {
    client: reqwest::Client,
    base_url: String,
    conversations: Arc<Mutex<HashMap<String, Vec<Message>>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OllamaState {
    NotInstalled,
    InstalledOffline,
    Running,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModel {
    pub name: String,
    pub model: Option<String>,
    pub size: Option<u64>,
    pub digest: Option<String>,
    pub modified_at: Option<String>,
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullProgress {
    pub status: String,
    pub digest: Option<String>,
    pub total: Option<u64>,
    pub completed: Option<u64>,
    pub percent: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct PullCancellation(Arc<AtomicBool>);
impl PullCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
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

    pub async fn installation_state(&self) -> OllamaState {
        match self
            .client
            .get(format!("{}/tags", self.base_url))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => OllamaState::Running,
            Ok(_) => OllamaState::Error,
            Err(_)
                if std::process::Command::new("ollama")
                    .arg("--version")
                    .output()
                    .is_ok() =>
            {
                OllamaState::InstalledOffline
            }
            Err(_) => OllamaState::NotInstalled,
        }
    }

    pub async fn list_models(&self) -> std::result::Result<Vec<OllamaModel>, ProviderFailure> {
        let response = self
            .client
            .get(format!("{}/tags", self.base_url))
            .send()
            .await
            .map_err(|_| ProviderFailure::Offline)?;
        if !response.status().is_success() {
            return Err(ProviderFailure::Execution(format!(
                "Ollama returned HTTP {}",
                response.status()
            )));
        }
        let value: Value = response
            .json()
            .await
            .map_err(|error| ProviderFailure::MalformedResponse(error.to_string()))?;
        value
            .get("models")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderFailure::MalformedResponse("missing models".into()))?
            .iter()
            .map(parse_model)
            .collect()
    }

    pub async fn running_models(&self) -> std::result::Result<Vec<OllamaModel>, ProviderFailure> {
        let response = self
            .client
            .get(format!("{}/ps", self.base_url))
            .send()
            .await
            .map_err(|_| ProviderFailure::Offline)?;
        if !response.status().is_success() {
            return Err(ProviderFailure::Execution(format!(
                "Ollama returned HTTP {}",
                response.status()
            )));
        }
        let value: Value = response
            .json()
            .await
            .map_err(|error| ProviderFailure::MalformedResponse(error.to_string()))?;
        value
            .get("models")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderFailure::MalformedResponse("missing models".into()))?
            .iter()
            .map(parse_model)
            .collect()
    }

    pub async fn show_model(&self, name: &str) -> std::result::Result<Value, ProviderFailure> {
        let response = self
            .client
            .post(format!("{}/show", self.base_url))
            .json(&json!({"model":name}))
            .send()
            .await
            .map_err(|_| ProviderFailure::Offline)?;
        if !response.status().is_success() {
            return Err(ProviderFailure::Execution(format!(
                "Ollama returned HTTP {}",
                response.status()
            )));
        }
        response
            .json()
            .await
            .map_err(|error| ProviderFailure::MalformedResponse(error.to_string()))
    }

    pub async fn pull_model<F>(
        &self,
        name: &str,
        cancellation: &PullCancellation,
        mut on_progress: F,
    ) -> std::result::Result<(), ProviderFailure>
    where
        F: FnMut(PullProgress) + Send,
    {
        let mut response = self
            .client
            .post(format!("{}/pull", self.base_url))
            .json(&json!({"model":name,"stream":true}))
            .send()
            .await
            .map_err(|_| ProviderFailure::Offline)?;
        if !response.status().is_success() {
            return Err(ProviderFailure::Execution(format!(
                "Ollama returned HTTP {}",
                response.status()
            )));
        }
        let mut pending = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| ProviderFailure::Execution(error.to_string()))?
        {
            if cancellation.cancelled() {
                return Err(ProviderFailure::Cancelled);
            }
            pending.extend_from_slice(&chunk);
            while let Some(position) = pending.iter().position(|byte| *byte == b'\n') {
                let line = pending.drain(..=position).collect::<Vec<_>>();
                if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                    on_progress(parse_pull_progress(&value));
                }
            }
        }
        if !pending.is_empty() {
            let value: Value = serde_json::from_slice(&pending)
                .map_err(|error| ProviderFailure::MalformedResponse(error.to_string()))?;
            on_progress(parse_pull_progress(&value));
        }
        Ok(())
    }

    pub async fn remove_model(&self, name: &str) -> std::result::Result<(), ProviderFailure> {
        let response = self
            .client
            .delete(format!("{}/delete", self.base_url))
            .json(&json!({"model":name}))
            .send()
            .await
            .map_err(|_| ProviderFailure::Offline)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(ProviderFailure::Execution(format!(
                "Ollama returned HTTP {}",
                response.status()
            )))
        }
    }
}

fn parse_model(value: &Value) -> std::result::Result<OllamaModel, ProviderFailure> {
    let name = value
        .get("name")
        .or_else(|| value.get("model"))
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderFailure::MalformedResponse("model missing name".into()))?
        .to_owned();
    Ok(OllamaModel {
        name,
        model: value
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_owned),
        size: value.get("size").and_then(Value::as_u64),
        digest: value
            .get("digest")
            .and_then(Value::as_str)
            .map(str::to_owned),
        modified_at: value
            .get("modified_at")
            .and_then(Value::as_str)
            .map(str::to_owned),
        details: value.get("details").cloned().unwrap_or(Value::Null),
    })
}

fn parse_pull_progress(value: &Value) -> PullProgress {
    let total = value.get("total").and_then(Value::as_u64);
    let completed = value.get("completed").and_then(Value::as_u64);
    PullProgress {
        status: value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .into(),
        digest: value
            .get("digest")
            .and_then(Value::as_str)
            .map(str::to_owned),
        total,
        completed,
        percent: completed
            .zip(total)
            .filter(|(_, total)| *total > 0)
            .map(|(completed, total)| completed as f64 * 100.0 / total as f64),
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
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    fn fake_server(status: &str, body: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let body = body.to_owned();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            let response=format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{address}/api")
    }
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

    #[tokio::test]
    async fn fake_ollama_lists_models_and_handles_failure_and_offline() {
        let provider = OllamaProvider::new(fake_server(
            "200 OK",
            r#"{"models":[{"name":"qwen:3b","size":123,"digest":"abc","details":{"family":"qwen"}}]}"#,
        ));
        let models = provider.list_models().await.unwrap();
        assert_eq!(models[0].name, "qwen:3b");
        assert_eq!(models[0].size, Some(123));
        let failure = OllamaProvider::new(fake_server("500 Internal Server Error", "{}"));
        assert!(matches!(
            failure.show_model("x").await,
            Err(ProviderFailure::Execution(_))
        ));
        let offline = OllamaProvider::new("http://127.0.0.1:1/api");
        assert_eq!(
            offline.list_models().await.unwrap_err(),
            ProviderFailure::Offline
        );
    }
    #[tokio::test]
    async fn fake_pull_reports_real_stream_progress_and_can_cancel() {
        let provider=OllamaProvider::new(fake_server("200 OK","{\"status\":\"downloading\",\"total\":100,\"completed\":50}\n{\"status\":\"success\"}\n"));
        let mut progress = Vec::new();
        provider
            .pull_model("qwen", &PullCancellation::default(), |item| {
                progress.push(item)
            })
            .await
            .unwrap();
        assert_eq!(progress[0].percent, Some(50.0));
        assert_eq!(progress[1].status, "success");
        let cancellation = PullCancellation::default();
        cancellation.cancel();
        let provider = OllamaProvider::new(fake_server("200 OK", "{\"status\":\"downloading\"}\n"));
        assert_eq!(
            provider
                .pull_model("qwen", &cancellation, |_| {})
                .await
                .unwrap_err(),
            ProviderFailure::Cancelled
        );
    }
}
