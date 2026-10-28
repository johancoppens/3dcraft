//! `sheetMetal.bendSequence`: whether a sheet-metal part can be bent on a
//! press brake with a given punch and die, and in which order
//! (docs/design/sheetmetal.md › *Bend sequence*). A check: it changes
//! nothing in the document.
//!
//! The punch and the die are polygons in the plane square to the bend,
//! extruded along it; at each bend they stand in the bend's frame (the
//! punch's origin on the bend's axis, the die's on the outer skin under
//! it, `y` along the bisector of the two flats towards the inside). The
//! part at a stage is the built part's pieces moved rigidly
//! (`sheetmetal::sequence`). A step (a bend made after a set of others)
//! clears when the proven distance between each tool and every piece but
//! the bend's own strip and its two flats is at least the clearance over
//! the whole stroke: proven at sampled stroke angles, and between two of
//! them less how far the part can move against the tools in between
//! (`stroke_speed`), the interval halved until that holds. The order is
//! searched depth first, the bends whose flanges carry the fewest others
//! first, each failed set of bends made remembered.

use crate::commands::{Run, fail, length_unit, spec};
use crate::params::Params;
use crate::verify::{self, Part as Measured};
use crate::{CommandError, CommandSpec, Session};
use craft3d_kernel::boolean::contains_points;
use craft3d_kernel::sheetmetal::sequence::{
    bend_faces, bend_frame, final_worlds, prism, stage_worlds, strip_slab, stroke_speed,
};
use craft3d_kernel::sheetmetal::{self, flange_length, flat_faces};
use craft3d_math::{Tolerance, Transform, Vec3};
use craft3d_topo::Body;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "sheetMetal.bendSequence",
    "Bend Sequence",
    r#"{"feature": id or name, "punch": [[x, y], ...], "die": [[x, y], ...], "length": number = the part's reach, "clearance": number = the document's tolerance}"#,
    "Checks whether a sheet-metal part (feature.sheetMetal) can be bent on a press brake with the punch and die given, and finds an order of its bends (changes nothing). punch and die are closed polygons (at least 3 points, in the document's length unit) in the plane square to a bend: the punch's with the centre of its tip at the origin and its body along +y, the die's with the point under the bend on the part's outer skin at the origin and its body below (y < 0); each is extruded length along the bend, centred on its middle (default: long enough to cover the whole part). At every bend both stand in the bend's frame: origin on the bend's axis (the die's R + T further out), y along the bisector of the two flats towards the inside (as on a press brake where both legs rise half the angle), z along the axis. A step, a bend made with some others already made and the rest still flat, clears when the proven distance (its lower bound) between each tool and every piece of the part but that bend's own strip and its two flats is at least clearance over the whole stroke from 0 to the bend's angle: proven at eight stroke angles, and between two of them less how far any point of the part can move against the tools in between, the interval halved (to 1/1024 of the stroke) until that holds; a piece wholly inside a tool counts as 0. The order is searched depth first over all orders, the bends whose flanges carry the fewest others first (then the shorter flange, then the lower index), every set of bends made that leads nowhere remembered. Returns feasible, order (the flanges by index, first bent first; null when none clears), steps (per bend of the order: flange, done (the flanges made before it), least (the least proven distance over its stroke) and at (the stroke angle of it, degrees)), collisions (every step tried that did not clear: flange, done, tool (punch or die), piece (the base, flange i, or the strip of flange i, bent or flat), angle (degrees), distance and lower (the distance found and what is proven below it), or notProven with the stroke interval no halving could prove), and work (distances measured). Refused: a part with a hole across a bend (owed), a polygon that crosses itself, a clearance that is not more than 0, and a search of more than 20000 distances. The closing of a bend's own two flats onto the punch (an acute bend with a punch of a wider angle) is not looked for: those flats are the tools' by design.",
    doc: true,
    names: ["feature"],
    bend_sequence
)];

/// Most distances one check measures.
const MAX_WORK: usize = 20_000;

/// Most points in a tool's polygon.
const MAX_POINTS: usize = 1_000;

/// Stroke angles sampled per bend before halving.
const SAMPLES: usize = 8;

