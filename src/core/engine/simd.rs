//! 4-way AVX2 SIMD field arithmetic (`GF(2^255 - 19)`, 10 limbs radix-2^25.5)
//! and Montgomery `(U : W)` differential stepping for `CpuMkEngine`.

#[cfg(target_arch = "x86_64")]
pub(super) mod avx2 {
    use crate::core::base32::PrefixMatcher;
    use crate::core::engine::MintMilestone;
    use crate::core::engine::SharedTelemetry;
    use crate::core::engine::helper::{Fe, MontPoint, MontStep, derive_expanded_secret};
    use crate::core::onion::{OnionKeypair, derive_address};
    use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    use curve25519_dalek::scalar::Scalar;
    use rand::Rng;
    use sha2::{Digest, Sha512};
    use std::arch::x86_64::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;

    const MASK26: u64 = 0x03ff_ffff;
    const MASK25: u64 = 0x01ff_ffff;

    const BATCH_PER_STREAM: usize = 64;
    const TELEMETRY_FLUSH_BATCHES: usize = 64; // 64 * 256 = 16,384 keys per atomic add
    const RESEED_STEPS: u64 = 1 << 24;

    #[derive(Clone, Copy)]
    struct Fe4([__m256i; 10]);

    impl Fe4 {
        #[inline]
        #[target_feature(enable = "avx2")]
        unsafe fn zero() -> Self {
            let z = _mm256_setzero_si256();
            Self([z; 10])
        }

        #[inline]
        #[target_feature(enable = "avx2")]
        unsafe fn splat(f: &Fe) -> Self {
            let limbs = fe5_to_10(f);
            Self([
                _mm256_set1_epi64x(limbs[0] as i64),
                _mm256_set1_epi64x(limbs[1] as i64),
                _mm256_set1_epi64x(limbs[2] as i64),
                _mm256_set1_epi64x(limbs[3] as i64),
                _mm256_set1_epi64x(limbs[4] as i64),
                _mm256_set1_epi64x(limbs[5] as i64),
                _mm256_set1_epi64x(limbs[6] as i64),
                _mm256_set1_epi64x(limbs[7] as i64),
                _mm256_set1_epi64x(limbs[8] as i64),
                _mm256_set1_epi64x(limbs[9] as i64),
            ])
        }

        #[inline]
        #[target_feature(enable = "avx2")]
        unsafe fn from_4_fe(f: [&Fe; 4]) -> Self {
            let l0 = fe5_to_10(f[0]);
            let l1 = fe5_to_10(f[1]);
            let l2 = fe5_to_10(f[2]);
            let l3 = fe5_to_10(f[3]);
            let mut out = [_mm256_setzero_si256(); 10];
            for i in 0..10 {
                out[i] = _mm256_set_epi64x(l3[i] as i64, l2[i] as i64, l1[i] as i64, l0[i] as i64);
            }
            Self(out)
        }

        #[inline]
        #[target_feature(enable = "avx2")]
        unsafe fn add(a: &Self, b: &Self) -> Self {
            Self([
                _mm256_add_epi64(a.0[0], b.0[0]),
                _mm256_add_epi64(a.0[1], b.0[1]),
                _mm256_add_epi64(a.0[2], b.0[2]),
                _mm256_add_epi64(a.0[3], b.0[3]),
                _mm256_add_epi64(a.0[4], b.0[4]),
                _mm256_add_epi64(a.0[5], b.0[5]),
                _mm256_add_epi64(a.0[6], b.0[6]),
                _mm256_add_epi64(a.0[7], b.0[7]),
                _mm256_add_epi64(a.0[8], b.0[8]),
                _mm256_add_epi64(a.0[9], b.0[9]),
            ])
        }

