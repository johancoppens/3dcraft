//! The subcommands for a trusted local user: `commands`, `run`, `info`,
//! `render` and `convert`. They run engine commands in a session with
//! unrestricted file access, because the paths come from the person at the
//! command line, not from a remote peer.

use crate::args::{self, Arg, Args};
use crate::{CliError, Parsed, print};
use craft3d_engine::{Session, Workspace};
use serde_json::{Map, Value, json};
use std::path::Path;

/// Largest script file `run --script` reads: a script is a list of
/// commands, not data.
const MAX_SCRIPT_BYTES: u64 = 16 << 20;

fn unknown_flag(name: &str) -> CliError {
    CliError::Usage(format!("unknown flag `{name}`"))
}

fn unexpected(argument: &str) -> CliError {
    CliError::Usage(format!("unexpected argument `{argument}`"))
}

fn given_twice(flag: &str) -> CliError {
    CliError::Usage(format!("`{flag}` was given twice"))
}

/// Stores a flag's value, refusing a second one.
fn set_once<T>(slot: &mut Option<T>, flag: &str, value: T) -> Result<(), CliError> {
    if slot.replace(value).is_some() {
        return Err(given_twice(flag));
    }
    Ok(())
}

/// A flag value that must be a JSON object (command params).
fn json_object(flag: &str, text: &str) -> Result<Value, CliError> {
    match serde_json::from_str::<Value>(text) {
        Ok(value @ Value::Object(_)) => Ok(value),
        Ok(_) => Err(CliError::Usage(format!(
            "`{flag}` must be a JSON object such as '{{\"units\": \"mm\"}}', got `{text}`"
        ))),
        Err(e) => Err(CliError::Usage(format!(
            "`{flag}` is not valid JSON ({e}): `{text}`"
        ))),
    }
}

/// Runs a command; a failure ends the program with exit code 1.
fn execute(session: &mut Session, command: &str, params: &Value) -> Result<Value, CliError> {
    session
        .execute(command, params)
        .map_err(|e| CliError::Failed(e.message().to_string()))
}

fn open(session: &mut Session, file: &str) -> Result<(), CliError> {
    execute(session, "file.open", &json!({ "path": file })).map(|_| ())
}

fn print_json(value: &Value) -> Result<(), CliError> {
    // Serialising a `Value` does not fail; if it ever does, say so.
    let line = serde_json::to_string(value)
        .map_err(|e| CliError::Failed(format!("cannot write the result as JSON: {e}")))?;
    print(&line)
}

// --------------------------------------------------------------- commands

pub(crate) fn commands(mut args: Args) -> Result<Parsed<()>, CliError> {
    let mut filter: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg {
            Arg::Flag { name, inline } => match name.as_str() {
                "--help" | "-h" => {
                    args::no_value(&name, inline.as_ref())?;
                    return Ok(Parsed::Help);
                }
                "--filter" => {
                    let value = args.value(&name, inline)?;
                    set_once(&mut filter, &name, value)?;
                }
                _ => return Err(unknown_flag(&name)),
            },
            Arg::Positional(extra) => return Err(unexpected(&extra)),
        }
    }
    let needle = filter.as_deref().map(str::to_lowercase);
    let mut blocks = Vec::new();
    for spec in Session::commands() {
        let matches = needle.as_deref().is_none_or(|needle| {
            spec.id.to_lowercase().contains(needle) || spec.label.to_lowercase().contains(needle)
        });
        if matches {
            blocks.push(format!(
                "{}\n  params:  {}\n  summary: {}",
                spec.id, spec.params, spec.summary
            ));
        }
    }
    if blocks.is_empty() {
        // Not an error: the question "is there a command like X" was answered.
        return print(&format!(
            "No command matches `{}`.",
            filter.unwrap_or_default()
        ))
        .map(Parsed::Run);
    }
    print(&blocks.join("\n\n")).map(Parsed::Run)
}

// -------------------------------------------------------------------- run

/// One command to run, from `--cmd` or a script.
#[derive(Debug, PartialEq)]
struct Step {
    command: String,
    /// A JSON object, or null for none.
    params: Value,
}

