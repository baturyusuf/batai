# Organization Governance

Batai separates human authority, organizational identity, permissions and model intelligence. GOD is the human operator and is never an AI agent. Director is the single top AI executive. Leads and workers can act only inside explicit scopes and permissions.

## Authority routing

Precedence is deterministic:

1. system/project policy constraints;
2. explicit GOD action or decision;
3. accepted task requirements;
4. Director authority;
5. delegated lead permissions;
6. worker permissions.

A higher seniority does not grant authority. `SENIOR`, `STAFF` and `PRINCIPAL` describe capability/organizational level; `authority` and `permissions` determine allowed operations. GOD cannot be assigned to an agent. Director authority requires the Director function, and exactly one active Director is valid.

The runtime routes every organization change through a typed `OrganizationMutation`. It validates the actor, optimistic organization revision, hierarchy, role catalog, provider allow/deny policy, active-agent/depth limits and protected-operation rules before writing. A denied or stale request does not perform the requested mutation. Stale clients receive a revision conflict and must reload before retrying.

Protected operations require a mutation-bound GOD decision when requested by an AI actor. These include permanent-agent creation/termination, authority or high-risk permission grants, PAYG/provider policy expansion, project-policy changes, production deployment, GitHub issue/PR creation and audit deletion. Approval applies only the exact serialized mutation; rejection applies nothing. Resolution is idempotent.

## Lifecycle and Agent Factory

Agent lifecycle is typed:

- `TASK_SCOPED` (default): automatically soft-terminates after its last non-terminal assigned task;
- `PROJECT`: remains available for the project;
- `PERMANENT`: durable organizational employee and GOD-gated when an AI requests creation or termination.

The Agent Factory derives a safe deterministic ID, selects a readable unused name when none is supplied, enforces the function/seniority catalog and explicit department, checks reporting depth/cycles/orphans and applies provider policy. Defaults are eight active agents and three reporting layers. There is no implicit provider fallback or PAYG enablement.

Soft termination preserves task runs, events, reviews, decisions, audit history and the declarative agent record. Runtime state remains in SQLite. Organizational identity, lifecycle, reporting line, permissions and intelligence policy remain under `.batai`.

## Persistent organization and relationships

`parent_agent_id` in each agent config is the sole source for reporting. `.batai/organization.json` stores its schema/revision and persistent non-reporting relationships:

- `COLLABORATION`
- `REVIEW`
- `ADVISORY`

Task dependencies are derived from tasks. Handoffs are runtime/organizational-memory records. Reporting, dependency and handoff edges are therefore not editable as generic relationships.

Cross-store writes use the recoverable operation journal described in [Recovery Model](RECOVERY_MODEL.md). The journal is durably `PREPARED` before organization files or SQLite entities change. Files use same-directory temporary files, flushes and replace/backup behavior; related SQLite changes run in one database transaction where possible. A recursive debounced watcher validates external config/policy/organization edits before accepting them into runtime state and ignores paths owned by an unfinished journal operation. Malformed, conflicting or partially written JSON is rejected and surfaced as an observable event.

## Decisions, approvals and audit

The Decision Ledger persists GOD decisions both in SQLite and `.batai/decisions/`, so open and resolved decisions survive restart. Each entry includes requester, exact bound mutation, impact, status, timestamps and optional resolution note.

Provider approvals are a separate per-request queue. In the interactive desktop, Codex uses the official App Server request/response protocol and can pause one live operation for GOD. The record is bound to provider process/session, thread, turn, JSON-RPC request, agent, task and worktree. The only user actions are `Allow once` and `Deny`; the response is returned to that exact live request and the same turn continues. Duplicate requests share the persisted record and decision.

Headless execution, an unavailable UI, an out-of-worktree target or a policy denial fails closed. A configurable timeout also denies the request. Shutdown/cancellation wakes the waiter and cancels or denies it. On restart, an old pending request becomes `ORPHANED`, because Batai never assumes that a new process can answer an earlier process's request; the associated task requires review. An approved response whose delivery cannot be confirmed becomes `RESPONSE_UNCERTAIN`, not `RESPONDED`, preventing blind replay.

Provider approval does not disable workspace sandboxing or Batai policy. It authorizes one observable provider operation, not an organizational mutation and not future requests. Credential-like fields and suspicious bearer/key strings are redacted before persistence. Hidden reasoning is never part of the approval record.

Every mutation attempt records actor, resolved authority, action, target, outcome, reason, task/decision linkage and organization revision. Recovery writes separate human-readable `RECOVERED`, `ROLLED_BACK` or `RECOVERY_REQUIRED` audit outcomes. The journal remains a recovery mechanism rather than audit history; audit storage is append-only and has no update or delete API.

## Intelligence, memory and review

Each agent has an `IntelligencePolicy` separate from organizational identity. It supports automatic or explicit assignment, preferred providers, allowed models, PAYG permission and an optional minimum capability threshold. Model changes do not change the agent's identity.

Task handoff and meeting types provide a typed foundation for organizational memory. Existing journals/reports/handoffs remain compatible. Explicit review outcomes (`ACCEPTED`, `CHANGES_REQUESTED`, `REJECTED`) are stored durably and drive review-acceptance metrics; absent review data remains unknown.

The UI shows observable execution events and audit history. Hidden chain-of-thought is neither persisted nor displayed.

## Current boundary

The governance and live provider-approval queues are implemented for the Rust desktop runtime. The Node control plane remains a compatibility layer and has not yet been fully redirected through Rust mutations. Meeting scheduling/execution and automatic summaries are typed foundations, not a completed meeting product. A provider request cannot survive the death of its original App Server process; restart therefore orphans it and deliberately requires review rather than replay.
