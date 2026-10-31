//! The action matrix (docs/testing.md › Oracle 12): which local operations
//! are tried on which faces and edges of a model, how one is run on the
//! finished document and undone again, and what its outcome is.
//!
//! Pure: it sees the engine only through a function that runs a command by
//! id with JSON params, so the same code drives an in-process
//! `craft3d_engine::Session` (the gated test, `tests/action_matrix.rs`) and
//! a `craft3d-cli serve` process per worker (`cargo xtask actions`, which
//! includes this file by path).

use craft3d_math::Portable;
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
    /// chamfer's distance) along edges `length` long in all: the volume
    /// changes (a blend that changes nothing is not built), by at most the
    /// cross-section's bound times how far it travels. The cross-section
    /// of a chamfer is a triangle with two sides `size` long, at most
    /// `size²/2`; a fillet's is at most `r` times its setback
    /// `r·cot(θ/2)`, with θ the `wedge` the faces meet at where it is
    /// known (both faces flat or of revolution about an axis along the
    /// edge, from their normals: the smaller of the angle between them
    /// and its supplement), else 10° or more, `11.43·r` (assumed, not
    /// checked: a sharper edge is reported wrong, never let through).
    /// Along a curved edge the cross-section travels further than the
    /// edge by the setback times the angle it turns through: `turning` in
    /// all for the circles, `2π` for each of the `curved` others. Past
    /// each of the `ends` of the edges (the vertices only one of them ends
    /// in) the blend runs on until it leaves the end face, at most the
    /// setback times `cot γ` further for an end face at γ to the edge:
    /// `tilt` is `cot γ` summed over the ends, measured where the end face
    /// is flat and the edge straight, else 10° or more assumed (as for the
    /// exact change: a more slanted end is reported wrong, never let
    /// through); `slant` is `1/sin γ` summed the same way, for the area
    /// the end faces gain or lose. With `exact` (one straight edge between
    /// two flat faces) the change is the cross-section's own area times
    /// the length, with its sign.
    Blend {
        size: f64,
        length: f64,
        curved: usize,
        turning: f64,
        ends: usize,
        tilt: f64,
        slant: f64,
        wedge: Option<f64>,
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
    /// The length of the blend's own face across: a fillet's arc
    /// `r·(π − φ)`, a chamfer's chord `2d·sin(φ/2)`.
    pub across: f64,
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
    /// How far the body's volume and area before can be off by the
    /// integrator's own account (`document.measure` with `bound`,
    /// [`bound_all`]).
    pub volume_bound: Option<f64>,
    pub area_bound: Option<f64>,
}

/// A blend's expectation before the faces are looked at: no wedge known,
/// every end at 10° ([`refine`] and [`orient`] know more).
pub fn blend(size: f64, length: f64, curved: usize, ends: usize, chamfer: bool) -> Expect {
    // `ends` is a small count.
    #[allow(clippy::cast_precision_loss)]
    let n = ends as f64;
    Expect::Blend {
        size,
        length,
        curved,
        turning: 0.0,
        ends,
        tilt: n * END_TILT,
        slant: n * END_SLANT,
        wedge: None,
        chamfer,
        exact: None,
    }
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
        x.abs().plog10().floor()
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
    /// Edges neither straight nor circles.
    curved: usize,
    /// What the circles turn through, in all.
    turning: f64,
    /// The edges themselves.
    edges: Vec<Edge>,
    /// The ends of a straight one.
    line: Option<([f64; 3], [f64; 3])>,
}

fn unit_vector(a: [f64; 3]) -> Option<[f64; 3]> {
    let n = norm(a);
    (n.is_finite() && n > 0.0).then(|| [a[0] / n, a[1] / n, a[2] / n])
}

