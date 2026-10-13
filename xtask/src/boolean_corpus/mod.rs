//! `cargo xtask boolean-corpus [--check]`: the pinned corpus of boolean
//! cases with the answers of an independent kernel (docs/testing.md,
//! oracle 8).
//!
//! The cases are generated here from a seed ([`cases`]); FreeCAD, run as a
//! separate program, builds each pair from its own primitives and reports
//! what it measures for the common, the cut and the fuse
//! (`reference.py`); this module decides which of those answers can be
//! trusted and writes `crates/kernel/tests/fixtures/boolean_corpus.json`.
//! The kernel's test (`crates/kernel/tests/boolean_corpus.rs`) reads that
//! file and needs no FreeCAD.
//!
//! A reference that is wrong is worse than none, so an answer is only kept
//! as a reference when it survives every check there is without our
//! kernel: FreeCAD calls its results valid, the volumes of the three
//! results obey `vol(A ∪ B) = vol A + vol B − vol(A ∩ B)` and
//! `vol(A − B) = vol A − vol(A ∩ B)`, the common volume agrees with a
//! count of sampled points that are inside both primitives by their
//! defining inequalities, and with what the construction of the case says
//! (apart, one inside the other, the same solid). Everything else is
//! marked `unreliable`, with what happened.

mod cases;
mod geometry;

use cases::{Case, Expect, TOLERANCE};
use geometry::{Placed, Solid, V};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The seed of the pinned corpus.
const SEED: u64 = 2026_1013;
const SCRIPT: &str = include_str!("reference.py");
const FIXTURE: &str = "crates/kernel/tests/fixtures/boolean_corpus.json";
/// Seconds FreeCAD may spend on one pair before it is given up on.
const LIMIT: u64 = 120;

/// The volume identities must hold to this, relative to `vol A + vol B`,
/// for a reference to be called `exact`.
const EXACT: f64 = 1e-9;
/// Beyond this the reference is `unreliable`; in between it is
/// `approximate`, and the test compares with a bound made of the measured
/// error.
const APPROXIMATE: f64 = 1e-4;
/// Points sampled per pair for the independent estimate of the common
/// volume.
const SAMPLES: u32 = 20_000;
/// How far FreeCAD's area of a domed block may be from ours (the volume
/// and the centroid are held to 1e-9 like every operand's). FreeCAD
/// integrates the area of a spline face numerically and does not take it
/// to convergence: for the dome 2 × 1.5 × 2.5 it reports 23.678674443567235
/// where 24-, 32- and 64-point Gauss–Legendre rules all give
/// 23.67869696607736, 9.5e-7 off; its volume and centroid of the same
/// solid are exact (polynomial integrands, which its rule integrates
/// exactly). The run prints the largest deviation it saw.
const FREEFORM_AREA: f64 = 1e-5;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn number(x: f64) -> String {
    // The shortest text that reads back as the same f64.
    format!("{x:?}")
}

fn vector(v: V) -> String {
    format!("[{},{},{}]", number(v.0), number(v.1), number(v.2))
}

fn solid_json(p: &Placed) -> String {
    let shape = match p.solid {
        Solid::Block { size } | Solid::Dome { size, .. } => format!("\"size\":{}", vector(size)),
        Solid::Cylinder { radius, height } | Solid::Cone { radius, height } => {
            format!(
                "\"radius\":{},\"height\":{}",
                number(radius),
                number(height)
            )
        }
        Solid::Sphere { radius } => format!("\"radius\":{}", number(radius)),
        Solid::Torus { major, minor } => {
            format!("\"major\":{},\"minor\":{}", number(major), number(minor))
        }
    };
    format!(
        "{{\"kind\":\"{}\",{shape},\"origin\":{},\"z\":{},\"x\":{}}}",
        p.solid.kind().name(),
        vector(p.o),
        vector(p.r.z),
        vector(p.r.x)
    )
}

/// What FreeCAD is given: the pair, at scale 1.
fn input_line(case: &Case) -> String {
    format!(
        "{{\"id\":\"{}\",\"a\":{},\"b\":{}}}\n",
        case.id,
        solid_json(&case.a),
        solid_json(&case.b)
    )
}