        #[inline]
        #[target_feature(enable = "avx2")]
        unsafe fn sub(a: &Self, b: &Self) -> Self {
            let m26 = _mm256_set1_epi64x(MASK26 as i64);
            let m25 = _mm256_set1_epi64x(MASK25 as i64);
            let k19 = _mm256_set1_epi64x(19);

            let mut c0 = _mm256_sub_epi64(
                _mm256_add_epi64(a.0[0], _mm256_set1_epi64x(0x0fff_ffb4)),
                b.0[0],
            );
            let mut c1 = _mm256_sub_epi64(
                _mm256_add_epi64(a.0[1], _mm256_set1_epi64x(0x07ff_fffc)),
                b.0[1],
            );
            let mut c2 = _mm256_sub_epi64(
                _mm256_add_epi64(a.0[2], _mm256_set1_epi64x(0x0fff_fffc)),
                b.0[2],
            );
            let mut c3 = _mm256_sub_epi64(
                _mm256_add_epi64(a.0[3], _mm256_set1_epi64x(0x07ff_fffc)),
                b.0[3],
            );
            let mut c4 = _mm256_sub_epi64(
                _mm256_add_epi64(a.0[4], _mm256_set1_epi64x(0x0fff_fffc)),
                b.0[4],
            );
            let mut c5 = _mm256_sub_epi64(
                _mm256_add_epi64(a.0[5], _mm256_set1_epi64x(0x07ff_fffc)),
                b.0[5],
            );
            let mut c6 = _mm256_sub_epi64(
                _mm256_add_epi64(a.0[6], _mm256_set1_epi64x(0x0fff_fffc)),
                b.0[6],
            );
            let mut c7 = _mm256_sub_epi64(
                _mm256_add_epi64(a.0[7], _mm256_set1_epi64x(0x07ff_fffc)),
                b.0[7],
            );
            let mut c8 = _mm256_sub_epi64(
                _mm256_add_epi64(a.0[8], _mm256_set1_epi64x(0x0fff_fffc)),
                b.0[8],
            );
            let mut c9 = _mm256_sub_epi64(
                _mm256_add_epi64(a.0[9], _mm256_set1_epi64x(0x07ff_fffc)),
                b.0[9],
            );

            c1 = _mm256_add_epi64(c1, _mm256_srli_epi64::<26>(c0));
            c0 = _mm256_and_si256(c0, m26);
            c2 = _mm256_add_epi64(c2, _mm256_srli_epi64::<25>(c1));
            c1 = _mm256_and_si256(c1, m25);
            c3 = _mm256_add_epi64(c3, _mm256_srli_epi64::<26>(c2));
            c2 = _mm256_and_si256(c2, m26);
            c4 = _mm256_add_epi64(c4, _mm256_srli_epi64::<25>(c3));
            c3 = _mm256_and_si256(c3, m25);
            c5 = _mm256_add_epi64(c5, _mm256_srli_epi64::<26>(c4));
            c4 = _mm256_and_si256(c4, m26);
            c6 = _mm256_add_epi64(c6, _mm256_srli_epi64::<25>(c5));
            c5 = _mm256_and_si256(c5, m25);
            c7 = _mm256_add_epi64(c7, _mm256_srli_epi64::<26>(c6));
            c6 = _mm256_and_si256(c6, m26);
            c8 = _mm256_add_epi64(c8, _mm256_srli_epi64::<25>(c7));
            c7 = _mm256_and_si256(c7, m25);
            c9 = _mm256_add_epi64(c9, _mm256_srli_epi64::<26>(c8));
            c8 = _mm256_and_si256(c8, m26);
            c0 = _mm256_add_epi64(c0, _mm256_mul_epu32(_mm256_srli_epi64::<25>(c9), k19));
            c9 = _mm256_and_si256(c9, m25);
            c1 = _mm256_add_epi64(c1, _mm256_srli_epi64::<26>(c0));
            c0 = _mm256_and_si256(c0, m26);

            Self([c0, c1, c2, c3, c4, c5, c6, c7, c8, c9])
        }

