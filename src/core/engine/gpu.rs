//! `GpuMkEngine`: discrete-GPU `wgpu` minting engine using 10-limb radix-2^25.5
//! `GF(2^255 - 19)` arithmetic, extended Edwards `+ 8*B` stepping, and
//! workgroup-cooperative Montgomery batch Z-inversion.

use crate::core::base32::PrefixMatcher;
use crate::core::engine::helper::{
    CachedStep, ExtPoint, Fe, derive_expanded_secret, seed_to_scalar,
};
use crate::core::engine::{
    EngineHandle, MintConfig, MintEngine, MintMilestone, SharedTelemetry, default_threads,
};
use crate::core::onion::{OnionKeypair, derive_address};
use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
use curve25519_dalek::scalar::Scalar;
use rand::Rng;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc::Sender};
use std::thread;
use std::time::Instant;

pub(crate) const LABEL: &str = "GPU";

const WG_SIZE: u32 = 128;
const NUM_WORKGROUPS: u32 = 512;
const NUM_THREADS: usize = (WG_SIZE * NUM_WORKGROUPS) as usize; // 65,536 threads
const BATCH_SIZE: u64 = 16;
const INITIAL_STEPS_PER_DISPATCH: u32 = 4;
const MIN_STEPS_PER_DISPATCH: u32 = 1;
const MAX_STEPS_PER_DISPATCH: u32 = 16;
const RESEED_STEPS: u64 = 1 << 24;
const MAX_HITS_PER_DISPATCH: usize = 64;

const SHADER_SRC: &str = include_str!("ed25519_mint.wgsl");
const SHADER_SRC_U64: &str = include_str!(concat!(env!("OUT_DIR"), "/ed25519_mint_u64.wgsl"));

pub(crate) struct GpuMkEngine {
    pub(crate) adapter_index: usize,
    pub(crate) name: String,
}

fn hardware_adapters() -> Vec<wgpu::Adapter> {
    let instance = wgpu::Instance::default();
    let all = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    let mut selected: Vec<wgpu::Adapter> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for adapter in all {
        let info = adapter.get_info();
        if info.device_type != wgpu::DeviceType::DiscreteGpu {
            continue;
        }
        if seen.insert((info.vendor, info.device)) {
            selected.push(adapter);
        }
    }
    selected
}

