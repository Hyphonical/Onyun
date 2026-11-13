//! Human-facing CLI output, formatting helpers, and logarithmic progress bar.

use super::style::{self, Color};
use std::borrow::Cow;
use std::io::{self, IsTerminal, Write};
use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
const RULE_WIDTH: usize = 80;
const BAR_WIDTH: usize = 28;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OutputFlags {
    pub(crate) quiet: bool,
    pub(crate) no_color: bool,
}

static INTERACTIVE: LazyLock<bool> = LazyLock::new(|| {
    std::io::stderr().is_terminal() && std::env::var_os("TERM").is_none_or(|t| t != "dumb")
});

pub(crate) fn interactive() -> bool {
    *INTERACTIVE
}

fn colored(flags: OutputFlags) -> bool {
    interactive() && !flags.no_color
}

pub(crate) fn clean(text: &str) -> Cow<'_, str> {
    if !text.chars().any(char::is_control) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .map(|ch| if ch.is_control() { '\u{FFFD}' } else { ch })
            .collect(),
    )
}

pub(crate) fn blank(flags: OutputFlags) {
    if !flags.quiet && interactive() {
        eprintln!();
    }
}

pub(crate) fn rule(flags: OutputFlags, label: &str) {
    if flags.quiet || !interactive() {
        return;
    }
    let label = clean(label);
    let rest = "━".repeat(RULE_WIDTH.saturating_sub(label.chars().count() + 8));
    eprintln!(
        "  {}  {}  {}",
        style::paint("━━", Color::Muted, colored(flags)),
        style::paint(&label, Color::Heading, colored(flags)),
        style::paint(&rest, Color::Muted, colored(flags)),
    );
}

pub(crate) fn event(flags: OutputFlags, command: &str, message: &str) {
    if flags.quiet {
        return;
    }
    if interactive() {
        eprintln!("  {}", style::paint(&clean(message), Color::Text, colored(flags)));
    } else {
        eprintln!("{}  {command}  {}", utc_timestamp(), clean(message));
    }
}

pub(crate) fn row(flags: OutputFlags, command: &str, label: &str, value: &str, color: Color) {
    if flags.quiet {
        return;
    }
    if !interactive() {
        event(flags, command, &format!("{label} {value}"));
        return;
    }
    let label = column(label, 10);
    let value = clean(value);
    eprintln!(
        "  {}{}",
        style::paint(&label, Color::Dim, colored(flags)),
        style::paint(&value, color, colored(flags)),
    );
}

pub(crate) fn done(flags: OutputFlags, command: &str, label: &str, message: &str) {
    if flags.quiet {
        return;
    }
    if interactive() {
        let mut err = io::stderr().lock();
        let label = column(label, 10);
        let message = format!("✓  {}", clean(message));
        let c = colored(flags);
        let _ = writeln!(
            err,
            "{}  {}{}",
            style::clear_line(),
            style::paint(&label, Color::Dim, c),
            style::paint(&message, Color::Accent, c),
        );
        let _ = err.flush();
    } else {
        event(flags, command, &format!("{label} {message}"));
    }
}

pub(crate) fn failure(flags: OutputFlags, command: &str, message: &str) {
    let text = clean(message);
    if interactive() {
        let mut err = io::stderr().lock();
        let _ = writeln!(
            err,
            "{}  {}",
            style::clear_line(),
            style::paint(&format!("✕  {text}"), Color::Error, colored(flags)),
        );
        let _ = err.flush();
    } else if command.is_empty() {
        eprintln!("{}  error  {text}", utc_timestamp());
    } else {
        eprintln!("{}  {command}  error  {text}", utc_timestamp());
    }
}

pub(crate) fn clear_progress(flags: OutputFlags) {
    if !flags.quiet && interactive() {
        let mut err = io::stderr().lock();
        let _ = write!(err, "{}", style::clear_line());
        let _ = err.flush();
    }
}

pub(crate) fn progress_line(
    flags: OutputFlags,
    progress: f64,
    best_len: usize,
    target_len: usize,
    hashes_per_sec: f64,
    eta_secs: Option<f64>,
) {
    if flags.quiet || !interactive() {
        return;
    }
    let fill = ((progress * BAR_WIDTH as f64).round() as usize).min(BAR_WIDTH);
    let percent = ((progress * 100.0).round() as u64).min(100);
    let label = column("Minting", 10);
    let rate = format_rate(hashes_per_sec);
    let eta = eta_secs.map_or_else(|| "—".to_string(), format_duration_secs);
    let summary = format!(
        "  {label}{}{}  {percent:>3}%  {best_len}/{target_len}  {rate}  ETA {eta}",
        "━".repeat(fill),
        "─".repeat(BAR_WIDTH - fill),
    );
    let mut err = io::stderr().lock();
    let _ = write!(
        err,
        "\r{}\x1b[K",
        style::paint(&summary, Color::Accent, colored(flags))
    );
    let _ = err.flush();
}

pub(crate) fn column(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let text = clean(text);
    let count = text.chars().count();
    if count <= width {
        return format!("{text}{}", " ".repeat(width - count));
    }
    let keep = width.saturating_sub(1);
    let mut truncated: String = text.chars().take(keep).collect();
    truncated.push('…');
    truncated
}

pub(crate) fn format_rate(hashes_per_sec: f64) -> String {
    if hashes_per_sec >= 1_000_000_000.0 {
        format!("{:.2} GH/s", hashes_per_sec / 1_000_000_000.0)
    } else if hashes_per_sec >= 1_000_000.0 {
        format!("{:.2} MH/s", hashes_per_sec / 1_000_000.0)
    } else if hashes_per_sec >= 1_000.0 {
        format!("{:.1} kH/s", hashes_per_sec / 1_000.0)
    } else {
        format!("{hashes_per_sec:.0} H/s")
    }
}

pub(crate) fn format_duration(d: Duration) -> String {
    format!("{:.1}s", d.as_secs_f64())
}

pub(crate) fn format_delta(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 10.0 {
        format!("{secs:.1}s")
    } else {
        format_duration_secs(secs)
    }
}
pub(crate) fn format_duration_secs(secs: f64) -> String {
    if !secs.is_finite() || secs > 31_536_000.0 * 10.0 {
        return ">10y".to_string();
    }
    let s = secs.round() as u64;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {}s", s / 60, s % 60)
    } else if s < 86400 {
        format!("{}h {}m", s / 3600, (s % 3600) / 60)
    } else {
        format!("{}d {}h", s / 86400, (s % 86400) / 3600)
    }
}

/// Formats the current system time as `YYYY-MM-DDTHH:MM:SSZ` without external crates.
fn utc_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = (secs / 86400) as i64;
    let sod = secs % 86400;
    let hour = sod / 3600;
    let min = (sod % 3600) / 60;
    let sec = sod % 60;

    // Howard Hinnant's civil-from-days algorithm
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(m <= 2);

    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}
