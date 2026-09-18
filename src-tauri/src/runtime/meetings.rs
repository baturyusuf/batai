//! Bounded, persisted multi-agent coordination.
//!
//! Meetings are deliberately not chat rooms. The backend enforces participant,
//! round and token ceilings and stores only user-visible contributions and
//! observable execution metadata.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs::{self, File},
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use chrono::Utc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{watch, Semaphore};
use tokio::task::JoinHandle;

use super::{
    agents::AgentRegistry,
    context::{
        ContextBudget, ContextBuildRequest, ContextBuildStatus, ContextBuilder,
        ContextOperationType, ContextPackage, ContextPackageRecord,
    },
    economic::{BillingMode, QuotaAwareEconomicRouter, RoutingOutcome, TaskRequirements, TaskRisk},
    errors::{Result, RuntimeError},
    events::EventEngine,
    execution_provider::{ProviderExecutionResult, ProviderFailure, UsageSnapshot, UsageSource},
    gates::{ExecutionGate, ExecutionGateStatus, ExecutionGateType},
    governance::{
        Actor, AuthorityScope, DecisionStatus, GovernanceService, MutationDisposition,
        MutationRequest, OrganizationMutation, ProtectedOperation,
    },
    organization::{AgentFunction, AuthorityRole, CapabilityProfile, ModelAssignment},
    sessions::SessionManager,
    store::RuntimeStore,
    tasks::TaskEngine,
    types::{Agent, AgentStatus, EventType, Task, TaskExecution, TaskStatus, TaskSuccessAction},
};

const DEFAULT_MAX_PARTICIPANTS: usize = 5;
const DEFAULT_MAX_ROUNDS: u8 = 2;
const DEFAULT_RESPONSE_TOKENS: u64 = 800;
const DEFAULT_TOTAL_TOKENS: u64 = 8_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MeetingStatus {
    Planned,
    Ready,
    Running,
    Paused,
    Blocked,
    Completed,
    Cancelled,
    Failed,
}