pub(crate) fn enumerate_gpus() -> Vec<GpuMkEngine> {
    hardware_adapters()
        .into_iter()
        .enumerate()
        .map(|(adapter_index, adapter)| GpuMkEngine {
            adapter_index,
            name: adapter.get_info().name,
        })
        .collect()
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Fe10([u32; 10]);

impl Fe10 {
    fn from_le_bytes(bytes: &[u8; 32]) -> Self {
        let mut limbs = [0u32; 10];
        let mut bit_pos = 0usize;
        let widths = [26usize, 25, 26, 25, 26, 25, 26, 25, 26, 25];
        for (i, &w) in widths.iter().enumerate() {
            let mut val = 0u32;
            for b in 0..w {
                let abs_bit = bit_pos + b;
                let byte_idx = abs_bit >> 3;
                let bit_in_byte = abs_bit & 7;
                if byte_idx < 32 && ((bytes[byte_idx] >> bit_in_byte) & 1) != 0 {
                    val |= 1u32 << b;
                }
            }
            limbs[i] = val;
            bit_pos += w;
        }
        Self(limbs)
    }

    fn from_fe(fe: &Fe) -> Self {
        Self::from_le_bytes(&fe.to_bytes())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ExtPointGpu {
    x: Fe10,
    y: Fe10,
    z: Fe10,
    t: Fe10,
}

impl ExtPointGpu {
    fn from_cpu(p: &ExtPoint) -> Self {
        Self {
            x: Fe10::from_fe(&p.x),
            y: Fe10::from_fe(&p.y),
            z: Fe10::from_fe(&p.z),
            t: Fe10::from_fe(&p.t),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CachedStepGpu {
    y_plus_x: Fe10,
    y_minus_x: Fe10,
    t2d: Fe10,
}

impl CachedStepGpu {
    fn from_cpu(c: &CachedStep) -> Self {
        Self {
            y_plus_x: Fe10::from_fe(&c.y_plus_x),
            y_minus_x: Fe10::from_fe(&c.y_minus_x),
            t2d: Fe10::from_fe(&c.t2d),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HitGpu {
    thread_id: u32,
    step_offset: u32,
    pubkey: [u32; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HitBufferGpu {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    hits: [HitGpu; MAX_HITS_PER_DISPATCH],
}

fn generate_seed_points(
    seeds: &mut [[u8; 32]],
    base_scalars: &mut [Scalar],
    gpu_points: &mut [ExtPointGpu],
) {
    let workers = default_threads().min(seeds.len().max(1));
    let chunk = seeds.len().div_ceil(workers);

    std::thread::scope(|s| {
        for ((seed_chunk, scalar_chunk), pt_chunk) in seeds
            .chunks_mut(chunk)
            .zip(base_scalars.chunks_mut(chunk))
            .zip(gpu_points.chunks_mut(chunk))
        {
            s.spawn(move || {
                let mut rng = rand::rng();
                for i in 0..seed_chunk.len() {
                    rng.fill_bytes(&mut seed_chunk[i]);
                    let scalar = seed_to_scalar(&seed_chunk[i]);
                    scalar_chunk[i] = scalar;
                    let start_dalek = ED25519_BASEPOINT_POINT * scalar;
                    let ext = ExtPoint::from_dalek(&start_dalek);
                    pt_chunk[i] = ExtPointGpu::from_cpu(&ext);
                }
            });
        }
    });
}

fn words_to_pubkey(words: &[u32; 8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, &w) in words.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    out
}

fn make_params(
    matcher: &PrefixMatcher,
    best_len: usize,
    steps_per_dispatch: u32,
    base_step_offset: u32,
) -> MintParams {
    let needed_chars = (best_len + 1).min(matcher.len());
    let (mask64, expected64) = matcher.fast_mask_u64(needed_chars);
    MintParams {
        mask_hi: (mask64 >> 32) as u32,
        mask_lo: mask64 as u32,
        expected_hi: (expected64 >> 32) as u32,
        expected_lo: expected64 as u32,
        steps_per_dispatch,
        base_step_offset,
        _pad0: 0,
        _pad1: 0,
    }
}

impl MintEngine for GpuMkEngine {
    fn label(&self) -> &'static str {
        LABEL
    }

    fn device_name(&self) -> String {
        self.name.clone()
    }

    fn start(
        &self,
        config: MintConfig,
        telemetry: Arc<SharedTelemetry>,
        on_milestone: Sender<MintMilestone>,
        cancel: Arc<AtomicBool>,
    ) -> Result<EngineHandle> {
        let adapter = hardware_adapters()
            .into_iter()
            .nth(self.adapter_index)
            .with_context(|| {
                format!(
                    "GPU adapter #{} ({}) is no longer available",
                    self.adapter_index, self.name
                )
            })?;

        let use_int64 = adapter.features().contains(wgpu::Features::SHADER_INT64);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("onyun-gpu-device"),
            required_features: if use_int64 {
                wgpu::Features::SHADER_INT64
            } else {
                wgpu::Features::empty()
            },
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .context("failed to open GPU device")?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("onyun-ed25519-mint"),
            source: wgpu::ShaderSource::Wgsl(if use_int64 {
                SHADER_SRC_U64.into()
            } else {
                SHADER_SRC.into()
            }),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("onyun-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("onyun-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("onyun-compute-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let matcher = config.goal.matcher();
        let step_dalek = ED25519_BASEPOINT_POINT * Scalar::from(8u64);
        let step_cached = CachedStepGpu::from_cpu(&CachedStep::from_dalek(&step_dalek));

        let mut seeds = vec![[0u8; 32]; NUM_THREADS];
        let mut base_scalars = vec![Scalar::ZERO; NUM_THREADS];
        let mut gpu_points = vec![ExtPointGpu::zeroed(); NUM_THREADS];
        generate_seed_points(&mut seeds, &mut base_scalars, &mut gpu_points);

        let initial_best = telemetry.global_best_len.load(Ordering::Relaxed) as usize;
        let params = make_params(&matcher, initial_best, INITIAL_STEPS_PER_DISPATCH, 0);

        let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("onyun-params"),
            size: std::mem::size_of::<MintParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&params_buf, 0, bytemuck::bytes_of(&params));

        let step_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("onyun-step-cached"),
            size: std::mem::size_of::<CachedStepGpu>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&step_buf, 0, bytemuck::bytes_of(&step_cached));

        let points_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("onyun-points"),
            size: (NUM_THREADS * std::mem::size_of::<ExtPointGpu>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&points_buf, 0, bytemuck::cast_slice(&gpu_points));

        let hit_buf_size = std::mem::size_of::<HitBufferGpu>() as u64;
        let hit_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("onyun-hits"),
            size: hit_buf_size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let staging_bufs = [
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("onyun-staging-0"),
                size: hit_buf_size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("onyun-staging-1"),
                size: hit_buf_size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        ];

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("onyun-bg"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: step_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: points_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: hit_buf.as_entire_binding(),
                },
            ],
        });

        let telemetry_slot = config.telemetry_offset;
        let handle = thread::Builder::new()
            .name(format!("onyun-gpu-{}", self.adapter_index))
            .spawn(move || {
                let ctx = GpuContext {
                    device,
                    queue,
                    pipeline,
                    bind_group,
                    params_buf,
                    points_buf,
                    hit_buf,
                    staging_bufs,
                };
                let seed_state = GpuSeedState {
                    seeds,
                    base_scalars,
                    gpu_points,
                };
                coordinator_loop(
                    &ctx,
                    &matcher,
                    seed_state,
                    &telemetry,
                    telemetry_slot,
                    &on_milestone,
                    &cancel,
                );
            })?;
        Ok(EngineHandle {
            workers: vec![handle],
        })
    }
}

struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    params_buf: wgpu::Buffer,
    points_buf: wgpu::Buffer,
    hit_buf: wgpu::Buffer,
    staging_bufs: [wgpu::Buffer; 2],
}

struct GpuSeedState {
    seeds: Vec<[u8; 32]>,
    base_scalars: Vec<Scalar>,
    gpu_points: Vec<ExtPointGpu>,
}

fn coordinator_loop(
    ctx: &GpuContext,
    matcher: &PrefixMatcher,
    mut seed_state: GpuSeedState,
    telemetry: &Arc<SharedTelemetry>,
    telemetry_slot: usize,
    on_milestone: &Sender<MintMilestone>,
    cancel: &Arc<AtomicBool>,
) {
    let target_len = matcher.len();
    let mut local_best = telemetry.global_best_len.load(Ordering::Relaxed) as usize;
    let mut steps_per_dispatch = INITIAL_STEPS_PER_DISPATCH;
    let mut step_index: u64 = 0;
    let mut staging_idx: usize = 0;
    let hit_buf_size = std::mem::size_of::<HitBufferGpu>() as u64;

    struct InFlight {
        staging_idx: usize,
        steps: u32,
        started_at: Instant,
        rx_map: std::sync::mpsc::Receiver<std::result::Result<(), wgpu::BufferAsyncError>>,
    }

    let submit_next = |step_index: &mut u64,
                       staging_idx: &mut usize,
                       steps_per_dispatch: u32,
                       local_best: usize,
                       seed_state: &mut GpuSeedState|
     -> InFlight {
        if *step_index >= RESEED_STEPS {
            generate_seed_points(
                &mut seed_state.seeds,
                &mut seed_state.base_scalars,
                &mut seed_state.gpu_points,
            );
            ctx.queue.write_buffer(
                &ctx.points_buf,
                0,
                bytemuck::cast_slice(&seed_state.gpu_points),
            );
            *step_index = 0;
        }
        let params = make_params(matcher, local_best, steps_per_dispatch, *step_index as u32);
        ctx.queue
            .write_buffer(&ctx.params_buf, 0, bytemuck::bytes_of(&params));

        let cur_staging = *staging_idx;
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("onyun-encoder"),
            });
        encoder.clear_buffer(&ctx.hit_buf, 0, Some(4));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("onyun-compute-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&ctx.pipeline);
            pass.set_bind_group(0, &ctx.bind_group, &[]);
            pass.dispatch_workgroups(NUM_WORKGROUPS, 1, 1);
        }
        encoder.copy_buffer_to_buffer(
            &ctx.hit_buf,
            0,
            &ctx.staging_bufs[cur_staging],
            0,
            hit_buf_size,
        );

        let started_at = Instant::now();
        ctx.queue.submit(Some(encoder.finish()));

        let (tx_map, rx_map) = std::sync::mpsc::channel();
        ctx.staging_bufs[cur_staging]
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |res| {
                let _ = tx_map.send(res);
            });

        let keys_this_dispatch = (steps_per_dispatch as u64) * BATCH_SIZE;
        *step_index += keys_this_dispatch;
        *staging_idx ^= 1;

        InFlight {
            staging_idx: cur_staging,
            steps: steps_per_dispatch,
            started_at,
            rx_map,
        }
    };

    let mut in_flight = if !cancel.load(Ordering::Relaxed) && local_best < target_len {
        Some(submit_next(
            &mut step_index,
            &mut staging_idx,
            steps_per_dispatch,
            local_best,
            &mut seed_state,
        ))
    } else {
        None
    };

    while let Some(cur) = in_flight.take() {
        let global_best = telemetry.global_best_len.load(Ordering::Acquire) as usize;
        if global_best > local_best {
            local_best = global_best;
        }

        let next_in_flight = if !cancel.load(Ordering::Relaxed) && local_best < target_len {
            Some(submit_next(
                &mut step_index,
                &mut staging_idx,
                steps_per_dispatch,
                local_best,
                &mut seed_state,
            ))
        } else {
            None
        };

        if ctx.device.poll(wgpu::PollType::wait_indefinitely()).is_err() {
            break;
        }
        if !matches!(cur.rx_map.recv(), Ok(Ok(()))) {
            break;
        }

        let dispatch_ms = cur.started_at.elapsed().as_secs_f64() * 1000.0;
        let staging = &ctx.staging_bufs[cur.staging_idx];
        let slice = staging.slice(..);

        if let Ok(view) = slice.get_mapped_range() {
            let hit_buf_cpu: &HitBufferGpu = bytemuck::from_bytes(&view);
            let count = (hit_buf_cpu.count as usize).min(MAX_HITS_PER_DISPATCH);

            for hit in &hit_buf_cpu.hits[..count] {
                let tid = hit.thread_id as usize;
                if tid >= NUM_THREADS {
                    continue;
                }
                let exact_step = hit.step_offset as u64;
                let pubkey = words_to_pubkey(&hit.pubkey);
                let address = derive_address(&pubkey);
                let full_matched = matcher.matched_len_hostname(&address);
                let current_global = telemetry.global_best_len.load(Ordering::Acquire) as usize;
                if full_matched > current_global {
                    local_best = full_matched;
                    let scalar =
                        seed_state.base_scalars[tid] + Scalar::from(exact_step.wrapping_mul(8));
                    let expanded_secret =
                        derive_expanded_secret(&seed_state.seeds[tid], exact_step, &scalar);
                    let kp = OnionKeypair {
                        address,
                        public_key: pubkey,
                        expanded_secret,
                    };
                    let _ = on_milestone.send(MintMilestone {
                        matched_len: full_matched,
                        keypair: kp,
                    });
                }
            }
        }
        staging.unmap();

        let keys_this_dispatch = (cur.steps as u64) * BATCH_SIZE;
        let hashes = (NUM_THREADS as u64) * keys_this_dispatch;
        telemetry.thread_hashes[telemetry_slot]
            .0
            .fetch_add(hashes, Ordering::Relaxed);

        // Keep dispatch duration in ~15–40 ms to stay well below Windows WDDM TDR.
        if dispatch_ms < 12.0 && steps_per_dispatch < MAX_STEPS_PER_DISPATCH {
            steps_per_dispatch = (steps_per_dispatch * 2).min(MAX_STEPS_PER_DISPATCH);
        } else if dispatch_ms > 45.0 && steps_per_dispatch > MIN_STEPS_PER_DISPATCH {
            steps_per_dispatch = (steps_per_dispatch / 2).max(MIN_STEPS_PER_DISPATCH);
        }

        in_flight = next_in_flight;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    #[test]
    fn gpu_u64_shader_matches_cpu_field_ops() {
        let Some(adapter) = hardware_adapters().into_iter().next() else {
            return;
        };
        if !adapter.features().contains(wgpu::Features::SHADER_INT64) {
            return;
        }
        let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("onyun-gpu-test"),
            required_features: wgpu::Features::SHADER_INT64,
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        })) else {
            return;
        };

        let test_src = format!(
            "{SHADER_SRC_U64}\n\
            struct TestPair {{ a: Fe, b: Fe, }}\n\
            struct TestOut {{ mul_words: array<u32, 8>, sq_words: array<u32, 8>, }}\n\
            @group(1) @binding(0) var<storage, read> test_in: array<TestPair>;\n\
            @group(1) @binding(1) var<storage, read_write> test_out: array<TestOut>;\n\
            @compute @workgroup_size(64)\n\
            fn test_mul_sq(@builtin(global_invocation_id) id: vec3<u32>) {{\n\
                let idx = id.x;\n\
                let p = test_in[idx];\n\
                let m = fe_mul(p.a, p.b);\n\
                let s = fe_sq(p.a);\n\
                test_out[idx].mul_words = fe_to_words(m);\n\
                test_out[idx].sq_words = fe_to_words(s);\n\
            }}"
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("onyun-u64-test"),
            source: wgpu::ShaderSource::Wgsl(test_src.into()),
        });

        #[repr(C)]
        #[derive(Clone, Copy, Pod, Zeroable)]
        struct TestPair {
            a: Fe10,
            b: Fe10,
        }

        #[repr(C)]
        #[derive(Clone, Copy, Pod, Zeroable)]
        struct TestOut {
            mul_words: [u32; 8],
            sq_words: [u32; 8],
        }

        const N: usize = 256;
        let mut rng = rand::rng();
        let mut pairs = Vec::with_capacity(N);
        let mut expected_mul = Vec::with_capacity(N);
        let mut expected_sq = Vec::with_capacity(N);
        let mut buf = [0u8; 32];

        for _ in 0..N {
            rng.fill_bytes(&mut buf);
            buf[31] &= 0x7f;
            let a = Fe::from_bytes(&buf);
            rng.fill_bytes(&mut buf);
            buf[31] &= 0x7f;
            let b = Fe::from_bytes(&buf);
            pairs.push(TestPair {
                a: Fe10::from_fe(&a),
                b: Fe10::from_fe(&b),
            });
            expected_mul.push(Fe::mul(&a, &b).to_bytes());
            expected_sq.push(a.sq().to_bytes());
        }

        let in_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (N * std::mem::size_of::<TestPair>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&in_buf, 0, bytemuck::cast_slice(&pairs));

        let out_size = (N * std::mem::size_of::<TestOut>()) as u64;
        let out_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: out_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: out_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let dummy_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&dummy_bgl), Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(&pl),
            module: &shader,
            entry_point: Some("test_mul_sq"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: in_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: out_buf.as_entire_binding(),
                },
            ],
        });
        let dummy_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &dummy_bgl,
            entries: &[],
        });

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &dummy_bg, &[]);
            pass.set_bind_group(1, &bg, &[]);
            pass.dispatch_workgroups((N as u32) / 64, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&out_buf, 0, &staging, 0, out_size);
        queue.submit(Some(encoder.finish()));

        let (tx, rx) = std::sync::mpsc::channel();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        assert!(matches!(rx.recv(), Ok(Ok(()))));

        let Ok(view) = staging.slice(..).get_mapped_range() else {
            panic!("failed to map staging buffer");
        };
        let results: &[TestOut] = bytemuck::cast_slice(&view);
        for i in 0..N {
            assert_eq!(words_to_pubkey(&results[i].mul_words), expected_mul[i], "mul mismatch at {i}");
            assert_eq!(words_to_pubkey(&results[i].sq_words), expected_sq[i], "sq mismatch at {i}");
        }
    }
}
