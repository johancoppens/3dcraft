//! The command line of the desktop app, and the environment variables that
//! stand in for its flags (docs/development.md, "Environment variables").
//!
//! `--flag VALUE` or `--flag=VALUE`; `--` ends the flags, so a file name may
//! start with a dash after it. A flag wins over its environment variable.

use craft3d_ui_egui::RendererChoice;
use std::path::PathBuf;
use std::time::Duration;

pub const HELP: &str = "\
craft3d: the 3DCraft desktop viewer

Usage: craft3d [FILE] [--control PORT] [--control-token-file FILE]
               [--automation-read-root DIR] [--automation-write-root DIR]
               [--state-dir DIR]
               [--renderer auto|cpu]
               [--recovery-dir DIR|none] [--autosave-interval SECONDS]
               [--recent-file FILE|none] [--window-size WIDTHxHEIGHT]
       craft3d --version | --help

Opens FILE (a .3dcraft document, or a .3dcraftjig Jig, opened to set its
parameters) when one is given, and shows the active
document: bodies shaded with their edges, sketches as lines. The window
does not model; the document changes through commands, sent over the
control channel. It does open and save files: the File menu, with the
system's file dialog, wherever you may read and write.

AI agents: the switch Allow AI agents (on the start screen, in the Help
menu) lets agents drive the window over MCP with `craft3d-cli mcp
--bridge`, which finds the window by itself. Off by default; the choice is
remembered. The window then listens on 127.0.0.1, port 7878 or the next
free one, behind the control token in control.token in the state folder.

Options:
  --control PORT              Listen on 127.0.0.1:PORT for JSON-lines requests
                              (docs/control-protocol.md) from the start, as
                              if Allow AI agents were on. 0 picks a free
                              port; the address is printed on standard error.
  --control-token-file FILE   The file with the control token. Created with a
                              fresh token (mode 0600) when it does not exist.
                              Default: control.token in the state folder.
  --automation-read-root DIR  Requests may read files beneath DIR only
  --automation-write-root DIR Requests may write files beneath DIR only
                              Without a root, that direction is denied; with
                              neither, an agent the switch let in gets
                              Documents/3DCraft for both.
  --state-dir DIR             3DCraft's own folder: the control token, the
                              entries the bridge finds windows by, the
                              remembered switch, and the default recovery
                              directory and recent files. Default: 3dcraft
                              in your state directory (see --recovery-dir).
  --renderer auto|cpu         What shades the model in the viewport: auto
                              (the GPU when there is one, else the CPU; the
                              default) or cpu (the reference renderer, to
                              compare). The screenshot is always the CPU's.
  --recovery-dir DIR|none     Where documents with unsaved changes are
                              autosaved, and where the next start looks for
                              what a crash left behind. Default: 3dcraft/
                              recovery in your state directory (Linux:
                              $XDG_STATE_HOME or ~/.local/state; macOS:
                              ~/Library/Application Support; Windows:
                              %LOCALAPPDATA%). none turns autosave off.
  --autosave-interval SECONDS How long after a change a document is
                              autosaved. Default: 30
  --recent-file FILE|none     Where the list of recent files is kept.
                              Default: 3dcraft/recent-files.json in your
                              state directory, beside the default recovery
                              directory; with --recovery-dir given (or
                              none), or none here, the list is kept for
                              this run only.
  --window-size WIDTHxHEIGHT  The window's size when it opens, in points
                              (e.g. 1920x1080; at least 480x320).
                              Default: 1280x800

Environment: CRAFT3D_CONTROL_PORT, CRAFT3D_CONTROL_TOKEN_FILE, CRAFT3D_STATE_DIR,
CRAFT3D_AUTOMATION_READ_ROOT, CRAFT3D_AUTOMATION_WRITE_ROOT,
CRAFT3D_RENDERER, CRAFT3D_RECOVERY_DIR, CRAFT3D_AUTOSAVE_INTERVAL,
CRAFT3D_RECENT_FILE and CRAFT3D_WINDOW_SIZE stand in for the flags of the
same name. CRAFT3D_AGENT_PORT is the first port Allow AI agents tries
(default 7878; a test's window keeps off the person's port with it).

