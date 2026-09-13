use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use chrono::Utc;
use fs2::FileExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    domain::{AppSnapshot, MessageReceipt, ProviderConnection},
    project::ProjectStore,
    providers,
    runtime::{
        delivery::{DeliveryCheckpoint, DeliveryPolicy},
        errors::{Result, RuntimeError},
        governance::{
            Actor, AuthorityScope, CreateAgentRequest, MutationDisposition, MutationRequest,
            MutationResult, OrganizationMutation, ProtectedOperation,
        },
        organization::{legacy_identity, AgentLifecycle, AuthorityRole, Seniority},
        recovery::RecoveryEngine,
        types::{Task, TaskExecution, TaskStatus, TaskSuccessAction},
        worktrees::WorktreeBinding,
        BataiRuntime,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApplicationMode {
    Desktop,
    Http,
    Mcp,
}

impl ApplicationMode {
    fn label(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Http => "serve",
            Self::Mcp => "mcp",
        }
    }
}

/// Process-scoped exclusive ownership of one project's active runtime.
///
/// The lock file is only metadata; safety comes from the OS file lock, which is
/// released on process exit. A stale-looking file is never deleted blindly.
struct ProjectRuntimeLock {
    file: File,
}

impl ProjectRuntimeLock {
    fn acquire(root: &Path, mode: ApplicationMode) -> Result<Self> {
        let directory = root.join(".runtime");
        fs::create_dir_all(&directory)?;
        let path = directory.join("control-plane.lock");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        if file.try_lock_exclusive().is_err() {
            let mut owner = String::new();
            let _ = file.read_to_string(&mut owner);
            let detail = owner.trim();
            return Err(RuntimeError::Governance(if detail.is_empty() {
                "this project already has an active Batai runtime owner".into()
            } else {
                format!("this project already has an active Batai runtime owner: {detail}")
            }));
        }
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        let metadata = json!({
            "pid": std::process::id(),
            "mode": mode.label(),
            "startedAt": Utc::now().to_rfc3339(),
        });
        writeln!(file, "{}", serde_json::to_string(&metadata)?)?;
        file.sync_all()?;
        Ok(Self { file })
    }
}

impl Drop for ProjectRuntimeLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[derive(Clone)]
pub struct BataiApplication {
    root: PathBuf,
    runtime: Arc<BataiRuntime>,
    started: Arc<AtomicBool>,
    _owner: Arc<ProjectRuntimeLock>,
}

impl BataiApplication {
    pub fn open(root: PathBuf, mode: ApplicationMode) -> Result<Arc<Self>> {
        let root = root.canonicalize().unwrap_or(root);
        let owner = Arc::new(ProjectRuntimeLock::acquire(&root, mode)?);
        let runtime = BataiRuntime::open_with_interactive_approvals(
            root.clone(),
            mode == ApplicationMode::Desktop,
        )?;
        Ok(Arc::new(Self {
            root,
            runtime,
            started: Arc::new(AtomicBool::new(false)),
            _owner: owner,
        }))
    }

    pub async fn start(self: &Arc<Self>) -> Result<()> {
        if self.started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        if let Err(error) = self.runtime.start().await {
            self.started.store(false, Ordering::Release);
            return Err(error);
        }
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        if !self.started.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        self.runtime.shutdown().await
    }

    pub fn runtime(&self) -> &Arc<BataiRuntime> {
        &self.runtime
    }

    pub fn project_root(&self) -> &Path {
        &self.root
    }

    pub fn snapshot(&self) -> Result<AppSnapshot> {
        self.runtime.snapshot()
    }

    /// Secret-filtered external snapshot with additive legacy aliases.
    pub fn external_snapshot(&self) -> Result<Value> {
        let snapshot = self.snapshot()?;
        let mut value = serde_json::to_value(&snapshot)?;
        if let Some(object) = value.as_object_mut() {
            object.insert("events".into(), serde_json::to_value(&snapshot.activity)?);
            object.insert(
                "decisions".into(),
                serde_json::to_value(&snapshot.governance.decisions)?,
            );
            object.insert(
                "director_inbox".into(),
                serde_json::to_value(self.read_director_inbox(false)?)?,
            );
            if let Some(agents) = object.get_mut("agents").and_then(Value::as_array_mut) {
                for agent in agents {
                    if let Some(agent) = agent.as_object_mut() {
                        copy_alias(agent, "title", "role_template");
                        copy_alias(agent, "reportsTo", "parent_agent_id");
                        copy_alias(agent, "reasoningEffort", "reasoning_effort");
                        copy_alias(agent, "authMode", "auth_mode");
                        copy_alias(agent, "currentTaskId", "current_task_id");
                    }
                }
            }
            if let Some(tasks) = object.get_mut("tasks").and_then(Value::as_array_mut) {
                for task in tasks {
                    if let Some(task) = task.as_object_mut() {
                        copy_alias(task, "assignedTo", "assigned_to");
                    }
                }
            }
        }
        Ok(value)
    }

