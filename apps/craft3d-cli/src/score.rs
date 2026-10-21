//! `craft3d-cli score`: runs a candidate script for a task of the task set
//! (`docs/tasks/`) headless and prints its scorecard, the core of a CAD gym
//! (`docs/testing.md` › *The task set*). The work is
//! `craft3d_engine::task_score`; this file reads the files.

use crate::args::{self, Arg, Args};
use crate::{CliError, Parsed, print};
use serde_json::Value;
use std::path::PathBuf;

/// Largest task or script file read: both are lists, not data.
const MAX_FILE_BYTES: u64 = 16 << 20;

/// Where a task id is looked up when `--tasks` is not given.
const DEFAULT_TASKS_DIR: &str = "docs/tasks";

fn read_json(what: &str, path: &PathBuf) -> Result<Value, CliError> {
    let shown = path.display();
    let fail = |e: std::io::Error| CliError::Failed(format!("cannot read {what} `{shown}`: {e}"));
    let size = std::fs::metadata(path).map_err(fail)?.len();
    if size > MAX_FILE_BYTES {
        return Err(CliError::Failed(format!(
            "{what} `{shown}` is {size} bytes; the limit is {MAX_FILE_BYTES}"
        )));
    }
    let text = std::fs::read_to_string(path).map_err(fail)?;
    serde_json::from_str(&text)
        .map_err(|e| CliError::Failed(format!("{what} `{shown}` is not valid JSON: {e}")))
}

/// A task given as a file, or as an id under the tasks directory.
fn task_path(task: &str, dir: Option<String>) -> PathBuf {
    let as_file = PathBuf::from(task);
    if task.ends_with(".json") || (dir.is_none() && as_file.is_file()) {
        return as_file;
    }
    PathBuf::from(dir.unwrap_or_else(|| DEFAULT_TASKS_DIR.to_string())).join(format!("{task}.json"))
}

pub(crate) fn score(mut args: Args) -> Result<Parsed<()>, CliError> {
    let mut task: Option<String> = None;
    let mut script: Option<String> = None;
    let mut dir: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg {
            Arg::Flag { name, inline } => match name.as_str() {
                "--help" | "-h" => {
                    args::no_value(&name, inline.as_ref())?;
                    return Ok(Parsed::Help);
                }
                "--script" | "--tasks" => {
                    let value = args.value(&name, inline)?;
                    let slot = if name == "--script" {
                        &mut script
                    } else {
                        &mut dir
                    };
                    if slot.replace(value).is_some() {
                        return Err(CliError::Usage(format!("`{name}` was given twice")));
                    }
                }
                _ => return Err(CliError::Usage(format!("unknown flag `{name}`"))),
            },
            Arg::Positional(value) => {
                if task.replace(value.clone()).is_some() {
                    return Err(CliError::Usage(format!("unexpected argument `{value}`")));
                }
            }
        }
    }
    let task = task.ok_or_else(|| CliError::Usage("missing TASK".to_string()))?;
    let task_file = task_path(&task, dir);
    let task_json = read_json("task", &task_file)?;
    let script_json = match &script {
        Some(path) => read_json("script", &PathBuf::from(path))?,
        None => craft3d_engine::task_score::reference_solution(&task_json)
            .map_err(|e| CliError::Failed(e.message().to_string()))?
            .clone(),
    };
    let card = craft3d_engine::task_score::score_task(&task_json, &script_json)
        .map_err(|e| CliError::Failed(e.message().to_string()))?;
    let line = serde_json::to_string(&card)
        .map_err(|e| CliError::Failed(format!("cannot write the scorecard as JSON: {e}")))?;
    print(&line).map(Parsed::Run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_is_a_file_or_an_id_in_the_tasks_directory() {
        assert_eq!(task_path("a/b.json", None), PathBuf::from("a/b.json"));
        assert_eq!(
            task_path("plate-holes", None),
            PathBuf::from("docs/tasks/plate-holes.json")
        );
        assert_eq!(
            task_path("plate-holes", Some("x".into())),
            PathBuf::from("x/plate-holes.json")
        );
    }
}
