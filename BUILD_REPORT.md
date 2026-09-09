# Batai MVP Build Report

Build date: 2026-09-09

## Verification

- Core automated suite: **19/19 passed**
- JavaScript/ESM syntax checks: passed
- HTTP runtime smoke test: passed
- GOD inbox + Decision Ledger API smoke test: passed
- MCP initialize + tools/list smoke test: passed
- File-watcher task dispatch smoke test: passed
- Git worktree creation/binding test: passed
- Ollama adapter tested using a local fake HTTP endpoint

## Current working surface

```bash
npm test
npm start
```

Control plane: `http://127.0.0.1:4317`

Director MCP server:

```bash
BATAI_PROJECT_ROOT=/path/to/project npm run mcp
```

## External integrations requiring a developer machine

The current build environment does not have these binaries/accounts installed:

- `codex`
- `claude`
- `gh`
- Rust/Cargo

Therefore Codex/Claude authenticated sessions, GitHub CLI writes and the Tauri desktop binary remain integration steps, not mocked claims.

## Local Git snapshot

Initial implementation is committed on `main`. See `git log --oneline` after extracting the source package if `.git` is retained; the downloadable ZIP intentionally excludes Git internals for portability.