/// Deepest an interval of the stroke is halved (1/1024 of it with the 8
/// samples).
const MAX_DEPTH: u32 = 7;

/// Most collisions listed.
const MAX_LISTED: usize = 200;

fn polygon(p: &Params, key: &str) -> Result<Vec<[f64; 2]>, CommandError> {
    let id = p.command();
    let Some(value) = p.get(key) else {
        return Err(p.missing(key));
    };
    let bad = || {
        CommandError::new(format!(
            "{id}: `{key}` must be a list of at least 3 points [x, y] (finite numbers, at most {MAX_POINTS})"
        ))
    };
    let list = value.as_array().ok_or_else(bad)?;
    if list.len() < 3 || list.len() > MAX_POINTS {
        return Err(bad());
    }
    list.iter()
        .map(|q| {
            let pair = q.as_array().filter(|a| a.len() == 2).ok_or_else(bad)?;
            let x = pair
                .first()
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite());
            let y = pair
                .get(1)
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite());
            match (x, y) {
                (Some(x), Some(y)) => Ok([x, y]),
                _ => Err(bad()),
            }
        })
        .collect()
}

/// A piece of the part at a stage: what it is, the faces it is measured
/// by, and which flat's motion carries it.
struct Piece<'a> {
    what: Kind,
    measured: Measured<'a>,
    /// Points of its faces, for whether it lies wholly inside a tool.
    points: Vec<Vec3>,
    /// The flat whose transform from built to stage moves it.
    carrier: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Flat(usize),
    /// Bend `i` as built (made).
    Bend(usize),
    /// Bend `i`'s strip not yet bent.
    Strip(usize),
}

impl Kind {
    fn text(self) -> String {
        match self {
            Kind::Flat(0) => "the base".to_string(),
            Kind::Flat(f) => format!("flange {}", f - 1),
            Kind::Bend(i) => format!("the strip of flange {i}, bent"),
            Kind::Strip(i) => format!("the strip of flange {i}, flat"),
        }
    }
}

fn points_of(m: &Measured<'_>) -> Vec<Vec3> {
    m.triangles
        .values()
        .filter_map(|t| t.first().map(|tri| tri[0]))
        .collect()
}

#[derive(Clone, Debug)]
enum Failure {
    Collision {
        tool: &'static str,
        piece: Kind,
        angle: f64,
        distance: f64,
        lower: f64,
    },
    NotProven {
        from: f64,
        to: f64,
    },
}

#[derive(Clone, Copy, Debug)]
struct Cleared {
    least: f64,
    at: f64,
}

struct Check<'a> {
    part: &'a sheetmetal::Part,
    tol: Tolerance,
    punch: Vec<[f64; 2]>,
    die: Vec<[f64; 2]>,
    length: Option<f64>,
    clearance: f64,
    flats: Vec<Piece<'a>>,
    bends: Vec<Piece<'a>>,
    strips: Vec<Piece<'a>>,
    built: Vec<Transform>,
    work: std::cell::Cell<usize>,
}

