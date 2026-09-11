use std::{
    collections::HashMap,
    fs,
    path::Path,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use tempfile::TempDir;

use crate::runtime::CodexAppServerProvider;

use super::{
    agents::AgentRegistry,
    economic::{
        native_resource_profile, BillingMode, EconomicPolicy, RoutingDecision, RoutingOutcome,
        TermsState,
    },
    errors::RuntimeError,
    events::EventEngine,
    execution_provider::{ExecutionProvider, MockOutcome, MockProvider, UsageState},
    migrations::SCHEMA_VERSION,
    organization::{CapabilityProfile, CapabilityScore},
    scheduler::DurableScheduler,
    sessions::SessionManager,
    store::RuntimeStore,
    tasks::TaskEngine,
    types::{
        Agent, AgentStatus, EventType, IngestDisposition, ResourceStatus, SchedulerJobStatus, Task,
        TaskExecution, TaskRunStatus, TaskStatus,
    },
    watcher::TaskWatcher,
    worktrees::WorktreeManager,
};

fn checked_git(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .output()
        .expect("run git command");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

struct Harness {
    store: RuntimeStore,
    events: EventEngine,
    agents: AgentRegistry,
    sessions: SessionManager,
    engine: Arc<TaskEngine>,
    mock: Arc<MockProvider>,
}

fn harness(store: RuntimeStore) -> Harness {
    let events = EventEngine::new(store.clone());
    let agents = AgentRegistry::new(store.clone(), events.clone());
    let mock = Arc::new(MockProvider::default());
    let mut providers = HashMap::<String, Arc<dyn ExecutionProvider>>::new();
    providers.insert("mock".into(), mock.clone());
    let sessions = SessionManager::new(store.clone(), providers);
    let engine = Arc::new(TaskEngine::new(
        store.clone(),
        events.clone(),
        agents.clone(),
        sessions.clone(),
        Duration::from_millis(50),
    ));
    Harness {
        store,
        events,
        agents,
        sessions,
        engine,
        mock,
    }
}

fn agent(id: &str) -> Agent {
    Agent {
        schema_version: 1,
        id: id.into(),
        name: id.into(),
        role_template: "SoftwareEngineer".into(),
        seniority: None,
        function: None,
        department: None,
        title_override: None,
        model_capabilities: Default::default(),
        effective_capabilities: Default::default(),
        lifecycle: Default::default(),
        authority: Default::default(),
        permissions: vec![],
        intelligence_policy: Default::default(),
        parent_agent_id: Some("director".into()),
        provider: "mock".into(),
        model: "mock-medium".into(),
        reasoning_effort: "medium".into(),
        auth_mode: "local".into(),
        worktree: None,
        status: AgentStatus::Ready,
        current_task_id: None,
        extra: Default::default(),
    }
}

fn task(id: &str, assigned_to: &[&str], dependencies: &[&str]) -> Task {
    Task {
        id: id.into(),
        created_by: "director".into(),
        objective: format!("Execute {id}"),
        assigned_to: assigned_to.iter().map(|v| (*v).into()).collect(),
        dependencies: dependencies.iter().map(|v| (*v).into()).collect(),
        acceptance_criteria: vec![],
        inputs: vec![],
        outputs: vec![],
        status: TaskStatus::Ready,
        execution: TaskExecution::default(),
        on_success: Default::default(),
        on_failure: Default::default(),
        weight: 1.0,
        extra: Default::default(),
    }
}

fn register(h: &Harness, ids: &[&str]) {
    for id in ids {
        h.agents.register(&agent(id)).expect("register agent");
    }
}

async fn wait_status(store: &RuntimeStore, id: &str, status: TaskStatus) {
    for _ in 0..100 {
        if store
            .get_task(id)
            .expect("read task")
            .is_some_and(|task| task.status == status)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("task {id} did not reach {status}");
}

#[test]
fn database_migration_creates_fresh_database() {
    let store = RuntimeStore::open_memory().expect("open database");
    assert_eq!(
        store.schema_version().expect("schema version"),
        SCHEMA_VERSION
    );
}

#[test]
fn database_migration_upgrades_existing_legacy_database_without_data_loss() {
    let directory = TempDir::new().expect("temp directory");
    let path = directory.path().join("runtime.sqlite");
    let connection = rusqlite::Connection::open(&path).expect("legacy database");
    connection.execute_batch("CREATE TABLE tasks(id TEXT PRIMARY KEY,status TEXT NOT NULL,created_by TEXT NOT NULL,objective TEXT NOT NULL,assigned_to_json TEXT NOT NULL,source_file TEXT,task_json TEXT NOT NULL,result_json TEXT,updated_at TEXT NOT NULL); INSERT INTO tasks VALUES('legacy','COMPLETED','director','legacy','[]',NULL,'{\"id\":\"legacy\",\"created_by\":\"director\",\"objective\":\"legacy\",\"assigned_to\":[\"a\"],\"status\":\"COMPLETED\"}',NULL,'now');").expect("legacy schema");
    drop(connection);
    let store = RuntimeStore::open(&path).expect("migrate");
    assert_eq!(
        store
            .get_task("legacy")
            .expect("read")
            .expect("task")
            .status,
        TaskStatus::Completed
    );
    assert_eq!(store.schema_version().expect("version"), SCHEMA_VERSION);
}

#[test]
fn economic_resources_policy_and_routing_audit_round_trip_without_credentials() {
    let store = RuntimeStore::open_memory().expect("store");
    let profile = native_resource_profile("local", "ollama", "Local", BillingMode::Local);
    store.upsert_intelligence_resource(&profile).unwrap();
    assert_eq!(store.list_intelligence_resources().unwrap(), vec![profile]);
    let policy = EconomicPolicy {
        allow_payg: true,
        ..EconomicPolicy::default()
    };
    store.set_economic_policy(&policy).unwrap();
    assert_eq!(store.economic_policy().unwrap(), policy);
    let decision = RoutingDecision {
        id: "r".into(),
        task_id: "t".into(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        outcome: RoutingOutcome::NoSuitableResource,
        selected_resource_id: None,
        selected_provider: None,
        selected_model: None,
        reasoning_effort: None,
        reasons: vec!["unknown capability".into()],
        alternatives: vec![],
        candidates: vec![],
        policy,
    };
    store.save_routing_decision(&decision).unwrap();
    assert_eq!(
        store.list_routing_decisions(Some("t"), 10).unwrap(),
        vec![decision]
    );
}

#[tokio::test]
async fn auto_agent_routes_once_before_provider_session_and_checkpoints_explanation() {
    let store = RuntimeStore::open_memory().unwrap();
    let events = EventEngine::new(store.clone());
    let agents = AgentRegistry::new(store.clone(), events.clone());
    let mock = Arc::new(MockProvider::default());
    let mut providers = HashMap::<String, Arc<dyn ExecutionProvider>>::new();
    providers.insert("mock".into(), mock.clone());
    let sessions = SessionManager::new(store.clone(), providers);
    let engine = Arc::new(
        TaskEngine::new(
            store.clone(),
            events,
            agents.clone(),
            sessions,
            Duration::from_millis(20),
        )
        .with_economic_routing(),
    );
    let auto: Agent = serde_json::from_value(serde_json::json!({
        "id":"auto-worker","name":"Auto","provider":"auto","model":"auto",
        "function":"SOFTWARE_ENGINEER","seniority":"JUNIOR",
        "intelligence_policy":{"assignment":"AUTO"}
    }))
    .unwrap();
    agents.register(&auto).unwrap();
    let mut resource =
        native_resource_profile("mock-local", "mock", "Mock Local", BillingMode::Local);
    resource.status = "AVAILABLE".into();
    resource.supported_models = vec!["mock-model".into()];
    resource.terms.allowed_use_mode = TermsState::Allowed;
    resource.capabilities = CapabilityProfile {
        coding: Some(CapabilityScore::new(90).unwrap()),
        tool_use: Some(CapabilityScore::new(90).unwrap()),
        ..CapabilityProfile::default()
    };
    store.upsert_intelligence_resource(&resource).unwrap();
    let task: Task = serde_json::from_value(serde_json::json!({
        "id":"auto-task","created_by":"director","objective":"safe task","assigned_to":["auto-worker"]
    }))
    .unwrap();
    engine.ingest(task, None).await.unwrap();
    wait_status(&store, "auto-task", TaskStatus::Completed).await;
    assert_eq!(mock.total_calls(), 1);
    let run = store
        .get_task_run("auto-task", "auto-worker")
        .unwrap()
        .unwrap();
    let checkpoint = run.checkpoint.unwrap();
    assert_eq!(
        checkpoint
            .pointer("/routing_decision/resource_id")
            .and_then(serde_json::Value::as_str),
        Some("mock-local")
    );
    assert_eq!(
        store
            .list_routing_decisions(Some("auto-task"), 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn agent_persistence_round_trips_typed_state() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    assert_eq!(h.agents.get("a").expect("read").expect("agent"), agent("a"));
}

#[test]
fn registry_preserves_runtime_worktree_updates_and_external_refresh_state() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    let mut runtime_update = h.agents.get("a").unwrap().unwrap();
    runtime_update.worktree = Some("C:/tmp/managed-worktree".into());
    h.agents.register(&runtime_update).unwrap();
    assert_eq!(
        h.agents.get("a").unwrap().unwrap().worktree.as_deref(),
        Some("C:/tmp/managed-worktree")
    );
    h.agents
        .transition("a", AgentStatus::Running, Some("task"))
        .unwrap();
    let mut declarative_refresh = agent("a");
    declarative_refresh.name = "Updated Name".into();
    h.agents.register(&declarative_refresh).unwrap();
    let current = h.agents.get("a").unwrap().unwrap();
    assert_eq!(current.status, AgentStatus::Running);
    assert_eq!(current.current_task_id.as_deref(), Some("task"));
    assert_eq!(current.name, "Updated Name");
}

#[test]
fn organizational_fields_and_parent_persist_without_using_display_title_as_truth() {
    use super::organization::{AgentFunction, Department, Seniority};

    let store = RuntimeStore::open_memory().expect("store");
    let events = EventEngine::new(store.clone());
    let registry = AgentRegistry::new(store, events);
    let mut nova = agent("nova");
    nova.schema_version = 2;
    nova.name = "Nova".into();
    nova.role_template = "legacy-label-is-not-authoritative".into();
    nova.seniority = Some(Seniority::Senior);
    nova.function = Some(AgentFunction::BackendEngineering);
    nova.department = Some(Department::Engineering);
    nova.parent_agent_id = Some("director".into());
    registry
        .register(&nova)
        .expect("register organizational agent");
    let persisted = registry.get("nova").unwrap().unwrap();
    assert_eq!(persisted.seniority, Some(Seniority::Senior));
    assert_eq!(persisted.function, Some(AgentFunction::BackendEngineering));
    assert_eq!(persisted.department, Some(Department::Engineering));
    assert_eq!(persisted.parent_agent_id.as_deref(), Some("director"));
    assert_eq!(persisted.display_title(), "Senior Backend Engineer");
}

#[test]
fn new_schema_rejects_meaningless_seniority_function_combination() {
    use super::organization::{AgentFunction, Seniority};

    let store = RuntimeStore::open_memory().expect("store");
    let events = EventEngine::new(store.clone());
    let registry = AgentRegistry::new(store, events);
    let mut invalid = agent("invalid-pm");
    invalid.schema_version = 2;
    invalid.seniority = Some(Seniority::Intern);
    invalid.function = Some(AgentFunction::ProductManager);
    assert!(registry.register(&invalid).is_err());
}

#[test]
fn organization_snapshot_aggregates_identity_relationships_usage_and_metrics() {
    use super::{
        execution_provider::{UsageSnapshot, UsageSource},
        organization::{AgentFunction, Department, Seniority},
        BataiRuntime,
    };

    let root = TempDir::new().expect("temporary project");
    let store = RuntimeStore::open_memory().expect("store");
    let mut providers = HashMap::<String, Arc<dyn ExecutionProvider>>::new();
    providers.insert("mock".into(), Arc::new(MockProvider::default()));
    let runtime = BataiRuntime::with_store(root.path().to_path_buf(), store.clone(), providers)
        .expect("runtime");
    let mut director = agent("director");
    director.schema_version = 2;
    director.seniority = Some(Seniority::Director);
    director.function = Some(AgentFunction::Director);
    director.department = Some(Department::Leadership);
    director.parent_agent_id = None;
    runtime.agents.register(&director).unwrap();
    let mut nova = agent("nova");
    nova.schema_version = 2;
    nova.name = "Nova".into();
    nova.seniority = Some(Seniority::Senior);
    nova.function = Some(AgentFunction::BackendEngineering);
    nova.department = Some(Department::Engineering);
    runtime.agents.register(&nova).unwrap();
    let mut completed = task("TASK-ORG", &["nova"], &[]);
    completed.status = TaskStatus::Completed;
    completed.weight = 5.0;
    store
        .ingest_task(&completed, None, "organization-hash")
        .unwrap();
    let usage = UsageSnapshot {
        source: UsageSource::Subscription,
        input_tokens: Some(10),
        output_tokens: Some(5),
        ..UsageSnapshot::default()
    };
    store
        .mark_task_run(
            "TASK-ORG",
            "nova",
            TaskRunStatus::Completed,
            Some(&serde_json::json!({"usage": usage})),
            Some("mock"),
            Some("real-session-shape"),
        )
        .unwrap();
    let snapshot = runtime.snapshot().expect("organization snapshot");
    let nova = snapshot
        .agents
        .iter()
        .find(|agent| agent.id == "nova")
        .unwrap();
    assert_eq!(nova.title, "Senior Backend Engineer");
    assert_eq!(nova.department, "Engineering");
    assert_eq!(nova.reports_to.as_deref(), Some("director"));
    assert_eq!(nova.usage.total_tokens, Some(15));
    assert_eq!(snapshot.project.progress, 100.0);
    assert_eq!(snapshot.project.usage.total_tokens, Some(15));
    assert!(snapshot.relationships.iter().any(|relationship| {
        relationship.source == "director" && relationship.target == "nova"
    }));
    assert!(snapshot.hierarchy_warnings.is_empty());
}

#[test]
fn task_persistence_round_trips_definition() {
    let store = RuntimeStore::open_memory().expect("store");
    let value = task("t", &["a"], &[]);
    assert_eq!(
        store.ingest_task(&value, None, "hash").expect("ingest"),
        IngestDisposition::Created
    );
    assert_eq!(store.get_task("t").expect("read").expect("task"), value);
}

#[test]
fn event_is_persisted_before_it_is_broadcast() {
    let store = RuntimeStore::open_memory().expect("store");
    let events = EventEngine::new(store.clone());
    let mut subscriber = events.subscribe();
    let event = events
        .publish(
            EventType::TaskCreated,
            "director",
            None,
            Some("t".into()),
            serde_json::json!({}),
        )
        .expect("publish");
    let received = subscriber.try_recv().expect("broadcast");
    assert_eq!(received.id, event.id);
    assert_eq!(store.list_events(10).expect("events")[0].id, event.id);
}

#[test]
fn persisted_state_survives_database_restart() {
    let directory = TempDir::new().expect("temp");
    let path = directory.path().join("runtime.sqlite");
    {
        let store = RuntimeStore::open(&path).expect("open");
        store.upsert_agent(&agent("a")).expect("agent");
        store
            .ingest_task(&task("t", &["a"], &[]), None, "h")
            .expect("task");
    }
    let reopened = RuntimeStore::open(&path).expect("reopen");
    assert!(reopened.get_agent("a").expect("agent").is_some());
    assert!(reopened.get_task("t").expect("task").is_some());
}

#[test]
fn invalid_agent_transition_is_rejected() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    let error = h
        .agents
        .transition("a", AgentStatus::Completed, None)
        .expect_err("invalid transition");
    assert!(matches!(error, RuntimeError::InvalidAgentTransition { .. }));
}

#[tokio::test]
async fn task_scoped_agent_terminates_after_its_last_task() {
    use super::organization::{AgentFunction, AgentLifecycle, Department, Seniority};

    let h = harness(RuntimeStore::open_memory().expect("store"));
    let mut temporary = agent("temporary");
    temporary.schema_version = 2;
    temporary.seniority = Some(Seniority::Senior);
    temporary.function = Some(AgentFunction::SoftwareEngineer);
    temporary.department = Some(Department::Engineering);
    temporary.lifecycle = AgentLifecycle::TaskScoped;
    h.agents.register(&temporary).expect("temporary agent");
    h.engine
        .ingest(task("one-shot", &["temporary"], &[]), None)
        .await
        .expect("task");
    assert_eq!(
        h.agents
            .get("temporary")
            .expect("read")
            .expect("agent")
            .status,
        AgentStatus::Terminated
    );
}

#[tokio::test]
async fn task_watcher_initial_scan_ingests_existing_files() {
    let directory = TempDir::new().expect("temp");
    let tasks_dir = directory.path().join(".batai/tasks");
    fs::create_dir_all(&tasks_dir).expect("tasks");
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    fs::write(
        tasks_dir.join("t.json"),
        serde_json::to_vec(&task("t", &["a"], &[])).expect("json"),
    )
    .expect("task file");
    let mut watcher = TaskWatcher::start(
        &tasks_dir,
        h.engine.clone(),
        h.events.clone(),
        Duration::from_millis(20),
    )
    .await
    .expect("watcher");
    assert_eq!(
        h.store.get_task("t").expect("read").expect("task").status,
        TaskStatus::Completed
    );
    watcher.shutdown().expect("shutdown");
}

#[tokio::test]
async fn watcher_duplicate_events_do_not_duplicate_execution() {
    let directory = TempDir::new().expect("temp");
    let tasks_dir = directory.path().join("tasks");
    fs::create_dir_all(&tasks_dir).expect("tasks");
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    let mut watcher = TaskWatcher::start(
        &tasks_dir,
        h.engine.clone(),
        h.events.clone(),
        Duration::from_millis(30),
    )
    .await
    .expect("watcher");
    let bytes = serde_json::to_vec(&task("t", &["a"], &[])).expect("json");
    for _ in 0..3 {
        fs::write(tasks_dir.join("t.json"), &bytes).expect("write");
    }
    wait_status(&h.store, "t", TaskStatus::Completed).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(h.mock.calls_for("a"), 1);
    watcher.shutdown().expect("shutdown");
}

#[tokio::test]
async fn invalid_partial_task_file_does_not_stop_watcher() {
    let directory = TempDir::new().expect("temp");
    let tasks_dir = directory.path().join("tasks");
    fs::create_dir_all(&tasks_dir).expect("tasks");
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    fs::write(tasks_dir.join("partial.json"), "{\"id\":").expect("partial");
    let mut watcher = TaskWatcher::start(
        &tasks_dir,
        h.engine.clone(),
        h.events.clone(),
        Duration::from_millis(20),
    )
    .await
    .expect("watcher");
    fs::write(
        tasks_dir.join("valid.json"),
        serde_json::to_vec(&task("valid", &["a"], &[])).expect("json"),
    )
    .expect("valid");
    wait_status(&h.store, "valid", TaskStatus::Completed).await;
    assert!(h
        .store
        .list_events(20)
        .expect("events")
        .iter()
        .any(|event| event.event_type == EventType::TaskFailed));
    watcher.shutdown().expect("shutdown");
}

#[tokio::test]
async fn unsatisfied_dependency_blocks_task() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    h.engine
        .ingest(task("b", &["a"], &["a-task"]), None)
        .await
        .expect("ingest");
    assert_eq!(
        h.store.get_task("b").expect("read").expect("task").status,
        TaskStatus::Blocked
    );
}

#[tokio::test]
async fn dependency_completion_starts_downstream_task() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    h.engine
        .ingest(task("b", &["a"], &["up"]), None)
        .await
        .expect("downstream");
    h.engine
        .ingest(task("up", &["a"], &[]), None)
        .await
        .expect("upstream");
    wait_status(&h.store, "b", TaskStatus::Completed).await;
}

#[tokio::test]
async fn dependency_dag_completes_each_node_once() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a", "b", "c", "d"]);
    for value in [
        task("e", &["a"], &["c", "d"]),
        task("c", &["c"], &["b"]),
        task("d", &["d"], &["b"]),
        task("b", &["b"], &["root"]),
    ] {
        h.engine.ingest(value, None).await.expect("node");
    }
    h.engine
        .ingest(task("root", &["a"], &[]), None)
        .await
        .expect("root");
    wait_status(&h.store, "e", TaskStatus::Completed).await;
    assert_eq!(h.mock.total_calls(), 5);
}

#[tokio::test]
async fn circular_dependency_is_detected_without_evaluation_loop() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    h.engine
        .ingest(task("a-task", &["a"], &["b-task"]), None)
        .await
        .expect("first");
    let error = h
        .engine
        .ingest(task("b-task", &["a"], &["a-task"]), None)
        .await
        .expect_err("cycle");
    assert!(matches!(error, RuntimeError::DependencyCycle(_)));
    assert_eq!(h.mock.total_calls(), 0);
}

