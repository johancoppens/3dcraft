//! `cargo xtask abc`: 3DCraft against real-world CAD, the STEP files of
//! the ABC dataset (A Big CAD Model Dataset, NYU 2019: about a million
//! Onshape models in chunks of 10 000; docs/testing.md › Real-world
//! models). Downloads one chunk (cached outside the repository, reused
//! across runs), then runs `craft3d-fuzz step` once per model, each in its
//! own process with a time limit and a memory cap (the launcher of
//! `cargo xtask fuzz`):
//!
//! 1. **import**: `file.import`, `document.check`, a mesh, a measure:
//!    ok, refused (with the reason), invalid (accepted, but the check finds
//!    fault), bug (refused with a bug the code caught itself), crash (a
//!    panic or a signal), timeout, oom;
//! 2. **booleans** (with `--booleans`): on the models stage 1 read as valid
//!    solids, three tools sized to the part (a drill, a slab, a corner
//!    block), each united, subtracted and intersected, held to
//!    `V(A ∪ B) + V(A ∩ B) = V(A) + V(B)`, `V(A − B) + V(A ∩ B) = V(A)` and
//!    `document.check`.
//!
//! With `--freecad N` (and `CRAFT3D_FREECAD`), FreeCAD reads N of the
//! models stage 1 accepted and the volumes are compared.
//!
//! **Licence.** The models' copyright is their creators' (Onshape's terms
//! of use): no model file, or part of one, is ever written into the
//! repository. What this task leaves under `--out` (default `target/abc`)
//! holds model ids, file hashes and results only: `import.jsonl`,
//! `booleans.jsonl`, `freecad.jsonl`, `summary.json` and `scorecard.md`.
//!
//! **What it trusts.** The archive is downloaded only from a plain https
//! URL on NYU's archive and kept only when it matches the dataset's MD5
//! sum; it is unpacked only when no entry leaves the directory or is a
//! link, and models are found without following links. A lock per chunk
//! keeps two runs apart; at most `--keep` chunks stay in the cache.

use crate::fuzz::{Cap, Launcher, cap_for, commit, now};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const USAGE: &str = "usage: cargo xtask abc [--chunk N] [--cache DIR] [--files DIR] [--limit N] [--only ID,ID] \
[--jobs J] [--timeout SECONDS] [--memory SIZE] [--minutes M] [--booleans] [--boolean-limit N] \
[--boolean-minutes M] [--boolean-timeout SECONDS] [--freecad N] [--total-minutes M] [--keep N] \
[--min-free GB] [--out DIR] [--bin PATH] | cargo xtask abc --rescore DIR";

/// The list of the STEP chunks: one `URL file-name` per line.
const LIST_URL: &str = "https://deep-geometry.github.io/abc-dataset/data/step_v00.txt";
/// Their MD5 sums: `file-name: sum` per line.
const MD5_URL: &str = "https://deep-geometry.github.io/abc-dataset/data/md5.yml";

/// The dataset's chunks: 0 to 99.
const CHUNKS: u32 = 100;

/// Most processes at once (the Strix is shared).
const MAX_JOBS: usize = 8;

/// Refusal reasons listed in the scorecard.
const TOP: usize = 20;

struct Config {
    chunk: u32,
    cache: PathBuf,
    files: Option<PathBuf>,
    limit: Option<usize>,
    /// Only these models (ids as the scorecard writes them, `00001054`,
    /// `00006380/3`): to look again at a few without running the chunk.
    only: Vec<String>,
    jobs: usize,
    timeout: u64,
    memory: String,
    minutes: Option<u64>,
    booleans: bool,
    boolean_limit: Option<usize>,
    boolean_minutes: Option<u64>,
    boolean_timeout: u64,
    freecad: usize,
    /// Wall time for the whole task, its build and download included: no
    /// stage starts work past it (a Heavy job has a hard limit).
    total_minutes: Option<u64>,
    /// Chunks kept in the cache, this one included.
    keep: usize,
    /// Free space asked for before downloading and before unpacking.
    min_free_gb: u64,
    /// Remake `scorecard.md` from the `.jsonl` files and `summary.json` of
    /// an earlier run in this directory, and nothing else.
    rescore: Option<PathBuf>,
    out: PathBuf,
    bin: Option<PathBuf>,
}

/// Seconds a FreeCAD batch of models may take.
const FREECAD_BATCH_SECONDS: u64 = 300;
/// Models per FreeCAD batch.
const FREECAD_BATCH: usize = 20;

fn parse(args: &[String]) -> Result<Config, String> {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    let mut c = Config {
        chunk: 0,
        cache: home.join("3dcraft-abc"),
        files: None,
        limit: None,
        only: Vec::new(),
        jobs: 4,
        timeout: 60,
        memory: "3G".to_string(),
        minutes: None,
        booleans: false,
        boolean_limit: None,
        boolean_minutes: None,
        boolean_timeout: 180,
        freecad: 0,
        total_minutes: None,
        keep: 2,
        min_free_gb: 25,
        rescore: None,
        out: crate::fuzz::workspace_root().join("target/abc"),
        bin: None,
    };
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        if flag == "--booleans" {
            c.booleans = true;
            continue;
        }
        let value = it
            .next()
            .ok_or_else(|| format!("{flag} needs a value; {USAGE}"))?;
        let number = || -> Result<u64, String> {
            value
                .parse()
                .map_err(|_| format!("{flag}: `{value}` is not a whole number"))
        };
        let count =
            || -> Result<usize, String> { Ok(usize::try_from(number()?).unwrap_or(usize::MAX)) };
        match flag.as_str() {
            "--chunk" => {
                c.chunk = u32::try_from(number()?)
                    .ok()
                    .filter(|n| *n < CHUNKS)
                    .ok_or_else(|| format!("--chunk: `{value}` is not a chunk number"))?;
            }
            "--cache" => c.cache = PathBuf::from(value),
            "--files" => c.files = Some(PathBuf::from(value)),
            "--limit" => c.limit = Some(count()?),
            "--only" => {
                c.only = value
                    .split(',')
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            "--jobs" => c.jobs = count()?.clamp(1, MAX_JOBS),
            "--timeout" => c.timeout = number()?.max(1),
            "--memory" => {
                if crate::fuzz::bytes(value).is_none() {
                    return Err(format!("--memory: `{value}` is not a size (such as 3G)"));
                }
                c.memory = value.clone();
            }
            "--minutes" => c.minutes = Some(number()?),
            "--boolean-limit" => c.boolean_limit = Some(count()?),
            "--boolean-minutes" => c.boolean_minutes = Some(number()?),
            "--boolean-timeout" => c.boolean_timeout = number()?.max(1),
            "--freecad" => c.freecad = count()?,
            "--total-minutes" => c.total_minutes = Some(number()?),
            "--keep" => c.keep = count()?.max(1),
            "--min-free" => c.min_free_gb = number()?,
            "--rescore" => c.rescore = Some(PathBuf::from(value)),
            "--out" => c.out = PathBuf::from(value),
            "--bin" => c.bin = Some(PathBuf::from(value)),
            other => return Err(format!("unknown option {other}; {USAGE}")),
        }
    }
    Ok(c)
}