/// What one run of FreeCAD over some cases gave: an answer per case, or
/// why there is none.
struct Answers {
    version: Option<String>,
    by_id: BTreeMap<String, Result<String, String>>,
}

fn run_freecad(
    freecad: &str,
    dir: &Path,
    worker: usize,
    cases: &[&Case],
) -> Result<Answers, String> {
    let script = dir.join("boolean_reference.py");
    let input = dir.join(format!("in-{worker}.jsonl"));
    let output = dir.join(format!("out-{worker}.tsv"));
    let mut answers = Answers {
        version: None,
        by_id: BTreeMap::new(),
    };
    let mut rest: Vec<&Case> = cases.to_vec();
    while !rest.is_empty() {
        let text: String = rest.iter().map(|c| input_line(c)).collect();
        std::fs::write(&input, text).map_err(|e| format!("cannot write {input:?}: {e}"))?;
        let _ = std::fs::remove_file(&output);
        let mut command = Command::new(freecad);
        if freecad.ends_with(".AppImage") {
            command.arg("freecadcmd");
        }
        let run = command
            .arg(&script)
            .env("CRAFT3D_FC_IN", &input)
            .env("CRAFT3D_FC_OUT", &output)
            .env("CRAFT3D_FC_LIMIT", LIMIT.to_string())
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot run {freecad}: {e}"))?;
        let text = std::fs::read_to_string(&output).map_err(|_| {
            format!(
                "FreeCAD wrote nothing ({}):\n{}\n{}",
                run.status,
                String::from_utf8_lossy(&run.stdout),
                String::from_utf8_lossy(&run.stderr)
            )
        })?;
        let mut started: Option<&str> = None;
        let mut done = false;
        for line in text.lines() {
            match line.split_once('\t') {
                Some(("#version", v)) => answers.version = Some(v.to_string()),
                Some(("#start", id)) => started = Some(id),
                Some((id, json)) => {
                    answers.by_id.insert(id.to_string(), Ok(json.to_string()));
                    started = None;
                }
                None => done |= line == "#done",
            }
        }
        if done {
            break;
        }
        // FreeCAD stopped in the middle: the pair it was on gets no
        // answer, the ones after it another run.
        let Some(stuck) = started else {
            return Err(format!(
                "FreeCAD stopped without saying where ({}):\n{}",
                run.status,
                String::from_utf8_lossy(&run.stderr)
            ));
        };
        answers.by_id.insert(
            stuck.to_string(),
            Err(format!(
                "FreeCAD crashed or did not finish within {LIMIT} s ({})",
                run.status
            )),
        );
        let at = rest.iter().position(|c| c.id == stuck).unwrap_or(0);
        rest = rest.split_off((at + 1).min(rest.len()));
    }
    Ok(answers)
}

/// The fraction of points of B's own box that are in both solids, times
/// the volume of that box: the common volume by the solids' defining
/// inequalities, and its standard deviation.
fn sampled_common(case: &Case) -> (f64, f64) {
    let (lo, hi) = case.b.solid.local_box();
    let size = hi - lo;
    let box_volume = size.0 * size.1 * size.2;
    // A fixed lattice of irrational steps (a Kronecker sequence): the same
    // points every time, spread evenly.
    let steps = V(
        0.819_172_513_396_164_4,
        0.671_043_606_703_789_2,
        0.549_700_477_901_970_2,
    );
    let mut hits = 0u32;
    for i in 1..=SAMPLES {
        let t = f64::from(i);
        let local = lo
            + V(
                size.0 * (t * steps.0).fract(),
                size.1 * (t * steps.1).fract(),
                size.2 * (t * steps.2).fract(),
            );
        if case.b.solid.contains(local) && case.a.solid.contains(case.a.local(case.b.world(local)))
        {
            hits += 1;
        }
    }
    let p = f64::from(hits) / f64::from(SAMPLES);
    let sigma = (p * (1.0 - p) / f64::from(SAMPLES)).sqrt();
    (p * box_volume, sigma * box_volume)
}

