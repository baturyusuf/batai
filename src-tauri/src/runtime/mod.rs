pub mod agents;
pub mod errors;
pub mod events;
pub mod execution_provider;
pub mod migrations;
pub mod organization;
pub mod scheduler;
pub mod sessions;
pub mod store;
pub mod tasks;
pub mod types;
pub mod watcher;
pub mod worktrees;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::domain::{
    task_progress, weighted_progress, ActivityView, AgentView, AppSnapshot, GodView,
    PerformanceSummary, ProjectSummary, ResourceSummary, TaskView, UsageSummary,
};
use crate::providers::{
    claude_code::ClaudeCodeProvider, codex_app_server::CodexAppServerProvider,
    ollama::OllamaProvider,
};
use agents::AgentRegistry;
use errors::Result;
use events::EventEngine;
use execution_provider::{ExecutionProvider, MockProvider};
use organization::{hierarchy_warnings, OrganizationRelationship, RelationshipType};
use scheduler::DurableScheduler;
use sessions::SessionManager;
use store::RuntimeStore;
use tasks::TaskEngine;
use watcher::TaskWatcher;

pub struct BataiRuntime {
    root: PathBuf,
    pub store: RuntimeStore,
    pub events: EventEngine,
    pub agents: AgentRegistry,
    pub tasks: Arc<TaskEngine>,
    pub sessions: SessionManager,
    watcher: Mutex<Option<TaskWatcher>>,
    scheduler: tokio::sync::Mutex<Option<DurableScheduler>>,
}

impl BataiRuntime {
    pub fn open(root: PathBuf) -> Result<Arc<Self>> {
        let store = RuntimeStore::open(root.join(".runtime/runtime.sqlite"))?;
        let mock = Arc::new(MockProvider::default());
        let mut providers = HashMap::<String, Arc<dyn ExecutionProvider>>::new();
        providers.insert("mock".into(), mock);
        providers.insert("codex".into(), Arc::new(CodexAppServerProvider::default()));
        providers.insert(
            "codex-app-server".into(),
            Arc::new(CodexAppServerProvider::default()),
        );
        providers.insert("claude".into(), Arc::new(ClaudeCodeProvider::default()));
        providers.insert(
            "claude-code".into(),
            Arc::new(ClaudeCodeProvider::default()),
        );
        providers.insert("ollama".into(), Arc::new(OllamaProvider::default()));
        Self::with_store(root, store, providers)
    }

    pub fn with_store(
        root: PathBuf,
        store: RuntimeStore,
        providers: HashMap<String, Arc<dyn ExecutionProvider>>,
    ) -> Result<Arc<Self>> {
        let events = EventEngine::new(store.clone());
        let agents = AgentRegistry::new(store.clone(), events.clone());
        let sessions = SessionManager::new(store.clone(), providers);
        let mut task_engine = TaskEngine::new(
            store.clone(),
            events.clone(),
            agents.clone(),
            sessions.clone(),
            Duration::from_secs(15 * 60),
        );
        if let Ok(manager) = worktrees::WorktreeManager::discover(&root) {
            task_engine = task_engine.with_worktrees(manager);
        }
        let tasks = Arc::new(task_engine);
        Ok(Arc::new(Self {
            root,
            store,
            events,
            agents,
            tasks,
            sessions,
            watcher: Mutex::new(None),
            scheduler: tokio::sync::Mutex::new(None),
        }))
    }

    pub async fn start(self: &Arc<Self>) -> Result<()> {
        self.store.reconcile_interrupted()?;
        self.load_agents()?;
        let watcher = TaskWatcher::start(
            &self.root.join(".batai/tasks"),
            Arc::clone(&self.tasks),
            self.events.clone(),
            Duration::from_millis(100),
        )
        .await?;
        self.tasks.reconcile().await?;
        *self
            .watcher
            .lock()
            .map_err(|_| errors::RuntimeError::Lock("watcher"))? = Some(watcher);
        *self.scheduler.lock().await = Some(DurableScheduler::start(
            self.store.clone(),
            self.events.clone(),
            self.agents.clone(),
            self.sessions.clone(),
            Arc::clone(&self.tasks),
            Duration::from_secs(30),
            Duration::from_secs(15 * 60),
        ));
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        let watcher = self
            .watcher
            .lock()
            .map_err(|_| errors::RuntimeError::Lock("watcher"))?
            .take();
        if let Some(mut watcher) = watcher {
            watcher.shutdown()?;
        }
        if let Some(mut scheduler) = self.scheduler.lock().await.take() {
            scheduler.shutdown().await;
        }
        Ok(())
    }