#[derive(Debug, Default, PartialEq)]
struct RunOptions {
    file: Option<String>,
    new: Option<Value>,
    script: Option<String>,
    steps: Vec<Step>,
    out: Option<String>,
}

fn parse_run(mut args: Args) -> Result<Parsed<RunOptions>, CliError> {
    let mut options = RunOptions::default();
    // Whether the last `--cmd` already has its `--params`.
    let mut params_given = true;
    while let Some(arg) = args.next() {
        match arg {
            Arg::Flag { name, inline } => match name.as_str() {
                "--help" | "-h" => {
                    args::no_value(&name, inline.as_ref())?;
                    return Ok(Parsed::Help);
                }
                "--new" => {
                    let value = json_object(&name, &args.value(&name, inline)?)?;
                    set_once(&mut options.new, &name, value)?;
                }
                "--script" => {
                    let value = args.value(&name, inline)?;
                    set_once(&mut options.script, &name, value)?;
                }
                "--out" => {
                    let value = args.value(&name, inline)?;
                    set_once(&mut options.out, &name, value)?;
                }
                "--cmd" => {
                    options.steps.push(Step {
                        command: args.value(&name, inline)?,
                        params: Value::Null,
                    });
                    params_given = false;
                }
                "--params" => {
                    let value = json_object(&name, &args.value(&name, inline)?)?;
                    match options.steps.last_mut() {
                        Some(step) if !params_given => {
                            step.params = value;
                            params_given = true;
                        }
                        Some(step) => {
                            return Err(CliError::Usage(format!(
                                "`--params` was given twice for `--cmd {}`",
                                step.command
                            )));
                        }
                        None => {
                            return Err(CliError::Usage(
                                "`--params` must follow the `--cmd` it belongs to".to_string(),
                            ));
                        }
                    }
                }
                _ => return Err(unknown_flag(&name)),
            },
            Arg::Positional(file) => {
                if let Some(first) = &options.file {
                    return Err(CliError::Usage(format!(
                        "unexpected argument `{file}`: the document is already `{first}`"
                    )));
                }
                options.file = Some(file);
            }
        }
    }
    if options.file.is_some() && options.new.is_some() {
        return Err(CliError::Usage(
            "give FILE to open a document or `--new` to create one, not both".to_string(),
        ));
    }
    Ok(Parsed::Run(options))
}

/// Reads a script: a JSON array whose items are `["id", {params}]` (the
/// params optional) or `{"command": "id", "params": {...}}`.
fn parse_script(name: &str, text: &str) -> Result<Vec<Step>, CliError> {
    let fail = |message: String| CliError::Failed(format!("script `{name}`: {message}"));
    let script: Value =
        serde_json::from_str(text).map_err(|e| fail(format!("not valid JSON: {e}")))?;
    let Value::Array(items) = script else {
        return Err(fail(
            "must be a JSON array of steps, each [\"id\", {params}] or {\"command\": \"id\", \"params\": {...}}"
                .to_string(),
        ));
    };
    let mut steps = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        let shape = || {
            fail(format!(
                "step {index} must be [\"id\", {{params}}] or {{\"command\": \"id\", \"params\": {{...}}}}"
            ))
        };
        let (command, params) = match item {
            Value::Array(pair) => {
                let mut parts = pair.into_iter();
                let (Some(Value::String(command)), params, None) =
                    (parts.next(), parts.next(), parts.next())
                else {
                    return Err(shape());
                };
                (command, params.unwrap_or(Value::Null))
            }
            Value::Object(mut map) => {
                let Some(Value::String(command)) = map.remove("command") else {
                    return Err(shape());
                };
                let params = map.remove("params").unwrap_or(Value::Null);
                if let Some(unknown) = map.keys().next() {
                    return Err(fail(format!(
                        "step {index} has an unknown key `{unknown}`; a step has `command` and `params`"
                    )));
                }
                (command, params)
            }
            _ => return Err(shape()),
        };
        if !(params.is_object() || params.is_null()) {
            return Err(fail(format!(
                "step {index} (`{command}`): params must be a JSON object"
            )));
        }
        steps.push(Step { command, params });
    }
    Ok(steps)
}

