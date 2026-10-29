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
//! clears when the proven distance between the punch and every piece but
//! the bend's own strip and its two flats, and between the die lowered by
//! the clearance and the tolerance (so the sheet may rest on it) and
//! every piece but the bend's own strip, is at least the clearance over
//! the whole stroke: proven at sampled stroke angles, and between two of
//! them less how far the part can move against the tools in between
//! (`stroke_speed`, from the pieces' farthest reach from the axis), the
//! interval halved until that holds. The order is
//! searched depth first, the bends whose flanges carry the fewest others
//! first, each failed set of bends made remembered.

use crate::commands::{Run, fail, length_unit, spec};
use crate::params::Params;
use crate::verify::{self, Part as Measured};
use crate::verify_cmds::{Ctx, Dim, Outcome};
use crate::{CommandError, CommandSpec, Session};
use craft3d_kernel::boolean::contains_points;
use craft3d_kernel::sheetmetal::sequence::{
    PieceOf, bend_faces, bend_frame, final_worlds, from_axis, inside_piece, piece_lowest,
    piece_points, prism, stage_worlds, strip_slab, stroke_speed,
};
use craft3d_kernel::sheetmetal::{self, flange_length, flat_faces};
use craft3d_math::{Tolerance, Transform, Vec3};
use craft3d_topo::Body;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "sheetMetal.bendSequence",
    "Bend Sequence",
    r#"{"feature": id or name, "punch": [[x, y], ...], "die": [[x, y], ...], "length": number = the part's reach, "clearance": number = the document's tolerance}"#,
    "Checks whether a sheet-metal part (feature.sheetMetal) can be bent on a press brake with the punch and die given, and finds an order of its bends (changes nothing). punch and die are closed polygons (at least 3 points, in the document's length unit) in the plane square to a bend: the punch's with the centre of its tip at the origin and its body along +y, the die's with the point under the bend on the part's outer skin at the origin and its body below (y < 0); each is extruded length along the bend, centred on its middle (default: long enough to cover the whole part). At every bend both stand in the bend's frame: origin on the bend's axis (the die's R + T further out), y along the bisector of the two flats towards the inside (as on a press brake where both legs rise half the angle), z along the axis. A step, a bend made with some others already made and the rest still flat, clears when the proven distance (its lower bound) between the punch and every piece of the part but that bend's own strip and its two flats, and between the die counted lowered (moved away from the punch) by clearance plus the document's tolerance and every piece but the bend's own strip, is at least clearance over the whole stroke from 0 to the bend's angle (so the sheet may rest on the die's top, but not go into it by more than the tolerance nor come nearer than clearance from any other side), with the part as it lies or turned end for end (about the tools' y, as an operator turns it: the tools mirrored in x): proven at eight stroke angles, and between two of them less how far any point of the part can move against the tools in between (half its distance from the bend's axis, the farthest at either end of the stroke, plus the bend's neutral radius, per radian), the interval halved (to 1/1024 of the stroke) until that holds, or, for a die wholly below its top (y <= 0), by the piece's lowest point staying above the die's top less the tolerance between them (its height bounded by its heights there and how fast it can bend, halved as needed: what proves a piece lifting off the die); a piece and a tool wholly inside one another count as 0. The order is searched depth first over all orders, the bends whose flanges carry the fewest others first (then the shorter flange, then the lower index), every set of bends made that leads nowhere remembered. Returns feasible, order (the flanges by index, first bent first; null when none clears), steps (per bend of the order: flange, done (the flanges made before it), least (the least distance to the punch proven at the stroke angles measured), at (the stroke angle of it, degrees), proven (the least distance to the punch proven over the whole stroke, between the angles measured too) die (least, at and proven for the lowered die) and turned (whether the part is turned end for end for it)), collisions (every step tried that did not clear: flange, done, tool (punch or die), piece (the base, flange i, or the strip of flange i, bent or flat), angle (degrees), distance and lower (the distance found and what is proven below it; to the lowered die for the die), or notProven with the stroke interval no halving could prove; certain: whether it fails for certain, a tool and a piece found nearer than the clearance at a real place or wholly inside one another, where a collision only not proven clear is not, both as the part lies and turned; turned: whether the failure listed is the turned one, the one not certain when only one is), infeasibleProven (with feasible false: whether every step tried that failed failed for certain, so that no order exists; false means no order was found but one may exist), unchecked (what is not looked at) and work (distances measured). Refused: a part with a hole across a bend (owed), a polygon that crosses itself, a clearance that is not more than 0, and a search of more than 20000 distances. Not looked at (said in unchecked): a bend's own strip during its stroke, and its own two flats against the punch (an acute bend with a punch of a wider angle closing its flats onto it is not found).",
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

/// What the check does not look at, said in its result.
const UNCHECKED: &str = "each bend's own strip during its stroke (against both tools), and its two flats against the punch (its tip forms the bend between them)";

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
    /// Points whose hull holds its material exactly, in the built part
    /// (`sequence::piece_points`).
    corners: Vec<Vec3>,
    /// The flat whose transform from built to stage moves it.
    carrier: usize,
}

