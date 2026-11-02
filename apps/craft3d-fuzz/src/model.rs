//! One real-world STEP model held to what 3DCraft must do with it
//! (docs/testing.md › Real-world models: the ABC dataset): import it,
//! check it, mesh it, measure it, and, for a model read as a valid solid,
//! a few booleans with tools sized to the part, each held to the volume
//! identities and to `document.check`.
//!
//! `cargo xtask abc` runs this once per model, each in its own process
//! with a time limit and a memory cap, and makes the scorecard; a process
//! that dies, hangs or runs out of memory is classified there.
//!
//! The verdicts of a stage: `ok`; `refused` (a command said no, with its
//! reason: not a bug, the scorecard's content); `invalid` (a command said
//! yes and left a body `document.check` finds fault with, or a boolean
//! whose volumes break an identity); `crash` (a command panicked: the
//! engine catches it and says "internal error").

use crate::runner::Operands;
use craft3d_engine::{Session, Workspace};
use serde_json::{Map, Value, json};
use std::path::Path;
use std::time::Instant;

/// The mark of a caught panic in an engine error (`Session::execute`).
const INTERNAL: &str = "internal error (a bug in 3DCraft";

/// Longest reason kept, in characters.
const KEPT: usize = 2000;

/// The document's tolerance (`file.new`'s default), in mm.
const TOLERANCE: f64 = 1e-6;

/// What a model's run is asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelOptions {
    /// Also run the boolean stage (on a model read as a valid solid).
    pub booleans: bool,
    /// The deep `document.check` (self-intersection on a fine mesh)
    /// after the import; the booleans always get the shallow one.
    pub deep: bool,
}

fn cut(text: &str) -> String {
    text.chars().take(KEPT).collect()
}

fn ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The verdict of one step: `Ok(result)`, or the verdict and the reason.
fn step(
    session: &mut Session,
    command: &str,
    params: &Value,
) -> Result<Value, (&'static str, String)> {
    match session.execute(command, params) {
        Ok(result) => {
            if result.get("status").and_then(Value::as_str) == Some("failed") {
                let reason = result
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("failed without a reason");
                return Err(("refused", cut(reason)));
            }
            Ok(result)
        }
        Err(e) => {
            let message = e.message();
            let verdict = if message.contains(INTERNAL) {
                "crash"
            } else {
                "refused"
            };
            Err((verdict, cut(message)))
        }
    }
}

/// The first problem `document.check` lists, as `(count, text)`.
fn first_problem(check: &Value) -> Option<(usize, String)> {
    let problems = check.get("problems").and_then(Value::as_array)?;
    let first = problems.first()?;
    let text = first
        .get("problem")
        .and_then(Value::as_str)
        .unwrap_or("a problem without a text");
    Some((problems.len(), cut(text)))
}

/// The run of one model: one line of JSON.
pub struct ModelRun {
    session: Session,
    out: Map<String, Value>,
    options: ModelOptions,
    scratch: std::path::PathBuf,
    /// The names the file gave its bodies, longest first: taken out of
    /// every reason, as `<body>` (no model data in what is kept).
    names: Vec<String>,
}

/// The solid the booleans cut: its feature, box, volume, area and the
/// integrator's bound on its volume.
#[derive(Clone, Debug)]
struct Target {
    feature: String,
    bbox: [[f64; 3]; 2],
    volume: f64,
    area: f64,
    bound: f64,
}

/// The verdict of a step that went wrong: a crash stays a crash, anything
/// else is a refusal.
fn refused_or_crash(verdict: &str) -> &'static str {
    if verdict == "crash" {
        "crash"
    } else {
        "refused"
    }
}

impl ModelRun {
    /// `text` with the bodies' names taken out.
    fn scrub(&self, text: &str) -> String {
        let mut out = text.to_string();
        for name in &self.names {
            out = out.replace(name.as_str(), "<body>");
        }
        out
    }

    fn set(&mut self, key: &str, value: impl Into<Value>) {
        self.out.insert(key.to_string(), value.into());
    }

    fn stop(&mut self, at: &str, verdict: &str, reason: &str) {
        let reason = cut(&self.scrub(reason));
        self.set("verdict", verdict);
        self.set("at", at);
        self.set("reason", reason);
    }