/// Runs a program, its stdout when it succeeds.
fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "`{program} {}` failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn fetch_text(url: &str) -> Result<String, String> {
    output(
        "curl",
        &[
            "-sSfL",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--retry",
            "3",
            "--max-time",
            "120",
            "--",
            url,
        ],
    )
}

/// Where the dataset's list may send a download: NYU's archive, over https.
const ARCHIVE_PREFIX: &str = "https://archive.nyu.edu/";

/// The URL of `name` in the list of chunks: only one on NYU's archive,
/// over https, with nothing in it a shell or curl could read otherwise.
fn chunk_url(list: &str, name: &str) -> Result<String, String> {
    let url = list
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let url = parts.next()?;
            (parts.next()? == name).then(|| url.to_string())
        })
        .ok_or_else(|| format!("{LIST_URL} does not list {name}"))?;
    let tail = url.strip_prefix(ARCHIVE_PREFIX).unwrap_or("");
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.');
    if tail.is_empty() || !tail.chars().all(plain) || tail.contains("..") {
        return Err(format!(
            "{LIST_URL} gives `{url}` for {name}, not a plain https URL on {ARCHIVE_PREFIX}"
        ));
    }
    Ok(url)
}

/// The MD5 sum of `name` in `md5.yml`: 32 hexadecimal digits, or an error.
fn chunk_md5(text: &str, name: &str) -> Result<String, String> {
    let sum = text
        .lines()
        .find_map(|line| {
            let (file, sum) = line.split_once(':')?;
            (file.trim() == name).then(|| sum.trim().to_ascii_lowercase())
        })
        .ok_or_else(|| format!("{MD5_URL} has no MD5 sum for {name}; it is not downloaded"))?;
    if sum.len() == 32 && sum.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(sum)
    } else {
        Err(format!(
            "{MD5_URL} gives `{sum}` for {name}, not an MD5 sum"
        ))
    }
}

fn md5_of(path: &Path) -> Result<String, String> {
    let text = path.to_string_lossy().to_string();
    let got = output("md5sum", &["--", &text])?;
    Ok(got
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase())
}

/// Free bytes on the file system that holds `dir` (`df`).
fn free_bytes(dir: &Path) -> Result<u64, String> {
    let text = dir.to_string_lossy().to_string();
    let out = output("df", &["--output=avail", "-B1", "--", &text])?;
    out.lines()
        .filter_map(|l| l.trim().parse::<u64>().ok())
        .next()
        .ok_or_else(|| format!("cannot read the free space of {text} from df"))
}

fn need_free(dir: &Path, gb: u64, what: &str) -> Result<(), String> {
    let free = free_bytes(dir)?;
    let want = gb.saturating_mul(1 << 30);
    if free < want {
        return Err(format!(
            "{what} needs {gb} GB free in {}, there are {:.1} GB",
            dir.display(),
            free as f64 / f64::from(1_u32 << 30)
        ));
    }
    Ok(())
}

/// A chunk's lock: held (an advisory lock on `abc_NNNN.lock`, which the
/// system releases when the process ends, also when it is killed) for the
/// whole run, so that two runs never download, unpack or read the same
/// chunk at once, nor delete one another's.
fn lock_chunk(cache: &Path, chunk: u32) -> Result<Option<std::fs::File>, String> {
    let path = cache.join(format!("abc_{chunk:04}.lock"));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(format!("cannot lock {}: {e}", path.display())),
    }
}

/// The chunk's archive in the cache, downloaded when it is not there yet
/// and checked against the dataset's MD5 sum, which is required: `.ok`
/// beside it says the sum matched.
fn download(c: &Config) -> Result<PathBuf, String> {
    let name = format!("abc_{:04}_step_v00.7z", c.chunk);
    let archive = c.cache.join(&name);
    let verified = c.cache.join(format!("{name}.ok"));
    if archive.is_file() && verified.is_file() {
        return Ok(archive);
    }
    let _ = std::fs::remove_file(&verified);
    let url = chunk_url(&fetch_text(LIST_URL)?, &name)?;
    let want = chunk_md5(&fetch_text(MD5_URL)?, &name)?;
    // An archive kept from before (a run killed after its download):
    // kept only when it matches.
    if archive.is_file() {
        if md5_of(&archive)? == want {
            std::fs::write(&verified, "").map_err(|e| format!("cannot write the mark: {e}"))?;
            return Ok(archive);
        }
        let _ = std::fs::remove_file(&archive);
    }
    need_free(&c.cache, c.min_free_gb, "the download")?;
    let partial = c.cache.join(format!("{name}.partial"));
    let path = partial.to_string_lossy().to_string();
    eprintln!("abc: downloading {name} from {url}");
    let started = Instant::now();
    output(
        "curl",
        &[
            "-sSfL",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--retry",
            "3",
            "-C",
            "-",
            "-o",
            &path,
            "--",
            &url,
        ],
    )?;
    eprintln!(
        "abc: downloaded in {:.0} s",
        started.elapsed().as_secs_f64()
    );
    let got = md5_of(&partial)?;
    if got != want {
        let _ = std::fs::remove_file(&partial);
        return Err(format!(
            "{name}: MD5 {got}, the dataset says {want}; removed"
        ));
    }
    std::fs::rename(&partial, &archive).map_err(|e| format!("cannot keep {name}: {e}"))?;
    std::fs::write(&verified, "").map_err(|e| format!("cannot write the mark: {e}"))?;
    Ok(archive)
}

