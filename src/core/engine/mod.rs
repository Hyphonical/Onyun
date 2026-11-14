//! `MintEngine` trait, shared telemetry, logarithmic progress estimation, and
//! milestone/snapshot types.

pub(crate) mod cpu;
#[cfg(feature = "gpu")]
pub(crate) mod gpu;
mod helper;
mod simd;
use crate::core::onion::OnionKeypair;
use anyhow::Result;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, mpsc::Sender};
use std::thread::JoinHandle;
use std::time::Instant;

#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "web"), allow(dead_code))]
pub(crate) struct MintConfig {
    pub(crate) target: String,
    pub(crate) threads: usize,
    pub(crate) telemetry_offset: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct MintMilestone {
    pub(crate) matched_len: usize,
    pub(crate) keypair: OnionKeypair,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(not(feature = "web"), allow(dead_code))]
pub(crate) struct MintSnapshot {
    pub(crate) active: bool,
    pub(crate) completed: bool,
    pub(crate) target: String,
    pub(crate) best_address: Option<String>,
    pub(crate) best_len: usize,
    pub(crate) target_len: usize,
    pub(crate) total_hashes: u64,
    pub(crate) hashes_since_best: u64,
    pub(crate) hashes_per_sec: f64,
    pub(crate) elapsed_secs: f64,
    /// Logarithmic + exponential intra-step progress in `[0.0, 1.0]`.
    pub(crate) progress: f64,
    /// Pace ratio relative to expected `32^(best_len + 1)` (`<1.0` lucky, `>1.0` overdue).
    pub(crate) pace_ratio: f64,
    pub(crate) eta_secs: Option<f64>,
}

/// Cache-line padded atomic counter to eliminate false sharing across worker threads.
#[repr(align(64))]
pub(crate) struct PaddedCounter(pub(crate) AtomicU64);

impl Default for PaddedCounter {
    fn default() -> Self {
        Self(AtomicU64::new(0))
    }
}
#[cfg_attr(not(feature = "web"), allow(dead_code))]
pub(crate) struct SharedTelemetry {
    pub(crate) started_at: Instant,
    pub(crate) thread_hashes: Vec<PaddedCounter>,
    pub(crate) global_best_len: AtomicU8,
    pub(crate) hashes_at_last_best: AtomicU64,
}

impl SharedTelemetry {
    pub(crate) fn new(threads: usize, initial_best_len: usize) -> Self {
        let mut thread_hashes = Vec::with_capacity(threads);
        for _ in 0..threads {
            thread_hashes.push(PaddedCounter::default());
        }
        Self {
            started_at: Instant::now(),
            thread_hashes,
            global_best_len: AtomicU8::new(initial_best_len as u8),
            hashes_at_last_best: AtomicU64::new(0),
        }
    }

    pub(crate) fn total_hashes(&self) -> u64 {
        self.thread_hashes
            .iter()
            .map(|c| c.0.load(Ordering::Relaxed))
            .sum()
    }

    pub(crate) fn record_new_best(&self, new_len: usize) {
        self.global_best_len.store(new_len as u8, Ordering::Release);
        self.hashes_at_last_best
            .store(self.total_hashes(), Ordering::Relaxed);
    }
}

pub(crate) struct EngineHandle {
    pub(crate) workers: Vec<JoinHandle<()>>,
}

impl EngineHandle {
    pub(crate) fn join(self) {
        for handle in self.workers {
            let _ = handle.join();
        }
    }
}

#[cfg_attr(not(feature = "web"), allow(dead_code))]
pub(crate) trait MintEngine: Send + Sync {
    fn label(&self) -> &'static str;
    fn device_name(&self) -> String;
    fn slot_count(&self, _cpu_threads: usize) -> usize {
        1
    }
    fn start(
        &self,
        config: MintConfig,
        telemetry: Arc<SharedTelemetry>,
        on_milestone: Sender<MintMilestone>,
        cancel: Arc<AtomicBool>,
    ) -> Result<EngineHandle>;
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(not(feature = "web"), allow(dead_code))]
pub(crate) struct ComputeDevice {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(not(feature = "web"), allow(dead_code))]
pub(crate) enum EngineKind {
    #[default]
    Auto,
    Cpu,
    Gpu,
    Hybrid,
}

impl EngineKind {
    pub(crate) fn parse(s: &str) -> anyhow::Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "cpu" => Ok(Self::Cpu),
            "gpu" => Ok(Self::Gpu),
            "hybrid" | "both" | "all" => Ok(Self::Hybrid),
            other => anyhow::bail!(
                "invalid engine '{other}' (expected `auto`, `cpu`, `gpu`, or `hybrid`)"
            ),
        }
    }

    #[cfg_attr(not(feature = "web"), allow(dead_code))]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
            Self::Hybrid => "hybrid",
        }
    }
}