    /// Stage 1: import, check, mesh, measure. The largest solid when the
    /// model was read as valid solids.
    fn import_stage(&mut self, file: &Path) -> Option<Target> {
        let path = file.to_string_lossy().to_string();
        let mut times = Map::new();
        let started = Instant::now();
        let imported = step(&mut self.session, "file.import", &json!({ "path": path }));
        times.insert("import".into(), ms(started).into());
        self.set("ms", Value::Object(times.clone()));
        let imported = match imported {
            Ok(r) => r,
            Err((verdict, reason)) => {
                self.stop("import", verdict, &reason);
                return None;
            }
        };
        if let Some(sha) = imported.get("sha256") {
            self.set("sha256", sha.clone());
        }
        let mut names: Vec<String> = imported
            .get("bodies")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|b| b.get("name").and_then(Value::as_str))
            // A name of one or two characters would take out the same
            // letters or digits anywhere in a reason.
            .filter(|n| n.chars().count() >= 3)
            .map(str::to_string)
            .collect();
        names.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        names.dedup();
        self.names = names;
        let reports: Vec<Value> = imported
            .get("bodies")
            .and_then(Value::as_array)
            .map(|b| b.iter().filter_map(|x| x.get("import").cloned()).collect())
            .unwrap_or_default();
        if let Some(unit) = reports.first().and_then(|r| r.get("fileUnit")) {
            self.set("fileUnit", unit.clone());
        }
        // The loosest entity any solid recorded, and how many edges are
        // loose, so that two runs can be held to each other per model.
        let largest = reports
            .iter()
            .filter_map(|r| r.get("largestTolerance").and_then(Value::as_f64))
            .fold(0.0, f64::max);
        let loose: u64 = reports
            .iter()
            .filter_map(|r| r.get("looseEdges").and_then(Value::as_u64))
            .sum();
        self.set("largestTolerance", largest);
        self.set("looseEdges", loose);
        // A feature the import added that failed when regenerated.
        if let Some(failed) = imported
            .get("bodies")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|b| b.get("status").and_then(Value::as_str) == Some("failed"))
        {
            let reason = failed
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("an import feature failed");
            self.stop("import", "refused", reason);
            return None;
        }

        let started = Instant::now();
        let check = step(
            &mut self.session,
            "document.check",
            &json!({ "deep": self.options.deep }),
        );
        times.insert("check".into(), ms(started).into());
        self.set("ms", Value::Object(times.clone()));
        match check {
            // The check could not run: not a fault it found.
            Err((verdict, reason)) => {
                self.stop("check", refused_or_crash(verdict), &reason);
                return None;
            }
            Ok(check) => {
                if let Some((count, text)) = first_problem(&check) {
                    self.set("problems", count);
                    self.stop("check", "invalid", &text);
                    return None;
                }
            }
        }

        let inspect = match step(
            &mut self.session,
            "document.inspect",
            &json!({ "measures": true }),
        ) {
            Ok(r) => r,
            Err((verdict, reason)) => {
                self.stop("inspect", verdict, &reason);
                return None;
            }
        };
        let bodies: Vec<Value> = inspect
            .get("bodies")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let count = |kind: &str| {
            bodies
                .iter()
                .filter(|b| b.get("kind").and_then(Value::as_str) == Some(kind))
                .count()
        };
        let sum = |key: &str| {
            bodies
                .iter()
                .filter_map(|b| b.get(key).and_then(Value::as_u64))
                .sum::<u64>()
        };
        self.set("bodies", bodies.len());
        self.set("solids", count("solid"));
        self.set("sheets", count("sheet"));
        self.set("faces", sum("faces"));
        self.set("edges", sum("edges"));

        let started = Instant::now();
        let _ = std::fs::create_dir_all(&self.scratch);
        let stl = self.scratch.join(format!("abc-{}.stl", std::process::id()));
        let exported = step(
            &mut self.session,
            "file.export",
            &json!({ "path": stl.to_string_lossy(), "format": "stl" }),
        );
        let _ = std::fs::remove_file(&stl);
        times.insert("mesh".into(), ms(started).into());
        self.set("ms", Value::Object(times.clone()));
        match exported {
            Err((verdict, reason)) => {
                self.stop("mesh", verdict, &reason);
                return None;
            }
            Ok(mut result) => {
                if let Some(map) = result.as_object_mut() {
                    map.remove("path");
                }
                self.set("mesh", result);
            }
        }

        let started = Instant::now();
        let measured = step(
            &mut self.session,
            "document.measure",
            &json!({ "bound": true }),
        );
        times.insert("measure".into(), ms(started).into());
        self.set("ms", Value::Object(times));
        let measured = match measured {
            Ok(r) => r,
            Err((verdict, reason)) => {
                self.stop("measure", verdict, &reason);
                return None;
            }
        };
        for key in ["volume", "area", "volumeBound"] {
            if let Some(v) = measured.get(key) {
                self.set(key, v.clone());
            }
        }
        self.set("verdict", "ok");

        // The largest solid, for the booleans.
        let measures: Vec<Value> = measured
            .get("bodies")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let solid = bodies
            .iter()
            .filter(|b| b.get("kind").and_then(Value::as_str) == Some("solid"))
            .filter_map(|b| {
                let feature = b.get("feature")?.as_str()?.to_string();
                let m = measures
                    .iter()
                    .find(|m| m.get("feature").and_then(Value::as_str) == Some(feature.as_str()))?;
                let volume = m.get("volume")?.as_f64()?;
                let area = m.get("area")?.as_f64()?;
                let bound = m.get("volumeBound")?.as_f64()?;
                let bbox = b.get("bbox")?.as_array()?;
                let corner = |i: usize| -> Option<[f64; 3]> {
                    let p = bbox.get(i)?.as_array()?;
                    Some([
                        p.first()?.as_f64()?,
                        p.get(1)?.as_f64()?,
                        p.get(2)?.as_f64()?,
                    ])
                };
                let bbox = [corner(0)?, corner(1)?];
                let fine =
                    volume.is_finite() && volume > 0.0 && area.is_finite() && bound.is_finite();
                fine.then_some(Target {
                    feature,
                    bbox,
                    volume,
                    area,
                    bound,
                })
            })
            .max_by(|a, b| a.volume.total_cmp(&b.volume));
        self.set("booleanReady", solid.is_some());
        solid
    }

    /// Stage 2: three tools sized to the part, each with union, subtract
    /// and intersect, held to the identities and to `document.check`.
    fn boolean_stage(&mut self, target: &Target) {
        let [lo, hi] = target.bbox;
        let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let centre = [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ];
        let largest = size.iter().copied().fold(0.0_f64, f64::max);
        if !(largest.is_finite() && largest > 0.0) {
            self.set(
                "booleans",
                json!({"verdict": "refused", "reason": "the part has no size"}),
            );
            return;
        }
        // No extent thinner than a hundredth of the largest, so a flat
        // part still gets tools of a size.
        let e = size.map(|s| s.max(largest * 0.01));
        let m = largest * 0.05;
        // Off the exact centre by an odd fraction, so the tools are not
        // aligned with the part's symmetry planes by chance.
        let off = [0.0137 * e[0], 0.0071 * e[1], 0.0093 * e[2]];
        let radius = 0.1 * e[0].min(e[1]);
        let tools = [
            (
                "drill",
                "feature.cylinder",
                json!({
                    "radius": radius,
                    "height": e[2] + 2.0 * m,
                    "position": [centre[0] + off[0], centre[1] + off[1], lo[2] - m],
                    "name": "__tool",
                }),
            ),
            (
                "slab",
                "feature.block",
                json!({
                    "size": [e[0] + 2.0 * m, e[1] + 2.0 * m, 0.2 * e[2]],
                    "position": [lo[0] - m, lo[1] - m, centre[2] + off[2] - 0.1 * e[2]],
                    "name": "__tool",
                }),
            ),
            (
                "corner",
                "feature.block",
                json!({
                    "size": [0.3 * e[0], 0.3 * e[1], 0.3 * e[2]],
                    "position": [hi[0] - 0.15 * e[0], hi[1] - 0.15 * e[1], hi[2] - 0.15 * e[2]],
                    "name": "__tool",
                }),
            ),
        ];
        let mut results = Vec::new();
        let mut worst = "ok";
        let rank = |v: &str| match v {
            "crash" => 3,
            "invalid" => 2,
            "refused" => 1,
            _ => 0,
        };
        for (label, command, params) in tools {
            let mut entry = Map::new();
            entry.insert("tool".into(), label.into());
            let verdict = self.one_tool(target, command, &params, &mut entry);
            if let Some(Value::String(reason)) = entry.get("reason") {
                let scrubbed = self.scrub(reason);
                entry.insert("reason".into(), scrubbed.into());
            }
            entry.insert("verdict".into(), verdict.into());
            if rank(verdict) > rank(worst) {
                worst = verdict;
            }
            results.push(Value::Object(entry));
        }
        self.set("booleans", json!({"verdict": worst, "tools": results}));
    }

    /// One tool: its three booleans on copies of the session. The verdict;
    /// what it found goes into `entry`.
    fn one_tool(
        &mut self,
        a: &Target,
        command: &str,
        params: &Value,
        entry: &mut Map<String, Value>,
    ) -> &'static str {
        let target = a.feature.as_str();
        let mut base = self.session.clone();
        let started = Instant::now();
        if let Err((verdict, reason)) = step(&mut base, command, params) {
            entry.insert("at".into(), command.into());
            entry.insert("reason".into(), reason.into());
            return if verdict == "crash" {
                "crash"
            } else {
                "refused"
            };
        }
        // The tool's measure: one that fails is a refusal (or a crash) of
        // the measure, never a broken identity.
        let tool_measure = step(
            &mut base,
            "document.measure",
            &json!({"feature": "__tool", "bound": true}),
        );
        let tool_measured = match tool_measure {
            Ok(m) => measured(&m),
            Err((verdict, reason)) => Err((refused_or_crash(verdict), reason)),
        };
        let (vb, sb, bound_b) = match tool_measured {
            Ok(v) => v,
            Err((verdict, reason)) => {
                entry.insert("at".into(), "tool measure".into());
                entry.insert("reason".into(), reason.into());
                return verdict;
            }
        };
        let mut volumes = Map::new();
        // Every measured volume's bound: A's (from stage 1), B's and the
        // three results'.
        let mut bounds = a.bound + bound_b;
        let mut got = [0.0_f64; 3];
        for (k, op) in ["union", "subtract", "intersect"].into_iter().enumerate() {
            let mut side = base.clone();
            let p = json!({"op": op, "target": target, "tool": "__tool", "name": "__result"});
            if let Err((verdict, reason)) = step(&mut side, "feature.boolean", &p) {
                if reason.contains("the result is empty") {
                    volumes.insert(op.into(), 0.0.into());
                    continue;
                }
                entry.insert("at".into(), op.into());
                entry.insert("reason".into(), reason.into());
                return if verdict == "crash" {
                    "crash"
                } else {
                    "refused"
                };
            }
            let check = step(&mut side, "document.check", &json!({"feature": "__result"}));
            match check {
                // The check could not run: not a fault it found.
                Err((verdict, reason)) => {
                    entry.insert("at".into(), format!("{op} check").into());
                    entry.insert("reason".into(), reason.into());
                    return refused_or_crash(verdict);
                }
                Ok(c) => {
                    if let Some((_, text)) = first_problem(&c) {
                        entry.insert("at".into(), format!("{op} check").into());
                        entry.insert("reason".into(), text.into());
                        return "invalid";
                    }
                }
            }
            let result = step(
                &mut side,
                "document.measure",
                &json!({"feature": "__result", "bound": true}),
            );
            let result = match result {
                Ok(m) => measured(&m),
                Err((verdict, reason)) => Err((refused_or_crash(verdict), reason)),
            };
            match result {
                Ok((v, _, bound)) => {
                    bounds += bound;
                    if let Some(slot) = got.get_mut(k) {
                        *slot = v;
                    }
                    volumes.insert(op.into(), v.into());
                }
                Err((verdict, reason)) => {
                    entry.insert("at".into(), format!("{op} measure").into());
                    entry.insert("reason".into(), reason.into());
                    return verdict;
                }
            }
        }
        entry.insert("ms".into(), ms(started).into());
        entry.insert("volumes".into(), Value::Object(volumes));
        let operands = Operands {
            va: a.volume,
            sa: a.area,
            vb,
            sb,
        };
        if let Some(gap) = gaps(operands, got, TOLERANCE, bounds) {
            entry.insert("at".into(), "identity".into());
            entry.insert("reason".into(), gap.into());
            return "invalid";
        }
        "ok"
    }
}