impl<'a> Piece<'a> {
    fn new(part: &sheetmetal::Part, what: Kind, measured: Measured<'a>, carrier: usize) -> Self {
        Piece {
            what,
            points: points_of(&measured),
            corners: piece_points(part, what.piece()),
            measured,
            carrier,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
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

    fn piece(self) -> PieceOf {
        match self {
            Kind::Flat(f) => PieceOf::Flat(f),
            Kind::Bend(i) => PieceOf::Bend(i),
            Kind::Strip(i) => PieceOf::Strip(i),
        }
    }
}

fn points_of(m: &Measured<'_>) -> Vec<Vec3> {
    m.triangles
        .values()
        .filter_map(|t| t.first().map(|tri| tri[0]))
        .collect()
}

/// The tools, in the order they are checked.
const PUNCH: usize = 0;
const DIE: usize = 1;
const TOOLS: [&str; 2] = ["punch", "die"];

#[derive(Clone, Debug)]
enum Failure {
    Collision {
        tool: &'static str,
        piece: Kind,
        angle: f64,
        distance: f64,
        lower: f64,
        /// Found with the part turned end for end (the tools mirrored).
        turned: bool,
    },
    NotProven {
        from: f64,
        to: f64,
        turned: bool,
    },
}

impl Failure {
    /// Whether the step fails for certain: a tool and a piece found at a
    /// real pair of places (or wholly inside one another) nearer than the
    /// clearance, so the true distance is nearer too. A collision only
    /// not proven clear (the distance found at least the clearance, what
    /// is proven below it), or an interval of the stroke no halving
    /// proved, is not.
    fn certain(&self, clearance: f64) -> bool {
        matches!(self, Failure::Collision { distance, .. } if *distance < clearance)
    }

    /// The failure in words, for a step: flange `b` after `done`.
    fn words(&self, b: usize, done: &BTreeSet<usize>, clearance: f64) -> String {
        let step = if done.is_empty() {
            format!("flange {b} first")
        } else {
            let before: Vec<String> = done.iter().map(ToString::to_string).collect();
            format!("flange {b} after {}", before.join(", "))
        };
        let step = match self {
            Failure::Collision { turned: true, .. } | Failure::NotProven { turned: true, .. } => {
                format!("{step} (the part turned end for end)")
            }
            _ => step,
        };
        match self {
            Failure::Collision {
                tool,
                piece,
                angle,
                distance,
                ..
            } if self.certain(clearance) => format!(
                "{step}: the {tool} meets {} at {:.3} degrees ({} apart, the clearance {})",
                piece.text(),
                degrees(*angle),
                crate::assert_cmds::shown(*distance),
                crate::assert_cmds::shown(clearance)
            ),
            Failure::Collision {
                tool,
                piece,
                angle,
                distance,
                lower,
                ..
            } => format!(
                "{step}: the {tool} and {} at {:.3} degrees, {} found apart and proven only above {}",
                piece.text(),
                degrees(*angle),
                crate::assert_cmds::shown(*distance),
                crate::assert_cmds::shown(*lower)
            ),
            Failure::NotProven { from, to, .. } => format!(
                "{step}: the stroke between {:.3} and {:.3} degrees is proven neither clear nor colliding",
                degrees(*from),
                degrees(*to)
            ),
        }
    }
}

/// Per tool: the least distance proven at the stroke angles measured,
/// the angle of it, and the least distance proven over the whole stroke
/// (each interval's `min(la, lc) − v h / 2`).
#[derive(Clone, Copy, Debug)]
struct Cleared {
    least: [f64; 2],
    at: [f64; 2],
    proven: [f64; 2],
    /// Made with the part turned end for end (the tools mirrored).
    turned: bool,
}

/// What a step's stroke needs besides the stages: how fast the part can
/// move against the tools per radian (`stroke_speed`), how fast that
/// motion can turn (`bow`: no point's acceleration against the tools per
/// radian² is more), how long the tools are, and whether the die lies
/// wholly below its top (`y ≤ 0`), so the rest proof holds.
#[derive(Clone, Copy, Debug)]
struct Stroke {
    speed: f64,
    bow: f64,
    length: f64,
    die_below: bool,
}

/// Per tool and piece checked: the distance proven at one stroke angle.
type Lowers = BTreeMap<(usize, Kind), f64>;

/// Deepest the rest proof halves an interval (below the stroke's
/// sampling and halving).
const MAX_REST_DEPTH: u32 = 40;

/// A tool's outline in the plane square to a bend.
type Polygon = Vec<[f64; 2]>;

struct Check<'a> {
    part: &'a sheetmetal::Part,
    tol: Tolerance,
    punch: Vec<[f64; 2]>,
    die: Vec<[f64; 2]>,
    /// The tools mirrored in `x` (`turned_tools`): where they stand
    /// against the part turned end for end; `None` when both are
    /// symmetric, so turning changes nothing.
    turned_tools: Option<(Polygon, Polygon)>,
    /// Whether the step being checked has the part turned (`Check::step`).
    turned: std::cell::Cell<bool>,
    length: Option<f64>,
    clearance: f64,
    pieces: Vec<Piece<'a>>,
    built: Vec<Transform>,
    work: std::cell::Cell<usize>,
    /// Most distances and heights this check measures.
    max_work: usize,
    /// Whether it stopped for measuring more than `max_work`.
    ran_out: std::cell::Cell<bool>,
}

impl Check<'_> {
    /// The stage's bend angles: those made at theirs, `b` at `rho`, the
    /// rest flat.
    fn angles(&self, done: &BTreeSet<usize>, b: usize, rho: f64) -> Vec<f64> {
        self.part
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
            .collect()
    }

