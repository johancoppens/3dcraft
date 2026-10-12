//! Headless command-line interface for 3DCraft.
//!
//! Only `--version` and `--help` exist so far; the subcommands in
//! docs/development.md arrive with the engine (M2).

use std::process::ExitCode;

const HELP: &str = "\
craft3d-cli: headless command-line interface for 3DCraft

Usage: craft3d-cli [--version | --help]

No subcommands exist yet. Planned: convert, info, run, render, commands, serve, mcp.";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next()) {
        (Some("--version" | "-V"), None) => {
            println!("craft3d-cli {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        (None | Some("--help" | "-h"), None) => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        (Some(first), _) => {
            eprintln!("craft3d-cli: unknown or unexpected argument `{first}`; try --help");
            ExitCode::from(2)
        }
        (None, Some(_)) => ExitCode::from(2),
    }
}