Mouse: drag to orbit, right-drag or shift-drag to pan, wheel to zoom, F to
fly to the selection (or frame everything).
Keys: Ctrl+N new, Ctrl+O open, Ctrl+S save, Ctrl+Shift+S save as; F1
lists every shortcut.";

/// What to start with.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Options {
    /// A `.3dcraft` document or a `.3dcraftjig` Jig to open.
    pub file: Option<String>,
    /// The port of the control listener; `None` for no listener.
    pub control: Option<u16>,
    pub token_file: Option<PathBuf>,
    pub read_root: Option<String>,
    pub write_root: Option<String>,
    /// What renders the viewport.
    pub renderer: RendererChoice,
    /// Where documents are autosaved; `None` for no autosave. Parsing
    /// fills in the platform's default; `Options::default()` has none, so
    /// nothing that builds its options by hand writes there.
    pub recovery_dir: Option<PathBuf>,
    /// How long after a change a document is autosaved; `None` for the
    /// engine's default.
    pub autosave_interval: Option<Duration>,
    /// Where the recent files are kept; `None` keeps them for the run
    /// only. Parsing fills in the platform's default, as for
    /// `recovery_dir`.
    pub recent_file: Option<PathBuf>,
    /// The window's size when it opens, in points; `None` for the default.
    pub window_size: Option<[u32; 2]>,
    /// 3DCraft's own folder in the state directory: the default token
    /// file, the discovery entries and the remembered switch for agents.
    /// Parsing fills in the platform's default, as for `recovery_dir`.
    pub state_dir: Option<PathBuf>,
    /// The first port the switch Allow AI agents tries
    /// (`CRAFT3D_AGENT_PORT`; 7878 when not given): a test's or an agent's
    /// window keeps off the person's port with it. 0 takes any free port.
    pub agent_port: Option<u16>,
}

/// What the command line asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Options),
    Help,
    Version,
}

/// The smallest window `--window-size` takes, width and height (the
/// window's own least size), and the largest side.
const WINDOW_LEAST: [u32; 2] = [480, 320];
const WINDOW_MOST: u32 = 16384;

fn window_size(text: &str, from: &str) -> Result<[u32; 2], String> {
    let shown: String = text.chars().take(40).collect();
    let bad = || {
        format!(
            "{from}: `{shown}` is not a window size (WIDTHxHEIGHT, at least {}x{}, at most {} each)",
            WINDOW_LEAST[0], WINDOW_LEAST[1], WINDOW_MOST
        )
    };
    let (w, h) = text.trim().split_once(['x', 'X']).ok_or_else(bad)?;
    let side = |t: &str, least: u32| {
        t.trim()
            .parse::<u32>()
            .ok()
            .filter(|n| (least..=WINDOW_MOST).contains(n))
            .ok_or_else(bad)
    };
    Ok([side(w, WINDOW_LEAST[0])?, side(h, WINDOW_LEAST[1])?])
}

fn port(text: &str, from: &str) -> Result<u16, String> {
    text.trim().parse::<u16>().map_err(|_| {
        let shown: String = text.chars().take(40).collect();
        format!("{from}: `{shown}` is not a port number (0 to 65535)")
    })
}

