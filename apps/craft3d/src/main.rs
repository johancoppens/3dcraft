//! `craft3d`: the 3DCraft desktop viewer. See `craft3d --help`.
//!
//! Exit codes: 0 success, 1 failure, 2 usage error. Errors go to stderr as
//! one line; there is no panic path.

use craft3d::args::{self, Parsed};
use std::io::Write;
use std::process::ExitCode;

fn say(stream: &mut dyn Write, text: &str) {
    // A closed stream is no reason to fail.
    let _ = writeln!(stream, "{text}");
}

fn main() -> ExitCode {
    let mut arguments = Vec::new();
    for argument in std::env::args_os().skip(1) {
        match argument.into_string() {
            Ok(argument) => arguments.push(argument),
            Err(bad) => {
                say(
                    &mut std::io::stderr(),
                    &format!(
                        "craft3d: argument `{}` is not valid Unicode",
                        bad.to_string_lossy()
                    ),
                );
                return ExitCode::from(2);
            }
        }
    }
    let env = |name: &str| std::env::var(name).ok();
    let options = match args::parse(arguments, &env) {
        Ok(Parsed::Run(options)) => options,
        Ok(Parsed::Help) => {
            say(&mut std::io::stdout(), args::HELP);
            return ExitCode::SUCCESS;
        }
        Ok(Parsed::Version) => {
            say(
                &mut std::io::stdout(),
                &format!("craft3d {}", env!("CARGO_PKG_VERSION")),
            );
            return ExitCode::SUCCESS;
        }
        Err(usage) => {
            say(
                &mut std::io::stderr(),
                &format!("craft3d: {usage}\nTry `craft3d --help`."),
            );
            return ExitCode::from(2);
        }
    };
    let announce = |text: &str| say(&mut std::io::stderr(), &format!("craft3d: {text}"));
    match craft3d::app::run(&options, &announce) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            say(&mut std::io::stderr(), &format!("craft3d: {error}"));
            ExitCode::FAILURE
        }
    }
}
