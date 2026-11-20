//! `serve` and `mcp`: the automation servers on stdin and stdout.
//!
//! These are the subcommands an agent talks to, so they get no file access
//! from the environment: only the two root flags grant any, each in its own
//! direction (docs/control-protocol.md, "File access").

use crate::args::{self, Arg, Args};
use crate::{CliError, Parsed};
use craft3d_engine::{Access, RecoveryOptions, Workspace};
use std::io::ErrorKind;

pub(crate) enum Protocol {
    JsonLines,
    Mcp,
}

/// Turns a root flag's value into an access rule. The directory must exist,
/// and is made absolute with its symbolic links resolved: the engine
/// compares paths as text, so the root has to be the one true spelling.
///
/// With `make` (the write root), a directory that does not exist yet is
/// made first when its parent exists: that one folder, never its parents.
/// The MCP bundle starts the server with the work folder the person chose
/// in the client, whose default (`${DOCUMENTS}/3DCraft`) a fresh machine
/// does not have yet (2026-11-20). A read root is never made: there would
/// be nothing in it to read.
///
/// A root that does not exist and starts with a variable (`${DOCUMENTS}`)
/// is one a client passed without filling it in, as MCPB's issue 251
/// reports of Claude Desktop with `${HOME}` in a default (Claude Desktop on
/// Linux shows the bundle's default `${DOCUMENTS}/3DCraft` unfilled and
/// saves nothing until a folder is chosen, 2026-11-20). It is refused with
/// what to do, and nothing is made; the server fills in no variable.
fn root(flag: &str, dir: &str, make: bool) -> Result<Access, CliError> {
    let wanted = std::path::Path::new(dir);
    let missing = matches!(wanted.try_exists(), Ok(false));
    if missing && let Some(variable) = unfilled(dir) {
        return Err(CliError::Failed(format!(
            "`{flag} {dir}`: `{variable}` was not filled in by the client that started 3DCraft, \
             which passed its default folder as written; choose a folder in its settings for \
             3DCraft, such as 3DCraft in your documents"
        )));
    }
    if make
        && missing
        && wanted
            .parent()
            .is_some_and(|parent| parent.as_os_str().is_empty() || parent.is_dir())
    {
        // One made meanwhile (by another start) is no error: the checks
        // below hold it to being a directory.
        if let Err(e) = std::fs::create_dir(wanted)
            && e.kind() != ErrorKind::AlreadyExists
        {
            return Err(CliError::Failed(format!(
                "`{flag} {dir}`: cannot make this directory: {e}"
            )));
        }
    }
    let path = craft3d_engine::canonical(wanted)
        .map_err(|e| CliError::Failed(format!("`{flag} {dir}`: cannot use this directory: {e}")))?;
    if !path.is_dir() {
        return Err(CliError::Failed(format!("`{flag} {dir}`: not a directory")));
    }
    Ok(Access::Root(path))
}

/// The variable at the start of a root's text, `${...}`, if there is one
/// (the whole text when its `}` is missing).
fn unfilled(dir: &str) -> Option<&str> {
    if !dir.starts_with("${") {
        return None;
    }
    Some(dir.find('}').and_then(|end| dir.get(..=end)).unwrap_or(dir))
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
        craft3d_automation::discovery::check_private(std::path::Path::new(path))
            .map_err(|e| CliError::Failed(format!("`--control-token-file {path}`: {e}")))?;
        Ok(token.to_ascii_lowercase())
    } else {
        Err(CliError::Failed(format!(
            "`--control-token-file {path}`: the file does not hold a control token (64 hexadecimal characters)"
        )))
    }
}