    /// The pieces tool `tool` is checked against at bend `b` with `done`
    /// made: every piece of the stage (the flats, the bends made, the
    /// strips not yet bent) but `b`'s own strip (half rolled, half flat
    /// on its child during the stroke: not a piece), and for the punch
    /// not `b`'s two flats either (its tip forms the bend between them).
    fn checked<'s>(
        &'s self,
        done: &'s BTreeSet<usize>,
        b: usize,
        tool: usize,
    ) -> impl Iterator<Item = &'s Piece<'s>> + 's {
        let own = self
            .part
            .bends
            .get(b)
            .map(|x| (x.parent, x.child))
            .unwrap_or((usize::MAX, usize::MAX));
        self.pieces.iter().filter(move |p| match p.what {
            Kind::Flat(f) => tool != PUNCH || (f != own.0 && f != own.1),
            Kind::Bend(i) => done.contains(&i),
            Kind::Strip(i) => i != b && !done.contains(&i),
        })
    }

    /// The speed bound and the tools' length of bend `b`'s stroke with
    /// `done` made: the farthest any corner of a checked piece's box lies
    /// from the axis at either end of the stroke (`stroke_speed`), and
    /// (unless given) twice the farthest one lies along it from the
    /// bend's middle, so the tools reach past every piece.
    fn stroke(&self, done: &BTreeSet<usize>, b: usize) -> Result<Stroke, String> {
        let part = self.part;
        let theta = part.bends.get(b).ok_or("a bend is missing")?.theta();
        let start =
            stage_worlds(part, &self.angles(done, b, 0.0), &self.tol).map_err(|e| e.to_string())?;
        let end = stage_worlds(part, &self.angles(done, b, theta), &self.tol)
            .map_err(|e| e.to_string())?;
        let frame = bend_frame(part, b, 0.0, &start).map_err(|e| e.to_string())?;
        let mut reach = 0.0_f64;
        let mut along = 0.0_f64;
        let mut seen = BTreeSet::new();
        for piece in self
            .checked(done, b, PUNCH)
            .chain(self.checked(done, b, DIE))
        {
            if !seen.insert(piece.what) {
                continue;
            }
            let built = self
                .built
                .get(piece.carrier)
                .and_then(Transform::inverse)
                .ok_or("a flat's place cannot be undone")?;
            for worlds in [&start, &end] {
                let stage = worlds
                    .get(piece.carrier)
                    .ok_or("a flat's place is missing")?;
                let moved = built.then(stage);
                if piece.corners.is_empty() {
                    return Err(format!("{} has no points to bound it", piece.what.text()));
                }
                for &c in &piece.corners {
                    let (d, a) = from_axis(frame.point, frame.axis, moved.point(c));
                    reach = reach.max(d);
                    along = along.max(a.abs());
                }
            }
        }
        let speed = stroke_speed(part, b, reach).ok_or("the part has no finite size")?;
        let length = self
            .length
            .unwrap_or(2.0 * (along + self.tol.linear).max(frame.reach));
        if !(length.is_finite() && length > 0.0) {
            return Err("the part has no finite size".into());
        }
        // Against the tools a parent point turns at rate ½ about the axis
        // (acceleration d/4) and a child point also takes up the shift
        // back across the strip, `ρ r` turned with it (r more): no point
        // of the pieces accelerates faster than `reach/4 + r`.
        let bow = 0.25 * reach + part.bends.get(b).map_or(0.0, |x| x.neutral());
        Ok(Stroke {
            speed,
            bow,
            length,
            die_below: self.die.iter().all(|p| p[1] <= 0.0),
        })
    }

    /// The least proven distance between each tool and the pieces it is
    /// checked against at bend `b`'s stroke angle `rho`, with the others
    /// made (`done`) or flat; a failure when one is below the clearance.
    fn at(
        &self,
        done: &BTreeSet<usize>,
        b: usize,
        rho: f64,
        stroke: Stroke,
    ) -> Result<Result<Lowers, Failure>, String> {
        let part = self.part;
        let worlds =
            stage_worlds(part, &self.angles(done, b, rho), &self.tol).map_err(|e| e.to_string())?;
        let frame = bend_frame(part, b, rho, &worlds).map_err(|e| e.to_string())?;
        let mut least = Lowers::new();
        let mut doubtful: Option<Failure> = None;
        let turned = self.turned.get();
        let (punch, die) = match &self.turned_tools {
            Some((punch, die)) if turned => (punch, die),
            _ => (&self.punch, &self.die),
        };
        for (tool, name) in TOOLS.iter().enumerate() {
            let (points, below) = if tool == PUNCH {
                (punch, 0.0)
            } else {
                // The die counted lowered (away from the punch) by the
                // clearance and the tolerance: the sheet rests on its top
                // (its outer skin on the die's top, from the punch's side,
                // at distance 0), which then measures the clearance and
                // the tolerance and clears, while a piece that goes into
                // the die by more than the tolerance, or comes nearer than
                // the clearance from any other side, does not.
                (
                    die,
                    frame.radius + frame.thickness + self.clearance + self.tol.linear,
                )
            };
            let tool_frame = frame.tool(below).ok_or("a tool's frame is not finite")?;
            let pieces: Vec<&Piece<'_>> = self.checked(done, b, tool).collect();
            // The tool carried into each carrier's built place, built once
            // per carrier (every piece it carries shares it).
            let carriers: Vec<usize> = pieces
                .iter()
                .map(|p| p.carrier)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let mut bodies: Vec<(Body, craft3d_math::Frame)> = Vec::with_capacity(carriers.len());
            for &carrier in &carriers {
                let (Some(stage), Some(built)) = (worlds.get(carrier), self.built.get(carrier))
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
                let body = prism(
                    points,
                    &local,
                    -0.5 * stroke.length,
                    0.5 * stroke.length,
                    &self.tol,
                )
                .map_err(|e| format!("the {name}: {e}"))?;
                bodies.push((body, local));
            }
            let measured: Vec<Measured<'_>> = bodies
                .iter()
                .map(|(body, _)| Measured::new(body, None, &self.tol))
                .collect::<Result<_, _>>()?;
            for piece in pieces {
                self.work.set(self.work.get().saturating_add(1));
                if self.work.get() > self.max_work {
                    self.ran_out.set(true);
                    return Err(format!(
                        "more than {} distances measured: the search over orders is too large (many bends, most orders colliding or close to it)",
                        self.max_work
                    ));
                }
                let k = carriers
                    .binary_search(&piece.carrier)
                    .map_err(|_| "a piece's carrier is missing")?;
                let (Some((body, local)), Some(tool_m)) = (bodies.get(k), measured.get(k)) else {
                    return Err("a tool is missing".into());
                };
                let near = verify::distance(
                    std::slice::from_ref(tool_m),
                    std::slice::from_ref(&piece.measured),
                )?;
                // Wholly inside one another: a point of the piece inside
                // the tool, or a point of the tool inside the piece.
                let tool_point = points
                    .first()
                    .map(|q| local.point(q[0], q[1], 0.0))
                    .ok_or("a tool has no points")?;
                let inside = inside_piece(part, piece.what.piece(), tool_point)
                    || contains_points(body, &piece.points, &self.tol)
                        .map_err(|e| e.to_string())?
                        .contains(&Some(true));
                let mut lower = if inside { 0.0 } else { near.lower.max(0.0) };
                // A piece lying on a die below its top: what the mesh
                // cannot prove (a bend's skin along the die's top), its
                // lowest point's height does (`Check::rest`).
                if !inside && lower < self.clearance && tool == DIE && stroke.die_below {
                    let low = self
                        .heights(done, b, rho, &[piece.what])?
                        .first()
                        .copied()
                        .unwrap_or(f64::NEG_INFINITY);
                    if low >= -self.tol.linear {
                        lower = low + self.clearance + self.tol.linear;
                    }
                }
                if lower < self.clearance {
                    let failure = Failure::Collision {
                        tool: name,
                        piece: piece.what,
                        angle: rho,
                        distance: if inside { 0.0 } else { near.distance },
                        lower,
                        turned,
                    };
                    // A collision found at a real place is the answer; one
                    // only not proven clear is kept while the other
                    // pieces are looked at for a real one.
                    if failure.certain(self.clearance) {
                        return Ok(Err(failure));
                    }
                    doubtful.get_or_insert(failure);
                    continue;
                }
                least.insert((tool, piece.what), lower);
            }
        }
        if let Some(failure) = doubtful {
            return Ok(Err(failure));
        }
        Ok(Ok(least))
    }

    /// Whether bend `b` can be made with `done` made, the part as it
    /// lies or turned end for end. A press brake lets the operator turn
    /// the part about the brake's vertical (the tools' `y`) between
    /// bends; for tools extruded symmetrically along the bend (they are,
    /// centred on its middle) that is the tools mirrored in `x`. So the
    /// step clears when either clears, and fails for certain only when
    /// both do; otherwise the failure reported is one not certain.
    fn step(&self, done: &BTreeSet<usize>, b: usize) -> Result<Result<Cleared, Failure>, String> {
        self.turned.set(false);
        let given = self.step_as(done, b)?;
        let Err(failure) = given else {
            return Ok(given);
        };
        if self.turned_tools.is_none() {
            return Ok(Err(failure));
        }
        self.turned.set(true);
        let turned = self.step_as(done, b);
        self.turned.set(false);
        Ok(turn_or(failure, turned?, self.clearance))
    }

    /// Whether bend `b` can be made with `done` made over its whole
    /// stroke, the part turned as `turned` says.
    fn step_as(
        &self,
        done: &BTreeSet<usize>,
        b: usize,
    ) -> Result<Result<Cleared, Failure>, String> {
        let part = self.part;
        let theta = part.bends.get(b).ok_or("a bend is missing")?.theta();
        let stroke = self.stroke(done, b)?;
        let mut best = Cleared {
            least: [f64::INFINITY; 2],
            at: [0.0; 2],
            proven: [f64::INFINITY; 2],
            turned: self.turned.get(),
        };
        let mut prev: Option<(f64, Lowers)> = None;
        for j in 0..=SAMPLES {
            let rho = theta * j as f64 / SAMPLES as f64;
            let l = match self.at(done, b, rho, stroke)? {
                Ok(l) => l,
                Err(f) => return Ok(Err(f)),
            };
            best.sampled(rho, &l);
            if let Some((a, la)) = &prev
                && let Err(f) = self.between(done, b, (*a, la), (rho, &l), stroke, 0, &mut best)?
            {
                return Ok(Err(f));
            }
            prev = Some((rho, l));
        }
        Ok(Ok(best))
    }

    /// Proves the stroke between two angles whose distances are proven,
    /// halving it where the part could come closer in between: per tool
    /// and piece, both distances less how far the piece can move in
    /// between, or for the die the rest proof ([`Check::rest`]).
    #[allow(clippy::too_many_arguments)]
    fn between(
        &self,
        done: &BTreeSet<usize>,
        b: usize,
        (a, la): (f64, &Lowers),
        (c, lc): (f64, &Lowers),
        stroke: Stroke,
        depth: u32,
        best: &mut Cleared,
    ) -> Result<Result<(), Failure>, String> {
        let slack = stroke.speed * (c - a) * 0.5;
        let mut proven = [f64::INFINITY; 2];
        let mut resting = Vec::new();
        for (key, l) in la {
            let lo = l.min(lc.get(key).copied().unwrap_or(f64::NEG_INFINITY)) - slack;
            if lo >= self.clearance {
                if let Some(p) = proven.get_mut(key.0) {
                    *p = p.min(lo);
                }
            } else if key.0 == DIE && stroke.die_below {
                resting.push(key.1);
            } else {
                resting.clear();
                proven = [f64::NEG_INFINITY; 2];
                break;
            }
        }
        if proven.iter().all(|p| *p >= self.clearance) {
            let rested = if resting.is_empty() {
                Some(f64::INFINITY)
            } else {
                let ya = self.heights(done, b, a, &resting)?;
                let yc = self.heights(done, b, c, &resting)?;
                self.rest(done, b, &resting, (a, &ya), (c, &yc), stroke, 0)?
            };
            if let Some(r) = rested {
                for (best, p) in best.proven.iter_mut().zip(proven) {
                    *best = best.min(p);
                }
                if let Some(d) = best.proven.get_mut(DIE) {
                    *d = d.min(r);
                }
                return Ok(Ok(()));
            }
        }
        if depth >= MAX_DEPTH {
            return Ok(Err(Failure::NotProven {
                from: a,
                to: c,
                turned: self.turned.get(),
            }));
        }
        let m = 0.5 * (a + c);
        let lm = match self.at(done, b, m, stroke)? {
            Ok(l) => l,
            Err(f) => return Ok(Err(f)),
        };
        best.sampled(m, &lm);
        if let Err(f) = self.between(done, b, (a, la), (m, &lm), stroke, depth + 1, best)? {
            return Ok(Err(f));
        }
        self.between(done, b, (m, &lm), (c, lc), stroke, depth + 1, best)
    }

    /// Per piece of `kinds`, the height of its lowest point above the
    /// die's top (the die's `y = 0`, before it is lowered) at stroke
    /// angle `rho`: the least over its material
    /// (`sequence::piece_lowest`), exact as the piece is rigid (a lower
    /// bound for a spline's flat).
    fn heights(
        &self,
        done: &BTreeSet<usize>,
        b: usize,
        rho: f64,
        kinds: &[Kind],
    ) -> Result<Vec<f64>, String> {
        let part = self.part;
        self.work.set(self.work.get().saturating_add(1));
        if self.work.get() > self.max_work {
            self.ran_out.set(true);
            return Err(format!(
                "more than {} distances and heights measured: the search over orders is too large (many bends, most orders colliding or close to it)",
                self.max_work
            ));
        }
        let worlds =
            stage_worlds(part, &self.angles(done, b, rho), &self.tol).map_err(|e| e.to_string())?;
        let frame = bend_frame(part, b, rho, &worlds).map_err(|e| e.to_string())?;
        let top = frame.point - frame.inside * (frame.radius + frame.thickness);
        kinds
            .iter()
            .map(|k| {
                let piece = self
                    .pieces
                    .iter()
                    .find(|p| p.what == *k)
                    .ok_or("a piece is missing")?;
                let moved = self
                    .built
                    .get(piece.carrier)
                    .and_then(Transform::inverse)
                    .zip(worlds.get(piece.carrier))
                    .map(|(back, stage)| back.then(stage))
                    .ok_or("a flat's place is missing")?;
                // Exact over the piece's material (not its hull points,
                // whose tangent apexes lie below a bend's skin), so a skin
                // along the die's top is proven at 0; a piece with no
                // material found fails closed.
                piece_lowest(part, k.piece(), |p| {
                    (moved.point(p) - top).dot(frame.inside)
                })
                .ok_or_else(|| format!("{} has no lowest point", k.text()))
            })
            .collect()
    }

    /// The rest proof, for a die that lies wholly below its top (`y ≤ 0`):
    /// a piece whose lowest point stays above `−tolerance` over `[a, c]`
    /// keeps at least the clearance from the die lowered by the clearance
    /// and the tolerance, whatever the die's shape below its top. Each hull
    /// point's height is smooth in the stroke angle with its second
    /// derivative at most `bow`, so over `[a, c]` it is at least the lower
    /// of its heights at the ends less `bow h² / 8`; where that is not
    /// enough the interval is halved while both ends stay above
    /// `−tolerance`. The least distance proven (the lowest height plus the
    /// lowering), or `None`.
    #[allow(clippy::too_many_arguments)]
    fn rest(
        &self,
        done: &BTreeSet<usize>,
        b: usize,
        kinds: &[Kind],
        (a, ya): (f64, &[f64]),
        (c, yc): (f64, &[f64]),
        stroke: Stroke,
        depth: u32,
    ) -> Result<Option<f64>, String> {
        let tol = self.tol.linear;
        let h = c - a;
        let sag = stroke.bow * h * h / 8.0;
        let ends: Vec<f64> = ya.iter().zip(yc).map(|(x, y)| x.min(*y)).collect();
        let low = ends.iter().fold(f64::INFINITY, |m, e| m.min(*e));
        if low - sag >= -tol {
            return Ok(Some(low - sag + self.clearance + tol));
        }
        if low < -tol || depth >= MAX_REST_DEPTH {
            return Ok(None);
        }
        let m = 0.5 * (a + c);
        let ym = self.heights(done, b, m, kinds)?;
        let Some(left) = self.rest(done, b, kinds, (a, ya), (m, &ym), stroke, depth + 1)? else {
            return Ok(None);
        };
        Ok(self
            .rest(done, b, kinds, (m, &ym), (c, yc), stroke, depth + 1)?
            .map(|right| left.min(right)))
    }
}

