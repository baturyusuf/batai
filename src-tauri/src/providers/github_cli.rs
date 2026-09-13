//! Official GitHub CLI transport. Remote collaboration is kept out of the
//! provider-neutral delivery state machine and every command is repository-bound.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};

use serde::Deserialize;

use crate::runtime::{
    delivery::{
        CheckRunSummary, CheckState, CiState, GitHubAuthState, GitHubIssue, GitHubReview,
        IssueState, MergeMethod, PullRequest, PullRequestState, RepositoryIdentity, ReviewState,
    },
    errors::{Result, RuntimeError},
};

pub trait GhRunner: Send + Sync {
    fn run(&self, cwd: &Path, args: &[String]) -> std::io::Result<Output>;
}

#[derive(Debug, Clone)]
pub struct ProcessGhRunner {
    executable: PathBuf,
}

impl Default for ProcessGhRunner {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("gh"),
        }
    }
}

impl GhRunner for ProcessGhRunner {
    fn run(&self, cwd: &Path, args: &[String]) -> std::io::Result<Output> {
        Command::new(&self.executable)
            .current_dir(cwd)
            .args(args)
            .env("GH_PROMPT_DISABLED", "1")
            .output()
    }
}

#[derive(Clone)]
pub struct GitHubService {
    cwd: PathBuf,
    runner: Arc<dyn GhRunner>,
}