fn field(v: &Value, path: &[&str]) -> Option<f64> {
    let mut at = v;
    for key in path {
        at = at.get(key)?;
    }
    at.as_f64()
}

/// How far FreeCAD's answers for a case can be trusted.
struct Grade {
    /// `exact`, `approximate` or `unreliable`.
    verdict: &'static str,
    /// Why it is unreliable.
    why: String,
    /// By how much the volume identities fail, relative to `vol A + vol B`.
    volume_residual: Option<f64>,
    /// By how much `area(A ∪ B) + area(A ∩ B) = area A + area B` fails,
    /// relative to the right-hand side, where that identity holds.
    area_residual: Option<f64>,
}

/// Categories in which no face of A lies on a face of B, so that the
/// areas of the union and the intersection add up to those of A and B.
const NOTHING_COINCIDENT: [&str; 5] = ["general", "parallel", "coaxial", "inside", "apart"];

/// The grade of FreeCAD's answers for a case; an error when FreeCAD did
/// not build the operands we described, which is a bug in the mapping and
/// not a property of the case.
fn grade(case: &Case, reference: &Value) -> Result<Grade, String> {
    let unreliable = |why: String| {
        Ok(Grade {
            verdict: "unreliable",
            why,
            volume_residual: None,
            area_residual: None,
        })
    };
    if let Some(error) = reference.get("err").and_then(Value::as_str) {
        return unreliable(format!("FreeCAD could not build the operands: {error}"));
    }
    // The operands are known solids: their volume and area are known.
    for (name, placed) in [("a", &case.a), ("b", &case.b)] {
        let (Some(v), Some(a)) = (
            field(reference, &[name, "v"]),
            field(reference, &[name, "a"]),
        ) else {
            return Err(format!("{}: FreeCAD measured no operand {name}", case.id));
        };
        let (volume, area) = (placed.solid.volume(), placed.solid.area());
        let area_bound = if placed.solid.kind().domed() {
            FREEFORM_AREA
        } else {
            1e-9
        };
        if (v - volume).abs() > 1e-9 * volume || (a - area).abs() > area_bound * area {
            return Err(format!(
                "{}: FreeCAD's operand {name} has volume {v} and area {a}, \
                 the solid has {volume} and {area}",
                case.id
            ));
        }
    }
    for op in ["common", "cut", "fuse"] {
        let Some(result) = reference.get(op) else {
            return unreliable(format!("{op}: no answer"));
        };
        if let Some(error) = result.get("err").and_then(Value::as_str) {
            return unreliable(format!("{op}: FreeCAD raised {error}"));
        }
        if result.get("ok").and_then(Value::as_bool) != Some(true) {
            return unreliable(format!("{op}: FreeCAD's isValid() is false"));
        }
        if let Some(loose) = result.get("loose") {
            return unreliable(format!("{op}: {loose} faces that bound no solid"));
        }
    }
    let get = |path: &[&str]| field(reference, path).unwrap_or(f64::NAN);
    let (va, vb) = (get(&["a", "v"]), get(&["b", "v"]));
    let (vc, vd, vf) = (
        get(&["common", "v"]),
        get(&["cut", "v"]),
        get(&["fuse", "v"]),
    );
    let sum = va + vb;
    if ![vc, vd, vf].iter().all(|v| v.is_finite() && *v >= 0.0) {
        return unreliable("a volume that is negative or not a number".to_string());
    }
    let residual = (vf - (sum - vc)).abs().max((vd - (va - vc)).abs());
    if residual > APPROXIMATE * sum {
        return unreliable(format!(
            "the volumes of FreeCAD's three results disagree by {:.1e} of vol A + vol B",
            residual / sum
        ));
    }
    // The identities compare the booleans with each other; a count of
    // points compares the common with the solids themselves.
    let (sampled, sigma) = sampled_common(case);
    let slack = 6.0 * sigma + 3e-3 * va.min(vb);
    if (vc - sampled).abs() > slack {
        return unreliable(format!(
            "FreeCAD's common has volume {vc:.6}, sampled points say {sampled:.6} +- {sigma:.6}"
        ));
    }
    let close = |x: f64, y: f64| (x - y).abs() <= EXACT.max(4.0 * residual / sum) * sum;
    let says = |what: &str, got: f64, want: f64| {
        format!("the construction says {what}; FreeCAD's volume is {got:?}, not {want:?}")
    };
    let contradiction = match case.expect {
        Some(Expect::Apart | Expect::Disjoint) if !close(vc, 0.0) => {
            Some(says("no common volume", vc, 0.0))
        }
        Some(Expect::BInA) if !close(vc, vb) => Some(says("B is inside A", vc, vb)),
        Some(Expect::AInB) if !close(vc, va) => Some(says("A is inside B", vc, va)),
        Some(Expect::Same) if !close(vc, va) || !close(vd, 0.0) => {
            Some(says("A and B are the same", vc, va))
        }
        _ => None,
    };
    if let Some(why) = contradiction {
        return unreliable(why);
    }
    let areas = get(&["a", "a"]) + get(&["b", "a"]);
    if case.expect == Some(Expect::Apart) {
        // Nothing touches, so nothing may have been merged either.
        let area = get(&["fuse", "a"]);
        if (area - areas).abs() > 1e-9 * areas {
            return unreliable(format!(
                "the solids are apart, yet FreeCAD's fuse has area {area:?}, not {areas:?}"
            ));
        }
    }
    let area_residual = NOTHING_COINCIDENT
        .contains(&case.category)
        .then(|| (get(&["fuse", "a"]) + get(&["common", "a"]) - areas).abs() / areas);
    if let Some(off) = area_residual
        && (off.is_nan() || off > APPROXIMATE)
    {
        return unreliable(format!(
            "the areas of FreeCAD's fuse and common disagree with those of A and B by {off:.1e}"
        ));
    }
    let exact = residual <= EXACT * sum && area_residual.is_none_or(|off| off <= EXACT);
    Ok(Grade {
        verdict: if exact { "exact" } else { "approximate" },
        why: String::new(),
        volume_residual: Some(residual / sum),
        area_residual,
    })
}

