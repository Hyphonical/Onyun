//! Shared state and background mint coordinator for the Axum / Tauri UI.

use crate::core::base32::normalize_target;
use crate::core::engine::{
    compute_progress, select_engine, EngineHandle, EngineKind, MintConfig, MintMilestone,
    MintSnapshot, SharedTelemetry,
};
use crate::core::storage::{load_existing_best, save_keypair_atomic, target_dir};
use anyhow::{bail, Result};
use hmac::{Hmac, KeyInit, Mac};
use rand::Rng;
use serde::Serialize;
use sha2::Sha256;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;
use tokio::sync::broadcast;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SavedMintEntry {
    pub(crate) target: String,
    pub(crate) best_address: String,
    pub(crate) matched_len: usize,
    pub(crate) target_len: usize,
    pub(crate) completed: bool,
    pub(crate) modified_at: u64,
    pub(crate) path: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct StatusPayload {
    pub(crate) version: &'static str,
    pub(crate) engine: &'static str,
    pub(crate) cpu_threads: usize,
    pub(crate) out_dir: String,
    pub(crate) auth_required: bool,
    pub(crate) authenticated: bool,
    pub(crate) snapshot: MintSnapshot,
}

pub(crate) struct AuthConfig {
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) secret: [u8; 32],
}

impl AuthConfig {
    pub(crate) fn from_env() -> Option<Self> {
        let username = std::env::var("ONYUN_USER").ok()?.trim().to_string();
        let password = std::env::var("ONYUN_PASS").ok()?;
        if username.is_empty() || password.is_empty() {
            return None;
        }
        let mut secret = [0u8; 32];
        rand::rng().fill_bytes(&mut secret);
        Some(Self {
            username,
            password,
            secret,
        })
    }

    pub(crate) fn verify_credentials(&self, user: &str, pass: &str) -> bool {
        let user_ok: bool = user.as_bytes().ct_eq(self.username.as_bytes()).into();
        let pass_ok: bool = pass.as_bytes().ct_eq(self.password.as_bytes()).into();
        user_ok && pass_ok
    }

    pub(crate) fn issue_token(&self) -> Option<String> {
        let exp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_secs()
            .saturating_add(60 * 60 * 24 * 7);
        let payload = format!("{}:{exp}", self.username);
        let mut mac = HmacSha256::new_from_slice(&self.secret).ok()?;
        mac.update(payload.as_bytes());
        let sig = hex_encode(&mac.finalize().into_bytes());
        Some(format!("{exp}.{sig}"))
    }

    pub(crate) fn verify_token(&self, token: &str) -> bool {
        let Some((exp_str, sig_hex)) = token.split_once('.') else {
            return false;
        };
        let Ok(exp) = exp_str.parse::<u64>() else {
            return false;
        };
        let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return false;
        };
        if now.as_secs() > exp {
            return false;
        }
        let payload = format!("{}:{exp}", self.username);
        let Ok(mut mac) = HmacSha256::new_from_slice(&self.secret) else {
            return false;
        };
        mac.update(payload.as_bytes());
        let expected = hex_encode(&mac.finalize().into_bytes());
        sig_hex.as_bytes().ct_eq(expected.as_bytes()).into()
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

struct ActiveJob {
    cancel: Arc<AtomicBool>,
    coordinator: JoinHandle<()>,
}

pub(crate) struct AppState {
    pub(crate) out_dir: PathBuf,
    pub(crate) auth: Option<AuthConfig>,
    pub(crate) cpu_threads: usize,
    snapshot: Mutex<MintSnapshot>,
    active_job: Mutex<Option<ActiveJob>>,
    pub(crate) events_tx: broadcast::Sender<MintSnapshot>,
}

impl AppState {
    pub(crate) fn new(out_dir: PathBuf) -> Arc<Self> {
        let cpu_threads = std::thread::available_parallelism()
            .map(std::num::NonZero::get)
            .unwrap_or(4);
        let (events_tx, _) = broadcast::channel(64);
        Arc::new(Self {
            out_dir,
            auth: AuthConfig::from_env(),
            cpu_threads,
            snapshot: Mutex::new(MintSnapshot {
                active: false,
                completed: false,
                target: String::new(),
                best_address: None,
                best_len: 0,
                target_len: 0,
                total_hashes: 0,
                hashes_since_best: 0,
                hashes_per_sec: 0.0,
                elapsed_secs: 0.0,
                progress: 0.0,
                pace_ratio: 0.0,
                eta_secs: None,
            }),
            active_job: Mutex::new(None),
            events_tx,
        })
    }

