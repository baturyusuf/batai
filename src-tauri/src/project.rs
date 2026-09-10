use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::domain::{
    task_progress, weighted_progress, AgentView, AppSnapshot, MessageReceipt, ProjectSummary,
    TaskView,
};

pub struct ProjectStore {
    root: PathBuf,
}

pub fn discover_project_root(start: PathBuf) -> PathBuf {
    start
        .ancestors()
        .find(|candidate| candidate.join(".batai").is_dir())
        .map(Path::to_path_buf)
        .unwrap_or(start)
}

impl ProjectStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn snapshot(&self) -> io::Result<AppSnapshot> {
        let agents = load_agents(&self.root.join(".batai/agents"))?;
        let tasks = load_tasks(&self.root.join(".batai/tasks"))?;
        let name = self
            .root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Batai Project")
            .to_string();
        let summary = ProjectSummary {
            name,
            root: self.root.display().to_string(),
            progress: weighted_progress(&tasks),
            active_agents: agents
                .iter()
                .filter(|agent| matches!(agent.status.as_str(), "RUNNING" | "READY"))
                .count(),
            blocked_tasks: tasks.iter().filter(|task| task.status == "BLOCKED").count(),
            review_tasks: tasks.iter().filter(|task| task.status == "REVIEW").count(),
        };
        Ok(AppSnapshot {
            project: summary,
            agents,
            tasks,
        })
    }

    pub fn send_director_message(&self, content: &str) -> io::Result<MessageReceipt> {
        let content = content.trim();
        if content.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Message cannot be empty",
            ));
        }
        let now = chrono::Utc::now();
        let timestamp = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let id = format!("MSG-rust-{}", now.timestamp_millis());
        let directory = self.root.join(".batai/inbox/director");
        fs::create_dir_all(&directory)?;
        let file_stamp = timestamp.replace(':', "-");
        let file = directory.join(format!("{file_stamp}-{id}.json"));
        let temporary = file.with_extension("json.tmp");
        let message = serde_json::json!({
            "id": id,
            "source": "GOD",
            "target": "director",
            "authority": 100,
            "scope": "project",
            "content": content,
            "status": "PENDING",
            "timestamp": timestamp,
            "metadata": { "client": "batai-rust-desktop" }
        });
        fs::write(
            &temporary,
            format!("{}\n", serde_json::to_string_pretty(&message)?),
        )?;
        fs::rename(temporary, file)?;
        Ok(MessageReceipt { id, timestamp })
    }
}

fn json_files(directory: &Path) -> io::Result<Vec<PathBuf>> {
    if !directory.exists() {
        return Ok(vec![]);
    }
    let mut files = vec![];
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let config = path.join("config.json");
            if config.is_file() {
                files.push(config);
            }
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn read_json(path: &Path) -> io::Result<Value> {
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {error}", path.display()),
        )
    })
}

fn text(value: &Value, key: &str, fallback: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

fn optional_text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn string_list(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
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
    .to_string()
}

fn load_agents(directory: &Path) -> io::Result<Vec<AgentView>> {
    let mut agents = vec![];
    for file in json_files(directory)? {
        let value = read_json(&file)?;
        let title = text(&value, "role_template", "Agent");
        agents.push(AgentView {
            id: text(&value, "id", "unknown"),
            name: text(&value, "name", "Unnamed agent"),
            department: department_for(&title),
            title,
            reports_to: optional_text(&value, "parent_agent_id"),
            provider: text(&value, "provider", "unassigned"),
            model: text(&value, "model", "auto"),
            auth_mode: text(&value, "auth_mode", "unknown"),
            status: text(&value, "status", "CREATED"),
            current_task_id: optional_text(&value, "current_task_id"),
            worktree: optional_text(&value, "worktree"),
        });
    }
    agents.sort_by_key(|agent| if agent.id == "director" { 0 } else { 1 });
    Ok(agents)
}

fn load_tasks(directory: &Path) -> io::Result<Vec<TaskView>> {
    let mut tasks = vec![];
    for file in json_files(directory)? {
        let value = read_json(&file)?;
        let status = text(&value, "status", "PENDING");
        tasks.push(TaskView {
            id: text(&value, "id", "unknown"),
            objective: text(&value, "objective", "No objective"),
            assigned_to: string_list(&value, "assigned_to"),
            dependencies: string_list(&value, "dependencies"),
            weight: value.get("weight").and_then(Value::as_f64).unwrap_or(1.0),
            progress: value
                .get("progress")
                .and_then(Value::as_f64)
                .unwrap_or_else(|| task_progress(&status)),
            status,
        });
    }
    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_project() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("batai-rust-{suffix}"));
        fs::create_dir_all(root.join(".batai/agents/director")).expect("agent directory");
        fs::create_dir_all(root.join(".batai/tasks")).expect("task directory");
        root
    }

    #[test]
    fn snapshot_reads_existing_repository_contract() {
        let root = temporary_project();
        fs::write(
            root.join(".batai/agents/director/config.json"),
            r#"{"id":"director","name":"Director","role_template":"Director","provider":"codex","model":"gpt","auth_mode":"subscription","status":"READY"}"#,
        )
        .expect("agent fixture");
        fs::write(
            root.join(".batai/tasks/foundation.json"),
            r#"{"id":"foundation","objective":"Build foundation","status":"COMPLETED","assigned_to":["director"],"dependencies":[],"weight":4}"#,
        )
        .expect("task fixture");

        let snapshot = ProjectStore::new(root.clone())
            .snapshot()
            .expect("snapshot");
        assert_eq!(snapshot.agents.len(), 1);
        assert_eq!(snapshot.tasks.len(), 1);
        assert_eq!(snapshot.project.progress, 100.0);
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn director_message_uses_existing_authority_contract() {
        let root = temporary_project();
        let store = ProjectStore::new(root.clone());
        let receipt = store
            .send_director_message("Review the release")
            .expect("message");
        let message_path = fs::read_dir(root.join(".batai/inbox/director"))
            .expect("inbox")
            .next()
            .expect("message entry")
            .expect("message file")
            .path();
        let message = read_json(&message_path).expect("message json");

        assert_eq!(message["id"], receipt.id);
        assert_eq!(message["authority"], 100);
        assert_eq!(message["content"], "Review the release");
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn project_discovery_walks_up_from_build_directory() {
        let root = temporary_project();
        let nested = root.join("src-tauri/target/debug");
        fs::create_dir_all(&nested).expect("nested directory");
        assert_eq!(discover_project_root(nested), root);
        fs::remove_dir_all(root).expect("remove fixture");
    }
}