#[cfg_attr(not(feature = "web"), allow(dead_code))]
pub(crate) fn list_devices() -> Vec<ComputeDevice> {
    #[allow(unused_mut)]
    let mut devices = vec![ComputeDevice {
        id: "cpu".to_string(),
        kind: "cpu".to_string(),
        name: cpu::CpuMkEngine.device_name(),
    }];
    #[cfg(feature = "gpu")]
    {
        let mut gpus: Vec<ComputeDevice> = gpu::enumerate_gpus()
            .into_iter()
            .map(|g| ComputeDevice {
                id: format!("gpu:{}", g.adapter_index),
                kind: "gpu".to_string(),
                name: g.name,
            })
            .collect();
        gpus.extend(devices);
        devices = gpus;
    }
    devices
}

struct CompositeEngine {
    label: &'static str,
    engines: Vec<Box<dyn MintEngine>>,
}

impl MintEngine for CompositeEngine {
    fn label(&self) -> &'static str {
        self.label
    }

    fn device_name(&self) -> String {
        let names: Vec<String> = self.engines.iter().map(|e| e.device_name()).collect();
        names.join(" + ")
    }

    fn slot_count(&self, cpu_threads: usize) -> usize {
        self.engines.iter().map(|e| e.slot_count(cpu_threads)).sum()
    }

    fn start(
        &self,
        config: MintConfig,
        telemetry: Arc<SharedTelemetry>,
        on_milestone: Sender<MintMilestone>,
        cancel: Arc<AtomicBool>,
    ) -> Result<EngineHandle> {
        let mut all_workers = Vec::new();
        let mut offset = config.telemetry_offset;
        for engine in &self.engines {
            let slots = engine.slot_count(config.threads);
            let sub_config = MintConfig {
                target: config.target.clone(),
                threads: config.threads,
                telemetry_offset: offset,
            };
            let handle = engine.start(
                sub_config,
                Arc::clone(&telemetry),
                on_milestone.clone(),
                Arc::clone(&cancel),
            )?;
            all_workers.extend(handle.workers);
            offset += slots;
        }
        Ok(EngineHandle {
            workers: all_workers,
        })
    }
}

pub(crate) fn select_engine(kind: EngineKind) -> anyhow::Result<Box<dyn MintEngine>> {
    select_engine_with_devices(kind, None)
}

