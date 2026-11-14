// Ed25519 vanity .onion address minting compute shader.
//
// Field arithmetic over GF(2^255 - 19) using 10 limbs in radix 2^25.5
// (alternating 26-bit and 25-bit capacities in u32) with pure u32
// 32x32 -> 64-bit multiplication emulation.
//
// Workgroup size is 128 threads; each step advances every thread's ExtPoint
// by 8*B via add_cached, then amortizes 1 field inversion across all 128
// threads using parallel prefix/suffix products in workgroup shared memory.

const WG_SIZE: u32 = 128u;
const MASK26: u32 = 0x03ffffffu;
const MASK25: u32 = 0x01ffffffu;

struct Fe {
    v: array<u32, 10>,
}

struct ExtPoint {
    x: Fe,
    y: Fe,
    z: Fe,
    t: Fe,
}

struct CachedStep {
    y_plus_x: Fe,
    y_minus_x: Fe,
    t2d: Fe,
}

struct MintParams {
    mask_hi: u32,
    mask_lo: u32,
    expected_hi: u32,
    expected_lo: u32,
    steps_per_dispatch: u32,
    base_step_offset: u32,
    _pad0: u32,
    _pad1: u32,
}

struct Hit {
    thread_id: u32,
    step_offset: u32,
    pubkey: array<u32, 8>,
}

struct HitBuffer {
    count: atomic<u32>,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    hits: array<Hit, 64>,
}

@group(0) @binding(0) var<uniform> params: MintParams;
@group(0) @binding(1) var<storage, read> step_cached: CachedStep;
@group(0) @binding(2) var<storage, read_write> points: array<ExtPoint>;
@group(0) @binding(3) var<storage, read_write> hit_buf: HitBuffer;

const BATCH_SIZE: u32 = 8u;

fn fe_zero() -> Fe {
    return Fe(array<u32, 10>(0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u));
}

fn fe_one() -> Fe {
    return Fe(array<u32, 10>(1u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u));
}

fn mul32_to_64(a: u32, b: u32) -> vec2<u32> {
    let a_lo = a & 0xffffu;
    let a_hi = a >> 16u;
    let b_lo = b & 0xffffu;
    let b_hi = b >> 16u;

    let p0 = a_lo * b_lo;
    let p1 = a_lo * b_hi;
    let p2 = a_hi * b_lo;
    let p3 = a_hi * b_hi;

    let mid = (p0 >> 16u) + (p1 & 0xffffu) + (p2 & 0xffffu);
    let lo = (p0 & 0xffffu) | ((mid & 0xffffu) << 16u);
    let hi = p3 + (p1 >> 16u) + (p2 >> 16u) + (mid >> 16u);
    return vec2<u32>(lo, hi);
}

fn add64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let lo = a.x + b.x;
    let carry = select(0u, 1u, lo < a.x);
    let hi = a.y + b.y + carry;
    return vec2<u32>(lo, hi);
}

fn shr64(a: vec2<u32>, shift: u32) -> vec2<u32> {
    return vec2<u32>((a.x >> shift) | (a.y << (32u - shift)), a.y >> shift);
}

fn fe_add(a: Fe, b: Fe) -> Fe {
    var r: Fe;
    for (var i = 0u; i < 10u; i = i + 1u) {
        r.v[i] = a.v[i] + b.v[i];
    }
    return r;
}

