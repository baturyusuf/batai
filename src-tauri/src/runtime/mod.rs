pub mod agents;
pub mod errors;
pub mod events;
pub mod execution_provider;
pub mod migrations;
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
    task_progress, weighted_progress, AgentView, AppSnapshot, ProjectSummary, TaskView,
};
use crate::providers::{
    claude_code::ClaudeCodeProvider, codex_app_server::CodexAppServerProvider,
    ollama::OllamaProvider,
};
use agents::AgentRegistry;
use errors::Result;
use events::EventEngine;
use execution_provider::{ExecutionProvider, MockProvider};
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
        let agents = self
            .agents
            .list()?
            .into_iter()
            .map(|agent| {
                let title = agent.role_template.clone();
                AgentView {
                    id: agent.id,
                    name: agent.name,
                    department: department_for(&title),
                    title,
                    reports_to: agent.parent_agent_id,
                    provider: agent.provider,
                    model: agent.model,
                    auth_mode: agent.auth_mode,
                    status: agent.status.to_string(),
                    current_task_id: agent.current_task_id,
                    worktree: agent.worktree,
                }
            })
            .collect::<Vec<_>>();
        let tasks = self
            .store
            .list_tasks()?
            .into_iter()
            .map(|task| TaskView {
                id: task.id,
                objective: task.objective,
                status: task.status.to_string(),
                assigned_to: task.assigned_to,
                dependencies: task.dependencies,
                weight: task.weight,
                progress: task_progress(&task.status.to_string()),
            })
            .collect::<Vec<_>>();
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
                .filter(|a| matches!(a.status.as_str(), "RUNNING" | "READY"))
                .count(),
            blocked_tasks: tasks.iter().filter(|t| t.status == "BLOCKED").count(),
            review_tasks: tasks.iter().filter(|t| t.status == "REVIEW").count(),
        };
        Ok(AppSnapshot {
            project,
            agents,
            tasks,
        })
    }
}

fn department_for(role: &str) -> String {
    let role = role.to_lowercase();
    if role.contains("director") || role.contains("product") {
        "Leadership"
    } else if role.contains("review") || role.contains("qa") || role.contains("test") {
        "Quality"
    } else if role.contains("design") || role.contains("frontend") {
        "Product Design"
    } else {
        "Engineering"
    }
    .into()
}

pub fn runtime_db_path(root: &Path) -> PathBuf {
    root.join(".runtime/runtime.sqlite")
}

#[cfg(test)]
mod tests;
