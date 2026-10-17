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
    pub centroid: [f64; 3],
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
    /// Its length (`document.edges`: exact for a line, a polygon through
    /// the curve otherwise).
    pub length: Option<f64>,
}

/// What a successful action's volume (and area and centroid) must
/// satisfy: stated, not tuned, each from what the operation does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Expect {
    /// A positive, finite volume.
    Positive,
    /// A blend of one pair of faces with size `size` (a fillet's radius, a
    /// chamfer's distance) along edges `length` long in all, `curved` of
    /// them not straight: the volume changes (a blend that changes nothing
    /// is not built), by at most the cross-section's bound times the
    /// length. The cross-section of a chamfer is a triangle with two sides
    /// `size` long, at most `size²/2`; a fillet's is at most `r` times its
    /// setback `r·cot(θ/2)`, which is at most `11.43·r` when the faces
    /// meet at θ of 10° or more (assumed, not checked: a sharper edge is
    /// reported wrong, never let through). Along a curved edge the
    /// cross-section travels at most `2π` times the setback further than
    /// the edge. With `exact` (one straight edge between two flat faces)
    /// the change is the cross-section's own area times the length, with
    /// its sign.
    Blend {
        size: f64,
        length: f64,
        curved: usize,
        chamfer: bool,
        exact: Option<Exact>,
    },
    /// A face moved along its normal by `d` (positive is outward): the
    /// volume changes with the sign of `d` (and does change), by at most
    /// `|d|` times the larger area of the body before and after (the swept
    /// layer is no thicker than `|d|` and no larger than the boundary).
    Offset(f64),
    /// A flat face moved by `d` along its normal, either way round: the
    /// volume changes, by at most `|d|` times the larger area.
    Move(f64),
    /// A face tilted by 3°: no point of it moves further than
    /// `tan 3°` times the body's diagonal `diag`, so the volume changes by
    /// at most that times the larger area.
    Draft(f64),
    /// A shell with walls `t` thick: less than the body, and at most `t`
    /// times its own area (each wall lies within `t` of the boundary).
    Hollow(f64),
    /// A layer of thickness `t`: positive and at most `t` times its own
    /// area.
    Layer(f64),
    /// A mirror image in the plane `x = x0`: the same volume and area, the
    /// centroid mirrored.
    Same(f64),
    /// Two instances `step` apart along x: twice the volume and the area,
    /// the centroid half a step along.
    Double(f64),
}

/// The blend of one straight edge between two flat faces, worked out from
/// their outward normals ([`orient`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Exact {
    /// The cross-section's area: a fillet of radius `r` in a wedge of
    /// angle φ, `r²·(cot(φ/2) − (π − φ)/2)`; a chamfer of distance `d`,
    /// `d²·sin(φ)/2`.
    pub section: f64,
    /// The edge is an inside one: the blend adds material (an outside one
    /// takes it away).
    pub gains: bool,
    /// How far the blend reaches along each face from the edge.
    pub setback: f64,
    /// The edge's length.
    pub length: f64,
}