impl GitHubService {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            runner: Arc::new(ProcessGhRunner::default()),
        }
    }

    #[cfg(test)]
    pub fn with_runner(cwd: impl Into<PathBuf>, runner: Arc<dyn GhRunner>) -> Self {
        Self {
            cwd: cwd.into(),
            runner,
        }
    }

    pub fn auth_status(&self) -> GitHubAuthState {
        match self.invoke(&["auth", "status"]) {
            Ok(_) => GitHubAuthState::Available,
            Err(GitHubCliError::NotInstalled) => GitHubAuthState::NotInstalled,
            Err(GitHubCliError::AuthRequired) => GitHubAuthState::AuthRequired,
            Err(_) => GitHubAuthState::Error,
        }
    }

    pub fn repository(&self) -> Result<RepositoryIdentity> {
        let value: RepoJson = self.json(
            &[
                "repo",
                "view",
                "--json",
                "nameWithOwner,url,defaultBranchRef",
            ]
            .map(str::to_owned),
        )?;
        let (owner, name) = value.name_with_owner.split_once('/').ok_or_else(|| {
            RuntimeError::Provider("GitHub returned an invalid repository identity".into())
        })?;
        Ok(RepositoryIdentity {
            owner: owner.into(),
            name: name.into(),
            slug: value.name_with_owner,
            remote_url: value.url,
            default_branch: value.default_branch_ref.name,
            remote_name: "origin".into(),
            current_upstream: None,
        })
    }

    pub fn issue(&self, repo: &RepositoryIdentity, number: u64) -> Result<GitHubIssue> {
        let value: IssueJson = self.json(&repo_args(
            &["issue", "view", &number.to_string(), "--json", ISSUE_FIELDS],
            repo,
        ))?;
        Ok(value.into())
    }

    pub fn create_issue(
        &self,
        repo: &RepositoryIdentity,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<GitHubIssue> {
        let mut args = repo_args(&["issue", "create", "--title", title, "--body", body], repo);
        for label in labels {
            args.push("--label".into());
            args.push(label.clone());
        }
        let url = self.text(&args)?;
        let value: IssueJson = self.json(&repo_args(
            &["issue", "view", url.trim(), "--json", ISSUE_FIELDS],
            repo,
        ))?;
        Ok(value.into())
    }

    pub fn find_issue_by_task_marker(
        &self,
        repo: &RepositoryIdentity,
        task_id: &str,
    ) -> Result<Option<GitHubIssue>> {
        let marker = format!("Batai-Task: {task_id}");
        let values: Vec<IssueJson> = self.json(&repo_args(
            &[
                "issue",
                "list",
                "--state",
                "all",
                "--search",
                &format!("\"{marker}\" in:body"),
                "--limit",
                "100",
                "--json",
                ISSUE_FIELDS,
            ],
            repo,
        ))?;
        Ok(values
            .into_iter()
            .find(|issue| issue.body.lines().any(|line| line.trim() == marker))
            .map(Into::into))
    }

    pub fn close_issue(&self, repo: &RepositoryIdentity, number: u64) -> Result<GitHubIssue> {
        self.text(&repo_args(&["issue", "close", &number.to_string()], repo))?;
        self.issue(repo, number)
    }

    pub fn find_open_pull_request(
        &self,
        repo: &RepositoryIdentity,
        head: &str,
    ) -> Result<Option<PullRequest>> {
        let values: Vec<PullRequestJson> = self.json(&repo_args(
            &[
                "pr", "list", "--head", head, "--state", "open", "--json", PR_FIELDS,
            ],
            repo,
        ))?;
        Ok(values
            .into_iter()
            .find(|value| value.head_ref_name == head)
            .map(Into::into))
    }

    pub fn create_pull_request(
        &self,
        repo: &RepositoryIdentity,
        title: &str,
        body: &str,
        base: &str,
        head: &str,
        draft: bool,
    ) -> Result<PullRequest> {
        if let Some(existing) = self.find_open_pull_request(repo, head)? {
            return Ok(existing);
        }
        let mut args = repo_args(
            &[
                "pr", "create", "--title", title, "--body", body, "--base", base, "--head", head,
            ],
            repo,
        );
        if draft {
            args.push("--draft".into());
        }
        self.text(&args)?;
        self.find_open_pull_request(repo, head)?.ok_or_else(|| {
            RuntimeError::Provider("GitHub created a PR but it could not be reconciled".into())
        })
    }

    pub fn pull_request(&self, repo: &RepositoryIdentity, number: u64) -> Result<PullRequest> {
        let value: PullRequestJson = self.json(&repo_args(
            &["pr", "view", &number.to_string(), "--json", PR_FIELDS],
            repo,
        ))?;
        Ok(value.into())
    }

    pub fn request_review(
        &self,
        repo: &RepositoryIdentity,
        number: u64,
        reviewer: &str,
    ) -> Result<PullRequest> {
        self.text(&repo_args(
            &[
                "pr",
                "edit",
                &number.to_string(),
                "--add-reviewer",
                reviewer,
            ],
            repo,
        ))?;
        self.pull_request(repo, number)
    }

    #[allow(dead_code)]
    pub fn close_pull_request(
        &self,
        repo: &RepositoryIdentity,
        number: u64,
    ) -> Result<PullRequest> {
        self.text(&repo_args(&["pr", "close", &number.to_string()], repo))?;
        self.pull_request(repo, number)
    }

    pub fn checks(
        &self,
        repo: &RepositoryIdentity,
        number: u64,
    ) -> Result<(CiState, Vec<CheckRunSummary>)> {
        let values: Vec<CheckJson> = self.json_allow_pending(&repo_args(
            &[
                "pr",
                "checks",
                &number.to_string(),
                "--json",
                "name,state,bucket,link",
            ],
            repo,
        ))?;
        let checks = values.into_iter().map(Into::into).collect::<Vec<_>>();
        Ok((aggregate_checks(&checks), checks))
    }

    pub fn merge(
        &self,
        repo: &RepositoryIdentity,
        number: u64,
        expected_head: &str,
        method: MergeMethod,
    ) -> Result<PullRequest> {
        let current = self.pull_request(repo, number)?;
        if current.head_sha != expected_head {
            return Err(RuntimeError::Governance(format!(
                "stale merge approval: expected {expected_head}, current {}",
                current.head_sha
            )));
        }
        if current.state == PullRequestState::Merged {
            return Ok(current);
        }
        let flag = match method {
            MergeMethod::Squash => "--squash",
            MergeMethod::MergeCommit => "--merge",
            MergeMethod::Rebase => "--rebase",
        };
        self.text(&repo_args(
            &[
                "pr",
                "merge",
                &number.to_string(),
                flag,
                "--match-head-commit",
                expected_head,
            ],
            repo,
        ))?;
        self.pull_request(repo, number)
    }

    fn json<T: serde::de::DeserializeOwned>(&self, args: &[String]) -> Result<T> {
        let output = self.invoke_strings(args).map_err(to_runtime)?;
        serde_json::from_slice(&output.stdout)
            .map_err(|error| RuntimeError::Provider(format!("invalid gh JSON: {error}")))
    }

    fn json_allow_pending<T: serde::de::DeserializeOwned>(&self, args: &[String]) -> Result<T> {
        let output = self
            .runner
            .run(&self.cwd, args)
            .map_err(classify_io)
            .map_err(to_runtime)?;
        if !output.status.success() && output.status.code() != Some(8) {
            return Err(to_runtime(classify_output(&output)));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| RuntimeError::Provider(format!("invalid gh JSON: {error}")))
    }

    fn text(&self, args: &[String]) -> Result<String> {
        let output = self.invoke_strings(args).map_err(to_runtime)?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    }

    fn invoke(&self, args: &[&str]) -> std::result::Result<Output, GitHubCliError> {
        let args = args
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        self.invoke_strings(&args)
    }

    fn invoke_strings(&self, args: &[String]) -> std::result::Result<Output, GitHubCliError> {
        let output = self.runner.run(&self.cwd, args).map_err(classify_io)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(classify_output(&output))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitHubCliError {
    NotInstalled,
    AuthRequired,
    PermissionDenied,
    Network,
    Conflict,
    NotFound,
    Other,
}

fn classify_io(error: std::io::Error) -> GitHubCliError {
    if error.kind() == std::io::ErrorKind::NotFound {
        GitHubCliError::NotInstalled
    } else {
        GitHubCliError::Other
    }
}

fn classify_output(output: &Output) -> GitHubCliError {
    let detail = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    )
    .to_ascii_lowercase();
    if detail.contains("auth") || detail.contains("login") || detail.contains("token") {
        GitHubCliError::AuthRequired
    } else if detail.contains("permission") || detail.contains("forbidden") {
        GitHubCliError::PermissionDenied
    } else if detail.contains("network")
        || detail.contains("connection")
        || detail.contains("timeout")
    {
        GitHubCliError::Network
    } else if detail.contains("conflict") || detail.contains("not mergeable") {
        GitHubCliError::Conflict
    } else if detail.contains("not found") {
        GitHubCliError::NotFound
    } else {
        GitHubCliError::Other
    }
}

fn to_runtime(error: GitHubCliError) -> RuntimeError {
    RuntimeError::Provider(format!("GitHub CLI operation failed: {error:?}"))
}

fn repo_args(args: &[&str], repo: &RepositoryIdentity) -> Vec<String> {
    let mut values = args
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    values.push("--repo".into());
    values.push(repo.slug.clone());
    values
}

const ISSUE_FIELDS: &str = "number,title,body,state,url,labels,assignees";
const PR_FIELDS: &str = "number,title,baseRefName,baseRefOid,headRefName,headRefOid,state,isDraft,mergeable,mergeStateStatus,reviewDecision,url,reviews";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoJson {
    name_with_owner: String,
    url: String,
    default_branch_ref: NamedRef,
}

#[derive(Deserialize)]
struct NamedRef {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueJson {
    number: u64,
    title: String,
    body: String,
    state: String,
    url: String,
    #[serde(default)]
    labels: Vec<NameValue>,
    #[serde(default)]
    assignees: Vec<LoginValue>,
}

impl From<IssueJson> for GitHubIssue {
    fn from(value: IssueJson) -> Self {
        Self {
            number: value.number,
            title: value.title,
            body_summary: value.body.chars().take(2_000).collect(),
            state: if value.state.eq_ignore_ascii_case("closed") {
                IssueState::Closed
            } else {
                IssueState::Open
            },
            labels: value.labels.into_iter().map(|label| label.name).collect(),
            assignees: value.assignees.into_iter().map(|user| user.login).collect(),
            url: value.url,
        }
    }
}

#[derive(Deserialize)]
struct NameValue {
    name: String,
}

#[derive(Deserialize)]
struct LoginValue {
    login: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestJson {
    number: u64,
    title: String,
    base_ref_name: String,
    base_ref_oid: Option<String>,
    head_ref_name: String,
    head_ref_oid: String,
    state: String,
    is_draft: bool,
    mergeable: Option<String>,
    merge_state_status: Option<String>,
    review_decision: Option<String>,
    url: String,
    #[serde(default)]
    reviews: Vec<ReviewJson>,
}

impl From<PullRequestJson> for PullRequest {
    fn from(value: PullRequestJson) -> Self {
        let reviews = value
            .reviews
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>();
        let review_state = match value.review_decision.as_deref() {
            Some("APPROVED") => ReviewState::Approved,
            Some("CHANGES_REQUESTED") => ReviewState::ChangesRequested,
            Some("REVIEW_REQUIRED") => ReviewState::Required,
            _ if reviews.is_empty() => ReviewState::Unknown,
            _ => ReviewState::Pending,
        };
        let state = match value.state.as_str() {
            "MERGED" => PullRequestState::Merged,
            "CLOSED" => PullRequestState::Closed,
            _ => PullRequestState::Open,
        };
        Self {
            number: value.number,
            title: value.title,
            base: value.base_ref_name,
            base_sha: value.base_ref_oid,
            head: value.head_ref_name,
            head_sha: value.head_ref_oid,
            state,
            draft: value.is_draft,
            mergeability: value.mergeable.or(value.merge_state_status),
            review_state,
            ci_state: CiState::Unknown,
            checks: Vec::new(),
            reviews,
            url: value.url,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewJson {
    #[serde(default)]
    author: Option<LoginValue>,
    state: String,
    #[serde(default)]
    body: String,
    submitted_at: Option<String>,
}

impl From<ReviewJson> for GitHubReview {
    fn from(value: ReviewJson) -> Self {
        Self {
            reviewer: value
                .author
                .map(|author| author.login)
                .unwrap_or_else(|| "unknown".into()),
            state: match value.state.as_str() {
                "APPROVED" => ReviewState::Approved,
                "CHANGES_REQUESTED" => ReviewState::ChangesRequested,
                "DISMISSED" => ReviewState::Dismissed,
                _ => ReviewState::Pending,
            },
            summary: value.body.chars().take(1_000).collect(),
            timestamp: value.submitted_at,
        }
    }
}

#[derive(Deserialize)]
struct CheckJson {
    name: String,
    state: String,
    bucket: String,
    link: Option<String>,
}

impl From<CheckJson> for CheckRunSummary {
    fn from(value: CheckJson) -> Self {
        Self {
            name: value.name,
            status: value.state,
            conclusion: match value.bucket.as_str() {
                "pass" => CheckState::Pass,
                "fail" => CheckState::Fail,
                "pending" => CheckState::Pending,
                "cancel" => CheckState::Cancelled,
                "skipping" => CheckState::Skipped,
                _ => CheckState::Unknown,
            },
            url: value.link,
        }
    }
}

pub fn aggregate_checks(checks: &[CheckRunSummary]) -> CiState {
    if checks
        .iter()
        .any(|check| check.conclusion == CheckState::Fail)
    {
        CiState::Fail
    } else if checks
        .iter()
        .any(|check| check.conclusion == CheckState::Pending)
    {
        CiState::Pending
    } else if checks
        .iter()
        .any(|check| check.conclusion == CheckState::Cancelled)
    {
        CiState::Cancelled
    } else if !checks.is_empty()
        && checks
            .iter()
            .all(|check| matches!(check.conclusion, CheckState::Pass | CheckState::Skipped))
    {
        CiState::Pass
    } else {
        CiState::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, process::ExitStatus, sync::Mutex};

    #[derive(Default)]
    struct FakeRunner {
        outputs: Mutex<VecDeque<(i32, String, String)>>,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl FakeRunner {
        fn with(outputs: Vec<(i32, &str, &str)>) -> Arc<Self> {
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

    impl GhRunner for FakeRunner {
        fn run(&self, _cwd: &Path, args: &[String]) -> std::io::Result<Output> {
            self.calls.lock().unwrap().push(args.to_vec());
            let (code, stdout, stderr) = self.outputs.lock().unwrap().pop_front().unwrap();
            Ok(Output {
                status: exit_status(code),
                stdout: stdout.into_bytes(),
                stderr: stderr.into_bytes(),
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

    fn repo() -> RepositoryIdentity {
        RepositoryIdentity {
            owner: "acme".into(),
            name: "app".into(),
            slug: "acme/app".into(),
            remote_url: "https://github.com/acme/app".into(),
            default_branch: "trunk".into(),
            remote_name: "origin".into(),
            current_upstream: None,
        }
    }

    #[test]
    fn repository_and_issue_contracts_use_json_and_explicit_binding() {
        let runner = FakeRunner::with(vec![
            (
                0,
                r#"{"nameWithOwner":"acme/app","url":"https://github.com/acme/app","defaultBranchRef":{"name":"trunk"}}"#,
                "",
            ),
            (
                0,
                r#"{"number":7,"title":"Fix","body":"Details","state":"OPEN","url":"https://github.com/acme/app/issues/7","labels":[{"name":"bug"}],"assignees":[{"login":"ada"}]}"#,
                "",
            ),
        ]);
        let service = GitHubService::with_runner(".", runner.clone());
        let identity = service.repository().unwrap();
        assert_eq!(identity.default_branch, "trunk");
        assert_eq!(service.issue(&identity, 7).unwrap().labels, vec!["bug"]);
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls[1][calls[1].len() - 2..], ["--repo", "acme/app"]);
    }

    #[test]
    fn issue_reconciliation_requires_exact_task_marker() {
        let runner = FakeRunner::with(vec![(
            0,
            r#"[{"number":7,"title":"Other","body":"Batai-Task: TASK-2","state":"OPEN","url":"https://github.com/acme/app/issues/7","labels":[],"assignees":[]},{"number":8,"title":"Match","body":"Created by Batai\n\nBatai-Task: TASK-1","state":"OPEN","url":"https://github.com/acme/app/issues/8","labels":[],"assignees":[]}]"#,
            "",
        )]);
        let service = GitHubService::with_runner(".", runner);
        let issue = service
            .find_issue_by_task_marker(&repo(), "TASK-1")
            .unwrap()
            .unwrap();
        assert_eq!(issue.number, 8);
    }

    #[test]
    fn duplicate_pr_is_reused_and_merge_is_sha_bound() {
        let pr = r#"[{"number":4,"title":"Ship","baseRefName":"trunk","headRefName":"batai/a/t","headRefOid":"abc","state":"OPEN","isDraft":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","reviewDecision":"APPROVED","url":"https://github.com/acme/app/pull/4","reviews":[]}]"#;
        let runner = FakeRunner::with(vec![(0, pr, "")]);
        let service = GitHubService::with_runner(".", runner.clone());
        let found = service
            .create_pull_request(&repo(), "Ship", "Body", "trunk", "batai/a/t", false)
            .unwrap();
        assert_eq!(found.number, 4);
        assert_eq!(runner.calls.lock().unwrap().len(), 1);

        let current = pr.trim_start_matches('[').trim_end_matches(']');
        let runner = FakeRunner::with(vec![(0, current, "")]);
        let service = GitHubService::with_runner(".", runner);
        assert!(service
            .merge(&repo(), 4, "stale", MergeMethod::Squash)
            .is_err());
    }

    #[test]
    fn merge_uses_server_side_head_guard_and_adopts_merged_result() {
        let open = r#"{"number":4,"title":"Ship","baseRefName":"trunk","baseRefOid":"base","headRefName":"batai/a/t","headRefOid":"abc","state":"OPEN","isDraft":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","reviewDecision":"APPROVED","url":"https://github.com/acme/app/pull/4","reviews":[]}"#;
        let merged = open.replace("\"state\":\"OPEN\"", "\"state\":\"MERGED\"");
        let runner = FakeRunner::with(vec![(0, open, ""), (0, "merged", ""), (0, &merged, "")]);
        let service = GitHubService::with_runner(".", runner.clone());
        let result = service
            .merge(&repo(), 4, "abc", MergeMethod::Squash)
            .unwrap();
        assert_eq!(result.state, PullRequestState::Merged);
        let calls = runner.calls.lock().unwrap();
        assert!(calls[1]
            .windows(2)
            .any(|values| values == ["--match-head-commit", "abc"]));
        assert!(calls[1].contains(&"--squash".into()));
    }

    #[test]
    fn check_aggregation_never_treats_unknown_as_pass() {
        let check = |conclusion| CheckRunSummary {
            name: "ci".into(),
            status: "done".into(),
            conclusion,
            url: None,
        };
        assert_eq!(aggregate_checks(&[]), CiState::Unknown);
        assert_eq!(
            aggregate_checks(&[check(CheckState::Unknown)]),
            CiState::Unknown
        );
        assert_eq!(aggregate_checks(&[check(CheckState::Pass)]), CiState::Pass);
        assert_eq!(
            aggregate_checks(&[check(CheckState::Pending)]),
            CiState::Pending
        );
        assert_eq!(aggregate_checks(&[check(CheckState::Fail)]), CiState::Fail);
    }

    #[test]
    fn auth_failures_are_classified_without_exposing_output() {
        let runner = FakeRunner::with(vec![(1, "", "please login with token secret")]);
        let service = GitHubService::with_runner(".", runner);
        assert_eq!(service.auth_status(), GitHubAuthState::AuthRequired);
    }

    #[test]
    fn real_github_delivery_smoke_is_explicit_and_disposable_only() {
        if std::env::var("BATAI_REAL_GITHUB_E2E").as_deref() != Ok("1") {
            return;
        }
        let root = PathBuf::from(
            std::env::var_os("BATAI_REAL_GITHUB_E2E_ROOT")
                .expect("set BATAI_REAL_GITHUB_E2E_ROOT to a disposable checkout"),
        );
        let expected_repo = std::env::var("BATAI_REAL_GITHUB_E2E_REPOSITORY")
            .expect("set the exact disposable owner/repository slug");
        let marker = std::fs::read_to_string(root.join(".batai-real-github-e2e"))
            .expect("disposable repository must contain the explicit safety marker");
        assert_eq!(
            marker.trim(),
            "I understand this repository will receive a temporary Batai PR"
        );
        let service = GitHubService::new(&root);
        assert_eq!(service.auth_status(), GitHubAuthState::Available);
        let repo = service
            .repository()
            .expect("detect real disposable repository");
        assert_eq!(repo.slug, expected_repo);
        let task = format!("e2e-{}", uuid::Uuid::new_v4().simple());
        let worktrees = crate::runtime::worktrees::WorktreeManager::discover(&root).unwrap();
        let binding = worktrees
            .ensure_from("github-e2e", &task, &repo.default_branch)
            .unwrap();
        std::fs::write(
            binding.path.join("batai-github-e2e.txt"),
            "Batai GitHub E2E\n",
        )
        .unwrap();
        let head = worktrees
            .commit(&binding, "test: Batai GitHub delivery E2E")
            .unwrap()
            .unwrap();
        let pushed = Command::new("git")
            .current_dir(&binding.path)
            .args(["push", "--set-upstream", "origin", &binding.branch])
            .output()
            .unwrap();
        assert!(
            pushed.status.success(),
            "push failed without exposing credentials"
        );
        let pr = service
            .create_pull_request(
                &repo,
                "Batai disposable delivery E2E",
                "Temporary opt-in validation. Safe to close.",
                &repo.default_branch,
                &binding.branch,
                true,
            )
            .unwrap();
        assert_eq!(pr.head_sha, head);
        let closed = service.close_pull_request(&repo, pr.number).unwrap();
        assert_eq!(closed.state, PullRequestState::Closed);
        let deleted = Command::new("git")
            .current_dir(&root)
            .args(["push", "origin", "--delete", &binding.branch])
            .output()
            .unwrap();
        assert!(deleted.status.success());
        worktrees.remove(&binding).unwrap();
    }
}
