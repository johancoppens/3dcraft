//! `craft3d-cli check-range`: proves a Jig over its parameter range
//! (the command `document.sample`) and exits non-zero when any point
//! fails, for a Jig repository's CI. The work is the engine's; this file
//! reads the flags and writes the report.

use crate::args::{self, Arg, Args};
use crate::{CliError, Parsed, print};
use craft3d_engine::{Session, Workspace};
use serde_json::{Map, Value, json};

/// The options, as `document.sample` params plus how to print.
struct Options {
    file: String,
    params: Map<String, Value>,
    json: bool,
}

fn whole(flag: &str, text: &str) -> Result<Value, CliError> {
    text.parse::<u64>()
        .map(Value::from)
        .map_err(|_| CliError::Usage(format!("`{flag}` must be a whole number, got `{text}`")))
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
                    "--deep" | "--json" | "--no-corners" => {
                        args::no_value(&name, inline.as_ref())?;
                        match name.as_str() {
                            "--deep" => params.insert("deep".into(), true.into()),
                            "--no-corners" => params.insert("corners".into(), false.into()),
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
                    _ => return Err(CliError::Usage(format!("unknown flag `{name}`"))),
                };
                let value = args.value(&name, inline)?;
                let value = if key == "budgetMs" {
                    value
                        .parse::<f64>()
                        .ok()
                        .filter(|ms| ms.is_finite() && *ms > 0.0)
                        .map(Value::from)
                        .ok_or_else(|| {
                            CliError::Usage(format!(
                                "`{name}` must be a positive number of milliseconds, got `{value}`"
                            ))
                        })?
                } else {
                    whole(&name, &value)?
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

/// The report for a person: one line of totals, the times, then one
/// block per failing point.
fn report(file: &str, result: &Value) -> String {
    let counts = field(result, "counts");
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let mut lines = vec![format!(
        "{file}: {} parameters, {} points ({} default, {} corners{}, {} choices, {} random): {} passed, {} failed",
        field(result, "parameters").as_array().map_or(0, Vec::len),
        n(field(counts, "points")),
        n(field(counts, "default")),
        n(field(counts, "corner")),
        if field(counts, "allCorners") == true {
            ", all"
        } else {
            ""
        },
        n(field(counts, "choice")),
        n(field(counts, "random")),
        n(field(result, "passed")),
        n(field(result, "failed")),
    )];
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
        let values: Vec<String> = field(failure, "values")
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, value)| format!("{name}={}", value.as_str().unwrap_or("?")))
            .collect();
        lines.push(format!(
            "FAIL point {} ({}): {}",
            field(failure, "index"),
            field(failure, "kind").as_str().unwrap_or("?"),
            values.join(", ")
        ));
        for problem in field(failure, "problems").as_array().into_iter().flatten() {
            let what = ["name", "parameter", "assertion", "variable"]
                .iter()
                .find_map(|key| field(problem, key).as_str())
                .map(|w| format!("{w}: "))
                .unwrap_or_default();
            lines.push(format!(
                "  {what}{}",
                field(problem, "problem")
                    .as_str()
                    .map_or_else(|| field(problem, "problem").to_string(), str::to_string)
            ));
        }
    }
    if field(result, "truncated") == true {
        lines.push(format!(
            "... and {} more failing points (--json lists the first 100)",
            n(field(result, "failed"))
                .saturating_sub(field(result, "failures").as_array().map_or(0, Vec::len) as u64)
        ));
    }
    lines.join("\n")
}

pub(crate) fn check_range(args: Args) -> Result<Parsed<()>, CliError> {
    let options = match parse(args)? {
        Parsed::Run(options) => options,
        Parsed::Help => return Ok(Parsed::Help),
    };
    let mut session = Session::new(Workspace::unrestricted());
    session
        .execute("file.open", &json!({ "path": options.file }))
        .map_err(|e| CliError::Failed(e.message().to_string()))?;
    let result = session
        .execute("document.sample", &Value::Object(options.params))
        .map_err(|e| CliError::Failed(e.message().to_string()))?;
    let text = if options.json {
        serde_json::to_string(&result)
            .map_err(|e| CliError::Failed(format!("cannot write the result as JSON: {e}")))?
    } else {
        report(&options.file, &result)
    };
    print(&text)?;
    if field(&result, "ok") == true {
        Ok(Parsed::Run(()))
    } else {
        Err(CliError::Failed(format!(
            "{} of {} points fail",
            field(&result, "failed"),
            field(field(&result, "counts"), "points")
        )))
    }
}