#[tokio::test]
async fn same_agent_tasks_are_serialized_before_running_state() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    h.mock
        .push_outcome("a", MockOutcome::Delay(Duration::from_millis(100)));
    let first = {
        let engine = h.engine.clone();
        tokio::spawn(async move { engine.ingest(task("one", &["a"], &[]), None).await })
    };
    tokio::time::sleep(Duration::from_millis(15)).await;
    h.engine
        .ingest(task("two", &["a"], &[]), None)
        .await
        .expect("second");
    assert_eq!(
        h.store.get_task("two").expect("read").expect("task").status,
        TaskStatus::Ready
    );
    first.await.expect("join").expect("first");
    wait_status(&h.store, "two", TaskStatus::Completed).await;
}

#[tokio::test]
async fn different_agents_execute_in_parallel() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a", "b"]);
    h.mock
        .push_outcome("a", MockOutcome::Delay(Duration::from_millis(120)));
    h.mock
        .push_outcome("b", MockOutcome::Delay(Duration::from_millis(120)));
    let started = Instant::now();
    let one = {
        let e = h.engine.clone();
        tokio::spawn(async move { e.ingest(task("one", &["a"], &[]), None).await })
    };
    let two = {
        let e = h.engine.clone();
        tokio::spawn(async move { e.ingest(task("two", &["b"], &[]), None).await })
    };
    one.await.expect("join").expect("one");
    two.await.expect("join").expect("two");
    assert!(started.elapsed() < Duration::from_millis(220));
}

