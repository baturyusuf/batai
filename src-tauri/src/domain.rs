use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    pub id: String,
    pub name: String,
    pub title: String,
    pub department: String,
    pub reports_to: Option<String>,
    pub provider: String,
    pub model: String,
    pub auth_mode: String,
    pub status: String,
    pub current_task_id: Option<String>,
    pub worktree: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    pub id: String,
    pub objective: String,
    pub status: String,
    pub assigned_to: Vec<String>,
    pub dependencies: Vec<String>,
    pub weight: f64,
    pub progress: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub name: String,
    pub root: String,
    pub progress: f64,
    pub active_agents: usize,
    pub blocked_tasks: usize,
    pub review_tasks: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub project: ProjectSummary,
    pub agents: Vec<AgentView>,
    pub tasks: Vec<TaskView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnection {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub status: String,
    pub status_label: String,
    pub account_label: String,
    pub detail: String,
    pub action_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionGuide {
    pub provider_id: String,
    pub title: String,
    pub description: String,
    pub command: Option<String>,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MessageReceipt {
    pub id: String,
    pub timestamp: String,
}

pub fn task_progress(status: &str) -> f64 {
    match status {
        "COMPLETED" => 1.0,
        "REVIEW" => 0.9,
        "RUNNING" => 0.55,
        "WAITING_RESOURCE" => 0.4,
        "BLOCKED" => 0.25,
        _ => 0.0,
    }
}

pub fn weighted_progress(tasks: &[TaskView]) -> f64 {
    let total_weight: f64 = tasks.iter().map(|task| task.weight.max(0.0)).sum();
    if total_weight == 0.0 {
        return 0.0;
    }
    let earned: f64 = tasks
        .iter()
        .map(|task| task.weight.max(0.0) * task.progress.clamp(0.0, 1.0))
        .sum();
    ((earned / total_weight) * 1000.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_uses_task_weights_instead_of_raw_task_count() {
        let tasks = vec![
            TaskView {
                id: "foundation".into(),
                objective: "Foundation".into(),
                status: "COMPLETED".into(),
                assigned_to: vec![],
                dependencies: vec![],
                weight: 8.0,
                progress: 1.0,
            },
            TaskView {
                id: "polish".into(),
                objective: "Polish".into(),
                status: "READY".into(),
                assigned_to: vec![],
                dependencies: vec![],
                weight: 2.0,
                progress: 0.0,
            },
        ];
        assert_eq!(weighted_progress(&tasks), 80.0);
    }

    #[test]
    fn task_status_maps_to_conservative_progress() {
        assert_eq!(task_progress("COMPLETED"), 1.0);
        assert_eq!(task_progress("RUNNING"), 0.55);
        assert_eq!(task_progress("READY"), 0.0);
    }
}
