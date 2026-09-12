# Capability Learning

Batai treats production task history as evidence, not as unquestionable truth. The learner is a deterministic Rust component: it makes no evaluator-model calls, stores no prompts or source code, and never records hidden reasoning.

## Evidence model

`TaskOutcomeEvidence` links a task attempt to its agent, resource, provider/model, function, declared complexity/risk, required capabilities, reasoning effort, validation, retries, duration, usage, known provider cost and original routing decision. It stores compact outcome facts only. Every attempt has its own stable evidence record.

Outcome quality is typed as strong, medium or weak positive, mixed, negative, or not quality evidence. Completion alone is not treated as strong proof. Automated test counts and independent review strengthen an observation; self-review is recorded separately and carries less weight.

Failures are classified as model quality, provider failure, rate limit, authentication, infrastructure, test environment, invalid task, user cancellation or unknown. Only failures meaningfully related to output quality reduce capability. Network, quota and authentication failures contribute to service reliability instead.

Raw model evidence remains append-only by provenance: external benchmark, Batai benchmark, real task history, manual GOD evidence or unknown. A real outcome never overwrites a benchmark or manual observation.

## Calibrated profile

The derived `CalibratedCapabilityProfile` keeps, per dimension:

- estimated score and a lower conservative routing estimate;
- numeric confidence and a display band;
- real-task sample count and evidence-source composition;
- per-source scores, last observation and disagreement flag.

Only task-relevant dimensions are updated. Declared task difficulty anchors the observation, so repeated success on easy work does not establish senior-level capability. Evidence decays with a configurable half-life, individual task influence is capped, and mutable hosted aliases such as `latest` receive reduced weight. One or two outcomes cannot reach high confidence because configurable sample ceilings apply.

The current transparent aggregate uses recency-weighted source observations. The routing estimate subtracts a confidence-dependent uncertainty penalty and a further bounded penalty when evidence sources disagree. This is a conservative score, not a formal statistical confidence interval or Bayesian claim.

Local quality observations can inform the same model identity, while measured latency is accepted only for the matching hardware fingerprint. Hosted mutable aliases remain explicitly marked as such.

## Routing and calibration

Every new decision records router/scoring-policy versions, the classified task requirements, candidate snapshot, selected candidate and ordered shadow ranking. Shadow candidates are not executed and never receive outcome evidence.

When an outcome arrives, Batai creates a `RoutingCalibrationRecord` linking prediction to actual validation. Measurable aggregates include sufficiency success, under-routing, retries, review rejection, robust median duration and known provider-reported spend. Quality reliability and service reliability remain separate.

Offline replay applies a candidate policy to the historical resource snapshot without starting a provider, worktree or inference turn. It answers only which resource the policy would select. The alternative result remains `UNKNOWN`; Batai does not claim counterfactual quality or savings.

Calibration suggestions require a minimum body of validated evidence and are suggestion-only. Automatic calibration is rejected in this release. Manual capability evidence is GOD-only, append-only, attributed and audited. Promotion signals stop at L4 and require the existing governance mutation; no agent is automatically promoted or demoted.

## Bias and safety boundaries

- Production exploration is disabled; routing is deterministic.
- Confidence acts as an uncertainty gate/penalty, not an unlimited popularity bonus.
- High-risk work fails closed when relevant evidence confidence is too low.
- Benchmark/history disagreement is visible and makes routing more conservative.
- Infrastructure and quota failures do not punish model quality.
- Historical records are not backfilled when old data lacks enough semantics.
- Selection bias remains: frequently selected resources collect more evidence. Shadow ranking improves auditability but does not manufacture observations for alternatives.
- Task taxonomy, learner, router and scoring-policy versions preserve interpretation as policies evolve.

The default thresholds are operational policy defaults, not scientific constants. They can be reviewed and changed by GOD within validation bounds; automatic calibration remains off.
