use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::runtime::{
    benchmark::{BenchmarkResult, LocalModelCatalogEntry, ModelFitAssessment},
    economic::{EconomicPolicy, ResourceProfile, RoutingDecision},
    execution_provider::UsageSnapshot,
    governance::GovernanceSnapshot,
    learning::{
        CalibratedCapabilityProfile, CalibrationDashboard, CalibrationSuggestion,
        CapabilityLearningPolicy, PromotionSuggestion, RoutingCalibrationRecord,
        TaskOutcomeEvidence,
    },
    organization::{CapabilityProfile, OrganizationRelationship},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cost: Option<f64>,
    pub currency: Option<String>,
    pub sources: Vec<String>,
}

impl UsageSummary {
    pub fn add(&mut self, usage: &UsageSnapshot) {
        add_optional(&mut self.input_tokens, usage.input_tokens);
        add_optional(&mut self.cached_input_tokens, usage.cached_input_tokens);
        add_optional(&mut self.output_tokens, usage.output_tokens);
        add_optional(
            &mut self.total_tokens,
            usage
                .input_tokens
                .zip(usage.output_tokens)
                .map(|(input, output)| input + output),
        );
        add_optional_f64(&mut self.cost, usage.cost);
        if let Some(currency) = &usage.currency {
            match &self.currency {
                None => self.currency = Some(currency.clone()),
                Some(existing) if existing != currency => self.currency = None,
                _ => {}
            }
        }
        let source = serde_json::to_value(&usage.source)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".into());
        if source != "unknown" && !self.sources.contains(&source) {
            self.sources.push(source);
            self.sources.sort();
        }
    }

    pub fn merge(&mut self, other: &Self) {
        add_optional(&mut self.input_tokens, other.input_tokens);
        add_optional(&mut self.cached_input_tokens, other.cached_input_tokens);
        add_optional(&mut self.output_tokens, other.output_tokens);
        add_optional(&mut self.total_tokens, other.total_tokens);
        add_optional_f64(&mut self.cost, other.cost);
        if self.currency.is_none() {
            self.currency = other.currency.clone();
        } else if other.currency.is_some() && self.currency != other.currency {
            self.currency = None;
        }
        for source in &other.sources {
            if !self.sources.contains(source) {
                self.sources.push(source.clone());
            }
        }
        self.sources.sort();
    }
}

fn add_optional(target: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *target = Some(target.unwrap_or_default().saturating_add(value));
    }
}