/// Parses the arguments after the program name. `env` looks up an
/// environment variable. Errors are usage errors, as sentences.
pub fn parse(
    args: impl IntoIterator<Item = String>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Parsed, String> {
    let mut options = Options::default();
    let mut args = args.into_iter();
    let mut flags_ended = false;
    let mut seen: Vec<String> = Vec::new();
    let mut recovery_dir: Option<String> = None;
    let mut recent_file: Option<String> = None;
    let mut state_dir: Option<String> = None;
    while let Some(arg) = args.next() {
        if flags_ended || !arg.starts_with('-') || arg == "-" {
            if options.file.is_some() {
                return Err(format!(
                    "unexpected argument `{arg}`: only one file can be opened"
                ));
            }
            options.file = Some(arg);
            continue;
        }
        if arg == "--" {
            flags_ended = true;
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) => (name.to_string(), Some(value.to_string())),
            None => (arg, None),
        };
        match name.as_str() {
            "--help" | "-h" | "--version" | "-V" => {
                if inline.is_some() {
                    return Err(format!("`{name}` takes no value"));
                }
                return Ok(if name.contains('h') {
                    Parsed::Help
                } else {
                    Parsed::Version
                });
            }
            "--control"
            | "--control-token-file"
            | "--automation-read-root"
            | "--automation-write-root"
            | "--renderer"
            | "--recovery-dir"
            | "--autosave-interval"
            | "--recent-file"
            | "--state-dir"
            | "--window-size" => {
                if seen.contains(&name) {
                    return Err(format!("`{name}` was given twice"));
                }
                let value = inline
                    .or_else(|| args.next())
                    .ok_or_else(|| format!("`{name}` needs a value"))?;
                match name.as_str() {
                    "--control" => options.control = Some(port(&value, "--control")?),
                    "--control-token-file" => options.token_file = Some(PathBuf::from(value)),
                    "--automation-read-root" => options.read_root = Some(value),
                    "--recovery-dir" => recovery_dir = Some(value),
                    "--recent-file" => recent_file = Some(value),
                    "--state-dir" => {
                        if value.is_empty() {
                            return Err("`--state-dir` needs a directory".to_string());
                        }
                        state_dir = Some(value);
                    }
                    "--window-size" => {
                        options.window_size = Some(window_size(&value, "--window-size")?);
                    }
                    "--autosave-interval" => {
                        options.autosave_interval = Some(
                            craft3d_engine::parse_autosave_interval(&value)
                                .map_err(|e| format!("--autosave-interval: {e}"))?,
                        );
                    }
                    "--renderer" => {
                        options.renderer = RendererChoice::parse(&value)
                            .map_err(|e| format!("--renderer: {e}"))?;
                    }
                    _ => options.write_root = Some(value),
                }
                seen.push(name);
            }
            _ => return Err(format!("unknown flag `{name}`")),
        }
    }
    let from_env = |name: &str| env(name).filter(|value| !value.is_empty());
    if options.control.is_none()
        && let Some(value) = from_env("CRAFT3D_CONTROL_PORT")
    {
        options.control = Some(port(&value, "CRAFT3D_CONTROL_PORT")?);
    }
    if options.token_file.is_none() {
        options.token_file = from_env("CRAFT3D_CONTROL_TOKEN_FILE").map(PathBuf::from);
    }
    if options.read_root.is_none() {
        options.read_root = from_env("CRAFT3D_AUTOMATION_READ_ROOT");
    }
    if options.write_root.is_none() {
        options.write_root = from_env("CRAFT3D_AUTOMATION_WRITE_ROOT");
    }
    if !seen.iter().any(|flag| flag == "--renderer")
        && let Some(value) = from_env("CRAFT3D_RENDERER")
    {
        options.renderer =
            RendererChoice::parse(&value).map_err(|e| format!("CRAFT3D_RENDERER: {e}"))?;
    }
    if options.autosave_interval.is_none()
        && let Some(value) = from_env("CRAFT3D_AUTOSAVE_INTERVAL")
    {
        options.autosave_interval = Some(
            craft3d_engine::parse_autosave_interval(&value)
                .map_err(|e| format!("CRAFT3D_AUTOSAVE_INTERVAL: {e}"))?,
        );
    }
    if let Some(value) = from_env("CRAFT3D_AGENT_PORT") {
        options.agent_port = Some(port(&value, "CRAFT3D_AGENT_PORT")?);
    }
    if options.window_size.is_none()
        && let Some(value) = from_env("CRAFT3D_WINDOW_SIZE")
    {
        options.window_size = Some(window_size(&value, "CRAFT3D_WINDOW_SIZE")?);
    }
    // The state folder: the flag, else the variable, else the platform's
    // place (the folder of the default recovery directory).
    let state_given = state_dir
        .or_else(|| from_env("CRAFT3D_STATE_DIR"))
        .map(PathBuf::from);
    options.state_dir = state_given.clone().or_else(|| {
        craft3d_engine::default_recovery_directory(env)
            .and_then(|recovery| recovery.parent().map(std::path::Path::to_path_buf))
    });
    // The flag, else the variable, else the platform's place (in a state
    // folder given, there); `none` turns autosave off, and so does a
    // platform that names no place.
    let recovery_given = recovery_dir.or_else(|| from_env("CRAFT3D_RECOVERY_DIR"));
    let own_recovery = recovery_given.is_none();
    options.recovery_dir = match recovery_given {
        Some(none) if none == "none" => None,
        Some(dir) => Some(PathBuf::from(dir)),
        None => match &state_given {
            Some(state) => Some(state.join("recovery")),
            None => craft3d_engine::default_recovery_directory(env),
        },
    };
    // Beside the default recovery directory only: a window given a
    // recovery directory of its own (a test's, an agent's, or none) keeps
    // the list for its run only, unless told where, so it neither litters
    // the folder above that directory nor changes the person's list.
    options.recent_file = match recent_file.or_else(|| from_env("CRAFT3D_RECENT_FILE")) {
        Some(none) if none == "none" => None,
        Some(file) => Some(PathBuf::from(file)),
        None if own_recovery => options
            .recovery_dir
            .as_deref()
            .and_then(crate::places::recent_file_beside),
        None => None,
    };
    if options.control.is_some() && options.token_file.is_none() && options.state_dir.is_none() {
        return Err(
            "`--control` needs `--control-token-file FILE` here: there is no state folder for the default one (give `--state-dir DIR`), and the token is never given on the command line"
                .to_string(),
        );
    }
    Ok(Parsed::Run(options))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<String> {
        None
    }

    fn run(list: &[&str]) -> Result<Parsed, String> {
        parse(list.iter().map(|s| (*s).to_string()), &none)
    }

    fn options(list: &[&str]) -> Options {
        match run(list) {
            Ok(Parsed::Run(options)) => options,
            other => panic!("{list:?}: {other:?}"),
        }
    }

    #[test]
    fn the_window_size_is_two_sides_in_range() {
        assert_eq!(
            options(&["--window-size", "1920x1080"]).window_size,
            Some([1920, 1080])
        );
        assert_eq!(
            options(&["--window-size=800X600"]).window_size,
            Some([800, 600])
        );
        for bad in [
            "1920",
            "x",
            "10x10",
            "400x600",
            "1920x99999",
            "-5x100",
            "axb",
            "1e3x800",
        ] {
            let error = run(&["--window-size", bad]).unwrap_err();
            assert!(error.contains("not a window size"), "{bad}: {error}");
        }
        let env = |name: &str| (name == "CRAFT3D_WINDOW_SIZE").then(|| "1000x700".to_string());
        let parsed = parse(Vec::new(), &env).unwrap();
        assert!(matches!(parsed, Parsed::Run(o) if o.window_size == Some([1000, 700])));
    }

    #[test]
    fn nothing_given_is_an_empty_viewer() {
        assert_eq!(options(&[]), Options::default());
    }

    #[test]
    fn a_file_and_the_control_flags() {
        let got = options(&[
            "part.3dcraft",
            "--control",
            "7878",
            "--control-token-file=.private/control.token",
            "--automation-read-root",
            "work",
            "--automation-write-root=out dir",
        ]);
        assert_eq!(
            got,
            Options {
                file: Some("part.3dcraft".to_string()),
                control: Some(7878),
                token_file: Some(PathBuf::from(".private/control.token")),
                read_root: Some("work".to_string()),
                write_root: Some("out dir".to_string()),
                renderer: RendererChoice::Auto,
                recovery_dir: None,
                autosave_interval: None,
                recent_file: None,
                window_size: None,
                state_dir: None,
                agent_port: None,
            }
        );
        // Port 0 asks for a free one.
        assert_eq!(
            options(&["--control=0", "--control-token-file", "t"]).control,
            Some(0)
        );
        // After `--` a dash does not start a flag.
        assert_eq!(
            options(&["--", "--odd.3dcraft"]).file.as_deref(),
            Some("--odd.3dcraft")
        );
    }

    #[test]
    fn the_renderer_is_chosen_by_flag_or_variable_and_is_the_gpu_by_default() {
        assert_eq!(options(&[]).renderer, RendererChoice::Auto);
        assert_eq!(
            options(&["--renderer", "cpu"]).renderer,
            RendererChoice::Cpu
        );
        assert_eq!(options(&["--renderer=CPU"]).renderer, RendererChoice::Cpu);
        assert_eq!(options(&["--renderer=gpu"]).renderer, RendererChoice::Auto);
        assert_eq!(
            options(&["--renderer", "auto"]).renderer,
            RendererChoice::Auto
        );
        let error = run(&["--renderer", "software"]).unwrap_err();
        assert!(
            error.starts_with("--renderer:") && error.contains("`software`"),
            "{error}"
        );
        assert!(run(&["--renderer"]).unwrap_err().contains("needs a value"));
        let env = |name: &str| (name == "CRAFT3D_RENDERER").then(|| "cpu".to_string());
        let parse = |list: &[&str]| match parse(list.iter().map(|s| (*s).to_string()), &env) {
            Ok(Parsed::Run(options)) => options,
            other => panic!("{other:?}"),
        };
        assert_eq!(parse(&[]).renderer, RendererChoice::Cpu);
        assert_eq!(
            parse(&["--renderer", "auto"]).renderer,
            RendererChoice::Auto
        );
        let bad = |name: &str| (name == "CRAFT3D_RENDERER").then(|| "metal".to_string());
        let error = super::parse(Vec::new(), &bad).unwrap_err();
        assert!(error.starts_with("CRAFT3D_RENDERER:"), "{error}");
        assert!(HELP.contains("--renderer") && HELP.contains("CRAFT3D_RENDERER"));
    }

    #[test]
    fn the_recovery_directory_is_a_flag_a_variable_or_the_platforms_place() {
        // Nothing in the environment names a place: no autosave.
        assert_eq!(options(&[]).recovery_dir, None);
        assert_eq!(
            options(&["--recovery-dir", "rec"]).recovery_dir,
            Some(PathBuf::from("rec"))
        );
        assert_eq!(options(&["--recovery-dir=none"]).recovery_dir, None);
        assert_eq!(
            options(&["--autosave-interval", "2.5"]).autosave_interval,
            Some(Duration::from_millis(2500))
        );
        for (args, words) in [
            (&["--recovery-dir"][..], "needs a value"),
            (&["--recovery-dir=a", "--recovery-dir=b"], "given twice"),
            (
                &["--autosave-interval", "soon"],
                "--autosave-interval: `soon`",
            ),
            (&["--autosave-interval", "0"], "above 0"),
        ] {
            let error = run(args).unwrap_err();
            assert!(error.contains(words), "{args:?}: {error}");
        }
        let home = |name: &str| match name {
            "HOME" => Some("/home/ann".to_string()),
            "LOCALAPPDATA" => Some("C:\\Users\\ann\\AppData\\Local".to_string()),
            _ => None,
        };
        let with = |env: &dyn Fn(&str) -> Option<String>, list: &[&str]| match parse(
            list.iter().map(|s| (*s).to_string()),
            env,
        ) {
            Ok(Parsed::Run(options)) => options,
            other => panic!("{other:?}"),
        };
        // The platform's place, when the environment names one.
        let default = with(&home, &[]).recovery_dir.unwrap();
        assert!(default.ends_with("3dcraft/recovery"), "{default:?}");
        assert_eq!(
            Some(default),
            craft3d_engine::default_recovery_directory(&home)
        );
        assert_eq!(with(&home, &["--recovery-dir", "none"]).recovery_dir, None);
        let env = |name: &str| match name {
            "HOME" => Some("/home/ann".to_string()),
            "CRAFT3D_RECOVERY_DIR" => Some("/var/rec".to_string()),
            "CRAFT3D_AUTOSAVE_INTERVAL" => Some("5".to_string()),
            _ => None,
        };
        let from_env = with(&env, &[]);
        assert_eq!(from_env.recovery_dir, Some(PathBuf::from("/var/rec")));
        assert_eq!(from_env.autosave_interval, Some(Duration::from_secs(5)));
        // A flag wins over its variable.
        let flags = with(&env, &["--recovery-dir=mine", "--autosave-interval=9"]);
        assert_eq!(flags.recovery_dir, Some(PathBuf::from("mine")));
        assert_eq!(flags.autosave_interval, Some(Duration::from_secs(9)));
        let off = |name: &str| (name == "CRAFT3D_RECOVERY_DIR").then(|| "none".to_string());
        assert_eq!(with(&off, &[]).recovery_dir, None);
        let bad = |name: &str| (name == "CRAFT3D_AUTOSAVE_INTERVAL").then(|| "-1".to_string());
        let error = super::parse(Vec::new(), &bad).unwrap_err();
        assert!(error.starts_with("CRAFT3D_AUTOSAVE_INTERVAL:"), "{error}");
        for word in [
            "--recovery-dir",
            "--autosave-interval",
            "CRAFT3D_RECOVERY_DIR",
        ] {
            assert!(HELP.contains(word), "{word}");
        }
    }

    #[test]
    fn the_recent_files_are_kept_beside_the_recovery_directory_unless_told() {
        let home = |name: &str| match name {
            "HOME" => Some("/home/ann".to_string()),
            "LOCALAPPDATA" => Some("C:\\Users\\ann\\AppData\\Local".to_string()),
            _ => None,
        };
        let with = |env: &dyn Fn(&str) -> Option<String>, list: &[&str]| match parse(
            list.iter().map(|s| (*s).to_string()),
            env,
        ) {
            Ok(Parsed::Run(options)) => options,
            other => panic!("{other:?}"),
        };
        let default = with(&home, &[]);
        let recovery = default.recovery_dir.unwrap();
        assert_eq!(
            default.recent_file,
            Some(recovery.parent().unwrap().join("recent-files.json"))
        );
        // A recovery directory of its own: the list for the run only.
        assert_eq!(
            with(&home, &["--recovery-dir", "/t/w/recovery"]).recent_file,
            None
        );
        assert_eq!(
            with(&home, &["--recent-file", "/tmp/r.json"]).recent_file,
            Some(PathBuf::from("/tmp/r.json"))
        );
        assert_eq!(with(&home, &["--recent-file=none"]).recent_file, None);
        // A throwaway window keeps none, unless told where.
        assert_eq!(with(&home, &["--recovery-dir", "none"]).recent_file, None);
        assert_eq!(
            with(
                &home,
                &["--recovery-dir", "none", "--recent-file", "r.json"]
            )
            .recent_file,
            Some(PathBuf::from("r.json"))
        );
        let env = |name: &str| match name {
            "HOME" => Some("/home/ann".to_string()),
            "CRAFT3D_RECENT_FILE" => Some("/var/recent.json".to_string()),
            _ => None,
        };
        assert_eq!(
            with(&env, &[]).recent_file,
            Some(PathBuf::from("/var/recent.json"))
        );
        assert!(
            run(&["--recent-file"])
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(HELP.contains("--recent-file") && HELP.contains("CRAFT3D_RECENT_FILE"));
        // No environment: no place.
        assert_eq!(options(&[]).recent_file, None);
    }

    #[test]
    fn help_and_version() {
        assert_eq!(run(&["--help"]), Ok(Parsed::Help));
        assert_eq!(run(&["part.3dcraft", "-h"]), Ok(Parsed::Help));
        assert_eq!(run(&["--version"]), Ok(Parsed::Version));
        assert_eq!(run(&["-V"]), Ok(Parsed::Version));
        assert!(HELP.contains("--control-token-file") && HELP.contains("3DCraft"));
    }

    #[test]
    fn the_state_folder_holds_the_defaults_and_control_needs_no_token_file_then() {
        let home = |name: &str| match name {
            "HOME" => Some("/home/ann".to_string()),
            "LOCALAPPDATA" => Some("C:\\Users\\ann\\AppData\\Local".to_string()),
            _ => None,
        };
        let with = |env: &dyn Fn(&str) -> Option<String>, list: &[&str]| match parse(
            list.iter().map(|s| (*s).to_string()),
            env,
        ) {
            Ok(Parsed::Run(options)) => options,
            other => panic!("{other:?}"),
        };
        // The platform's: the folder of the default recovery directory.
        let default = with(&home, &[]);
        let state = default.state_dir.clone().unwrap();
        assert_eq!(
            Some(state.as_path()),
            default.recovery_dir.as_ref().unwrap().parent()
        );
        // `--control` alone: the default token file in that folder.
        let control = with(&home, &["--control", "0"]);
        assert_eq!(control.control, Some(0));
        assert_eq!(control.token_file, None, "resolved when the window listens");
        // A state folder given: the recovery directory and the recent
        // files go there too, unless given.
        let given = with(&home, &["--state-dir", "/t/state"]);
        assert_eq!(given.state_dir, Some(PathBuf::from("/t/state")));
        let at = PathBuf::from("/t/state");
        assert_eq!(given.recovery_dir, Some(at.join("recovery")));
        assert_eq!(given.recent_file, Some(at.join("recent-files.json")));
        let none = with(
            &home,
            &["--state-dir", "/t/state", "--recovery-dir", "none"],
        );
        assert_eq!((none.recovery_dir, none.recent_file), (None, None));
        let env = |name: &str| (name == "CRAFT3D_STATE_DIR").then(|| "/v/state".to_string());
        assert_eq!(with(&env, &[]).state_dir, Some(PathBuf::from("/v/state")));
        let env = |name: &str| (name == "CRAFT3D_AGENT_PORT").then(|| "7990".to_string());
        assert_eq!(with(&env, &[]).agent_port, Some(7990));
        let bad = |name: &str| (name == "CRAFT3D_AGENT_PORT").then(|| "x".to_string());
        assert!(
            super::parse(Vec::new(), &bad)
                .unwrap_err()
                .contains("CRAFT3D_AGENT_PORT")
        );
        assert!(
            run(&["--state-dir="])
                .unwrap_err()
                .contains("needs a directory")
        );
        assert!(HELP.contains("--state-dir") && HELP.contains("CRAFT3D_STATE_DIR"));
    }

    #[test]
    fn usage_errors_say_what_was_wrong() {
        let cases = [
            (&["--control"][..], "needs a value"),
            (&["--control", "http"], "not a port number"),
            (
                &["--control", "70000", "--control-token-file", "t"],
                "not a port number",
            ),
            (
                &["--control", "-1", "--control-token-file", "t"],
                "not a port number",
            ),
            (&["--control", "7878"], "--control-token-file"),
            (
                &["--control=1", "--control=2", "--control-token-file=t"],
                "given twice",
            ),
            (&["--frobnicate"], "unknown flag `--frobnicate`"),
            (&["a.3dcraft", "b.3dcraft"], "only one file"),
            (&["--help=yes"], "takes no value"),
            (&["--automation-read-root"], "needs a value"),
        ];
        for (args, expected) in cases {
            let error = run(args).unwrap_err();
            assert!(error.contains(expected), "{args:?}: {error}");
        }
    }

    #[test]
    fn the_environment_stands_in_for_missing_flags_only() {
        let env = |name: &str| match name {
            "CRAFT3D_CONTROL_PORT" => Some("9000".to_string()),
            "CRAFT3D_CONTROL_TOKEN_FILE" => Some("env.token".to_string()),
            "CRAFT3D_AUTOMATION_READ_ROOT" => Some("env-read".to_string()),
            "CRAFT3D_AUTOMATION_WRITE_ROOT" => Some(String::new()),
            _ => None,
        };
        let parse = |list: &[&str]| match parse(list.iter().map(|s| (*s).to_string()), &env) {
            Ok(Parsed::Run(options)) => options,
            other => panic!("{other:?}"),
        };
        let from_env = parse(&[]);
        assert_eq!(from_env.control, Some(9000));
        assert_eq!(from_env.token_file, Some(PathBuf::from("env.token")));
        assert_eq!(from_env.read_root.as_deref(), Some("env-read"));
        assert_eq!(from_env.write_root, None, "an empty variable is not a root");
        let flags = parse(&["--control", "1234", "--automation-read-root", "flag-read"]);
        assert_eq!(flags.control, Some(1234));
        assert_eq!(flags.read_root.as_deref(), Some("flag-read"));
        assert_eq!(flags.token_file, Some(PathBuf::from("env.token")));

        let bad = |name: &str| (name == "CRAFT3D_CONTROL_PORT").then(|| "soon".to_string());
        let error = super::parse(Vec::new(), &bad).unwrap_err();
        assert!(error.contains("CRAFT3D_CONTROL_PORT"), "{error}");
    }
}
