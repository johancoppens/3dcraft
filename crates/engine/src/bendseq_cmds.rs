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
use craft3d_math::Portable;
use craft3d_math::{Tolerance, Transform, Vec3};
use craft3d_topo::Body;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "sheetMetal.bendSequence",
    "Bend Sequence",
    r#"{"feature": id or name, "punch": [[x, y], ...], "die": [[x, y], ...], "length": number or [number or null, ...] (one per flange) = the part's reach, "segmented": boolean = false, "clearance": number = the document's tolerance, "order": [flange or [flange, ...], ...] = searched, "maxWork": number = 20000}"#,
    "Checks whether a sheet-metal part (feature.sheetMetal) can be bent on a press brake with the punch and die given, and finds an order of its bends (changes nothing). punch and die are closed polygons (at least 3 points, in the document's length unit) in the plane square to a bend: the punch's with the centre of its tip at the origin and its body along +y, the die's with the point under the bend on the part's outer skin at the origin and its body below (y < 0); each is extruded along the bend: one piece length long centred on the middle of the bends made in the stroke (default: long enough to cover the whole part), or, with segmented true or a list of lengths, segmented tooling set up per bend, a segment per bend centred on its middle, as long as length gives for it (one number for every bend, or a list of one per flange with null for the bend's own) or else as long as the bend; unchecked states which. Bends on one line of one flat with the same angle, direction, radius and K-factor (tabs on one edge) are made in one stroke. At every bend both stand in the bend's frame: origin on the bend's axis (the die's R + T further out), y along the bisector of the two flats towards the inside (as on a press brake where both legs rise half the angle), z along the axis. A step, a bend made with some others already made and the rest still flat, clears when the proven distance (its lower bound) between the punch and every piece of the part but that bend's own strip and its two flats, and between the die counted lowered (moved away from the punch) by clearance plus the document's tolerance and every piece but the bend's own strip, is at least clearance over the whole stroke from 0 to the bend's angle (so the sheet may rest on the die's top, but not go into it by more than the tolerance nor come nearer than clearance from any other side), with the part as it lies or turned end for end (about the tools' y, as an operator turns it: the tools mirrored in x; segments stay at their bends), unless both tools are their own mirror image (the same polygon, compared as a cyclic list of points): proven at eight stroke angles, and between two of them less how far any point of the part can move against the tools in between (half its distance from the bend's axis, the farthest at either end of the stroke, plus the bend's neutral radius, per radian), the interval halved (to 1/1024 of the stroke) until that holds, or, for a die wholly below its top (y <= 0), by the piece's lowest point staying above the die's top less the tolerance between them (its height bounded by its heights there and how fast it can bend, halved as needed: what proves a piece lifting off the die); a piece and a tool wholly inside one another count as 0. With order (a list of strokes, each a flange or a list of flanges on one line; every flange once) that order is checked step by step, every step, instead of a search. Otherwise the order is searched depth first over all orders of the strokes (with one-piece tools a stroke is every bend on its line, and an order given must keep them together; with segmented tooling the bends of a line are tried together and one by one), the bends whose flanges carry the fewest others first (then the shorter flange, then the lower index), every set of bends made that leads nowhere remembered. Returns feasible, given (whether the order was given), order (the strokes, first bent first: a flange by index, or a list for a stroke of several; null when none clears), steps (per stroke of the order, also those that cleared of an order given that does not: flange (its first), flanges, segments (with segmented tooling: the tools' length at each of its bends), done (the flanges made before it), least (the least distance to the punch proven at the stroke angles measured), at (the stroke angle of it, degrees), proven (the least distance to the punch proven over the whole stroke, between the angles measured too) die (least, at and proven for the lowered die) and turned (whether the part is turned end for end for it)), collisions (every step tried that did not clear, per orientation tried: flange, flanges, done, tool (punch or die), piece (the base, flange i, or the strip of flange i, bent or flat), angle (degrees), distance and lower (the distance found and what is proven below it; to the lowered die for the die), or notProven with the stroke interval no halving could prove; certain: whether this failure is certain, a tool and a piece found nearer than the clearance at a real place or wholly inside one another, where a collision only not proven clear is not; stepCertain: whether the step fails for certain, both as the part lies and turned; turned: whether this failure is the turned one), infeasibleProven (with feasible false: whether every step tried that failed failed for certain, so that no order exists with these tools, or for an order given whether a step of it fails for certain; false means no order was found but one may exist), unchecked (what is not looked at) and work (distances measured). Refused: a part with a hole across a bend (owed), a polygon that crosses itself, a clearance that is not more than 0, and more distances and heights measured than maxWork (default 20000, at most 10000000; give the order, or raise it). Not looked at (said in unchecked): a bend's own strip during its stroke, and its own two flats against the punch (an acute bend with a punch of a wider angle closing its flats onto it is not found).",
    doc: true,
    names: ["feature"],
    bend_sequence
)];

/// Most distances one check measures, unless the caller gives more.
const MAX_WORK: usize = 20_000;

/// Most distances a caller of the command may ask for (`maxWork`).
const MAX_WORK_ASKED: u64 = 10_000_000;

/// Most points in a tool's polygon.
const MAX_POINTS: usize = 1_000;

/// Stroke angles sampled per bend before halving.
const SAMPLES: usize = 8;

/// Deepest an interval of the stroke is halved (1/1024 of it with the 8
/// samples).
const MAX_DEPTH: u32 = 7;

/// What the check does not look at, said in its result.
const UNCHECKED: &str = "each bend's own strip during its stroke (against both tools), and its two flats against the punch (its tip forms the bend between them)";

