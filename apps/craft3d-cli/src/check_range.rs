//! `craft3d-cli check-range`: proves a Jig over its parameter range
//! (the command `document.sample`) for a Jig repository's CI. The work is
//! the engine's; this file reads the flags and writes the report.
//!
//! Exit codes: 0 no point failed (a point with an assertion not proven is
//! counted and listed apart, and fails with `--strict`); 1 a point failed; 2 a usage error
//! (the caps included); 3 the file could not be opened or checked, or the
//! check's process was stopped by a signal (the out-of-memory killer); 4
//! the run stopped at `--time-limit-ms` (or because the process that
//! started its own is gone) before every point was checked, with none
//! failing so far.

use crate::args::{self, Arg, Args};
use crate::{CliError, Parsed, print};
use craft3d_engine::{Session, Workspace};
use serde_json::{Map, Value, json};
use std::io::Write;

/// The options, as `document.sample` params plus how to print.
struct Options {
    file: String,
    params: Map<String, Value>,
    json: bool,
}

/// The caps of `document.sample`, checked here so that going past one is
/// a usage error.
const MAX_SAMPLES: u64 = 10_000;
const MAX_CORNERS: u64 = 4096;

/// Exit codes beyond the usage error.
const EXIT_ERROR: u8 = 3;
const EXIT_INCOMPLETE: u8 = 4;

fn whole(flag: &str, text: &str, max: u64) -> Result<Value, CliError> {
    match text.parse::<u64>() {
        Ok(n) if n <= max => Ok(Value::from(n)),
        Ok(n) => Err(CliError::Usage(format!(
            "`{flag}` is at most {max}, got {n}"
        ))),
        Err(_) => Err(CliError::Usage(format!(
            "`{flag}` must be a whole number, got `{text}`"
        ))),
    }
}

fn positive_ms(flag: &str, text: &str) -> Result<Value, CliError> {
    text.parse::<f64>()
        .ok()
        .filter(|ms| ms.is_finite() && *ms > 0.0)
        .map(Value::from)
        .ok_or_else(|| {
            CliError::Usage(format!(
                "`{flag}` must be a positive number of milliseconds, got `{text}`"
            ))
        })
}

fn parse(mut args: Args) -> Result<Parsed<Options>, CliError> {
    let mut file: Option<String> = None;
    let mut params = Map::new();
    let mut json = false;
    while let Some(arg) = args.next() {
        match arg {
            Arg::Flag { name, inline } => {
                let key = match name.as_str() {
                    "--help" | "-h" => {
                        args::no_value(&name, inline.as_ref())?;
                        return Ok(Parsed::Help);
                    }
                    "--deep" | "--json" | "--no-corners" | "--strict" => {
                        args::no_value(&name, inline.as_ref())?;
                        match name.as_str() {
                            "--deep" => params.insert("deep".into(), true.into()),
                            "--no-corners" => params.insert("corners".into(), false.into()),
                            "--strict" => params.insert("strict".into(), true.into()),
                            _ => {
                                json = true;
                                None
                            }
                        };
                        continue;
                    }
                    "--samples" => "samples",
                    "--seed" => "seed",
                    "--max-corners" => "maxCorners",
                    "--budget-ms" => "budgetMs",
                    "--time-limit-ms" => "timeLimitMs",
                    _ => return Err(CliError::Usage(format!("unknown flag `{name}`"))),
                };
                let value = args.value(&name, inline)?;
                let value = match key {
                    "budgetMs" | "timeLimitMs" => positive_ms(&name, &value)?,
                    "samples" => whole(&name, &value, MAX_SAMPLES)?,
                    "maxCorners" => whole(&name, &value, MAX_CORNERS)?,
                    _ => whole(&name, &value, u64::MAX)?,
                };
                if params.insert(key.into(), value).is_some() {
                    return Err(CliError::Usage(format!("`{name}` was given twice")));
                }
            }
            Arg::Positional(value) => {
                if file.replace(value.clone()).is_some() {
                    return Err(CliError::Usage(format!("unexpected argument `{value}`")));
                }
            }
        }
    }
    let file = file.ok_or_else(|| CliError::Usage("missing FILE".to_string()))?;
    // At the command line a run has no time limit unless one is asked for.
    params.entry("timeLimitMs").or_insert(Value::Null);
    Ok(Parsed::Run(Options { file, params, json }))
}

/// `value[key]`, null when it is not there.
fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

