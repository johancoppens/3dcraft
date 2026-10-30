//! `cargo xtask web-size`: holds the size of the web build's module to a
//! budget (docs/testing.md, Web module size), as the benchmark holds the
//! work counts to theirs. "3DCraft stays light" (the developer,
//! 2026-10-23).
//!
//! It builds the module exactly as `cargo xtask package --formats web`
//! does (the `web` profile, wasm-bindgen, wasm-opt: `crate::web::module`), then
//! measures the final `_bg.wasm` raw and compressed with gzip at level 9
//! (flate2's pure-Rust backend, so the number is the same on every
//! machine), and compares the gzip size, what a server with gzip sends,
//! with:
//!
//! - a hard limit of [`LIMIT_GZIP`] bytes;
//! - the committed baseline (`apps/craft3d-web/size-baseline.txt`): more
//!   than [`MARGIN_PERCENT`] % above it fails; more than that below it is
//!   a note asking to lower the baseline.
//!
//! `--update` writes the measured sizes as the new baseline (an
//! expectation change: docs/testing.md, Changing an expectation).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The gzip size no module may exceed, whatever the baseline: 12 MiB
/// (the developer, 2026-10-30; 8 MiB before, docs/decisions.md).
pub const LIMIT_GZIP: u64 = 12 * 1024 * 1024;
/// How far the gzip size may grow over the baseline, in percent.
pub const MARGIN_PERCENT: u64 = 10;
/// The baseline, relative to the repository.
const BASELINE: &str = "apps/craft3d-web/size-baseline.txt";

/// What the command line asked for.
#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    update: bool,
    build: bool,
    jobs: Option<String>,
    wasm_bindgen: Option<PathBuf>,
    wasm_opt: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        build: true,
        ..Options::default()
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--update" => o.update = true,
            "--no-build" => o.build = false,
            "-j" | "--jobs" => o.jobs = Some(value()?),
            "--wasm-bindgen" => o.wasm_bindgen = Some(PathBuf::from(value()?)),
            "--wasm-opt" => o.wasm_opt = Some(PathBuf::from(value()?)),
            other => {
                return Err(format!(
                    "web-size: unknown argument `{other}`; usage: cargo xtask web-size \
                     [--update] [--no-build] [-j N] [--wasm-bindgen PATH] [--wasm-opt PATH]"
                ));
            }
        }
    }
    Ok(o)
}

/// The sizes of one module, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sizes {
    pub raw: u64,
    pub gzip: u64,
}

/// The committed baseline: the sizes and the commit they were measured at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Baseline {
    pub sizes: Sizes,
    pub commit: String,
}

/// Reads a baseline: `key = value` lines (`raw`, `gzip`, `commit`), `#`
/// comments and blank lines. A missing, repeated or unknown key, or a size
/// that is not a whole number, is an error.
pub fn parse_baseline(text: &str) -> Result<Baseline, String> {
    let (mut raw, mut gzip, mut commit) = (None, None, None);
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = n.saturating_add(1);
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("line {at}: `{line}` is not `key = value`"))?;
        let (key, value) = (key.trim(), value.trim());
        let size = || {
            value
                .parse::<u64>()
                .map_err(|_| format!("line {at}: `{key}` is `{value}`, not a size in bytes"))
        };
        let slot_was_empty = match key {
            "raw" => raw.replace(size()?).is_none(),
            "gzip" => gzip.replace(size()?).is_none(),
            "commit" => commit.replace(value.to_string()).is_none(),
            other => return Err(format!("line {at}: unknown key `{other}`")),
        };
        if !slot_was_empty {
            return Err(format!("line {at}: `{key}` is given twice"));
        }
    }
    let missing = |key: &str| format!("the baseline has no `{key}`");
    let gzip = gzip.ok_or_else(|| missing("gzip"))?;
    if gzip == 0 {
        return Err("the baseline's `gzip` is 0".to_string());
    }
    Ok(Baseline {
        sizes: Sizes {
            raw: raw.ok_or_else(|| missing("raw"))?,
            gzip,
        },
        commit: commit.ok_or_else(|| missing("commit"))?,
    })
}