// Subtract with 4*p bias so all limbs remain positive, then carry-reduce
// to canonical limb bounds (< 2^26 even, < 2^25 odd).
fn fe_sub(a: Fe, b: Fe) -> Fe {
    var c0 = a.v[0] + 0x0fffffb4u - b.v[0];
    var c1 = a.v[1] + 0x07fffffcu - b.v[1];
    var c2 = a.v[2] + 0x0ffffffcu - b.v[2];
    var c3 = a.v[3] + 0x07fffffcu - b.v[3];
    var c4 = a.v[4] + 0x0ffffffcu - b.v[4];
    var c5 = a.v[5] + 0x07fffffcu - b.v[5];
    var c6 = a.v[6] + 0x0ffffffcu - b.v[6];
    var c7 = a.v[7] + 0x07fffffcu - b.v[7];
    var c8 = a.v[8] + 0x0ffffffcu - b.v[8];
    var c9 = a.v[9] + 0x07fffffcu - b.v[9];

    c1 = c1 + (c0 >> 26u); c0 = c0 & MASK26;
    c2 = c2 + (c1 >> 25u); c1 = c1 & MASK25;
    c3 = c3 + (c2 >> 26u); c2 = c2 & MASK26;
    c4 = c4 + (c3 >> 25u); c3 = c3 & MASK25;
    c5 = c5 + (c4 >> 26u); c4 = c4 & MASK26;
    c6 = c6 + (c5 >> 25u); c5 = c5 & MASK25;
    c7 = c7 + (c6 >> 26u); c6 = c6 & MASK26;
    c8 = c8 + (c7 >> 25u); c7 = c7 & MASK25;
    c9 = c9 + (c8 >> 26u); c8 = c8 & MASK26;
    c0 = c0 + (c9 >> 25u) * 19u; c9 = c9 & MASK25;
    c1 = c1 + (c0 >> 26u); c0 = c0 & MASK26;

    return Fe(array<u32, 10>(c0, c1, c2, c3, c4, c5, c6, c7, c8, c9));
}

