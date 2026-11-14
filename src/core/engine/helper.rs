//! Shared Ed25519 field (`GF(2^255 - 19)`), extended Twisted Edwards
//! point arithmetic, and key-derivation helpers used by `CpuMkEngine` and
//! `GpuMkEngine`.

use curve25519_dalek::edwards::EdwardsPoint;
use curve25519_dalek::scalar::Scalar;
use sha2::{Digest, Sha512};

pub(super) fn derive_expanded_secret(seed: &[u8; 32], step: u64, scalar: &Scalar) -> [u8; 64] {
    let mut nonce_hasher = Sha512::new();
    nonce_hasher.update(seed);
    nonce_hasher.update(step.to_le_bytes());
    let nonce_digest = nonce_hasher.finalize();

    let mut out = [0u8; 64];
    out[..32].copy_from_slice(&scalar.to_bytes());
    out[32..].copy_from_slice(&nonce_digest[32..64]);
    out
}

const MASK51: u64 = (1u64 << 51) - 1;

#[cfg(any(feature = "gpu", test))]
/// `d = -121665 / 121666 mod (2^255 - 19)`
const EDWARDS_D: Fe = Fe([
    0x0003_4dca_1359_78a3,
    0x0001_a828_3b15_6ebd,
    0x0005_e7a2_6001_c029,
    0x0007_39c6_63a0_3cbb,
    0x0005_2036_cee2_b6ff,
]);

#[derive(Clone, Copy)]
pub(super) struct Fe(pub(super) [u64; 5]);

impl Fe {
    pub(super) const ZERO: Self = Self([0, 0, 0, 0, 0]);
    pub(super) const ONE: Self = Self([1, 0, 0, 0, 0]);

    #[inline(always)]
    pub(super) fn add(a: &Self, b: &Self) -> Self {
        Self([
            a.0[0] + b.0[0],
            a.0[1] + b.0[1],
            a.0[2] + b.0[2],
            a.0[3] + b.0[3],
            a.0[4] + b.0[4],
        ])
    }

    #[inline(always)]
    pub(super) fn sub(a: &Self, b: &Self) -> Self {
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
    pub(super) fn mul(a: &Self, b: &Self) -> Self {
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
    pub(super) fn sq(&self) -> Self {
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

    #[cfg(any(feature = "gpu", test))]
    pub(super) fn pow22523(&self) -> Self {
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
    pub(super) fn invert(&self) -> Self {
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

    pub(super) fn from_bytes(bytes: &[u8; 32]) -> Self {
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

    #[cfg(any(feature = "gpu", test))]
    #[inline(always)]
    pub(super) fn to_bytes(self) -> [u8; 32] {
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
        h[4] &= MASK51;

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
    pub(super) fn first_u64_be(self) -> u64 {
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

#[cfg(any(feature = "gpu", test))]
#[derive(Clone, Copy)]
pub(super) struct ExtPoint {
    pub(super) x: Fe,
    pub(super) y: Fe,
    pub(super) z: Fe,
    pub(super) t: Fe,
}

#[cfg(any(feature = "gpu", test))]
#[derive(Clone, Copy)]
pub(super) struct CachedStep {
    pub(super) y_plus_x: Fe,
    pub(super) y_minus_x: Fe,
    pub(super) t2d: Fe,
}

#[cfg(any(feature = "gpu", test))]
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

#[cfg(any(feature = "gpu", test))]
impl ExtPoint {
    pub(super) fn from_dalek(p: &EdwardsPoint) -> Self {
        let (x, y) = decompress_dalek(p);
        Self {
            x,
            y,
            z: Fe::ONE,
            t: Fe::mul(&x, &y),
        }
    }

    #[cfg(test)]
    /// Unified extended-plus-cached addition where cached point has `Z = 1`.
    #[inline(always)]
    pub(super) fn add_cached(&self, q: &CachedStep) -> Self {
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

#[cfg(any(feature = "gpu", test))]
impl CachedStep {
    pub(super) fn from_dalek(p: &EdwardsPoint) -> Self {
        let (x, y) = decompress_dalek(p);
        let t = Fe::mul(&x, &y);
        Self {
            y_plus_x: Fe::add(&y, &x),
            y_minus_x: Fe::sub(&y, &x),
            t2d: Fe::mul(&t, &Fe::add(&EDWARDS_D, &EDWARDS_D)),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct MontPoint {
    pub(super) u: Fe,
    pub(super) w: Fe,
}

#[derive(Clone, Copy)]
pub(super) struct MontStep {
    pub(super) u_plus_w: Fe,
    pub(super) u_minus_w: Fe,
}

impl MontPoint {
    /// Converts a `curve25519_dalek::EdwardsPoint` to Montgomery `(U : W) = (1 + y : 1 - y)`.
    pub(super) fn from_dalek(p: &EdwardsPoint) -> Self {
        let mut bytes = *p.compress().as_bytes();
        bytes[31] &= 0x7f;
        let y = Fe::from_bytes(&bytes);
        Self {
            u: Fe::add(&Fe::ONE, &y),
            w: Fe::sub(&Fe::ONE, &y),
        }
    }

    /// Differential addition `self + step` given `prev = self - step`.
    /// Returns `(next, u_plus_w, u_minus_w)` so the caller can reuse `u_plus_w` and `u_minus_w`
    /// for `y = (U - W) / (U + W)` without recomputing them.
    #[inline(always)]
    pub(super) fn step_with(&self, prev: &Self, step: &MontStep) -> (Self, Fe, Fe) {
        let a = Fe::add(&self.u, &self.w);
        let b = Fe::sub(&self.u, &self.w);
        let da = Fe::mul(&step.u_minus_w, &a);
        let cb = Fe::mul(&step.u_plus_w, &b);
        let sum = Fe::add(&da, &cb);
        let diff = Fe::sub(&da, &cb);
        let u_next = Fe::mul(&prev.w, &sum.sq());
        let w_next = Fe::mul(&prev.u, &diff.sq());
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

impl MontStep {
    pub(super) fn from_dalek(p: &EdwardsPoint) -> Self {
        let mp = MontPoint::from_dalek(p);
        Self {
            u_plus_w: Fe::add(&mp.u, &mp.w),
            u_minus_w: Fe::sub(&mp.u, &mp.w),
        }
    }
}
