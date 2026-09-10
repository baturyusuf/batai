# Provider Runtime

The Rust runtime executes agents through provider-neutral sessions and normalized results. Provider-specific protocol handling lives under `src-tauri/src/providers/`; task, event, checkpoint and resource state remain provider-neutral.

## Supported execution paths

| Provider | Transport | Session recovery | Cancellation | Usage source |
| --- | --- | --- | --- | --- |
| Codex | Official `codex app-server` JSONL protocol over stdio | Persisted App Server thread ID; `thread/resume` after restart | `turn/interrupt` | ChatGPT subscription rate-limit snapshot plus per-turn tokens |
| Claude Code | Official non-interactive CLI JSON output | Persisted CLI session ID; `--resume` | Supervised child-process termination | Per-result tokens/cost when exposed; quota remains unknown |
| Ollama | Local HTTP API (`/api/chat`, `/api/tags`) | Persisted conversation messages | Request/task cancellation boundary | Local token counters; no subscription cost |

There is no silent provider fallback. A task configured for Codex, Claude Code or Ollama remains on that provider unless an explicit policy or user action changes the agent configuration. This prevents duplicate mutations and unexpected billing.

## Normalized result contract

Every successful adapter result records:

- provider, model, agent, task, session and optional turn identity;
- completion state and user-facing summary;
- artifacts and changed files;
- normalized input, cached-input and output tokens when available;
- cost/currency only when the provider reports them;
- start/completion timestamps and execution mode;
- provider metadata that excludes credentials and private reasoning;
- session metadata required for deterministic resume.

Unavailable metrics are `null`, never guessed. Subscription quota, API/PAYG cost and local usage are distinct sources.

## Process supervision and crash safety

CLI processes are launched with structured executable/argument arrays, an explicit working directory, bounded stdout/stderr collection and a timeout. Active processes can be cancelled and sensitive-looking tokens are redacted from diagnostics.

Task-run checkpoints store provider/session/turn identity, managed worktree, branch, base and ending commits, process identity when available, changed files and execution state. If a provider disappears after it may have changed files, the run becomes `UNKNOWN_AFTER_CRASH` and the task moves to `REVIEW`. Batai does not automatically rerun that turn.

Codex App Server approval requests are never auto-accepted. The headless runtime responds with `cancel`/`decline`; the thread uses `workspace-write` with approval policy `never`, so access outside the assigned worktree is not escalated.

## Worktrees

Coding roles receive a task-scoped worktree under the repository sibling directory `.batai-worktrees/<repository>/`. Identifiers are validated against traversal, branches use `batai/<agent>/<task>`, and an existing valid binding is reused. Product and analyst roles do not receive worktrees automatically.

The manager exposes create/reuse, status, binary diff, commit, remove and prune operations. Removal verifies the resolved target is inside the managed root before invoking Git.

## Setup and diagnostics

### Codex

1. Install/update the official Codex CLI.
2. Run `codex login` and complete the official ChatGPT sign-in.
3. Verify `codex login status` and `codex app-server --help`.

The Accounts screen distinguishes authentication from App Server capability. Older connected CLIs are reported as requiring an update.

### Claude Code

1. Install Claude Code from Anthropic's official instructions.
2. Run `claude auth login` and verify `claude auth status`.

If the executable is missing, Batai reports setup required. Auth, rate-limit, malformed-response and process-crash failures are classified separately.

### Ollama

1. Install Ollama.
2. Start the local runtime with `ollama serve`.
3. Pull the model named in the agent configuration and verify it with `ollama list`.

Batai defaults to `http://127.0.0.1:11434/api`. The Accounts screen distinguishes installed-but-offline from ready.

## Tests

Run the deterministic suite:

```text
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
```

Authenticated tests are opt-in and do not run in ordinary CI:

```text
BATAI_REAL_PROVIDER_SMOKE=codex cargo test --manifest-path src-tauri/Cargo.toml authenticated_app_server_smoke_is_explicitly_opt_in -- --nocapture
```

The Codex smoke test starts the real App Server, creates a resumable thread and reads the authenticated rate-limit snapshot without running an inference turn.