fn ms(value: &Value) -> String {
    value
        .as_f64()
        .map_or_else(|| "-".to_string(), |ms| format!("{ms:.1} ms"))
}

/// A point's values as `name=value, ...`.
fn values_text(point: &Value) -> String {
    let values: Vec<String> = field(point, "values")
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, value)| format!("{name}={}", value.as_str().unwrap_or("?")))
        .collect();
    values.join(", ")
}

/// One problem of a point as `what: problem`.
fn problem_text(problem: &Value) -> String {
    let what = ["name", "parameter", "assertion", "variable"]
        .iter()
        .find_map(|key| field(problem, key).as_str())
        .map(|w| format!("{w}: "))
        .unwrap_or_default();
    format!(
        "{what}{}",
        field(problem, "problem")
            .as_str()
            .map_or_else(|| field(problem, "problem").to_string(), str::to_string)
    )
}

/// The line a point gets as soon as it is checked (`document.sample`'s
/// point sink), so that a run cut short has said what it found: passed,
/// not proven with its assertions, or FAIL with the first problem; with
/// the values of a point that did not pass, to reproduce it.
fn point_line(point: &Value) -> String {
    let head = format!(
        "point {} ({}, {})",
        field(point, "index"),
        field(point, "kind").as_str().unwrap_or("?"),
        ms(field(point, "ms"))
    );
    let problems = field(point, "problems").as_array().map_or(0, Vec::len);
    let unproven: Vec<&str> = field(point, "unproven")
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| field(a, "assertion").as_str())
        .collect();
    let line = if field(point, "ok") != true {
        let first = field(point, "problems")
            .get(0)
            .map(problem_text)
            .unwrap_or_default();
        format!(
            "{head}: FAIL, {problems} problem{} ({first}): {}",
            if problems == 1 { "" } else { "s" },
            values_text(point)
        )
    } else if !unproven.is_empty() {
        format!(
            "{head}: not proven: {}: {}",
            unproven.join(", "),
            values_text(point)
        )
    } else {
        format!("{head}: passed")
    };
    line + &clamped_text(point)
}

/// The moves of the parameters a point clamped into their ranges, to go
/// after its line: ` (clamped n 5 -> 3)`; empty when there are none.
fn clamped_text(point: &Value) -> String {
    let moves: Vec<String> = field(point, "clamped")
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            format!(
                "{} {} -> {}",
                field(m, "parameter").as_str().unwrap_or("?"),
                field(m, "from").as_str().unwrap_or("?"),
                field(m, "to").as_str().unwrap_or("?")
            )
        })
        .collect();
    if moves.is_empty() {
        String::new()
    } else {
        format!(" (clamped {})", moves.join(", "))
    }
}