#[tokio::test]
async fn director_review_gate_precedes_completion() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    let mut value = task("review", &["a"], &[]);
    value.execution.requires_director_review = true;
    h.engine.ingest(value, None).await.expect("ingest");
    assert_eq!(
        h.store
            .get_task("review")
            .expect("read")
            .expect("task")
            .status,
        TaskStatus::Review
    );
    assert!(h
        .store
        .list_events(20)
        .expect("events")
        .iter()
        .any(|event| event.event_type == EventType::ReviewRequired));
    h.engine
        .approve_review("review", "director")
        .await
        .expect("approve");
    assert_eq!(
        h.store
            .get_task("review")
            .expect("read")
            .expect("task")
            .status,
        TaskStatus::Completed
    );
}

#[tokio::test]
async fn resource_rate_limit_parks_only_affected_agent() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a", "b"]);
    h.mock.push_outcome("b", MockOutcome::RateLimited(None));
    h.engine
        .ingest(task("multi", &["a", "b"], &[]), None)
        .await
        .expect("ingest");
    assert_eq!(
        h.store
            .get_resource("b")
            .expect("resource")
            .expect("state")
            .status,
        ResourceStatus::RateLimited
    );
    assert_eq!(
        h.store
            .get_resource("a")
            .expect("resource")
            .expect("successful agent resource")
            .status,
        ResourceStatus::Available
    );
}