impl Check<'_> {
    /// The least proven distance between the tools and the pieces at
    /// bend `b`'s stroke angle `rho`, with the others made (`done`) or
    /// flat; a failure when it is below the clearance.
    fn at(
        &self,
        done: &BTreeSet<usize>,
        b: usize,
        rho: f64,
    ) -> Result<Result<f64, Failure>, String> {
        let part = self.part;
        let bend = part.bends.get(b).ok_or("a bend is missing")?;
        let angles: Vec<f64> = part
            .bends
            .iter()
            .enumerate()
            .map(|(i, x)| {
                if done.contains(&i) {
                    x.angle
                } else if i == b {
                    rho * x.angle.signum()
                } else {
                    0.0
                }
            })
            .collect();
        let worlds = stage_worlds(part, &angles, &self.tol).map_err(|e| e.to_string())?;
        let frame = bend_frame(part, b, rho, &worlds).map_err(|e| e.to_string())?;
        let speed = stroke_speed(part, b).ok_or("the part has no finite size")?;
        let length = self.length.unwrap_or(2.0 * speed);
        let exempt = |k: Kind| match k {
            Kind::Flat(f) => f == bend.parent || f == bend.child,
            Kind::Bend(i) | Kind::Strip(i) => i == b,
        };
        let mut least = f64::INFINITY;
        for (tool, points, below) in [
            ("punch", &self.punch, 0.0),
            ("die", &self.die, frame.radius + frame.thickness),
        ] {
            let tool_frame = frame.tool(below).ok_or("a tool's frame is not finite")?;
            let pieces = self
                .flats
                .iter()
                .chain(
                    self.bends
                        .iter()
                        .filter(|p| matches!(p.what, Kind::Bend(i) if done.contains(&i))),
                )
                .chain(
                    self.strips
                        .iter()
                        .filter(|p| matches!(p.what, Kind::Strip(i) if !done.contains(&i))),
                );
            for piece in pieces {
                if exempt(piece.what) {
                    continue;
                }
                self.work.set(self.work.get().saturating_add(1));
                if self.work.get() > MAX_WORK {
                    return Err(format!(
                        "more than {MAX_WORK} distances measured: the search over orders is too large (many bends, most orders colliding or close to it)"
                    ));
                }
                let (Some(stage), Some(built)) =
                    (worlds.get(piece.carrier), self.built.get(piece.carrier))
                else {
                    return Err("a flat's place is missing".into());
                };
                // The tool where the piece stands in the built part.
                let back = stage
                    .inverse()
                    .ok_or("a stage's place cannot be undone")?
                    .then(built);
                let local = back
                    .frame(&tool_frame)
                    .ok_or("a tool's frame is not finite")?;
                let body: Body = prism(points, &local, -0.5 * length, 0.5 * length, &self.tol)
                    .map_err(|e| format!("the {tool}: {e}"))?;
                let measured = Measured::new(&body, None, &self.tol)?;
                let near = verify::distance(&[measured], std::slice::from_ref(&piece.measured))?;
                let inside = contains_points(&body, &piece.points, &self.tol)
                    .map_err(|e| e.to_string())?
                    .iter()
                    .any(|x| *x == Some(true));
                let lower = if inside { 0.0 } else { near.lower.max(0.0) };
                if lower < self.clearance {
                    return Ok(Err(Failure::Collision {
                        tool,
                        piece: piece.what,
                        angle: rho,
                        distance: if inside { 0.0 } else { near.distance },
                        lower,
                    }));
                }
                least = least.min(lower);
            }
        }
        Ok(Ok(least))
    }

    /// Whether bend `b` can be made with `done` made: over its whole
    /// stroke.
    fn step(&self, done: &BTreeSet<usize>, b: usize) -> Result<Result<Cleared, Failure>, String> {
        let part = self.part;
        let theta = part.bends.get(b).ok_or("a bend is missing")?.theta();
        let speed = stroke_speed(part, b).ok_or("the part has no finite size")?;
        let mut best = Cleared {
            least: f64::INFINITY,
            at: 0.0,
        };
        let mut prev: Option<(f64, f64)> = None;
        for j in 0..=SAMPLES {
            let rho = theta * j as f64 / SAMPLES as f64;
            let l = match self.at(done, b, rho)? {
                Ok(l) => l,
                Err(f) => return Ok(Err(f)),
            };
            if l < best.least {
                best = Cleared { least: l, at: rho };
            }
            if let Some((a, la)) = prev
                && let Err(f) = self.between(done, b, (a, la), (rho, l), speed, 0, &mut best)?
            {
                return Ok(Err(f));
            }
            prev = Some((rho, l));
        }
        Ok(Ok(best))
    }

    /// Proves the stroke between two angles whose distances are proven,
    /// halving it where the part could come closer in between.
    #[allow(clippy::too_many_arguments)]
    fn between(
        &self,
        done: &BTreeSet<usize>,
        b: usize,
        (a, la): (f64, f64),
        (c, lc): (f64, f64),
        speed: f64,
        depth: u32,
        best: &mut Cleared,
    ) -> Result<Result<(), Failure>, String> {
        if la.min(lc) - speed * (c - a) * 0.5 >= self.clearance {
            return Ok(Ok(()));
        }
        if depth >= MAX_DEPTH {
            return Ok(Err(Failure::NotProven { from: a, to: c }));
        }
        let m = 0.5 * (a + c);
        let lm = match self.at(done, b, m)? {
            Ok(l) => l,
            Err(f) => return Ok(Err(f)),
        };
        if lm < best.least {
            *best = Cleared { least: lm, at: m };
        }
        if let Err(f) = self.between(done, b, (a, la), (m, lm), speed, depth + 1, best)? {
            return Ok(Err(f));
        }
        self.between(done, b, (m, lm), (c, lc), speed, depth + 1, best)
    }
}

