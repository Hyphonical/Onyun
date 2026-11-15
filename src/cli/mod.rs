//! CLI command handlers for `onyun`.

pub(crate) mod args;
pub(crate) mod help;
pub(crate) mod render;
pub(crate) mod style;
use crate::cli::style::Color;

use crate::cli::args::MintArgs;
use crate::core::base32::normalize_target;
use crate::core::engine::{
    MintConfig, MintGoal, MintMilestone, SharedTelemetry, compute_progress, select_engine,
};
use crate::core::storage::{load_existing_best, target_dir};
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

pub(crate) fn run_mint(args: &MintArgs) -> Result<()> {
    let target = normalize_target(&args.target)?;
    let flags = args.flags;

    let existing = load_existing_best(&args.out_dir, &target);
    let initial_best_len = existing.as_ref().map_or(0, |(_, len)| *len);
    let best_address = existing.as_ref().map(|(addr, _)| addr.clone());

    let out_path = target_dir(&args.out_dir, &target);
    let difficulty = format!(
        "1 in 32^{} ({:.2e})",
        target.len(),
        32f64.powi(target.len() as i32)
    );

    let engine = select_engine(args.engine)?;
    render::blank(flags);
    render::rule(
        flags,
        &format!("Onyun Mint (v{})", env!("CARGO_PKG_VERSION")),
    );
    render::row(flags, "mint", "Target", &target, Color::Heading);
    render::row(flags, "mint", "Engine", engine.label(), Color::Text);
    render::row(flags, "mint", "Device", &engine.device_name(), Color::Text);
    render::row(flags, "mint", "Odds", &difficulty, Color::Dim);
    if engine.label() == "CPU" {
        render::row(
            flags,
            "mint",
            "Threads",
            &args.threads.to_string(),
            Color::Text,
        );
    }
    render::row(
        flags,
        "mint",
        "Output",
        &out_path.display().to_string(),
        Color::Dim,
    );
    if let Some(addr) = &best_address {
        render::done(
            flags,
            "mint",
            "Resumed",
            &format!("{addr} ({initial_best_len}/{})", target.len()),
        );
    }

    if initial_best_len >= target.len() {
        render::done(
            flags,
            "mint",
            "Minted",
            best_address.as_deref().unwrap_or(""),
        );
        render::blank(flags);
        if flags.quiet
            && let Some(addr) = best_address
        {
            println!("{addr}");
        }
        return Ok(());
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_signal = Arc::clone(&cancel);
    let _ = ctrlc::set_handler(move || {
        cancel_signal.store(true, Ordering::SeqCst);
    });

    let slots = engine.slot_count(args.threads);
    let telemetry = Arc::new(SharedTelemetry::new(slots, initial_best_len));
    let (tx, rx) = mpsc::channel::<MintMilestone>();

    let handle = engine.start(
        MintConfig {
            goal: MintGoal::Prefix(target.clone()),
            threads: args.threads,
            telemetry_offset: 0,
        },
        Arc::clone(&telemetry),
        tx,
        Arc::clone(&cancel),
    )?;

    let mut milestones = render::MilestoneState {
        out_dir: &args.out_dir,
        target: &target,
        flags,
        command: "mint",
        best_len: initial_best_len,
        best_address,
        last_mint_at: telemetry.started_at,
    };
    let mut last_hashes: u64 = 0;
    let mut last_tick = Instant::now();
    let mut ewma = render::EwmaRate::default();

    while !cancel.load(Ordering::Relaxed) && milestones.best_len < target.len() {
        if milestones.drain(&rx, &telemetry) {
            cancel.store(true, Ordering::SeqCst);
            break;
        }

        if milestones.best_len >= target.len() || cancel.load(Ordering::Relaxed) {
            break;
        }

        std::thread::sleep(Duration::from_millis(100));

        let now = Instant::now();
        let dt = now.duration_since(last_tick).as_secs_f64().max(0.001);
        let total = telemetry.total_hashes();
        let delta = total.saturating_sub(last_hashes);
        ewma.update(delta, dt);
        last_hashes = total;
        last_tick = now;

        let hashes_since_best =
            total.saturating_sub(telemetry.hashes_at_last_best.load(Ordering::Relaxed));
        let rate = ewma.get();
        let (progress, _pace, eta) =
            compute_progress(milestones.best_len, target.len(), hashes_since_best, rate);

        render::progress_line(flags, progress, milestones.best_len, target.len(), rate, eta);
    }

    cancel.store(true, Ordering::SeqCst);
    handle.join();
    render::clear_progress(flags);

    let elapsed = telemetry.started_at.elapsed();
    let total_hashes = telemetry.total_hashes();
    let avg_rate = (total_hashes as f64) / elapsed.as_secs_f64().max(0.001);

    if let Some(addr) = &milestones.best_address {
        if flags.quiet {
            println!("{addr}");
        } else {
            render::mint_summary(flags, "mint", elapsed, avg_rate, Some(&out_path));
        }
    }

    Ok(())
}