impl Cleared {
    fn sampled(&mut self, rho: f64, l: &Lowers) {
        for (&(tool, _), &v) in l {
            if let (Some(least), Some(at)) = (self.least.get_mut(tool), self.at.get_mut(tool))
                && v < *least
            {
                *least = v;
                *at = rho;
            }
        }
    }

    /// `least`, `at` (degrees) and `proven` of tool `tool` (null where
    /// the tool met no piece).
    fn json(&self, tool: usize) -> serde_json::Map<String, Value> {
        let finite = |v: Option<&f64>| match v {
            Some(v) if v.is_finite() => json!(v),
            _ => Value::Null,
        };
        let mut v = serde_json::Map::new();
        v.insert("least".into(), finite(self.least.get(tool)));
        let at = match (self.least.get(tool), self.at.get(tool)) {
            (Some(l), Some(a)) if l.is_finite() => json!(degrees(*a)),
            _ => Value::Null,
        };
        v.insert("at".into(), at);
        // Both are proven below the distance: the less of the two.
        let proven = match (self.proven.get(tool), self.least.get(tool)) {
            (Some(p), Some(l)) => Some(p.min(*l)),
            (p, _) => p.copied(),
        };
        v.insert("proven".into(), finite(proven.as_ref()));
        v
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

/// Per bend, its strip not yet bent as a slab on its parent.
fn slabs_of(part: &sheetmetal::Part, tol: &Tolerance) -> Result<Vec<Body>, String> {
    (0..part.bends.len())
        .map(|i| strip_slab(part, i, tol).map_err(|e| e.to_string()))
        .collect()
}

/// Every piece a stage can have: the flats, the bends as built and the
/// strips not yet bent (`slabs`, from [`slabs_of`]).
fn pieces_of<'a>(
    part: &'a sheetmetal::Part,
    slabs: &'a [Body],
    tol: &Tolerance,
) -> Result<Vec<Piece<'a>>, String> {
    let body = &part.solid.body;
    let mut pieces = Vec::new();
    let faces = (0..part.flats.len())
        .map(|f| (Kind::Flat(f), flat_faces(part, f), f))
        .chain(
            part.bends
                .iter()
                .enumerate()
                .map(|(i, b)| (Kind::Bend(i), bend_faces(part, i), b.parent)),
        );
    for (what, faces, carrier) in faces {
        if faces.is_empty() {
            continue;
        }
        pieces.push(Piece::new(
            part,
            what,
            Measured::new(body, Some(&faces), tol)?,
            carrier,
        ));
    }
    for (i, (b, slab)) in part.bends.iter().zip(slabs).enumerate() {
        pieces.push(Piece::new(
            part,
            Kind::Strip(i),
            Measured::new(slab, None, tol)?,
            b.parent,
        ));
    }
    Ok(pieces)
}