/// Whether a path from an archive stays inside the directory it is
/// unpacked into: relative, and only plain names (no `..`, no root).
fn stays_inside(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path).components().all(|c| {
            matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

/// The entries of 7-Zip's technical listing (`7z l -slt`) after its
/// header (whose `Path = ` is the archive's own): each entry's path, and
/// whether it is a link (a Unix mode that starts with `l`, or a link
/// target of its own).
fn listed_entries(listing: &str) -> Vec<(String, bool)> {
    let body = listing.split_once("\n----------").map_or("", |(_, e)| e);
    let mut out: Vec<(String, bool)> = Vec::new();
    for line in body.lines() {
        if let Some(path) = line.strip_prefix("Path = ") {
            out.push((path.to_string(), false));
        } else if let Some(last) = out.last_mut() {
            let link_mode = line.strip_prefix("Attributes = ").is_some_and(|a| {
                a.split_whitespace()
                    .any(|t| t.len() == 10 && t.starts_with('l'))
            });
            let target = ["Symbolic Link = ", "Hard Link = ", "Link = "]
                .iter()
                .any(|k| line.strip_prefix(k).is_some_and(|v| !v.trim().is_empty()));
            if link_mode || target {
                last.1 = true;
            }
        }
    }
    out
}

/// The 7-Zip there is on this machine.
fn seven_zip() -> Result<&'static str, String> {
    ["7z", "7za", "7zr", "7zz"]
        .into_iter()
        .find(|p| {
            Command::new(p)
                .arg("i")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok()
        })
        .ok_or_else(|| "no 7-Zip (7z, 7za, 7zr or 7zz) on this machine".to_string())
}

/// Every symbolic link under `dir`, without following any.
fn links_under(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            out.push(entry.path());
        } else if kind.is_dir() {
            links_under(&entry.path(), depth + 1, out);
        }
    }
}

/// The archive unpacked beside it, once: every entry it lists is checked
/// first to stay inside and not to be a link; it is unpacked into a directory of its own and
/// renamed into place only when 7-Zip succeeded and left no symbolic link.
fn extract(c: &Config, archive: &Path) -> Result<PathBuf, String> {
    let dir = c.cache.join(format!("abc_{:04}_step", c.chunk));
    let done = dir.join(".extracted");
    if done.is_file() {
        return Ok(dir);
    }
    let _ = std::fs::remove_dir_all(&dir);
    let seven = seven_zip()?;
    let a = archive.to_string_lossy().to_string();
    let listing = output(seven, &["l", "-slt", "--", &a])?;
    let entries = listed_entries(&listing);
    if entries.is_empty() {
        return Err(format!("{} lists no files", archive.display()));
    }
    if let Some((bad, _)) = entries.iter().find(|(p, _)| !stays_inside(p)) {
        return Err(format!(
            "{} has an entry that leaves the directory it is unpacked into: `{bad}`; not unpacked",
            archive.display()
        ));
    }
    if let Some((link, _)) = entries.iter().find(|(_, link)| *link) {
        return Err(format!(
            "{} holds a link (`{link}`); not unpacked",
            archive.display()
        ));
    }
    need_free(&c.cache, c.min_free_gb, "unpacking")?;
    let partial = c.cache.join(format!("abc_{:04}_step.partial", c.chunk));
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial)
        .map_err(|e| format!("cannot make {}: {e}", partial.display()))?;
    let to = format!("-o{}", partial.to_string_lossy());
    eprintln!("abc: unpacking with {seven}");
    let unpacked = output(seven, &["x", "-y", &to, "--", &a]);
    let mut links = Vec::new();
    if unpacked.is_ok() {
        links_under(&partial, 0, &mut links);
    }
    match (unpacked, links.first()) {
        (Err(e), _) => {
            let _ = std::fs::remove_dir_all(&partial);
            Err(format!("cannot unpack {}: {e}", archive.display()))
        }
        (Ok(_), Some(link)) => {
            let link = link.display().to_string();
            let _ = std::fs::remove_dir_all(&partial);
            Err(format!(
                "{} holds a symbolic link ({link}); removed",
                archive.display()
            ))
        }
        (Ok(_), None) => {
            std::fs::write(partial.join(".extracted"), "")
                .map_err(|e| format!("cannot write the mark: {e}"))?;
            std::fs::rename(&partial, &dir)
                .map_err(|e| format!("cannot move the unpacked chunk into place: {e}"))?;
            Ok(dir)
        }
    }
}

/// Keeps at most `keep` chunks in the cache, this one among them: the
/// others, least recently used first (the time their lock file was last
/// written), are deleted (archive and unpacked directory), each only when
/// no other run holds its lock. Lock files stay: removing one another run
/// may have just opened would let two runs hold "the" lock.
fn evict(cache: &Path, chunk: u32, keep: usize) {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    let mut others: Vec<(std::time::SystemTime, u32)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n: u32 = name
                .strip_prefix("abc_")?
                .strip_suffix(".lock")?
                .parse()
                .ok()?;
            let used = e.metadata().and_then(|m| m.modified()).ok()?;
            let present = cache.join(format!("abc_{n:04}_step")).exists()
                || cache.join(format!("abc_{n:04}_step_v00.7z")).exists();
            (n != chunk && present).then_some((used, n))
        })
        .collect();
    others.sort();
    let surplus = (others.len() + 1).saturating_sub(keep.max(1));
    for (_, n) in others.into_iter().take(surplus) {
        let Ok(Some(_lock)) = lock_chunk(cache, n) else {
            continue;
        };
        eprintln!("abc: removing chunk {n:04} from the cache");
        let _ = std::fs::remove_dir_all(cache.join(format!("abc_{n:04}_step")));
        let _ = std::fs::remove_dir_all(cache.join(format!("abc_{n:04}_step.partial")));
        for suffix in ["", ".ok", ".partial"] {
            let _ = std::fs::remove_file(cache.join(format!("abc_{n:04}_step_v00.7z{suffix}")));
        }
    }
}

/// Every `.step`/`.stp` file under `dir`, sorted, at most `depth` deep;
/// symbolic links are never followed nor listed.
fn step_files(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<(PathBuf, std::fs::FileType)> = entries
        .flatten()
        .filter_map(|e| Some((e.path(), e.file_type().ok()?)))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, kind) in entries {
        if kind.is_dir() {
            step_files(&path, depth + 1, out);
        } else if kind.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("step") || e.eq_ignore_ascii_case("stp"))
        {
            out.push(path);
        }
    }
}