        #[inline]
        #[target_feature(enable = "avx2")]
        unsafe fn mul(a: &Self, b: &Self) -> Self {
            let k19 = _mm256_set1_epi64x(19);
            let m26 = _mm256_set1_epi64x(MASK26 as i64);
            let m25 = _mm256_set1_epi64x(MASK25 as i64);

            let a0 = a.0[0];
            let a1 = a.0[1];
            let a2 = a.0[2];
            let a3 = a.0[3];
            let a4 = a.0[4];
            let a5 = a.0[5];
            let a6 = a.0[6];
            let a7 = a.0[7];
            let a8 = a.0[8];
            let a9 = a.0[9];

            let b0 = b.0[0];
            let b1 = b.0[1];
            let b2 = b.0[2];
            let b3 = b.0[3];
            let b4 = b.0[4];
            let b5 = b.0[5];
            let b6 = b.0[6];
            let b7 = b.0[7];
            let b8 = b.0[8];
            let b9 = b.0[9];

            let a1_2 = _mm256_slli_epi64::<1>(a1);
            let a3_2 = _mm256_slli_epi64::<1>(a3);
            let a5_2 = _mm256_slli_epi64::<1>(a5);
            let a7_2 = _mm256_slli_epi64::<1>(a7);
            let a9_2 = _mm256_slli_epi64::<1>(a9);

            let b1_19 = _mm256_mul_epu32(b1, k19);
            let b2_19 = _mm256_mul_epu32(b2, k19);
            let b3_19 = _mm256_mul_epu32(b3, k19);
            let b4_19 = _mm256_mul_epu32(b4, k19);
            let b5_19 = _mm256_mul_epu32(b5, k19);
            let b6_19 = _mm256_mul_epu32(b6, k19);
            let b7_19 = _mm256_mul_epu32(b7, k19);
            let b8_19 = _mm256_mul_epu32(b8, k19);
            let b9_19 = _mm256_mul_epu32(b9, k19);

            macro_rules! mac {
                ($acc:expr, $x:expr, $y:expr) => {
                    $acc = _mm256_add_epi64($acc, _mm256_mul_epu32($x, $y))
                };
            }

            let mut t0 = _mm256_mul_epu32(a0, b0);
            mac!(t0, a1_2, b9_19);
            mac!(t0, a2, b8_19);
            mac!(t0, a3_2, b7_19);
            mac!(t0, a4, b6_19);
            mac!(t0, a5_2, b5_19);
            mac!(t0, a6, b4_19);
            mac!(t0, a7_2, b3_19);
            mac!(t0, a8, b2_19);
            mac!(t0, a9_2, b1_19);

            let mut t1 = _mm256_mul_epu32(a0, b1);
            mac!(t1, a1, b0);
            mac!(t1, a2, b9_19);
            mac!(t1, a3, b8_19);
            mac!(t1, a4, b7_19);
            mac!(t1, a5, b6_19);
            mac!(t1, a6, b5_19);
            mac!(t1, a7, b4_19);
            mac!(t1, a8, b3_19);
            mac!(t1, a9, b2_19);

            let mut t2 = _mm256_mul_epu32(a0, b2);
            mac!(t2, a1_2, b1);
            mac!(t2, a2, b0);
            mac!(t2, a3_2, b9_19);
            mac!(t2, a4, b8_19);
            mac!(t2, a5_2, b7_19);
            mac!(t2, a6, b6_19);
            mac!(t2, a7_2, b5_19);
            mac!(t2, a8, b4_19);
            mac!(t2, a9_2, b3_19);

            let mut t3 = _mm256_mul_epu32(a0, b3);
            mac!(t3, a1, b2);
            mac!(t3, a2, b1);
            mac!(t3, a3, b0);
            mac!(t3, a4, b9_19);
            mac!(t3, a5, b8_19);
            mac!(t3, a6, b7_19);
            mac!(t3, a7, b6_19);
            mac!(t3, a8, b5_19);
            mac!(t3, a9, b4_19);

            let mut t4 = _mm256_mul_epu32(a0, b4);
            mac!(t4, a1_2, b3);
            mac!(t4, a2, b2);
            mac!(t4, a3_2, b1);
            mac!(t4, a4, b0);
            mac!(t4, a5_2, b9_19);
            mac!(t4, a6, b8_19);
            mac!(t4, a7_2, b7_19);
            mac!(t4, a8, b6_19);
            mac!(t4, a9_2, b5_19);

            let mut t5 = _mm256_mul_epu32(a0, b5);
            mac!(t5, a1, b4);
            mac!(t5, a2, b3);
            mac!(t5, a3, b2);
            mac!(t5, a4, b1);
            mac!(t5, a5, b0);
            mac!(t5, a6, b9_19);
            mac!(t5, a7, b8_19);
            mac!(t5, a8, b7_19);
            mac!(t5, a9, b6_19);

            let mut t6 = _mm256_mul_epu32(a0, b6);
            mac!(t6, a1_2, b5);
            mac!(t6, a2, b4);
            mac!(t6, a3_2, b3);
            mac!(t6, a4, b2);
            mac!(t6, a5_2, b1);
            mac!(t6, a6, b0);
            mac!(t6, a7_2, b9_19);
            mac!(t6, a8, b8_19);
            mac!(t6, a9_2, b7_19);

            let mut t7 = _mm256_mul_epu32(a0, b7);
            mac!(t7, a1, b6);
            mac!(t7, a2, b5);
            mac!(t7, a3, b4);
            mac!(t7, a4, b3);
            mac!(t7, a5, b2);
            mac!(t7, a6, b1);
            mac!(t7, a7, b0);
            mac!(t7, a8, b9_19);
            mac!(t7, a9, b8_19);

            let mut t8 = _mm256_mul_epu32(a0, b8);
            mac!(t8, a1_2, b7);
            mac!(t8, a2, b6);
            mac!(t8, a3_2, b5);
            mac!(t8, a4, b4);
            mac!(t8, a5_2, b3);
            mac!(t8, a6, b2);
            mac!(t8, a7_2, b1);
            mac!(t8, a8, b0);
            mac!(t8, a9_2, b9_19);

            let mut t9 = _mm256_mul_epu32(a0, b9);
            mac!(t9, a1, b8);
            mac!(t9, a2, b7);
            mac!(t9, a3, b6);
            mac!(t9, a4, b5);
            mac!(t9, a5, b4);
            mac!(t9, a6, b3);
            mac!(t9, a7, b2);
            mac!(t9, a8, b1);
            mac!(t9, a9, b0);

            t1 = _mm256_add_epi64(t1, _mm256_srli_epi64::<26>(t0));
            let mut r0 = _mm256_and_si256(t0, m26);
            t2 = _mm256_add_epi64(t2, _mm256_srli_epi64::<25>(t1));
            let mut r1 = _mm256_and_si256(t1, m25);
            t3 = _mm256_add_epi64(t3, _mm256_srli_epi64::<26>(t2));
            let mut r2 = _mm256_and_si256(t2, m26);
            t4 = _mm256_add_epi64(t4, _mm256_srli_epi64::<25>(t3));
            let r3 = _mm256_and_si256(t3, m25);
            t5 = _mm256_add_epi64(t5, _mm256_srli_epi64::<26>(t4));
            let r4 = _mm256_and_si256(t4, m26);
            t6 = _mm256_add_epi64(t6, _mm256_srli_epi64::<25>(t5));
            let r5 = _mm256_and_si256(t5, m25);
            t7 = _mm256_add_epi64(t7, _mm256_srli_epi64::<26>(t6));
            let r6 = _mm256_and_si256(t6, m26);
            t8 = _mm256_add_epi64(t8, _mm256_srli_epi64::<25>(t7));
            let r7 = _mm256_and_si256(t7, m25);
            t9 = _mm256_add_epi64(t9, _mm256_srli_epi64::<26>(t8));
            let r8 = _mm256_and_si256(t8, m26);

            let c9 = _mm256_srli_epi64::<25>(t9);
            let r9 = _mm256_and_si256(t9, m25);
            let r0_64 = _mm256_add_epi64(r0, _mm256_mul_epu32(c9, k19));
            r0 = _mm256_and_si256(r0_64, m26);
            r1 = _mm256_add_epi64(r1, _mm256_srli_epi64::<26>(r0_64));
            let r2_carry = _mm256_srli_epi64::<25>(r1);
            r1 = _mm256_and_si256(r1, m25);
            r2 = _mm256_add_epi64(r2, r2_carry);

            Self([r0, r1, r2, r3, r4, r5, r6, r7, r8, r9])
        }