/// The tooling a check assumes, said in its result after [`UNCHECKED`].
fn unchecked(segmented: bool) -> String {
    let tooling = if segmented {
        "segmented tooling assumed: the tools are a segment per bend, centred on its middle and as long as given or as the bend, set up for each stroke; whatever lies beyond a segment's ends, or between two segments, is not met by the tools, so a tool run longer than its bend may collide where this check does not"
    } else {
        "one-piece tools assumed: centred on the middle of the bends of each stroke and as long as given (by default reaching past the whole part, so everything along the bend line is met)"
    };
    format!("{UNCHECKED}; {tooling}")
}

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

    /// The failure in words, for a step: the stroke `g` after `done`.
    fn words(&self, g: &[usize], done: &BTreeSet<usize>, clearance: f64) -> String {
        let b = match g {
            [one] => format!("flange {one}"),
            many => format!("flanges {}", stroke_words(many)),
        };
        let step = if done.is_empty() {
            format!("{b} first")
        } else {
            let before: Vec<String> = done.iter().map(ToString::to_string).collect();
            format!("{b} after {}", before.join(", "))
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
/// radian² is more), where the tools are along the axis, whether the
/// die lies wholly below its top (`y ≤ 0`), so the rest proof holds, and
/// whether the part is turned end for end.
#[derive(Clone, Debug)]
struct Stroke {
    speed: f64,
    bow: f64,
    /// The tools' pieces along the axis from the first bend's middle:
    /// one for one-piece tools, one per bend of the stroke for segmented
    /// tooling ([`Tooling`]).
    segments: Vec<(f64, f64)>,
    die_below: bool,
    /// The part turned end for end: the tools' polygons mirrored in `x`
    /// against it (`Check::turned_tools`); the segments stay where they
    /// are, set up at the bends either way.
    turned: bool,
}

/// How long the tools are along the bends.
#[derive(Clone, Debug)]
enum Tooling {
    /// One piece per stroke, centred on the middle of its bends, this
    /// long (`None`: reaching past the whole part, the default).
    Whole(Option<f64>),
    /// Segmented tooling set up per stroke: a segment per bend, centred
    /// on its middle, per bend this long (`None`: the bend's own length).
    Segmented(Vec<Option<f64>>),
}

impl Tooling {
    fn segmented(&self) -> bool {
        matches!(self, Tooling::Segmented(_))
    }
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
    /// The tools mirrored in `x` (`mirrored`): where they stand against
    /// the part turned end for end; `None` when both are their own
    /// mirror image (`symmetric`), so turning changes nothing.
    turned_tools: Option<(Polygon, Polygon)>,
    tooling: Tooling,
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
    /// The stage's bend angles: those made at theirs, the stroke's `g` at
    /// `rho`, the rest flat.
    fn angles(&self, done: &BTreeSet<usize>, g: &[usize], rho: f64) -> Vec<f64> {
        self.part
            .bends
            .iter()
            .enumerate()
            .map(|(i, x)| {
                if done.contains(&i) {
                    x.angle
                } else if g.contains(&i) {
                    rho * x.angle.signum()
                } else {
                    0.0
                }
            })
            .collect()
    }

    /// The pieces tool `tool` is checked against at the stroke of the
    /// bends `g` with `done` made: every piece of the stage (the flats,
    /// the bends made, the strips not yet bent) but the stroke's own
    /// strips (half rolled, half flat on their children during the
    /// stroke: not pieces), and for the punch not their flats either (its
    /// tip forms each bend between them).
    fn checked<'s>(
        &'s self,
        done: &'s BTreeSet<usize>,
        g: &'s [usize],
        tool: usize,
    ) -> impl Iterator<Item = &'s Piece<'s>> + 's {
        let own: Vec<usize> = g
            .iter()
            .filter_map(|&b| self.part.bends.get(b))
            .flat_map(|x| [x.parent, x.child])
            .collect();
        self.pieces.iter().filter(move |p| match p.what {
            Kind::Flat(f) => tool != PUNCH || !own.contains(&f),
            Kind::Bend(i) => done.contains(&i),
            Kind::Strip(i) => !g.contains(&i) && !done.contains(&i),
        })
    }

    /// The speed bound and the tools' pieces of the stroke of the bends
    /// `g` (on one line, [`strokes`]; the first's frame) with `done` made:
    /// the farthest any hull point of a checked piece lies from the axis
    /// at either end of the stroke (`stroke_speed`), and the tools along
    /// the axis ([`Tooling`]): one piece centred on the middle of the
    /// stroke's bends, as long as given or else twice the farthest a hull
    /// point lies along the axis from that middle (so it reaches past
    /// every piece), or per bend a segment centred on its middle, as long
    /// as given for it or else the bend itself.
    fn stroke(&self, done: &BTreeSet<usize>, g: &[usize]) -> Result<Stroke, String> {
        let part = self.part;
        let b = *g.first().ok_or("a stroke has no bend")?;
        let first = part.bends.get(b).ok_or("a bend is missing")?;
        let theta = first.theta();
        let start =
            stage_worlds(part, &self.angles(done, g, 0.0), &self.tol).map_err(|e| e.to_string())?;
        let end = stage_worlds(part, &self.angles(done, g, theta), &self.tol)
            .map_err(|e| e.to_string())?;
        let frame = bend_frame(part, b, 0.0, &start).map_err(|e| e.to_string())?;
        // In the pattern the bends of a stroke lie on one line along the
        // first's direction, which the frame's axis carries.
        let d = unit2([first.end[0] - first.start[0], first.end[1] - first.start[1]])
            .ok_or("a bend has no length")?;
        let middle =
            |x: &sheetmetal::Bend| [0.5 * (x.start[0] + x.end[0]), 0.5 * (x.start[1] + x.end[1])];
        let m0 = middle(first);
        let along_line = |q: [f64; 2]| (q[0] - m0[0]) * d[0] + (q[1] - m0[1]) * d[1];
        // The middle of the stroke's bends along the axis.
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for &i in g {
            let x = part.bends.get(i).ok_or("a bend is missing")?;
            for q in [x.start, x.end] {
                lo = lo.min(along_line(q));
                hi = hi.max(along_line(q));
            }
        }
        let centre = 0.5 * (lo + hi);
        if !centre.is_finite() {
            return Err("a bend has no finite length".into());
        }
        let mut reach = 0.0_f64;
        let mut along = 0.0_f64;
        let mut seen = BTreeSet::new();
        for piece in self
            .checked(done, g, PUNCH)
            .chain(self.checked(done, g, DIE))
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
                    let (r, a) = from_axis(frame.point, frame.axis, moved.point(c));
                    reach = reach.max(r);
                    along = along.max((a - centre).abs());
                }
            }
        }
        let speed = stroke_speed(part, b, reach).ok_or("the part has no finite size")?;
        let segments = match &self.tooling {
            Tooling::Whole(given) => {
                let length = given.unwrap_or(2.0 * (along + self.tol.linear).max(frame.reach));
                if !(length.is_finite() && length > 0.0) {
                    return Err("the part has no finite size".into());
                }
                vec![(centre - 0.5 * length, centre + 0.5 * length)]
            }
            Tooling::Segmented(lengths) => {
                let mut segments = Vec::with_capacity(g.len());
                for &i in g {
                    let x = part.bends.get(i).ok_or("a bend is missing")?;
                    let length = segment_length(x, lengths.get(i).copied().flatten());
                    if !(length.is_finite() && length > 0.0) {
                        return Err(format!("bend {i} has no length"));
                    }
                    let at = along_line(middle(x));
                    segments.push((at - 0.5 * length, at + 0.5 * length));
                }
                segments
            }
        };
        // Against the tools a parent point turns at rate ½ about the axis
        // (acceleration d/4) and a child point also takes up the shift
        // back across the strip, `ρ r` turned with it (r more): no point
        // of the pieces accelerates faster than `reach/4 + r`.
        let bow = 0.25 * reach + part.bends.get(b).map_or(0.0, |x| x.neutral());
        Ok(Stroke {
            speed,
            bow,
            segments,
            die_below: self.die.iter().all(|p| p[1] <= 0.0),
            turned: false,
        })
    }

    /// One more distance or height measured; an error past the cap.
    fn count(&self) -> Result<(), String> {
        self.work.set(self.work.get().saturating_add(1));
        if self.work.get() > self.max_work {
            self.ran_out.set(true);
            return Err(format!(
                "more than {} distances and heights measured: the search over orders is too large (many bends, most orders colliding or close to it); raise `maxWork`, or give the command the `order` to check",
                self.max_work
            ));
        }
        Ok(())
    }

    /// The least proven distance between each tool and the pieces it is
    /// checked against at the stroke angle `rho` of the bends `g`, with
    /// the others made (`done`) or flat, the part turned as `stroke`
    /// says; a failure when one is below the clearance (one found for
    /// certain before one only not proven clear).
    fn at(
        &self,
        done: &BTreeSet<usize>,
        g: &[usize],
        rho: f64,
        stroke: &Stroke,
    ) -> Result<Result<Lowers, Failure>, String> {
        let part = self.part;
        let b = *g.first().ok_or("a stroke has no bend")?;
        let worlds =
            stage_worlds(part, &self.angles(done, g, rho), &self.tol).map_err(|e| e.to_string())?;
        let frame = bend_frame(part, b, rho, &worlds).map_err(|e| e.to_string())?;
        let mut least = Lowers::new();
        let mut doubtful: Option<Failure> = None;
        let turned = stroke.turned;
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
            let pieces: Vec<&Piece<'_>> = self.checked(done, g, tool).collect();
            // The tool carried into each carrier's built place, built once
            // per carrier and segment (every piece it carries shares it).
            let carriers: Vec<usize> = pieces
                .iter()
                .map(|p| p.carrier)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let mut bodies: Vec<(usize, Body, craft3d_math::Frame, f64)> =
                Vec::with_capacity(carriers.len().saturating_mul(stroke.segments.len()));
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
                for &(z0, z1) in &stroke.segments {
                    let body = prism(points, &local, z0, z1, &self.tol)
                        .map_err(|e| format!("the {name}: {e}"))?;
                    bodies.push((carrier, body, local, 0.5 * (z0 + z1)));
                }
            }
            let measured: Vec<Measured<'_>> = bodies
                .iter()
                .map(|(_, body, _, _)| Measured::new(body, None, &self.tol))
                .collect::<Result<_, _>>()?;
            for piece in pieces {
                // The least over the tool's pieces along the axis.
                let mut lower = f64::INFINITY;
                let mut found = f64::INFINITY;
                let mut inside = false;
                for ((carrier, body, local, zmid), tool_m) in bodies.iter().zip(&measured) {
                    if *carrier != piece.carrier {
                        continue;
                    }
                    self.count()?;
                    let near = verify::distance(
                        std::slice::from_ref(tool_m),
                        std::slice::from_ref(&piece.measured),
                    )?;
                    // Wholly inside one another: a point of the piece
                    // inside the tool, or a point of the tool (its
                    // middle section) inside the piece.
                    let tool_point = points
                        .first()
                        .map(|q| local.point(q[0], q[1], *zmid))
                        .ok_or("a tool has no points")?;
                    let within = inside_piece(part, piece.what.piece(), tool_point)
                        || contains_points(body, &piece.points, &self.tol)
                            .map_err(|e| e.to_string())?
                            .contains(&Some(true));
                    inside |= within;
                    lower = lower.min(if within { 0.0 } else { near.lower.max(0.0) });
                    found = found.min(if within { 0.0 } else { near.distance });
                }
                if !lower.is_finite() {
                    return Err("a tool is missing".into());
                }
                // A piece lying on a die below its top: what the mesh
                // cannot prove (a bend's skin along the die's top), its
                // lowest point's height does (`Check::rest`).
                if !inside && lower < self.clearance && tool == DIE && stroke.die_below {
                    let low = self
                        .heights(done, g, rho, &[piece.what])?
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
                        distance: found,
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

    /// Whether the bends `g` (one stroke) can be made with `done` made,
    /// the part as it lies or turned end for end. A press brake lets the
    /// operator turn the part about the brake's vertical (the tools' `y`)
    /// between bends; for tools extruded along the bend and set up at
    /// its bends that is the tools' polygons mirrored in `x`. So the step
    /// clears when either clears, and fails for certain only when both
    /// do (`turn_or`); both orientations' failures are kept for the
    /// listing.
    fn step(&self, done: &BTreeSet<usize>, g: &[usize]) -> Result<Result<Cleared, Failed>, String> {
        let stroke = self.stroke(done, g)?;
        let given = match self.step_as(done, g, &stroke)? {
            Ok(cleared) => return Ok(Ok(cleared)),
            Err(f) => f,
        };
        if self.turned_tools.is_none() {
            return Ok(Err(Failed(vec![given])));
        }
        let turned = Stroke {
            turned: true,
            ..stroke
        };
        let other = self.step_as(done, g, &turned)?;
        Ok(turn_or(given, other))
    }

    /// Whether the bends `g` can be made with `done` made over their
    /// whole stroke, the part turned as `stroke.turned` says.
    fn step_as(
        &self,
        done: &BTreeSet<usize>,
        g: &[usize],
        stroke: &Stroke,
    ) -> Result<Result<Cleared, Failure>, String> {
        let part = self.part;
        let b = *g.first().ok_or("a stroke has no bend")?;
        let theta = part.bends.get(b).ok_or("a bend is missing")?.theta();
        let mut best = Cleared {
            least: [f64::INFINITY; 2],
            at: [0.0; 2],
            proven: [f64::INFINITY; 2],
            turned: stroke.turned,
        };
        let mut prev: Option<(f64, Lowers)> = None;
        for j in 0..=SAMPLES {
            let rho = theta * j as f64 / SAMPLES as f64;
            let l = match self.at(done, g, rho, stroke)? {
                Ok(l) => l,
                Err(f) => return Ok(Err(f)),
            };
            best.sampled(rho, &l);
            if let Some((a, la)) = &prev
                && let Err(f) = self.between(done, g, (*a, la), (rho, &l), stroke, 0, &mut best)?
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
        g: &[usize],
        (a, la): (f64, &Lowers),
        (c, lc): (f64, &Lowers),
        stroke: &Stroke,
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
                let ya = self.heights(done, g, a, &resting)?;
                let yc = self.heights(done, g, c, &resting)?;
                self.rest(done, g, &resting, (a, &ya), (c, &yc), stroke, 0)?
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
                turned: stroke.turned,
            }));
        }
        let m = 0.5 * (a + c);
        let lm = match self.at(done, g, m, stroke)? {
            Ok(l) => l,
            Err(f) => return Ok(Err(f)),
        };
        best.sampled(m, &lm);
        if let Err(f) = self.between(done, g, (a, la), (m, &lm), stroke, depth + 1, best)? {
            return Ok(Err(f));
        }
        self.between(done, g, (m, &lm), (c, lc), stroke, depth + 1, best)
    }

    /// Per piece of `kinds`, the height of its lowest point above the
    /// die's top (the die's `y = 0`, before it is lowered) at stroke
    /// angle `rho`: the least over its material
    /// (`sequence::piece_lowest`), exact as the piece is rigid (a lower
    /// bound for a spline's flat).
    fn heights(
        &self,
        done: &BTreeSet<usize>,
        g: &[usize],
        rho: f64,
        kinds: &[Kind],
    ) -> Result<Vec<f64>, String> {
        let part = self.part;
        self.count()?;
        let b = *g.first().ok_or("a stroke has no bend")?;
        let worlds =
            stage_worlds(part, &self.angles(done, g, rho), &self.tol).map_err(|e| e.to_string())?;
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
        g: &[usize],
        kinds: &[Kind],
        (a, ya): (f64, &[f64]),
        (c, yc): (f64, &[f64]),
        stroke: &Stroke,
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
        let ym = self.heights(done, g, m, kinds)?;
        let Some(left) = self.rest(done, g, kinds, (a, ya), (m, &ym), stroke, depth + 1)? else {
            return Ok(None);
        };
        Ok(self
            .rest(done, g, kinds, (m, &ym), (c, yc), stroke, depth + 1)?
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

/// How a step failed: its failure with the part as it lies, then the
/// turned one when that was tried (tools that are not their own mirror
/// image).
#[derive(Clone, Debug)]
struct Failed(Vec<Failure>);

impl Failed {
    /// Whether the step fails for certain: every orientation tried does.
    fn certain(&self, clearance: f64) -> bool {
        !self.0.is_empty() && self.0.iter().all(|f| f.certain(clearance))
    }

    /// The failure the step is reported by: one not certain while there
    /// is one (so no order is not proven while it stands), else the
    /// first (as the part lies).
    fn reported(&self, clearance: f64) -> Option<&Failure> {
        self.0
            .iter()
            .find(|f| !f.certain(clearance))
            .or_else(|| self.0.first())
    }
}

/// Per stroke tried (the bends made before it, the stroke's bends), how
/// it went.
type Tried = BTreeMap<(BTreeSet<usize>, Vec<usize>), Result<Cleared, Failed>>;

/// A step of an order: the stroke's bends, those made before it, and how
/// it cleared.
type Step = (Vec<usize>, BTreeSet<usize>, Cleared);

/// Whether bend `i` is made in one stroke with bend `b`: the same parent
/// flat, the same signed angle (within the angular tolerance), the same
/// radius and neutral fibre (within the tolerance), and its tangent line
/// on `b`'s line running the same way (both ends within the tolerance of
/// it). Then both turn about the same axis by the same motion, so one
/// punch and die along that line make both. Running the same way is not
/// a restriction but the test of the same side: a bend's tangent line
/// runs with its parent on its left (`sheetmetal::Bend::start`), so two
/// bends of one parent on one line listed opposite ways have their
/// children on opposite sides of it (one bends towards `+across`, the
/// other away), and no one punch and die make both.
fn one_line(part: &sheetmetal::Part, b: usize, i: usize, tol: &Tolerance) -> bool {
    let (Some(x), Some(y)) = (part.bends.get(b), part.bends.get(i)) else {
        return false;
    };
    let Some(d) = unit2([x.end[0] - x.start[0], x.end[1] - x.start[1]]) else {
        return false;
    };
    let off = |p: [f64; 2]| ((p[0] - x.start[0]) * d[1] - (p[1] - x.start[1]) * d[0]).abs();
    let ahead = (y.end[0] - y.start[0]) * d[0] + (y.end[1] - y.start[1]) * d[1] > 0.0;
    x.parent == y.parent
        && (x.angle - y.angle).abs() <= tol.angular
        && (x.radius - y.radius).abs() <= tol.linear
        && ((x.k - y.k) * x.thickness).abs() <= tol.linear
        && ahead
        && off(y.start) <= tol.linear
        && off(y.end) <= tol.linear
}

/// The strokes the search makes: the bends grouped by [`one_line`] with
/// the first of each group, in order of their first bend.
fn strokes(part: &sheetmetal::Part, tol: &Tolerance) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    for i in 0..part.bends.len() {
        match out
            .iter_mut()
            .find(|g| g.first().is_some_and(|&b| one_line(part, b, i, tol)))
        {
            Some(g) => g.push(i),
            None => out.push(vec![i]),
        }
    }
    out
}

/// The first order (depth first) of the strokes `ranked` whose every step
/// clears (`step`: [`Check::step`]). `ranked` may hold a stroke and its
/// bends one by one as well (segmented tooling): a step skips every
/// stroke with a bend already made.
fn search<F>(
    step: &mut F,
    done: &mut BTreeSet<usize>,
    order: &mut Vec<Step>,
    dead: &mut BTreeSet<BTreeSet<usize>>,
    tried: &mut Tried,
    ranked: &[Vec<usize>],
) -> Result<bool, String>
where
    F: FnMut(&BTreeSet<usize>, &[usize]) -> Result<Result<Cleared, Failed>, String>,
{
    if ranked.iter().all(|g| g.iter().all(|b| done.contains(b))) {
        return Ok(true);
    }
    if dead.contains(done) {
        return Ok(false);
    }
    for g in ranked {
        if g.iter().any(|b| done.contains(b)) {
            continue;
        }
        let key = (done.clone(), g.clone());
        let result = match tried.get(&key) {
            Some(r) => r.clone(),
            None => {
                let r = step(done, g)?;
                tried.insert(key, r.clone());
                r
            }
        };
        if let Ok(cleared) = result {
            order.push((g.clone(), done.clone(), cleared));
            done.extend(g.iter().copied());
            if search(step, done, order, dead, tried, ranked)? {
                return Ok(true);
            }
            for b in g {
                done.remove(b);
            }
            order.pop();
        }
    }
    dead.insert(done.clone());
    Ok(false)
}

/// The strokes the search tries, best first. One-piece tools along a line
/// bend every bend on it at once, so with them the strokes are exactly
/// the groups of [`strokes`]; segmented tooling may bend a group's bends
/// together or one by one (a segment set up only at the bend made), so
/// with it every group of more than one bend is an option beside each
/// bend alone, and no order is missed for having been forced into a
/// stroke. Ranked: the strokes whose flanges carry the fewest others
/// first, then the shorter flange, then the larger stroke, then the lower
/// index.
fn ranked_strokes(part: &sheetmetal::Part, tol: &Tolerance, segmented: bool) -> Vec<Vec<usize>> {
    let groups = strokes(part, tol);
    let mut ranked: Vec<Vec<usize>> = if segmented {
        let mut all: Vec<Vec<usize>> = groups.iter().filter(|g| g.len() > 1).cloned().collect();
        all.extend((0..part.bends.len()).map(|b| vec![b]));
        all
    } else {
        groups
    };
    let key = |g: &Vec<usize>| {
        let carries = g.iter().map(|&b| carried(part, b)).min().unwrap_or(0);
        let short = g
            .iter()
            .map(|&b| flange_length(part, b).unwrap_or(f64::INFINITY))
            .fold(f64::INFINITY, f64::min);
        (carries, short, g.len(), g.first().copied().unwrap_or(0))
    };
    ranked.sort_by(|a, b| {
        let (ka, kb) = (key(a), key(b));
        ka.0.cmp(&kb.0)
            .then(ka.1.total_cmp(&kb.1))
            .then(kb.2.cmp(&ka.2))
            .then(ka.3.cmp(&kb.3))
    });
    ranked
}

/// `v` scaled to unit length, if it has one.
fn unit2(v: [f64; 2]) -> Option<[f64; 2]> {
    let l = v[0].phypot(v[1]);
    (l.is_finite() && l > 0.0).then(|| [v[0] / l, v[1] / l])
}

/// A stroke as the result lists it: its flange, or a list of them.
fn stroke_json(g: &[usize]) -> Value {
    match g {
        [one] => json!(one),
        many => json!(many),
    }
}

/// A stroke in words: `3`, or `[0, 1]` for several bent at once.
fn stroke_words(g: &[usize]) -> String {
    match g {
        [one] => one.to_string(),
        many => {
            let each: Vec<String> = many.iter().map(ToString::to_string).collect();
            format!("[{}]", each.join(", "))
        }
    }
}

/// The length of a segment of segmented tooling at bend `x`: as given,
/// or else the bend's own.
fn segment_length(x: &sheetmetal::Bend, given: Option<f64>) -> f64 {
    given.unwrap_or_else(|| (x.end[0] - x.start[0]).phypot(x.end[1] - x.start[1]))
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

/// What the search over orders (or the check of an order given) found
/// for one part.
struct Proof {
    feasible: bool,
    /// Whether the order was given (checked, not searched).
    given: bool,
    order: Vec<Step>,
    tried: Tried,
    work: usize,
    clearance: f64,
    /// With segmented tooling, the tools' length at each bend.
    segments: Option<Vec<f64>>,
}

impl Proof {
    /// A step tried that failed not for certain (a collision only not
    /// proven clear, or an interval no halving proved, in either
    /// orientation tried), with its words: while there is one, no order
    /// is not proven either.
    fn doubtful(&self) -> Option<String> {
        self.tried.iter().find_map(|((done, g), r)| match r {
            Err(f) if !f.certain(self.clearance) => f
                .reported(self.clearance)
                .map(|x| x.words(g, done, self.clearance)),
            _ => None,
        })
    }

    /// The first step that failed for certain, in words.
    fn collision(&self) -> Option<String> {
        self.tried.iter().find_map(|((done, g), r)| match r {
            Err(f) if f.certain(self.clearance) => f
                .reported(self.clearance)
                .map(|x| x.words(g, done, self.clearance)),
            _ => None,
        })
    }

    /// The verdict on one part and why (`name` its feature): `bendable`
    /// with the order; for a search, `notProven` while a step failed not
    /// for certain and `collides` only when every step tried failed for
    /// certain; for an order given, `collides` when a step of it fails
    /// for certain, else `notProven`.
    fn verdict(&self, name: &str) -> Verdict {
        if self.feasible {
            return Verdict::Bendable(self.order.iter().map(|(g, _, _)| g.clone()).collect());
        }
        if self.given {
            return match (self.collision(), self.doubtful()) {
                (Some(why), _) => {
                    Verdict::Collides(format!("`{name}` cannot be bent in the order given: {why}"))
                }
                (None, why) => Verdict::NotProven(format!(
                    "`{name}`: the order given is not proven: {}",
                    why.unwrap_or_else(|| "a step does not clear".into())
                )),
            };
        }
        if let Some(why) = self.doubtful() {
            Verdict::NotProven(format!("`{name}`: no order was found, but {why}"))
        } else {
            let why = self
                .collision()
                .unwrap_or_else(|| "no bend can be made first".into());
            Verdict::Collides(format!(
                "`{name}` cannot be bent in any order with these tools: {why}"
            ))
        }
    }
}

/// What the measure says of one part.
#[derive(Debug, PartialEq)]
enum Verdict {
    /// The strokes in order.
    Bendable(Vec<Vec<usize>>),
    NotProven(String),
    Collides(String),
}

/// Why a search over orders stopped without an answer.
enum Stopped {
    /// It measured more than its work allows: not proven either way.
    OutOfWork(String),
    Error(String),
}

/// The tools' length as asked for: one for every bend (`None`: the
/// default of the tooling), or one per flange (`None`: the bend's own).
#[derive(Clone, Debug)]
enum Length {
    One(Option<f64>),
    PerFlange(Vec<Option<f64>>),
}

/// The tools and bounds a check is made with.
struct Tools {
    punch: Vec<[f64; 2]>,
    die: Vec<[f64; 2]>,
    length: Length,
    /// Segmented tooling asked for (`None`: as `length` implies, one
    /// piece unless it lists a length per flange).
    segmented: Option<bool>,
    clearance: Option<f64>,
    /// The order to check, as given (flange indices per stroke).
    order: Option<Vec<Vec<usize>>>,
    max_work: usize,
}

/// A step that failed with the part as it lies (`given`), given what it
/// does turned end for end: cleared when turned clears; otherwise both
/// failures, so the step fails for certain only when both fail for
/// certain ([`Failed::certain`]) and is reported by the one not certain
/// ([`Failed::reported`]).
fn turn_or(given: Failure, turned: Result<Cleared, Failure>) -> Result<Cleared, Failed> {
    match turned {
        Ok(cleared) => Ok(cleared),
        Err(t) => Err(Failed(vec![given, t])),
    }
}

/// A tool's polygon mirrored in `x` (the part turned end for end), its
/// points reversed so it keeps its sense of turning.
fn mirrored(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    points.iter().rev().map(|p| [-p[0], p[1]]).collect()
}

/// Whether a tool's polygon is its own mirror image in `x`: the mirrored
/// polygon is the same cyclic list of points (any first point, either
/// sense), compared exactly (−0 equal to 0). The same SET of points is
/// not enough: an asymmetric outline can have a symmetric set of
/// corners. Any difference, however small, has the turned orientation
/// checked as well.
fn symmetric(points: &[[f64; 2]]) -> bool {
    let key = |q: &[f64; 2]| ((q[0] + 0.0).to_bits(), (q[1] + 0.0).to_bits());
    let a: Vec<(u64, u64)> = points.iter().map(key).collect();
    let m: Vec<(u64, u64)> = mirrored(points).iter().map(key).collect();
    let back: Vec<(u64, u64)> = m.iter().rev().copied().collect();
    let n = a.len();
    if n == 0 || m.len() != n {
        return false;
    }
    (0..n).any(|shift| {
        [&m, &back]
            .iter()
            .any(|c| (0..n).all(|i| a.get(i) == c.get((i + shift) % n)))
    })
}

/// The order given, checked against the part: each stroke's flanges on
/// one line with its first ([`one_line`]), every flange once; with
/// one-piece tools (not `segmented`) each stroke the whole of its group
/// of [`strokes`], since a punch along the line bends every bend on it at
/// once (a group split would check a step with a bend flat that the
/// brake has bent).
fn checked_order(
    part: &sheetmetal::Part,
    tol: &Tolerance,
    order: &[Vec<usize>],
    segmented: bool,
) -> Result<Vec<Vec<usize>>, String> {
    let n = part.bends.len();
    let mut seen = BTreeSet::new();
    for g in order {
        let Some(&first) = g.first() else {
            return Err("`order` has an empty stroke".into());
        };
        for &b in g {
            if b >= n {
                return Err(format!(
                    "`order` names flanges by their index, 0 to {}; got {b}",
                    n.saturating_sub(1)
                ));
            }
            if !seen.insert(b) {
                return Err(format!("`order` lists flange {b} twice"));
            }
            if !one_line(part, first, b, tol) {
                return Err(format!(
                    "`order`: flanges {first} and {b} cannot be bent in one stroke: they are not on one line of one flat with the same angle, direction, radius and K-factor"
                ));
            }
        }
    }
    if seen.len() != n {
        let missing: Vec<usize> = (0..n).filter(|i| !seen.contains(i)).collect();
        return Err(format!("`order` leaves out flanges {missing:?}"));
    }
    if !segmented {
        let groups = strokes(part, tol);
        for g in order {
            let mut given: Vec<usize> = g.clone();
            given.sort_unstable();
            let Some(group) = g
                .first()
                .and_then(|b| groups.iter().find(|x| x.contains(b)))
            else {
                continue;
            };
            if given != *group {
                return Err(format!(
                    "`order`: with one-piece tools a punch along the line bends flanges {} in one stroke: give them together as one stroke, or set `segmented` true to bend them one by one",
                    stroke_words(group)
                ));
            }
        }
    }
    Ok(order.to_vec())
}

/// The tooling asked for, per bend of a part of `n`.
fn tooling_of(length: &Length, segmented: Option<bool>, n: usize) -> Result<Tooling, String> {
    match (length, segmented) {
        (Length::One(l), None | Some(false)) => Ok(Tooling::Whole(*l)),
        (Length::One(l), Some(true)) => Ok(Tooling::Segmented(vec![*l; n])),
        (Length::PerFlange(_), Some(false)) => Err(
            "`length` lists a length per flange, which is segmented tooling: leave out `segmented` or set it true"
                .into(),
        ),
        (Length::PerFlange(list), _) => {
            if list.len() == n {
                Ok(Tooling::Segmented(list.clone()))
            } else {
                Err(format!(
                    "`length` lists one length per flange ({n}), got {}",
                    list.len()
                ))
            }
        }
    }
}

/// Searches for an order in which part `feature` of `doc` can be bent
/// with `tools`, or checks the order given (the command's and the
/// measure's one proof).
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
    let n = part.bends.len();
    let tooling = tooling_of(&tools.length, tools.segmented, n).map_err(Stopped::Error)?;
    let given = match &tools.order {
        Some(order) => {
            Some(checked_order(part, &tol, order, tooling.segmented()).map_err(Stopped::Error)?)
        }
        None => None,
    };
    // The tools must be simple outlines: checked once here, so a bad one
    // is an error before any search.
    let unit = craft3d_math::Frame::WORLD;
    for (name, points) in [("punch", &tools.punch), ("die", &tools.die)] {
        // As tall as the polygon is wide, so the check holds at any scale.
        let size = points
            .iter()
            .flat_map(|p| p.iter())
            .fold(0.0_f64, |m, v| m.max(v.abs()))
            .max(tol.linear * 1e3);
        prism(points, &unit, 0.0, size, &tol)
            .map_err(|e| Stopped::Error(format!("`{name}`: {e}")))?;
    }
    let slabs = slabs_of(part, &tol).map_err(Stopped::Error)?;
    let pieces = pieces_of(part, &slabs, &tol).map_err(Stopped::Error)?;
    let turned_tools = (!(symmetric(&tools.punch) && symmetric(&tools.die)))
        .then(|| (mirrored(&tools.punch), mirrored(&tools.die)));
    let segments = match &tooling {
        Tooling::Whole(_) => None,
        Tooling::Segmented(lengths) => Some(
            part.bends
                .iter()
                .enumerate()
                .map(|(i, x)| segment_length(x, lengths.get(i).copied().flatten()))
                .collect(),
        ),
    };
    let check = Check {
        part,
        tol,
        punch: tools.punch,
        die: tools.die,
        turned_tools,
        tooling,
        clearance,
        pieces,
        built: final_worlds(part),
        work: std::cell::Cell::new(0),
        max_work: tools.max_work,
        ran_out: std::cell::Cell::new(false),
    };
    let stopped = |e: String| {
        if check.ran_out.get() {
            Stopped::OutOfWork(e)
        } else {
            Stopped::Error(e)
        }
    };
    let mut done = BTreeSet::new();
    let mut order: Vec<Step> = Vec::new();
    let mut tried: Tried = BTreeMap::new();
    let feasible = match &given {
        // The caller's order, checked step by step (every step, so each
        // one that does not clear is listed).
        Some(strokes) => {
            let mut every = true;
            for g in strokes {
                let r = check.step(&done, g).map_err(stopped)?;
                match &r {
                    Ok(cleared) => order.push((g.clone(), done.clone(), *cleared)),
                    Err(_) => every = false,
                }
                tried.insert((done.clone(), g.clone()), r);
                done.extend(g.iter().copied());
            }
            every
        }
        None => {
            let ranked = ranked_strokes(part, &tol, check.tooling.segmented());
            let mut dead = BTreeSet::new();
            let mut step = |done: &BTreeSet<usize>, g: &[usize]| check.step(done, g);
            search(
                &mut step, &mut done, &mut order, &mut dead, &mut tried, &ranked,
            )
            .map_err(stopped)?
        }
    };
    Ok(Proof {
        feasible,
        given: given.is_some(),
        order,
        tried,
        work: check.work.get(),
        clearance,
        segments,
    })
}

/// `length` of the command: one number for every bend, or a list of one
/// per flange (a number, or null for the bend's own length).
fn length_param(p: &Params) -> Result<Length, CommandError> {
    let id = p.command();
    let positive = |v: &Value| v.as_f64().filter(|l| l.is_finite() && *l > 0.0);
    match p.get("length") {
        None | Some(Value::Null) => Ok(Length::One(None)),
        Some(Value::Array(list)) => list
            .iter()
            .enumerate()
            .map(|(i, v)| match v {
                Value::Null => Ok(None),
                v => positive(v).map(Some).ok_or_else(|| {
                    CommandError::new(format!(
                        "{id}: `length` of flange {i} must be more than 0 or null"
                    ))
                }),
            })
            .collect::<Result<_, _>>()
            .map(Length::PerFlange),
        Some(v) => match positive(v) {
            Some(l) => Ok(Length::One(Some(l))),
            None => fail(format!(
                "{id}: `length` must be more than 0, or a list of one per flange"
            )),
        },
    }
}

/// `order` of the command as given: strokes, each a flange index or a
/// list of them (checked against the part in [`checked_order`]).
fn order_param(p: &Params) -> Result<Option<Vec<Vec<usize>>>, CommandError> {
    let id = p.command();
    let list = match p.get("order") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(list)) => list,
        Some(_) => {
            return fail(format!(
                "{id}: `order` is a list of flanges (indices), each a number or a list of flanges bent in one stroke"
            ));
        }
    };
    let index = |v: &Value| -> Result<usize, CommandError> {
        v.as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .ok_or_else(|| {
                CommandError::new(format!(
                    "{id}: `order` names flanges by their index (whole numbers from 0)"
                ))
            })
    };
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        out.push(match item {
            Value::Array(many) => many.iter().map(index).collect::<Result<_, _>>()?,
            one => vec![index(one)?],
        });
    }
    Ok(Some(out))
}