/// A model's id: the ABC file `00000050_80d90bfdd2e74e709956122a_step_000.step`
/// is `00000050` (with `/1`, `/2` for a model's second and third file).
fn model_id(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut parts = stem.split('_');
    let first = parts.next().unwrap_or("").to_string();
    let last = stem.rsplit('_').next().unwrap_or("");
    match last.parse::<u32>() {
        Ok(n) if n > 0 && stem.contains("_step_") => format!("{first}/{n}"),
        _ => first,
    }
}

/// A reason with what differs per model taken out, so refusals of one kind
/// count together and nothing of a model reaches the scorecard: the
/// import's `` `file`, solid N (#id): `` prefix, the body named by an
/// export, numbers, quoted and parenthesised details (names from the file
/// are quoted); the first words.
fn reason_key(reason: &str) -> String {
    let mut text = reason;
    // `file.export: <body>: face 3 ...`: the body's name is the model's (a
    // product name from the file), never kept.
    if let Some(rest) = text.strip_prefix("file.export: ") {
        text = rest.split_once(": ").map_or(rest, |(_, after)| after);
    }
    if let Some(rest) = text.strip_prefix("file.import: ") {
        // `name.step`: ... or `name.step`, solid 0 (#123 at a/b): ...
        text = rest;
        if let Some(after) = rest.strip_prefix('`')
            && let Some(close) = after.find('`')
        {
            let tail = after.get(close + 1..).unwrap_or("");
            text = match tail.strip_prefix(", solid ") {
                Some(solid) => solid
                    .find("): ")
                    .map_or(solid, |i| solid.get(i + 3..).unwrap_or("")),
                None => tail.trim_start_matches(':').trim_start(),
            };
        }
    }
    let mut out = String::new();
    let mut depth = 0_u32;
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '`' => quoted = !quoted,
            _ if depth > 0 || quoted => {}
            c if c.is_ascii_digit() => {
                if !out.ends_with('#') {
                    out.push('#');
                }
            }
            c => out.push(c),
        }
    }
    // What the stripping leaves in front: a `: ` or the `#` of an entity.
    let out =
        out.trim_start_matches(|c: char| c == ':' || c == '#' || c == ',' || c.is_whitespace());
    let words: Vec<&str> = out.split_whitespace().take(16).collect();
    words.join(" ").chars().take(140).collect()
}

/// One model's line in a stage's `.jsonl`: its report from the process,
/// or how the process died, with the id and the file's name in place of
/// its path.
fn classify(launcher: &Launcher, args: &[String], file: &Path, stage: &str) -> Value {
    let ran = launcher.run(args);
    let mut line = Map::new();
    line.insert("id".into(), model_id(file).into());
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    line.insert("name".into(), name.into());
    match (ran.report, ran.died) {
        (Some(Value::Object(report)), _) => {
            for (k, v) in report {
                if k != "file" {
                    line.insert(k, v);
                }
            }
            if stage == "booleans" {
                // The stage's verdict is the booleans'; a model whose
                // import now fails says so.
                let verdict = line
                    .get("booleans")
                    .and_then(|b| b.get("verdict"))
                    .cloned()
                    .unwrap_or_else(|| json!("import-changed"));
                line.insert(
                    "importVerdict".into(),
                    line.get("verdict").cloned().unwrap_or(Value::Null),
                );
                line.insert("verdict".into(), verdict);
            }
        }
        (_, Some((kind, detail))) => {
            line.insert("verdict".into(), kind.into());
            line.insert(
                "reason".into(),
                detail.chars().take(600).collect::<String>().into(),
            );
        }
        _ => {
            line.insert("verdict".into(), "crash".into());
            line.insert("reason".into(), "the process printed no report".into());
        }
    }
    Value::Object(line)
}

/// Runs `files` through `args_of`, `jobs` at a time, until `deadline`;
/// the lines, in the order of `files` (those not reached left out).
fn run_stage(
    launcher: &Launcher,
    files: &[PathBuf],
    jobs: usize,
    deadline: Option<Instant>,
    stage: &str,
    args_of: &(dyn Fn(&Path) -> Vec<String> + Sync),
) -> Vec<Value> {
    let next = AtomicUsize::new(0);
    let lines: Mutex<Vec<(usize, Value)>> = Mutex::new(Vec::new());
    let started = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    if deadline.is_some_and(|d| Instant::now() >= d) {
                        return;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(file) = files.get(i) else {
                        return;
                    };
                    let line = classify(launcher, &args_of(file), file, stage);
                    let mut all = lines
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    all.push((i, line));
                    let n = all.len();
                    if n.is_multiple_of(250) {
                        eprintln!(
                            "abc: {stage}: {n} of {} in {:.0} s",
                            files.len(),
                            started.elapsed().as_secs_f64()
                        );
                    }
                }
            });
        }
    });
    let mut lines = lines
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    lines.sort_by_key(|(i, _)| *i);
    lines.into_iter().map(|(_, v)| v).collect()
}

fn verdict(line: &Value) -> &str {
    line.get("verdict").and_then(Value::as_str).unwrap_or("?")
}

fn id(line: &Value) -> &str {
    line.get("id").and_then(Value::as_str).unwrap_or("?")
}

