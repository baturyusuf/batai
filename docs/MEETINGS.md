# Bounded multi-agent meetings

Batai meetings are short coordination operations, not open-ended agent chat rooms. The per-project Rust daemon owns the Meeting Engine, its SQLite records, provider turns and live events. Closing Desktop or an MCP bridge does not stop a running meeting.

Meeting prompts consume the shared bounded [Context Builder](CONTEXT_BUILDER.md). Each participant receives common task/directive evidence plus a role-specific source package with provenance, safety exclusions and a stable fingerprint. Round-one packages never include peer positions.

## Domain and limits

A meeting records its title, structured agenda, objective, organizer, organizational participants, capability/role requirements, closure owner, optional task/decision links, trigger, timestamps, status, usage and typed outcome. Status moves through `PLANNED`, `READY`, `RUNNING`, `PAUSED`, `BLOCKED`, `COMPLETED`, `CANCELLED` or `FAILED`.

Safe defaults are five participants, two rounds, 800 tokens per response and 8,000 total charged tokens. Project policy may lower or raise these within hard system ceilings of 12 participants, four rounds, 8,192 tokens per response and 100,000 total tokens. Unlimited meetings are not representable. The backend validates the bounds; UI values are convenience, not the safety boundary.

Participants are agents such as “Nova — Senior Backend Engineer,” never provider/model identities. Explicit participants must exist, be unique and available. Role/capability selection chooses the smallest set that covers the requested functions and known capability minimums. Unknown capability is not invented. A Director is the default closure owner when one exists; an explicit participating owner is also valid.

## Execution

Round one gathers independent positions concurrently. Participants do not see each other's first response. Round two runs only when explicit disagreements or objections remain, the policy allows it and the remaining total budget can cover the bounded round. It receives concise positions and disagreements, not an unbounded raw transcript. Per-agent task lanes prevent the same agent from working a task and meeting turn simultaneously; provider lanes respect known meeting concurrency limits.

Each turn has a stable identity `(meeting, round, participant, kind)` plus provider session and turn IDs. Provider output is parsed into user-visible position, agreement, disagreement, objection, open-question and action-item fields. Batai neither requests nor persists hidden chain-of-thought. Missing provider token usage is conservatively charged at the response ceiling; monetary cost remains unknown unless reported.

Closure is deterministic and occurs exactly once. It preserves minority disagreement and unresolved questions instead of manufacturing consensus. Agreement state is observable (`UNANIMOUS`, `MAJORITY`, `UNRESOLVED` or `NOT_OBSERVED`), not a fabricated numeric confidence score. A concise final artifact is atomically written under `.batai/meetings/`; raw transcripts are not committed.

## Authority and economics

Workers cannot create meetings. Directors may create them only when project policy allows; Leads are denied by default. GOD remains subject to system safety ceilings. A meeting-triggered meeting is rejected to prevent recursive coordination.

AUTO participants use the existing Economic Router at the attempt boundary. Explicit agents keep their configured intelligence after resource checks. Local and subscription sources are eligible according to current policy and availability. PAYG requires both general and meeting policy permission. A PAYG-only choice creates a resource/model/policy/context/routing-bound `PAYG_SPEND` GOD decision and blocks before inference. Decision resolution wakes the engine through runtime events: exact approval resumes at the pre-turn boundary, rejection reroutes with PAYG disabled, and a stale approval is superseded. Context freshness is checked after resource acquisition and again immediately before `send_task`; a bounded rebuild/reroute happens before any provider call, and unstable context is parked safely.

## Tasks, recovery and events

Meeting action items are proposals. An authorized Director/GOD explicitly converts an item into a normal governed task with an assignee. The meeting/action link makes this conversion idempotent. Meeting participation is not recorded as coding-task capability evidence.

On daemon restart, completed turns remain complete and are skipped. A turn that was active at the crash becomes `UNKNOWN_AFTER_CRASH`; the meeting becomes `BLOCKED` and is not blindly replayed. A ready meeting with no uncertain active turn may resume. Cancellation requests provider-turn cancellation, preserves observable partial contributions and never promotes them to a final meeting decision.

Typed `MEETING_*` events drive the Desktop snapshot refresh and temporary Organization Map group. The graph overlay highlights participants without changing reporting structure. Meeting history, contributions, outcome, actions and usage remain available in the Meetings screen after the temporary group disappears.

## Control surfaces

Desktop uses the daemon RPC commands to create, read, list, cancel and convert meeting actions. The Rust MCP Director bridge exposes `batai_create_meeting`, `batai_get_meeting`, `batai_list_meetings` and `batai_cancel_meeting`; it remains a thin Director client and cannot spoof GOD. HTTP `/api/state` reads the same snapshot, including meetings and turns.
