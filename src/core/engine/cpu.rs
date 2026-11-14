//! `CpuMkEngine`: high-speed CPU minting engine using Montgomery `(U : W)`
//! differential stepping (`+ 8*B`), 4-way AVX2 SIMD field arithmetic (with
//! scalar fallback), and Montgomery batch inversion over GF(2^255 - 19).

use crate::core::base32::PrefixMatcher;
#[cfg(test)]
use crate::core::engine::helper::{CachedStep, ExtPoint};
use crate::core::engine::helper::{Fe, MontPoint, MontStep, derive_expanded_secret};
use crate::core::engine::{EngineHandle, MintConfig, MintEngine, MintMilestone, SharedTelemetry};
use crate::core::onion::{OnionKeypair, derive_address};
use anyhow::Result;
use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
use curve25519_dalek::scalar::Scalar;
use rand::Rng;
use sha2::{Digest, Sha512};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc::Sender};
use std::thread;

const STREAMS: usize = 4;
const BATCH_PER_STREAM: usize = 64;
const BATCH_SIZE: usize = STREAMS * BATCH_PER_STREAM;
const TELEMETRY_FLUSH_BATCHES: usize = 64; // 64 * 256 = 16,384 keys per atomic add
const RESEED_STEPS: u64 = 1 << 24;

pub(crate) const LABEL: &str = "CPU";

pub(crate) struct CpuMkEngine;

impl MintEngine for CpuMkEngine {
    fn label(&self) -> &'static str {
        LABEL
    }

    fn device_name(&self) -> String {
        #[cfg(target_os = "windows")]
        if let Ok(id) = std::env::var("PROCESSOR_IDENTIFIER") {
            return id;
        }
        #[cfg(target_os = "linux")]
        if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in cpuinfo.lines() {
                if let Some(rest) = line.strip_prefix("model name")
                    && let Some(name) = rest.split(':').nth(1)
                {
                    return name.trim().to_string();
                }
            }
        }
        format!(
            "{} ({} cores)",
            std::env::consts::ARCH,
            crate::cli::args::default_threads()
        )
    }

    fn slot_count(&self, cpu_threads: usize) -> usize {
        cpu_threads.max(1)
    }

    fn start(
        &self,
        config: MintConfig,
        telemetry: Arc<SharedTelemetry>,
        on_milestone: Sender<MintMilestone>,
        cancel: Arc<AtomicBool>,
    ) -> Result<EngineHandle> {
        let matcher = PrefixMatcher::new(&config.target);
        let step_dalek = ED25519_BASEPOINT_POINT * Scalar::from(8u64);
        let step_mont = MontStep::from_dalek(&step_dalek);
        let mut workers = Vec::with_capacity(config.threads);

        #[cfg(target_arch = "x86_64")]
        let use_avx2 = std::arch::is_x86_feature_detected!("avx2");
        #[cfg(not(target_arch = "x86_64"))]
        let use_avx2 = false;

        for thread_idx in 0..config.threads {
            let telemetry_slot = config.telemetry_offset + thread_idx;
            let matcher = matcher.clone();
            let telemetry = Arc::clone(&telemetry);
            let on_milestone = on_milestone.clone();
            let cancel = Arc::clone(&cancel);

            let handle = thread::Builder::new()
                .name(format!("onyun-w{thread_idx}"))
                .spawn(move || {
                    #[cfg(target_arch = "x86_64")]
                    if use_avx2 {
                        // SAFETY: `is_x86_feature_detected!("avx2")` checked above.
                        unsafe {
                            crate::core::engine::simd::avx2::worker_loop_avx2(
                                telemetry_slot,
                                &matcher,
                                step_mont,
                                &telemetry,
                                &on_milestone,
                                &cancel,
                            );
                        }
                        return;
                    }
                    let _ = use_avx2;
                    worker_loop(
                        telemetry_slot,
                        &matcher,
                        step_mont,
                        &telemetry,
                        &on_milestone,
                        &cancel,
                    );
                })?;
            workers.push(handle);
        }

        Ok(EngineHandle { workers })
    }
}