#[tokio::test]
async fn waiting_resource_state_is_persisted() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    h.mock.push_outcome("a", MockOutcome::AuthRequired);
    h.engine
        .ingest(task("t", &["a"], &[]), None)
        .await
        .expect("ingest");
    assert_eq!(
        h.store.get_task("t").expect("task").expect("task").status,
        TaskStatus::WaitingResource
    );
    assert_eq!(
        h.store
            .get_task_run("t", "a")
            .expect("run")
            .expect("run")
            .status,
        TaskRunStatus::WaitingResource
    );
}

#[tokio::test]
async fn durable_resume_scheduler_recovers_available_resource() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    h.mock.push_outcome(
        "a",
        MockOutcome::RateLimited(Some(chrono::Utc::now().to_rfc3339())),
    );
    h.engine
        .ingest(task("t", &["a"], &[]), None)
        .await
        .expect("ingest");
    h.mock.set_usage(
        "a",
        UsageState {
            status: ResourceStatus::Available,
            reset_at: None,
            details: serde_json::Value::Null,
        },
    );
    let mut scheduler = DurableScheduler::start(
        h.store.clone(),
        h.events.clone(),
        h.agents.clone(),
        h.sessions.clone(),
        h.engine.clone(),
        Duration::from_millis(10),
        Duration::from_millis(50),
    );
    wait_status(&h.store, "t", TaskStatus::Completed).await;
    scheduler.shutdown().await;
}