/// The largest relative difference between FreeCAD's area of a domed
/// operand and ours, and the case it was seen on.
fn freeform_area_deviation(
    cases: &[Case],
    answers: &BTreeMap<String, Result<String, String>>,
) -> Option<(f64, String)> {
    let mut worst: Option<(f64, String)> = None;
    for case in cases {
        let Some(Ok(raw)) = answers.get(&case.id) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        for (name, placed) in [("a", &case.a), ("b", &case.b)] {
            if !placed.solid.kind().domed() {
                continue;
            }
            let Some(a) = field(&value, &[name, "a"]) else {
                continue;
            };
            let area = placed.solid.area();
            let off = ((a - area) / area).abs();
            if worst.as_ref().is_none_or(|(w, _)| off > *w) {
                worst = Some((off, case.id.clone()));
            }
        }
    }
    worst
}

fn escape(text: &str) -> String {
    Value::String(text.to_string()).to_string()
}

fn case_line(case: &Case, answer: &Result<String, String>) -> Result<String, String> {
    let mut line = format!(
        "{{\"id\":\"{}\",\"category\":\"{}\",\"variant\":\"{}\",\"pair\":\"{}\",\"scale\":{}",
        case.id,
        case.category,
        case.variant,
        case.id.split('/').nth(1).unwrap_or(""),
        number(case.scale)
    );
    if let Some(gap) = case.gap {
        let _ = write!(line, ",\"gap\":{}", number(gap));
    }
    if let Some(snap) = &case.snap {
        let _ = write!(line, ",\"snap\":\"{snap}\"");
    }
    if let Some(expect) = case.expect {
        let _ = write!(line, ",\"expect\":\"{}\"", expect.name());
    }
    let _ = write!(
        line,
        ",\"a\":{},\"b\":{}",
        solid_json(&case.a),
        solid_json(&case.b)
    );
    let failed = |why: String| Grade {
        verdict: "unreliable",
        why,
        volume_residual: None,
        area_residual: None,
    };
    let (grade, raw) = match answer {
        Err(why) => (failed(why.clone()), None),
        Ok(raw) => match serde_json::from_str::<Value>(raw) {
            Err(e) => (failed(format!("FreeCAD's answer is not JSON: {e}")), None),
            Ok(value) => (grade(case, &value)?, Some(raw)),
        },
    };
    let _ = write!(line, ",\"reference\":\"{}\"", grade.verdict);
    if !grade.why.is_empty() {
        let _ = write!(line, ",\"why\":{}", escape(&grade.why));
    }
    // What FreeCAD's own answers say about their accuracy, relative.
    if let Some(off) = grade.volume_residual {
        let _ = write!(line, ",\"volume_residual\":{off:.2e}");
    }
    if let Some(off) = grade.area_residual {
        let _ = write!(line, ",\"area_residual\":{off:.2e}");
    }
    // FreeCAD's numbers as it printed them: nothing is rounded on the way.
    if let Some(raw) = raw {
        let _ = write!(line, ",\"freecad\":{raw}");
    }
    line.push('}');
    Ok(line)
}