/// The report for a person: one line of totals, the times, then one
/// block per failing point and per point with an assertion not proven.
fn report(file: &str, result: &Value) -> String {
    let counts = field(result, "counts");
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let tried = n(field(counts, "corner"));
    let in_box = field(counts, "cornersInBox").as_u64();
    let corners = match in_box {
        Some(all) if field(counts, "allCorners") == true => format!("all {all} corners"),
        Some(all) => format!("{tried} of {all} corners"),
        None => format!("{tried} corners"),
    };
    let mut lines = vec![format!(
        "{file}: {} parameters, {} points ({} default, {corners}, {} choices, {} random): {} passed, {} with unproven assertions, {} failed",
        field(result, "parameters").as_array().map_or(0, Vec::len),
        n(field(counts, "points")),
        n(field(counts, "default")),
        n(field(counts, "choice")),
        n(field(counts, "random")),
        n(field(result, "passed")),
        n(field(result, "unproven")),
        n(field(result, "failed")),
    )];
    if let Some(all) = in_box
        && tried < all
    {
        lines.push(format!(
            "corners: {tried} of the {all} tried; --samples does not change them: --max-corners N tries more (all 2^n while that is at most N, at most 4096), --no-corners none"
        ));
    }
    if field(result, "complete") == false {
        lines.push(format!(
            "INCOMPLETE: stopped by {} before every point was checked",
            field(result, "stoppedBy").as_str().unwrap_or("?")
        ));
    }
    let timing = field(result, "timing");
    if !timing.is_null() {
        lines.push(format!(
            "regeneration: median {}, max {} (point {}), total {}",
            ms(field(timing, "medianMs")),
            ms(field(timing, "maxMs")),
            field(timing, "slowest"),
            ms(field(timing, "totalMs"))
        ));
    }
    if let Some(unbounded) = field(result, "unbounded")
        .as_array()
        .filter(|u| !u.is_empty())
    {
        let names: Vec<&str> = unbounded.iter().filter_map(Value::as_str).collect();
        lines.push(format!(
            "without a range (only the default tried): {}",
            names.join(", ")
        ));
    }
    for failure in field(result, "failures").as_array().into_iter().flatten() {
        lines.push(format!(
            "FAIL point {} ({}): {}",
            field(failure, "index"),
            field(failure, "kind").as_str().unwrap_or("?"),
            values_text(failure)
        ));
        for problem in field(failure, "problems").as_array().into_iter().flatten() {
            lines.push(format!("  {}", problem_text(problem)));
        }
    }
    if field(result, "truncated") == true {
        lines.push(format!(
            "... and {} more failing points (--json lists the first 100)",
            n(field(result, "failed"))
                .saturating_sub(field(result, "failures").as_array().map_or(0, Vec::len) as u64)
        ));
    }
    for point in field(result, "unprovenPoints")
        .as_array()
        .into_iter()
        .flatten()
    {
        lines.push(format!(
            "NOT PROVEN point {} ({}): {}",
            field(point, "index"),
            field(point, "kind").as_str().unwrap_or("?"),
            values_text(point)
        ));
        for assertion in field(point, "assertions").as_array().into_iter().flatten() {
            lines.push(format!(
                "  {}: {}",
                field(assertion, "assertion").as_str().unwrap_or("?"),
                field(assertion, "message").as_str().unwrap_or("")
            ));
        }
    }
    if field(result, "unprovenTruncated") == true {
        lines.push(format!(
            "... and {} more points with unproven assertions (--json lists the first 100)",
            n(field(result, "unproven")).saturating_sub(
                field(result, "unprovenPoints")
                    .as_array()
                    .map_or(0, Vec::len) as u64
            )
        ));
    }
    if n(field(result, "unproven")) > 0 {
        lines.push(
            "an assertion that is not proven is neither passed nor failed: --strict fails its point"
                .to_string(),
        );
    }
    lines.join("\n")
}

/// How many arenas the C library's allocator keeps for a check in a
/// process of its own ([`in_few_arenas`]).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const ARENAS: &str = "2";

/// The variable that tells the process of [`in_few_arenas`] which process
/// started it (its id): it is not started again, and it stops when that
/// process is gone (killed by a timeout or by hand), not to run on
/// unwatched.
const PARENT: &str = "CRAFT3D_CHECK_PARENT";

/// Runs this same check again in a process of its own whose allocator
/// keeps [`ARENAS`] arenas (unless the environment names a number of its
/// own), and gives its outcome; `None` where that cannot be done (the
/// program cannot be found or started), or need not be: this is that
/// process ([`PARENT`] is set).
///
/// Why: the C library's allocator (glibc) gives threads arenas of their
/// own, as many as eight per core, and keeps what each arena held at its
/// peak. A check runs many points one after the other, each of them on as
/// many threads as the machine has (meshing, measuring, booleans), and the
/// memory grew with every point although each point lets go of everything
/// it made: a spur gear with a meshing copy over 40 points, on a machine
/// of 32 threads, from 150 MB to 1.1 GB; with two arenas it levels off
/// below 500 MB, the peak of a point, in the same time (2026-10-26,
/// docs/design/verification.md). The number of arenas is read once, when
/// the program starts, from its environment: only a process started with
/// it set has them. Nothing else changes: the same arguments, standard
/// input and output, and exit code.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn in_few_arenas() -> Option<Result<Parsed<()>, CliError>> {
    const SETTING: &str = "MALLOC_ARENA_MAX";
    if std::env::var_os(PARENT).is_some() {
        return None;
    }
    let program = std::env::current_exe().ok()?;
    let mut command = std::process::Command::new(program);
    command
        .args(std::env::args_os().skip(1))
        .env(PARENT, std::process::id().to_string());
    if std::env::var_os(SETTING).is_none() {
        command.env(SETTING, ARENAS);
    }
    let status = command.status().ok()?;
    Some(outcome_of(status))
}

/// The outcome of the check's own process, from how it ended: its exit
/// code, which it explained itself; or a signal, which is an error of the
/// check (exit code 3), not a failing point: the kernel's out-of-memory
/// killer sends 9.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn outcome_of(status: std::process::ExitStatus) -> Result<Parsed<()>, CliError> {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(0), _) => Ok(Parsed::Run(())),
        (Some(code), _) => Err(CliError::Quiet(u8::try_from(code).unwrap_or(1))),
        (None, signal) => Err(CliError::Code(
            EXIT_ERROR,
            format!(
                "the check was stopped by signal {} before it finished (9: killed, as the out-of-memory killer does)",
                signal.map_or_else(|| "?".to_string(), |s| s.to_string())
            ),
        )),
    }
}

