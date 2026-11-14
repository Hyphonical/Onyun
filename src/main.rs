mod cli;
mod core;
#[cfg(feature = "web")]
mod web;

use crate::cli::render::OutputFlags;
use anyhow::Result;
#[cfg(not(feature = "web"))]
use anyhow::bail;
use cli::args::{Command, parse_args};

fn main() {
    if let Err(err) = run() {
        cli::render::failure(OutputFlags::default(), "", &format!("{err:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match parse_args(std::env::args())? {
        Command::Help(cmd, flags) => {
            cli::help::show(flags, cmd);
            Ok(())
        }
        Command::Version => {
            println!("onyun {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Mint(args) => cli::run_mint(&args),
        Command::Ui(args) => {
            #[cfg(feature = "web")]
            {
                web::run_ui(&args)
            }
            #[cfg(not(feature = "web"))]
            {
                let _ = args;
                bail!("`onyun ui` requires building with `--features web` or `--features desktop`");
            }
        }
    }
}
