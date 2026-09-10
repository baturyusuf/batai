# Rust Rewrite

The Rust rewrite moves Batai from a Node-hosted control-plane prototype to a Tauri desktop application while preserving the repository-level .batai contract.

## Product direction

- Organization first, model second.
- Workspace and Organization Map are separate primary surfaces.
- Director remains the user's main conversational interface.
- Agent identity, role, authority and history remain stable when the intelligence provider changes.
- Provider authentication uses official CLI or local-runtime flows. Raw credentials are never stored by Batai.
- Mechanical state transitions remain deterministic and provider-neutral.

## Current Rust foundation

- Tauri 2 desktop executable.
- Provider-neutral Rust domain and project-state modules.
- Existing .batai agent and task readers.
- Weighted project progress calculation.
- Compatible GOD-to-Director message creation.
- Provider-specific Codex, Claude Code, GitHub and Ollama probes under src-tauri/src/providers.
- Account connection guides for official authentication paths.
- New three-column desktop UI with Overview, Workspace, Organization Map, Task Board, Accounts and Agent Inspector surfaces.

## Build

Requirements:

- Rust stable toolchain.
- Visual Studio Build Tools with the Desktop development with C++ workload on Windows.
- WebView2 runtime.

    cargo test --manifest-path src-tauri/Cargo.toml
    cargo build --manifest-path src-tauri/Cargo.toml

The legacy Node runtime remains in the branch during protocol migration. It provides compatibility and regression tests until the Rust task, event, session and resource engines reach feature parity.

## Next migration slices

1. Port runtime SQLite state and migrations to Rust.
2. Port task watcher, dependency evaluation and idempotent dispatch.
3. Port session and quota recovery behind provider-neutral traits.
4. Bind coding agents to worktrees automatically.
5. Add native project picker, terminal, diff and test panels.
6. Remove the Node runtime after parity and migration tests pass.