    pub fn providers(&self) -> Vec<ProviderConnection> {
        providers::probe_all()
    }

    pub fn create_agent(&self, input: LegacyCreateAgent) -> Result<MutationResult> {
        input.validate()?;
        let (parsed_level, function) = legacy_identity(&input.role_template);
        let seniority = parsed_level.unwrap_or_else(|| {
            Seniority::ALL
                .into_iter()
                .find(|level| function.supports(*level))
                .unwrap_or(Seniority::Junior)
        });
        let lifecycle = match input.lifetime.as_deref().unwrap_or("project") {
            "task_scoped" | "TASK_SCOPED" => AgentLifecycle::TaskScoped,
            "project" | "PROJECT" => AgentLifecycle::Project,
            "permanent" | "PERMANENT" => AgentLifecycle::Permanent,
            value => {
                return Err(RuntimeError::Governance(format!(
                    "invalid agent lifetime: {value}"
                )))
            }
        };
        let revision = self.runtime.governance.snapshot()?.revision;
        self.runtime.governance.mutate(MutationRequest {
            actor: director_actor(),
            expected_revision: revision,
            task_id: None,
            reason: Some("External Director create-agent request".into()),
            mutation: OrganizationMutation::CreateAgent(CreateAgentRequest {
                id: Some(input.id),
                name: Some(input.name),
                seniority,
                function,
                department: function.department(),
                reports_to: input.parent_agent_id.or_else(|| Some("director".into())),
                lifecycle,
                authority: AuthorityRole::Worker,
                provider: normalize_provider(&input.provider),
                model: input.model,
                reasoning_effort: input.reasoning_effort,
                permissions: Vec::new(),
                intelligence_policy: Default::default(),
            }),
        })
    }

    pub async fn assign_task(&self, input: LegacyAssignTask) -> Result<Task> {
        input.validate()?;
        let parallel = input.assigned_to.len() > 1;
        let task = Task {
            id: input.id,
            created_by: "director".into(),
            objective: input.objective,
            assigned_to: input.assigned_to,
            dependencies: input.dependencies,
            acceptance_criteria: if input.acceptance_criteria.is_empty() {
                vec!["Complete the objective and run relevant tests".into()]
            } else {
                input.acceptance_criteria
            },
            inputs: input.inputs,
            outputs: input.outputs,
            status: TaskStatus::Ready,
            execution: TaskExecution {
                parallel,
                requires_director_review: input.requires_director_review,
            },
            on_success: TaskSuccessAction {
                start: input.start_on_success,
                notify: input.notify_on_success,
                reason: None,
            },
            on_failure: TaskSuccessAction {
                notify: Some("director".into()),
                ..Default::default()
            },
            weight: 1.0,
            extra: Default::default(),
        };
        self.runtime.tasks.ingest(task.clone(), None).await?;
        Ok(self.runtime.store.get_task(&task.id)?.unwrap_or(task))
    }

    pub async fn approve_task(&self, task_id: &str) -> Result<Task> {
        self.runtime.tasks.approve_review(task_id, "director").await
    }

    pub fn create_worktree(
        &self,
        agent_id: &str,
        task_id: &str,
        base_ref: Option<&str>,
    ) -> Result<WorktreeBinding> {
        let checkpoint = self.runtime.delivery.prepare_worktree(
            task_id,
            agent_id,
            base_ref,
            DeliveryPolicy::default(),
        )?;
        Ok(checkpoint.binding())
    }

    pub async fn resume_agent(&self, agent_id: &str) -> Result<Value> {
        self.runtime.tasks.resume_agent(agent_id).await?;
        Ok(json!({"agentId":agent_id,"status":"RESUME_REQUESTED"}))
    }