fn add_optional_f64(target: &mut Option<f64>, value: Option<f64>) {
    if let Some(value) = value {
        *target = Some(target.unwrap_or_default() + value);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceSummary {
    pub completed_tasks: usize,
    pub active_tasks: usize,
    pub failed_tasks: usize,
    pub first_attempt_success_rate: Option<f64>,
    pub final_success_rate: Option<f64>,
    pub average_attempts: Option<f64>,
    pub review_acceptance_rate: Option<f64>,
    pub average_task_duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    pub id: String,
    pub name: String,
    pub title: String,
    pub department: String,
    pub seniority: Option<String>,
    pub level: Option<u8>,
    pub function: String,
    pub reports_to: Option<String>,
    pub direct_reports: Vec<String>,
    pub provider: String,
    pub model: String,
    pub reasoning_effort: String,
    pub auth_mode: String,
    pub status: String,
    pub activity: String,
    pub current_task_id: Option<String>,
    pub current_task_objective: Option<String>,
    pub worktree: Option<String>,
    pub session_state: Option<String>,
    pub model_capabilities: CapabilityProfile,
    pub effective_capabilities: CapabilityProfile,
    pub performance: PerformanceSummary,
    pub usage: UsageSummary,
    pub quota_status: Option<String>,
    pub quota_reset_at: Option<String>,
    pub lifecycle: String,
    pub authority: String,
    pub permissions: Vec<String>,
    pub intelligence_policy: crate::runtime::organization::IntelligencePolicy,
    pub history: Vec<ActivityView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    pub id: String,
    pub objective: String,
    pub status: String,
    pub assigned_to: Vec<String>,
    pub dependencies: Vec<String>,
    pub weight: f64,
    pub progress: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub name: String,
    pub root: String,
    pub progress: f64,
    pub active_agents: usize,
    pub waiting_agents: usize,
    pub completed_tasks: usize,
    pub running_tasks: usize,
    pub blocked_tasks: usize,
    pub remaining_tasks: usize,
    pub review_tasks: usize,
    pub usage: UsageSummary,
    pub usage_by_source: BTreeMap<String, UsageSummary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSummary {
    pub provider: String,
    pub status: String,
    pub usage_sources: Vec<String>,
    pub active_agents: usize,
    pub used_percent: Option<f64>,
    pub reset_at: Option<String>,
    pub usage: UsageSummary,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityView {
    pub id: String,
    pub timestamp: String,
    pub event_type: String,
    pub source: String,
    pub target: Option<String>,
    pub task_id: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GodView {
    pub id: String,
    pub label: String,
    pub authority: String,
    pub pending_decisions: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelView {
    pub catalog: LocalModelCatalogEntry,
    pub fit: ModelFitAssessment,
    pub installed: Option<bool>,
    pub benchmark: Option<BenchmarkResult>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub god: GodView,
    pub project: ProjectSummary,
    pub agents: Vec<AgentView>,
    pub tasks: Vec<TaskView>,
    pub relationships: Vec<OrganizationRelationship>,
    pub resources: Vec<ResourceSummary>,
    pub activity: Vec<ActivityView>,
    pub hierarchy_warnings: Vec<String>,
    pub governance: GovernanceSnapshot,
    #[serde(default)]
    pub hardware: Option<crate::runtime::hardware::HardwareProfile>,
    #[serde(default)]
    pub local_models: Vec<LocalModelView>,
    #[serde(default)]
    pub intelligence_resources: Vec<ResourceProfile>,
    #[serde(default)]
    pub economic_policy: EconomicPolicy,
    #[serde(default)]
    pub routing_decisions: Vec<RoutingDecision>,
    #[serde(default)]
    pub task_outcomes: Vec<TaskOutcomeEvidence>,
    #[serde(default)]
    pub calibrated_capabilities: Vec<CalibratedCapabilityProfile>,
    #[serde(default)]
    pub routing_calibrations: Vec<RoutingCalibrationRecord>,
    #[serde(default)]
    pub calibration_dashboard: CalibrationDashboard,
    #[serde(default)]
    pub calibration_suggestions: Vec<CalibrationSuggestion>,
    #[serde(default)]
    pub promotion_suggestions: Vec<PromotionSuggestion>,
    #[serde(default)]
    pub capability_learning_policy: CapabilityLearningPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnection {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub status: String,
    pub status_label: String,
    pub account_label: String,
    pub detail: String,
    pub action_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionGuide {
    pub provider_id: String,
    pub title: String,
    pub description: String,
    pub command: Option<String>,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MessageReceipt {
    pub id: String,
    pub timestamp: String,
}

pub fn task_progress(status: &str) -> f64 {
    match status {
        "COMPLETED" => 1.0,
        "REVIEW" => 0.9,
        "RUNNING" => 0.55,
        "WAITING_RESOURCE" => 0.4,
        "BLOCKED" => 0.25,
        _ => 0.0,
    }
}

pub fn weighted_progress(tasks: &[TaskView]) -> f64 {
    let total_weight: f64 = tasks.iter().map(|task| task.weight.max(0.0)).sum();
    if total_weight == 0.0 {
        return 0.0;
    }
    let earned: f64 = tasks
        .iter()
        .map(|task| task.weight.max(0.0) * task.progress.clamp(0.0, 1.0))
        .sum();
    ((earned / total_weight) * 1000.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::execution_provider::{UsageSnapshot, UsageSource};

    fn task(id: &str, weight: f64, progress: f64) -> TaskView {
        TaskView {
            id: id.into(),
            objective: id.into(),
            status: "READY".into(),
            assigned_to: vec![],
            dependencies: vec![],
            weight,
            progress,
        }
    }

    #[test]
    fn progress_uses_task_weights_instead_of_raw_task_count() {
        assert_eq!(
            weighted_progress(&[task("foundation", 8.0, 1.0), task("polish", 2.0, 0.0)]),
            80.0
        );
    }

    #[test]
    fn task_status_maps_to_conservative_progress() {
        assert_eq!(task_progress("COMPLETED"), 1.0);
        assert_eq!(task_progress("RUNNING"), 0.55);
        assert_eq!(task_progress("READY"), 0.0);
    }

    #[test]
    fn usage_aggregation_preserves_unknowns_and_known_cost_only() {
        let mut total = UsageSummary::default();
        total.add(&UsageSnapshot::default());
        assert_eq!(total.total_tokens, None);
        assert_eq!(total.cost, None);
        total.add(&UsageSnapshot {
            source: UsageSource::Subscription,
            input_tokens: Some(7),
            output_tokens: Some(3),
            ..UsageSnapshot::default()
        });
        assert_eq!(total.total_tokens, Some(10));
        assert_eq!(total.cost, None);
        assert_eq!(total.sources, vec!["subscription"]);
    }
}
