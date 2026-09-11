use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use super::{
    agents::AgentRegistry,
    errors::{Result, RuntimeError},
    events::EventEngine,
    execution_provider::{
        ProviderApprovalDecision, ProviderApprovalDirective, ProviderApprovalHandler,
        ProviderApprovalRequest,
    },
    organization::{
        hierarchy_warnings, AgentFunction, AgentLifecycle, AgentPermission, AuthorityRole,
        Department, IntelligencePolicy, OrganizationRelationship, RelationshipType, Seniority,
    },
    recovery::{
        agent_db_entity, json_bytes, OperationJournal, RecoveryAction, RecoveryEngine,
        RecoverySummary,
    },
    store::{GovernanceAuditRow, RuntimeStore},
    types::{Agent, AgentStatus, EventType},
};

const AGENT_NAMES: [&str; 12] = [
    "Nova", "Atlas", "Iris", "Mira", "Orion", "Lyra", "Vega", "Sage", "Echo", "Aster", "Coda",
    "Rune",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AuthorityScope {
    Global,
    Project,
    Task,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MutationDisposition {
    Applied,
    Denied,
    PendingGodDecision,
    Conflict,
    Recovered,
    RolledBack,
    RecoveryRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DecisionStatus {
    Open,
    Approved,
    Rejected,
    Superseded,
    Cancelled,
}

impl DecisionStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "OPEN",
            Self::Approved => "APPROVED",
            Self::Rejected => "REJECTED",
            Self::Superseded => "SUPERSEDED",
            Self::Cancelled => "CANCELLED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProviderApprovalStatus {
    Received,
    PendingGod,
    Approved,
    Denied,
    Responded,
    Resolved,
    Expired,
    Cancelled,
    Orphaned,
    ResponseUncertain,
}

impl ProviderApprovalStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Received => "RECEIVED",
            Self::PendingGod => "PENDING_GOD",
            Self::Approved => "APPROVED",
            Self::Denied => "DENIED",
            Self::Responded => "RESPONDED",
            Self::Resolved => "RESOLVED",
            Self::Expired => "EXPIRED",
            Self::Cancelled => "CANCELLED",
            Self::Orphaned => "ORPHANED",
            Self::ResponseUncertain => "RESPONSE_UNCERTAIN",
        }
    }

    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Resolved
                | Self::Expired
                | Self::Cancelled
                | Self::Orphaned
                | Self::ResponseUncertain
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewOutcomeKind {
    Accepted,
    ChangesRequested,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MeetingStatus {
    Proposed,
    Active,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProtectedOperation {
    ProductionDeployment,
    CreateGithubIssue,
    CreatePullRequest,
    PaygSpend,
    ModifyAuthority,
    DeleteAudit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OrganizationLimits {
    #[serde(default = "default_max_active", alias = "max_active_agents")]
    pub max_active_agents: usize,
    #[serde(default = "default_max_depth", alias = "max_hierarchy_depth")]
    pub max_hierarchy_depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProjectGovernancePolicy {
    pub schema_version: u32,
    pub revision: u64,
    pub limits: OrganizationLimits,
    pub allow_payg: bool,
    pub auto_agent_creation: bool,
    pub permanent_agents_require_god: bool,
    pub allowed_providers: Vec<String>,
    pub denied_providers: Vec<String>,
    pub production_deploy_requires_god: bool,
    #[serde(default = "default_provider_approval_timeout")]
    pub provider_approval_timeout_seconds: u64,
    #[serde(flatten)]
    pub legacy: serde_json::Map<String, Value>,
}

impl Default for ProjectGovernancePolicy {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            limits: OrganizationLimits::default(),
            allow_payg: false,
            auto_agent_creation: false,
            permanent_agents_require_god: true,
            allowed_providers: Vec::new(),
            denied_providers: Vec::new(),
            production_deploy_requires_god: true,
            provider_approval_timeout_seconds: default_provider_approval_timeout(),
            legacy: serde_json::Map::new(),
        }
    }
}

impl Default for OrganizationLimits {
    fn default() -> Self {
        Self {
            max_active_agents: default_max_active(),
            max_hierarchy_depth: default_max_depth(),
        }
    }
}

fn default_max_active() -> usize {
    8
}

fn default_max_depth() -> usize {
    3
}

fn default_provider_approval_timeout() -> u64 {
    300
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OrganizationDocument {
    pub schema_version: u32,
    pub revision: u64,
    pub project: Option<String>,
    pub god: Option<String>,
    pub director: Option<String>,
    pub hierarchy: HashMap<String, Vec<String>>,
    pub limits: OrganizationLimits,
    pub relationships: Vec<OrganizationRelationship>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Actor {
    pub id: String,
    pub scope: AuthorityScope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase"
)]
pub enum OrganizationMutation {
    CreateAgent(CreateAgentRequest),
    UpdateIdentity {
        agent_id: String,
        name: Option<String>,
        title_override: Option<String>,
    },
    ChangeReportingLine {
        agent_id: String,
        reports_to: Option<String>,
    },
    ChangeRole {
        agent_id: String,
        seniority: Seniority,
        function: AgentFunction,
        department: Department,
    },
    ChangeProviderPolicy {
        agent_id: String,
        provider: String,
        model: String,
        intelligence_policy: IntelligencePolicy,
    },
    AddRelationship {
        relationship: OrganizationRelationship,
    },
    RemoveRelationship {
        relationship_id: String,
    },
    PauseAgent {
        agent_id: String,
    },
    ResumeAgent {
        agent_id: String,
    },
    TerminateAgent {
        agent_id: String,
        reason: String,
    },
    UpdateProjectPolicy {
        policy: ProjectGovernancePolicy,
    },
    RequestProtectedAction {
        operation: ProtectedOperation,
        target: String,
        detail: Value,
    },
}

impl OrganizationMutation {
    pub fn action(&self) -> &'static str {
        match self {
            Self::CreateAgent(_) => "CREATE_AGENT",
            Self::UpdateIdentity { .. } => "UPDATE_AGENT_IDENTITY",
            Self::ChangeReportingLine { .. } => "CHANGE_REPORTING_LINE",
            Self::ChangeRole { .. } => "CHANGE_AGENT_ROLE",
            Self::ChangeProviderPolicy { .. } => "CHANGE_PROVIDER_POLICY",
            Self::AddRelationship { .. } => "ADD_RELATIONSHIP",
            Self::RemoveRelationship { .. } => "REMOVE_RELATIONSHIP",
            Self::PauseAgent { .. } => "PAUSE_AGENT",
            Self::ResumeAgent { .. } => "RESUME_AGENT",
            Self::TerminateAgent { .. } => "TERMINATE_AGENT",
            Self::UpdateProjectPolicy { .. } => "UPDATE_PROJECT_POLICY",
            Self::RequestProtectedAction { .. } => "REQUEST_PROTECTED_ACTION",
        }
    }

    pub fn target(&self) -> Option<&str> {
        match self {
            Self::CreateAgent(request) => request.id.as_deref(),
            Self::UpdateIdentity { agent_id, .. }
            | Self::ChangeReportingLine { agent_id, .. }
            | Self::ChangeRole { agent_id, .. }
            | Self::ChangeProviderPolicy { agent_id, .. }
            | Self::PauseAgent { agent_id }
            | Self::ResumeAgent { agent_id }
            | Self::TerminateAgent { agent_id, .. } => Some(agent_id),
            Self::AddRelationship { relationship } => Some(&relationship.id),
            Self::RemoveRelationship { relationship_id } => Some(relationship_id),
            Self::UpdateProjectPolicy { .. } => Some("project-policy"),
            Self::RequestProtectedAction { target, .. } => Some(target),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationRequest {
    pub actor: Actor,
    pub expected_revision: u64,
    pub task_id: Option<String>,
    pub reason: Option<String>,
    pub mutation: OrganizationMutation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationResult {
    pub disposition: MutationDisposition,
    pub revision: u64,
    pub decision_id: Option<String>,
    pub message: String,
    pub affected_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentRequest {
    pub id: Option<String>,
    pub name: Option<String>,
    pub seniority: Seniority,
    pub function: AgentFunction,
    pub department: Department,
    pub reports_to: Option<String>,
    pub lifecycle: AgentLifecycle,
    #[serde(default)]
    pub authority: AuthorityRole,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub reasoning_effort: String,
    #[serde(default)]
    pub permissions: Vec<AgentPermission>,
    #[serde(default)]
    pub intelligence_policy: IntelligencePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GodDecision {
    pub id: String,
    pub status: DecisionStatus,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub requested_by: String,
    pub question: String,
    pub impact: String,
    pub request: MutationRequest,
    pub resolution_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecord {
    pub id: String,
    pub timestamp: String,
    pub actor: String,
    pub authority: AuthorityRole,
    pub action: String,
    pub target: Option<String>,
    pub outcome: MutationDisposition,
    pub reason: Option<String>,
    pub decision_id: Option<String>,
    pub task_id: Option<String>,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderApproval {
    pub id: String,
    pub provider: String,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
    #[serde(default)]
    pub process_id: Option<u32>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub worktree: Option<String>,
    pub operation: String,
    #[serde(default)]
    pub requested_target: Option<String>,
    #[serde(default)]
    pub risk: Option<String>,
    pub detail: Value,
    pub status: ProviderApprovalStatus,
    pub created_at: String,
    #[serde(default)]
    pub expires_at: Option<String>,
    pub resolved_at: Option<String>,
    pub resolved_by: Option<String>,
    #[serde(default)]
    pub responded_at: Option<String>,
    #[serde(default)]
    pub decision: Option<ProviderApprovalDecision>,
    #[serde(default)]
    pub response_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewOutcome {
    pub id: String,
    pub task_id: String,
    pub reviewer_id: String,
    pub subject_agent_id: String,
    pub outcome: ReviewOutcomeKind,
    pub note: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskHandoff {
    pub id: String,
    pub task_id: String,
    pub from_agent_id: String,
    pub to_agent_id: String,
    pub summary: String,
    pub changed_files: Vec<String>,
    pub next_actions: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationMeeting {
    pub id: String,
    pub title: String,
    pub participants: Vec<String>,
    pub status: MeetingStatus,
    pub agenda: Vec<String>,
    pub outcomes: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceSnapshot {
    pub revision: u64,
    pub policy: ProjectGovernancePolicy,
    pub decisions: Vec<GodDecision>,
    pub provider_approvals: Vec<ProviderApproval>,
    pub audit: Vec<AuditRecord>,
    pub recovery_operations: Vec<OperationJournal>,
    pub recovery_requires_review: usize,
}

#[derive(Clone)]
struct LiveApproval {
    session_id: String,
    sender: watch::Sender<Option<ProviderApprovalDecision>>,
}

#[derive(Clone)]
pub struct GovernanceService {
    root: PathBuf,
    store: RuntimeStore,
    agents: AgentRegistry,
    events: EventEngine,
    mutation_lock: Arc<Mutex<()>>,
    live_approvals: Arc<Mutex<HashMap<String, LiveApproval>>>,
    interactive_approvals: Arc<AtomicBool>,
    approval_shutdown: watch::Sender<bool>,
    recovery: RecoveryEngine,
}

impl GovernanceService {
    pub fn new(
        root: PathBuf,
        store: RuntimeStore,
        agents: AgentRegistry,
        events: EventEngine,
    ) -> Self {
        let (approval_shutdown, _) = watch::channel(false);
        Self {
            root: root.clone(),
            recovery: RecoveryEngine::new(root.clone(), store.clone(), events.clone()),
            store,
            agents,
            events,
            mutation_lock: Arc::new(Mutex::new(())),
            live_approvals: Arc::new(Mutex::new(HashMap::new())),
            interactive_approvals: Arc::new(AtomicBool::new(false)),
            approval_shutdown,
        }
    }

    pub fn set_interactive_approvals(&self, enabled: bool) {
        self.interactive_approvals.store(enabled, Ordering::Release);
    }

    pub fn snapshot(&self) -> Result<GovernanceSnapshot> {
        let organization = self.load_organization()?;
        Ok(GovernanceSnapshot {
            revision: organization.revision,
            policy: self.load_policy()?,
            decisions: self.store.list_governance_records("GOD_DECISION")?,
            provider_approvals: self.store.list_governance_records("PROVIDER_APPROVAL")?,
            audit: self.store.list_governance_audit(150)?,
            recovery_operations: self.store.list_operation_journals(50)?,
            recovery_requires_review: self.store.count_operation_phase("NEEDS_REVIEW")?,
        })
    }

    pub fn persistent_relationships(&self) -> Result<Vec<OrganizationRelationship>> {
        Ok(self.load_organization()?.relationships)
    }

    pub fn refresh_external_path(&self, path: &Path) -> Result<()> {
        if self.recovery.path_in_flight(path)? {
            return Ok(());
        }
        let file_name = path.file_name().and_then(|name| name.to_str());
        match file_name {
            Some("config.json") => {
                let agent: Agent = serde_json::from_slice(&fs::read(path)?)?;
                let policy = self.load_policy()?;
                self.validate_agent_change(&agent, Some(&agent.id), &policy)?;
                self.agents.register(&agent)?;
            }
            Some("organization.json") => {
                let organization = self.load_organization()?;
                let agents = self.agents.list()?;
                for relationship in &organization.relationships {
                    validate_persistent_relationship(relationship, &agents)?;
                }
            }
            Some("policies.json") => validate_policy(&self.load_policy()?)?,
            _ => return Ok(()),
        }
        self.events.publish(
            EventType::OrganizationChanged,
            "filesystem-watcher",
            None,
            None,
            serde_json::json!({"path":path,"external":true}),
        )?;
        Ok(())
    }

    pub fn mutate(&self, request: MutationRequest) -> Result<MutationResult> {
        let _guard = self
            .mutation_lock
            .lock()
            .map_err(|_| RuntimeError::Lock("governance mutation"))?;
        self.mutate_locked(request, None)
    }

    fn mutate_locked(
        &self,
        request: MutationRequest,
        god_decision_id: Option<&str>,
    ) -> Result<MutationResult> {
        if self.recovery.has_blocking_operations()? {
            return Err(RuntimeError::Governance(
                "an unfinished recovery must resolve before organization changes can continue"
                    .into(),
            ));
        }
        let organization = self.load_organization()?;
        let authority = if god_decision_id.is_some() {
            AuthorityRole::God
        } else {
            self.resolve_authority(&request.actor.id)?
        };
        if request.expected_revision != organization.revision {
            self.audit(
                &request,
                authority,
                MutationDisposition::Conflict,
                None,
                organization.revision,
            )?;
            return Err(RuntimeError::OrganizationConflict {
                expected: request.expected_revision,
                current: organization.revision,
            });
        }
        let policy = self.load_policy()?;
        if let Err(error) = self.enforce_system_policy(&request, &policy) {
            self.audit(
                &request,
                authority,
                MutationDisposition::Denied,
                None,
                organization.revision,
            )?;
            return Err(error);
        }
        match self.route(authority, &request)? {
            AuthorityRoute::Deny(message) => {
                self.audit(
                    &request,
                    authority,
                    MutationDisposition::Denied,
                    None,
                    organization.revision,
                )?;
                Ok(MutationResult {
                    disposition: MutationDisposition::Denied,
                    revision: organization.revision,
                    decision_id: None,
                    message,
                    affected_id: request.mutation.target().map(str::to_owned),
                })
            }
            AuthorityRoute::RequireGod(question) => {
                let decision = self.create_decision(request.clone(), question)?;
                self.audit(
                    &request,
                    authority,
                    MutationDisposition::PendingGodDecision,
                    Some(&decision.id),
                    organization.revision,
                )?;
                self.events.publish(
                    EventType::GodDecisionRequired,
                    request.actor.id,
                    Some(decision.id.clone()),
                    request.task_id,
                    serde_json::to_value(&decision)?,
                )?;
                Ok(MutationResult {
                    disposition: MutationDisposition::PendingGodDecision,
                    revision: organization.revision,
                    decision_id: Some(decision.id),
                    message: "GOD decision required; no mutation was applied".into(),
                    affected_id: request.mutation.target().map(str::to_owned),
                })
            }
            AuthorityRoute::Allow => {
                let revision = organization.revision;
                match self.apply(
                    request.clone(),
                    authority,
                    organization,
                    policy,
                    god_decision_id,
                ) {
                    Ok(result) => Ok(result),
                    Err(error) => {
                        let disposition = if self.recovery.has_blocking_operations()? {
                            MutationDisposition::RecoveryRequired
                        } else {
                            MutationDisposition::Denied
                        };
                        self.audit(&request, authority, disposition, None, revision)?;
                        Err(error)
                    }
                }
            }
        }
    }

    pub fn resolve_decision(
        &self,
        decision_id: &str,
        approve: bool,
        note: Option<String>,
    ) -> Result<MutationResult> {
        let _guard = self
            .mutation_lock
            .lock()
            .map_err(|_| RuntimeError::Lock("governance mutation"))?;
        let mut decisions: Vec<GodDecision> = self.store.list_governance_records("GOD_DECISION")?;
        let mut decision = decisions
            .drain(..)
            .find(|decision| decision.id == decision_id)
            .ok_or_else(|| {
                RuntimeError::Governance(format!("decision not found: {decision_id}"))
            })?;
        if decision.status != DecisionStatus::Open {
            return Ok(MutationResult {
                disposition: if decision.status == DecisionStatus::Approved {
                    MutationDisposition::Applied
                } else {
                    MutationDisposition::Denied
                },
                revision: self.load_organization()?.revision,
                decision_id: Some(decision.id),
                message: "decision was already resolved; no duplicate mutation ran".into(),
                affected_id: decision.request.mutation.target().map(str::to_owned),
            });
        }
        if !approve {
            decision.status = DecisionStatus::Rejected;
            decision.resolved_at = Some(now());
            decision.resolution_note = note;
            self.persist_decision(&decision)?;
            self.events.publish(
                EventType::DecisionResolved,
                "god",
                Some(decision.id.clone()),
                decision.request.task_id.clone(),
                serde_json::json!({"status":"REJECTED"}),
            )?;
            self.audit(
                &decision.request,
                AuthorityRole::God,
                MutationDisposition::Denied,
                Some(&decision.id),
                self.load_organization()?.revision,
            )?;
            return Ok(MutationResult {
                disposition: MutationDisposition::Denied,
                revision: self.load_organization()?.revision,
                decision_id: Some(decision.id),
                message: "decision rejected; no mutation was applied".into(),
                affected_id: decision.request.mutation.target().map(str::to_owned),
            });
        }
        let result = self.mutate_locked(decision.request.clone(), Some(&decision.id))?;
        decision.status = DecisionStatus::Approved;
        decision.resolved_at = Some(now());
        decision.resolution_note = note;
        self.persist_decision(&decision)?;
        self.events.publish(
            EventType::DecisionResolved,
            "god",
            Some(decision.id.clone()),
            decision.request.task_id,
            serde_json::json!({"status":"APPROVED","revision":result.revision}),
        )?;
        Ok(MutationResult {
            decision_id: Some(decision.id),
            ..result
        })
    }

    pub fn request_provider_approval(
        &self,
        provider: String,
        agent_id: Option<String>,
        task_id: Option<String>,
        operation: String,
        detail: Value,
    ) -> Result<ProviderApproval> {
        let approval = ProviderApproval {
            id: format!("APR-{}", uuid::Uuid::new_v4()),
            provider,
            agent_id,
            task_id: task_id.clone(),
            process_id: None,
            session_id: None,
            thread_id: None,
            turn_id: None,
            request_id: None,
            worktree: None,
            operation,
            requested_target: None,
            risk: None,
            detail: redact_provider_detail(detail),
            status: ProviderApprovalStatus::PendingGod,
            created_at: now(),
            expires_at: None,
            resolved_at: None,
            resolved_by: None,
            responded_at: None,
            decision: None,
            response_error: None,
        };
        self.persist_provider_approval(&approval)?;
        self.events.publish(
            EventType::ProviderApprovalRequested,
            approval.provider.clone(),
            Some(approval.id.clone()),
            task_id,
            serde_json::to_value(&approval)?,
        )?;
        Ok(approval)
    }

    pub fn resolve_provider_approval(
        &self,
        approval_id: &str,
        approve: bool,
    ) -> Result<ProviderApproval> {
        let mut approvals: Vec<ProviderApproval> =
            self.store.list_governance_records("PROVIDER_APPROVAL")?;
        let mut approval = approvals
            .drain(..)
            .find(|approval| approval.id == approval_id)
            .ok_or_else(|| {
                RuntimeError::Governance(format!("approval not found: {approval_id}"))
            })?;
        let decision = if approve {
            ProviderApprovalDecision::AllowOnce
        } else {
            ProviderApprovalDecision::Deny
        };
        if approval.status == ProviderApprovalStatus::PendingGod {
            let live = self
                .live_approvals
                .lock()
                .map_err(|_| RuntimeError::Lock("live approvals"))?
                .get(approval_id)
                .cloned()
                .ok_or_else(|| {
                    RuntimeError::Governance(
                        "approval is no longer bound to a live provider request".into(),
                    )
                })?;
            approval.status = if approve {
                ProviderApprovalStatus::Approved
            } else {
                ProviderApprovalStatus::Denied
            };
            approval.resolved_at = Some(now());
            approval.resolved_by = Some("god".into());
            approval.decision = Some(decision);
            self.persist_provider_approval(&approval)?;
            let _ = live.sender.send(Some(decision));
            self.events.publish(
                if approve {
                    EventType::ProviderApprovalApproved
                } else {
                    EventType::ProviderApprovalDenied
                },
                "god",
                Some(approval.id.clone()),
                approval.task_id.clone(),
                serde_json::to_value(&approval)?,
            )?;
        } else if approval.decision == Some(decision) {
            return Ok(approval);
        } else {
            return Err(RuntimeError::Governance(format!(
                "approval {} is stale or already resolved as {}",
                approval.id,
                approval.status.label()
            )));
        }
        Ok(approval)
    }

    pub fn record_provider_auto_denial(&self, provider: &str, detail: Value) -> Result<()> {
        let operation = detail
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN_PROVIDER_REQUEST")
            .to_owned();
        let approval =
            self.request_provider_approval(provider.into(), None, None, operation, detail)?;
        let mut approval = approval;
        approval.status = ProviderApprovalStatus::Denied;
        approval.decision = Some(ProviderApprovalDecision::Deny);
        approval.resolved_at = Some(now());
        approval.resolved_by = Some("system-policy".into());
        self.persist_provider_approval(&approval)?;
        Ok(())
    }

    pub fn reconcile_startup(&self) -> Result<RecoverySummary> {
        let recovery = self.recovery.reconcile_startup()?;
        let approvals: Vec<ProviderApproval> =
            self.store.list_governance_records("PROVIDER_APPROVAL")?;
        for mut approval in approvals {
            if approval.status.terminal() {
                continue;
            }
            approval.status = ProviderApprovalStatus::Orphaned;
            approval.resolved_at = Some(now());
            approval.resolved_by = Some("startup-reconciliation".into());
            approval.response_error =
                Some("the original provider process/request is not live after restart".into());
            self.persist_provider_approval(&approval)?;
            if let Some(task_id) = &approval.task_id {
                if let Ok(task) = self
                    .store
                    .set_task_status(task_id, super::types::TaskStatus::Review)
                {
                    let _ = self.events.publish(
                        EventType::ReviewRequired,
                        "recovery",
                        Some("director".into()),
                        Some(task.id),
                        serde_json::json!({"reason":"orphaned_provider_approval","approvalId":approval.id}),
                    );
                }
            }
            self.events.publish(
                EventType::ProviderApprovalOrphaned,
                "recovery",
                Some(approval.id.clone()),
                approval.task_id.clone(),
                serde_json::to_value(&approval)?,
            )?;
        }
        Ok(recovery)
    }

    pub fn resolve_recovery_operation(
        &self,
        operation_id: &str,
        action: RecoveryAction,
    ) -> Result<OperationJournal> {
        let _guard = self
            .mutation_lock
            .lock()
            .map_err(|_| RuntimeError::Lock("governance mutation"))?;
        self.recovery.resolve_review(operation_id, action)
    }

    pub fn shutdown_approvals(&self) {
        let _ = self.approval_shutdown.send(true);
        if let Ok(approvals) = self.live_approvals.lock() {
            for approval in approvals.values() {
                let _ = approval.sender.send(Some(ProviderApprovalDecision::Cancel));
            }
        }
    }

    fn persist_provider_approval(&self, approval: &ProviderApproval) -> Result<()> {
        self.store.upsert_governance_record(
            &approval.id,
            "PROVIDER_APPROVAL",
            approval.status.label(),
            approval,
        )
    }

    fn provider_approval_by_id(&self, id: &str) -> Result<Option<ProviderApproval>> {
        Ok(self
            .store
            .list_governance_records::<ProviderApproval>("PROVIDER_APPROVAL")?
            .into_iter()
            .find(|approval| approval.id == id))
    }

    async fn await_provider_approval(
        &self,
        request: ProviderApprovalRequest,
    ) -> ProviderApprovalDirective {
        let approval_id = exact_approval_id(&request);
        if let Ok(Some(existing)) = self.provider_approval_by_id(&approval_id) {
            if existing.status.terminal() || existing.decision.is_some() {
                return ProviderApprovalDirective {
                    approval_id,
                    decision: existing.decision.unwrap_or(ProviderApprovalDecision::Deny),
                };
            }
            if existing.status == ProviderApprovalStatus::PendingGod {
                let receiver =
                    self.live_approvals.lock().ok().and_then(|live| {
                        live.get(&approval_id).map(|entry| entry.sender.subscribe())
                    });
                if let Some(receiver) = receiver {
                    let timeout = approval_remaining_seconds(&existing).max(1);
                    let decision = self
                        .await_live_signal(&approval_id, existing, receiver, timeout)
                        .await;
                    return ProviderApprovalDirective {
                        approval_id,
                        decision,
                    };
                }
                return ProviderApprovalDirective {
                    approval_id,
                    decision: ProviderApprovalDecision::Deny,
                };
            }
        }
        let policy = match self.load_policy() {
            Ok(policy) => policy,
            Err(_) => {
                return ProviderApprovalDirective {
                    approval_id,
                    decision: ProviderApprovalDecision::Deny,
                }
            }
        };
        let timeout_seconds = policy.provider_approval_timeout_seconds.max(1);
        let expires_at = (Utc::now() + chrono::Duration::seconds(timeout_seconds as i64))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let mut approval = ProviderApproval {
            id: approval_id.clone(),
            provider: request.provider.clone(),
            agent_id: Some(request.agent_id.clone()),
            task_id: Some(request.task_id.clone()),
            process_id: request.process_id,
            session_id: Some(request.session_id.clone()),
            thread_id: Some(request.thread_id.clone()),
            turn_id: Some(request.turn_id.clone()),
            request_id: Some(request.request_id.clone()),
            worktree: request.worktree.clone(),
            operation: request.requested_operation.clone(),
            requested_target: request.requested_target.clone(),
            risk: Some(request.risk.clone()),
            detail: redact_provider_detail(request.detail.clone()),
            status: ProviderApprovalStatus::Received,
            created_at: now(),
            expires_at: Some(expires_at),
            resolved_at: None,
            resolved_by: None,
            responded_at: None,
            decision: None,
            response_error: None,
        };
        if self.persist_provider_approval(&approval).is_err() {
            return ProviderApprovalDirective {
                approval_id,
                decision: ProviderApprovalDecision::Deny,
            };
        }
        let safe_to_prompt = provider_request_within_worktree(&request)
            && provider_allowed(&policy, &request.provider);
        if !self.interactive_approvals.load(Ordering::Acquire) || !safe_to_prompt {
            approval.status = ProviderApprovalStatus::Denied;
            approval.decision = Some(ProviderApprovalDecision::Deny);
            approval.resolved_at = Some(now());
            approval.resolved_by = Some(if safe_to_prompt {
                "headless-fail-closed".into()
            } else {
                "system-policy".into()
            });
            let _ = self.persist_provider_approval(&approval);
            let _ = self.events.publish(
                EventType::ProviderApprovalDenied,
                "batai",
                Some(approval_id.clone()),
                Some(request.task_id),
                serde_json::to_value(&approval).unwrap_or(Value::Null),
            );
            return ProviderApprovalDirective {
                approval_id,
                decision: ProviderApprovalDecision::Deny,
            };
        }
        let (sender, receiver) = watch::channel(None);
        if let Ok(mut live) = self.live_approvals.lock() {
            live.insert(
                approval_id.clone(),
                LiveApproval {
                    session_id: request.session_id.clone(),
                    sender,
                },
            );
        } else {
            return ProviderApprovalDirective {
                approval_id,
                decision: ProviderApprovalDecision::Deny,
            };
        }
        approval.status = ProviderApprovalStatus::PendingGod;
        let _ = self.persist_provider_approval(&approval);
        let _ = self.events.publish(
            EventType::ProviderApprovalWaiting,
            request.agent_id,
            Some(approval_id.clone()),
            Some(request.task_id),
            serde_json::to_value(&approval).unwrap_or(Value::Null),
        );
        let decision = self
            .await_live_signal(&approval_id, approval, receiver, timeout_seconds)
            .await;
        if let Ok(mut live) = self.live_approvals.lock() {
            live.remove(&approval_id);
        }
        ProviderApprovalDirective {
            approval_id,
            decision,
        }
    }

    async fn await_live_signal(
        &self,
        approval_id: &str,
        mut approval: ProviderApproval,
        mut receiver: watch::Receiver<Option<ProviderApprovalDecision>>,
        timeout_seconds: u64,
    ) -> ProviderApprovalDecision {
        let mut shutdown = self.approval_shutdown.subscribe();
        tokio::select! {
            result = async {
                loop {
                    if let Some(decision) = *receiver.borrow() {
                        break decision;
                    }
                    if receiver.changed().await.is_err() {
                        break ProviderApprovalDecision::Deny;
                    }
                }
            } => {
                if result == ProviderApprovalDecision::Cancel {
                    approval.status = ProviderApprovalStatus::Cancelled;
                    approval.decision = Some(ProviderApprovalDecision::Cancel);
                    approval.resolved_at = Some(now());
                    approval.resolved_by = Some("task-or-provider-cancellation".into());
                    let _ = self.persist_provider_approval(&approval);
                }
                result
            },
            _ = tokio::time::sleep(Duration::from_secs(timeout_seconds)) => {
                approval.status = ProviderApprovalStatus::Expired;
                approval.decision = Some(ProviderApprovalDecision::Deny);
                approval.resolved_at = Some(now());
                approval.resolved_by = Some("timeout".into());
                let _ = self.persist_provider_approval(&approval);
                let _ = self.events.publish(EventType::ProviderApprovalExpired, "batai", Some(approval_id.to_owned()), approval.task_id.clone(), serde_json::to_value(&approval).unwrap_or(Value::Null));
                ProviderApprovalDecision::Deny
            },
            _ = shutdown.changed() => {
                approval.status = ProviderApprovalStatus::Cancelled;
                approval.decision = Some(ProviderApprovalDecision::Cancel);
                approval.resolved_at = Some(now());
                approval.resolved_by = Some("runtime-shutdown".into());
                let _ = self.persist_provider_approval(&approval);
                ProviderApprovalDecision::Cancel
            }
        }
    }

    fn complete_provider_response(
        &self,
        approval_id: &str,
        result: std::result::Result<(), String>,
    ) {
        let Ok(Some(mut approval)) = self.provider_approval_by_id(approval_id) else {
            return;
        };
        match result {
            Ok(()) => {
                approval.responded_at = Some(now());
                if matches!(
                    approval.status,
                    ProviderApprovalStatus::Approved | ProviderApprovalStatus::Denied
                ) {
                    approval.status = ProviderApprovalStatus::Responded;
                    let _ = self.persist_provider_approval(&approval);
                    approval.status = ProviderApprovalStatus::Resolved;
                    approval.resolved_at.get_or_insert_with(now);
                }
                let _ = self.persist_provider_approval(&approval);
                let _ = self.events.publish(
                    EventType::ProviderApprovalResolved,
                    "provider",
                    Some(approval.id.clone()),
                    approval.task_id.clone(),
                    serde_json::to_value(&approval).unwrap_or(Value::Null),
                );
            }
            Err(error) => {
                approval.status = ProviderApprovalStatus::ResponseUncertain;
                approval.response_error = Some(
                    super::super::providers::process_supervisor::redact_secrets(&error),
                );
                let _ = self.persist_provider_approval(&approval);
                if let Some(task_id) = &approval.task_id {
                    let _ = self
                        .store
                        .set_task_status(task_id, super::types::TaskStatus::Review);
                }
            }
        }
    }

    pub fn record_review(&self, review: ReviewOutcome) -> Result<()> {
        self.store.append_review_outcome(
            &review.id,
            &review.task_id,
            &review.reviewer_id,
            &review.subject_agent_id,
            &format!("{:?}", review.outcome).to_ascii_uppercase(),
            &review,
        )?;
        self.events.publish(
            EventType::ReviewOutcomeRecorded,
            review.reviewer_id.clone(),
            Some(review.subject_agent_id.clone()),
            Some(review.task_id.clone()),
            serde_json::to_value(review)?,
        )?;
        Ok(())
    }

    pub fn review_acceptance(&self, agent_id: &str) -> Result<Option<f64>> {
        let reviews: Vec<ReviewOutcome> = self.store.list_review_outcomes(Some(agent_id))?;
        if reviews.is_empty() {
            return Ok(None);
        }
        let accepted = reviews
            .iter()
            .filter(|review| review.outcome == ReviewOutcomeKind::Accepted)
            .count();
        Ok(Some(accepted as f64 * 100.0 / reviews.len() as f64))
    }

    fn apply(
        &self,
        request: MutationRequest,
        authority: AuthorityRole,
        mut organization: OrganizationDocument,
        mut policy: ProjectGovernancePolicy,
        decision_id: Option<&str>,
    ) -> Result<MutationResult> {
        let mut affected_id = request.mutation.target().map(str::to_owned);
        let event_type;
        let mut agent_change: Option<(Option<Agent>, Agent)> = None;
        match &request.mutation {
            OrganizationMutation::CreateAgent(create) => {
                let agent = self.build_agent(create, &policy)?;
                self.validate_agent_change(&agent, None, &policy)?;
                affected_id = Some(agent.id.clone());
                agent_change = Some((None, agent));
                event_type = EventType::AgentCreated;
            }
            OrganizationMutation::UpdateIdentity {
                agent_id,
                name,
                title_override,
            } => {
                let mut agent = self.require_agent(agent_id)?;
                if let Some(name) = name {
                    validate_name(name)?;
                    agent.name = name.trim().to_owned();
                }
                agent.title_override.clone_from(title_override);
                agent_change = Some((Some(self.require_agent(agent_id)?), agent));
                event_type = EventType::AgentUpdated;
            }
            OrganizationMutation::ChangeReportingLine {
                agent_id,
                reports_to,
            } => {
                let mut agent = self.require_agent(agent_id)?;
                agent.parent_agent_id.clone_from(reports_to);
                self.validate_agent_change(&agent, Some(agent_id), &policy)?;
                agent_change = Some((Some(self.require_agent(agent_id)?), agent));
                event_type = EventType::OrganizationChanged;
            }
            OrganizationMutation::ChangeRole {
                agent_id,
                seniority,
                function,
                department,
            } => {
                let mut agent = self.require_agent(agent_id)?;
                agent.seniority = Some(*seniority);
                agent.function = Some(*function);
                agent.department = Some(*department);
                if *function == AgentFunction::Director {
                    agent.authority = AuthorityRole::Director;
                }
                self.validate_agent_change(&agent, Some(agent_id), &policy)?;
                agent_change = Some((Some(self.require_agent(agent_id)?), agent));
                event_type = EventType::AgentUpdated;
            }
            OrganizationMutation::ChangeProviderPolicy {
                agent_id,
                provider,
                model,
                intelligence_policy,
            } => {
                let mut agent = self.require_agent(agent_id)?;
                agent.provider = provider.trim().to_ascii_lowercase();
                agent.model = model.trim().to_owned();
                agent.intelligence_policy = intelligence_policy.clone();
                self.validate_agent_change(&agent, Some(agent_id), &policy)?;
                agent_change = Some((Some(self.require_agent(agent_id)?), agent));
                event_type = EventType::AgentUpdated;
            }
            OrganizationMutation::AddRelationship { relationship } => {
                validate_persistent_relationship(relationship, &self.agents.list()?)?;
                if organization
                    .relationships
                    .iter()
                    .any(|item| item.id == relationship.id)
                {
                    return Err(RuntimeError::Governance(format!(
                        "relationship already exists: {}",
                        relationship.id
                    )));
                }
                let mut relationship = relationship.clone();
                relationship.persistent = true;
                organization.relationships.push(relationship);
                event_type = EventType::RelationshipAdded;
            }
            OrganizationMutation::RemoveRelationship { relationship_id } => {
                let original = organization.relationships.len();
                organization
                    .relationships
                    .retain(|item| item.id != *relationship_id);
                if original == organization.relationships.len() {
                    return Err(RuntimeError::Governance(format!(
                        "relationship not found: {relationship_id}"
                    )));
                }
                event_type = EventType::RelationshipRemoved;
            }
            OrganizationMutation::PauseAgent { agent_id } => {
                let before = self.require_agent(agent_id)?;
                if before.status != AgentStatus::Paused
                    && !super::agents::transition_allowed(before.status, AgentStatus::Paused)
                {
                    return Err(RuntimeError::InvalidAgentTransition {
                        from: before.status,
                        to: AgentStatus::Paused,
                    });
                }
                let mut after = before.clone();
                after.status = AgentStatus::Paused;
                after.current_task_id = None;
                agent_change = Some((Some(before), after));
                event_type = EventType::AgentUpdated;
            }
            OrganizationMutation::ResumeAgent { agent_id } => {
                let before = self.require_agent(agent_id)?;
                if before.status != AgentStatus::Ready
                    && !super::agents::transition_allowed(before.status, AgentStatus::Ready)
                {
                    return Err(RuntimeError::InvalidAgentTransition {
                        from: before.status,
                        to: AgentStatus::Ready,
                    });
                }
                let mut after = before.clone();
                after.status = AgentStatus::Ready;
                after.current_task_id = None;
                agent_change = Some((Some(before), after));
                event_type = EventType::AgentUpdated;
            }
            OrganizationMutation::TerminateAgent { agent_id, .. } => {
                let agent = self.require_agent(agent_id)?;
                if agent.authority == AuthorityRole::Director
                    || agent.function == Some(AgentFunction::Director)
                {
                    return Err(RuntimeError::Governance(
                        "the Director cannot be terminated".into(),
                    ));
                }
                if !super::agents::transition_allowed(agent.status, AgentStatus::Terminated) {
                    return Err(RuntimeError::InvalidAgentTransition {
                        from: agent.status,
                        to: AgentStatus::Terminated,
                    });
                }
                let mut persisted = agent.clone();
                persisted.status = AgentStatus::Terminated;
                persisted.current_task_id = None;
                agent_change = Some((Some(agent), persisted));
                event_type = EventType::AgentTerminated;
            }
            OrganizationMutation::UpdateProjectPolicy {
                policy: replacement,
            } => {
                validate_policy(replacement)?;
                policy = replacement.clone();
                event_type = EventType::PolicyChanged;
            }
            OrganizationMutation::RequestProtectedAction { .. } => {
                event_type = EventType::OrganizationChanged;
            }
        }
        organization.revision = organization.revision.saturating_add(1);
        organization.schema_version = 1;
        policy.revision = organization.revision;
        let mut files = Vec::new();
        let mut db_entities = Vec::new();
        if let Some((before, after)) = &agent_change {
            files.push((self.agent_config_path(&after.id), json_bytes(after)?));
            db_entities.push(agent_db_entity(before.clone(), Some(after.clone()))?);
        }
        files.push((
            self.root.join(".batai/organization.json"),
            json_bytes(&organization)?,
        ));
        if matches!(
            request.mutation,
            OrganizationMutation::UpdateProjectPolicy { .. }
        ) {
            files.push((self.root.join(".batai/policies.json"), json_bytes(&policy)?));
        }
        let mut journal = self.recovery.prepare(
            request.mutation.action(),
            request.actor.id.clone(),
            affected_id.clone(),
            request.expected_revision,
            organization.revision,
            files,
            db_entities,
            decision_id.map(str::to_owned),
            request.task_id.clone(),
        )?;
        if let Err(error) = self.recovery.apply_files(&mut journal) {
            let rollback = self
                .recovery
                .fail_and_rollback(&mut journal, &error.to_string());
            if let Err(rollback_error) = rollback {
                return Err(RuntimeError::Governance(format!(
                    "mutation failed: {error}; recovery failed: {rollback_error}"
                )));
            }
            return Err(error);
        }
        if let Err(error) = self.recovery.apply_db(&mut journal) {
            let rollback = self
                .recovery
                .fail_and_rollback(&mut journal, &error.to_string());
            if let Err(rollback_error) = rollback {
                return Err(RuntimeError::Governance(format!(
                    "database mutation failed: {error}; recovery failed: {rollback_error}"
                )));
            }
            return Err(error);
        }
        self.audit(
            &request,
            authority,
            MutationDisposition::Applied,
            None,
            organization.revision,
        )?;
        self.events.publish(
            event_type,
            request.actor.id,
            affected_id.clone(),
            request.task_id,
            serde_json::json!({"revision":organization.revision,"action":request.mutation.action()}),
        )?;
        self.recovery.commit(&mut journal)?;
        Ok(MutationResult {
            disposition: MutationDisposition::Applied,
            revision: organization.revision,
            decision_id: None,
            message: "organization mutation applied".into(),
            affected_id,
        })
    }

    fn agent_config_path(&self, agent_id: &str) -> PathBuf {
        self.root
            .join(".batai/agents")
            .join(agent_id)
            .join("config.json")
    }

    fn route(&self, authority: AuthorityRole, request: &MutationRequest) -> Result<AuthorityRoute> {
        if authority == AuthorityRole::God {
            return Ok(AuthorityRoute::Allow);
        }
        let protected = match &request.mutation {
            OrganizationMutation::CreateAgent(agent) => {
                agent.lifecycle == AgentLifecycle::Permanent
                    || matches!(
                        agent.authority,
                        AuthorityRole::God | AuthorityRole::Director
                    )
                    || agent.function == AgentFunction::Director
                    || agent.permissions.iter().any(|permission| {
                        matches!(
                            permission,
                            AgentPermission::ManageAgents
                                | AgentPermission::ManageOrganization
                                | AgentPermission::ManageProviders
                                | AgentPermission::ApprovePayg
                        )
                    })
            }
            OrganizationMutation::TerminateAgent { agent_id, .. } => {
                self.require_agent(agent_id)
                    .map(|agent| agent.lifecycle == AgentLifecycle::Permanent)?
            }
            OrganizationMutation::UpdateProjectPolicy { .. } => true,
            OrganizationMutation::RequestProtectedAction { .. } => true,
            OrganizationMutation::ChangeProviderPolicy {
                intelligence_policy,
                ..
            } => intelligence_policy.allow_payg,
            _ => false,
        };
        if protected {
            return Ok(AuthorityRoute::RequireGod(format!(
                "Approve protected operation {}?",
                request.mutation.action()
            )));
        }
        if authority == AuthorityRole::Director {
            return Ok(AuthorityRoute::Allow);
        }
        let actor = self.require_agent(&request.actor.id)?;
        let required = match request.mutation {
            OrganizationMutation::AddRelationship { .. }
            | OrganizationMutation::RemoveRelationship { .. }
            | OrganizationMutation::ChangeReportingLine { .. } => {
                AgentPermission::ManageOrganization
            }
            _ => AgentPermission::ManageAgents,
        };
        if actor.permissions.contains(&required) {
            Ok(AuthorityRoute::Allow)
        } else {
            Ok(AuthorityRoute::Deny(format!(
                "{} lacks authority for {}",
                request.actor.id,
                request.mutation.action()
            )))
        }
    }

    fn enforce_system_policy(
        &self,
        request: &MutationRequest,
        policy: &ProjectGovernancePolicy,
    ) -> Result<()> {
        let provider = match &request.mutation {
            OrganizationMutation::CreateAgent(agent) => Some(agent.provider.as_str()),
            OrganizationMutation::ChangeProviderPolicy { provider, .. } => Some(provider.as_str()),
            _ => None,
        };
        if let Some(provider) = provider {
            let provider = provider.to_ascii_lowercase();
            if policy
                .denied_providers
                .iter()
                .any(|item| item.eq_ignore_ascii_case(&provider))
            {
                return Err(RuntimeError::Governance(format!(
                    "provider denied by system policy: {provider}"
                )));
            }
            if !policy.allowed_providers.is_empty()
                && !policy
                    .allowed_providers
                    .iter()
                    .any(|item| item.eq_ignore_ascii_case(&provider))
            {
                return Err(RuntimeError::Governance(format!(
                    "provider not in allowlist: {provider}"
                )));
            }
        }
        Ok(())
    }

    fn resolve_authority(&self, actor_id: &str) -> Result<AuthorityRole> {
        if actor_id.eq_ignore_ascii_case("god") || actor_id.eq_ignore_ascii_case("user") {
            return Ok(AuthorityRole::God);
        }
        Ok(self
            .agents
            .get(actor_id)?
            .ok_or_else(|| RuntimeError::Governance(format!("unknown actor: {actor_id}")))?
            .with_backfilled_organization()
            .authority)
    }

    fn build_agent(
        &self,
        request: &CreateAgentRequest,
        policy: &ProjectGovernancePolicy,
    ) -> Result<Agent> {
        let existing = self.agents.list()?;
        if existing
            .iter()
            .filter(|agent| agent.status != AgentStatus::Terminated)
            .count()
            >= policy.limits.max_active_agents
        {
            return Err(RuntimeError::Governance(format!(
                "active agent limit reached ({})",
                policy.limits.max_active_agents
            )));
        }
        if !request.function.supports(request.seniority) {
            return Err(RuntimeError::Governance(
                "invalid seniority/function combination".into(),
            ));
        }
        if request.department != request.function.department() {
            return Err(RuntimeError::Governance(
                "department must match the function catalog".into(),
            ));
        }
        let name = request
            .name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| next_name(&existing));
        validate_name(&name)?;
        let id = request
            .id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .map(slug)
            .unwrap_or_else(|| unique_id(&name, &existing));
        if existing.iter().any(|agent| agent.id == id) {
            return Err(RuntimeError::Governance(format!(
                "agent already exists: {id}"
            )));
        }
        if request.authority == AuthorityRole::God {
            return Err(RuntimeError::Governance(
                "GOD is human authority and cannot be assigned to an AI agent".into(),
            ));
        }
        let authority = if request.function == AgentFunction::Director {
            AuthorityRole::Director
        } else {
            request.authority
        };
        Ok(Agent {
            schema_version: 2,
            id,
            name,
            role_template: request.function.base_title().into(),
            seniority: Some(request.seniority),
            function: Some(request.function),
            department: Some(request.department),
            title_override: None,
            model_capabilities: Default::default(),
            effective_capabilities: Default::default(),
            lifecycle: request.lifecycle,
            authority,
            permissions: request.permissions.clone(),
            intelligence_policy: request.intelligence_policy.clone(),
            parent_agent_id: request.reports_to.clone(),
            provider: request.provider.trim().to_ascii_lowercase(),
            model: request.model.trim().to_owned(),
            reasoning_effort: if request.reasoning_effort.trim().is_empty() {
                "medium".into()
            } else {
                request.reasoning_effort.clone()
            },
            auth_mode: "unknown".into(),
            worktree: None,
            status: AgentStatus::Created,
            current_task_id: None,
            extra: serde_json::Map::new(),
        })
    }

    fn validate_agent_change(
        &self,
        candidate: &Agent,
        replacing: Option<&str>,
        policy: &ProjectGovernancePolicy,
    ) -> Result<()> {
        if !candidate.organization_is_valid() {
            return Err(RuntimeError::Governance(
                "invalid seniority/function combination".into(),
            ));
        }
        if candidate.department != candidate.function.map(AgentFunction::department) {
            return Err(RuntimeError::Governance(
                "department must match function".into(),
            ));
        }
        let mut agents = self.agents.list()?;
        agents.retain(|agent| replacing != Some(agent.id.as_str()));
        agents.push(candidate.clone());
        let directors = agents
            .iter()
            .filter(|agent| agent.status != AgentStatus::Terminated)
            .filter(|agent| {
                agent.function == Some(AgentFunction::Director)
                    || agent.authority == AuthorityRole::Director
            })
            .count();
        if directors != 1 {
            return Err(RuntimeError::Governance(
                "exactly one active Director is required".into(),
            ));
        }
        if candidate.authority == AuthorityRole::Director
            && candidate.function != Some(AgentFunction::Director)
        {
            return Err(RuntimeError::Governance(
                "Director authority requires the Director function".into(),
            ));
        }
        if candidate.parent_agent_id.as_deref() == Some("god")
            && candidate.function != Some(AgentFunction::Director)
        {
            return Err(RuntimeError::Governance(
                "only the Director may report directly to GOD".into(),
            ));
        }
        if candidate.parent_agent_id.is_none()
            && candidate.function != Some(AgentFunction::Director)
        {
            return Err(RuntimeError::Governance(
                "non-Director agents require an organizational parent".into(),
            ));
        }
        let parents = agents
            .iter()
            .filter(|agent| agent.status != AgentStatus::Terminated)
            .map(|agent| (agent.id.clone(), agent.parent_agent_id.clone()))
            .collect::<HashMap<_, _>>();
        let warnings = hierarchy_warnings(&parents);
        if let Some(warning) = warnings.first() {
            return Err(RuntimeError::Governance(format!(
                "invalid hierarchy: {warning}"
            )));
        }
        let depth = hierarchy_depth(&candidate.id, &parents);
        if depth > policy.limits.max_hierarchy_depth {
            return Err(RuntimeError::Governance(format!(
                "hierarchy depth {depth} exceeds policy limit {}",
                policy.limits.max_hierarchy_depth
            )));
        }
        Ok(())
    }

    fn require_agent(&self, id: &str) -> Result<Agent> {
        self.agents
            .get(id)?
            .ok_or_else(|| RuntimeError::AgentNotFound(id.into()))
    }

    fn load_organization(&self) -> Result<OrganizationDocument> {
        read_json_or_default(&self.root.join(".batai/organization.json"))
    }

    fn load_policy(&self) -> Result<ProjectGovernancePolicy> {
        read_json_or_default(&self.root.join(".batai/policies.json"))
    }

    fn persist_agent(&self, agent: &Agent) -> Result<()> {
        let path = self
            .root
            .join(".batai/agents")
            .join(&agent.id)
            .join("config.json");
        let previous = fs::read(&path).ok();
        atomic_json(&path, agent)?;
        if let Err(error) = self.agents.register(agent) {
            restore_file(&path, previous.as_deref())?;
            return Err(error);
        }
        Ok(())
    }

    fn persist_organization(&self, organization: &OrganizationDocument) -> Result<()> {
        atomic_json(&self.root.join(".batai/organization.json"), organization)
    }

    fn persist_policy(&self, policy: &ProjectGovernancePolicy) -> Result<()> {
        atomic_json(&self.root.join(".batai/policies.json"), policy)
    }

    fn create_decision(&self, request: MutationRequest, question: String) -> Result<GodDecision> {
        let decision = GodDecision {
            id: format!("DEC-{}", uuid::Uuid::new_v4()),
            status: DecisionStatus::Open,
            created_at: now(),
            resolved_at: None,
            requested_by: request.actor.id.clone(),
            question,
            impact: format!(
                "Would apply {} at organization revision {}",
                request.mutation.action(),
                request.expected_revision
            ),
            request,
            resolution_note: None,
        };
        self.persist_decision(&decision)?;
        Ok(decision)
    }

    fn persist_decision(&self, decision: &GodDecision) -> Result<()> {
        self.store.upsert_governance_record(
            &decision.id,
            "GOD_DECISION",
            decision.status.label(),
            decision,
        )?;
        atomic_json(
            &self
                .root
                .join(".batai/decisions")
                .join(format!("{}.json", decision.id)),
            decision,
        )
    }

    fn audit(
        &self,
        request: &MutationRequest,
        authority: AuthorityRole,
        outcome: MutationDisposition,
        decision_id: Option<&str>,
        revision: u64,
    ) -> Result<()> {
        let record = AuditRecord {
            id: format!("AUD-{}", uuid::Uuid::new_v4()),
            timestamp: now(),
            actor: request.actor.id.clone(),
            authority,
            action: request.mutation.action().into(),
            target: request.mutation.target().map(str::to_owned),
            outcome,
            reason: request.reason.clone(),
            decision_id: decision_id.map(str::to_owned),
            task_id: request.task_id.clone(),
            revision,
        };
        let outcome = format!("{:?}", record.outcome).to_ascii_uppercase();
        self.store.append_governance_audit(&GovernanceAuditRow {
            id: &record.id,
            timestamp: &record.timestamp,
            actor: &record.actor,
            action: &record.action,
            target: record.target.as_deref(),
            outcome: &outcome,
            record_json: serde_json::to_string(&record)?,
        })?;
        self.events.publish(
            EventType::AuditRecorded,
            record.actor.clone(),
            record.target.clone(),
            record.task_id.clone(),
            serde_json::to_value(record)?,
        )?;
        Ok(())
    }
}

#[async_trait]
impl ProviderApprovalHandler for GovernanceService {
    async fn handle(&self, request: ProviderApprovalRequest) -> ProviderApprovalDirective {
        self.await_provider_approval(request).await
    }

    fn response_result(&self, approval_id: &str, result: std::result::Result<(), String>) {
        self.complete_provider_response(approval_id, result);
    }

    fn cancel_session(&self, session_id: &str) {
        let live = self
            .live_approvals
            .lock()
            .map(|approvals| {
                approvals
                    .values()
                    .filter(|approval| approval.session_id == session_id)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for approval in live {
            let _ = approval.sender.send(Some(ProviderApprovalDecision::Cancel));
        }
    }
}

fn exact_approval_id(request: &ProviderApprovalRequest) -> String {
    let binding = serde_json::json!({
        "provider":request.provider,
        "processId":request.process_id,
        "sessionId":request.session_id,
        "threadId":request.thread_id,
        "turnId":request.turn_id,
        "requestId":request.request_id,
        "agentId":request.agent_id,
        "taskId":request.task_id,
        "worktree":request.worktree,
        "operation":request.requested_operation,
        "target":request.requested_target,
    });
    format!("APR-{:x}", Sha256::digest(binding.to_string().as_bytes()))
}

fn approval_remaining_seconds(approval: &ProviderApproval) -> u64 {
    approval
        .expires_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|deadline| {
            deadline
                .signed_duration_since(Utc::now())
                .num_seconds()
                .max(0) as u64
        })
        .unwrap_or_default()
}

fn provider_request_within_worktree(request: &ProviderApprovalRequest) -> bool {
    let Some(worktree) = request.worktree.as_deref() else {
        return false;
    };
    let Some(target) = request.requested_target.as_deref() else {
        return true;
    };
    let target_path = PathBuf::from(target);
    if !target_path.is_absolute() {
        return !target_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir));
    }
    !target_path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
        && target_path.starts_with(Path::new(worktree))
}

fn provider_allowed(policy: &ProjectGovernancePolicy, provider: &str) -> bool {
    let provider = provider.to_ascii_lowercase();
    !policy
        .denied_providers
        .iter()
        .any(|value| value.eq_ignore_ascii_case(&provider))
        && (policy.allowed_providers.is_empty()
            || policy
                .allowed_providers
                .iter()
                .any(|value| value.eq_ignore_ascii_case(&provider)))
}

enum AuthorityRoute {
    Allow,
    Deny(String),
    RequireGod(String),
}

fn validate_policy(policy: &ProjectGovernancePolicy) -> Result<()> {
    if policy.limits.max_active_agents == 0 || policy.limits.max_hierarchy_depth == 0 {
        return Err(RuntimeError::Governance(
            "agent and hierarchy limits must be greater than zero".into(),
        ));
    }
    if !(1..=3600).contains(&policy.provider_approval_timeout_seconds) {
        return Err(RuntimeError::Governance(
            "provider approval timeout must be between 1 and 3600 seconds".into(),
        ));
    }
    let allowed = policy
        .allowed_providers
        .iter()
        .map(|provider| provider.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    if policy
        .denied_providers
        .iter()
        .any(|provider| allowed.contains(&provider.to_ascii_lowercase()))
    {
        return Err(RuntimeError::Governance(
            "a provider cannot be both allowed and denied".into(),
        ));
    }
    Ok(())
}

fn validate_persistent_relationship(
    relationship: &OrganizationRelationship,
    agents: &[Agent],
) -> Result<()> {
    if !matches!(
        relationship.relationship_type,
        RelationshipType::Collaboration | RelationshipType::Review | RelationshipType::Advisory
    ) {
        return Err(RuntimeError::Governance(
            "only collaboration, review, and advisory relationships are persistent".into(),
        ));
    }
    if relationship.source == relationship.target {
        return Err(RuntimeError::Governance(
            "self relationships are invalid".into(),
        ));
    }
    for endpoint in [&relationship.source, &relationship.target] {
        if !agents.iter().any(|agent| &agent.id == endpoint) {
            return Err(RuntimeError::Governance(format!(
                "relationship endpoint not found: {endpoint}"
            )));
        }
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 64 {
        return Err(RuntimeError::Governance(
            "agent name must contain 1-64 characters".into(),
        ));
    }
    Ok(())
}

fn hierarchy_depth(id: &str, parents: &HashMap<String, Option<String>>) -> usize {
    let mut depth = 0;
    let mut current = parents.get(id).and_then(Option::as_deref);
    while let Some(parent) = current {
        if parent == "god" {
            break;
        }
        depth += 1;
        current = parents.get(parent).and_then(Option::as_deref);
    }
    depth
}

fn next_name(agents: &[Agent]) -> String {
    AGENT_NAMES
        .iter()
        .find(|name| {
            !agents
                .iter()
                .any(|agent| agent.name.eq_ignore_ascii_case(name))
        })
        .map(|name| (*name).to_owned())
        .unwrap_or_else(|| format!("Agent {}", agents.len() + 1))
}

fn unique_id(name: &str, agents: &[Agent]) -> String {
    let base = slug(name);
    if !agents.iter().any(|agent| agent.id == base) {
        return base;
    }
    (2..)
        .map(|index| format!("{base}-{index}"))
        .find(|candidate| !agents.iter().any(|agent| agent.id == *candidate))
        .expect("unbounded deterministic id sequence")
}

fn slug(value: &str) -> String {
    let slug = value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let compact = slug
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if compact.is_empty() {
        "agent".into()
    } else {
        compact
    }
}

fn read_json_or_default<T>(path: &Path) -> Result<T>
where
    T: serde::de::DeserializeOwned + Default,
{
    if !path.exists() {
        return Ok(T::default());
    }
    serde_json::from_slice(&fs::read(path)?).map_err(|error| {
        RuntimeError::Governance(format!(
            "invalid governance file {}: {error}",
            path.display()
        ))
    })
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
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
    let bytes = serde_json::to_vec_pretty(value)?;
    {
        use std::io::Write;
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    let backup = path.with_extension(format!("bak.{}", uuid::Uuid::new_v4()));
    if path.exists() {
        fs::rename(path, &backup)?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    if backup.exists() {
        fs::remove_file(backup)?;
    }
    Ok(())
}

fn restore_file(path: &Path, previous: Option<&[u8]>) -> Result<()> {
    match previous {
        Some(bytes) => {
            let value: Value = serde_json::from_slice(bytes)?;
            atomic_json(path, &value)
        }
        None if path.exists() => {
            fs::remove_file(path)?;
            Ok(())
        }
        None => Ok(()),
    }
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn redact_provider_detail(value: Value) -> Value {
    match value {
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    let normalized = key.to_ascii_lowercase();
                    let sensitive = [
                        "token",
                        "secret",
                        "password",
                        "authorization",
                        "credential",
                        "api_key",
                        "apikey",
                    ]
                    .iter()
                    .any(|marker| normalized.contains(marker));
                    (
                        key,
                        if sensitive {
                            Value::String("[REDACTED]".into())
                        } else {
                            redact_provider_detail(value)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(values) => {
            Value::Array(values.into_iter().map(redact_provider_detail).collect())
        }
        Value::String(value)
            if value.len() > 4096
                || value.to_ascii_lowercase().contains("bearer ")
                || value.contains("sk-") =>
        {
            Value::String("[REDACTED]".into())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct Harness {
        _root: TempDir,
        service: GovernanceService,
        agents: AgentRegistry,
    }

    fn test_agent(id: &str, function: AgentFunction, parent: Option<&str>) -> Agent {
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
            model_capabilities: Default::default(),
            effective_capabilities: Default::default(),
            lifecycle: AgentLifecycle::Project,
            authority: if function == AgentFunction::Director {
                AuthorityRole::Director
            } else {
                AuthorityRole::Worker
            },
            permissions: vec![],
            intelligence_policy: Default::default(),
            parent_agent_id: parent.map(str::to_owned),
            provider: "mock".into(),
            model: "mock".into(),
            reasoning_effort: "medium".into(),
            auth_mode: "local".into(),
            worktree: None,
            status: AgentStatus::Ready,
            current_task_id: None,
            extra: Default::default(),
        }
    }

    fn harness() -> Harness {
        let root = TempDir::new().unwrap();
        let store = RuntimeStore::open(root.path().join(".runtime/runtime.sqlite")).unwrap();
        let events = EventEngine::new(store.clone());
        let agents = AgentRegistry::new(store.clone(), events.clone());
        agents
            .register(&test_agent(
                "director",
                AgentFunction::Director,
                Some("god"),
            ))
            .unwrap();
        agents
            .register(&test_agent(
                "worker",
                AgentFunction::SoftwareEngineer,
                Some("director"),
            ))
            .unwrap();
        let service =
            GovernanceService::new(root.path().to_path_buf(), store, agents.clone(), events);
        Harness {
            _root: root,
            service,
            agents,
        }
    }

    fn create_request(lifecycle: AgentLifecycle) -> CreateAgentRequest {
        CreateAgentRequest {
            id: Some("new-agent".into()),
            name: Some("New Agent".into()),
            seniority: Seniority::Junior,
            function: AgentFunction::BackendEngineering,
            department: Department::Engineering,
            reports_to: Some("director".into()),
            lifecycle,
            authority: AuthorityRole::Worker,
            provider: "mock".into(),
            model: "mock".into(),
            reasoning_effort: "medium".into(),
            permissions: vec![AgentPermission::ReadWorkspace],
            intelligence_policy: Default::default(),
        }
    }

    fn approval_request(root: &Path, request_id: &str, agent_id: &str) -> ProviderApprovalRequest {
        ProviderApprovalRequest {
            provider: "codex".into(),
            process_id: Some(42),
            session_id: format!("thread-{agent_id}"),
            thread_id: format!("thread-{agent_id}"),
            turn_id: format!("turn-{agent_id}"),
            request_id: request_id.into(),
            agent_id: agent_id.into(),
            task_id: format!("TASK-{agent_id}"),
            worktree: Some(root.display().to_string()),
            requested_operation: "cargo test".into(),
            requested_target: Some(root.display().to_string()),
            risk: "COMMAND_EXECUTION".into(),
            detail: serde_json::json!({"command":"cargo test"}),
        }
    }

    fn mutation(actor: &str, revision: u64, mutation: OrganizationMutation) -> MutationRequest {
        MutationRequest {
            actor: Actor {
                id: actor.into(),
                scope: AuthorityScope::Project,
            },
            expected_revision: revision,
            task_id: None,
            reason: Some("test".into()),
            mutation,
        }
    }

    #[test]
    fn deterministic_ids_are_stable_and_unique() {
        assert_eq!(slug("Nova Prime"), "nova-prime");
        let agents = vec![Agent {
            schema_version: 2,
            id: "nova".into(),
            name: "Nova".into(),
            role_template: "Agent".into(),
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
            parent_agent_id: None,
            provider: "mock".into(),
            model: "mock".into(),
            reasoning_effort: "medium".into(),
            auth_mode: "unknown".into(),
            worktree: None,
            status: AgentStatus::Ready,
            current_task_id: None,
            extra: Default::default(),
        }];
        assert_eq!(unique_id("Nova", &agents), "nova-2");
    }

    #[test]
    fn desktop_mutation_contract_uses_readable_camel_case_payloads() {
        let request: MutationRequest = serde_json::from_value(serde_json::json!({
            "actor":{"id":"god","scope":"PROJECT"},
            "expectedRevision":3,
            "taskId":null,
            "reason":"create team",
            "mutation":{"type":"CREATE_AGENT","data":{
                "id":null,"name":"Nova","seniority":"SENIOR",
                "function":"BACKEND_ENGINEERING","department":"ENGINEERING",
                "reportsTo":"director","lifecycle":"PROJECT","authority":"WORKER",
                "provider":"codex","model":"auto","reasoningEffort":"medium",
                "permissions":["READ_WORKSPACE"],
                "intelligencePolicy":{"assignment":"AUTO"}
            }}
        }))
        .unwrap();
        assert_eq!(request.expected_revision, 3);
        assert!(matches!(
            request.mutation,
            OrganizationMutation::CreateAgent(CreateAgentRequest {
                function: AgentFunction::BackendEngineering,
                lifecycle: AgentLifecycle::Project,
                ..
            })
        ));
    }

    #[test]
    fn policy_rejects_overlapping_provider_rules() {
        let policy = ProjectGovernancePolicy {
            allowed_providers: vec!["codex".into()],
            denied_providers: vec!["CODEX".into()],
            ..Default::default()
        };
        assert!(validate_policy(&policy).is_err());
    }

    #[test]
    fn organization_mutation_waits_for_conflicted_recovery_review() {
        let harness = harness();
        let path = harness._root.path().join(".batai/organization.json");
        let mut journal = harness
            .service
            .recovery
            .prepare(
                "UPDATE_ORGANIZATION",
                "god",
                None,
                0,
                1,
                vec![(path.clone(), b"intended".to_vec())],
                vec![],
                None,
                None,
            )
            .unwrap();
        harness.service.recovery.apply_files(&mut journal).unwrap();
        fs::write(path, b"external").unwrap();
        harness.service.recovery.reconcile_startup().unwrap();

        let error = harness
            .service
            .mutate(mutation(
                "director",
                0,
                OrganizationMutation::CreateAgent(create_request(AgentLifecycle::Project)),
            ))
            .unwrap_err();
        assert!(error.to_string().contains("unfinished recovery"));
    }

    #[test]
    fn hierarchy_depth_counts_agent_layers_below_god() {
        let parents = HashMap::from([
            ("director".into(), Some("god".into())),
            ("lead".into(), Some("director".into())),
            ("worker".into(), Some("lead".into())),
        ]);
        assert_eq!(hierarchy_depth("worker", &parents), 2);
    }

    #[test]
    fn director_can_create_project_agent_but_worker_is_denied() {
        let harness = harness();
        let denied = harness
            .service
            .mutate(mutation(
                "worker",
                0,
                OrganizationMutation::CreateAgent(create_request(AgentLifecycle::Project)),
            ))
            .unwrap();
        assert_eq!(denied.disposition, MutationDisposition::Denied);
        assert!(harness.agents.get("new-agent").unwrap().is_none());

        let applied = harness
            .service
            .mutate(mutation(
                "director",
                0,
                OrganizationMutation::CreateAgent(create_request(AgentLifecycle::Project)),
            ))
            .unwrap();
        assert_eq!(applied.disposition, MutationDisposition::Applied);
        assert_eq!(applied.revision, 1);
        assert!(harness.agents.get("new-agent").unwrap().is_some());
    }

    #[test]
    fn permanent_agent_requires_restart_safe_god_decision_and_resolves_once() {
        let harness = harness();
        let pending = harness
            .service
            .mutate(mutation(
                "director",
                0,
                OrganizationMutation::CreateAgent(create_request(AgentLifecycle::Permanent)),
            ))
            .unwrap();
        assert_eq!(pending.disposition, MutationDisposition::PendingGodDecision);
        assert!(harness.agents.get("new-agent").unwrap().is_none());
        let decision_id = pending.decision_id.unwrap();
        let applied = harness
            .service
            .resolve_decision(&decision_id, true, None)
            .unwrap();
        assert_eq!(applied.disposition, MutationDisposition::Applied);
        let duplicate = harness
            .service
            .resolve_decision(&decision_id, true, None)
            .unwrap();
        assert_eq!(duplicate.revision, 1);
        assert_eq!(
            harness
                .agents
                .list()
                .unwrap()
                .iter()
                .filter(|agent| agent.id == "new-agent")
                .count(),
            1
        );
        assert!(harness
            ._root
            .path()
            .join(".batai/decisions")
            .join(format!("{decision_id}.json"))
            .is_file());
    }

    #[test]
    fn stale_revision_and_hierarchy_cycle_fail_without_persisting() {
        let harness = harness();
        let stale = harness.service.mutate(mutation(
            "director",
            5,
            OrganizationMutation::PauseAgent {
                agent_id: "worker".into(),
            },
        ));
        assert!(matches!(
            stale,
            Err(RuntimeError::OrganizationConflict { .. })
        ));
        let cycle = harness.service.mutate(mutation(
            "director",
            0,
            OrganizationMutation::ChangeReportingLine {
                agent_id: "director".into(),
                reports_to: Some("worker".into()),
            },
        ));
        assert!(cycle.is_err());
        assert_eq!(
            harness
                .agents
                .get("director")
                .unwrap()
                .unwrap()
                .parent_agent_id
                .as_deref(),
            Some("god")
        );
    }

    #[test]
    fn persistent_relationships_and_soft_termination_survive_snapshot() {
        let harness = harness();
        let relationship = OrganizationRelationship {
            id: "advisory:director:worker".into(),
            relationship_type: RelationshipType::Advisory,
            source: "director".into(),
            target: "worker".into(),
            persistent: true,
            task_id: None,
            label: Some("advises".into()),
        };
        harness
            .service
            .mutate(mutation(
                "director",
                0,
                OrganizationMutation::AddRelationship { relationship },
            ))
            .unwrap();
        assert_eq!(harness.service.persistent_relationships().unwrap().len(), 1);
        harness
            .service
            .mutate(mutation(
                "director",
                1,
                OrganizationMutation::TerminateAgent {
                    agent_id: "worker".into(),
                    reason: "complete".into(),
                },
            ))
            .unwrap();
        assert_eq!(
            harness.agents.get("worker").unwrap().unwrap().status,
            AgentStatus::Terminated
        );
        assert!(!harness.service.snapshot().unwrap().audit.is_empty());
    }

    #[tokio::test]
    async fn provider_approval_is_per_request_and_idempotent() {
        let harness = harness();
        harness.service.set_interactive_approvals(true);
        let request = ProviderApprovalRequest {
            provider: "codex".into(),
            process_id: Some(42),
            session_id: "thread-1".into(),
            thread_id: "thread-1".into(),
            turn_id: "turn-1".into(),
            request_id: "rpc-7".into(),
            agent_id: "worker".into(),
            task_id: "TASK-1".into(),
            worktree: Some(harness._root.path().display().to_string()),
            requested_operation: "write hello.txt".into(),
            requested_target: Some(harness._root.path().join("hello.txt").display().to_string()),
            risk: "FILESYSTEM_CHANGE".into(),
            detail: serde_json::json!({"path":"hello.txt"}),
        };
        let approval_id = exact_approval_id(&request);
        let service = harness.service.clone();
        let request_duplicate = request.clone();
        let waiter = tokio::spawn(async move { service.handle(request).await });
        for _ in 0..50 {
            if harness
                .service
                .provider_approval_by_id(&approval_id)
                .unwrap()
                .is_some_and(|approval| approval.status == ProviderApprovalStatus::PendingGod)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        let duplicate_service = harness.service.clone();
        let duplicate =
            tokio::spawn(async move { duplicate_service.handle(request_duplicate).await });
        let resolved = harness
            .service
            .resolve_provider_approval(&approval_id, false)
            .unwrap();
        assert_eq!(resolved.status, ProviderApprovalStatus::Denied);
        let directive = waiter.await.unwrap();
        assert_eq!(directive.decision, ProviderApprovalDecision::Deny);
        assert_eq!(
            duplicate.await.unwrap().decision,
            ProviderApprovalDecision::Deny
        );
        assert_eq!(
            harness
                .service
                .snapshot()
                .unwrap()
                .provider_approvals
                .iter()
                .filter(|approval| approval.id == approval_id)
                .count(),
            1
        );
        harness.service.response_result(&approval_id, Ok(()));
        assert_eq!(
            harness
                .service
                .resolve_provider_approval(&approval_id, false)
                .unwrap()
                .status,
            ProviderApprovalStatus::Resolved
        );
        assert!(harness
            .service
            .resolve_provider_approval(&approval_id, true)
            .is_err());
    }

    #[tokio::test]
    async fn provider_approval_headless_and_outside_worktree_fail_closed() {
        let harness = harness();
        let request = approval_request(harness._root.path(), "headless", "worker");
        let id = exact_approval_id(&request);
        let directive = harness.service.handle(request).await;
        assert_eq!(directive.decision, ProviderApprovalDecision::Deny);
        assert_eq!(
            harness
                .service
                .provider_approval_by_id(&id)
                .unwrap()
                .unwrap()
                .resolved_by
                .as_deref(),
            Some("headless-fail-closed")
        );

        harness.service.set_interactive_approvals(true);
        let mut outside = approval_request(harness._root.path(), "outside", "worker");
        outside.requested_target = Some(
            harness
                ._root
                .path()
                .parent()
                .unwrap()
                .join("outside-danger")
                .display()
                .to_string(),
        );
        let outside_id = exact_approval_id(&outside);
        assert_eq!(
            harness.service.handle(outside).await.decision,
            ProviderApprovalDecision::Deny
        );
        assert_eq!(
            harness
                .service
                .provider_approval_by_id(&outside_id)
                .unwrap()
                .unwrap()
                .resolved_by
                .as_deref(),
            Some("system-policy")
        );

        let mut traversal = approval_request(harness._root.path(), "traversal", "worker");
        traversal.requested_target = Some("../outside-danger".into());
        let traversal_id = exact_approval_id(&traversal);
        assert_eq!(
            harness.service.handle(traversal).await.decision,
            ProviderApprovalDecision::Deny
        );
        assert_eq!(
            harness
                .service
                .provider_approval_by_id(&traversal_id)
                .unwrap()
                .unwrap()
                .resolved_by
                .as_deref(),
            Some("system-policy")
        );
    }

    #[tokio::test]
    async fn provider_approval_timeout_and_cancellation_are_bounded() {
        let harness = harness();
        harness.service.set_interactive_approvals(true);
        harness
            .service
            .persist_policy(&ProjectGovernancePolicy {
                provider_approval_timeout_seconds: 1,
                ..ProjectGovernancePolicy::default()
            })
            .unwrap();
        let timeout_request = approval_request(harness._root.path(), "timeout", "worker");
        let timeout_id = exact_approval_id(&timeout_request);
        assert_eq!(
            harness.service.handle(timeout_request).await.decision,
            ProviderApprovalDecision::Deny
        );
        assert_eq!(
            harness
                .service
                .provider_approval_by_id(&timeout_id)
                .unwrap()
                .unwrap()
                .status,
            ProviderApprovalStatus::Expired
        );

        let cancel_request = approval_request(harness._root.path(), "cancel", "worker");
        let session = cancel_request.session_id.clone();
        let cancel_id = exact_approval_id(&cancel_request);
        let service = harness.service.clone();
        let waiter = tokio::spawn(async move { service.handle(cancel_request).await });
        for _ in 0..50 {
            if harness
                .service
                .live_approvals
                .lock()
                .unwrap()
                .contains_key(&cancel_id)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        harness.service.cancel_session(&session);
        assert_eq!(
            waiter.await.unwrap().decision,
            ProviderApprovalDecision::Cancel
        );
        assert_eq!(
            harness
                .service
                .provider_approval_by_id(&cancel_id)
                .unwrap()
                .unwrap()
                .status,
            ProviderApprovalStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn concurrent_approvals_are_independent_and_response_failure_is_uncertain() {
        let harness = harness();
        harness.service.set_interactive_approvals(true);
        let request_a = approval_request(harness._root.path(), "a", "worker");
        let mut request_b = approval_request(harness._root.path(), "b", "agent-b");
        request_b.session_id = "thread-b".into();
        request_b.thread_id = "thread-b".into();
        let id_a = exact_approval_id(&request_a);
        let id_b = exact_approval_id(&request_b);
        let service_a = harness.service.clone();
        let service_b = harness.service.clone();
        let wait_a = tokio::spawn(async move { service_a.handle(request_a).await });
        let wait_b = tokio::spawn(async move { service_b.handle(request_b).await });
        for _ in 0..100 {
            if harness.service.live_approvals.lock().unwrap().len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
        harness
            .service
            .resolve_provider_approval(&id_b, true)
            .unwrap();
        assert_eq!(
            wait_b.await.unwrap().decision,
            ProviderApprovalDecision::AllowOnce
        );
        assert!(
            !wait_a.is_finished(),
            "agent A wait must not block or resolve agent B"
        );
        harness
            .service
            .resolve_provider_approval(&id_a, false)
            .unwrap();
        assert_eq!(
            wait_a.await.unwrap().decision,
            ProviderApprovalDecision::Deny
        );
        harness
            .service
            .response_result(&id_b, Err("app server closed with bearer secret".into()));
        let failed = harness
            .service
            .provider_approval_by_id(&id_b)
            .unwrap()
            .unwrap();
        assert_eq!(failed.status, ProviderApprovalStatus::ResponseUncertain);
        assert!(!failed.response_error.unwrap().contains("secret"));
    }

    #[test]
    fn restart_marks_live_provider_approval_orphaned() {
        let harness = harness();
        let approval = harness
            .service
            .request_provider_approval(
                "codex".into(),
                Some("worker".into()),
                None,
                "cargo test".into(),
                serde_json::json!({}),
            )
            .unwrap();
        harness.service.reconcile_startup().unwrap();
        assert_eq!(
            harness
                .service
                .provider_approval_by_id(&approval.id)
                .unwrap()
                .unwrap()
                .status,
            ProviderApprovalStatus::Orphaned
        );
        assert!(harness
            .service
            .resolve_provider_approval(&approval.id, true)
            .is_err());
    }

    #[test]
    fn provider_approval_details_redact_credentials() {
        let redacted = redact_provider_detail(serde_json::json!({
            "authorization":"Bearer private",
            "nested":{"api_key":"sk-private"},
            "path":"hello.txt"
        }));
        assert_eq!(redacted["authorization"], "[REDACTED]");
        assert_eq!(redacted["nested"]["api_key"], "[REDACTED]");
        assert_eq!(redacted["path"], "hello.txt");
    }

    #[test]
    fn system_provider_deny_precedes_even_god_and_second_director_is_rejected() {
        let harness = harness();
        harness
            .service
            .persist_policy(&ProjectGovernancePolicy {
                denied_providers: vec!["mock".into()],
                ..Default::default()
            })
            .unwrap();
        let denied = harness.service.mutate(mutation(
            "god",
            0,
            OrganizationMutation::CreateAgent(create_request(AgentLifecycle::Project)),
        ));
        assert!(matches!(denied, Err(RuntimeError::Governance(_))));
        assert!(harness.agents.get("new-agent").unwrap().is_none());

        harness
            .service
            .persist_policy(&ProjectGovernancePolicy::default())
            .unwrap();
        let mut second = create_request(AgentLifecycle::Project);
        second.id = Some("second-director".into());
        second.function = AgentFunction::Director;
        second.seniority = Seniority::Director;
        second.department = Department::Leadership;
        second.authority = AuthorityRole::Director;
        second.reports_to = Some("god".into());
        assert!(harness
            .service
            .mutate(mutation(
                "god",
                0,
                OrganizationMutation::CreateAgent(second)
            ))
            .is_err());
    }

    #[test]
    fn protected_external_action_is_bound_to_god_decision() {
        let harness = harness();
        let pending = harness
            .service
            .mutate(mutation(
                "director",
                0,
                OrganizationMutation::RequestProtectedAction {
                    operation: ProtectedOperation::ProductionDeployment,
                    target: "production".into(),
                    detail: serde_json::json!({"release":"v1"}),
                },
            ))
            .unwrap();
        assert_eq!(pending.disposition, MutationDisposition::PendingGodDecision);
        let decision = harness
            .service
            .snapshot()
            .unwrap()
            .decisions
            .into_iter()
            .find(|decision| decision.id == pending.decision_id.clone().unwrap())
            .unwrap();
        assert!(matches!(
            decision.request.mutation,
            OrganizationMutation::RequestProtectedAction {
                operation: ProtectedOperation::ProductionDeployment,
                ..
            }
        ));
    }

    #[test]
    fn review_outcomes_drive_acceptance_metric_without_fake_values() {
        let harness = harness();
        assert_eq!(harness.service.review_acceptance("worker").unwrap(), None);
        for (index, outcome) in [ReviewOutcomeKind::Accepted, ReviewOutcomeKind::Rejected]
            .into_iter()
            .enumerate()
        {
            harness
                .service
                .record_review(ReviewOutcome {
                    id: format!("review-{index}"),
                    task_id: format!("task-{index}"),
                    reviewer_id: "director".into(),
                    subject_agent_id: "worker".into(),
                    outcome,
                    note: None,
                    created_at: now(),
                })
                .unwrap();
        }
        assert_eq!(
            harness.service.review_acceptance("worker").unwrap(),
            Some(50.0)
        );
    }

    #[test]
    fn partial_external_agent_write_is_rejected_without_corrupting_runtime() {
        let harness = harness();
        let path = harness
            ._root
            .path()
            .join(".batai/agents/worker/config.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, br#"{"id":"worker""#).unwrap();
        assert!(harness.service.refresh_external_path(&path).is_err());
        assert_eq!(
            harness.agents.get("worker").unwrap().unwrap().name,
            "worker"
        );
    }
}
