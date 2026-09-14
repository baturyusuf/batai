# Batai

**Batai** is a Director-managed, cost-aware runtime for heterogeneous AI developer teams.

> The `codex/rust-rewrite` branch contains a per-project Rust daemon with concurrent Tauri, HTTP and MCP clients. Node remains only as a compatibility-test reference.

The core idea is that the **Director AI operates the organization through Batai tools**. Batai itself stays deterministic wherever possible: task routing, dependencies, events, worktrees, quota waiting, session recovery and authority state should not spend LLM turns.

## Implemented MVP

- Director-controlled Agent Registry with lifecycle validation
- Agent Factory policy checks: active-agent limits, hierarchy depth, GOD approval for permanent agents
- Rust organization governance with typed mutations, authority/permission routing, optimistic revisions and append-only audit
- Task-scoped, project and permanent agent lifecycles with soft termination and history preservation
- Provider/model/reasoning configuration per agent
- Typed L0-L7 organizational seniority, function, department and reporting identity independent of model assignment
- Separate model/effective capability profiles with unknown-safe scores and function-specific assignment recommendations
- SQLite runtime state, durable cross-store operation journal and restart recovery
- Declarative JSON task queue under `.batai/tasks/`
- File watcher with debounce and idempotent dispatch
- Dependency graph and automatic downstream task triggering
- Director review gates
- Structured event stream
- Timestamped organizational journals, reports and handoffs
- Agent `ROLE.md` / `DIRECTIVES.md` files and directive-change events
- GOD inbox with authority `100`
- Decision Ledger with GOD-locked decisions
- Persistent non-reporting relationships and explicit review outcomes
- Resource states and `WAITING_RESOURCE` / automatic resume scheduling
- Persistent provider session metadata
- Git worktree manager
- Codex CLI adapter
- Claude Code CLI adapter
- Ollama local-model adapter
- Mock provider for deterministic tests
- Rust-native GitHub delivery through the official authenticated `gh` CLI: issue↔task links, isolated commit/push, PR/review/CI state, SHA-bound merge governance and restart reconciliation
- One long-lived Rust daemon per project with one SQLite/runtime owner and concurrent Desktop, MCP and HTTP clients
- Official Rust SDK-based MCP Director bridge and secure daemon-hosted loopback HTTP API
- Versioned, project-bound local RPC, OS-vault credentials, transport-assigned actors, bounded live-event fan-out and persistent mutation deduplication
- Bounded multi-agent meetings with independent first positions, conditional disagreement response, deterministic closure, economic/provider gates and restart-safe turn identity
- Live node-edge Organization Map with Hierarchy, Workflow and Combined modes, semantic edges, search, filters, pan/zoom and Agent Inspector
- Organization Edit mode, Agent Factory form, GOD decisions, exact live provider approvals, recovery queue and project policy settings
- Resource Dashboard with provider health, usage source, quota, token and provider-reported cost summaries
- Economic Intelligence Portfolio with typed resource tiers/billing, hardware-aware local model catalog, secure subscription resources and deterministic quota-aware routing
- Windows-first hardware profiling, NVIDIA VRAM discovery, explicit Ollama pull/model management and hardware-bound local capability benchmarks
- Official OpenAI-compatible Kimi Code, Z.AI Coding Plan and MiniMax Token Plan adapters with OS-vault credentials and provider-specific terms/model catalogs
- Evidence-calibrated capability profiles from validated real task outcomes, with confidence, provenance, disagreement detection and failure classification
- Versioned confidence-aware routing, shadow rankings, calibration metrics and no-inference offline policy replay with unknown counterfactuals
- Tauri 2 desktop application backed by the Rust runtime
- Rust bundled-SQLite migrations, typed runtime state and persist-first events
- Rust task watcher, dependency DAG, idempotent dispatch and per-agent serialization
- Persistent multi-agent checkpoints, session fingerprints and durable quota recovery jobs

## Run the tested control plane

The production control plane requires Rust; Node.js 22.5+ is needed only for the compatibility test suite.

```bash
npm test
npm start
```

Open:

```text
http://127.0.0.1:4317
```

`npm start` is a convenience alias for `batai-control serve`. It attaches to an existing project daemon or starts one safely. Mutations require a bearer token supplied through `BATAI_CONTROL_TOKEN`; read-only health/state remain loopback-local. Direct native usage is documented in [Control Plane](docs/CONTROL_PLANE.md) and the ownership model in [Daemon Architecture](docs/DAEMON_ARCHITECTURE.md).

Closing the browser, Desktop or MCP bridge leaves the organization running. Use `npm run daemon:stop` (or native `batai-control stop`) for an authenticated graceful stop when the runtime is idle.

## Run the Rust desktop

Requires the Rust stable toolchain, WebView2 and Visual Studio Build Tools with the C++ desktop workload on Windows.


```bash
npm run test:all
npm run desktop:dev
```

The Rust desktop attaches to the project daemon rather than opening SQLite or starting watchers/providers itself. The daemon ingests the existing `.batai` repository contract, watches tasks, assigns coding agents safe task worktrees and executes Codex App Server, Claude Code or Ollama through provider-neutral sessions. Closing the UI does not cancel the organization. State and usage updates stream back to reconnecting clients without storing raw credentials. See [Provider Runtime](docs/PROVIDER_RUNTIME.md) and [GitHub Delivery](docs/GITHUB_DELIVERY.md).

