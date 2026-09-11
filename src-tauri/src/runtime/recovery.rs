use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
    errors::{Result, RuntimeError},
    events::EventEngine,
    store::{GovernanceAuditRow, RuntimeStore},
    types::{Agent, EventType},
};

const MAX_PREIMAGE_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OperationPhase {
    Prepared,
    ApplyingFiles,
    FilesApplied,
    ApplyingDb,
    DbApplied,
    Committed,
    RollbackRequired,
    RollingBack,
    RolledBack,
    RecoveryRequired,
    NeedsReview,
}

impl OperationPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Prepared => "PREPARED",
            Self::ApplyingFiles => "APPLYING_FILES",
            Self::FilesApplied => "FILES_APPLIED",
            Self::ApplyingDb => "APPLYING_DB",
            Self::DbApplied => "DB_APPLIED",
            Self::Committed => "COMMITTED",
            Self::RollbackRequired => "ROLLBACK_REQUIRED",
            Self::RollingBack => "ROLLING_BACK",
            Self::RolledBack => "ROLLED_BACK",
            Self::RecoveryRequired => "RECOVERY_REQUIRED",
            Self::NeedsReview => "NEEDS_REVIEW",
        }
    }

    pub fn terminal(self) -> bool {
        matches!(self, Self::Committed | Self::RolledBack | Self::NeedsReview)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecoveryDisposition {
    ForwardComplete,
    Rollback,
    HumanReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecoveryAction {
    AcceptCurrentState,
    Rollback,
    RetryComplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalFile {
    pub path: String,
    pub temporary_path: String,
    pub backup_path: String,
    pub before_exists: bool,
    pub before_fingerprint: Option<String>,
    pub intended_after_fingerprint: String,
    pub before_content: Option<String>,
    pub intended_after_content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalDbEntity {
    pub entity_type: String,
    pub entity_id: String,
    pub before: Option<Value>,
    pub intended_after: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationJournal {
    pub operation_id: String,
    pub operation_type: String,
    pub actor: String,
    pub target: Option<String>,
    pub organization_revision_before: u64,
    pub expected_revision_after: u64,
    pub created_at: String,
    pub updated_at: String,
    pub current_phase: OperationPhase,
    pub affected_files: Vec<JournalFile>,
    pub affected_db_entities: Vec<JournalDbEntity>,
    pub correlation_decision_id: Option<String>,
    pub task_id: Option<String>,
    pub failure_details: Option<String>,
    pub recovery_disposition: RecoveryDisposition,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverySummary {
    pub recovered: usize,
    pub rolled_back: usize,
    pub needs_review: usize,
}

#[derive(Clone)]
pub struct RecoveryEngine {
    root: PathBuf,
    store: RuntimeStore,
    events: EventEngine,
}

impl RecoveryEngine {
    pub fn new(root: PathBuf, store: RuntimeStore, events: EventEngine) -> Self {
        Self {
            root,
            store,
            events,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &self,
        operation_type: impl Into<String>,
        actor: impl Into<String>,
        target: Option<String>,
        revision_before: u64,
        revision_after: u64,
        files: Vec<(PathBuf, Vec<u8>)>,
        db_entities: Vec<JournalDbEntity>,
        decision_id: Option<String>,
        task_id: Option<String>,
    ) -> Result<OperationJournal> {
        let operation_id = format!("OP-{}", uuid::Uuid::new_v4());
        let mut affected_files = Vec::with_capacity(files.len());
        for (path, intended) in files {
            let relative = self.relative_path(&path)?;
            let parent = path.parent().ok_or_else(|| {
                RuntimeError::Governance(format!("path has no parent: {}", path.display()))
            })?;
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("record");
            let temporary_path =
                self.relative_path(&parent.join(format!(".{file_name}.{operation_id}.tmp")))?;
            let backup_path = self.relative_path(
                &parent.join(format!(".{file_name}.{operation_id}.recovery-backup")),
            )?;
            let before = fs::read(&path).ok();
            if before
                .as_ref()
                .is_some_and(|bytes| bytes.len() > MAX_PREIMAGE_BYTES)
                || intended.len() > MAX_PREIMAGE_BYTES
            {
                return Err(RuntimeError::Governance(format!(
                    "journal file exceeds bounded preimage size: {}",
                    path.display()
                )));
            }
            let before_content = before
                .as_ref()
                .map(|bytes| String::from_utf8(bytes.clone()))
                .transpose()
                .map_err(|_| {
                    RuntimeError::Governance("journal only accepts UTF-8 config files".into())
                })?;
            let intended_after_content = String::from_utf8(intended.clone()).map_err(|_| {
                RuntimeError::Governance("journal only accepts UTF-8 config files".into())
            })?;
            affected_files.push(JournalFile {
                path: relative,
                temporary_path,
                backup_path,
                before_exists: before.is_some(),
                before_fingerprint: before.as_deref().map(fingerprint),
                intended_after_fingerprint: fingerprint(&intended),
                before_content,
                intended_after_content,
            });
        }
        let timestamp = now();
        let journal = OperationJournal {
            operation_id,
            operation_type: operation_type.into(),
            actor: actor.into(),
            target,
            organization_revision_before: revision_before,
            expected_revision_after: revision_after,
            created_at: timestamp.clone(),
            updated_at: timestamp,
            current_phase: OperationPhase::Prepared,
            affected_files,
            affected_db_entities: db_entities,
            correlation_decision_id: decision_id,
            task_id,
            failure_details: None,
            recovery_disposition: RecoveryDisposition::ForwardComplete,
        };
        self.store.upsert_operation_journal(&journal)?;
        self.events.publish(
            EventType::OperationPrepared,
            journal.actor.clone(),
            Some(journal.operation_id.clone()),
            journal.task_id.clone(),
            serde_json::to_value(&journal)?,
        )?;
        Ok(journal)
    }

    pub fn apply_files(&self, journal: &mut OperationJournal) -> Result<()> {
        self.set_phase(journal, OperationPhase::ApplyingFiles, None)?;
        for file in &journal.affected_files {
            recoverable_replace(
                &self.absolute_path(&file.path)?,
                file.intended_after_content.as_bytes(),
                &self.absolute_path(&file.temporary_path)?,
                &self.absolute_path(&file.backup_path)?,
            )?;
        }
        self.set_phase(journal, OperationPhase::FilesApplied, None)
    }

    pub fn apply_db(&self, journal: &mut OperationJournal) -> Result<()> {
        self.set_phase(journal, OperationPhase::ApplyingDb, None)?;
        journal.current_phase = OperationPhase::DbApplied;
        journal.updated_at = now();
        self.store.apply_journal_db_entities(journal)
    }

    pub fn commit(&self, journal: &mut OperationJournal) -> Result<()> {
        self.cleanup_artifacts(journal)?;
        self.set_phase(journal, OperationPhase::Committed, None)?;
        self.events.publish(
            EventType::OperationCommitted,
            journal.actor.clone(),
            Some(journal.operation_id.clone()),
            journal.task_id.clone(),
            serde_json::json!({"operationType":journal.operation_type}),
        )?;
        Ok(())
    }

    pub fn fail_and_rollback(&self, journal: &mut OperationJournal, error: &str) -> Result<()> {
        self.set_phase(
            journal,
            OperationPhase::RollbackRequired,
            Some(error.to_owned()),
        )?;
        self.rollback(journal)
    }

    pub fn reconcile_startup(&self) -> Result<RecoverySummary> {
        let mut summary = RecoverySummary::default();
        let journals: Vec<OperationJournal> = self.store.list_unfinished_operation_journals()?;
        for mut journal in journals {
            match self.reconcile_one(&mut journal)? {
                OperationPhase::Committed => summary.recovered += 1,
                OperationPhase::RolledBack => summary.rolled_back += 1,
                OperationPhase::NeedsReview => summary.needs_review += 1,
                _ => {}
            }
        }
        Ok(summary)
    }

    pub fn path_in_flight(&self, path: &Path) -> Result<bool> {
        let relative = self.relative_path(path)?;
        Ok(self
            .store
            .list_blocking_operation_journals()?
            .iter()
            .any(|journal| {
                journal
                    .affected_files
                    .iter()
                    .any(|file| file.path == relative)
            }))
    }

    pub fn requires_review(&self) -> Result<bool> {
        Ok(self.store.count_operation_phase("NEEDS_REVIEW")? > 0)
    }

    pub fn has_blocking_operations(&self) -> Result<bool> {
        Ok(self.store.count_blocking_operations()? > 0)
    }

    pub fn resolve_review(
        &self,
        operation_id: &str,
        action: RecoveryAction,
    ) -> Result<OperationJournal> {
        let mut journal = self
            .store
            .list_operation_journals(1000)?
            .into_iter()
            .find(|journal| journal.operation_id == operation_id)
            .ok_or_else(|| {
                RuntimeError::Governance(format!("operation not found: {operation_id}"))
            })?;
        if journal.current_phase != OperationPhase::NeedsReview {
            return Ok(journal);
        }
        match action {
            RecoveryAction::AcceptCurrentState => {
                if !self.files_match_after(&journal)? || !self.db_matches_after(&journal)? {
                    return Err(RuntimeError::Governance(
                        "current state does not match the journal's intended state".into(),
                    ));
                }
                self.commit(&mut journal)?;
            }
            RecoveryAction::Rollback => self.rollback(&mut journal)?,
            RecoveryAction::RetryComplete => {
                self.set_phase(&mut journal, OperationPhase::RecoveryRequired, None)?;
                self.reconcile_one(&mut journal)?;
            }
        }
        Ok(journal)
    }

    fn reconcile_one(&self, journal: &mut OperationJournal) -> Result<OperationPhase> {
        let files_after = self.files_match_after(journal)?;
        let files_before = self.files_match_before(journal)?;
        match journal.current_phase {
            OperationPhase::Prepared if files_before => self.rollback(journal)?,
            OperationPhase::Prepared
            | OperationPhase::ApplyingFiles
            | OperationPhase::FilesApplied
            | OperationPhase::ApplyingDb
            | OperationPhase::RecoveryRequired
                if files_after =>
            {
                self.apply_db(journal)?;
                self.commit(journal)?;
                self.events.publish(
                    EventType::OperationRecovered,
                    "recovery",
                    Some(journal.operation_id.clone()),
                    journal.task_id.clone(),
                    serde_json::json!({"disposition":"FORWARD_COMPLETE"}),
                )?;
                self.audit_recovery(journal, "RECOVERED")?;
            }
            OperationPhase::RecoveryRequired if files_before => {
                self.apply_files(journal)?;
                self.apply_db(journal)?;
                self.commit(journal)?;
                self.events.publish(
                    EventType::OperationRecovered,
                    "recovery",
                    Some(journal.operation_id.clone()),
                    journal.task_id.clone(),
                    serde_json::json!({"disposition":"RETRY_FORWARD_COMPLETE"}),
                )?;
                self.audit_recovery(journal, "RECOVERED")?;
            }
            OperationPhase::ApplyingFiles
            | OperationPhase::FilesApplied
            | OperationPhase::ApplyingDb
            | OperationPhase::RollbackRequired
            | OperationPhase::RollingBack
                if files_before =>
            {
                self.rollback(journal)?
            }
            OperationPhase::DbApplied if files_after && self.db_matches_after(journal)? => {
                self.commit(journal)?;
                self.events.publish(
                    EventType::OperationRecovered,
                    "recovery",
                    Some(journal.operation_id.clone()),
                    journal.task_id.clone(),
                    serde_json::json!({"disposition":"COMMIT_MARK_RECOVERED"}),
                )?;
                self.audit_recovery(journal, "RECOVERED")?;
            }
            OperationPhase::Committed
            | OperationPhase::RolledBack
            | OperationPhase::NeedsReview => {}
            _ => self.needs_review(
                journal,
                "fingerprint or revision state differs from the journal",
            )?,
        }
        Ok(journal.current_phase)
    }

    fn rollback(&self, journal: &mut OperationJournal) -> Result<()> {
        if !self.files_match_after(journal)? && !self.files_match_before(journal)? {
            return self.needs_review(journal, "external file change prevents safe rollback");
        }
        self.set_phase(journal, OperationPhase::RollingBack, None)?;
        for file in &journal.affected_files {
            let path = self.absolute_path(&file.path)?;
            match (&file.before_content, file.before_exists) {
                (Some(content), true) => atomic_write(&path, content.as_bytes())?,
                (_, false) if path.exists() => fs::remove_file(path)?,
                _ => {}
            }
        }
        self.cleanup_artifacts(journal)?;
        self.store.rollback_journal_db_entities(journal)?;
        self.mark_rolled_back(journal)
    }

    fn mark_rolled_back(&self, journal: &mut OperationJournal) -> Result<()> {
        journal.recovery_disposition = RecoveryDisposition::Rollback;
        self.set_phase(journal, OperationPhase::RolledBack, None)?;
        self.events.publish(
            EventType::OperationRolledBack,
            "recovery",
            Some(journal.operation_id.clone()),
            journal.task_id.clone(),
            serde_json::json!({"operationType":journal.operation_type}),
        )?;
        self.audit_recovery(journal, "ROLLED_BACK")?;
        Ok(())
    }

    fn needs_review(&self, journal: &mut OperationJournal, reason: &str) -> Result<()> {
        journal.recovery_disposition = RecoveryDisposition::HumanReview;
        self.set_phase(
            journal,
            OperationPhase::NeedsReview,
            Some(reason.to_owned()),
        )?;
        self.events.publish(
            EventType::OperationRecoveryRequired,
            "recovery",
            Some(journal.operation_id.clone()),
            journal.task_id.clone(),
            serde_json::json!({"reason":reason,"operationType":journal.operation_type}),
        )?;
        self.audit_recovery(journal, "RECOVERY_REQUIRED")?;
        Ok(())
    }

    fn audit_recovery(&self, journal: &OperationJournal, outcome: &str) -> Result<()> {
        let timestamp = now();
        let id = format!("AUD-{}-{outcome}", journal.operation_id);
        let record = serde_json::json!({
            "id":id,
            "timestamp":timestamp,
            "actor":"system-recovery",
            "authority":"GOD",
            "action":journal.operation_type,
            "target":journal.target,
            "outcome":outcome,
            "reason":journal.failure_details,
            "decisionId":journal.correlation_decision_id,
            "taskId":journal.task_id,
            "revision":journal.expected_revision_after,
        });
        self.store.append_governance_audit(&GovernanceAuditRow {
            id: &id,
            timestamp: &timestamp,
            actor: "system-recovery",
            action: &journal.operation_type,
            target: journal.target.as_deref(),
            outcome,
            record_json: serde_json::to_string(&record)?,
        })
    }

    fn set_phase(
        &self,
        journal: &mut OperationJournal,
        phase: OperationPhase,
        failure: Option<String>,
    ) -> Result<()> {
        journal.current_phase = phase;
        journal.updated_at = now();
        if failure.is_some() {
            journal.failure_details = failure;
        }
        self.store.upsert_operation_journal(journal)
    }

    fn files_match_after(&self, journal: &OperationJournal) -> Result<bool> {
        journal
            .affected_files
            .iter()
            .map(|file| {
                let bytes = fs::read(self.absolute_path(&file.path)?).ok();
                Ok(bytes.as_deref().map(fingerprint).as_deref()
                    == Some(file.intended_after_fingerprint.as_str()))
            })
            .try_fold(true, |all, item: Result<bool>| Ok(all && item?))
    }

    fn cleanup_artifacts(&self, journal: &OperationJournal) -> Result<()> {
        for file in &journal.affected_files {
            for relative in [&file.temporary_path, &file.backup_path] {
                let path = self.absolute_path(relative)?;
                if path.exists() {
                    fs::remove_file(path)?;
                }
            }
        }
        Ok(())
    }

    fn files_match_before(&self, journal: &OperationJournal) -> Result<bool> {
        journal
            .affected_files
            .iter()
            .map(|file| {
                let path = self.absolute_path(&file.path)?;
                let backup = self.absolute_path(&file.backup_path)?;
                let bytes = fs::read(path).ok().or_else(|| fs::read(backup).ok());
                Ok(match (&file.before_fingerprint, bytes) {
                    (Some(expected), Some(bytes)) => fingerprint(&bytes) == *expected,
                    (None, None) if !file.before_exists => true,
                    _ => false,
                })
            })
            .try_fold(true, |all, item: Result<bool>| Ok(all && item?))
    }

    fn db_matches_after(&self, journal: &OperationJournal) -> Result<bool> {
        for entity in &journal.affected_db_entities {
            if entity.entity_type == "AGENT" {
                let actual = self
                    .store
                    .get_agent(&entity.entity_id)?
                    .map(serde_json::to_value)
                    .transpose()?;
                if actual != entity.intended_after {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn relative_path(&self, path: &Path) -> Result<String> {
        let relative = path.strip_prefix(&self.root).map_err(|_| {
            RuntimeError::Governance(format!(
                "journal path is outside project: {}",
                path.display()
            ))
        })?;
        Ok(relative.to_string_lossy().replace('\\', "/"))
    }

    fn absolute_path(&self, relative: &str) -> Result<PathBuf> {
        if relative.contains("..") || Path::new(relative).is_absolute() {
            return Err(RuntimeError::Governance("unsafe journal path".into()));
        }
        Ok(self.root.join(relative))
    }
}

pub fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn agent_db_entity(before: Option<Agent>, after: Option<Agent>) -> Result<JournalDbEntity> {
    let id = after
        .as_ref()
        .or(before.as_ref())
        .ok_or_else(|| RuntimeError::Governance("agent journal entity has no identity".into()))?
        .id
        .clone();
    Ok(JournalDbEntity {
        entity_type: "AGENT".into(),
        entity_id: id,
        before: before.map(serde_json::to_value).transpose()?,
        intended_after: after.map(serde_json::to_value).transpose()?,
    })
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        RuntimeError::Governance(format!("path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("record"),
        uuid::Uuid::new_v4()
    ));
    let backup = parent.join(format!(
        ".{}.{}.recovery-backup",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("record"),
        uuid::Uuid::new_v4()
    ));
    recoverable_replace(path, bytes, &temporary, &backup)
}

fn recoverable_replace(path: &Path, bytes: &[u8], temporary: &Path, backup: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if temporary.exists() {
        fs::remove_file(temporary)?;
    }
    if backup.exists() {
        fs::remove_file(backup)?;
    }
    {
        let mut file = fs::File::create(temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if path.exists() {
        fs::rename(path, backup)?;
    }
    if let Err(error) = fs::rename(temporary, path) {
        if backup.exists() {
            let _ = fs::rename(backup, path);
        }
        let _ = fs::remove_file(temporary);
        return Err(error.into());
    }
    if backup.exists() {
        fs::remove_file(backup)?;
    }
    Ok(())
}

fn fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_agent(id: &str) -> Agent {
        serde_json::from_value(serde_json::json!({
            "id":id,"name":"Recovery Agent","provider":"mock","model":"mock"
        }))
        .expect("agent")
    }

    fn open_engine(root: &TempDir) -> (RecoveryEngine, RuntimeStore) {
        let store = RuntimeStore::open(root.path().join(".runtime/runtime.sqlite")).expect("store");
        let events = EventEngine::new(store.clone());
        (
            RecoveryEngine::new(root.path().to_path_buf(), store.clone(), events),
            store,
        )
    }

    fn prepared_agent_operation(
        engine: &RecoveryEngine,
        root: &TempDir,
        agent: &Agent,
    ) -> OperationJournal {
        engine
            .prepare(
                "CREATE_AGENT",
                "god",
                Some(agent.id.clone()),
                0,
                1,
                vec![(
                    root.path().join(".batai/agents/a/config.json"),
                    json_bytes(agent).expect("json"),
                )],
                vec![agent_db_entity(None, Some(agent.clone())).expect("entity")],
                None,
                None,
            )
            .expect("prepare")
    }

    #[test]
    fn crash_before_files_safely_aborts_prepared_operation() {
        let root = TempDir::new().expect("temp");
        let (engine, store) = open_engine(&root);
        prepared_agent_operation(&engine, &root, &test_agent("a"));
        let summary = engine.reconcile_startup().expect("reconcile");
        assert_eq!(summary.rolled_back, 1, "{summary:?}");
        assert!(store.get_agent("a").expect("agent").is_none());
        assert!(!root.path().join(".batai/agents/a/config.json").exists());
    }

    #[test]
    fn crash_after_files_forward_completes_sqlite_and_commit() {
        let root = TempDir::new().expect("temp");
        let (engine, _) = open_engine(&root);
        let mut journal = prepared_agent_operation(&engine, &root, &test_agent("a"));
        engine.apply_files(&mut journal).expect("files");
        drop(engine);
        let (reopened, store) = open_engine(&root);
        let summary = reopened.reconcile_startup().expect("reconcile");
        assert_eq!(summary.recovered, 1);
        assert!(store.get_agent("a").expect("agent").is_some());
        assert_eq!(
            store.list_operation_journals(1).expect("journal")[0].current_phase,
            OperationPhase::Committed
        );
    }

    #[test]
    fn crash_after_sqlite_recovers_only_missing_commit_mark() {
        let root = TempDir::new().expect("temp");
        let (engine, _) = open_engine(&root);
        let mut journal = prepared_agent_operation(&engine, &root, &test_agent("a"));
        engine.apply_files(&mut journal).expect("files");
        engine.apply_db(&mut journal).expect("db");
        drop(engine);
        let (reopened, store) = open_engine(&root);
        let summary = reopened.reconcile_startup().expect("reconcile");
        assert_eq!(summary.recovered, 1);
        assert!(store.get_agent("a").expect("agent").is_some());
    }

    #[test]
    fn external_change_after_crash_requires_review_without_overwrite() {
        let root = TempDir::new().expect("temp");
        let (engine, store) = open_engine(&root);
        let mut journal = prepared_agent_operation(&engine, &root, &test_agent("a"));
        engine.apply_files(&mut journal).expect("files");
        let path = root.path().join(".batai/agents/a/config.json");
        fs::write(&path, b"external change").expect("external write");
        let summary = engine.reconcile_startup().expect("reconcile");
        assert_eq!(summary.needs_review, 1);
        assert_eq!(fs::read(&path).expect("read"), b"external change");
        assert!(store.get_agent("a").expect("agent").is_none());
        assert!(engine.path_in_flight(&path).expect("in-flight path"));

        fs::remove_file(&path).expect("restore absent preimage");
        let resolved = engine
            .resolve_review(&journal.operation_id, RecoveryAction::RetryComplete)
            .expect("retry forward completion");
        assert_eq!(resolved.current_phase, OperationPhase::Committed);
        assert!(store.get_agent("a").expect("agent").is_some());
        assert!(!engine.path_in_flight(&path).expect("released path"));
    }

    #[test]
    fn failure_before_sqlite_rolls_back_file_and_database_preimage() {
        let root = TempDir::new().expect("temp");
        let (engine, store) = open_engine(&root);
        let mut journal = prepared_agent_operation(&engine, &root, &test_agent("a"));
        engine.apply_files(&mut journal).expect("files");
        engine
            .fail_and_rollback(&mut journal, "injected before sqlite")
            .expect("rollback");
        assert!(!root.path().join(".batai/agents/a/config.json").exists());
        assert!(store.get_agent("a").expect("agent").is_none());
        assert_eq!(journal.current_phase, OperationPhase::RolledBack);
    }

    #[test]
    fn atomic_replace_cleans_temporary_and_backup_files() {
        let root = TempDir::new().expect("temp");
        let path = root.path().join("organization.json");
        atomic_write(&path, b"one").expect("first");
        atomic_write(&path, b"two").expect("replace");
        assert_eq!(fs::read(&path).expect("read"), b"two");
        let leftovers = fs::read_dir(root.path())
            .expect("directory")
            .filter_map(std::result::Result::ok)
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.contains(".tmp") || name.contains("recovery-backup")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn restart_recovers_interrupted_windows_style_replace() {
        let root = TempDir::new().expect("temp");
        let (engine, store) = open_engine(&root);
        let path = root.path().join(".batai/organization.json");
        fs::create_dir_all(path.parent().unwrap()).expect("directory");
        fs::write(&path, b"before").expect("preimage");
        let mut journal = engine
            .prepare(
                "UPDATE_ORGANIZATION",
                "god",
                None,
                1,
                2,
                vec![(path.clone(), b"after".to_vec())],
                vec![],
                None,
                None,
            )
            .expect("prepare");
        engine
            .set_phase(&mut journal, OperationPhase::ApplyingFiles, None)
            .expect("applying");
        let file = &journal.affected_files[0];
        let temporary = engine
            .absolute_path(&file.temporary_path)
            .expect("temp path");
        let backup = engine
            .absolute_path(&file.backup_path)
            .expect("backup path");
        fs::write(&temporary, b"after").expect("temporary content");
        fs::rename(&path, &backup).expect("simulated interrupted replace");
        assert!(
            engine.files_match_before(&journal).expect("preimage match"),
            "{journal:?} backup={} bytes={:?}",
            backup.display(),
            fs::read(&backup)
        );

        let summary = engine.reconcile_startup().expect("reconcile");
        assert_eq!(summary.rolled_back, 1, "{summary:?}");
        assert_eq!(fs::read(&path).expect("restored"), b"before");
        assert!(!temporary.exists());
        assert!(!backup.exists());
        assert_eq!(
            store.list_operation_journals(1).expect("journal")[0].current_phase,
            OperationPhase::RolledBack
        );
    }
}
