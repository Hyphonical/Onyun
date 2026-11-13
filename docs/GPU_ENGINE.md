# GPU Minting Engine (`GpuMkEngine`) — Architecture & Design Notes

This document captures the design for adding a cross-platform GPU vanity-address minting engine (`GpuMkEngine`) to Onyun using **wgpu (WGSL)**, while remaining cleanly abstracted behind `MintEngine`.

---

## 1. Integration with Existing Architecture

Onyun's engine abstraction (`src/core/engine/mod.rs`) is already well-suited for a GPU backend:

- **Trait:** `MintEngine::start(&self, config: MintConfig, telemetry: Arc<SharedTelemetry>, on_milestone: Sender<MintMilestone>, cancel: Arc<AtomicBool>) -> Result<EngineHandle>`
- **Cargo feature gating:** Gate behind an optional Cargo feature (e.g., `gpu = ["dep:wgpu", "dep:bytemuck", "dep:pollster"]`) so headless CLI/Docker builds remain lightweight and free of GPU driver dependencies.
- **Engine selection:** CLI (`--engine auto|cpu|gpu`) and Web/Desktop UI can pick `GpuMkEngine` when available and fall back to `CpuMkEngine` when no adapter is found.
- **Telemetry:** `SharedTelemetry::new(1, initial_best_len)` (or 1 GPU coordinator thread) reports total hashes evaluated per dispatch, keeping `compute_progress`, EWMA speed, and ETA logic unchanged.

---

## 2. Core Algorithmic Mapping to GPU

Vanity `.onion` generation is embarrassingly parallel: millions of independent Ed25519 scalar-step chains `(base_i + k * 8) * B` with no inter-stream communication until a prefix match occurs.

### 2.1 Field Arithmetic (`GF(2^255 - 19)`) on GPUs

`CpuMkEngine` uses **5 × 51-bit limbs** (`u64` with `u128` intermediates). **Do not port 5 × 51-bit directly to WGSL:**
- GPUs are native **32-bit** integer machines (`u32` ALU at full rate).
- Core WGSL has **no `u128`** and `u64` (`SHADER_INT64`) is an optional extension not universally available across Metal/DX12/Vulkan.

Instead, use **radix-$2^{25.5}$ (10 × 25/26-bit limbs)** in `u32` (or `i32`):
- Limbs alternate 26-bit and 25-bit capacities (`26, 25, 26, 25, 26, 25, 26, 25, 26, 25` = 255 bits).
- Two options for 32×32 → 64-bit products in WGSL:
  1. **`SHADER_INT64` path (desktop Vulkan/DX12):** Use `u64` accumulators (`u32` × `u32` → `u64`) and carry-propagate once per `mul`/`sq`.
  2. **Pure `u32` portable WGSL path (Metal/WebGPU-safe):** Split each 26-bit limb into 13-bit halves or use 16-bit products (`u32` × `u32` fits in `u32` when limbs ≤ 16 bits, e.g., **16 × 16-bit** or **17 × 15-bit** limbs, or WGSL `mulExtended` if/when available).
     - **17 × 15-bit limbs** (`17 * 15 = 255` bits): `15-bit * 15-bit = 30-bit`, summing up to 17 terms fits in `u32`? `17 * 2^30` overflows `u32`, so split into two `u32` accumulators or accumulate with periodic carry (`u32` hi/lo via `mul32_hi_lo(a, b)` using 16-bit splits).
     - Even simpler in 10×25.5-bit: a 10-line `mul_u32_to_u64(a: u32, b: u32) -> vec2<u32>` (lo, hi) using 16-bit halves + carry, which works on **every** WGSL backend (Vulkan, Metal, DX12) with zero extensions.

### 2.2 Curve Representation & Step Table

Use **Extended Twisted Edwards coordinates** $(X, Y, Z, T)$ with `add_cached` against a precomputed `CachedStep` for $8B$:
- `CachedStep`: `(Y + X, Y - X, 2dXY)` — uploaded once in a small uniform buffer (`~120 bytes`).
- Each GPU thread `tid` owns one `ExtPoint` in a `storage<read_write>` buffer:
  - Total state for `N = 131,072` threads: `131,072 × 4 coords × 10 limbs × 4 bytes = 20 MB` VRAM.
  - Because state stays in VRAM across dispatches, **zero `ExtPoint` data is transferred between CPU and GPU** during the hot loop.

---

## 3. Solving Register Pressure: Workgroup-Cooperative Montgomery Inversion

On CPU, `CpuMkEngine` stores `BATCH_SIZE = 256` `ExtPoint`s in L1 cache (`~20 KB`) per thread and inverts them with 1 `Fe::invert()`.

On GPU, storing 128–256 points per thread in local arrays (`~20 KB/thread`) would **spill registers to global VRAM** and destroy occupancy.

### Solution: Workgroup Shared-Memory Montgomery Batching
1. Launch workgroups of **`WG_SIZE = 256` threads** (or `128`).
2. Each thread holds only **1** `ExtPoint` in registers.
3. For each step `k` in `0..STEPS_PER_DISPATCH` (e.g., `STEPS_PER_DISPATCH = 64`):
   - Every thread advances its point: `P = P.add_cached(step_cached)`.
   - Every thread writes its `Z` coordinate to **workgroup shared memory** (`var<workgroup> zs: array<Fe, 256>`, `256 × 40 B = 10 KB`, fits in 32–48 KB shared memory).
   - `workgroupBarrier()`.
   - **Parallel prefix-product tree** (or subgroup/warp scan) over `zs[0..256]` in shared memory:
     - Up-sweep (`8` steps for `256` threads) computes `prod[i] = Z_0 * ... * Z_i`.
     - Thread `0` computes `inv = invert(prod[255])` (1 inversion amortized over 256 keys!).
     - Down-sweep distributes `z_inv[i]` to each thread.
   - Each thread computes `y_affine = Y * z_inv` in registers and runs the fast prefix check.

