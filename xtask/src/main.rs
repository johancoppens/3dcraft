//! Build tasks: `cargo xtask layers | wasm | ci | stats`.

mod layers;
mod stats;

use std::process::{Command, ExitCode};

fn cargo(args: &[&str]) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    eprintln!("$ cargo {}", args.join(" "));
    let status = Command::new(cargo)
        .args(args)
        .status()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed ({status})", args.join(" ")))
    }
}

/// Every library crate must build for the web (AGENTS.md rule 11).
fn wasm() -> Result<(), String> {
    let mut args = vec!["check", "--target", "wasm32-unknown-unknown"];
    let packages: Vec<String> = layers::LAYERS
        .iter()
        .filter(|(_, layer)| *layer != layers::APP_LAYER)
        .map(|(name, _)| format!("craft3d-{name}"))
        .collect();
    for p in &packages {
        args.extend(["-p", p.as_str()]);
    }
    cargo(&args)
}

fn ci() -> Result<(), String> {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&["test", "--workspace"])?;
    layers::check()?;
    wasm()
}

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    let result = match task.as_str() {
        "layers" => layers::check(),
        "wasm" => wasm(),
        "ci" => ci(),
        "stats" => stats::run(),
        other => Err(format!(
            "unknown task `{other}`; tasks: layers, wasm, ci, stats"
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}
