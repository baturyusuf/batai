# Economic Intelligence Routing

Batai applies one deterministic principle: **use the minimum sufficient intelligence for the task**. A resource is a billable or capacity-bearing identity, not a provider name. `kimi-personal-membership` and a future Kimi PAYG account can therefore share a provider while retaining different billing, quota and policy behavior.

## Resource contract

`ResourceProfile` separates credentials from non-secret routing data and contains a tier, billing mode, verified model catalog, optional capability evidence, status, context, concurrency, quota, usage, latency, reliability and terms profile. Secrets live in the operating-system credential vault, with read-only environment variables as a deployment fallback. They are not written to `.batai`, SQLite, events or diagnostics.

Tiers are deterministic, local, free hosted, subscription quota, native subscription client, cheap PAYG and premium PAYG. Billing modes are `LOCAL`, `FREE_TIER`, `SUBSCRIPTION_QUOTA`, `NATIVE_SUBSCRIPTION_CLIENT` and `PAYG`. Provider names never imply billing mode.

Unknown terms, quality, quota and marginal cost remain unknown. A candidate requiring unknown quality or unapproved usage terms fails closed. Fixed monthly subscription cost is reporting data and is not presented as the marginal cost of one more task.

## Deterministic pipeline

1. Eligibility removes forbidden, offline, unauthenticated, terms-incompatible, context-limited, quota-exhausted, concurrency-exhausted and disallowed PAYG candidates.
2. Quality checks the function-relevant capability dimensions plus explicit task requirements and risk margin.
3. Economics prefers sufficient local/free capacity, then subscription capacity, and considers unused subscription quota that resets soon.
4. Availability considers concurrency and observed latency without inventing either.
5. Selection uses a stable resource/model lexical tie-break. With identical inputs the result is identical.

The result is either `SELECTED` or `NO_SUITABLE_RESOURCE`. A no-selection result proposes explicit options—install/benchmark local capacity, wait for quota, or ask GOD to permit PAYG—rather than assigning a weak model.

Every decision records candidates, hard rejection reasons, score components, selected resource/model, ordered shadow candidates, generic reasoning effort, task-taxonomy/router/policy versions and a user-facing rationale. This is an observable deterministic explanation, not hidden chain-of-thought.

The quality stage uses the conservative estimate from the [capability-learning model](CAPABILITY_LEARNING.md), not a raw mean. Low confidence increases the safety margin; high-risk tasks reject evidence below the configured confidence gate. Benchmark/history disagreement is visible and adds a bounded conservative penalty. Confidence is an uncertainty control, not a reward that lets the most frequently selected provider win forever.

## Runtime boundary

Existing explicit agents stay on their configured provider/model. New agents created with automatic intelligence use provider `auto`; routing occurs at the attempt boundary before a worktree or provider session begins. The chosen resource and explanation enter the task-run checkpoint. An active turn is never moved to another provider. Automatic reroute after exhaustion is disabled by default.

Each completed or failed attempt can create compact outcome evidence and a routing calibration record. Offline replay reruns only deterministic selection against the decision's historical resource snapshot. It performs no inference and never assigns an outcome or claimed savings to an unselected alternative.

PAYG remains disabled by default and is also constrained by the agent policy. A future non-zero PAYG threshold must enter the existing GOD governance/Decision Ledger rather than creating a second approval system.

## Current provider facts

Provider contracts were checked against official documentation on 2026-09-11:

- Kimi Code membership uses its official coding base and a distinct membership key/resource. Batai sends its genuine `Batai/<version>` user agent.
- Z.AI Coding Plan uses its dedicated coding endpoint. Batai marks general third-party Batai use `REQUIRES_REVIEW` because supported-tool language is plan-specific.
- MiniMax Token Plan uses a plan key distinct from PAYG; unknown plan quota remains unknown.
- Codex and Claude remain native subscription-client resources.

No OAuth extraction, consumer UI automation, user-agent spoofing, account rotation, quota evasion or silent PAYG is implemented.

A GOD acknowledgement records that the Z.AI terms warning was seen and writes an audit entry. It deliberately does **not** convert `REQUIRES_REVIEW` to `ALLOWED`; acknowledgement cannot change the provider's terms or bypass the router's eligibility gate.

## Score interpretation

The initial score is deliberately a configurable heuristic, not a scientific optimum. Quality carries a hard sufficiency constraint before economics is ranked. The score combines economic preference, availability and capability only for eligible candidates. Task risk increases the quality threshold. Quota reset adds a bounded preference only when documented unused capacity is known.