fn reason(line: &Value) -> String {
    line.get("reason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// For the booleans, the reason of the first tool that was not ok, with
/// its tool and operation.
fn boolean_reason(line: &Value) -> String {
    let tools = line
        .get("booleans")
        .and_then(|b| b.get("tools"))
        .and_then(Value::as_array);
    for tool in tools.into_iter().flatten() {
        if tool.get("verdict").and_then(Value::as_str) != Some("ok") {
            let field = |k: &str| tool.get(k).and_then(Value::as_str).unwrap_or("");
            return format!("{} {}: {}", field("tool"), field("at"), field("reason"));
        }
    }
    reason(line)
}

/// Counts per verdict, in a fixed order.
fn counts(lines: &[Value]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for line in lines {
        *out.entry(verdict(line).to_string()).or_insert(0) += 1;
    }
    out
}

fn percent(n: usize, of: usize) -> String {
    if of == 0 {
        "0.0".to_string()
    } else {
        format!("{:.1}", 100.0 * n as f64 / of as f64)
    }
}

/// The top reasons among the lines of `wanted` verdicts, with up to three
/// example ids each.
fn top_reasons(
    lines: &[Value],
    wanted: &[&str],
    why: &dyn Fn(&Value) -> String,
) -> Vec<(String, usize, Vec<String>)> {
    let mut by: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    for line in lines.iter().filter(|l| wanted.contains(&verdict(l))) {
        let key = reason_key(&why(line));
        let entry = by.entry(key).or_insert((0, Vec::new()));
        entry.0 += 1;
        if entry.1.len() < 3 {
            entry.1.push(id(line).to_string());
        }
    }
    let mut top: Vec<(String, usize, Vec<String>)> =
        by.into_iter().map(|(k, (n, ids))| (k, n, ids)).collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top.truncate(TOP);
    top
}

/// FreeCAD reads up to `n` models stage 1 accepted (spread over them)
/// and gives their volumes; the lines of `freecad.jsonl`.
fn freecad(
    c: &Config,
    files: &[PathBuf],
    import: &[Value],
    deadline: Option<Instant>,
) -> Result<Vec<Value>, String> {
    let program = std::env::var("CRAFT3D_FREECAD")
        .map_err(|_| "--freecad needs CRAFT3D_FREECAD".to_string())?;
    let accepted: Vec<(&PathBuf, &Value)> = files
        .iter()
        .zip(import)
        .filter(|(_, l)| verdict(l) == "ok")
        .collect();
    if accepted.is_empty() || c.freecad == 0 {
        return Ok(Vec::new());
    }
    // N spread evenly over them (all of them when there are fewer).
    let n = c.freecad.min(accepted.len());
    let sample: Vec<&(&PathBuf, &Value)> = (0..n)
        .filter_map(|i| accepted.get(i.saturating_mul(accepted.len()) / n))
        .collect();
    let work = c.out.join("freecad-work");
    std::fs::create_dir_all(&work).map_err(|e| format!("cannot make {}: {e}", work.display()))?;
    let script = work.join("volumes.py");
    std::fs::write(&script, FREECAD_SCRIPT).map_err(|e| format!("cannot write the script: {e}"))?;
    let mut theirs: BTreeMap<String, Value> = BTreeMap::new();
    // In batches, so a FreeCAD that dies costs one batch.
    // A batch starts only when it can end before the deadline; the models
    // of the batches left out are not in the result.
    let mut reached = 0;
    for (b, batch) in sample.chunks(FREECAD_BATCH).enumerate() {
        let room = deadline.map(|d| d.saturating_duration_since(Instant::now()));
        if room.is_some_and(|r| r < Duration::from_secs(FREECAD_BATCH_SECONDS + 30)) {
            eprintln!("abc: FreeCAD: out of time after {reached} models");
            break;
        }
        reached += batch.len();
        let listing = work.join(format!("list-{b}.txt"));
        let report = work.join(format!("report-{b}.jsonl"));
        let text: String = batch
            .iter()
            .map(|(f, _)| format!("{}\n", f.display()))
            .collect();
        std::fs::write(&listing, text).map_err(|e| format!("cannot write the list: {e}"))?;
        let mut command = Command::new("timeout");
        let seconds = FREECAD_BATCH_SECONDS.to_string();
        command.args([
            "-k",
            "10",
            seconds.as_str(),
            "sh",
            "-c",
            "ulimit -v 4194304 && exec \"$@\"",
            "sh",
        ]);
        command.arg(&program);
        if program.ends_with(".AppImage") {
            command.arg("freecadcmd");
        }
        let _ = command
            .arg(&script)
            .env("CRAFT3D_FC_IN", &listing)
            .env("CRAFT3D_FC_OUT", &report)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let text = std::fs::read_to_string(&report).unwrap_or_default();
        for line in text.lines() {
            if let Ok(v) = serde_json::from_str::<Value>(line)
                && let Some(file) = v.get("file").and_then(Value::as_str)
            {
                theirs.insert(file.to_string(), v);
            }
        }
    }
    let mut out = Vec::new();
    for (file, ours) in sample.into_iter().take(reached) {
        let key = file.to_string_lossy().to_string();
        let mut line = Map::new();
        line.insert("id".into(), model_id(file).into());
        let our_volume = ours.get("volume").and_then(Value::as_f64);
        line.insert("ours".into(), our_volume.map_or(Value::Null, Value::from));
        match theirs.get(&key) {
            None => {
                line.insert("verdict".into(), "freecad-died".into());
            }
            Some(t) => {
                let their_volume = t.get("volume").and_then(Value::as_f64);
                line.insert(
                    "freecad".into(),
                    their_volume.map_or(Value::Null, Value::from),
                );
                for k in ["solids", "valid", "error"] {
                    if let Some(v) = t.get(k) {
                        line.insert(k.into(), v.clone());
                    }
                }
                let v = match (our_volume, their_volume) {
                    (Some(a), Some(b)) if b.abs() > 0.0 => {
                        let off = (a - b).abs() / b.abs();
                        line.insert("relative".into(), off.into());
                        if off <= 1e-3 { "agree" } else { "disagree" }
                    }
                    (Some(_), Some(_)) => "freecad-no-volume",
                    _ => "freecad-refused",
                };
                line.insert("verdict".into(), v.into());
            }
        }
        out.push(Value::Object(line));
    }
    let _ = std::fs::remove_dir_all(&work);
    Ok(out)
}

/// FreeCAD's side: the summed volume of the solids of each file.
const FREECAD_SCRIPT: &str = r#"import json, os
import Part
out = open(os.environ["CRAFT3D_FC_OUT"], "w")
for line in open(os.environ["CRAFT3D_FC_IN"]):
    path = line.strip()
    if not path:
        continue
    rec = {"file": path}
    try:
        shape = Part.Shape()
        shape.read(path)
        rec["solids"] = len(shape.Solids)
        rec["volume"] = sum(s.Volume for s in shape.Solids)
        rec["valid"] = shape.isValid()
    except Exception as e:
        rec["error"] = str(e)[:200]
    out.write(json.dumps(rec) + "\n")
    out.flush()
out.close()
"#;

fn write_lines(path: &Path, lines: &[Value]) -> Result<(), String> {
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// The scorecard, in Markdown: numbers and ids only.
fn scorecard(
    head: &Map<String, Value>,
    total: usize,
    import: &[Value],
    booleans: &[Value],
    fc: &[Value],
) -> String {
    let mut s = String::new();
    let field = |k: &str| {
        head.get(k)
            .map(|v| v.to_string().trim_matches('"').to_string())
            .unwrap_or_default()
    };
    let _ = writeln!(s, "# ABC scorecard: chunk {}\n", field("chunk"));
    let _ = writeln!(
        s,
        "Commit {}, {}; {} of the chunk's {total} STEP files run, {} at once, {} s and {} per model (docs/testing.md › Real-world models).\n",
        field("commit"),
        field("date"),
        import.len(),
        field("jobs"),
        field("timeout"),
        field("memory"),
    );
    let _ = writeln!(s, "## Stage 1: import, check, mesh, measure\n");
    let _ = writeln!(s, "| Verdict | Models | % |\n|---|---:|---:|");
    let c = counts(import);
    for (v, n) in &c {
        let _ = writeln!(s, "| {v} | {n} | {} |", percent(*n, import.len()));
    }
    let ready = import
        .iter()
        .filter(|l| l.get("booleanReady").and_then(Value::as_bool) == Some(true))
        .count();
    let _ = writeln!(s, "\nRead as valid solids (ready for stage 2): {ready}.\n");
    let mut at: BTreeMap<String, usize> = BTreeMap::new();
    for l in import
        .iter()
        .filter(|l| matches!(verdict(l), "refused" | "invalid"))
    {
        let step = l.get("at").and_then(Value::as_str).unwrap_or("?");
        *at.entry(format!("{} at {step}", verdict(l))).or_insert(0) += 1;
    }
    if !at.is_empty() {
        let _ = writeln!(s, "| Where | Models |\n|---|---:|");
        for (k, n) in &at {
            let _ = writeln!(s, "| {k} | {n} |");
        }
        s.push('\n');
    }
    let table = |s: &mut String, title: &str, rows: Vec<(String, usize, Vec<String>)>| {
        if rows.is_empty() {
            return;
        }
        let _ = writeln!(
            s,
            "### {title}\n\n| Reason | Models | Examples |\n|---|---:|---|"
        );
        for (k, n, ids) in rows {
            let k = k.replace('|', "\\|");
            let _ = writeln!(s, "| {k} | {n} | {} |", ids.join(", "));
        }
        s.push('\n');
    };
    table(
        &mut s,
        "Top refusal reasons",
        top_reasons(import, &["refused"], &reason),
    );
    table(
        &mut s,
        "Top problems of invalid imports",
        top_reasons(import, &["invalid"], &reason),
    );
    let died: Vec<&Value> = import
        .iter()
        .filter(|l| matches!(verdict(l), "crash" | "bug" | "timeout" | "oom"))
        .collect();
    if !died.is_empty() {
        let _ = writeln!(
            s,
            "### Crashes, timeouts, out of memory\n\n| Model | Verdict | Detail |\n|---|---|---|"
        );
        for l in died {
            let r: String = reason(l).replace('|', "\\|").chars().take(200).collect();
            let _ = writeln!(s, "| {} | {} | {r} |", id(l), verdict(l));
        }
        s.push('\n');
    }
    if !booleans.is_empty() {
        let _ = writeln!(
            s,
            "## Stage 2: booleans (drill, slab, corner block; union, subtract, intersect)\n"
        );
        let _ = writeln!(s, "| Verdict | Models | % |\n|---|---:|---:|");
        for (v, n) in &counts(booleans) {
            let _ = writeln!(s, "| {v} | {n} | {} |", percent(*n, booleans.len()));
        }
        let mut per_tool: BTreeMap<(String, String), usize> = BTreeMap::new();
        for l in booleans {
            let tools = l
                .get("booleans")
                .and_then(|b| b.get("tools"))
                .and_then(Value::as_array);
            for t in tools.into_iter().flatten() {
                let name = t
                    .get("tool")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string();
                let v = t
                    .get("verdict")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string();
                *per_tool.entry((name, v)).or_insert(0) += 1;
            }
        }
        let _ = writeln!(s, "\n| Tool | Verdict | Models |\n|---|---|---:|");
        for ((t, v), n) in &per_tool {
            let _ = writeln!(s, "| {t} | {v} | {n} |");
        }
        s.push('\n');
        table(
            &mut s,
            "Top boolean refusals",
            top_reasons(booleans, &["refused"], &boolean_reason),
        );
        table(
            &mut s,
            "Wrong booleans (identity or check)",
            top_reasons(booleans, &["invalid"], &boolean_reason),
        );
        let died: Vec<&Value> = booleans
            .iter()
            .filter(|l| matches!(verdict(l), "crash" | "bug" | "timeout" | "oom"))
            .collect();
        if !died.is_empty() {
            let _ = writeln!(
                s,
                "### Boolean crashes, bugs, timeouts, out of memory\n\n| Model | Verdict | Detail |\n|---|---|---|"
            );
            for l in died {
                let r: String = boolean_reason(l)
                    .replace('|', "\\|")
                    .chars()
                    .take(200)
                    .collect();
                let _ = writeln!(s, "| {} | {} | {r} |", id(l), verdict(l));
            }
            s.push('\n');
        }
    }
    if !fc.is_empty() {
        let _ = writeln!(
            s,
            "## Volumes against FreeCAD (a sample of stage 1's accepted models)\n"
        );
        let _ = writeln!(s, "| Verdict | Models |\n|---|---:|");
        for (v, n) in &counts(fc) {
            let _ = writeln!(s, "| {v} | {n} |");
        }
        let off: Vec<&Value> = fc.iter().filter(|l| verdict(l) == "disagree").collect();
        if !off.is_empty() {
            let _ = writeln!(
                s,
                "\n| Model | Ours | FreeCAD | Relative |\n|---|---:|---:|---:|"
            );
            for l in off {
                let n = |k: &str| {
                    l.get(k)
                        .and_then(Value::as_f64)
                        .map_or("-".to_string(), |v| format!("{v:.6e}"))
                };
                let _ = writeln!(
                    s,
                    "| {} | {} | {} | {} |",
                    id(l),
                    n("ours"),
                    n("freecad"),
                    n("relative")
                );
            }
        }
        s.push('\n');
    }
    s
}

/// The lines of a `.jsonl` file (none when it is missing).
fn read_lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// `--rescore DIR`: the scorecard of an earlier run made again from what it
/// left (after a change to how reasons are counted, say).
fn rescore(dir: &Path) -> Result<(), String> {
    let summary: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("summary.json"))
            .map_err(|e| format!("cannot read {}/summary.json: {e}", dir.display()))?,
    )
    .map_err(|e| format!("{}/summary.json: {e}", dir.display()))?;
    let head = summary.as_object().cloned().unwrap_or_default();
    let total = head
        .get("files")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0);
    let import = read_lines(&dir.join("import.jsonl"));
    let booleans = read_lines(&dir.join("booleans.jsonl"));
    let fc = read_lines(&dir.join("freecad.jsonl"));
    let card = scorecard(&head, total, &import, &booleans, &fc);
    std::fs::write(dir.join("scorecard.md"), &card)
        .map_err(|e| format!("cannot write the scorecard: {e}"))?;
    println!("{card}");
    Ok(())
}

