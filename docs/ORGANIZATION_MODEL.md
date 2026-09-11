# Organization Model

Batai treats an agent as a durable organizational identity and a model as replaceable intelligence. Nova can remain a Senior Backend Engineer while moving from a local model to Claude or Codex; identity, reporting line, responsibilities, history and task/worktree context remain attached to Nova.

## Identity and schema evolution

New agent configs may use `schema_version: 2` and these typed fields:

```json
{
  "schema_version": 2,
  "id": "nova",
  "name": "Nova",
  "seniority": "SENIOR",
  "function": "BACKEND_ENGINEERING",
  "department": "ENGINEERING",
  "parent_agent_id": "director",
  "provider": "codex",
  "model": "auto"
}
```

`seniority` is the single source of truth; its numeric level is derived. The ladder is INTERN L0, JUNIOR L1, ASSOCIATE L2, MID L3, SENIOR L4, STAFF L5, PRINCIPAL L6 and DIRECTOR L7. GOD is the human authority and is not a seniority value or runtime agent.

Function and seniority are independent. Display titles are generated from both, for example `SENIOR` plus `BACKEND_ENGINEERING` becomes “Senior Backend Engineer.” `title_override` is presentation-only. A validity catalog rejects nonsensical new-schema combinations such as Intern Product Manager.

Legacy configs containing only `role_template` remain readable. Batai parses recognizable level/function terms in memory and safely falls back to an unspecified level plus `GENERIC_SOFTWARE_AGENT`; it does not force-rewrite project files.

Department is explicit project organization data. The Rust domain supplies it to the UI; JavaScript does not infer department from role text. Seniority, function, department and `parent_agent_id` belong under `.batai`. Current status, active task, session, tokens and task-run outcomes remain SQLite runtime state.

## Capabilities and recommendations

`model_capabilities` describes observed or benchmarked model ability. `effective_capabilities` describes the agent after model, tools, role configuration and Batai history are considered. Both support optional 0-100 scores for coding, debugging, planning, architecture, tool use, instruction following, long context, test generation, review, research, speed and reliability. Unknown scores remain `null` and are never invented.

The initial configurable recommendation policy uses a function-relevant dimension rather than one global model title. Coding can recommend one level while planning recommends another. Recommendations are candidates, not scientific truth, and are constrained by valid function/seniority combinations.

## Organization snapshot and telemetry

The desktop receives one initial organization snapshot containing virtual GOD metadata, agents, reporting relationships, tasks, active workflow edges, hierarchy warnings, project metrics, resource summaries and a recent observable activity trace. Runtime events then trigger a debounced refresh while preserving mode, selection and viewport.

Performance is derived only when task-run evidence exists: completed/active/failed counts, first-attempt and final success, average attempts and average duration. Review acceptance remains unknown until an explicit review-outcome contract exists. Usage aggregates provider-reported input/output/cached tokens and cost by agent, project and source (`local`, `subscription`, `api`). Missing costs and quota values display as Unknown or an em dash.

Hidden chain-of-thought is never persisted or displayed. The Activity tab is an execution trace of observable events such as task assignment, worktree creation, provider turn start, changed files, review and completion. Fine-grained activity falls back to Working when the provider exposes no safe signal.

## Organization Map

The graph uses a dependency-free SVG renderer so the static Tauri frontend remains small and does not require a React migration.

- Hierarchy renders reporting relationships top-down, with virtual GOD above Director.
- Workflow renders the task dependency DAG.
- Combined renders agents, tasks, ownership/handoffs, dependencies, collaboration and review together.

Reporting is solid; collaboration, dependency, handoff and review use distinct dash patterns and labels so color is not the only signal. The canvas supports pan, trackpad pan, modifier-wheel zoom, buttons, fit, reset, search, filters and keyboard focus. Detail changes with zoom. Cycles, missing parents and orphans do not crash layout and are surfaced as warnings.

The Inspector separates organizational identity from assigned intelligence and provides Overview, Tasks, Activity, Usage, Performance and Permissions/Context tabs. With no selection it shows weighted project progress, task/agent state and real known resource totals.

## Current boundaries

Persistent non-reporting relationship editing and meeting-derived groups are not implemented. Provider event vocabularies do not yet expose reliable reading/testing distinctions for every provider. Promotion workflows, energy estimates, model benchmarking and hidden reasoning are outside this slice.