impl MeetingStatus {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MeetingTrigger {
    #[default]
    Manual,
    Director,
    ReviewEscalation,
    ArchitectureDisagreement,
    BlockedTask,
    Meeting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MeetingTurnKind {
    Position,
    Response,
    Closure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MeetingTurnStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    UnknownAfterCrash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgreementState {
    Unanimous,
    Majority,
    Unresolved,
    NotObserved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct MeetingPolicy {
    pub meetings_enabled: bool,
    pub max_participants: usize,
    pub max_rounds: u8,
    pub max_tokens: u64,
    pub max_response_tokens: u64,
    pub allow_payg_for_meetings: bool,
    pub director_can_create: bool,
    pub lead_can_create: bool,
    pub context_budget: ContextBudget,
}

impl Default for MeetingPolicy {
    fn default() -> Self {
        Self {
            meetings_enabled: true,
            max_participants: DEFAULT_MAX_PARTICIPANTS,
            max_rounds: DEFAULT_MAX_ROUNDS,
            max_tokens: DEFAULT_TOTAL_TOKENS,
            max_response_tokens: DEFAULT_RESPONSE_TOKENS,
            allow_payg_for_meetings: false,
            director_can_create: true,
            lead_can_create: false,
            context_budget: ContextBudget::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub struct CreateMeetingRequest {
    pub title: String,
    pub agenda: Vec<String>,
    pub objective: String,
    pub participants: Vec<String>,
    pub required_roles: Vec<AgentFunction>,
    pub required_capabilities: BTreeMap<String, u8>,
    pub closure_owner: Option<String>,
    pub max_rounds: Option<u8>,
    pub per_response_token_limit: Option<u64>,
    pub total_meeting_token_budget: Option<u64>,
    pub linked_task: Option<String>,
    pub linked_decision: Option<String>,
    pub trigger: MeetingTrigger,
    pub explicit_files: Vec<String>,
    pub context_budget: Option<ContextBudget>,
}

impl Default for CreateMeetingRequest {
    fn default() -> Self {
        Self {
            title: String::new(),
            agenda: Vec::new(),
            objective: String::new(),
            participants: Vec::new(),
            required_roles: Vec::new(),
            required_capabilities: BTreeMap::new(),
            closure_owner: None,
            max_rounds: None,
            per_response_token_limit: None,
            total_meeting_token_budget: None,
            linked_task: None,
            linked_decision: None,
            trigger: MeetingTrigger::Manual,
            explicit_files: Vec::new(),
            context_budget: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingParticipant {
    pub agent_id: String,
    pub name: String,
    pub title: String,
    pub function: AgentFunction,
    pub provider: String,
    pub model: String,
    pub selected_resource_id: Option<String>,
    pub routing_decision_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct MeetingContribution {
    pub position: String,
    pub agreements: Vec<String>,
    pub disagreements: Vec<String>,
    pub objections: Vec<String>,
    #[serde(alias = "open_questions")]
    pub open_questions: Vec<String>,
    #[serde(alias = "action_items")]
    pub action_items: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingTurn {
    pub id: String,
    pub meeting_id: String,
    pub round: u8,
    pub participant_id: String,
    pub kind: MeetingTurnKind,
    pub status: MeetingTurnStatus,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub provider_session_id: Option<String>,
    pub provider_turn_id: Option<String>,
    pub contribution: Option<MeetingContribution>,
    pub usage: UsageSnapshot,
    pub charged_tokens: u64,
    pub failure: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    #[serde(default)]
    pub context_id: Option<String>,
    #[serde(default)]
    pub context_fingerprint: Option<String>,
    #[serde(default)]
    pub estimated_context_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct MeetingUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub known_cost: Option<f64>,
    pub currency: Option<String>,
    pub by_source: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingActionItem {
    pub id: String,
    pub description: String,
    pub suggested_owner: Option<String>,
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct MeetingOutcome {
    pub summary: String,
    pub agreements: Vec<String>,
    pub disagreements: Vec<String>,
    pub decisions: Vec<String>,
    pub action_items: Vec<MeetingActionItem>,
    pub unresolved_questions: Vec<String>,
    pub recommended_next_step: Option<String>,
    pub agreement_state: Option<AgreementState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub agenda: Vec<String>,
    pub objective: String,
    pub organizer: String,
    pub participants: Vec<MeetingParticipant>,
    pub required_roles: Vec<AgentFunction>,
    pub required_capabilities: BTreeMap<String, u8>,
    pub closure_owner: String,
    pub max_rounds: u8,
    pub per_response_token_limit: u64,
    pub total_meeting_token_budget: u64,
    pub status: MeetingStatus,
    pub current_round: u8,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub linked_task: Option<String>,
    pub linked_decision: Option<String>,
    pub trigger: MeetingTrigger,
    pub result: Option<MeetingOutcome>,
    pub usage: MeetingUsage,
    pub recovery_note: Option<String>,
    #[serde(default)]
    pub explicit_files: Vec<String>,
    #[serde(default)]
    pub context_budget: ContextBudget,
    #[serde(default)]
    pub context_packages: Vec<ContextPackageRecord>,
    #[serde(default)]
    pub execution_gates: Vec<ExecutionGate>,
    #[serde(default)]
    pub payg_rejected: bool,
}

#[derive(Clone)]
pub struct MeetingEngine {
    root: PathBuf,
    store: RuntimeStore,
    events: EventEngine,
    agents: AgentRegistry,
    sessions: SessionManager,
    tasks: Arc<TaskEngine>,
    governance: GovernanceService,
    context: ContextBuilder,
    running: Arc<Mutex<HashSet<String>>>,
    payg_decision_claims: Arc<Mutex<HashSet<String>>>,
    provider_lanes: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
    cancellations: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
    event_listener: Arc<Mutex<Option<JoinHandle<()>>>>,
    event_listener_shutdown: watch::Sender<bool>,
}

impl MeetingEngine {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: PathBuf,
        store: RuntimeStore,
        events: EventEngine,
        agents: AgentRegistry,
        sessions: SessionManager,
        tasks: Arc<TaskEngine>,
        governance: GovernanceService,
        context: ContextBuilder,
    ) -> Self {
        let (event_listener_shutdown, _) = watch::channel(false);
        Self {
            root,
            store,
            events,
            agents,
            sessions,
            tasks,
            governance,
            context,
            running: Default::default(),
            payg_decision_claims: Default::default(),
            provider_lanes: Default::default(),
            cancellations: Default::default(),
            event_listener: Default::default(),
            event_listener_shutdown,
        }
    }

    pub fn get(&self, id: &str) -> Result<Option<Meeting>> {
        self.store.get_meeting(id)
    }

    pub fn list(&self) -> Result<Vec<Meeting>> {
        self.store.list_meetings()
    }

    pub fn turns(&self, meeting_id: &str) -> Result<Vec<MeetingTurn>> {
        self.store.list_meeting_turns(meeting_id)
    }

    pub fn has_active(&self) -> Result<bool> {
        Ok(self.list()?.iter().any(|meeting| {
            matches!(
                meeting.status,
                MeetingStatus::Planned | MeetingStatus::Ready | MeetingStatus::Running
            ) || (meeting.status == MeetingStatus::Blocked
                && meeting
                    .execution_gates
                    .iter()
                    .any(|gate| gate.status == ExecutionGateStatus::Waiting))
        }))
    }

    pub async fn shutdown(&self) -> Result<()> {
        let _ = self.event_listener_shutdown.send(true);
        if let Some(listener) = self
            .event_listener
            .lock()
            .map_err(|_| RuntimeError::Lock("meeting event listener"))?
            .take()
        {
            listener.abort();
        }
        let active = self
            .list()?
            .into_iter()
            .filter(|meeting| meeting.status == MeetingStatus::Running)
            .map(|meeting| meeting.id)
            .collect::<Vec<_>>();
        for id in active {
            let _ = self
                .cancel(
                    Actor {
                        id: "god".into(),
                        scope: AuthorityScope::Global,
                    },
                    &id,
                )
                .await;
        }
        Ok(())
    }

    pub fn start_event_listener(self: &Arc<Self>) -> Result<()> {
        let mut listener = self
            .event_listener
            .lock()
            .map_err(|_| RuntimeError::Lock("meeting event listener"))?;
        if listener.is_some() {
            return Ok(());
        }
        let mut events = self.events.subscribe();
        let mut shutdown = self.event_listener_shutdown.subscribe();
        let engine = Arc::clone(self);
        *listener = Some(tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() { break; }
                    }
                    event = events.recv() => {
                        match event {
                            Ok(event) if event.event_type == EventType::DecisionResolved => {
                                if let Some(decision_id) = event.target.as_deref() {
                                    let _ = engine.handle_decision_resolution(decision_id);
                                }
                            }
                            Ok(event) if event.event_type == EventType::PolicyChanged => {
                                let _ = engine.supersede_stale_policy_gates();
                            }
                            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
        }));
        Ok(())
    }

    fn supersede_stale_policy_gates(&self) -> Result<()> {
        let current_revision = self.governance.snapshot()?.revision;
        let decision_ids = self
            .store
            .list_meetings()?
            .into_iter()
            .flat_map(|meeting| meeting.execution_gates)
            .filter(|gate| {
                gate.status == ExecutionGateStatus::Waiting
                    && gate.expected_policy_revision != current_revision
            })
            .filter_map(|gate| gate.decision_id)
            .collect::<Vec<_>>();
        for decision_id in decision_ids {
            self.governance.supersede_open_decision(
                &decision_id,
                "project policy changed while the exact execution gate was waiting".into(),
            )?;
        }
        Ok(())
    }

    pub fn create(
        self: &Arc<Self>,
        actor: Actor,
        request: CreateMeetingRequest,
    ) -> Result<Meeting> {
        let policy = self.governance.snapshot()?.policy.meetings;
        self.authorize_create(&actor, &policy)?;
        validate_request(&request, &policy)?;
        if request.trigger == MeetingTrigger::Meeting {
            return Err(RuntimeError::Governance(
                "meeting-triggered meetings are disabled to prevent recursive coordination".into(),
            ));
        }
        if let Some(task_id) = request.linked_task.as_deref() {
            if self.store.get_task(task_id)?.is_none() {
                return Err(RuntimeError::TaskNotFound(task_id.into()));
            }
        }
        let participants = self.select_participants(&request, &policy)?;
        let closure_owner = match request.closure_owner.clone() {
            Some(owner) => owner,
            None => self
                .agents
                .list()?
                .into_iter()
                .find(|agent| {
                    agent.authority == AuthorityRole::Director
                        || agent.with_backfilled_organization().function
                            == Some(AgentFunction::Director)
                })
                .map(|agent| agent.id)
                .ok_or_else(|| {
                    RuntimeError::Governance(
                        "meeting requires a Director closure owner or an explicit closure owner"
                            .into(),
                    )
                })?,
        };
        self.validate_closure_owner(&closure_owner, &participants)?;
        let per_response_token_limit = request
            .per_response_token_limit
            .unwrap_or(DEFAULT_RESPONSE_TOKENS);
        let total_meeting_token_budget = request
            .total_meeting_token_budget
            .unwrap_or(DEFAULT_TOTAL_TOKENS);
        let minimum_first_round =
            per_response_token_limit.saturating_mul(participants.len() as u64);
        if total_meeting_token_budget < minimum_first_round {
            return Err(RuntimeError::Governance(format!(
                "meeting token budget must cover one bounded response per participant ({minimum_first_round})"
            )));
        }
        let now = Utc::now().to_rfc3339();
        let meeting = Meeting {
            id: format!("MTG-{}", uuid::Uuid::new_v4()),
            title: request.title.trim().to_owned(),
            agenda: request.agenda,
            objective: request.objective.trim().to_owned(),
            organizer: actor.id.clone(),
            participants,
            required_roles: request.required_roles,
            required_capabilities: request.required_capabilities,
            closure_owner,
            max_rounds: request.max_rounds.unwrap_or(DEFAULT_MAX_ROUNDS),
            per_response_token_limit,
            total_meeting_token_budget,
            status: MeetingStatus::Ready,
            current_round: 0,
            created_at: now,
            started_at: None,
            completed_at: None,
            linked_task: request.linked_task,
            linked_decision: request.linked_decision,
            trigger: request.trigger,
            result: None,
            usage: MeetingUsage::default(),
            recovery_note: None,
            explicit_files: request.explicit_files,
            context_budget: request.context_budget.unwrap_or(policy.context_budget),
            context_packages: Vec::new(),
            execution_gates: Vec::new(),
            payg_rejected: false,
        };
        self.store.upsert_meeting(&meeting)?;
        self.governance.audit_delivery(
            &actor.id,
            "CREATE_MEETING",
            Some(&meeting.id),
            meeting.linked_task.as_deref(),
            MutationDisposition::Applied,
            Some(format!(
                "bounded participants={}; rounds={}; token_budget={}",
                meeting.participants.len(),
                meeting.max_rounds,
                meeting.total_meeting_token_budget
            )),
        )?;
        self.events.publish(
            EventType::MeetingCreated,
            actor.id,
            Some(meeting.id.clone()),
            meeting.linked_task.clone(),
            json!({"meetingId":meeting.id,"participants":meeting.participants.iter().map(|p|&p.agent_id).collect::<Vec<_>>(),"maxRounds":meeting.max_rounds,"tokenBudget":meeting.total_meeting_token_budget}),
        )?;
        self.start_background(&meeting.id)?;
        Ok(meeting)
    }

    pub fn start_background(self: &Arc<Self>, meeting_id: &str) -> Result<()> {
        {
            let mut running = self
                .running
                .lock()
                .map_err(|_| RuntimeError::Lock("running meetings"))?;
            if !running.insert(meeting_id.to_owned()) {
                return Ok(());
            }
        }
        let engine = Arc::clone(self);
        let id = meeting_id.to_owned();
        tokio::spawn(async move {
            let _ = engine.run(&id).await;
            if let Ok(mut running) = engine.running.lock() {
                running.remove(&id);
            }
        });
        Ok(())
    }

    fn start_after_gate(self: &Arc<Self>, meeting_id: &str) {
        let engine = Arc::clone(self);
        let id = meeting_id.to_owned();
        tokio::spawn(async move {
            for _ in 0..200 {
                let active = engine
                    .running
                    .lock()
                    .map(|running| running.contains(&id))
                    .unwrap_or(true);
                if !active {
                    let _ = engine.start_background(&id);
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });
    }

    pub async fn cancel(&self, actor: Actor, meeting_id: &str) -> Result<Meeting> {
        self.authorize_actor(&actor)?;
        let mut meeting = self.required_meeting(meeting_id)?;
        if meeting.status.terminal() {
            return Ok(meeting);
        }
        meeting.status = MeetingStatus::Cancelled;
        meeting.completed_at = Some(Utc::now().to_rfc3339());
        self.store.upsert_meeting(&meeting)?;
        if let Some(sender) = self
            .cancellations
            .lock()
            .map_err(|_| RuntimeError::Lock("meeting cancellations"))?
            .get(meeting_id)
        {
            let _ = sender.send(true);
        }
        for turn in self.store.list_meeting_turns(meeting_id)? {
            if turn.status == MeetingTurnStatus::Running {
                if let (Some(agent), Some(session_id)) = (
                    self.agents.get(&turn.participant_id)?,
                    turn.provider_session_id.as_deref(),
                ) {
                    let provider = self.sessions.provider_for(&agent)?;
                    let _ = provider.cancel_turn(session_id).await;
                }
            }
        }
        self.events.publish(
            EventType::MeetingCancelled,
            actor.id,
            Some(meeting.id.clone()),
            meeting.linked_task.clone(),
            json!({"meetingId":meeting.id}),
        )?;
        Ok(meeting)
    }

    pub fn reconcile_startup(self: &Arc<Self>) -> Result<()> {
        for mut meeting in self.store.list_meetings()? {
            if meeting.status == MeetingStatus::Blocked
                && meeting
                    .execution_gates
                    .iter()
                    .any(|gate| gate.status == ExecutionGateStatus::Waiting)
            {
                for decision_id in meeting
                    .execution_gates
                    .iter()
                    .filter(|gate| gate.status == ExecutionGateStatus::Waiting)
                    .filter_map(|gate| gate.decision_id.clone())
                    .collect::<Vec<_>>()
                {
                    self.handle_decision_resolution(&decision_id)?;
                }
                continue;
            }
            if meeting.status != MeetingStatus::Running && meeting.status != MeetingStatus::Ready {
                continue;
            }
            let mut uncertain = false;
            for mut turn in self.store.list_meeting_turns(&meeting.id)? {
                if turn.status == MeetingTurnStatus::Running {
                    turn.status = MeetingTurnStatus::UnknownAfterCrash;
                    turn.failure = Some(
                        "daemon stopped while provider turn outcome was uncertain; blind replay denied"
                            .into(),
                    );
                    turn.completed_at = Some(Utc::now().to_rfc3339());
                    self.store.upsert_meeting_turn(&turn)?;
                    uncertain = true;
                }
            }
            if uncertain {
                meeting.status = MeetingStatus::Blocked;
                meeting.recovery_note = Some(
                    "An active participant turn became uncertain after restart and requires review"
                        .into(),
                );
                self.store.upsert_meeting(&meeting)?;
                self.events.publish(
                    EventType::MeetingBlocked,
                    "recovery",
                    Some(meeting.id.clone()),
                    meeting.linked_task.clone(),
                    json!({"meetingId":meeting.id,"reason":"UNKNOWN_PARTICIPANT_AFTER_CRASH"}),
                )?;
            } else {
                meeting.status = MeetingStatus::Ready;
                meeting.recovery_note = Some("Safely resumed before the next provider turn".into());
                self.store.upsert_meeting(&meeting)?;
                self.start_background(&meeting.id)?;
            }
        }
        Ok(())
    }

    fn handle_decision_resolution(self: &Arc<Self>, decision_id: &str) -> Result<()> {
        let Some(decision) = self
            .governance
            .snapshot()?
            .decisions
            .into_iter()
            .find(|decision| decision.id == decision_id)
        else {
            return Ok(());
        };
        if decision.status == DecisionStatus::Open {
            return Ok(());
        }
        let Some(mut meeting) = self.store.list_meetings()?.into_iter().find(|meeting| {
            meeting.execution_gates.iter().any(|gate| {
                gate.decision_id.as_deref() == Some(decision_id)
                    && gate.status == ExecutionGateStatus::Waiting
            })
        }) else {
            return Ok(());
        };
        let gate_index = meeting
            .execution_gates
            .iter()
            .position(|gate| gate.decision_id.as_deref() == Some(decision_id))
            .expect("matching gate");
        let claim_key = format!(
            "{}|{}|{}|{}",
            meeting.id,
            meeting.execution_gates[gate_index]
                .resource_id
                .as_deref()
                .unwrap_or_default(),
            meeting.execution_gates[gate_index]
                .provider
                .as_deref()
                .unwrap_or_default(),
            meeting.execution_gates[gate_index]
                .model
                .as_deref()
                .unwrap_or_default()
        );
        self.payg_decision_claims
            .lock()
            .map_err(|_| RuntimeError::Lock("meeting PAYG decision claims"))?
            .remove(&claim_key);
        let stale = !self.gate_binding_is_current(
            &meeting,
            &meeting.execution_gates[gate_index],
            &decision,
            meeting.execution_gates[gate_index]
                .context_fingerprint
                .as_deref()
                .unwrap_or_default(),
            meeting.execution_gates[gate_index]
                .routing_decision_id
                .as_deref(),
        )?;
        let now = Utc::now().to_rfc3339();
        let gate = &mut meeting.execution_gates[gate_index];
        gate.resolved_at = Some(now);
        if stale {
            gate.status = ExecutionGateStatus::Superseded;
            gate.resolution_reason = Some(
                "meeting, resource, or project policy changed while approval was pending".into(),
            );
            meeting.recovery_note = Some(
                "Stale approval was not consumed; routing and context will be evaluated again"
                    .into(),
            );
        } else if decision.status == DecisionStatus::Approved {
            gate.status = ExecutionGateStatus::Approved;
            gate.resolution_reason = Some("exact GOD decision approved".into());
            meeting.recovery_note =
                Some("GOD gate resolved; resuming from provider-safe boundary".into());
        } else {
            gate.status = ExecutionGateStatus::Rejected;
            gate.resolution_reason = Some("GOD rejected the exact PAYG operation".into());
            meeting.payg_rejected = true;
            meeting.recovery_note =
                Some("PAYG rejected; retrying eligibility with PAYG disabled".into());
        }
        if !self
            .store
            .list_meeting_turns(&meeting.id)?
            .iter()
            .any(|turn| turn.status == MeetingTurnStatus::UnknownAfterCrash)
        {
            meeting.status = MeetingStatus::Ready;
            meeting.completed_at = None;
        }
        self.store.upsert_meeting(&meeting)?;
        self.events.publish(
            EventType::ExecutionGateResolved,
            "meeting-gate",
            Some(meeting.id.clone()),
            meeting.linked_task.clone(),
            json!({"meetingId":meeting.id,"decisionId":decision_id,"gateStatus":meeting.execution_gates[gate_index].status}),
        )?;
        if meeting.status == MeetingStatus::Ready {
            self.events.publish(
                EventType::MeetingResumed,
                "meeting-gate",
                Some(meeting.id.clone()),
                meeting.linked_task.clone(),
                json!({"meetingId":meeting.id,"decisionId":decision_id}),
            )?;
            self.start_after_gate(&meeting.id);
        }
        Ok(())
    }

    pub async fn create_action_task(
        &self,
        actor: Actor,
        meeting_id: &str,
        action_id: &str,
        assigned_to: Vec<String>,
    ) -> Result<Task> {
        self.authorize_actor(&actor)?;
        let mut meeting = self.required_meeting(meeting_id)?;
        let result = meeting
            .result
            .as_mut()
            .ok_or_else(|| RuntimeError::Governance("meeting has no completed outcome".into()))?;
        let action = result
            .action_items
            .iter_mut()
            .find(|action| action.id == action_id)
            .ok_or_else(|| RuntimeError::Governance("meeting action item not found".into()))?;
        if let Some(task_id) = &action.task_id {
            return self
                .store
                .get_task(task_id)?
                .ok_or_else(|| RuntimeError::TaskNotFound(task_id.clone()));
        }
        if assigned_to.is_empty() {
            return Err(RuntimeError::Governance(
                "meeting action task requires an explicit assignee".into(),
            ));
        }
        let task_id = format!("TASK-MTG-{}", uuid::Uuid::new_v4());
        let mut extra = serde_json::Map::new();
        extra.insert("meetingId".into(), Value::String(meeting.id.clone()));
        extra.insert("meetingActionId".into(), Value::String(action.id.clone()));
        let task = Task {
            id: task_id.clone(),
            created_by: actor.id.clone(),
            objective: action.description.clone(),
            assigned_to,
            dependencies: Vec::new(),
            acceptance_criteria: vec![
                "Complete the agreed meeting action and validate the result".into()
            ],
            inputs: vec![format!("Meeting {}: {}", meeting.id, meeting.objective)],
            outputs: Vec::new(),
            status: TaskStatus::Ready,
            execution: TaskExecution::default(),
            on_success: TaskSuccessAction::default(),
            on_failure: TaskSuccessAction::default(),
            weight: 1.0,
            extra,
        };
        self.tasks.ingest(task.clone(), None).await?;
        action.task_id = Some(task_id);
        self.store.upsert_meeting(&meeting)?;
        Ok(task)
    }

    async fn run(&self, meeting_id: &str) -> Result<()> {
        let mut meeting = self.required_meeting(meeting_id)?;
        if meeting.status != MeetingStatus::Ready && meeting.status != MeetingStatus::Planned {
            return Ok(());
        }
        meeting.status = MeetingStatus::Running;
        meeting
            .started_at
            .get_or_insert_with(|| Utc::now().to_rfc3339());
        self.store.upsert_meeting(&meeting)?;
        let (cancel, receiver) = watch::channel(false);
        self.cancellations
            .lock()
            .map_err(|_| RuntimeError::Lock("meeting cancellations"))?
            .insert(meeting.id.clone(), cancel);
        self.events.publish(
            EventType::MeetingStarted,
            meeting.organizer.clone(),
            Some(meeting.id.clone()),
            meeting.linked_task.clone(),
            json!({"meetingId":meeting.id}),
        )?;

        let round_one = self
            .run_round(&mut meeting, 1, MeetingTurnKind::Position, None, &receiver)
            .await;
        if let Err(error) = round_one {
            self.finish_with_error(&mut meeting, error)?;
            return Ok(());
        }
        let first = self.store.list_meeting_turns(&meeting.id)?;
        let disagreements = structured_disagreements(&first);
        let round_two_cost = meeting
            .per_response_token_limit
            .saturating_mul(meeting.participants.len() as u64);
        if meeting.max_rounds > 1
            && !disagreements.is_empty()
            && remaining_budget(&meeting, &first) >= round_two_cost
        {
            let context = round_two_context(&first, &disagreements);
            if let Err(error) = self
                .run_round(
                    &mut meeting,
                    2,
                    MeetingTurnKind::Response,
                    Some(context),
                    &receiver,
                )
                .await
            {
                self.finish_with_error(&mut meeting, error)?;
                return Ok(());
            }
        }
        if *receiver.borrow() {
            return Ok(());
        }
        self.close(&mut meeting)?;
        self.cancellations
            .lock()
            .map_err(|_| RuntimeError::Lock("meeting cancellations"))?
            .remove(meeting_id);
        Ok(())
    }

    async fn run_round(
        &self,
        meeting: &mut Meeting,
        round: u8,
        kind: MeetingTurnKind,
        structured_context: Option<String>,
        cancel: &watch::Receiver<bool>,
    ) -> Result<()> {
        if round > meeting.max_rounds {
            return Err(RuntimeError::Governance(
                "meeting round limit exceeded".into(),
            ));
        }
        meeting.current_round = round;
        self.store.upsert_meeting(meeting)?;
        self.events.publish(
            EventType::MeetingRoundStarted,
            "meeting-coordinator",
            Some(meeting.id.clone()),
            meeting.linked_task.clone(),
            json!({"meetingId":meeting.id,"round":round,"kind":kind}),
        )?;
        let mut handles = Vec::new();
        for participant in meeting.participants.clone() {
            if self
                .store
                .meeting_turn(&meeting.id, round, &participant.agent_id, kind)?
                .is_some_and(|turn| turn.status == MeetingTurnStatus::Completed)
            {
                continue;
            }
            let engine = self.clone();
            let meeting = meeting.clone();
            let context = structured_context.clone();
            let receiver = cancel.clone();
            handles.push(tokio::spawn(async move {
                engine
                    .execute_participant(&meeting, participant, round, kind, context, receiver)
                    .await
            }));
        }
        for handle in handles {
            let result = handle.await.map_err(|error| {
                RuntimeError::Provider(format!("meeting worker stopped: {error}"))
            })?;
            result?;
        }
        Ok(())
    }

    async fn execute_participant(
        &self,
        meeting: &Meeting,
        participant: MeetingParticipant,
        round: u8,
        kind: MeetingTurnKind,
        context: Option<String>,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<()> {
        if *cancel.borrow() {
            return Err(RuntimeError::Cancelled(meeting.id.clone()));
        }
        let _lane = self.tasks.acquire_agent_lane(&participant.agent_id).await?;
        let mut agent = self
            .agents
            .get(&participant.agent_id)?
            .ok_or_else(|| RuntimeError::AgentNotFound(participant.agent_id.clone()))?;
        let mut context_package = self.build_participant_context(meeting, &agent)?;
        if context_package.status == ContextBuildStatus::Blocked {
            return Err(RuntimeError::Governance(format!(
                "required meeting context is blocked: {}",
                context_package.warnings.join("; ")
            )));
        }
        self.record_context_package(meeting, &mut context_package)?;
        let (routed, route_id, resource_id) =
            self.route_agent(meeting, &agent, &context_package.fingerprint)?;
        let (routed, route_id, resource_id, context_package) = self
            .refresh_context_before_provider(
                meeting,
                routed,
                route_id,
                resource_id,
                context_package,
                2,
            )?;
        agent = routed;
        let mut context_package = context_package;
        let provider_lane = self.provider_lane(&agent, resource_id.as_deref())?;
        let _provider_permit = if let Some(lane) = provider_lane {
            Some(tokio::select! {
                permit = lane.acquire_owned() => permit.map_err(|_| RuntimeError::ResourceUnavailable {
                    agent_id: agent.id.clone(), status: "PROVIDER_CONCURRENCY_CLOSED".into(), reset_at: None,
                })?,
                changed = cancel.changed() => {
                    if changed.is_ok() && *cancel.borrow() {
                        return Err(RuntimeError::Cancelled(meeting.id.clone()));
                    }
                    return Err(RuntimeError::ResourceUnavailable { agent_id: agent.id.clone(), status: "PROVIDER_CONCURRENCY_UNAVAILABLE".into(), reset_at: None });
                }
            })
        } else {
            None
        };
        let initial_resource_id = resource_id.clone();
        let (rerouted, refreshed_route_id, refreshed_resource_id, refreshed_package) = self
            .refresh_context_before_provider(
                meeting,
                agent.clone(),
                route_id,
                resource_id,
                context_package,
                1,
            )?;
        if rerouted.provider != agent.provider
            || rerouted.model != agent.model
            || refreshed_resource_id != initial_resource_id
        {
            return Err(RuntimeError::ResourceUnavailable {
                agent_id: agent.id,
                status: "CONTEXT_CHANGED_REQUIRES_REROUTE".into(),
                reset_at: None,
            });
        }
        agent = rerouted;
        let route_id = refreshed_route_id;
        let resource_id = refreshed_resource_id;
        context_package = refreshed_package;
        let now = Utc::now().to_rfc3339();
        let turn_id = format!(
            "MTURN-{}-{}-{}-{}",
            meeting.id,
            round,
            participant.agent_id,
            turn_kind_label(kind)
        );
        let mut turn = MeetingTurn {
            id: turn_id,
            meeting_id: meeting.id.clone(),
            round,
            participant_id: participant.agent_id.clone(),
            kind,
            status: MeetingTurnStatus::Running,
            provider: Some(agent.provider.clone()),
            model: Some(agent.model.clone()),
            provider_session_id: None,
            provider_turn_id: None,
            contribution: None,
            usage: UsageSnapshot::default(),
            charged_tokens: 0,
            failure: None,
            created_at: now.clone(),
            started_at: Some(now),
            completed_at: None,
            context_id: Some(context_package.context_id.clone()),
            context_fingerprint: Some(context_package.fingerprint.clone()),
            estimated_context_tokens: Some(context_package.estimated_tokens),
        };
        let session = self.sessions.ensure(&agent).await?;
        turn.provider_session_id = Some(session.provider_session_id.clone());
        self.store.upsert_meeting_turn(&turn)?;
        self.events.publish(
            EventType::MeetingParticipantStarted,
            participant.agent_id.clone(),
            Some(meeting.id.clone()),
            meeting.linked_task.clone(),
            json!({"meetingId":meeting.id,"round":round,"kind":kind,"activity":"MEETING","resourceId":resource_id,"routingDecisionId":route_id}),
        )?;
        if !self.context_is_fresh_for_meeting(meeting, &agent, &context_package)? {
            turn.status = MeetingTurnStatus::Failed;
            turn.failure = Some("CONTEXT_CHANGED_BEFORE_PROVIDER_TURN".into());
            turn.completed_at = Some(Utc::now().to_rfc3339());
            self.store.upsert_meeting_turn(&turn)?;
            return Err(RuntimeError::Governance(
                "meeting context changed immediately before provider execution; retry is required"
                    .into(),
            ));
        }
        let (final_agent, _final_route_id, final_resource_id) =
            match self.route_agent(meeting, &agent, &context_package.fingerprint) {
                Ok(value) => value,
                Err(error) => {
                    turn.status = MeetingTurnStatus::Failed;
                    turn.failure = Some("PAYG_GATE_REVALIDATION_FAILED".into());
                    turn.completed_at = Some(Utc::now().to_rfc3339());
                    self.store.upsert_meeting_turn(&turn)?;
                    return Err(error);
                }
            };
        if final_agent.provider != agent.provider
            || final_agent.model != agent.model
            || final_resource_id != resource_id
        {
            turn.status = MeetingTurnStatus::Failed;
            turn.failure = Some("RESOURCE_CHANGED_BEFORE_PROVIDER_TURN".into());
            turn.completed_at = Some(Utc::now().to_rfc3339());
            self.store.upsert_meeting_turn(&turn)?;
            return Err(RuntimeError::ResourceUnavailable {
                agent_id: agent.id.clone(),
                status: "RESOURCE_CHANGED_REQUIRES_REROUTE".into(),
                reset_at: None,
            });
        }
        agent = final_agent;
        let task = meeting_task(meeting, &agent, round, kind, &context_package, context);
        let provider = self.sessions.provider_for(&agent)?;
        let outcome = tokio::select! {
            result = provider.send_task(&session.provider_session_id, &agent, &task) => result,
            changed = cancel.changed() => {
                if changed.is_ok() && *cancel.borrow() {
                    let _ = provider.cancel_turn(&session.provider_session_id).await;
                }
                Err(ProviderFailure::Cancelled)
            }
        };
        match outcome {
            Ok(result) => {
                self.complete_turn(meeting, &agent, &session.provider_session_id, turn, result)
            }
            Err(error) => {
                turn.status = match error {
                    ProviderFailure::Cancelled => MeetingTurnStatus::Cancelled,
                    ProviderFailure::Timeout | ProviderFailure::ProcessCrash { .. } => {
                        MeetingTurnStatus::UnknownAfterCrash
                    }
                    _ => MeetingTurnStatus::Failed,
                };
                turn.failure = Some(provider_failure_label(&error).into());
                turn.completed_at = Some(Utc::now().to_rfc3339());
                self.store.upsert_meeting_turn(&turn)?;
                Err(match error {
                    ProviderFailure::RateLimited { reset_at } => {
                        RuntimeError::ResourceUnavailable {
                            agent_id: agent.id,
                            status: "RATE_LIMITED".into(),
                            reset_at,
                        }
                    }
                    ProviderFailure::Cancelled => RuntimeError::Cancelled(meeting.id.clone()),
                    ProviderFailure::Timeout | ProviderFailure::ProcessCrash { .. } => {
                        RuntimeError::UnknownAfterCrash(format!(
                            "meeting {} participant {} outcome is uncertain",
                            meeting.id, participant.agent_id
                        ))
                    }
                    other => RuntimeError::Provider(provider_failure_label(&other).into()),
                })
            }
        }
    }

    fn complete_turn(
        &self,
        meeting: &Meeting,
        agent: &Agent,
        expected_session_id: &str,
        mut turn: MeetingTurn,
        result: ProviderExecutionResult,
    ) -> Result<()> {
        self.sessions.update_after_turn(
            &agent.id,
            expected_session_id,
            &result.session_id,
            &result.session_metadata,
        )?;
        turn.status = MeetingTurnStatus::Completed;
        turn.provider_session_id = Some(result.session_id);
        turn.provider_turn_id = result.turn_id;
        turn.contribution = Some(parse_contribution(&result.summary));
        turn.charged_tokens = result
            .usage
            .output_tokens
            .unwrap_or(meeting.per_response_token_limit)
            .min(meeting.per_response_token_limit);
        turn.usage = result.usage;
        turn.completed_at = Some(Utc::now().to_rfc3339());
        self.store.upsert_meeting_turn(&turn)?;
        self.events.publish(
            EventType::MeetingParticipantCompleted,
            agent.id.clone(),
            Some(meeting.id.clone()),
            meeting.linked_task.clone(),
            json!({"meetingId":meeting.id,"round":turn.round,"kind":turn.kind,"chargedTokens":turn.charged_tokens}),
        )?;
        Ok(())
    }

    fn route_agent(
        &self,
        meeting: &Meeting,
        agent: &Agent,
        context_fingerprint: &str,
    ) -> Result<(Agent, Option<String>, Option<String>)> {
        if agent.intelligence_policy.assignment != ModelAssignment::Auto {
            self.validate_explicit_resource(meeting, agent, context_fingerprint)?;
            return Ok((agent.clone(), None, None));
        }
        let mut policy = self.store.economic_policy()?;
        let meeting_policy = self.governance.snapshot()?.policy.meetings;
        policy.allow_payg &= meeting_policy.allow_payg_for_meetings;
        if meeting.payg_rejected {
            policy.allow_payg = false;
        }
        let function = agent
            .with_backfilled_organization()
            .function
            .unwrap_or(AgentFunction::GenericSoftwareAgent);
        let requirements = TaskRequirements {
            function,
            complexity: 55,
            risk: TaskRisk::Medium,
            context_tokens: Some(meeting.context_budget.max_tokens),
            required_capabilities: meeting.required_capabilities.clone(),
            requires_tools: false,
            requires_worktree: false,
            deadline_at: None,
            priority: 50,
            taxonomy_version: "meeting-v1".into(),
        };
        let resources = self.store.list_intelligence_resources()?;
        let evidence = self.store.list_task_outcomes(None, 10_000)?;
        let learning = self.store.capability_learning_policy()?;
        let decision = QuotaAwareEconomicRouter.route_with_evidence(
            &format!("{}:round:{}", meeting.id, meeting.current_round.max(1)),
            &requirements,
            &resources,
            &policy,
            &evidence,
            &learning,
            Utc::now(),
        );
        self.store.save_routing_decision(&decision)?;
        if decision.outcome != RoutingOutcome::Selected {
            return Err(RuntimeError::ResourceUnavailable {
                agent_id: agent.id.clone(),
                status: "NO_SUITABLE_RESOURCE".into(),
                reset_at: None,
            });
        }
        let resource_id = decision.selected_resource_id.clone();
        let mut routed = agent.clone();
        routed.provider = decision.selected_provider.clone().unwrap_or_default();
        routed.model = decision.selected_model.clone().unwrap_or_default();
        routed.reasoning_effort = decision
            .reasoning_effort
            .clone()
            .unwrap_or_else(|| "MEDIUM".into())
            .to_ascii_lowercase();
        if let Some(resource) = resource_id
            .as_deref()
            .and_then(|id| resources.iter().find(|resource| resource.id == id))
        {
            if resource.billing_mode == BillingMode::Payg && {
                let bound_route = meeting
                    .execution_gates
                    .iter()
                    .find(|gate| {
                        gate.status == ExecutionGateStatus::Approved
                            && gate.resource_id.as_deref() == Some(resource.id.as_str())
                            && gate.provider.as_deref() == Some(routed.provider.as_str())
                            && gate.model.as_deref() == Some(routed.model.as_str())
                            && gate.context_fingerprint.as_deref() == Some(context_fingerprint)
                    })
                    .and_then(|gate| gate.routing_decision_id.as_deref());
                let current_route = bound_route.or(Some(decision.id.as_str()));
                !self.has_approved_payg_gate(
                    meeting,
                    &resource.id,
                    &routed.provider,
                    &routed.model,
                    context_fingerprint,
                    current_route,
                )?
            } {
                self.request_payg_decision(
                    meeting,
                    &resource.id,
                    &routed.provider,
                    &routed.model,
                    Some(&decision.id),
                    context_fingerprint,
                )?;
                return Err(RuntimeError::Governance(
                    "PAYG meeting execution requires an exact GOD-bound decision".into(),
                ));
            }
        }
        Ok((routed, Some(decision.id), resource_id))
    }

    fn validate_explicit_resource(
        &self,
        meeting: &Meeting,
        agent: &Agent,
        context_fingerprint: &str,
    ) -> Result<()> {
        let resources = self.store.list_intelligence_resources()?;
        if let Some(resource) = resources.iter().find(|r| r.provider == agent.provider) {
            if !matches!(resource.status.as_str(), "AVAILABLE" | "LOW" | "DEGRADED") {
                return Err(RuntimeError::ResourceUnavailable {
                    agent_id: agent.id.clone(),
                    status: resource.status.clone(),
                    reset_at: resource.quota.reset_at.clone(),
                });
            }
            if resource.billing_mode == BillingMode::Payg {
                if meeting.payg_rejected {
                    return Err(RuntimeError::Governance(
                        "GOD rejected PAYG and this explicit provider has no automatic fallback"
                            .into(),
                    ));
                }
                let policy = self.governance.snapshot()?.policy;
                if !policy.meetings.allow_payg_for_meetings || !policy.allow_payg {
                    return Err(RuntimeError::Governance(
                        "PAYG is disabled for meetings".into(),
                    ));
                }
                if !self.has_approved_payg_gate(
                    meeting,
                    &resource.id,
                    &agent.provider,
                    &agent.model,
                    context_fingerprint,
                    None,
                )? {
                    self.request_payg_decision(
                        meeting,
                        &resource.id,
                        &agent.provider,
                        &agent.model,
                        None,
                        context_fingerprint,
                    )?;
                    return Err(RuntimeError::Governance(
                        "PAYG meeting execution requires an exact GOD-bound decision".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn provider_lane(
        &self,
        agent: &Agent,
        resource_id: Option<&str>,
    ) -> Result<Option<Arc<Semaphore>>> {
        let resources = self.store.list_intelligence_resources()?;
        let Some(resource) = resources.iter().find(|resource| {
            resource_id.is_some_and(|id| id == resource.id) || resource.provider == agent.provider
        }) else {
            return Ok(None);
        };
        let Some(limit) = resource.concurrency_limit else {
            return Ok(None);
        };
        if resource.current_concurrency >= limit {
            return Err(RuntimeError::ResourceUnavailable {
                agent_id: agent.id.clone(),
                status: "CONCURRENCY_EXHAUSTED".into(),
                reset_at: None,
            });
        }
        let available = limit.saturating_sub(resource.current_concurrency).max(1) as usize;
        let mut lanes = self
            .provider_lanes
            .lock()
            .map_err(|_| RuntimeError::Lock("meeting provider lanes"))?;
        Ok(Some(
            lanes
                .entry(resource.id.clone())
                .or_insert_with(|| Arc::new(Semaphore::new(available)))
                .clone(),
        ))
    }

    fn has_approved_payg_gate(
        &self,
        meeting: &Meeting,
        resource_id: &str,
        provider: &str,
        model: &str,
        context_fingerprint: &str,
        routing_decision_id: Option<&str>,
    ) -> Result<bool> {
        let snapshot = self.governance.snapshot()?;
        for gate in meeting.execution_gates.iter().filter(|gate| {
            gate.gate_type == ExecutionGateType::GodDecision
                && gate.status == ExecutionGateStatus::Approved
                && gate.resource_id.as_deref() == Some(resource_id)
                && gate.provider.as_deref() == Some(provider)
                && gate.model.as_deref() == Some(model)
        }) {
            let Some(decision_id) = gate.decision_id.as_deref() else {
                continue;
            };
            let Some(decision) = snapshot
                .decisions
                .iter()
                .find(|item| item.id == decision_id)
            else {
                continue;
            };
            if self.gate_binding_is_current(
                meeting,
                gate,
                decision,
                context_fingerprint,
                routing_decision_id,
            )? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn gate_binding_is_current(
        &self,
        meeting: &Meeting,
        gate: &ExecutionGate,
        decision: &super::governance::GodDecision,
        context_fingerprint: &str,
        routing_decision_id: Option<&str>,
    ) -> Result<bool> {
        let expected_revision = gate.expected_policy_revision;
        let required_current_revision = if decision.status == DecisionStatus::Approved {
            expected_revision.saturating_add(1)
        } else {
            expected_revision
        };
        if decision.request.expected_revision != expected_revision
            || self.governance.snapshot()?.revision != required_current_revision
            || decision.request.mutation.target() != Some(gate.operation_id.as_str())
            || gate.gate_type != ExecutionGateType::GodDecision
            || gate.safe_continuation_phase != "BEFORE_PROVIDER_TURN"
            || gate.context_fingerprint.as_deref() != Some(context_fingerprint)
            || gate.routing_decision_id.as_deref() != routing_decision_id
        {
            return Ok(false);
        }
        let Some(detail) = decision.request.mutation.protected_detail() else {
            return Ok(false);
        };
        let resources = self.store.list_intelligence_resources()?;
        let resource_current = gate.resource_id.as_deref().is_some_and(|resource_id| {
            resources.iter().any(|resource| {
                resource.id == resource_id
                    && resource.provider == gate.provider.as_deref().unwrap_or_default()
                    && resource
                        .supported_models
                        .iter()
                        .any(|model| model == gate.model.as_deref().unwrap_or_default())
            })
        });
        let policy = self.governance.snapshot()?.policy;
        let resource_eligible_for_approval = decision.status != DecisionStatus::Approved
            || resources.iter().any(|resource| {
                gate.resource_id.as_deref() == Some(resource.id.as_str())
                    && resource.billing_mode == BillingMode::Payg
                    && matches!(resource.status.as_str(), "AVAILABLE" | "LOW" | "DEGRADED")
            });
        let configuration_fingerprint = meeting_configuration_fingerprint(meeting);
        Ok((decision.status != DecisionStatus::Approved
            || (policy.allow_payg && policy.meetings.allow_payg_for_meetings))
            && resource_current
            && resource_eligible_for_approval
            && detail.get("expectedPolicyRevision").and_then(Value::as_u64)
                == Some(expected_revision)
            && detail.get("meetingId").and_then(Value::as_str) == Some(meeting.id.as_str())
            && detail.get("resourceId").and_then(Value::as_str) == gate.resource_id.as_deref()
            && detail.get("provider").and_then(Value::as_str) == gate.provider.as_deref()
            && detail.get("model").and_then(Value::as_str) == gate.model.as_deref()
            && detail.get("maximumMeetingTokens").and_then(Value::as_u64) == gate.maximum_tokens
            && detail
                .get("meetingConfigurationFingerprint")
                .and_then(Value::as_str)
                == Some(configuration_fingerprint.as_str())
            && detail.get("contextFingerprint").and_then(Value::as_str)
                == Some(context_fingerprint)
            && detail.get("routingDecisionId").and_then(Value::as_str) == routing_decision_id
            && detail.get("safeContinuationPhase").and_then(Value::as_str)
                == Some(gate.safe_continuation_phase.as_str()))
    }

    fn build_participant_context(
        &self,
        meeting: &Meeting,
        agent: &Agent,
    ) -> Result<ContextPackage> {
        self.context
            .build(self.participant_context_request(meeting, agent))
    }

    fn participant_context_request(&self, meeting: &Meeting, agent: &Agent) -> ContextBuildRequest {
        let worktree = agent
            .worktree
            .as_deref()
            .map(PathBuf::from)
            .filter(|path| path.is_dir());
        ContextBuildRequest {
            operation: ContextOperationType::Meeting,
            objective: meeting.objective.clone(),
            actor: agent.clone(),
            linked_task: meeting.linked_task.clone(),
            linked_meeting: Some(meeting.id.clone()),
            requested_capabilities: meeting.required_capabilities.keys().cloned().collect(),
            repository: self.root.clone(),
            worktree,
            explicit_files: meeting.explicit_files.clone(),
            budget: meeting.context_budget.clone(),
        }
    }

    fn context_is_fresh_for_meeting(
        &self,
        meeting: &Meeting,
        agent: &Agent,
        package: &ContextPackage,
    ) -> Result<bool> {
        if package.objective != meeting.objective {
            return Ok(false);
        }
        let request = self.participant_context_request(meeting, agent);
        let project_root = self.root.canonicalize()?;
        let root = self.context.resolved_source_root(&request, &project_root)?;
        self.context.is_fresh(&package.record(), &root)
    }

    fn refresh_context_before_provider(
        &self,
        meeting: &Meeting,
        mut agent: Agent,
        mut route_id: Option<String>,
        mut resource_id: Option<String>,
        mut package: ContextPackage,
        max_rebuilds: usize,
    ) -> Result<(Agent, Option<String>, Option<String>, ContextPackage)> {
        for _ in 0..=max_rebuilds {
            if self.context_is_fresh_for_meeting(meeting, &agent, &package)? {
                return Ok((agent, route_id, resource_id, package));
            }
            let rebuilt = self.build_participant_context(meeting, &agent)?;
            if rebuilt.status == ContextBuildStatus::Blocked {
                return Err(RuntimeError::Governance(format!(
                    "required meeting context became blocked: {}",
                    rebuilt.warnings.join("; ")
                )));
            }
            let mut rebuilt = rebuilt;
            self.record_context_package(meeting, &mut rebuilt)?;
            let (rerouted, reroute_id, reroute_resource) =
                self.route_agent(meeting, &agent, &rebuilt.fingerprint)?;
            if rerouted.provider != agent.provider || rerouted.model != agent.model {
                return Err(RuntimeError::ResourceUnavailable {
                    agent_id: agent.id,
                    status: "CONTEXT_CHANGED_REQUIRES_REROUTE".into(),
                    reset_at: None,
                });
            }
            agent = rerouted;
            route_id = reroute_id;
            resource_id = reroute_resource;
            package = rebuilt;
        }
        Err(RuntimeError::Governance(
            "meeting context changed repeatedly before provider execution".into(),
        ))
    }

    fn record_context_package(
        &self,
        meeting: &Meeting,
        package: &mut ContextPackage,
    ) -> Result<()> {
        let mut persisted = self.required_meeting(&meeting.id)?;
        let previous = persisted
            .context_packages
            .iter()
            .rev()
            .find(|record| record.actor_id == package.actor_id)
            .cloned();
        if let Some(previous) = &previous {
            if previous.fingerprint == package.fingerprint {
                package.context_id = previous.context_id.clone();
                return Ok(());
            }
        }
        let rebuilt = previous.is_some();
        persisted.context_packages.push(package.record());
        if persisted.context_packages.len() > 24 {
            let remove = persisted.context_packages.len() - 24;
            persisted.context_packages.drain(0..remove);
        }
        self.store.upsert_meeting(&persisted)?;
        self.events.publish(
            if rebuilt {
                EventType::ContextRebuilt
            } else {
                EventType::ContextBuilt
            },
            package.actor_id.clone(),
            Some(meeting.id.clone()),
            meeting.linked_task.clone(),
            json!({
                "meetingId":meeting.id,
                "contextId":package.context_id,
                "fingerprint":package.fingerprint,
                "estimatedTokens":package.estimated_tokens,
                "warnings":package.warnings.len(),
            }),
        )?;
        if !package.warnings.is_empty() {
            self.events.publish(
                EventType::ContextWarning,
                package.actor_id.clone(),
                Some(meeting.id.clone()),
                meeting.linked_task.clone(),
                json!({"meetingId":meeting.id,"contextId":package.context_id,"warnings":package.warnings}),
            )?;
        }
        Ok(())
    }

    fn request_payg_decision(
        &self,
        meeting: &Meeting,
        resource_id: &str,
        provider: &str,
        model: &str,
        routing_decision_id: Option<&str>,
        context_fingerprint: &str,
    ) -> Result<()> {
        let claim_key = format!("{}|{resource_id}|{provider}|{model}", meeting.id);
        let mut claims = self
            .payg_decision_claims
            .lock()
            .map_err(|_| RuntimeError::Lock("meeting PAYG decision claims"))?;
        if !claims.insert(claim_key) {
            return Ok(());
        }
        drop(claims);
        let mut persisted = self.required_meeting(&meeting.id)?;
        let stale_decisions = persisted
            .execution_gates
            .iter_mut()
            .filter(|gate| {
                matches!(
                    gate.status,
                    ExecutionGateStatus::Waiting | ExecutionGateStatus::Approved
                ) && gate.resource_id.as_deref() == Some(resource_id)
                    && gate.provider.as_deref() == Some(provider)
                    && gate.model.as_deref() == Some(model)
                    && (gate.context_fingerprint.as_deref() != Some(context_fingerprint)
                        || gate.routing_decision_id.as_deref() != routing_decision_id)
            })
            .filter_map(|gate| {
                gate.status = ExecutionGateStatus::Superseded;
                gate.resolved_at = Some(Utc::now().to_rfc3339());
                gate.resolution_reason =
                    Some("context or routing binding changed before PAYG consumption".into());
                gate.decision_id.clone()
            })
            .collect::<Vec<_>>();
        if !stale_decisions.is_empty() {
            self.store.upsert_meeting(&persisted)?;
            for decision_id in stale_decisions {
                self.governance.supersede_decision(
                    &decision_id,
                    "PAYG approval binding changed before consumption".into(),
                )?;
            }
            persisted = self.required_meeting(&meeting.id)?;
        }
        if persisted.execution_gates.iter().any(|gate| {
            matches!(
                gate.status,
                ExecutionGateStatus::Waiting | ExecutionGateStatus::Approved
            ) && gate.resource_id.as_deref() == Some(resource_id)
                && gate.provider.as_deref() == Some(provider)
                && gate.model.as_deref() == Some(model)
                && gate.context_fingerprint.as_deref() == Some(context_fingerprint)
                && gate.routing_decision_id.as_deref() == routing_decision_id
        }) {
            return Ok(());
        }
        let revision = self.governance.snapshot()?.revision;
        let configuration_fingerprint = meeting_configuration_fingerprint(meeting);
        let operation_id = format!("meeting:{}:payg", meeting.id);
        let existing = self
            .governance
            .snapshot()?
            .decisions
            .into_iter()
            .find(|decision| {
                decision.request.mutation.target() == Some(operation_id.as_str())
                    && matches!(
                        decision.status,
                        DecisionStatus::Open | DecisionStatus::Approved
                    )
                    && decision
                        .request
                        .mutation
                        .protected_detail()
                        .is_some_and(|detail| {
                            detail.get("resourceId").and_then(Value::as_str) == Some(resource_id)
                                && detail.get("provider").and_then(Value::as_str) == Some(provider)
                                && detail.get("model").and_then(Value::as_str) == Some(model)
                                && detail.get("expectedPolicyRevision").and_then(Value::as_u64)
                                    == Some(revision)
                                && detail.get("maximumMeetingTokens").and_then(Value::as_u64)
                                    == Some(meeting.total_meeting_token_budget)
                                && detail
                                    .get("meetingConfigurationFingerprint")
                                    .and_then(Value::as_str)
                                    == Some(configuration_fingerprint.as_str())
                                && detail.get("contextFingerprint").and_then(Value::as_str)
                                    == Some(context_fingerprint)
                                && detail.get("routingDecisionId").and_then(Value::as_str)
                                    == routing_decision_id
                        })
            });
        if let Some(decision) = existing {
            persisted.linked_decision = Some(decision.id.clone());
            persisted.execution_gates.push(ExecutionGate {
                id: format!("GATE-{}", uuid::Uuid::new_v4()),
                gate_type: ExecutionGateType::GodDecision,
                status: if decision.status == DecisionStatus::Approved {
                    ExecutionGateStatus::Approved
                } else {
                    ExecutionGateStatus::Waiting
                },
                operation_id,
                safe_continuation_phase: "BEFORE_PROVIDER_TURN".into(),
                decision_id: Some(decision.id),
                resource_id: Some(resource_id.into()),
                provider: Some(provider.into()),
                model: Some(model.into()),
                expected_policy_revision: decision.request.expected_revision,
                maximum_tokens: Some(meeting.total_meeting_token_budget),
                context_fingerprint: Some(context_fingerprint.into()),
                routing_decision_id: routing_decision_id.map(str::to_owned),
                created_at: Utc::now().to_rfc3339(),
                resolved_at: None,
                resolution_reason: Some("reconciled existing exact decision".into()),
            });
            self.store.upsert_meeting(&persisted)?;
            return Ok(());
        }
        let result = self.governance.mutate(MutationRequest {
            actor: Actor {
                id: meeting.organizer.clone(),
                scope: AuthorityScope::Project,
            },
            expected_revision: revision,
            task_id: meeting.linked_task.clone(),
            reason: Some("A bounded meeting requires a PAYG intelligence resource".into()),
            mutation: OrganizationMutation::RequestProtectedAction {
                operation: ProtectedOperation::PaygSpend,
                target: operation_id.clone(),
                detail: json!({
                    "meetingId": meeting.id,
                    "resourceId": resource_id,
                    "provider": provider,
                    "model": model,
                    "expectedPolicyRevision": revision,
                    "maximumMeetingTokens": meeting.total_meeting_token_budget,
                    "meetingConfigurationFingerprint": configuration_fingerprint,
                    "contextFingerprint": context_fingerprint,
                    "routingDecisionId": routing_decision_id,
                    "safeContinuationPhase": "BEFORE_PROVIDER_TURN",
                    "scope": "MEETING_ONLY"
                }),
            },
        })?;
        if let Some(decision_id) = result.decision_id {
            persisted.linked_decision = Some(decision_id.clone());
            persisted.execution_gates.push(ExecutionGate {
                id: format!("GATE-{}", uuid::Uuid::new_v4()),
                gate_type: ExecutionGateType::GodDecision,
                status: ExecutionGateStatus::Waiting,
                operation_id,
                safe_continuation_phase: "BEFORE_PROVIDER_TURN".into(),
                decision_id: Some(decision_id.clone()),
                resource_id: Some(resource_id.into()),
                provider: Some(provider.into()),
                model: Some(model.into()),
                expected_policy_revision: revision,
                maximum_tokens: Some(meeting.total_meeting_token_budget),
                context_fingerprint: Some(context_fingerprint.into()),
                routing_decision_id: routing_decision_id.map(str::to_owned),
                created_at: Utc::now().to_rfc3339(),
                resolved_at: None,
                resolution_reason: None,
            });
            self.store.upsert_meeting(&persisted)?;
            self.events.publish(
                EventType::ExecutionGateOpened,
                "meeting-engine",
                Some(meeting.id.clone()),
                meeting.linked_task.clone(),
                json!({"meetingId":meeting.id,"decisionId":decision_id,"resourceId":resource_id,"phase":"BEFORE_PROVIDER_TURN"}),
            )?;
        }
        Ok(())
    }

    fn close(&self, meeting: &mut Meeting) -> Result<()> {
        if meeting.result.is_some() || meeting.status == MeetingStatus::Completed {
            return Ok(());
        }
        let turns = self.store.list_meeting_turns(&meeting.id)?;
        let result = deterministic_outcome(meeting, &turns);
        let closure_turn = MeetingTurn {
            id: format!("MTURN-{}-closure", meeting.id),
            meeting_id: meeting.id.clone(),
            round: meeting.current_round.saturating_add(1),
            participant_id: meeting.closure_owner.clone(),
            kind: MeetingTurnKind::Closure,
            status: MeetingTurnStatus::Completed,
            provider: None,
            model: None,
            provider_session_id: None,
            provider_turn_id: None,
            contribution: Some(MeetingContribution {
                position: result.summary.clone(),
                agreements: result.agreements.clone(),
                disagreements: result.disagreements.clone(),
                objections: Vec::new(),
                open_questions: result.unresolved_questions.clone(),
                action_items: result
                    .action_items
                    .iter()
                    .map(|item| item.description.clone())
                    .collect(),
            }),
            usage: UsageSnapshot::default(),
            charged_tokens: 0,
            failure: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: Some(Utc::now().to_rfc3339()),
            completed_at: Some(Utc::now().to_rfc3339()),
            context_id: None,
            context_fingerprint: None,
            estimated_context_tokens: None,
        };
        self.store.upsert_meeting_turn(&closure_turn)?;
        meeting.result = Some(result);
        meeting.usage = aggregate_usage(&turns);
        meeting.status = MeetingStatus::Completed;
        meeting.completed_at = Some(Utc::now().to_rfc3339());
        self.store.upsert_meeting(meeting)?;
        self.write_artifact(meeting)?;
        self.events.publish(
            EventType::MeetingCompleted,
            meeting.closure_owner.clone(),
            Some(meeting.id.clone()),
            meeting.linked_task.clone(),
            json!({"meetingId":meeting.id,"roundsUsed":meeting.current_round,"participants":meeting.participants.len(),"actionItems":meeting.result.as_ref().map(|r|r.action_items.len()).unwrap_or(0)}),
        )?;
        Ok(())
    }

    fn finish_with_error(&self, meeting: &mut Meeting, error: RuntimeError) -> Result<()> {
        if let Some(persisted) = self.store.get_meeting(&meeting.id)? {
            meeting.linked_decision = persisted.linked_decision;
            meeting.context_packages = persisted.context_packages;
            meeting.execution_gates = persisted.execution_gates;
            meeting.payg_rejected = persisted.payg_rejected;
        }
        if matches!(error, RuntimeError::Cancelled(_)) {
            meeting.status = MeetingStatus::Cancelled;
        } else if matches!(
            error,
            RuntimeError::ResourceUnavailable { .. } | RuntimeError::UnknownAfterCrash(_)
        ) || meeting.linked_decision.is_some()
        {
            meeting.status = MeetingStatus::Blocked;
        } else {
            meeting.status = MeetingStatus::Failed;
        }
        meeting.recovery_note = Some(error.to_string());
        meeting.completed_at = meeting.status.terminal().then(|| Utc::now().to_rfc3339());
        self.store.upsert_meeting(meeting)?;
        self.events.publish(
            EventType::MeetingBlocked,
            "meeting-engine",
            Some(meeting.id.clone()),
            meeting.linked_task.clone(),
            json!({"meetingId":meeting.id,"status":meeting.status,"reason":error.to_string()}),
        )?;
        self.cancellations
            .lock()
            .map_err(|_| RuntimeError::Lock("meeting cancellations"))?
            .remove(&meeting.id);
        Ok(())
    }

    fn write_artifact(&self, meeting: &Meeting) -> Result<()> {
        let directory = self.root.join(".batai/meetings");
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{}.json", meeting.id));
        let temporary = directory.join(format!(".{}.tmp", meeting.id));
        let artifact = json!({
            "schemaVersion":1,
            "meetingId":meeting.id,
            "agenda":meeting.agenda,
            "objective":meeting.objective,
            "participants":meeting.participants.iter().map(|p|json!({"agentId":p.agent_id,"name":p.name,"title":p.title})).collect::<Vec<_>>(),
            "result":meeting.result,
            "completedAt":meeting.completed_at
        });
        let mut file = File::create(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(&artifact)?)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    }

    fn authorize_create(&self, actor: &Actor, policy: &MeetingPolicy) -> Result<()> {
        if !policy.meetings_enabled {
            return Err(RuntimeError::Governance(
                "meetings are disabled by project policy".into(),
            ));
        }
        match self.actor_role(actor)? {
            AuthorityRole::God => Ok(()),
            AuthorityRole::Director if policy.director_can_create => Ok(()),
            AuthorityRole::Lead if policy.lead_can_create => Ok(()),
            _ => Err(RuntimeError::Governance(
                "actor is not authorized to create meetings".into(),
            )),
        }
    }

    fn authorize_actor(&self, actor: &Actor) -> Result<()> {
        match self.actor_role(actor)? {
            AuthorityRole::God | AuthorityRole::Director => Ok(()),
            _ => Err(RuntimeError::Governance(
                "actor is not authorized to control meetings".into(),
            )),
        }
    }

    fn actor_role(&self, actor: &Actor) -> Result<AuthorityRole> {
        if actor.id.eq_ignore_ascii_case("god") {
            return Ok(AuthorityRole::God);
        }
        if actor.id.eq_ignore_ascii_case("director") {
            return Ok(AuthorityRole::Director);
        }
        Ok(self
            .agents
            .get(&actor.id)?
            .map(|agent| agent.authority)
            .unwrap_or(AuthorityRole::Worker))
    }

    fn select_participants(
        &self,
        request: &CreateMeetingRequest,
        policy: &MeetingPolicy,
    ) -> Result<Vec<MeetingParticipant>> {
        let agents = self.agents.list()?;
        let by_id = agents
            .iter()
            .map(|agent| (agent.id.as_str(), agent))
            .collect::<HashMap<_, _>>();
        let ids = if !request.participants.is_empty() {
            request.participants.clone()
        } else {
            select_minimum_agents(
                &agents,
                &request.required_roles,
                &request.required_capabilities,
            )?
        };
        if ids.is_empty() {
            return Err(RuntimeError::Governance(
                "meeting requires participants or capability requirements".into(),
            ));
        }
        if ids.len() > policy.max_participants {
            return Err(RuntimeError::Governance(format!(
                "meeting participant limit is {}",
                policy.max_participants
            )));
        }
        let unique = ids.iter().collect::<HashSet<_>>();
        if unique.len() != ids.len() {
            return Err(RuntimeError::Governance(
                "meeting participants must be unique".into(),
            ));
        }
        ids.into_iter()
            .map(|id| {
                let agent = by_id
                    .get(id.as_str())
                    .ok_or_else(|| RuntimeError::AgentNotFound(id.clone()))?;
                if !matches!(agent.status, AgentStatus::Ready | AgentStatus::Created) {
                    return Err(RuntimeError::ResourceUnavailable {
                        agent_id: id,
                        status: agent.status.to_string(),
                        reset_at: None,
                    });
                }
                let normalized = agent.with_backfilled_organization();
                Ok(MeetingParticipant {
                    agent_id: normalized.id.clone(),
                    name: normalized.name.clone(),
                    title: normalized.display_title(),
                    function: normalized
                        .function
                        .unwrap_or(AgentFunction::GenericSoftwareAgent),
                    provider: normalized.provider,
                    model: normalized.model,
                    selected_resource_id: None,
                    routing_decision_id: None,
                })
            })
            .collect()
    }

    fn validate_closure_owner(
        &self,
        closure_owner: &str,
        participants: &[MeetingParticipant],
    ) -> Result<()> {
        let owner = self
            .agents
            .get(closure_owner)?
            .ok_or_else(|| RuntimeError::AgentNotFound(closure_owner.into()))?;
        if participants.iter().any(|p| p.agent_id == closure_owner)
            || matches!(
                owner.authority,
                AuthorityRole::Director | AuthorityRole::God
            )
        {
            Ok(())
        } else {
            Err(RuntimeError::Governance(
                "closure owner must participate or hold Director authority".into(),
            ))
        }
    }

    fn required_meeting(&self, id: &str) -> Result<Meeting> {
        self.store
            .get_meeting(id)?
            .ok_or_else(|| RuntimeError::Governance(format!("meeting not found: {id}")))
    }
}

fn validate_request(request: &CreateMeetingRequest, policy: &MeetingPolicy) -> Result<()> {
    if request.title.trim().is_empty() || request.objective.trim().is_empty() {
        return Err(RuntimeError::Governance(
            "meeting title and objective are required".into(),
        ));
    }
    if request.agenda.is_empty() || request.agenda.iter().any(|item| item.trim().is_empty()) {
        return Err(RuntimeError::Governance(
            "meeting requires a non-empty structured agenda".into(),
        ));
    }
    let rounds = request.max_rounds.unwrap_or(DEFAULT_MAX_ROUNDS);
    if rounds == 0 || rounds > policy.max_rounds {
        return Err(RuntimeError::Governance(format!(
            "meeting rounds must be between 1 and {}",
            policy.max_rounds
        )));
    }
    let response = request
        .per_response_token_limit
        .unwrap_or(DEFAULT_RESPONSE_TOKENS);
    let total = request
        .total_meeting_token_budget
        .unwrap_or(DEFAULT_TOTAL_TOKENS);
    if response == 0
        || response > policy.max_response_tokens
        || total == 0
        || total > policy.max_tokens
    {
        return Err(RuntimeError::Governance(
            "meeting token limits exceed project policy".into(),
        ));
    }
    if request
        .required_capabilities
        .values()
        .any(|score| *score > 100)
    {
        return Err(RuntimeError::Governance(
            "meeting capability requirements must be between 0 and 100".into(),
        ));
    }
    if let Some(context) = &request.context_budget {
        let maximum = &policy.context_budget;
        if context.max_tokens == 0
            || context.max_files == 0
            || context.max_bytes_per_file == 0
            || context.max_total_bytes == 0
            || context.max_tokens > maximum.max_tokens
            || context.max_files > maximum.max_files
            || context.max_bytes_per_file > maximum.max_bytes_per_file
            || context.max_total_bytes > maximum.max_total_bytes
            || context.max_historical_artifacts > maximum.max_historical_artifacts
        {
            return Err(RuntimeError::Governance(
                "meeting context budget exceeds project policy".into(),
            ));
        }
    }
    Ok(())
}

fn select_minimum_agents(
    agents: &[Agent],
    roles: &[AgentFunction],
    capabilities: &BTreeMap<String, u8>,
) -> Result<Vec<String>> {
    let mut selected = Vec::new();
    for role in roles {
        let candidate = agents
            .iter()
            .filter(|agent| matches!(agent.status, AgentStatus::Ready | AgentStatus::Created))
            .filter(|agent| agent.with_backfilled_organization().function == Some(*role))
            .max_by_key(|agent| {
                agent
                    .seniority
                    .map(|value| value.level())
                    .unwrap_or_default()
            })
            .ok_or_else(|| {
                RuntimeError::Governance(format!("no available agent for role {role:?}"))
            })?;
        if !selected.contains(&candidate.id) {
            selected.push(candidate.id.clone());
        }
    }
    for (dimension, minimum) in capabilities {
        let candidate = agents
            .iter()
            .filter(|agent| matches!(agent.status, AgentStatus::Ready | AgentStatus::Created))
            .filter_map(|agent| {
                capability_value(&agent.effective_capabilities, dimension)
                    .map(|score| (agent, score))
            })
            .filter(|(_, score)| score >= minimum)
            .max_by_key(|(_, score)| *score)
            .map(|(agent, _)| agent)
            .ok_or_else(|| {
                RuntimeError::Governance(format!(
                    "no available agent satisfies {dimension} >= {minimum}"
                ))
            })?;
        if !selected.contains(&candidate.id) {
            selected.push(candidate.id.clone());
        }
    }
    Ok(selected)
}

fn capability_value(profile: &CapabilityProfile, dimension: &str) -> Option<u8> {
    match dimension.to_ascii_lowercase().as_str() {
        "coding" => profile.coding,
        "debugging" => profile.debugging,
        "planning" => profile.planning,
        "architecture" => profile.architecture,
        "tool_use" => profile.tool_use,
        "instruction_following" => profile.instruction_following,
        "long_context" => profile.long_context,
        "test_generation" => profile.test_generation,
        "review" => profile.review,
        "research" => profile.research,
        "speed" => profile.speed,
        "reliability" => profile.reliability,
        _ => None,
    }
    .map(|score| score.get())
}

fn meeting_task(
    meeting: &Meeting,
    agent: &Agent,
    round: u8,
    kind: MeetingTurnKind,
    context_package: &ContextPackage,
    peer_context: Option<String>,
) -> Task {
    let phase = if kind == MeetingTurnKind::Position {
        "Give an independent position. Do not assume other participants' views."
    } else {
        "Respond only to the listed disagreements and open questions. Do not start new topics."
    };
    let objective = format!(
        "Bounded Batai meeting contribution. Agenda: {}. Objective: {}. Your organizational role: {} — {}. {} Return concise user-visible JSON with position, agreements, disagreements, objections, open_questions, and action_items. Do not expose hidden reasoning. Do not mutate files. Maximum response tokens: {}.\n\nBOUNDED PROJECT CONTEXT\n{}\n\nROUND-SPECIFIC CONTEXT\n{}",
        meeting.agenda.join(" | "),
        meeting.objective,
        agent.name,
        agent.display_title(),
        phase,
        meeting.per_response_token_limit,
        context_package.render(),
        peer_context.unwrap_or_default()
    );
    let mut extra = serde_json::Map::new();
    extra.insert("meetingId".into(), Value::String(meeting.id.clone()));
    extra.insert("meetingRound".into(), Value::Number(round.into()));
    extra.insert("noFilesystemMutation".into(), Value::Bool(true));
    extra.insert(
        "contextId".into(),
        Value::String(context_package.context_id.clone()),
    );
    extra.insert(
        "contextFingerprint".into(),
        Value::String(context_package.fingerprint.clone()),
    );
    Task {
        id: format!("{}:R{}:{}", meeting.id, round, agent.id),
        created_by: "meeting-coordinator".into(),
        objective,
        assigned_to: vec![agent.id.clone()],
        dependencies: Vec::new(),
        acceptance_criteria: vec!["Return a concise structured coordination contribution".into()],
        inputs: Vec::new(),
        outputs: Vec::new(),
        status: TaskStatus::Running,
        execution: TaskExecution::default(),
        on_success: TaskSuccessAction::default(),
        on_failure: TaskSuccessAction::default(),
        weight: 0.0,
        extra,
    }
}

fn meeting_configuration_fingerprint(meeting: &Meeting) -> String {
    let stable = json!({
        "id": meeting.id,
        "objective": meeting.objective,
        "agenda": meeting.agenda,
        "participants": meeting.participants.iter().map(|participant| &participant.agent_id).collect::<Vec<_>>(),
        "requiredRoles": meeting.required_roles,
        "requiredCapabilities": meeting.required_capabilities,
        "maxRounds": meeting.max_rounds,
        "responseTokens": meeting.per_response_token_limit,
        "totalTokens": meeting.total_meeting_token_budget,
        "contextBudget": meeting.context_budget,
        "explicitFiles": meeting.explicit_files,
    });
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&stable).unwrap_or_default())
    )
}

fn parse_contribution(summary: &str) -> MeetingContribution {
    let cleaned = summary
        .trim()
        .strip_prefix("```json")
        .or_else(|| summary.trim().strip_prefix("```"))
        .unwrap_or(summary.trim())
        .strip_suffix("```")
        .unwrap_or(summary.trim())
        .trim();
    serde_json::from_str(cleaned).unwrap_or_else(|_| MeetingContribution {
        position: truncate(summary, 4_000),
        ..MeetingContribution::default()
    })
}

fn structured_disagreements(turns: &[MeetingTurn]) -> Vec<String> {
    unique_strings(
        turns
            .iter()
            .filter_map(|turn| turn.contribution.as_ref())
            .flat_map(|value| {
                value
                    .disagreements
                    .iter()
                    .chain(value.objections.iter())
                    .cloned()
            }),
    )
}

fn round_two_context(turns: &[MeetingTurn], disagreements: &[String]) -> String {
    let positions = turns
        .iter()
        .filter_map(|turn| {
            turn.contribution
                .as_ref()
                .map(|value| (&turn.participant_id, &value.position))
        })
        .map(|(agent, position)| format!("{agent}: {}", truncate(position, 600)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Concise positions:\n{positions}\nDisagreements/open objections:\n{}",
        disagreements.join("\n")
    )
}

fn deterministic_outcome(meeting: &Meeting, turns: &[MeetingTurn]) -> MeetingOutcome {
    let contributions = turns
        .iter()
        .filter_map(|turn| turn.contribution.as_ref())
        .collect::<Vec<_>>();
    let agreements = unique_strings(
        contributions
            .iter()
            .flat_map(|value| value.agreements.iter().cloned()),
    );
    let disagreements = unique_strings(contributions.iter().flat_map(|value| {
        value
            .disagreements
            .iter()
            .chain(value.objections.iter())
            .cloned()
    }));
    let unresolved = unique_strings(
        contributions
            .iter()
            .flat_map(|value| value.open_questions.iter().cloned()),
    );
    let actions = unique_strings(
        contributions
            .iter()
            .flat_map(|value| value.action_items.iter().cloned()),
    )
    .into_iter()
    .enumerate()
    .map(|(index, description)| MeetingActionItem {
        id: format!("{}-ACTION-{}", meeting.id, index + 1),
        description,
        suggested_owner: None,
        task_id: None,
    })
    .collect::<Vec<_>>();
    let agreement_state = if !disagreements.is_empty() || !unresolved.is_empty() {
        AgreementState::Unresolved
    } else if agreements.is_empty() {
        AgreementState::NotObserved
    } else {
        AgreementState::Unanimous
    };
    let positions = contributions
        .iter()
        .map(|value| value.position.trim())
        .filter(|value| !value.is_empty())
        .take(5)
        .collect::<Vec<_>>();
    MeetingOutcome {
        summary: if positions.is_empty() {
            format!("Meeting completed for: {}", meeting.objective)
        } else {
            positions.join("\n\n")
        },
        agreements,
        disagreements,
        decisions: Vec::new(),
        action_items: actions,
        unresolved_questions: unresolved,
        recommended_next_step: None,
        agreement_state: Some(agreement_state),
    }
}

fn aggregate_usage(turns: &[MeetingTurn]) -> MeetingUsage {
    let mut usage = MeetingUsage::default();
    for turn in turns
        .iter()
        .filter(|turn| turn.status == MeetingTurnStatus::Completed)
    {
        add_optional(&mut usage.input_tokens, turn.usage.input_tokens);
        add_optional(&mut usage.output_tokens, turn.usage.output_tokens);
        if let Some(cost) = turn.usage.cost {
            usage.known_cost = Some(usage.known_cost.unwrap_or_default() + cost);
            if usage.currency.is_none() {
                usage.currency = turn.usage.currency.clone();
            } else if turn.usage.currency.is_some() && usage.currency != turn.usage.currency {
                usage.currency = None;
            }
        }
        let source = match turn.usage.source {
            UsageSource::Local => "LOCAL",
            UsageSource::Subscription => "SUBSCRIPTION",
            UsageSource::Api => "PAYG",
            UsageSource::Unknown => "UNKNOWN",
        };
        *usage.by_source.entry(source.into()).or_default() += turn.charged_tokens;
    }
    usage.total_tokens = usage
        .input_tokens
        .zip(usage.output_tokens)
        .map(|(input, output)| input.saturating_add(output));
    usage
}

fn add_optional(target: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *target = Some(target.unwrap_or_default().saturating_add(value));
    }
}

fn remaining_budget(meeting: &Meeting, turns: &[MeetingTurn]) -> u64 {
    meeting
        .total_meeting_token_budget
        .saturating_sub(turns.iter().map(|turn| turn.charged_tokens).sum::<u64>())
}

fn unique_strings(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .filter_map(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty() && seen.insert(trimmed.to_owned())).then(|| trimmed.to_owned())
        })
        .collect()
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn turn_kind_label(kind: MeetingTurnKind) -> &'static str {
    match kind {
        MeetingTurnKind::Position => "POSITION",
        MeetingTurnKind::Response => "RESPONSE",
        MeetingTurnKind::Closure => "CLOSURE",
    }
}

fn provider_failure_label(failure: &ProviderFailure) -> &'static str {
    match failure {
        ProviderFailure::RateLimited { .. } => "RATE_LIMITED",
        ProviderFailure::AuthRequired => "AUTH_REQUIRED",
        ProviderFailure::Offline => "OFFLINE",
        ProviderFailure::Cancelled => "CANCELLED",
        ProviderFailure::Timeout => "TIMEOUT",
        ProviderFailure::ProcessCrash { .. } => "PROCESS_CRASH",
        ProviderFailure::AppServerUnavailable(_) => "APP_SERVER_UNAVAILABLE",
        ProviderFailure::UnsupportedVersion(_) => "UNSUPPORTED_VERSION",
        ProviderFailure::MalformedResponse(_) => "MALFORMED_RESPONSE",
        ProviderFailure::Execution(_) => "EXECUTION_FAILURE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        economic::{
            QuotaState, ResourceProfile, ResourceTier, ResourceUsage, TermsProfile, TermsState,
        },
        execution_provider::{ExecutionProvider, MockOutcome, MockProvider},
        organization::{CapabilityScore, IntelligencePolicy, Seniority},
    };
    use std::time::Duration;

    fn actor(id: &str) -> Actor {
        Actor {
            id: id.into(),
            scope: if id == "god" {
                AuthorityScope::Global
            } else {
                AuthorityScope::Project
            },
        }
    }

    fn agent(id: &str, function: AgentFunction, authority: AuthorityRole) -> Agent {
        Agent {
            schema_version: 2,
            id: id.into(),
            name: id.into(),
            role_template: function.base_title().into(),
            seniority: Some(if function == AgentFunction::Director {
                Seniority::Director
            } else {
                Seniority::Senior
            }),
            function: Some(function),
            department: Some(function.department()),
            title_override: None,
            model_capabilities: CapabilityProfile::default(),
            effective_capabilities: CapabilityProfile::default(),
            lifecycle: Default::default(),
            authority,
            permissions: Vec::new(),
            intelligence_policy: IntelligencePolicy {
                assignment: ModelAssignment::Explicit,
                ..Default::default()
            },
            parent_agent_id: (id != "director").then(|| "director".into()),
            provider: "mock".into(),
            model: "mock-model".into(),
            reasoning_effort: "medium".into(),
            auth_mode: "test".into(),
            worktree: None,
            status: AgentStatus::Ready,
            current_task_id: None,
            extra: Default::default(),
        }
    }

    fn setup(mock: Arc<MockProvider>) -> Arc<MeetingEngine> {
        let root = std::env::temp_dir().join(format!("batai-meeting-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let store = RuntimeStore::open_memory().unwrap();
        let events = EventEngine::new(store.clone());
        let agents = AgentRegistry::new(store.clone(), events.clone());
        agents
            .register(&agent(
                "director",
                AgentFunction::Director,
                AuthorityRole::Director,
            ))
            .unwrap();
        agents
            .register(&agent(
                "architect",
                AgentFunction::SoftwareArchitecture,
                AuthorityRole::Lead,
            ))
            .unwrap();
        agents
            .register(&agent(
                "backend",
                AgentFunction::BackendEngineering,
                AuthorityRole::Worker,
            ))
            .unwrap();
        agents
            .register(&agent(
                "reviewer",
                AgentFunction::Reviewer,
                AuthorityRole::Worker,
            ))
            .unwrap();
        let mut providers = HashMap::<String, Arc<dyn ExecutionProvider>>::new();
        providers.insert("mock".into(), mock);
        let sessions = SessionManager::new(store.clone(), providers.clone());
        let governance =
            GovernanceService::new(root.clone(), store.clone(), agents.clone(), events.clone());
        let tasks = Arc::new(TaskEngine::new(
            store.clone(),
            events.clone(),
            agents.clone(),
            sessions.clone(),
            Duration::from_secs(10),
        ));
        let context = ContextBuilder::new(root.clone(), store.clone());
        Arc::new(MeetingEngine::new(
            root, store, events, agents, sessions, tasks, governance, context,
        ))
    }

    fn request() -> CreateMeetingRequest {
        CreateMeetingRequest {
            title: "Architecture decision".into(),
            agenda: vec!["Choose a safe boundary".into()],
            objective: "Recommend a bounded design".into(),
            participants: vec!["architect".into(), "backend".into(), "reviewer".into()],
            closure_owner: Some("director".into()),
            ..Default::default()
        }
    }

    fn resource(billing_mode: BillingMode, concurrency_limit: Option<u32>) -> ResourceProfile {
        ResourceProfile {
            id: "mock-resource".into(),
            provider: "mock".into(),
            display_name: "Mock resource".into(),
            tier: if billing_mode == BillingMode::Payg {
                ResourceTier::CheapPayg
            } else {
                ResourceTier::SubscriptionQuota
            },
            billing_mode,
            plan_name: None,
            fixed_monthly_cost: None,
            fixed_cost_currency: None,
            marginal_cost_per_million_tokens: None,
            supported_models: vec!["mock-model".into()],
            capabilities: CapabilityProfile::default(),
            capability_evidence: vec![],
            context_window: Some(8_000),
            concurrency_limit,
            current_concurrency: 0,
            quota: QuotaState::default(),
            usage: ResourceUsage::default(),
            status: "AVAILABLE".into(),
            terms: TermsProfile {
                third_party_allowed: TermsState::Allowed,
                automation_allowed: TermsState::Allowed,
                allowed_use_mode: TermsState::Allowed,
                ..Default::default()
            },
            latency_ms: None,
            reliability: None,
        }
    }

    async fn wait_terminal(engine: &MeetingEngine, id: &str) -> Meeting {
        for _ in 0..100 {
            let meeting = engine.get(id).unwrap().unwrap();
            if meeting.status.terminal() || meeting.status == MeetingStatus::Blocked {
                return meeting;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("meeting did not finish")
    }

    async fn wait_until_completed(engine: &MeetingEngine, id: &str) -> Meeting {
        for _ in 0..200 {
            let meeting = engine.get(id).unwrap().unwrap();
            if meeting.status == MeetingStatus::Completed {
                return meeting;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("meeting did not resume and complete")
    }

    fn enable_payg_meetings(engine: &MeetingEngine) {
        engine
            .store
            .upsert_intelligence_resource(&resource(BillingMode::Payg, Some(3)))
            .unwrap();
        let mut policy = engine.governance.snapshot().unwrap().policy;
        policy.allow_payg = true;
        policy.meetings.allow_payg_for_meetings = true;
        let revision = policy.revision;
        engine
            .governance
            .mutate(MutationRequest {
                actor: actor("god"),
                expected_revision: revision,
                task_id: None,
                reason: Some("test PAYG meeting gate".into()),
                mutation: OrganizationMutation::UpdateProjectPolicy { policy },
            })
            .unwrap();
    }

    #[tokio::test]
    async fn valid_meeting_is_bounded_and_closes_once() {
        let engine = setup(Arc::new(MockProvider::default()));
        let meeting = engine.create(actor("director"), request()).unwrap();
        let completed = wait_terminal(&engine, &meeting.id).await;
        assert_eq!(completed.status, MeetingStatus::Completed);
        assert_eq!(completed.current_round, 1);
        assert_eq!(
            engine
                .turns(&meeting.id)
                .unwrap()
                .iter()
                .filter(|turn| turn.kind == MeetingTurnKind::Closure)
                .count(),
            1
        );
    }

    #[test]
    fn domain_limits_duplicates_missing_agents_and_invalid_owner() {
        let engine = setup(Arc::new(MockProvider::default()));
        let mut duplicate = request();
        duplicate.participants = vec!["backend".into(), "backend".into()];
        assert!(engine.create(actor("director"), duplicate).is_err());
        let mut missing = request();
        missing.participants = vec!["missing".into()];
        assert!(engine.create(actor("director"), missing).is_err());
        let mut owner = request();
        owner.closure_owner = Some("backend".into());
        owner.participants.retain(|id| id != "backend");
        assert!(engine.create(actor("director"), owner).is_err());
        let mut rounds = request();
        rounds.max_rounds = Some(3);
        assert!(engine.create(actor("director"), rounds).is_err());
    }

    #[test]
    fn worker_and_recursive_meeting_are_denied() {
        let engine = setup(Arc::new(MockProvider::default()));
        assert!(engine.create(actor("backend"), request()).is_err());
        let mut recursive = request();
        recursive.trigger = MeetingTrigger::Meeting;
        assert!(engine.create(actor("director"), recursive).is_err());
    }

    #[test]
    fn capability_selection_uses_minimum_sufficient_set() {
        let engine = setup(Arc::new(MockProvider::default()));
        let mut backend = engine.agents.get("backend").unwrap().unwrap();
        backend.effective_capabilities.coding = Some(CapabilityScore::new(82).unwrap());
        engine.agents.register(&backend).unwrap();
        let request = CreateMeetingRequest {
            title: "Backend review".into(),
            agenda: vec!["Review implementation".into()],
            objective: "Find a sufficient backend position".into(),
            required_roles: vec![AgentFunction::BackendEngineering],
            required_capabilities: BTreeMap::from([("coding".into(), 75)]),
            closure_owner: Some("director".into()),
            ..Default::default()
        };
        let selected = engine
            .select_participants(&request, &MeetingPolicy::default())
            .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].agent_id, "backend");
    }

    #[tokio::test]
    async fn structured_disagreement_runs_second_round_without_groupthink() {
        let mock = Arc::new(MockProvider::default());
        for id in ["architect", "backend", "reviewer"] {
            mock.push_outcome(id, MockOutcome::Result(json!({"position":format!("{id} independent"),"disagreements":["database boundary"],"action_items":[]}).to_string()));
            mock.push_outcome(id, MockOutcome::Result(json!({"position":format!("{id} response"),"agreements":["bounded adapter"],"disagreements":[],"action_items":[]}).to_string()));
        }
        let engine = setup(mock);
        let meeting = engine.create(actor("director"), request()).unwrap();
        let completed = wait_terminal(&engine, &meeting.id).await;
        assert_eq!(completed.current_round, 2);
        let turns = engine.turns(&meeting.id).unwrap();
        assert_eq!(
            turns
                .iter()
                .filter(|turn| turn.kind == MeetingTurnKind::Position)
                .count(),
            3
        );
        assert_eq!(
            turns
                .iter()
                .filter(|turn| turn.kind == MeetingTurnKind::Response)
                .count(),
            3
        );
    }

    #[tokio::test]
    async fn rate_limit_blocks_without_quality_evidence_or_replay() {
        let mock = Arc::new(MockProvider::default());
        mock.push_outcome("backend", MockOutcome::RateLimited(None));
        let engine = setup(mock);
        let meeting = engine.create(actor("director"), request()).unwrap();
        let blocked = wait_terminal(&engine, &meeting.id).await;
        assert_eq!(blocked.status, MeetingStatus::Blocked);
        assert!(engine
            .turns(&meeting.id)
            .unwrap()
            .iter()
            .any(|turn| turn.failure.as_deref() == Some("RATE_LIMITED")));
    }

    #[tokio::test]
    async fn provider_concurrency_is_bounded_and_payg_routes_one_god_decision() {
        let engine = setup(Arc::new(MockProvider::default()));
        engine
            .store
            .upsert_intelligence_resource(&resource(BillingMode::SubscriptionQuota, Some(1)))
            .unwrap();
        let backend = engine.agents.get("backend").unwrap().unwrap();
        let lane = engine.provider_lane(&backend, None).unwrap().unwrap();
        let permit = lane.clone().try_acquire_owned().unwrap();
        assert!(lane.clone().try_acquire_owned().is_err());
        drop(permit);
        assert!(lane.try_acquire_owned().is_ok());

        enable_payg_meetings(&engine);
        let meeting = engine.create(actor("director"), request()).unwrap();
        let blocked = wait_terminal(&engine, &meeting.id).await;
        assert_eq!(blocked.status, MeetingStatus::Blocked);
        assert!(blocked.linked_decision.is_some());
        assert_eq!(
            engine
                .governance
                .snapshot()
                .unwrap()
                .decisions
                .into_iter()
                .filter(|decision| decision.status == super::super::governance::DecisionStatus::Open)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn exact_payg_approval_resumes_and_does_not_duplicate_turns() {
        let engine = setup(Arc::new(MockProvider::default()));
        enable_payg_meetings(&engine);
        engine.start_event_listener().unwrap();
        let meeting = engine.create(actor("director"), request()).unwrap();
        let blocked = wait_terminal(&engine, &meeting.id).await;
        let decision_id = blocked.linked_decision.clone().unwrap();
        assert_eq!(blocked.execution_gates.len(), 1);
        engine
            .governance
            .resolve_decision(&decision_id, true, Some("bounded meeting only".into()))
            .unwrap();
        let completed = wait_until_completed(&engine, &meeting.id).await;
        assert_eq!(completed.status, MeetingStatus::Completed);
        let turns = engine.turns(&meeting.id).unwrap();
        assert_eq!(
            turns
                .iter()
                .filter(|turn| turn.kind == MeetingTurnKind::Position)
                .count(),
            3
        );
        assert!(completed
            .execution_gates
            .iter()
            .any(|gate| gate.status == ExecutionGateStatus::Approved));
    }

    #[tokio::test]
    async fn rejected_payg_reroutes_only_after_payg_is_disabled() {
        let engine = setup(Arc::new(MockProvider::default()));
        enable_payg_meetings(&engine);
        engine.start_event_listener().unwrap();
        let meeting = engine.create(actor("director"), request()).unwrap();
        let blocked = wait_terminal(&engine, &meeting.id).await;
        let decision_id = blocked.linked_decision.clone().unwrap();
        engine
            .store
            .upsert_intelligence_resource(&resource(BillingMode::SubscriptionQuota, Some(3)))
            .unwrap();
        engine
            .governance
            .resolve_decision(&decision_id, false, Some("use subscription".into()))
            .unwrap();
        let completed = wait_until_completed(&engine, &meeting.id).await;
        assert_eq!(completed.status, MeetingStatus::Completed);
        assert!(completed.payg_rejected);
        assert!(completed
            .execution_gates
            .iter()
            .any(|gate| gate.status == ExecutionGateStatus::Rejected));
    }

    #[tokio::test]
    async fn approved_before_restart_is_reconciled_and_resumed_once() {
        let engine = setup(Arc::new(MockProvider::default()));
        enable_payg_meetings(&engine);
        let meeting = engine.create(actor("director"), request()).unwrap();
        let blocked = wait_terminal(&engine, &meeting.id).await;
        let decision_id = blocked.linked_decision.unwrap();
        engine
            .governance
            .resolve_decision(&decision_id, true, None)
            .unwrap();
        engine.start_event_listener().unwrap();
        engine.reconcile_startup().unwrap();
        let completed = wait_until_completed(&engine, &meeting.id).await;
        assert_eq!(completed.status, MeetingStatus::Completed);
        assert_eq!(
            engine
                .turns(&meeting.id)
                .unwrap()
                .iter()
                .filter(|turn| turn.kind == MeetingTurnKind::Position)
                .count(),
            3
        );
    }

    #[tokio::test]
    async fn changed_meeting_configuration_supersedes_old_approval() {
        let engine = setup(Arc::new(MockProvider::default()));
        enable_payg_meetings(&engine);
        engine.start_event_listener().unwrap();
        let meeting = engine.create(actor("director"), request()).unwrap();
        let mut blocked = wait_terminal(&engine, &meeting.id).await;
        let decision_id = blocked.linked_decision.clone().unwrap();
        blocked.objective = "Changed while approval was pending".into();
        engine.store.upsert_meeting(&blocked).unwrap();
        engine
            .governance
            .resolve_decision(&decision_id, true, None)
            .unwrap();
        for _ in 0..200 {
            let current = engine.get(&meeting.id).unwrap().unwrap();
            if current.execution_gates.len() >= 2 && current.status == MeetingStatus::Blocked {
                assert_eq!(
                    current.execution_gates[0].status,
                    ExecutionGateStatus::Superseded
                );
                assert_eq!(
                    current.execution_gates[1].status,
                    ExecutionGateStatus::Waiting
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("stale approval was not re-evaluated")
    }

    #[tokio::test]
    async fn policy_change_supersedes_open_payg_decision_and_reroutes() {
        let engine = setup(Arc::new(MockProvider::default()));
        enable_payg_meetings(&engine);
        engine.start_event_listener().unwrap();
        let meeting = engine.create(actor("director"), request()).unwrap();
        let blocked = wait_terminal(&engine, &meeting.id).await;
        let decision_id = blocked.linked_decision.clone().unwrap();

        let snapshot = engine.governance.snapshot().unwrap();
        let mut policy = snapshot.policy;
        policy.allow_payg = false;
        policy.meetings.allow_payg_for_meetings = false;
        engine
            .governance
            .mutate(MutationRequest {
                actor: actor("god"),
                expected_revision: snapshot.revision,
                task_id: None,
                reason: Some("disable PAYG while a meeting gate is open".into()),
                mutation: OrganizationMutation::UpdateProjectPolicy { policy },
            })
            .unwrap();

        for _ in 0..200 {
            let current = engine.get(&meeting.id).unwrap().unwrap();
            let decision = engine
                .governance
                .snapshot()
                .unwrap()
                .decisions
                .into_iter()
                .find(|decision| decision.id == decision_id)
                .unwrap();
            if current.execution_gates[0].status == ExecutionGateStatus::Superseded
                && decision.status == DecisionStatus::Superseded
                && current.status == MeetingStatus::Blocked
            {
                assert!(!current.payg_rejected);
                assert_eq!(current.execution_gates.len(), 1);
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("policy change did not supersede and safely re-evaluate the PAYG gate")
    }

    #[tokio::test]
    async fn uncertain_restart_blocks_and_completed_participant_is_not_rerun() {
        let engine = setup(Arc::new(MockProvider::default()));
        let mut meeting = Meeting {
            id: "MTG-recovery".into(),
            title: "Recovery".into(),
            agenda: vec!["Recover".into()],
            objective: "Do not replay".into(),
            organizer: "director".into(),
            participants: vec![MeetingParticipant {
                agent_id: "backend".into(),
                name: "backend".into(),
                title: "Senior Backend Engineer".into(),
                function: AgentFunction::BackendEngineering,
                provider: "mock".into(),
                model: "mock-model".into(),
                selected_resource_id: None,
                routing_decision_id: None,
            }],
            required_roles: vec![],
            required_capabilities: BTreeMap::new(),
            closure_owner: "director".into(),
            max_rounds: 2,
            per_response_token_limit: 800,
            total_meeting_token_budget: 8000,
            status: MeetingStatus::Running,
            current_round: 1,
            created_at: Utc::now().to_rfc3339(),
            started_at: Some(Utc::now().to_rfc3339()),
            completed_at: None,
            linked_task: None,
            linked_decision: None,
            trigger: MeetingTrigger::Manual,
            result: None,
            usage: MeetingUsage::default(),
            recovery_note: None,
            explicit_files: Vec::new(),
            context_budget: ContextBudget::default(),
            context_packages: Vec::new(),
            execution_gates: Vec::new(),
            payg_rejected: false,
        };
        engine.store.upsert_meeting(&meeting).unwrap();
        let turn = MeetingTurn {
            id: "turn".into(),
            meeting_id: meeting.id.clone(),
            round: 1,
            participant_id: "backend".into(),
            kind: MeetingTurnKind::Position,
            status: MeetingTurnStatus::Running,
            provider: Some("mock".into()),
            model: Some("mock-model".into()),
            provider_session_id: Some("s".into()),
            provider_turn_id: None,
            contribution: None,
            usage: UsageSnapshot::default(),
            charged_tokens: 0,
            failure: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: Some(Utc::now().to_rfc3339()),
            completed_at: None,
            context_id: None,
            context_fingerprint: None,
            estimated_context_tokens: None,
        };
        engine.store.upsert_meeting_turn(&turn).unwrap();
        engine.reconcile_startup().unwrap();
        meeting = engine.get(&meeting.id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Blocked);
        assert_eq!(
            engine.turns(&meeting.id).unwrap()[0].status,
            MeetingTurnStatus::UnknownAfterCrash
        );
    }

    #[tokio::test]
    async fn cancellation_keeps_partial_outputs_non_final() {
        let mock = Arc::new(MockProvider::default());
        for id in ["architect", "backend", "reviewer"] {
            mock.push_outcome(id, MockOutcome::Delay(Duration::from_secs(1)));
        }
        let engine = setup(mock);
        let meeting = engine.create(actor("director"), request()).unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        let cancelled = engine.cancel(actor("director"), &meeting.id).await.unwrap();
        assert_eq!(cancelled.status, MeetingStatus::Cancelled);
        assert!(cancelled.result.is_none());
    }

    #[tokio::test]
    async fn review_escalation_action_creates_one_linked_task() {
        let mock = Arc::new(MockProvider::default());
        mock.push_outcome(
            "backend",
            MockOutcome::Result(
                json!({
                    "position":"Apply the requested correction",
                    "agreements":["keep the existing boundary"],
                    "disagreements":[],
                    "actionItems":["Add the missing regression test"]
                })
                .to_string(),
            ),
        );
        let engine = setup(mock);
        let linked = Task {
            id: "TASK-review".into(),
            created_by: "director".into(),
            objective: "Resolve repeated review feedback".into(),
            assigned_to: vec!["backend".into()],
            dependencies: vec![],
            acceptance_criteria: vec![],
            inputs: vec![],
            outputs: vec![],
            status: TaskStatus::Review,
            execution: TaskExecution::default(),
            on_success: TaskSuccessAction::default(),
            on_failure: TaskSuccessAction::default(),
            weight: 1.0,
            extra: Default::default(),
        };
        engine
            .store
            .ingest_task(&linked, None, "linked-task")
            .unwrap();
        let mut meeting_request = request();
        meeting_request.participants = vec!["backend".into()];
        meeting_request.trigger = MeetingTrigger::ReviewEscalation;
        meeting_request.linked_task = Some(linked.id.clone());
        let meeting = engine.create(actor("director"), meeting_request).unwrap();
        let completed = wait_terminal(&engine, &meeting.id).await;
        assert_eq!(completed.linked_task.as_deref(), Some("TASK-review"));
        let action_id = completed.result.unwrap().action_items[0].id.clone();
        let first = engine
            .create_action_task(
                actor("director"),
                &meeting.id,
                &action_id,
                vec!["backend".into()],
            )
            .await
            .unwrap();
        let second = engine
            .create_action_task(
                actor("director"),
                &meeting.id,
                &action_id,
                vec!["backend".into()],
            )
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.extra["meetingId"], meeting.id);
    }
}
