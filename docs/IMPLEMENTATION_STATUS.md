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
- HTTP control-plane validation for malformed JSON and invalid payloads

Automated test suite: **23 passing tests** at the time of this snapshot.

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

## Rust and Tauri rewrite branch

- Tauri 2 desktop executable compiles on Windows.
- Rust project-state, weighted-progress and provider-connection modules.
- Organization-first Overview and Organization Map.
- Workspace, Task Board, Accounts and Agent Inspector surfaces.
- Official connection guidance for Codex, Claude Code, GitHub and Ollama.
- Six passing Rust unit tests.

The Node HTTP control plane remains as a compatibility layer until the Rust task, event, session and resource engines reach feature parity.

## Next engineering milestones

1. Install Codex CLI and exercise ChatGPT-authenticated sessions end-to-end.
2. Install Claude Code and exercise authenticated session/resume behavior.
3. Bind agent creation to persistent worktrees automatically when requested by policy.
4. Add richer provider-specific quota telemetry/reset extraction.
5. Add Agent-to-Agent meeting/message primitives on top of structured events.
6. Run real GitHub issue → task → PR workflow with authenticated `gh`.
7. Complete the Rust task/event/session/resource migration and remove the compatibility runtime.
8. Add native editor/LSP/terminal integration beyond the workspace scaffold.
9. Add crash/reconciliation tests for concurrent real provider processes.