fn read_script(path: &str) -> Result<Vec<Step>, CliError> {
    let fail = |e: std::io::Error| CliError::Failed(format!("cannot read script `{path}`: {e}"));
    let size = std::fs::metadata(path).map_err(fail)?.len();
    if size > MAX_SCRIPT_BYTES {
        return Err(CliError::Failed(format!(
            "script `{path}` is {size} bytes; the limit is {MAX_SCRIPT_BYTES}"
        )));
    }
    let text = std::fs::read_to_string(path).map_err(fail)?;
    parse_script(path, &text)
}

pub(crate) fn run(args: Args) -> Result<Parsed<()>, CliError> {
    let options = match parse_run(args)? {
        Parsed::Run(options) => options,
        Parsed::Help => return Ok(Parsed::Help),
    };
    // The script is read before anything runs: a broken script should not
    // leave half a session behind it.
    let mut steps = match &options.script {
        Some(path) => read_script(path)?,
        None => Vec::new(),
    };
    steps.extend(options.steps);

    let mut session = Session::new(Workspace::unrestricted());
    match &options.file {
        Some(file) => open(&mut session, file)?,
        None => {
            let params = options.new.unwrap_or_else(|| Value::Object(Map::new()));
            execute(&mut session, "file.new", &params)?;
        }
    }
    for step in &steps {
        let result = execute(&mut session, &step.command, &step.params)?;
        print_json(&result)?;
    }
    if let Some(out) = &options.out {
        execute(&mut session, "file.save", &json!({ "path": out }))?;
    }
    Ok(Parsed::Run(()))
}

// ------------------------------------------------------------------- info

/// Parses a subcommand that takes positionals and value flags only.
/// Returns the positionals; `flag` is called for each `--flag VALUE`.
fn parse_simple(
    mut args: Args,
    flags: &[&str],
    mut flag: impl FnMut(&str, String) -> Result<(), CliError>,
) -> Result<Parsed<Vec<String>>, CliError> {
    let mut positionals = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        match arg {
            Arg::Flag { name, inline } => {
                if matches!(name.as_str(), "--help" | "-h") {
                    args::no_value(&name, inline.as_ref())?;
                    return Ok(Parsed::Help);
                }
                if !flags.contains(&name.as_str()) {
                    return Err(unknown_flag(&name));
                }
                if seen.contains(&name) {
                    return Err(given_twice(&name));
                }
                let value = args.value(&name, inline)?;
                flag(&name, value)?;
                seen.push(name);
            }
            Arg::Positional(value) => positionals.push(value),
        }
    }
    Ok(Parsed::Run(positionals))
}

pub(crate) fn info(args: Args) -> Result<Parsed<()>, CliError> {
    let positionals = match parse_simple(args, &[], |_, _| Ok(()))? {
        Parsed::Run(positionals) => positionals,
        Parsed::Help => return Ok(Parsed::Help),
    };
    let [file] = positionals.as_slice() else {
        return Err(match positionals.get(1) {
            Some(extra) => unexpected(extra),
            None => CliError::Usage("missing FILE".to_string()),
        });
    };
    let mut session = Session::new(Workspace::unrestricted());
    open(&mut session, file)?;
    let inspect = execute(&mut session, "document.inspect", &Value::Null)?;
    let pretty = serde_json::to_string_pretty(&inspect)
        .map_err(|e| CliError::Failed(format!("cannot write the result as JSON: {e}")))?;
    print(&pretty).map(Parsed::Run)
}

// ----------------------------------------------------------------- render

/// `WxH`, each a whole number. The engine checks the range.
fn parse_size(text: &str) -> Result<(u64, u64), CliError> {
    text.split_once(['x', 'X'])
        .and_then(|(width, height)| Some((width.parse().ok()?, height.parse().ok()?)))
        .ok_or_else(|| {
            CliError::Usage(format!(
                "`--size` must be WIDTHxHEIGHT in pixels, such as 1200x800, got `{text}`"
            ))
        })
}

