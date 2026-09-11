# Rust Rewrite

The Rust rewrite moves Batai from a Node-hosted control-plane prototype to a Tauri desktop application while preserving the repository-level .batai contract.

## Product direction

- Organization first, model second.
- Workspace and Organization Map are separate primary surfaces.
- Director remains the user's main conversational interface.
- Agent identity, role, authority and history remain stable when the intelligence provider changes.
- Provider authentication uses official CLI or local-runtime flows. Raw credentials are never stored by Batai.
- Mechanical state transitions remain deterministic and provider-neutral.

## Current Rust runtime

- Tauri 2 desktop executable.
- Provider-neutral Rust domain and project-state modules.
- Bundled SQLite operational store at `.runtime/runtime.sqlite`, with WAL, foreign keys and idempotent schema migrations compatible with the Node tables.
- Typed agent, task, resource, scheduler-job, task-run and event states using the existing uppercase JSON contract.
- Typed organizational identity: L0-L7 seniority, function, explicit department, reporting parent and model-independent title generation.
- Unknown-safe model/effective capability profiles and configurable role-specific recommendation policy.
- Persist-first event engine with a broadcast boundary for later live Tauri subscriptions.
- `.batai/tasks/*.json` initial scan and cross-platform debounced watcher. Invalid or partially written JSON is isolated and reported without stopping the watcher.
- Content-hash/revision based task ingestion, protected terminal/running states and cycle-safe dependency evaluation.
- Per-agent execution locks: one agent is serialized while different agents can execute concurrently.
- Persistent per-agent task-run checkpoints, so successful members of a multi-agent task are not rerun after another member is rate-limited or after restart.
- Provider-neutral execution/session traits and normalized provider result/usage contracts.
- Real Codex App Server, Claude Code CLI and Ollama HTTP execution adapters.
- Structured process supervision with cancellation, timeouts, bounded/redacted diagnostics and conservative crash handling.
- Exact, bounded Codex App Server approval pause/resume with fail-closed headless behavior, per-request identity and restart orphaning.
- Durable cross-store operation journal with explicit phases, atomic file replacement, SQLite transactions, forward recovery, safe rollback and conflict review.
- Automatic managed Git worktrees for coding roles and enriched per-run Git/process checkpoints.
- Session persistence with provider/model/reasoning/worktree/config fingerprints.
- Persistent resource states and SQLite-backed, deduplicated resource-recheck jobs with restart reconciliation.
- Director review gate and deterministic downstream task activation.
- Runtime-backed organization snapshot for agent/task status, relationships, observable activity, performance, usage/resources and weighted progress.
- Weighted project progress calculation.
- Compatible GOD-to-Director message creation.
- Provider-specific execution and diagnostic modules under `src-tauri/src/providers`.
- Account connection guides for official authentication paths.
- New three-column desktop UI with a real SVG Organization graph, Hierarchy/Workflow/Combined modes, Resource Dashboard and tabbed Agent Inspector.
- Push-based `batai://runtime-event` updates from Rust to the Tauri UI.

## Build

Requirements:

- Rust stable toolchain.
- Visual Studio Build Tools with the Desktop development with C++ workload on Windows.
- WebView2 runtime.

    cargo fmt --check --manifest-path src-tauri/Cargo.toml
    cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
    cargo test --manifest-path src-tauri/Cargo.toml
    cargo build --manifest-path src-tauri/Cargo.toml

The legacy Node runtime remains in the branch during protocol migration. It provides compatibility and regression tests until the Rust task, event, session and resource engines reach feature parity.

The Rust suite currently contains 92 tests. GitHub Actions validates the 32-test Node compatibility suite on Linux and formatting, linting, tests and the desktop build on Windows. Real-provider smoke and mutation-level acceptance tests remain explicitly opt-in.

## Remaining migration slices

1. Exercise Claude Code and Ollama against installed authenticated/local runtimes.
2. Extend journal-backed operations to future GitHub lifecycle actions and add a guided recovery inspection surface.
3. Add native project picker, terminal, diff and test panels.
4. Implement meeting scheduling and safe observable summaries on the typed meeting foundation.
5. Remove the Node runtime only after real-provider and control-plane parity tests pass.