*Alternative (simpler first milestone):* Each thread holds a tiny register batch of `4` or `8` `Z`s and cooperates across a `64`-thread workgroup (`64 × 4 = 256` keys per inversion, `64 × 40 B = 2.5 KB` shared memory).

---

## 4. Fast Prefix Matching & Zero-Noise Hit Readback

### 4.1 Uniform Buffer (`MintParams`)
Updated by CPU only when `global_best` improves:
```wgsl
struct MintParams {
    mask_hi: u32,      // top 32 bits of big-endian pubkey[0..4]
    mask_lo: u32,      // next 32 bits of pubkey[4..8]
    expected_hi: u32,
    expected_lo: u32,
    steps_per_dispatch: u32,
    base_step_offset_lo: u32,
    base_step_offset_hi: u32,
    _pad: u32,
}
```
- Up to **12 Base32 characters** (60 bits) are checked with two bitwise ANDs (`(w0 & mask_hi) == expected_hi && (w1 & mask_lo) == expected_lo`).
- If `target_len > 12` and `global_best >= 12`, include the full 52-char target nibble array in the uniform buffer for a secondary loop only when the 60-bit prefix matches (1 in $2^{60}$ keys).

### 4.2 Canonicalization Before Masking
`y_affine` from `Fe::mul` is reduced mod $2^{255}-19$ with small bounded limbs, **not necessarily canonical `< p`**. Before extracting `w0, w1` for the prefix mask, perform the standard carry-propagation + conditional subtraction of $p = 2^{255}-19$ so `y_bytes[0..8]` is exact.

### 4.3 Hit Output Buffer
```wgsl
struct Hit {
    thread_id: u32,
    step_offset: u32,
    pubkey: array<u32, 8>, // 32 bytes (with sign bit of x if needed)
}

struct HitBuffer {
    count: atomic<u32>,
    hits: array<Hit, 64>, // capped at 64 hits per dispatch
}
```
- On a match, `let idx = atomicAdd(&hit_buf.count, 1u); if (idx < 64u) { ... }`.
- Host resets `hit_buf.count = 0` before each dispatch and reads back **only `4 bytes`** (`count`) if `count == 0`, or `4 + min(count, 64) * sizeof(Hit)` bytes when `count > 0`.

---

## 5. Host Coordinator Lifecycle

1. **Initialization (`start`):**
   - Pick `wgpu::Instance` → high-performance `Adapter` → `Device` + `Queue`.
   - Choose `num_threads` based on adapter limits/compute units (e.g., `131,072` = `512` workgroups × `256` threads).
   - On CPU (parallelized with `std::thread::scope`), generate `num_threads` random 32-byte `seeds[tid]`, derive `base_scalars[tid]`, compute `base_scalars[tid] * B`, and pack into `ExtPoint` GPU buffer.
   - Upload `ExtPoint`s and `CachedStep(8*B)` to GPU.

2. **Dispatch Loop (1 coordinator thread):**
   - While `!cancel.load(Relaxed) && best_len < target_len`:
     - Reset `hit_buf.count = 0`.
     - Encode compute pass (`dispatch_workgroups(num_workgroups, 1, 1)`) and copy `hit_buf.count` (or whole `hit_buf`) to a small `MAP_READ` staging buffer.
     - `queue.submit(...)` and `device.poll(Maintain::Wait)`.
     - **Tune `steps_per_dispatch`** dynamically so each dispatch takes ~15–40 ms (keeps cancellation and progress UI snappy while hiding submit overhead).
     - Increment `telemetry.thread_hashes[0]` by `num_threads * steps_per_dispatch`.
     - If `hit_count > 0`:
       - For each hit: reconstruct `scalar = base_scalars[tid] + Scalar::from(exact_step * 8)`, compute `derive_expanded_secret(&seeds[tid], exact_step, &scalar)` and `derive_address(&pubkey)`, verify `matched_len_hostname(&address) > global_best`, and emit `MintMilestone`.
       - Update `MintParams` uniform with new `mask` / `expected`.
     - Periodically reseed after `2^24` steps per thread (same `RESEED_STEPS` safety margin as CPU).

---

## 6. Expected Performance & Verification Plan

- **Expected speedup:** A mid-range desktop GPU (e.g. RTX 3060 / RX 6700 XT / Apple M-series GPU) typically achieves **5×–20×** higher Ed25519 keygen throughput than a 16–24 thread CPU, depending on integer `u32`/`u64` ALU throughput and shared-memory bank conflicts.
- **Verification plan when implementing:**
  1. **Field unit test on GPU:** Dispatch a 1-thread shader computing `a * b`, `a^2`, `invert(a)`, and `add_cached` on known test vectors and assert bit-exact equality with `CpuMkEngine` / `curve25519-dalek`.
  2. **End-to-end keypair test:** Mint a 3–4 char prefix with `GpuMkEngine` and verify `OnionKeypair::public_key` matches `expanded_secret` scalar × `B` and `derive_address` passes Tor v3 checksum validation.
  3. **Benchmark:** Compare `onyun mint <6-char> --engine cpu` vs `--engine gpu` on identical hardware.