        #[inline]
        #[target_feature(enable = "avx2")]
        unsafe fn sq(&self) -> Self {
            let k19 = _mm256_set1_epi64x(19);
            let m26 = _mm256_set1_epi64x(MASK26 as i64);
            let m25 = _mm256_set1_epi64x(MASK25 as i64);

            let a0 = self.0[0];
            let a1 = self.0[1];
            let a2 = self.0[2];
            let a3 = self.0[3];
            let a4 = self.0[4];
            let a5 = self.0[5];
            let a6 = self.0[6];
            let a7 = self.0[7];
            let a8 = self.0[8];
            let a9 = self.0[9];

            let a0_2 = _mm256_slli_epi64::<1>(a0);
            let a1_2 = _mm256_slli_epi64::<1>(a1);
            let a2_2 = _mm256_slli_epi64::<1>(a2);
            let a3_2 = _mm256_slli_epi64::<1>(a3);
            let a4_2 = _mm256_slli_epi64::<1>(a4);
            let a5_2 = _mm256_slli_epi64::<1>(a5);
            let a6_2 = _mm256_slli_epi64::<1>(a6);
            let a7_2 = _mm256_slli_epi64::<1>(a7);
            let a8_2 = _mm256_slli_epi64::<1>(a8);
            let a9_2 = _mm256_slli_epi64::<1>(a9);

            let a1_4 = _mm256_slli_epi64::<2>(a1);
            let a3_4 = _mm256_slli_epi64::<2>(a3);
            let a5_4 = _mm256_slli_epi64::<2>(a5);
            let a7_4 = _mm256_slli_epi64::<2>(a7);

            let a5_19 = _mm256_mul_epu32(a5, k19);
            let a6_19 = _mm256_mul_epu32(a6, k19);
            let a7_19 = _mm256_mul_epu32(a7, k19);
            let a8_19 = _mm256_mul_epu32(a8, k19);
            let a9_19 = _mm256_mul_epu32(a9, k19);

            macro_rules! mac {
                ($acc:expr, $x:expr, $y:expr) => {
                    $acc = _mm256_add_epi64($acc, _mm256_mul_epu32($x, $y))
                };
            }

            let mut t0 = _mm256_mul_epu32(a0, a0);
            mac!(t0, a1_4, a9_19);
            mac!(t0, a2_2, a8_19);
            mac!(t0, a3_4, a7_19);
            mac!(t0, a4_2, a6_19);
            mac!(t0, a5_2, a5_19);

            let mut t1 = _mm256_mul_epu32(a0_2, a1);
            mac!(t1, a2_2, a9_19);
            mac!(t1, a3_2, a8_19);
            mac!(t1, a4_2, a7_19);
            mac!(t1, a5_2, a6_19);

            let mut t2 = _mm256_mul_epu32(a0_2, a2);
            mac!(t2, a1_2, a1);
            mac!(t2, a3_4, a9_19);
            mac!(t2, a4_2, a8_19);
            mac!(t2, a5_4, a7_19);
            mac!(t2, a6, a6_19);

            let mut t3 = _mm256_mul_epu32(a0_2, a3);
            mac!(t3, a1_2, a2);
            mac!(t3, a4_2, a9_19);
            mac!(t3, a5_2, a8_19);
            mac!(t3, a6_2, a7_19);

            let mut t4 = _mm256_mul_epu32(a0_2, a4);
            mac!(t4, a1_2, a3_2);
            mac!(t4, a2, a2);
            mac!(t4, a5_4, a9_19);
            mac!(t4, a6_2, a8_19);
            mac!(t4, a7_2, a7_19);

            let mut t5 = _mm256_mul_epu32(a0_2, a5);
            mac!(t5, a1_2, a4);
            mac!(t5, a2_2, a3);
            mac!(t5, a6_2, a9_19);
            mac!(t5, a7_2, a8_19);

            let mut t6 = _mm256_mul_epu32(a0_2, a6);
            mac!(t6, a1_2, a5_2);
            mac!(t6, a2_2, a4);
            mac!(t6, a3_2, a3);
            mac!(t6, a7_4, a9_19);
            mac!(t6, a8, a8_19);

            let mut t7 = _mm256_mul_epu32(a0_2, a7);
            mac!(t7, a1_2, a6);
            mac!(t7, a2_2, a5);
            mac!(t7, a3_2, a4);
            mac!(t7, a8_2, a9_19);

            let mut t8 = _mm256_mul_epu32(a0_2, a8);
            mac!(t8, a1_2, a7_2);
            mac!(t8, a2_2, a6);
            mac!(t8, a3_2, a5_2);
            mac!(t8, a4, a4);
            mac!(t8, a9_2, a9_19);

            let mut t9 = _mm256_mul_epu32(a0_2, a9);
            mac!(t9, a1_2, a8);
            mac!(t9, a2_2, a7);
            mac!(t9, a3_2, a6);
            mac!(t9, a4_2, a5);

            t1 = _mm256_add_epi64(t1, _mm256_srli_epi64::<26>(t0));
            let mut r0 = _mm256_and_si256(t0, m26);
            t2 = _mm256_add_epi64(t2, _mm256_srli_epi64::<25>(t1));
            let mut r1 = _mm256_and_si256(t1, m25);
            t3 = _mm256_add_epi64(t3, _mm256_srli_epi64::<26>(t2));
            let mut r2 = _mm256_and_si256(t2, m26);
            t4 = _mm256_add_epi64(t4, _mm256_srli_epi64::<25>(t3));
            let r3 = _mm256_and_si256(t3, m25);
            t5 = _mm256_add_epi64(t5, _mm256_srli_epi64::<26>(t4));
            let r4 = _mm256_and_si256(t4, m26);
            t6 = _mm256_add_epi64(t6, _mm256_srli_epi64::<25>(t5));
            let r5 = _mm256_and_si256(t5, m25);
            t7 = _mm256_add_epi64(t7, _mm256_srli_epi64::<26>(t6));
            let r6 = _mm256_and_si256(t6, m26);
            t8 = _mm256_add_epi64(t8, _mm256_srli_epi64::<25>(t7));
            let r7 = _mm256_and_si256(t7, m25);
            t9 = _mm256_add_epi64(t9, _mm256_srli_epi64::<26>(t8));
            let r8 = _mm256_and_si256(t8, m26);

            let c9 = _mm256_srli_epi64::<25>(t9);
            let r9 = _mm256_and_si256(t9, m25);
            let r0_64 = _mm256_add_epi64(r0, _mm256_mul_epu32(c9, k19));
            r0 = _mm256_and_si256(r0_64, m26);
            r1 = _mm256_add_epi64(r1, _mm256_srli_epi64::<26>(r0_64));
            let r2_carry = _mm256_srli_epi64::<25>(r1);
            r1 = _mm256_and_si256(r1, m25);
            r2 = _mm256_add_epi64(r2, r2_carry);

            Self([r0, r1, r2, r3, r4, r5, r6, r7, r8, r9])
        }

