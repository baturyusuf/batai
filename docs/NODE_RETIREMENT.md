# Node retirement inventory

The Rust/Tauri desktop no longer needs the Node GitHub wrapper for production delivery. Node is retained as an explicit compatibility server, MCP boundary and migration test specification until the remaining entry points have Rust parity. Files are not deleted merely because an equivalent exists.

| Node module | Status | Rust parity | Production desktop dependency |
|---|---|---|---|
| `core/github-cli-manager.mjs` | DEPRECATED | Typed issues, PRs, reviews, checks, merge and recovery in Rust | No |
| `core/git-worktree-manager.mjs` | COMPATIBILITY_TEST | Rust worktree/status/diff/commit/push lifecycle | No |
| `core/authority-controller.mjs` | COMPATIBILITY_TEST | Rust AuthorityRouter, Decision Ledger and audit | No |
| `core/task-engine.mjs` | COMPATIBILITY_TEST | Rust tasks, checkpoints, delivery sub-state and scheduler | No |
| `core/session-manager.mjs` | COMPATIBILITY_TEST | Rust provider sessions and recovery | No |
| `core/resource-manager.mjs` | COMPATIBILITY_TEST | Rust resource portfolio and quota scheduling | No |
| `core/director-tools.mjs` | STILL_REQUIRED | Most deterministic operations exist in Rust/Tauri; Node MCP still imports this module | No; yes for `npm run mcp` |
| `core/batai-runtime.mjs` | STILL_REQUIRED | Rust desktop runtime is primary; Node HTTP/MCP compatibility entry points still construct this runtime | No; yes for `npm start` and `npm run mcp` |
| `server.mjs` | STILL_REQUIRED | Compatibility HTTP server only | No |
| `control/mcp-server.mjs` | STILL_REQUIRED | Rust delivery commands exist, but the published Director MCP stdio process is still Node | No; yes for Node MCP |

## Removal gate

Node removal remains blocked until the public HTTP and MCP entry points are redirected to Rust (or formally retired), their compatibility fixtures are preserved, and no documented workflow calls `npm start` or `npm run mcp`. The desktop’s GitHub lifecycle, governance, tasks, sessions, resources and recovery are already Rust-native. `.batai` compatibility and the Node tests remain required during the transition.
