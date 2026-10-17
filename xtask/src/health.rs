//! `cargo xtask health`: the morning health report, a short fixed page of
//! Markdown on stdout (docs/development.md › Morning health report).
//!
//! It only reads: the branches, the logs of the gate worktree, the
//! evidence the gate's last run left (the boolean corpus score, the
//! benchmark's counts), the summary of the last `cargo xtask fuzz` run,
//! the sources, the showcases' findings under
//! `target/work` and the dev log. Whatever is missing is reported as
//! "unknown"; the report itself never fails. It is also kept in
//! `target/health/<date>.md`, and the dev log line it read up to in
//! `target/health/last.txt`, so the next report lists only the expectation
//! changes logged since this one.
//!
//! Where the gate lives: `$CRAFT3D_GATE_DIR`, else
//! `<main checkout>/.claude/worktrees/gate` (log/gate.sh).

use crate::stats;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

const UNKNOWN: &str = "unknown";

/// Expectation changes listed at most; the count covers them all.
const SHOWN_CHANGES: usize = 10;

/// Output of a command run in `dir`, trimmed; `None` when it fails.
fn output(dir: &Path, program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The main checkout: the parent of the repository's common git dir (a
/// worktree shares it), else the current directory.
pub(crate) fn main_checkout(here: &Path) -> PathBuf {
    output(
        here,
        "git",
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .and_then(|dir| Path::new(&dir).parent().map(Path::to_path_buf))
    .unwrap_or_else(|| here.to_path_buf())
}

/// `time` as local time, `YYYY-MM-DD HH:MM`.
fn when(time: SystemTime) -> String {
    let Ok(since) = time.duration_since(SystemTime::UNIX_EPOCH) else {
        return UNKNOWN.to_string();
    };
    let at = format!("@{}", since.as_secs());
    output(Path::new("."), "date", &["-d", &at, "+%Y-%m-%d %H:%M"])
        .unwrap_or_else(|| format!("{} s after 1970 (UTC)", since.as_secs()))
}

fn minutes(seconds: u64) -> String {
    if seconds >= 3600 {
        format!("{} h {:02} min", seconds / 3600, (seconds % 3600) / 60)
    } else {
        format!("{} min {:02} s", seconds / 60, seconds % 60)
    }
}

/// The gate log with the highest number: `gate-<n>.log`.
fn last_gate_log(target: &Path) -> Option<(u64, PathBuf)> {
    std::fs::read_dir(target)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let number = name
                .strip_prefix("gate-")?
                .strip_suffix(".log")?
                .parse()
                .ok()?;
            Some((number, entry.path()))
        })
        .max_by_key(|(number, _)| *number)
}

fn gate_section(out: &mut String, gate: &Path) {
    let target = gate.join("target");
    let Some((number, log)) = last_gate_log(&target) else {
        let _ = writeln!(
            out,
            "| Last gate | {UNKNOWN} (no gate log in {}) |",
            target.display()
        );
        return;
    };
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    let last = text.lines().last().unwrap_or_default();
    // log/gate.sh ends the log with `exit <status> commit <hash>`.
    let verdict = match last
        .strip_prefix("exit ")
        .and_then(|r| r.split_once(" commit "))
    {
        Some(("0", commit)) => format!("**passed** on {commit}"),
        Some((status, commit)) => format!("**FAILED** (exit {status}) on {commit}"),
        None => "still running (or cut off)".to_string(),
    };
    let meta = std::fs::metadata(&log).ok();
    let finished = meta.as_ref().and_then(|m| m.modified().ok());
    let took = meta
        .as_ref()
        .and_then(|m| m.modified().ok()?.duration_since(m.created().ok()?).ok())
        .map_or(UNKNOWN.to_string(), |d| minutes(d.as_secs()));
    let tests = text
        .lines()
        .find(|line| line.starts_with("tests: "))
        .map_or(String::new(), |line| {
            format!("; {}", line.trim_start_matches("tests: "))
        });
    let failed: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("== ") && line.ends_with(": FAILED"))
        .map(|line| line.trim_start_matches("== "))
        .collect();
    let _ = writeln!(
        out,
        "| Last gate | gate-{number}: {verdict}, finished {}, took {took}{tests} |",
        finished.map_or(UNKNOWN.to_string(), when)
    );
    if !failed.is_empty() {
        let _ = writeln!(out, "| Failed binaries | {} |", failed.join("; "));
    }
}