/// Volume, area and volume bound of a `document.measure` result; a result
/// without finite numbers is a refusal of the measure.
fn measured(m: &Value) -> Result<(f64, f64, f64), (&'static str, String)> {
    let get = |key: &str| m.get(key).and_then(Value::as_f64).filter(|v| v.is_finite());
    match (get("volume"), get("area"), get("volumeBound")) {
        (Some(v), Some(a), Some(b)) => Ok((v, a, b)),
        _ => Err((
            "refused",
            "document.measure gave no finite volume, area and bound".to_string(),
        )),
    }
}

/// [`crate::runner::identity_gaps`] with the integrator's own estimate of
/// how far the measured volumes can be off added to the allowance:
/// `volume_bounds` is the sum of the bounds of all five measured volumes
/// (A, B and the three results); each identity sums at most four of them,
/// once each, so the sum covers either. A real model's spline faces are
/// measured less closely than the fuzz's analytic ones.
pub fn gaps(o: Operands, [u, s, i]: [f64; 3], t: f64, volume_bounds: f64) -> Option<String> {
    let base = crate::runner::identity_gaps(o, [u, s, i], t)?;
    let Operands { va, sa, vb, sb } = o;
    let allowance =
        2.0 * (2.0 * t * (sa + sb)) + 1e-9 * (va.abs() + vb.abs()) + volume_bounds.abs();
    let holds = |gap: f64| gap.is_finite() && gap.abs() <= allowance;
    if holds((u + i) - (va + vb)) && holds((s + i) - va) {
        return None;
    }
    Some(format!(
        "{base}; with the volume bounds allowed {allowance:e}"
    ))
}

