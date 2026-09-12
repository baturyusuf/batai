use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{
    economic::{role_recommendations, CapabilityEvidence, CapabilityEvidenceSource},
    hardware::HardwareProfile,
    organization::{AgentFunction, CapabilityProfile, CapabilityScore, Seniority},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HardwareFit {
    Excellent,
    Good,
    PartialOffload,
    Poor,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelCatalogEntry {
    pub id: String,
    pub display_name: String,
    pub family: String,
    pub parameter_scale: String,
    pub quantization: Option<String>,
    pub approximate_disk_bytes: u64,
    pub approximate_vram_bytes: u64,
    pub context_tokens: u64,
    pub strengths: Vec<String>,
    pub intended_functions: Vec<AgentFunction>,
    pub runtime: String,
    pub estimate_notice: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFitAssessment {
    pub model_id: String,
    pub fit: HardwareFit,
    pub estimated: bool,
    pub reasons: Vec<String>,
    pub safety_margin_bytes: u64,
}

pub fn curated_catalog() -> Vec<LocalModelCatalogEntry> {
    const GIB: u64 = 1024 * 1024 * 1024;
    vec![
        LocalModelCatalogEntry {
            id: "qwen2.5-coder:3b-instruct-q4_K_M".into(), display_name: "Qwen2.5 Coder 3B".into(), family: "Qwen2.5 Coder".into(),
            parameter_scale: "3B".into(), quantization: Some("Q4_K_M".into()), approximate_disk_bytes: 2 * GIB,
            approximate_vram_bytes: 3 * GIB, context_tokens: 32_768, strengths: vec!["coding".into(), "structured output".into()],
            intended_functions: vec![AgentFunction::SoftwareEngineer, AgentFunction::AutomationEngineering], runtime: "ollama".into(),
            estimate_notice: "Memory requirements are conservative estimates; benchmark on this hardware before assignment.".into(),
        },
        LocalModelCatalogEntry {
            id: "qwen2.5-coder:7b-instruct-q4_K_M".into(), display_name: "Qwen2.5 Coder 7B".into(), family: "Qwen2.5 Coder".into(),
            parameter_scale: "7B".into(), quantization: Some("Q4_K_M".into()), approximate_disk_bytes: 5 * GIB,
            approximate_vram_bytes: 6 * GIB, context_tokens: 32_768, strengths: vec!["coding".into(), "debugging".into()],
            intended_functions: vec![AgentFunction::SoftwareEngineer, AgentFunction::BackendEngineering, AgentFunction::QaEngineering], runtime: "ollama".into(),
            estimate_notice: "Memory requirements are conservative estimates; benchmark on this hardware before assignment.".into(),
        },
        LocalModelCatalogEntry {
            id: "qwen3:8b-q4_K_M".into(), display_name: "Qwen3 8B".into(), family: "Qwen3".into(), parameter_scale: "8B".into(),
            quantization: Some("Q4_K_M".into()), approximate_disk_bytes: 6 * GIB, approximate_vram_bytes: 7 * GIB,
            context_tokens: 40_960, strengths: vec!["planning".into(), "instruction following".into()],
            intended_functions: vec![AgentFunction::ProductAnalyst, AgentFunction::ResearchAnalyst, AgentFunction::SoftwareEngineer], runtime: "ollama".into(),
            estimate_notice: "Memory requirements are conservative estimates; benchmark on this hardware before assignment.".into(),
        },
    ]
}

pub fn assess_fit(
    model: &LocalModelCatalogEntry,
    hardware: &HardwareProfile,
    safety_margin_percent: u8,
) -> ModelFitAssessment {
    let margin = model
        .approximate_vram_bytes
        .saturating_mul(u64::from(safety_margin_percent.min(50)))
        / 100;
    let required = model.approximate_vram_bytes.saturating_add(margin);
    let best_vram = hardware
        .gpus
        .iter()
        .filter_map(|gpu| gpu.vram_total_bytes)
        .max();
    let (fit, reasons) = match best_vram {
        Some(vram) if vram >= required => (
            HardwareFit::Excellent,
            vec!["Estimated model and safety headroom fit in GPU VRAM".into()],
        ),
        Some(vram) if vram >= model.approximate_vram_bytes => (
            HardwareFit::Good,
            vec!["Estimated model fits VRAM, but headroom is limited".into()],
        ),
        Some(_)
            if hardware.memory.total_bytes >= model.approximate_vram_bytes.saturating_mul(2) =>
        {
            (
                HardwareFit::PartialOffload,
                vec![
                    "GPU VRAM is insufficient; system RAM may permit slower partial offload".into(),
                ],
            )
        }
        Some(_) => (
            HardwareFit::Poor,
            vec!["Estimated VRAM and RAM headroom are insufficient".into()],
        ),
        None if hardware.memory.total_bytes >= model.approximate_vram_bytes.saturating_mul(2) => (
            HardwareFit::PartialOffload,
            vec!["No supported GPU detected; CPU/RAM execution may be possible and slower".into()],
        ),
        None => (
            HardwareFit::Unsupported,
            vec!["No supported GPU and insufficient estimated system RAM".into()],
        ),
    };
    ModelFitAssessment {
        model_id: model.id.clone(),
        fit,
        estimated: true,
        reasons,
        safety_margin_bytes: margin,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BenchmarkCategory {
    Coding,
    Debugging,
    Script,
    TestGeneration,
    Planning,
    InstructionFollowing,
    ToolPatch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkCaseResult {
    pub category: BenchmarkCategory,
    pub correctness: f64,
    pub tests_passed: Option<bool>,
    pub structured_output_valid: bool,
    pub retry_count: u32,
    pub latency_ms: u64,
    pub time_to_first_token_ms: Option<u64>,
    pub tokens_per_second: Option<f64>,
    pub total_tokens: Option<u64>,
    pub peak_memory_bytes: Option<u64>,
    pub failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkResult {
    pub id: String,
    pub model_id: String,
    pub hardware_fingerprint: String,
    pub suite_version: u32,
    pub observed_at: String,
    pub cases: Vec<BenchmarkCaseResult>,
    pub capabilities: CapabilityProfile,
    pub evidence: Vec<CapabilityEvidence>,
    pub recommended_roles: Vec<RoleRecommendation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleRecommendation {
    pub function: AgentFunction,
    pub seniority: Seniority,
}

#[async_trait]
pub trait BenchmarkExecutor: Send + Sync {
    async fn execute(&self, model: &str, category: BenchmarkCategory) -> BenchmarkCaseResult;
}

pub async fn run_benchmark(
    executor: &dyn BenchmarkExecutor,
    model: &str,
    hardware_fingerprint: &str,
) -> BenchmarkResult {
    let categories = [
        BenchmarkCategory::Coding,
        BenchmarkCategory::Debugging,
        BenchmarkCategory::Script,
        BenchmarkCategory::TestGeneration,
        BenchmarkCategory::Planning,
        BenchmarkCategory::InstructionFollowing,
        BenchmarkCategory::ToolPatch,
    ];
    let mut cases = Vec::with_capacity(categories.len());
    for category in categories {
        cases.push(executor.execute(model, category).await);
    }
    score_benchmark(model, hardware_fingerprint, cases)
}

fn score_benchmark(
    model: &str,
    hardware_fingerprint: &str,
    cases: Vec<BenchmarkCaseResult>,
) -> BenchmarkResult {
    let score = |categories: &[BenchmarkCategory]| -> Option<CapabilityScore> {
        let selected = cases
            .iter()
            .filter(|case| categories.contains(&case.category))
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return None;
        }
        let value = selected
            .iter()
            .map(|case| {
                let correctness = case.correctness.clamp(0.0, 1.0) * 70.0;
                let structure = if case.structured_output_valid {
                    15.0
                } else {
                    0.0
                };
                let tests = match case.tests_passed {
                    Some(true) => 15.0,
                    Some(false) => 0.0,
                    None => 7.5,
                };
                (correctness + structure + tests - f64::from(case.retry_count) * 5.0)
                    .clamp(0.0, 100.0)
            })
            .sum::<f64>()
            / selected.len() as f64;
        CapabilityScore::new(value.round() as u8).ok()
    };
    let speed_value = if cases.iter().any(|case| case.latency_ms > 0) {
        let average = cases.iter().map(|case| case.latency_ms).sum::<u64>() / cases.len() as u64;
        Some(CapabilityScore::new((100u64.saturating_sub(average / 200)).min(100) as u8).unwrap())
    } else {
        None
    };
    let reliability_value = Some(
        CapabilityScore::new(
            ((cases.iter().filter(|case| case.failure.is_none()).count() * 100) / cases.len())
                as u8,
        )
        .unwrap(),
    );
    let capabilities = CapabilityProfile {
        coding: score(&[BenchmarkCategory::Coding, BenchmarkCategory::Script]),
        debugging: score(&[BenchmarkCategory::Debugging]),
        planning: score(&[BenchmarkCategory::Planning]),
        architecture: None,
        tool_use: score(&[BenchmarkCategory::ToolPatch]),
        instruction_following: score(&[BenchmarkCategory::InstructionFollowing]),
        long_context: None,
        test_generation: score(&[BenchmarkCategory::TestGeneration]),
        review: score(&[BenchmarkCategory::Debugging]),
        research: None,
        speed: speed_value,
        reliability: reliability_value,
    };
    let observed_at = chrono::Utc::now().to_rfc3339();
    let dimensions = [
        ("coding", capabilities.coding),
        ("debugging", capabilities.debugging),
        ("planning", capabilities.planning),
        ("tool_use", capabilities.tool_use),
        ("instruction_following", capabilities.instruction_following),
        ("test_generation", capabilities.test_generation),
        ("speed", capabilities.speed),
        ("reliability", capabilities.reliability),
    ];
    let evidence = dimensions
        .into_iter()
        .filter_map(|(dimension, score)| {
            score.map(|score| CapabilityEvidence {
                dimension: dimension.into(),
                score,
                source: CapabilityEvidenceSource::BataiBenchmark,
                model: Some(model.into()),
                observed_at: observed_at.clone(),
                hardware_fingerprint: Some(hardware_fingerprint.into()),
                sample_count: cases.len() as u32,
                note: Some("Batai local deterministic suite v1".into()),
            })
        })
        .collect();
    let recommended_roles = role_recommendations(&capabilities)
        .into_iter()
        .map(|(function, seniority)| RoleRecommendation {
            function,
            seniority,
        })
        .collect();
    BenchmarkResult {
        id: format!("BENCH-{}", uuid::Uuid::new_v4()),
        model_id: model.into(),
        hardware_fingerprint: hardware_fingerprint.into(),
        suite_version: 1,
        observed_at,
        cases,
        capabilities,
        evidence,
        recommended_roles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        quality: f64,
        malformed: bool,
        timeout: bool,
        slow: bool,
    }
    #[async_trait]
    impl BenchmarkExecutor for Fake {
        async fn execute(&self, _model: &str, category: BenchmarkCategory) -> BenchmarkCaseResult {
            BenchmarkCaseResult {
                category,
                correctness: if self.timeout { 0.0 } else { self.quality },
                tests_passed: Some(!self.timeout && self.quality > 0.7),
                structured_output_valid: !self.malformed && !self.timeout,
                retry_count: u32::from(self.malformed),
                latency_ms: if self.slow { 30_000 } else { 500 },
                time_to_first_token_ms: Some(100),
                tokens_per_second: Some(20.0),
                total_tokens: Some(50),
                peak_memory_bytes: None,
                failure: self.timeout.then(|| "timeout".into()),
            }
        }
    }

    #[tokio::test]
    async fn deterministic_results_cover_pass_partial_malformed_timeout_and_slow() {
        let all_pass = run_benchmark(
            &Fake {
                quality: 1.0,
                malformed: false,
                timeout: false,
                slow: false,
            },
            "m",
            "h",
        )
        .await;
        assert_eq!(all_pass.capabilities.coding.unwrap().get(), 100);
        let partial = run_benchmark(
            &Fake {
                quality: 0.5,
                malformed: false,
                timeout: false,
                slow: false,
            },
            "m",
            "h",
        )
        .await;
        assert!(
            partial.capabilities.coding.unwrap().get()
                < all_pass.capabilities.coding.unwrap().get()
        );
        let malformed = run_benchmark(
            &Fake {
                quality: 1.0,
                malformed: true,
                timeout: false,
                slow: false,
            },
            "m",
            "h",
        )
        .await;
        assert!(malformed.capabilities.coding.unwrap().get() < 100);
        let timeout = run_benchmark(
            &Fake {
                quality: 1.0,
                malformed: false,
                timeout: true,
                slow: false,
            },
            "m",
            "h",
        )
        .await;
        assert_eq!(timeout.capabilities.reliability.unwrap().get(), 0);
        let slow = run_benchmark(
            &Fake {
                quality: 1.0,
                malformed: false,
                timeout: false,
                slow: true,
            },
            "m",
            "h",
        )
        .await;
        assert!(
            slow.capabilities.speed.unwrap().get() < all_pass.capabilities.speed.unwrap().get()
        );
    }

    #[test]
    fn hardware_fit_leaves_headroom_and_handles_no_gpu() {
        let model = &curated_catalog()[0];
        let profile = HardwareProfile {
            cpu: super::super::hardware::CpuProfile {
                name: None,
                logical_cores: 4,
                physical_cores: None,
            },
            memory: super::super::hardware::MemoryProfile {
                total_bytes: 16 * 1024 * 1024 * 1024,
                available_bytes: 8 * 1024 * 1024 * 1024,
            },
            gpus: vec![],
            operating_system: "test".into(),
            fingerprint: "x".into(),
            available_disk_bytes: None,
            observed_at: "now".into(),
        };
        let fit = assess_fit(model, &profile, 20);
        assert_eq!(fit.fit, HardwareFit::PartialOffload);
        assert!(fit.safety_margin_bytes > 0);
    }
}
