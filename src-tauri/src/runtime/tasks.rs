use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::Path,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use sha2::{Digest, Sha256};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

type AgentGuards = Vec<(String, OwnedMutexGuard<()>)>;

use super::{
    agents::AgentRegistry,
    economic::{
        EconomicPolicy, QuotaAwareEconomicRouter, RoutingOutcome, TaskRequirements, TaskRisk,
    },
    errors::{Result, RuntimeError},
    events::EventEngine,
    execution_provider::provider_failure,
    organization::AgentLifecycle,
    recovery::{agent_db_entity, RecoveryEngine},
    sessions::SessionManager,
    store::RuntimeStore,
    types::{
        AgentStatus, EventType, IngestDisposition, ResourceState, ResourceStatus, Task,
        TaskRunStatus, TaskStatus,
    },
};

#[derive(Clone)]
pub struct TaskEngine {
    store: RuntimeStore,
    events: EventEngine,
    agents: AgentRegistry,
    sessions: SessionManager,
    agent_locks: Arc<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
    running_tasks: Arc<Mutex<HashSet<String>>>,
    unknown_quota_retry: Duration,
    worktrees: Option<super::worktrees::WorktreeManager>,
    recovery: Option<RecoveryEngine>,
    economic_routing: bool,
}

impl TaskEngine {
    pub fn new(
        store: RuntimeStore,
        events: EventEngine,
        agents: AgentRegistry,
        sessions: SessionManager,
        unknown_quota_retry: Duration,
    ) -> Self {
        Self {
            store,
            events,
            agents,
            sessions,
            agent_locks: Default::default(),
            running_tasks: Default::default(),
            unknown_quota_retry,
            worktrees: None,
            recovery: None,
            economic_routing: false,
        }
    }

    pub fn with_worktrees(mut self, manager: super::worktrees::WorktreeManager) -> Self {
        self.worktrees = Some(manager);
        self
    }

    pub fn with_recovery(mut self, recovery: RecoveryEngine) -> Self {
        self.recovery = Some(recovery);
        self
    }

    pub fn with_economic_routing(mut self) -> Self {
        self.economic_routing = true;
        self
    }

