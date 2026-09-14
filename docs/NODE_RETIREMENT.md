# Node retirement inventory

Desktop, HTTP and MCP production control surfaces are Rust-native clients of one per-project Rust daemon and one `BataiApplication`. Node is retained only as a compatibility-test oracle, deprecated reference implementation and legacy unit-test specification. Files are not deleted merely because an equivalent exists.

| Node module | Status | Rust parity | Production desktop dependency |
|---|---|---|---|
| `core/github-cli-manager.mjs` | DEPRECATED | Typed issues, PRs, reviews, checks, merge and recovery in Rust | No |
| `core/git-worktree-manager.mjs` | COMPATIBILITY_TEST | Rust worktree/status/diff/commit/push lifecycle | No |
| `core/authority-controller.mjs` | COMPATIBILITY_TEST | Rust AuthorityRouter, Decision Ledger and audit | No |
| `core/task-engine.mjs` | COMPATIBILITY_TEST | Rust tasks, checkpoints, delivery sub-state and scheduler | No |
| `core/session-manager.mjs` | COMPATIBILITY_TEST | Rust provider sessions and recovery | No |
| `core/resource-manager.mjs` | COMPATIBILITY_TEST | Rust resource portfolio and quota scheduling | No |
| `core/director-tools.mjs` | COMPATIBILITY_TEST | Daemon `ControlService` calls Rust governance/tasks/worktrees/delivery | No |
| `core/batai-runtime.mjs` | COMPATIBILITY_TEST | Daemon-owned Rust `BataiRuntime` is the only production runtime | No |
| `server.mjs` | DEPRECATED_REFERENCE | Daemon-hosted Rust loopback HTTP router, validation, embedded UI and shared snapshot | No |
| `control/mcp-server.mjs` | DEPRECATED_REFERENCE | Official Rust `rmcp` thin stdio bridge with legacy tool names | No |

## Removal gate

The production retirement gate is satisfied: the daemon, desktop, HTTP, MCP, GitHub, authority, tasks, sessions and resources are Rust-native; production scripts do not spawn Node; `.batai` remains compatible. `src/server.mjs` and `src/control/mcp-server.mjs` stay temporarily so Node compatibility tests can compare the frozen fixture and downstream users have a readable migration oracle. Their explicit `*:node-reference` scripts are not production paths. Removing the wider Node implementation and its migration tests is a separate cleanup after one compatibility release.

The runtime dependency graph is now:

```text
batai desktop ─┐
batai-control mcp ─┼─> batai-control daemon ─> Rust BataiApplication
HTTP clients ──┘

Node reference/test modules ──> no production edge
```