fn today() -> String {
    // Days since 1970-01-01 to a civil date (the usual era arithmetic).
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = seconds / 86_400 + 719_468;
    let era = days / 146_097;
    let doe = days % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

struct Options {
    check: bool,
    seed: u64,
    jobs: usize,
    only: Option<String>,
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        check: false,
        seed: SEED,
        jobs: std::thread::available_parallelism().map_or(4, usize::from),
        only: None,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .ok_or_else(|| format!("{name} needs a value"))
                .cloned()
        };
        match arg.as_str() {
            "--check" => options.check = true,
            "--seed" => {
                options.seed = value("--seed")?
                    .parse()
                    .map_err(|e| format!("--seed: {e}"))?;
            }
            "--jobs" => {
                options.jobs = value("--jobs")?
                    .parse::<usize>()
                    .map_err(|e| format!("--jobs: {e}"))?
                    .max(1);
            }
            "--only" => options.only = Some(value("--only")?),
            other => {
                return Err(format!(
                    "unknown option `{other}`; options: --check, --seed N, --jobs N, \
                     --only <part of a case id> (prints, writes nothing)"
                ));
            }
        }
    }
    Ok(options)
}

/// The header and the case lines of a corpus file.
fn split(text: &str) -> (Option<Value>, Vec<&str>) {
    let mut lines = text.lines();
    let header = lines
        .next()
        .and_then(|l| l.strip_prefix("{\"header\":"))
        .and_then(|l| l.strip_suffix(','))
        .and_then(|l| serde_json::from_str(l).ok());
    let cases = lines
        .filter(|l| l.starts_with("{\"id\""))
        .map(|l| l.strip_suffix(',').unwrap_or(l))
        .collect();
    (header, cases)
}

fn compare(pinned: &str, fresh: &str) -> Result<(), String> {
    let (old_header, old) = split(pinned);
    let (new_header, new) = split(fresh);
    let version = |h: &Option<Value>| {
        h.as_ref()
            .and_then(|h| h.get("freecad"))
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string()
    };
    let mut differing = Vec::new();
    for (i, line) in new.iter().enumerate() {
        if old.get(i) != Some(line) {
            let id = line.split('"').nth(3).unwrap_or("?");
            differing.push(id.to_string());
        }
    }
    if differing.is_empty() && old.len() == new.len() {
        eprintln!(
            "boolean corpus: {} cases, identical to the pinned file",
            new.len()
        );
        return Ok(());
    }
    let mut message = format!(
        "the pinned corpus has {} cases, the regenerated one {}; {} differ",
        old.len(),
        new.len(),
        differing.len()
    );
    for id in differing.iter().take(10) {
        let _ = write!(message, "\n  {id}");
    }
    if version(&old_header) != version(&new_header) {
        let _ = write!(
            message,
            "\nnote: pinned with `{}`, regenerated with `{}`; another FreeCAD measures \
             slightly other numbers",
            version(&old_header),
            version(&new_header)
        );
    }
    Err(message)
}

