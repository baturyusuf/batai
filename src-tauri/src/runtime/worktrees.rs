use super::errors::{Result, RuntimeError};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorktreeBinding {
    pub path: PathBuf,
    pub branch: String,
    pub base_commit: String,
    pub starting_head: String,
    pub reused: bool,
}

#[derive(Debug, Clone)]
pub struct WorktreeManager {
    repository: PathBuf,
    root: PathBuf,
}

impl WorktreeManager {
    pub fn discover(repository: impl AsRef<Path>) -> Result<Self> {
        let repository = run_git(repository.as_ref(), &["rev-parse", "--show-toplevel"])?;
        let repository = friendly_path(PathBuf::from(repository.trim()).canonicalize()?);
        let name = repository
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("repository");
        let parent = repository
            .parent()
            .ok_or_else(|| RuntimeError::Provider("repository has no parent".into()))?;
        Ok(Self {
            root: parent.join(".batai-worktrees").join(name),
            repository,
        })
    }

    pub fn ensure(&self, agent_id: &str, task_id: &str) -> Result<WorktreeBinding> {
        self.ensure_from(agent_id, task_id, "HEAD")
    }

    pub fn ensure_from(
        &self,
        agent_id: &str,
        task_id: &str,
        base_ref: &str,
    ) -> Result<WorktreeBinding> {
        let agent = safe_component(agent_id)?;
        let task = safe_component(task_id)?;
        if base_ref.trim().is_empty() || base_ref.starts_with('-') {
            return Err(RuntimeError::Provider("unsafe base ref".into()));
        }
        std::fs::create_dir_all(&self.root)?;
        let path = self.root.join(format!("{agent}-{task}"));
        let branch = format!("batai/{agent}/{task}");
        let base_commit = run_git(&self.repository, &["rev-parse", base_ref])?
            .trim()
            .to_owned();
        if path.exists() {
            let actual = path.canonicalize()?;
            let root = self.root.canonicalize()?;
            if !actual.starts_with(&root) {
                return Err(RuntimeError::Provider(
                    "worktree escaped managed root".into(),
                ));
            }
            let actual_branch = run_git(&actual, &["branch", "--show-current"])?
                .trim()
                .to_owned();
            if actual_branch != branch {
                return Err(RuntimeError::Governance(format!(
                    "managed worktree branch mismatch: expected {branch}, found {actual_branch}"
                )));
            }
            let starting_head = run_git(&path, &["rev-parse", "HEAD"])?.trim().to_owned();
            return Ok(WorktreeBinding {
                path: actual,
                branch,
                base_commit,
                starting_head,
                reused: true,
            });
        }
        let exists = Command::new("git")
            .current_dir(&self.repository)
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ])
            .status()?
            .success();
        let path_text = path.to_string_lossy().into_owned();
        if exists {
            checked(
                Command::new("git")
                    .current_dir(&self.repository)
                    .args(["worktree", "add", &path_text, &branch])
                    .output()?,
            )?;
        } else {
            checked(
                Command::new("git")
                    .current_dir(&self.repository)
                    .args(["worktree", "add", "-b", &branch, &path_text, base_ref])
                    .output()?,
            )?;
        }
        let actual = path.canonicalize()?;
        let starting_head = run_git(&actual, &["rev-parse", "HEAD"])?.trim().to_owned();
        Ok(WorktreeBinding {
            path: actual,
            branch,
            base_commit,
            starting_head,
            reused: false,
        })
    }

    pub fn status(&self, binding: &WorktreeBinding) -> Result<Vec<String>> {
        Ok(run_git(&binding.path, &["status", "--porcelain=v1"])?
            .lines()
            .map(str::to_owned)
            .collect())
    }

    pub fn validate_context_worktree(
        &self,
        path: &Path,
        agent_id: &str,
        task_id: Option<&str>,
    ) -> Result<()> {
        let actual = path.canonicalize().map_err(|_| {
            RuntimeError::Governance(format!(
                "managed context worktree is missing: {}",
                path.display()
            ))
        })?;
        let repository = self.repository.canonicalize()?;
        if actual == repository {
            return Err(RuntimeError::Governance(
                "default repository root cannot be used as an isolated agent worktree".into(),
            ));
        }
        let managed_root = self
            .root
            .canonicalize()
            .map_err(|_| RuntimeError::Governance("managed worktree root is missing".into()))?;
        if !actual.starts_with(&managed_root) {
            return Err(RuntimeError::Governance(
                "context worktree is not under the Batai-managed worktree root".into(),
            ));
        }
        let common_dir = run_git(&actual, &["rev-parse", "--git-common-dir"])?
            .trim()
            .to_owned();
        let common_dir = PathBuf::from(common_dir);
        let common_dir = if common_dir.is_absolute() {
            common_dir
        } else {
            actual.join(common_dir)
        }
        .canonicalize()?;
        let repository_git_dir = repository.join(".git").canonicalize()?;
        if common_dir != repository_git_dir {
            return Err(RuntimeError::Governance(
                "context worktree belongs to a different repository".into(),
            ));
        }
        let branch = run_git(&actual, &["branch", "--show-current"])?
            .trim()
            .to_owned();
        let agent = safe_component(agent_id)?;
        let prefix = format!("batai/{agent}/");
        if !branch.starts_with(&prefix) {
            return Err(RuntimeError::Governance(format!(
                "context worktree branch is not managed for agent {agent_id}: {branch}"
            )));
        }
        if let Some(task_id) = task_id {
            let expected = format!("batai/{agent}/{}", safe_component(task_id)?);
            if branch != expected {
                return Err(RuntimeError::Governance(format!(
                    "context worktree branch mismatch: expected {expected}, found {branch}"
                )));
            }
        }
        Ok(())
    }
    pub fn diff(&self, binding: &WorktreeBinding) -> Result<String> {
        run_git(&binding.path, &["diff", "--binary", "HEAD"])
    }
    pub fn validate_commit_files(
        &self,
        binding: &WorktreeBinding,
        max_file_bytes: u64,
    ) -> Result<()> {
        let output = checked(
            Command::new("git")
                .current_dir(&binding.path)
                .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
                .output()?,
        )?;
        let root = binding.path.canonicalize()?;
        let mut skip_rename_source = false;
        for entry in output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
        {
            if skip_rename_source {
                skip_rename_source = false;
                continue;
            }
            if entry.len() < 4 {
                continue;
            }
            let status = &entry[..2];
            let relative = std::str::from_utf8(&entry[3..])
                .map_err(|_| RuntimeError::Provider("Git path is not valid UTF-8".into()))?;
            if matches!(status[0], b'R' | b'C') || matches!(status[1], b'R' | b'C') {
                skip_rename_source = true;
            }
            let candidate = binding.path.join(relative);
            if !candidate.exists() {
                continue;
            }
            let actual = candidate.canonicalize()?;
            if !actual.starts_with(&root) {
                return Err(RuntimeError::Provider(
                    "changed file escapes the managed worktree".into(),
                ));
            }
            let metadata = std::fs::metadata(&actual)?;
            if metadata.is_file() && metadata.len() > max_file_bytes {
                return Err(RuntimeError::Provider(format!(
                    "changed file exceeds safe automatic commit limit: {relative}"
                )));
            }
        }
        Ok(())
    }
    pub fn head(&self, binding: &WorktreeBinding) -> Result<String> {
        Ok(run_git(&binding.path, &["rev-parse", "HEAD"])?
            .trim()
            .into())
    }
    pub fn commit(&self, binding: &WorktreeBinding, message: &str) -> Result<Option<String>> {
        if self.status(binding)?.is_empty() {
            return Ok(None);
        }
        checked(
            Command::new("git")
                .current_dir(&binding.path)
                .args(["add", "--all"])
                .output()?,
        )?;
        checked(
            Command::new("git")
                .current_dir(&binding.path)
                .args(["commit", "-m", message])
                .output()?,
        )?;
        Ok(Some(self.head(binding)?))
    }
    pub fn remove(&self, binding: &WorktreeBinding) -> Result<()> {
        let root = self.root.canonicalize()?;
        let target = binding.path.canonicalize()?;
        if !target.starts_with(&root) || target == root {
            return Err(RuntimeError::Provider(
                "refusing to remove unmanaged worktree".into(),
            ));
        }
        let target_text = target.to_string_lossy().into_owned();
        checked(
            Command::new("git")
                .current_dir(&self.repository)
                .args(["worktree", "remove", &target_text])
                .output()?,
        )?;
        Ok(())
    }
    pub fn prune(&self) -> Result<()> {
        checked(
            Command::new("git")
                .current_dir(&self.repository)
                .args(["worktree", "prune"])
                .output()?,
        )?;
        Ok(())
    }
}

