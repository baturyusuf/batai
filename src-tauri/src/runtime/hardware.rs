use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sysinfo::{Disks, System};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuProfile {
    pub name: Option<String>,
    pub logical_cores: usize,
    pub physical_cores: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProfile {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuProfile {
    pub vendor: String,
    pub name: String,
    pub vram_total_bytes: Option<u64>,
    pub vram_available_bytes: Option<u64>,
    pub cuda_available: bool,
    pub driver_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProfile {
    pub cpu: CpuProfile,
    pub memory: MemoryProfile,
    pub gpus: Vec<GpuProfile>,
    pub operating_system: String,
    pub fingerprint: String,
    pub available_disk_bytes: Option<u64>,
    pub observed_at: String,
}

#[derive(Debug, Default, Clone)]
pub struct HardwareProfiler;

impl HardwareProfiler {
    pub fn detect(&self) -> HardwareProfile {
        let mut system = System::new_all();
        system.refresh_all();
        let cpu = CpuProfile {
            name: system
                .cpus()
                .first()
                .map(|cpu| cpu.brand().trim().to_owned())
                .filter(|name| !name.is_empty()),
            logical_cores: system.cpus().len(),
            physical_cores: system.physical_core_count(),
        };
        let memory = MemoryProfile {
            total_bytes: system.total_memory(),
            available_bytes: system.available_memory(),
        };
        let gpus = nvidia_profiles();
        let operating_system = format!(
            "{} {}",
            System::name().unwrap_or_else(|| std::env::consts::OS.into()),
            System::os_version().unwrap_or_default()
        )
        .trim()
        .to_owned();
        let disks = Disks::new_with_refreshed_list();
        let current = std::env::current_dir().ok();
        let available_disk_bytes = current
            .as_deref()
            .and_then(|path| {
                disks
                    .iter()
                    .filter(|disk| path.starts_with(disk.mount_point()))
                    .max_by_key(|disk| disk.mount_point().as_os_str().len())
                    .map(|disk| disk.available_space())
            })
            .or_else(|| disks.iter().map(|disk| disk.available_space()).max());
        let fingerprint = fingerprint(&cpu, &memory, &gpus, &operating_system);
        HardwareProfile {
            cpu,
            memory,
            gpus,
            operating_system,
            fingerprint,
            available_disk_bytes,
            observed_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

fn nvidia_profiles() -> Vec<GpuProfile> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,memory.free,driver_version",
            "--format=csv,noheader,nounits",
        ])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            parse_nvidia_smi(&String::from_utf8_lossy(&output.stdout))
        }
        _ => Vec::new(),
    }
}

fn parse_nvidia_smi(output: &str) -> Vec<GpuProfile> {
    output
        .lines()
        .filter_map(|line| {
            let columns = line.split(',').map(str::trim).collect::<Vec<_>>();
            if columns.len() != 4 || columns[0].is_empty() {
                return None;
            }
            Some(GpuProfile {
                vendor: "NVIDIA".into(),
                name: columns[0].into(),
                vram_total_bytes: mib(columns[1]),
                vram_available_bytes: mib(columns[2]),
                cuda_available: true,
                driver_version: (!columns[3].is_empty()).then(|| columns[3].into()),
            })
        })
        .collect()
}

fn mib(value: &str) -> Option<u64> {
    value
        .parse::<u64>()
        .ok()
        .map(|value| value.saturating_mul(1024 * 1024))
}

fn fingerprint(cpu: &CpuProfile, memory: &MemoryProfile, gpus: &[GpuProfile], os: &str) -> String {
    let static_identity = serde_json::json!({
        "cpu": cpu.name,
        "logicalCores": cpu.logical_cores,
        "physicalCores": cpu.physical_cores,
        "totalMemory": memory.total_bytes,
        "gpus": gpus.iter().map(|gpu| serde_json::json!({"vendor":gpu.vendor,"name":gpu.name,"vram":gpu.vram_total_bytes,"driver":gpu.driver_version})).collect::<Vec<_>>(),
        "os": os,
    });
    format!("{:x}", Sha256::digest(static_identity.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nvidia_structured_query_and_dynamic_free_vram() {
        let first = parse_nvidia_smi("NVIDIA RTX 4060 Laptop GPU, 8188, 6123, 555.99\n");
        let second = parse_nvidia_smi("NVIDIA RTX 4060 Laptop GPU, 8188, 2048, 555.99\n");
        assert_eq!(first[0].vram_total_bytes, second[0].vram_total_bytes);
        assert_ne!(
            first[0].vram_available_bytes,
            second[0].vram_available_bytes
        );
        assert_eq!(first[0].driver_version.as_deref(), Some("555.99"));
    }

    #[test]
    fn malformed_and_unsupported_gpu_output_is_safe() {
        assert!(parse_nvidia_smi("not, enough").is_empty());
        assert!(parse_nvidia_smi("").is_empty());
    }

    #[test]
    fn fingerprint_ignores_dynamic_free_memory() {
        let cpu = CpuProfile {
            name: Some("CPU".into()),
            logical_cores: 8,
            physical_cores: Some(4),
        };
        let mut gpu = GpuProfile {
            vendor: "NVIDIA".into(),
            name: "GPU".into(),
            vram_total_bytes: Some(8),
            vram_available_bytes: Some(7),
            cuda_available: true,
            driver_version: Some("1".into()),
        };
        let memory = MemoryProfile {
            total_bytes: 32,
            available_bytes: 20,
        };
        let first = fingerprint(&cpu, &memory, &[gpu.clone()], "Windows");
        gpu.vram_available_bytes = Some(1);
        let second = fingerprint(
            &cpu,
            &MemoryProfile {
                available_bytes: 1,
                ..memory
            },
            &[gpu],
            "Windows",
        );
        assert_eq!(first, second);
    }

    #[test]
    fn low_ram_profile_remains_representable() {
        let profile = MemoryProfile {
            total_bytes: 2 * 1024 * 1024 * 1024,
            available_bytes: 256 * 1024 * 1024,
        };
        assert!(profile.available_bytes < profile.total_bytes);
    }
}
