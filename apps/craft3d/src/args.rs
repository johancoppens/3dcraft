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

Usage: craft3d [FILE] [--control PORT --control-token-file FILE]
               [--automation-read-root DIR] [--automation-write-root DIR]
               [--renderer auto|cpu]
               [--recovery-dir DIR|none] [--autosave-interval SECONDS]
       craft3d --version | --help

Opens FILE (a .3dcraft file) when one is given, and shows the active
document: bodies shaded with their edges, sketches as lines. The window
does not model; the document changes through commands, sent over the
control channel. It does open and save files: the File menu, with the
system's file dialog, wherever you may read and write.

Options:
  --control PORT              Listen on 127.0.0.1:PORT for JSON-lines requests
                              (docs/control-protocol.md). 0 picks a free port;
                              the address is printed on standard error.
  --control-token-file FILE   The file with the control token. Created with a
                              fresh token (mode 0600) when it does not exist.
                              Required with --control.
  --automation-read-root DIR  Requests may read files beneath DIR only
  --automation-write-root DIR Requests may write files beneath DIR only
                              Without a root, that direction is denied.
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

Environment: CRAFT3D_CONTROL_PORT, CRAFT3D_CONTROL_TOKEN_FILE,
CRAFT3D_AUTOMATION_READ_ROOT, CRAFT3D_AUTOMATION_WRITE_ROOT,
CRAFT3D_RENDERER, CRAFT3D_RECOVERY_DIR and CRAFT3D_AUTOSAVE_INTERVAL stand
in for the flags of the same name.

Mouse: drag to orbit, right-drag or shift-drag to pan, wheel to zoom, F to
frame everything.
Keys: Ctrl+N new, Ctrl+O open, Ctrl+S save, Ctrl+Shift+S save as.";

/// What to start with.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Options {
    /// A `.3dcraft` file to open.
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
}

/// What the command line asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Options),
    Help,
    Version,
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
            | "--autosave-interval" => {
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
    // The flag, else the variable, else the platform's place; `none`
    // turns autosave off, and so does a platform that names no place.
    options.recovery_dir = match recovery_dir.or_else(|| from_env("CRAFT3D_RECOVERY_DIR")) {
        Some(none) if none == "none" => None,
        Some(dir) => Some(PathBuf::from(dir)),
        None => craft3d_engine::default_recovery_directory(env),
    };
    if options.control.is_some() && options.token_file.is_none() {
        return Err(
            "`--control` needs `--control-token-file FILE`: the token is never given on the command line"
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
    fn help_and_version() {
        assert_eq!(run(&["--help"]), Ok(Parsed::Help));
        assert_eq!(run(&["part.3dcraft", "-h"]), Ok(Parsed::Help));
        assert_eq!(run(&["--version"]), Ok(Parsed::Version));
        assert_eq!(run(&["-V"]), Ok(Parsed::Version));
        assert!(HELP.contains("--control-token-file") && HELP.contains("3DCraft"));
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