    fn load_agents(&self) -> Result<()> {
        let directory = self.root.join(".batai/agents");
        if !directory.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(directory)? {
            let path = entry?.path().join("config.json");
            if path.is_file() {
                let agent = serde_json::from_slice(&std::fs::read(path)?)?;
                self.agents.register(&agent)?;
            }
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Result<AppSnapshot> {
        let runtime_agents = self.agents.list()?;
        let runtime_tasks = self.store.list_tasks()?;
        let all_runs = self.store.list_all_task_runs()?;
        let events = self.store.list_events(250)?;
        let task_by_id = runtime_tasks
            .iter()
            .map(|task| (task.id.as_str(), task))
            .collect::<HashMap<_, _>>();
        let mut direct_reports = runtime_agents.iter().fold(
            HashMap::<String, Vec<String>>::new(),
            |mut reports, agent| {
                if let Some(parent) = &agent.parent_agent_id {
                    reports
                        .entry(parent.clone())
                        .or_default()
                        .push(agent.id.clone());
                }
                reports
            },
        );
        let parents = runtime_agents
            .iter()
            .map(|agent| (agent.id.clone(), agent.parent_agent_id.clone()))
            .collect::<HashMap<_, _>>();
        let mut project_usage = UsageSummary::default();
        let mut project_usage_by_source = std::collections::BTreeMap::new();
        let mut resources_by_provider = HashMap::<String, ResourceSummary>::new();
        for provider in ["codex", "claude", "ollama"] {
            resources_by_provider.insert(
                provider.into(),
                ResourceSummary {
                    provider: provider.into(),
                    status: "UNKNOWN".into(),
                    ..ResourceSummary::default()
                },
            );
        }

        let mut agents = Vec::with_capacity(runtime_agents.len());
        for raw_agent in runtime_agents {
            let agent = raw_agent.with_backfilled_organization();
            let runs = all_runs
                .iter()
                .filter(|run| run.agent_id == agent.id)
                .cloned()
                .collect::<Vec<_>>();
            let mut usage = UsageSummary::default();
            for run in &runs {
                if let Some(parsed) = run
                    .result
                    .as_ref()
                    .and_then(|result| result.get("usage"))
                    .and_then(|value| serde_json::from_value(value.clone()).ok())
                {
                    usage.add(&parsed);
                    project_usage.add(&parsed);
                    let source =
                        enum_name(parsed.source.clone()).unwrap_or_else(|| "unknown".into());
                    project_usage_by_source
                        .entry(source)
                        .or_insert_with(UsageSummary::default)
                        .add(&parsed);
                }
            }
            let resource = self.store.get_resource(&agent.id)?;
            let resource_usage: Option<execution_provider::UsageSnapshot> = resource
                .as_ref()
                .and_then(|state| serde_json::from_value(state.details.clone()).ok());
            let resource_summary = resources_by_provider
                .entry(agent.provider.clone())
                .or_insert_with(|| ResourceSummary {
                    provider: agent.provider.clone(),
                    status: "UNKNOWN".into(),
                    ..ResourceSummary::default()
                });
            if matches!(agent.status, types::AgentStatus::Running) {
                resource_summary.active_agents += 1;
            }
            resource_summary.usage.merge(&usage);
            resource_summary.usage_sources = resource_summary.usage.sources.clone();
            if let Some(state) = &resource {
                resource_summary.status = state.status.to_string();
                resource_summary.reset_at = state.reset_at.clone();
            }
            if let Some(resource_usage) = resource_usage {
                resource_summary.used_percent = resource_usage.used_percent;
                if resource_summary.usage.total_tokens.is_none()
                    && resource_summary.usage.cost.is_none()
                {
                    resource_summary.usage.add(&resource_usage);
                }
                resource_summary.usage_sources = resource_summary.usage.sources.clone();
            }
            let seniority = agent.seniority.and_then(enum_name);
            let function = agent
                .function
                .and_then(enum_name)
                .unwrap_or_else(|| "GENERIC_SOFTWARE_AGENT".into());
            let performance = performance_summary(&runs);
            let latest_event = events.iter().find(|event| {
                event.source == agent.id || event.target.as_deref() == Some(&agent.id)
            });
            let activity = current_activity(&agent, latest_event);
            let title = agent.display_title();
            let department = agent
                .department
                .map_or_else(|| "Engineering".into(), |department| department.to_string());
            let current_task_objective = agent
                .current_task_id
                .as_deref()
                .and_then(|task_id| task_by_id.get(task_id))
                .map(|task| task.objective.clone());
            let session_state = self
                .store
                .get_session(&agent.id)?
                .map(|_| "PERSISTED".into());
            agents.push(AgentView {
                id: agent.id.clone(),
                name: agent.name,
                title,
                department,
                seniority,
                level: agent.seniority.map(|level| level.level()),
                function,
                reports_to: agent.parent_agent_id,
                direct_reports: direct_reports.remove(&agent.id).unwrap_or_default(),
                provider: agent.provider,
                model: agent.model,
                reasoning_effort: agent.reasoning_effort,
                auth_mode: agent.auth_mode,
                status: agent.status.to_string(),
                activity,
                current_task_id: agent.current_task_id,
                current_task_objective,
                worktree: agent.worktree,
                session_state,
                model_capabilities: agent.model_capabilities,
                effective_capabilities: agent.effective_capabilities,
                performance,
                usage,
                quota_status: resource.as_ref().map(|state| state.status.to_string()),
                quota_reset_at: resource.and_then(|state| state.reset_at),
            });
        }
        let tasks = runtime_tasks
            .iter()
            .map(|task| TaskView {
                id: task.id.clone(),
                objective: task.objective.clone(),
                status: task.status.to_string(),
                assigned_to: task.assigned_to.clone(),
                dependencies: task.dependencies.clone(),
                weight: task.weight,
                progress: task_progress(&task.status.to_string()),
            })
            .collect::<Vec<_>>();
        let relationships = organization_relationships(&runtime_tasks, &agents);
        let activity = events
            .into_iter()
            .rev()
            .map(|event| ActivityView {
                summary: event_summary(&event),
                id: event.id,
                timestamp: event.timestamp,
                event_type: event.event_type.to_string(),
                source: event.source,
                target: event.target,
                task_id: event.task_id,
            })
            .collect();
        let name = self
            .root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Batai Project")
            .to_owned();
        let project = ProjectSummary {
            name,
            root: self.root.display().to_string(),
            progress: weighted_progress(&tasks),
            active_agents: agents
                .iter()
                .filter(|agent| agent.status == "RUNNING")
                .count(),
            waiting_agents: agents
                .iter()
                .filter(|agent| matches!(agent.status.as_str(), "WAITING_RESOURCE" | "BLOCKED"))
                .count(),
            completed_tasks: tasks
                .iter()
                .filter(|task| task.status == "COMPLETED")
                .count(),
            running_tasks: tasks.iter().filter(|task| task.status == "RUNNING").count(),
            blocked_tasks: tasks.iter().filter(|t| t.status == "BLOCKED").count(),
            remaining_tasks: tasks
                .iter()
                .filter(|task| !matches!(task.status.as_str(), "COMPLETED" | "CANCELLED"))
                .count(),
            review_tasks: tasks.iter().filter(|t| t.status == "REVIEW").count(),
            usage: project_usage,
            usage_by_source: project_usage_by_source,
        };
        Ok(AppSnapshot {
            god: GodView {
                id: "god".into(),
                label: "User".into(),
                authority: "Highest authority".into(),
                pending_decisions: None,
            },
            project,
            agents,
            tasks,
            relationships,
            resources: resources_by_provider.into_values().collect(),
            activity,
            hierarchy_warnings: hierarchy_warnings(&parents),
        })
    }
}

fn enum_name<T: serde::Serialize>(value: T) -> Option<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
}

fn current_activity(agent: &types::Agent, event: Option<&types::RuntimeEvent>) -> String {
    if agent.status == types::AgentStatus::Blocked {
        return "BLOCKED".into();
    }
    if agent.status == types::AgentStatus::WaitingResource {
        return "WAITING".into();
    }
    if agent.status != types::AgentStatus::Running {
        return "IDLE".into();
    }
    match event.map(|event| event.event_type) {
        Some(types::EventType::ReviewRequired) => "REVIEWING",
        Some(types::EventType::WorktreeDirty) => "CODING",
        Some(types::EventType::AgentRateLimited) => "RATE_LIMITED",
        Some(types::EventType::ProviderTurnStarted | types::EventType::TaskStarted) => "PROCESSING",
        _ => "WORKING",
    }
    .into()
}

fn performance_summary(runs: &[types::TaskRun]) -> PerformanceSummary {
    use types::TaskRunStatus;
    let completed = runs
        .iter()
        .filter(|run| run.status == TaskRunStatus::Completed)
        .count();
    let failed = runs
        .iter()
        .filter(|run| {
            matches!(
                run.status,
                TaskRunStatus::Failed | TaskRunStatus::UnknownAfterCrash
            )
        })
        .count();
    let active = runs
        .iter()
        .filter(|run| {
            matches!(
                run.status,
                TaskRunStatus::Running | TaskRunStatus::WaitingResource
            )
        })
        .count();
    let terminal = completed + failed;
    let first_attempt = runs
        .iter()
        .filter(|run| run.status == TaskRunStatus::Completed && run.attempt == 1)
        .count();
    let durations = runs
        .iter()
        .filter_map(|run| {
            let start = chrono::DateTime::parse_from_rfc3339(run.started_at.as_deref()?).ok()?;
            let end = chrono::DateTime::parse_from_rfc3339(run.completed_at.as_deref()?).ok()?;
            Some((end - start).num_milliseconds() as f64 / 1000.0)
        })
        .collect::<Vec<_>>();
    PerformanceSummary {
        completed_tasks: completed,
        active_tasks: active,
        failed_tasks: failed,
        first_attempt_success_rate: (completed > 0)
            .then_some(first_attempt as f64 * 100.0 / completed as f64),
        final_success_rate: (terminal > 0).then_some(completed as f64 * 100.0 / terminal as f64),
        average_attempts: (!runs.is_empty())
            .then_some(runs.iter().map(|run| run.attempt as f64).sum::<f64>() / runs.len() as f64),
        review_acceptance_rate: None,
        average_task_duration_seconds: (!durations.is_empty())
            .then_some(durations.iter().sum::<f64>() / durations.len() as f64),
    }
}

fn organization_relationships(
    tasks: &[types::Task],
    agents: &[AgentView],
) -> Vec<OrganizationRelationship> {
    let mut relationships = Vec::new();
    for agent in agents {
        let parent = agent.reports_to.as_deref().or_else(|| {
            (agent.function == "DIRECTOR" || agent.id.eq_ignore_ascii_case("director"))
                .then_some("god")
        });
        if let Some(parent) = parent {
            relationships.push(OrganizationRelationship {
                id: format!("reporting:{parent}:{}", agent.id),
                relationship_type: RelationshipType::Reporting,
                source: parent.into(),
                target: agent.id.clone(),
                persistent: agent.reports_to.is_some(),
                task_id: None,
                label: Some("reports to".into()),
            });
        }
    }
    for task in tasks {
        for dependency in &task.dependencies {
            relationships.push(OrganizationRelationship {
                id: format!("dependency:{dependency}:{}", task.id),
                relationship_type: RelationshipType::Dependency,
                source: format!("task:{dependency}"),
                target: format!("task:{}", task.id),
                persistent: true,
                task_id: Some(task.id.clone()),
                label: Some("depends on".into()),
            });
        }
        for pair in task.assigned_to.windows(2) {
            relationships.push(OrganizationRelationship {
                id: format!("collaboration:{}:{}:{}", task.id, pair[0], pair[1]),
                relationship_type: RelationshipType::Collaboration,
                source: pair[0].clone(),
                target: pair[1].clone(),
                persistent: false,
                task_id: Some(task.id.clone()),
                label: Some("collaborates".into()),
            });
        }
        if task.execution.requires_director_review {
            for agent in &task.assigned_to {
                relationships.push(OrganizationRelationship {
                    id: format!("review:{}:{agent}", task.id),
                    relationship_type: RelationshipType::Review,
                    source: agent.clone(),
                    target: "director".into(),
                    persistent: false,
                    task_id: Some(task.id.clone()),
                    label: Some("review".into()),
                });
            }
        }
    }
    relationships
}

fn event_summary(event: &types::RuntimeEvent) -> String {
    if event.event_type == types::EventType::TaskResultRecorded {
        if let Some(files) = event
            .payload
            .get("changed_files")
            .and_then(serde_json::Value::as_array)
        {
            let files = files
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>();
            if !files.is_empty() {
                return format!("Changed: {}", files.join(", "));
            }
        }
    }
    let subject = event
        .task_id
        .as_deref()
        .or(event.target.as_deref())
        .unwrap_or(&event.source);
    format!("{} · {subject}", event.event_type)
}

pub fn runtime_db_path(root: &Path) -> PathBuf {
    root.join(".runtime/runtime.sqlite")
}

#[cfg(test)]
mod tests;
