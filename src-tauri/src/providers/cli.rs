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

pub fn local_provider(id: &str, name: &str, kind: &str) -> ProviderConnection {
    let installed = Command::new(id)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .is_ok();
    ProviderConnection {
        id: id.into(),
        name: name.into(),
        kind: kind.into(),
        status: if installed { "local" } else { "unavailable" }.into(),
        status_label: if installed {
            "Local runtime"
        } else {
            "Not installed"
        }
        .into(),
        account_label: if installed {
            "This computer"
        } else {
            "Setup required"
        }
        .into(),
        detail: if installed {
            "Local models can be used without an online account"
        } else {
            "Install Ollama to use local models"
        }
        .into(),
        action_label: if installed {
            "View setup"
        } else {
            "Setup guide"
        }
        .into(),
    }
}