/// The text of a baseline file.
pub fn format_baseline(b: &Baseline) -> String {
    format!(
        "# The web build's module (`craft3d-web-<hash>_bg.wasm` after wasm-opt), in bytes:\n\
         # raw, and compressed with gzip -9 (flate2). Held by `cargo xtask web-size`\n\
         # (docs/testing.md, Web module size); updating it is an expectation change.\n\
         raw = {}\ngzip = {}\ncommit = {}\n",
        b.sizes.raw, b.sizes.gzip, b.commit
    )
}

/// What the comparison found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// Why the check fails; empty when it passes.
    pub failures: Vec<String>,
    /// What to tell without failing (a gain to lock in).
    pub notes: Vec<String>,
}

fn mib(bytes: u64) -> String {
    // A size in bytes is far below 2^53: the conversion is exact.
    #[allow(clippy::cast_precision_loss)]
    let m = bytes as f64 / (1024.0 * 1024.0);
    format!("{m:.2} MiB")
}

/// The change from `from` to `to` in percent, one decimal.
fn percent(from: u64, to: u64) -> String {
    if from == 0 {
        return "n/a".to_string();
    }
    #[allow(clippy::cast_precision_loss)]
    let p = (to as f64 - from as f64) * 100.0 / from as f64;
    format!("{p:+.1} %")
}

/// Holds `measured` to the hard limit and to `baseline`.
pub fn compare(measured: Sizes, baseline: &Baseline) -> Verdict {
    let mut v = Verdict {
        failures: Vec::new(),
        notes: Vec::new(),
    };
    let base = baseline.sizes.gzip;
    if measured.gzip > LIMIT_GZIP {
        v.failures.push(format!(
            "the module is {} bytes ({}) compressed, over the limit of {} bytes ({})",
            measured.gzip,
            mib(measured.gzip),
            LIMIT_GZIP,
            mib(LIMIT_GZIP)
        ));
    }
    // Integer arithmetic: gzip / base > 1 + margin, as gzip * 100 > base * (100 + margin).
    let scaled = u128::from(measured.gzip).saturating_mul(100);
    let over = u128::from(base).saturating_mul(u128::from(100 + MARGIN_PERCENT));
    let under = u128::from(base).saturating_mul(u128::from(100 - MARGIN_PERCENT));
    if scaled > over {
        v.failures.push(format!(
            "the module grew to {} bytes compressed, {} over the baseline of {} bytes \
             (measured at {}); more than {MARGIN_PERCENT} % fails. Make it smaller, or, \
             when the growth is wanted, accept it with `cargo xtask web-size --update` \
             and log it as an expectation change (docs/testing.md)",
            measured.gzip,
            percent(base, measured.gzip),
            base,
            baseline.commit
        ));
    } else if scaled < under {
        v.notes.push(format!(
            "the module shrank to {} bytes compressed, {} below the baseline of {} bytes: \
             lock the gain in with `cargo xtask web-size --update`",
            measured.gzip,
            percent(base, measured.gzip),
            base
        ));
    }
    v
}

/// Compresses `bytes` as `gzip -9` does, and returns the compressed size.
pub fn gzip_size(bytes: &[u8]) -> Result<u64, String> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder
        .write_all(bytes)
        .map_err(|e| format!("cannot compress the module: {e}"))?;
    let out = encoder
        .finish()
        .map_err(|e| format!("cannot compress the module: {e}"))?;
    u64::try_from(out.len()).map_err(|_| "the compressed module is too large".to_string())
}

