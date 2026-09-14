# Context Builder

Batai intelligence operations receive the minimum sufficient project context through one reusable Rust `ContextBuilder`. The first production consumer is the Meeting Engine; the contract also defines task execution, review, Director planning and handoff operation types.

## Selection model

The deterministic priority order is active project/GOD and participant directives; the current objective and linked task; relevant interfaces/contracts; accepted decisions and ADRs; explicit and task-referenced files; review/handoff summaries; and prior linked meeting outcomes. The builder does not crawl or dump the repository. Version 1 uses explicit references, task metadata, bounded ADR directories and a small reviewed architecture-path catalog. Embeddings and hidden evaluator calls are not used.

## Hard budgets

The backend enforces independent limits for estimated input tokens, file count, bytes per file, total bytes and historical artifacts. Default project ceilings are 6,000 estimated tokens, 12 files, 64 KiB per file, 192 KiB total and 8 historical artifacts. Callers may request smaller limits but cannot exceed project policy. Context estimates and actual provider input usage are separate metrics.

## Safety and provenance

Every included source records its type, ID/path, deterministic inclusion reason, size estimate, fingerprint and freshness metadata. Packages carry a base commit SHA and stable fingerprint. Persistent meeting state retains these compact references and hashes, not copied source bodies.

Paths are canonicalized inside the repository or task worktree. Parent traversal, absolute paths, symlink escape, binary/non-UTF-8 content and oversized files are denied or excluded. Conservative name rules exclude `.env`, private keys, credentials and secret/token stores. This is a safety filter, not a claim of perfect secret detection.

## Worktrees and participant views

When an agent has a task worktree, content and base SHA come from that worktree. Common task/directive evidence is shared. Deterministic role filters emphasize architecture sources for architects, implementation contracts for backend workers and policy/authentication surfaces for security workers. First-round packages never contain peer positions.

## Freshness and gated continuation

Relevant file fingerprints and base SHA make stale packages detectable. Context is rebuilt immediately before each provider-safe turn. Changed sources produce a new fingerprint and `CONTEXT_REBUILT` event.

PAYG uses a reusable typed execution gate bound to operation, resource, provider, model, policy revision, meeting configuration, token scope, routing decision and safe continuation phase. Resolution arrives through the daemon event engine. Approval resumes only before a provider turn; rejection reruns eligibility with PAYG disabled; an intervening policy change supersedes the still-open decision; stale resolved bindings are re-evaluated; and `UNKNOWN_AFTER_CRASH` turns are never replayed. Provider-operation approval remains a separate live gate.