/// The TOTAL row of the corpus score: its numbers by column name.
fn corpus_totals(text: &str) -> Option<BTreeMap<String, u64>> {
    let header = text.lines().find(|l| l.starts_with("category / pair"))?;
    let names: Vec<&str> = header
        .trim_start_matches("category / pair")
        .split_whitespace()
        .collect();
    let total = text.lines().find(|l| l.starts_with("TOTAL"))?;
    let numbers: Vec<u64> = total
        .split_whitespace()
        .skip(1)
        .map_while(|w| w.parse().ok())
        .collect();
    Some(
        names
            .iter()
            .zip(numbers)
            .map(|(name, n)| (name.to_string(), n))
            .collect(),
    )
}

fn corpus_section(out: &mut String, gate: &Path) {
    let path = gate.join("target/evidence/booleans/corpus-score.txt");
    let Ok(text) = std::fs::read_to_string(&path) else {
        let _ = writeln!(out, "Boolean corpus: {UNKNOWN} (no {})", path.display());
        return;
    };
    let written = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map_or(UNKNOWN.to_string(), when);
    let digest = text
        .lines()
        .find_map(|l| l.strip_prefix("digest of all results: "))
        .unwrap_or(UNKNOWN);
    match corpus_totals(&text) {
        Some(t) => {
            let get = |k: &str| t.get(k).copied().unwrap_or(0);
            let agree = get("match") + get("match~") + get("empty");
            let _ = writeln!(
                out,
                "Boolean corpus ({written}): agree **{agree}** (match {}, match~ {}, empty {}) \
                 + {} refused by design (zero thickness, the body without the refusal agreeing), \
                 refused {}, unscored {}, mismatch **{}**, invalid **{}**, of {} outcomes; digest {digest}",
                get("match"),
                get("match~"),
                get("empty"),
                get("zero-thick"),
                get("unsupported"),
                get("unscored"),
                get("MISMATCH"),
                get("INVALID"),
                get("ops"),
            );
        }
        None => {
            let _ = writeln!(
                out,
                "Boolean corpus: {UNKNOWN} (no TOTAL row in {})",
                path.display()
            );
        }
    }
}

/// `model counter value` lines.
fn counts(path: &Path) -> Option<BTreeMap<(String, String), u64>> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(
        text.lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let mut w = l.split_whitespace();
                let model = w.next()?.to_string();
                let counter = w.next()?.to_string();
                Some(((model, counter), w.next()?.parse().ok()?))
            })
            .collect(),
    )
}

const BENCH_COLUMNS: [(&str, &str); 6] = [
    ("features_rebuilt", "rebuilt"),
    ("booleans", "booleans"),
    ("boolean_face_pairs", "face pairs"),
    ("mass_evaluations", "mass evals"),
    ("triangles", "triangles"),
    ("refusals", "refusals"),
];