fn fe_mul(a: Fe, b: Fe) -> Fe {
    let a0 = a.v[0]; let a1 = a.v[1]; let a2 = a.v[2]; let a3 = a.v[3]; let a4 = a.v[4];
    let a5 = a.v[5]; let a6 = a.v[6]; let a7 = a.v[7]; let a8 = a.v[8]; let a9 = a.v[9];

    let b0 = b.v[0]; let b1 = b.v[1]; let b2 = b.v[2]; let b3 = b.v[3]; let b4 = b.v[4];
    let b5 = b.v[5]; let b6 = b.v[6]; let b7 = b.v[7]; let b8 = b.v[8]; let b9 = b.v[9];

    let a1_2 = a1 * 2u;
    let a3_2 = a3 * 2u;
    let a5_2 = a5 * 2u;
    let a7_2 = a7 * 2u;
    let a9_2 = a9 * 2u;

    let b1_19 = b1 * 19u;
    let b2_19 = b2 * 19u;
    let b3_19 = b3 * 19u;
    let b4_19 = b4 * 19u;
    let b5_19 = b5 * 19u;
    let b6_19 = b6 * 19u;
    let b7_19 = b7 * 19u;
    let b8_19 = b8 * 19u;
    let b9_19 = b9 * 19u;

    let b1_38 = b1_19 * 2u;
    let b3_38 = b3_19 * 2u;
    let b5_38 = b5_19 * 2u;
    let b7_38 = b7_19 * 2u;
    let b9_38 = b9_19 * 2u;

    var t0 = mul32_to_64(a0, b0);
    t0 = add64(t0, mul32_to_64(a1_2, b9_19));
    t0 = add64(t0, mul32_to_64(a2, b8_19));
    t0 = add64(t0, mul32_to_64(a3_2, b7_19));
    t0 = add64(t0, mul32_to_64(a4, b6_19));
    t0 = add64(t0, mul32_to_64(a5_2, b5_19));
    t0 = add64(t0, mul32_to_64(a6, b4_19));
    t0 = add64(t0, mul32_to_64(a7_2, b3_19));
    t0 = add64(t0, mul32_to_64(a8, b2_19));
    t0 = add64(t0, mul32_to_64(a9_2, b1_19));

    var t1 = mul32_to_64(a0, b1);
    t1 = add64(t1, mul32_to_64(a1, b0));
    t1 = add64(t1, mul32_to_64(a2, b9_19));
    t1 = add64(t1, mul32_to_64(a3, b8_19));
    t1 = add64(t1, mul32_to_64(a4, b7_19));
    t1 = add64(t1, mul32_to_64(a5, b6_19));
    t1 = add64(t1, mul32_to_64(a6, b5_19));
    t1 = add64(t1, mul32_to_64(a7, b4_19));
    t1 = add64(t1, mul32_to_64(a8, b3_19));
    t1 = add64(t1, mul32_to_64(a9, b2_19));

    var t2 = mul32_to_64(a0, b2);
    t2 = add64(t2, mul32_to_64(a1_2, b1));
    t2 = add64(t2, mul32_to_64(a2, b0));
    t2 = add64(t2, mul32_to_64(a3_2, b9_19));
    t2 = add64(t2, mul32_to_64(a4, b8_19));
    t2 = add64(t2, mul32_to_64(a5_2, b7_19));
    t2 = add64(t2, mul32_to_64(a6, b6_19));
    t2 = add64(t2, mul32_to_64(a7_2, b5_19));
    t2 = add64(t2, mul32_to_64(a8, b4_19));
    t2 = add64(t2, mul32_to_64(a9_2, b3_19));

    var t3 = mul32_to_64(a0, b3);
    t3 = add64(t3, mul32_to_64(a1, b2));
    t3 = add64(t3, mul32_to_64(a2, b1));
    t3 = add64(t3, mul32_to_64(a3, b0));
    t3 = add64(t3, mul32_to_64(a4, b9_19));
    t3 = add64(t3, mul32_to_64(a5, b8_19));
    t3 = add64(t3, mul32_to_64(a6, b7_19));
    t3 = add64(t3, mul32_to_64(a7, b6_19));
    t3 = add64(t3, mul32_to_64(a8, b5_19));
    t3 = add64(t3, mul32_to_64(a9, b4_19));

    var t4 = mul32_to_64(a0, b4);
    t4 = add64(t4, mul32_to_64(a1_2, b3));
    t4 = add64(t4, mul32_to_64(a2, b2));
    t4 = add64(t4, mul32_to_64(a3_2, b1));
    t4 = add64(t4, mul32_to_64(a4, b0));
    t4 = add64(t4, mul32_to_64(a5_2, b9_19));
    t4 = add64(t4, mul32_to_64(a6, b8_19));
    t4 = add64(t4, mul32_to_64(a7_2, b7_19));
    t4 = add64(t4, mul32_to_64(a8, b6_19));
    t4 = add64(t4, mul32_to_64(a9_2, b5_19));

    var t5 = mul32_to_64(a0, b5);
    t5 = add64(t5, mul32_to_64(a1, b4));
    t5 = add64(t5, mul32_to_64(a2, b3));
    t5 = add64(t5, mul32_to_64(a3, b2));
    t5 = add64(t5, mul32_to_64(a4, b1));
    t5 = add64(t5, mul32_to_64(a5, b0));
    t5 = add64(t5, mul32_to_64(a6, b9_19));
    t5 = add64(t5, mul32_to_64(a7, b8_19));
    t5 = add64(t5, mul32_to_64(a8, b7_19));
    t5 = add64(t5, mul32_to_64(a9, b6_19));

    var t6 = mul32_to_64(a0, b6);
    t6 = add64(t6, mul32_to_64(a1_2, b5));
    t6 = add64(t6, mul32_to_64(a2, b4));
    t6 = add64(t6, mul32_to_64(a3_2, b3));
    t6 = add64(t6, mul32_to_64(a4, b2));
    t6 = add64(t6, mul32_to_64(a5_2, b1));
    t6 = add64(t6, mul32_to_64(a6, b0));
    t6 = add64(t6, mul32_to_64(a7_2, b9_19));
    t6 = add64(t6, mul32_to_64(a8, b8_19));
    t6 = add64(t6, mul32_to_64(a9_2, b7_19));

    var t7 = mul32_to_64(a0, b7);
    t7 = add64(t7, mul32_to_64(a1, b6));
    t7 = add64(t7, mul32_to_64(a2, b5));
    t7 = add64(t7, mul32_to_64(a3, b4));
    t7 = add64(t7, mul32_to_64(a4, b3));
    t7 = add64(t7, mul32_to_64(a5, b2));
    t7 = add64(t7, mul32_to_64(a6, b1));
    t7 = add64(t7, mul32_to_64(a7, b0));
    t7 = add64(t7, mul32_to_64(a8, b9_19));
    t7 = add64(t7, mul32_to_64(a9, b8_19));

    var t8 = mul32_to_64(a0, b8);
    t8 = add64(t8, mul32_to_64(a1_2, b7));
    t8 = add64(t8, mul32_to_64(a2, b6));
    t8 = add64(t8, mul32_to_64(a3_2, b5));
    t8 = add64(t8, mul32_to_64(a4, b4));
    t8 = add64(t8, mul32_to_64(a5_2, b3));
    t8 = add64(t8, mul32_to_64(a6, b2));
    t8 = add64(t8, mul32_to_64(a7_2, b1));
    t8 = add64(t8, mul32_to_64(a8, b0));
    t8 = add64(t8, mul32_to_64(a9_2, b9_19));

    var t9 = mul32_to_64(a0, b9);
    t9 = add64(t9, mul32_to_64(a1, b8));
    t9 = add64(t9, mul32_to_64(a2, b7));
    t9 = add64(t9, mul32_to_64(a3, b6));
    t9 = add64(t9, mul32_to_64(a4, b5));
    t9 = add64(t9, mul32_to_64(a5, b4));
    t9 = add64(t9, mul32_to_64(a6, b3));
    t9 = add64(t9, mul32_to_64(a7, b2));
    t9 = add64(t9, mul32_to_64(a8, b1));
    t9 = add64(t9, mul32_to_64(a9, b0));

    // Carry propagation
    t1 = add64(t1, shr64(t0, 26u)); var r0 = t0.x & MASK26;
    t2 = add64(t2, shr64(t1, 25u)); var r1 = t1.x & MASK25;
    t3 = add64(t3, shr64(t2, 26u)); var r2 = t2.x & MASK26;
    t4 = add64(t4, shr64(t3, 25u)); var r3 = t3.x & MASK25;
    t5 = add64(t5, shr64(t4, 26u)); var r4 = t4.x & MASK26;
    t6 = add64(t6, shr64(t5, 25u)); var r5 = t5.x & MASK25;
    t7 = add64(t7, shr64(t6, 26u)); var r6 = t6.x & MASK26;
    t8 = add64(t8, shr64(t7, 25u)); var r7 = t7.x & MASK25;
    t9 = add64(t9, shr64(t8, 26u)); var r8 = t8.x & MASK26;

    let c9 = shr64(t9, 25u);
    let r9 = t9.x & MASK25;
    // c9 fits in u32 (< 2^32 after >> 25 from < 2^57), but use mul32_to_64 to be unconditionally safe.
    let c9_19 = mul32_to_64(c9.x, 19u);
    let r0_64 = add64(vec2<u32>(r0, 0u), c9_19);
    r0 = r0_64.x & MASK26;
    r1 = r1 + shr64(r0_64, 26u).x;
    let r2_carry = r1 >> 25u;
    r1 = r1 & MASK25;
    r2 = r2 + r2_carry;

    return Fe(array<u32, 10>(r0, r1, r2, r3, r4, r5, r6, r7, r8, r9));
}