/// Whether the process that started this one (the one [`PARENT`] names)
/// is gone: this one is then an orphan nobody waits for.
fn orphaned(parent: Option<u32>) -> bool {
    #[cfg(unix)]
    {
        parent.is_some_and(|pid| std::os::unix::process::parent_id() != pid)
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
        false
    }
}

pub(crate) fn check_range(args: Args) -> Result<Parsed<()>, CliError> {
    let options = match parse(args)? {
        Parsed::Run(options) => options,
        Parsed::Help => return Ok(Parsed::Help),
    };
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    if let Some(outcome) = in_few_arenas() {
        return outcome;
    }
    let error =
        |e: craft3d_engine::CommandError| CliError::Code(EXIT_ERROR, e.message().to_string());
    let mut session = Session::new(Workspace::unrestricted());
    // A run may take hours: what the kernel keeps goes after every point.
    session.set_lean_sampling(true);
    session
        .execute("file.open", &json!({ "path": options.file }))
        .map_err(error)?;
    let parent: Option<u32> = std::env::var(PARENT).ok().and_then(|p| p.parse().ok());
    // A line per point as it is checked, so a run stopped from outside (a
    // timeout, a kill) has said what it found: on standard output before
    // the report, or on standard error with --json, whose standard output
    // is the result alone.
    let to_stderr = options.json;
    let params = Value::Object(options.params);
    let result = craft3d_engine::with_point_sink(
        move |point| {
            let line = point_line(point);
            // A line that cannot be written is not the run's failure.
            let _ = if to_stderr {
                writeln!(std::io::stderr().lock(), "{line}")
            } else {
                let mut out = std::io::stdout().lock();
                writeln!(out, "{line}").and_then(|()| out.flush())
            };
            // Started by another run of this program that is gone: stop.
            !orphaned(parent)
        },
        || session.execute("document.sample", &params),
    )
    .map_err(error)?;
    let text = if options.json {
        serde_json::to_string(&result)
            .map_err(|e| CliError::Failed(format!("cannot write the result as JSON: {e}")))?
    } else {
        report(&options.file, &result)
    };
    print(&text)?;
    let failed = field(&result, "failed").as_u64().unwrap_or(0);
    let points = field(field(&result, "counts"), "points");
    if failed > 0 {
        Err(CliError::Failed(format!(
            "{failed} of {points} points fail"
        )))
    } else if field(&result, "complete") != true || field(&result, "ok") != true {
        Err(CliError::Code(
            EXIT_INCOMPLETE,
            format!(
                "the run stopped ({}) after {points} points, none failing: not every point was checked",
                field(&result, "stoppedBy").as_str().unwrap_or("early")
            ),
        ))
    } else {
        Ok(Parsed::Run(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A check's process killed by a signal (the out-of-memory killer
    /// sends 9) is an error of the check, exit code 3, not a failing point
    /// (exit code 1, which it was).
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn a_check_killed_by_a_signal_is_an_error_not_a_failing_point() {
        use std::os::unix::process::ExitStatusExt;
        match outcome_of(std::process::ExitStatus::from_raw(9)) {
            Err(CliError::Code(code, message)) => {
                assert_eq!(code, EXIT_ERROR);
                assert!(message.contains("signal 9"), "{message}");
            }
            _ => panic!("not an error of the check"),
        }
        // An exit code is passed on as it is: that run said why itself.
        assert!(matches!(
            outcome_of(std::process::ExitStatus::from_raw(1 << 8)),
            Err(CliError::Quiet(1))
        ));
        assert!(matches!(
            outcome_of(std::process::ExitStatus::from_raw(0)),
            Ok(Parsed::Run(()))
        ));
    }

    #[test]
    fn a_point_line_says_what_was_clamped() {
        let point = json!({
            "index": 3, "kind": "choice", "ok": true, "ms": 1.0, "problems": [],
            "unproven": [], "values": {"size": "1", "n": "3"},
            "clamped": [{"parameter": "n", "from": "5", "to": "3"}],
        });
        assert_eq!(
            point_line(&point),
            "point 3 (choice, 1.0 ms): passed (clamped n 5 -> 3)"
        );
    }
}
