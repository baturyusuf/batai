# Rust external control plane

Batai has one Rust application kernel. The Tauri desktop, loopback HTTP server and Director MCP stdio server all open `BataiApplication`, which owns the same `BataiRuntime` services for tasks, governance, providers, resources, delivery, recovery and snapshots. Transport code validates and converts requests; it does not perform file, GitHub or provider mutations directly.

```text
Tauri commands ─┐
HTTP /api/* ────┼─> BataiApplication ─> BataiRuntime services
MCP stdio ──────┘
```

## Native modes

The desktop binary remains `batai`. External transports use the small `batai-control` binary:

```powershell
$env:BATAI_PROJECT_ROOT='C:\path\to\project'
cargo run --manifest-path src-tauri/Cargo.toml --bin batai-control -- serve
cargo run --manifest-path src-tauri/Cargo.toml --bin batai-control -- mcp
```

`npm start` and `npm run mcp` are developer conveniences that launch these Rust modes. The release control surfaces do not spawn Node. `BATAI_PORT` defaults to `4317`; HTTP always binds to `127.0.0.1` and refuses a non-loopback address.

## HTTP compatibility and security

The version-one compatibility routes remain:

- `GET /api/state`, `GET /api/providers`, `GET /api/health`
- `POST /api/agents`, `POST /api/tasks`, `POST /api/god/messages`
- `POST /api/decisions`, `POST /api/decisions/:id/resolve`
- `POST /api/tasks/:id/approve`

`/api/state` is built from the Rust application snapshot and adds legacy snake-case aliases rather than maintaining a second state builder. The embedded browser UI comes directly from `src/ui`; there is no copied frontend.

Security intentionally improves on the old Node server. Host and Origin must name a loopback authority, cross-origin preflight is rejected, JSON content type is required for body-bearing mutations, request bodies are limited to 1 MiB and static paths cannot traverse the embedded asset root. Mutation routes require `Authorization: Bearer <BATAI_CONTROL_TOKEN>`. If the environment variable is absent, a non-persisted random token locks mutations for that run. Temporary migration-only clients can explicitly set `BATAI_LEGACY_HTTP_MUTATIONS=1`; this is not the secure default. Tokens are not stored in `.batai`, SQLite, URLs, responses or logs.

Errors use a stable JSON envelope with `error`, safe `message` and `details`. Validation is `400`, authority denial `403`, stale revision `409`, missing entity/route `404`, oversized content `413`, and unexpected internal failures `500` without Rust debug output.

## MCP compatibility

The stdio implementation uses the official `modelcontextprotocol/rust-sdk` (`rmcp` 3.3). Typed request structs generate JSON Schema, structured content is returned with the SDK's text fallback, stdout is reserved for protocol frames, and EOF cleanly stops the server/runtime.

All eleven legacy tool names are preserved. Additive read-only tools expose tasks, resources, delivery, decisions and recovery state. MCP calls execute as `DIRECTOR`; protected agent/GitHub/provider operations still route through Rust governance and can yield a GOD decision. `batai_update_directives` uses the recoverable operation journal instead of writing directly.

Checked 2026-09-13 against the official SDK and MCP lifecycle documentation. The server supports the current `2026-07-28` discover lifecycle exposed by the SDK and retains the `2025-11-25` initialize negotiation used by existing clients. An initialize request cannot negotiate the newer discover-only lifecycle; the SDK correctly falls back to the newest handshake-based revision instead of echoing an unsupported version.

## Runtime ownership

Each mode takes an OS-backed exclusive lock on `.runtime/control-plane.lock`. The file records PID, mode and start time for diagnostics, but the OS lock is the authority. Process exit releases it. Batai never deletes a lock merely because its metadata looks old; a second desktop/HTTP/MCP writer for the same project is rejected. This is the smallest safe boundary today and leaves room for a future single daemon with multiple transport clients.

## Compatibility fixtures

`specs/control-plane-contract-v1.json` freezes legacy routes, MCP names and intentional security differences. Rust tests validate HTTP behavior, typed SDK schemas/results, current/legacy MCP lifecycle and project locking. Node tests keep the old entry points as a migration oracle while confirming production scripts use Rust.
