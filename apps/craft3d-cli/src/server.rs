//! `serve` and `mcp`: the automation servers on stdin and stdout.
//!
//! These are the subcommands an agent talks to, so they get no file access
//! from the environment: only the two root flags grant any, each in its own
//! direction (docs/control-protocol.md, "File access").

use crate::args::{self, Arg, Args};
use crate::{CliError, Parsed};
use craft3d_engine::{Access, RecoveryOptions, Session, Workspace};
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

/// The address of a running window's control channel: `7878`,
/// `127.0.0.1:7878` or `localhost:7878`. Only this machine is accepted,
/// because the token is sent as it is.
fn bridge_address(text: &str) -> Result<std::net::SocketAddr, CliError> {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
    let refused = |why: &str| {
        CliError::Usage(format!(
            "`--bridge {text}`: {why}; give the port of a 3DCraft window on this machine, such as 7878 or 127.0.0.1:7878"
        ))
    };
    if let Ok(port) = text.parse::<u16>() {
        if port == 0 {
            return Err(refused("0 is not a port"));
        }
        return Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
    }
    let address = text
        .to_socket_addrs()
        .ok()
        .and_then(|mut found| found.next())
        .ok_or_else(|| refused("not an address"))?;
    if !address.ip().is_loopback() {
        return Err(refused("the bridge only connects to this machine"));
    }
    Ok(address)
}

/// The token a window wrote to its token file.
fn bridge_token(path: &str) -> Result<String, CliError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        CliError::Failed(format!(
            "`--control-token-file {path}`: cannot read it ({e}); it is the file the window was started with"
        ))
    })?;
    let token = text.trim();
    if token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(token.to_ascii_lowercase())
    } else {
        Err(CliError::Failed(format!(
            "`--control-token-file {path}`: the file does not hold a control token (64 hexadecimal characters)"
        )))
    }
}

/// How the session autosaves: not at all unless `--recovery-dir` names a
/// directory (`none` says so explicitly). The directory is made absolute,
/// so it does not move with the working directory; it need not exist yet.
fn recovery(
    dir: Option<&str>,
    interval: Option<&str>,
) -> Result<Option<RecoveryOptions>, CliError> {
    let interval = interval
        .map(|text| {
            craft3d_engine::parse_autosave_interval(text)
                .map_err(|e| CliError::Usage(format!("`--autosave-interval`: {e}")))
        })
        .transpose()?;
    let dir = match dir {
        None | Some("none") => {
            if interval.is_some() {
                return Err(CliError::Usage(
                    "`--autosave-interval` needs `--recovery-dir DIR`: without a recovery directory nothing is autosaved".to_string(),
                ));
            }
            return Ok(None);
        }
        Some("") => {
            return Err(CliError::Usage(
                "`--recovery-dir` needs a directory, or `none`".to_string(),
            ));
        }
        Some(dir) => dir,
    };
    let path = std::path::absolute(dir).map_err(|e| {
        CliError::Failed(format!(
            "`--recovery-dir {dir}`: cannot use this directory: {e}"
        ))
    })?;
    let mut options = RecoveryOptions::new(path);
    if let Some(interval) = interval {
        options = options.with_interval(interval);
    }
    Ok(Some(options))
}

pub(crate) fn serve(mut args: Args, protocol: Protocol) -> Result<Parsed<()>, CliError> {
    let mut workspace = Workspace::none();
    let mut seen = (false, false);
    let mut bridge = None;
    let mut token_file = None;
    let mut recovery_dir = None;
    let mut interval = None;
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
                "--recovery-dir" | "--autosave-interval" => {
                    let slot = if name == "--recovery-dir" {
                        &mut recovery_dir
                    } else {
                        &mut interval
                    };
                    if slot.is_some() {
                        return Err(CliError::Usage(format!("`{name}` was given twice")));
                    }
                    *slot = Some(args.value(&name, inline)?);
                }
                "--bridge" | "--control-token-file" if matches!(protocol, Protocol::Mcp) => {
                    let slot = if name == "--bridge" {
                        &mut bridge
                    } else {
                        &mut token_file
                    };
                    if slot.is_some() {
                        return Err(CliError::Usage(format!("`{name}` was given twice")));
                    }
                    *slot = Some(args.value(&name, inline)?);
                }
                _ => return Err(CliError::Usage(format!("unknown flag `{name}`"))),
            },
            Arg::Positional(extra) => {
                return Err(CliError::Usage(format!("unexpected argument `{extra}`")));
            }
        }
    }

    let recovery = recovery(recovery_dir.as_deref(), interval.as_deref())?;
    let input = std::io::stdin().lock();
    let output = std::io::stdout().lock();
    if bridge.is_some() || token_file.is_some() {
        let (Some(address), Some(token_file)) = (bridge, token_file) else {
            return Err(CliError::Usage(
                "`--bridge` and `--control-token-file` go together: the window's port and the token file it was started with".to_string(),
            ));
        };
        if seen.0 || seen.1 {
            return Err(CliError::Usage(
                "with `--bridge` the files a command may touch are the window's: give the root flags to `craft3d`, not here".to_string(),
            ));
        }
        if recovery.is_some() {
            return Err(CliError::Usage(
                "with `--bridge` the documents are the window's, and so is their autosave: give `--recovery-dir` to `craft3d`, not here".to_string(),
            ));
        }
        let config = craft3d_automation::bridge::BridgeConfig::new(
            bridge_address(&address)?,
            bridge_token(&token_file)?,
        );
        return finish(craft3d_automation::bridge::serve(&config, input, output));
    }
    let mut session = Session::new(workspace);
    if let Some(options) = recovery {
        session.enable_recovery(options);
    }
    let served = match protocol {
        Protocol::JsonLines => craft3d_automation::rpc::serve(&mut session, input, output),
        Protocol::Mcp => craft3d_automation::mcp::serve(&mut session, input, output),
    };
    // The client closed its end: a clean end, which leaves no recovery
    // files. A server that is killed never gets here and leaves them.
    session.end_recovery();
    finish(served)
}

fn finish(served: std::io::Result<()>) -> Result<Parsed<()>, CliError> {
    match served {
        Ok(()) => Ok(Parsed::Run(())),
        // The client closed its end: the normal way for a session to end.
        Err(e) if e.kind() == ErrorKind::BrokenPipe => Ok(Parsed::Run(())),
        Err(e) => Err(CliError::Failed(format!(
            "the connection on standard input and output failed: {e}"
        ))),
    }
}