pub fn run(args: &[String]) -> Result<(), String> {
    let options = options(args)?;
    let freecad = std::env::var("CRAFT3D_FREECAD").map_err(|_| {
        "set CRAFT3D_FREECAD to a FreeCAD AppImage or FreeCADCmd executable".to_string()
    })?;
    let mut cases = cases::cases(options.seed);
    if let Some(only) = &options.only {
        cases.retain(|c| c.id.contains(only.as_str()));
    }
    let dir = root().join("target/tmp/boolean-corpus");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {dir:?}: {e}"))?;
    // Not `freecad.py`: that name shadows FreeCAD's own module.
    std::fs::write(dir.join("boolean_reference.py"), SCRIPT)
        .map_err(|e| format!("cannot write the script: {e}"))?;

    eprintln!(
        "boolean corpus: {} cases, seed {}, {} FreeCAD processes",
        cases.len(),
        options.seed,
        options.jobs
    );
    // Dealt out in turn, so that every process gets some of the slow ones.
    let mut shares: Vec<Vec<&Case>> = vec![Vec::new(); options.jobs];
    for (i, case) in cases.iter().enumerate() {
        if let Some(share) = shares.get_mut(i % options.jobs) {
            share.push(case);
        }
    }
    let results: Vec<Result<Answers, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = shares
            .iter()
            .enumerate()
            .map(|(worker, share)| {
                let (freecad, dir) = (&freecad, &dir);
                scope.spawn(move || run_freecad(freecad, dir, worker, share))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err("a worker thread panicked".to_string()))
            })
            .collect()
    });
    let mut version = None;
    let mut answers = BTreeMap::new();
    for result in results {
        let mut result = result?;
        version = version.or(result.version.take());
        answers.append(&mut result.by_id);
    }
    let version = version.unwrap_or_else(|| "unknown".to_string());

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut lines = Vec::new();
    let missing = Err("FreeCAD gave no answer".to_string());
    for case in &cases {
        let line = case_line(case, answers.get(&case.id).unwrap_or(&missing))?;
        for verdict in ["exact", "approximate", "unreliable"] {
            if line.contains(&format!("\"reference\":\"{verdict}\"")) {
                *counts.entry(verdict).or_default() += 1;
            }
        }
        lines.push(line);
    }
    eprintln!("boolean corpus: references {counts:?}");
    if let Some((off, id)) = freeform_area_deviation(&cases, &answers) {
        eprintln!(
            "boolean corpus: FreeCAD's area of a domed block is at most {off:.1e} off ours \
             (on {id}; allowed {FREEFORM_AREA:e})"
        );
    }
    if options.only.is_some() {
        for line in &lines {
            println!("{line}");
        }
        return Ok(());
    }

    let header = serde_json::json!({
        "corpus": "3DCraft boolean cases with reference answers",
        "format": 1,
        "command": "cargo xtask boolean-corpus",
        "seed": options.seed,
        "freecad": version,
        "date": today(),
        "cases": cases.len(),
        "tolerance": TOLERANCE,
        "note": "Lengths are at scale 1; a case's `scale` multiplies every length, on \
                 both sides. `freecad` holds FreeCAD's measurements at scale 1: v volume, \
                 a area, c centre of mass, bb bounding box, n [solids, shells], ok \
                 isValid(), t largest tolerance, spl an approximated (spline) edge, len \
                 total edge length. See xtask/src/boolean_corpus and docs/testing.md.",
    });
    let mut text = format!("{{\"header\":{header},\n\"cases\":[\n");
    text.push_str(&lines.join(",\n"));
    text.push_str("\n]}\n");

    let fixture = root().join(FIXTURE);
    if options.check {
        let fresh = dir.join("boolean_corpus.json");
        std::fs::write(&fresh, &text).map_err(|e| format!("cannot write {fresh:?}: {e}"))?;
        let pinned = std::fs::read_to_string(&fixture)
            .map_err(|e| format!("cannot read {fixture:?}: {e}"))?;
        compare(&pinned, &text).map_err(|m| format!("{m}\nregenerated file: {}", fresh.display()))
    } else {
        if let Some(parent) = fixture.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {parent:?}: {e}"))?;
        }
        std::fs::write(&fixture, &text).map_err(|e| format!("cannot write {fixture:?}: {e}"))?;
        eprintln!("boolean corpus: wrote {} ({} bytes)", FIXTURE, text.len());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geometry::{KINDS, Kind};

    #[test]
    fn the_same_seed_gives_the_same_cases() {
        let text = |seed| -> Vec<String> { cases::cases(seed).iter().map(input_line).collect() };
        let first = text(SEED);
        assert_eq!(first, text(SEED));
        assert_ne!(first, text(SEED + 1));
        let mut ids: Vec<String> = cases::cases(SEED).into_iter().map(|c| c.id).collect();
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count, "ids are unique");
        assert!(count > 2000);
    }

    /// The closed forms and the point-in-solid tests describe the same
    /// solids: the two things a reference is checked against agree.
    #[test]
    fn sampled_volumes_match_the_closed_forms() {
        for case in cases::cases(SEED)
            .iter()
            .filter(|c| c.category == "general")
        {
            // B against itself: the common is B.
            let twice = Case {
                a: case.b,
                ..case.clone()
            };
            let (sampled, sigma) = sampled_common(&twice);
            let volume = case.b.solid.volume();
            assert!(
                (sampled - volume).abs() <= 6.0 * sigma + 3e-3 * volume,
                "{}: sampled {sampled}, closed form {volume}",
                case.id
            );
        }
    }

    /// What a construction claims holds for the solids' own definitions,
    /// before any kernel is asked.
    #[test]
    fn constructions_are_what_they_say() {
        let mut checked = 0;
        for case in &cases::cases(SEED) {
            let Some(expect) = case.expect else { continue };
            let (sampled, sigma) = sampled_common(case);
            let (va, vb) = (case.a.solid.volume(), case.b.solid.volume());
            let want = match expect {
                Expect::Apart | Expect::Disjoint => 0.0,
                Expect::BInA => vb,
                Expect::AInB | Expect::Same => va,
            };
            assert!(
                (sampled - want).abs() <= 6.0 * sigma + 3e-3 * va.min(vb),
                "{} ({}): sampled {sampled}, the construction says {want}",
                case.id,
                expect.name()
            );
            checked += 1;
        }
        assert!(checked > 800, "{checked}");
    }

    #[test]
    fn every_pair_is_in_every_category_that_fits() {
        let all = cases::cases(SEED);
        let count = |category: &str, a: Kind, b: Kind| {
            let pair = format!("{category}/{}-{}/", a.name(), b.name());
            all.iter().filter(|c| c.id.starts_with(&pair)).count()
        };
        for (i, &a) in KINDS.iter().enumerate() {
            for &b in KINDS.iter().skip(i) {
                for category in ["general", "parallel", "coaxial", "inside", "apart", "near"] {
                    assert!(count(category, a, b) >= 12, "{category} {a:?} {b:?}");
                }
                let flat = |k: Kind| !matches!(k, Kind::Sphere | Kind::Torus);
                assert_eq!(count("flush", a, b) > 0, (flat(a) && flat(b)) || a == b);
                assert_eq!(
                    count("tangent", a, b) > 0,
                    (a, b) != (Kind::Block, Kind::Block)
                );
                assert_eq!(count("contact", a, b) > 0, flat(a) || flat(b));
            }
        }
    }

    #[test]
    fn the_date_is_a_date() {
        let date = today();
        assert_eq!(date.len(), 10, "{date}");
        assert!(date.as_str() >= "2026-01-01", "{date}");
    }

    #[test]
    fn unknown_options_are_refused() {
        assert!(options(&["--chekc".to_string()]).is_err());
        assert!(options(&["--seed".to_string()]).is_err());
        assert!(options(&["--check".to_string()]).is_ok_and(|o| o.check));
    }
}