#[test]
fn scheduler_reconciliation_requeues_interrupted_job() {
    let store = RuntimeStore::open_memory().expect("store");
    let job = store
        .schedule_resource_recheck(
            "a",
            &chrono::Utc::now().to_rfc3339(),
            &serde_json::json!({}),
        )
        .expect("job");
    store
        .set_job_status(&job.id, SchedulerJobStatus::Running, None)
        .expect("running");
    store.reconcile_interrupted().expect("reconcile");
    assert_eq!(
        store.get_job(&job.id).expect("read").expect("job").status,
        SchedulerJobStatus::Pending
    );
}

#[tokio::test]
async fn session_is_persisted_after_creation() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    let value = agent("a");
    h.sessions.ensure(&value).await.expect("session");
    assert!(h.store.get_session("a").expect("read").is_some());
}

#[tokio::test]
async fn compatible_persisted_session_is_resumed() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    let value = agent("a");
    h.sessions.ensure(&value).await.expect("create");
    h.sessions.ensure(&value).await.expect("resume");
    assert_eq!(h.mock.resumed_count(), 1);
}

#[tokio::test]
async fn provider_or_model_change_invalidates_session() {
    let store = RuntimeStore::open_memory().expect("store");
    let mock = Arc::new(MockProvider::default());
    let mut providers = HashMap::<String, Arc<dyn ExecutionProvider>>::new();
    providers.insert("mock".into(), mock.clone());
    providers.insert("alternate".into(), mock);
    let sessions = SessionManager::new(store, providers);
    let mut value = agent("a");
    let first = sessions.ensure(&value).await.expect("first");
    value.provider = "alternate".into();
    let second = sessions.ensure(&value).await.expect("second");
    assert_ne!(first.provider_session_id, second.provider_session_id);
    value.model = "mock-high".into();
    let third = sessions.ensure(&value).await.expect("third");
    assert_ne!(second.provider_session_id, third.provider_session_id);
}