        #[inline]
        #[target_feature(enable = "avx2")]
        unsafe fn invert(&self) -> Self {
            unsafe {
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
        }

        /// Computes the canonical first 8 bytes (big-endian u64) for all 4 lanes.
        #[inline]
        #[target_feature(enable = "avx2")]
        unsafe fn first_u64_be_4(&self) -> [u64; 4] {
            unsafe {
                let m26 = _mm256_set1_epi64x(MASK26 as i64);
                let m25 = _mm256_set1_epi64x(MASK25 as i64);
                let k19 = _mm256_set1_epi64x(19);

                let mut h = self.0;
                let mut q = _mm256_srli_epi64::<26>(_mm256_add_epi64(
                    h[0],
                    _mm256_mul_epu32(_mm256_srli_epi64::<25>(h[9]), k19),
                ));
                q = _mm256_srli_epi64::<25>(_mm256_add_epi64(h[1], q));
                q = _mm256_srli_epi64::<26>(_mm256_add_epi64(h[2], q));
                q = _mm256_srli_epi64::<25>(_mm256_add_epi64(h[3], q));
                q = _mm256_srli_epi64::<26>(_mm256_add_epi64(h[4], q));
                q = _mm256_srli_epi64::<25>(_mm256_add_epi64(h[5], q));
                q = _mm256_srli_epi64::<26>(_mm256_add_epi64(h[6], q));
                q = _mm256_srli_epi64::<25>(_mm256_add_epi64(h[7], q));
                q = _mm256_srli_epi64::<26>(_mm256_add_epi64(h[8], q));
                q = _mm256_srli_epi64::<25>(_mm256_add_epi64(h[9], q));

                h[0] = _mm256_add_epi64(h[0], _mm256_mul_epu32(q, k19));
                h[1] = _mm256_add_epi64(h[1], _mm256_srli_epi64::<26>(h[0]));
                h[0] = _mm256_and_si256(h[0], m26);
                h[2] = _mm256_add_epi64(h[2], _mm256_srli_epi64::<25>(h[1]));
                h[1] = _mm256_and_si256(h[1], m25);
                h[3] = _mm256_add_epi64(h[3], _mm256_srli_epi64::<26>(h[2]));
                h[2] = _mm256_and_si256(h[2], m26);
                h[4] = _mm256_add_epi64(h[4], _mm256_srli_epi64::<25>(h[3]));
                h[3] = _mm256_and_si256(h[3], m25);
                h[5] = _mm256_add_epi64(h[5], _mm256_srli_epi64::<26>(h[4]));
                h[4] = _mm256_and_si256(h[4], m26);
                h[6] = _mm256_add_epi64(h[6], _mm256_srli_epi64::<25>(h[5]));
                h[5] = _mm256_and_si256(h[5], m25);
                h[7] = _mm256_add_epi64(h[7], _mm256_srli_epi64::<26>(h[6]));
                h[6] = _mm256_and_si256(h[6], m26);
                h[8] = _mm256_add_epi64(h[8], _mm256_srli_epi64::<25>(h[7]));
                h[7] = _mm256_and_si256(h[7], m25);
                h[9] = _mm256_add_epi64(h[9], _mm256_srli_epi64::<26>(h[8]));
                h[8] = _mm256_and_si256(h[8], m26);
                h[9] = _mm256_and_si256(h[9], m25);

                let mut t0 = _mm256_add_epi64(h[0], k19);
                let mut t1 = _mm256_add_epi64(h[1], _mm256_srli_epi64::<26>(t0));
                t0 = _mm256_and_si256(t0, m26);
                let mut t2 = _mm256_add_epi64(h[2], _mm256_srli_epi64::<25>(t1));
                t1 = _mm256_and_si256(t1, m25);
                let t3 = _mm256_add_epi64(h[3], _mm256_srli_epi64::<26>(t2));
                t2 = _mm256_and_si256(t2, m26);
                let t4 = _mm256_add_epi64(h[4], _mm256_srli_epi64::<25>(t3));
                let t5 = _mm256_add_epi64(h[5], _mm256_srli_epi64::<26>(t4));
                let t6 = _mm256_add_epi64(h[6], _mm256_srli_epi64::<25>(t5));
                let t7 = _mm256_add_epi64(h[7], _mm256_srli_epi64::<26>(t6));
                let t8 = _mm256_add_epi64(h[8], _mm256_srli_epi64::<25>(t7));
                let t9 = _mm256_add_epi64(h[9], _mm256_srli_epi64::<26>(t8));
                let overflow = _mm256_srli_epi64::<25>(t9);
                let mask = _mm256_sub_epi64(_mm256_setzero_si256(), overflow);

                let r0 =
                    _mm256_or_si256(_mm256_andnot_si256(mask, h[0]), _mm256_and_si256(mask, t0));
                let r1 =
                    _mm256_or_si256(_mm256_andnot_si256(mask, h[1]), _mm256_and_si256(mask, t1));
                let r2 =
                    _mm256_or_si256(_mm256_andnot_si256(mask, h[2]), _mm256_and_si256(mask, t2));

                // First 64 bits: r0[26] | r1[25] | r2[13]
                let packed = _mm256_or_si256(
                    r0,
                    _mm256_or_si256(_mm256_slli_epi64::<26>(r1), _mm256_slli_epi64::<51>(r2)),
                );
                let mut out = [0u64; 4];
                _mm256_storeu_si256(out.as_mut_ptr().cast::<__m256i>(), packed);
                [
                    out[0].swap_bytes(),
                    out[1].swap_bytes(),
                    out[2].swap_bytes(),
                    out[3].swap_bytes(),
                ]
            }
        }
    }

