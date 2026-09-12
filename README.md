# Batai

**Batai** is a Director-managed, cost-aware runtime for heterogeneous AI developer teams.

> The `codex/rust-rewrite` branch contains the Rust/Tauri desktop and deterministic execution runtime. The Node control plane remains temporarily as a compatibility/reference layer while real provider and remaining control-plane modules are migrated.

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
- GitHub CLI issue/PR adapter when `gh` is available
- MCP-compatible Director control server
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

Requires Node.js 22.5+.

```bash
npm test
npm start
```

Open:

```text
http://127.0.0.1:4317
```

The core currently has no npm runtime dependencies; it uses Node built-ins, including Node 22 SQLite.

## Run the Rust desktop

Requires the Rust stable toolchain, WebView2 and Visual Studio Build Tools with the C++ desktop workload on Windows.


```bash
npm run test:all
npm run desktop:dev
```

The Rust desktop ingests the existing `.batai` repository contract into SQLite, watches tasks, assigns coding agents safe task worktrees and executes Codex App Server, Claude Code or Ollama through provider-neutral sessions. State and usage updates stream into the UI without storing raw credentials. See [Provider Runtime](docs/PROVIDER_RUNTIME.md) for setup, security and diagnostics.

Run the authenticated, mutation-level Codex acceptance test only on an explicitly opted-in development machine:

```powershell
$env:BATAI_REAL_PROVIDER_E2E='codex'
cargo test --manifest-path src-tauri/Cargo.toml real_codex_turn_mutates_only_a_managed_disposable_worktree -- --nocapture
```

The test creates its own disposable Git repository and managed worktree; it verifies a real Codex turn, filesystem mutation, durable metadata and duplicate suppression without changing the Batai checkout.

## Director MCP server

```bash
BATAI_PROJECT_ROOT=/path/to/project npm run mcp
```

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
- GitHub lifecycle parity and some organizational-memory workflows still come from the Node compatibility layer.
- Runtime events expose observable execution trace, not hidden model reasoning. Fine-grained activities such as reading versus testing remain generic when a provider does not report them.
- Subscription quota remains Unknown when no official quota endpoint is available; plan prices are user-supplied reporting data and never hard-coded as routing truth.
- Offline replay can show a policy's alternative selection but cannot know whether that unexecuted resource would have succeeded or saved money; production exploration and automatic calibration are disabled.

See [Organization Model](docs/ORGANIZATION_MODEL.md), [Organization Governance](docs/GOVERNANCE.md), [Recovery Model](docs/RECOVERY_MODEL.md), `docs/IMPLEMENTATION_STATUS.md` and `specs/` for the remaining roadmap.