    pub fn update_directives(&self, agent_id: &str, content: &str) -> Result<Value> {
        validate_component("agent id", agent_id)?;
        let path = self
            .root
            .join(".batai/agents")
            .join(agent_id)
            .join("DIRECTIVES.md");
        let normalized = if content.ends_with('\n') {
            content.to_owned()
        } else {
            format!("{content}\n")
        };
        let revision = self.runtime.governance.snapshot()?.revision;
        let recovery = RecoveryEngine::new(
            self.root.clone(),
            self.runtime.store.clone(),
            self.runtime.events.clone(),
        );
        let mut journal = recovery.prepare(
            "UPDATE_DIRECTIVES",
            "director",
            Some(agent_id.into()),
            revision,
            revision,
            vec![(path.clone(), normalized.into_bytes())],
            vec![],
            None,
            None,
        )?;
        recovery.apply_files(&mut journal)?;
        recovery.apply_db(&mut journal)?;
        recovery.commit(&mut journal)?;
        self.runtime.governance.audit_delivery(
            "director",
            "UPDATE_DIRECTIVES",
            Some(agent_id),
            None,
            MutationDisposition::Applied,
            Some("origin=MCP; recoverable file write".into()),
        )?;
        Ok(json!({"agentId":agent_id,"path":path,"operationId":journal.operation_id}))
    }

    pub fn send_god_message(&self, content: &str) -> Result<MessageReceipt> {
        ProjectStore::new(self.root.clone())
            .send_director_message(content)
            .map_err(RuntimeError::from)
    }

    pub fn read_director_inbox(&self, pending_only: bool) -> Result<Vec<Value>> {
        let directory = self.root.join(".batai/inbox/director");
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut messages = Vec::new();
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let value: Value = serde_json::from_slice(&fs::read(path)?)?;
            if !pending_only || value.get("status").and_then(Value::as_str) == Some("PENDING") {
                messages.push(value);
            }
        }
        messages
            .sort_by(|left, right| left["timestamp"].as_str().cmp(&right["timestamp"].as_str()));
        Ok(messages)
    }

    pub fn acknowledge_god_message(&self, message_id: &str) -> Result<Value> {
        validate_component("message id", message_id)?;
        let directory = self.root.join(".batai/inbox/director");
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let mut value: Value = serde_json::from_slice(&fs::read(&path)?)?;
            if value.get("id").and_then(Value::as_str) != Some(message_id) {
                continue;
            }
            value["status"] = Value::String("ACKNOWLEDGED".into());
            value["acknowledgedAt"] = Value::String(Utc::now().to_rfc3339());
            let revision = self.runtime.governance.snapshot()?.revision;
            let recovery = RecoveryEngine::new(
                self.root.clone(),
                self.runtime.store.clone(),
                self.runtime.events.clone(),
            );
            let bytes = format!("{}\n", serde_json::to_string_pretty(&value)?).into_bytes();
            let mut journal = recovery.prepare(
                "ACKNOWLEDGE_GOD_MESSAGE",
                "director",
                Some(message_id.into()),
                revision,
                revision,
                vec![(path, bytes)],
                vec![],
                None,
                None,
            )?;
            recovery.apply_files(&mut journal)?;
            recovery.apply_db(&mut journal)?;
            recovery.commit(&mut journal)?;
            return Ok(value);
        }
        Err(RuntimeError::Governance(format!(
            "message not found: {message_id}"
        )))
    }

    pub fn request_god_decision(&self, input: DecisionRequest) -> Result<MutationResult> {
        input.validate()?;
        let revision = self.runtime.governance.snapshot()?.revision;
        self.runtime.governance.mutate(MutationRequest {
            actor: director_actor(),
            expected_revision: revision,
            task_id: None,
            reason: input.director_recommendation.as_ref().map(Value::to_string),
            mutation: OrganizationMutation::RequestProtectedAction {
                operation: ProtectedOperation::ModifyAuthority,
                target: format!("director-decision:{}", input.topic),
                detail: json!({
                    "requestedId": input.id,
                    "topic": input.topic,
                    "question": input.question,
                    "recommendation": input.director_recommendation,
                    "alternatives": input.alternatives,
                    "impact": input.impact,
                    "origin": "MCP"
                }),
            },
        })
    }

    pub fn resolve_decision(
        &self,
        decision_id: &str,
        approve: bool,
        note: Option<String>,
    ) -> Result<MutationResult> {
        self.runtime
            .governance
            .resolve_decision(decision_id, approve, note)
    }

    pub fn create_github_issue(&self, input: CreateGithubIssueRequest) -> Result<MutationResult> {
        input.validate()?;
        let revision = self.runtime.governance.snapshot()?.revision;
        self.runtime.governance.mutate(MutationRequest {
            actor: director_actor(),
            expected_revision: revision,
            task_id: None,
            reason: Some("External MCP Director requested a GitHub issue".into()),
            mutation: OrganizationMutation::RequestProtectedAction {
                operation: ProtectedOperation::CreateGithubIssue,
                target: input.title,
                detail: json!({"body":input.body,"labels":input.labels,"origin":"MCP"}),
            },
        })
    }

    pub fn get_task(&self, task_id: &str) -> Result<Option<Task>> {
        self.runtime.store.get_task(task_id)
    }

    pub fn get_delivery(&self, task_id: &str) -> Result<Option<DeliveryCheckpoint>> {
        self.runtime.store.get_delivery_checkpoint(task_id)
    }
}

