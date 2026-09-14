# Rust external control plane

Batai has one Rust application kernel and exactly one runtime owner per project. The long-lived daemon opens `BataiApplication`; Tauri, loopback HTTP and Director MCP are concurrent clients of that daemon. Transport code validates and converts requests but never starts a watcher, scheduler, provider supervisor or second SQLite writer.

```text
Tauri client ───┐
HTTP /api/* ────┼─> Rust daemon ─> one BataiApplication ─> BataiRuntime
MCP bridge ─────┘
```

## Native modes

The desktop binary remains `batai`. External transports use the small `batai-control` binary:

```powershell
$env:BATAI_PROJECT_ROOT='C:\path\to\project'
cargo run --manifest-path src-tauri/Cargo.toml --bin batai-control -- serve
cargo run --manifest-path src-tauri/Cargo.toml --bin batai-control -- mcp
cargo run --manifest-path src-tauri/Cargo.toml --bin batai-control -- daemon
cargo run --manifest-path src-tauri/Cargo.toml --bin batai-control -- stop
```

`serve` and `mcp` attach to a compatible daemon or safely start one; neither constructs `BataiApplication`. `npm start`, `npm run mcp` and `npm run daemon` are developer conveniences for these Rust modes. The release control surfaces do not spawn Node. `BATAI_PORT` defaults to `4317`; HTTP always binds to `127.0.0.1` and refuses a non-loopback address.

## HTTP compatibility and security

The version-one compatibility routes remain:

- `GET /api/state`, `GET /api/providers`, `GET /api/health`
- `POST /api/agents`, `POST /api/tasks`, `POST /api/god/messages`
- `POST /api/decisions`, `POST /api/decisions/:id/resolve`
- `POST /api/tasks/:id/approve`

`/api/state` is built from the Rust application snapshot and adds legacy snake-case aliases rather than maintaining a second state builder. The embedded browser UI comes directly from `src/ui`; there is no copied frontend.

Security intentionally improves on the old Node server. Host and Origin must name a loopback authority, cross-origin preflight is rejected, JSON content type is required for body-bearing mutations, request bodies are limited to 1 MiB and static paths cannot traverse the embedded asset root. Mutation routes require `Authorization: Bearer <BATAI_CONTROL_TOKEN>`. If the environment variable is absent, the daemon generates a project-bound token in the operating-system credential vault, leaving external mutations locked unless an authorized local client is explicitly configured. Set the environment token before daemon startup when compatibility HTTP mutations are required. Temporary migration-only clients can explicitly set `BATAI_LEGACY_HTTP_MUTATIONS=1`; this is not the secure default. Tokens are not stored in `.batai`, SQLite, URLs, responses or logs.

Errors use a stable JSON envelope with `error`, safe `message` and `details`. Validation is `400`, authority denial `403`, stale revision `409`, missing entity/route `404`, oversized content `413`, and unexpected internal failures `500` without Rust debug output.

## MCP compatibility

The stdio implementation uses the official `modelcontextprotocol/rust-sdk` (`rmcp` 3.3). Typed request structs generate JSON Schema, structured content is returned with the SDK's text fallback and stdout is reserved for protocol frames. EOF stops only the thin MCP bridge; the project daemon and active tasks continue.

All eleven legacy tool names are preserved. Additive read-only tools expose tasks, resources, delivery, decisions and recovery state. MCP calls execute as `DIRECTOR`; protected agent/GitHub/provider operations still route through Rust governance and can yield a GOD decision. `batai_update_directives` uses the recoverable operation journal instead of writing directly.

Checked 2026-09-13 against the official SDK and MCP lifecycle documentation. The server supports the current `2026-07-28` discover lifecycle exposed by the SDK and retains the `2025-11-25` initialize negotiation used by existing clients. An initialize request cannot negotiate the newer discover-only lifecycle; the SDK correctly falls back to the newest handshake-based revision instead of echoing an unsupported version.

## Runtime ownership and local RPC

Only daemon mode takes the OS-backed exclusive lock on `.runtime/control-plane.lock`. The file records PID, mode and start time for diagnostics, but the OS lock is the authority. Concurrent client startup may spawn multiple contenders; exactly one obtains the lock and all clients attach to the winner.

Discovery uses the non-secret `.runtime/daemon.json` descriptor plus an authenticated, versioned handshake bound to a canonical-project fingerprint. Desktop and MCP have separate operating-system-vault credentials and the daemon assigns their actor identity. Mutations are claimed by stable RPC request ID before execution and completed results are durable, preventing automatic duplicate mutation after a lost response. Runtime events use bounded fan-out; a lagging client is told to refresh a snapshot.

See [Daemon Architecture](DAEMON_ARCHITECTURE.md) for lifecycle, reconnect, security and crash semantics.

## Compatibility fixtures

`specs/control-plane-contract-v1.json` freezes legacy routes, MCP names and intentional security differences. Rust tests validate HTTP behavior, typed SDK schemas/results, current/legacy MCP lifecycle, one-owner startup, concurrent Desktop/MCP/HTTP access, actor isolation, event fan-out and restart-persistent RPC deduplication. Node tests keep the old entry points as a migration oracle while confirming production scripts use Rust.
