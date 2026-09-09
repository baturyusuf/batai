# Codex Bootstrap Prompt

You are implementing **Batai**, a desktop AI developer organization runtime.

Before writing code:

1. Read every file in this specification pack.
2. Treat `MASTER_SPEC.md` as the primary product contract.
3. Treat JSON schemas under `schemas/` as authoritative protocol definitions.
4. Do not invent product scope beyond these documents.
5. If a technical choice is not specified and does not affect product behavior, choose the simplest robust implementation and record it as an ADR.
6. Do not add academic research, legal analysis, market research or unrelated features.
7. Prefer deterministic code over LLM calls wherever possible.
8. Build incrementally and run tests after every meaningful module.

## First implementation goal

Implement Phase 0–2 only:

### Phase 0
- Tauri 2 + React + TypeScript shell
- project open/create
- SQLite runtime database
- repository detection

### Phase 1
- Agent Registry
- AgentConfig model
- mock provider adapter
- agent lifecycle state machine
- basic agent list UI

### Phase 2
- `.batai/tasks/*.json`
- JSON schema validation
- filesystem watcher
- runtime DB synchronization
- dependency resolution
- event engine
- automatic dispatch to mock agent
- completion event handling

## Required tests

- invalid task JSON rejected
- valid task persisted to DB
- dependency-blocked task does not start
- dependency completion unblocks next task
- agent status transitions are valid
- duplicate filesystem events do not duplicate task execution
- app restart reconstructs runtime state safely

## Engineering constraints

- Keep core domain logic independent of UI.
- Use interfaces/traits for provider adapters.
- Never hard-code provider-specific logic into AgentManager.
- No production API integration yet.
- No browser automation.
- No full IDE features yet.
- No GitHub integration yet.
- No local LLM integration yet.
- No quota manager yet.

When complete, produce:
1. implementation summary
2. tests run/results
3. created ADRs
4. known limitations
5. recommended next task