pub fn role_requires_worktree(role: &str) -> bool {
    let role = role.to_ascii_lowercase();
    [
        "engineer",
        "developer",
        "frontend",
        "backend",
        "software",
        "qa",
        "test",
        "review",
    ]
    .iter()
    .any(|term| role.contains(term))
}

fn safe_component(value: &str) -> Result<String> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(RuntimeError::Provider(format!(
            "unsafe worktree identifier: {value}"
        )));
    }
    Ok(value.to_ascii_lowercase())
}

fn friendly_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{}", rest));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path
}
fn run_git(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(cwd).args(args).output()?;
    let output = checked(output)?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
fn checked(output: Output) -> Result<Output> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(RuntimeError::Provider(format!(
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        checked(
            Command::new("git")
                .current_dir(dir.path())
                .args(["init"])
                .output()
                .unwrap(),
        )
        .unwrap();
        checked(
            Command::new("git")
                .current_dir(dir.path())
                .args(["config", "user.email", "batai@example.test"])
                .output()
                .unwrap(),
        )
        .unwrap();
        checked(
            Command::new("git")
                .current_dir(dir.path())
                .args(["config", "user.name", "Batai Test"])
                .output()
                .unwrap(),
        )
        .unwrap();
        std::fs::write(dir.path().join("README.md"), "test").unwrap();
        checked(
            Command::new("git")
                .current_dir(dir.path())
                .args(["add", "README.md"])
                .output()
                .unwrap(),
        )
        .unwrap();
        checked(
            Command::new("git")
                .current_dir(dir.path())
                .args(["commit", "-m", "initial"])
                .output()
                .unwrap(),
        )
        .unwrap();
        dir
    }
    #[test]
    fn creates_and_reuses_controlled_worktree() {
        let repo = repo();
        let manager = WorktreeManager::discover(repo.path()).unwrap();
        let first = manager.ensure("coder", "TASK-1").unwrap();
        let second = manager.ensure("coder", "TASK-1").unwrap();
        assert_eq!(first.path, second.path);
        assert_eq!(first.branch, "batai/coder/task-1");
        assert!(first.path.exists());
        assert!(!first.reused);
        assert!(second.reused);
    }
    #[test]
    fn rejects_path_traversal_and_non_coding_roles() {
        let repo = repo();
        let manager = WorktreeManager::discover(repo.path()).unwrap();
        assert!(manager.ensure("../outside", "task").is_err());
        assert!(!role_requires_worktree("Product Analyst"));
        assert!(role_requires_worktree("Software Engineer"));
    }

    #[test]
    fn reused_worktree_must_still_be_on_its_task_branch() {
        let repo = repo();
        let manager = WorktreeManager::discover(repo.path()).unwrap();
        let binding = manager.ensure("coder", "TASK-1").unwrap();
        run_git(&binding.path, &["checkout", "-b", "unexpected"]).unwrap();
        let error = manager.ensure("coder", "TASK-1").unwrap_err();
        assert!(error.to_string().contains("branch mismatch"));
    }

    #[test]
    fn context_validation_requires_managed_repository_worktree() {
        let repo = repo();
        let manager = WorktreeManager::discover(repo.path()).unwrap();
        let binding = manager.ensure("coder", "TASK-1").unwrap();
        manager
            .validate_context_worktree(&binding.path, "coder", Some("TASK-1"))
            .unwrap();
        assert!(manager
            .validate_context_worktree(repo.path(), "coder", Some("TASK-1"))
            .is_err());
        run_git(&binding.path, &["checkout", "-b", "unmanaged"]).unwrap();
        assert!(manager
            .validate_context_worktree(&binding.path, "coder", Some("TASK-1"))
            .is_err());
    }

    #[test]
    fn status_diff_commit_remove_and_prune_are_scoped() {
        let repo = repo();
        let manager = WorktreeManager::discover(repo.path()).unwrap();
        let binding = manager.ensure("coder", "lifecycle").unwrap();
        std::fs::write(binding.path.join("README.md"), "changed").unwrap();
        assert!(!manager.status(&binding).unwrap().is_empty());
        assert!(manager.diff(&binding).unwrap().contains("changed"));
        let commit = manager
            .commit(&binding, "test: worktree lifecycle")
            .unwrap();
        assert!(commit.is_some());
        manager.remove(&binding).unwrap();
        assert!(!binding.path.exists());
        manager.prune().unwrap();
    }
}
