use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command, sync::oneshot};

use crate::runtime::execution_provider::ProviderFailure;

const OUTPUT_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub executable: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub timeout: Duration,
    pub stdin: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct ProcessOutput {
    pub id: String,
    pub process_id: Option<u32>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

#[derive(Clone, Default)]
pub struct ProcessSupervisor {
    active: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
}

impl ProcessSupervisor {
    pub async fn run(
        &self,
        id: impl Into<String>,
        spec: CommandSpec,
    ) -> Result<ProcessOutput, ProviderFailure> {
        let id = id.into();
        if spec.executable.trim().is_empty() {
            return Err(ProviderFailure::Execution("empty executable".into()));
        }
        let mut command = Command::new(&spec.executable);
        command
            .args(&spec.args)
            .stdin(if spec.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        let mut child = command.spawn().map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => ProviderFailure::AppServerUnavailable(format!(
                "{} is not installed",
                spec.executable
            )),
            _ => ProviderFailure::Execution(redact_secrets(&error.to_string())),
        })?;
        let process_id = child.id();
        if let Some(bytes) = spec.stdin {
            use tokio::io::AsyncWriteExt;
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(&bytes)
                    .await
                    .map_err(|e| ProviderFailure::Execution(e.to_string()))?;
            }
        }
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ProviderFailure::Execution("stdout unavailable".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ProviderFailure::Execution("stderr unavailable".into()))?;
        let stdout_task = tokio::spawn(read_bounded(stdout));
        let stderr_task = tokio::spawn(read_bounded(stderr));
        let (cancel_tx, cancel_rx) = oneshot::channel();
        self.active
            .lock()
            .map_err(|_| ProviderFailure::Execution("process registry lock failed".into()))?
            .insert(id.clone(), cancel_tx);
        let outcome = tokio::select! {
            status = child.wait() => status.map_err(|e| ProviderFailure::Execution(e.to_string())),
            _ = tokio::time::sleep(spec.timeout) => { let _ = child.kill().await; Err(ProviderFailure::Timeout) },
            _ = cancel_rx => { let _ = child.kill().await; Err(ProviderFailure::Cancelled) },
        };
        self.active
            .lock()
            .map_err(|_| ProviderFailure::Execution("process registry lock failed".into()))?
            .remove(&id);
        let (stdout, out_cut) = stdout_task
            .await
            .map_err(|e| ProviderFailure::Execution(e.to_string()))?
            .map_err(|e| ProviderFailure::Execution(e.to_string()))?;
        let (stderr, err_cut) = stderr_task
            .await
            .map_err(|e| ProviderFailure::Execution(e.to_string()))?
            .map_err(|e| ProviderFailure::Execution(e.to_string()))?;
        let status = outcome?;
        Ok(ProcessOutput {
            id,
            process_id,
            exit_code: status.code(),
            stdout: redact_secrets(&stdout),
            stderr: redact_secrets(&stderr),
            truncated: out_cut || err_cut,
        })
    }

    pub fn cancel(&self, id: &str) -> bool {
        self.active
            .lock()
            .ok()
            .and_then(|mut active| active.remove(id))
            .is_some_and(|sender| sender.send(()).is_ok())
    }

    #[cfg(test)]
    pub fn active_count(&self) -> usize {
        self.active
            .lock()
            .map(|active| active.len())
            .unwrap_or_default()
    }
}

async fn read_bounded(
    mut reader: impl tokio::io::AsyncRead + Unpin,
) -> std::io::Result<(String, bool)> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let available = OUTPUT_LIMIT.saturating_sub(output.len());
        output.extend_from_slice(&buffer[..count.min(available)]);
        truncated |= count > available;
    }
    Ok((String::from_utf8_lossy(&output).into_owned(), truncated))
}

pub fn redact_secrets(input: &str) -> String {
    if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(input) {
        redact_json(&mut value);
        return value.to_string();
    }
    let mut redact_next = false;
    input
        .split_whitespace()
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            let sensitive = redact_next
                || lower.starts_with("sk-")
                || lower.starts_with("bearer:")
                || lower.starts_with("authorization:")
                || lower.contains("token=")
                || lower.contains("api_key=")
                || lower.contains("password=");
            redact_next = lower == "bearer"
                || lower.starts_with("bearer:")
                || lower.starts_with("authorization:");
            if sensitive {
                "[REDACTED]"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                let key = key.to_ascii_lowercase();
                if [
                    "token",
                    "authorization",
                    "api_key",
                    "apikey",
                    "secret",
                    "password",
                ]
                .iter()
                .any(|term| key.contains(term))
                {
                    *value = serde_json::Value::String("[REDACTED]".into());
                } else {
                    redact_json(value);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_json(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_redacted_from_diagnostics() {
        let text = redact_secrets("failed sk-secret token=very-secret-value safe");
        assert_eq!(text, "failed [REDACTED] [REDACTED] safe");
        assert_eq!(
            redact_secrets(r#"{"token":"secret","nested":{"password":"hidden"},"safe":1}"#),
            r#"{"nested":{"password":"[REDACTED]"},"safe":1,"token":"[REDACTED]"}"#
        );
    }

    #[tokio::test]
    async fn structured_command_captures_output() {
        let supervisor = ProcessSupervisor::default();
        #[cfg(windows)]
        let (executable, args) = ("cmd".into(), vec!["/C".into(), "echo hello".into()]);
        #[cfg(not(windows))]
        let (executable, args) = ("printf".into(), vec!["hello".into()]);
        let output = supervisor
            .run(
                "test",
                CommandSpec {
                    executable,
                    args,
                    cwd: None,
                    timeout: Duration::from_secs(5),
                    stdin: None,
                },
            )
            .await
            .expect("process output");
        assert_eq!(output.exit_code, Some(0));
        assert!(output.stdout.contains("hello"));
        assert_eq!(supervisor.active_count(), 0);
    }

    #[tokio::test]
    async fn timeout_terminates_child_process() {
        let supervisor = ProcessSupervisor::default();
        #[cfg(windows)]
        let (executable, args) = (
            "cmd".into(),
            vec!["/C".into(), "ping -n 6 127.0.0.1 >nul".into()],
        );
        #[cfg(not(windows))]
        let (executable, args) = ("sh".into(), vec!["-c".into(), "sleep 5".into()]);
        let result = supervisor
            .run(
                "timeout",
                CommandSpec {
                    executable,
                    args,
                    cwd: None,
                    timeout: Duration::from_millis(30),
                    stdin: None,
                },
            )
            .await;
        assert_eq!(result.unwrap_err(), ProviderFailure::Timeout);
        assert_eq!(supervisor.active_count(), 0);
    }

    #[tokio::test]
    async fn cancellation_terminates_child_process() {
        let supervisor = ProcessSupervisor::default();
        let runner = supervisor.clone();
        #[cfg(windows)]
        let (executable, args) = (
            "cmd".into(),
            vec!["/C".into(), "ping -n 6 127.0.0.1 >nul".into()],
        );
        #[cfg(not(windows))]
        let (executable, args) = ("sh".into(), vec!["-c".into(), "sleep 5".into()]);
        let task = tokio::spawn(async move {
            runner
                .run(
                    "cancel",
                    CommandSpec {
                        executable,
                        args,
                        cwd: None,
                        timeout: Duration::from_secs(10),
                        stdin: None,
                    },
                )
                .await
        });
        for _ in 0..50 {
            if supervisor.active_count() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(supervisor.cancel("cancel"));
        assert_eq!(task.await.unwrap().unwrap_err(), ProviderFailure::Cancelled);
    }
}