/// The faces of an edge-pair action and, when the pair is one straight
/// edge, its ends.
#[derive(Clone, Debug, PartialEq)]
pub struct Pair {
    pub faces: [String; 2],
    pub line: Option<([f64; 3], [f64; 3])>,
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
    /// Volume, area and centroid of the body before, and its diagonal.
    pub volume: f64,
    pub area: f64,
    pub centroid: [f64; 3],
    pub diag: f64,
    /// For an edge-pair action: the faces and the edge.
    pub pair: Option<Pair>,
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
    /// a timeout, an undo that did not work or did not restore the
    /// document).
    pub restart: bool,
    /// Volume and area measured after a built action.
    pub after: Option<(f64, f64)>,
    /// The undo did not bring the document back as it was (counted as a
    /// problem of the run, whatever the verdict).
    pub unrestored: bool,
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
                centroid: point(b.get("centroid"))?,
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
                length: e.get("length").and_then(num),
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

/// Decimals that leave six significant digits of `x`.
fn decimals(x: f64) -> usize {
    let magnitude = if x.abs() > 0.0 && x.is_finite() {
        x.abs().log10().floor()
    } else {
        0.0
    };
    let digits = (5.0 - magnitude).clamp(0.0, 15.0);
    // `digits` is a small whole number, 0 to 15.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let digits = digits as usize;
    digits
}

/// A number as an expression in `unit`, with six significant digits.
pub fn expr(x: f64, unit: &str) -> String {
    let digits = decimals(x);
    format!("{x:.digits$} {unit}")
}

/// `x` as [`expr`] writes it: the value the command is given.
fn rounded(x: f64) -> f64 {
    let digits = decimals(x);
    format!("{x:.digits$}").parse().unwrap_or(x)
}

/// The entity name of an edge pair: the two face names in order.
pub fn pair_key(a: &str, b: &str) -> String {
    if a <= b {
        format!("{a}|{b}")
    } else {
        format!("{b}|{a}")
    }
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

/// The edges between one pair of faces.
struct PairEdges {
    faces: [String; 2],
    length: f64,
    count: usize,
    curved: usize,
    /// The ends of a straight one.
    line: Option<([f64; 3], [f64; 3])>,
}

fn unit_vector(a: [f64; 3]) -> Option<[f64; 3]> {
    let n = norm(a);
    (n.is_finite() && n > 0.0).then(|| [a[0] / n, a[1] / n, a[2] / n])
}

/// The direction from the straight edge `p`–`q` into the flat face
/// `face`, square to the edge: from the face's other edges at the edge's
/// ends, which must all agree (a reflex corner of the face at an end makes
/// it unknown).
fn into_face(face: &str, p: [f64; 3], q: [f64; 3], edges: &[Edge]) -> Option<[f64; 3]> {
    let e = unit_vector(sub(q, p))?;
    let same = |a: [f64; 3], b: [f64; 3]| norm(sub(a, b)) <= 1e-9 * (1.0 + norm(sub(q, p)));
    let mut found: Vec<[f64; 3]> = Vec::new();
    for edge in edges
        .iter()
        .filter(|x| x.faces[0] == face || x.faces[1] == face)
    {
        if (same(edge.start, p) && same(edge.end, q)) || (same(edge.start, q) && same(edge.end, p))
        {
            continue;
        }
        for (at, other) in [(edge.start, edge.end), (edge.end, edge.start)] {
            if same(at, p) || same(at, q) {
                let w = sub(other, at);
                let along = dot(w, e);
                let across = [
                    w[0] - along * e[0],
                    w[1] - along * e[1],
                    w[2] - along * e[2],
                ];
                found.push(unit_vector(across)?);
            }
        }
    }
    let first = *found.first()?;
    if found.len() < 2 || found.iter().any(|d| dot(*d, first) < 1.0 - 1e-6) {
        return None;
    }
    Some(first)
}

/// Works out the exact volume change of a blend of one straight edge
/// between two flat faces, from the faces' outward normals (`outward`,
/// as `datum.plane` gives them): the wedge between the faces and whether
/// the edge is an outside or an inside one. Leaves the action as it is
/// when that cannot be told.
pub fn orient(action: &mut Action, edges: &[Edge], outward: &BTreeMap<String, [f64; 3]>) {
    let Expect::Blend {
        size,
        chamfer,
        ref mut exact,
        ..
    } = action.expect
    else {
        return;
    };
    let Some(Pair {
        faces: [a, b],
        line: Some((p, q)),
    }) = &action.pair
    else {
        return;
    };
    let (Some(na), Some(nb)) = (outward.get(a), outward.get(b)) else {
        return;
    };
    let (Some(da), Some(db)) = (into_face(a, *p, *q, edges), into_face(b, *p, *q, edges)) else {
        return;
    };
    // Outside edge: each face runs into the material behind the other.
    let (sa, sb) = (dot(db, *na), dot(da, *nb));
    if sa.abs() < 1e-9 || sb.abs() < 1e-9 || (sa < 0.0) != (sb < 0.0) {
        return;
    }
    let phi = dot(da, db).clamp(-1.0, 1.0).acos();
    if !(phi > 1e-6 && phi < std::f64::consts::PI - 1e-6) {
        return;
    }
    let half = (0.5 * phi).tan();
    let (section, setback) = if chamfer {
        (0.5 * size * size * phi.sin(), size)
    } else {
        (
            size * size * (1.0 / half - 0.5 * (std::f64::consts::PI - phi)),
            size / half,
        )
    };
    *exact = Some(Exact {
        section,
        gains: sa > 0.0,
        setback,
        length: norm(sub(*q, *p)),
    });
}

/// [`orient`]s every blend of `actions` it can: asks the engine for the
/// outward normal of each flat face such a blend needs (a datum plane on
/// the face, undone again) and works the exact change out. `edges` are
/// the edges of each body by its feature. An error when an undo fails:
/// the document is then not the finished model any more.
pub fn orient_all<F>(
    call: &mut F,
    actions: &mut [Action],
    edges: &BTreeMap<String, Vec<Edge>>,
) -> Result<usize, String>
where
    F: FnMut(&str, &Value) -> Result<Value, CallError>,
{
    // The faces of every blend of one straight edge; `datum.plane` says
    // which of them are flat.
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    for a in actions.iter() {
        if let (
            Expect::Blend { .. },
            Some(Pair {
                faces,
                line: Some(_),
            }),
        ) = (a.expect, &a.pair)
        {
            wanted.extend(faces.iter().cloned());
        }
    }
    let mut outward: BTreeMap<String, [f64; 3]> = BTreeMap::new();
    for face in wanted {
        let made = call("datum.plane", &json!({"from": face, "distance": "0"}));
        if let Ok(made) = &made {
            let flat = made.get("status").and_then(Value::as_str) == Some("ok");
            if let Some(n) = made
                .get("datum")
                .and_then(|d| point(d.get("normal")))
                .filter(|_| flat)
            {
                outward.insert(face.clone(), n);
            }
            call("edit.undo", &json!({}))
                .map_err(|e| format!("edit.undo after a datum plane on {face}: {e:?}"))?;
        }
    }
    let mut oriented = 0;
    for a in actions.iter_mut() {
        if let Some(list) = edges.get(&a.body) {
            orient(a, list, &outward);
            if matches!(a.expect, Expect::Blend { exact: Some(_), .. }) {
                oriented += 1;
            }
        }
    }
    Ok(oriented)
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
    // The key is the two names in order, so that the same pair is the
    // same entity whichever way round an edge lists them.
    let mut pairs: BTreeMap<String, PairEdges> = BTreeMap::new();
    for e in edges {
        let [a, b] = &e.faces;
        if a == b {
            continue;
        }
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        // An edge without a length (an answer of an older engine) counts
        // as long as half the body's box goes round: π times the diagonal.
        let l = e.length.unwrap_or(std::f64::consts::PI * diag);
        let entry = pairs.entry(pair_key(a, b)).or_insert_with(|| PairEdges {
            faces: [a.clone(), b.clone()],
            length: 0.0,
            count: 0,
            curved: 0,
            line: None,
        });
        entry.length += l;
        entry.count += 1;
        if e.curve == "line" {
            entry.line = Some((e.start, e.end));
        } else {
            entry.curved += 1;
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
        let pair = pairs.get(entity).map(|p| Pair {
            faces: p.faces.clone(),
            line: if p.count == 1 { p.line } else { None },
        });
        out.push(Action {
            pair,
            kind,
            body: source.clone(),
            entity: entity.to_string(),
            command: command.to_string(),
            params,
            expect,
            volume: body.volume,
            area: body.area,
            centroid: body.centroid,
            diag,
        });
    };
    let pair_names: Vec<String> = pairs.keys().cloned().collect();
    for &kind in kinds {
        match kind {
            Kind::FilletSmall | Kind::FilletLarge | Kind::Chamfer => {
                for key in &pair_names {
                    let Some(PairEdges {
                        faces: [a, b],
                        length,
                        curved,
                        ..
                    }) = pairs.get(key)
                    else {
                        continue;
                    };
                    let near = [shortest_of(a), shortest_of(b)]
                        .into_iter()
                        .flatten()
                        .min_by(f64::total_cmp)
                        .unwrap_or(fallback);
                    let (command, key_name, size) = match kind {
                        Kind::FilletSmall => ("feature.fillet", "radius", 0.1 * near),
                        Kind::FilletLarge => ("feature.fillet", "radius", 0.3 * near),
                        _ => ("feature.chamfer", "distance", 0.15 * near),
                    };
                    let params = json!({"edges": [[a, b]], key_name: expr(size, unit)});
                    let expect = Expect::Blend {
                        size: rounded(size),
                        length: *length,
                        curved: *curved,
                        chamfer: kind == Kind::Chamfer,
                        exact: None,
                    };
                    push(kind, key, command, params, expect);
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
                            Expect::Offset(rounded(d)),
                        ),
                        Kind::OffsetIn => push(
                            kind,
                            face,
                            "feature.offsetFace",
                            json!({"faces": [face], "distance": expr(-d, unit)}),
                            Expect::Offset(-rounded(d)),
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
                            Expect::Hollow(rounded(wall)),
                        ),
                        _ => push(
                            kind,
                            face,
                            "feature.thicken",
                            json!({"faces": [face], "thickness": expr(wall, unit)}),
                            Expect::Layer(rounded(wall)),
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
                        Expect::Draft(diag),
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
                        Expect::Move(d),
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
                        Expect::Same(rounded(x)),
                    );
                } else {
                    let step = extent[0] + 0.5 * diag;
                    push(
                        kind,
                        &source,
                        "feature.pattern",
                        json!({"source": source, "count": "2", "step": [expr(step, unit), "0", "0"]}),
                        Expect::Double(rounded(step)),
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

/// What is measured of the body an action made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measured {
    pub volume: f64,
    pub area: f64,
    pub centroid: [f64; 3],
}

/// The fillet's setback over its radius at the smallest dihedral angle
/// the general bound assumes, 10°: `cot 5°`.
const SETBACK: f64 = 11.430_052_302_761_343;

/// How far an end face at the smallest angle to the edge assumed, 10°,
/// moves the end of a blend, per unit of setback: `cot 10°`.
const END_TILT: f64 = 5.671_281_819_617_709;

/// Whether what was measured after `action` keeps its identity; the
/// failed identity when not.
pub fn identity(action: &Action, after: &Measured) -> Result<(), String> {
    let v0 = action.volume;
    let a0 = action.area;
    let Measured {
        volume,
        area,
        centroid,
    } = *after;
    if !(volume.is_finite() && volume > 0.0) {
        return Err(format!("volume {volume} is not positive"));
    }
    let change = volume - v0;
    // Changes this small are within what the measure itself can be off by
    // (relative 1e-9 of the body): no conclusion either way.
    let noise = 1e-9 * v0.abs();
    let larger = a0.max(area);
    // The centroid is held to 1e-6 of the body's diagonal.
    let near = |a: [f64; 3], b: [f64; 3]| norm(sub(a, b)) <= 1e-6 * action.diag;
    let changes = |what: &str| {
        if change.abs() > noise {
            Ok(())
        } else {
            Err(format!(
                "{what} left the volume as it was: {v0} to {volume}"
            ))
        }
    };
    let at_most = |what: &str, most: f64| {
        if change.abs() <= most * (1.0 + 1e-6) + noise {
            Ok(())
        } else {
            Err(format!(
                "{what} changed the volume by {change}, more than {most}"
            ))
        }
    };
    match action.expect {
        Expect::Positive => Ok(()),
        Expect::Blend {
            size,
            length,
            curved,
            chamfer,
            exact,
        } => {
            changes("a blend")?;
            if let Some(x) = exact {
                // Where it ends, an end face cut at γ to the edge moves the
                // end by up to the setback times cot γ: γ of 10° or more
                // is assumed (cot 10° = 5.67).
                let ends = 2.0 * END_TILT * x.setback * x.section;
                let expected = x.section * x.length;
                let (lo, hi) = ((expected - ends).max(0.0), expected + ends);
                let taken = if x.gains { change } else { -change };
                if taken <= noise {
                    return Err(format!(
                        "a blend of an {} edge {} the volume: {v0} to {volume}",
                        if x.gains { "inside" } else { "outside" },
                        if x.gains {
                            "did not add to"
                        } else {
                            "did not take from"
                        }
                    ));
                }
                if taken < lo * (1.0 - 1e-6) - noise || taken > hi * (1.0 + 1e-6) + noise {
                    return Err(format!(
                        "a blend of section {} along {} changed the volume by {change}, not within [{lo}, {hi}]",
                        x.section, x.length
                    ));
                }
                return Ok(());
            }
            let (section, setback) = if chamfer {
                (0.5 * size * size, size)
            } else {
                (SETBACK * size * size, SETBACK * size)
            };
            // `curved` is a small count.
            #[allow(clippy::cast_precision_loss)]
            let travel = length + std::f64::consts::TAU * setback * curved as f64;
            at_most("a blend", section * travel)
        }
        Expect::Offset(d) => {
            if change * d.signum() <= noise {
                return Err(format!(
                    "an offset by {d} did not change the volume that way: {v0} to {volume}"
                ));
            }
            at_most(&format!("an offset by {d}"), d.abs() * larger)
        }
        Expect::Move(d) => {
            changes("a move of a face along its normal")?;
            at_most(&format!("a move by {d}"), d.abs() * larger)
        }
        Expect::Draft(diag) => {
            changes("a draft")?;
            at_most("a draft by 3°", 3f64.to_radians().tan() * diag * larger)
        }
        Expect::Hollow(t) => {
            if volume >= v0 {
                Err(format!(
                    "a shell has volume {volume}, not less than the body's {v0}"
                ))
            } else if volume > t * area * (1.0 + 1e-6) {
                Err(format!(
                    "a shell with walls {t} thick has volume {volume}, more than {t} times its area {area}"
                ))
            } else {
                Ok(())
            }
        }
        Expect::Layer(t) => {
            if volume <= t * area * (1.0 + 1e-6) {
                Ok(())
            } else {
                Err(format!(
                    "a layer {t} thick has volume {volume}, more than {t} times its area {area}"
                ))
            }
        }
        Expect::Same(x0) => {
            let c = action.centroid;
            let mirrored = [2.0 * x0 - c[0], c[1], c[2]];
            if change.abs() > 1e-6 * v0 {
                Err(format!("a mirror image has volume {volume}, the body {v0}"))
            } else if (area - a0).abs() > 1e-6 * a0 {
                Err(format!("a mirror image has area {area}, the body {a0}"))
            } else if !near(centroid, mirrored) {
                Err(format!(
                    "a mirror image has its centroid at {centroid:?}, not at {mirrored:?}"
                ))
            } else {
                Ok(())
            }
        }
        Expect::Double(step) => {
            let c = action.centroid;
            let between = [c[0] + 0.5 * step, c[1], c[2]];
            if (volume - 2.0 * v0).abs() > 2e-6 * v0 {
                Err(format!(
                    "two instances apart have volume {volume}, twice the body is {}",
                    2.0 * v0
                ))
            } else if (area - 2.0 * a0).abs() > 2e-6 * a0 {
                Err(format!(
                    "two instances apart have area {area}, twice the body is {}",
                    2.0 * a0
                ))
            } else if !near(centroid, between) {
                Err(format!(
                    "two instances apart have their centroid at {centroid:?}, not at {between:?}"
                ))
            } else {
                Ok(())
            }
        }
    }
}

fn outcome(verdict: Verdict, reason: impl Into<String>, restart: bool) -> Outcome {
    Outcome {
        verdict,
        reason: reason.into(),
        restart,
        after: None,
        unrestored: false,
    }
}

/// How many features and variables the active document has
/// (`file.list`): what an undo must give back.
fn shape<F>(call: &mut F) -> Option<(u64, u64)>
where
    F: FnMut(&str, &Value) -> Result<Value, CallError>,
{
    let list = call("file.list", &json!({})).ok()?;
    let active = list.get("active").and_then(Value::as_u64)?;
    let doc = list
        .get("documents")?
        .as_array()?
        .iter()
        .find(|d| d.get("document").and_then(Value::as_u64) == Some(active))?;
    Some((
        doc.get("features").and_then(Value::as_u64)?,
        doc.get("variables").and_then(Value::as_u64)?,
    ))
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
    let before = shape(call);
    let result = match call(&action.command, &action.params) {
        Ok(result) => result,
        // A command that fails changes nothing: there is nothing to undo.
        Err(e) => return failed_call(e, &action.command),
    };
    let mut verdict = judge(call, action, &result);
    let undone = call("edit.undo", &json!({}));
    // The undo must have taken the feature out again: asked about, it is
    // not there.
    let restored = match (&undone, result.get("id").and_then(Value::as_str)) {
        (Ok(_), Some(id)) => match call("document.check", &json!({"feature": id})) {
            Err(CallError::Command(message)) => message.contains("there is no feature"),
            _ => false,
        },
        (Ok(_), None) => true,
        (Err(_), _) => false,
    } && before.is_some()
        && shape(call) == before;
    if let Err(e) = undone {
        if verdict.verdict == Verdict::Ok || verdict.verdict == Verdict::Refused {
            let undo = failed_call(e, "edit.undo");
            let after = verdict.after;
            verdict = outcome(undo.verdict, format!("edit.undo: {}", undo.reason), true);
            verdict.after = after;
        }
        verdict.restart = true;
    } else if !restored {
        // Not a verdict on the action, but the next one would not start
        // from the finished model.
        verdict.restart = true;
    }
    verdict.unrestored = !restored;
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
    if check.get("bodiesChecked").and_then(Value::as_u64) != Some(1) {
        return outcome(
            Verdict::Wrong,
            format!("document.check of the new feature did not check one body: {check}"),
            false,
        );
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
    let centroid = point(measure.get("centroid"));
    let (Some(volume), Some(area), Some(centroid)) = (volume, area, centroid) else {
        return outcome(
            Verdict::Wrong,
            format!("no volume, area or centroid in {measure}"),
            false,
        );
    };
    let measured = Measured {
        volume,
        area,
        centroid,
    };
    let mut verdict = match identity(action, &measured) {
        Ok(()) => outcome(Verdict::Ok, "", false),
        Err(why) => outcome(Verdict::Wrong, why, false),
    };
    verdict.after = Some((volume, area));
    verdict
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

/// The ratchet of its file's JSON; an error for anything that is not
/// one (a ratchet read wrong would let every regression through).
pub fn ratchet_from(value: &Value) -> Result<Ratchet, String> {
    let mut out = Ratchet::new();
    let models = value
        .get("models")
        .and_then(Value::as_object)
        .ok_or("the ratchet has no `models` object")?;
    for (model, kinds) in models {
        let kinds = kinds
            .as_object()
            .ok_or_else(|| format!("the ratchet's {model} is not an object of kinds"))?;
        let entry = out.entry(model.clone()).or_default();
        for (kind, names) in kinds {
            if !KINDS.iter().any(|k| k.as_str() == kind) {
                return Err(format!("the ratchet's {model} has an unknown kind {kind}"));
            }
            let names = names
                .as_array()
                .ok_or_else(|| format!("the ratchet's {model} {kind} is not a list"))?;
            let mut set = BTreeSet::new();
            for n in names {
                let n = n.as_str().ok_or_else(|| {
                    format!("the ratchet's {model} {kind} has a name that is not text: {n}")
                })?;
                set.insert(n.to_string());
            }
            entry.insert(kind.clone(), set);
        }
    }
    Ok(out)
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
        "about": "The action matrix's ratchet (docs/testing.md, oracle 12): per model and kind, the entities whose action succeeded (built, valid, volume as expected). Each must still succeed. Entries are only added (cargo xtask actions --update or --raise, an expectation change in the developer's favour, logged); an entity is a face name (all the faces that share it: the pieces of a face a boolean split), an edge as its two faces in order (every edge between them), or the feature of a body; removing one is an expectation change against the developer and needs the four conditions of docs/testing.md.",
        "models": models,
    })
}
