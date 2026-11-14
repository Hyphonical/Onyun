//! Handwritten zero-dependency CLI argument parser for `onyun`.

use crate::cli::render::OutputFlags;
use crate::core::engine::EngineKind;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

#[derive(Debug)]
pub(crate) enum Command {
    Mint(MintArgs),
    Ui(UiArgs),
    Help(Option<&'static str>, OutputFlags),
    Version,
}

#[derive(Debug)]
pub(crate) struct MintArgs {
    pub(crate) target: String,
    pub(crate) threads: usize,
    pub(crate) out_dir: PathBuf,
    pub(crate) engine: EngineKind,
    pub(crate) flags: OutputFlags,
}

#[derive(Debug)]
#[cfg_attr(not(feature = "web"), allow(dead_code))]
pub(crate) struct UiArgs {
    pub(crate) port: u16,
    pub(crate) bind: String,
    pub(crate) web: bool,
    pub(crate) engine: EngineKind,
    pub(crate) out_dir: PathBuf,
    pub(crate) flags: OutputFlags,
}

pub(crate) fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(4)
}

pub(crate) fn parse_args<I>(args: I) -> Result<Command>
where
    I: Iterator<Item = String>,
{
    let mut flags = OutputFlags::default();
    let mut rest = Vec::new();
    for arg in args.skip(1) {
        if arg == "--no-color" {
            flags.no_color = true;
        } else {
            rest.push(arg);
        }
    }

    let mut iter = rest.into_iter();
    let Some(sub) = iter.next() else {
        return Ok(Command::Help(None, flags));
    };

    match sub.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help(None, flags)),
        "-V" | "--version" | "version" => Ok(Command::Version),
        "mint" => parse_mint(iter, flags),
        "ui" => parse_ui(iter, flags),
        other => {
            bail!("unknown subcommand '{other}' (expected `mint` or `ui`; run `onyun --help`)")
        }
    }
}

fn parse_mint<I>(mut args: I, mut flags: OutputFlags) -> Result<Command>
where
    I: Iterator<Item = String>,
{
    let mut target: Option<String> = None;
    let mut threads = default_threads();
    let mut engine = EngineKind::Auto;
    let mut out_dir = PathBuf::from("./mints");

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help(Some("mint"), flags)),
            "-q" | "--quiet" => flags.quiet = true,
            "--no-color" => flags.no_color = true,
            "-t" | "--threads" => {
                let val = args.next().context("missing value for --threads")?;
                threads = val
                    .parse::<usize>()
                    .context("--threads must be a positive integer")?;
                if threads == 0 {
                    bail!("--threads must be at least 1");
                }
            }
            "-o" | "--out" => {
                let val = args.next().context("missing value for --out")?;
                out_dir = PathBuf::from(val);
            }
            "-e" | "--engine" => {
                let val = args.next().context("missing value for --engine")?;
                engine = EngineKind::parse(&val)?;
            }
            a if a.starts_with('-') => bail!("unknown flag '{a}' for `onyun mint`"),
            pos => {
                if target.is_some() {
                    bail!("unexpected extra argument '{pos}'");
                }
                target = Some(pos.to_string());
            }
        }
    }

    let target = target.context("missing <TARGET> prefix (usage: `onyun mint <TARGET>`)")?;
    Ok(Command::Mint(MintArgs {
        target,
        threads,
        engine,
        out_dir,
        flags,
    }))
}

fn parse_ui<I>(mut args: I, flags: OutputFlags) -> Result<Command>
where
    I: Iterator<Item = String>,
{
    let mut port: Option<u16> = None;
    let mut bind = String::from("127.0.0.1");
    let mut web = false;
    let mut engine = EngineKind::Auto;
    let mut out_dir = PathBuf::from("./mints");

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help(Some("ui"), flags)),
            "--web" | "--headless" => web = true,
            "-p" | "--port" => {
                let val = args.next().context("missing value for --port")?;
                port = Some(val.parse::<u16>().context("--port must be 0..=65535")?);
            }
            "-b" | "--bind" => {
                bind = args.next().context("missing value for --bind")?;
            }
            "-e" | "--engine" => {
                let val = args.next().context("missing value for --engine")?;
                engine = EngineKind::parse(&val)?;
            }
            "-o" | "--out" => {
                let val = args.next().context("missing value for --out")?;
                out_dir = PathBuf::from(val);
            }
            a if a.starts_with('-') => bail!("unknown flag '{a}' for `onyun ui`"),
            pos => bail!("unexpected argument '{pos}' for `onyun ui`"),
        }
    }

    let resolved_port = port.unwrap_or(if web || bind != "127.0.0.1" { 8080 } else { 0 });
    Ok(Command::Ui(UiArgs {
        port: resolved_port,
        bind,
        web,
        engine,
        out_dir,
        flags,
    }))
}