/// How many flanges flange `i` carries (on its tip, its sides, and on).
fn carried(part: &sheetmetal::Part, i: usize) -> usize {
    let mut count = 0usize;
    let mut stack = vec![i + 1];
    let mut seen = BTreeSet::new();
    while let Some(flat) = stack.pop() {
        if !seen.insert(flat) {
            continue;
        }
        for b in part.bends.iter().filter(|b| b.parent == flat) {
            count = count.saturating_add(1);
            stack.push(b.child);
        }
    }
    count
}

type Tried = BTreeMap<(BTreeSet<usize>, usize), Result<Cleared, Failure>>;

/// The first order (depth first) whose every step clears.
fn search(
    check: &Check<'_>,
    done: &mut BTreeSet<usize>,
    order: &mut Vec<(usize, BTreeSet<usize>, Cleared)>,
    dead: &mut BTreeSet<BTreeSet<usize>>,
    tried: &mut Tried,
    ranked: &[usize],
) -> Result<bool, String> {
    if done.len() == ranked.len() {
        return Ok(true);
    }
    if dead.contains(done) {
        return Ok(false);
    }
    for &b in ranked {
        if done.contains(&b) {
            continue;
        }
        let key = (done.clone(), b);
        let result = match tried.get(&key) {
            Some(r) => r.clone(),
            None => {
                let r = check.step(done, b)?;
                tried.insert(key, r.clone());
                r
            }
        };
        if let Ok(cleared) = result {
            order.push((b, done.clone(), cleared));
            done.insert(b);
            if search(check, done, order, dead, tried, ranked)? {
                return Ok(true);
            }
            done.remove(&b);
            order.pop();
        }
    }
    dead.insert(done.clone());
    Ok(false)
}

fn degrees(r: f64) -> f64 {
    r.to_degrees()
}