pub(crate) fn render(args: Args) -> Result<Parsed<()>, CliError> {
    let mut params = Map::new();
    let parsed = parse_simple(
        args,
        &["--out", "--size", "--view", "--mode"],
        |name, value| {
            match name {
                "--out" => {
                    params.insert("path".into(), value.into());
                }
                "--size" => {
                    let (width, height) = parse_size(&value)?;
                    params.insert("width".into(), width.into());
                    params.insert("height".into(), height.into());
                }
                "--view" => {
                    params.insert("view".into(), value.into());
                }
                _ => {
                    params.insert("mode".into(), value.into());
                }
            }
            Ok(())
        },
    )?;
    let positionals = match parsed {
        Parsed::Run(positionals) => positionals,
        Parsed::Help => return Ok(Parsed::Help),
    };
    let [file] = positionals.as_slice() else {
        return Err(match positionals.get(1) {
            Some(extra) => unexpected(extra),
            None => CliError::Usage("missing FILE".to_string()),
        });
    };
    if !params.contains_key("path") {
        // Without a path the engine would return the image as base64,
        // which is no use on a terminal.
        return Err(CliError::Usage("missing `--out PNG`".to_string()));
    }
    let mut session = Session::new(Workspace::unrestricted());
    open(&mut session, file)?;
    let result = execute(&mut session, "view.render", &Value::Object(params))?;
    print_json(&result).map(Parsed::Run)
}

// ---------------------------------------------------------------- convert