#[tokio::test]
async fn worktree_or_config_change_invalidates_session() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    let mut value = agent("a");
    let first = h.sessions.ensure(&value).await.expect("first");
    value.worktree = Some("worktrees/a".into());
    let second = h.sessions.ensure(&value).await.expect("second");
    assert_ne!(first.provider_session_id, second.provider_session_id);
    value
        .extra
        .insert("max_turns".into(), serde_json::json!(20));
    let third = h.sessions.ensure(&value).await.expect("third");
    assert_ne!(second.provider_session_id, third.provider_session_id);
}

#[tokio::test]
async fn multi_agent_partial_success_is_checkpointed() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a", "b"]);
    h.mock.push_outcome("b", MockOutcome::RateLimited(None));
    h.engine
        .ingest(task("multi", &["a", "b"], &[]), None)
        .await
        .expect("ingest");
    assert_eq!(
        h.store
            .get_task_run("multi", "a")
            .expect("run")
            .expect("run")
            .status,
        TaskRunStatus::Completed
    );
    assert_eq!(
        h.store
            .get_task_run("multi", "b")
            .expect("run")
            .expect("run")
            .status,
        TaskRunStatus::WaitingResource
    );
}

#[tokio::test]
async fn retry_does_not_rerun_successful_agent_checkpoint() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a", "b"]);
    h.mock.push_outcome("b", MockOutcome::RateLimited(None));
    h.engine
        .ingest(task("multi", &["a", "b"], &[]), None)
        .await
        .expect("ingest");
    h.engine.resume_agent("b").await.expect("resume");
    wait_status(&h.store, "multi", TaskStatus::Completed).await;
    assert_eq!(h.mock.calls_for("a"), 1);
    assert_eq!(h.mock.calls_for("b"), 2);
}

#[tokio::test]
async fn restart_preserves_completed_task_run_checkpoint() {
    let directory = TempDir::new().expect("temp");
    let path = directory.path().join("runtime.sqlite");
    let store = RuntimeStore::open(&path).expect("store");
    let first = harness(store.clone());
    register(&first, &["a", "b"]);
    first.mock.push_outcome("b", MockOutcome::RateLimited(None));
    first
        .engine
        .ingest(task("multi", &["a", "b"], &[]), None)
        .await
        .expect("first run");
    drop(first);
    let reopened = RuntimeStore::open(&path).expect("reopen");
    reopened.reconcile_interrupted().expect("reconcile");
    let second = harness(reopened);
    register(&second, &["a", "b"]);
    second.engine.resume_agent("b").await.expect("resume");
    wait_status(&second.store, "multi", TaskStatus::Completed).await;
    assert_eq!(second.mock.calls_for("a"), 0);
    assert_eq!(second.mock.calls_for("b"), 1);
}

