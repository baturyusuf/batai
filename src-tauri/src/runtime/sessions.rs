use std::{collections::HashMap, sync::Arc};

use sha2::{Digest, Sha256};

use super::{
    errors::{Result, RuntimeError},
    execution_provider::ExecutionProvider,
    store::RuntimeStore,
    types::{Agent, Session},
};

#[derive(Clone)]
pub struct SessionManager {
    store: RuntimeStore,
    providers: Arc<HashMap<String, Arc<dyn ExecutionProvider>>>,
}

impl SessionManager {
    pub fn new(
        store: RuntimeStore,
        providers: HashMap<String, Arc<dyn ExecutionProvider>>,
    ) -> Self {
        Self {
            store,
            providers: Arc::new(providers),
        }
    }
    pub fn provider_for(&self, agent: &Agent) -> Result<Arc<dyn ExecutionProvider>> {
        self.providers
            .get(&agent.provider)
            .cloned()
            .ok_or_else(|| RuntimeError::ProviderNotRegistered(agent.provider.clone()))
    }
    pub async fn ensure(&self, agent: &Agent) -> Result<Session> {
        let provider = self.provider_for(agent)?;
        let fingerprint = fingerprint(agent)?;
        if let Some(existing) = self.store.get_session(&agent.id)? {
            if existing.provider == agent.provider && existing.fingerprint == fingerprint {
                provider
                    .restore_session(&existing.provider_session_id, &existing.metadata, agent)
                    .await?;
                return Ok(existing);
            }
            if let Some(old) = self.providers.get(&existing.provider) {
                let _ = old.close_session(&existing.provider_session_id).await;
            }
            self.store.delete_session(&agent.id)?;
        }
        let created = provider.create_session(agent).await?;
        let session = Session {
            agent_id: agent.id.clone(),
            provider: agent.provider.clone(),
            provider_session_id: created.id,
            fingerprint,
            metadata: created.metadata,
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        self.store.upsert_session(&session)?;
        Ok(session)
    }

    pub fn update_after_turn(
        &self,
        agent_id: &str,
        expected_session_id: &str,
        returned_session_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<()> {
        if let Some(mut session) = self.store.get_session(agent_id)? {
            if session.provider_session_id == expected_session_id {
                session.provider_session_id = returned_session_id.to_owned();
                session.metadata = metadata.clone();
                session.updated_at = chrono::Utc::now().to_rfc3339();
                self.store.upsert_session(&session)?;
            }
        }
        Ok(())
    }
}

pub fn fingerprint(agent: &Agent) -> Result<String> {
    let relevant = serde_json::json!({"provider":agent.provider,"model":agent.model,
        "reasoning_effort":agent.reasoning_effort,"worktree":agent.worktree,"config":agent.extra});
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&relevant)?)
    ))
}