pub(crate) fn convert(args: Args) -> Result<Parsed<()>, CliError> {
    let mut tolerance: Option<f64> = None;
    let parsed = parse_simple(args, &["--tolerance"], |_, value| {
        match value.parse::<f64>() {
            Ok(t) if t.is_finite() && t > 0.0 => {
                tolerance = Some(t);
                Ok(())
            }
            _ => Err(CliError::Usage(format!(
                "`--tolerance` must be a positive number, got `{value}`"
            ))),
        }
    })?;
    let positionals = match parsed {
        Parsed::Run(positionals) => positionals,
        Parsed::Help => return Ok(Parsed::Help),
    };
    let [input, output] = positionals.as_slice() else {
        return Err(match positionals.get(2) {
            Some(extra) => unexpected(extra),
            None => CliError::Usage("give the input file and the output file: IN OUT".to_string()),
        });
    };
    let native = Path::new(output)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("3dcraft"));
    let exact = Path::new(output)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("step") || e.eq_ignore_ascii_case("stp"));
    if (native || exact) && tolerance.is_some() {
        return Err(CliError::Usage(
            "`--tolerance` applies to mesh output (.stl, .3mf, .obj), not to .3dcraft or STEP"
                .to_string(),
        ));
    }
    let mut session = Session::new(Workspace::unrestricted());
    open(&mut session, input)?;
    let result = if native {
        execute(&mut session, "file.save", &json!({ "path": output }))?
    } else {
        // The engine picks the format from the extension, and says
        // which extensions it knows when this one is not among them.
        let mut params = Map::new();
        params.insert("path".into(), output.as_str().into());
        if let Some(tolerance) = tolerance {
            params.insert("tolerance".into(), tolerance.into());
        }
        execute(&mut session, "file.export", &Value::Object(params))?
    };
    print_json(&result).map(Parsed::Run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn args(list: &[&str]) -> Args {
        Args::from_os(list.iter().map(OsString::from)).unwrap()
    }

    fn run_options(list: &[&str]) -> Result<RunOptions, String> {
        match parse_run(args(list)) {
            Ok(Parsed::Run(options)) => Ok(options),
            Ok(Parsed::Help) => Err("help".to_string()),
            Err(error) => Err(error.to_string()),
        }
    }

    fn step(command: &str, params: Value) -> Step {
        Step {
            command: command.to_string(),
            params,
        }
    }

    #[test]
    fn run_pairs_each_params_with_the_cmd_before_it() {
        let options = run_options(&[
            "--new",
            "{\"units\": \"cm\"}",
            "--cmd",
            "feature.block",
            "--params",
            "{\"size\": [1, 2, 3]}",
            "--cmd",
            "document.measure",
            "--cmd=edit.undo",
            "--params={}",
            "--out",
            "x.3dcraft",
        ])
        .unwrap();
        assert_eq!(options.file, None);
        assert_eq!(options.new, Some(json!({"units": "cm"})));
        assert_eq!(
            options.steps,
            [
                step("feature.block", json!({"size": [1, 2, 3]})),
                step("document.measure", Value::Null),
                step("edit.undo", json!({})),
            ]
        );
        assert_eq!(options.out.as_deref(), Some("x.3dcraft"));
        assert_eq!(
            run_options(&["a.3dcraft"]).unwrap().file.as_deref(),
            Some("a.3dcraft")
        );
    }

    #[test]
    fn run_refuses_what_it_cannot_make_sense_of() {
        for (list, expected) in [
            (
                &["--params", "{}"][..],
                "`--params` must follow the `--cmd`",
            ),
            (
                &["--cmd", "a", "--params", "{}", "--params", "{}"],
                "`--params` was given twice for `--cmd a`",
            ),
            (
                &["--cmd", "a", "--params", "[1]"],
                "`--params` must be a JSON object",
            ),
            (
                &["--cmd", "a", "--params", "{oops"],
                "`--params` is not valid JSON",
            ),
            (&["--cmd"], "`--cmd` needs a value"),
            (&["--new", "5"], "`--new` must be a JSON object"),
            (&["--new", "{}", "--new", "{}"], "`--new` was given twice"),
            (&["a.3dcraft", "--new", "{}"], "not both"),
            (
                &["a.3dcraft", "b.3dcraft"],
                "unexpected argument `b.3dcraft`",
            ),
            (&["--out", "a", "--out", "b"], "`--out` was given twice"),
            (
                &["--script", "a", "--script", "b"],
                "`--script` was given twice",
            ),
            (&["--step-delay", "800ms"], "unknown flag `--step-delay`"),
            (&["-x"], "unknown flag `-x`"),
        ] {
            let error = run_options(list).unwrap_err();
            assert!(error.contains(expected), "{list:?}: {error}");
        }
        assert_eq!(run_options(&["--cmd", "a", "--help"]).unwrap_err(), "help");
    }

    #[test]
    fn a_script_is_pairs_or_objects() {
        let steps = parse_script(
            "s.json",
            r#"[["file.new"], ["feature.block", {"size": [1, 2, 3]}], {"command": "edit.undo"},
                {"command": "variable.set", "params": {"name": "w", "expr": 1}}, ["edit.redo", null]]"#,
        )
        .unwrap();
        assert_eq!(
            steps,
            [
                step("file.new", Value::Null),
                step("feature.block", json!({"size": [1, 2, 3]})),
                step("edit.undo", Value::Null),
                step("variable.set", json!({"name": "w", "expr": 1})),
                step("edit.redo", Value::Null),
            ]
        );
        assert_eq!(parse_script("s.json", "[]").unwrap(), []);
    }

    #[test]
    fn a_malformed_script_is_refused_with_the_step_number() {
        for (text, expected) in [
            ("{", "not valid JSON"),
            ("{}", "must be a JSON array of steps"),
            ("[[\"a\"], 5]", "step 1 must be"),
            ("[[]]", "step 0 must be"),
            ("[[1, {}]]", "step 0 must be"),
            ("[[\"a\", {}, {}]]", "step 0 must be"),
            (
                "[[\"a\", [1]]]",
                "step 0 (`a`): params must be a JSON object",
            ),
            ("[{\"params\": {}}]", "step 0 must be"),
            (
                "[{\"command\": \"a\", \"params\": 3}]",
                "step 0 (`a`): params must be a JSON object",
            ),
            (
                "[{\"command\": \"a\", \"parms\": {}}]",
                "step 0 has an unknown key `parms`",
            ),
        ] {
            let error = parse_script("s.json", text).unwrap_err().to_string();
            assert!(error.contains(expected), "{text}: {error}");
            assert!(error.starts_with("script `s.json`"), "{error}");
        }
    }

    #[test]
    fn sizes_are_width_x_height() {
        assert_eq!(parse_size("1200x800").unwrap(), (1200, 800));
        assert_eq!(parse_size("16X16").unwrap(), (16, 16));
        for bad in [
            "",
            "800",
            "800x",
            "x600",
            "800x600x3",
            "-1x5",
            "8.5x6",
            "a×b",
        ] {
            assert!(parse_size(bad).is_err(), "{bad}");
        }
    }
}