#[tokio::test]
async fn clean_shutdown_stops_watcher_and_scheduler() {
    let directory = TempDir::new().expect("temp");
    let tasks_dir = directory.path().join("tasks");
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    let mut watcher = TaskWatcher::start(
        &tasks_dir,
        h.engine.clone(),
        h.events.clone(),
        Duration::from_millis(10),
    )
    .await
    .expect("watcher");
    let mut scheduler = DurableScheduler::start(
        h.store.clone(),
        h.events.clone(),
        h.agents.clone(),
        h.sessions.clone(),
        h.engine.clone(),
        Duration::from_secs(5),
        Duration::from_secs(5),
    );
    watcher.shutdown().expect("watcher shutdown");
    scheduler.shutdown().await;
}

#[test]
fn duplicate_resource_jobs_are_coalesced_per_agent() {
    let store = RuntimeStore::open_memory().expect("store");
    let first = store
        .schedule_resource_recheck("a", "2026-01-01T00:00:00Z", &serde_json::json!({"n":1}))
        .expect("first");
    let second = store
        .schedule_resource_recheck("a", "2026-01-02T00:00:00Z", &serde_json::json!({"n":2}))
        .expect("second");
    assert_eq!(first.id, second.id);
    assert_eq!(second.run_at, "2026-01-02T00:00:00Z");
}

#[tokio::test]
async fn interrupted_running_task_requires_review_instead_of_duplicate_execution() {
    let store = RuntimeStore::open_memory().expect("store");
    store
        .ingest_task(&task("t", &["a"], &[]), None, "hash")
        .expect("ingest");
    store
        .set_task_status("t", TaskStatus::Running)
        .expect("running");
    store.reconcile_interrupted().expect("reconcile store");
    let h = harness(store);
    register(&h, &["a"]);
    h.engine.reconcile().await.expect("runtime reconcile");
    assert_eq!(
        h.store.get_task("t").expect("read").expect("task").status,
        TaskStatus::Review
    );
}

#[tokio::test]
async fn cancellation_stops_a_running_turn_and_is_durable() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    h.mock
        .push_outcome("a", MockOutcome::Delay(Duration::from_secs(2)));
    let engine = h.engine.clone();
    let execution =
        tokio::spawn(async move { engine.ingest(task("cancel-me", &["a"], &[]), None).await });
    wait_status(&h.store, "cancel-me", TaskStatus::Running).await;
    h.engine.cancel("cancel-me", "user").await.expect("cancel");
    execution.await.expect("worker").expect("dispatch result");
    assert_eq!(
        h.store.get_task("cancel-me").unwrap().unwrap().status,
        TaskStatus::Cancelled
    );
    assert_eq!(
        h.store
            .get_task_run("cancel-me", "a")
            .unwrap()
            .unwrap()
            .status,
        TaskRunStatus::Cancelled
    );
    assert!(h
        .store
        .list_events(50)
        .unwrap()
        .iter()
        .any(|event| event.event_type == EventType::TaskCancellationRequested));
}

#[tokio::test]
async fn provider_crash_is_not_retried_when_execution_may_have_mutated_files() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    h.mock
        .push_outcome("a", MockOutcome::Crash("fixture process exited".into()));
    h.engine
        .ingest(task("crash", &["a"], &[]), None)
        .await
        .expect("ingest");
    assert_eq!(
        h.store.get_task("crash").unwrap().unwrap().status,
        TaskStatus::Review
    );
    let run = h.store.get_task_run("crash", "a").unwrap().unwrap();
    assert_eq!(run.status, TaskRunStatus::UnknownAfterCrash);
    assert_eq!(
        run.checkpoint.unwrap()["execution_state"],
        "unknown_after_crash"
    );
    assert_eq!(h.mock.calls_for("a"), 1);
}

#[tokio::test]
async fn normalized_usage_is_persisted_after_success() {
    let h = harness(RuntimeStore::open_memory().expect("store"));
    register(&h, &["a"]);
    h.engine
        .ingest(task("usage", &["a"], &[]), None)
        .await
        .expect("ingest");
    let resource = h.store.get_resource("a").unwrap().expect("resource state");
    assert_eq!(resource.status, ResourceStatus::Available);
    assert_eq!(resource.details["source"], "unknown");
    assert!(h
        .store
        .list_events(50)
        .unwrap()
        .iter()
        .any(|event| event.event_type == EventType::UsageUpdated));
}

