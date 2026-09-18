# Context Builder

Batai intelligence operations receive the minimum sufficient project context through one reusable Rust `ContextBuilder`. The first production consumer is the Meeting Engine; the contract also defines task execution, review, Director planning and handoff operation types.

## Selection model

The deterministic priority order is active project/GOD and participant directives; the current objective and linked task; relevant interfaces/contracts; accepted decisions and ADRs; explicit and task-referenced files; review/handoff summaries; and prior linked meeting outcomes. The builder does not crawl or dump the repository. Version 1 uses explicit references, task metadata, bounded ADR directories and a small reviewed architecture-path catalog. Embeddings and hidden evaluator calls are not used.

## Hard budgets

The backend enforces independent limits for estimated input tokens, file count, bytes per file, total bytes and historical artifacts. Default project ceilings are 6,000 estimated tokens, 12 files, 64 KiB per file, 192 KiB total and 8 historical artifacts. Callers may request smaller limits but cannot exceed project policy. Context estimates and actual provider input usage are separate metrics.

## Safety and provenance

Every included source records its type, ID/path, deterministic inclusion reason, size estimate, fingerprint and freshness metadata. Packages carry a base commit SHA and stable fingerprint. Persistent meeting state retains these compact references and hashes, not copied source bodies.

Every file-backed source (explicit files, interfaces, directives, ADRs and handoffs) goes through the same bounded reader. It canonicalizes the selected root, rejects parent traversal and absolute paths, verifies symlink containment, checks regular-file type and the per-file byte limit, and rejects binary/non-UTF-8 content. Optional artifacts become typed exclusions/warnings; explicitly required files block the package when they are unsafe or missing. Conservative name rules exclude `.env`, private keys, credentials and secret/token stores. This is a safety filter, not a claim of perfect secret detection.

## Worktrees and participant views

When an agent has a task worktree, content and base SHA come from that worktree. A non-root worktree must be a Batai-managed Git worktree for the requesting agent and (when linked) the exact task branch; arbitrary directories and another repository are rejected. The project root remains valid for operations that do not require isolation. Common task/directive evidence is shared. Deterministic role filters emphasize architecture sources for architects, implementation contracts for backend workers and policy/authentication surfaces for security workers. First-round packages never contain peer positions.

## Freshness and gated continuation

All included mutable sources are re-read through the same bounded reader during freshness checks: task/decision/review/meeting records are re-serialized deterministically and file-backed sources are re-hashed. The recorded Git base SHA is an additional conservative barrier, not a replacement for working-tree fingerprints. Context is checked after resource acquisition and again immediately before `provider.send_task`; a bounded rebuild/reroute is attempted, and unstable or changed routing is parked without inference. Changed sources produce a new fingerprint and `CONTEXT_REBUILT` event. The fingerprint is deterministic for the package contents; `context_id` is an opaque per-build identifier used for traceability.

PAYG uses a reusable typed execution gate bound to operation, resource, provider, model, policy revision, meeting configuration, context fingerprint, token scope, routing decision and safe continuation phase. At consumption time Batai revalidates the exact decision status, current policy, resource billing/status/model, meeting configuration, context and routing binding; stale approvals are superseded rather than consumed. Resolution arrives through the daemon event engine. Approval resumes only before a provider turn; rejection reruns eligibility with PAYG disabled; an intervening policy change supersedes the still-open decision; stale resolved bindings are re-evaluated; and `UNKNOWN_AFTER_CRASH` turns are never replayed. Provider-operation approval remains a separate live gate.