    pub async fn ingest_file(self: &Arc<Self>, path: &Path) -> Result<IngestDisposition> {
        let bytes = std::fs::read(path)?;
        let task: Task =
            serde_json::from_slice(&bytes).map_err(|error| RuntimeError::InvalidTaskFile {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
        self.ingest(task, Some(path)).await
    }

    pub async fn ingest(
        self: &Arc<Self>,
        task: Task,
        source_file: Option<&Path>,
    ) -> Result<IngestDisposition> {
        validate(&task)?;
        let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&task)?));
        let disposition = self.store.ingest_task(&task, source_file, &hash)?;
        if disposition == IngestDisposition::Duplicate {
            return Ok(disposition);
        }
        if let Err(error) = self.ensure_acyclic() {
            let _ = self.set_status(&task.id, TaskStatus::Failed);
            self.events.publish(
                EventType::TaskFailed,
                "batai",
                None,
                Some(task.id.clone()),
                serde_json::json!({"error":error.to_string()}),
            )?;
            return Err(error);
        }
        let event_type = if disposition == IngestDisposition::Created {
            EventType::TaskCreated
        } else {
            EventType::TaskUpdated
        };
        self.events.publish(event_type, task.created_by.clone(), None, Some(task.id.clone()),
            serde_json::json!({"assigned_to":task.assigned_to,"source_file":source_file.map(|p|p.to_string_lossy())}))?;
        self.evaluate(&task.id).await?;
        Ok(disposition)
    }

    pub fn evaluate<'a>(
        self: &'a Arc<Self>,
        task_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Task>> + Send + 'a>> {
        Box::pin(async move {
            let task = self
                .store
                .get_task(task_id)?
                .ok_or_else(|| RuntimeError::TaskNotFound(task_id.into()))?;
            if matches!(
                task.status,
                TaskStatus::Completed
                    | TaskStatus::Cancelled
                    | TaskStatus::Running
                    | TaskStatus::WaitingResource
                    | TaskStatus::Review
            ) {
                return Ok(task);
            }
            if !self.dependencies_satisfied(&task)? {
                if task.status != TaskStatus::Blocked {
                    self.set_status(task_id, TaskStatus::Blocked)?;
                    self.events.publish(
                        EventType::TaskBlocked,
                        "batai",
                        None,
                        Some(task_id.into()),
                        serde_json::json!({"dependencies":task.dependencies}),
                    )?;
                }
                return self
                    .store
                    .get_task(task_id)?
                    .ok_or_else(|| RuntimeError::TaskNotFound(task_id.into()));
            }
            for agent_id in &task.assigned_to {
                let agent = self
                    .agents
                    .get(agent_id)?
                    .ok_or_else(|| RuntimeError::AgentNotFound(agent_id.clone()))?;
                if agent.status == AgentStatus::Terminated {
                    return Err(RuntimeError::InvalidTask {
                        task_id: task.id.clone(),
                        message: format!("agent {agent_id} is terminated"),
                    });
                }
            }
            if task.status != TaskStatus::Ready {
                self.set_status(task_id, TaskStatus::Ready)?;
            }
            let guards = match self.try_agent_guards(&task.assigned_to)? {
                Some(guards) => guards,
                None => {
                    return self
                        .store
                        .get_task(task_id)?
                        .ok_or_else(|| RuntimeError::TaskNotFound(task_id.into()))
                }
            };
            self.dispatch(task_id, guards).await
        })
    }

    async fn dispatch(
        self: &Arc<Self>,
        task_id: &str,
        guards: Vec<(String, OwnedMutexGuard<()>)>,
    ) -> Result<Task> {
        {
            let mut running = self
                .running_tasks
                .lock()
                .map_err(|_| RuntimeError::Lock("running tasks"))?;
            if !running.insert(task_id.into()) {
                return self
                    .store
                    .get_task(task_id)?
                    .ok_or_else(|| RuntimeError::TaskNotFound(task_id.into()));
            }
        }
        let result = self.dispatch_inner(task_id, guards).await;
        self.running_tasks
            .lock()
            .map_err(|_| RuntimeError::Lock("running tasks"))?
            .remove(task_id);
        self.evaluate_ready().await?;
        result
    }

    async fn dispatch_inner(
        self: &Arc<Self>,
        task_id: &str,
        guards: Vec<(String, OwnedMutexGuard<()>)>,
    ) -> Result<Task> {
        let task = self
            .store
            .get_task(task_id)?
            .ok_or_else(|| RuntimeError::TaskNotFound(task_id.into()))?;
        self.set_status(task_id, TaskStatus::Running)?;
        self.events.publish(
            EventType::TaskStarted,
            "batai",
            None,
            Some(task_id.into()),
            serde_json::json!({"assigned_to":task.assigned_to}),
        )?;

        let mut handles = Vec::new();
        for (agent_id, guard) in guards {
            if self
                .store
                .get_task_run(task_id, &agent_id)?
                .is_some_and(|run| run.status == TaskRunStatus::Completed)
            {
                continue;
            }
            let engine = Arc::clone(self);
            let task_clone = task.clone();
            handles.push(tokio::spawn(async move {
                engine.run_agent(agent_id, task_clone, guard).await
            }));
        }

        let mut resource_error = None;
        let mut provider_error = None;
        let mut unknown_error = None;
        let mut cancelled = false;
        for handle in handles {
            match handle.await.map_err(|error| {
                RuntimeError::Provider(format!("execution worker stopped: {error}"))
            })? {
                Ok(()) => {}
                Err(error @ RuntimeError::ResourceUnavailable { .. }) => {
                    resource_error = Some(error)
                }
                Err(RuntimeError::Cancelled(_)) => cancelled = true,
                Err(error @ RuntimeError::UnknownAfterCrash(_)) => unknown_error = Some(error),
                Err(error) => provider_error = Some(error),
            }
        }
        if cancelled || self.required_task(task_id)?.status == TaskStatus::Cancelled {
            return self.required_task(task_id);
        }
        if let Some(error) = unknown_error {
            self.set_status(task_id, TaskStatus::Review)?;
            self.events.publish(
                EventType::ReviewRequired,
                "batai",
                Some("director".into()),
                Some(task_id.into()),
                serde_json::json!({"reason":"unknown_after_crash","error":error.to_string()}),
            )?;
            return self.required_task(task_id);
        }
        if let Some(error) = provider_error {
            self.set_status(task_id, TaskStatus::Failed)?;
            self.events.publish(
                EventType::TaskFailed,
                "batai",
                task.on_failure.notify.clone(),
                Some(task_id.into()),
                serde_json::json!({"error":error.to_string()}),
            )?;
            return self.required_task(task_id);
        }
        if resource_error.is_some() {
            self.set_status(task_id, TaskStatus::WaitingResource)?;
            return self.required_task(task_id);
        }

        let runs = self.store.list_task_runs(task_id)?;
        if runs.len() != task.assigned_to.len()
            || runs
                .iter()
                .any(|run| run.status != TaskRunStatus::Completed)
        {
            self.set_status(task_id, TaskStatus::Ready)?;
            return self.required_task(task_id);
        }
        let combined = serde_json::json!({"task_id":task_id,"results":runs.iter().filter_map(|run|run.result.clone()).collect::<Vec<_>>(),
            "completed_at":chrono::Utc::now().to_rfc3339()});
        self.store.set_task_result(task_id, &combined)?;
        if task.execution.requires_director_review {
            self.set_status(task_id, TaskStatus::Review)?;
            self.events.publish(
                EventType::ReviewRequired,
                "batai",
                Some("director".into()),
                Some(task_id.into()),
                combined,
            )?;
        } else {
            self.complete(task_id, "batai", combined).await?;
        }
        self.required_task(task_id)
    }

    async fn run_agent(
        &self,
        agent_id: String,
        task: Task,
        _guard: OwnedMutexGuard<()>,
    ) -> Result<()> {
        let mut agent = self
            .agents
            .get(&agent_id)?
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_id.clone()))?;
        let configured_intelligence = (
            agent.provider.clone(),
            agent.model.clone(),
            agent.reasoning_effort.clone(),
        );
        let mut routing_decision = None;
        if self.economic_routing
            && agent.intelligence_policy.assignment == super::organization::ModelAssignment::Auto
            && agent.provider.eq_ignore_ascii_case("auto")
        {
            let function = agent
                .with_backfilled_organization()
                .function
                .unwrap_or(super::organization::AgentFunction::GenericSoftwareAgent);
            let complexity = task
                .extra
                .get("complexity")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(50)
                .min(100) as u8;
            let risk = match task.extra.get("risk").and_then(serde_json::Value::as_str) {
                Some("LOW") => TaskRisk::Low,
                Some("HIGH") => TaskRisk::High,
                Some("CRITICAL") => TaskRisk::Critical,
                _ => TaskRisk::Medium,
            };
            let requirements = TaskRequirements {
                function,
                complexity,
                risk,
                context_tokens: task
                    .extra
                    .get("context_tokens")
                    .and_then(serde_json::Value::as_u64),
                requires_tools: function.is_coding(),
                requires_worktree: function.is_coding(),
                priority: task
                    .extra
                    .get("priority")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(50)
                    .min(100) as u8,
                ..TaskRequirements::default()
            };
            let mut policy = self
                .store
                .economic_policy()
                .unwrap_or_else(|_| EconomicPolicy::default());
            policy.allow_payg &= agent.intelligence_policy.allow_payg;
            if !agent.intelligence_policy.preferred_providers.is_empty() {
                let preferred = agent
                    .intelligence_policy
                    .preferred_providers
                    .iter()
                    .map(|value| value.to_ascii_lowercase())
                    .collect::<HashSet<_>>();
                for resource in self.store.list_intelligence_resources()? {
                    if !preferred.contains(&resource.provider.to_ascii_lowercase()) {
                        policy
                            .forbidden_providers
                            .insert(resource.provider.to_ascii_lowercase());
                    }
                }
            }
            let resources = self.store.list_intelligence_resources()?;
            let decision = QuotaAwareEconomicRouter.route(
                &task.id,
                &requirements,
                &resources,
                &policy,
                chrono::Utc::now(),
            );
            self.store.save_routing_decision(&decision)?;
            if decision.outcome == RoutingOutcome::NoSuitableResource {
                return Err(RuntimeError::ResourceUnavailable {
                    agent_id: agent_id.clone(),
                    status: "NO_SUITABLE_RESOURCE".into(),
                    reset_at: None,
                });
            }
            agent.provider = decision
                .selected_provider
                .clone()
                .expect("selected provider");
            agent.model = decision.selected_model.clone().unwrap_or_default();
            agent.reasoning_effort = decision
                .reasoning_effort
                .clone()
                .unwrap_or_else(|| "MEDIUM".into())
                .to_ascii_lowercase();
            routing_decision = Some(decision);
        }
        let mut binding = None;
        let coding_function = agent
            .with_backfilled_organization()
            .function
            .is_some_and(super::organization::AgentFunction::is_coding);
        if coding_function || super::worktrees::role_requires_worktree(&agent.role_template) {
            if let Some(manager) = &self.worktrees {
                let before = self
                    .agents
                    .get(&agent_id)?
                    .ok_or_else(|| RuntimeError::AgentNotFound(agent_id.clone()))?;
                let created = manager.ensure(&agent_id, &task.id)?;
                agent.worktree = Some(created.path.to_string_lossy().into_owned());
                let mut persisted_after = agent.clone();
                persisted_after.provider = configured_intelligence.0.clone();
                persisted_after.model = configured_intelligence.1.clone();
                persisted_after.reasoning_effort = configured_intelligence.2.clone();
                if let Some(recovery) = &self.recovery {
                    let mut journal = recovery.prepare(
                        "ASSIGN_WORKTREE_METADATA",
                        "batai",
                        Some(agent_id.clone()),
                        0,
                        0,
                        vec![],
                        vec![agent_db_entity(Some(before), Some(persisted_after))?],
                        None,
                        Some(task.id.clone()),
                    )?;
                    recovery.apply_files(&mut journal)?;
                    if let Err(error) = recovery.apply_db(&mut journal) {
                        let _ = recovery.fail_and_rollback(&mut journal, &error.to_string());
                        return Err(error);
                    }
                    recovery.commit(&mut journal)?;
                } else {
                    self.agents.register(&persisted_after)?;
                }
                self.events.publish(
                    if created.reused {
                        EventType::WorktreeReused
                    } else {
                        EventType::WorktreeCreated
                    },
                    agent_id.clone(),
                    None,
                    Some(task.id.clone()),
                    serde_json::to_value(&created)?,
                )?;
                binding = Some(created);
            }
        }
        match agent.status {
            AgentStatus::Created => {
                self.agents
                    .transition(&agent_id, AgentStatus::Initializing, None)?;
                self.agents
                    .transition(&agent_id, AgentStatus::Ready, None)?;
            }
            AgentStatus::Initializing => {
                self.agents
                    .transition(&agent_id, AgentStatus::Ready, None)?;
            }
            AgentStatus::Blocked
            | AgentStatus::WaitingResource
            | AgentStatus::Paused
            | AgentStatus::Failed
            | AgentStatus::Completed => {
                self.agents
                    .transition(&agent_id, AgentStatus::Ready, None)?;
            }
            _ => {}
        }
        agent = self
            .agents
            .get(&agent_id)?
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_id.clone()))?;
        if let Some(decision) = &routing_decision {
            agent.provider = decision
                .selected_provider
                .clone()
                .expect("selected decision has provider");
            agent.model = decision.selected_model.clone().unwrap_or_default();
            agent.reasoning_effort = decision
                .reasoning_effort
                .clone()
                .unwrap_or_else(|| "MEDIUM".into())
                .to_ascii_lowercase();
        }
        if agent.status == AgentStatus::Ready {
            self.agents
                .transition(&agent_id, AgentStatus::Running, Some(&task.id))?;
        }
        let resumed_session = self.store.get_session(&agent_id)?.is_some();
        let session = self.sessions.ensure(&agent).await?;
        self.store.mark_task_run(
            &task.id,
            &agent_id,
            TaskRunStatus::Running,
            None,
            Some(&agent.provider),
            Some(&session.provider_session_id),
        )?;
        self.store.update_task_run_checkpoint(&task.id, &agent_id, &serde_json::json!({
            "execution_state":"running","provider":agent.provider,"session_id":session.provider_session_id,
            "worktree":binding.as_ref().map(|b| b.path.to_string_lossy().into_owned()),
            "branch":binding.as_ref().map(|b| b.branch.clone()),"base_commit":binding.as_ref().map(|b| b.base_commit.clone()),
            "starting_head":binding.as_ref().map(|b| b.starting_head.clone()),"started_at":chrono::Utc::now().to_rfc3339(),
            "routing_decision":routing_decision.as_ref().map(|decision| serde_json::json!({"id":decision.id,"resource_id":decision.selected_resource_id,"reasons":decision.reasons}))
        }))?;
        self.events.publish(
            if resumed_session {
                EventType::ProviderSessionResumed
            } else {
                EventType::ProviderSessionStarted
            },
            agent_id.clone(),
            None,
            Some(task.id.clone()),
            serde_json::json!({"provider":agent.provider,"session_id":session.provider_session_id}),
        )?;
        self.events.publish(
            EventType::ProviderTurnStarted,
            agent_id.clone(),
            None,
            Some(task.id.clone()),
            serde_json::json!({"provider":agent.provider,"session_id":session.provider_session_id}),
        )?;
        self.events.publish(
            EventType::TaskDispatched,
            "batai",
            Some(agent_id.clone()),
            Some(task.id.clone()),
            serde_json::json!({"session_id":session.provider_session_id}),
        )?;
        let provider = self.sessions.provider_for(&agent)?;
        self.events.publish(
            EventType::ProviderProcessStarted,
            agent_id.clone(),
            None,
            Some(task.id.clone()),
            serde_json::json!({"provider":agent.provider,"session_id":session.provider_session_id}),
        )?;
        let provider_outcome = provider
            .send_task(&session.provider_session_id, &agent, &task)
            .await;
        self.events.publish(EventType::ProviderProcessExited, agent_id.clone(), None, Some(task.id.clone()),
            serde_json::json!({"provider":agent.provider,"session_id":session.provider_session_id,"success":provider_outcome.is_ok()}))?;
        match provider_outcome {
            Ok(mut result) => {
                if self.required_task(&task.id)?.status == TaskStatus::Cancelled {
                    self.store.mark_task_run(
                        &task.id,
                        &agent_id,
                        TaskRunStatus::Cancelled,
                        None,
                        Some(&agent.provider),
                        Some(&session.provider_session_id),
                    )?;
                    self.agents
                        .transition(&agent_id, AgentStatus::Ready, None)?;
                    return Err(RuntimeError::Cancelled(task.id));
                }
                let (ending_head, changed_files) =
                    if let (Some(manager), Some(binding)) = (&self.worktrees, &binding) {
                        let status = manager.status(binding)?;
                        (Some(manager.head(binding)?), status)
                    } else {
                        (None, vec![])
                    };
                if result.changed_files.is_empty() {
                    result.changed_files = changed_files;
                }
                let returned_session_id = result.session_id.clone();
                self.sessions.update_after_turn(
                    &agent_id,
                    &session.provider_session_id,
                    &returned_session_id,
                    &result.session_metadata,
                )?;
                let resource_status = if result
                    .usage
                    .used_percent
                    .is_some_and(|value| value >= 100.0)
                {
                    ResourceStatus::RateLimited
                } else if result.usage.used_percent.is_some_and(|value| value >= 90.0) {
                    ResourceStatus::Low
                } else {
                    ResourceStatus::Available
                };
                self.store.upsert_resource(&ResourceState {
                    agent_id: agent_id.clone(),
                    status: resource_status,
                    reset_at: result.usage.reset_at.clone(),
                    details: serde_json::to_value(&result.usage)?,
                    updated_at: chrono::Utc::now().to_rfc3339(),
                })?;
                let result = result.into_value();
                self.store.mark_task_run(
                    &task.id,
                    &agent_id,
                    TaskRunStatus::Completed,
                    Some(&result),
                    Some(&agent.provider),
                    Some(&returned_session_id),
                )?;
                self.store.update_task_run_checkpoint(&task.id, &agent_id, &serde_json::json!({
                    "execution_state":"completed","provider":agent.provider,"session_id":returned_session_id,
                    "turn_id":result.get("turn_id"),"process_id":result.pointer("/provider_metadata/process_pid"),
                    "worktree":agent.worktree,"ending_head":ending_head,
                    "changed_files":result.get("changed_files"),"completed_at":chrono::Utc::now().to_rfc3339(),
                    "routing_decision":routing_decision.as_ref().map(|decision| serde_json::json!({"id":decision.id,"resource_id":decision.selected_resource_id,"reasons":decision.reasons}))
                }))?;
                self.events.publish(EventType::ProviderTurnCompleted, agent_id.clone(), None, Some(task.id.clone()),
                    serde_json::json!({"provider":agent.provider,"session_id":returned_session_id,"turn_id":result.get("turn_id")}))?;
                self.events.publish(
                    EventType::UsageUpdated,
                    agent_id.clone(),
                    None,
                    Some(task.id.clone()),
                    result.get("usage").cloned().unwrap_or_default(),
                )?;
                self.events.publish(
                    EventType::TaskResultRecorded,
                    agent_id.clone(),
                    None,
                    Some(task.id.clone()),
                    result,
                )?;
                self.agents
                    .transition(&agent_id, AgentStatus::Ready, None)?;
                Ok(())
            }
            Err(provider_error) => {
                if provider_error == super::execution_provider::ProviderFailure::Cancelled {
                    self.store.mark_task_run(
                        &task.id,
                        &agent_id,
                        TaskRunStatus::Cancelled,
                        None,
                        Some(&agent.provider),
                        Some(&session.provider_session_id),
                    )?;
                    self.agents
                        .transition(&agent_id, AgentStatus::Ready, None)?;
                    self.events.publish(
                        EventType::ProviderTurnFailed,
                        agent_id.clone(),
                        None,
                        Some(task.id.clone()),
                        serde_json::json!({"cancelled":true}),
                    )?;
                    return Err(RuntimeError::Cancelled(task.id));
                }
                if provider_error.leaves_execution_unknown() {
                    let changed_files =
                        if let (Some(manager), Some(binding)) = (&self.worktrees, &binding) {
                            manager.status(binding).unwrap_or_default()
                        } else {
                            vec![]
                        };
                    self.store.mark_task_run(
                        &task.id,
                        &agent_id,
                        TaskRunStatus::UnknownAfterCrash,
                        None,
                        Some(&agent.provider),
                        Some(&session.provider_session_id),
                    )?;
                    self.store.update_task_run_checkpoint(&task.id, &agent_id, &serde_json::json!({"execution_state":"unknown_after_crash","provider":agent.provider,
                        "session_id":session.provider_session_id,"worktree":agent.worktree,"changed_files":changed_files,"failed_at":chrono::Utc::now().to_rfc3339()}))?;
                    self.agents
                        .transition(&agent_id, AgentStatus::Ready, None)?;
                    self.events.publish(
                        EventType::ProviderCrashDetected,
                        agent_id.clone(),
                        None,
                        Some(task.id.clone()),
                        serde_json::json!({"changed_files":changed_files}),
                    )?;
                    if !changed_files.is_empty() {
                        self.events.publish(
                            EventType::WorktreeDirty,
                            agent_id.clone(),
                            None,
                            Some(task.id.clone()),
                            serde_json::json!({"files":changed_files}),
                        )?;
                    }
                    return Err(RuntimeError::UnknownAfterCrash(task.id));
                }
                let error = provider_failure(provider_error, &agent_id);
                match &error {
                    RuntimeError::ResourceUnavailable {
                        status, reset_at, ..
                    } => {
                        let resource_status = status.parse().unwrap_or(ResourceStatus::Unknown);
                        self.store.mark_task_run(
                            &task.id,
                            &agent_id,
                            TaskRunStatus::WaitingResource,
                            None,
                            Some(&agent.provider),
                            Some(&session.provider_session_id),
                        )?;
                        self.store.upsert_resource(&ResourceState {
                            agent_id: agent_id.clone(),
                            status: resource_status,
                            reset_at: reset_at.clone(),
                            details: serde_json::json!({"task_id":task.id}),
                            updated_at: chrono::Utc::now().to_rfc3339(),
                        })?;
                        self.agents.transition(
                            &agent_id,
                            AgentStatus::WaitingResource,
                            Some(&task.id),
                        )?;
                        let run_at = reset_at.clone().unwrap_or_else(|| {
                            (chrono::Utc::now()
                                + chrono::Duration::from_std(self.unknown_quota_retry)
                                    .unwrap_or(chrono::Duration::minutes(15)))
                            .to_rfc3339()
                        });
                        self.store.schedule_resource_recheck(
                            &agent_id,
                            &run_at,
                            &serde_json::json!({"agent_id":agent_id}),
                        )?;
                        self.events.publish(
                            if resource_status == ResourceStatus::RateLimited {
                                EventType::AgentRateLimited
                            } else {
                                EventType::ResourceStatusChanged
                            },
                            agent_id.clone(),
                            None,
                            Some(task.id.clone()),
                            serde_json::json!({"status":resource_status,"reset_at":reset_at}),
                        )?;
                    }
                    _ => {
                        self.store.mark_task_run(
                            &task.id,
                            &agent_id,
                            TaskRunStatus::Failed,
                            None,
                            Some(&agent.provider),
                            Some(&session.provider_session_id),
                        )?;
                        self.agents
                            .transition(&agent_id, AgentStatus::Ready, None)?;
                    }
                }
                Err(error)
            }
        }
    }

    pub async fn cancel(self: &Arc<Self>, task_id: &str, requested_by: &str) -> Result<Task> {
        let task = self.required_task(task_id)?;
        if matches!(task.status, TaskStatus::Completed | TaskStatus::Cancelled) {
            return Ok(task);
        }
        self.events.publish(
            EventType::TaskCancellationRequested,
            requested_by,
            None,
            Some(task_id.into()),
            serde_json::json!({}),
        )?;
        for agent_id in &task.assigned_to {
            if let Some(session) = self.store.get_session(agent_id)? {
                if let Some(agent) = self.agents.get(agent_id)? {
                    let provider = self.sessions.provider_for(&agent)?;
                    let _ = provider.cancel_turn(&session.provider_session_id).await;
                }
            }
            if self
                .store
                .get_task_run(task_id, agent_id)?
                .is_some_and(|run| run.status == TaskRunStatus::Running)
            {
                self.store.mark_task_run(
                    task_id,
                    agent_id,
                    TaskRunStatus::Cancelled,
                    None,
                    None,
                    None,
                )?;
            }
            if self
                .agents
                .get(agent_id)?
                .is_some_and(|agent| agent.status == AgentStatus::Running)
            {
                self.agents.transition(agent_id, AgentStatus::Ready, None)?;
            }
        }
        self.set_status(task_id, TaskStatus::Cancelled)?;
        self.events.publish(
            EventType::TaskCancelled,
            requested_by,
            None,
            Some(task_id.into()),
            serde_json::json!({}),
        )?;
        self.required_task(task_id)
    }

    pub async fn approve_review(self: &Arc<Self>, task_id: &str, reviewer: &str) -> Result<Task> {
        let task = self
            .store
            .get_task(task_id)?
            .ok_or_else(|| RuntimeError::TaskNotFound(task_id.into()))?;
        if task.status != TaskStatus::Review {
            return Err(RuntimeError::InvalidTask {
                task_id: task_id.into(),
                message: "not awaiting review".into(),
            });
        }
        self.complete(task_id, reviewer, serde_json::json!({"approved":true}))
            .await?;
        self.store
            .get_task(task_id)?
            .ok_or_else(|| RuntimeError::TaskNotFound(task_id.into()))
    }

    async fn complete(
        self: &Arc<Self>,
        task_id: &str,
        source: &str,
        payload: serde_json::Value,
    ) -> Result<()> {
        self.set_status(task_id, TaskStatus::Completed)?;
        self.events.publish(
            EventType::TaskCompleted,
            source,
            None,
            Some(task_id.into()),
            payload,
        )?;
        let task = self.required_task(task_id)?;
        let all_tasks = self.store.list_tasks()?;
        for agent_id in &task.assigned_to {
            let has_other_work = all_tasks.iter().any(|candidate| {
                candidate.id != task.id
                    && candidate.assigned_to.contains(agent_id)
                    && !matches!(
                        candidate.status,
                        TaskStatus::Completed | TaskStatus::Cancelled | TaskStatus::Failed
                    )
            });
            if !has_other_work {
                if let Some(agent) = self.agents.get(agent_id)? {
                    if agent.schema_version >= 2
                        && agent.lifecycle == AgentLifecycle::TaskScoped
                        && agent.status != AgentStatus::Terminated
                    {
                        self.agents
                            .transition(agent_id, AgentStatus::Terminated, None)?;
                    }
                }
            }
        }
        let mut candidates = task.on_success.start;
        for candidate in self.store.list_tasks()? {
            if candidate.dependencies.contains(&task_id.to_owned()) {
                candidates.push(candidate.id);
            }
        }
        candidates.sort();
        candidates.dedup();
        for candidate in candidates {
            let engine = Arc::clone(self);
            tokio::spawn(async move {
                let _ = engine.evaluate(&candidate).await;
            });
        }
        Ok(())
    }

    pub async fn retry(self: &Arc<Self>, task_id: &str) -> Result<Task> {
        let task = self
            .store
            .get_task(task_id)?
            .ok_or_else(|| RuntimeError::TaskNotFound(task_id.into()))?;
        if !matches!(
            task.status,
            TaskStatus::Failed | TaskStatus::WaitingResource | TaskStatus::Blocked
        ) {
            return Err(RuntimeError::InvalidTask {
                task_id: task_id.into(),
                message: "task is not retryable".into(),
            });
        }
        self.set_status(task_id, TaskStatus::Ready)?;
        self.evaluate(task_id).await
    }

    pub async fn resume_agent(self: &Arc<Self>, agent_id: &str) -> Result<()> {
        if let Some(agent) = self.agents.get(agent_id)? {
            if agent.status == AgentStatus::WaitingResource {
                self.agents.transition(agent_id, AgentStatus::Ready, None)?;
            }
        }
        for task in self.store.list_tasks()? {
            if task.status == TaskStatus::WaitingResource
                && task.assigned_to.contains(&agent_id.to_owned())
            {
                if let Some(run) = self.store.get_task_run(&task.id, agent_id)? {
                    if run.status == TaskRunStatus::WaitingResource {
                        self.store.mark_task_run(
                            &task.id,
                            agent_id,
                            TaskRunStatus::Pending,
                            None,
                            None,
                            None,
                        )?;
                    }
                }
                self.set_status(&task.id, TaskStatus::Ready)?;
                let _ = self.evaluate(&task.id).await;
            }
        }
        self.events.publish(
            EventType::AgentResumed,
            agent_id,
            None,
            None,
            serde_json::json!({}),
        )?;
        Ok(())
    }

    pub async fn reconcile(self: &Arc<Self>) -> Result<()> {
        for task in self.store.list_tasks()? {
            if matches!(
                task.status,
                TaskStatus::Pending | TaskStatus::Ready | TaskStatus::Blocked
            ) {
                let _ = self.evaluate(&task.id).await;
            }
        }
        Ok(())
    }

    async fn evaluate_ready(self: &Arc<Self>) -> Result<()> {
        for task in self.store.list_tasks()? {
            if matches!(
                task.status,
                TaskStatus::Pending | TaskStatus::Ready | TaskStatus::Blocked
            ) {
                let engine = Arc::clone(self);
                let id = task.id;
                tokio::spawn(async move {
                    let _ = engine.evaluate(&id).await;
                });
            }
        }
        Ok(())
    }

    fn try_agent_guards(&self, agent_ids: &[String]) -> Result<Option<AgentGuards>> {
        let mut ids = agent_ids.to_vec();
        ids.sort();
        ids.dedup();
        let locks = {
            let mut map = self
                .agent_locks
                .lock()
                .map_err(|_| RuntimeError::Lock("agent locks"))?;
            ids.iter()
                .map(|id| {
                    (
                        id.clone(),
                        map.entry(id.clone())
                            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                            .clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut guards = Vec::new();
        for (id, lock) in locks {
            match lock.try_lock_owned() {
                Ok(guard) => guards.push((id, guard)),
                Err(_) => return Ok(None),
            }
        }
        Ok(Some(guards))
    }

    fn dependencies_satisfied(&self, task: &Task) -> Result<bool> {
        for id in &task.dependencies {
            if self
                .store
                .get_task(id)?
                .is_none_or(|dependency| dependency.status != TaskStatus::Completed)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn ensure_acyclic(&self) -> Result<()> {
        let tasks = self.store.list_tasks()?;
        let graph: HashMap<_, _> = tasks
            .into_iter()
            .map(|task| (task.id, task.dependencies))
            .collect();
        fn visit(
            node: &str,
            graph: &HashMap<String, Vec<String>>,
            visiting: &mut HashSet<String>,
            visited: &mut HashSet<String>,
        ) -> std::result::Result<(), String> {
            if visiting.contains(node) {
                return Err(node.into());
            }
            if visited.contains(node) {
                return Ok(());
            }
            visiting.insert(node.into());
            if let Some(edges) = graph.get(node) {
                for edge in edges {
                    if graph.contains_key(edge) {
                        visit(edge, graph, visiting, visited)?;
                    }
                }
            }
            visiting.remove(node);
            visited.insert(node.into());
            Ok(())
        }
        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        for node in graph.keys() {
            if let Err(at) = visit(node, &graph, &mut visiting, &mut visited) {
                return Err(RuntimeError::DependencyCycle(at));
            }
        }
        Ok(())
    }

    fn set_status(&self, id: &str, to: TaskStatus) -> Result<Task> {
        let task = self
            .store
            .get_task(id)?
            .ok_or_else(|| RuntimeError::TaskNotFound(id.into()))?;
        if task.status == to {
            return Ok(task);
        }
        if !task_transition(task.status, to) {
            return Err(RuntimeError::InvalidTaskTransition {
                from: task.status,
                to,
            });
        }
        self.store.set_task_status(id, to)
    }

    fn required_task(&self, id: &str) -> Result<Task> {
        self.store
            .get_task(id)?
            .ok_or_else(|| RuntimeError::TaskNotFound(id.into()))
    }
}

fn validate(task: &Task) -> Result<()> {
    if task.id.trim().is_empty() {
        return Err(RuntimeError::InvalidTask {
            task_id: task.id.clone(),
            message: "id is empty".into(),
        });
    }
    if task.objective.trim().is_empty() {
        return Err(RuntimeError::InvalidTask {
            task_id: task.id.clone(),
            message: "objective is empty".into(),
        });
    }
    if task.assigned_to.is_empty() {
        return Err(RuntimeError::InvalidTask {
            task_id: task.id.clone(),
            message: "assigned_to is empty".into(),
        });
    }
    if task.dependencies.contains(&task.id) {
        return Err(RuntimeError::DependencyCycle(task.id.clone()));
    }
    Ok(())
}

fn task_transition(from: TaskStatus, to: TaskStatus) -> bool {
    use TaskStatus::*;
    match from {
        Pending => matches!(to, Ready | Blocked | Failed | Cancelled),
        Ready => matches!(to, Running | Blocked | Failed | Cancelled),
        Running => matches!(
            to,
            Ready | WaitingResource | Review | Completed | Failed | Cancelled
        ),
        Blocked => matches!(to, Ready | Failed | Cancelled),
        WaitingResource => matches!(to, Ready | Failed | Cancelled),
        Review => matches!(to, Completed | Failed | Cancelled),
        Failed => matches!(to, Ready | Cancelled),
        Completed | Cancelled => false,
    }
}