/// What the search over orders found for one part.
struct Proof {
    feasible: bool,
    order: Vec<(usize, BTreeSet<usize>, Cleared)>,
    tried: Tried,
    work: usize,
    clearance: f64,
}

impl Proof {
    /// A step tried that failed not for certain (a collision only not
    /// proven clear, or an interval no halving proved), with its words:
    /// while there is one, no order is not proven either.
    fn doubtful(&self) -> Option<String> {
        self.tried.iter().find_map(|((done, b), r)| match r {
            Err(f) if !f.certain(self.clearance) => Some(f.words(*b, done, self.clearance)),
            _ => None,
        })
    }

    /// The first step that failed for certain, in words.
    fn collision(&self) -> Option<String> {
        self.tried.iter().find_map(|((done, b), r)| match r {
            Err(f) if f.certain(self.clearance) => Some(f.words(*b, done, self.clearance)),
            _ => None,
        })
    }

    /// The measure's verdict on one part and why (`name` its feature):
    /// `bendable` with the order, `notProven` while a step failed not for
    /// certain, `collides` only when every step tried failed for certain.
    fn verdict(&self, name: &str) -> Verdict {
        if self.feasible {
            Verdict::Bendable(self.order.iter().map(|(b, _, _)| *b).collect())
        } else if let Some(why) = self.doubtful() {
            Verdict::NotProven(format!("`{name}`: no order was found, but {why}"))
        } else {
            let why = self
                .collision()
                .unwrap_or_else(|| "no bend can be made first".into());
            Verdict::Collides(format!("`{name}` cannot be bent in any order: {why}"))
        }
    }
}

/// What the measure says of one part.
#[derive(Debug, PartialEq)]
enum Verdict {
    Bendable(Vec<usize>),
    NotProven(String),
    Collides(String),
}

/// Why a search over orders stopped without an answer.
enum Stopped {
    /// It measured more than its work allows: not proven either way.
    OutOfWork(String),
    Error(String),
}

/// The tools and bounds a check is made with.
struct Tools {
    punch: Vec<[f64; 2]>,
    die: Vec<[f64; 2]>,
    length: Option<f64>,
    clearance: Option<f64>,
    max_work: usize,
}

/// A step that failed with the part as it lies (`given`), given what it
/// does turned end for end: cleared when turned clears; failing for
/// certain only when both fail for certain; otherwise the failure not
/// certain (so no order is not proven while it stands).
fn turn_or(
    given: Failure,
    turned: Result<Cleared, Failure>,
    clearance: f64,
) -> Result<Cleared, Failure> {
    match turned {
        Ok(cleared) => Ok(cleared),
        Err(t) if given.certain(clearance) && !t.certain(clearance) => Err(t),
        Err(_) => Err(given),
    }
}

/// A tool's polygon mirrored in `x` (the part turned end for end), its
/// points reversed so it keeps its sense of turning.
fn mirrored(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    points.iter().rev().map(|p| [-p[0], p[1]]).collect()
}

/// Whether two polygons have the same points (in any order): a tool
/// symmetric in `x` is its own mirror image, so turning the part changes
/// nothing and is not checked twice.
fn same_points(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    let sorted = |p: &[[f64; 2]]| {
        let mut p = p.to_vec();
        p.sort_by(|x, y| x[0].total_cmp(&y[0]).then(x[1].total_cmp(&y[1])));
        p
    };
    a.len() == b.len() && sorted(a) == sorted(b)
}