#[cfg_attr(not(feature = "web"), allow(dead_code))]
pub(crate) fn select_engine_with_devices(
    kind: EngineKind,
    selected_ids: Option<&[String]>,
) -> anyhow::Result<Box<dyn MintEngine>> {
    let want_id = |id: &str| -> bool {
        match selected_ids {
            None => true,
            Some(ids) => ids.iter().any(|x| x == id),
        }
    };

    #[cfg(feature = "gpu")]
    let mut gpus: Vec<gpu::GpuMkEngine> = gpu::enumerate_gpus()
        .into_iter()
        .filter(|g| want_id(&format!("gpu:{}", g.adapter_index)))
        .collect();
    #[cfg(not(feature = "gpu"))]
    let gpus: Vec<Box<dyn MintEngine>> = Vec::new();

    match kind {
        EngineKind::Cpu => Ok(Box::new(cpu::CpuMkEngine)),
        EngineKind::Gpu => {
            #[cfg(feature = "gpu")]
            {
                if gpus.is_empty() {
                    anyhow::bail!("GPU engine requested, but no compatible GPU adapter was found");
                }
                if gpus.len() == 1 {
                    return Ok(Box::new(gpus.remove(0)));
                }
                let engines: Vec<Box<dyn MintEngine>> = gpus
                    .into_iter()
                    .map(|g| Box::new(g) as Box<dyn MintEngine>)
                    .collect();
                Ok(Box::new(CompositeEngine {
                    label: "GPU",
                    engines,
                }))
            }
            #[cfg(not(feature = "gpu"))]
            {
                let _ = gpus;
                anyhow::bail!(
                    "GPU engine requested, but this binary was built without `--features gpu`"
                )
            }
        }
        EngineKind::Hybrid => {
            let mut engines: Vec<Box<dyn MintEngine>> = Vec::new();
            #[allow(unused_mut)]
            let mut has_gpu = false;
            #[cfg(feature = "gpu")]
            {
                for g in gpus {
                    has_gpu = true;
                    engines.push(Box::new(g));
                }
            }
            let has_cpu = want_id("cpu");
            if has_cpu {
                engines.push(Box::new(cpu::CpuMkEngine));
            }
            if engines.is_empty() {
                anyhow::bail!("at least one compute device must be enabled");
            }
            if engines.len() == 1 {
                return Ok(engines.remove(0));
            }
            let label = match (has_gpu, has_cpu) {
                (true, true) => "GPU + CPU",
                (true, false) => "GPU",
                (false, true) => "CPU",
                (false, false) => unreachable!(),
            };
            Ok(Box::new(CompositeEngine { label, engines }))
        }
        EngineKind::Auto => {
            #[cfg(feature = "gpu")]
            {
                if !gpus.is_empty() {
                    if gpus.len() == 1 {
                        return Ok(Box::new(gpus.remove(0)));
                    }
                    let engines: Vec<Box<dyn MintEngine>> = gpus
                        .into_iter()
                        .map(|g| Box::new(g) as Box<dyn MintEngine>)
                        .collect();
                    return Ok(Box::new(CompositeEngine {
                        label: "GPU",
                        engines,
                    }));
                }
            }
            Ok(Box::new(cpu::CpuMkEngine))
        }
    }
}

/// Computes logarithmic milestone + memoryless exponential intra-step progress,
/// pace ratio, and ETA.
pub(crate) fn compute_progress(
    best_len: usize,
    target_len: usize,
    hashes_since_best: u64,
    hashes_per_sec: f64,
) -> (f64, f64, Option<f64>) {
    if target_len == 0 || best_len >= target_len {
        return (1.0, 0.0, Some(0.0));
    }

    let expected_next = 32f64.powi((best_len + 1) as i32);
    let pace_ratio = (hashes_since_best as f64) / expected_next;
    let step_fill = (1.0 - (-pace_ratio).exp()).clamp(0.0, 0.985);
    let progress = ((best_len as f64 + step_fill) / (target_len as f64)).clamp(0.0, 0.999);

    let eta_secs = if hashes_per_sec > 1.0 {
        let mut expected_remaining = 0.0f64;
        for k in (best_len + 1)..=target_len {
            expected_remaining += 32f64.powi(k as i32);
        }
        let adj =
            (expected_remaining - (hashes_since_best as f64).min(expected_next * 0.9)).max(0.0);
        Some(adj / hashes_per_sec)
    } else {
        None
    };

    (progress, pace_ratio, eta_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logarithmic_progress_is_monotonic_per_milestone() {
        let (p0, _, _) = compute_progress(0, 6, 0, 1_000_000.0);
        let (p1, _, _) = compute_progress(1, 6, 0, 1_000_000.0);
        let (p_mid, _, _) = compute_progress(1, 6, 1024, 1_000_000.0);
        let (p_done, _, eta_done) = compute_progress(6, 6, 0, 1_000_000.0);

        assert!(p0 < p1);
        assert!(p1 < p_mid);
        assert!(p_mid < (2.0 / 6.0));
        assert_eq!(p_done, 1.0);
        assert_eq!(eta_done, Some(0.0));
    }
}