fn worker_loop(
    thread_idx: usize,
    matcher: &PrefixMatcher,
    step_mont: MontStep,
    telemetry: &SharedTelemetry,
    on_milestone: &Sender<MintMilestone>,
    cancel: &AtomicBool,
) {
    let mut rng = rand::rng();
    let mut local_best = telemetry.global_best_len.load(Ordering::Relaxed) as usize;
    let target_len = matcher.len();

    let mut denom = [Fe::ZERO; BATCH_SIZE];
    let mut numer = [Fe::ZERO; BATCH_SIZE];
    let mut prod = [Fe::ZERO; BATCH_SIZE];

    let mut local_hashes: u64 = 0;
    let mut batch_count: usize = 0;

    let mut needed_chars = (local_best + 1).min(target_len);
    let (mut mask64, mut expected64) = matcher.fast_mask_u64(needed_chars);

    let mut seeds = [[0u8; 32]; STREAMS];
    let mut base_scalars = [Scalar::ZERO; STREAMS];
    let mut prevs = [MontPoint {
        u: Fe::ZERO,
        w: Fe::ZERO,
    }; STREAMS];
    let mut currents = [MontPoint {
        u: Fe::ZERO,
        w: Fe::ZERO,
    }; STREAMS];
    let step8 = Scalar::from(8u64);

    while !cancel.load(Ordering::Relaxed) && local_best < target_len {
        for s in 0..STREAMS {
            rng.fill_bytes(&mut seeds[s]);
            let hash = Sha512::digest(seeds[s]);
            let mut base_scalar_bytes = [0u8; 32];
            base_scalar_bytes.copy_from_slice(&hash[..32]);
            base_scalar_bytes[0] &= 248;
            base_scalar_bytes[31] &= 127;
            base_scalar_bytes[31] |= 64;

            base_scalars[s] = Scalar::from_bytes_mod_order(base_scalar_bytes);
            let p0 = ED25519_BASEPOINT_POINT * base_scalars[s];
            let p_prev = ED25519_BASEPOINT_POINT * (base_scalars[s] - step8);
            currents[s] = MontPoint::from_dalek(&p0);
            prevs[s] = MontPoint::from_dalek(&p_prev);
        }
        let mut step_index: u64 = 0;

        while step_index < RESEED_STEPS
            && !cancel.load(Ordering::Relaxed)
            && local_best < target_len
        {
            let batch_start_step = step_index;
            for k in 0..BATCH_PER_STREAM {
                let base_idx = k * STREAMS;
                for s in 0..STREAMS {
                    let idx = base_idx + s;
                    let (next, a, b) = currents[s].step_with(&prevs[s], &step_mont);
                    denom[idx] = a;
                    numer[idx] = b;
                    prevs[s] = currents[s];
                    currents[s] = next;
                }
            }
            step_index += BATCH_PER_STREAM as u64;

            // Montgomery's trick: invert all BATCH_SIZE (U + W) denominators with 1 field inversion.
            prod[0] = denom[0];
            for i in 1..BATCH_SIZE {
                prod[i] = Fe::mul(&prod[i - 1], &denom[i]);
            }
            let mut inv = prod[BATCH_SIZE - 1].invert();

            for i in (0..BATCH_SIZE).rev() {
                let d_inv = if i == 0 {
                    inv
                } else {
                    let di = Fe::mul(&inv, &prod[i - 1]);
                    inv = Fe::mul(&inv, &denom[i]);
                    di
                };

                let y_affine = Fe::mul(&numer[i], &d_inv);
                if (y_affine.first_u64_be() & mask64) != expected64 {
                    continue;
                }

                let s = i % STREAMS;
                let k = i / STREAMS;
                let exact_step = batch_start_step + (k as u64);
                let scalar = base_scalars[s] + Scalar::from(exact_step.wrapping_mul(8));
                let full_pt = ED25519_BASEPOINT_POINT * scalar;
                let pubkey = full_pt.compress().to_bytes();

                let matched = matcher.matched_len(&pubkey);
                if matched > local_best || (target_len == 52 && matched == 51) {
                    let global_best = telemetry.global_best_len.load(Ordering::Acquire) as usize;
                    if matched > global_best || (target_len == 52 && matched == 51) {
                        let address = derive_address(&pubkey);
                        let full_matched = matcher.matched_len_hostname(&address);
                        if full_matched > global_best {
                            local_best = full_matched;
                            needed_chars = (local_best + 1).min(target_len);
                            (mask64, expected64) = matcher.fast_mask_u64(needed_chars);

                            let expanded_secret =
                                derive_expanded_secret(&seeds[s], exact_step, &scalar);
                            let kp = OnionKeypair {
                                address,
                                public_key: pubkey,
                                expanded_secret,
                            };
                            let _ = on_milestone.send(MintMilestone {
                                matched_len: full_matched,
                                keypair: kp,
                            });
                        } else if global_best > local_best {
                            local_best = global_best;
                            needed_chars = (local_best + 1).min(target_len);
                            (mask64, expected64) = matcher.fast_mask_u64(needed_chars);
                        }
                    } else {
                        local_best = global_best;
                        needed_chars = (local_best + 1).min(target_len);
                        (mask64, expected64) = matcher.fast_mask_u64(needed_chars);
                    }
                }
            }

            local_hashes += BATCH_SIZE as u64;
            batch_count += 1;

            if batch_count >= TELEMETRY_FLUSH_BATCHES {
                telemetry.thread_hashes[thread_idx]
                    .0
                    .fetch_add(local_hashes, Ordering::Relaxed);
                local_hashes = 0;
                batch_count = 0;
                let global_best = telemetry.global_best_len.load(Ordering::Relaxed) as usize;
                if global_best > local_best {
                    local_best = global_best;
                    needed_chars = (local_best + 1).min(target_len);
                    (mask64, expected64) = matcher.fast_mask_u64(needed_chars);
                }
            }
        }
    }

    if local_hashes > 0 {
        telemetry.thread_hashes[thread_idx]
            .0
            .fetch_add(local_hashes, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_cached_addition_matches_dalek_compress() {
        let seed = [9u8; 32];
        let hash = Sha512::digest(seed);
        let mut s_bytes = [0u8; 32];
        s_bytes.copy_from_slice(&hash[..32]);
        s_bytes[0] &= 248;
        s_bytes[31] &= 127;
        s_bytes[31] |= 64;

        let base = Scalar::from_bytes_mod_order(s_bytes);
        let step_dalek = ED25519_BASEPOINT_POINT * Scalar::from(8u64);
        let step_cached = CachedStep::from_dalek(&step_dalek);

        let base_pt = ED25519_BASEPOINT_POINT * base;
        let mut ext = ExtPoint::from_dalek(&base_pt);
        assert_eq!(
            ext.y.to_bytes(),
            {
                let mut b = *base_pt.compress().as_bytes();
                b[31] &= 0x7f;
                b
            },
            "from_dalek y"
        );
        assert_eq!(
            Fe::mul(&ext.x, &ext.x.invert()).to_bytes(),
            Fe::ONE.to_bytes(),
            "invert"
        );
        let expected_0 = base_pt.compress().to_bytes();
        let mut got_0 = ext.y.to_bytes();
        got_0[31] |= (ext.x.to_bytes()[0] & 1) << 7;
        assert_eq!(got_0, expected_0, "from_dalek 0 steps");

        for _ in 0..7 {
            ext = ext.add_cached(&step_cached);
        }

        let z_inv = ext.z.invert();
        let mut got = Fe::mul(&ext.y, &z_inv).to_bytes();
        let x_bytes = Fe::mul(&ext.x, &z_inv).to_bytes();
        got[31] |= (x_bytes[0] & 1) << 7;

        let expected = (ED25519_BASEPOINT_POINT * (base + Scalar::from(7u64 * 8u64)))
            .compress()
            .to_bytes();

        assert_eq!(got, expected, "7 steps");
    }

    #[test]
    fn dedicated_sq_and_first_u64_be_match_reference() {
        let seed = [42u8; 32];
        let hash = Sha512::digest(seed);
        let mut s_bytes = [0u8; 32];
        s_bytes.copy_from_slice(&hash[..32]);
        s_bytes[0] &= 248;
        s_bytes[31] &= 127;
        s_bytes[31] |= 64;
        let pt = ED25519_BASEPOINT_POINT * Scalar::from_bytes_mod_order(s_bytes);
        let ext = ExtPoint::from_dalek(&pt);

        // Verify dedicated sq() matches mul(x, x)
        assert_eq!(ext.x.sq().to_bytes(), Fe::mul(&ext.x, &ext.x).to_bytes());
        assert_eq!(ext.y.sq().to_bytes(), Fe::mul(&ext.y, &ext.y).to_bytes());

        // Verify first_u64_be() matches first 8 bytes of to_bytes()
        let bytes = ext.y.to_bytes();
        let expected_u64 = u64::from_be_bytes(bytes[0..8].try_into().unwrap_or([0u8; 8]));
        assert_eq!(ext.y.first_u64_be(), expected_u64);
    }

    #[test]
    fn fast_mask_u64_matches_prefix_matcher() {
        let bytes: [u8; 32] = [
            0x73, 0x74, 0xd4, 0x11, 0xaa, 0xbb, 0xcc, 0xdd, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let encoded = crate::core::base32::encode_onion_bytes(&[
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        let target = std::str::from_utf8(&encoded[..12]).unwrap_or("");
        let matcher = PrefixMatcher::new(target);
        let w0 = u64::from_be_bytes(bytes[0..8].try_into().unwrap_or([0u8; 8]));
        for chars in 1..=12 {
            let (mask, expected) = matcher.fast_mask_u64(chars);
            assert_eq!(w0 & mask, expected, "chars={chars}");
            let (mask_bad, expected_bad) = matcher.fast_mask_u64(chars);
            assert_ne!((w0 ^ (1u64 << (64 - chars * 5))) & mask_bad, expected_bad);
        }
    }
}