/// The angle a circular edge turns through, from its chord and its length
/// (`sin(t/2)/(t/2)` is the chord over the length): `2π` for a closed one.
/// `document.edges` gives the length of the polygon through 64 points,
/// short of the arc by the factor `sinc(t/128)`; the length is taken that
/// much longer for an angle at least the true one (first 2π, then the
/// angle found, which is still at least the true one), so the angle is
/// never too small.
pub fn turning(e: &Edge) -> Option<f64> {
    if e.closed {
        return Some(std::f64::consts::TAU);
    }
    let polygon = e.length?;
    let chord = norm(sub(e.end, e.start));
    if !(polygon.is_finite() && polygon > 0.0 && chord.is_finite()) {
        return None;
    }
    let sinc = |x: f64| if x == 0.0 { 1.0 } else { x.psin() / x };
    // The half angle whose sinc is `q`, sinc decreasing on [0, π].
    let half = |q: f64| {
        if q >= 1.0 {
            return 0.0;
        }
        let (mut lo, mut hi) = (0.0, std::f64::consts::PI);
        if sinc(hi) >= q {
            return hi;
        }
        for _ in 0..100 {
            let mid = 0.5 * (lo + hi);
            if sinc(mid) > q {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        hi
    };
    let mut t = std::f64::consts::TAU;
    for _ in 0..8 {
        let length = polygon / sinc(t / 128.0);
        t = 2.0 * half(chord / length);
    }
    // For a short arc the chord over the length is 1 − t²/24, so its
    // rounding moves t by about 1e-15/t: a microradian more covers it.
    Some(t * (1.0 + 1e-9) + 1e-6)
}

/// The vertices only one of `edges` ends in (closed edges end nowhere),
/// each with that edge.
pub fn loose_ends(edges: &[Edge], diag: f64) -> Vec<([f64; 3], Edge)> {
    let tol = 1e-9 * diag.max(1e-300);
    let mut points: Vec<([f64; 3], usize, Edge)> = Vec::new();
    for e in edges.iter().filter(|e| !e.closed) {
        for p in [e.start, e.end] {
            match points.iter_mut().find(|(q, _, _)| norm(sub(p, *q)) <= tol) {
                Some((_, n, _)) => *n += 1,
                None => points.push((p, 1, e.clone())),
            }
        }
    }
    points
        .into_iter()
        .filter(|(_, n, _)| *n == 1)
        .map(|(p, _, e)| (p, e))
        .collect()
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
    let phi = dot(da, db).clamp(-1.0, 1.0).pacos();
    if !(phi > 1e-6 && phi < std::f64::consts::PI - 1e-6) {
        return;
    }
    let half = (0.5 * phi).ptan();
    let (section, setback, across) = if chamfer {
        (
            0.5 * size * size * phi.psin(),
            size,
            2.0 * size * (0.5 * phi).psin(),
        )
    } else {
        (
            size * size * (1.0 / half - 0.5 * (std::f64::consts::PI - phi)),
            size / half,
            size * (std::f64::consts::PI - phi),
        )
    };
    *exact = Some(Exact {
        section,
        gains: sa > 0.0,
        setback,
        across,
        length: norm(sub(*q, *p)),
    });
}

/// The edges of `list` between the faces `a` and `b`.
fn between(list: &[Edge], a: &str, b: &str) -> Vec<Edge> {
    list.iter()
        .filter(|e| (e.faces[0] == a && e.faces[1] == b) || (e.faces[0] == b && e.faces[1] == a))
        .cloned()
        .collect()
}

/// The faces other than `a` and `b` of the edges of `list` that end in
/// `p`: the end faces of a blend of the edge between `a` and `b` there.
fn end_faces(list: &[Edge], p: [f64; 3], a: &str, b: &str, diag: f64) -> BTreeSet<String> {
    let tol = 1e-9 * diag.max(1e-300);
    list.iter()
        .filter(|e| !e.closed && (norm(sub(e.start, p)) <= tol || norm(sub(e.end, p)) <= tol))
        .flat_map(|e| e.faces.iter())
        .filter(|f| f.as_str() != a && f.as_str() != b)
        .cloned()
        .collect()
}

/// The faces' normals as far as they are known: flat faces by their
/// outward normal, faces of revolution by their axis (origin, direction).
#[derive(Clone, Debug, Default)]
pub struct Normals {
    pub flat: BTreeMap<String, [f64; 3]>,
    pub axes: BTreeMap<String, ([f64; 3], [f64; 3])>,
}

impl Normals {
    /// The normal (either way round) of `face` all along the straight
    /// edge `p`–`q`: a flat face's, or a face of revolution's when its axis
    /// runs along the edge (then the face is a cylinder along the edge
    /// and its normal is the one from the axis to the edge).
    fn along(&self, face: &str, p: [f64; 3], q: [f64; 3]) -> Option<[f64; 3]> {
        if let Some(n) = self.flat.get(face) {
            return Some(*n);
        }
        let (o, u) = self.axes.get(face)?;
        let u = unit_vector(*u)?;
        let e = unit_vector(sub(q, p))?;
        if dot(u, e).abs() < 1.0 - 1e-9 {
            return None;
        }
        let w = sub(p, *o);
        let t = dot(w, u);
        unit_vector([w[0] - t * u[0], w[1] - t * u[1], w[2] - t * u[2]])
    }
}

/// What the faces at a blend's edges and ends say beyond its plan
/// ([`blend`]): the wedge between the two faces, where every edge of the
/// pair is straight and both faces' normals are known along it, and for
/// each end of a straight edge at a single flat end face, its angle γ to
/// the edge (`tilt` and `slant` then count that end's `cot γ` and
/// `1/sin γ` instead of 10°'s).
pub fn refine(action: &mut Action, list: &[Edge], normals: &Normals) {
    let Some(Pair { faces: [a, b], .. }) = action.pair.clone() else {
        return;
    };
    let diag = action.diag;
    let Expect::Blend {
        ref mut tilt,
        ref mut slant,
        ref mut wedge,
        ..
    } = action.expect
    else {
        return;
    };
    let mine = between(list, &a, &b);
    if mine.is_empty() {
        return;
    }
    if mine.iter().all(|e| e.curve == "line") {
        let mut least: Option<f64> = None;
        for e in &mine {
            let (Some(na), Some(nb)) = (
                normals.along(&a, e.start, e.end),
                normals.along(&b, e.start, e.end),
            ) else {
                least = None;
                break;
            };
            // The wedge is π less the angle between the outward normals;
            // either way round, it is at least the smaller of that angle
            // and its supplement.
            let beta = dot(na, nb).abs().min(1.0).pacos();
            least = Some(least.map_or(beta, |l: f64| l.min(beta)));
        }
        *wedge = least.filter(|w| *w > 1e-6);
    }
    let (mut t, mut s) = (0.0, 0.0);
    for (p, e) in loose_ends(&mine, diag) {
        let measured = (e.curve == "line")
            .then(|| unit_vector(sub(e.end, e.start)))
            .flatten()
            .and_then(|d| {
                let ends = end_faces(list, p, &a, &b, diag);
                let only = (ends.len() == 1)
                    .then(|| ends.into_iter().next())
                    .flatten()?;
                let n = normals.flat.get(&only)?;
                let sin = dot(d, *n).abs().min(1.0);
                (sin > 1e-9).then(|| ((1.0 - sin * sin).max(0.0).sqrt() / sin, 1.0 / sin))
            });
        let (c, k) = measured.unwrap_or((END_TILT, END_SLANT));
        t += c;
        s += k;
    }
    *tilt = t;
    *slant = s;
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
    // The faces of every blend whose edges are all straight, and the end
    // faces at their ends; `datum.plane` says which of them are flat, and
    // `datum.axis` gives the axis of the pair's faces of revolution.
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    let mut pairs: BTreeSet<String> = BTreeSet::new();
    for a in actions.iter() {
        let (
            Expect::Blend { .. },
            Some(Pair {
                faces: [fa, fb], ..
            }),
            Some(list),
        ) = (a.expect, &a.pair, edges.get(&a.body))
        else {
            continue;
        };
        let mine = between(list, fa, fb);
        if mine.is_empty() || mine.iter().any(|e| e.curve != "line") {
            continue;
        }
        pairs.insert(fa.clone());
        pairs.insert(fb.clone());
        wanted.insert(fa.clone());
        wanted.insert(fb.clone());
        for (p, _) in loose_ends(&mine, a.diag) {
            wanted.extend(end_faces(list, p, fa, fb, a.diag));
        }
    }
    let mut normals = Normals::default();
    let outward = &mut normals.flat;
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
    for face in pairs {
        if normals.flat.contains_key(&face) {
            continue;
        }
        if let Ok(made) = call("datum.axis", &json!({"face": face})) {
            let ok = made.get("status").and_then(Value::as_str) == Some("ok");
            let datum = made.get("datum");
            if let (true, Some(o), Some(u)) = (
                ok,
                point(datum.and_then(|d| d.get("origin"))),
                point(datum.and_then(|d| d.get("direction"))),
            ) {
                normals.axes.insert(face.clone(), (o, u));
            }
            call("edit.undo", &json!({}))
                .map_err(|e| format!("edit.undo after a datum axis on {face}: {e:?}"))?;
        }
    }
    let mut oriented = 0;
    for a in actions.iter_mut() {
        if let Some(list) = edges.get(&a.body) {
            refine(a, list, &normals);
            orient(a, list, &normals.flat);
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
            turning: 0.0,
            edges: Vec::new(),
            line: None,
        });
        entry.length += l;
        entry.count += 1;
        entry.edges.push(e.clone());
        if e.curve == "line" {
            entry.line = Some((e.start, e.end));
        } else if let Some(t) = (e.curve == "circle").then(|| turning(e)).flatten() {
            entry.turning += t;
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
            volume_bound: None,
            area_bound: None,
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
                        turning,
                        edges: mine,
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
                    let ends = loose_ends(mine, diag).len();
                    let mut expect =
                        blend(rounded(size), *length, *curved, ends, kind == Kind::Chamfer);
                    if let Expect::Blend { turning: t, .. } = &mut expect {
                        *t = *turning;
                    }
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
    /// How far `volume` can be off by the integrator's own account.
    pub volume_bound: Option<f64>,
    /// How far `area` can be off by the same account.
    pub area_bound: Option<f64>,
    /// For a blend whose change the volume cannot tell from nothing:
    /// whether the result shows by other means that it was applied (its
    /// own faces are there and the edges between the pair are gone,
    /// [`blend_applied`]); `None` when not looked at.
    pub applied: Option<bool>,
}

/// What a change of volume after `action` must exceed to be one:
/// relative 1e-9 of the body, or the sum of the integrator's own bounds
/// on the volume before and after when both are known and that is less,
/// so never looser than 1e-9. A change no larger decides nothing either
/// way. The bounds are each measure's own estimate of how far it is off,
/// so their sum bounds a change's error by that same estimate; measured,
/// the jitter of a change that changes nothing (the same body measured
/// again after a move by nothing, its centre and its faces' order new)
/// is far below it (a block 1800 times, the bracket 6000 times, a drum
/// 5600 times below; `a_tiny_blend_on_a_large_body_is_told_by_its_faces`
/// holds the drum's). On freeform bodies the bounds are far looser than
/// 1e-9 (the impeller's: 0.36 of 3067, 1.2e-4, above its offsets' real
/// changes of 0.08 to 0.7), and there 1e-9 stays the threshold.
pub fn noise(action: &Action, after: &Measured) -> f64 {
    let relative = 1e-9 * action.volume.abs();
    match (action.volume_bound, after.volume_bound) {
        (Some(b0), Some(b1)) if b0.is_finite() && b1.is_finite() => {
            relative.min(b0.abs() + b1.abs())
        }
        _ => relative,
    }
}

/// The noise of the area, as [`noise`] of the volume: relative 1e-9 of
/// the body's area, or the integrator's bounds where those are less.
pub fn area_noise(action: &Action, after: &Measured) -> f64 {
    let relative = 1e-9 * action.area.abs();
    match (action.area_bound, after.area_bound) {
        (Some(b0), Some(b1)) if b0.is_finite() && b1.is_finite() => {
            relative.min(b0.abs() + b1.abs())
        }
        _ => relative,
    }
}

/// A bound on a blend's cross-section and its setback without the exact
/// change: a chamfer's triangle of sides `d`, at most `d²/2`; a fillet's
/// `r` times its setback `r·cot(θ/2)`, at the `wedge` θ where known, else
/// at 10°.
fn general_section(size: f64, chamfer: bool, wedge: Option<f64>) -> (f64, f64) {
    if chamfer {
        return (0.5 * size * size, size);
    }
    let per_r = wedge
        .map(|w| 1.0 / (0.5 * w.min(std::f64::consts::PI)).ptan())
        .filter(|x| x.is_finite() && *x >= 0.0)
        .unwrap_or(SETBACK);
    (per_r * size * size, per_r * size)
}

/// Whether the volume cannot tell what `action` made from what it was
/// made from, for a blend: then [`identity`] needs [`Measured::applied`].
pub fn needs_witness(action: &Action, after: &Measured) -> bool {
    matches!(action.expect, Expect::Blend { .. })
        && (after.volume - action.volume).abs() <= noise(action, after)
}

/// The name the body of feature `id`, made from the body a face named
/// `name` is of as its instance 0 (a blend, a face operation), gives the
/// image of that face: `id:copy(0)/name`, with a leading run of instance
/// 0 merged into the new one (docs/design/names.md, *Copies of copies*).
pub fn copied(id: &str, name: &str) -> String {
    let feature = |head: &str| {
        head.strip_prefix('F')
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    };
    let rest = name
        .split_once(":copy(0)/")
        .filter(|(head, _)| feature(head))
        .map_or(name, |(_, rest)| rest);
    format!("{id}:copy(0)/{rest}")
}

/// Evidence besides the volume that the blend `action`, built as feature
/// `id`, was applied: the result has faces of its own (named by `id`, not
/// copies), both faces of the pair are still there under the names the
/// result gives them ([`copied`]), and no edge lies between them any more.
/// The reason when not.
pub fn blend_applied<F>(call: &mut F, action: &Action, id: &str) -> Result<(), String>
where
    F: FnMut(&str, &Value) -> Result<Value, CallError>,
{
    let Some(Pair { faces: [a, b], .. }) = &action.pair else {
        return Err("not an edge-pair action".into());
    };
    let inspect =
        call("document.inspect", &json!({})).map_err(|e| format!("document.inspect: {e:?}"))?;
    let body = inspect
        .get("bodies")
        .and_then(Value::as_array)
        .and_then(|bodies| {
            bodies
                .iter()
                .find(|b| b.get("feature").and_then(Value::as_str) == Some(id))
        })
        .ok_or_else(|| format!("{id} has no body"))?;
    let mut names: BTreeSet<&str> = BTreeSet::new();
    for list in [body.get("faceNames")].into_iter().flatten().chain(
        body.get("faceAliases")
            .and_then(Value::as_array)
            .into_iter()
            .flatten(),
    ) {
        names.extend(
            list.as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        );
    }
    let own = format!("{id}:");
    let copy = format!("{id}:copy(");
    if !names
        .iter()
        .any(|n| n.starts_with(&own) && !n.starts_with(&copy))
    {
        return Err(format!("{id} has no face of its own"));
    }
    let pair = [copied(id, a), copied(id, b)];
    if let Some(gone) = pair.iter().find(|n| !names.contains(n.as_str())) {
        return Err(format!("{id} has no face {gone}"));
    }
    let edges = call(
        "document.edges",
        &json!({"feature": id, "between": [pair[0], pair[1]]}),
    )
    .map_err(|e| format!("document.edges: {e:?}"))?;
    match edges.get("count").and_then(Value::as_u64) {
        Some(0) => Ok(()),
        Some(n) => Err(format!(
            "{n} edges still lie between {} and {}",
            pair[0], pair[1]
        )),
        None => Err(format!("no count in {edges}")),
    }
}

/// Asks the engine once per body of `actions` how far its volume can be
/// off (`document.measure` with `bound`) and gives each action its body's
/// bound. A body whose bound cannot be had keeps none (the noise is then
/// relative 1e-9 of it, [`noise`]).
pub fn bound_all<F>(call: &mut F, actions: &mut [Action])
where
    F: FnMut(&str, &Value) -> Result<Value, CallError>,
{
    let mut bounds: BTreeMap<String, (Option<f64>, Option<f64>)> = BTreeMap::new();
    for a in actions.iter_mut() {
        let (volume, area) = *bounds.entry(a.body.clone()).or_insert_with(|| {
            let m = call(
                "document.measure",
                &json!({"feature": a.body, "bound": true}),
            )
            .ok();
            let get = |key: &str| m.as_ref().and_then(|m| m.get(key)).and_then(num);
            (get("volumeBound"), get("areaBound"))
        });
        a.volume_bound = volume;
        a.area_bound = area;
    }
}

/// The fillet's setback over its radius at the smallest dihedral angle
/// the general bound assumes, 10°: `cot 5°`.
const SETBACK: f64 = 11.430_052_302_761_343;

/// How far an end face at the smallest angle to the edge assumed, 10°,
/// moves the end of a blend, per unit of setback: `cot 10°`.
const END_TILT: f64 = 5.671_281_819_617_709;

/// How much more area of an end face at 10° to the edge a cross-section
/// covers than its own: `1/sin 10°`.
const END_SLANT: f64 = 5.758_770_483_143_634;

/// Whether what was measured after `action` keeps its identity; the
/// failed identity when not.
pub fn identity(action: &Action, after: &Measured) -> Result<(), String> {
    let v0 = action.volume;
    let a0 = action.area;
    let Measured {
        volume,
        area,
        centroid,
        applied,
        ..
    } = *after;
    if !(volume.is_finite() && volume > 0.0) {
        return Err(format!("volume {volume} is not positive"));
    }
    let change = volume - v0;
    // Changes this small are within what the measure itself can be off by
    // (the integrator's own bounds before and after): no conclusion
    // either way.
    let noise = noise(action, after);
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
            turning,
            tilt,
            slant,
            wedge,
            chamfer,
            exact,
            ..
        } => {
            let (section, setback) = general_section(size, chamfer, wedge);
            // Around a curved edge the cross-section travels further than
            // the edge by the setback times the angle the edge turns
            // through (2π where that is not known); past each end it runs
            // on onto the end face, up to the setback times cot γ (a
            // straight edge between curved faces ending on a 45° chamfer,
            // the turbofan's scallop in its flange, runs on by the setback
            // at each end: 2.8 % more than the length alone allows, worked
            // out independently in `a_blend_runs_on_past_slanted_ends`).
            // `curved` is a small count.
            #[allow(clippy::cast_precision_loss)]
            let travel = length
                + setback * (turning + std::f64::consts::TAU * curved as f64)
                + setback * tilt;
            // An exact blend's change, with what its ends may move it by.
            let window = exact.map(|x| {
                let run_on = tilt * x.setback * x.section;
                let expected = x.section * x.length;
                ((expected - run_on).max(0.0), expected + run_on)
            });
            let most = window.map_or(section * travel, |(_, hi)| hi);
            if change.abs() <= noise {
                // The volume cannot tell this blend from nothing. It may
                // do without a change only when all it can change it by
                // is within the noise (a small radius on a large body);
                // then other evidence must show it was applied, and its
                // area must have changed as such a blend changes it.
                if most * (1.0 + 1e-6) > noise {
                    return Err(format!(
                        "a blend left the volume as it was ({v0} to {volume}, within {noise}), but may change it by up to {most}"
                    ));
                }
                match applied {
                    Some(true) => {}
                    Some(false) => {
                        return Err(format!(
                            "a blend left the volume as it was ({v0} to {volume}, within {noise}) and the result does not show it was applied"
                        ));
                    }
                    None => {
                        return Err(format!(
                            "a blend left the volume as it was: {v0} to {volume} (within {noise}; nothing else looked at)"
                        ));
                    }
                }
                let gained = area - a0;
                let room = area_noise(action, after);
                if let Some(x) = exact {
                    // The two faces lose a strip a setback wide each, the
                    // blend's own face is `across` wide; each end face
                    // gains or loses the cross-section (over sin γ), and
                    // past each end the strips and the blend run on by up
                    // to the setback times cot γ.
                    let strip = x.across - 2.0 * x.setback;
                    let expected = x.length * strip;
                    let ends = slant * x.section + strip.abs() * x.setback * tilt;
                    if (gained - expected).abs() > ends * (1.0 + 1e-6) + room {
                        return Err(format!(
                            "a blend changed the area by {gained}, not {expected} ± {ends}"
                        ));
                    }
                } else {
                    // The faces lose strips no wider than the setback, which
                    // the blend's narrower face only partly gives back; the
                    // end faces may gain at most the cross-section each.
                    let (lo, hi) = (-(2.0 * setback * travel + slant * section), slant * section);
                    if gained < lo * (1.0 + 1e-6) - room || gained > hi * (1.0 + 1e-6) + room {
                        return Err(format!(
                            "a blend changed the area by {gained}, not within [{lo}, {hi}]"
                        ));
                    }
                }
                return Ok(());
            }
            if let (Some(x), Some((lo, hi))) = (exact, window) {
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
            at_most("a draft by 3°", 3f64.to_radians().ptan() * diag * larger)
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
    let mut measured = Measured {
        volume,
        area,
        centroid,
        volume_bound: None,
        area_bound: None,
        applied: None,
    };
    // The noise is never more than 1e-9 of the body; only a change below
    // that needs the integrator's bounds, which cost an integration more.
    if (volume - action.volume).abs() <= 1e-9 * action.volume.abs()
        && let Ok(bounded) = call("document.measure", &json!({"feature": id, "bound": true}))
    {
        measured.volume_bound = bounded.get("volumeBound").and_then(num);
        measured.area_bound = bounded.get("areaBound").and_then(num);
    }
    let mut witness = String::new();
    if needs_witness(action, &measured) {
        let applied = blend_applied(call, action, &id);
        measured.applied = Some(applied.is_ok());
        if let Err(why) = applied {
            witness = why;
        }
    }
    let mut verdict = match identity(action, &measured) {
        Ok(()) => outcome(Verdict::Ok, "", false),
        Err(why) if witness.is_empty() => outcome(Verdict::Wrong, why, false),
        Err(why) => outcome(Verdict::Wrong, format!("{why}: {witness}"), false),
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