/// Searches for an order in which part `feature` of `doc` can be bent
/// with `tools` (the command's and the measure's one search).
fn prove(
    doc: &craft3d_doc::Document,
    regen: &craft3d_regen::Regen,
    feature: craft3d_doc::FeatureId,
    tools: Tools,
) -> Result<Proof, Stopped> {
    let built = craft3d_regen::sheet_metal_part(doc, regen, feature).map_err(Stopped::Error)?;
    let part = &built.part;
    let tol = built.tolerance;
    if !part.cuts.is_empty() {
        return Err(Stopped::Error(
            "the part has a hole across a bend; a strip not yet bent with a hole in it is not built as a piece (not supported)".into(),
        ));
    }
    let clearance = tools.clearance.unwrap_or(tol.linear);
    // The tools must be simple outlines: checked once here, so a bad one
    // is an error before any search.
    let unit = craft3d_math::Frame::WORLD;
    for (name, points) in [("punch", &tools.punch), ("die", &tools.die)] {
        // As tall as the polygon is wide, so the check holds at any scale.
        let size = points
            .iter()
            .flat_map(|q| q.iter())
            .fold(0.0_f64, |m, v| m.max(v.abs()))
            .max(tol.linear * 1e3);
        prism(points, &unit, 0.0, size, &tol)
            .map_err(|e| Stopped::Error(format!("`{name}`: {e}")))?;
    }
    let slabs = slabs_of(part, &tol).map_err(Stopped::Error)?;
    let pieces = pieces_of(part, &slabs, &tol).map_err(Stopped::Error)?;
    let turned_tools = {
        let (punch, die) = (mirrored(&tools.punch), mirrored(&tools.die));
        (!(same_points(&punch, &tools.punch) && same_points(&die, &tools.die)))
            .then_some((punch, die))
    };
    let check = Check {
        part,
        tol,
        punch: tools.punch,
        die: tools.die,
        turned_tools,
        turned: std::cell::Cell::new(false),
        length: tools.length,
        clearance,
        pieces,
        built: final_worlds(part),
        work: std::cell::Cell::new(0),
        max_work: tools.max_work,
        ran_out: std::cell::Cell::new(false),
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
    .map_err(|e| {
        if check.ran_out.get() {
            Stopped::OutOfWork(e)
        } else {
            Stopped::Error(e)
        }
    })?;
    Ok(Proof {
        feasible,
        order,
        tried,
        work: check.work.get(),
        clearance,
    })
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
    let tools = Tools {
        punch,
        die,
        length,
        clearance,
        max_work: MAX_WORK,
    };
    let proof = prove(doc, &regen, feature, tools).map_err(|e| match e {
        Stopped::OutOfWork(e) | Stopped::Error(e) => CommandError::new(format!("{ID}: {e}")),
    })?;
    let feasible = proof.feasible;
    let steps: Vec<Value> = proof
        .order
        .iter()
        .map(|(b, before, c)| {
            let mut v = c.json(PUNCH);
            v.insert("flange".into(), json!(b));
            v.insert("done".into(), json!(before.iter().collect::<Vec<_>>()));
            v.insert("turned".into(), json!(c.turned));
            v.insert("die".into(), Value::Object(c.json(DIE)));
            Value::Object(v)
        })
        .collect();
    let collisions: Vec<Value> = proof
        .tried
        .iter()
        .filter_map(|((before, b), r)| r.as_ref().err().map(|f| (before, b, f)))
        .take(MAX_LISTED)
        .map(|(before, b, f)| {
            let mut v = serde_json::Map::new();
            v.insert("flange".into(), json!(b));
            v.insert("done".into(), json!(before.iter().collect::<Vec<_>>()));
            match f {
                Failure::Collision {
                    tool,
                    piece,
                    angle,
                    distance,
                    lower,
                    turned,
                } => {
                    v.insert("turned".into(), json!(turned));
                    v.insert("tool".into(), json!(tool));
                    v.insert("piece".into(), json!(piece.text()));
                    v.insert("angle".into(), json!(degrees(*angle)));
                    v.insert("distance".into(), json!(distance));
                    v.insert("lower".into(), json!(lower));
                }
                Failure::NotProven { from, to, turned } => {
                    v.insert("turned".into(), json!(turned));
                    v.insert("notProven".into(), json!([degrees(*from), degrees(*to)]));
                }
            }
            v.insert("certain".into(), json!(f.certain(proof.clearance)));
            Value::Object(v)
        })
        .collect();
    // Whether no order is proven: every step that failed failed for
    // certain (null when an order was found).
    let proven = if feasible {
        Value::Null
    } else {
        json!(matches!(proof.verdict(""), Verdict::Collides(_)))
    };
    Ok(json!({
        "feature": feature.to_string(),
        "feasible": feasible,
        "infeasibleProven": proven,
        "order": if feasible { json!(proof.order.iter().map(|(b, _, _)| b).collect::<Vec<_>>()) } else { Value::Null },
        "steps": if feasible { json!(steps) } else { json!([]) },
        "collisions": collisions,
        "clearance": proof.clearance,
        "unchecked": UNCHECKED,
        "work": proof.work,
        "lengthUnit": length_unit(doc),
    }))
}

// ------------------------------------------------------------- the measure

/// The parameters of the assertion measure `bendable`.
pub(crate) const MEASURE_KEYS: [&str; 6] = ["of", "punch", "die", "length", "clearance", "maxWork"];

/// Its keys whose values are expressions (lists of points of them for a
/// tool).
pub(crate) const EXPRS: [&str; 4] = ["punch", "die", "length", "clearance"];

/// A tool's polygon for the measure: points `[x, y]`, each coordinate a
/// number in the document's unit or a length expression (of the
/// variables, a table's row, the outputs).
fn tool_points(ctx: &Ctx<'_>, p: &Map<String, Value>, key: &str) -> Result<Vec<[f64; 2]>, String> {
    let bad = || {
        format!(
            "`{key}` must be a list of at least 3 points [x, y] (numbers or length expressions, at most {MAX_POINTS})"
        )
    };
    let list = p
        .get(key)
        .filter(|v| !v.is_null())
        .ok_or_else(|| format!("`{key}` is required: a polygon [[x, y], ...]"))?
        .as_array()
        .ok_or_else(bad)?;
    if list.len() < 3 || list.len() > MAX_POINTS {
        return Err(bad());
    }
    list.iter()
        .map(|q| {
            let [x, y] = q.as_array().map(Vec::as_slice).ok_or_else(bad)? else {
                return Err(bad());
            };
            Ok([ctx.quantity(x, key, 1)?, ctx.quantity(y, key, 1)?])
        })
        .collect()
}

/// An optional length more than 0, a number or an expression.
fn positive(ctx: &Ctx<'_>, p: &Map<String, Value>, key: &str) -> Result<Option<f64>, String> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let value = ctx.quantity(v, key, 1)?;
    if value > 0.0 && value.is_finite() {
        Ok(Some(value))
    } else {
        Err(format!("`{key}` must be more than 0, got {value}"))
    }
}

/// The assertion measure `bendable`: whether every part of `of` (as the
/// other sheet-metal measures take it) can be bent with the tools, as
/// `sheetMetal.bendSequence` proves it. Yes when an order is found for
/// each; no when for one of them none is, and every step the search
/// tried that failed failed for certain (a tool and a piece found nearer
/// than the clearance at a real place); otherwise not proven, with the
/// step that could not be told and why: a collision only not proven
/// clear, an interval of the stroke no halving proved, or the search
/// running out of its work (`maxWork`, shared by the parts, at most
/// 20000 distances).
pub(crate) fn take(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    crate::verify_cmds::only(p, &MEASURE_KEYS)?;
    let punch = tool_points(ctx, p, "punch")?;
    let die = tool_points(ctx, p, "die")?;
    let length = positive(ctx, p, "length")?;
    let clearance = positive(ctx, p, "clearance")?;
    let max_work = match p.get("maxWork").filter(|v| !v.is_null()) {
        None => MAX_WORK,
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=MAX_WORK).contains(n))
            .ok_or_else(|| format!("`maxWork` must be a whole number from 1 to {MAX_WORK}"))?,
    };
    let parts = crate::sheetmetal_cmds::parts_of(ctx, p)?;
    let mut left = max_work;
    let mut listed = Vec::new();
    let mut fails = Vec::new();
    let mut doubts = Vec::new();
    let mut orders = Vec::new();
    for id in parts {
        let name = ctx.feature_name(id);
        let mut one = Map::new();
        one.insert("feature".into(), name.clone().into());
        if left == 0 {
            let why = format!(
                "`{name}`: the work ({max_work} distances) was used up by the parts before it"
            );
            one.insert("verdict".into(), "notProven".into());
            one.insert("why".into(), why.clone().into());
            doubts.push(why);
            listed.push(Value::Object(one));
            continue;
        }
        let tools = Tools {
            punch: punch.clone(),
            die: die.clone(),
            length,
            clearance,
            max_work: left,
        };
        match prove(ctx.doc, ctx.regen, id, tools) {
            Ok(proof) => {
                left = left.saturating_sub(proof.work);
                one.insert("work".into(), proof.work.into());
                match proof.verdict(&name) {
                    Verdict::Bendable(order) => {
                        let words: Vec<String> = order.iter().map(ToString::to_string).collect();
                        orders.push(format!("`{name}` in the order {}", words.join(", ")));
                        one.insert("verdict".into(), "bendable".into());
                        one.insert("order".into(), json!(order));
                    }
                    Verdict::NotProven(why) => {
                        one.insert("verdict".into(), "notProven".into());
                        one.insert("why".into(), why.clone().into());
                        doubts.push(why);
                    }
                    Verdict::Collides(why) => {
                        one.insert("verdict".into(), "collides".into());
                        one.insert("why".into(), why.clone().into());
                        fails.push(why);
                    }
                }
            }
            Err(Stopped::OutOfWork(e)) => {
                left = 0;
                let why = format!("`{name}`: {e}");
                one.insert("verdict".into(), "notProven".into());
                one.insert("why".into(), why.clone().into());
                doubts.push(why);
            }
            Err(Stopped::Error(e)) => return Err(format!("`{name}`: {e}")),
        }
        listed.push(Value::Object(one));
    }
    let mut out = if !fails.is_empty() {
        let mut out = Outcome::flag(false);
        out.note = Some(fails.join("; "));
        out
    } else if !doubts.is_empty() {
        Outcome::unproven(Dim::Flag, doubts.join("; "))
    } else {
        let mut out = Outcome::flag(true);
        out.note = Some(orders.join("; "));
        out
    };
    out.detail.insert("parts".into(), Value::Array(listed));
    out.detail
        .insert("work".into(), max_work.saturating_sub(left).into());
    out.detail.insert("unchecked".into(), UNCHECKED.into());
    Ok(out)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::Workspace;

    fn run(s: &mut Session, id: &str, params: Value) -> Value {
        s.execute(id, &params)
            .unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    /// A hairpin: the base is a strip 100 long with a tab at its right
    /// end, flange 0 (100 long) is bent 180 degrees up from the tab's left
    /// edge so it runs on past the base's right end, flange 1 (10 long)
    /// stands up at the base's left end. In the pattern flange 0 lies back
    /// over the strip, so the part reaches about 190 from flange 1's axis
    /// while the pattern's diagonal is about 119: a point's distance from
    /// an axis is bounded by its distance along the sheet, not by the
    /// pattern's size.
    fn hairpin(s: &mut Session, k: f64) {
        run(s, "file.new", json!({"units": "mm", "tolerance": 1e-6 * k}));
        run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
        let points: Vec<[f64; 2]> = [
            [0.0, 0.0],
            [100.0, 0.0],
            [100.0, 30.0],
            [90.0, 30.0],
            [90.0, 10.0],
            [0.0, 10.0],
        ]
        .iter()
        .map(|p| [p[0] * k, p[1] * k])
        .collect();
        let made = run(
            s,
            "sketch.polyline",
            json!({"sketch": "Base", "points": points, "closed": true}),
        );
        let lines = made["lines"].as_array().unwrap().clone();
        let mm = |v: f64| format!("{} mm", v * k);
        let out = run(
            s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Hairpin", "thickness": mm(1.0), "radius": mm(1.0),
                   "kFactor": 0.5, "flanges": [
                       {"edge": lines[3], "length": mm(100.0), "angle": "180 deg",
                        "insetEnd": mm(5.0)},
                       {"edge": lines[5], "length": mm(10.0), "angle": "90 deg"}]}),
        );
        assert!(out.get("failed").is_none_or(Value::is_null), "{out}");
    }

    #[test]
    fn the_stroke_speed_bounds_every_point_of_a_part_that_reaches_past_its_pattern() {
        for k in [1e-3, 1.0, 1e6] {
            let mut s = Session::new(Workspace::default());
            hairpin(&mut s, k);
            let params = json!({"feature": "Hairpin"});
            let p = Params::new("test", &params, &["feature"]).unwrap();
            let feature = {
                let doc = s.document().unwrap().history.current();
                crate::commands::feature_ref(doc, &p, "test", "feature").unwrap()
            };
            let open = s.document_mut().unwrap();
            let regen = open.regen();
            let doc = open.history.current();
            let built = craft3d_regen::sheet_metal_part(doc, &regen, feature).unwrap();
            let part = &built.part;
            let tol = built.tolerance;
            let slabs = slabs_of(part, &tol).unwrap();
            let check = Check {
                part,
                tol,
                punch: vec![[-k, 0.0], [k, 0.0], [k, 100.0 * k], [-k, 100.0 * k]],
                die: vec![[-k, -k], [k, -k], [k, 0.0], [-k, 0.0]],
                turned_tools: None,
                turned: std::cell::Cell::new(false),
                length: None,
                clearance: tol.linear,
                pieces: pieces_of(part, &slabs, &tol).unwrap(),
                built: final_worlds(part),
                work: std::cell::Cell::new(0),
                max_work: MAX_WORK,
                ran_out: std::cell::Cell::new(false),
            };
            // The pattern's diagonal (the old bound's premise): the flats'
            // outlines and the strips.
            let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            let mut grow = |p: [f64; 2]| {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            };
            for flat in &part.flats {
                for seg in &flat.outer {
                    grow(seg.start);
                }
            }
            for x in &part.bends {
                for q in [x.start, x.end] {
                    grow(q);
                    grow([
                        q[0] + x.allowance * x.across[0],
                        q[1] + x.allowance * x.across[1],
                    ]);
                }
            }
            let diagonal = (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
            let b = 1;
            let bend = &part.bends[b];
            let old_premise = diagonal + 2.0 * (bend.radius + bend.thickness);
            for done in [BTreeSet::from([0]), BTreeSet::new()] {
                let stroke = check.stroke(&done, b).unwrap();
                let steps = 256;
                let theta = bend.theta();
                let h = theta / steps as f64;
                let mut farthest = 0.0_f64;
                let mut fastest = 0.0_f64;
                let mut previous: Option<Vec<Vec3>> = None;
                for j in 0..=steps {
                    let rho = h * j as f64;
                    let worlds = stage_worlds(part, &check.angles(&done, b, rho), &tol).unwrap();
                    let frame = bend_frame(part, b, rho, &worlds).unwrap();
                    let into_tool = Transform::from_frame(&frame.tool(0.0).unwrap())
                        .inverse()
                        .unwrap();
                    let mut now = Vec::new();
                    let mut seen = BTreeSet::new();
                    for piece in check
                        .checked(&done, b, PUNCH)
                        .chain(check.checked(&done, b, DIE))
                    {
                        if !seen.insert(piece.what) {
                            continue;
                        }
                        let moved = check.built[piece.carrier]
                            .inverse()
                            .unwrap()
                            .then(&worlds[piece.carrier]);
                        for v in piece.measured.triangles.values().flatten().flatten() {
                            let at = moved.point(*v);
                            farthest = farthest.max(from_axis(frame.point, frame.axis, at).0);
                            now.push(into_tool.point(at));
                        }
                    }
                    if let Some(before) = &previous {
                        for (a, c) in before.iter().zip(&now) {
                            fastest = fastest.max(a.distance(*c) / h);
                        }
                    }
                    previous = Some(now);
                }
                assert!(
                    fastest <= stroke.speed * (1.0 + 1e-9),
                    "scale {k}, done {done:?}: a point moves at {fastest} against the tools, the bound is {}",
                    stroke.speed
                );
                assert!(
                    2.0 * (stroke.speed - bend.neutral()) >= farthest * (1.0 - 1e-12),
                    "scale {k}: the bound's reach {} against the farthest point {farthest}",
                    2.0 * (stroke.speed - bend.neutral())
                );
                if done.contains(&0) {
                    // Flange 0 folded out reaches past what the pattern's
                    // diagonal allowed.
                    assert!(
                        farthest > 1.5 * old_premise,
                        "scale {k}: farthest {farthest}, pattern diagonal bound {old_premise}"
                    );
                }
            }
        }
    }

    /// A piece with no points to bound it fails closed (review
    /// 2026-10-29: its reach folded to 0, so the speed bound came out too
    /// small, and its lowest point to +∞, a proof of nothing).
    #[test]
    fn a_piece_without_points_is_an_error_not_a_proof() {
        let k = 1.0;
        let mut s = Session::new(Workspace::default());
        hairpin(&mut s, k);
        let params = json!({"feature": "Hairpin"});
        let p = Params::new("test", &params, &["feature"]).unwrap();
        let feature = {
            let doc = s.document().unwrap().history.current();
            crate::commands::feature_ref(doc, &p, "test", "feature").unwrap()
        };
        let open = s.document_mut().unwrap();
        let regen = open.regen();
        let doc = open.history.current();
        let built = craft3d_regen::sheet_metal_part(doc, &regen, feature).unwrap();
        let part = &built.part;
        let tol = built.tolerance;
        let slabs = slabs_of(part, &tol).unwrap();
        let mut check = Check {
            part,
            tol,
            punch: vec![[-k, 0.0], [k, 0.0], [k, 100.0 * k], [-k, 100.0 * k]],
            die: vec![[-k, -k], [k, -k], [k, 0.0], [-k, 0.0]],
            turned_tools: None,
            turned: std::cell::Cell::new(false),
            length: None,
            clearance: tol.linear,
            pieces: pieces_of(part, &slabs, &tol).unwrap(),
            built: final_worlds(part),
            work: std::cell::Cell::new(0),
            max_work: MAX_WORK,
            ran_out: std::cell::Cell::new(false),
        };
        let done = BTreeSet::new();
        assert!(check.stroke(&done, 1).is_ok());
        assert!(check.heights(&done, 1, 0.1, &[Kind::Flat(0)]).is_ok());
        // A piece the part does not have: no lowest point.
        let err = check.heights(&done, 1, 0.1, &[Kind::Bend(7)]).unwrap_err();
        assert!(err.contains("missing") || err.contains("lowest"), "{err}");
        let base = check
            .pieces
            .iter_mut()
            .find(|p| p.what == Kind::Flat(0))
            .unwrap();
        base.corners.clear();
        let err = check.stroke(&done, 1).unwrap_err();
        assert!(err.contains("no points"), "{err}");
    }

    fn collision(distance: f64, lower: f64, turned: bool) -> Failure {
        Failure::Collision {
            tool: "punch",
            piece: Kind::Flat(0),
            angle: 1.0,
            distance,
            lower,
            turned,
        }
    }

    fn proof(tried: Vec<Result<Cleared, Failure>>) -> Proof {
        Proof {
            feasible: false,
            order: Vec::new(),
            tried: tried
                .into_iter()
                .enumerate()
                .map(|(b, r)| ((BTreeSet::new(), b), r))
                .collect(),
            work: 0,
            clearance: 0.1,
        }
    }

    /// The decisive branch: a step whose collision is only not proven
    /// clear (`lower < clearance <= distance`), or whose stroke has an
    /// interval no halving proved, leaves no order not proven (the
    /// measure's notProven, the command's infeasibleProven false); only
    /// when every step failed for certain is it a no.
    #[test]
    fn a_doubtful_step_leaves_no_order_not_proven() {
        let certain = collision(0.05, 0.0, false);
        let doubtful = collision(0.2, 0.05, false);
        let interval = Failure::NotProven {
            from: 0.0,
            to: 0.1,
            turned: false,
        };
        assert!(certain.certain(0.1));
        assert!(!doubtful.certain(0.1));
        assert!(!interval.certain(0.1));
        for other in [doubtful, interval] {
            let p = proof(vec![Err(certain.clone()), Err(other)]);
            let Verdict::NotProven(why) = p.verdict("U") else {
                panic!("{:?}", p.verdict("U"));
            };
            assert!(why.contains("no order was found, but"), "{why}");
        }
        let p = proof(vec![Err(certain.clone()), Err(certain)]);
        assert!(matches!(p.verdict("U"), Verdict::Collides(_)));
    }

    /// Turning the part end for end: a step clears when either way clears,
    /// fails for certain only when both ways do, and otherwise reports the
    /// way not certain.
    #[test]
    fn a_step_fails_for_certain_only_when_it_does_turned_too() {
        let cleared = Cleared {
            least: [1.0; 2],
            at: [0.0; 2],
            proven: [1.0; 2],
            turned: true,
        };
        let certain = collision(0.05, 0.0, false);
        let turned_certain = collision(0.0, 0.0, true);
        let turned_doubtful = collision(0.2, 0.05, true);
        let turned_interval = Failure::NotProven {
            from: 0.0,
            to: 0.1,
            turned: true,
        };
        assert!(turn_or(certain.clone(), Ok(cleared), 0.1).is_ok_and(|c| c.turned));
        let r = turn_or(certain.clone(), Err(turned_doubtful), 0.1).unwrap_err();
        assert!(!r.certain(0.1) && matches!(r, Failure::Collision { turned: true, .. }));
        let r = turn_or(certain.clone(), Err(turned_interval), 0.1).unwrap_err();
        assert!(matches!(r, Failure::NotProven { turned: true, .. }));
        let r = turn_or(certain.clone(), Err(turned_certain), 0.1).unwrap_err();
        assert!(r.certain(0.1) && matches!(r, Failure::Collision { turned: false, .. }));
        // Given doubtful: it stays, whatever turned finds.
        let doubtful = collision(0.2, 0.05, false);
        let r = turn_or(doubtful, Err(certain), 0.1).unwrap_err();
        assert!(!r.certain(0.1));
        // A symmetric tool is its own mirror image.
        let straight = [[-5.0, 0.0], [5.0, 0.0], [5.0, 100.0], [-5.0, 100.0]];
        assert!(same_points(&mirrored(&straight), &straight));
        let goose = [[-25.0, 0.0], [5.0, 0.0], [5.0, 30.0], [-25.0, 30.0]];
        assert!(!same_points(&mirrored(&goose), &goose));
    }
}
