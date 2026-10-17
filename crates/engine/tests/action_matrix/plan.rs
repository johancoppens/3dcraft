//! The action matrix (docs/testing.md › Oracle 12): which local operations
//! are tried on which faces and edges of a model, how one is run on the
//! finished document and undone again, and what its outcome is.
//!
//! Pure: it sees the engine only through a function that runs a command by
//! id with JSON params, so the same code drives an in-process
//! `craft3d_engine::Session` (the gated test, `tests/action_matrix.rs`) and
//! a `craft3d-cli serve` process per worker (`cargo xtask actions`, which
//! includes this file by path).

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The kinds of action, in the order they are reported.
pub const KINDS: &[Kind] = &[
    Kind::FilletSmall,
    Kind::FilletLarge,
    Kind::Chamfer,
    Kind::OffsetOut,
    Kind::OffsetIn,
    Kind::DeleteFace,
    Kind::Shell,
    Kind::Thicken,
    Kind::Draft,
    Kind::MoveFace,
    Kind::Mirror,
    Kind::Pattern,
];

/// One kind of local operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Fillet one edge with a tenth of the shortest edge next to it.
    FilletSmall,
    /// Fillet one edge with three tenths of it.
    FilletLarge,
    /// Chamfer one edge by fifteen hundredths of it.
    Chamfer,
    /// Offset one face outward by a tenth of its shortest edge.
    OffsetOut,
    /// The same inward.
    OffsetIn,
    /// Delete one face.
    DeleteFace,
    /// Shell the body with one face open.
    Shell,
    /// Thicken one face outside.
    Thicken,
    /// Draft one flat face by 3 degrees, a flat neighbour square to it as
    /// the neutral plane.
    Draft,
    /// Move one flat face along its normal.
    MoveFace,
    /// Mirror the body (once per body).
    Mirror,
    /// Pattern the body twice, apart (once per body).
    Pattern,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::FilletSmall => "fillet.small",
            Kind::FilletLarge => "fillet.large",
            Kind::Chamfer => "chamfer",
            Kind::OffsetOut => "offset.out",
            Kind::OffsetIn => "offset.in",
            Kind::DeleteFace => "deleteFace",
            Kind::Shell => "shell",
            Kind::Thicken => "thicken",
            Kind::Draft => "draft",
            Kind::MoveFace => "moveFace",
            Kind::Mirror => "mirror",
            Kind::Pattern => "pattern",
        }
    }
}

/// A finished body of the model: one that no later feature takes.
#[derive(Clone, Debug)]
pub struct Body {
    /// The feature that made it, "F12".
    pub feature: String,
    pub faces: Vec<String>,
    pub volume: f64,
    pub area: f64,
    pub bbox: [[f64; 3]; 2],
    pub solid: bool,
}

/// One edge of a body, from `document.edges`.
#[derive(Clone, Debug)]
pub struct Edge {
    pub faces: [String; 2],
    pub curve: String,
    pub start: [f64; 3],
    pub end: [f64; 3],
    pub closed: bool,
}

/// What a successful action's volume must satisfy (stated, not tuned:
/// each follows from what the operation does).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Expect {
    /// A positive, finite volume.
    Positive,
    /// A blend of one edge: positive, and it changes the volume by less
    /// than half (a blend that takes or adds half the body is not a blend
    /// of one edge).
    Blend,
    /// A face moved along its normal by `d` (positive is outward): the
    /// volume changes with the sign of `d`, by at most `|d|` times the
    /// larger area of the body before and after (the swept layer is no
    /// thicker than `|d|` and no larger than the boundary).
    Offset(f64),
    /// A shell: positive and less than the body.
    Hollow,
    /// A layer of thickness `t`: positive and at most `t` times its own
    /// area.
    Layer(f64),
    /// A mirror image: the same volume.
    Same,
    /// Two instances apart: twice the volume.
    Double,
}

