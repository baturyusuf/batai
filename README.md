# Batai

**Batai** is a Director-managed, cost-aware runtime for heterogeneous AI developer teams.

> The `codex/rust-rewrite` branch contains the new Rust/Tauri desktop foundation and organization-first interface. The Node control plane remains temporarily as a compatibility layer while runtime modules are migrated.

The core idea is that the **Director AI operates the organization through Batai tools**. Batai itself stays deterministic wherever possible: task routing, dependencies, events, worktrees, quota waiting, session recovery and authority state should not spend LLM turns.

## Implemented MVP

- Director-controlled Agent Registry with lifecycle validation
- Agent Factory policy checks: active-agent limits, hierarchy depth, GOD approval for permanent agents
- Provider/model/reasoning configuration per agent
- SQLite runtime state and restart recovery
- Declarative JSON task queue under `.batai/tasks/`
- File watcher with debounce and idempotent dispatch
- Dependency graph and automatic downstream task triggering
- Director review gates
- Structured event stream
- Timestamped organizational journals, reports and handoffs
- Agent `ROLE.md` / `DIRECTIVES.md` files and directive-change events
- GOD inbox with authority `100`
- Decision Ledger with GOD-locked decisions
- Resource states and `WAITING_RESOURCE` / automatic resume scheduling
- Persistent provider session metadata
- Git worktree manager
- Codex CLI adapter
- Claude Code CLI adapter
- Ollama local-model adapter
- Mock provider for deterministic tests
- GitHub CLI issue/PR adapter when `gh` is available
- MCP-compatible Director control server
- Browser control plane with organization, tasks, providers, GOD Console and Decision Ledger
- Tauri 2 desktop source scaffold

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

The Rust desktop reads the existing `.batai` repository contract, shows agents by organizational role, exposes weighted task progress and checks official Codex, Claude Code, GitHub and Ollama connection states without storing raw credentials.

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

## Current environment limitations

The implementation has automated tests and a runnable Node control plane. In the build environment used to create this repository:

- Rust/Cargo is not installed, so the Tauri shell could not be compiled.
- `codex`, `claude` and `gh` binaries are not installed, so their authenticated end-to-end sessions could not be exercised here.
- Ollama integration is tested through a local fake HTTP server; a real Ollama daemon can be used on a developer machine.

See `docs/IMPLEMENTATION_STATUS.md` and `specs/` for the remaining roadmap.