/// `segmented` as given: true or false, or left out.
fn segmented_param(v: Option<&Value>) -> Result<Option<bool>, String> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err("`segmented` must be true or false".into()),
    }
}

fn bend_sequence(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "sheetMetal.bendSequence";
    let p = Params::new(
        ID,
        params,
        &[
            "feature",
            "punch",
            "die",
            "length",
            "segmented",
            "clearance",
            "order",
            "maxWork",
        ],
    )?;
    let punch = polygon(&p, "punch")?;
    let die = polygon(&p, "die")?;
    let length = length_param(&p)?;
    let segmented =
        segmented_param(p.get("segmented")).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let order = order_param(&p)?;
    let max_work = match p.get("maxWork") {
        None | Some(Value::Null) => MAX_WORK,
        Some(v) => match v.as_u64() {
            Some(n) if (1..=MAX_WORK_ASKED).contains(&n) => usize::try_from(n).unwrap_or(MAX_WORK),
            _ => {
                return fail(format!(
                    "{ID}: `maxWork` is a whole number from 1 to {MAX_WORK_ASKED}"
                ));
            }
        },
    };
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
        segmented,
        clearance,
        order,
        max_work,
    };
    let proof = prove(doc, &regen, feature, tools).map_err(|e| match e {
        Stopped::OutOfWork(e) | Stopped::Error(e) => CommandError::new(format!("{ID}: {e}")),
    })?;
    let feasible = proof.feasible;
    let steps: Vec<Value> = proof
        .order
        .iter()
        .map(|(g, before, c)| {
            let mut v = c.json(PUNCH);
            v.insert("flange".into(), json!(g.first()));
            v.insert("flanges".into(), json!(g));
            v.insert("done".into(), json!(before.iter().collect::<Vec<_>>()));
            v.insert("turned".into(), json!(c.turned));
            if let Some(lengths) = &proof.segments {
                let each: Vec<Value> = g
                    .iter()
                    .map(|&i| lengths.get(i).map_or(Value::Null, |l| json!(l)))
                    .collect();
                v.insert("segments".into(), Value::Array(each));
            }
            v.insert("die".into(), Value::Object(c.json(DIE)));
            Value::Object(v)
        })
        .collect();
    let collisions: Vec<Value> = proof
        .tried
        .iter()
        .filter_map(|((before, g), r)| r.as_ref().err().map(|f| (before, g, f)))
        .flat_map(|(before, g, failed)| failed.0.iter().map(move |f| (before, g, failed, f)))
        .take(MAX_LISTED)
        .map(|(before, g, failed, f)| {
            let mut v = serde_json::Map::new();
            v.insert("flange".into(), json!(g.first()));
            v.insert("flanges".into(), json!(g));
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
            // This orientation's failure, and the step's (every
            // orientation tried fails for certain).
            v.insert("certain".into(), json!(f.certain(proof.clearance)));
            v.insert("stepCertain".into(), json!(failed.certain(proof.clearance)));
            Value::Object(v)
        })
        .collect();
    // Whether no order is proven (for an order given: that it fails);
    // null when it clears.
    let proven = if feasible {
        Value::Null
    } else {
        json!(matches!(proof.verdict(""), Verdict::Collides(_)))
    };
    Ok(json!({
        "feature": feature.to_string(),
        "feasible": feasible,
        "infeasibleProven": proven,
        "given": proof.given,
        "order": if feasible { json!(proof.order.iter().map(|(g, _, _)| stroke_json(g)).collect::<Vec<_>>()) } else { Value::Null },
        "steps": if feasible || proof.given { json!(steps) } else { json!([]) },
        "collisions": collisions,
        "clearance": proof.clearance,
        "unchecked": unchecked(proof.segments.is_some()),
        "work": proof.work,
        "lengthUnit": length_unit(doc),
    }))
}

