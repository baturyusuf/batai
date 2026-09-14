# Per-project Rust daemon

Batai runs exactly one long-lived Rust runtime for each canonical project root. The daemon owns `BataiApplication`, SQLite writes, recovery, task and organization watchers, the scheduler, provider processes, resource monitoring, GitHub delivery and runtime events. Desktop, MCP and HTTP are concurrent control surfaces; they never create a second production runtime.

```text
                       Batai Rust daemon
                one BataiApplication / one writer
                              │
             authenticated loopback RPC + events
                 ┌────────────┼────────────┐
              Desktop      MCP bridge     HTTP API
              GOD actor     DIRECTOR      governed local
```

## Process and discovery model

`batai-control daemon` starts a foreground daemon. `batai-control serve`, `batai-control mcp` and the Tauri desktop first attach to an existing compatible daemon. If none answers, they start the same executable in daemon mode and wait with bounded backoff. The OS-backed exclusive lock at `.runtime/control-plane.lock` remains the authority during simultaneous startup; only one contender can become the runtime owner.

The daemon writes a non-secret `.runtime/daemon.json` descriptor after taking the lock. It contains the protocol version, package version, canonical-project fingerprint, PID, loopback endpoint and start time. The fingerprint is SHA-256 over the canonical path (case-normalized on Windows), so path aliases and projects cannot share an endpoint accidentally. Clients never trust descriptor presence alone: an authenticated handshake must confirm both protocol and project. A stale descriptor is replaced only by the process that acquired the project lock.

Version 1 uses authenticated loopback HTTP for internal RPC. This was selected over a split Windows named-pipe/Unix-domain-socket implementation to keep one cross-platform framing, cancellation and test surface. It is not treated as trusted merely because it is local:

- the listener is hard-bound to `127.0.0.1`;
- separate high-entropy credentials identify Desktop and MCP surfaces;
- credentials are held by the operating-system credential store, never the repository, `.batai`, SQLite, command line, URL or descriptor;
- every request is bound to the project fingerprint and IPC protocol version;
- bodies are limited to 1 MiB and errors/logs are redacted;
- desktop-only GOD operations and MCP Director operations are selected by the authenticated transport, not a caller-supplied actor field.

HTTP compatibility mutations keep their separate bearer-token and Host/Origin protections. Internal daemon credentials are not HTTP API credentials.

## Protocol and service boundary

The internal envelope includes protocol version, stable request ID, client ID, project fingerprint, method and typed JSON parameters. The handshake returns daemon/client compatibility, daemon version and runtime state. Exact protocol version matching is intentionally conservative for this first local protocol; incompatible clients receive a restart-required result rather than undefined parsing.

The allowlisted command surface maps to the same `ControlService` and Rust domain services used by the daemon. It includes snapshots, resources, governed organization/task operations, bounded meetings, delivery, provider approvals and the legacy Director MCP operations. Transports do not write files, run providers or open SQLite directly. Meeting turns continue when Desktop or MCP disconnects because only the daemon owns them.

Mutations use a durable request-deduplication record. A request ID is claimed as `APPLYING` before dispatch and replaced with its result after success. Repeating a completed request returns the stored result. If the daemon dies after the effect but before the result is stored, the surviving marker returns `RESPONSE_UNCERTAIN`; Batai does not replay the mutation blindly. Cross-store and remote effects still use the stronger existing recovery/remote-operation journals.

## Runtime and client lifecycle

The daemon exposes `RECOVERING` while startup reconciliation is in progress and rejects mutations with a retryable response until `READY`. Read-only health and handshake remain available. The desktop shows Starting, Connected, Recovering, Disconnected and Error states.

Runtime events are fanned out through bounded per-subscriber broadcast queues. A lagging client receives a resync indication instead of blocking task execution or another client. Desktop reconnect uses bounded exponential backoff and fetches a fresh snapshot after reconnection. MCP reports a lost mutation response as uncertain and never automatically replays it.

Closing Desktop or an MCP stdio session does not cancel tasks or shut down the daemon. The initial policy is deliberately conservative: an automatically launched daemon remains alive for the user session until explicitly terminated or the process exits. Foreground `batai-control daemon` supports graceful Ctrl+C shutdown; `batai-control stop` performs an authenticated graceful stop only when no active or review-required work exists. `batai-control stop --force` exists for an explicit user-confirmed emergency stop. Automatic idle shutdown and OS service installation are future policies; keeping the runtime alive protects scheduled work, pending approvals and provider processes.

On daemon crash, clients display Disconnected. A subsequent attach starts or reaches a replacement daemon, which takes the project lock, runs the existing operation/task/session/GitHub reconciliation and then becomes Ready. Provider children are owned only by the daemon. A desktop close is therefore not an execution cancellation, while a daemon crash follows the same conservative no-duplicate recovery rules as any runtime restart.

## Current boundary and future evolution

This is a per-project daemon, not a global multi-project service. Project A and Project B have independent fingerprints, descriptors, locks and daemon processes. The transport-neutral `ControlService` leaves room for a future global broker or OS-native IPC without changing task/governance/provider business logic. GitHub webhooks, Windows Service/systemd installation, remote TCP and multi-user authentication are intentionally outside this slice.