/// One action: a command on a copy of the finished document.
#[derive(Clone, Debug)]
pub struct Action {
    pub kind: Kind,
    /// The feature of the body it works on.
    pub body: String,
    /// The persistent name of what it is tried on: a face, an edge as
    /// "face|face" (every edge between the two), or for mirror and
    /// pattern the feature of the body.
    pub entity: String,
    pub command: String,
    pub params: Value,
    pub expect: Expect,
    /// Volume and area of the body before.
    pub volume: f64,
    pub area: f64,
}

/// The outcome of an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// Built, valid (`document.check` deep) and the volume as expected.
    Ok,
    /// Refused, with a reason.
    Refused,
    /// Built, but the check or the volume says it is not right.
    Wrong,
    /// A panic (caught or not), or the process died.
    Crash,
    /// Took longer than allowed.
    Timeout,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::Refused => "refused",
            Verdict::Wrong => "wrong",
            Verdict::Crash => "crash",
            Verdict::Timeout => "timeout",
        }
    }

    pub const ALL: [Verdict; 5] = [
        Verdict::Ok,
        Verdict::Refused,
        Verdict::Wrong,
        Verdict::Crash,
        Verdict::Timeout,
    ];
}

/// Why a call into the engine did not give a result.
#[derive(Clone, Debug)]
pub enum CallError {
    /// The command returned an error.
    Command(String),
    /// The process that runs it died.
    Dead(String),
    /// It took too long.
    Timeout,
}

/// What an action came to.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub verdict: Verdict,
    /// The error, the check's problem or the failed identity; empty when
    /// ok.
    pub reason: String,
    /// The engine must be started again before the next action (a crash,
    /// a timeout, an undo that did not work).
    pub restart: bool,
}

/// Marks the message of a panic `Session::execute` caught.
const INTERNAL: &str = "internal error (a bug in 3DCraft";

fn num(value: &Value) -> Option<f64> {
    value.as_f64().filter(|x| x.is_finite())
}

fn point(value: Option<&Value>) -> Option<[f64; 3]> {
    let items = value?.as_array()?;
    let [x, y, z] = items.as_slice() else {
        return None;
    };
    Some([num(x)?, num(y)?, num(z)?])
}

/// The finished bodies of `document.inspect`.
pub fn bodies(inspect: &Value) -> Vec<Body> {
    let Some(list) = inspect.get("bodies").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|b| {
            let feature = b.get("feature")?.as_str()?.to_string();
            let faces = b
                .get("faceNames")?
                .as_array()?
                .iter()
                .filter_map(|f| f.as_str().map(str::to_string))
                .collect();
            let bbox = b.get("bbox")?.as_array()?;
            let [lo, hi] = bbox.as_slice() else {
                return None;
            };
            Some(Body {
                feature,
                faces,
                volume: num(b.get("volume")?)?,
                area: num(b.get("area")?)?,
                bbox: [point(Some(lo))?, point(Some(hi))?],
                solid: b.get("kind").and_then(Value::as_str) == Some("solid"),
            })
        })
        .collect()
}

/// The length unit of `document.inspect` ("mm").
pub fn length_unit(inspect: &Value) -> String {
    inspect
        .get("units")
        .and_then(|u| u.get("length"))
        .and_then(Value::as_str)
        .unwrap_or("mm")
        .to_string()
}

