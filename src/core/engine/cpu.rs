//! `CpuMkEngine`: high-speed CPU minting engine using Ed25519 scalar stepping
//! (`+ 8*B`) in extended coordinates, cached basepoint addition, and
//! Montgomery batch Z-inversion over GF(2^255 - 19) (1 field inversion per
//! 128 keys instead of 1 per key).

use crate::core::base32::PrefixMatcher;
use crate::core::engine::{
    EngineHandle, MintConfig, MintEngine, MintMilestone, SharedTelemetry,
};
use crate::core::onion::{derive_address, OnionKeypair};
use anyhow::Result;
use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
use curve25519_dalek::edwards::EdwardsPoint;
use curve25519_dalek::scalar::Scalar;
use rand::Rng;
use sha2::{Digest, Sha512};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc::Sender, Arc};
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

    fn start(
        &self,
        config: MintConfig,
        telemetry: Arc<SharedTelemetry>,
        on_milestone: Sender<MintMilestone>,
        cancel: Arc<AtomicBool>,
    ) -> Result<EngineHandle> {
        let matcher = PrefixMatcher::new(&config.target);
        let step_dalek = ED25519_BASEPOINT_POINT * Scalar::from(8u64);
        let step_cached = CachedStep::from_dalek(&step_dalek);
        let mut workers = Vec::with_capacity(config.threads);

        for thread_idx in 0..config.threads {
            let matcher = matcher.clone();
            let telemetry = Arc::clone(&telemetry);
            let on_milestone = on_milestone.clone();
            let cancel = Arc::clone(&cancel);

            let handle = thread::Builder::new()
                .name(format!("onyun-w{thread_idx}"))
                .spawn(move || {
                    worker_loop(
                        thread_idx,
                        &matcher,
                        step_cached,
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
    step_cached: CachedStep,
    telemetry: &SharedTelemetry,
    on_milestone: &Sender<MintMilestone>,
    cancel: &AtomicBool,
) {
    let mut rng = rand::rng();
    let mut local_best = telemetry.global_best_len.load(Ordering::Relaxed) as usize;
    let target_len = matcher.len();

    let mut xs = [Fe::ZERO; BATCH_SIZE];
    let mut ys = [Fe::ZERO; BATCH_SIZE];
    let mut zs = [Fe::ZERO; BATCH_SIZE];
    let mut prod = [Fe::ZERO; BATCH_SIZE];

    let mut local_hashes: u64 = 0;
    let mut batch_count: usize = 0;

    let mut needed_chars = (local_best + 1).min(target_len);
    let (mut mask64, mut expected64) = matcher.fast_mask_u64(needed_chars);

    let mut seeds = [[0u8; 32]; STREAMS];
    let mut base_scalars = [Scalar::ZERO; STREAMS];
    let mut currents = [ExtPoint {
        x: Fe::ZERO,
        y: Fe::ZERO,
        z: Fe::ZERO,
        t: Fe::ZERO,
    }; STREAMS];

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
            let start_dalek = ED25519_BASEPOINT_POINT * base_scalars[s];
            currents[s] = ExtPoint::from_dalek(&start_dalek);
        }
        let mut step_index: u64 = 0;

        while step_index < RESEED_STEPS
            && !cancel.load(Ordering::Relaxed)
            && local_best < target_len
        {
            let batch_start_step = step_index;
            for k in 0..BATCH_PER_STREAM {
                let base_idx = k * STREAMS;
                for (s, current) in currents.iter_mut().enumerate() {
                    let idx = base_idx + s;
                    xs[idx] = current.x;
                    ys[idx] = current.y;
                    zs[idx] = current.z;
                    *current = current.add_cached(&step_cached);
                }
            }
            step_index += BATCH_PER_STREAM as u64;

            // Montgomery's trick: invert all BATCH_SIZE Z coordinates with 1 field inversion.
            prod[0] = zs[0];
            for i in 1..BATCH_SIZE {
                prod[i] = Fe::mul(&prod[i - 1], &zs[i]);
            }
            let mut inv = prod[BATCH_SIZE - 1].invert();

            for i in (0..BATCH_SIZE).rev() {
                let z_inv = if i == 0 {
                    inv
                } else {
                    let zi = Fe::mul(&inv, &prod[i - 1]);
                    inv = Fe::mul(&inv, &zs[i]);
                    zi
                };

                // Compute affine y = Y * Z^-1 and check up to 12 prefix chars via a single u64 mask.
                let y_affine = Fe::mul(&ys[i], &z_inv);
                if (y_affine.first_u64_be() & mask64) != expected64 {
                    continue;
                }

                let mut pubkey = y_affine.to_bytes();
                let mut matched = matcher.matched_len(&pubkey);
                if matched > local_best || matched >= 50 {
                    let x_affine = Fe::mul(&xs[i], &z_inv);
                    let x_bytes = x_affine.to_bytes();
                    pubkey[31] |= (x_bytes[0] & 1) << 7;
                    matched = matcher.matched_len(&pubkey);
                }

                if matched > local_best || (target_len == 52 && matched == 51) {
                    let global_best = telemetry.global_best_len.load(Ordering::Acquire) as usize;
                    if matched > global_best || (target_len == 52 && matched == 51) {
                        let x_affine = Fe::mul(&xs[i], &z_inv);
                        let x_bytes = x_affine.to_bytes();
                        pubkey[31] = (pubkey[31] & 0x7f) | ((x_bytes[0] & 1) << 7);

                        let address = derive_address(&pubkey);
                        let full_matched = matcher.matched_len_hostname(&address);
                        if full_matched > global_best {
                            local_best = full_matched;
                            needed_chars = (local_best + 1).min(target_len);
                            (mask64, expected64) = matcher.fast_mask_u64(needed_chars);

                            let s = i % STREAMS;
                            let k = i / STREAMS;
                            let exact_step = batch_start_step + (k as u64);
                            let scalar = base_scalars[s] + Scalar::from(exact_step.wrapping_mul(8));
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

fn derive_expanded_secret(seed: &[u8; 32], step: u64, scalar: &Scalar) -> [u8; 64] {
    let mut nonce_hasher = Sha512::new();
    nonce_hasher.update(seed);
    nonce_hasher.update(step.to_le_bytes());
    let nonce_digest = nonce_hasher.finalize();

    let mut out = [0u8; 64];
    out[..32].copy_from_slice(&scalar.to_bytes());
    out[32..].copy_from_slice(&nonce_digest[32..64]);
    out
}

// ---------------------------------------------------------------------------
// 5-limb (51-bit) GF(2^255 - 19) field & extended Edwards arithmetic
// ---------------------------------------------------------------------------

const MASK51: u64 = (1u64 << 51) - 1;

/// `d = -121665 / 121666 mod (2^255 - 19)`
const EDWARDS_D: Fe = Fe([
    0x0003_4dca_1359_78a3,
    0x0001_a828_3b15_6ebd,
    0x0005_e7a2_6001_c029,
    0x0007_39c6_63a0_3cbb,
    0x0005_2036_cee2_b6ff,
]);

#[derive(Clone, Copy)]
struct Fe([u64; 5]);

impl Fe {
    const ZERO: Self = Self([0, 0, 0, 0, 0]);
    const ONE: Self = Self([1, 0, 0, 0, 0]);

    #[inline(always)]
    fn add(a: &Self, b: &Self) -> Self {
        Self([
            a.0[0] + b.0[0],
            a.0[1] + b.0[1],
            a.0[2] + b.0[2],
            a.0[3] + b.0[3],
            a.0[4] + b.0[4],
        ])
    }

    #[inline(always)]
    fn sub(a: &Self, b: &Self) -> Self {
        // Add 8*p so unreduced limbs stay positive, then carry-reduce so limbs < 2^52.
        let mut c0 = a.0[0] + 0x003f_ffff_ffff_ff68 - b.0[0];
        let mut c1 = a.0[1] + 0x003f_ffff_ffff_fff8 - b.0[1];
        let mut c2 = a.0[2] + 0x003f_ffff_ffff_fff8 - b.0[2];
        let mut c3 = a.0[3] + 0x003f_ffff_ffff_fff8 - b.0[3];
        let mut c4 = a.0[4] + 0x003f_ffff_ffff_fff8 - b.0[4];
        c1 += c0 >> 51;
        c0 &= MASK51;
        c2 += c1 >> 51;
        c1 &= MASK51;
        c3 += c2 >> 51;
        c2 &= MASK51;
        c4 += c3 >> 51;
        c3 &= MASK51;
        c0 += (c4 >> 51) * 19;
        c4 &= MASK51;
        Self([c0, c1, c2, c3, c4])
    }

    #[inline(always)]
    fn mul(a: &Self, b: &Self) -> Self {
        let a0 = a.0[0] as u128;
        let a1 = a.0[1] as u128;
        let a2 = a.0[2] as u128;
        let a3 = a.0[3] as u128;
        let a4 = a.0[4] as u128;

        let b0 = b.0[0] as u128;
        let b1 = b.0[1] as u128;
        let b2 = b.0[2] as u128;
        let b3 = b.0[3] as u128;
        let b4 = b.0[4] as u128;

        let b1_19 = b1 * 19;
        let b2_19 = b2 * 19;
        let b3_19 = b3 * 19;
        let b4_19 = b4 * 19;

        let mut c0 = a0 * b0 + a1 * b4_19 + a2 * b3_19 + a3 * b2_19 + a4 * b1_19;
        let mut c1 = a0 * b1 + a1 * b0 + a2 * b4_19 + a3 * b3_19 + a4 * b2_19;
        let mut c2 = a0 * b2 + a1 * b1 + a2 * b0 + a3 * b4_19 + a4 * b3_19;
        let mut c3 = a0 * b3 + a1 * b2 + a2 * b1 + a3 * b0 + a4 * b4_19;
        let mut c4 = a0 * b4 + a1 * b3 + a2 * b2 + a3 * b1 + a4 * b0;

        c1 += c0 >> 51;
        c0 &= MASK51 as u128;
        c2 += c1 >> 51;
        c1 &= MASK51 as u128;
        c3 += c2 >> 51;
        c2 &= MASK51 as u128;
        c4 += c3 >> 51;
        c3 &= MASK51 as u128;
        c0 += (c4 >> 51) * 19;
        c4 &= MASK51 as u128;
        c1 += c0 >> 51;
        c0 &= MASK51 as u128;

        Self([c0 as u64, c1 as u64, c2 as u64, c3 as u64, c4 as u64])
    }

    #[inline(always)]
    fn sq(&self) -> Self {
        let a0 = self.0[0] as u128;
        let a1 = self.0[1] as u128;
        let a2 = self.0[2] as u128;
        let a3 = self.0[3] as u128;
        let a4 = self.0[4] as u128;

        let a0_2 = a0 * 2;
        let a1_2 = a1 * 2;
        let a2_38 = a2 * 38;
        let a3_19 = a3 * 19;
        let a4_19 = a4 * 19;

        let mut c0 = a0 * a0 + a1_2 * a4_19 + a2_38 * a3;
        let mut c1 = a0_2 * a1 + a2_38 * a4 + a3 * a3_19;
        let mut c2 = a0_2 * a2 + a1 * a1 + (a3_19 * 2) * a4;
        let mut c3 = a0_2 * a3 + a1_2 * a2 + a4 * a4_19;
        let mut c4 = a0_2 * a4 + a1_2 * a3 + a2 * a2;

        c1 += c0 >> 51;
        c0 &= MASK51 as u128;
        c2 += c1 >> 51;
        c1 &= MASK51 as u128;
        c3 += c2 >> 51;
        c2 &= MASK51 as u128;
        c4 += c3 >> 51;
        c3 &= MASK51 as u128;
        c0 += (c4 >> 51) * 19;
        c4 &= MASK51 as u128;
        c1 += c0 >> 51;
        c0 &= MASK51 as u128;

        Self([c0 as u64, c1 as u64, c2 as u64, c3 as u64, c4 as u64])
    }

    fn pow22523(&self) -> Self {
        let t0 = self.sq();
        let t1 = t0.sq().sq();
        let t2 = Self::mul(self, &t1);
        let t3 = Self::mul(&t0, &t2);
        let t4 = t3.sq();
        let t5 = Self::mul(&t2, &t4);
        let mut t6 = t5.sq();
        for _ in 1..5 {
            t6 = t6.sq();
        }
        let t7 = Self::mul(&t6, &t5);
        let mut t8 = t7.sq();
        for _ in 1..10 {
            t8 = t8.sq();
        }
        let t9 = Self::mul(&t8, &t7);
        let mut t10 = t9.sq();
        for _ in 1..20 {
            t10 = t10.sq();
        }
        let t11 = Self::mul(&t10, &t9);
        let mut t12 = t11.sq();
        for _ in 1..10 {
            t12 = t12.sq();
        }
        let t13 = Self::mul(&t12, &t7);
        let mut t14 = t13.sq();
        for _ in 1..50 {
            t14 = t14.sq();
        }
        let t15 = Self::mul(&t14, &t13);
        let mut t16 = t15.sq();
        for _ in 1..100 {
            t16 = t16.sq();
        }
        let t17 = Self::mul(&t16, &t15);
        let mut t18 = t17.sq();
        for _ in 1..50 {
            t18 = t18.sq();
        }
        let t19 = Self::mul(&t18, &t13);
        let t20 = t19.sq().sq();
        Self::mul(&t20, self)
    }

    /// Computes `self^(p - 2) mod p`.
    fn invert(&self) -> Self {
        let mut t0 = self.sq();
        let mut t1 = t0.sq().sq();
        let mut t2 = Self::mul(self, &t1);
        t0 = Self::mul(&t0, &t2);
        let t3 = t0.sq();
        t1 = Self::mul(&t2, &t3);
        t2 = t1.sq();
        for _ in 1..5 {
            t2 = t2.sq();
        }
        t1 = Self::mul(&t2, &t1);
        t2 = t1.sq();
        for _ in 1..10 {
            t2 = t2.sq();
        }
        t2 = Self::mul(&t2, &t1);
        let mut t4 = t2.sq();
        for _ in 1..20 {
            t4 = t4.sq();
        }
        t2 = Self::mul(&t4, &t2);
        t2 = t2.sq();
        for _ in 1..10 {
            t2 = t2.sq();
        }
        t1 = Self::mul(&t2, &t1);
        t2 = t1.sq();
        for _ in 1..50 {
            t2 = t2.sq();
        }
        t2 = Self::mul(&t2, &t1);
        t4 = t2.sq();
        for _ in 1..100 {
            t4 = t4.sq();
        }
        t2 = Self::mul(&t4, &t2);
        t2 = t2.sq();
        for _ in 1..50 {
            t2 = t2.sq();
        }
        t1 = Self::mul(&t2, &t1);
        t1 = t1.sq().sq().sq().sq().sq();
        Self::mul(&t1, &t0)
    }

    fn from_bytes(bytes: &[u8; 32]) -> Self {
        let load8 = |off: usize| -> u64 {
            u64::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
                bytes[off + 4],
                bytes[off + 5],
                bytes[off + 6],
                bytes[off + 7],
            ])
        };
        let l0 = load8(0) & MASK51;
        let l1 = (load8(6) >> 3) & MASK51;
        let l2 = (load8(12) >> 6) & MASK51;
        let l3 = (load8(19) >> 1) & MASK51;
        let l4 = (load8(24) >> 12) & MASK51;
        Self([l0, l1, l2, l3, l4])
    }

    #[inline(always)]
    fn to_bytes(self) -> [u8; 32] {
        let mut h = self.0;
        // Carry propagation
        let mut q = (h[0] + (19 * (h[4] >> 51))) >> 51;
        q = (h[1] + q) >> 51;
        q = (h[2] + q) >> 51;
        q = (h[3] + q) >> 51;
        q = (h[4] + q) >> 51;

        h[0] += 19 * q;
        h[1] += h[0] >> 51;
        h[0] &= MASK51;
        h[2] += h[1] >> 51;
        h[1] &= MASK51;
        h[3] += h[2] >> 51;
        h[2] &= MASK51;
        h[4] += h[3] >> 51;
        h[3] &= MASK51;
        h[4] &= MASK51;

        // Conditional subtraction of p = 2^255 - 19
        let mut t = h;
        t[0] = t[0].wrapping_add(19);
        t[1] += t[0] >> 51;
        t[0] &= MASK51;
        t[2] += t[1] >> 51;
        t[1] &= MASK51;
        t[3] += t[2] >> 51;
        t[2] &= MASK51;
        t[4] += t[3] >> 51;
        t[3] &= MASK51;
        let overflow = t[4] >> 51;
        t[4] &= MASK51;

        let mask = 0u64.wrapping_sub(overflow);
        for i in 0..5 {
            h[i] = (h[i] & !mask) | (t[i] & mask);
        }

        let mut out = [0u8; 32];
        let w0 = h[0] | (h[1] << 51);
        let w1 = (h[1] >> 13) | (h[2] << 38);
        let w2 = (h[2] >> 26) | (h[3] << 25);
        let w3 = (h[3] >> 39) | (h[4] << 12);
        out[0..8].copy_from_slice(&w0.to_le_bytes());
        out[8..16].copy_from_slice(&w1.to_le_bytes());
        out[16..24].copy_from_slice(&w2.to_le_bytes());
        out[24..32].copy_from_slice(&w3.to_le_bytes());
        out
    }
    /// Returns the first 8 canonical bytes in big-endian bit order for fast prefix masking.
    #[inline(always)]
    fn first_u64_be(self) -> u64 {
        let mut h = self.0;
        let mut q = (h[0] + (19 * (h[4] >> 51))) >> 51;
        q = (h[1] + q) >> 51;
        q = (h[2] + q) >> 51;
        q = (h[3] + q) >> 51;
        q = (h[4] + q) >> 51;

        h[0] += 19 * q;
        h[1] += h[0] >> 51;
        h[0] &= MASK51;
        h[2] += h[1] >> 51;
        h[1] &= MASK51;
        h[3] += h[2] >> 51;
        h[2] &= MASK51;
        h[4] += h[3] >> 51;
        h[3] &= MASK51;

        let mut t0 = h[0].wrapping_add(19);
        let mut t1 = h[1] + (t0 >> 51);
        t0 &= MASK51;
        let t2 = h[2] + (t1 >> 51);
        t1 &= MASK51;
        let t3 = h[3] + (t2 >> 51);
        let t4 = (h[4] & MASK51) + (t3 >> 51);
        let overflow = t4 >> 51;

        let mask = 0u64.wrapping_sub(overflow);
        let r0 = (h[0] & !mask) | (t0 & mask);
        let r1 = (h[1] & !mask) | (t1 & mask);
        (r0 | (r1 << 51)).swap_bytes()
    }
}

#[derive(Clone, Copy)]
struct ExtPoint {
    x: Fe,
    y: Fe,
    z: Fe,
    t: Fe,
}

#[derive(Clone, Copy)]
struct CachedStep {
    y_plus_x: Fe,
    y_minus_x: Fe,
    t2d: Fe,
}

/// Recovers affine `(x, y)` from a `curve25519_dalek::EdwardsPoint`.
fn decompress_dalek(p: &EdwardsPoint) -> (Fe, Fe) {
    let comp = p.compress();
    let mut bytes = *comp.as_bytes();
    let x_sign = bytes[31] >> 7;
    bytes[31] &= 0x7f;

    let y = Fe::from_bytes(&bytes);
    let y2 = y.sq();
    let u = Fe::sub(&y2, &Fe::ONE);
    let v = Fe::add(&Fe::mul(&y2, &EDWARDS_D), &Fe::ONE);

    let v3 = Fe::mul(&v.sq(), &v);
    let v7 = Fe::mul(&v3.sq(), &v);
    let uv7 = Fe::mul(&u, &v7);
    let mut x = Fe::mul(&Fe::mul(&u, &v3), &uv7.pow22523());

    let vx2 = Fe::mul(&v, &x.sq());
    let check_neg = Fe::sub(&vx2, &u).to_bytes();
    if check_neg != [0u8; 32] {
        // Multiply by sqrt(-1) mod p
        const SQRT_M1: Fe = Fe([
            0x0006_1b27_4a0e_a0b0,
            0x0000_d5a5_fc8f_189d,
            0x0007_ef5e_9cbd_0c60,
            0x0007_8595_a680_4c9e,
            0x0002_b832_4804_fc1d,
        ]);
        x = Fe::mul(&x, &SQRT_M1);
    }

    let x_bytes = x.to_bytes();
    if (x_bytes[0] & 1) != x_sign {
        x = Fe::sub(&Fe::ZERO, &x);
    }

    (x, y)
}
impl ExtPoint {
    fn from_dalek(p: &EdwardsPoint) -> Self {
        let (x, y) = decompress_dalek(p);
        Self {
            x,
            y,
            z: Fe::ONE,
            t: Fe::mul(&x, &y),
        }
    }

    /// Unified extended-plus-cached addition where cached point has `Z = 1`.
    #[inline(always)]
    fn add_cached(&self, q: &CachedStep) -> Self {
        let a = Fe::mul(&Fe::sub(&self.y, &self.x), &q.y_minus_x);
        let b = Fe::mul(&Fe::add(&self.y, &self.x), &q.y_plus_x);
        let c = Fe::mul(&self.t, &q.t2d);
        let d = Fe::add(&self.z, &self.z);

        let e = Fe::sub(&b, &a);
        let f = Fe::sub(&d, &c);
        let g = Fe::add(&d, &c);
        let h = Fe::add(&b, &a);

        Self {
            x: Fe::mul(&e, &f),
            y: Fe::mul(&g, &h),
            z: Fe::mul(&f, &g),
            t: Fe::mul(&e, &h),
        }
    }
}

impl CachedStep {
    fn from_dalek(p: &EdwardsPoint) -> Self {
        let (x, y) = decompress_dalek(p);
        let t = Fe::mul(&x, &y);
        Self {
            y_plus_x: Fe::add(&y, &x),
            y_minus_x: Fe::sub(&y, &x),
            t2d: Fe::mul(&t, &Fe::add(&EDWARDS_D, &EDWARDS_D)),
        }
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
        assert_eq!(ext.y.to_bytes(), {
            let mut b = *base_pt.compress().as_bytes();
            b[31] &= 0x7f;
            b
        }, "from_dalek y");
        assert_eq!(Fe::mul(&ext.x, &ext.x.invert()).to_bytes(), Fe::ONE.to_bytes(), "invert");
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
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
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
