use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    agents::AgentRegistry,
    errors::{Result, RuntimeError},
    events::EventEngine,
    organization::{
        hierarchy_warnings, AgentFunction, AgentLifecycle, AgentPermission, AuthorityRole,
        Department, IntelligencePolicy, OrganizationRelationship, RelationshipType, Seniority,
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
    Pending,
    Approved,
    Rejected,
    Expired,
}

impl ProviderApprovalStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Approved => "APPROVED",
            Self::Rejected => "REJECTED",
            Self::Expired => "EXPIRED",
        }
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
    pub operation: String,
    pub detail: Value,
    pub status: ProviderApprovalStatus,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub resolved_by: Option<String>,
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
}

#[derive(Clone)]
pub struct GovernanceService {
    root: PathBuf,
    store: RuntimeStore,
    agents: AgentRegistry,
    events: EventEngine,
    mutation_lock: Arc<Mutex<()>>,
}

impl GovernanceService {
    pub fn new(
        root: PathBuf,
        store: RuntimeStore,
        agents: AgentRegistry,
        events: EventEngine,
    ) -> Self {
        Self {
            root,
            store,
            agents,
            events,
            mutation_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn snapshot(&self) -> Result<GovernanceSnapshot> {
        let organization = self.load_organization()?;
        Ok(GovernanceSnapshot {
            revision: organization.revision,
            policy: self.load_policy()?,
            decisions: self.store.list_governance_records("GOD_DECISION")?,
            provider_approvals: self.store.list_governance_records("PROVIDER_APPROVAL")?,
            audit: self.store.list_governance_audit(150)?,
        })
    }

    pub fn persistent_relationships(&self) -> Result<Vec<OrganizationRelationship>> {
        Ok(self.load_organization()?.relationships)
    }

    pub fn refresh_external_path(&self, path: &Path) -> Result<()> {
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
        self.mutate_locked(request, false)
    }

    fn mutate_locked(
        &self,
        request: MutationRequest,
        god_resolution: bool,
    ) -> Result<MutationResult> {
        let organization = self.load_organization()?;
        let authority = if god_resolution {
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
                match self.apply(request.clone(), authority, organization, policy) {
                    Ok(result) => Ok(result),
                    Err(error) => {
                        self.audit(
                            &request,
                            authority,
                            MutationDisposition::Denied,
                            None,
                            revision,
                        )?;
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
        let result = self.mutate_locked(decision.request.clone(), true)?;
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
            operation,
            detail: redact_provider_detail(detail),
            status: ProviderApprovalStatus::Pending,
            created_at: now(),
            resolved_at: None,
            resolved_by: None,
        };
        self.store.upsert_governance_record(
            &approval.id,
            "PROVIDER_APPROVAL",
            approval.status.label(),
            &approval,
        )?;
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
        if approval.status == ProviderApprovalStatus::Pending {
            approval.status = if approve {
                ProviderApprovalStatus::Approved
            } else {
                ProviderApprovalStatus::Rejected
            };
            approval.resolved_at = Some(now());
            approval.resolved_by = Some("god".into());
            self.store.upsert_governance_record(
                &approval.id,
                "PROVIDER_APPROVAL",
                approval.status.label(),
                &approval,
            )?;
            self.events.publish(
                EventType::ProviderApprovalResolved,
                "god",
                Some(approval.id.clone()),
                approval.task_id.clone(),
                serde_json::to_value(&approval)?,
            )?;
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
        self.resolve_provider_approval(&approval.id, false)?;
        Ok(())
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
    ) -> Result<MutationResult> {
        let affected_id = request.mutation.target().map(str::to_owned);
        let event_type;
        match &request.mutation {
            OrganizationMutation::CreateAgent(create) => {
                let agent = self.build_agent(create, &policy)?;
                self.validate_agent_change(&agent, None, &policy)?;
                self.persist_agent(&agent)?;
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
                self.persist_agent(&agent)?;
                event_type = EventType::AgentUpdated;
            }
            OrganizationMutation::ChangeReportingLine {
                agent_id,
                reports_to,
            } => {
                let mut agent = self.require_agent(agent_id)?;
                agent.parent_agent_id.clone_from(reports_to);
                self.validate_agent_change(&agent, Some(agent_id), &policy)?;
                self.persist_agent(&agent)?;
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
                self.persist_agent(&agent)?;
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
                self.persist_agent(&agent)?;
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
                self.agents
                    .transition(agent_id, AgentStatus::Paused, None)?;
                event_type = EventType::AgentUpdated;
            }
            OrganizationMutation::ResumeAgent { agent_id } => {
                self.agents.transition(agent_id, AgentStatus::Ready, None)?;
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
                self.agents
                    .transition(agent_id, AgentStatus::Terminated, None)?;
                let mut persisted = agent;
                persisted.status = AgentStatus::Terminated;
                self.persist_agent(&persisted)?;
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
        self.persist_organization(&organization)?;
        if matches!(
            request.mutation,
            OrganizationMutation::UpdateProjectPolicy { .. }
        ) {
            self.persist_policy(&policy)?;
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
        Ok(MutationResult {
            disposition: MutationDisposition::Applied,
            revision: organization.revision,
            decision_id: None,
            message: "organization mutation applied".into(),
            affected_id,
        })
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

    #[test]
    fn provider_approval_is_per_request_and_idempotent() {
        let harness = harness();
        let approval = harness
            .service
            .request_provider_approval(
                "codex".into(),
                Some("worker".into()),
                Some("TASK-1".into()),
                "WRITE_FILE".into(),
                serde_json::json!({"path":"hello.txt"}),
            )
            .unwrap();
        let resolved = harness
            .service
            .resolve_provider_approval(&approval.id, false)
            .unwrap();
        assert_eq!(resolved.status, ProviderApprovalStatus::Rejected);
        assert_eq!(
            harness
                .service
                .resolve_provider_approval(&approval.id, true)
                .unwrap()
                .status,
            ProviderApprovalStatus::Rejected
        );
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
