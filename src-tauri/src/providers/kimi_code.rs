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
    "k3",
    "k3-256k",
    "kimi-for-coding",
    "kimi-for-coding-highspeed",
];
pub fn provider(credentials: Arc<dyn CredentialStore>) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(
        ProviderDefinition {
            name: "kimi-code",
            resource_id: "kimi-personal-membership",
            base_url: "https://api.kimi.com/coding/v1",
            allowed_models: MODELS,
            usage_source: UsageSource::Subscription,
        },
        credentials,
    )
}
pub fn resource_profile() -> ResourceProfile {
    profile(
        "kimi-personal-membership",
        "kimi-code",
        "Kimi Code Membership",
        MODELS,
        "https://www.kimi.com/code/docs/en/",
    )
}
fn profile(id: &str, provider: &str, name: &str, models: &[&str], source: &str) -> ResourceProfile {
    ResourceProfile {
        id: id.into(),
        provider: provider.into(),
        display_name: name.into(),
        tier: ResourceTier::SubscriptionQuota,
        billing_mode: BillingMode::SubscriptionQuota,
        plan_name: None,
        fixed_monthly_cost: None,
        fixed_cost_currency: None,
        marginal_cost_per_million_tokens: None,
        supported_models: models.iter().map(|model| (*model).into()).collect(),
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
            user_agent_integrity_required: true,
            checked_at: Some("2026-09-11".into()),
            source_url: Some(source.into()),
            ..TermsProfile::default()
        },
        latency_ms: None,
        reliability: None,
    }
}
