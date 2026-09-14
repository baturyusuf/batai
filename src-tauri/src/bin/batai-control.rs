use std::path::PathBuf;

use batai::{
    daemon::{self, ClientOrigin, DaemonClient},
    project::discover_project_root,
};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Batai control plane failed: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let command = std::env::args().nth(1).unwrap_or_else(|| "help".into());
    if matches!(command.as_str(), "help" | "--help" | "-h") {
        eprintln!(
            "Batai Rust control plane\n\nUsage:\n  batai-control daemon\n  batai-control serve\n  batai-control mcp\n  batai-control stop [--force]\n\nEnvironment:\n  BATAI_PROJECT_ROOT\n  BATAI_PORT (daemon HTTP, default 4317)\n  BATAI_CONTROL_TOKEN (HTTP mutations)"
        );
        return Ok(());
    }
    let root = std::env::var_os("BATAI_PROJECT_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok().map(discover_project_root))
        .unwrap_or_else(|| PathBuf::from("."));
    match command.as_str() {
        "daemon" => daemon::run_daemon(root, configured_port()?).await,
        "serve" => {
            let client = DaemonClient::connect_or_spawn(&root, ClientOrigin::Desktop).await?;
            eprintln!("Batai daemon HTTP surface: {}", client.endpoint());
            let _ = tokio::signal::ctrl_c().await;
            Ok(())
        }
        "mcp" => {
            let client = DaemonClient::connect_or_spawn(&root, ClientOrigin::Mcp).await?;
            batai::mcp::run_stdio_bridge(client).await
        }
        "stop" => {
            let client = DaemonClient::connect(&root, ClientOrigin::Desktop).await?;
            let force = std::env::args().any(|argument| argument == "--force");
            let result = client
                .call_value("shutdown_daemon", serde_json::json!({"force":force}))
                .await?;
            eprintln!("Batai daemon shutdown accepted: {result}");
            Ok(())
        }
        other => Err(format!("unknown mode: {other}")),
    }
}

fn configured_port() -> Result<Option<u16>, String> {
    std::env::var("BATAI_PORT")
        .ok()
        .map(|value| {
            value
                .parse::<u16>()
                .map(Some)
                .map_err(|_| "BATAI_PORT must be a valid TCP port".to_string())
        })
        .unwrap_or(Ok(None))
}