fn director_actor() -> Actor {
    Actor {
        id: "director".into(),
        scope: AuthorityScope::Project,
    }
}

fn normalize_provider(provider: &str) -> String {
    match provider {
        "codex-cli" => "codex",
        "claude-cli" => "claude",
        other => other,
    }
    .into()
}

fn copy_alias(object: &mut serde_json::Map<String, Value>, source: &str, alias: &str) {
    if let Some(value) = object.get(source).cloned() {
        object.insert(alias.into(), value);
    }
}

fn validate_component(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
    {
        return Err(RuntimeError::Governance(format!("invalid {label}")));
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct LegacyCreateAgent {
    pub id: String,
    pub name: String,
    pub role_template: String,
    #[serde(default)]
    pub parent_agent_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub reasoning_effort: String,
    #[serde(default)]
    pub auth_mode: Option<String>,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub max_turns: Option<u32>,
    #[serde(default)]
    pub lifetime: Option<String>,
}

impl LegacyCreateAgent {
    fn validate(&self) -> Result<()> {
        validate_component("agent id", &self.id)?;
        if self.name.trim().is_empty() || self.role_template.trim().is_empty() {
            return Err(RuntimeError::Governance(
                "agent name and role_template are required".into(),
            ));
        }
        if self.provider.trim().is_empty() || self.model.trim().is_empty() {
            return Err(RuntimeError::Governance(
                "provider and model are required".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct LegacyAssignTask {
    pub id: String,
    pub objective: String,
    pub assigned_to: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub requires_director_review: bool,
    #[serde(default)]
    pub notify_on_success: Option<String>,
    #[serde(default)]
    pub start_on_success: Vec<String>,
}

impl LegacyAssignTask {
    fn validate(&self) -> Result<()> {
        validate_component("task id", &self.id)?;
        if self.objective.trim().is_empty() || self.assigned_to.is_empty() {
            return Err(RuntimeError::Governance(
                "task objective and assigned_to are required".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DecisionRequest {
    #[serde(default)]
    pub id: Option<String>,
    pub topic: String,
    pub question: String,
    #[serde(default)]
    pub director_recommendation: Option<Value>,
    #[serde(default)]
    pub alternatives: Vec<Value>,
    #[serde(default = "default_impact")]
    pub impact: String,
}

impl DecisionRequest {
    fn validate(&self) -> Result<()> {
        if self.topic.trim().is_empty() || self.question.trim().is_empty() {
            return Err(RuntimeError::Governance(
                "decision topic and question are required".into(),
            ));
        }
        if !matches!(self.impact.as_str(), "low" | "medium" | "high" | "critical") {
            return Err(RuntimeError::Governance("invalid decision impact".into()));
        }
        Ok(())
    }
}

fn default_impact() -> String {
    "medium".into()
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct CreateGithubIssueRequest {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub labels: Vec<String>,
}

impl CreateGithubIssueRequest {
    fn validate(&self) -> Result<()> {
        if self.title.trim().is_empty() {
            return Err(RuntimeError::Governance("issue title is required".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn project() -> TempDir {
        let temp = tempfile::tempdir().expect("temp project");
        fs::create_dir_all(temp.path().join(".batai/tasks")).expect("tasks");
        temp
    }

    #[test]
    fn project_lock_rejects_a_second_writer_without_deleting_metadata() {
        let temp = project();
        let first = ProjectRuntimeLock::acquire(temp.path(), ApplicationMode::Http).expect("lock");
        let error = ProjectRuntimeLock::acquire(temp.path(), ApplicationMode::Mcp)
            .err()
            .expect("second owner denied");
        assert!(error.to_string().contains("active Batai runtime owner"));
        drop(first);
        ProjectRuntimeLock::acquire(temp.path(), ApplicationMode::Mcp).expect("released lock");
    }

    #[test]
    fn legacy_role_is_converted_at_the_transport_boundary() {
        let (_, function) = legacy_identity("Senior Backend Engineer");
        assert_eq!(function.department().to_string(), "Engineering");
        assert_eq!(normalize_provider("codex-cli"), "codex");
    }
}