Run the authenticated, mutation-level Codex acceptance test only on an explicitly opted-in development machine:

```powershell
$env:BATAI_REAL_PROVIDER_E2E='codex'
cargo test --manifest-path src-tauri/Cargo.toml real_codex_turn_mutates_only_a_managed_disposable_worktree -- --nocapture
```

The test creates its own disposable Git repository and managed worktree; it verifies a real Codex turn, filesystem mutation, durable metadata and duplicate suppression without changing the Batai checkout.

## Director MCP server

```bash
BATAI_PROJECT_ROOT=/path/to/project cargo run --quiet --manifest-path src-tauri/Cargo.toml --bin batai-control -- mcp
```

For npm-based development use `npm run --silent mcp`; MCP clients should launch the native `batai-control mcp` binary directly so stdout contains protocol frames only. The MCP process is a thin Director bridge to the same daemon used by Desktop and HTTP; closing it does not stop active work.

Director tools currently include:

- `batai_create_agent`
- `batai_assign_task`
- `batai_read_project_state`
- `batai_approve_task`
- `batai_create_worktree`
- `batai_update_directives`
- `batai_resume_agent`
- `batai_create_github_issue`
- `batai_read_director_inbox`
- `batai_acknowledge_god_message`
- `batai_request_god_decision`
- `batai_create_meeting`
- `batai_get_meeting`
- `batai_list_meetings`
- `batai_cancel_meeting`

The bridge uses the official Rust MCP SDK, supports current discovery plus 2025-11-25 initialize compatibility, and executes as Director through Rust authority checks. It does not construct a Node or second Rust runtime.

Meetings are deliberately bounded coordination primitives, not chat rooms. See [Meetings](docs/MEETINGS.md) for limits, economic routing, authority, recovery and the live Organization Map overlay.

## Project model

```text
GOD (human)
   │
   ▼
Director AI
   │  Batai MCP/control tools
   ▼
Batai deterministic control plane
   │
   ├─ Codex agent/session
   ├─ Claude agent/session
   ├─ Ollama/local agent
   └─ other adapters
```

Each coding agent can be assigned its own Git worktree. Director chooses the minimum sufficient model/reasoning level for a task and can use local, subscription-backed or API-backed adapters according to project policy.

New `AUTO` agents use the [Economic Router](docs/ECONOMIC_ROUTING.md) before dispatch. Existing explicit agents are not migrated or silently rerouted. See [Local Models](docs/LOCAL_MODELS.md) for hardware detection and benchmark semantics, and [Capability Learning](docs/CAPABILITY_LEARNING.md) for real-task evidence, confidence, calibration and replay boundaries.

## Declarative coordination example

Director writes a task such as:

```json
{
  "id": "TASK-248",
  "created_by": "director",
  "objective": "Resolve authentication regression",
  "assigned_to": ["agent8", "agent11"],
  "dependencies": [],
  "acceptance_criteria": ["All auth tests pass"],
  "inputs": ["github_issue:184"],
  "outputs": ["implementation-summary.json"],
  "status": "READY",
  "execution": {
    "parallel": true,
    "requires_director_review": true
  },
  "on_success": {
    "notify": "director"
  },
  "on_failure": {
    "notify": "director"
  }
}
```

Batai detects the file, validates it, resolves dependencies and wakes the assigned sessions without another LLM being used as a scheduler.

## Repository layout

```text
src/
  core/         deterministic orchestration and state
  providers/    Codex / Claude / Ollama / mock adapters
  control/      Director MCP server
  ui/           control-plane web UI
src-tauri/      Tauri desktop source scaffold
specs/          product/architecture contracts
schemas/        machine-readable protocol schemas
tests/          orchestration/provider tests
.batai/         declarative organization/project state
```

## Current limitations

- Claude Code and Ollama require their local executables/services and have not been authenticated on every CI host; their deterministic contracts are tested without credentials.
- Crash recovery preserves provider/session/worktree evidence and requires review after an uncertain provider mutation; it does not reattach to an independently surviving OS process.
- Interactive Codex approvals pause the exact live request for bounded `Allow once`/`Deny` input. Headless, unsafe, expired and restarted requests fail closed; no blanket authorization is cached.
- Cross-store organization mutations use a durable operation journal with forward completion, safe rollback and fingerprint-conflict review. Batai does not claim distributed ACID across the filesystem and SQLite.
- Deprecated Node HTTP/MCP files remain only as compatibility-test references; production scripts and release binaries use Rust. See [Control Plane](docs/CONTROL_PLANE.md) and [Node Retirement](docs/NODE_RETIREMENT.md).
- Runtime events expose observable execution trace, not hidden model reasoning. Fine-grained activities such as reading versus testing remain generic when a provider does not report them.
- Subscription quota remains Unknown when no official quota endpoint is available; plan prices are user-supplied reporting data and never hard-coded as routing truth.
- Offline replay can show a policy's alternative selection but cannot know whether that unexecuted resource would have succeeded or saved money; production exploration and automatic calibration are disabled.

See [Organization Model](docs/ORGANIZATION_MODEL.md), [Organization Governance](docs/GOVERNANCE.md), [Recovery Model](docs/RECOVERY_MODEL.md), `docs/IMPLEMENTATION_STATUS.md` and `specs/` for the remaining roadmap.
