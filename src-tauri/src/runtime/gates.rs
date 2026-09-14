//! Durable execution gates shared by background intelligence operations.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionGateType {
    GodDecision,
    ResourceWait,
    ProviderApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionGateStatus {
    Waiting,
    Approved,
    Rejected,
    Superseded,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionGate {
    pub id: String,
    pub gate_type: ExecutionGateType,
    pub status: ExecutionGateStatus,
    pub operation_id: String,
    pub safe_continuation_phase: String,
    pub decision_id: Option<String>,
    pub resource_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub expected_policy_revision: u64,
    pub maximum_tokens: Option<u64>,
    pub context_fingerprint: Option<String>,
    pub routing_decision_id: Option<String>,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub resolution_reason: Option<String>,
}

impl ExecutionGate {
    pub fn matches_payg(&self, resource_id: &str, provider: &str, model: &str) -> bool {
        self.gate_type == ExecutionGateType::GodDecision
            && self.status == ExecutionGateStatus::Approved
            && self.resource_id.as_deref() == Some(resource_id)
            && self.provider.as_deref() == Some(provider)
            && self.model.as_deref() == Some(model)
    }
}