fn fe_sq(a: Fe) -> Fe {
    let a0 = a.v[0]; let a1 = a.v[1]; let a2 = a.v[2]; let a3 = a.v[3]; let a4 = a.v[4];
    let a5 = a.v[5]; let a6 = a.v[6]; let a7 = a.v[7]; let a8 = a.v[8]; let a9 = a.v[9];

    let a0_2 = a0 * 2u;
    let a1_2 = a1 * 2u;
    let a2_2 = a2 * 2u;
    let a3_2 = a3 * 2u;
    let a4_2 = a4 * 2u;
    let a5_2 = a5 * 2u;
    let a6_2 = a6 * 2u;
    let a7_2 = a7 * 2u;
    let a8_2 = a8 * 2u;
    let a9_2 = a9 * 2u;

    let a1_4 = a1 * 4u;
    let a3_4 = a3 * 4u;
    let a5_4 = a5 * 4u;
    let a7_4 = a7 * 4u;

    let a5_19 = a5 * 19u;
    let a6_19 = a6 * 19u;
    let a7_19 = a7 * 19u;
    let a8_19 = a8 * 19u;
    let a9_19 = a9 * 19u;

    var t0 = mul32_to_64(a0, a0);
    t0 = add64(t0, mul32_to_64(a1_4, a9_19));
    t0 = add64(t0, mul32_to_64(a2_2, a8_19));
    t0 = add64(t0, mul32_to_64(a3_4, a7_19));
    t0 = add64(t0, mul32_to_64(a4_2, a6_19));
    t0 = add64(t0, mul32_to_64(a5_2, a5_19));

    var t1 = mul32_to_64(a0_2, a1);
    t1 = add64(t1, mul32_to_64(a2_2, a9_19));
    t1 = add64(t1, mul32_to_64(a3_2, a8_19));
    t1 = add64(t1, mul32_to_64(a4_2, a7_19));
    t1 = add64(t1, mul32_to_64(a5_2, a6_19));

    var t2 = mul32_to_64(a0_2, a2);
    t2 = add64(t2, mul32_to_64(a1_2, a1));
    t2 = add64(t2, mul32_to_64(a3_4, a9_19));
    t2 = add64(t2, mul32_to_64(a4_2, a8_19));
    t2 = add64(t2, mul32_to_64(a5_4, a7_19));
    t2 = add64(t2, mul32_to_64(a6, a6_19));

    var t3 = mul32_to_64(a0_2, a3);
    t3 = add64(t3, mul32_to_64(a1_2, a2));
    t3 = add64(t3, mul32_to_64(a4_2, a9_19));
    t3 = add64(t3, mul32_to_64(a5_2, a8_19));
    t3 = add64(t3, mul32_to_64(a6_2, a7_19));

    var t4 = mul32_to_64(a0_2, a4);
    t4 = add64(t4, mul32_to_64(a1_2, a3_2));
    t4 = add64(t4, mul32_to_64(a2, a2));
    t4 = add64(t4, mul32_to_64(a5_4, a9_19));
    t4 = add64(t4, mul32_to_64(a6_2, a8_19));
    t4 = add64(t4, mul32_to_64(a7_2, a7_19));

    var t5 = mul32_to_64(a0_2, a5);
    t5 = add64(t5, mul32_to_64(a1_2, a4));
    t5 = add64(t5, mul32_to_64(a2_2, a3));
    t5 = add64(t5, mul32_to_64(a6_2, a9_19));
    t5 = add64(t5, mul32_to_64(a7_2, a8_19));

    var t6 = mul32_to_64(a0_2, a6);
    t6 = add64(t6, mul32_to_64(a1_2, a5_2));
    t6 = add64(t6, mul32_to_64(a2_2, a4));
    t6 = add64(t6, mul32_to_64(a3_2, a3));
    t6 = add64(t6, mul32_to_64(a7_4, a9_19));
    t6 = add64(t6, mul32_to_64(a8, a8_19));

    var t7 = mul32_to_64(a0_2, a7);
    t7 = add64(t7, mul32_to_64(a1_2, a6));
    t7 = add64(t7, mul32_to_64(a2_2, a5));
    t7 = add64(t7, mul32_to_64(a3_2, a4));
    t7 = add64(t7, mul32_to_64(a8_2, a9_19));

    var t8 = mul32_to_64(a0_2, a8);
    t8 = add64(t8, mul32_to_64(a1_2, a7_2));
    t8 = add64(t8, mul32_to_64(a2_2, a6));
    t8 = add64(t8, mul32_to_64(a3_2, a5_2));
    t8 = add64(t8, mul32_to_64(a4, a4));
    t8 = add64(t8, mul32_to_64(a9_2, a9_19));

    var t9 = mul32_to_64(a0_2, a9);
    t9 = add64(t9, mul32_to_64(a1_2, a8));
    t9 = add64(t9, mul32_to_64(a2_2, a7));
    t9 = add64(t9, mul32_to_64(a3_2, a6));
    t9 = add64(t9, mul32_to_64(a4_2, a5));

    t1 = add64(t1, shr64(t0, 26u)); var r0 = t0.x & MASK26;
    t2 = add64(t2, shr64(t1, 25u)); var r1 = t1.x & MASK25;
    t3 = add64(t3, shr64(t2, 26u)); var r2 = t2.x & MASK26;
    t4 = add64(t4, shr64(t3, 25u)); var r3 = t3.x & MASK25;
    t5 = add64(t5, shr64(t4, 26u)); var r4 = t4.x & MASK26;
    t6 = add64(t6, shr64(t5, 25u)); var r5 = t5.x & MASK25;
    t7 = add64(t7, shr64(t6, 26u)); var r6 = t6.x & MASK26;
    t8 = add64(t8, shr64(t7, 25u)); var r7 = t7.x & MASK25;
    t9 = add64(t9, shr64(t8, 26u)); var r8 = t8.x & MASK26;

    let c9 = shr64(t9, 25u);
    let r9 = t9.x & MASK25;
    let c9_19 = mul32_to_64(c9.x, 19u);
    let r0_64 = add64(vec2<u32>(r0, 0u), c9_19);
    r0 = r0_64.x & MASK26;
    r1 = r1 + shr64(r0_64, 26u).x;
    let r2_carry = r1 >> 25u;
    r1 = r1 & MASK25;
    r2 = r2 + r2_carry;

    return Fe(array<u32, 10>(r0, r1, r2, r3, r4, r5, r6, r7, r8, r9));
}
fn fe_invert(z: Fe) -> Fe {
    var t0 = fe_sq(z);
    var t1 = fe_sq(fe_sq(t0));
    var t2 = fe_mul(z, t1);
    t0 = fe_mul(t0, t2);
    let t3 = fe_sq(t0);
    t1 = fe_mul(t2, t3);
    t2 = fe_sq(t1);
    for (var i = 1u; i < 5u; i = i + 1u) {
        t2 = fe_sq(t2);
    }
    t1 = fe_mul(t2, t1);
    t2 = fe_sq(t1);
    for (var i = 1u; i < 10u; i = i + 1u) {
        t2 = fe_sq(t2);
    }
    t2 = fe_mul(t2, t1);
    var t4 = fe_sq(t2);
    for (var i = 1u; i < 20u; i = i + 1u) {
        t4 = fe_sq(t4);
    }
    t2 = fe_mul(t4, t2);
    t2 = fe_sq(t2);
    for (var i = 1u; i < 10u; i = i + 1u) {
        t2 = fe_sq(t2);
    }
    t1 = fe_mul(t2, t1);
    t2 = fe_sq(t1);
    for (var i = 1u; i < 50u; i = i + 1u) {
        t2 = fe_sq(t2);
    }
    t2 = fe_mul(t2, t1);
    t4 = fe_sq(t2);
    for (var i = 1u; i < 100u; i = i + 1u) {
        t4 = fe_sq(t4);
    }
    t2 = fe_mul(t4, t2);
    t2 = fe_sq(t2);
    for (var i = 1u; i < 50u; i = i + 1u) {
        t2 = fe_sq(t2);
    }
    t1 = fe_mul(t2, t1);
    t1 = fe_sq(fe_sq(fe_sq(fe_sq(fe_sq(t1)))));
    return fe_mul(t1, t0);
}