/// The edges of `document.edges`.
pub fn edges(answer: &Value) -> Vec<Edge> {
    let Some(list) = answer.get("edges").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|e| {
            let faces = e.get("faces")?.as_array()?;
            let [a, b] = faces.as_slice() else {
                return None;
            };
            Some(Edge {
                faces: [a.as_str()?.to_string(), b.as_str()?.to_string()],
                curve: e.get("curve")?.as_str().unwrap_or("").to_string(),
                start: point(e.get("start"))?,
                end: point(e.get("end"))?,
                closed: e.get("closed").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn diagonal(body: &Body) -> f64 {
    norm(sub(body.bbox[1], body.bbox[0]))
}

/// The length of an edge that is not closed (its chord for a curve: a
/// lower bound), `None` for a closed one.
fn length(edge: &Edge) -> Option<f64> {
    let l = norm(sub(edge.end, edge.start));
    (!edge.closed && l.is_finite() && l > 0.0).then_some(l)
}

/// A number as an expression in `unit`, with six significant digits.
pub fn expr(x: f64, unit: &str) -> String {
    let magnitude = if x.abs() > 0.0 && x.is_finite() {
        x.abs().log10().floor()
    } else {
        0.0
    };
    let digits = (5.0 - magnitude).clamp(0.0, 15.0);
    // `digits` is a small whole number, 0 to 15.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let digits = digits as usize;
    format!("{x:.digits$} {unit}")
}

/// The plane of a face bounded by straight edges only, as its unit normal
/// (either way round), when all their ends lie in one plane.
fn plane_of(face: &str, edges: &[Edge], diag: f64) -> Option<[f64; 3]> {
    let mine: Vec<&Edge> = edges
        .iter()
        .filter(|e| e.faces[0] == face || e.faces[1] == face)
        .collect();
    if mine.is_empty() || mine.iter().any(|e| e.curve != "line") {
        return None;
    }
    let points: Vec<[f64; 3]> = mine.iter().flat_map(|e| [e.start, e.end]).collect();
    let p0 = *points.first()?;
    let p1 = *points
        .iter()
        .max_by(|a, b| norm(sub(**a, p0)).total_cmp(&norm(sub(**b, p0))))?;
    let along = sub(p1, p0);
    let p2 = *points.iter().max_by(|a, b| {
        norm(cross(along, sub(**a, p0))).total_cmp(&norm(cross(along, sub(**b, p0))))
    })?;
    let n = cross(along, sub(p2, p0));
    let size = norm(n);
    if !(size.is_finite() && size > 1e-12 * diag * diag) {
        return None;
    }
    let n = [n[0] / size, n[1] / size, n[2] / size];
    let flat = points
        .iter()
        .all(|p| dot(n, sub(*p, p0)).abs() <= 1e-7 * diag + 1e-12);
    flat.then_some(n)
}

/// Every action of the kinds asked for on `body`, whose edges are
/// `edges`: each edge kind on every pair of faces an edge lies between,
/// each face kind on every face it applies to, mirror and pattern once.
/// Which of them are run is the caller's choice ([`sample`]).
pub fn plan(body: &Body, edges: &[Edge], unit: &str, kinds: &[Kind]) -> Vec<Action> {
    let diag = diagonal(body);
    let mut out = Vec::new();
    if !(diag.is_finite() && diag > 0.0) {
        return out;
    }
    // Edges by the pair of faces they lie between (a pair means every
    // edge between them).
    let mut pairs: BTreeMap<String, [String; 2]> = BTreeMap::new();
    for e in edges {
        if e.faces[0] != e.faces[1] {
            pairs
                .entry(format!("{}|{}", e.faces[0], e.faces[1]))
                .or_insert_with(|| e.faces.clone());
        }
    }
    let shortest_of = |face: &str| -> Option<f64> {
        edges
            .iter()
            .filter(|e| e.faces[0] == face || e.faces[1] == face)
            .filter_map(length)
            .min_by(f64::total_cmp)
    };
    let fallback = 0.05 * diag;
    let mut planes: BTreeMap<&str, [f64; 3]> = BTreeMap::new();
    for face in &body.faces {
        if let Some(n) = plane_of(face, edges, diag) {
            planes.insert(face, n);
        }
    }
    let lo = body.bbox[0];
    let hi = body.bbox[1];
    let extent = sub(hi, lo);
    let thin = extent
        .iter()
        .copied()
        .filter(|x| *x > 0.0)
        .min_by(f64::total_cmp)
        .unwrap_or(diag);
    let wall = 0.05 * thin;
    let source = body.feature.clone();
    let mut push = |kind: Kind, entity: &str, command: &str, params: Value, expect: Expect| {
        out.push(Action {
            kind,
            body: source.clone(),
            entity: entity.to_string(),
            command: command.to_string(),
            params,
            expect,
            volume: body.volume,
            area: body.area,
        });
    };
    let pair_names: Vec<String> = pairs.keys().cloned().collect();
    for &kind in kinds {
        match kind {
            Kind::FilletSmall | Kind::FilletLarge | Kind::Chamfer => {
                for key in &pair_names {
                    let Some([a, b]) = pairs.get(key) else {
                        continue;
                    };
                    let near = [shortest_of(a), shortest_of(b)]
                        .into_iter()
                        .flatten()
                        .min_by(f64::total_cmp)
                        .unwrap_or(fallback);
                    let (command, params) = match kind {
                        Kind::FilletSmall => (
                            "feature.fillet",
                            json!({"edges": [[a, b]], "radius": expr(0.1 * near, unit)}),
                        ),
                        Kind::FilletLarge => (
                            "feature.fillet",
                            json!({"edges": [[a, b]], "radius": expr(0.3 * near, unit)}),
                        ),
                        _ => (
                            "feature.chamfer",
                            json!({"edges": [[a, b]], "distance": expr(0.15 * near, unit)}),
                        ),
                    };
                    push(kind, key, command, params, Expect::Blend);
                }
            }
            Kind::OffsetOut | Kind::OffsetIn | Kind::DeleteFace | Kind::Shell | Kind::Thicken => {
                for face in &body.faces {
                    let d = 0.1 * shortest_of(face).unwrap_or(fallback);
                    match kind {
                        Kind::OffsetOut => push(
                            kind,
                            face,
                            "feature.offsetFace",
                            json!({"faces": [face], "distance": expr(d, unit)}),
                            Expect::Offset(d),
                        ),
                        Kind::OffsetIn => push(
                            kind,
                            face,
                            "feature.offsetFace",
                            json!({"faces": [face], "distance": expr(-d, unit)}),
                            Expect::Offset(-d),
                        ),
                        Kind::DeleteFace => push(
                            kind,
                            face,
                            "feature.deleteFace",
                            json!({"faces": [face]}),
                            Expect::Positive,
                        ),
                        Kind::Shell => push(
                            kind,
                            face,
                            "feature.shell",
                            json!({"source": source, "thickness": expr(wall, unit), "open": [face]}),
                            Expect::Hollow,
                        ),
                        _ => push(
                            kind,
                            face,
                            "feature.thicken",
                            json!({"faces": [face], "thickness": expr(wall, unit)}),
                            Expect::Layer(wall),
                        ),
                    }
                }
            }
            Kind::Draft => {
                // A flat face with a flat neighbour square to it: that
                // neighbour is the neutral plane.
                let mut neutral: BTreeMap<String, String> = BTreeMap::new();
                for (face, n) in &planes {
                    let found = edges
                        .iter()
                        .filter_map(|e| {
                            if e.faces[0] == *face {
                                Some(e.faces[1].as_str())
                            } else if e.faces[1] == *face {
                                Some(e.faces[0].as_str())
                            } else {
                                None
                            }
                        })
                        .filter(|g| g != face)
                        .filter(|g| planes.get(g).is_some_and(|m| dot(*m, *n).abs() < 1e-9))
                        .min();
                    if let Some(g) = found {
                        neutral.insert((*face).to_string(), g.to_string());
                    }
                }
                for (face, g) in &neutral {
                    push(
                        kind,
                        face,
                        "feature.draft",
                        json!({"faces": [face], "neutral": g, "angle": "3 deg"}),
                        Expect::Positive,
                    );
                }
            }
            Kind::MoveFace => {
                for (face, n) in &planes {
                    let d = 0.1 * shortest_of(face).unwrap_or(fallback);
                    let t: Vec<String> = n.iter().map(|c| expr(c * d, unit)).collect();
                    push(
                        kind,
                        face,
                        "feature.moveFace",
                        json!({"faces": [face], "translate": t}),
                        Expect::Positive,
                    );
                }
            }
            Kind::Mirror | Kind::Pattern => {
                if kind == Kind::Mirror {
                    let x = hi[0] + 0.1 * diag;
                    push(
                        kind,
                        &source,
                        "feature.mirror",
                        json!({"source": source, "normal": [1, 0, 0], "point": [expr(x, unit), "0", "0"]}),
                        Expect::Same,
                    );
                } else {
                    let step = extent[0] + 0.5 * diag;
                    push(
                        kind,
                        &source,
                        "feature.pattern",
                        json!({"source": source, "count": "2", "step": [expr(step, unit), "0", "0"]}),
                        Expect::Double,
                    );
                }
            }
        }
    }
    out
}

/// FNV-1a of `text`: the stable order in which entities are sampled.
pub fn fnv(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The `k` names of `names` with the smallest hashes (with the kind), so
/// that a name is chosen or not whatever else the body has (bottom-k
/// sampling); every name when `k` is `None`.
pub fn sample(kind: Kind, names: &[String], k: Option<usize>) -> BTreeSet<String> {
    let Some(k) = k else {
        return names.iter().cloned().collect();
    };
    let mut keyed: Vec<(u64, &String)> = names
        .iter()
        .map(|n| (fnv(&format!("{}:{n}", kind.as_str())), n))
        .collect();
    keyed.sort();
    keyed.into_iter().take(k).map(|(_, n)| n.clone()).collect()
}

/// The actions to run of `actions` (all of one model): per kind the `k`
/// whose entities [`sample`] picks, and every one `keep` asks for (the
/// ratchet's), in kind order and then by entity.
pub fn choose(
    actions: Vec<Action>,
    k: Option<usize>,
    keep: &dyn Fn(Kind, &str) -> bool,
) -> Vec<Action> {
    let mut by_kind: BTreeMap<Kind, Vec<Action>> = BTreeMap::new();
    for action in actions {
        by_kind.entry(action.kind).or_default().push(action);
    }
    let mut out = Vec::new();
    for (kind, list) in by_kind {
        // A name can be given to several faces (the pieces of a face a
        // boolean split): it is one entity, tried once.
        let names: Vec<String> = list
            .iter()
            .map(|a| a.entity.clone())
            .collect::<BTreeSet<String>>()
            .into_iter()
            .collect();
        let picked = sample(kind, &names, k);
        let mut chosen: Vec<Action> = list
            .into_iter()
            .filter(|a| picked.contains(&a.entity) || keep(kind, &a.entity))
            .collect();
        chosen.sort_by(|a, b| a.entity.cmp(&b.entity));
        chosen.dedup_by(|a, b| a.entity == b.entity);
        out.extend(chosen);
    }
    out
}

/// The first problem `document.check` listed, as text.
fn first_problem(check: &Value) -> Option<String> {
    let problems = check.get("problems")?.as_array()?;
    let first = problems.first()?;
    Some(
        first
            .get("problem")
            .and_then(Value::as_str)
            .map_or_else(|| first.to_string(), str::to_string),
    )
}

/// Whether `volume` (and `area`) after `action` keep its identity; the
/// failed identity when not.
pub fn identity(action: &Action, volume: f64, area: f64) -> Result<(), String> {
    let v0 = action.volume;
    if !(volume.is_finite() && volume > 0.0) {
        return Err(format!("volume {volume} is not positive"));
    }
    let change = volume - v0;
    // Changes this small are within what the measure itself can be off by
    // (relative 1e-9 of the body): no conclusion either way.
    let noise = 1e-9 * v0.abs();
    match action.expect {
        Expect::Positive => Ok(()),
        Expect::Blend => {
            if change.abs() <= 0.5 * v0 {
                Ok(())
            } else {
                Err(format!(
                    "a blend of one edge changed the volume from {v0} to {volume}"
                ))
            }
        }
        Expect::Offset(d) => {
            let most = d.abs() * action.area.max(area) * (1.0 + 1e-9) + noise;
            if change.abs() <= noise {
                Ok(())
            } else if change * d < 0.0 {
                Err(format!(
                    "an offset by {d} changed the volume the other way: {v0} to {volume}"
                ))
            } else if change.abs() > most {
                Err(format!(
                    "an offset by {d} changed the volume by {change}, more than {most}"
                ))
            } else {
                Ok(())
            }
        }
        Expect::Hollow => {
            if volume < v0 {
                Ok(())
            } else {
                Err(format!(
                    "a shell has volume {volume}, not less than the body's {v0}"
                ))
            }
        }
        Expect::Layer(t) => {
            if volume <= t * area * (1.0 + 1e-9) {
                Ok(())
            } else {
                Err(format!(
                    "a layer {t} thick has volume {volume}, more than {t} times its area {area}"
                ))
            }
        }
        Expect::Same => {
            if change.abs() <= 1e-6 * v0 {
                Ok(())
            } else {
                Err(format!("a mirror image has volume {volume}, the body {v0}"))
            }
        }
        Expect::Double => {
            if (volume - 2.0 * v0).abs() <= 2e-6 * v0 {
                Ok(())
            } else {
                Err(format!(
                    "two instances apart have volume {volume}, twice the body is {}",
                    2.0 * v0
                ))
            }
        }
    }
}

fn outcome(verdict: Verdict, reason: impl Into<String>, restart: bool) -> Outcome {
    Outcome {
        verdict,
        reason: reason.into(),
        restart,
    }
}

fn failed_call(error: CallError, what: &str) -> Outcome {
    match error {
        CallError::Command(message) if message.contains(INTERNAL) => {
            outcome(Verdict::Crash, message, true)
        }
        CallError::Command(message) => outcome(Verdict::Refused, message, false),
        CallError::Dead(message) => outcome(Verdict::Crash, format!("{what}: {message}"), true),
        CallError::Timeout => outcome(Verdict::Timeout, format!("{what} did not finish"), true),
    }
}

/// Runs `action` on the document `call` drives and undoes it again, so
/// the next action starts from the finished model as it was.
pub fn run_action<F>(call: &mut F, action: &Action) -> Outcome
where
    F: FnMut(&str, &Value) -> Result<Value, CallError>,
{
    let result = match call(&action.command, &action.params) {
        Ok(result) => result,
        // A command that fails changes nothing: there is nothing to undo.
        Err(e) => return failed_call(e, &action.command),
    };
    let mut verdict = judge(call, action, &result);
    match call("edit.undo", &json!({})) {
        Ok(_) => {}
        Err(e) => {
            if verdict.verdict == Verdict::Ok || verdict.verdict == Verdict::Refused {
                let undo = failed_call(e, "edit.undo");
                verdict = outcome(undo.verdict, format!("edit.undo: {}", undo.reason), true);
            } else {
                verdict.restart = true;
            }
        }
    }
    verdict
}

fn judge<F>(call: &mut F, action: &Action, result: &Value) -> Outcome
where
    F: FnMut(&str, &Value) -> Result<Value, CallError>,
{
    let status = result.get("status").and_then(Value::as_str).unwrap_or("");
    let error = result.get("error").and_then(Value::as_str).unwrap_or("");
    if status != "ok" {
        let verdict = if error.contains(INTERNAL) {
            Verdict::Crash
        } else {
            Verdict::Refused
        };
        let reason = if error.is_empty() {
            format!("status {status}")
        } else {
            error.to_string()
        };
        return outcome(verdict, reason, false);
    }
    let Some(id) = result.get("id").and_then(Value::as_str).map(str::to_string) else {
        return outcome(Verdict::Wrong, format!("no feature id in {result}"), false);
    };
    let check = match call("document.check", &json!({"deep": true, "feature": id})) {
        Ok(check) => check,
        Err(CallError::Command(message)) if !message.contains(INTERNAL) => {
            return outcome(Verdict::Wrong, format!("document.check: {message}"), false);
        }
        Err(e) => return failed_call(e, "document.check"),
    };
    if check.get("ok").and_then(Value::as_bool) != Some(true) {
        let problem = first_problem(&check).unwrap_or_else(|| check.to_string());
        return outcome(Verdict::Wrong, problem, false);
    }
    let measure = match call("document.measure", &json!({"feature": id})) {
        Ok(measure) => measure,
        Err(CallError::Command(message)) if !message.contains(INTERNAL) => {
            return outcome(
                Verdict::Wrong,
                format!("document.measure: {message}"),
                false,
            );
        }
        Err(e) => return failed_call(e, "document.measure"),
    };
    let volume = measure.get("volume").and_then(num);
    let area = measure.get("area").and_then(num);
    let (Some(volume), Some(area)) = (volume, area) else {
        return outcome(
            Verdict::Wrong,
            format!("no volume or area in {measure}"),
            false,
        );
    };
    match identity(action, volume, area) {
        Ok(()) => outcome(Verdict::Ok, "", false),
        Err(why) => outcome(Verdict::Wrong, why, false),
    }
}

/// The class of a reason: its words without names, numbers and the
/// parameter it starts with, at most 90 characters, so that refusals of
/// one kind count together.
pub fn reason_class(reason: &str) -> String {
    // Drop the parameter and what names it: "edges.0 (between ...): ".
    let mut text = reason;
    if let Some(at) = text.find("): ")
        && text
            .get(..at)
            .is_some_and(|head| head.starts_with(|c: char| c.is_ascii_lowercase()))
    {
        text = text.get(at + 3..).unwrap_or(text);
    }
    // And a parameter alone: "faces: ", "distance: ".
    if let Some((head, rest)) = text.split_once(": ")
        && !head.is_empty()
        && head
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
    {
        text = rest;
    }
    let mut out = String::new();
    for word in text.split_whitespace() {
        let bare = word.trim_matches(|c: char| ",.;:()[]`'\"".contains(c));
        let name = bare.contains(':') && bare.starts_with('F')
            || bare.starts_with("F")
                && bare
                    .get(1..)
                    .is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
            || bare.contains("copy(");
        let number = !bare.is_empty() && bare.parse::<f64>().is_ok();
        let piece = if name {
            "<name>"
        } else if number {
            "<n>"
        } else {
            word
        };
        if out.ends_with("<name>") && piece == "<name>" {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(piece);
        if out.len() >= 90 {
            break;
        }
    }
    let mut cut: String = out.chars().take(90).collect();
    if let Some(at) = cut.find("; ") {
        cut.truncate(at);
    }
    cut
}

/// The ratchet: per model and kind, the entities whose action succeeded.
pub type Ratchet = BTreeMap<String, BTreeMap<String, BTreeSet<String>>>;

pub fn ratchet_from(value: &Value) -> Ratchet {
    let mut out = Ratchet::new();
    let Some(models) = value.get("models").and_then(Value::as_object) else {
        return out;
    };
    for (model, kinds) in models {
        let Some(kinds) = kinds.as_object() else {
            continue;
        };
        let entry = out.entry(model.clone()).or_default();
        for (kind, names) in kinds {
            let set = names
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|n| n.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            entry.insert(kind.clone(), set);
        }
    }
    out
}

pub fn ratchet_json(ratchet: &Ratchet) -> Value {
    let models: serde_json::Map<String, Value> = ratchet
        .iter()
        .map(|(model, kinds)| {
            let kinds: serde_json::Map<String, Value> = kinds
                .iter()
                .filter(|(_, names)| !names.is_empty())
                .map(|(kind, names)| (kind.clone(), json!(names)))
                .collect();
            (model.clone(), Value::Object(kinds))
        })
        .collect();
    json!({
        "about": "The action matrix's ratchet (docs/testing.md, oracle 12): per model and kind, the entities whose action succeeded (built, valid, volume as expected). Each must still succeed. Entries are only added (cargo xtask actions --update, an expectation change in the developer's favour, logged); removing one is an expectation change against the developer and needs the four conditions of docs/testing.md.",
        "models": models,
    })
}