/// What `--bridge` asks for: the window at a fixed address with the token
/// file given, or one found through discovery in the state folder (on the
/// port given, when one is).
fn bridge_config(
    address: Option<&str>,
    token_file: Option<&str>,
    state_dir: Option<&str>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<craft3d_automation::bridge::BridgeConfig, CliError> {
    use craft3d_automation::bridge::BridgeConfig;
    let address = address.map(bridge_address).transpose()?;
    if let Some(token_file) = token_file {
        let Some(address) = address else {
            return Err(CliError::Usage(
                "`--control-token-file` goes with `--bridge PORT`: the window's port and the token file it was started with".to_string(),
            ));
        };
        if state_dir.is_some() {
            return Err(CliError::Usage(
                "`--state-dir` finds a window by discovery; with `--control-token-file` the window is the one at `--bridge PORT`: give one or the other".to_string(),
            ));
        }
        return Ok(BridgeConfig::new(address, bridge_token(token_file)?));
    }
    let folder = match state_dir {
        Some("") => {
            return Err(CliError::Usage(
                "`--state-dir` needs a directory".to_string(),
            ));
        }
        Some(dir) => std::path::absolute(dir).map_err(|e| {
            CliError::Failed(format!("`--state-dir {dir}`: cannot use this directory: {e}"))
        })?,
        None => match env("CRAFT3D_STATE_DIR").filter(|dir| !dir.is_empty()) {
            Some(dir) => std::path::absolute(&dir).map_err(|e| {
                CliError::Failed(format!(
                    "CRAFT3D_STATE_DIR `{dir}`: cannot use this directory: {e}"
                ))
            })?,
            None => craft3d_automation::discovery::default_state_folder(env).ok_or_else(|| {
                CliError::Failed(
                    "there is no state directory to find a 3DCraft window in (HOME is not set); give `--bridge PORT --control-token-file FILE`".to_string(),
                )
            })?,
        },
    };
    Ok(BridgeConfig::discover(
        folder,
        address.map(|address| address.port()),
    ))
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
    // The values of `--automation-read-root` and `--automation-write-root`.
    let mut roots: (Option<String>, Option<String>) = (None, None);
    let mut bridge: Option<Option<String>> = None;
    let mut token_file = None;
    let mut state_dir = None;
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
                    let slot = if name == "--automation-read-root" {
                        &mut roots.0
                    } else {
                        &mut roots.1
                    };
                    if slot.is_some() {
                        return Err(CliError::Usage(format!("`{name}` was given twice")));
                    }
                    *slot = Some(args.value(&name, inline)?);
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
                "--bridge" if matches!(protocol, Protocol::Mcp) => {
                    if bridge.is_some() {
                        return Err(CliError::Usage(format!("`{name}` was given twice")));
                    }
                    // The port may be left out: the window is found then.
                    bridge = Some(args.optional_value(inline));
                }
                "--control-token-file" | "--state-dir" if matches!(protocol, Protocol::Mcp) => {
                    let slot = if name == "--state-dir" {
                        &mut state_dir
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
    if bridge.is_some() || token_file.is_some() || state_dir.is_some() {
        let Some(address) = bridge else {
            return Err(CliError::Usage(
                "`--control-token-file` and `--state-dir` go with `--bridge`: they say which window to pass the messages to".to_string(),
            ));
        };
        if roots.0.is_some() || roots.1.is_some() {
            return Err(CliError::Usage(
                "with `--bridge` the files a command may touch are the window's: give the root flags to `craft3d`, not here".to_string(),
            ));
        }
        if recovery.is_some() {
            return Err(CliError::Usage(
                "with `--bridge` the documents are the window's, and so is their autosave: give `--recovery-dir` to `craft3d`, not here".to_string(),
            ));
        }
        let env = |name: &str| std::env::var(name).ok();
        let config = bridge_config(
            address.as_deref(),
            token_file.as_deref(),
            state_dir.as_deref(),
            &env,
        )?;
        return finish(craft3d_automation::bridge::serve(&config, input, output));
    }
    // The roots once the rest of the command line was found sound, so no
    // folder is made for a usage error. The write root first: a missing
    // one is made, and a read root naming the same folder (as the MCP
    // bundle's does) then finds it. A read root refused after that leaves
    // the write root made, as it was asked for.
    let mut workspace = Workspace::none();
    if let Some(dir) = &roots.1 {
        workspace.write = root("--automation-write-root", dir, true)?;
    }
    if let Some(dir) = &roots.0 {
        workspace.read = root("--automation-read-root", dir, false)?;
    }
    let mut session = crate::session(workspace);
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
