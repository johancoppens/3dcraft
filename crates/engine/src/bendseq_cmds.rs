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
use craft3d_kernel::sheetmetal::sequence::Tool;
use craft3d_kernel::sheetmetal::sequence::{
    PieceOf, bend_faces, bend_frame, final_worlds, from_axis, inside_piece, piece_lowest,
    piece_lowest_within, piece_points, stage_worlds, strip_slab, stroke_speed,
};
use craft3d_kernel::sheetmetal::{self, cut_pieces, flange_length, flat_faces};
use craft3d_math::Portable;
use craft3d_math::{Tolerance, Transform, Vec3};
use craft3d_topo::Body;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "sheetMetal.bendSequence",
    "Bend Sequence",
    r#"{"feature": id or name, "punch": [[x, y] or {through: [x, y]}, ...], "die": [[x, y] or {through: [x, y]}, ...], "length": number or [number or null, ...] (one per flange) = the part's reach, "segmented": boolean = false, "clearance": number = the document's tolerance, "order": [flange or [flange, ...], ...] = searched, "formed": [flange, ...] = none, "hem": {flanges: [flange, ...], preBend: degrees, pad: [[x, y] or {through: [x, y]}, ...]} = none, "maxWork": number = 1000000}"#,
    "Checks whether a sheet-metal part (feature.sheetMetal) can be bent on a press brake with the punch and die given, and finds an order of its bends (changes nothing). punch and die are closed outlines (at least 3 points, in the document's length unit; an object {through: [x, y]} between two points makes the side between them the arc through that point, so a punch's round tip or a die's shoulder radius is exact, not a polygon) in the plane square to a bend: the punch's with the centre of its tip at the origin and its body along +y, the die's with the point under the bend on the part's outer skin at the origin and its body below (y < 0); each is extruded along the bend: one piece length long centred on the middle of the bends made in the stroke (default: long enough to cover the whole part), or, with segmented true or a list of lengths, segmented tooling set up per bend, a segment per bend centred on its middle, as long as length gives for it (one number for every bend, or a list of one per flange with null for the bend's own) or else as long as the bend; unchecked states which. Bends on one line of one flat with the same angle, direction, radius and K-factor (tabs on one edge) are made in one stroke. At every bend both stand in the bend's frame: origin on the bend's axis (the die's R + T further out), y along the bisector of the two flats towards the inside (as on a press brake where both legs rise half the angle), z along the axis. A step, a bend made with some others already made and the rest still flat, clears when the proven distance (its lower bound) between the punch and every piece of the part but that bend's own strip and its two flats, and between the die counted lowered (moved away from the punch) by clearance plus the document's tolerance and every piece but the bend's own strip, is at least clearance over the whole stroke from 0 to the bend's angle (so the sheet may rest on the die's top, but not go into it by more than the tolerance nor come nearer than clearance from any other side), with the part as it lies or turned end for end (about the tools' y, as an operator turns it: the tools mirrored in x; segments stay at their bends), unless both tools are their own mirror image (the same outline, compared as a cyclic list of points and arcs): proven at eight stroke angles, and between two of them less how far any point of the part can move against the tools in between (half its distance from the bend's axis, the farthest at either end of the stroke, plus the bend's neutral radius, per radian), the interval halved (to 1/1024 of the stroke) until that holds, or, for a die wholly below its top (y <= 0), by the piece's lowest point staying above the die's top less the tolerance between them (its height bounded by its heights there and how fast it can bend, halved as needed: what proves a piece lifting off the die), or by the piece cut at its bend line (material past it beside the bend's ends, such as ears or a closed corner's extension, goes down into the die's opening) with each half kept apart from each triangle of the die (a die of at most 200 points) along the tools' y or a side's outward normal, bounded and halved the same way; a piece and a tool wholly inside one another count as 0. formed lists flanges made before the press brake by a tool of their own (a louvre on a punch press, a jog by an offset die, a hem closed by a flattening tool): they are made from the start, are no step of the order and are not checked themselves (said in unchecked). hem lists flanges bent 180 degrees that are pressed flat: each is two strokes of its own, an air bend with the punch and die to preBend degrees, then a flatten stroke from there to 180 between a flat die (the half space under the parent's outer skin, which lies still on it) and the pad (an outline in the plane square to the bend, its body above its y = 0, standing on the hem flat's highest point at every angle), checked as the other steps are: the pad against every piece but the hem's flat and strip, the flat die by every piece's lowest point, proven between angles by how fast the hem's side can move (its farthest point from the axis plus the neutral radius per radian); the steps list both, stage pre-bend and flatten (pad and die least, at and proven). With order (a list of strokes, each a flange or a list of flanges on one line; every flange once) that order is checked step by step, every step, instead of a search. Otherwise the order is searched depth first over all orders of the strokes (with one-piece tools a stroke is every bend on its line, and an order given must keep them together; with segmented tooling the bends of a line are tried together and one by one), the bends whose flanges carry the fewest others first (then the shorter flange, then the lower index), every set of bends made that leads nowhere remembered. Returns feasible, given (whether the order was given), order (the strokes, first bent first: a flange by index, or a list for a stroke of several; null when none clears), steps (per stroke of the order, also those that cleared of an order given that does not: flange (its first), flanges, segments (with segmented tooling: the tools' length at each of its bends), done (the flanges made before it), least (the least distance to the punch proven at the stroke angles measured), at (the stroke angle of it, degrees), proven (the least distance to the punch proven over the whole stroke, between the angles measured too) die (least, at and proven for the lowered die) and turned (whether the part is turned end for end for it)), collisions (every step tried that did not clear, per orientation tried: flange, flanges, done, tool (punch or die), piece (the base, flange i, or the strip of flange i, bent or flat), angle (degrees), distance and lower (the distance found and what is proven below it; to the lowered die for the die), or notProven with the stroke interval no halving could prove; certain: whether this failure is certain, a tool and a piece found nearer than the clearance at a real place or wholly inside one another, where a collision only not proven clear is not; stepCertain: whether the step fails for certain, both as the part lies and turned; turned: whether this failure is the turned one), infeasibleProven (with feasible false: whether every step tried that failed failed for certain, so that no order exists with these tools, or for an order given whether a step of it fails for certain; false means no order was found but one may exist), unchecked (what is not looked at) and work (distances measured). A part with a hole across a bend is checked with each strip not yet bent as its whole slab (more material than it has, so a clearance proven is proven; a collision with such a strip, or found inside a flat by its outline, is not certain). Refused: an outline that crosses itself, an arc whose point lies in a line with its side's ends, a clearance that is not more than 0, and more distances and heights measured than maxWork (default 1000000, some 5 to 8 minutes at most; at most 10000000; give the order, or raise it; the refusal says about how much the whole search would take). Not looked at (said in unchecked): a bend's own strip during its stroke, and its own two flats against the punch (an acute bend with a punch of a wider angle closing its flats onto it is not found), and a part's formed louvres' hoods (pressed before the press brake, no pieces of its stages).",
    doc: true,
    names: ["feature"],
    bend_sequence
)];

/// Most distances a check by the measure `bendable` measures, unless the
/// caller gives more (it runs on the assertions' one job thread).
const MAX_WORK: usize = 20_000;

/// Most distances `sheetMetal.bendSequence` measures unless the caller
/// gives more (2026-11-05: 20000 stopped the search on a chassis of 7
/// bends, which needed some 3 000 000). A whole search of a box of 7 bends
/// without an order that clears takes about 79 000 with one-piece tools
/// and 676 000 with segmented ones (each bend alone and its line
/// together), some 2000 to 3500 a second; the cap holds a search that
/// would blow up to some 5 to 8 minutes on the engine thread (covering
/// the whole search of 7 bends segmented), and a search that runs out
/// says how much the whole one would likely take.
const COMMAND_WORK: usize = 1_000_000;

/// Most distances a caller of the command may ask for (`maxWork`).
const MAX_WORK_ASKED: u64 = 10_000_000;
/// Most distances and heights the measure `bendable` may measure when it
/// checks an order given (a search stays at [`MAX_WORK`]): it runs on the
/// assertions' one job thread and in the web build's one thread.
const MEASURE_ORDER_WORK: usize = 1_000_000;

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
/// What `unchecked` says of a part's formed louvres (`{n}` their count).
const HOODS_UNCHECKED: &str = "the formed louvres' hoods ({n}): pressed on a punch press before the press brake, they are not pieces of its stages, so a tool or a piece that reaches over one is not checked against it";

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

