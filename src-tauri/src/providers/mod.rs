mod cli;

use crate::domain::{ConnectionGuide, ProviderConnection};

pub fn probe_all() -> Vec<ProviderConnection> {
    vec![
        cli::probe("codex", "Codex", "Subscription", &["login", "status"]),
        cli::probe("claude", "Claude Code", "Subscription", &["auth", "status"]),
        cli::probe("gh", "GitHub", "Account", &["auth", "status"]),
        cli::local_provider("ollama", "Ollama", "Local runtime"),
    ]
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
        _ => return None,
    };
    Some(guide)
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
}