fn bend_sequence(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "sheetMetal.bendSequence";
    let p = Params::new(
        ID,
        params,
        &["feature", "punch", "die", "length", "clearance"],
    )?;
    let punch = polygon(&p, "punch")?;
    let die = polygon(&p, "die")?;
    let length = p.number("length")?;
    if let Some(l) = length
        && !(l.is_finite() && l > 0.0)
    {
        return fail(format!("{ID}: `length` must be more than 0, got {l}"));
    }
    let clearance = p.number("clearance")?;
    if let Some(c) = clearance
        && !(c.is_finite() && c > 0.0)
    {
        return fail(format!("{ID}: `clearance` must be more than 0, got {c}"));
    }
    let feature = {
        let doc = session.document()?.history.current();
        crate::commands::feature_ref(doc, &p, ID, "feature")?
    };
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let built = craft3d_regen::sheet_metal_part(doc, &regen, feature)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let part = &built.part;
    let tol = built.tolerance;
    if !part.cuts.is_empty() {
        return fail(format!(
            "{ID}: the part has a hole across a bend; a strip not yet bent with a hole in it is not built as a piece (not supported)"
        ));
    }
    let clearance = clearance.unwrap_or(tol.linear);
    // The tools must be simple outlines: checked once here, so a bad one
    // is an error before any search.
    let unit = craft3d_math::Frame::WORLD;
    for (name, points) in [("punch", &punch), ("die", &die)] {
        // As tall as the polygon is wide, so the check holds at any scale.
        let size = points
            .iter()
            .flat_map(|q| q.iter())
            .fold(0.0_f64, |m, v| m.max(v.abs()))
            .max(tol.linear * 1e3);
        prism(points, &unit, 0.0, size, &tol)
            .map_err(|e| CommandError::new(format!("{ID}: `{name}`: {e}")))?;
    }
    let body = &part.solid.body;
    let measured = |faces: Vec<usize>| -> Result<Option<(Measured<'_>, Vec<Vec3>)>, CommandError> {
        if faces.is_empty() {
            return Ok(None);
        }
        let m = Measured::new(body, Some(&faces), &tol)
            .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
        let points = points_of(&m);
        Ok(Some((m, points)))
    };
    let mut flats = Vec::new();
    for f in 0..part.flats.len() {
        if let Some((m, points)) = measured(flat_faces(part, f))? {
            flats.push(Piece {
                what: Kind::Flat(f),
                measured: m,
                points,
                carrier: f,
            });
        }
    }
    let mut bends = Vec::new();
    for (i, b) in part.bends.iter().enumerate() {
        if let Some((m, points)) = measured(bend_faces(part, i))? {
            bends.push(Piece {
                what: Kind::Bend(i),
                measured: m,
                points,
                carrier: b.parent,
            });
        }
    }
    let slabs: Vec<Body> = (0..part.bends.len())
        .map(|i| strip_slab(part, i, &tol))
        .collect::<Result<_, _>>()
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let mut strips = Vec::new();
    for (i, (b, slab)) in part.bends.iter().zip(&slabs).enumerate() {
        let m =
            Measured::new(slab, None, &tol).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
        let points = points_of(&m);
        strips.push(Piece {
            what: Kind::Strip(i),
            measured: m,
            points,
            carrier: b.parent,
        });
    }
    let check = Check {
        part,
        tol,
        punch,
        die,
        length,
        clearance,
        flats,
        bends,
        strips,
        built: final_worlds(part),
        work: std::cell::Cell::new(0),
    };
    let mut ranked: Vec<usize> = (0..part.bends.len()).collect();
    ranked.sort_by(|&a, &b| {
        carried(part, a)
            .cmp(&carried(part, b))
            .then(
                flange_length(part, a)
                    .unwrap_or(f64::INFINITY)
                    .total_cmp(&flange_length(part, b).unwrap_or(f64::INFINITY)),
            )
            .then(a.cmp(&b))
    });
    let mut done = BTreeSet::new();
    let mut order = Vec::new();
    let mut dead = BTreeSet::new();
    let mut tried: Tried = BTreeMap::new();
    let feasible = search(
        &check, &mut done, &mut order, &mut dead, &mut tried, &ranked,
    )
    .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let steps: Vec<Value> = order
        .iter()
        .map(|(b, before, c)| {
            json!({
                "flange": b,
                "done": before.iter().collect::<Vec<_>>(),
                "least": if c.least.is_finite() { json!(c.least) } else { Value::Null },
                "at": degrees(c.at),
            })
        })
        .collect();
    let collisions: Vec<Value> = tried
        .iter()
        .filter_map(|((before, b), r)| r.as_ref().err().map(|f| (before, b, f)))
        .take(MAX_LISTED)
        .map(|(before, b, f)| {
            let mut v = json!({
                "flange": b,
                "done": before.iter().collect::<Vec<_>>(),
            });
            match f {
                Failure::Collision {
                    tool,
                    piece,
                    angle,
                    distance,
                    lower,
                } => {
                    v["tool"] = json!(tool);
                    v["piece"] = json!(piece.text());
                    v["angle"] = json!(degrees(*angle));
                    v["distance"] = json!(distance);
                    v["lower"] = json!(lower);
                }
                Failure::NotProven { from, to } => {
                    v["notProven"] = json!([degrees(*from), degrees(*to)]);
                }
            }
            v
        })
        .collect();
    Ok(json!({
        "feature": feature.to_string(),
        "feasible": feasible,
        "order": if feasible { json!(order.iter().map(|(b, _, _)| b).collect::<Vec<_>>()) } else { Value::Null },
        "steps": if feasible { json!(steps) } else { json!([]) },
        "collisions": collisions,
        "clearance": clearance,
        "work": check.work.get(),
        "lengthUnit": length_unit(doc),
    }))
}
