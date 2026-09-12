//! Deterministic capability learning from compact, observable task outcomes.
//!
//! Task history is evidence, never ground truth. The learner keeps provenance,
//! ignores non-quality failures, normalizes observations by task difficulty and
//! applies bounded recency-weighted aggregation. It never invokes an LLM.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{
    economic::{
        CapabilityEvidence, CapabilityEvidenceSource, EconomicPolicy, ResourceProfile,
        RoutingDecision, RoutingOutcome, TaskRequirements, TaskRisk,
    },
    governance::{ReviewOutcome, ReviewOutcomeKind},
    organization::{AgentFunction, CapabilityProfile, CapabilityScore, Department, Seniority},
};

pub const LEARNER_VERSION: &str = "capability-learning-v1";
pub const TASK_TAXONOMY_VERSION: &str = "task-taxonomy-v1";

pub const CAPABILITY_DIMENSIONS: [&str; 12] = [
    "coding",
    "debugging",
    "planning",
    "architecture",
    "tool_use",
    "instruction_following",
    "long_context",
    "test_generation",
    "review",
    "research",
    "speed",
    "reliability",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OutcomeQuality {
    StrongPositive,
    MediumPositive,
    WeakPositive,
    Mixed,
    Negative,
    NotQualityEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FailureClassification {
    ModelQuality,
    ProviderFailure,
    RateLimit,
    Auth,
    Infrastructure,
    TestEnvironment,
    InvalidTask,
    UserCancelled,
    Unknown,
}

impl FailureClassification {
    pub const fn affects_capability(self) -> bool {
        matches!(self, Self::ModelQuality)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RetryClassification {
    Quality,
    Network,
    Infrastructure,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewEvidenceKind {
    Independent,
    SelfReview,
    None,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TestEvidence {
    pub total: Option<u32>,
    pub passed: Option<u32>,
    pub failed: Option<u32>,
}

impl TestEvidence {
    pub fn all_passed(&self) -> bool {
        self.total.is_some_and(|total| total > 0)
            && self.failed == Some(0)
            && self.passed == self.total
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskOutcomeEvidence {
    pub id: String,
    pub task_id: String,
    pub task_run_id: String,
    pub agent_id: String,
    pub resource_id: String,
    pub provider: String,
    pub model: String,
    pub model_identity_mutable: bool,
    pub hardware_fingerprint: Option<String>,
    pub function: AgentFunction,
    pub complexity: u8,
    pub risk: TaskRisk,
    pub required_capabilities: BTreeMap<String, u8>,
    pub taxonomy_version: String,
    pub selected_reasoning_effort: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: String,
    pub succeeded: bool,
    pub outcome_quality: OutcomeQuality,
    pub failure_classification: Option<FailureClassification>,
    pub retries: u32,
    pub retry_classification: Option<RetryClassification>,
    pub review_outcome: Option<ReviewOutcomeKind>,
    pub review_evidence_kind: ReviewEvidenceKind,
    pub tests: TestEvidence,
    pub changed_files_count: Option<u32>,
    pub duration_ms: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub provider_reported_cost: Option<f64>,
    pub currency: Option<String>,
    pub routing_decision_id: Option<String>,
}

impl TaskOutcomeEvidence {
    pub fn apply_review(&mut self, review: &ReviewOutcome) {
        self.review_outcome = Some(review.outcome);
        self.review_evidence_kind = if review.reviewer_id == review.subject_agent_id {
            ReviewEvidenceKind::SelfReview
        } else {
            ReviewEvidenceKind::Independent
        };
        self.outcome_quality = match (review.outcome, self.review_evidence_kind) {
            (ReviewOutcomeKind::Accepted, ReviewEvidenceKind::Independent)
                if self.tests.all_passed() =>
            {
                OutcomeQuality::StrongPositive
            }
            (ReviewOutcomeKind::Accepted, ReviewEvidenceKind::Independent) => {
                OutcomeQuality::MediumPositive
            }
            (ReviewOutcomeKind::Accepted, ReviewEvidenceKind::SelfReview) => {
                OutcomeQuality::WeakPositive
            }
            (ReviewOutcomeKind::ChangesRequested, _) => OutcomeQuality::Mixed,
            (ReviewOutcomeKind::Rejected, _) => OutcomeQuality::Negative,
            _ => self.outcome_quality,
        };
    }

    pub fn relevant_dimensions(&self) -> Vec<String> {
        relevant_dimensions(
            self.function,
            &self.required_capabilities,
            self.tests.total.is_some(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConfidenceBand {
    VeryLow,
    Low,
    Medium,
    High,
    VeryHigh,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibratedDimension {
    pub estimated_score: Option<f64>,
    pub routing_estimate: Option<f64>,
    pub confidence: f64,
    pub confidence_band: ConfidenceBand,
    pub sample_count: u32,
    pub evidence_composition: BTreeMap<CapabilityEvidenceSource, u32>,
    pub last_observed: Option<String>,
    pub disagreement: bool,
    pub source_scores: BTreeMap<CapabilityEvidenceSource, f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibratedCapabilityProfile {
    pub resource_id: String,
    pub model: Option<String>,
    pub learner_version: String,
    pub dimensions: BTreeMap<String, CalibratedDimension>,
    pub quality_reliability: Option<f64>,
    pub service_reliability: Option<f64>,
    pub median_latency_ms: Option<u64>,
    pub observed_tasks: u32,
    pub last_evidence_at: Option<String>,
}

impl CalibratedCapabilityProfile {
    pub fn routing_profile(&self, fallback: &CapabilityProfile) -> CapabilityProfile {
        let score = |name: &str| {
            self.dimensions
                .get(name)
                .and_then(|dimension| dimension.routing_estimate)
                .map(|value| value.round().clamp(0.0, 100.0) as u8)
                .and_then(|value| CapabilityScore::new(value).ok())
                .or_else(|| profile_score(fallback, name))
        };
        CapabilityProfile {
            coding: score("coding"),
            debugging: score("debugging"),
            planning: score("planning"),
            architecture: score("architecture"),
            tool_use: score("tool_use"),
            instruction_following: score("instruction_following"),
            long_context: score("long_context"),
            test_generation: score("test_generation"),
            review: score("review"),
            research: score("research"),
            speed: score("speed"),
            reliability: score("reliability"),
        }
    }

    pub fn minimum_confidence_for(&self, requirements: &TaskRequirements) -> Option<f64> {
        let dimensions = relevant_dimensions(
            requirements.function,
            &requirements.required_capabilities,
            false,
        );
        dimensions
            .iter()
            .filter_map(|dimension| self.dimensions.get(dimension).map(|value| value.confidence))
            .reduce(f64::min)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CapabilityLearningPolicy {
    pub recency_half_life_days: f64,
    pub low_confidence_penalty: f64,
    pub disagreement_threshold: f64,
    pub high_risk_min_confidence: f64,
    pub min_medium_samples: u32,
    pub min_high_samples: u32,
    pub min_very_high_samples: u32,
    pub max_task_weight: f64,
    pub automatic_calibration: bool,
}

impl Default for CapabilityLearningPolicy {
    fn default() -> Self {
        Self {
            recency_half_life_days: 120.0,
            low_confidence_penalty: 14.0,
            disagreement_threshold: 20.0,
            high_risk_min_confidence: 0.45,
            min_medium_samples: 6,
            min_high_samples: 16,
            min_very_high_samples: 40,
            max_task_weight: 1.5,
            automatic_calibration: false,
        }
    }
}

impl CapabilityLearningPolicy {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if !(1.0..=3650.0).contains(&self.recency_half_life_days) {
            return Err("recency half-life must be between 1 and 3650 days".into());
        }
        if !(0.0..=40.0).contains(&self.low_confidence_penalty) {
            return Err("low-confidence penalty must be between 0 and 40".into());
        }
        if !(1.0..=100.0).contains(&self.disagreement_threshold) {
            return Err("disagreement threshold must be between 1 and 100".into());
        }
        if !(0.0..=1.0).contains(&self.high_risk_min_confidence) {
            return Err("high-risk minimum confidence must be between 0 and 1".into());
        }
        if self.min_medium_samples < 3
            || self.min_medium_samples > self.min_high_samples
            || self.min_high_samples > self.min_very_high_samples
        {
            return Err("sample thresholds must be ordered and medium must be at least 3".into());
        }
        if !(0.1..=3.0).contains(&self.max_task_weight) {
            return Err("maximum task weight must be between 0.1 and 3".into());
        }
        if self.automatic_calibration {
            return Err("automatic calibration is not enabled in this release".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingCalibrationRecord {
    pub id: String,
    pub routing_decision_id: String,
    pub task_id: String,
    pub resource_id: String,
    pub provider: String,
    pub model: String,
    pub function: AgentFunction,
    pub predicted_sufficiency: bool,
    pub predicted_quality_score: Option<f64>,
    pub actual_outcome: OutcomeQuality,
    pub confidence: Option<f64>,
    pub retries: u32,
    pub validation_strength: OutcomeQuality,
    pub review_outcome: Option<ReviewOutcomeKind>,
    pub under_routing: Option<bool>,
    pub overqualification_indicator: Option<bool>,
    pub cost: Option<f64>,
    pub currency: Option<String>,
    pub latency_ms: Option<u64>,
    pub completed_at: String,
    pub router_version: String,
    pub scoring_policy_version: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationSlice {
    pub key: String,
    pub total: u32,
    pub validated: u32,
    pub successes: u32,
    pub under_routed: u32,
    pub retries: u32,
    pub review_rejections: u32,
    pub success_rate: Option<f64>,
    pub median_duration_ms: Option<u64>,
    pub known_cost: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationDashboard {
    pub total_routed_tasks: u32,
    pub calibrated_outcomes: u32,
    pub success_rate: Option<f64>,
    pub under_routing_rate: Option<f64>,
    pub retry_rate: Option<f64>,
    pub review_rejection_rate: Option<f64>,
    pub median_duration_ms: Option<u64>,
    pub known_marginal_spend: Option<f64>,
    pub by_resource: Vec<CalibrationSlice>,
    pub by_function: Vec<CalibrationSlice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationSuggestion {
    pub id: String,
    pub function: AgentFunction,
    pub capability: String,
    pub current_margin: u8,
    pub suggested_margin: u8,
    pub evidence_count: u32,
    pub confidence: ConfidenceBand,
    pub reason: String,
    pub automatic_apply: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotionSuggestion {
    pub agent_id: String,
    pub current_seniority: Seniority,
    pub suggested_seniority: Seniority,
    pub validated_tasks: u32,
    pub confidence: ConfidenceBand,
    pub reason: String,
    pub requires_governance: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterReplayResult {
    pub routing_decision_id: String,
    pub original_resource_id: Option<String>,
    pub replay_resource_id: Option<String>,
    pub original_outcome: RoutingOutcome,
    pub replay_outcome: RoutingOutcome,
    pub selection_changed: bool,
    pub eligibility_changes: Vec<String>,
    pub cost_class_changed: bool,
    pub actual_outcome: Option<OutcomeQuality>,
    pub alternative_outcome: Option<OutcomeQuality>,
    pub note: String,
}

#[derive(Debug, Clone, Default)]
pub struct CapabilityLearner;

impl CapabilityLearner {
    pub fn calibrate(
        &self,
        resource: &ResourceProfile,
        outcomes: &[TaskOutcomeEvidence],
        policy: &CapabilityLearningPolicy,
        now: DateTime<Utc>,
    ) -> CalibratedCapabilityProfile {
        self.calibrate_model(
            resource,
            resource.supported_models.first().map(String::as_str),
            outcomes,
            policy,
            now,
        )
    }

    pub fn calibrate_model(
        &self,
        resource: &ResourceProfile,
        model: Option<&str>,
        outcomes: &[TaskOutcomeEvidence],
        policy: &CapabilityLearningPolicy,
        now: DateTime<Utc>,
    ) -> CalibratedCapabilityProfile {
        let relevant = outcomes
            .iter()
            .filter(|outcome| {
                outcome.resource_id == resource.id
                    && model.is_none_or(|model| outcome.model == model)
            })
            .collect::<Vec<_>>();
        let mut dimensions = BTreeMap::new();
        let model_priors = resource
            .capability_evidence
            .iter()
            .filter(|evidence| {
                evidence.model.is_none() || model.is_none() || evidence.model.as_deref() == model
            })
            .cloned()
            .collect::<Vec<_>>();
        for dimension in CAPABILITY_DIMENSIONS {
            let calibrated = calibrate_dimension(dimension, &model_priors, &relevant, policy, now);
            if calibrated.estimated_score.is_some() {
                dimensions.insert(dimension.to_owned(), calibrated);
            }
        }
        let quality_outcomes = relevant
            .iter()
            .filter(|outcome| {
                outcome.succeeded
                    || outcome
                        .failure_classification
                        .is_some_and(FailureClassification::affects_capability)
            })
            .collect::<Vec<_>>();
        let successful = quality_outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome.outcome_quality,
                    OutcomeQuality::StrongPositive
                        | OutcomeQuality::MediumPositive
                        | OutcomeQuality::WeakPositive
                )
            })
            .count();
        let service_failures = relevant
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome.failure_classification,
                    Some(
                        FailureClassification::ProviderFailure
                            | FailureClassification::RateLimit
                            | FailureClassification::Auth
                            | FailureClassification::Infrastructure
                    )
                )
            })
            .count();
        let latency = relevant
            .iter()
            .filter(|outcome| {
                resource.billing_mode != super::economic::BillingMode::Local
                    || outcome.hardware_fingerprint == resource_hardware(resource)
            })
            .filter_map(|outcome| outcome.duration_ms)
            .collect::<Vec<_>>();
        CalibratedCapabilityProfile {
            resource_id: resource.id.clone(),
            model: model.map(str::to_owned),
            learner_version: LEARNER_VERSION.into(),
            quality_reliability: ratio(successful, quality_outcomes.len()),
            service_reliability: if relevant.is_empty() {
                None
            } else {
                Some(1.0 - service_failures as f64 / relevant.len() as f64)
            },
            median_latency_ms: median(latency),
            observed_tasks: relevant.len() as u32,
            last_evidence_at: relevant
                .iter()
                .map(|outcome| outcome.completed_at.clone())
                .max(),
            dimensions,
        }
    }
}

fn calibrate_dimension(
    dimension: &str,
    priors: &[CapabilityEvidence],
    outcomes: &[&TaskOutcomeEvidence],
    policy: &CapabilityLearningPolicy,
    now: DateTime<Utc>,
) -> CalibratedDimension {
    let mut weighted = Vec::<(f64, f64, CapabilityEvidenceSource, String, u32)>::new();
    for evidence in priors
        .iter()
        .filter(|evidence| evidence.dimension == dimension)
    {
        let source_weight = match evidence.source {
            CapabilityEvidenceSource::Manual => 2.5,
            CapabilityEvidenceSource::BataiBenchmark => 2.0,
            CapabilityEvidenceSource::ExternalBenchmark => 1.5,
            CapabilityEvidenceSource::RealTaskHistory => 1.0,
            CapabilityEvidenceSource::Unknown => 0.5,
        };
        let recency = recency_weight(&evidence.observed_at, now, policy.recency_half_life_days);
        let samples = evidence.sample_count.max(1);
        let weight = source_weight * f64::from(samples).sqrt() * recency;
        weighted.push((
            f64::from(evidence.score.get()),
            weight,
            evidence.source,
            evidence.observed_at.clone(),
            samples,
        ));
    }
    let mut task_bands = BTreeSet::new();
    let mut task_functions = BTreeSet::new();
    for outcome in outcomes.iter().copied().filter(|outcome| {
        outcome
            .relevant_dimensions()
            .iter()
            .any(|value| value == dimension)
    }) {
        let Some((score, strength)) = outcome_observation(outcome) else {
            continue;
        };
        let recency = recency_weight(&outcome.completed_at, now, policy.recency_half_life_days);
        let alias_penalty = if outcome.model_identity_mutable {
            0.6
        } else {
            1.0
        };
        let weight = (strength * recency * alias_penalty).min(policy.max_task_weight);
        weighted.push((
            score,
            weight,
            CapabilityEvidenceSource::RealTaskHistory,
            outcome.completed_at.clone(),
            1,
        ));
        task_bands.insert(outcome.complexity / 20);
        task_functions.insert(outcome.function);
    }
    let total_weight: f64 = weighted.iter().map(|(_, weight, _, _, _)| weight).sum();
    let estimated = (total_weight > 0.0).then(|| {
        weighted
            .iter()
            .map(|(score, weight, _, _, _)| score * weight)
            .sum::<f64>()
            / total_weight
    });
    let sample_count = weighted
        .iter()
        .filter(|(_, _, source, _, _)| *source == CapabilityEvidenceSource::RealTaskHistory)
        .map(|(_, _, _, _, count)| *count)
        .sum::<u32>();
    let source_count = weighted
        .iter()
        .map(|(_, _, source, _, _)| *source)
        .collect::<BTreeSet<_>>()
        .len();
    let diversity = (task_bands.len() + task_functions.len() + source_count).min(6) as f64 / 6.0;
    let base_confidence = total_weight / (total_weight + 8.0);
    let sample_ceiling = if sample_count < 3 {
        0.24
    } else if sample_count < policy.min_medium_samples {
        0.44
    } else if sample_count < policy.min_high_samples {
        0.68
    } else if sample_count < policy.min_very_high_samples {
        0.86
    } else {
        0.96
    };
    let confidence = (base_confidence * (0.7 + diversity * 0.3))
        .min(sample_ceiling)
        .clamp(0.0, 1.0);
    let mut composition = BTreeMap::new();
    let mut source_values = BTreeMap::<CapabilityEvidenceSource, Vec<f64>>::new();
    for (score, _, source, _, count) in &weighted {
        *composition.entry(*source).or_default() += *count;
        source_values.entry(*source).or_default().push(*score);
    }
    let source_scores = source_values
        .into_iter()
        .map(|(source, values)| {
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            (source, mean)
        })
        .collect::<BTreeMap<_, _>>();
    let disagreement = source_scores
        .values()
        .copied()
        .reduce(f64::max)
        .zip(source_scores.values().copied().reduce(f64::min))
        .is_some_and(|(max, min)| max - min >= policy.disagreement_threshold);
    let conservative = estimated.map(|mean| {
        let uncertainty = policy.low_confidence_penalty * (1.0 - confidence);
        let disagreement_penalty = if disagreement { 5.0 } else { 0.0 };
        (mean - uncertainty - disagreement_penalty).clamp(0.0, 100.0)
    });
    CalibratedDimension {
        estimated_score: estimated,
        routing_estimate: conservative,
        confidence,
        confidence_band: confidence_band(confidence),
        sample_count,
        evidence_composition: composition,
        last_observed: weighted.iter().map(|(_, _, _, date, _)| date.clone()).max(),
        disagreement,
        source_scores,
    }
}

fn outcome_observation(outcome: &TaskOutcomeEvidence) -> Option<(f64, f64)> {
    if !outcome.succeeded
        && !outcome
            .failure_classification
            .is_some_and(FailureClassification::affects_capability)
    {
        return None;
    }
    let required = outcome
        .required_capabilities
        .values()
        .copied()
        .max()
        .unwrap_or(outcome.complexity);
    let (offset, mut strength) = match outcome.outcome_quality {
        OutcomeQuality::StrongPositive => (12.0, 1.5),
        OutcomeQuality::MediumPositive => (7.0, 1.0),
        OutcomeQuality::WeakPositive => (2.0, 0.55),
        OutcomeQuality::Mixed => (-4.0, 0.8),
        OutcomeQuality::Negative => (-15.0, 1.2),
        OutcomeQuality::NotQualityEvidence => return None,
    };
    if outcome.retries > 0 {
        strength *= match outcome.retry_classification {
            Some(RetryClassification::Quality) => 1.0,
            Some(RetryClassification::Network | RetryClassification::Infrastructure) => 0.65,
            Some(RetryClassification::Unknown) | None => 0.8,
        };
    }
    Some(((f64::from(required) + offset).clamp(0.0, 100.0), strength))
}

pub fn relevant_dimensions(
    function: AgentFunction,
    explicit: &BTreeMap<String, u8>,
    has_tests: bool,
) -> Vec<String> {
    let mut dimensions = explicit.keys().cloned().collect::<BTreeSet<_>>();
    if dimensions.is_empty() {
        match function.department() {
            Department::Product | Department::Leadership => {
                dimensions.extend(["planning", "instruction_following"].map(str::to_owned));
            }
            Department::Architecture => {
                dimensions.extend(["architecture", "planning", "long_context"].map(str::to_owned));
            }
            Department::Quality => {
                dimensions.extend(
                    ["test_generation", "review", "instruction_following"].map(str::to_owned),
                );
            }
            Department::Research => {
                dimensions.extend(
                    ["research", "long_context", "instruction_following"].map(str::to_owned),
                );
            }
            _ => {
                dimensions.extend(
                    ["coding", "debugging", "tool_use", "instruction_following"].map(str::to_owned),
                );
            }
        }
    }
    if has_tests && function.department() == Department::Quality {
        dimensions.insert("test_generation".into());
    }
    dimensions
        .into_iter()
        .filter(|dimension| CAPABILITY_DIMENSIONS.contains(&dimension.as_str()))
        .collect()
}

pub fn calibration_record(
    decision: &RoutingDecision,
    outcome: &TaskOutcomeEvidence,
    calibrated: Option<&CalibratedCapabilityProfile>,
) -> Option<RoutingCalibrationRecord> {
    let resource_id = decision.selected_resource_id.as_ref()?;
    if resource_id != &outcome.resource_id {
        return None;
    }
    let candidate = decision
        .candidates
        .iter()
        .find(|candidate| {
            candidate.resource_id == *resource_id
                && candidate.model.as_deref() == Some(&outcome.model)
        })
        .or_else(|| {
            decision
                .candidates
                .iter()
                .find(|candidate| candidate.resource_id == *resource_id)
        });
    let under_routing = match outcome.outcome_quality {
        OutcomeQuality::Negative | OutcomeQuality::Mixed
            if outcome
                .failure_classification
                .is_none_or(FailureClassification::affects_capability) =>
        {
            Some(true)
        }
        OutcomeQuality::StrongPositive | OutcomeQuality::MediumPositive => Some(false),
        _ => None,
    };
    let confidence =
        calibrated.and_then(|profile| profile.minimum_confidence_for(&decision.requirements));
    Some(RoutingCalibrationRecord {
        id: format!("CAL-{}", outcome.id),
        routing_decision_id: decision.id.clone(),
        task_id: outcome.task_id.clone(),
        resource_id: outcome.resource_id.clone(),
        provider: outcome.provider.clone(),
        model: outcome.model.clone(),
        function: outcome.function,
        predicted_sufficiency: candidate.is_some_and(|candidate| candidate.sufficient),
        predicted_quality_score: candidate.and_then(|candidate| candidate.quality_score),
        actual_outcome: outcome.outcome_quality,
        confidence,
        retries: outcome.retries,
        validation_strength: outcome.outcome_quality,
        review_outcome: outcome.review_outcome,
        under_routing,
        overqualification_indicator: candidate.and_then(|candidate| {
            (outcome.outcome_quality == OutcomeQuality::StrongPositive).then(|| {
                candidate.quality_score.is_some_and(|score| {
                    score >= f64::from(decision.requirements.complexity.saturating_add(25))
                })
            })
        }),
        cost: outcome.provider_reported_cost,
        currency: outcome.currency.clone(),
        latency_ms: outcome.duration_ms,
        completed_at: outcome.completed_at.clone(),
        router_version: decision.router_version.clone(),
        scoring_policy_version: decision.scoring_policy_version.clone(),
    })
}

pub fn calibration_dashboard(
    decisions: &[RoutingDecision],
    records: &[RoutingCalibrationRecord],
) -> CalibrationDashboard {
    let mut by_resource = BTreeMap::<String, Vec<&RoutingCalibrationRecord>>::new();
    let mut by_function = BTreeMap::<String, Vec<&RoutingCalibrationRecord>>::new();
    for record in records {
        by_resource
            .entry(record.resource_id.clone())
            .or_default()
            .push(record);
        by_function
            .entry(format!("{:?}", record.function).to_ascii_uppercase())
            .or_default()
            .push(record);
    }
    let total = slice("project", records.iter().collect());
    CalibrationDashboard {
        total_routed_tasks: decisions
            .iter()
            .filter(|decision| decision.outcome == RoutingOutcome::Selected)
            .count() as u32,
        calibrated_outcomes: total.validated,
        success_rate: total.success_rate,
        under_routing_rate: rate(total.under_routed, total.validated),
        retry_rate: rate(total.retries.min(total.total), total.total),
        review_rejection_rate: rate(total.review_rejections, total.validated),
        median_duration_ms: total.median_duration_ms,
        known_marginal_spend: total.known_cost,
        by_resource: by_resource
            .into_iter()
            .map(|(key, records)| slice(&key, records))
            .collect(),
        by_function: by_function
            .into_iter()
            .map(|(key, records)| slice(&key, records))
            .collect(),
    }
}

fn slice(key: &str, records: Vec<&RoutingCalibrationRecord>) -> CalibrationSlice {
    let validated = records
        .iter()
        .filter(|record| record.under_routing.is_some())
        .count() as u32;
    let successes = records
        .iter()
        .filter(|record| record.under_routing == Some(false))
        .count() as u32;
    let under_routed = records
        .iter()
        .filter(|record| record.under_routing == Some(true))
        .count() as u32;
    let retries = records.iter().filter(|record| record.retries > 0).count() as u32;
    let review_rejections = records
        .iter()
        .filter(|record| record.review_outcome == Some(ReviewOutcomeKind::Rejected))
        .count() as u32;
    let known_cost = records
        .iter()
        .filter_map(|record| record.cost)
        .reduce(|left, right| left + right);
    CalibrationSlice {
        key: key.into(),
        total: records.len() as u32,
        validated,
        successes,
        under_routed,
        retries,
        review_rejections,
        success_rate: rate(successes, validated),
        median_duration_ms: median(
            records
                .iter()
                .filter_map(|record| record.latency_ms)
                .collect(),
        ),
        known_cost,
    }
}

pub fn calibration_suggestions(
    records: &[RoutingCalibrationRecord],
    policy: &EconomicPolicy,
) -> Vec<CalibrationSuggestion> {
    let mut groups = BTreeMap::<AgentFunction, Vec<&RoutingCalibrationRecord>>::new();
    for record in records {
        groups.entry(record.function).or_default().push(record);
    }
    groups
        .into_iter()
        .filter_map(|(function, records)| {
            let validated = records.iter().filter(|record| record.under_routing.is_some()).count();
            let under = records.iter().filter(|record| record.under_routing == Some(true)).count();
            if validated < 6 || under * 4 < validated {
                return None;
            }
            Some(CalibrationSuggestion {
                id: format!("SUGGEST-{:?}-margin", function),
                function,
                capability: relevant_dimensions(function, &BTreeMap::new(), false)
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "coding".into()),
                current_margin: policy.min_capability_margin,
                suggested_margin: policy.min_capability_margin.saturating_add(4).min(30),
                evidence_count: validated as u32,
                confidence: confidence_band(validated as f64 / (validated as f64 + 8.0)),
                reason: "Validated outcomes show repeated under-routing; manual policy review is recommended".into(),
                automatic_apply: false,
            })
        })
        .collect()
}

pub fn replay_result(
    original: &RoutingDecision,
    replayed: &RoutingDecision,
    actual: Option<&TaskOutcomeEvidence>,
    original_billing: Option<super::economic::BillingMode>,
    replay_billing: Option<super::economic::BillingMode>,
) -> RouterReplayResult {
    let original_eligible = original
        .candidates
        .iter()
        .filter(|candidate| candidate.eligible)
        .map(|candidate| candidate.resource_id.clone())
        .collect::<BTreeSet<_>>();
    let replay_eligible = replayed
        .candidates
        .iter()
        .filter(|candidate| candidate.eligible)
        .map(|candidate| candidate.resource_id.clone())
        .collect::<BTreeSet<_>>();
    let eligibility_changes = original_eligible
        .symmetric_difference(&replay_eligible)
        .cloned()
        .collect();
    RouterReplayResult {
        routing_decision_id: original.id.clone(),
        original_resource_id: original.selected_resource_id.clone(),
        replay_resource_id: replayed.selected_resource_id.clone(),
        original_outcome: original.outcome.clone(),
        replay_outcome: replayed.outcome.clone(),
        selection_changed: original.selected_resource_id != replayed.selected_resource_id,
        eligibility_changes,
        cost_class_changed: original_billing
            .zip(replay_billing)
            .is_some_and(|(a, b)| a != b),
        actual_outcome: actual.map(|outcome| outcome.outcome_quality),
        alternative_outcome: None,
        note: "Replay performs no inference. The unselected alternative outcome remains unknown"
            .into(),
    }
}

pub fn next_seniority(current: Seniority) -> Option<Seniority> {
    match current {
        Seniority::Intern => Some(Seniority::Junior),
        Seniority::Junior => Some(Seniority::Associate),
        Seniority::Associate => Some(Seniority::Mid),
        Seniority::Mid => Some(Seniority::Senior),
        Seniority::Senior | Seniority::Staff | Seniority::Principal | Seniority::Director => None,
    }
}

pub fn promotion_suggestion(
    agent_id: &str,
    current: Seniority,
    outcomes: &[TaskOutcomeEvidence],
) -> Option<PromotionSuggestion> {
    let relevant = outcomes
        .iter()
        .filter(|outcome| {
            outcome.agent_id == agent_id
                && matches!(
                    outcome.outcome_quality,
                    OutcomeQuality::StrongPositive | OutcomeQuality::MediumPositive
                )
                && outcome.complexity >= ((current.level() + 1) * 15).min(90)
        })
        .count() as u32;
    let suggested = next_seniority(current)?;
    (relevant >= 6).then(|| PromotionSuggestion {
        agent_id: agent_id.into(),
        current_seniority: current,
        suggested_seniority: suggested,
        validated_tasks: relevant,
        confidence: confidence_band(relevant as f64 / (relevant as f64 + 8.0)),
        reason:
            "Repeated validated outcomes at the next difficulty band support a governance review"
                .into(),
        requires_governance: true,
    })
}

pub fn confidence_band(value: f64) -> ConfidenceBand {
    if value < 0.2 {
        ConfidenceBand::VeryLow
    } else if value < 0.4 {
        ConfidenceBand::Low
    } else if value < 0.65 {
        ConfidenceBand::Medium
    } else if value < 0.85 {
        ConfidenceBand::High
    } else {
        ConfidenceBand::VeryHigh
    }
}

fn recency_weight(observed_at: &str, now: DateTime<Utc>, half_life_days: f64) -> f64 {
    let age_days = DateTime::parse_from_rfc3339(observed_at)
        .ok()
        .map(|observed| (now - observed.with_timezone(&Utc)).num_seconds().max(0) as f64 / 86_400.0)
        .unwrap_or(0.0);
    if half_life_days <= 0.0 {
        1.0
    } else {
        0.5_f64.powf(age_days / half_life_days)
    }
}

fn profile_score(profile: &CapabilityProfile, dimension: &str) -> Option<CapabilityScore> {
    match dimension {
        "coding" => profile.coding,
        "debugging" => profile.debugging,
        "planning" => profile.planning,
        "architecture" => profile.architecture,
        "tool_use" => profile.tool_use,
        "instruction_following" => profile.instruction_following,
        "long_context" => profile.long_context,
        "test_generation" => profile.test_generation,
        "review" => profile.review,
        "research" => profile.research,
        "speed" => profile.speed,
        "reliability" => profile.reliability,
        _ => None,
    }
}

fn resource_hardware(resource: &ResourceProfile) -> Option<String> {
    resource
        .capability_evidence
        .iter()
        .find_map(|evidence| evidence.hardware_fingerprint.clone())
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

fn rate(numerator: u32, denominator: u32) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

fn median(mut values: Vec<u64>) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        values[middle - 1].saturating_add(values[middle]) / 2
    } else {
        values[middle]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::economic::{
        native_resource_profile, BillingMode, QuotaAwareEconomicRouter,
    };

    fn outcome(id: &str, complexity: u8, quality: OutcomeQuality) -> TaskOutcomeEvidence {
        TaskOutcomeEvidence {
            id: id.into(),
            task_id: id.into(),
            task_run_id: format!("{id}:a:1"),
            agent_id: "a".into(),
            resource_id: "local".into(),
            provider: "ollama".into(),
            model: "m".into(),
            model_identity_mutable: false,
            hardware_fingerprint: Some("h".into()),
            function: AgentFunction::BackendEngineering,
            complexity,
            risk: TaskRisk::Medium,
            required_capabilities: BTreeMap::new(),
            taxonomy_version: TASK_TAXONOMY_VERSION.into(),
            selected_reasoning_effort: Some("MEDIUM".into()),
            started_at: None,
            completed_at: Utc::now().to_rfc3339(),
            succeeded: !matches!(quality, OutcomeQuality::Negative),
            outcome_quality: quality,
            failure_classification: matches!(quality, OutcomeQuality::Negative)
                .then_some(FailureClassification::ModelQuality),
            retries: 0,
            retry_classification: None,
            review_outcome: None,
            review_evidence_kind: ReviewEvidenceKind::None,
            tests: TestEvidence::default(),
            changed_files_count: Some(1),
            duration_ms: Some(100),
            input_tokens: None,
            output_tokens: None,
            provider_reported_cost: None,
            currency: None,
            routing_decision_id: None,
        }
    }

    fn resource(prior: Option<u8>) -> ResourceProfile {
        let mut resource = native_resource_profile("local", "ollama", "Local", BillingMode::Local);
        resource.supported_models = vec!["m".into()];
        if let Some(prior) = prior {
            resource.capabilities.coding = CapabilityScore::new(prior).ok();
            resource.capability_evidence.push(CapabilityEvidence {
                dimension: "coding".into(),
                score: CapabilityScore::new(prior).unwrap(),
                source: CapabilityEvidenceSource::BataiBenchmark,
                model: Some("m".into()),
                observed_at: Utc::now().to_rfc3339(),
                hardware_fingerprint: Some("h".into()),
                sample_count: 1,
                note: None,
            });
        }
        resource
    }

    #[test]
    fn sparse_easy_success_stays_low_confidence_and_does_not_imply_senior() {
        let profile = CapabilityLearner.calibrate(
            &resource(None),
            &[outcome("one", 25, OutcomeQuality::StrongPositive)],
            &CapabilityLearningPolicy::default(),
            Utc::now(),
        );
        let coding = &profile.dimensions["coding"];
        assert_eq!(coding.confidence_band, ConfidenceBand::VeryLow);
        assert!(coding.routing_estimate.unwrap() < 55.0);
    }

    #[test]
    fn diverse_samples_raise_confidence_and_old_evidence_decays() {
        let mut outcomes = (0..20)
            .map(|index| {
                outcome(
                    &format!("o{index}"),
                    55 + index % 3,
                    OutcomeQuality::StrongPositive,
                )
            })
            .collect::<Vec<_>>();
        outcomes[0].completed_at = (Utc::now() - chrono::Duration::days(500)).to_rfc3339();
        let profile = CapabilityLearner.calibrate(
            &resource(Some(60)),
            &outcomes,
            &CapabilityLearningPolicy::default(),
            Utc::now(),
        );
        assert!(profile.dimensions["coding"].confidence >= 0.65);
        assert!(profile.dimensions["coding"].sample_count >= 20);
    }

    #[test]
    fn infra_and_rate_limit_failures_do_not_reduce_quality() {
        let mut infra = outcome("infra", 80, OutcomeQuality::NotQualityEvidence);
        infra.succeeded = false;
        infra.failure_classification = Some(FailureClassification::Infrastructure);
        let mut rate = infra.clone();
        rate.id = "rate".into();
        rate.failure_classification = Some(FailureClassification::RateLimit);
        let profile = CapabilityLearner.calibrate(
            &resource(Some(75)),
            &[infra, rate],
            &CapabilityLearningPolicy::default(),
            Utc::now(),
        );
        assert_eq!(profile.dimensions["coding"].estimated_score, Some(75.0));
        assert_eq!(profile.dimensions["coding"].sample_count, 0);
    }

    #[test]
    fn unknown_failure_does_not_punish_quality_without_model_attribution() {
        let mut unknown = outcome("unknown", 70, OutcomeQuality::Negative);
        unknown.succeeded = false;
        unknown.failure_classification = Some(FailureClassification::Unknown);
        let profile = CapabilityLearner.calibrate(
            &resource(None),
            &[unknown],
            &CapabilityLearningPolicy::default(),
            Utc::now(),
        );
        assert!(!profile.dimensions.contains_key("coding"));
        assert_eq!(profile.quality_reliability, None);
    }

    #[test]
    fn benchmark_history_and_manual_provenance_survive_disagreement() {
        let mut resource = resource(Some(90));
        resource.capability_evidence.push(CapabilityEvidence {
            dimension: "coding".into(),
            score: CapabilityScore::new(80).unwrap(),
            source: CapabilityEvidenceSource::Manual,
            model: Some("m".into()),
            observed_at: Utc::now().to_rfc3339(),
            hardware_fingerprint: None,
            sample_count: 1,
            note: Some("GOD: conservative override".into()),
        });
        let outcomes = (0..18)
            .map(|i| outcome(&format!("bad{i}"), 70, OutcomeQuality::Negative))
            .collect::<Vec<_>>();
        let profile = CapabilityLearner.calibrate(
            &resource,
            &outcomes,
            &CapabilityLearningPolicy::default(),
            Utc::now(),
        );
        let coding = &profile.dimensions["coding"];
        assert!(coding.disagreement);
        assert_eq!(
            coding.evidence_composition[&CapabilityEvidenceSource::Manual],
            1
        );
        assert_eq!(
            coding.evidence_composition[&CapabilityEvidenceSource::BataiBenchmark],
            1
        );
        assert_eq!(
            coding.evidence_composition[&CapabilityEvidenceSource::RealTaskHistory],
            18
        );
        assert!(coding.routing_estimate.unwrap() < coding.estimated_score.unwrap());
    }

    #[test]
    fn model_specific_manual_evidence_does_not_leak_to_another_model() {
        let mut resource = resource(None);
        resource.supported_models = vec!["m".into(), "other".into()];
        resource.capability_evidence.push(CapabilityEvidence {
            dimension: "coding".into(),
            score: CapabilityScore::new(88).unwrap(),
            source: CapabilityEvidenceSource::Manual,
            model: Some("m".into()),
            observed_at: Utc::now().to_rfc3339(),
            hardware_fingerprint: None,
            sample_count: 1,
            note: Some("GOD".into()),
        });
        let policy = CapabilityLearningPolicy::default();
        let selected =
            CapabilityLearner.calibrate_model(&resource, Some("m"), &[], &policy, Utc::now());
        let other =
            CapabilityLearner.calibrate_model(&resource, Some("other"), &[], &policy, Utc::now());
        assert_eq!(selected.dimensions["coding"].estimated_score, Some(88.0));
        assert!(!other.dimensions.contains_key("coding"));
    }

    #[test]
    fn relevant_dimensions_do_not_touch_unrelated_research() {
        let dimensions =
            relevant_dimensions(AgentFunction::FrontendEngineering, &BTreeMap::new(), false);
        assert!(dimensions.contains(&"coding".into()));
        assert!(!dimensions.contains(&"research".into()));
    }

    #[test]
    fn independent_review_is_stronger_than_self_review() {
        let mut self_review = outcome("self", 60, OutcomeQuality::WeakPositive);
        self_review.apply_review(&ReviewOutcome {
            id: "r1".into(),
            task_id: "self".into(),
            reviewer_id: "a".into(),
            subject_agent_id: "a".into(),
            outcome: ReviewOutcomeKind::Accepted,
            note: None,
            created_at: Utc::now().to_rfc3339(),
        });
        let mut independent = outcome("ind", 60, OutcomeQuality::WeakPositive);
        independent.tests = TestEvidence {
            total: Some(2),
            passed: Some(2),
            failed: Some(0),
        };
        independent.apply_review(&ReviewOutcome {
            id: "r2".into(),
            task_id: "ind".into(),
            reviewer_id: "qa".into(),
            subject_agent_id: "a".into(),
            outcome: ReviewOutcomeKind::Accepted,
            note: None,
            created_at: Utc::now().to_rfc3339(),
        });
        assert_eq!(self_review.outcome_quality, OutcomeQuality::WeakPositive);
        assert_eq!(independent.outcome_quality, OutcomeQuality::StrongPositive);
    }

    #[test]
    fn shadow_replay_never_invents_alternative_outcome() {
        let decision = QuotaAwareEconomicRouter::test_fixture("t", "local");
        let replay = replay_result(
            &decision,
            &decision,
            None,
            Some(BillingMode::Local),
            Some(BillingMode::Local),
        );
        assert_eq!(replay.alternative_outcome, None);
        assert!(!replay.selection_changed);
    }

    #[test]
    fn promotion_is_suggestion_only_and_stops_before_staff() {
        let outcomes = (0..6)
            .map(|i| outcome(&format!("p{i}"), 60, OutcomeQuality::StrongPositive))
            .collect::<Vec<_>>();
        let suggestion = promotion_suggestion("a", Seniority::Junior, &outcomes).unwrap();
        assert_eq!(suggestion.suggested_seniority, Seniority::Associate);
        assert!(suggestion.requires_governance);
        assert!(promotion_suggestion("a", Seniority::Senior, &outcomes).is_none());
    }

    #[test]
    fn local_latency_is_separated_by_hardware_fingerprint() {
        let mut on_h = outcome("h1", 60, OutcomeQuality::MediumPositive);
        on_h.duration_ms = Some(100);
        let mut other = outcome("h2", 60, OutcomeQuality::MediumPositive);
        other.hardware_fingerprint = Some("other".into());
        other.duration_ms = Some(9_000);
        let profile = CapabilityLearner.calibrate_model(
            &resource(Some(70)),
            Some("m"),
            &[on_h, other],
            &CapabilityLearningPolicy::default(),
            Utc::now(),
        );
        assert_eq!(profile.median_latency_ms, Some(100));
        assert_eq!(profile.observed_tasks, 2);
    }

    #[test]
    fn calibration_metrics_distinguish_success_under_routing_and_review_rejection() {
        let decision = QuotaAwareEconomicRouter::test_fixture("t", "local");
        let mut success = outcome("success", 60, OutcomeQuality::StrongPositive);
        success.routing_decision_id = Some(decision.id.clone());
        let mut failed = outcome("failed", 70, OutcomeQuality::Negative);
        failed.routing_decision_id = Some(decision.id.clone());
        failed.review_outcome = Some(ReviewOutcomeKind::Rejected);
        let first = calibration_record(&decision, &success, None).unwrap();
        let second = calibration_record(&decision, &failed, None).unwrap();
        let dashboard = calibration_dashboard(&[decision], &[first, second]);
        assert_eq!(dashboard.calibrated_outcomes, 2);
        assert_eq!(dashboard.success_rate, Some(0.5));
        assert_eq!(dashboard.under_routing_rate, Some(0.5));
        assert_eq!(dashboard.review_rejection_rate, Some(0.5));
    }

    #[test]
    fn replay_is_deterministic_and_preserves_actual_only_for_original() {
        let original = QuotaAwareEconomicRouter::test_fixture("t", "local");
        let mut replayed = original.clone();
        replayed.selected_resource_id = Some("subscription".into());
        let actual = outcome("actual", 60, OutcomeQuality::StrongPositive);
        let first = replay_result(
            &original,
            &replayed,
            Some(&actual),
            Some(BillingMode::Local),
            Some(BillingMode::SubscriptionQuota),
        );
        let second = replay_result(
            &original,
            &replayed,
            Some(&actual),
            Some(BillingMode::Local),
            Some(BillingMode::SubscriptionQuota),
        );
        assert_eq!(first, second);
        assert_eq!(first.actual_outcome, Some(OutcomeQuality::StrongPositive));
        assert_eq!(first.alternative_outcome, None);
        assert!(first.selection_changed);
        assert!(first.cost_class_changed);
    }
}