// ------------------------------------------------------------- the measure

/// The parameters of the assertion measure `bendable`.
pub(crate) const MEASURE_KEYS: [&str; 7] = [
    "of",
    "punch",
    "die",
    "length",
    "segmented",
    "clearance",
    "maxWork",
];

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
/// 20000 distances). The tools are one piece reaching past the part
/// unless `length` says how long or `segmented` asks for segmented
/// tooling (a segment per bend), as in the command.
pub(crate) fn take(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    crate::verify_cmds::only(p, &MEASURE_KEYS)?;
    let punch = tool_points(ctx, p, "punch")?;
    let die = tool_points(ctx, p, "die")?;
    let length = positive(ctx, p, "length")?;
    let clearance = positive(ctx, p, "clearance")?;
    let segmented = segmented_param(p.get("segmented"))?;
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
            length: Length::One(length),
            segmented,
            clearance,
            order: None,
            max_work: left,
        };
        match prove(ctx.doc, ctx.regen, id, tools) {
            Ok(proof) => {
                left = left.saturating_sub(proof.work);
                one.insert("work".into(), proof.work.into());
                match proof.verdict(&name) {
                    Verdict::Bendable(order) => {
                        let words: Vec<String> = order.iter().map(|g| stroke_words(g)).collect();
                        orders.push(format!("`{name}` in the order {}", words.join(", ")));
                        one.insert("verdict".into(), "bendable".into());
                        one.insert(
                            "order".into(),
                            order
                                .iter()
                                .map(|g| stroke_json(g))
                                .collect::<Vec<_>>()
                                .into(),
                        );
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
    out.detail.insert(
        "unchecked".into(),
        unchecked(segmented == Some(true)).into(),
    );
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
    use craft3d_math::Portable;

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
                tooling: Tooling::Whole(None),
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
            let diagonal = (hi[0] - lo[0]).phypot(hi[1] - lo[1]);
            let b = 1;
            let bend = &part.bends[b];
            let old_premise = diagonal + 2.0 * (bend.radius + bend.thickness);
            for done in [BTreeSet::from([0]), BTreeSet::new()] {
                let stroke = check.stroke(&done, &[b]).unwrap();
                let steps = 256;
                let theta = bend.theta();
                let h = theta / steps as f64;
                let mut farthest = 0.0_f64;
                let mut fastest = 0.0_f64;
                let mut previous: Option<Vec<Vec3>> = None;
                for j in 0..=steps {
                    let rho = h * j as f64;
                    let worlds = stage_worlds(part, &check.angles(&done, &[b], rho), &tol).unwrap();
                    let frame = bend_frame(part, b, rho, &worlds).unwrap();
                    let into_tool = Transform::from_frame(&frame.tool(0.0).unwrap())
                        .inverse()
                        .unwrap();
                    let mut now = Vec::new();
                    let mut seen = BTreeSet::new();
                    for piece in
                        check
                            .checked(&done, &[b], PUNCH)
                            .chain(check.checked(&done, &[b], DIE))
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
            tooling: Tooling::Whole(None),
            clearance: tol.linear,
            pieces: pieces_of(part, &slabs, &tol).unwrap(),
            built: final_worlds(part),
            work: std::cell::Cell::new(0),
            max_work: MAX_WORK,
            ran_out: std::cell::Cell::new(false),
        };
        let done = BTreeSet::new();
        assert!(check.stroke(&done, &[1]).is_ok());
        assert!(check.heights(&done, &[1], 0.1, &[Kind::Flat(0)]).is_ok());
        // A piece the part does not have: no lowest point.
        let err = check
            .heights(&done, &[1], 0.1, &[Kind::Bend(7)])
            .unwrap_err();
        assert!(err.contains("missing") || err.contains("lowest"), "{err}");
        let base = check
            .pieces
            .iter_mut()
            .find(|p| p.what == Kind::Flat(0))
            .unwrap();
        base.corners.clear();
        let err = check.stroke(&done, &[1]).unwrap_err();
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
            given: false,
            order: Vec::new(),
            tried: tried
                .into_iter()
                .enumerate()
                .map(|(b, r)| ((BTreeSet::new(), vec![b]), r.map_err(|f| Failed(vec![f]))))
                .collect(),
            work: 0,
            clearance: 0.1,
            segments: None,
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
        let c = 0.1;
        assert!(turn_or(certain.clone(), Ok(cleared)).is_ok_and(|c| c.turned));
        // Both failures are kept; the step is reported by the one not
        // certain and fails for certain only when both do.
        let f = turn_or(certain.clone(), Err(turned_doubtful)).unwrap_err();
        assert_eq!(f.0.len(), 2);
        assert!(!f.certain(c));
        let r = f.reported(c).unwrap();
        assert!(!r.certain(c) && matches!(r, Failure::Collision { turned: true, .. }));
        let f = turn_or(certain.clone(), Err(turned_interval)).unwrap_err();
        assert!(!f.certain(c));
        assert!(matches!(
            f.reported(c),
            Some(Failure::NotProven { turned: true, .. })
        ));
        let f = turn_or(certain.clone(), Err(turned_certain)).unwrap_err();
        assert!(f.certain(c));
        let r = f.reported(c).unwrap();
        assert!(r.certain(c) && matches!(r, Failure::Collision { turned: false, .. }));
        // Given doubtful: it stays, whatever turned finds.
        let doubtful = collision(0.2, 0.05, false);
        let f = turn_or(doubtful, Err(certain)).unwrap_err();
        assert!(!f.certain(c));
        assert!(matches!(
            f.reported(c),
            Some(Failure::Collision { turned: false, .. })
        ));
        // A symmetric tool is its own mirror image, whichever point it
        // starts at and either way round; −0 is 0.
        let straight = [[-5.0, 0.0], [5.0, 0.0], [5.0, 100.0], [-5.0, 100.0]];
        assert!(symmetric(&straight));
        assert!(symmetric(&[
            [5.0, 100.0],
            [-5.0, 100.0],
            [-5.0, 0.0],
            [5.0, 0.0]
        ]));
        assert!(symmetric(&[
            [0.0, 0.0],
            [5.0, 10.0],
            [-0.0, 20.0],
            [-5.0, 10.0]
        ]));
        let goose = [[-25.0, 0.0], [5.0, 0.0], [5.0, 30.0], [-25.0, 30.0]];
        assert!(!symmetric(&goose));
    }

    /// An asymmetric outline whose set of points is symmetric (review
    /// 2026-10-29): compared as a set it was taken for its own mirror
    /// image, so the part was never turned and a step could be a false
    /// certain collision. As a cyclic list it is not symmetric.
    #[test]
    fn a_symmetric_set_of_points_is_not_a_symmetric_tool() {
        let tool = [[-2.0, 0.0], [2.0, 0.0], [1.0, 2.0], [-1.0, 2.0], [0.0, 1.0]];
        let mut set: Vec<[f64; 2]> = mirrored(&tool);
        let mut own = tool.to_vec();
        let order = |p: &[f64; 2], q: &[f64; 2]| p[0].total_cmp(&q[0]).then(p[1].total_cmp(&q[1]));
        set.sort_by(order);
        own.sort_by(order);
        let zero = |v: Vec<[f64; 2]>| -> Vec<[f64; 2]> {
            v.into_iter().map(|q| [q[0] + 0.0, q[1] + 0.0]).collect()
        };
        assert_eq!(zero(set), zero(own), "the same set of points");
        assert!(!symmetric(&tool));
        // Its mirror image is a polygon of its own, not symmetric either.
        assert!(!symmetric(&mirrored(&tool)));
    }

    /// An order given fails for certain when a step of it does, and is
    /// otherwise not proven.
    #[test]
    fn an_order_given_collides_when_a_step_of_it_does() {
        let mut p = proof(vec![Err(collision(0.2, 0.05, false))]);
        p.given = true;
        assert!(matches!(p.verdict("U"), Verdict::NotProven(_)));
        let mut p = proof(vec![
            Err(collision(0.2, 0.05, false)),
            Err(collision(0.0, 0.0, false)),
        ]);
        p.given = true;
        let Verdict::Collides(why) = p.verdict("U") else {
            panic!("{:?}", p.verdict("U"));
        };
        assert!(why.contains("order given"), "{why}");
    }

    /// A step that fails for certain as the part lies but only doubtfully
    /// turned is not a certain failure: no order is not proven
    /// (`notProven`, the command's infeasibleProven false).
    #[test]
    fn a_step_certain_one_way_and_doubtful_the_other_is_not_proven() {
        let certain = collision(0.05, 0.0, false);
        let doubtful = collision(0.2, 0.05, true);
        let mut p = proof(Vec::new());
        p.tried.insert(
            (BTreeSet::new(), vec![0]),
            Err(Failed(vec![certain, doubtful])),
        );
        let Verdict::NotProven(why) = p.verdict("U") else {
            panic!("{:?}", p.verdict("U"));
        };
        assert!(why.contains("turned end for end"), "{why}");
        assert!(!matches!(p.verdict(""), Verdict::Collides(_)));
    }

    /// Segmented tooling must not force a line's bends into one stroke
    /// (review 2026-10-29). Bends 0 and 1 lie on one line, 2 elsewhere:
    /// 0 is blocked once 2 stands, and 2 once 1 stands, so the only order
    /// is 0, 2, 1. Searched with the line as one stroke, both orders fail
    /// for certain, a false "cannot be bent"; with the bends also one by
    /// one (`ranked_strokes` with segmented tooling), the order is found.
    #[test]
    fn segmented_tooling_does_not_force_a_stroke() {
        fn step(done: &BTreeSet<usize>, g: &[usize]) -> Result<Result<Cleared, Failed>, String> {
            let blocked =
                (g.contains(&0) && done.contains(&2)) || (g.contains(&2) && done.contains(&1));
            Ok(if blocked {
                Err(Failed(vec![collision(0.0, 0.0, false)]))
            } else {
                Ok(Cleared {
                    least: [1.0; 2],
                    at: [0.0; 2],
                    proven: [1.0; 2],
                    turned: false,
                })
            })
        }
        let run = |ranked: &[Vec<usize>]| {
            let mut p = proof(Vec::new());
            let (mut done, mut dead) = (BTreeSet::new(), BTreeSet::new());
            let mut order = Vec::new();
            p.feasible = search(
                &mut step,
                &mut done,
                &mut order,
                &mut dead,
                &mut p.tried,
                ranked,
            )
            .unwrap();
            p.order = order;
            p
        };
        let forced = run(&[vec![0, 1], vec![2]]);
        assert!(matches!(forced.verdict("U"), Verdict::Collides(_)));
        let options = run(&[vec![0, 1], vec![2], vec![0], vec![1]]);
        assert_eq!(
            options.verdict("U"),
            Verdict::Bendable(vec![vec![0], vec![2], vec![1]])
        );
    }
}
