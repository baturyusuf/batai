# Implementation Status

## Implemented and automatically tested

- SQLite runtime state + restart recovery
- Agent Registry and lifecycle state machine
- Policy Engine for active-agent limits, hierarchy depth and permanent-agent GOD approval
- Director deterministic tools
- Task JSON ingestion and filesystem watcher
- Dependency graph and automatic downstream execution
- Director review gate
- Idempotent dispatch
- Event Engine
- Agent config/directive watcher
- Persistent provider-session metadata
- Resource/quota parking and scheduled resume primitives
- Organizational journals, reports and handoffs
- GOD authority inbox
- GOD Decision Ledger
- Git worktree creation/removal
- Mock provider
- Ollama provider adapter
- Quota reset parser
- Web control plane

Automated test suite: **19 passing tests** at the time of this snapshot.

## Implemented and smoke-tested at protocol/process level

- Director MCP-compatible stdio control server
- HTTP control plane API
- GOD Console API
- Decision Ledger API

## Implemented; requires provider binary/account for authenticated integration test

### Codex CLI adapter
- `codex login status`
- non-interactive `codex exec`
- cwd-scoped `codex exec resume --last`
- model selection
- reasoning config hook
- workspace-write sandbox

### Claude Code adapter
- `claude auth status`
- print-mode JSON output
- model and effort selection
- named session + resume

### GitHub CLI adapter
- issue creation/read
- pull-request creation

## Scaffolded, not compiled in this environment

- Tauri 2 desktop shell (`src-tauri/`)

Rust/Cargo is not installed in the current build environment. The working user surface in this snapshot is the Node HTTP control plane; the Tauri shell is source-only.

## Next engineering milestones

1. Install Codex CLI and exercise ChatGPT-authenticated sessions end-to-end.
2. Install Claude Code and exercise authenticated session/resume behavior.
3. Bind agent creation to persistent worktrees automatically when requested by policy.
4. Add richer provider-specific quota telemetry/reset extraction.
5. Add Agent-to-Agent meeting/message primitives on top of structured events.
6. Run real GitHub issue → task → PR workflow with authenticated `gh`.
7. Compile Tauri shell and bridge Tauri commands to the runtime.
8. Add editor/LSP/terminal integration beyond the control-plane UI.
9. Add crash/reconciliation tests for concurrent real provider processes.