fn polygon(p: &Params, key: &str) -> Result<Tool, CommandError> {
    let id = p.command();
    let Some(value) = p.get(key) else {
        return Err(p.missing(key));
    };
    tool_of(value, key, &mut |v: &Value| {
        v.as_f64().filter(|v| v.is_finite()).ok_or_else(String::new)
    })
    .map_err(|why| CommandError::new(format!("{id}: {why}")))
}

/// A tool's outline from its list: corners `[x, y]`, and between two
/// corners (or after the last, for the side back to the first) a
/// `{"through": [x, y]}` that makes that side the arc through the point;
/// each coordinate read by `number` (an error string, empty for "not a
/// number").
fn tool_of(
    value: &Value,
    key: &str,
    number: &mut dyn FnMut(&Value) -> Result<f64, String>,
) -> Result<Tool, String> {
    let bad = |why: &str| {
        format!(
            "`{key}` must be a list of at least 3 points [x, y] (finite numbers or length expressions, at most {MAX_POINTS}), a side an arc where a {{\"through\": [x, y]}} stands between its two points{why}"
        )
    };
    let list = value.as_array().ok_or_else(|| bad(""))?;
    if list.len() < 3 || list.len() > MAX_POINTS {
        return Err(bad(""));
    }
    let mut pair = |q: &Value| -> Result<[f64; 2], String> {
        let [x, y] = q.as_array().map(Vec::as_slice).ok_or_else(|| bad(""))? else {
            return Err(bad(""));
        };
        let read = |v: &Value, n: &mut dyn FnMut(&Value) -> Result<f64, String>| {
            n(v).map_err(|e| if e.is_empty() { bad("") } else { e })
        };
        Ok([read(x, number)?, read(y, number)?])
    };
    let mut corners: Vec<[f64; 2]> = Vec::new();
    let mut through: Vec<Option<[f64; 2]>> = Vec::new();
    for q in list {
        match q.as_object() {
            Some(o) => {
                let (Some(point), 1) = (o.get("through"), o.len()) else {
                    return Err(bad(
                        ": an object in it is {\"through\": [x, y]} and nothing else",
                    ));
                };
                match through.last_mut() {
                    Some(slot @ None) => *slot = Some(pair(point)?),
                    _ => {
                        return Err(bad(
                            ": a \"through\" follows a corner, never another \"through\" nor the list's start",
                        ));
                    }
                }
            }
            None => {
                corners.push(pair(q)?);
                through.push(None);
            }
        }
    }
    if corners.len() < 2 || (corners.len() == 2 && through.iter().any(Option::is_none)) {
        return Err(bad(": at least 3 corners, or 2 with both sides arcs"));
    }
    Ok(Tool { corners, through })
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
        /// Whether `distance` is between real places of the piece: not for
        /// a strip not yet bent that a hole across a bend crosses (its
        /// slab holds the hole's material too) nor for a point found
        /// inside a flat by its outline when holes cross its bends.
        real: bool,
    },
    NotProven {
        from: f64,
        to: f64,
        turned: bool,
    },
}

impl Failure {
    /// The failure as found with the part turned end for end or not.
    fn turned(self, turned: bool) -> Failure {
        match self {
            Failure::Collision {
                tool,
                piece,
                angle,
                distance,
                lower,
                real,
                ..
            } => Failure::Collision {
                tool,
                piece,
                angle,
                distance,
                lower,
                turned,
                real,
            },
            Failure::NotProven { from, to, .. } => Failure::NotProven { from, to, turned },
        }
    }

    /// Whether the step fails for certain: a tool and a piece found at a
    /// real pair of places (or wholly inside one another) nearer than the
    /// clearance, so the true distance is nearer too. A collision only
    /// not proven clear (the distance found at least the clearance, what
    /// is proven below it), or an interval of the stroke no halving
    /// proved, is not.
    fn certain(&self, clearance: f64) -> bool {
        matches!(self, Failure::Collision { distance, real: true, .. } if *distance < clearance)
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
    /// For a hem, its flatten stroke after this pre-bend.
    flat: Option<Flatten>,
}

/// A hem's flatten stroke cleared: per tool (the pad, the flat die) the
/// least distance proven at the stroke angles measured, its angle and the
/// least proven over the whole stroke; and whether the part is turned end
/// for end (the pad mirrored).
#[derive(Clone, Copy, Debug)]
struct Flatten {
    least: [f64; 2],
    at: [f64; 2],
    proven: [f64; 2],
    turned: bool,
}

/// The hems pressed flat (`hem`): the flanges, the angle the air bend
/// takes them to first (the pre-bend), and the pad that presses them flat
/// (as given, and mirrored for the part turned end for end unless it is
/// its own mirror image).
#[derive(Clone, Debug)]
struct Hem {
    flanges: BTreeSet<usize>,
    pre: f64,
    pad: Tool,
    turned_pad: Option<Tool>,
}

/// The flatten stroke's tools in [`Check::flatten_at`]'s keys.
const PAD: usize = 0;
const FLAT_DIE: usize = 1;
const FLATTEN_TOOLS: [&str; 2] = ["pad", "flat die"];

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
    /// Where the stroke ends: the bends' angle, or a hem's pre-bend.
    until: f64,
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

/// What [`Check::near`] finds between a tool and a piece.
#[derive(Clone, Copy, Debug)]
struct Near {
    lower: f64,
    found: f64,
    inside: bool,
    real: bool,
}

/// Per tool and piece checked: the distance proven at one stroke angle.
type Lowers = BTreeMap<(usize, Kind), f64>;

/// Deepest the rest proof halves an interval (below the stroke's
/// sampling and halving).
const MAX_REST_DEPTH: u32 = 40;

/// Directions per half of a piece and piece of the die in
/// [`Check::apart_values`]: the tools' `y` and the die piece's three
/// sides' outward normals.
const DIRECTIONS: usize = 4;

/// A tool's outline in the plane square to a bend.
type Polygon = Tool;

/// A convex piece of a tool's outline, counter-clockwise.
type Triangle = [[f64; 2]; 3];

/// Most points of a die cut into convex pieces for the proof of pieces
/// cut at their bend line (the cutting is quadratic or worse in them).
const MAX_CUT_POINTS: usize = 200;

/// The die cut into triangles (its outline held by a polygon, an arc by
/// its tangents where it bulges out: `Tool::enclosing`), `None` when it
/// has too many points or cannot be cut.
fn die_pieces(die: &Tool) -> Option<Vec<Triangle>> {
    let polygon = die.enclosing()?;
    (polygon.len() <= MAX_CUT_POINTS)
        .then(|| sheetmetal::sequence::triangles(&polygon))
        .flatten()
}

struct Check<'a> {
    part: &'a sheetmetal::Part,
    tol: Tolerance,
    punch: Tool,
    die: Tool,
    /// The tools mirrored in `x` (`mirrored`): where they stand against
    /// the part turned end for end; `None` when both are their own
    /// mirror image (`symmetric`), so turning changes nothing.
    turned_tools: Option<(Polygon, Polygon)>,
    /// The die cut into convex pieces (`sequence::triangles`), as given
    /// and mirrored for the part turned end for end: what the proof of a
    /// piece cut at its bend line keeps apart ([`Check::apart`]); `None`
    /// when the die could not be cut (that proof is then not tried).
    die_pieces: [Option<Vec<Triangle>>; 2],
    /// The hems pressed flat, if any (`hem`).
    hem: Option<Hem>,
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
    #[cfg(test)]
    fn stroke(&self, done: &BTreeSet<usize>, g: &[usize]) -> Result<Stroke, String> {
        self.stroke_until(done, g, None)
    }

