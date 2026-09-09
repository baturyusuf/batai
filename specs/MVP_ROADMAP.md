# Batai MVP Roadmap

## Phase 0 — Skeleton
- Tauri + React app
- project open/create
- SQLite
- basic settings
- Git repository detection

## Phase 1 — Agent Runtime
- Agent Registry
- AgentConfig
- session lifecycle
- mock provider adapter
- chat UI

## Phase 2 — Task/Event Engine
- `.batai/tasks/*.json`
- schema validation
- file watcher
- runtime DB sync
- dependency resolution
- event bus
- automatic task dispatch

## Phase 3 — Git Worktree Manager
- create/remove worktree
- branch policy
- agent-worktree binding
- diff view
- commit/PR helpers

## Phase 4 — First Real Provider
- Codex adapter
- auth/session integration
- send/resume/cancel
- usage state if available

## Phase 5 — Second Provider
- Claude/ACP/CLI adapter
- provider capability matrix
- model selection

## Phase 6 — Director Tools
- create_agent
- assign_task
- create_dependency
- pause/resume
- request GOD decision
- read status/results

## Phase 7 — GOD Console
- authority router
- GOD review gate
- decision ledger
- natural-language intervention

## Phase 8 — Resource/Quota Manager
- WAITING_RESOURCE
- scheduler
- auto resume
- provider availability state

## Phase 9 — Project Memory
- directives
- journals
- ADR
- handoff reports
- context builder

## Phase 10 — GitHub Integration
- issue create/read/assign
- PR create
- CI state

## Phase 11 — Local Models
- Ollama adapter
- Director local model assignment
- fallback policy

## Phase 12 — QA / Hardening
- concurrency tests
- crash recovery
- state reconciliation
- security review
