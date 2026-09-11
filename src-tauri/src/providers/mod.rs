pub mod claude_code;
mod cli;
pub mod codex_app_server;
pub mod kimi_code;
pub mod minimax_token;
pub mod ollama;
pub mod openai_compatible;
pub mod process_supervisor;
pub mod zai_coding;

use crate::domain::{ConnectionGuide, ProviderConnection};
use crate::runtime::credentials::{CredentialStore, DefaultCredentialStore};

pub fn probe_all() -> Vec<ProviderConnection> {
    vec![
        cli::probe_codex(),
        cli::probe("claude", "Claude Code", "Subscription", &["auth", "status"]),
        cli::probe("gh", "GitHub", "Account", &["auth", "status"]),
        cli::local_provider("ollama", "Ollama", "Local runtime"),
        secure_subscription("kimi-personal-membership", "Kimi Code"),
        secure_subscription("zai-coding-plan", "Z.AI Coding Plan"),
        secure_subscription("minimax-token-plan", "MiniMax Token Plan"),
    ]
}

fn secure_subscription(id: &str, name: &str) -> ProviderConnection {
    let connected = DefaultCredentialStore::default()
        .get(id)
        .ok()
        .flatten()
        .is_some();
    ProviderConnection {
        id: id.into(),
        name: name.into(),
        kind: "Subscription quota".into(),
        status: if connected {
            "connected"
        } else {
            "disconnected"
        }
        .into(),
        status_label: if connected {
            "Connected"
        } else {
            "Not connected"
        }
        .into(),
        account_label: "Secure key".into(),
        detail: if connected {
            "Credential available in the operating-system secure store"
        } else {
            "Add the official subscription API key; it is never stored in project data"
        }
        .into(),
        action_label: if connected { "Manage" } else { "Connect" }.into(),
    }
}

pub fn connection_guide(provider_id: &str) -> Option<ConnectionGuide> {
    let guide = match provider_id {
        "codex" => ConnectionGuide {
            provider_id: provider_id.into(), title: "Connect Codex".into(),
            description: "Use the official Codex CLI login flow. Batai stores only connection metadata, never your token.".into(),
            command: Some("codex login".into()),
            steps: vec!["Open a terminal".into(), "Run the login command".into(), "Complete the official ChatGPT sign-in".into(), "Return to Batai and check again".into()],
        },
        "claude" => ConnectionGuide {
            provider_id: provider_id.into(), title: "Connect Claude Code".into(),
            description: "Authenticate through the official Claude Code CLI flow.".into(),
            command: Some("claude auth login".into()),
            steps: vec!["Install Claude Code if needed".into(), "Run the login command".into(), "Finish authentication in the official flow".into(), "Return and check again".into()],
        },
        "gh" => ConnectionGuide {
            provider_id: provider_id.into(), title: "Connect GitHub".into(),
            description: "GitHub is used for repository, issue and pull request workflows.".into(),
            command: Some("gh auth login".into()),
            steps: vec!["Open a terminal".into(), "Run the GitHub login command".into(), "Choose GitHub.com and HTTPS".into(), "Return and check again".into()],
        },
        "ollama" => ConnectionGuide {
            provider_id: provider_id.into(), title: "Start local models".into(),
            description: "Ollama runs models on your computer and does not require an online account.".into(),
            command: Some("ollama serve".into()),
            steps: vec!["Install Ollama".into(), "Start the local runtime".into(), "Download a model".into(), "Return and check again".into()],
        },
        "kimi-personal-membership" => api_key_guide(provider_id, "Connect Kimi Code", "Create an official Kimi Code membership API key. Batai stores it in the operating-system credential vault."),
        "zai-coding-plan" => api_key_guide(provider_id, "Connect Z.AI Coding Plan", "Use a Coding Plan key only. Batai marks use terms as requiring review until your plan authorizes this client."),
        "minimax-token-plan" => api_key_guide(provider_id, "Connect MiniMax Token Plan", "Use the Token Plan key, not a PAYG key. Batai stores it in the operating-system credential vault."),
        _ => return None,
    };
    Some(guide)
}

fn api_key_guide(provider_id: &str, title: &str, description: &str) -> ConnectionGuide {
    ConnectionGuide {
        provider_id: provider_id.into(),
        title: title.into(),
        description: description.into(),
        command: None,
        steps: vec![
            "Create a key in the provider's official subscription console".into(),
            "Paste it into Batai's secure connection dialog".into(),
            "Run the connection diagnostic".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_guides_only_use_official_local_flows() {
        let codex = connection_guide("codex").expect("Codex guide");
        assert_eq!(codex.command.as_deref(), Some("codex login"));
        assert!(codex.description.contains("official"));
        assert!(connection_guide("unknown").is_none());
    }

    #[tokio::test]
    async fn subscription_provider_smoke_is_explicitly_opt_in() {
        use crate::runtime::{
            credentials::EnvironmentCredentialStore,
            execution_provider::ExecutionProvider,
            types::{Agent, Task},
        };
        use std::sync::Arc;
        let Ok(selected) = std::env::var("BATAI_REAL_PROVIDER_SMOKE") else {
            return;
        };
        let credentials = Arc::new(EnvironmentCredentialStore::default());
        let (provider, provider_name, model): (Arc<dyn ExecutionProvider>, &str, &str) =
            match selected.as_str() {
                "kimi" => (
                    Arc::new(kimi_code::provider(credentials)),
                    "kimi-code",
                    kimi_code::MODELS[0],
                ),
                "zai" => (
                    Arc::new(zai_coding::provider(credentials)),
                    "zai-coding",
                    zai_coding::MODELS[0],
                ),
                "minimax" => (
                    Arc::new(minimax_token::provider(credentials)),
                    "minimax-token",
                    minimax_token::MODELS[0],
                ),
                _ => return,
            };
        let agent: Agent = serde_json::from_value(
            serde_json::json!({"id":"smoke","name":"Smoke","provider":provider_name,"model":model}),
        )
        .unwrap();
        let task:Task=serde_json::from_value(serde_json::json!({"id":"smoke","created_by":"god","objective":"Reply with exactly BATAI_OK and perform no tool or filesystem action.","assigned_to":["smoke"]})).unwrap();
        let session = provider
            .create_session(&agent)
            .await
            .expect("create session");
        let result = provider
            .send_task(&session.id, &agent, &task)
            .await
            .expect("real subscription inference");
        assert!(result.summary.contains("BATAI_OK"));
    }
}