// Canonicalize mod p = 2^255 - 19 and pack into 8 little-endian u32 words.
fn fe_to_words(f: Fe) -> array<u32, 8> {
    var h = f.v;

    // Two carry passes to ensure bounded limbs
    var q = (h[0] + 19u * (h[9] >> 25u)) >> 26u;
    q = (h[1] + q) >> 25u;
    q = (h[2] + q) >> 26u;
    q = (h[3] + q) >> 25u;
    q = (h[4] + q) >> 26u;
    q = (h[5] + q) >> 25u;
    q = (h[6] + q) >> 26u;
    q = (h[7] + q) >> 25u;
    q = (h[8] + q) >> 26u;
    q = (h[9] + q) >> 25u;

    h[0] = h[0] + 19u * q;
    h[1] = h[1] + (h[0] >> 26u); h[0] = h[0] & MASK26;
    h[2] = h[2] + (h[1] >> 25u); h[1] = h[1] & MASK25;
    h[3] = h[3] + (h[2] >> 26u); h[2] = h[2] & MASK26;
    h[4] = h[4] + (h[3] >> 25u); h[3] = h[3] & MASK25;
    h[5] = h[5] + (h[4] >> 26u); h[4] = h[4] & MASK26;
    h[6] = h[6] + (h[5] >> 25u); h[5] = h[5] & MASK25;
    h[7] = h[7] + (h[6] >> 26u); h[6] = h[6] & MASK26;
    h[8] = h[8] + (h[7] >> 25u); h[7] = h[7] & MASK25;
    h[9] = h[9] + (h[8] >> 26u); h[8] = h[8] & MASK26;
    h[9] = h[9] & MASK25;

    // Conditional subtraction of p = 2^255 - 19 via adding 19
    var t = h;
    t[0] = t[0] + 19u;
    t[1] = t[1] + (t[0] >> 26u); t[0] = t[0] & MASK26;
    t[2] = t[2] + (t[1] >> 25u); t[1] = t[1] & MASK25;
    t[3] = t[3] + (t[2] >> 26u); t[2] = t[2] & MASK26;
    t[4] = t[4] + (t[3] >> 25u); t[3] = t[3] & MASK25;
    t[5] = t[5] + (t[4] >> 26u); t[4] = t[4] & MASK26;
    t[6] = t[6] + (t[5] >> 25u); t[5] = t[5] & MASK25;
    t[7] = t[7] + (t[6] >> 26u); t[6] = t[6] & MASK26;
    t[8] = t[8] + (t[7] >> 25u); t[7] = t[7] & MASK25;
    t[9] = t[9] + (t[8] >> 26u); t[8] = t[8] & MASK26;
    let overflow = t[9] >> 25u;
    t[9] = t[9] & MASK25;

    let mask = 0u - overflow;
    for (var i = 0u; i < 10u; i = i + 1u) {
        h[i] = (h[i] & ~mask) | (t[i] & mask);
    }

    // Pack 10 limbs (26, 25, 26, 25, 26, 25, 26, 25, 26, 25) into 8 x u32 LE words:
    // w0: h0[26] | h1[6]
    // w1: h1[19] | h2[13]
    // w2: h2[13] | h3[19]
    // w3: h3[6]  | h4[26]
    // w4: h5[25] | h6[7]
    // w5: h6[19] | h7[13]
    // w6: h7[12] | h8[20]
    // w7: h8[6]  | h9[25] | 0[1]
    var w: array<u32, 8>;
    w[0] = h[0] | (h[1] << 26u);
    w[1] = (h[1] >> 6u) | (h[2] << 19u);
    w[2] = (h[2] >> 13u) | (h[3] << 13u);
    w[3] = (h[3] >> 19u) | (h[4] << 6u);
    w[4] = h[5] | (h[6] << 25u);
    w[5] = (h[6] >> 7u) | (h[7] << 19u);
    w[6] = (h[7] >> 13u) | (h[8] << 12u);
    w[7] = (h[8] >> 20u) | (h[9] << 6u);
    return w;
}
fn fe_top_two_words_approx(f: Fe) -> vec2<u32> {
    var h = f.v;
    var q = (h[0] + 19u * (h[9] >> 25u)) >> 26u;
    q = (h[1] + q) >> 25u;
    q = (h[2] + q) >> 26u;
    q = (h[3] + q) >> 25u;
    q = (h[4] + q) >> 26u;
    q = (h[5] + q) >> 25u;
    q = (h[6] + q) >> 26u;
    q = (h[7] + q) >> 25u;
    q = (h[8] + q) >> 26u;
    q = (h[9] + q) >> 25u;

    h[0] = h[0] + 19u * q;
    h[1] = h[1] + (h[0] >> 26u); h[0] = h[0] & MASK26;
    h[2] = h[2] + (h[1] >> 25u); h[1] = h[1] & MASK25;
    let w0 = h[0] | (h[1] << 26u);
    let w1 = (h[1] >> 6u) | (h[2] << 19u);
    return vec2<u32>(w0, w1);
}