#[tokio::test]
async fn real_codex_turn_mutates_only_a_managed_disposable_worktree() {
    if std::env::var("BATAI_REAL_PROVIDER_E2E").as_deref() != Ok("codex") {
        eprintln!("skipped: set BATAI_REAL_PROVIDER_E2E=codex to run authenticated inference");
        return;
    }

    let batai_repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Batai repository root");
    let batai_status_before = checked_git(
        batai_repository,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    );

    let disposable = tempfile::Builder::new()
        .prefix("batai-real-codex-e2e-")
        .tempdir()
        .expect("create disposable repository");
    checked_git(disposable.path(), &["init", "--initial-branch=main"]);
    checked_git(disposable.path(), &["config", "user.name", "Batai E2E"]);
    checked_git(
        disposable.path(),
        &["config", "user.email", "batai-e2e@example.invalid"],
    );
    fs::write(disposable.path().join("README.md"), "# Disposable E2E\n")
        .expect("seed disposable repository");
    checked_git(disposable.path(), &["add", "README.md"]);
    checked_git(disposable.path(), &["commit", "-m", "seed"]);

    let store = RuntimeStore::open_memory().expect("open runtime store");
    let events = EventEngine::new(store.clone());
    let agents = AgentRegistry::new(store.clone(), events.clone());
    let mut providers = HashMap::<String, Arc<dyn ExecutionProvider>>::new();
    providers.insert("codex".into(), Arc::new(CodexAppServerProvider::default()));
    let sessions = SessionManager::new(store.clone(), providers);
    let worktrees = WorktreeManager::discover(disposable.path()).expect("discover worktrees");
    let engine = Arc::new(
        TaskEngine::new(
            store.clone(),
            events,
            agents.clone(),
            sessions,
            Duration::from_secs(30),
        )
        .with_worktrees(worktrees.clone()),
    );
    let mut coding_agent = agent("codex-e2e");
    coding_agent.parent_agent_id = None;
    coding_agent.provider = "codex".into();
    coding_agent.model = "auto".into();
    coding_agent.auth_mode = "subscription".into();
    agents
        .register(&coding_agent)
        .expect("register Codex agent");

    let mut coding_task = task("real-codex-mutation", &["codex-e2e"], &[]);
    coding_task.created_by = "e2e-test".into();
    coding_task.objective = "Create hello.txt in the current working directory with exactly the text `Batai Codex E2E` followed by one newline. Do not modify any other file.".into();
    let first_disposition = engine
        .ingest(coding_task.clone(), None)
        .await
        .expect("execute authenticated Codex task");

    let binding = worktrees
        .ensure("codex-e2e", "real-codex-mutation")
        .expect("load managed worktree binding");
    let hello_path = binding.path.join("hello.txt");
    let hello = fs::read_to_string(&hello_path).expect("Codex created hello.txt");
    let changed_files = worktrees.status(&binding).expect("read worktree status");
    let completed_task = store
        .get_task("real-codex-mutation")
        .expect("read task")
        .expect("persisted task");
    let first_run = store
        .get_task_run("real-codex-mutation", "codex-e2e")
        .expect("read task run")
        .expect("persisted task run");
    let session = store
        .get_session("codex-e2e")
        .expect("read session")
        .expect("real provider session");
    let first_turn_id = first_run
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.get("turn_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);

    let second_disposition = engine
        .ingest(coding_task, None)
        .await
        .expect("deduplicate completed task");
    let second_run = store
        .get_task_run("real-codex-mutation", "codex-e2e")
        .expect("read deduplicated task run")
        .expect("task run remains available");

    drop(engine);
    tokio::time::sleep(Duration::from_millis(250)).await;
    fs::remove_file(&hello_path).expect("clean disposable worktree mutation");
    worktrees
        .remove(&binding)
        .expect("remove disposable worktree");
    let batai_status_after = checked_git(
        batai_repository,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    );

    assert_eq!(first_disposition, IngestDisposition::Created);
    assert_eq!(second_disposition, IngestDisposition::Duplicate);
    assert_eq!(hello, "Batai Codex E2E\n");
    assert_eq!(changed_files, vec!["?? hello.txt"]);
    assert!(!disposable.path().join("hello.txt").exists());
    assert!(matches!(
        completed_task.status,
        TaskStatus::Completed | TaskStatus::Review
    ));
    assert_eq!(first_run.status, TaskRunStatus::Completed);
    assert_eq!(first_run.provider.as_deref(), Some("codex"));
    assert_eq!(
        first_run.provider_session_id.as_deref(),
        Some(session.provider_session_id.as_str())
    );
    assert!(first_turn_id.as_deref().is_some_and(|id| !id.is_empty()));
    assert_eq!(
        first_run
            .checkpoint
            .as_ref()
            .and_then(|checkpoint| checkpoint.get("changed_files"))
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(second_run.attempt, 1);
    assert_eq!(
        second_run
            .checkpoint
            .as_ref()
            .and_then(|checkpoint| checkpoint.get("turn_id"))
            .and_then(serde_json::Value::as_str),
        first_turn_id.as_deref()
    );
    assert_eq!(batai_status_after, batai_status_before);
}
