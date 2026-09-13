# Node retirement inventory

Desktop, HTTP and MCP production control surfaces are now Rust-native and share one `BataiApplication` kernel. Node is retained only as a compatibility-test oracle, deprecated reference implementation and legacy unit-test specification. Files are not deleted merely because an equivalent exists.

| Node module | Status | Rust parity | Production desktop dependency |
|---|---|---|---|
| `core/github-cli-manager.mjs` | DEPRECATED | Typed issues, PRs, reviews, checks, merge and recovery in Rust | No |
| `core/git-worktree-manager.mjs` | COMPATIBILITY_TEST | Rust worktree/status/diff/commit/push lifecycle | No |
| `core/authority-controller.mjs` | COMPATIBILITY_TEST | Rust AuthorityRouter, Decision Ledger and audit | No |
| `core/task-engine.mjs` | COMPATIBILITY_TEST | Rust tasks, checkpoints, delivery sub-state and scheduler | No |
| `core/session-manager.mjs` | COMPATIBILITY_TEST | Rust provider sessions and recovery | No |
| `core/resource-manager.mjs` | COMPATIBILITY_TEST | Rust resource portfolio and quota scheduling | No |
| `core/director-tools.mjs` | COMPATIBILITY_TEST | `BataiApplication` Director operations call Rust governance/tasks/worktrees/delivery | No |
| `core/batai-runtime.mjs` | COMPATIBILITY_TEST | Rust `BataiRuntime` is the only production runtime | No |
| `server.mjs` | DEPRECATED_REFERENCE | Rust loopback HTTP router, validation, embedded UI and shared snapshot | No |
| `control/mcp-server.mjs` | DEPRECATED_REFERENCE | Official Rust `rmcp` stdio server with legacy tool names | No |

## Removal gate

The production retirement gate is satisfied: the desktop, HTTP, MCP, GitHub, authority, tasks, sessions and resources are Rust-native; production scripts do not spawn Node; `.batai` remains compatible. `src/server.mjs` and `src/control/mcp-server.mjs` stay temporarily so Node compatibility tests can compare the frozen fixture and downstream users have a readable migration oracle. Their explicit `*:node-reference` scripts are not production paths. Removing the wider Node implementation and its migration tests is a separate cleanup after one compatibility release.
