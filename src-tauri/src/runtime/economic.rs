//! Deterministic economic selection of an intelligence resource.
//!
//! The router does not guess missing quality, quota, terms, or prices. A candidate
//! must carry enough evidence to pass eligibility and quality gates before cost is
//! considered.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::organization::{AgentFunction, CapabilityProfile, CapabilityScore, Seniority};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResourceTier {
    Deterministic,
    Local,
    FreeHosted,
    SubscriptionQuota,
    NativeSubscriptionClient,
    CheapPayg,
    PremiumPayg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BillingMode {
    Local,
    FreeTier,
    SubscriptionQuota,
    NativeSubscriptionClient,
    Payg,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TermsState {
    Allowed,
    Disallowed,
    RequiresReview,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TermsProfile {
    pub third_party_allowed: TermsState,
    pub automation_allowed: TermsState,
    pub interactive_only: bool,
    pub native_client_only: bool,
    pub subscription_quota: bool,
    pub user_agent_integrity_required: bool,
    pub allowed_use_mode: TermsState,
    pub checked_at: Option<String>,
    pub source_url: Option<String>,
    /// Records that GOD saw the terms warning. This never changes eligibility by itself.
    pub acknowledged_at: Option<String>,
    pub acknowledged_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CapabilityEvidenceSource {
    ExternalBenchmark,
    BataiBenchmark,
    RealTaskHistory,
    Manual,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityEvidence {
    pub dimension: String,
    pub score: CapabilityScore,
    pub source: CapabilityEvidenceSource,
    #[serde(default)]
    pub model: Option<String>,
    pub observed_at: String,
    pub hardware_fingerprint: Option<String>,
    pub sample_count: u32,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QuotaState {
    pub used_percent: Option<f64>,
    pub remaining_units: Option<u64>,
    pub reset_at: Option<String>,
    pub observed_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ResourceUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub known_cost: Option<f64>,
    pub currency: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceProfile {
    pub id: String,
    pub provider: String,
    pub display_name: String,
    pub tier: ResourceTier,
    pub billing_mode: BillingMode,
    pub plan_name: Option<String>,
    pub fixed_monthly_cost: Option<f64>,
    pub fixed_cost_currency: Option<String>,
    pub marginal_cost_per_million_tokens: Option<f64>,
    pub supported_models: Vec<String>,
    pub capabilities: CapabilityProfile,
    pub capability_evidence: Vec<CapabilityEvidence>,
    pub context_window: Option<u64>,
    pub concurrency_limit: Option<u32>,
    pub current_concurrency: u32,
    pub quota: QuotaState,
    pub usage: ResourceUsage,
    pub status: String,
    pub terms: TermsProfile,
    pub latency_ms: Option<u64>,
    pub reliability: Option<CapabilityScore>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskRisk {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TaskRequirements {
    pub function: AgentFunction,
    pub complexity: u8,
    pub risk: TaskRisk,
    pub context_tokens: Option<u64>,
    pub required_capabilities: BTreeMap<String, u8>,
    pub requires_tools: bool,
    pub requires_worktree: bool,
    pub deadline_at: Option<String>,
    pub priority: u8,
    #[serde(default = "default_taxonomy_version")]
    pub taxonomy_version: String,
}

impl Default for TaskRequirements {
    fn default() -> Self {
        Self {
            function: AgentFunction::GenericSoftwareAgent,
            complexity: 50,
            risk: TaskRisk::Medium,
            context_tokens: None,
            required_capabilities: BTreeMap::new(),
            requires_tools: false,
            requires_worktree: false,
            deadline_at: None,
            priority: 50,
            taxonomy_version: default_taxonomy_version(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EconomicPolicy {
    pub prefer_local: bool,
    pub prefer_subscription: bool,
    pub allow_payg: bool,
    pub max_payg_per_task: Option<f64>,
    pub min_capability_margin: u8,
    pub quota_conservation_mode: bool,
    pub forbidden_providers: BTreeSet<String>,
    pub allow_automatic_provider_change: bool,
}

impl Default for EconomicPolicy {
    fn default() -> Self {
        Self {
            prefer_local: true,
            prefer_subscription: true,
            allow_payg: false,
            max_payg_per_task: None,
            min_capability_margin: 3,
            quota_conservation_mode: true,
            forbidden_providers: BTreeSet::new(),
            allow_automatic_provider_change: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateDecision {
    pub resource_id: String,
    pub provider: String,
    pub model: Option<String>,
    pub eligible: bool,
    pub sufficient: bool,
    pub rejected_reasons: Vec<String>,
    pub quality_score: Option<f64>,
    pub economic_score: Option<f64>,
    pub availability_score: Option<f64>,
    pub total_score: Option<f64>,
    #[serde(default)]
    pub capability_confidence: Option<f64>,
    #[serde(default)]
    pub conservative_quality_score: Option<f64>,
    #[serde(default)]
    pub evidence_disagreement: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RoutingOutcome {
    Selected,
    NoSuitableResource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingDecision {
    pub id: String,
    pub task_id: String,
    pub timestamp: String,
    pub outcome: RoutingOutcome,
    pub selected_resource_id: Option<String>,
    pub selected_provider: Option<String>,
    pub selected_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub reasons: Vec<String>,
    pub alternatives: Vec<String>,
    pub candidates: Vec<CandidateDecision>,
    pub policy: EconomicPolicy,
    #[serde(default = "default_router_version")]
    pub router_version: String,
    #[serde(default = "default_scoring_policy_version")]
    pub scoring_policy_version: String,
    #[serde(default)]
    pub requirements: TaskRequirements,
    #[serde(default)]
    pub shadow_ranking: Vec<String>,
    #[serde(default)]
    pub resource_snapshot: Vec<ResourceProfile>,
}

fn default_router_version() -> String {
    "economic-router-v2".into()
}

fn default_scoring_policy_version() -> String {
    "economic-policy-v2-confidence-aware".into()
}

fn default_taxonomy_version() -> String {
    super::learning::TASK_TAXONOMY_VERSION.into()
}

#[derive(Debug, Default, Clone)]
pub struct QuotaAwareEconomicRouter;

impl QuotaAwareEconomicRouter {
    pub fn route(
        &self,
        task_id: &str,
        requirements: &TaskRequirements,
        resources: &[ResourceProfile],
        policy: &EconomicPolicy,
        now: DateTime<Utc>,
    ) -> RoutingDecision {
        self.route_with_evidence(
            task_id,
            requirements,
            resources,
            policy,
            &[],
            &super::learning::CapabilityLearningPolicy::default(),
            now,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn route_with_evidence(
        &self,
        task_id: &str,
        requirements: &TaskRequirements,
        resources: &[ResourceProfile],
        policy: &EconomicPolicy,
        outcomes: &[super::learning::TaskOutcomeEvidence],
        learning_policy: &super::learning::CapabilityLearningPolicy,
        now: DateTime<Utc>,
    ) -> RoutingDecision {
        let calibrations = resources
            .iter()
            .flat_map(|resource| {
                let models = if resource.supported_models.is_empty() {
                    vec![None]
                } else {
                    resource
                        .supported_models
                        .iter()
                        .map(|model| Some(model.clone()))
                        .collect()
                };
                models.into_iter().map(|model| {
                    let profile = super::learning::CapabilityLearner.calibrate_model(
                        resource,
                        model.as_deref(),
                        outcomes,
                        learning_policy,
                        now,
                    );
                    ((resource.id.clone(), model), profile)
                })
            })
            .collect::<BTreeMap<_, _>>();
        let calibration_index = &calibrations;
        let mut decisions = resources
            .iter()
            .flat_map(|resource| {
                let models = if resource.supported_models.is_empty() {
                    vec![None]
                } else {
                    resource
                        .supported_models
                        .iter()
                        .map(|model| Some(model.clone()))
                        .collect()
                };
                models.into_iter().map(move |model| {
                    let calibration = calibration_index.get(&(resource.id.clone(), model.clone()));
                    evaluate_candidate(
                        resource,
                        model,
                        requirements,
                        policy,
                        calibration,
                        learning_policy,
                        now,
                    )
                })
            })
            .collect::<Vec<_>>();
        decisions.sort_by(|left, right| {
            right
                .total_score
                .partial_cmp(&left.total_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.resource_id.cmp(&right.resource_id))
                .then_with(|| left.model.cmp(&right.model))
        });

        let selected = decisions
            .iter()
            .find(|candidate| candidate.eligible && candidate.sufficient);
        let (outcome, resource_id, provider, model, effort, reasons, alternatives) =
            if let Some(selected) = selected {
                let resource = resources
                    .iter()
                    .find(|resource| resource.id == selected.resource_id)
                    .expect("candidate came from resource");
                let mut reasons = vec![format!(
                    "Quality requirement satisfied with a {:.1} capability score",
                    selected.quality_score.unwrap_or_default()
                )];
                reasons.push(economic_reason(resource, now));
                if let Some(confidence) = selected.capability_confidence {
                    reasons.push(format!(
                        "Capability confidence is {} ({confidence:.2}) from retained evidence",
                        super::learning::confidence_band(confidence).label()
                    ));
                }
                if selected.evidence_disagreement {
                    reasons.push(
                    "Benchmark and real-task evidence disagree; conservative capability was used"
                        .into(),
                );
                }
                if resource.billing_mode == BillingMode::SubscriptionQuota
                    && resource.quota.reset_at.is_some()
                {
                    reasons
                        .push("Available subscription quota is considered before its reset".into());
                }
                let alternatives = decisions
                    .iter()
                    .filter(|candidate| candidate.resource_id != selected.resource_id)
                    .take(3)
                    .map(|candidate| {
                        if candidate.rejected_reasons.is_empty() {
                            format!("{} ranked lower economically", candidate.resource_id)
                        } else {
                            format!(
                                "{}: {}",
                                candidate.resource_id,
                                candidate.rejected_reasons.join(", ")
                            )
                        }
                    })
                    .collect();
                (
                    RoutingOutcome::Selected,
                    Some(selected.resource_id.clone()),
                    Some(selected.provider.clone()),
                    selected.model.clone(),
                    Some(reasoning_effort(requirements).into()),
                    reasons,
                    alternatives,
                )
            } else {
                let mut reasons = decisions
                    .iter()
                    .flat_map(|candidate| candidate.rejected_reasons.iter().cloned())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                if reasons.is_empty() {
                    reasons.push("No intelligence resource is configured".into());
                }
                (
                    RoutingOutcome::NoSuitableResource,
                    None,
                    None,
                    None,
                    None,
                    reasons,
                    vec![
                        "Install or benchmark a capable local model".into(),
                        "Wait for subscription quota or connect an approved resource".into(),
                        "Ask GOD to permit PAYG for this task".into(),
                    ],
                )
            };

        let shadow_ranking = decisions
            .iter()
            .filter(|candidate| candidate.eligible && candidate.sufficient)
            .map(|candidate| {
                format!(
                    "{}:{}",
                    candidate.resource_id,
                    candidate.model.as_deref().unwrap_or("provider-managed")
                )
            })
            .collect();
        RoutingDecision {
            id: format!("ROUTE-{}", uuid::Uuid::new_v4()),
            task_id: task_id.into(),
            timestamp: now.to_rfc3339(),
            outcome,
            selected_resource_id: resource_id,
            selected_provider: provider,
            selected_model: model,
            reasoning_effort: effort,
            reasons,
            alternatives,
            candidates: decisions,
            policy: policy.clone(),
            router_version: default_router_version(),
            scoring_policy_version: default_scoring_policy_version(),
            requirements: requirements.clone(),
            shadow_ranking,
            resource_snapshot: resources.to_vec(),
        }
    }

    #[cfg(test)]
    pub fn test_fixture(task_id: &str, resource_id: &str) -> RoutingDecision {
        let resource = native_resource_profile(resource_id, "ollama", "Test", BillingMode::Local);
        RoutingDecision {
            id: format!("route-{task_id}"),
            task_id: task_id.into(),
            timestamp: Utc::now().to_rfc3339(),
            outcome: RoutingOutcome::Selected,
            selected_resource_id: Some(resource_id.into()),
            selected_provider: Some("ollama".into()),
            selected_model: Some("m".into()),
            reasoning_effort: Some("MEDIUM".into()),
            reasons: vec![],
            alternatives: vec![],
            candidates: vec![],
            policy: EconomicPolicy::default(),
            router_version: default_router_version(),
            scoring_policy_version: default_scoring_policy_version(),
            requirements: TaskRequirements::default(),
            shadow_ranking: vec![],
            resource_snapshot: vec![resource],
        }
    }
}

fn evaluate_candidate(
    resource: &ResourceProfile,
    model: Option<String>,
    requirements: &TaskRequirements,
    policy: &EconomicPolicy,
    calibration: Option<&super::learning::CalibratedCapabilityProfile>,
    learning_policy: &super::learning::CapabilityLearningPolicy,
    now: DateTime<Utc>,
) -> CandidateDecision {
    let mut rejected = Vec::new();
    if policy
        .forbidden_providers
        .contains(&resource.provider.to_ascii_lowercase())
    {
        rejected.push("provider forbidden by policy".into());
    }
    if !matches!(resource.status.as_str(), "AVAILABLE" | "LOW" | "DEGRADED") {
        rejected.push(format!("resource status is {}", resource.status));
    }
    if resource.terms.allowed_use_mode != TermsState::Allowed
        || resource.terms.automation_allowed == TermsState::Disallowed
        || resource.terms.third_party_allowed == TermsState::Disallowed
    {
        rejected.push("terms do not authorize this execution mode".into());
    }
    if resource.billing_mode == BillingMode::Payg && !policy.allow_payg {
        rejected.push("PAYG disabled by policy".into());
    }
    if resource.billing_mode == BillingMode::Payg
        && resource.marginal_cost_per_million_tokens.is_none()
    {
        rejected.push("PAYG marginal cost is unknown".into());
    }
    if resource.billing_mode == BillingMode::Payg {
        if let Some(cap) = policy.max_payg_per_task {
            match requirements
                .context_tokens
                .zip(resource.marginal_cost_per_million_tokens)
            {
                Some((tokens, rate))
                    if (tokens.saturating_mul(2) as f64 / 1_000_000.0) * rate > cap =>
                {
                    rejected.push("estimated PAYG cost exceeds per-task cap".into());
                }
                None => rejected.push("PAYG cap cannot be enforced with unknown task size".into()),
                _ => {}
            }
        }
    }
    if resource
        .concurrency_limit
        .is_some_and(|limit| resource.current_concurrency >= limit)
    {
        rejected.push("concurrency limit reached".into());
    }
    if requirements
        .context_tokens
        .zip(resource.context_window)
        .is_some_and(|(required, available)| required > available)
    {
        rejected.push("context window too small".into());
    }
    if resource
        .quota
        .used_percent
        .is_some_and(|used| used >= 100.0)
    {
        rejected.push("quota exhausted".into());
    }

    let routing_profile = calibration
        .map(|calibration| calibration.routing_profile(&resource.capabilities))
        .unwrap_or_else(|| resource.capabilities.clone());
    let quality = quality_score(&routing_profile, requirements);
    let capability_confidence =
        calibration.and_then(|calibration| calibration.minimum_confidence_for(requirements));
    let evidence_disagreement = calibration.is_some_and(|calibration| {
        super::learning::relevant_dimensions(
            requirements.function,
            &requirements.required_capabilities,
            false,
        )
        .iter()
        .any(|dimension| {
            calibration
                .dimensions
                .get(dimension)
                .is_some_and(|value| value.disagreement)
        })
    });
    if matches!(requirements.risk, TaskRisk::High | TaskRisk::Critical)
        && capability_confidence.unwrap_or(0.0) < learning_policy.high_risk_min_confidence
    {
        rejected.push("capability confidence too low for high-risk task".into());
    }
    let threshold = required_quality(requirements, policy);
    let dimensions_sufficient =
        requirements
            .required_capabilities
            .iter()
            .all(|(dimension, minimum)| {
                capability(&routing_profile, dimension).is_some_and(|score| {
                    score >= minimum.saturating_add(policy.min_capability_margin)
                })
            });
    let sufficient = quality.is_some_and(|score| score >= threshold) && dimensions_sufficient;
    if quality.is_none() {
        rejected.push("required capability evidence is unknown".into());
    } else if !sufficient {
        rejected.push(format!("capability below required {:.1}", threshold));
    }
    let eligible = rejected.is_empty();
    let economic = eligible.then(|| economic_score(resource, policy, now));
    let availability = eligible.then(|| availability_score(resource));
    let total =
        quality
            .zip(economic)
            .zip(availability)
            .map(|((quality, economics), availability)| {
                let bounded_quality = quality.min(threshold + 10.0);
                economics * 0.60 + availability * 0.20 + bounded_quality * 0.20
            });
    CandidateDecision {
        resource_id: resource.id.clone(),
        provider: resource.provider.clone(),
        model,
        eligible,
        sufficient,
        rejected_reasons: rejected,
        quality_score: quality,
        economic_score: economic,
        availability_score: availability,
        total_score: total,
        capability_confidence,
        conservative_quality_score: quality,
        evidence_disagreement,
    }
}

impl super::learning::ConfidenceBand {
    pub const fn label(self) -> &'static str {
        match self {
            Self::VeryLow => "VERY_LOW",
            Self::Low => "LOW",
            Self::Medium => "MEDIUM",
            Self::High => "HIGH",
            Self::VeryHigh => "VERY_HIGH",
        }
    }
}

fn capability(profile: &CapabilityProfile, name: &str) -> Option<u8> {
    let score = match name {
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
    };
    score.map(CapabilityScore::get)
}

fn quality_score(profile: &CapabilityProfile, requirements: &TaskRequirements) -> Option<f64> {
    let mut required = requirements.required_capabilities.clone();
    if required.is_empty() {
        let primary = match requirements.function.department() {
            super::organization::Department::Product
            | super::organization::Department::Leadership => "planning",
            super::organization::Department::Architecture => "architecture",
            super::organization::Department::Quality => "review",
            super::organization::Department::Research => "research",
            _ => "coding",
        };
        required.insert(primary.into(), requirements.complexity);
    }
    if requirements.requires_tools {
        required.entry("tool_use".into()).or_insert(55);
    }
    let values = required
        .keys()
        .map(|dimension| capability(profile, dimension))
        .collect::<Option<Vec<_>>>()?;
    Some(values.iter().map(|score| f64::from(*score)).sum::<f64>() / values.len() as f64)
}

fn required_quality(requirements: &TaskRequirements, policy: &EconomicPolicy) -> f64 {
    let risk_addition = match requirements.risk {
        TaskRisk::Low => 0,
        TaskRisk::Medium => 3,
        TaskRisk::High => 10,
        TaskRisk::Critical => 18,
    };
    f64::from(
        requirements
            .complexity
            .saturating_add(risk_addition)
            .saturating_add(policy.min_capability_margin)
            .min(100),
    )
}

fn economic_score(resource: &ResourceProfile, policy: &EconomicPolicy, now: DateTime<Utc>) -> f64 {
    let base = match resource.billing_mode {
        BillingMode::Local => 96.0,
        BillingMode::FreeTier => 94.0,
        BillingMode::SubscriptionQuota => 88.0,
        BillingMode::NativeSubscriptionClient => 82.0,
        BillingMode::Payg => resource
            .marginal_cost_per_million_tokens
            .map(|cost| (65.0 - cost.min(60.0)).max(0.0))
            .unwrap_or(0.0),
    };
    let preference = if policy.prefer_local && resource.billing_mode == BillingMode::Local {
        4.0
    } else if policy.prefer_subscription
        && matches!(
            resource.billing_mode,
            BillingMode::SubscriptionQuota | BillingMode::NativeSubscriptionClient
        )
    {
        3.0
    } else {
        0.0
    };
    let expiry_bonus = if policy.quota_conservation_mode
        && resource.billing_mode == BillingMode::SubscriptionQuota
    {
        quota_expiry_bonus(&resource.quota, now)
    } else {
        0.0
    };
    (base + preference + expiry_bonus).min(100.0)
}

fn quota_expiry_bonus(quota: &QuotaState, now: DateTime<Utc>) -> f64 {
    let unused = 100.0 - quota.used_percent.unwrap_or(100.0);
    let hours = quota
        .reset_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|reset| (reset.with_timezone(&Utc) - now).num_minutes() as f64 / 60.0);
    match hours {
        Some(hours) if (0.0..=6.0).contains(&hours) && unused >= 10.0 => 8.0,
        Some(hours) if (0.0..=24.0).contains(&hours) && unused >= 20.0 => 4.0,
        _ => 0.0,
    }
}

fn availability_score(resource: &ResourceProfile) -> f64 {
    let concurrency = match resource.concurrency_limit {
        Some(limit) if limit > 0 => {
            100.0 * (1.0 - f64::from(resource.current_concurrency) / f64::from(limit))
        }
        Some(_) => 0.0,
        None => 75.0,
    };
    let latency = resource
        .latency_ms
        .map(|latency| (100.0 - latency as f64 / 50.0).max(10.0))
        .unwrap_or(60.0);
    let reliability = resource
        .reliability
        .or(resource.capabilities.reliability)
        .map_or(50.0, |score| f64::from(score.get()));
    concurrency * 0.45 + latency * 0.25 + reliability * 0.30
}

fn economic_reason(resource: &ResourceProfile, now: DateTime<Utc>) -> String {
    match resource.billing_mode {
        BillingMode::Local => "Local execution has no provider-reported marginal API cost".into(),
        BillingMode::FreeTier => "Free-tier capacity has no configured marginal cost".into(),
        BillingMode::SubscriptionQuota => {
            if quota_expiry_bonus(&resource.quota, now) > 0.0 {
                "Subscription quota has unused capacity and resets soon".into()
            } else {
                "Subscription quota has no configured per-request marginal charge".into()
            }
        }
        BillingMode::NativeSubscriptionClient => {
            "Native subscription capacity is available without PAYG selection".into()
        }
        BillingMode::Payg => "PAYG is explicitly allowed and within configured policy".into(),
    }
}

fn reasoning_effort(requirements: &TaskRequirements) -> &'static str {
    if requirements.risk == TaskRisk::Critical || requirements.complexity >= 85 {
        "MAX"
    } else if requirements.risk == TaskRisk::High || requirements.complexity >= 70 {
        "HIGH"
    } else if requirements.complexity >= 35 {
        "MEDIUM"
    } else {
        "LOW"
    }
}

pub fn role_recommendations(profile: &CapabilityProfile) -> Vec<(AgentFunction, Seniority)> {
    let policy = super::organization::RecommendationPolicy::default();
    [
        AgentFunction::SoftwareEngineer,
        AgentFunction::BackendEngineering,
        AgentFunction::ProductAnalyst,
        AgentFunction::ResearchAnalyst,
        AgentFunction::QaEngineering,
        AgentFunction::SoftwareArchitecture,
    ]
    .into_iter()
    .filter_map(|function| {
        policy
            .recommend(function, profile)
            .map(|level| (function, level))
    })
    .collect()
}

pub fn native_resource_profile(
    id: &str,
    provider: &str,
    display_name: &str,
    billing_mode: BillingMode,
) -> ResourceProfile {
    ResourceProfile {
        id: id.into(),
        provider: provider.into(),
        display_name: display_name.into(),
        tier: if billing_mode == BillingMode::Local {
            ResourceTier::Local
        } else {
            ResourceTier::NativeSubscriptionClient
        },
        billing_mode,
        plan_name: None,
        fixed_monthly_cost: None,
        fixed_cost_currency: None,
        marginal_cost_per_million_tokens: None,
        supported_models: vec![],
        capabilities: CapabilityProfile::default(),
        capability_evidence: vec![],
        context_window: None,
        concurrency_limit: None,
        current_concurrency: 0,
        quota: QuotaState::default(),
        usage: ResourceUsage::default(),
        status: "UNKNOWN".into(),
        terms: TermsProfile {
            third_party_allowed: TermsState::Allowed,
            automation_allowed: TermsState::Allowed,
            allowed_use_mode: TermsState::Allowed,
            native_client_only: billing_mode == BillingMode::NativeSubscriptionClient,
            checked_at: Some("2026-09-11".into()),
            ..TermsProfile::default()
        },
        latency_ms: None,
        reliability: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(
        id: &str,
        resource_id: &str,
        model: &str,
        complexity: u8,
        quality: super::super::learning::OutcomeQuality,
    ) -> super::super::learning::TaskOutcomeEvidence {
        super::super::learning::TaskOutcomeEvidence {
            id: id.into(),
            task_id: id.into(),
            task_run_id: format!("{id}:a:1"),
            agent_id: "a".into(),
            resource_id: resource_id.into(),
            provider: resource_id.into(),
            model: model.into(),
            model_identity_mutable: false,
            hardware_fingerprint: None,
            function: AgentFunction::BackendEngineering,
            complexity,
            risk: TaskRisk::Medium,
            required_capabilities: BTreeMap::new(),
            taxonomy_version: super::super::learning::TASK_TAXONOMY_VERSION.into(),
            selected_reasoning_effort: Some("MEDIUM".into()),
            started_at: None,
            completed_at: Utc::now().to_rfc3339(),
            succeeded: !matches!(quality, super::super::learning::OutcomeQuality::Negative),
            outcome_quality: quality,
            failure_classification: matches!(
                quality,
                super::super::learning::OutcomeQuality::Negative
            )
            .then_some(super::super::learning::FailureClassification::ModelQuality),
            retries: 0,
            retry_classification: None,
            review_outcome: None,
            review_evidence_kind: super::super::learning::ReviewEvidenceKind::None,
            tests: Default::default(),
            changed_files_count: Some(1),
            duration_ms: Some(100),
            input_tokens: None,
            output_tokens: None,
            provider_reported_cost: None,
            currency: None,
            routing_decision_id: None,
        }
    }

    fn profile(coding: u8, planning: u8) -> CapabilityProfile {
        CapabilityProfile {
            coding: Some(CapabilityScore::new(coding).unwrap()),
            debugging: Some(CapabilityScore::new(coding).unwrap()),
            planning: Some(CapabilityScore::new(planning).unwrap()),
            architecture: Some(CapabilityScore::new(planning).unwrap()),
            tool_use: Some(CapabilityScore::new(coding).unwrap()),
            instruction_following: Some(CapabilityScore::new(planning).unwrap()),
            review: Some(CapabilityScore::new(coding).unwrap()),
            research: Some(CapabilityScore::new(planning).unwrap()),
            ..CapabilityProfile::default()
        }
    }

    fn resource(id: &str, billing_mode: BillingMode, coding: u8) -> ResourceProfile {
        ResourceProfile {
            id: id.into(),
            provider: id.into(),
            display_name: id.into(),
            tier: if billing_mode == BillingMode::Local {
                ResourceTier::Local
            } else {
                ResourceTier::SubscriptionQuota
            },
            billing_mode,
            plan_name: None,
            fixed_monthly_cost: None,
            fixed_cost_currency: None,
            marginal_cost_per_million_tokens: None,
            supported_models: vec![format!("{id}-model")],
            capabilities: profile(coding, coding),
            capability_evidence: vec![],
            context_window: Some(128_000),
            concurrency_limit: Some(2),
            current_concurrency: 0,
            quota: QuotaState::default(),
            usage: ResourceUsage::default(),
            status: "AVAILABLE".into(),
            terms: TermsProfile {
                third_party_allowed: TermsState::Allowed,
                automation_allowed: TermsState::Allowed,
                allowed_use_mode: TermsState::Allowed,
                ..TermsProfile::default()
            },
            latency_ms: Some(100),
            reliability: Some(CapabilityScore::new(coding).unwrap()),
        }
    }

    fn route(
        resources: &[ResourceProfile],
        requirements: TaskRequirements,
        policy: EconomicPolicy,
    ) -> RoutingDecision {
        QuotaAwareEconomicRouter.route("TASK-1", &requirements, resources, &policy, Utc::now())
    }

    #[test]
    fn cheap_local_wins_when_sufficient_but_not_when_weak() {
        let local = resource("local", BillingMode::Local, 75);
        let subscription = resource("subscription", BillingMode::SubscriptionQuota, 90);
        let decision = route(
            &[local.clone(), subscription.clone()],
            TaskRequirements {
                complexity: 60,
                ..Default::default()
            },
            EconomicPolicy::default(),
        );
        assert_eq!(decision.selected_resource_id.as_deref(), Some("local"));
        let decision = route(
            &[resource("weak", BillingMode::Local, 45), subscription],
            TaskRequirements {
                complexity: 70,
                ..Default::default()
            },
            EconomicPolicy::default(),
        );
        assert_eq!(
            decision.selected_resource_id.as_deref(),
            Some("subscription")
        );
    }

    #[test]
    fn near_expiry_subscription_is_consumed_first_when_quality_equal() {
        let now = Utc::now();
        let mut soon = resource("soon", BillingMode::SubscriptionQuota, 80);
        soon.quota.used_percent = Some(40.0);
        soon.quota.reset_at = Some((now + chrono::Duration::hours(2)).to_rfc3339());
        let mut later = resource("later", BillingMode::SubscriptionQuota, 80);
        later.quota.used_percent = Some(90.0);
        later.quota.reset_at = Some((now + chrono::Duration::days(4)).to_rfc3339());
        let decision = QuotaAwareEconomicRouter.route(
            "T",
            &TaskRequirements {
                complexity: 60,
                ..Default::default()
            },
            &[later, soon],
            &EconomicPolicy::default(),
            now,
        );
        assert_eq!(decision.selected_resource_id.as_deref(), Some("soon"));
    }

    #[test]
    fn high_risk_rejects_weak_and_payg_policy_is_fail_closed() {
        let weak = resource("weak", BillingMode::Local, 82);
        let mut payg = resource("payg", BillingMode::Payg, 99);
        payg.tier = ResourceTier::PremiumPayg;
        payg.marginal_cost_per_million_tokens = Some(20.0);
        let decision = route(
            &[weak, payg],
            TaskRequirements {
                complexity: 80,
                risk: TaskRisk::High,
                ..Default::default()
            },
            EconomicPolicy::default(),
        );
        assert_eq!(decision.outcome, RoutingOutcome::NoSuitableResource);
        assert!(decision
            .reasons
            .iter()
            .any(|reason| reason.contains("PAYG")));
    }

    #[test]
    fn terms_concurrency_and_unavailability_are_hard_gates() {
        let mut blocked = resource("blocked", BillingMode::SubscriptionQuota, 90);
        blocked.terms.allowed_use_mode = TermsState::RequiresReview;
        let mut busy = resource("busy", BillingMode::SubscriptionQuota, 90);
        busy.current_concurrency = 2;
        let decision = route(
            &[blocked, busy],
            TaskRequirements {
                complexity: 60,
                ..Default::default()
            },
            EconomicPolicy::default(),
        );
        assert_eq!(decision.outcome, RoutingOutcome::NoSuitableResource);
    }

    #[test]
    fn role_recommendations_are_dimension_specific_and_never_auto_above_senior() {
        let recommendations = role_recommendations(&profile(60, 90));
        assert!(recommendations.contains(&(AgentFunction::SoftwareEngineer, Seniority::Junior)));
        assert!(recommendations.contains(&(AgentFunction::ProductAnalyst, Seniority::Senior)));
        assert!(recommendations.iter().all(|(_, level)| level.level() <= 4));
    }

    #[test]
    fn routing_is_stable_for_same_inputs() {
        let resources = [
            resource("b", BillingMode::SubscriptionQuota, 80),
            resource("a", BillingMode::SubscriptionQuota, 80),
        ];
        let now = Utc::now();
        let router = QuotaAwareEconomicRouter;
        let first = router.route(
            "T",
            &TaskRequirements {
                complexity: 60,
                ..Default::default()
            },
            &resources,
            &EconomicPolicy::default(),
            now,
        );
        let second = router.route(
            "T",
            &TaskRequirements {
                complexity: 60,
                ..Default::default()
            },
            &resources,
            &EconomicPolicy::default(),
            now,
        );
        assert_eq!(first.selected_resource_id, second.selected_resource_id);
        assert_eq!(first.selected_resource_id.as_deref(), Some("a"));
    }

    #[test]
    fn payg_cap_and_rate_limited_resources_are_excluded() {
        let mut payg = resource("payg", BillingMode::Payg, 95);
        payg.tier = ResourceTier::CheapPayg;
        payg.marginal_cost_per_million_tokens = Some(20.0);
        let policy = EconomicPolicy {
            allow_payg: true,
            max_payg_per_task: Some(0.01),
            ..EconomicPolicy::default()
        };
        let decision = route(
            &[payg],
            TaskRequirements {
                complexity: 50,
                context_tokens: Some(10_000),
                ..Default::default()
            },
            policy,
        );
        assert_eq!(decision.outcome, RoutingOutcome::NoSuitableResource);
        assert!(decision
            .reasons
            .iter()
            .any(|reason| reason.contains("cost")));
        let mut limited = resource("limited", BillingMode::SubscriptionQuota, 95);
        limited.status = "RATE_LIMITED".into();
        let decision = route(
            &[limited],
            TaskRequirements {
                complexity: 50,
                ..Default::default()
            },
            EconomicPolicy::default(),
        );
        assert_eq!(decision.outcome, RoutingOutcome::NoSuitableResource);
    }

    #[test]
    fn confidence_aware_history_can_reject_weak_local_and_allow_validated_local() {
        let local = resource("local", BillingMode::Local, 82);
        let subscription = resource("subscription", BillingMode::SubscriptionQuota, 88);
        let bad = (0..16)
            .map(|index| {
                outcome(
                    &format!("bad-{index}"),
                    "local",
                    "local-model",
                    70,
                    super::super::learning::OutcomeQuality::Negative,
                )
            })
            .collect::<Vec<_>>();
        let requirements = TaskRequirements {
            function: AgentFunction::BackendEngineering,
            complexity: 65,
            ..Default::default()
        };
        let decision = QuotaAwareEconomicRouter.route_with_evidence(
            "bad-history",
            &requirements,
            &[local.clone(), subscription.clone()],
            &EconomicPolicy::default(),
            &bad,
            &super::super::learning::CapabilityLearningPolicy::default(),
            Utc::now(),
        );
        assert_eq!(
            decision.selected_resource_id.as_deref(),
            Some("subscription")
        );
        let good = (0..20)
            .map(|index| {
                outcome(
                    &format!("good-{index}"),
                    "local",
                    "local-model",
                    72,
                    super::super::learning::OutcomeQuality::StrongPositive,
                )
            })
            .collect::<Vec<_>>();
        let decision = QuotaAwareEconomicRouter.route_with_evidence(
            "good-history",
            &requirements,
            &[local, subscription],
            &EconomicPolicy::default(),
            &good,
            &super::super::learning::CapabilityLearningPolicy::default(),
            Utc::now(),
        );
        assert_eq!(decision.selected_resource_id.as_deref(), Some("local"));
        assert!(
            decision
                .candidates
                .iter()
                .find(|candidate| candidate.resource_id == "local")
                .unwrap()
                .capability_confidence
                .unwrap()
                >= 0.65
        );
    }

    #[test]
    fn high_risk_task_rejects_sparse_uncertain_capability() {
        let local = resource("local", BillingMode::Local, 95);
        let sparse = vec![outcome(
            "one",
            "local",
            "local-model",
            90,
            super::super::learning::OutcomeQuality::StrongPositive,
        )];
        let decision = QuotaAwareEconomicRouter.route_with_evidence(
            "high-risk",
            &TaskRequirements {
                function: AgentFunction::BackendEngineering,
                complexity: 75,
                risk: TaskRisk::High,
                ..Default::default()
            },
            &[local],
            &EconomicPolicy::default(),
            &sparse,
            &super::super::learning::CapabilityLearningPolicy::default(),
            Utc::now(),
        );
        assert_eq!(decision.outcome, RoutingOutcome::NoSuitableResource);
        assert!(decision
            .reasons
            .iter()
            .any(|reason| reason.contains("confidence")));
    }
}