    /// [`Check::stroke`] of a stroke that ends at `until` (a hem's
    /// pre-bend) instead of the bends' angle.
    fn stroke_until(
        &self,
        done: &BTreeSet<usize>,
        g: &[usize],
        until: Option<f64>,
    ) -> Result<Stroke, String> {
        let part = self.part;
        let b = *g.first().ok_or("a stroke has no bend")?;
        let first = part.bends.get(b).ok_or("a bend is missing")?;
        let theta = until.unwrap_or_else(|| first.theta());
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
            die_below: self.die.top() <= 0.0,
            turned: false,
            until: theta,
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
            let nears = self.near(
                name,
                points,
                &tool_frame,
                &stroke.segments,
                &worlds,
                &pieces,
            )?;
            for (piece, near) in pieces.into_iter().zip(nears) {
                let Near {
                    mut lower,
                    found,
                    inside,
                    real,
                } = near;
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
                // Or cut at its bend line, each half kept apart from each
                // convex piece of the die along a direction of its own.
                if !inside && lower < self.clearance && tool == DIE && self.can_part(stroke) {
                    let v = self.apart_values(done, g, rho, &[piece.what], stroke)?;
                    let apart = v
                        .chunks(DIRECTIONS)
                        .map(|d| d.iter().copied().fold(f64::NEG_INFINITY, f64::max))
                        .fold(f64::INFINITY, f64::min);
                    if apart >= self.clearance {
                        lower = apart;
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
                        real,
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

    /// The least distance proven (and found) between the tool `points`
    /// standing in `tool_frame` (extruded over `segments` along its z) and
    /// each of `pieces` at the stage `worlds`, and whether they lie wholly
    /// inside one another and the distance is between real places of the
    /// piece (*Holes across a bend*).
    fn near(
        &self,
        name: &str,
        points: &Tool,
        tool_frame: &craft3d_math::Frame,
        segments: &[(f64, f64)],
        worlds: &[Transform],
        pieces: &[&Piece<'_>],
    ) -> Result<Vec<Near>, String> {
        let part = self.part;
        // The strips and flats a hole across a bend may lie in (by
        // boxes: a strip left out is its slab exactly).
        let (holed_strips, holed_flats) = cut_pieces(part, &self.tol);
        // The tool carried into each carrier's built place, built once
        // per carrier and segment (every piece it carries shares it).
        let carriers: Vec<usize> = pieces
            .iter()
            .map(|p| p.carrier)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut bodies: Vec<(usize, Body, craft3d_math::Frame, f64)> =
            Vec::with_capacity(carriers.len().saturating_mul(segments.len()));
        for &carrier in &carriers {
            let (Some(stage), Some(built)) = (worlds.get(carrier), self.built.get(carrier)) else {
                return Err("a flat's place is missing".into());
            };
            // The tool where the piece stands in the built part.
            let back = stage
                .inverse()
                .ok_or("a stage's place cannot be undone")?
                .then(built);
            let local = back
                .frame(tool_frame)
                .ok_or("a tool's frame is not finite")?;
            for &(z0, z1) in segments {
                let body = points
                    .prism(&local, z0, z1, &self.tol)
                    .map_err(|e| format!("the {name}: {e}"))?;
                bodies.push((carrier, body, local, 0.5 * (z0 + z1)));
            }
        }
        let measured: Vec<Measured<'_>> = bodies
            .iter()
            .map(|(_, body, _, _)| Measured::new(body, None, &self.tol))
            .collect::<Result<_, _>>()?;
        let mut out = Vec::with_capacity(pieces.len());
        for piece in pieces {
            // The least over the tool's pieces along the axis.
            let mut lower = f64::INFINITY;
            let mut found = f64::INFINITY;
            let mut inside = false;
            // A strip not yet bent that a hole across a bend may cross
            // is its slab whole (more material than it has: a clearance
            // proven to it is proven, a collision found with it may lie
            // in the hole), and such a flat's outline knows no hole
            // that crosses its tangent line.
            let mut real = !matches!(piece.what, Kind::Strip(i) if holed_strips.contains(&i));
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
                    .corners
                    .first()
                    .map(|q| local.point(q[0], q[1], *zmid))
                    .ok_or("a tool has no points")?;
                let by_points = contains_points(body, &piece.points, &self.tol)
                    .map_err(|e| e.to_string())?
                    .contains(&Some(true));
                let by_outline = inside_piece(part, piece.what.piece(), tool_point);
                if by_outline
                    && !by_points
                    && match piece.what {
                        Kind::Flat(f) => holed_flats.contains(&f),
                        // A bend made or a strip not yet bent: its
                        // ring sector or slab knows no hole across it
                        // either (review 2026-10-31: a made bend was
                        // left out, so a tool point in its hole was a
                        // certain collision, a false no).
                        Kind::Bend(i) | Kind::Strip(i) => holed_strips.contains(&i),
                    }
                {
                    real = false;
                }
                let within = by_outline || by_points;
                inside |= within;
                lower = lower.min(if within { 0.0 } else { near.lower.max(0.0) });
                found = found.min(if within { 0.0 } else { near.distance });
            }
            if !lower.is_finite() {
                return Err("a tool is missing".into());
            }
            out.push(Near {
                lower,
                found,
                inside,
                real,
            });
        }
        Ok(out)
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
        // A hem: the air bend to its pre-bend, then the flatten stroke.
        if let (Some(hem), [b]) = (&self.hem, g)
            && hem.flanges.contains(b)
        {
            let pre = match self.step_until(done, g, Some(hem.pre))? {
                Ok(cleared) => cleared,
                Err(failed) => return Ok(Err(failed)),
            };
            return Ok(match self.flatten(done, *b)? {
                Ok(flat) => Ok(Cleared {
                    flat: Some(flat),
                    ..pre
                }),
                Err(failed) => Err(failed),
            });
        }
        self.step_until(done, g, None)
    }

    /// [`Check::step`] of a stroke that ends at `until` (a hem's pre-bend)
    /// or at the bends' angle.
    fn step_until(
        &self,
        done: &BTreeSet<usize>,
        g: &[usize],
        until: Option<f64>,
    ) -> Result<Result<Cleared, Failed>, String> {
        let stroke = self.stroke_until(done, g, until)?;
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

    /// A hem's flatten stroke (bend `b` from its pre-bend to its angle,
    /// with `done` made), the part as it lies or turned end for end (the
    /// pad mirrored): it clears when either clears ([`turn_or`]).
    fn flatten(&self, done: &BTreeSet<usize>, b: usize) -> Result<Result<Flatten, Failed>, String> {
        let hem = self.hem.as_ref().ok_or("no hem is asked for")?;
        let given = match self.flatten_as(done, b, false)? {
            Ok(f) => return Ok(Ok(f)),
            Err(f) => f,
        };
        if hem.turned_pad.is_none() {
            return Ok(Err(Failed(vec![given])));
        }
        Ok(match self.flatten_as(done, b, true)? {
            Ok(f) => Ok(f),
            Err(other) => Err(Failed(vec![given, other])),
        })
    }

    /// Whether the child side of bend `b` carries flat `flat` (its child
    /// or a flat beyond it).
    fn on_child(&self, b: usize, mut flat: usize) -> bool {
        let Some(child) = self.part.bends.get(b).map(|x| x.child) else {
            return false;
        };
        for _ in 0..=self.part.flats.len() {
            if flat == child {
                return true;
            }
            match self.part.bends.iter().find(|x| x.child == flat) {
                Some(x) => flat = x.parent,
                None => return false,
            }
        }
        false
    }

    /// The flatten stroke of hem `b`, the pad as given or mirrored. The
    /// parent lies on the flat die, still: the stroke's frame is the
    /// parent's (origin on its outer skin under the hem's axis at the
    /// bend's middle, `y` its normal towards the inside of the hem, `z`
    /// the axis, `x = y × z`), the die the half space `y ≤ 0` counted
    /// lowered by the clearance and the tolerance, the pad's outline (its
    /// body above its `y = 0`) standing on the highest point of the hem's
    /// flat at each angle. Checked: the pad against every piece but the
    /// hem's flat and strip (what it presses), the die against every
    /// piece but the strip, by its lowest point (at least `−tolerance`:
    /// then at least the clearance from the lowered die). Proven between
    /// stroke angles by how fast each piece can move against each tool:
    /// the child side turns about the axis at rate 1 after the shift back
    /// across the strip, so its points move at most `d + r` per radian
    /// (`d` the farthest of its hull points from the axis at either end of
    /// the stroke, a distance convex in the angle; `r` the neutral radius)
    /// and accelerate at most `d + 2r`; the pad moves as the flat's
    /// highest point, at most `d + r`; the parent's pieces are still
    /// against the die.
    fn flatten_as(
        &self,
        done: &BTreeSet<usize>,
        b: usize,
        turned: bool,
    ) -> Result<Result<Flatten, Failure>, String> {
        let part = self.part;
        let hem = self.hem.as_ref().ok_or("no hem is asked for")?;
        let pad = if turned {
            hem.turned_pad.as_ref().ok_or("the pad is not mirrored")?
        } else {
            &hem.pad
        };
        let x = part.bends.get(b).ok_or("a bend is missing")?;
        let (from, to) = (hem.pre, x.theta());
        // The tools along the axis, as for the pre-bend.
        let stroke = self.stroke_until(done, &[b], Some(to))?;
        // How far the child side's hull points reach from the axis at
        // either end of the stroke.
        let mut reach = 0.0_f64;
        for rho in [from, to] {
            let worlds = stage_worlds(part, &self.angles(done, &[b], rho), &self.tol)
                .map_err(|e| e.to_string())?;
            let (_, point, axis, _) = self.flatten_frame(b, &worlds)?;
            for piece in self
                .pieces
                .iter()
                .filter(|p| self.on_child(b, p.carrier) && p.what != Kind::Strip(b))
            {
                let moved = self.moved(piece, &worlds)?;
                if piece.corners.is_empty() {
                    return Err(format!("{} has no points to bound it", piece.what.text()));
                }
                for &c in &piece.corners {
                    reach = reach.max(from_axis(point, axis, moved.point(c)).0);
                }
            }
        }
        let r = x.neutral();
        let child_speed = reach + r;
        let child_bow = reach + 2.0 * r;
        if !(child_speed.is_finite() && child_bow.is_finite()) {
            return Err("the part has no finite size".into());
        }
        let speed = |tool: usize, kind: Kind| -> f64 {
            // A piece not found moves as the hem's side (the larger bound).
            let child = self
                .carrier_of(kind)
                .is_none_or(|carrier| self.on_child(b, carrier));
            match (tool, child) {
                (PAD, false) => child_speed,
                (PAD, true) => 2.0 * child_speed,
                (_, false) => 0.0,
                (_, true) => child_bow,
            }
        };
        let mut best = Flatten {
            least: [f64::INFINITY; 2],
            at: [from; 2],
            proven: [f64::INFINITY; 2],
            turned,
        };
        let mut prev: Option<(f64, Lowers)> = None;
        for j in 0..=SAMPLES {
            let rho = from + (to - from) * j as f64 / SAMPLES as f64;
            let l = match self.flatten_at(done, b, rho, pad, &stroke)? {
                Ok(l) => l,
                Err(f) => return Ok(Err(f.turned(turned))),
            };
            best.sampled(rho, &l);
            if let Some((a, la)) = &prev
                && let Err(f) = self.flatten_between(
                    done,
                    b,
                    (*a, la),
                    (rho, &l),
                    pad,
                    &stroke,
                    &speed,
                    0,
                    &mut best,
                )?
            {
                return Ok(Err(f.turned(turned)));
            }
            prev = Some((rho, l));
        }
        Ok(Ok(best))
    }

    /// The flat a piece is carried by.
    fn carrier_of(&self, kind: Kind) -> Option<usize> {
        self.pieces
            .iter()
            .find(|p| p.what == kind)
            .map(|p| p.carrier)
    }

    /// A piece's motion from the built part to the stage `worlds`.
    fn moved(&self, piece: &Piece<'_>, worlds: &[Transform]) -> Result<Transform, String> {
        self.built
            .get(piece.carrier)
            .and_then(Transform::inverse)
            .zip(worlds.get(piece.carrier))
            .map(|(back, stage)| back.then(stage))
            .ok_or_else(|| "a flat's place is missing".to_string())
    }

    /// The flatten stroke's frame of hem `b` at the stage `worlds`: the
    /// origin on the parent's outer skin under the axis at the bend's
    /// middle, a point of the axis, the axis, and `y` (the parent's normal
    /// towards the inside of the hem).
    fn flatten_frame(
        &self,
        b: usize,
        worlds: &[Transform],
    ) -> Result<(Vec3, Vec3, Vec3, Vec3), String> {
        let x = self.part.bends.get(b).ok_or("a bend is missing")?;
        let wp = worlds.get(x.parent).ok_or("a bend's parent is missing")?;
        let up = x.angle > 0.0;
        let t = x.thickness;
        let zc = if up { t + x.radius } else { -x.radius };
        let mid = [0.5 * (x.start[0] + x.end[0]), 0.5 * (x.start[1] + x.end[1])];
        let axis_point = wp.point(Vec3::new(mid[0], mid[1], zc));
        let axis = wp
            .vector(Vec3::new(x.end[0] - x.start[0], x.end[1] - x.start[1], 0.0))
            .normalized()
            .ok_or("a bend has no length")?;
        let n = wp.vector(Vec3::Z) * if up { 1.0 } else { -1.0 };
        let origin = axis_point - n * (x.radius + t);
        Ok((origin, axis_point, axis, n))
    }

    /// At flatten angle `rho` of hem `b`: per tool and piece checked the
    /// least distance proven (the pad's by the mesh, the die's the lowest
    /// point's height plus the lowering), or the failure.
    fn flatten_at(
        &self,
        done: &BTreeSet<usize>,
        b: usize,
        rho: f64,
        pad: &Tool,
        stroke: &Stroke,
    ) -> Result<Result<Lowers, Failure>, String> {
        let part = self.part;
        let x = part.bends.get(b).ok_or("a bend is missing")?;
        let worlds = stage_worlds(part, &self.angles(done, &[b], rho), &self.tol)
            .map_err(|e| e.to_string())?;
        let (origin, _, axis, n) = self.flatten_frame(b, &worlds)?;
        // The pad stands on the hem's flat's highest point.
        let flat = self
            .pieces
            .iter()
            .find(|p| p.what == Kind::Flat(x.child))
            .ok_or("a hem's flat is missing")?;
        // The highest point is exact on lines and arcs; on a spline it is
        // taken over the control points, which bounds it from above, so
        // the pad would stand higher than the flat and keep further from
        // the other pieces than it does (review of 4116ed3e): refused.
        if part.flats.get(x.child).is_some_and(|f| {
            f.outer
                .iter()
                .any(|seg| matches!(seg.shape, craft3d_kernel::profile::Shape::Spline(_)))
        }) {
            return Err(format!(
                "hem {b}'s flat has a spline in its outline, so where the pad stands on it (its highest point) is known only from above, which would place the pad too high: pressing it flat is not supported (not proven); draw the hem's far side with lines and arcs, or check its bend as an air bend (leave it out of `hem`)"
            ));
        }
        let moved = self.moved(flat, &worlds)?;
        self.count()?;
        let top = -piece_lowest(part, PieceOf::Flat(x.child), |p| {
            -(moved.point(p) - origin).dot(n)
        })
        .ok_or("a hem's flat has no highest point")?;
        let pad_frame = craft3d_math::Frame::new(origin + n * top, axis, n.cross(axis))
            .ok_or("the pad's frame is not finite")?;
        let stage: Vec<&Piece<'_>> = self
            .pieces
            .iter()
            .filter(|p| match p.what {
                Kind::Flat(_) => true,
                Kind::Bend(i) => done.contains(&i),
                Kind::Strip(i) => i != b && !done.contains(&i),
            })
            .collect();
        let pressed: Vec<&Piece<'_>> = stage
            .iter()
            .copied()
            .filter(|p| p.what != Kind::Flat(x.child))
            .collect();
        let mut least = Lowers::new();
        let mut doubtful: Option<Failure> = None;
        let nears = self.near("pad", pad, &pad_frame, &stroke.segments, &worlds, &pressed)?;
        for (piece, near) in pressed.iter().zip(nears) {
            if near.lower < self.clearance {
                let failure = Failure::Collision {
                    tool: FLATTEN_TOOLS[PAD],
                    piece: piece.what,
                    angle: rho,
                    distance: near.found,
                    lower: near.lower,
                    turned: false,
                    real: near.real,
                };
                if failure.certain(self.clearance) {
                    return Ok(Err(failure));
                }
                doubtful.get_or_insert(failure);
                continue;
            }
            least.insert((PAD, piece.what), near.lower);
        }
        // The flat die: every piece's lowest point at least `−tolerance`.
        let lowering = self.clearance + self.tol.linear;
        for piece in &stage {
            self.count()?;
            let moved = self.moved(piece, &worlds)?;
            let low = piece_lowest(part, piece.what.piece(), |p| {
                (moved.point(p) - origin).dot(n)
            })
            .ok_or_else(|| format!("{} has no lowest point", piece.what.text()))?;
            let apart = low + lowering;
            if apart < self.clearance {
                // Exact but for a spline's control points.
                let exact = match piece.what {
                    Kind::Flat(f) => part.flats.get(f).is_some_and(|flat| {
                        flat.outer
                            .iter()
                            .all(|s| !matches!(s.shape, craft3d_kernel::profile::Shape::Spline(_)))
                    }),
                    _ => true,
                };
                let failure = Failure::Collision {
                    tool: FLATTEN_TOOLS[FLAT_DIE],
                    piece: piece.what,
                    angle: rho,
                    distance: apart.max(0.0),
                    lower: apart.max(0.0),
                    turned: false,
                    real: exact,
                };
                if failure.certain(self.clearance) {
                    return Ok(Err(failure));
                }
                doubtful.get_or_insert(failure);
                continue;
            }
            least.insert((FLAT_DIE, piece.what), apart);
        }
        if let Some(failure) = doubtful {
            return Ok(Err(failure));
        }
        Ok(Ok(least))
    }

    /// Proves the flatten stroke between two angles whose distances are
    /// proven, halving it where a piece could come nearer in between: per
    /// tool and piece both distances less how far it can move (the pad's,
    /// `speed h / 2`) or bend (the die's heights, `bow h² / 8`), as
    /// [`Check::flatten_as`] bounds them.
    #[allow(clippy::too_many_arguments)]
    fn flatten_between(
        &self,
        done: &BTreeSet<usize>,
        b: usize,
        (a, la): (f64, &Lowers),
        (c, lc): (f64, &Lowers),
        pad: &Tool,
        stroke: &Stroke,
        speed: &dyn Fn(usize, Kind) -> f64,
        depth: u32,
        best: &mut Flatten,
    ) -> Result<Result<(), Failure>, String> {
        let h = c - a;
        let mut proven = [f64::INFINITY; 2];
        let mut ok = true;
        for (key, l) in la {
            let other = lc.get(key).copied().unwrap_or(f64::NEG_INFINITY);
            let v = speed(key.0, key.1);
            let slack = if key.0 == PAD {
                v * h * 0.5
            } else {
                v * h * h / 8.0
            };
            let lo = l.min(other) - slack;
            if lo >= self.clearance {
                if let Some(p) = proven.get_mut(key.0) {
                    *p = p.min(lo);
                }
            } else {
                ok = false;
                break;
            }
        }
        if ok {
            for (best, p) in best.proven.iter_mut().zip(proven) {
                *best = best.min(p);
            }
            return Ok(Ok(()));
        }
        if depth >= MAX_DEPTH {
            return Ok(Err(Failure::NotProven {
                from: a,
                to: c,
                turned: false,
            }));
        }
        let m = 0.5 * (a + c);
        let lm = match self.flatten_at(done, b, m, pad, stroke)? {
            Ok(l) => l,
            Err(f) => return Ok(Err(f)),
        };
        best.sampled(m, &lm);
        if let Err(f) = self.flatten_between(
            done,
            b,
            (a, la),
            (m, &lm),
            pad,
            stroke,
            speed,
            depth + 1,
            best,
        )? {
            return Ok(Err(f));
        }
        self.flatten_between(
            done,
            b,
            (m, &lm),
            (c, lc),
            pad,
            stroke,
            speed,
            depth + 1,
            best,
        )
    }

    /// Whether the bends `g` can be made with `done` made over their
    /// whole stroke, the part turned as `stroke.turned` says.
    fn step_as(
        &self,
        done: &BTreeSet<usize>,
        g: &[usize],
        stroke: &Stroke,
    ) -> Result<Result<Cleared, Failure>, String> {
        let theta = stroke.until;
        let mut best = Cleared {
            least: [f64::INFINITY; 2],
            at: [0.0; 2],
            proven: [f64::INFINITY; 2],
            turned: stroke.turned,
            flat: None,
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
            } else if key.0 == DIE && (stroke.die_below || self.can_part(stroke)) {
                resting.push(key.1);
            } else {
                resting.clear();
                proven = [f64::NEG_INFINITY; 2];
                break;
            }
        }
        if proven.iter().all(|p| *p >= self.clearance) {
            let mut rested = if resting.is_empty() {
                Some(f64::INFINITY)
            } else if stroke.die_below {
                let ya = self.heights(done, g, a, &resting)?;
                let yc = self.heights(done, g, c, &resting)?;
                self.rest(done, g, &resting, (a, &ya), (c, &yc), stroke, 0)?
            } else {
                None
            };
            // Material past the bend line beside its ends goes down into
            // the die's opening while the rest lies on its shoulder: the
            // pieces cut at the bend line, each half kept apart from each
            // convex piece of the die (`Check::apart`).
            if rested.is_none() && self.can_part(stroke) {
                let va = self.apart_values(done, g, a, &resting, stroke)?;
                let vc = self.apart_values(done, g, c, &resting, stroke)?;
                rested = self.apart(done, g, &resting, (a, &va), (c, &vc), stroke, 0)?;
            }
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

    /// Per piece of `kinds` at stroke angle `rho`, how far apart along a
    /// direction it lies from each convex piece of the die (lowered by
    /// the clearance and the tolerance, as [`Check::at`] counts it): the
    /// piece cut in two at its bend line, the half behind it and the half
    /// past it, and per half and per piece of the die [`DIRECTIONS`]
    /// values, one per direction (the tools' `y`, then the outward normal
    /// of each side of the die's piece), each the least of the direction
    /// over the half's material (`sequence::piece_lowest_within`, a lower
    /// bound) less the most of it over the die's piece. A half kept that
    /// far apart from every piece of the die along some direction keeps
    /// at least that from the die (the prisms of the die along the axis
    /// are no nearer than their sections, whatever their length).
    ///
    /// The cut is the plane through the axis square to the flat each
    /// piece moves with: the parent's tangent line for a piece carried by
    /// the parent's side, the child's for one carried by a child of the
    /// stroke (its far tangent line, fixed on the child). Each half is
    /// rigid, so the bound between two stroke angles holds for it
    /// ([`Check::apart`]); the half behind the line rises from the die as
    /// the leg rises, the half past it (material beside the bend's ends:
    /// a rack panel's ears, a closed corner's extension) goes down into
    /// the die's opening.
    fn apart_values(
        &self,
        done: &BTreeSet<usize>,
        g: &[usize],
        rho: f64,
        kinds: &[Kind],
        stroke: &Stroke,
    ) -> Result<Vec<f64>, String> {
        let part = self.part;
        self.count()?;
        let die = self
            .die_pieces
            .get(usize::from(stroke.turned))
            .and_then(Option::as_ref)
            .ok_or("the die is not cut into convex pieces")?;
        let b = *g.first().ok_or("a stroke has no bend")?;
        let first = part.bends.get(b).ok_or("a bend is missing")?;
        let worlds =
            stage_worlds(part, &self.angles(done, g, rho), &self.tol).map_err(|e| e.to_string())?;
        let frame = bend_frame(part, b, rho, &worlds).map_err(|e| e.to_string())?;
        let below = frame.radius + frame.thickness + self.clearance + self.tol.linear;
        let tool = frame.tool(below).ok_or("a tool's frame is not finite")?;
        let (origin, ex, ey) = (tool.origin(), tool.x(), tool.y());
        let lift = |p: [f64; 2]| Vec3::new(p[0], p[1], 0.0);
        // The cut of the parent's side, and of each child's.
        let cut_of = |flat: usize, at: [f64; 2], across: [f64; 2], sign: f64| {
            worlds
                .get(flat)
                .map(|w| (w.point(lift(at)), w.vector(lift(across)) * sign))
        };
        let parent_cut = cut_of(first.parent, first.start, first.across, 1.0)
            .ok_or("a bend's parent is missing")?;
        // Per flat, the stroke's bend whose child's side carries it.
        let side_of = |mut flat: usize| -> Option<usize> {
            for _ in 0..=part.flats.len() {
                let (i, x) = part
                    .bends
                    .iter()
                    .enumerate()
                    .find(|(_, x)| x.child == flat)?;
                if g.contains(&i) {
                    return Some(i);
                }
                flat = x.parent;
            }
            None
        };
        let mut values = Vec::new();
        for kind in kinds {
            let piece = self
                .pieces
                .iter()
                .find(|p| p.what == *kind)
                .ok_or("a piece is missing")?;
            let moved = self
                .built
                .get(piece.carrier)
                .and_then(Transform::inverse)
                .zip(worlds.get(piece.carrier))
                .map(|(back, stage)| back.then(stage))
                .ok_or("a flat's place is missing")?;
            let (point, normal) = match side_of(piece.carrier) {
                Some(i) => {
                    let x = part.bends.get(i).ok_or("a bend is missing")?;
                    // Past the child's tangent line is back towards the
                    // parent.
                    cut_of(x.child, x.far_line().0, x.across, -1.0)
                        .ok_or("a flange's flat is missing")?
                }
                None => parent_cut,
            };
            for half in [-1.0, 1.0] {
                let side = |p: Vec3| half * (moved.point(p) - point).dot(normal);
                for tri in die {
                    // Each half against each piece of the die is a lowest
                    // point per direction: counted, so `maxWork` bounds
                    // this proof's time too (review of 4116ed3e: it was
                    // counted once per call, whatever the die's pieces).
                    self.count()?;
                    let mut directions = vec![[0.0, 1.0]];
                    for k in 0..3 {
                        let (Some(a), Some(c)) = (tri.get(k), tri.get((k + 1) % 3)) else {
                            return Err("a piece of the die has no corners".into());
                        };
                        let (dx, dy) = (c[0] - a[0], c[1] - a[1]);
                        let len = dx.phypot(dy);
                        // Outward of a counter-clockwise side: its right.
                        directions.push(if len > 0.0 {
                            [dy / len, -dx / len]
                        } else {
                            [0.0, 1.0]
                        });
                    }
                    for n in directions {
                        let most = tri
                            .iter()
                            .map(|q| n[0] * q[0] + n[1] * q[1])
                            .fold(f64::NEG_INFINITY, f64::max);
                        let along = ex * n[0] + ey * n[1];
                        let least = piece_lowest_within(
                            part,
                            kind.piece(),
                            |p| (moved.point(p) - origin).dot(along),
                            side,
                        )
                        .ok_or_else(|| format!("{} has no lowest point", kind.text()))?;
                        values.push(least - most);
                    }
                }
            }
        }
        Ok(values)
    }

    /// The proof of pieces cut at their bend line (values from
    /// [`Check::apart_values`] at both ends of `[a, c]`): every half kept
    /// apart from every piece of the die along one of its directions by
    /// at least the clearance over the whole interval. Along a fixed
    /// direction of the tools' frame each point of a half moves with its
    /// second derivative at most `bow` (as for the rest proof), so the
    /// least over the half is at least the lower of its values at the
    /// ends less `bow h² / 8`; where that is not enough the interval is
    /// halved while the values at its ends are. The least distance proven,
    /// or `None`.
    #[allow(clippy::too_many_arguments)]
    fn apart(
        &self,
        done: &BTreeSet<usize>,
        g: &[usize],
        kinds: &[Kind],
        (a, va): (f64, &[f64]),
        (c, vc): (f64, &[f64]),
        stroke: &Stroke,
        depth: u32,
    ) -> Result<Option<f64>, String> {
        let h = c - a;
        let sag = stroke.bow * h * h / 8.0;
        if va.len() != vc.len() || va.is_empty() {
            return Ok(None);
        }
        let mut least = f64::INFINITY;
        let mut halve = false;
        for (ga, gc) in va.chunks(DIRECTIONS).zip(vc.chunks(DIRECTIONS)) {
            let best = ga
                .iter()
                .zip(gc)
                .map(|(x, y)| x.min(*y))
                .fold(f64::NEG_INFINITY, f64::max);
            if best.is_nan() || best < self.clearance {
                return Ok(None);
            }
            if best - sag >= self.clearance {
                least = least.min(best - sag);
            } else {
                halve = true;
            }
        }
        if !halve {
            return Ok(Some(least));
        }
        if depth >= MAX_REST_DEPTH {
            return Ok(None);
        }
        let m = 0.5 * (a + c);
        let vm = self.apart_values(done, g, m, kinds, stroke)?;
        let Some(left) = self.apart(done, g, kinds, (a, va), (m, &vm), stroke, depth + 1)? else {
            return Ok(None);
        };
        Ok(self
            .apart(done, g, kinds, (m, &vm), (c, vc), stroke, depth + 1)?
            .map(|right| left.min(right).min(least)))
    }

    /// Whether the die's convex pieces are there for [`Check::apart`].
    fn can_part(&self, stroke: &Stroke) -> bool {
        self.die_pieces
            .get(usize::from(stroke.turned))
            .is_some_and(Option::is_some)
    }
}

impl Flatten {
    /// The least distances at a flatten angle measured.
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
    /// it met no piece).
    fn json(&self, tool: usize) -> serde_json::Map<String, Value> {
        Cleared {
            least: self.least,
            at: self.at,
            proven: self.proven,
            turned: self.turned,
            flat: None,
        }
        .json(tool)
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
    /// The formed louvres' hoods the part has: not pieces of the stages,
    /// so not checked (said in `unchecked`).
    hoods: usize,
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
    punch: Tool,
    die: Tool,
    length: Length,
    /// Segmented tooling asked for (`None`: as `length` implies, one
    /// piece unless it lists a length per flange).
    segmented: Option<bool>,
    clearance: Option<f64>,
    /// The order to check, as given (flange indices per stroke).
    order: Option<Vec<Vec<usize>>>,
    /// Flanges made before the press brake by a tool of their own (a
    /// louvre on a punch press, a jog by an offset die): made at the
    /// start, never a step of the order, not checked themselves.
    formed: BTreeSet<usize>,
    max_work: usize,
    /// The hems pressed flat, if asked for.
    hem: Option<HemSpec>,
}

/// `hem` as given: the flanges, the pre-bend (radians) and the pad.
#[derive(Clone, Debug)]
struct HemSpec {
    flanges: BTreeSet<usize>,
    pre: f64,
    pad: Tool,
}

/// `hem` read from its object: `flanges` (indices), `preBend` (degrees,
/// more than 0 and less than 180) and `pad` (an outline as the tools',
/// its coordinates read by `number`).
fn hem_of(
    value: Option<&Value>,
    number: &mut dyn FnMut(&Value) -> Result<f64, String>,
) -> Result<Option<HemSpec>, String> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let bad = || {
        "`hem` is {\"flanges\": [flange, ...], \"preBend\": degrees, \"pad\": an outline as the punch's}".to_string()
    };
    let o = value.as_object().ok_or_else(bad)?;
    if let Some(k) = o
        .keys()
        .find(|k| !matches!(k.as_str(), "flanges" | "preBend" | "pad"))
    {
        return Err(format!("`hem` has no `{k}`: {}", bad()));
    }
    let flanges = o
        .get("flanges")
        .and_then(Value::as_array)
        .filter(|l| !l.is_empty())
        .ok_or_else(|| format!("`hem.flanges` must list the hems' flanges: {}", bad()))?
        .iter()
        .map(|v| {
            v.as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .ok_or_else(|| "`hem.flanges` names flanges by their index".to_string())
        })
        .collect::<Result<BTreeSet<usize>, String>>()?;
    let pre = o
        .get("preBend")
        .and_then(Value::as_f64)
        .filter(|d| d.is_finite() && *d > 0.0 && *d < 180.0)
        .ok_or_else(|| {
            "`hem.preBend` is the angle in degrees the air bend takes a hem to before it is pressed flat, more than 0 and less than 180".to_string()
        })?;
    let pad = tool_of(
        o.get("pad")
            .ok_or_else(|| format!("`hem.pad` is required: {}", bad()))?,
        "hem.pad",
        number,
    )?;
    Ok(Some(HemSpec {
        flanges,
        pre: pre.to_radians(),
        pad,
    }))
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

/// A polygon mirrored in `x` (`Tool::mirrored`).
#[cfg(test)]
fn mirrored(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    Tool::polygon(points.to_vec()).mirrored().corners
}

/// Whether a polygon is its own mirror image in `x` (`Tool::symmetric`).
#[cfg(test)]
fn symmetric(points: &[[f64; 2]]) -> bool {
    Tool::polygon(points.to_vec()).symmetric()
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
    formed: &BTreeSet<usize>,
) -> Result<Vec<Vec<usize>>, String> {
    let n = part.bends.len();
    let mut seen = formed.clone();
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
            if formed.contains(&b) {
                return Err(format!(
                    "`order` lists flange {b}, which is `formed` (made before the press brake)"
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
        let groups = without(strokes(part, tol), formed);
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
    let clearance = tools.clearance.unwrap_or(tol.linear);
    let n = part.bends.len();
    let tooling = tooling_of(&tools.length, tools.segmented, n).map_err(Stopped::Error)?;
    if let Some(&b) = tools.formed.iter().find(|&&b| b >= part.flanges) {
        return Err(Stopped::Error(format!(
            "`formed` names flanges by their index, 0 to {}; got {b}",
            part.flanges.saturating_sub(1)
        )));
    }
    // A jog's two bends (after the flanges') are made in one hit by an
    // offset die before the press brake: formed (docs/design/sheetmetal.md
    // › *Jogs across a flat*).
    let mut tools = tools;
    tools.formed.extend(part.flanges..n);
    let given = match &tools.order {
        Some(order) => Some(
            checked_order(part, &tol, order, tooling.segmented(), &tools.formed)
                .map_err(Stopped::Error)?,
        ),
        None => None,
    };
    let hem = match &tools.hem {
        None => None,
        Some(spec) => Some(
            hem_checked(part, &tol, spec, &tooling, &tools.formed, given.as_deref())
                .map_err(Stopped::Error)?,
        ),
    };
    // The tools must be simple outlines: checked once here, so a bad one
    // is an error before any search.
    let unit = craft3d_math::Frame::WORLD;
    for (name, tool) in [("punch", &tools.punch), ("die", &tools.die)] {
        // As tall as the outline is wide, so the check holds at any scale.
        let size = tool
            .corners
            .iter()
            .chain(tool.through.iter().flatten())
            .flat_map(|p| p.iter())
            .fold(0.0_f64, |m, v| m.max(v.abs()))
            .max(tol.linear * 1e3);
        tool.prism(&unit, 0.0, size, &tol)
            .map_err(|e| Stopped::Error(format!("`{name}`: {e}")))?;
    }
    let slabs = slabs_of(part, &tol).map_err(Stopped::Error)?;
    let pieces = pieces_of(part, &slabs, &tol).map_err(Stopped::Error)?;
    let turned_tools = (!(tools.punch.symmetric() && tools.die.symmetric()))
        .then(|| (tools.punch.mirrored(), tools.die.mirrored()));
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
    let die_cut = [
        die_pieces(&tools.die),
        turned_tools.as_ref().and_then(|(_, die)| die_pieces(die)),
    ];
    let check = Check {
        part,
        tol,
        punch: tools.punch,
        die: tools.die,
        turned_tools,
        die_pieces: die_cut,
        hem,
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
    // The flanges formed before the press brake are made from the start.
    let mut done = tools.formed.clone();
    let mut order: Vec<Step> = Vec::new();
    let mut tried: Tried = BTreeMap::new();
    let feasible = match &given {
        // The caller's order, checked step by step (every step, so each
        // one that does not clear is listed).
        Some(strokes) => {
            let mut every = true;
            for (i, g) in strokes.iter().enumerate() {
                let r = check
                    .step(&done, g)
                    .map_err(|e| with_estimate(e, &check, i, strokes.len() as f64))
                    .map_err(stopped)?;
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
            let mut ranked = without(
                ranked_strokes(part, &tol, check.tooling.segmented()),
                &tools.formed,
            );
            // A hem is a stroke of its own (its pre-bend and flatten).
            if let Some(hem) = &check.hem {
                ranked.retain(|g| g.len() == 1 || !g.iter().any(|b| hem.flanges.contains(b)));
            }
            let mut dead = BTreeSet::new();
            // The steps a search can take at most: each stroke after each
            // set of the other bends made (the formed ones always are).
            let free = part.bends.len().saturating_sub(tools.formed.len());
            let bound: f64 = ranked
                .iter()
                .map(|g| two_to(free.saturating_sub(g.len())))
                .sum();
            let mut step = |done: &BTreeSet<usize>, g: &[usize]| check.step(done, g);
            let found = search(
                &mut step, &mut done, &mut order, &mut dead, &mut tried, &ranked,
            );
            found
                .map_err(|e| with_estimate(e, &check, tried.len(), bound))
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
        hoods: built.formed.len(),
    })
}

/// `2^n`, saturating far beyond any work a caller may allow.
fn two_to(n: usize) -> f64 {
    let mut v = 1.0_f64;
    for _ in 0..n.min(200) {
        v *= 2.0;
    }
    v
}

/// The error of a check that ran out of its work, with what the whole
/// search would likely take: the work per step so far times the steps it
/// can take at most (`bound`; an upper estimate, since a search stops at
/// the first order that clears and skips what is known dead). Any other
/// error as it is.
fn with_estimate(e: String, check: &Check<'_>, steps_done: usize, bound: f64) -> String {
    if !check.ran_out.get() || steps_done == 0 || !(bound.is_finite() && bound >= 1.0) {
        return e;
    }
    let per = check.work.get() as f64 / steps_done as f64;
    let whole = per * bound;
    // Two significant digits, rounded up.
    let mut unit = 1.0_f64;
    while whole / unit >= 100.0 && unit < 1e300 {
        unit *= 10.0;
    }
    let rounded = (whole / unit).ceil() * unit;
    format!(
        "{e} ({steps_done} of at most {bound} steps checked, about {} distances and heights each: the whole search may need up to about {rounded}; a search that finds an order stops sooner)",
        per.round()
    )
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

/// The hems asked for, checked against the part: each flange there, bent
/// 180 degrees (within the angular tolerance), not `formed`, its
/// pre-bend less than its angle; a stroke of its own (with one-piece
/// tools no other bend on its line, which the punch would bend with it;
/// in an order given, alone); the pad one simple outline.
fn hem_checked(
    part: &sheetmetal::Part,
    tol: &Tolerance,
    spec: &HemSpec,
    tooling: &Tooling,
    formed: &BTreeSet<usize>,
    given: Option<&[Vec<usize>]>,
) -> Result<Hem, String> {
    let n = part.bends.len();
    for &b in &spec.flanges {
        let x = part.bends.get(b).ok_or_else(|| {
            format!(
                "`hem.flanges` names flanges by their index, 0 to {}; got {b}",
                n.saturating_sub(1)
            )
        })?;
        if formed.contains(&b) {
            return Err(format!("flange {b} is both a hem and `formed`"));
        }
        let off = (x.theta() - std::f64::consts::PI).abs();
        if off.is_nan() || off > tol.angular {
            return Err(format!(
                "flange {b} is bent {:.3} degrees: a hem pressed flat is bent 180",
                degrees(x.theta())
            ));
        }
        if spec.pre.is_nan() || spec.pre >= x.theta() {
            return Err("`hem.preBend` must be less than the hem's 180 degrees".into());
        }
        if !tooling.segmented()
            && let Some(group) = strokes(part, tol)
                .into_iter()
                .find(|g| g.contains(&b) && g.len() > 1)
        {
            return Err(format!(
                "flange {b} is a hem on one line with flanges {}: one-piece tools would bend them in one stroke with it; set `segmented` true to press it alone",
                stroke_words(&group)
            ));
        }
        if let Some(g) = given
            .into_iter()
            .flatten()
            .find(|g| g.contains(&b) && g.len() > 1)
        {
            return Err(format!(
                "`order`: flange {b} is a hem, a stroke of its own (its pre-bend, then pressed flat), not one with flanges {}",
                stroke_words(g)
            ));
        }
    }
    let size = spec
        .pad
        .corners
        .iter()
        .chain(spec.pad.through.iter().flatten())
        .flat_map(|p| p.iter())
        .fold(0.0_f64, |m, v| m.max(v.abs()))
        .max(tol.linear * 1e3);
    spec.pad
        .prism(&craft3d_math::Frame::WORLD, 0.0, size, tol)
        .map_err(|e| format!("`hem.pad`: {e}"))?;
    Ok(Hem {
        flanges: spec.flanges.clone(),
        pre: spec.pre,
        turned_pad: (!spec.pad.symmetric()).then(|| spec.pad.mirrored()),
        pad: spec.pad.clone(),
    })
}

/// The strokes `groups` without the flanges `formed` (made before the
/// press brake), a stroke left empty dropped.
fn without(groups: Vec<Vec<usize>>, formed: &BTreeSet<usize>) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::with_capacity(groups.len());
    for g in groups {
        let kept: Vec<usize> = g.into_iter().filter(|b| !formed.contains(b)).collect();
        if !kept.is_empty() && !out.contains(&kept) {
            out.push(kept);
        }
    }
    out
}

/// `formed` as given: a list of flange indices (checked against the part
/// in [`prove`]).
fn formed_param(v: Option<&Value>) -> Result<BTreeSet<usize>, String> {
    let bad = || {
        "`formed` is a list of flanges (indices) made before the press brake by a tool of their own"
            .to_string()
    };
    match v {
        None | Some(Value::Null) => Ok(BTreeSet::new()),
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .ok_or_else(bad)
            })
            .collect(),
        Some(_) => Err(bad()),
    }
}

/// `order` of the measure: strokes, each a flange index or a list of
/// them, as the command takes it.
fn order_value(v: Option<&Value>) -> Result<Option<Vec<Vec<usize>>>, String> {
    let bad = || {
        "`order` is a list of flanges (indices), each a number or a list of flanges bent in one stroke"
            .to_string()
    };
    let index = |v: &Value| {
        v.as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .ok_or_else(bad)
    };
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(list)) => list
            .iter()
            .map(|item| match item {
                Value::Array(many) => many.iter().map(index).collect(),
                one => index(one).map(|i| vec![i]),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(bad()),
    }
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
            "formed",
            "maxWork",
            "hem",
        ],
    )?;
    let punch = polygon(&p, "punch")?;
    let die = polygon(&p, "die")?;
    let hem = hem_of(p.get("hem"), &mut |v: &Value| {
        v.as_f64().filter(|v| v.is_finite()).ok_or_else(String::new)
    })
    .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let length = length_param(&p)?;
    let segmented =
        segmented_param(p.get("segmented")).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let order = order_param(&p)?;
    let formed =
        formed_param(p.get("formed")).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let max_work = match p.get("maxWork") {
        None | Some(Value::Null) => COMMAND_WORK,
        Some(v) => match v.as_u64() {
            Some(n) if (1..=MAX_WORK_ASKED).contains(&n) => {
                usize::try_from(n).unwrap_or(COMMAND_WORK)
            }
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
    let hems: Vec<usize> = hem
        .as_ref()
        .map(|h| h.flanges.iter().copied().collect())
        .unwrap_or_default();
    let tools = Tools {
        punch,
        die,
        length,
        segmented,
        clearance,
        order,
        formed: formed.clone(),
        max_work,
        hem,
    };
    let proof = prove(doc, &regen, feature, tools).map_err(|e| match e {
        Stopped::OutOfWork(e) | Stopped::Error(e) => CommandError::new(format!("{ID}: {e}")),
    })?;
    let feasible = proof.feasible;
    let steps: Vec<Value> = proof
        .order
        .iter()
        .flat_map(|(g, before, c)| {
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
            let Some(flat) = c.flat else {
                return vec![Value::Object(v)];
            };
            // A hem: its pre-bend, then the flatten stroke.
            v.insert("stage".into(), json!("pre-bend"));
            let mut f = flat.json(PAD);
            f.insert("stage".into(), json!("flatten"));
            f.insert("flange".into(), json!(g.first()));
            f.insert("flanges".into(), json!(g));
            f.insert("done".into(), json!(before.iter().collect::<Vec<_>>()));
            f.insert("turned".into(), json!(flat.turned));
            f.insert("pad".into(), Value::Object(flat.json(PAD)));
            f.insert("die".into(), Value::Object(flat.json(FLAT_DIE)));
            vec![Value::Object(v), Value::Object(f)]
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
                    ..
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
    let unchecked_text = {
        let mut text = unchecked(proof.segments.is_some());
        if !formed.is_empty() {
            text.push_str(&format!(
                "; the flanges formed ({}) are made before the press brake by tools of their own and not checked",
                formed.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
            ));
        }
        if !hems.is_empty() {
            text.push_str(&format!(
                "; the hems pressed flat ({}): their flat and strip against the pad that presses them",
                hems.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
            ));
        }
        if proof.hoods > 0 {
            text.push_str(&format!(
                "; {}",
                HOODS_UNCHECKED.replace("{n}", &proof.hoods.to_string())
            ));
        }
        text
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
        "formed": formed.iter().collect::<Vec<_>>(),
        "unchecked": unchecked_text,
        "work": proof.work,
        "lengthUnit": length_unit(doc),
    }))
}

// ------------------------------------------------------------- the measure

/// The parameters of the assertion measure `bendable`.
pub(crate) const MEASURE_KEYS: [&str; 10] = [
    "of",
    "hem",
    "formed",
    "order",
    "punch",
    "die",
    "length",
    "segmented",
    "clearance",
    "maxWork",
];

/// Its keys whose values are expressions (lists of points of them for a
/// tool).
pub(crate) const EXPRS: [&str; 5] = ["punch", "die", "length", "clearance", "hem"];

/// A tool's polygon for the measure: points `[x, y]`, each coordinate a
/// number in the document's unit or a length expression (of the
/// variables, a table's row, the outputs).
fn tool_points(ctx: &Ctx<'_>, p: &Map<String, Value>, key: &str) -> Result<Tool, String> {
    let value = p
        .get(key)
        .filter(|v| !v.is_null())
        .ok_or_else(|| format!("`{key}` is required: a polygon [[x, y], ...]"))?;
    tool_of(value, key, &mut |v: &Value| ctx.quantity(v, key, 1))
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
/// running out of its work (`maxWork`, shared by the parts: at most 20000
/// distances for a search, at most [`MEASURE_ORDER_WORK`] for an `order`
/// given to check, of one part; about 2400 distances a second on the
/// chassis tray, so the cap is some minutes). The tools are one piece reaching past the part
/// unless `length` says how long or `segmented` asks for segmented
/// tooling (a segment per bend), as in the command.
pub(crate) fn take(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    crate::verify_cmds::only(p, &MEASURE_KEYS)?;
    let punch = tool_points(ctx, p, "punch")?;
    let die = tool_points(ctx, p, "die")?;
    let length = positive(ctx, p, "length")?;
    let clearance = positive(ctx, p, "clearance")?;
    let segmented = segmented_param(p.get("segmented"))?;
    let formed = formed_param(p.get("formed"))?;
    let order = order_value(p.get("order"))?;
    let hem = hem_of(p.get("hem"), &mut |v: &Value| ctx.quantity(v, "hem.pad", 1))?;
    // A search over orders is held to 20000 distances; an order given is
    // checked step by step, a bounded work the caller may allow more for.
    // A search over orders is held to 20000 distances; an order given is
    // checked step by step, a bounded work the caller may allow more for,
    // up to 1e6: the measure runs on the assertions' one job thread (and
    // the web build's one thread), so it stays below the command's 1e7
    // (coordinator, 2026-10-31).
    let most = if order.is_some() {
        MEASURE_ORDER_WORK
    } else {
        MAX_WORK
    };
    let max_work = match p.get("maxWork").filter(|v| !v.is_null()) {
        None => MAX_WORK,
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=most).contains(n))
            .ok_or_else(|| {
                format!(
                    "`maxWork` must be a whole number from 1 to {most}{}",
                    if order.is_some() {
                        String::new()
                    } else {
                        format!(" ({MEASURE_ORDER_WORK} with an `order` to check)")
                    }
                )
            })?,
    };
    let parts = crate::sheetmetal_cmds::parts_of(ctx, p)?;
    if (!formed.is_empty() || order.is_some()) && parts.len() != 1 {
        return Err(
            "`formed` and `order` name flanges of one part: give `of` one sheet-metal part"
                .to_string(),
        );
    }
    let mut left = max_work;
    let mut listed = Vec::new();
    let mut hoods = 0usize;
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
            order: order.clone(),
            formed: formed.clone(),
            hem: hem.clone(),
            max_work: left,
        };
        match prove(ctx.doc, ctx.regen, id, tools) {
            Ok(proof) => {
                left = left.saturating_sub(proof.work);
                one.insert("work".into(), proof.work.into());
                hoods = hoods.saturating_add(proof.hoods);
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
    let mut text = unchecked(segmented == Some(true));
    if hoods > 0 {
        text.push_str(&format!(
            "; {}",
            HOODS_UNCHECKED.replace("{n}", &hoods.to_string())
        ));
    }
    out.detail.insert("unchecked".into(), text.into());
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
                punch: Tool::polygon(vec![[-k, 0.0], [k, 0.0], [k, 100.0 * k], [-k, 100.0 * k]]),
                die: Tool::polygon(vec![[-k, -k], [k, -k], [k, 0.0], [-k, 0.0]]),
                turned_tools: None,
                die_pieces: [None, None],
                hem: None,
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
            punch: Tool::polygon(vec![[-k, 0.0], [k, 0.0], [k, 100.0 * k], [-k, 100.0 * k]]),
            die: Tool::polygon(vec![[-k, -k], [k, -k], [k, 0.0], [-k, 0.0]]),
            turned_tools: None,
            die_pieces: [None, None],
            hem: None,
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
            real: true,
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
            hoods: 0,
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
            flat: None,
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
                    flat: None,
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
