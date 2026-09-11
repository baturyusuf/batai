//! Secret storage boundary. Secrets are never persisted in project files or SQLite.

use std::collections::BTreeMap;

use super::errors::{Result, RuntimeError};

pub trait CredentialStore: Send + Sync {
    fn set(&self, resource_id: &str, secret: &str) -> Result<()>;
    fn get(&self, resource_id: &str) -> Result<Option<String>>;
    fn delete(&self, resource_id: &str) -> Result<()>;
}

#[derive(Debug, Default, Clone)]
pub struct OsCredentialStore;

impl OsCredentialStore {
    fn entry(resource_id: &str) -> Result<keyring::Entry> {
        keyring::Entry::new("dev.batai.intelligence", resource_id)
            .map_err(|error| RuntimeError::Provider(format!("secure credential store: {error}")))
    }
}

impl CredentialStore for OsCredentialStore {
    fn set(&self, resource_id: &str, secret: &str) -> Result<()> {
        if secret.trim().is_empty() {
            return Err(RuntimeError::Provider("credential cannot be empty".into()));
        }
        Self::entry(resource_id)?
            .set_password(secret)
            .map_err(|error| RuntimeError::Provider(format!("secure credential store: {error}")))
    }

    fn get(&self, resource_id: &str) -> Result<Option<String>> {
        match Self::entry(resource_id)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(RuntimeError::Provider(format!(
                "secure credential store: {error}"
            ))),
        }
    }

    fn delete(&self, resource_id: &str) -> Result<()> {
        match Self::entry(resource_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(RuntimeError::Provider(format!(
                "secure credential store: {error}"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnvironmentCredentialStore {
    variables: BTreeMap<String, String>,
}

impl Default for EnvironmentCredentialStore {
    fn default() -> Self {
        Self {
            variables: BTreeMap::from([
                (
                    "kimi-personal-membership".into(),
                    "BATAI_KIMI_CODE_API_KEY".into(),
                ),
                ("zai-coding-plan".into(), "BATAI_ZAI_CODING_API_KEY".into()),
                (
                    "minimax-token-plan".into(),
                    "BATAI_MINIMAX_TOKEN_PLAN_API_KEY".into(),
                ),
            ]),
        }
    }
}

impl CredentialStore for EnvironmentCredentialStore {
    fn set(&self, _resource_id: &str, _secret: &str) -> Result<()> {
        Err(RuntimeError::Provider(
            "environment credentials are read-only".into(),
        ))
    }

    fn get(&self, resource_id: &str) -> Result<Option<String>> {
        Ok(self
            .variables
            .get(resource_id)
            .and_then(|name| std::env::var(name).ok())
            .filter(|value| !value.trim().is_empty()))
    }

    fn delete(&self, _resource_id: &str) -> Result<()> {
        Err(RuntimeError::Provider(
            "environment credentials are read-only".into(),
        ))
    }
}

pub fn redacted(value: &str) -> &'static str {
    if value.is_empty() {
        "missing"
    } else {
        "[REDACTED]"
    }
}

#[derive(Debug, Default, Clone)]
pub struct DefaultCredentialStore {
    os: OsCredentialStore,
    environment: EnvironmentCredentialStore,
}
impl CredentialStore for DefaultCredentialStore {
    fn set(&self, resource_id: &str, secret: &str) -> Result<()> {
        self.os.set(resource_id, secret)
    }
    fn get(&self, resource_id: &str) -> Result<Option<String>> {
        self.os
            .get(resource_id)
            .or_else(|_| self.environment.get(resource_id))
            .and_then(|secret| {
                if secret.is_some() {
                    Ok(secret)
                } else {
                    self.environment.get(resource_id)
                }
            })
    }
    fn delete(&self, resource_id: &str) -> Result<()> {
        self.os.delete(resource_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_never_exposes_secret() {
        assert_eq!(redacted("sk-secret-value"), "[REDACTED]");
        assert!(!redacted("sk-secret-value").contains("secret"));
    }

    #[test]
    fn environment_variable_names_are_resource_specific() {
        let store = EnvironmentCredentialStore::default();
        assert!(store.variables.contains_key("kimi-personal-membership"));
        assert_ne!(
            store.variables["kimi-personal-membership"],
            store.variables["minimax-token-plan"]
        );
    }
}
