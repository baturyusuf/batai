use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use batai::{
    application::{ApplicationMode, BataiApplication},
    http::{self, HttpState, DEFAULT_PORT},
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
            "Batai Rust control plane\n\nUsage:\n  batai-control serve\n  batai-control mcp\n\nEnvironment:\n  BATAI_PROJECT_ROOT\n  BATAI_PORT (serve, default 4317)\n  BATAI_CONTROL_TOKEN (serve mutations)"
        );
        return Ok(());
    }
    let mode = match command.as_str() {
        "serve" => ApplicationMode::Http,
        "mcp" => ApplicationMode::Mcp,
        other => return Err(format!("unknown mode: {other}")),
    };
    let root = std::env::var_os("BATAI_PROJECT_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok().map(discover_project_root))
        .unwrap_or_else(|| PathBuf::from("."));
    let application = BataiApplication::open(root, mode).map_err(|error| error.to_string())?;
    application
        .start()
        .await
        .map_err(|error| error.to_string())?;

    let result = match mode {
        ApplicationMode::Http => {
            let port = std::env::var("BATAI_PORT")
                .ok()
                .map(|value| value.parse::<u16>())
                .transpose()
                .map_err(|_| "BATAI_PORT must be a valid TCP port".to_string())?
                .unwrap_or(DEFAULT_PORT);
            let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
            let state = HttpState::generated(application.clone());
            eprintln!("Batai Rust HTTP control plane: http://{address}");
            if std::env::var_os("BATAI_CONTROL_TOKEN").is_none()
                && std::env::var("BATAI_LEGACY_HTTP_MUTATIONS").ok().as_deref() != Some("1")
            {
                eprintln!("HTTP mutations are locked for this run; set BATAI_CONTROL_TOKEN before startup to enable authenticated clients.");
            }
            http::serve(state, address, async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .map_err(|error| error.to_string())
        }
        ApplicationMode::Mcp => batai::mcp::run_stdio(application.clone()).await,
        ApplicationMode::Desktop => unreachable!("desktop uses the Tauri binary"),
    };
    let shutdown = application
        .shutdown()
        .await
        .map_err(|error| error.to_string());
    result.and(shutdown)
}