/// Runs one model; the line of JSON to print.
pub fn run_model(file: &Path, options: ModelOptions, scratch: &Path) -> Result<Value, String> {
    let started = Instant::now();
    let mut session = Session::new(Workspace::unrestricted());
    session
        .execute("file.new", &json!({"units": "mm", "tolerance": TOLERANCE}))
        .map_err(|e| e.message().to_string())?;
    let mut run = ModelRun {
        session,
        out: Map::new(),
        options,
        scratch: scratch.to_path_buf(),
        names: Vec::new(),
    };
    run.set("file", file.to_string_lossy().to_string());
    let solid = run.import_stage(file);
    if options.booleans {
        match solid {
            Some(target) => run.boolean_stage(&target),
            None => run.set("booleans", Value::Null),
        }
    }
    run.set("totalMs", ms(started));
    Ok(Value::Object(run.out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_volume_bounds_widen_the_allowance() {
        let o = Operands {
            va: 1.0,
            sa: 6.0,
            vb: 1.0,
            sb: 6.0,
        };
        assert_eq!(gaps(o, [1.5, 0.5, 0.5], 1e-6, 0.0), None);
        assert!(gaps(o, [1.49, 0.5, 0.5], 1e-6, 0.0).is_some());
        assert_eq!(gaps(o, [1.49, 0.5, 0.5], 1e-6, 0.01), None);
        assert!(gaps(o, [f64::NAN, 0.5, 0.5], 1e-6, 1.0).is_some());
    }

    #[test]
    fn a_measure_without_numbers_is_a_refusal_not_a_broken_identity() {
        assert_eq!(
            measured(&json!({"volume": 1.0, "area": 6.0, "volumeBound": 1e-12})),
            Ok((1.0, 6.0, 1e-12))
        );
        for m in [
            json!({}),
            json!({"volume": 1.0, "area": 6.0}),
            json!({"volume": null, "area": 6.0, "volumeBound": 0.0}),
        ] {
            assert_eq!(measured(&m).map_err(|e| e.0), Err("refused"), "{m}");
        }
        assert_eq!(refused_or_crash("crash"), "crash");
        assert_eq!(refused_or_crash("invalid"), "refused");
    }

    #[test]
    fn the_bodies_names_are_taken_out_of_reasons() {
        let run = ModelRun {
            session: Session::new(Workspace::unrestricted()),
            out: Map::new(),
            options: ModelOptions {
                booleans: false,
                deep: false,
            },
            scratch: std::env::temp_dir(),
            names: vec!["M5 Socket head cap screw-3".into(), "Part 1".into()],
        };
        assert_eq!(
            run.scrub("file.export: M5 Socket head cap screw-3: face 4; Part 1 too"),
            "file.export: <body>: face 4; <body> too"
        );
    }

    #[test]
    fn a_block_imports_and_passes_its_booleans() {
        let file =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/io/tests/golden/block.step");
        let dir = std::env::temp_dir().join(format!("craft3d-abc-block-{}", std::process::id()));
        let out = run_model(
            &file,
            ModelOptions {
                booleans: true,
                deep: false,
            },
            &dir,
        )
        .unwrap();
        assert_eq!(out["verdict"], "ok", "{out}");
        assert_eq!(out["booleanReady"], true, "{out}");
        assert_eq!(out["booleans"]["verdict"], "ok", "{out}");
        assert_eq!(out["booleans"]["tools"].as_array().map(Vec::len), Some(3));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_step_is_refused_not_a_crash() {
        let dir = std::env::temp_dir().join(format!("craft3d-abc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("broken.step");
        std::fs::write(&file, "ISO-10303-21;\nHEADER;\nDATA;\n#1=FOO(\n").unwrap();
        let out = run_model(
            &file,
            ModelOptions {
                booleans: true,
                deep: false,
            },
            &dir,
        )
        .unwrap();
        assert_eq!(out["verdict"], "refused", "{out}");
        assert_eq!(out["at"], "import");
        assert!(out["booleans"].is_null());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
