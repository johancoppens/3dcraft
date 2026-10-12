//! `serve` and `mcp`: the automation servers on stdin and stdout.
//!
//! These are the subcommands an agent talks to, so they get no file access
//! from the environment: only the two root flags grant any, each in its own
//! direction (docs/control-protocol.md, "File access").

use crate::args::{self, Arg, Args};
use crate::{CliError, Parsed};
use craft3d_engine::{Access, Session, Workspace};
use std::io::ErrorKind;

pub(crate) enum Protocol {
    JsonLines,
    Mcp,
}

/// Turns a root flag's value into an access rule. The directory must exist,
/// and is made absolute with its symbolic links resolved: the engine
/// compares paths as text, so the root has to be the one true spelling.
fn root(flag: &str, dir: &str) -> Result<Access, CliError> {
    let path = std::fs::canonicalize(dir)
        .map_err(|e| CliError::Failed(format!("`{flag} {dir}`: cannot use this directory: {e}")))?;
    if !path.is_dir() {
        return Err(CliError::Failed(format!("`{flag} {dir}`: not a directory")));
    }
    Ok(Access::Root(path))
}

pub(crate) fn serve(mut args: Args, protocol: Protocol) -> Result<Parsed<()>, CliError> {
    let mut workspace = Workspace::none();
    let mut seen = (false, false);
    while let Some(arg) = args.next() {
        match arg {
            Arg::Flag { name, inline } => match name.as_str() {
                "--help" | "-h" => {
                    args::no_value(&name, inline.as_ref())?;
                    return Ok(Parsed::Help);
                }
                "--automation-read-root" | "--automation-write-root" => {
                    let reading = name == "--automation-read-root";
                    let given = if reading { &mut seen.0 } else { &mut seen.1 };
                    if std::mem::replace(given, true) {
                        return Err(CliError::Usage(format!("`{name}` was given twice")));
                    }
                    let dir = args.value(&name, inline)?;
                    let access = root(&name, &dir)?;
                    if reading {
                        workspace.read = access;
                    } else {
                        workspace.write = access;
                    }
                }
                _ => return Err(CliError::Usage(format!("unknown flag `{name}`"))),
            },
            Arg::Positional(extra) => {
                return Err(CliError::Usage(format!("unexpected argument `{extra}`")));
            }
        }
    }

    let mut session = Session::new(workspace);
    let input = std::io::stdin().lock();
    let output = std::io::stdout().lock();
    let served = match protocol {
        Protocol::JsonLines => craft3d_automation::rpc::serve(&mut session, input, output),
        Protocol::Mcp => craft3d_automation::mcp::serve(&mut session, input, output),
    };
    match served {
        Ok(()) => Ok(Parsed::Run(())),
        // The client closed its end: the normal way for a session to end.
        Err(e) if e.kind() == ErrorKind::BrokenPipe => Ok(Parsed::Run(())),
        Err(e) => Err(CliError::Failed(format!(
            "the connection on standard input and output failed: {e}"
        ))),
    }
}