/// The earlier of two deadlines.
fn earlier(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let c = parse(args)?;
    if let Some(dir) = &c.rescore {
        return rescore(dir);
    }
    let start = Instant::now();
    let end = c
        .total_minutes
        .map(|m| start + Duration::from_secs(m.saturating_mul(60)));
    let mut files = Vec::new();
    // Held until the task ends: no other run downloads, unpacks, reads or
    // removes this chunk meanwhile.
    let mut _lock = None;
    match &c.files {
        Some(dir) => step_files(dir, 0, &mut files),
        None => {
            std::fs::create_dir_all(&c.cache)
                .map_err(|e| format!("cannot make {}: {e}", c.cache.display()))?;
            let Some(mut lock) = lock_chunk(&c.cache, c.chunk)? else {
                return Err(format!(
                    "chunk {:04} is in use by another run (its lock in {} is held)",
                    c.chunk,
                    c.cache.display()
                ));
            };
            // Written, so its time says when the chunk was last used.
            {
                use std::io::Write as _;
                let _ = lock.set_len(0);
                let _ = writeln!(lock, "{} {}", std::process::id(), now());
            }
            _lock = Some(lock);
            evict(&c.cache, c.chunk, c.keep);
            let archive = download(&c)?;
            let dir = extract(&c, &archive)?;
            step_files(&dir, 0, &mut files);
        }
    }
    let total = files.len();
    if total == 0 {
        return Err("no STEP files found".to_string());
    }
    if !c.only.is_empty() {
        files.retain(|f| c.only.contains(&model_id(f)));
        if files.is_empty() {
            return Err(format!(
                "none of the models {} is in the chunk",
                c.only.join(", ")
            ));
        }
    }
    if let Some(limit) = c.limit {
        files.truncate(limit);
    }
    let bin = match &c.bin {
        Some(b) => b.clone(),
        None => crate::fuzz::build()?,
    };
    let capped = cap_for(&c.memory);
    let launcher = Launcher {
        bin: bin.clone(),
        timeout: c.timeout,
        memory: c.memory.clone(),
        capped,
    };
    std::fs::create_dir_all(&c.out).map_err(|e| format!("cannot make {}: {e}", c.out.display()))?;
    let scratch = c.out.join("scratch");
    let scratch_text = scratch.to_string_lossy().to_string();
    eprintln!(
        "abc: {} of {total} files, {} at once, {} s and {} each, into {}",
        files.len(),
        c.jobs,
        c.timeout,
        if capped == Cap::None {
            "no cap"
        } else {
            c.memory.as_str()
        },
        c.out.display()
    );
    let started = Instant::now();
    let deadline = earlier(
        c.minutes
            .map(|m| started + Duration::from_secs(m.saturating_mul(60))),
        end,
    );
    let args_of = |f: &Path| {
        vec![
            "step".to_string(),
            f.to_string_lossy().to_string(),
            "--scratch".to_string(),
            scratch_text.clone(),
        ]
    };
    let import = run_stage(&launcher, &files, c.jobs, deadline, "import", &args_of);
    let import_seconds = started.elapsed().as_secs_f64();
    write_lines(&c.out.join("import.jsonl"), &import)?;
    eprintln!(
        "abc: import: {} models in {import_seconds:.0} s: {:?}",
        import.len(),
        counts(&import)
    );

    let mut booleans = Vec::new();
    let mut boolean_seconds = 0.0;
    if c.booleans {
        let mut ready: Vec<PathBuf> = files
            .iter()
            .zip(&import)
            .filter(|(_, l)| l.get("booleanReady").and_then(Value::as_bool) == Some(true))
            .map(|(f, _)| f.clone())
            .collect();
        if let Some(limit) = c.boolean_limit {
            ready.truncate(limit);
        }
        let launcher = Launcher {
            bin,
            timeout: c.boolean_timeout,
            memory: c.memory.clone(),
            capped,
        };
        let started = Instant::now();
        let deadline = earlier(
            c.boolean_minutes
                .map(|m| started + Duration::from_secs(m.saturating_mul(60))),
            end,
        );
        let args_of = |f: &Path| {
            let mut a = args_of(f);
            a.push("--booleans".to_string());
            a
        };
        booleans = run_stage(&launcher, &ready, c.jobs, deadline, "booleans", &args_of);
        boolean_seconds = started.elapsed().as_secs_f64();
        write_lines(&c.out.join("booleans.jsonl"), &booleans)?;
        eprintln!(
            "abc: booleans: {} models in {boolean_seconds:.0} s: {:?}",
            booleans.len(),
            counts(&booleans)
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);

    let fc = if c.freecad > 0 {
        match freecad(&c, &files, &import, end) {
            Ok(lines) => lines,
            Err(e) => {
                eprintln!("abc: FreeCAD: {e}");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    if !fc.is_empty() {
        write_lines(&c.out.join("freecad.jsonl"), &fc)?;
    }

    let mut head = Map::new();
    head.insert("date".into(), now().into());
    head.insert("commit".into(), commit().into());
    head.insert(
        "chunk".into(),
        if c.files.is_some() {
            json!("local files")
        } else {
            json!(format!("{:04}", c.chunk))
        },
    );
    head.insert("jobs".into(), c.jobs.into());
    head.insert("timeout".into(), c.timeout.into());
    head.insert(
        "memory".into(),
        if capped == Cap::None {
            "no cap".into()
        } else {
            c.memory.clone().into()
        },
    );
    let card = scorecard(&head, total, &import, &booleans, &fc);
    std::fs::write(c.out.join("scorecard.md"), &card)
        .map_err(|e| format!("cannot write the scorecard: {e}"))?;
    let mut summary = head;
    summary.insert("files".into(), total.into());
    summary.insert("run".into(), import.len().into());
    summary.insert("import".into(), json!(counts(&import)));
    summary.insert("importSeconds".into(), import_seconds.into());
    summary.insert("booleans".into(), json!(counts(&booleans)));
    summary.insert("booleanSeconds".into(), boolean_seconds.into());
    summary.insert("freecad".into(), json!(counts(&fc)));
    let text = serde_json::to_string_pretty(&Value::Object(summary)).map_err(|e| e.to_string())?;
    std::fs::write(c.out.join("summary.json"), text)
        .map_err(|e| format!("cannot write the summary: {e}"))?;
    println!("{card}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_come_from_the_file_names() {
        let p = Path::new("/x/00000050/00000050_80d90bfdd2e74e709956122a_step_000.step");
        assert_eq!(model_id(p), "00000050");
        let p = Path::new("/x/00000050/00000050_80d90bfdd2e74e709956122a_step_002.step");
        assert_eq!(model_id(p), "00000050/2");
        assert_eq!(model_id(Path::new("block.step")), "block");
    }

    #[test]
    fn reasons_of_one_kind_count_together() {
        let a = "file.import: `00000050_80d9_step_000.step`, solid 0 (#123): unsupported surface: OFFSET_SURFACE of a B-spline (#77)";
        let b = "file.import: `00000051_aaaa_step_000.step`, solid 3 (#9 at a/b): unsupported surface: OFFSET_SURFACE of a B-spline (#12)";
        assert_eq!(reason_key(a), reason_key(b));
        assert_eq!(
            reason_key(a),
            "unsupported surface: OFFSET_SURFACE of a B-spline"
        );
        assert_eq!(
            reason_key("edge 12 is used 3 times"),
            "edge # is used # times"
        );
    }

    #[test]
    fn only_a_plain_https_url_on_the_archive_is_downloaded() {
        let name = "abc_0000_step_v00.7z";
        let good = "https://archive.nyu.edu/rest/bitstreams/88598/retrieve abc_0000_step_v00.7z\n";
        assert_eq!(
            chunk_url(good, name).unwrap(),
            "https://archive.nyu.edu/rest/bitstreams/88598/retrieve"
        );
        for bad in [
            "-o/etc/passwd abc_0000_step_v00.7z",
            "http://archive.nyu.edu/x abc_0000_step_v00.7z",
            "https://evil.example/x abc_0000_step_v00.7z",
            "https://archive.nyu.edu.evil.example/x abc_0000_step_v00.7z",
            "https://archive.nyu.edu/../x abc_0000_step_v00.7z",
            "https://archive.nyu.edu/x?a=$(id) abc_0000_step_v00.7z",
        ] {
            assert!(chunk_url(bad, name).is_err(), "{bad}");
        }
        assert!(chunk_url(good, "abc_0001_step_v00.7z").is_err());
    }

    #[test]
    fn an_md5_sum_is_required() {
        let name = "abc_0000_step_v00.7z";
        let text = "abc_0000_step_v00.7z: 695388be7a278c7798c8c8ae239772ac\n";
        assert_eq!(
            chunk_md5(text, name).unwrap(),
            "695388be7a278c7798c8c8ae239772ac"
        );
        assert!(chunk_md5(text, "abc_0001_step_v00.7z").is_err());
        assert!(chunk_md5("abc_0000_step_v00.7z: nope\n", name).is_err());
        assert!(chunk_md5("", name).is_err());
    }

    #[test]
    fn entries_that_leave_or_link_are_seen() {
        assert!(stays_inside("00000002/00000002_ab_step_000.step"));
        assert!(!stays_inside("../x.step"));
        assert!(!stays_inside("a/../../x.step"));
        assert!(!stays_inside("/etc/passwd"));
        assert!(!stays_inside(""));
        let listing = "Path = abc.7z\nType = 7z\n\n----------\nPath = 1/a.step\nAttributes = A_ -rw-r--r--\n\nPath = 1/l\nAttributes = A_ lrwxrwxrwx\n\nPath = 2/b.step\nSymbolic Link = /etc\n";
        let entries = listed_entries(listing);
        assert_eq!(
            entries,
            vec![
                ("1/a.step".to_string(), false),
                ("1/l".to_string(), true),
                ("2/b.step".to_string(), true)
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_are_not_followed() {
        let dir = std::env::temp_dir().join(format!("xtask-abc-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("m")).unwrap();
        std::fs::write(dir.join("m/a.step"), "x").unwrap();
        let outside = dir.with_extension("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("b.step"), "x").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("link")).unwrap();
        std::os::unix::fs::symlink(outside.join("b.step"), dir.join("m/c.step")).unwrap();
        let mut files = Vec::new();
        step_files(&dir, 0, &mut files);
        assert_eq!(files, vec![dir.join("m/a.step")]);
        let mut links = Vec::new();
        links_under(&dir, 0, &mut links);
        links.sort();
        assert_eq!(links, vec![dir.join("link"), dir.join("m/c.step")]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_held_lock_is_seen() {
        let dir = std::env::temp_dir().join(format!("xtask-abc-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = lock_chunk(&dir, 3).unwrap();
        assert!(first.is_some());
        assert!(lock_chunk(&dir, 3).unwrap().is_none());
        drop(first);
        assert!(lock_chunk(&dir, 3).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn options_read() {
        let args: Vec<String> = [
            "--chunk",
            "7",
            "--booleans",
            "--jobs",
            "99",
            "--limit",
            "10",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let c = parse(&args).unwrap();
        assert_eq!(c.chunk, 7);
        assert!(c.booleans);
        assert_eq!(c.jobs, MAX_JOBS);
        assert_eq!(c.limit, Some(10));
        assert!(parse(&["--chunk".to_string(), "100".to_string()]).is_err());
        assert!(parse(&["--chunk".to_string(), "99".to_string()]).is_ok());
        assert!(parse(&["--memory".to_string(), "lots".to_string()]).is_err());
    }
}