    #[inline]
    fn fe5_to_10(f: &Fe) -> [u64; 10] {
        let l = f.0;
        [
            l[0] & MASK26,
            (l[0] >> 26) & MASK25,
            l[1] & MASK26,
            (l[1] >> 26) & MASK25,
            l[2] & MASK26,
            (l[2] >> 26) & MASK25,
            l[3] & MASK26,
            (l[3] >> 26) & MASK25,
            l[4] & MASK26,
            (l[4] >> 26) & MASK25,
        ]
    }

    #[derive(Clone, Copy)]
    struct MontPoint4 {
        u: Fe4,
        w: Fe4,
    }

    impl MontPoint4 {
        #[inline]
        #[target_feature(enable = "avx2")]
        unsafe fn step_with(
            &self,
            prev: &Self,
            step_u_plus_w: &Fe4,
            step_u_minus_w: &Fe4,
        ) -> (Self, Fe4, Fe4) {
            unsafe {
                let a = Fe4::add(&self.u, &self.w);
                let b = Fe4::sub(&self.u, &self.w);
                let da = Fe4::mul(step_u_minus_w, &a);
                let cb = Fe4::mul(step_u_plus_w, &b);
                let sum = Fe4::add(&da, &cb);
                let diff = Fe4::sub(&da, &cb);
                let u_next = Fe4::mul(&prev.w, &sum.sq());
                let w_next = Fe4::mul(&prev.u, &diff.sq());
                (
                    Self {
                        u: u_next,
                        w: w_next,
                    },
                    a,
                    b,
                )
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub(in crate::core::engine) unsafe fn worker_loop_avx2(
        thread_idx: usize,
        matcher: &PrefixMatcher,
        step_mont: MontStep,
        telemetry: &SharedTelemetry,
        on_milestone: &Sender<MintMilestone>,
        cancel: &AtomicBool,
    ) {
        unsafe {
            let mut rng = rand::rng();
            let mut local_best = telemetry.global_best_len.load(Ordering::Relaxed) as usize;
            let target_len = matcher.len();

            let step_u_plus_w = Fe4::splat(&step_mont.u_plus_w);
            let step_u_minus_w = Fe4::splat(&step_mont.u_minus_w);

            let zero = Fe4::zero();
            let mut denom = [zero; BATCH_PER_STREAM];
            let mut numer = [zero; BATCH_PER_STREAM];
            let mut prod = [zero; BATCH_PER_STREAM];

            let mut local_hashes: u64 = 0;
            let mut batch_count: usize = 0;

            let mut needed_chars = (local_best + 1).min(target_len);
            let (mut mask64, mut expected64) = matcher.fast_mask_u64(needed_chars);

            let mut seeds = [[0u8; 32]; 4];
            let mut base_scalars = [Scalar::ZERO; 4];

            let step8 = Scalar::from(8u64);

            while !cancel.load(Ordering::Relaxed) && local_best < target_len {
                let mut prev_pts = [MontPoint {
                    u: Fe::ZERO,
                    w: Fe::ZERO,
                }; 4];
                let mut cur_pts = [MontPoint {
                    u: Fe::ZERO,
                    w: Fe::ZERO,
                }; 4];

                for s in 0..4 {
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
                    cur_pts[s] = MontPoint::from_dalek(&p0);
                    prev_pts[s] = MontPoint::from_dalek(&p_prev);
                }

                let mut prev4 = MontPoint4 {
                    u: Fe4::from_4_fe([
                        &prev_pts[0].u,
                        &prev_pts[1].u,
                        &prev_pts[2].u,
                        &prev_pts[3].u,
                    ]),
                    w: Fe4::from_4_fe([
                        &prev_pts[0].w,
                        &prev_pts[1].w,
                        &prev_pts[2].w,
                        &prev_pts[3].w,
                    ]),
                };
                let mut cur4 = MontPoint4 {
                    u: Fe4::from_4_fe([&cur_pts[0].u, &cur_pts[1].u, &cur_pts[2].u, &cur_pts[3].u]),
                    w: Fe4::from_4_fe([&cur_pts[0].w, &cur_pts[1].w, &cur_pts[2].w, &cur_pts[3].w]),
                };

                let mut step_index: u64 = 0;

                while step_index < RESEED_STEPS
                    && !cancel.load(Ordering::Relaxed)
                    && local_best < target_len
                {
                    let batch_start_step = step_index;
                    for k in 0..BATCH_PER_STREAM {
                        let (next4, a, b) = cur4.step_with(&prev4, &step_u_plus_w, &step_u_minus_w);
                        denom[k] = a;
                        numer[k] = b;
                        prev4 = cur4;
                        cur4 = next4;
                    }
                    step_index += BATCH_PER_STREAM as u64;

                    // 4-lane Montgomery batch inversion: 1 inversion per 256 keys (4 * 64).
                    prod[0] = denom[0];
                    for k in 1..BATCH_PER_STREAM {
                        prod[k] = Fe4::mul(&prod[k - 1], &denom[k]);
                    }
                    let mut inv = prod[BATCH_PER_STREAM - 1].invert();

                    for k in (0..BATCH_PER_STREAM).rev() {
                        let d_inv = if k == 0 {
                            inv
                        } else {
                            let di = Fe4::mul(&inv, &prod[k - 1]);
                            inv = Fe4::mul(&inv, &denom[k]);
                            di
                        };

                        let y_affine = Fe4::mul(&numer[k], &d_inv);
                        let top4 = y_affine.first_u64_be_4();

                        for (s, &top) in top4.iter().enumerate() {
                            if (top & mask64) != expected64 {
                                continue;
                            }

                            let exact_step = batch_start_step + (k as u64);
                            let scalar = base_scalars[s] + Scalar::from(exact_step.wrapping_mul(8));
                            let full_pt = ED25519_BASEPOINT_POINT * scalar;
                            let pubkey = full_pt.compress().to_bytes();

                            let matched = matcher.matched_len(&pubkey);
                            if matched > local_best || (target_len == 52 && matched == 51) {
                                let global_best =
                                    telemetry.global_best_len.load(Ordering::Acquire) as usize;
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
                    }

                    local_hashes += (4 * BATCH_PER_STREAM) as u64;
                    batch_count += 1;

                    if batch_count >= TELEMETRY_FLUSH_BATCHES {
                        telemetry.thread_hashes[thread_idx]
                            .0
                            .fetch_add(local_hashes, Ordering::Relaxed);
                        local_hashes = 0;
                        batch_count = 0;
                        let global_best =
                            telemetry.global_best_len.load(Ordering::Relaxed) as usize;
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
    }
}