    pub(crate) fn current_snapshot(&self) -> MintSnapshot {
        self.snapshot
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }

    pub(crate) fn set_snapshot(&self, snap: MintSnapshot) {
        if let Ok(mut guard) = self.snapshot.lock() {
            *guard = snap.clone();
        }
        let _ = self.events_tx.send(snap);
    }

    pub(crate) fn stop_mint(&self) {
        let job = {
            let Ok(mut guard) = self.active_job.lock() else {
                return;
            };
            guard.take()
        };
        if let Some(job) = job {
            job.cancel.store(true, Ordering::SeqCst);
            let _ = job.coordinator.join();
        }
        let mut snap = self.current_snapshot();
        if snap.active {
            snap.active = false;
            snap.hashes_per_sec = 0.0;
            self.set_snapshot(snap);
        }
    }

    pub(crate) fn start_mint(
        self: &Arc<Self>,
        raw_target: &str,
        requested_threads: Option<usize>,
    ) -> Result<MintSnapshot> {
        let target = normalize_target(raw_target)?;
        let threads = requested_threads
            .unwrap_or(self.cpu_threads)
            .clamp(1, self.cpu_threads.max(1) * 2);

        self.stop_mint();

        let existing = load_existing_best(&self.out_dir, &target);
        let initial_best_len = existing.as_ref().map_or(0, |(_, len)| *len);
        let mut best_address = existing.as_ref().map(|(addr, _)| addr.clone());

        if initial_best_len >= target.len() {
            let snap = MintSnapshot {
                active: false,
                completed: true,
                target: target.clone(),
                best_address,
                best_len: initial_best_len,
                target_len: target.len(),
                total_hashes: 0,
                hashes_since_best: 0,
                hashes_per_sec: 0.0,
                elapsed_secs: 0.0,
                progress: 1.0,
                pace_ratio: 0.0,
                eta_secs: Some(0.0),
            };
            self.set_snapshot(snap.clone());
            return Ok(snap);
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let telemetry = Arc::new(SharedTelemetry::new(threads, initial_best_len));
        let (tx, rx) = mpsc::channel::<MintMilestone>();

        let engine = select_engine(EngineKind::Auto)?;
        let handle: EngineHandle = engine.start(
            MintConfig {
                target: target.clone(),
                threads,
            },
            Arc::clone(&telemetry),
            tx,
            Arc::clone(&cancel),
        )?;

        let initial_snap = MintSnapshot {
            active: true,
            completed: false,
            target: target.clone(),
            best_address: best_address.clone(),
            best_len: initial_best_len,
            target_len: target.len(),
            total_hashes: 0,
            hashes_since_best: 0,
            hashes_per_sec: 0.0,
            elapsed_secs: 0.0,
            progress: (initial_best_len as f64) / (target.len() as f64),
            pace_ratio: 0.0,
            eta_secs: None,
        };
        self.set_snapshot(initial_snap.clone());

        let state = Arc::clone(self);
        let cancel_worker = Arc::clone(&cancel);
        let out_dir = self.out_dir.clone();
        let target_owned = target.clone();

        let coordinator = thread::spawn(move || {
            let mut best_len = initial_best_len;
            let mut last_hashes: u64 = 0;
            let mut last_tick = Instant::now();
            let mut ewma_rate: f64 = 0.0;

            while !cancel_worker.load(Ordering::Relaxed) && best_len < target_owned.len() {
                while let Ok(milestone) = rx.try_recv() {
                    if milestone.matched_len > best_len {
                        best_len = milestone.matched_len;
                        telemetry.record_new_best(best_len);
                        best_address = Some(milestone.keypair.address.clone());
                        let _ = save_keypair_atomic(&out_dir, &target_owned, &milestone.keypair);
                        if best_len >= target_owned.len() {
                            cancel_worker.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                }

                let now = Instant::now();
                let dt = now.duration_since(last_tick).as_secs_f64();
                let total_hashes = telemetry.total_hashes();
                let delta_hashes = total_hashes.saturating_sub(last_hashes);
                if dt > 0.02 {
                    let inst_rate = (delta_hashes as f64) / dt;
                    ewma_rate = if ewma_rate == 0.0 {
                        inst_rate
                    } else {
                        let alpha = 1.0 - (-dt / 0.8).exp();
                        alpha * inst_rate + (1.0 - alpha) * ewma_rate
                    };
                    last_hashes = total_hashes;
                    last_tick = now;
                }

                let hashes_at_best = telemetry.hashes_at_last_best.load(Ordering::Relaxed);
                let hashes_since_best = total_hashes.saturating_sub(hashes_at_best);
                let (progress, pace_ratio, eta_secs) =
                    compute_progress(best_len, target_owned.len(), hashes_since_best, ewma_rate);

                let completed = best_len >= target_owned.len();
                let snap = MintSnapshot {
                    active: !completed && !cancel_worker.load(Ordering::Relaxed),
                    completed,
                    target: target_owned.clone(),
                    best_address: best_address.clone(),
                    best_len,
                    target_len: target_owned.len(),
                    total_hashes,
                    hashes_since_best,
                    hashes_per_sec: if completed { 0.0 } else { ewma_rate },
                    elapsed_secs: telemetry.started_at.elapsed().as_secs_f64(),
                    progress,
                    pace_ratio,
                    eta_secs,
                };
                state.set_snapshot(snap);

                if completed || cancel_worker.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }

            cancel_worker.store(true, Ordering::SeqCst);
            handle.join();

            // Drain any final milestone delivered just as workers exited.
            while let Ok(milestone) = rx.try_recv() {
                if milestone.matched_len > best_len {
                    best_len = milestone.matched_len;
                    telemetry.record_new_best(best_len);
                    best_address = Some(milestone.keypair.address.clone());
                    let _ = save_keypair_atomic(&out_dir, &target_owned, &milestone.keypair);
                }
            }

            let total_hashes = telemetry.total_hashes();
            let hashes_at_best = telemetry.hashes_at_last_best.load(Ordering::Relaxed);
            let hashes_since_best = total_hashes.saturating_sub(hashes_at_best);
            let completed = best_len >= target_owned.len();
            let (progress, pace_ratio, eta_secs) =
                compute_progress(best_len, target_owned.len(), hashes_since_best, ewma_rate);

            state.set_snapshot(MintSnapshot {
                active: false,
                completed,
                target: target_owned,
                best_address,
                best_len,
                target_len: target_len_safe(best_len, state.current_snapshot().target_len),
                total_hashes,
                hashes_since_best,
                hashes_per_sec: 0.0,
                elapsed_secs: telemetry.started_at.elapsed().as_secs_f64(),
                progress: if completed { 1.0 } else { progress },
                pace_ratio,
                eta_secs: if completed { Some(0.0) } else { eta_secs },
            });
        });

        let Ok(mut guard) = self.active_job.lock() else {
            cancel.store(true, Ordering::SeqCst);
            bail!("internal lock error");
        };
        *guard = Some(ActiveJob {
            cancel,
            coordinator,
        });

        Ok(initial_snap)
    }

    pub(crate) fn list_mints(&self) -> Vec<SavedMintEntry> {
        let mut entries = Vec::new();
        let Ok(read_dir) = std::fs::read_dir(&self.out_dir) else {
            return entries;
        };

        for item in read_dir.flatten() {
            let Ok(ft) = item.file_type() else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }
            let target = item.file_name().to_string_lossy().to_string();
            if normalize_target(&target).is_err() {
                continue;
            }
            let Some((best_address, matched_len)) = load_existing_best(&self.out_dir, &target)
            else {
                continue;
            };
            let hostname_path = target_dir(&self.out_dir, &target).join("hostname");
            let modified_at = std::fs::metadata(&hostname_path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            let target_len = target.len();
            entries.push(SavedMintEntry {
                target,
                best_address,
                matched_len,
                target_len,
                completed: matched_len >= target_len,
                modified_at,
                path: target_dir(&self.out_dir, &item.file_name().to_string_lossy())
                    .display()
                    .to_string(),
            });
        }

        entries.sort_by_key(|a| std::cmp::Reverse(a.modified_at));
        entries
    }
}

fn target_len_safe(best_len: usize, stored_target_len: usize) -> usize {
    stored_target_len.max(best_len)
}
