//! Styled help and version presentation for `onyun`.

use super::{
    render::{clean, column},
    style::{self, Color},
};
use crate::cli::render::OutputFlags;
use std::io::IsTerminal;

fn stdout_interactive() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("TERM").is_none_or(|t| t != "dumb")
}

fn stdout_colored(flags: OutputFlags) -> bool {
    stdout_interactive() && !flags.no_color
}

fn help_rule(flags: OutputFlags, label: &str) {
    if !stdout_interactive() {
        return;
    }
    let label = clean(label);
    let rest = "━".repeat(80usize.saturating_sub(label.chars().count() + 8));
    let c = stdout_colored(flags);
    println!(
        "  {}  {}  {}",
        style::paint("━━", Color::Muted, c),
        style::paint(&label, Color::Heading, c),
        style::paint(&rest, Color::Muted, c),
    );
}

fn help_line(flags: OutputFlags, text: &str, color: Color) {
    println!(
        "  {}",
        style::paint(&clean(text), color, stdout_colored(flags))
    );
}

fn help_item(flags: OutputFlags, text: &str, color: Color) {
    println!(
        "    {}",
        style::paint(&clean(text), color, stdout_colored(flags))
    );
}

fn help_blank() {
    println!();
}

fn help_group(flags: OutputFlags, title: &str) {
    help_blank();
    help_line(flags, title, Color::Heading);
}

fn option(flags: OutputFlags, name: &str, alias: &str, desc: &str) {
    help_item(
        flags,
        &format!("{} {}  {desc}", column(name, 22), column(alias, 3)),
        Color::Text,
    );
}

pub(crate) fn show(flags: OutputFlags, command: Option<&str>) {
    let flags = OutputFlags {
        quiet: false,
        ..flags
    };
    let cmd = command.unwrap_or("");
    let ver = env!("CARGO_PKG_VERSION");
    let title = match cmd {
        "mint" => format!("Onyun Mint (v{ver})"),
        "ui" => format!("Onyun UI (v{ver})"),
        _ => format!("Onyun (v{ver})"),
    };
    help_rule(flags, &title);

    let description = match cmd {
        "mint" => "Mint a Tor v3 .onion address left-to-right with automatic best-so-far saves.",
        "ui" => "Launch the Onyun desktop window or headless Web UI.",
        _ => "High-speed Tor v3 .onion vanity address minter.",
    };
    help_line(flags, description, Color::Dim);

    help_group(flags, "Usage");
    let usage = match cmd {
        "mint" => "onyun mint <target> [options]",
        "ui" => "onyun ui [options]",
        _ => "onyun <command> [options]",
    };
    help_item(flags, usage, Color::Text);

    match cmd {
        "mint" => {
            help_group(flags, "Options");
            option(flags, "--threads <n>", "-t", "Worker threads (default: CPU cores)");
            option(flags, "--out <dir>", "-o", "Base output directory (default: ./mints)");
            option(flags, "--quiet", "-q", "Print final .onion address only");
            option(flags, "--no-color", "", "Disable ANSI colors");
            option(flags, "--help", "-h", "Show help");
            help_group(flags, "Examples");
            help_item(flags, "onyun mint onyun", Color::Dim);
            help_item(flags, "onyun mint onyunshop -t 16 -o ./keys", Color::Dim);
        }
        "ui" => {
            help_group(flags, "Options");
            option(flags, "--port <port>", "-p", "Port to bind (default: 0 desktop, 8080 web)");
            option(flags, "--bind <ip>", "-b", "Bind address (default: 127.0.0.1)");
            option(flags, "--web", "", "Serve Web UI only (skip desktop window)");
            option(flags, "--out <dir>", "-o", "Base output directory (default: ./mints)");
            option(flags, "--help", "-h", "Show help");
            help_group(flags, "Examples");
            help_item(flags, "onyun ui", Color::Dim);
            help_item(flags, "onyun ui --web --bind 0.0.0.0 --port 8080", Color::Dim);
        }
        _ => {
            help_group(flags, "Commands");
            for (name, desc) in [
                ("mint", "Mint a Tor v3 address left-to-right in the terminal"),
                ("ui", "Launch the Onyun desktop window or headless Web UI"),
            ] {
                help_item(
                    flags,
                    &format!("{}  {desc}", column(name, 14)),
                    Color::Text,
                );
            }

            help_group(flags, "Global settings");
            option(flags, "--no-color", "", "Disable ANSI colors");
            option(flags, "--help", "-h", "Show help");
            option(flags, "--version", "-V", "Show version");

            help_group(flags, "Examples");
            help_item(flags, "onyun mint onyun", Color::Dim);
            help_item(flags, "onyun mint onyunshop --threads 16", Color::Dim);
            help_item(flags, "onyun ui --web --port 8080", Color::Dim);
            help_blank();
            help_line(flags, "Run onyun <command> --help for command options.", Color::Dim);
        }
    }
    help_blank();
}
