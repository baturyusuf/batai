use std::process::{Command, Stdio};

use crate::domain::ProviderConnection;

fn output_text(output: &std::process::Output) -> String {
    let text = if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    String::from_utf8_lossy(text)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub fn probe(id: &str, name: &str, kind: &str, status_args: &[&str]) -> ProviderConnection {
    let result = Command::new(id)
        .args(status_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    match result {
        Ok(output) if output.status.success() => ProviderConnection {
            id: id.into(),
            name: name.into(),
            kind: kind.into(),
            status: "connected".into(),
            status_label: "Connected".into(),
            account_label: output_text(&output),
            detail: "Official CLI session is available".into(),
            action_label: "Check again".into(),
        },
        Ok(output) => ProviderConnection {
            id: id.into(),
            name: name.into(),
            kind: kind.into(),
            status: "action".into(),
            status_label: "Sign-in required".into(),
            account_label: "No active account".into(),
            detail: output_text(&output),
            action_label: "How to connect".into(),
        },
        Err(_) => ProviderConnection {
            id: id.into(),
            name: name.into(),
            kind: kind.into(),
            status: "unavailable".into(),
            status_label: "Not installed".into(),
            account_label: "Setup required".into(),
            detail: format!("{name} command was not found"),
            action_label: "Setup guide".into(),
        },
    }
}

pub fn probe_codex() -> ProviderConnection {
    let mut connection = probe("codex", "Codex", "Subscription", &["login", "status"]);
    if connection.status == "connected" {
        let capability = Command::new("codex")
            .args(["app-server", "--help"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if capability.is_ok_and(|status| status.success()) {
            connection.detail = "Official CLI session and App Server are available".into();
        } else {
            connection.status = "action".into();
            connection.status_label = "Update required".into();
            connection.detail =
                "Codex is connected, but this version does not expose App Server".into();
        }
    }
    connection
}

pub fn local_provider(id: &str, name: &str, kind: &str) -> ProviderConnection {
    let installed = Command::new(id)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .is_ok();
    let online = installed
        && Command::new(id)
            .arg("list")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
    ProviderConnection {
        id: id.into(),
        name: name.into(),
        kind: kind.into(),
        status: if online {
            "local"
        } else if installed {
            "action"
        } else {
            "unavailable"
        }
        .into(),
        status_label: if online {
            "Local runtime"
        } else if installed {
            "Runtime offline"
        } else {
            "Not installed"
        }
        .into(),
        account_label: if online {
            "This computer"
        } else if installed {
            "Start Ollama"
        } else {
            "Setup required"
        }
        .into(),
        detail: if online {
            "Local models can be used without an online account"
        } else if installed {
            "Ollama is installed but its local service is not responding"
        } else {
            "Install Ollama to use local models"
        }
        .into(),
        action_label: if online { "View setup" } else { "Setup guide" }.into(),
    }
}