fn head_commit(repo: &Path) -> String {
    Command::new("git")
        .current_dir(repo)
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

pub fn run(args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    let repo = crate::package::repo_root();
    let target = crate::package::target_dir(&repo);
    let stage = target.join("web-size");
    let jobs: Vec<&str> = match &options.jobs {
        Some(j) => vec!["-j", j.as_str()],
        None => Vec::new(),
    };
    let tools = crate::web::Tools {
        wasm_bindgen: options.wasm_bindgen.clone(),
        wasm_opt: options.wasm_opt.clone(),
    };
    let (wasm, _js) = crate::web::module(
        &repo,
        &target,
        &stage,
        &tools,
        options.build.then_some(jobs.as_slice()),
        true,
    )?;
    let measured = Sizes {
        raw: u64::try_from(wasm.len()).map_err(|_| "the module is too large".to_string())?,
        gzip: gzip_size(&wasm)?,
    };
    let commit = head_commit(&repo);
    let report = format!(
        "raw = {}\ngzip = {}\ncommit = {commit}\n",
        measured.raw, measured.gzip
    );
    crate::package::write(&stage.join("last.txt"), report.as_bytes())?;
    eprintln!(
        "web-size: the module is {} bytes raw ({}), {} bytes with gzip -9 ({}); limit {}",
        measured.raw,
        mib(measured.raw),
        measured.gzip,
        mib(measured.gzip),
        mib(LIMIT_GZIP)
    );
    let path = repo.join(BASELINE);
    if options.update {
        if measured.gzip > LIMIT_GZIP {
            return Err(format!(
                "web-size: {} bytes compressed is over the limit of {LIMIT_GZIP}; \
                 the baseline cannot be set above it",
                measured.gzip
            ));
        }
        let text = format_baseline(&Baseline {
            sizes: measured,
            commit,
        });
        crate::package::write(&path, text.as_bytes())?;
        eprintln!("web-size: wrote {}", path.display());
        return Ok(());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "web-size: cannot read {} ({e}); make it with `cargo xtask web-size --update`",
            path.display()
        )
    })?;
    let baseline = parse_baseline(&text).map_err(|e| format!("web-size: {BASELINE}: {e}"))?;
    eprintln!(
        "web-size: baseline {} bytes raw, {} bytes gzip (measured at {}): raw {}, gzip {}",
        baseline.sizes.raw,
        baseline.sizes.gzip,
        baseline.commit,
        percent(baseline.sizes.raw, measured.raw),
        percent(baseline.sizes.gzip, measured.gzip)
    );
    let verdict = compare(measured, &baseline);
    for note in &verdict.notes {
        eprintln!("web-size: note: {note}");
    }
    if verdict.failures.is_empty() {
        eprintln!("web-size: ok");
        Ok(())
    } else {
        Err(format!("web-size: {}", verdict.failures.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(gzip: u64) -> Baseline {
        Baseline {
            sizes: Sizes {
                raw: gzip.saturating_mul(3),
                gzip,
            },
            commit: "abc".to_string(),
        }
    }

    fn at(gzip: u64) -> Sizes {
        Sizes {
            raw: gzip.saturating_mul(3),
            gzip,
        }
    }

    #[test]
    fn within_the_margin_passes_quietly() {
        for gzip in [1_000_000, 1_100_000, 900_000, 1_050_000, 950_000] {
            let v = compare(at(gzip), &base(1_000_000));
            assert!(v.failures.is_empty(), "{gzip}: {v:?}");
            assert!(v.notes.is_empty(), "{gzip}: {v:?}");
        }
    }

    #[test]
    fn growth_over_ten_percent_fails() {
        let v = compare(at(1_100_001), &base(1_000_000));
        assert_eq!(v.failures.len(), 1, "{v:?}");
        assert!(v.failures[0].contains("--update"), "{v:?}");
        assert!(v.notes.is_empty());
    }

    #[test]
    fn shrinking_over_ten_percent_is_a_note_not_a_failure() {
        let v = compare(at(899_999), &base(1_000_000));
        assert!(v.failures.is_empty(), "{v:?}");
        assert_eq!(v.notes.len(), 1, "{v:?}");
        assert!(v.notes[0].contains("lock the gain in"), "{v:?}");
    }

    #[test]
    fn the_hard_limit_fails_whatever_the_baseline() {
        // Within the margin of a baseline that is itself over the limit.
        let v = compare(at(LIMIT_GZIP + 1), &base(LIMIT_GZIP));
        assert_eq!(v.failures.len(), 1, "{v:?}");
        assert!(v.failures[0].contains("over the limit"), "{v:?}");
        let v = compare(at(LIMIT_GZIP), &base(LIMIT_GZIP));
        assert!(v.failures.is_empty(), "{v:?}");
        // Both rules at once.
        let v = compare(at(LIMIT_GZIP + 1), &base(1_000_000));
        assert_eq!(v.failures.len(), 2, "{v:?}");
    }

    #[test]
    fn huge_sizes_do_not_overflow() {
        let v = compare(at(u64::MAX), &base(u64::MAX));
        assert_eq!(v.failures.len(), 1, "{v:?}");
        let v = compare(at(0), &base(u64::MAX));
        assert_eq!(v.notes.len(), 1, "{v:?}");
    }

    #[test]
    fn a_baseline_reads_back() {
        let b = Baseline {
            sizes: Sizes {
                raw: 15_540_156,
                gzip: 5_976_000,
            },
            commit: "dca10e06".to_string(),
        };
        assert_eq!(parse_baseline(&format_baseline(&b)), Ok(b));
    }

    #[test]
    fn a_malformed_baseline_is_an_error() {
        for (text, says) in [
            ("", "no `gzip`"),
            ("raw = 1\ncommit = x\n", "no `gzip`"),
            ("gzip = 1\ncommit = x\n", "no `raw`"),
            ("raw = 1\ngzip = 1\n", "no `commit`"),
            ("raw = 1\ngzip = big\ncommit = x\n", "not a size"),
            ("raw = -1\ngzip = 1\ncommit = x\n", "not a size"),
            (
                "raw = 1\ngzip = 99999999999999999999999\ncommit = x\n",
                "not a size",
            ),
            ("raw = 1\ngzip = 0\ncommit = x\n", "is 0"),
            ("raw = 1\ngzip = 1\ngzip = 2\ncommit = x\n", "twice"),
            ("raw = 1\ngzip = 1\ncommit = x\nwhat = 3\n", "unknown key"),
            ("raw 1\n", "not `key = value`"),
            ("\u{0}\u{ff}", "not `key = value`"),
        ] {
            let err = parse_baseline(text).expect_err(text);
            assert!(err.contains(says), "{text:?}: {err}");
        }
    }

    #[test]
    fn the_committed_baseline_reads_and_is_within_the_limit() {
        let path = crate::package::repo_root().join(BASELINE);
        let text = std::fs::read_to_string(&path).expect("the baseline is committed");
        let b = parse_baseline(&text).expect("the baseline reads");
        assert!(b.sizes.gzip <= LIMIT_GZIP);
        assert!(b.sizes.gzip < b.sizes.raw);
    }

    #[test]
    fn gzip_is_deterministic_and_compresses() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let a = gzip_size(&data).expect("compresses");
        assert_eq!(Ok(a), gzip_size(&data));
        assert!(a < 10_000, "{a}");
        assert!(gzip_size(&[]).expect("empty") > 0);
    }

    #[test]
    fn arguments_parse() {
        let args = |a: &[&str]| a.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let o = parse(&args(&["--update", "--no-build", "-j", "4"])).expect("parses");
        assert!(o.update && !o.build);
        assert_eq!(o.jobs.as_deref(), Some("4"));
        assert!(parse(&args(&["-j"])).is_err());
        assert!(parse(&args(&["--bogus"])).is_err());
        assert!(parse(&[]).expect("default").build);
    }
}
