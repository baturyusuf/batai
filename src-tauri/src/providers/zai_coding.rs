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
pub const MODELS: &[&str] = &["glm-5.1", "glm-5", "glm-5-turbo", "glm-4.7", "glm-4.5-air"];
pub fn provider(credentials: Arc<dyn CredentialStore>) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(
        ProviderDefinition {
            name: "zai-coding",
            resource_id: "zai-coding-plan",
            base_url: "https://api.z.ai/api/coding/paas/v4",
            allowed_models: MODELS,
            usage_source: UsageSource::Subscription,
        },
        credentials,
    )
}
pub fn resource_profile() -> ResourceProfile {
    ResourceProfile {
        id: "zai-coding-plan".into(),
        provider: "zai-coding".into(),
        display_name: "Z.AI Coding Plan".into(),
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
            third_party_allowed: TermsState::RequiresReview,
            automation_allowed: TermsState::RequiresReview,
            allowed_use_mode: TermsState::RequiresReview,
            subscription_quota: true,
            checked_at: Some("2026-09-11".into()),
            source_url: Some("https://docs.z.ai/api-reference/introduction".into()),
            ..TermsProfile::default()
        },
        latency_ms: None,
        reliability: None,
    }
}
