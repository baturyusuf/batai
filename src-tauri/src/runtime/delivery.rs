//! Provider-neutral software delivery lifecycle.
//!
//! Local Git and remote GitHub state are deliberately separate. Remote mutations use a
//! durable saga journal: Batai does not claim a transaction across Git, GitHub and SQLite.

use std::{path::PathBuf, process::Command};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::providers::github_cli::GitHubService;

use super::{
    errors::{Result, RuntimeError},
    events::EventEngine,
    governance::{
        Actor, GovernanceService, MutationDisposition, MutationRequest, OrganizationMutation,
        ProtectedOperation, ReviewOutcome, ReviewOutcomeKind,
    },
    store::RuntimeStore,
    types::{EventType, IngestDisposition, Task, TaskStatus},
    worktrees::{WorktreeBinding, WorktreeManager},
};

macro_rules! typed_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "SCREAMING_SNAKE_CASE")]
        pub enum $name { $($variant),+ }
    };
}

typed_enum!(GitHubAuthState {
    NotInstalled,
    AuthRequired,
    Available,
    Error
});
typed_enum!(IssueState { Open, Closed });
typed_enum!(PullRequestState {
    Open,
    Closed,
    Merged
});
typed_enum!(ReviewState {
    Unknown,
    Required,
    Pending,
    Approved,
    ChangesRequested,
    Dismissed,
    Rejected
});
typed_enum!(CheckState {
    Unknown,
    Pending,
    Pass,
    Fail,
    Cancelled,
    Skipped
});
typed_enum!(CiState {
    Unknown,
    Pending,
    Pass,
    Fail,
    Cancelled
});
typed_enum!(DeliveryState {
    NotRequired,
    Worktree,
    ChangesReady,
    Committed,
    Pushed,
    PullRequestOpen,
    Review,
    CiPending,
    MergeReady,
    Merged,
    Closed,
    Blocked,
    Failed
});
typed_enum!(MergePolicy {
    Manual,
    GodApproval,
    AutomaticSafe
});
typed_enum!(MergeMethod {
    Squash,
    MergeCommit,
    Rebase
});
typed_enum!(RemoteOperationType {
    CreateIssue,
    PushBranch,
    CreatePullRequest,
    UpdatePullRequest,
    MergePullRequest,
    CloseIssue
});
typed_enum!(RemoteOperationPhase {
    Prepared,
    RemoteApplied,
    LocalApplied,
    Committed,
    NeedsReview,
    RolledBack
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryIdentity {
    pub owner: String,
    pub name: String,
    pub slug: String,
    pub remote_url: String,
    pub default_branch: String,
    pub remote_name: String,
    pub current_upstream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitHubIssue {
    pub number: u64,
    pub title: String,
    pub body_summary: String,
    pub state: IssueState,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CheckRunSummary {
    pub name: String,
    pub status: String,
    pub conclusion: CheckState,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitHubReview {
    pub reviewer: String,
    pub state: ReviewState,
    pub summary: String,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub base: String,
    pub base_sha: Option<String>,
    pub head: String,
    pub head_sha: String,
    pub state: PullRequestState,
    pub draft: bool,
    pub mergeability: Option<String>,
    pub review_state: ReviewState,
    pub ci_state: CiState,
    pub checks: Vec<CheckRunSummary>,
    pub reviews: Vec<GitHubReview>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalLink {
    pub id: String,
    pub task_id: String,
    pub provider: String,
    pub repository: String,
    pub entity_type: String,
    pub entity_number: u64,
    pub url: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct DeliveryPolicy {
    pub required: bool,
    pub auto_commit: bool,
    pub auto_push: bool,
    pub create_pull_request: bool,
    pub draft_pull_request: bool,
    pub merge_policy: MergePolicy,
    pub merge_method: MergeMethod,
    pub close_linked_issue_after_merge: bool,
    pub max_delivery_attempts: u32,
}

impl Default for DeliveryPolicy {
    fn default() -> Self {
        Self {
            required: false,
            auto_commit: true,
            auto_push: true,
            create_pull_request: true,
            draft_pull_request: false,
            merge_policy: MergePolicy::GodApproval,
            merge_method: MergeMethod::Squash,
            close_linked_issue_after_merge: false,
            max_delivery_attempts: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryCheckpoint {
    pub task_id: String,
    pub repository: RepositoryIdentity,
    pub state: DeliveryState,
    pub policy: DeliveryPolicy,
    pub branch: String,
    pub base: String,
    pub base_sha: String,
    pub head_sha: Option<String>,
    pub commit_ids: Vec<String>,
    pub worktree_path: String,
    pub issue_number: Option<u64>,
    pub pull_request: Option<PullRequest>,
    pub merge_decision_id: Option<String>,
    pub merge_approved_head_sha: Option<String>,
    pub delivery_attempts: u32,
    pub blocked_reason: Option<String>,
    pub last_synced_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteOperationJournal {
    pub id: String,
    pub operation_type: RemoteOperationType,
    pub phase: RemoteOperationPhase,
    pub task_id: String,
    pub actor: String,
    pub repository: String,
    pub target: String,
    pub idempotency_key: String,
    pub expected_head_sha: Option<String>,
    pub remote_entity_number: Option<u64>,
    pub detail: Value,
    pub failure: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryRecoverySummary {
    pub recovered: usize,
    pub needs_review: usize,
}

#[derive(Clone)]
pub struct DeliveryService {
    root: PathBuf,
    store: RuntimeStore,
    events: EventEngine,
    governance: GovernanceService,
    github: GitHubService,
    worktrees: Option<WorktreeManager>,
}

impl DeliveryService {
    pub fn new(
        root: PathBuf,
        store: RuntimeStore,
        events: EventEngine,
        governance: GovernanceService,
    ) -> Self {
        let github = GitHubService::new(&root);
        let worktrees = WorktreeManager::discover(&root).ok();
        Self {
            root,
            store,
            events,
            governance,
            github,
            worktrees,
        }
    }

    #[cfg(test)]
    pub fn with_github(mut self, github: GitHubService) -> Self {
        self.github = github;
        self
    }

    pub fn auth_status(&self) -> GitHubAuthState {
        self.github.auth_status()
    }

    pub fn repository(&self) -> Result<RepositoryIdentity> {
        let mut identity = self.github.repository()?;
        identity.current_upstream = git_text(
            &self.root,
            &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
        )
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
        if let Some(remote) = identity
            .current_upstream
            .as_deref()
            .and_then(|upstream| upstream.split_once('/').map(|(remote, _)| remote))
        {
            identity.remote_name = remote.to_owned();
        }
        let remote_url = git_text(&self.root, &["remote", "get-url", &identity.remote_name])?
            .trim()
            .to_owned();
        let public_remote_url = sanitized_remote_url(&remote_url);
        if let Some(origin_slug) = github_slug(&public_remote_url) {
            if !origin_slug.eq_ignore_ascii_case(&identity.slug) {
                return Err(RuntimeError::Governance(format!(
                    "GitHub repository {} does not match remote {}",
                    identity.slug, origin_slug
                )));
            }
        }
        identity.remote_url = public_remote_url;
        Ok(identity)
    }

    pub fn import_issue(&self, number: u64, actor: &str) -> Result<Task> {
        let repo = self.repository()?;
        if let Some(link) = self
            .store
            .external_link_by_entity("GITHUB", &repo.slug, "ISSUE", number)?
        {
            return self
                .store
                .get_task(&link.task_id)?
                .ok_or_else(|| RuntimeError::TaskNotFound(link.task_id));
        }
        let issue = self.github.issue(&repo, number)?;
        let task_id = issue_task_id(&repo.slug, issue.number);
        let task = Task {
            id: task_id.clone(),
            created_by: actor.into(),
            objective: issue.title.clone(),
            assigned_to: Vec::new(),
            dependencies: Vec::new(),
            acceptance_criteria: Vec::new(),
            inputs: vec![format!(
                "GitHub issue #{}: {}",
                issue.number, issue.body_summary
            )],
            outputs: Vec::new(),
            status: TaskStatus::Pending,
            execution: Default::default(),
            on_success: Default::default(),
            on_failure: Default::default(),
            weight: 1.0,
            extra: serde_json::Map::from_iter([(
                String::from("delivery"),
                serde_json::to_value(DeliveryPolicy {
                    required: true,
                    ..DeliveryPolicy::default()
                })?,
            )]),
        };
        let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&task)?));
        let disposition = self.store.ingest_task(&task, None, &hash)?;
        let link = ExternalLink {
            id: format!("LINK-{}", uuid::Uuid::new_v4()),
            task_id: task_id.clone(),
            provider: "GITHUB".into(),
            repository: repo.slug.clone(),
            entity_type: "ISSUE".into(),
            entity_number: issue.number,
            url: issue.url.clone(),
            created_at: now(),
            updated_at: now(),
        };
        self.store.upsert_external_link(&link)?;
        if disposition == IngestDisposition::Created {
            self.events.publish(
                EventType::GithubIssueLinked,
                actor,
                None,
                Some(task_id.clone()),
                serde_json::to_value(&link)?,
            )?;
        }
        Ok(task)
    }

    pub fn create_issue_for_task(
        &self,
        task_id: &str,
        actor: Actor,
        labels: &[String],
    ) -> Result<ExternalLink> {
        let task = self.require_task(task_id)?;
        let repo = self.repository()?;
        if let Some(link) = self.store.external_link_by_task(task_id, "ISSUE")? {
            return Ok(link);
        }
        self.authorize(
            &actor,
            task_id,
            ProtectedOperation::CreateGithubIssue,
            &format!("{}#issue", repo.slug),
            serde_json::json!({"repository":repo.slug,"taskId":task_id}),
        )?;
        let mut journal = self.prepare_remote(
            RemoteOperationType::CreateIssue,
            task_id,
            &actor.id,
            &repo,
            task_id,
            None,
            serde_json::json!({"title":task.objective}),
        )?;
        let issue = self.github.create_issue(
            &repo,
            &task.objective,
            &format!("Created from Batai task `{task_id}`.\n\nBatai-Task: {task_id}"),
            labels,
        )?;
        journal.phase = RemoteOperationPhase::RemoteApplied;
        journal.remote_entity_number = Some(issue.number);
        journal.updated_at = now();
        self.store.upsert_remote_operation(&journal)?;
        let link = ExternalLink {
            id: format!("LINK-{}", uuid::Uuid::new_v4()),
            task_id: task_id.into(),
            provider: "GITHUB".into(),
            repository: repo.slug.clone(),
            entity_type: "ISSUE".into(),
            entity_number: issue.number,
            url: issue.url,
            created_at: now(),
            updated_at: now(),
        };
        self.store.upsert_external_link(&link)?;
        self.finish_remote(&mut journal)?;
        self.events.publish(
            EventType::GithubIssueLinked,
            actor.id,
            None,
            Some(task_id.into()),
            serde_json::to_value(&link)?,
        )?;
        Ok(link)
    }

    pub fn prepare_worktree(
        &self,
        task_id: &str,
        agent_id: &str,
        base: Option<&str>,
        policy: DeliveryPolicy,
    ) -> Result<DeliveryCheckpoint> {
        let repo = self.repository()?;
        let manager = self
            .worktrees
            .as_ref()
            .ok_or_else(|| RuntimeError::Provider("Git worktree manager unavailable".into()))?;
        let base = base.unwrap_or(&repo.default_branch).to_owned();
        let binding = manager.ensure_from(agent_id, task_id, &base)?;
        if binding.branch == repo.default_branch {
            return Err(RuntimeError::Governance(
                "coding agents cannot use the default branch".into(),
            ));
        }
        let checkpoint = DeliveryCheckpoint {
            task_id: task_id.into(),
            repository: repo,
            state: DeliveryState::Worktree,
            policy,
            branch: binding.branch,
            base,
            base_sha: binding.base_commit,
            head_sha: Some(binding.starting_head),
            commit_ids: Vec::new(),
            worktree_path: binding.path.to_string_lossy().into_owned(),
            issue_number: self
                .store
                .external_link_by_task(task_id, "ISSUE")?
                .map(|link| link.entity_number),
            pull_request: None,
            merge_decision_id: None,
            merge_approved_head_sha: None,
            delivery_attempts: 0,
            blocked_reason: None,
            last_synced_at: now(),
        };
        self.store.upsert_delivery_checkpoint(&checkpoint)?;
        Ok(checkpoint)
    }

    pub fn commit(
        &self,
        task_id: &str,
        agent_id: &str,
        summary: Option<&str>,
    ) -> Result<DeliveryCheckpoint> {
        let mut checkpoint = self.require_checkpoint(task_id)?;
        self.ensure_not_default_branch(&checkpoint)?;
        let manager = self
            .worktrees
            .as_ref()
            .ok_or_else(|| RuntimeError::Provider("Git worktree manager unavailable".into()))?;
        let binding = checkpoint.binding();
        let status = manager.status(&binding)?;
        if status.is_empty() {
            return Err(RuntimeError::Provider(
                "worktree has no changes to commit".into(),
            ));
        }
        manager.validate_commit_files(&binding, 25 * 1024 * 1024)?;
        let subject = summary
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("deliver task");
        let message = format!("chore: {subject}\n\nBatai-Task: {task_id}\nBatai-Agent: {agent_id}");
        let commit = manager
            .commit(&binding, &message)?
            .ok_or_else(|| RuntimeError::Provider("commit produced no revision".into()))?;
        checkpoint.head_sha = Some(commit.clone());
        checkpoint.commit_ids.push(commit);
        checkpoint.state = DeliveryState::Committed;
        checkpoint.last_synced_at = now();
        checkpoint.pull_request = checkpoint.pull_request.take().map(|mut pr| {
            pr.ci_state = CiState::Pending;
            pr.review_state = ReviewState::Required;
            pr
        });
        self.store.upsert_delivery_checkpoint(&checkpoint)?;
        self.events.publish(
            EventType::DeliveryCommitted,
            agent_id,
            None,
            Some(task_id.into()),
            serde_json::json!({"headSha":checkpoint.head_sha,"branch":checkpoint.branch}),
        )?;
        Ok(checkpoint)
    }

    pub fn push(&self, task_id: &str, actor: &str) -> Result<DeliveryCheckpoint> {
        let mut checkpoint = self.require_checkpoint(task_id)?;
        self.ensure_not_default_branch(&checkpoint)?;
        let head = checkpoint
            .head_sha
            .clone()
            .ok_or_else(|| RuntimeError::Provider("delivery has no commit".into()))?;
        let repo = checkpoint.repository.clone();
        self.authorize(
            &Actor {
                id: actor.into(),
                scope: super::governance::AuthorityScope::Project,
            },
            task_id,
            ProtectedOperation::PushBranch,
            &checkpoint.branch,
            serde_json::json!({"repository":repo.slug,"branch":checkpoint.branch,"headSha":head}),
        )?;
        let mut journal = self.prepare_remote(
            RemoteOperationType::PushBranch,
            task_id,
            actor,
            &repo,
            &checkpoint.branch,
            Some(&head),
            serde_json::json!({"remote":repo.remote_name,"branch":checkpoint.branch}),
        )?;
        let output = Command::new("git")
            .current_dir(&checkpoint.worktree_path)
            .args([
                "push",
                "--set-upstream",
                &repo.remote_name,
                &checkpoint.branch,
            ])
            .output()?;
        if !output.status.success() {
            journal.phase = RemoteOperationPhase::NeedsReview;
            journal.failure = Some(classify_push(&output));
            journal.updated_at = now();
            self.store.upsert_remote_operation(&journal)?;
            return Err(RuntimeError::Provider(
                journal.failure.clone().unwrap_or_default(),
            ));
        }
        journal.phase = RemoteOperationPhase::RemoteApplied;
        journal.updated_at = now();
        self.store.upsert_remote_operation(&journal)?;
        checkpoint.state = DeliveryState::Pushed;
        checkpoint.last_synced_at = now();
        self.store.upsert_delivery_checkpoint(&checkpoint)?;
        self.finish_remote(&mut journal)?;
        self.events.publish(
            EventType::BranchPushed,
            actor,
            None,
            Some(task_id.into()),
            serde_json::json!({"branch":checkpoint.branch,"headSha":head}),
        )?;
        Ok(checkpoint)
    }

    pub fn create_pull_request(
        &self,
        task_id: &str,
        actor: Actor,
        tests: &[String],
    ) -> Result<DeliveryCheckpoint> {
        let mut checkpoint = self.require_checkpoint(task_id)?;
        self.ensure_not_default_branch(&checkpoint)?;
        if let Some(pr) = &checkpoint.pull_request {
            if pr.state == PullRequestState::Open {
                return Ok(checkpoint);
            }
        }
        let task = self.require_task(task_id)?;
        let repo = checkpoint.repository.clone();
        self.authorize(&actor, task_id, ProtectedOperation::CreatePullRequest, &checkpoint.branch, serde_json::json!({"repository":repo.slug,"base":checkpoint.base,"head":checkpoint.branch}))?;
        let head = checkpoint
            .head_sha
            .clone()
            .ok_or_else(|| RuntimeError::Provider("delivery has no head commit".into()))?;
        let mut journal = self.prepare_remote(
            RemoteOperationType::CreatePullRequest,
            task_id,
            &actor.id,
            &repo,
            &checkpoint.branch,
            Some(&head),
            serde_json::json!({"base":checkpoint.base,"head":checkpoint.branch}),
        )?;
        let agent = task
            .assigned_to
            .first()
            .cloned()
            .unwrap_or_else(|| "unassigned".into());
        let body = pr_body(&task, &agent, tests, &checkpoint);
        let pr = self.github.create_pull_request(
            &repo,
            &task.objective,
            &body,
            &checkpoint.base,
            &checkpoint.branch,
            checkpoint.policy.draft_pull_request,
        )?;
        journal.phase = RemoteOperationPhase::RemoteApplied;
        journal.remote_entity_number = Some(pr.number);
        journal.updated_at = now();
        self.store.upsert_remote_operation(&journal)?;
        let link = ExternalLink {
            id: format!("LINK-{}", uuid::Uuid::new_v4()),
            task_id: task_id.into(),
            provider: "GITHUB".into(),
            repository: repo.slug.clone(),
            entity_type: "PULL_REQUEST".into(),
            entity_number: pr.number,
            url: pr.url.clone(),
            created_at: now(),
            updated_at: now(),
        };
        self.store.upsert_external_link(&link)?;
        checkpoint.pull_request = Some(pr.clone());
        checkpoint.state = DeliveryState::PullRequestOpen;
        checkpoint.last_synced_at = now();
        self.store.upsert_delivery_checkpoint(&checkpoint)?;
        self.finish_remote(&mut journal)?;
        let first_check = (Utc::now() + chrono::Duration::seconds(15)).to_rfc3339();
        self.store.schedule_github_check(task_id, &first_check)?;
        self.events.publish(
            EventType::PullRequestCreated,
            actor.id,
            None,
            Some(task_id.into()),
            serde_json::to_value(&pr)?,
        )?;
        Ok(checkpoint)
    }

    pub fn sync(&self, task_id: &str) -> Result<DeliveryCheckpoint> {
        let mut checkpoint = self.require_checkpoint(task_id)?;
        let old = checkpoint.pull_request.clone();
        let number = old
            .as_ref()
            .map(|pr| pr.number)
            .or_else(|| {
                self.store
                    .external_link_by_task(task_id, "PULL_REQUEST")
                    .ok()
                    .flatten()
                    .map(|link| link.entity_number)
            })
            .ok_or_else(|| RuntimeError::Provider("task has no pull request".into()))?;
        let mut pr = self.github.pull_request(&checkpoint.repository, number)?;
        let (ci, checks) = self.github.checks(&checkpoint.repository, number)?;
        pr.ci_state = ci;
        pr.checks = checks;
        if pr.base != checkpoint.base {
            checkpoint.state = DeliveryState::Blocked;
            checkpoint.blocked_reason = Some(format!(
                "pull request base changed from {} to {}",
                checkpoint.base, pr.base
            ));
            checkpoint.pull_request = Some(pr);
            checkpoint.last_synced_at = now();
            self.store.upsert_delivery_checkpoint(&checkpoint)?;
            return Ok(checkpoint);
        }
        if old
            .as_ref()
            .is_some_and(|previous| previous.head_sha != pr.head_sha)
        {
            checkpoint.merge_decision_id = None;
            checkpoint.merge_approved_head_sha = None;
        }
        checkpoint.head_sha = Some(pr.head_sha.clone());
        checkpoint.state = delivery_state_for_pr(&pr);
        checkpoint.pull_request = Some(pr.clone());
        checkpoint.last_synced_at = now();
        self.store.upsert_delivery_checkpoint(&checkpoint)?;
        if matches!(pr.ci_state, CiState::Pending | CiState::Unknown)
            && pr.state == PullRequestState::Open
        {
            let retry = (Utc::now() + chrono::Duration::minutes(2)).to_rfc3339();
            self.store.schedule_github_check(task_id, &retry)?;
        }
        if old.as_ref().map(|value| value.ci_state) != Some(pr.ci_state) {
            let event = match pr.ci_state {
                CiState::Pending => EventType::CiPending,
                CiState::Pass => EventType::CiPassed,
                CiState::Fail => EventType::CiFailed,
                _ => EventType::PullRequestUpdated,
            };
            self.events.publish(
                event,
                "github",
                None,
                Some(task_id.into()),
                serde_json::json!({"pullRequest":number,"ci":pr.ci_state}),
            )?;
        }
        if pr.state == PullRequestState::Merged
            && old
                .as_ref()
                .is_none_or(|value| value.state != PullRequestState::Merged)
        {
            self.store.set_task_status(task_id, TaskStatus::Completed)?;
            self.events.publish(
                EventType::PullRequestMerged,
                "github",
                None,
                Some(task_id.into()),
                serde_json::to_value(&pr)?,
            )?;
        }
        Ok(checkpoint)
    }

    pub fn request_github_review(
        &self,
        task_id: &str,
        actor: Actor,
        reviewer: &str,
    ) -> Result<DeliveryCheckpoint> {
        if reviewer.trim().is_empty() {
            return Err(RuntimeError::Governance(
                "an explicit GitHub username is required".into(),
            ));
        }
        let mut checkpoint = self.require_checkpoint(task_id)?;
        let pr = checkpoint
            .pull_request
            .clone()
            .ok_or_else(|| RuntimeError::Provider("task has no pull request".into()))?;
        self.authorize(
            &actor,
            task_id,
            ProtectedOperation::RequestGithubReview,
            &format!("{}#{}", checkpoint.repository.slug, pr.number),
            serde_json::json!({"pullRequest":pr.number,"reviewer":reviewer}),
        )?;
        let updated = self
            .github
            .request_review(&checkpoint.repository, pr.number, reviewer)?;
        checkpoint.pull_request = Some(updated);
        checkpoint.state = DeliveryState::Review;
        checkpoint.last_synced_at = now();
        self.store.upsert_delivery_checkpoint(&checkpoint)?;
        self.events.publish(
            EventType::PullRequestReviewRequired,
            actor.id,
            Some(reviewer.into()),
            Some(task_id.into()),
            serde_json::json!({"pullRequest":pr.number}),
        )?;
        Ok(checkpoint)
    }

    pub fn record_internal_review(&self, review: ReviewOutcome) -> Result<DeliveryCheckpoint> {
        self.governance.record_review(review.clone())?;
        let mut checkpoint = self.require_checkpoint(&review.task_id)?;
        match review.outcome {
            ReviewOutcomeKind::Accepted => {
                checkpoint.state = if checkpoint
                    .pull_request
                    .as_ref()
                    .is_some_and(|pr| pr.ci_state == CiState::Pass)
                {
                    DeliveryState::MergeReady
                } else {
                    DeliveryState::Review
                };
            }
            ReviewOutcomeKind::ChangesRequested => {
                checkpoint.delivery_attempts = checkpoint.delivery_attempts.saturating_add(1);
                checkpoint.state =
                    if checkpoint.delivery_attempts >= checkpoint.policy.max_delivery_attempts {
                        DeliveryState::Blocked
                    } else {
                        DeliveryState::ChangesReady
                    };
                checkpoint.blocked_reason = (checkpoint.state == DeliveryState::Blocked)
                    .then(|| "delivery retry budget exhausted".into());
            }
            ReviewOutcomeKind::Rejected => {
                checkpoint.state = DeliveryState::Blocked;
                checkpoint.blocked_reason = Some("internal review rejected the delivery".into());
            }
        }
        checkpoint.last_synced_at = now();
        self.store.upsert_delivery_checkpoint(&checkpoint)?;
        Ok(checkpoint)
    }

    pub fn request_merge_approval(
        &self,
        task_id: &str,
        actor: Actor,
    ) -> Result<DeliveryCheckpoint> {
        let mut checkpoint = self.sync(task_id)?;
        let pr = checkpoint
            .pull_request
            .clone()
            .ok_or_else(|| RuntimeError::Provider("task has no pull request".into()))?;
        if pr.ci_state != CiState::Pass || pr.review_state != ReviewState::Approved {
            return Err(RuntimeError::Governance(
                "merge gate requires CI PASS and approved review".into(),
            ));
        }
        if pr
            .mergeability
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("CONFLICTING"))
        {
            return Err(RuntimeError::Governance(
                "pull request has a merge conflict".into(),
            ));
        }
        let direct_god = actor.id.eq_ignore_ascii_case("god");
        let result=self.governance.mutate(MutationRequest{actor,expected_revision:self.governance.snapshot()?.revision,task_id:Some(task_id.into()),reason:Some("merge reviewed pull request".into()),mutation:OrganizationMutation::RequestProtectedAction{operation:ProtectedOperation::MergePullRequest,target:format!("{}#{}@{}",checkpoint.repository.slug,pr.number,pr.head_sha),detail:serde_json::json!({"repository":checkpoint.repository.slug,"pullRequest":pr.number,"headSha":pr.head_sha,"ci":pr.ci_state,"review":pr.review_state})}})?;
        checkpoint.merge_decision_id = result
            .decision_id
            .or_else(|| direct_god.then(|| format!("DIRECT_GOD:{}", pr.head_sha)));
        checkpoint.merge_approved_head_sha = Some(pr.head_sha);
        checkpoint.last_synced_at = now();
        self.store.upsert_delivery_checkpoint(&checkpoint)?;
        Ok(checkpoint)
    }

    pub fn merge(&self, task_id: &str, actor: &str) -> Result<DeliveryCheckpoint> {
        let mut checkpoint = self.sync(task_id)?;
        if checkpoint.policy.merge_policy == MergePolicy::Manual {
            return Err(RuntimeError::Governance(
                "merge policy is MANUAL; merge on GitHub".into(),
            ));
        }
        let pr = checkpoint
            .pull_request
            .clone()
            .ok_or_else(|| RuntimeError::Provider("task has no pull request".into()))?;
        if checkpoint.policy.merge_policy == MergePolicy::AutomaticSafe
            && !self
                .governance
                .snapshot()?
                .policy
                .automatic_safe_github_merge
        {
            return Err(RuntimeError::Governance(
                "AUTOMATIC_SAFE merge is disabled by project governance".into(),
            ));
        }
        if checkpoint.policy.merge_policy != MergePolicy::AutomaticSafe {
            let decision_id = checkpoint.merge_decision_id.as_ref().ok_or_else(|| {
                RuntimeError::Governance("merge requires an exact GOD decision".into())
            })?;
            let approved = if decision_id == &format!("DIRECT_GOD:{}", pr.head_sha) {
                true
            } else {
                self.governance
                    .snapshot()?
                    .decisions
                    .into_iter()
                    .find(|value| &value.id == decision_id)
                    .is_some_and(|decision| {
                        decision.status == super::governance::DecisionStatus::Approved
                    })
            };
            if !approved || checkpoint.merge_approved_head_sha.as_deref() != Some(&pr.head_sha) {
                return Err(RuntimeError::Governance(
                    "merge approval is unresolved or stale".into(),
                ));
            }
        }
        if pr.ci_state != CiState::Pass || pr.review_state != ReviewState::Approved {
            return Err(RuntimeError::Governance(
                "merge gate is no longer satisfied".into(),
            ));
        }
        let repo = checkpoint.repository.clone();
        let mut journal = self.prepare_remote(
            RemoteOperationType::MergePullRequest,
            task_id,
            actor,
            &repo,
            &pr.number.to_string(),
            Some(&pr.head_sha),
            serde_json::json!({"pullRequest":pr.number,"method":checkpoint.policy.merge_method}),
        )?;
        let merged = self.github.merge(
            &repo,
            pr.number,
            &pr.head_sha,
            checkpoint.policy.merge_method,
        )?;
        journal.phase = RemoteOperationPhase::RemoteApplied;
        journal.remote_entity_number = Some(pr.number);
        journal.updated_at = now();
        self.store.upsert_remote_operation(&journal)?;
        checkpoint.pull_request = Some(merged);
        checkpoint.state = DeliveryState::Merged;
        checkpoint.last_synced_at = now();
        self.store.upsert_delivery_checkpoint(&checkpoint)?;
        self.store.set_task_status(task_id, TaskStatus::Completed)?;
        self.finish_remote(&mut journal)?;
        self.events.publish(
            EventType::PullRequestMerged,
            actor,
            None,
            Some(task_id.into()),
            serde_json::json!({"pullRequest":pr.number,"headSha":pr.head_sha}),
        )?;
        Ok(checkpoint)
    }

    pub fn close_linked_issue(&self, task_id: &str, actor: Actor) -> Result<ExternalLink> {
        let checkpoint = self.require_checkpoint(task_id)?;
        if checkpoint.state != DeliveryState::Merged
            || !checkpoint.policy.close_linked_issue_after_merge
        {
            return Err(RuntimeError::Governance(
                "linked issue close is not enabled after merge".into(),
            ));
        }
        let link = self
            .store
            .external_link_by_task(task_id, "ISSUE")?
            .ok_or_else(|| RuntimeError::Provider("task has no linked issue".into()))?;
        self.authorize(
            &actor,
            task_id,
            ProtectedOperation::CloseGithubIssue,
            &format!("{}#{}", checkpoint.repository.slug, link.entity_number),
            serde_json::json!({"issue":link.entity_number}),
        )?;
        let mut journal = self.prepare_remote(
            RemoteOperationType::CloseIssue,
            task_id,
            &actor.id,
            &checkpoint.repository,
            &link.entity_number.to_string(),
            None,
            serde_json::json!({"issue":link.entity_number}),
        )?;
        let issue = self
            .github
            .close_issue(&checkpoint.repository, link.entity_number)?;
        journal.phase = RemoteOperationPhase::RemoteApplied;
        journal.remote_entity_number = Some(issue.number);
        journal.updated_at = now();
        self.store.upsert_remote_operation(&journal)?;
        self.finish_remote(&mut journal)?;
        Ok(link)
    }

    pub fn cleanup_worktree(&self, task_id: &str) -> Result<DeliveryCheckpoint> {
        let mut checkpoint = self.require_checkpoint(task_id)?;
        if checkpoint.state != DeliveryState::Merged {
            return Err(RuntimeError::Governance(
                "worktree cleanup requires a merged delivery".into(),
            ));
        }
        if self.store.has_unfinished_remote_operations(task_id)? {
            return Err(RuntimeError::Governance(
                "remote recovery is still active".into(),
            ));
        }
        let manager = self
            .worktrees
            .as_ref()
            .ok_or_else(|| RuntimeError::Provider("Git worktree manager unavailable".into()))?;
        let binding = checkpoint.binding();
        if !manager.status(&binding)?.is_empty() {
            return Err(RuntimeError::Governance(
                "worktree is dirty; cleanup refused".into(),
            ));
        }
        manager.remove(&binding)?;
        checkpoint.state = DeliveryState::Closed;
        checkpoint.last_synced_at = now();
        self.store.upsert_delivery_checkpoint(&checkpoint)?;
        Ok(checkpoint)
    }

    pub fn reconcile_startup(&self) -> Result<DeliveryRecoverySummary> {
        let mut summary = DeliveryRecoverySummary {
            recovered: 0,
            needs_review: 0,
        };
        for mut journal in self.store.list_unfinished_remote_operations()? {
            let checkpoint = self.store.get_delivery_checkpoint(&journal.task_id)?;
            if journal.operation_type == RemoteOperationType::CreateIssue {
                let recovered = self
                    .repository()
                    .ok()
                    .filter(|repo| repo.slug.eq_ignore_ascii_case(&journal.repository))
                    .and_then(|repo| {
                        self.github
                            .find_issue_by_task_marker(&repo, &journal.task_id)
                            .ok()
                            .flatten()
                            .map(|issue| (repo, issue))
                    });
                if let Some((repo, issue)) = recovered {
                    journal.remote_entity_number = Some(issue.number);
                    let link = ExternalLink {
                        id: format!("LINK-{}", uuid::Uuid::new_v4()),
                        task_id: journal.task_id.clone(),
                        provider: "GITHUB".into(),
                        repository: repo.slug,
                        entity_type: "ISSUE".into(),
                        entity_number: issue.number,
                        url: issue.url,
                        created_at: now(),
                        updated_at: now(),
                    };
                    self.store.upsert_external_link(&link)?;
                    self.finish_remote(&mut journal)?;
                    summary.recovered += 1;
                } else {
                    mark_remote_review(&self.store, &mut journal)?;
                    summary.needs_review += 1;
                }
                continue;
            }
            if journal.operation_type == RemoteOperationType::CloseIssue {
                let issue_number = journal
                    .remote_entity_number
                    .or_else(|| journal.target.parse().ok());
                let recovered = checkpoint
                    .as_ref()
                    .zip(issue_number)
                    .and_then(|(c, number)| self.github.issue(&c.repository, number).ok())
                    .is_some_and(|issue| issue.state == IssueState::Closed);
                if recovered {
                    self.finish_remote(&mut journal)?;
                    summary.recovered += 1;
                } else {
                    mark_remote_review(&self.store, &mut journal)?;
                    summary.needs_review += 1;
                }
                continue;
            }
            let result = match journal.operation_type {
                RemoteOperationType::CreatePullRequest => checkpoint
                    .as_ref()
                    .and_then(|c| {
                        self.github
                            .find_open_pull_request(&c.repository, &c.branch)
                            .ok()
                            .flatten()
                    })
                    .inspect(|pr| {
                        journal.remote_entity_number = Some(pr.number);
                    }),
                RemoteOperationType::MergePullRequest => checkpoint
                    .as_ref()
                    .and_then(|c| c.pull_request.as_ref().map(|pr| (c, pr.number)))
                    .and_then(|(c, n)| self.github.pull_request(&c.repository, n).ok())
                    .filter(|pr| pr.state == PullRequestState::Merged),
                RemoteOperationType::PushBranch => checkpoint
                    .as_ref()
                    .and_then(|c| {
                        remote_branch_head(&self.root, &c.repository.remote_name, &c.branch).ok()
                    })
                    .filter(|head| Some(head) == journal.expected_head_sha.as_ref())
                    .map(|_| {
                        checkpoint
                            .as_ref()
                            .and_then(|c| c.pull_request.clone())
                            .unwrap_or_else(empty_pr)
                    }),
                _ => None,
            };
            if let Some(remote) = result {
                if let Some(mut c) = checkpoint {
                    if journal.operation_type == RemoteOperationType::CreatePullRequest {
                        c.pull_request = Some(remote);
                        c.state = DeliveryState::PullRequestOpen;
                    } else if journal.operation_type == RemoteOperationType::MergePullRequest {
                        c.pull_request = Some(remote);
                        c.state = DeliveryState::Merged;
                        self.store
                            .set_task_status(&journal.task_id, TaskStatus::Completed)?;
                    } else {
                        c.state = DeliveryState::Pushed;
                    }
                    c.last_synced_at = now();
                    self.store.upsert_delivery_checkpoint(&c)?;
                }
                self.finish_remote(&mut journal)?;
                summary.recovered += 1;
            } else {
                mark_remote_review(&self.store, &mut journal)?;
                summary.needs_review += 1;
            }
        }
        Ok(summary)
    }

    fn require_task(&self, id: &str) -> Result<Task> {
        self.store
            .get_task(id)?
            .ok_or_else(|| RuntimeError::TaskNotFound(id.into()))
    }
    fn require_checkpoint(&self, id: &str) -> Result<DeliveryCheckpoint> {
        self.store
            .get_delivery_checkpoint(id)?
            .ok_or_else(|| RuntimeError::Provider(format!("delivery checkpoint not found: {id}")))
    }
    fn ensure_not_default_branch(&self, c: &DeliveryCheckpoint) -> Result<()> {
        if c.branch == c.repository.default_branch {
            Err(RuntimeError::Governance(
                "coding agents cannot deliver from the default branch".into(),
            ))
        } else {
            Ok(())
        }
    }
    fn authorize(
        &self,
        actor: &Actor,
        task_id: &str,
        operation: ProtectedOperation,
        target: &str,
        detail: Value,
    ) -> Result<()> {
        let mutation = OrganizationMutation::RequestProtectedAction {
            operation,
            target: target.into(),
            detail,
        };
        let snapshot = self.governance.snapshot()?;
        let serialized = serde_json::to_value(&mutation)?;
        if snapshot.decisions.iter().any(|decision| {
            decision.status == super::governance::DecisionStatus::Approved
                && decision.request.actor.id == actor.id
                && decision.request.task_id.as_deref() == Some(task_id)
                && serde_json::to_value(&decision.request.mutation)
                    .ok()
                    .as_ref()
                    == Some(&serialized)
        }) {
            return Ok(());
        }
        let result = self.governance.mutate(MutationRequest {
            actor: actor.clone(),
            expected_revision: snapshot.revision,
            task_id: Some(task_id.into()),
            reason: Some("GitHub delivery operation".into()),
            mutation,
        })?;
        if result.disposition == MutationDisposition::Applied {
            Ok(())
        } else {
            Err(RuntimeError::Governance(format!(
                "GitHub action requires GOD decision {}",
                result.decision_id.unwrap_or_default()
            )))
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn prepare_remote(
        &self,
        operation_type: RemoteOperationType,
        task_id: &str,
        actor: &str,
        repo: &RepositoryIdentity,
        target: &str,
        head: Option<&str>,
        detail: Value,
    ) -> Result<RemoteOperationJournal> {
        let idempotency_key = format!(
            "{:x}",
            Sha256::digest(
                format!(
                    "{:?}:{}/{target}:{}",
                    operation_type,
                    repo.slug,
                    head.unwrap_or("")
                )
                .as_bytes()
            )
        );
        if let Some(existing) = self
            .store
            .remote_operation_by_idempotency_key(&idempotency_key)?
        {
            return Ok(existing);
        }
        let journal = RemoteOperationJournal {
            id: format!("REMOTE-{}", uuid::Uuid::new_v4()),
            operation_type,
            phase: RemoteOperationPhase::Prepared,
            task_id: task_id.into(),
            actor: actor.into(),
            repository: repo.slug.clone(),
            target: target.into(),
            idempotency_key,
            expected_head_sha: head.map(str::to_owned),
            remote_entity_number: None,
            detail,
            failure: None,
            created_at: now(),
            updated_at: now(),
        };
        self.store.upsert_remote_operation(&journal)?;
        Ok(journal)
    }
    fn finish_remote(&self, journal: &mut RemoteOperationJournal) -> Result<()> {
        if journal.phase == RemoteOperationPhase::Committed {
            return Ok(());
        }
        journal.phase = RemoteOperationPhase::LocalApplied;
        journal.updated_at = now();
        self.store.upsert_remote_operation(journal)?;
        journal.phase = RemoteOperationPhase::Committed;
        journal.updated_at = now();
        self.store.upsert_remote_operation(journal)?;
        self.governance.audit_delivery(
            &journal.actor,
            &format!("GITHUB_{:?}", journal.operation_type).to_ascii_uppercase(),
            Some(&journal.target),
            Some(&journal.task_id),
            MutationDisposition::Applied,
            Some("remote side effect reconciled and committed".into()),
        )
    }
}

fn mark_remote_review(store: &RuntimeStore, journal: &mut RemoteOperationJournal) -> Result<()> {
    journal.phase = RemoteOperationPhase::NeedsReview;
    journal.failure = Some("remote side effect could not be proven after restart".into());
    journal.updated_at = now();
    store.upsert_remote_operation(journal)
}

impl DeliveryCheckpoint {
    fn binding(&self) -> WorktreeBinding {
        WorktreeBinding {
            path: PathBuf::from(&self.worktree_path),
            branch: self.branch.clone(),
            base_commit: self.base_sha.clone(),
            starting_head: self
                .head_sha
                .clone()
                .unwrap_or_else(|| self.base_sha.clone()),
            reused: true,
        }
    }
}
fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
fn issue_task_id(repo: &str, number: u64) -> String {
    let digest = format!("{:x}", Sha256::digest(repo.as_bytes()));
    format!("GH-{}-{number}", &digest[..8])
}
fn git_text(cwd: &std::path::Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(cwd).args(args).output()?;
    if !output.status.success() {
        return Err(RuntimeError::Provider(format!(
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into())
}
fn remote_branch_head(root: &std::path::Path, remote: &str, branch: &str) -> Result<String> {
    let value = git_text(
        root,
        &[
            "ls-remote",
            "--heads",
            remote,
            &format!("refs/heads/{branch}"),
        ],
    )?;
    value
        .split_whitespace()
        .next()
        .map(str::to_owned)
        .ok_or_else(|| RuntimeError::Provider("remote branch not found".into()))
}
fn github_slug(remote_url: &str) -> Option<String> {
    let value = remote_url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git");
    if let Some(rest) = value.strip_prefix("git@github.com:") {
        return Some(rest.into());
    }
    for prefix in [
        "https://github.com/",
        "http://github.com/",
        "ssh://git@github.com/",
    ] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return Some(rest.into());
        }
    }
    None
}
fn sanitized_remote_url(remote_url: &str) -> String {
    let value = remote_url.trim();
    let Some((scheme, remainder)) = value.split_once("://") else {
        return value.to_owned();
    };
    if matches!(scheme, "http" | "https") {
        if let Some((_, host_and_path)) = remainder.rsplit_once('@') {
            return format!("{scheme}://{host_and_path}");
        }
    }
    value.to_owned()
}
fn classify_push(output: &std::process::Output) -> String {
    let text = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    let class = if text.contains("authentication") || text.contains("could not read username") {
        "AUTH"
    } else if text.contains("non-fast-forward") || text.contains("rejected") {
        "REJECTED"
    } else if text.contains("permission") {
        "PERMISSION_DENIED"
    } else if text.contains("could not resolve") || text.contains("network") {
        "NETWORK"
    } else if text.contains("repository not found") {
        "REMOTE_MISSING"
    } else {
        "UNKNOWN"
    };
    format!("push failed [{class}]")
}
fn pr_body(task: &Task, agent: &str, tests: &[String], checkpoint: &DeliveryCheckpoint) -> String {
    format!("## Summary\n\n{}\n\n## Validation\n\n{}\n\n## Batai Task\n\n`{}`\n\n## Agent\n\n`{agent}`\n\n## Risk / Notes\n\nBase `{}` at `{}`. No provider transcript or private reasoning is included.\n",task.objective,if tests.is_empty(){"Not reported".into()}else{tests.iter().map(|test|format!("- {test}")).collect::<Vec<_>>().join("\n")},task.id,checkpoint.base,checkpoint.base_sha)
}
fn delivery_state_for_pr(pr: &PullRequest) -> DeliveryState {
    if pr.state == PullRequestState::Merged {
        DeliveryState::Merged
    } else if pr.state == PullRequestState::Closed
        || pr.ci_state == CiState::Fail
        || pr.review_state == ReviewState::ChangesRequested
    {
        DeliveryState::Blocked
    } else if pr.ci_state == CiState::Pass && pr.review_state == ReviewState::Approved {
        DeliveryState::MergeReady
    } else if pr.ci_state == CiState::Pending {
        DeliveryState::CiPending
    } else {
        DeliveryState::Review
    }
}
fn empty_pr() -> PullRequest {
    PullRequest {
        number: 0,
        title: String::new(),
        base: String::new(),
        base_sha: None,
        head: String::new(),
        head_sha: String::new(),
        state: PullRequestState::Open,
        draft: false,
        mergeability: None,
        review_state: ReviewState::Unknown,
        ci_state: CiState::Unknown,
        checks: Vec::new(),
        reviews: Vec::new(),
        url: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{providers::github_cli::GhRunner, runtime::agents::AgentRegistry};
    use std::{
        collections::VecDeque,
        path::Path,
        process::{ExitStatus, Output},
        sync::{Arc, Mutex},
    };

    #[derive(Default)]
    struct FakeGh {
        outputs: Mutex<VecDeque<(i32, String, String)>>,
        calls: Mutex<Vec<Vec<String>>>,
    }
    impl FakeGh {
        fn new(outputs: Vec<(i32, &str, &str)>) -> Arc<Self> {
            Arc::new(Self {
                outputs: Mutex::new(
                    outputs
                        .into_iter()
                        .map(|(c, o, e)| (c, o.into(), e.into()))
                        .collect(),
                ),
                calls: Mutex::new(Vec::new()),
            })
        }
    }
    impl GhRunner for FakeGh {
        fn run(&self, _: &Path, args: &[String]) -> std::io::Result<Output> {
            self.calls.lock().unwrap().push(args.to_vec());
            let (code, out, err) = self
                .outputs
                .lock()
                .unwrap()
                .pop_front()
                .expect("fake gh output");
            Ok(Output {
                status: exit_status(code),
                stdout: out.into_bytes(),
                stderr: err.into_bytes(),
            })
        }
    }
    #[cfg(unix)]
    fn exit_status(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }
    #[cfg(windows)]
    fn exit_status(code: i32) -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(code as u32)
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
        store: RuntimeStore,
        events: EventEngine,
        governance: GovernanceService,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("project");
            let remote = temp.path().join("remote.git");
            std::fs::create_dir_all(&root).unwrap();
            git_ok(temp.path(), &["init", "--bare", remote.to_str().unwrap()]);
            git_ok(&root, &["init", "-b", "trunk"]);
            git_ok(&root, &["config", "user.email", "batai@example.test"]);
            git_ok(&root, &["config", "user.name", "Batai Test"]);
            std::fs::write(root.join("README.md"), "base").unwrap();
            git_ok(&root, &["add", "README.md"]);
            git_ok(&root, &["commit", "-m", "initial"]);
            git_ok(
                &root,
                &["remote", "add", "origin", remote.to_str().unwrap()],
            );
            git_ok(&root, &["push", "-u", "origin", "trunk"]);
            let store = RuntimeStore::open_memory().unwrap();
            let events = EventEngine::new(store.clone());
            let agents = AgentRegistry::new(store.clone(), events.clone());
            let governance =
                GovernanceService::new(root.clone(), store.clone(), agents, events.clone());
            Self {
                _temp: temp,
                root,
                store,
                events,
                governance,
            }
        }
        fn service(&self, outputs: Vec<(i32, &str, &str)>) -> DeliveryService {
            DeliveryService::new(
                self.root.clone(),
                self.store.clone(),
                self.events.clone(),
                self.governance.clone(),
            )
            .with_github(GitHubService::with_runner(&self.root, FakeGh::new(outputs)))
        }
        fn task(&self, id: &str) {
            let task:Task=serde_json::from_value(serde_json::json!({"id":id,"created_by":"god","objective":"Ship safely","assigned_to":["nova"]})).unwrap();
            let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&task).unwrap()));
            self.store.ingest_task(&task, None, &hash).unwrap();
        }
    }
    fn git_ok(cwd: &Path, args: &[&str]) {
        let out = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    fn repo_json() -> &'static str {
        r#"{"nameWithOwner":"acme/app","url":"https://github.com/acme/app","defaultBranchRef":{"name":"trunk"}}"#
    }
    fn issue_json() -> &'static str {
        r#"{"number":7,"title":"Fix login","body":"Acceptance text","state":"OPEN","url":"https://github.com/acme/app/issues/7","labels":[{"name":"bug"}],"assignees":[]}"#
    }
    fn pr_json(state: &str, head: &str, review: &str) -> String {
        format!(
            r#"{{"number":4,"title":"Ship safely","baseRefName":"trunk","headRefName":"batai/nova/t1","headRefOid":"{head}","state":"{state}","isDraft":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","reviewDecision":"{review}","url":"https://github.com/acme/app/pull/4","reviews":[]}}"#
        )
    }

    #[test]
    fn issue_ids_are_repo_bound_and_stable() {
        assert_eq!(issue_task_id("a/b", 1), issue_task_id("a/b", 1));
        assert_ne!(issue_task_id("a/b", 1), issue_task_id("c/d", 1));
    }

    #[test]
    fn github_remote_identity_supports_https_and_ssh_without_guessing_other_hosts() {
        assert_eq!(
            github_slug("https://github.com/acme/app.git").as_deref(),
            Some("acme/app")
        );
        assert_eq!(
            github_slug("git@github.com:acme/app.git").as_deref(),
            Some("acme/app")
        );
        assert_eq!(github_slug("C:/local/remote.git"), None);
        assert_eq!(
            sanitized_remote_url("https://oauth2:secret@github.com/acme/app.git"),
            "https://github.com/acme/app.git"
        );
    }
    #[test]
    fn pr_state_requires_both_review_and_ci() {
        let mut pr = empty_pr();
        pr.review_state = ReviewState::Approved;
        assert_eq!(delivery_state_for_pr(&pr), DeliveryState::Review);
        pr.ci_state = CiState::Pass;
        assert_eq!(delivery_state_for_pr(&pr), DeliveryState::MergeReady);
        pr.ci_state = CiState::Fail;
        assert_eq!(delivery_state_for_pr(&pr), DeliveryState::Blocked);
    }
    #[test]
    fn structured_body_excludes_provider_transcript() {
        let task: Task = serde_json::from_value(
            serde_json::json!({"id":"T","created_by":"god","objective":"Ship","assigned_to":["a"]}),
        )
        .unwrap();
        let repo = RepositoryIdentity {
            owner: "o".into(),
            name: "r".into(),
            slug: "o/r".into(),
            remote_url: "u".into(),
            default_branch: "trunk".into(),
            remote_name: "origin".into(),
            current_upstream: None,
        };
        let checkpoint = DeliveryCheckpoint {
            task_id: "T".into(),
            repository: repo,
            state: DeliveryState::Committed,
            policy: Default::default(),
            branch: "batai/a/t".into(),
            base: "trunk".into(),
            base_sha: "abc".into(),
            head_sha: Some("def".into()),
            commit_ids: vec![],
            worktree_path: ".".into(),
            issue_number: None,
            pull_request: None,
            merge_decision_id: None,
            merge_approved_head_sha: None,
            delivery_attempts: 0,
            blocked_reason: None,
            last_synced_at: now(),
        };
        let body = pr_body(&task, "a", &["cargo test".into()], &checkpoint);
        assert!(body.contains("Batai Task"));
        assert!(!body.contains("transcript:"));
    }

    #[test]
    fn issue_import_is_repo_bound_and_idempotent() {
        let f = Fixture::new();
        let service = f.service(vec![
            (0, repo_json(), ""),
            (0, issue_json(), ""),
            (0, repo_json(), ""),
        ]);
        let first = service.import_issue(7, "god").unwrap();
        let second = service.import_issue(7, "god").unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(f.store.list_tasks().unwrap().len(), 1);
        assert_eq!(f.store.list_external_links(None).unwrap().len(), 1);
    }

    #[test]
    fn worktree_commit_and_push_use_isolated_non_default_branch() {
        let f = Fixture::new();
        f.task("T1");
        let service = f.service(vec![(0, repo_json(), "")]);
        let prepared = service
            .prepare_worktree(
                "T1",
                "nova",
                None,
                DeliveryPolicy {
                    required: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(prepared.base, "trunk");
        assert_ne!(prepared.branch, "trunk");
        std::fs::write(
            PathBuf::from(&prepared.worktree_path).join("feature.txt"),
            "done",
        )
        .unwrap();
        let committed = service.commit("T1", "nova", Some("ship feature")).unwrap();
        let pushed = service.push("T1", "god").unwrap();
        assert_eq!(pushed.state, DeliveryState::Pushed);
        assert_eq!(
            remote_branch_head(&f.root, "origin", &pushed.branch).unwrap(),
            committed.head_sha.unwrap()
        );
    }

    #[test]
    fn pr_creation_is_idempotent_and_persists_remote_journal() {
        let f = Fixture::new();
        f.task("T1");
        let list = format!("[{}]", pr_json("OPEN", "abc", "REVIEW_REQUIRED"));
        let service = f.service(vec![(0, repo_json(), ""), (0, &list, "")]);
        let mut checkpoint = service
            .prepare_worktree(
                "T1",
                "nova",
                None,
                DeliveryPolicy {
                    required: true,
                    ..Default::default()
                },
            )
            .unwrap();
        checkpoint.state = DeliveryState::Pushed;
        checkpoint.head_sha = Some("abc".into());
        f.store.upsert_delivery_checkpoint(&checkpoint).unwrap();
        let created = service
            .create_pull_request(
                "T1",
                Actor {
                    id: "god".into(),
                    scope: super::super::governance::AuthorityScope::Project,
                },
                &["cargo test: pass".into()],
            )
            .unwrap();
        assert_eq!(created.pull_request.unwrap().number, 4);
        assert_eq!(
            f.store.list_remote_operations(10).unwrap()[0].phase,
            RemoteOperationPhase::Committed
        );
    }

    #[test]
    fn review_changes_are_bounded_instead_of_immediate_task_failure() {
        let f = Fixture::new();
        f.task("T1");
        let service = f.service(vec![(0, repo_json(), "")]);
        let mut checkpoint = service
            .prepare_worktree(
                "T1",
                "nova",
                None,
                DeliveryPolicy {
                    required: true,
                    max_delivery_attempts: 2,
                    ..Default::default()
                },
            )
            .unwrap();
        f.store.upsert_delivery_checkpoint(&checkpoint).unwrap();
        for index in 0..2 {
            checkpoint = service
                .record_internal_review(ReviewOutcome {
                    id: format!("R{index}"),
                    task_id: "T1".into(),
                    reviewer_id: "qa".into(),
                    subject_agent_id: "nova".into(),
                    outcome: ReviewOutcomeKind::ChangesRequested,
                    note: Some("fix".into()),
                    created_at: now(),
                })
                .unwrap();
        }
        assert_eq!(checkpoint.state, DeliveryState::Blocked);
        assert_eq!(
            f.store.get_task("T1").unwrap().unwrap().status,
            TaskStatus::Pending
        );
    }

    #[test]
    fn new_head_invalidates_old_ci_and_merge_approval() {
        let f = Fixture::new();
        f.task("T1");
        let checks = r#"[{"name":"ci","state":"IN_PROGRESS","bucket":"pending","link":null}]"#;
        let service = f.service(vec![
            (0, repo_json(), ""),
            (0, &pr_json("OPEN", "new", "APPROVED"), ""),
            (0, checks, ""),
        ]);
        let mut checkpoint = service
            .prepare_worktree(
                "T1",
                "nova",
                None,
                DeliveryPolicy {
                    required: true,
                    ..Default::default()
                },
            )
            .unwrap();
        checkpoint.head_sha = Some("old".into());
        checkpoint.merge_decision_id = Some("DEC-old".into());
        checkpoint.merge_approved_head_sha = Some("old".into());
        checkpoint.pull_request = Some(PullRequest {
            head_sha: "old".into(),
            number: 4,
            title: "x".into(),
            base: "trunk".into(),
            base_sha: None,
            head: checkpoint.branch.clone(),
            state: PullRequestState::Open,
            draft: false,
            mergeability: None,
            review_state: ReviewState::Approved,
            ci_state: CiState::Pass,
            checks: vec![],
            reviews: vec![],
            url: "u".into(),
        });
        f.store.upsert_delivery_checkpoint(&checkpoint).unwrap();
        let synced = service.sync("T1").unwrap();
        assert_eq!(synced.pull_request.unwrap().ci_state, CiState::Pending);
        assert_eq!(synced.merge_decision_id, None);
    }

    #[test]
    fn startup_forward_recovers_pr_and_merge_without_duplicate_mutation() {
        let f = Fixture::new();
        f.task("T1");
        let service = f.service(vec![(0, repo_json(), "")]);
        let mut checkpoint = service
            .prepare_worktree(
                "T1",
                "nova",
                None,
                DeliveryPolicy {
                    required: true,
                    ..Default::default()
                },
            )
            .unwrap();
        checkpoint.head_sha = Some("abc".into());
        checkpoint.state = DeliveryState::Pushed;
        f.store.upsert_delivery_checkpoint(&checkpoint).unwrap();
        let prepared = service
            .prepare_remote(
                RemoteOperationType::CreatePullRequest,
                "T1",
                "god",
                &checkpoint.repository,
                &checkpoint.branch,
                Some("abc"),
                serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(prepared.phase, RemoteOperationPhase::Prepared);
        let open_list = format!("[{}]", pr_json("OPEN", "abc", "REVIEW_REQUIRED"));
        let recovery = f.service(vec![(0, &open_list, "")]);
        let summary = recovery.reconcile_startup().unwrap();
        assert_eq!(summary.recovered, 1);
        assert_eq!(
            f.store
                .get_delivery_checkpoint("T1")
                .unwrap()
                .unwrap()
                .pull_request
                .unwrap()
                .number,
            4
        );
        let merge_journal = recovery
            .prepare_remote(
                RemoteOperationType::MergePullRequest,
                "T1",
                "god",
                &checkpoint.repository,
                "4",
                Some("abc"),
                serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(merge_journal.phase, RemoteOperationPhase::Prepared);
        let merged = f.service(vec![(0, &pr_json("MERGED", "abc", "APPROVED"), "")]);
        assert_eq!(merged.reconcile_startup().unwrap().recovered, 1);
        assert_eq!(
            f.store
                .get_delivery_checkpoint("T1")
                .unwrap()
                .unwrap()
                .state,
            DeliveryState::Merged
        );
    }

    #[test]
    fn startup_recovers_created_issue_from_exact_task_marker() {
        let f = Fixture::new();
        f.task("T1");
        let repo = RepositoryIdentity {
            owner: "acme".into(),
            name: "app".into(),
            slug: "acme/app".into(),
            remote_url: f
                .root
                .parent()
                .unwrap()
                .join("remote.git")
                .to_string_lossy()
                .into_owned(),
            default_branch: "trunk".into(),
            remote_name: "origin".into(),
            current_upstream: Some("origin/trunk".into()),
        };
        f.service(Vec::new())
            .prepare_remote(
                RemoteOperationType::CreateIssue,
                "T1",
                "god",
                &repo,
                "T1",
                None,
                serde_json::json!({}),
            )
            .unwrap();
        let issues = r#"[{"number":7,"title":"Ship safely","body":"Created from Batai\n\nBatai-Task: T1","state":"OPEN","url":"https://github.com/acme/app/issues/7","labels":[],"assignees":[]}]"#;
        let recovery = f.service(vec![(0, repo_json(), ""), (0, issues, "")]);
        let summary = recovery.reconcile_startup().unwrap();
        assert_eq!(summary.recovered, 1);
        assert_eq!(
            f.store
                .external_link_by_task("T1", "ISSUE")
                .unwrap()
                .unwrap()
                .entity_number,
            7
        );
    }

    #[test]
    fn ambiguous_remote_recovery_requires_review() {
        let f = Fixture::new();
        f.task("T1");
        let service = f.service(vec![(0, repo_json(), "")]);
        let checkpoint = service
            .prepare_worktree(
                "T1",
                "nova",
                None,
                DeliveryPolicy {
                    required: true,
                    ..Default::default()
                },
            )
            .unwrap();
        service
            .prepare_remote(
                RemoteOperationType::CreatePullRequest,
                "T1",
                "god",
                &checkpoint.repository,
                &checkpoint.branch,
                checkpoint.head_sha.as_deref(),
                serde_json::json!({}),
            )
            .unwrap();
        let recovery = f.service(vec![(0, "[]", "")]);
        let summary = recovery.reconcile_startup().unwrap();
        assert_eq!(summary.needs_review, 1);
        assert_eq!(
            f.store.list_unfinished_remote_operations().unwrap()[0].phase,
            RemoteOperationPhase::NeedsReview
        );
    }

    #[test]
    fn cleanup_refuses_dirty_or_unmerged_worktree() {
        let f = Fixture::new();
        f.task("T1");
        let service = f.service(vec![(0, repo_json(), "")]);
        let mut checkpoint = service
            .prepare_worktree(
                "T1",
                "nova",
                None,
                DeliveryPolicy {
                    required: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(service.cleanup_worktree("T1").is_err());
        checkpoint.state = DeliveryState::Merged;
        f.store.upsert_delivery_checkpoint(&checkpoint).unwrap();
        std::fs::write(
            PathBuf::from(&checkpoint.worktree_path).join("dirty.txt"),
            "x",
        )
        .unwrap();
        assert!(service.cleanup_worktree("T1").is_err());
    }
}
