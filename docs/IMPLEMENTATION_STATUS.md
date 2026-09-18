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
- Rust-native loopback HTTP control plane with legacy routes, typed validation, embedded UI, mutation token, Host/Origin defense and bounded bodies
- Official Rust SDK-based Director MCP stdio control plane with structured output and legacy tool names
- Per-project Rust daemon with one `BataiApplication`, one SQLite writer and concurrent Desktop/MCP/HTTP clients
- Versioned, project-bound authenticated local RPC with OS-vault credentials, actor isolation, bounded event fan-out and restart-persistent mutation deduplication
- Daemon-owned bounded Meeting Engine with persisted turns, independent parallel positions, conditional disagreement round, economic routing, hard token/participant ceilings, action-to-task links and conservative crash recovery
- Shared deterministic Context Builder with one bounded reader for every file-backed source, worktree-aware selection, hard input budgets, provenance, sensitive-file exclusion, participant-specific packages and freshness fingerprints for mutable records
- Event-driven execution gates with exact PAYG decision binding, policy/resource/context/routing revalidation at consumption, final pre-provider freshness barrier, stale-approval protection, rejection-safe rerouting and restart-safe at-most-once meeting continuation

Automated suites at the time of this snapshot: **205 passing Rust tests** and **44 passing Node compatibility/UI tests**.

## Implemented and smoke-tested at protocol/process level