fn bench_section(out: &mut String, gate: &Path, repo: &Path) {
    let last = gate.join("target/tmp/benchmark/last-counts.txt");
    let baseline_rel = "crates/engine/tests/bench_models/baseline.txt";
    let baseline = counts(&gate.join(baseline_rel)).or_else(|| counts(&repo.join(baseline_rel)));
    let Some(now) = counts(&last) else {
        let _ = writeln!(out, "Benchmark: {UNKNOWN} (no {})", last.display());
        return;
    };
    let baseline = baseline.unwrap_or_default();
    let written = std::fs::metadata(&last)
        .and_then(|m| m.modified())
        .map_or(UNKNOWN.to_string(), when);
    let _ = writeln!(
        out,
        "Benchmark, work counts of the gate's last run ({written}) against the baseline:\n"
    );
    let mut header = "| model |".to_string();
    let mut rule = "|---|".to_string();
    for (_, label) in BENCH_COLUMNS {
        let _ = write!(header, " {label} |");
        rule.push_str("---:|");
    }
    let _ = writeln!(out, "{header} largest rise |\n{rule}---|");
    let mut models: Vec<&String> = now.keys().map(|(m, _)| m).collect();
    models.dedup();
    for model in models {
        let mut row = format!("| {model} |");
        for (counter, _) in BENCH_COLUMNS {
            let value = now.get(&(model.clone(), counter.to_string()));
            let _ = write!(row, " {} |", value.map_or("-".to_string(), u64::to_string));
        }
        // The counter furthest above its baseline, in per cent.
        let rise = now
            .iter()
            .filter(|((m, _), _)| m == model)
            .filter_map(|((m, c), v)| {
                let base = *baseline.get(&(m.clone(), c.clone()))?;
                let pct = if base == 0 {
                    if *v == 0 { 0.0 } else { f64::INFINITY }
                } else {
                    (*v as f64 / base as f64 - 1.0) * 100.0
                };
                Some((pct, c.clone()))
            })
            .max_by(|a, b| a.0.total_cmp(&b.0));
        let rise = match rise {
            Some((pct, counter)) if pct > 0.0 => format!("{counter} {pct:+.1} %"),
            Some(_) => "none".to_string(),
            None => format!("{UNKNOWN} (no baseline)"),
        };
        let _ = writeln!(out, "{row} {rise} |");
    }
}

/// `spec!(` followed by a string: one command each.
fn command_count(repo: &Path) -> Option<usize> {
    let dir = std::fs::read_dir(repo.join("crates/engine/src")).ok()?;
    let mut count = 0;
    for entry in dir.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        count += text
            .split("spec!(")
            .skip(1)
            .filter(|rest| rest.trim_start().starts_with('"'))
            .count();
    }
    Some(count)
}

/// The text of a source file without its `#[cfg(test)]` part (by
/// convention a `mod tests` at the end).
fn without_tests(text: &str) -> &str {
    text.find("#[cfg(test)]\nmod tests")
        .and_then(|at| text.get(..at))
        .unwrap_or(text)
}

/// Places in the non-test sources of the crates that refuse with a reason:
/// an `Unsupported` error built, a boolean's `Stop::of(..)`, or an
/// `unsupported(..)` helper called.
fn refusal_sites(dir: &Path, depth: u32) -> usize {
    if depth > 32 {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut sites = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if name == "tests" || name == "target" || name == "examples" || name == "benches" {
                continue;
            }
            sites += refusal_sites(&path, depth + 1);
        } else if path.extension().is_some_and(|e| e == "rs")
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            let code = without_tests(&text);
            sites += code
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .map(|l| {
                    l.matches("::Unsupported").count()
                        + l.matches("Stop::of(").count()
                        + l.matches("unsupported(").count()
                        - l.matches("fn unsupported(").count()
                })
                .sum::<usize>();
        }
    }
    sites
}

