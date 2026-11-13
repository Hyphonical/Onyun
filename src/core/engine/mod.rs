//! `MintEngine` trait, shared telemetry, logarithmic progress estimation, and
//! milestone/snapshot types.

pub(crate) mod cpu;

use crate::core::onion::OnionKeypair;
use anyhow::Result;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{mpsc::Sender, Arc};
use std::thread::JoinHandle;
use std::time::Instant;

#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "web"), allow(dead_code))]
pub(crate) struct MintConfig {
    pub(crate) target: String,
    pub(crate) threads: usize,
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
    fn start(
        &self,
        config: MintConfig,
        telemetry: Arc<SharedTelemetry>,
        on_milestone: Sender<MintMilestone>,
        cancel: Arc<AtomicBool>,
    ) -> Result<EngineHandle>;
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
