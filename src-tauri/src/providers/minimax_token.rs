use super::openai_compatible::{OpenAiCompatibleProvider, ProviderDefinition};
use crate::runtime::{
    credentials::CredentialStore,
    economic::{
        BillingMode, QuotaState, ResourceProfile, ResourceTier, ResourceUsage, TermsProfile,
        TermsState,
    },
    execution_provider::UsageSource,
    organization::CapabilityProfile,
};
use std::sync::Arc;
pub const MODELS: &[&str] = &[
    "MiniMax-M2.7",
    "MiniMax-M2.7-highspeed",
    "MiniMax-M2.5",
    "MiniMax-M2.1",
];
pub fn provider(credentials: Arc<dyn CredentialStore>) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(
        ProviderDefinition {
            name: "minimax-token",
            resource_id: "minimax-token-plan",
            base_url: "https://api.minimax.io/v1",
            allowed_models: MODELS,
            usage_source: UsageSource::Subscription,
        },
        credentials,
    )
}
pub fn resource_profile() -> ResourceProfile {
    ResourceProfile {
        id: "minimax-token-plan".into(),
        provider: "minimax-token".into(),
        display_name: "MiniMax Token Plan".into(),
        tier: ResourceTier::SubscriptionQuota,
        billing_mode: BillingMode::SubscriptionQuota,
        plan_name: None,
        fixed_monthly_cost: None,
        fixed_cost_currency: None,
        marginal_cost_per_million_tokens: None,
        supported_models: MODELS.iter().map(|model| (*model).into()).collect(),
        capabilities: CapabilityProfile::default(),
        capability_evidence: vec![],
        context_window: None,
        concurrency_limit: None,
        current_concurrency: 0,
        quota: QuotaState::default(),
        usage: ResourceUsage::default(),
        status: "AUTH_REQUIRED".into(),
        terms: TermsProfile {
            third_party_allowed: TermsState::Allowed,
            automation_allowed: TermsState::Allowed,
            allowed_use_mode: TermsState::Allowed,
            subscription_quota: true,
            checked_at: Some("2026-09-11".into()),
            source_url: Some(
                "https://platform.minimax.io/docs/api-reference/text-chat-openai".into(),
            ),
            ..TermsProfile::default()
        },
        latency_ms: None,
        reliability: None,
    }
}
