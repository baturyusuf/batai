//! Deterministic, bounded context selection shared by intelligence operations.
//!
//! The builder intentionally does not crawl the repository. It combines explicit
//! references with a small set of durable Batai artifacts and keeps source
//! provenance separate from transient source content.

use std::{
    collections::{BTreeSet, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

use chrono::Utc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    errors::{Result, RuntimeError},
    governance::{DecisionStatus, GodDecision, ReviewOutcome},
    meetings::Meeting,
    organization::AgentFunction,
    store::RuntimeStore,
    types::{Agent, Task},
};

const DEFAULT_MAX_TOKENS: u64 = 6_000;
const DEFAULT_MAX_FILES: usize = 12;
const DEFAULT_MAX_BYTES_PER_FILE: u64 = 64 * 1024;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 192 * 1024;
const DEFAULT_MAX_HISTORY: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContextOperationType {
    Meeting,
    TaskExecution,
    Review,
    DirectorPlanning,
    Handoff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContextBuildStatus {
    Ready,
    ReadyWithWarnings,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContextSourceType {
    GodDirective,
    Task,
    Interface,
    Decision,
    File,
    Handoff,
    Review,
    MeetingOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub struct ContextBudget {
    pub max_tokens: u64,
    pub max_files: usize,
    pub max_bytes_per_file: u64,
    pub max_total_bytes: u64,
    pub max_historical_artifacts: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_tokens: DEFAULT_MAX_TOKENS,
            max_files: DEFAULT_MAX_FILES,
            max_bytes_per_file: DEFAULT_MAX_BYTES_PER_FILE,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            max_historical_artifacts: DEFAULT_MAX_HISTORY,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextSourceRecord {
    pub source_type: ContextSourceType,
    pub source_id: String,
    pub reason_included: String,
    pub bytes: u64,
    pub estimated_tokens: u64,
    pub fingerprint: String,
    pub freshness: Option<String>,
    pub priority: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextExclusion {
    pub source_id: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ContextItem {
    pub record: ContextSourceRecord,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ContextPackage {
    pub context_id: String,
    pub fingerprint: String,
    pub created_at: String,
    pub operation: ContextOperationType,
    pub objective: String,
    pub actor_id: String,
    pub participant_role: Option<AgentFunction>,
    pub base_sha: Option<String>,
    pub status: ContextBuildStatus,
    pub items: Vec<ContextItem>,
    pub excluded: Vec<ContextExclusion>,
    pub warnings: Vec<String>,
    pub estimated_tokens: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextPackageRecord {
    pub context_id: String,
    pub fingerprint: String,
    pub created_at: String,
    pub operation: ContextOperationType,
    pub actor_id: String,
    pub participant_role: Option<AgentFunction>,
    pub base_sha: Option<String>,
    pub status: ContextBuildStatus,
    pub sources: Vec<ContextSourceRecord>,
    pub excluded: Vec<ContextExclusion>,
    pub warnings: Vec<String>,
    pub estimated_tokens: u64,
    pub total_bytes: u64,
}

impl ContextPackage {
    pub fn record(&self) -> ContextPackageRecord {
        ContextPackageRecord {
            context_id: self.context_id.clone(),
            fingerprint: self.fingerprint.clone(),
            created_at: self.created_at.clone(),
            operation: self.operation,
            actor_id: self.actor_id.clone(),
            participant_role: self.participant_role,
            base_sha: self.base_sha.clone(),
            status: self.status,
            sources: self.items.iter().map(|item| item.record.clone()).collect(),
            excluded: self.excluded.clone(),
            warnings: self.warnings.clone(),
            estimated_tokens: self.estimated_tokens,
            total_bytes: self.total_bytes,
        }
    }

    pub fn render(&self) -> String {
        self.items
            .iter()
            .map(|item| {
                format!(
                    "SOURCE: {:?}\n{}\nREASON: {}\n{}",
                    item.record.source_type,
                    item.record.source_id,
                    item.record.reason_included,
                    item.content
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[derive(Debug, Clone)]
pub struct ContextBuildRequest {
    pub operation: ContextOperationType,
    pub objective: String,
    pub actor: Agent,
    pub linked_task: Option<String>,
    pub linked_meeting: Option<String>,
    pub requested_capabilities: BTreeSet<String>,
    pub repository: PathBuf,
    pub worktree: Option<PathBuf>,
    pub explicit_files: Vec<String>,
    pub budget: ContextBudget,
}

#[derive(Clone)]
pub struct ContextBuilder {
    project_root: PathBuf,
    store: RuntimeStore,
}

#[derive(Debug)]
struct Candidate {
    source_type: ContextSourceType,
    source_id: String,
    reason: String,
    content: String,
    freshness: Option<String>,
    priority: u8,
    is_file: bool,
    historical: bool,
}

impl ContextBuilder {
    pub fn new(project_root: PathBuf, store: RuntimeStore) -> Self {
        Self {
            project_root,
            store,
        }
    }

    pub fn build(&self, request: ContextBuildRequest) -> Result<ContextPackage> {
        validate_budget(&request.budget)?;
        let source_root = request
            .worktree
            .as_deref()
            .unwrap_or(request.repository.as_path());
        let source_root = canonical_directory(source_root)?;
        let project_root = canonical_directory(&self.project_root)?;
        let base_sha = git_head(&source_root);
        let mut candidates = Vec::new();
        let mut excluded = Vec::new();
        let mut warnings = Vec::new();

        candidates.push(Candidate {
            source_type: ContextSourceType::Task,
            source_id: request
                .linked_task
                .clone()
                .unwrap_or_else(|| "CURRENT_OBJECTIVE".into()),
            reason: "current operation objective".into(),
            content: request.objective.trim().to_owned(),
            freshness: None,
            priority: 2,
            is_file: false,
            historical: false,
        });

        self.add_directives(&request, &project_root, &mut candidates, &mut warnings)?;
        let task = self.add_task(&request, &mut candidates)?;
        self.add_decisions(&request, &mut candidates)?;
        self.add_adrs(
            &request,
            &source_root,
            &project_root,
            &mut candidates,
            &mut excluded,
            &mut warnings,
        )?;
        self.add_handoff(&request, &project_root, &mut candidates, &mut warnings)?;
        self.add_reviews(&request, &mut candidates)?;
        self.add_meeting_history(&request, &mut candidates)?;

        let paths = collect_file_references(&request, task.as_ref());
        for (relative, reason, explicit) in paths {
            match read_safe_file(
                &source_root,
                &relative,
                request.budget.max_bytes_per_file,
                explicit,
            ) {
                Ok(Some((content, actual, freshness))) => {
                    let source_type = if is_interface_path(&relative) {
                        ContextSourceType::Interface
                    } else {
                        ContextSourceType::File
                    };
                    if participant_relevant(request.actor.function, &relative, explicit) {
                        candidates.push(Candidate {
                            source_type,
                            source_id: relative.clone(),
                            reason,
                            content,
                            freshness: Some(format!("{}@{}", actual.to_string_lossy(), freshness)),
                            priority: if explicit { 4 } else { 6 },
                            is_file: true,
                            historical: false,
                        });
                    } else {
                        excluded.push(ContextExclusion {
                            source_id: relative,
                            reason: "not relevant to participant role".into(),
                        });
                    }
                }
                Ok(None) => excluded.push(ContextExclusion {
                    source_id: relative,
                    reason: "sensitive, binary, or oversized file excluded".into(),
                }),
                Err(error) if explicit => {
                    warnings.push(error.to_string());
                    return Ok(blocked_package(request, base_sha, excluded, warnings));
                }
                Err(error) => warnings.push(error.to_string()),
            }
        }

        candidates.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.source_id.cmp(&right.source_id))
        });
        candidates.dedup_by(|left, right| {
            left.source_type == right.source_type && left.source_id == right.source_id
        });

        let mut items = Vec::new();
        let mut total_bytes = 0_u64;
        let mut estimated_tokens = 0_u64;
        let mut files = 0_usize;
        let mut historical = 0_usize;
        for candidate in candidates {
            let bytes = candidate.content.len() as u64;
            let tokens = estimate_tokens(bytes);
            let over_limit = (candidate.is_file && files >= request.budget.max_files)
                || (candidate.historical && historical >= request.budget.max_historical_artifacts)
                || total_bytes.saturating_add(bytes) > request.budget.max_total_bytes
                || estimated_tokens.saturating_add(tokens) > request.budget.max_tokens;
            if over_limit {
                excluded.push(ContextExclusion {
                    source_id: candidate.source_id,
                    reason: "excluded by context budget priority".into(),
                });
                continue;
            }
            if candidate.is_file {
                files += 1;
            }
            if candidate.historical {
                historical += 1;
            }
            total_bytes += bytes;
            estimated_tokens += tokens;
            let fingerprint = hash_bytes(candidate.content.as_bytes());
            items.push(ContextItem {
                record: ContextSourceRecord {
                    source_type: candidate.source_type,
                    source_id: candidate.source_id,
                    reason_included: candidate.reason,
                    bytes,
                    estimated_tokens: tokens,
                    fingerprint,
                    freshness: candidate.freshness,
                    priority: candidate.priority,
                },
                content: candidate.content,
            });
        }
        if !excluded.is_empty() {
            warnings.push(format!(
                "{} source(s) were excluded by safety or budget policy",
                excluded.len()
            ));
        }
        let fingerprint = package_fingerprint(
            request.operation,
            &request.objective,
            base_sha.as_deref(),
            &items,
        );
        Ok(ContextPackage {
            context_id: format!("CTX-{}", uuid::Uuid::new_v4()),
            fingerprint,
            created_at: Utc::now().to_rfc3339(),
            operation: request.operation,
            objective: request.objective,
            actor_id: request.actor.id,
            participant_role: request.actor.function,
            base_sha,
            status: if warnings.is_empty() {
                ContextBuildStatus::Ready
            } else {
                ContextBuildStatus::ReadyWithWarnings
            },
            items,
            excluded,
            warnings,
            estimated_tokens,
            total_bytes,
        })
    }

    pub fn is_fresh(&self, record: &ContextPackageRecord, root: &Path) -> Result<bool> {
        let root = canonical_directory(root)?;
        if record.base_sha != git_head(&root) {
            return Ok(false);
        }
        for source in &record.sources {
            if !matches!(
                source.source_type,
                ContextSourceType::File | ContextSourceType::Interface
            ) {
                continue;
            }
            let Some((content, _, _)) = read_safe_file(&root, &source.source_id, u64::MAX, true)?
            else {
                return Ok(false);
            };
            if hash_bytes(content.as_bytes()) != source.fingerprint {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn add_directives(
        &self,
        request: &ContextBuildRequest,
        project_root: &Path,
        candidates: &mut Vec<Candidate>,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        for (path, id, reason) in [
            (
                project_root.join(".batai/DIRECTIVES.md"),
                "PROJECT_DIRECTIVES".to_owned(),
                "active project/GOD directives".to_owned(),
            ),
            (
                project_root
                    .join(".batai/agents")
                    .join(&request.actor.id)
                    .join("DIRECTIVES.md"),
                format!("AGENT_DIRECTIVES:{}", request.actor.id),
                "active participant directives".to_owned(),
            ),
        ] {
            if !path.exists() {
                continue;
            }
            match fs::read_to_string(&path) {
                Ok(content) => candidates.push(Candidate {
                    source_type: ContextSourceType::GodDirective,
                    source_id: id,
                    reason,
                    content,
                    freshness: file_freshness(&path),
                    priority: 0,
                    is_file: false,
                    historical: false,
                }),
                Err(error) => warnings.push(format!(
                    "directive {} could not be read: {error}",
                    path.display()
                )),
            }
        }
        Ok(())
    }

    fn add_task(
        &self,
        request: &ContextBuildRequest,
        candidates: &mut Vec<Candidate>,
    ) -> Result<Option<Task>> {
        let Some(task_id) = request.linked_task.as_deref() else {
            return Ok(None);
        };
        let task = self
            .store
            .get_task(task_id)?
            .ok_or_else(|| RuntimeError::TaskNotFound(task_id.into()))?;
        candidates.push(Candidate {
            source_type: ContextSourceType::Task,
            source_id: task.id.clone(),
            reason: "linked task definition".into(),
            content: serde_json::to_string_pretty(&serde_json::json!({
                "objective": task.objective,
                "acceptanceCriteria": task.acceptance_criteria,
                "inputs": task.inputs,
                "outputs": task.outputs,
                "status": task.status,
            }))?,
            freshness: None,
            priority: 1,
            is_file: false,
            historical: false,
        });
        Ok(Some(task))
    }

    fn add_decisions(
        &self,
        request: &ContextBuildRequest,
        candidates: &mut Vec<Candidate>,
    ) -> Result<()> {
        let decisions: Vec<GodDecision> = self.store.list_governance_records("GOD_DECISION")?;
        let keywords = keywords(&request.objective);
        for decision in decisions.into_iter().filter(|decision| {
            decision.status == DecisionStatus::Approved
                && (request
                    .linked_task
                    .as_deref()
                    .is_some_and(|task| decision.request.task_id.as_deref() == Some(task))
                    || relevant_text(&decision.question, &keywords))
        }) {
            candidates.push(Candidate {
                source_type: ContextSourceType::Decision,
                source_id: decision.id.clone(),
                reason: "accepted relevant GOD decision".into(),
                content: format!(
                    "APPROVED\nQuestion: {}\nResolution: {}",
                    decision.question,
                    decision.resolution_note.unwrap_or_default()
                ),
                freshness: decision.resolved_at,
                priority: 3,
                is_file: false,
                historical: true,
            });
        }
        Ok(())
    }

    fn add_adrs(
        &self,
        request: &ContextBuildRequest,
        source_root: &Path,
        project_root: &Path,
        candidates: &mut Vec<Candidate>,
        excluded: &mut Vec<ContextExclusion>,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        let keywords = keywords(&request.objective);
        let directories = [
            source_root.join("docs/adr"),
            project_root.join(".batai/adrs"),
        ];
        let scan_limit = request
            .budget
            .max_historical_artifacts
            .saturating_mul(4)
            .max(4);
        for directory in directories {
            if !directory.is_dir() {
                continue;
            }
            let mut entries = fs::read_dir(&directory)?
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry.path().is_file())
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| {
                            matches!(value.to_ascii_lowercase().as_str(), "md" | "json")
                        })
                })
                .collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries.into_iter().take(scan_limit) {
                let path = entry.path();
                let id = path
                    .strip_prefix(source_root)
                    .or_else(|_| path.strip_prefix(project_root))
                    .unwrap_or(path.as_path())
                    .to_string_lossy()
                    .replace('\\', "/");
                match fs::read_to_string(&path) {
                    Ok(content) => {
                        let upper = content.to_ascii_uppercase();
                        if upper.contains("SUPERSEDED") || upper.contains("REJECTED") {
                            excluded.push(ContextExclusion {
                                source_id: id,
                                reason: "superseded/rejected decision is historical only".into(),
                            });
                        } else if relevant_text(&content, &keywords) {
                            candidates.push(Candidate {
                                source_type: ContextSourceType::Decision,
                                source_id: id,
                                reason: "accepted relevant architecture decision".into(),
                                content,
                                freshness: file_freshness(&path),
                                priority: 3,
                                is_file: false,
                                historical: true,
                            });
                        }
                    }
                    Err(error) => warnings.push(format!(
                        "architecture decision {} could not be read: {error}",
                        path.display()
                    )),
                }
            }
        }
        Ok(())
    }

    fn add_handoff(
        &self,
        request: &ContextBuildRequest,
        project_root: &Path,
        candidates: &mut Vec<Candidate>,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        let Some(task_id) = request.linked_task.as_deref() else {
            return Ok(());
        };
        let path = project_root
            .join(".batai/handoffs")
            .join(format!("{task_id}.json"));
        if path.exists() {
            match fs::read_to_string(&path) {
                Ok(content) => candidates.push(Candidate {
                    source_type: ContextSourceType::Handoff,
                    source_id: task_id.into(),
                    reason: "latest linked task handoff".into(),
                    content,
                    freshness: file_freshness(&path),
                    priority: 7,
                    is_file: false,
                    historical: true,
                }),
                Err(error) => warnings.push(format!("handoff could not be read: {error}")),
            }
        }
        Ok(())
    }

    fn add_reviews(
        &self,
        request: &ContextBuildRequest,
        candidates: &mut Vec<Candidate>,
    ) -> Result<()> {
        let Some(task_id) = request.linked_task.as_deref() else {
            return Ok(());
        };
        let reviews: Vec<ReviewOutcome> = self.store.list_review_outcomes(None)?;
        for review in reviews
            .into_iter()
            .filter(|review| review.task_id == task_id)
        {
            candidates.push(Candidate {
                source_type: ContextSourceType::Review,
                source_id: review.id,
                reason: "review finding for linked task".into(),
                content: format!(
                    "Outcome: {:?}\nReviewer: {}\nFinding: {}",
                    review.outcome,
                    review.reviewer_id,
                    review.note.unwrap_or_default()
                ),
                freshness: Some(review.created_at),
                priority: 6,
                is_file: false,
                historical: true,
            });
        }
        Ok(())
    }

    fn add_meeting_history(
        &self,
        request: &ContextBuildRequest,
        candidates: &mut Vec<Candidate>,
    ) -> Result<()> {
        let meetings: Vec<Meeting> = self.store.list_meetings()?;
        for meeting in meetings.into_iter().filter(|meeting| {
            meeting.result.is_some()
                && (request
                    .linked_meeting
                    .as_deref()
                    .is_some_and(|id| meeting.id == id)
                    || request
                        .linked_task
                        .as_deref()
                        .is_some_and(|task| meeting.linked_task.as_deref() == Some(task)))
        }) {
            let result = meeting.result.expect("filtered result");
            candidates.push(Candidate {
                source_type: ContextSourceType::MeetingOutcome,
                source_id: meeting.id,
                reason: "prior outcome linked to this operation".into(),
                content: serde_json::to_string_pretty(&result)?,
                freshness: meeting.completed_at,
                priority: 8,
                is_file: false,
                historical: true,
            });
        }
        Ok(())
    }
}

fn collect_file_references(
    request: &ContextBuildRequest,
    task: Option<&Task>,
) -> Vec<(String, String, bool)> {
    let mut values = Vec::new();
    for file in &request.explicit_files {
        values.push((file.clone(), "explicitly linked file".into(), true));
    }
    if let Some(task) = task {
        for input in &task.inputs {
            if looks_like_path(input) {
                values.push((
                    input.clone(),
                    "file referenced by linked task".into(),
                    false,
                ));
            }
        }
        for key in ["changed_files", "affected_files", "review_files"] {
            if let Some(files) = task.extra.get(key).and_then(|value| value.as_array()) {
                for file in files.iter().filter_map(|value| value.as_str()) {
                    values.push((file.into(), format!("{key} task evidence"), false));
                }
            }
        }
    }
    for candidate in [
        "README.md",
        "docs/ARCHITECTURE.md",
        "docs/adr/README.md",
        "src-tauri/src/domain.rs",
    ] {
        if participant_relevant(request.actor.function, candidate, false) {
            values.push((
                candidate.into(),
                "known architecture/interface path".into(),
                false,
            ));
        }
    }
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|(path, _, _)| seen.insert(path.replace('\\', "/").to_ascii_lowercase()))
        .collect()
}

fn participant_relevant(
    function: Option<AgentFunction>,
    path: &str,
    explicitly_linked: bool,
) -> bool {
    if explicitly_linked {
        return true;
    }
    let path = path.to_ascii_lowercase().replace('\\', "/");
    match function.unwrap_or(AgentFunction::GenericSoftwareAgent) {
        AgentFunction::SoftwareArchitecture
        | AgentFunction::SolutionArchitecture
        | AgentFunction::AiArchitecture
        | AgentFunction::Director => {
            path.contains("architect")
                || path.contains("adr")
                || path.ends_with("readme.md")
                || path.ends_with("domain.rs")
                || path.contains("interface")
                || path.contains("contract")
        }
        AgentFunction::BackendEngineering
        | AgentFunction::DatabaseEngineering
        | AgentFunction::PlatformEngineering
        | AgentFunction::DevopsEngineering => {
            path.contains("backend")
                || path.contains("server")
                || path.contains("runtime")
                || path.contains("api")
                || path.contains("database")
                || path.contains("domain")
        }
        AgentFunction::SecurityEngineering => {
            path.contains("security")
                || path.contains("auth")
                || path.contains("permission")
                || path.contains("policy")
                || path.contains("interface")
        }
        _ => true,
    }
}

fn read_safe_file(
    root: &Path,
    relative: &str,
    max_bytes: u64,
    required: bool,
) -> Result<Option<(String, PathBuf, String)>> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(RuntimeError::Governance(format!(
            "context path escapes repository: {relative}"
        )));
    }
    if sensitive_path(relative_path) {
        if required {
            return Err(RuntimeError::Governance(format!(
                "sensitive context path is denied: {relative}"
            )));
        }
        return Ok(None);
    }
    let candidate = root.join(relative_path);
    if !candidate.exists() {
        if required {
            return Err(RuntimeError::Governance(format!(
                "required context file is missing: {relative}"
            )));
        }
        return Ok(None);
    }
    let actual = candidate.canonicalize()?;
    if !actual.starts_with(root) {
        return Err(RuntimeError::Governance(format!(
            "context path resolves outside worktree: {relative}"
        )));
    }
    let metadata = fs::metadata(&actual)?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Ok(None);
    }
    let bytes = fs::read(&actual)?;
    if bytes.contains(&0) {
        return Ok(None);
    }
    let content = String::from_utf8(bytes).map_err(|_| {
        RuntimeError::Governance(format!("binary/non-UTF8 context excluded: {relative}"))
    })?;
    Ok(Some((
        content,
        actual,
        file_freshness(&candidate).unwrap_or_else(|| "UNKNOWN".into()),
    )))
}

fn sensitive_path(path: &Path) -> bool {
    path.components().any(|component| {
        let value = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        value == ".env"
            || value.starts_with(".env.")
            || value.contains("credential")
            || value.contains("secret")
            || value.contains("private_key")
            || value == "id_rsa"
            || value == "id_ed25519"
            || value.ends_with(".pem")
            || value.ends_with(".p12")
            || value.ends_with(".key")
    })
}

fn is_interface_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.contains("interface")
        || path.contains("contract")
        || path.ends_with("domain.rs")
        || path.contains("/api/")
}

fn looks_like_path(value: &str) -> bool {
    !value.contains('\n')
        && (value.contains('/') || value.contains('\\'))
        && Path::new(value).extension().is_some()
}

fn canonical_directory(path: &Path) -> Result<PathBuf> {
    let actual = path.canonicalize()?;
    if !actual.is_dir() {
        return Err(RuntimeError::Governance(format!(
            "context root is not a directory: {}",
            path.display()
        )));
    }
    Ok(actual)
}

fn git_head(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn file_freshness(path: &Path) -> Option<String> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(chrono::DateTime::<Utc>::from(modified).to_rfc3339())
}

fn estimate_tokens(bytes: u64) -> u64 {
    bytes.saturating_add(3) / 4
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn package_fingerprint(
    operation: ContextOperationType,
    objective: &str,
    base_sha: Option<&str>,
    items: &[ContextItem],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "{operation:?}\n{objective}\n{}",
        base_sha.unwrap_or("")
    ));
    for item in items {
        hasher.update(format!(
            "\n{:?}\n{}\n{}",
            item.record.source_type, item.record.source_id, item.record.fingerprint
        ));
    }
    format!("{:x}", hasher.finalize())
}

fn validate_budget(budget: &ContextBudget) -> Result<()> {
    if budget.max_tokens == 0
        || budget.max_files == 0
        || budget.max_bytes_per_file == 0
        || budget.max_total_bytes == 0
    {
        return Err(RuntimeError::Governance(
            "context budgets must be positive".into(),
        ));
    }
    Ok(())
}

fn blocked_package(
    request: ContextBuildRequest,
    base_sha: Option<String>,
    excluded: Vec<ContextExclusion>,
    warnings: Vec<String>,
) -> ContextPackage {
    let fingerprint = package_fingerprint(
        request.operation,
        &request.objective,
        base_sha.as_deref(),
        &[],
    );
    ContextPackage {
        context_id: format!("CTX-{}", uuid::Uuid::new_v4()),
        fingerprint,
        created_at: Utc::now().to_rfc3339(),
        operation: request.operation,
        objective: request.objective,
        actor_id: request.actor.id,
        participant_role: request.actor.function,
        base_sha,
        status: ContextBuildStatus::Blocked,
        items: Vec::new(),
        excluded,
        warnings,
        estimated_tokens: 0,
        total_bytes: 0,
    }
}

fn keywords(value: &str) -> BTreeSet<String> {
    value
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .map(str::to_ascii_lowercase)
        .filter(|word| word.len() >= 4)
        .collect()
}

fn relevant_text(value: &str, keywords: &BTreeSet<String>) -> bool {
    let value = value.to_ascii_lowercase();
    keywords.iter().any(|word| value.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        organization::{AuthorityRole, IntelligencePolicy, Seniority},
        types::{AgentStatus, TaskExecution, TaskStatus, TaskSuccessAction},
    };

    fn agent(id: &str, function: AgentFunction, root: &Path) -> Agent {
        Agent {
            schema_version: 2,
            id: id.into(),
            name: id.into(),
            role_template: function.base_title().into(),
            seniority: Some(Seniority::Senior),
            function: Some(function),
            department: Some(function.department()),
            title_override: None,
            model_capabilities: Default::default(),
            effective_capabilities: Default::default(),
            lifecycle: Default::default(),
            authority: AuthorityRole::Worker,
            permissions: Vec::new(),
            intelligence_policy: IntelligencePolicy::default(),
            parent_agent_id: Some("director".into()),
            provider: "mock".into(),
            model: "mock".into(),
            reasoning_effort: "medium".into(),
            auth_mode: "local".into(),
            worktree: Some(root.to_string_lossy().into()),
            status: AgentStatus::Ready,
            current_task_id: None,
            extra: Default::default(),
        }
    }

    fn request(root: &Path, store: &RuntimeStore, function: AgentFunction) -> ContextBuildRequest {
        let _ = store;
        ContextBuildRequest {
            operation: ContextOperationType::Meeting,
            objective: "Choose the authentication architecture".into(),
            actor: agent("participant", function, root),
            linked_task: None,
            linked_meeting: None,
            requested_capabilities: BTreeSet::new(),
            repository: root.into(),
            worktree: Some(root.into()),
            explicit_files: Vec::new(),
            budget: ContextBudget::default(),
        }
    }

    #[test]
    fn explicit_files_directives_provenance_and_fingerprint_are_deterministic() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".batai/agents/participant")).unwrap();
        fs::write(
            temp.path().join(".batai/DIRECTIVES.md"),
            "Prefer typed APIs",
        )
        .unwrap();
        fs::write(temp.path().join("service.rs"), "pub trait Service {}").unwrap();
        let store = RuntimeStore::open(temp.path().join("runtime.sqlite")).unwrap();
        let builder = ContextBuilder::new(temp.path().into(), store.clone());
        let mut first_request = request(temp.path(), &store, AgentFunction::BackendEngineering);
        first_request.explicit_files = vec!["service.rs".into()];
        let first = builder.build(first_request.clone()).unwrap();
        let second = builder.build(first_request).unwrap();
        assert_eq!(first.fingerprint, second.fingerprint);
        assert!(first.items.iter().any(|item| {
            item.record.source_type == ContextSourceType::GodDirective
                && item.record.reason_included == "active project/GOD directives"
        }));
        assert!(first
            .items
            .iter()
            .any(|item| item.record.source_id == "service.rs"));
        assert!(!first.render().contains("runtime.sqlite"));
    }

    #[test]
    fn path_traversal_secret_binary_and_budget_are_enforced() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".batai")).unwrap();
        fs::write(temp.path().join(".env"), "TOKEN=secret").unwrap();
        fs::write(temp.path().join("binary.bin"), [0, 1, 2]).unwrap();
        fs::write(temp.path().join("large.rs"), "x".repeat(128)).unwrap();
        let store = RuntimeStore::open(temp.path().join("runtime.sqlite")).unwrap();
        let builder = ContextBuilder::new(temp.path().into(), store.clone());

        for path in ["../outside", ".env"] {
            let mut value = request(temp.path(), &store, AgentFunction::BackendEngineering);
            value.explicit_files = vec![path.into()];
            assert_eq!(
                builder.build(value).unwrap().status,
                ContextBuildStatus::Blocked
            );
        }
        let mut value = request(temp.path(), &store, AgentFunction::BackendEngineering);
        value.explicit_files = vec!["binary.bin".into(), "large.rs".into()];
        value.budget.max_bytes_per_file = 32;
        let package = builder.build(value).unwrap();
        assert_eq!(package.items.len(), 1); // objective only
        assert!(package
            .excluded
            .iter()
            .any(|item| item.source_id == "binary.bin"));
        assert!(package
            .excluded
            .iter()
            .any(|item| item.source_id == "large.rs"));
    }

    #[test]
    fn task_review_and_participant_specific_files_are_selected_without_peer_positions() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".batai")).unwrap();
        fs::create_dir_all(temp.path().join("src/runtime")).unwrap();
        fs::write(
            temp.path().join("src/runtime/api.rs"),
            "pub fn backend() {}",
        )
        .unwrap();
        fs::write(temp.path().join("README.md"), "Architecture overview").unwrap();
        let store = RuntimeStore::open(temp.path().join("runtime.sqlite")).unwrap();
        let mut extra = serde_json::Map::new();
        extra.insert(
            "changed_files".into(),
            serde_json::json!(["src/runtime/api.rs"]),
        );
        let task = Task {
            id: "TASK-1".into(),
            created_by: "director".into(),
            objective: "Implement API".into(),
            assigned_to: vec!["participant".into()],
            dependencies: Vec::new(),
            acceptance_criteria: vec!["Tests pass".into()],
            inputs: Vec::new(),
            outputs: Vec::new(),
            status: TaskStatus::Ready,
            execution: TaskExecution::default(),
            on_success: TaskSuccessAction::default(),
            on_failure: TaskSuccessAction::default(),
            weight: 1.0,
            extra,
        };
        store.ingest_task(&task, None, "fixture").unwrap();
        store
            .append_review_outcome(
                "REV-1",
                "TASK-1",
                "reviewer",
                "participant",
                "CHANGES_REQUESTED",
                &ReviewOutcome {
                    id: "REV-1".into(),
                    task_id: "TASK-1".into(),
                    reviewer_id: "reviewer".into(),
                    subject_agent_id: "participant".into(),
                    outcome: super::super::governance::ReviewOutcomeKind::ChangesRequested,
                    note: Some("Keep the contract stable".into()),
                    created_at: Utc::now().to_rfc3339(),
                },
            )
            .unwrap();
        let builder = ContextBuilder::new(temp.path().into(), store.clone());
        let mut value = request(temp.path(), &store, AgentFunction::BackendEngineering);
        value.linked_task = Some("TASK-1".into());
        let package = builder.build(value).unwrap();
        assert!(package
            .items
            .iter()
            .any(|item| item.record.source_type == ContextSourceType::Review));
        assert!(package
            .items
            .iter()
            .any(|item| item.record.source_id == "src/runtime/api.rs"));
        assert!(!package.render().contains("peer position"));
    }

    #[test]
    fn relevant_file_change_is_detected_as_stale() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".batai")).unwrap();
        fs::write(temp.path().join("contract.rs"), "v1").unwrap();
        let store = RuntimeStore::open(temp.path().join("runtime.sqlite")).unwrap();
        let builder = ContextBuilder::new(temp.path().into(), store.clone());
        let mut value = request(temp.path(), &store, AgentFunction::BackendEngineering);
        value.explicit_files = vec!["contract.rs".into()];
        let package = builder.build(value).unwrap();
        assert!(builder.is_fresh(&package.record(), temp.path()).unwrap());
        fs::write(temp.path().join("contract.rs"), "v2").unwrap();
        assert!(!builder.is_fresh(&package.record(), temp.path()).unwrap());
    }

    #[test]
    fn worktree_content_and_role_specific_sources_are_used() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".batai")).unwrap();
        fs::create_dir_all(worktree.path().join("src/runtime")).unwrap();
        fs::write(temp.path().join("contract.rs"), "base version").unwrap();
        fs::write(worktree.path().join("contract.rs"), "worktree version").unwrap();
        fs::write(worktree.path().join("src/runtime/api.rs"), "backend API").unwrap();
        fs::write(worktree.path().join("README.md"), "architecture map").unwrap();
        let store = RuntimeStore::open(temp.path().join("runtime.sqlite")).unwrap();
        let builder = ContextBuilder::new(temp.path().into(), store.clone());

        let mut backend = request(temp.path(), &store, AgentFunction::BackendEngineering);
        backend.worktree = Some(worktree.path().into());
        backend.explicit_files = vec!["contract.rs".into()];
        let backend = builder.build(backend).unwrap();
        assert!(backend.render().contains("worktree version"));

        let mut architect = request(temp.path(), &store, AgentFunction::SoftwareArchitecture);
        architect.worktree = Some(worktree.path().into());
        let architect = builder.build(architect).unwrap();
        assert!(architect
            .items
            .iter()
            .any(|item| item.record.source_id == "README.md"));
        assert!(!architect
            .items
            .iter()
            .any(|item| item.record.source_id == "src/runtime/api.rs"));
    }

    #[test]
    fn accepted_adr_is_context_and_superseded_adr_is_only_an_exclusion() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".batai")).unwrap();
        fs::create_dir_all(temp.path().join("docs/adr")).unwrap();
        fs::write(
            temp.path().join("docs/adr/ADR-001.md"),
            "# APPROVED authentication architecture\nUse typed sessions.",
        )
        .unwrap();
        fs::write(
            temp.path().join("docs/adr/ADR-002.md"),
            "# SUPERSEDED authentication architecture\nUse global tokens.",
        )
        .unwrap();
        let store = RuntimeStore::open(temp.path().join("runtime.sqlite")).unwrap();
        let builder = ContextBuilder::new(temp.path().into(), store.clone());
        let package = builder
            .build(request(
                temp.path(),
                &store,
                AgentFunction::SoftwareArchitecture,
            ))
            .unwrap();
        assert!(package
            .items
            .iter()
            .any(|item| item.record.source_id.ends_with("ADR-001.md")));
        assert!(!package.render().contains("global tokens"));
        assert!(package
            .excluded
            .iter()
            .any(|item| item.source_id.ends_with("ADR-002.md")));
    }
}