- Rust Director MCP stdio bridge (`rmcp`), including 2025-11-25 initialize compatibility and current discover lifecycle
- Daemon-hosted Rust HTTP control plane API and Node-less binary smoke
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
- Bundled SQLite runtime with safe legacy-compatible migrations, WAL and foreign keys.
- Typed agent/task/resource/event/scheduler state and persist-first event engine.
- Rust task watcher, idempotent ingestion, dependency DAG/cycle handling and review gates.
- Per-agent execution serialization and persistent multi-agent task-run checkpoints.
- Provider-neutral execution/session abstraction with fingerprint-safe session recovery.
- Persistent quota state, durable deduplicated resume jobs and restart reconciliation.
- Runtime-backed agent/task/progress snapshots.
- Typed L0-L7 seniority, role/function catalog, explicit departments, reporting parents and backward-compatible legacy role parsing.
- Separate model/effective capability profiles and configurable function-specific seniority recommendations.
- Organization snapshot aggregation for reporting/workflow relationships, observable activity, task-run performance, per-agent/project usage and provider resources.
- Real SVG node-edge Organization Map with Hierarchy, Workflow and Combined modes, semantic edges, cycle/orphan safety, pan/zoom/fit/reset, search and explicit-field filters.
- Workspace, Task Board, Accounts, Resource Dashboard and tabbed Agent Inspector surfaces.
- Official connection guidance for Codex, Claude Code, GitHub and Ollama.
- Real Codex App Server transport with initialize, thread start/resume, turn streaming, interruption, exact live approval pause/resume and subscription usage telemetry.
- Supervised Claude Code JSON execution with named/resumable sessions, cancellation and normalized token/cost reporting.
- Ollama chat/tag HTTP adapter with persisted conversation context and local token counters; private thinking is discarded.
- Structured process supervision with bounded logs, timeout/cancellation, secret redaction and crash classification.
- Automatic task-scoped Git worktrees for coding roles, safe reuse/removal, dirty-state inspection and durable execution checkpoints.
- Conservative provider crash reconciliation: interrupted turns become `UNKNOWN_AFTER_CRASH` and require review instead of automatic duplicate execution.
- Restart-safe operation journal for cross-store organization and task/worktree metadata mutations, with durable phases, bounded preimages, fingerprints, forward completion, safe rollback and conflict-to-review behavior.
- Push-based Tauri runtime events with debounced snapshot refresh in the desktop UI.
- Typed Rust governance mutations with GOD/Director/Lead/Worker authority, scoped permissions, optimistic organization revisions and fail-closed policy checks.
- Task-scoped/project/permanent lifecycle, policy-aware Agent Factory, soft termination, one-Director and hierarchy invariants.
- Persistent collaboration/review/advisory relationships, external organization file watcher, append-only audit and restart-safe mutation-bound GOD decisions.
- Separate per-request provider approval ledger with official protocol identity, interactive `Allow once`/`Deny`, bounded expiry, headless fail-closed behavior, cancellation and restart orphaning.
- Explicit review outcomes, evidence-backed review acceptance metrics and a real bounded multi-agent Meeting Engine.
- Organization Edit mode, create-agent and relationship forms, explicit mutation confirmations, Decision Ledger, provider approval queue and policy Settings.
- Existing deterministic approval/recovery tests, explicit opt-in authenticated Codex smoke and real inference/worktree acceptance remain intact.
- Typed Economic Intelligence resources, billing/terms profiles, capability evidence and deterministic quota-aware routing with durable decision audit.
- Cross-platform CPU/RAM/disk profiling and structured NVIDIA VRAM/driver discovery with stable hardware fingerprinting.
- Ollama inventory/details/running-model/pull-progress/cancellation/removal APIs, conservative local catalog fit and an isolated deterministic benchmark suite.
- Kimi Code, Z.AI Coding Plan and MiniMax Token Plan subscription adapters over shared OpenAI-compatible transport, provider-specific verified model catalogs, error handling and operating-system vault credentials.
- Intelligence Portfolio, Local AI setup, model download/benchmark actions, role-specific recommendations and task/agent routing explanations in the desktop UI.
- Compact task-outcome evidence with quality/retry/failure classification, review/test validation, task-difficulty normalization and routing-decision correlation.
- Deterministic calibrated capability profiles with recency, sample/diversity confidence, per-source provenance, hardware-aware latency and disagreement-safe routing estimates.
- Versioned routing calibration, shadow rankings, no-inference offline replay, manual audited GOD evidence and governance-only promotion suggestions.
- Capability Evidence, Router Calibration, Routing History and replay views; connection smoke remains connectivity-only and Z.AI warning acknowledgement does not bypass terms eligibility.
- Rust-native GitHub delivery with official `gh` authentication/repository binding, typed issues/PRs/reviews/checks, issue-task links, isolated commit/push, PR idempotence, durable CI refresh, SHA-bound merge governance and remote saga recovery.
- Task delivery timeline and PR inspector with explicit merge controls; required delivery pauses task completion until a merged PR is observed.
- Node GitHub/worktree/HTTP/MCP code is no longer used by production entry points; remaining Node files are compatibility-test references documented in `NODE_RETIREMENT.md`.
- One hundred ninety-three Rust library tests, including meeting bounds/resource gates/recovery/task integration, HTTP security, official MCP lifecycle/tool calls, one-owner startup races, concurrent client access, actor isolation, event fan-out, recovery gating, authenticated shutdown and restart-persistent request deduplication.
- Manual UI smoke at 1440x900 and 1100x720 with no JavaScript console errors.

The external production control plane is Rust-native and daemon-owned. Closing Desktop or MCP no longer shuts down the project organization. Node remains only for compatibility fixtures and migration tests.

## Next engineering milestones

1. Run an opt-in longitudinal calibration pilot with reviewed production tasks, then tune thresholds through the suggestion/replay workflow.
2. Extend the shared Context Builder into task execution/review and add explicit meeting scheduling/queueing.
3. Add provider-specific observable activity adapters (reading/testing/tool use) without capturing private reasoning.
4. Install Claude Code and Ollama on a development host and run their authenticated/local opt-in end-to-end tests.
5. Add controlled daemon restart/upgrade handoff and an optional idle-exit policy that checks active runtime work before shutdown.
6. After one compatibility release, remove deprecated Node HTTP/MCP reference files and split the remaining Node migration suite from release packaging.
7. Add native editor/LSP/terminal integration beyond the workspace scaffold.