/// The last `cargo xtask fuzz` run (docs/testing.md › Random multi-step
/// designs): its counts and rates, from `target/fuzz/summary.json`.
fn fuzz_section(out: &mut String, main: &Path) {
    let path = main.join("target/fuzz/summary.json");
    let Some(s) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
    else {
        let _ = writeln!(out, "Random designs: {UNKNOWN} (no {})", path.display());
        return;
    };
    let n = |k: &str| s.get(k).and_then(serde_json::Value::as_u64).unwrap_or(0);
    let text = |k: &str| {
        s.get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or(UNKNOWN)
    };
    let rate = |k: &str| s.get(k).and_then(serde_json::Value::as_f64).unwrap_or(0.0) * 100.0;
    let by_oracle = s
        .get("wrongByOracle")
        .and_then(serde_json::Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let wrong = s
        .pointer("/verdicts/wrong")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let _ = writeln!(
        out,
        "Random designs (`cargo xtask fuzz`, {} on {}): {} seeds, {} groups; refused {:.1} % of groups; \
         wrong **{wrong}** ({:.2} % of seeds{}), crashes **{}**, timeouts **{}**, out of memory **{}**; \
         reproducers under {}",
        text("date"),
        text("commit"),
        n("seeds"),
        n("groups"),
        rate("refusalRate"),
        rate("wrongRate"),
        if by_oracle.is_empty() {
            String::new()
        } else {
            format!(": {by_oracle}")
        },
        n("crashes"),
        n("timeouts"),
        n("outOfMemory"),
        main.join("target/fuzz").display(),
    );
    let top: Vec<String> = s
        .get("refusedTop")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .take(3)
        .filter_map(|r| {
            Some(format!(
                "{} × {}",
                r.get("count")?.as_u64()?,
                r.get("reason")?.as_str()?
            ))
        })
        .collect();
    if !top.is_empty() {
        let _ = writeln!(out, "- most refused: {}", top.join("; "));
    }
}

fn findings_section(out: &mut String, main: &Path) {
    let work = main.join("target/work");
    let Ok(entries) = std::fs::read_dir(&work) else {
        let _ = writeln!(out, "Showcase findings: {UNKNOWN} (no {})", work.display());
        return;
    };
    let mut rows: Vec<(String, usize)> = entries
        .flatten()
        .filter_map(|entry| {
            let text = std::fs::read_to_string(entry.path().join("findings/results.txt")).ok()?;
            let failed = text
                .lines()
                .filter(|l| l.starts_with("== ") && l.contains(" -> FAILED"))
                .count();
            Some((entry.file_name().to_string_lossy().to_string(), failed))
        })
        .collect();
    rows.sort();
    if rows.is_empty() {
        let _ = writeln!(
            out,
            "Showcase findings: {UNKNOWN} (no findings/results.txt under {})",
            work.display()
        );
        return;
    }
    let list: Vec<String> = rows.iter().map(|(name, n)| format!("{name} {n}")).collect();
    let _ = writeln!(
        out,
        "Open showcase findings (`-> FAILED` in findings/results.txt): {}",
        list.join(", ")
    );
}

/// Expectation changes in the dev log after the line the last report read
/// up to; the new line count to remember.
fn expectation_section(out: &mut String, main: &Path, state: &Path) -> Option<usize> {
    let devlog = main.join("log/devlog.md");
    let Ok(text) = std::fs::read_to_string(&devlog) else {
        let _ = writeln!(
            out,
            "Expectation changes: {UNKNOWN} (no {})",
            devlog.display()
        );
        return None;
    };
    let lines: Vec<&str> = text.lines().collect();
    let previous = std::fs::read_to_string(state).ok().and_then(|s| {
        let mut words = s.split_whitespace();
        let line: usize = words.next()?.parse().ok()?;
        Some((line, words.collect::<Vec<_>>().join(" ")))
    });
    // A dev log that got shorter was rewritten: read it all again.
    let from = previous
        .as_ref()
        .map_or(0, |(line, _)| if *line <= lines.len() { *line } else { 0 });
    let since = previous
        .as_ref()
        .map_or("ever (no previous report)".to_string(), |(_, at)| {
            format!("the report of {at}")
        });
    let found: Vec<(usize, &str)> = lines
        .iter()
        .enumerate()
        .skip(from)
        .filter(|(_, l)| l.to_lowercase().contains("expectation change"))
        .map(|(i, l)| (i + 1, *l))
        .collect();
    let _ = writeln!(
        out,
        "Expectation changes logged since {since}: **{}**",
        found.len()
    );
    // The last few; a first report would list the whole dev log.
    let skipped = found.len().saturating_sub(SHOWN_CHANGES);
    if skipped > 0 {
        let _ = writeln!(out, "- … {skipped} earlier ones not shown");
    }
    for (number, line) in found.into_iter().skip(skipped) {
        let short: String = line.chars().take(200).collect();
        let more = if line.chars().count() > 200 {
            " …"
        } else {
            ""
        };
        let _ = writeln!(out, "- devlog:{number}: {short}{more}");
    }
    Some(lines.len())
}

pub fn run() -> Result<(), String> {
    let here = std::env::current_dir().map_err(|e| format!("no current directory: {e}"))?;
    let main = main_checkout(&here);
    let gate = std::env::var_os("CRAFT3D_GATE_DIR")
        .map_or_else(|| main.join(".claude/worktrees/gate"), PathBuf::from);
    let now = when(SystemTime::now());
    let mut out = String::new();
    let _ = writeln!(out, "# 3DCraft health report, {now}\n");
    let _ = writeln!(out, "| | |\n|---|---|");
    for branch in ["main", "work"] {
        let line = output(
            &here,
            "git",
            &[
                "log",
                "-1",
                "--format=%h %cd %s",
                "--date=format:%Y-%m-%d %H:%M",
                branch,
            ],
        )
        .unwrap_or_else(|| UNKNOWN.to_string());
        let _ = writeln!(out, "| {branch} | {line} |");
    }
    gate_section(&mut out, &gate);
    let commands = command_count(&here).map_or(UNKNOWN.to_string(), |n| n.to_string());
    let rows = stats::rows().ok();
    let tests = rows.as_ref().map_or(UNKNOWN.to_string(), |r| {
        r.iter().map(|x| x.2).sum::<usize>().to_string()
    });
    let lines = rows.as_ref().map_or(UNKNOWN.to_string(), |r| {
        r.iter().map(|x| x.1).sum::<usize>().to_string()
    });
    let _ = writeln!(out, "| Commands | {commands} |");
    let _ = writeln!(out, "| Tests (`#[test]`) | {tests} |");
    let _ = writeln!(out, "| Lines of Rust | {lines} |");
    let _ = writeln!(
        out,
        "| Refusal sites | {} (`::Unsupported`, `Stop::of(`, `unsupported(` in non-test code of crates/) |",
        refusal_sites(&here.join("crates"), 0)
    );
    out.push('\n');
    corpus_section(&mut out, &gate);
    out.push('\n');
    bench_section(&mut out, &gate, &here);
    out.push('\n');
    fuzz_section(&mut out, &main);
    out.push('\n');
    findings_section(&mut out, &main);
    out.push('\n');
    let health = main.join("target/health");
    let state = health.join("last.txt");
    let read_up_to = expectation_section(&mut out, &main, &state);
    if let Some(rows) = &rows {
        let _ = writeln!(
            out,
            "\n<details><summary>Lines and tests per crate</summary>\n"
        );
        let _ = writeln!(out, "| crate | lines | tests |\n|---|---:|---:|");
        for (name, lines, tests) in rows {
            let _ = writeln!(out, "| {name} | {lines} | {tests} |");
        }
        let _ = writeln!(out, "\n</details>");
    }
    print!("{out}");
    // Kept for the next report; failing to keep it loses nothing today.
    if std::fs::create_dir_all(&health).is_ok() {
        let day = now.split(' ').next().unwrap_or("report");
        let _ = std::fs::write(health.join(format!("{day}.md")), &out);
        if let Some(line) = read_up_to {
            let _ = std::fs::write(&state, format!("{line} {now}\n"));
        }
    }
    Ok(())
}