fn byte_swap32(x: u32) -> u32 {
    return ((x & 0x000000ffu) << 24u)
        | ((x & 0x0000ff00u) << 8u)
        | ((x & 0x00ff0000u) >> 8u)
        | ((x & 0xff000000u) >> 24u);
}

fn add_cached(p: ExtPoint, q: CachedStep) -> ExtPoint {
    let a = fe_mul(fe_sub(p.y, p.x), q.y_minus_x);
    let b = fe_mul(fe_add(p.y, p.x), q.y_plus_x);
    let c = fe_mul(p.t, q.t2d);
    let d = fe_add(p.z, p.z);

    let e = fe_sub(b, a);
    let f = fe_sub(d, c);
    let g = fe_add(d, c);
    let h = fe_add(b, a);

    return ExtPoint(
        fe_mul(e, f),
        fe_mul(g, h),
        fe_mul(f, g),
        fe_mul(e, h),
    );
}

var<workgroup> wg_pre: array<Fe, 128>;
var<workgroup> wg_suf: array<Fe, 128>;
var<workgroup> wg_total_inv: Fe;

@compute @workgroup_size(128)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
) {
    let tid = gid.x;
    let lid = local_id.x;
    var p = points[tid];

    var xs: array<Fe, 8>;
    var ys: array<Fe, 8>;
    var zs: array<Fe, 8>;
    var pre: array<Fe, 8>;

    for (var step = 0u; step < params.steps_per_dispatch; step = step + 1u) {
        var acc = fe_one();
        for (var i = 0u; i < BATCH_SIZE; i = i + 1u) {
            xs[i] = p.x;
            ys[i] = p.y;
            zs[i] = p.z;
            if (i == 0u) {
                acc = p.z;
            } else {
                acc = fe_mul(acc, p.z);
            }
            pre[i] = acc;
            p = add_cached(p, step_cached);
        }

        // Workgroup-cooperative Montgomery batch inversion across all 128 threads (1,024 keys).
        wg_pre[lid] = acc;
        wg_suf[lid] = acc;
        workgroupBarrier();

        for (var offset = 1u; offset < WG_SIZE; offset = offset << 1u) {
            var left = fe_one();
            if (lid >= offset) {
                left = wg_pre[lid - offset];
            }
            var right = fe_one();
            if (lid + offset < WG_SIZE) {
                right = wg_suf[lid + offset];
            }
            workgroupBarrier();
            if (lid >= offset) {
                wg_pre[lid] = fe_mul(left, wg_pre[lid]);
            }
            if (lid + offset < WG_SIZE) {
                wg_suf[lid] = fe_mul(wg_suf[lid], right);
            }
            workgroupBarrier();
        }

        if (lid == 0u) {
            wg_total_inv = fe_invert(wg_pre[WG_SIZE - 1u]);
        }
        workgroupBarrier();

        var inv = wg_total_inv;
        if (lid > 0u) {
            inv = fe_mul(inv, wg_pre[lid - 1u]);
        }
        if (lid + 1u < WG_SIZE) {
            inv = fe_mul(inv, wg_suf[lid + 1u]);
        }

        for (var rev = 0u; rev < BATCH_SIZE; rev = rev + 1u) {
            let i = BATCH_SIZE - 1u - rev;
            var z_inv = inv;
            if (i > 0u) {
                z_inv = fe_mul(inv, pre[i - 1u]);
            }
            inv = fe_mul(inv, zs[i]);

            let y_affine = fe_mul(ys[i], z_inv);
            let top = fe_top_two_words_approx(y_affine);
            let w0 = byte_swap32(top.x);
            let w1 = byte_swap32(top.y);

            if ((w0 & params.mask_hi) == params.expected_hi && (w1 & params.mask_lo) == params.expected_lo) {
                var words = fe_to_words(y_affine);
                let x_affine = fe_mul(xs[i], z_inv);
                let x_words = fe_to_words(x_affine);
                words[7] = words[7] | ((x_words[0] & 1u) << 31u);

                let idx = atomicAdd(&hit_buf.count, 1u);
                if (idx < 64u) {
                    hit_buf.hits[idx].thread_id = tid;
                    hit_buf.hits[idx].step_offset = params.base_step_offset + step * BATCH_SIZE + i;
                    hit_buf.hits[idx].pubkey = words;
                }
            }
        }
    }

    points[tid] = p;
}
