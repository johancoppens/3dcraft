//! The local operation the face operations share: faces of a solid are
//! given new surfaces, and the body is closed again around them.
//!
//! **The recipe** (docs/design/face-operations.md › *Face operations as
//! built*). Nothing about how the body hangs together changes: every
//! face, edge and vertex of the result is the face, edge and vertex of
//! the body with the same number. What changes is where they lie:
//!
//! 1. every vertex on a face that got a new surface is found again as the
//!    point where the surfaces of its faces meet, nearest to where it was
//!    (Newton's iteration on the distances to the surfaces; one step for
//!    planes);
//! 2. every edge that ends in such a vertex is made again on the surfaces
//!    of its two faces: a straight edge through its two vertices, a
//!    circular edge about the axis its faces share, and any other edge
//!    (an ellipse, a spline, a line or circle on a freeform face that is
//!    one no longer) as a cubic through points found where its faces
//!    meet, held to a share of the tolerance ([`fitted`]); an edge along
//!    a border of the parameters of a new freeform surface that keeps the
//!    old one's parameters (an offset) as that border of it ([`Iso`]).
//!    That is "extend the neighbours and intersect again" for the curves
//!    two such surfaces meet in: a neighbour is not cut or lengthened as
//!    a separate step, its boundary simply moves;
//! 3. the result is held against what a body is.
//!
//! **One exception, for the shell's hollow** ([`Corners::Split`]; the
//! kernel set's H-01, H-06, H-07, 2026-11-14): a vertex of four or more
//! faces that the new surfaces no longer meet in one point becomes the
//! tree of vertices of three faces its moved faces make ([`split`]), the
//! new vertices and edges after the old ones, the old vertex one of them.
//! Every other operation refuses such a vertex, as before.
//!
//! **Never a wrong solid.** A vertex whose faces no longer meet in a
//! point, an edge that would no longer lie on both its faces, an edge
//! that shrinks to nothing or turns round (which is how a neighbour that
//! vanishes shows), a flat face whose outline crosses itself or whose
//! hole leaves it, and a round face whose rims change places are each an
//! error that names the entity; so are two edges of any face that come
//! to meet where they did not, a vertex a moved face went through and an
//! edge made again that goes through a face not its own ([`passes`]:
//! faces moved up to or past what ends their neighbours, 2026-11-14).
//! What these cannot see, a moved face running into a smooth face far
//! from it with no vertex and no edge in the way, is left to the caller's
//! check on the whole body (regeneration meshes the result and looks for
//! places where it passes through itself).

use crate::KernelError;
use crate::profile::{Curve2, SketchCurve, detect_profiles};
use crate::shared::{curve_through, index_u32};

mod passes;
mod split;
use craft3d_geom::{Circle, Curve3, Line, NurbsCurve, Plane, Surface};
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, VertexId, edge_faces};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};

/// Why a body could not be closed around the new surfaces.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TweakError {
    Kernel(KernelError),
    /// The geometry is not built: the faces and the edge it is about.
    Unsupported {
        faces: Vec<FaceId>,
        edge: Option<EdgeId>,
        reason: &'static str,
    },
    /// The faces moved so far that this edge or face is no longer there,
    /// or that these faces no longer meet where they did.
    Vanishes {
        faces: Vec<FaceId>,
        edge: Option<EdgeId>,
        reason: &'static str,
    },
}

impl From<KernelError> for TweakError {
    fn from(error: KernelError) -> Self {
        TweakError::Kernel(error)
    }
}

pub(crate) const EDGE_GONE: &str =
    "the edge shrinks to nothing or turns round: a face next to it would vanish";
pub(crate) const EDGE_GONE_UNSAID: &str = "the edge shrinks to nothing or turns round: a face next to it would vanish (how far the faces can go before it does was not found: smaller distances fail while the edge is still there; try a smaller distance)";
pub(crate) const EDGE_GONE_AT_LEAST: &str = "the edge shrinks to nothing or turns round: a face next to it would vanish (how far the faces can go before it does was not found: what is said to fit here does, and more may)";
pub(crate) const NO_MEETING: &str =
    "the faces that meet in a vertex there no longer meet in one point";
pub(crate) const OFF_SURFACE: &str = "the edge would no longer be a line or a circle on both its faces: continuing these two surfaces until they meet again is not built";
pub(crate) const LOST: &str = "the edge could not be found again where its faces meet: they no longer meet along it as they did";
pub(crate) const FREEFORM: &str = "a freeform face next to a face that moves cannot be continued past its edge, and the faces no longer meet in the vertex there";
pub(crate) const OUTLINE: &str =
    "the outline of the face would cross itself, or a hole in it would leave it";
pub(crate) const RIMS: &str = "the two rims of the face would change places: the face would vanish";
pub(crate) const THROUGH: &str =
    "the face would pass through another part of the body, or lie in another face of it";
pub(crate) const THROUGH_UNSAID: &str = "the face would pass through another part of the body, or lie in another face of it (how far it can go before it first does was not found: a face that is not flat towards it lies in its way; try a smaller distance)";
pub(crate) const TURNED: &str = "the face would be turned inside out: it would vanish";
/// The reasons above, said where the search for how far the faces can go
/// stopped at its work bound with room left (`offset_face::bounded_fitting`):
/// what is said to fit does, and is no limit.
pub(crate) const THROUGH_AT_LEAST: &str = "the face would pass through another part of the body, or lie in another face of it (how far it can go before it first does was not found: what is said to fit here does, and more may)";
pub(crate) const CROSSED_AT_LEAST: &str = "two edges of one face would cross or touch: a face that moves would reach or pass the face that ends its neighbour there (how far it can go before they do was not found: what is said to fit here does, and more may)";
pub(crate) const OUTLINE_AT_LEAST: &str = "the outline of the face would cross itself, or a hole in it would leave it (how far the faces can go before it does was not found: what is said to fit here does, and more may)";
pub(crate) const RIMS_AT_LEAST: &str = "the two rims of the face would change places: the face would vanish (how far the faces can go before it does was not found: what is said to fit here does, and more may)";
pub(crate) const TURNED_AT_LEAST: &str = "the face would be turned inside out: it would vanish (how far the faces can go before it does was not found: what is said to fit here does, and more may)";
pub(crate) const CROSSED: &str = "two edges of one face would cross or touch: a face that moves would reach or pass the face that ends its neighbour there";
pub(crate) const INSIDE_OUT: &str = "the body would be turned inside out: a face that moves would pass right through the rest of it (how far it can go before it first does was not found: try a smaller distance)";

/// The signed distance from `point` to `surface` and the direction it
/// grows in. `None` where the surface has no nearest point with a normal
/// (a cone's apex, the axis of a cylinder).
fn distance(surface: &Surface, point: Vec3) -> Option<(f64, Vec3)> {
    distance_near(surface, point, &mut None)
}

/// Normals and offsets nearer to square than this sine are square: the
/// foot a local walk found is a nearest point.
pub(crate) const SQUARE: f64 = 1e-9;

/// [`distance`], for a Bézier or NURBS surface found by a walk from the
/// parameters in `guess` when there are some (the foot of a point
/// nearby), and by the search over the whole surface when there are none
/// or the walk did not end square to the surface; `guess` is left at the
/// foot found.
fn distance_near(
    surface: &Surface,
    point: Vec3,
    guess: &mut Option<(f64, f64)>,
) -> Option<(f64, Vec3)> {
    let local = guess.and_then(|start| {
        let (u, v) = surface.project_from(point, start).ok()?;
        let foot = surface.eval(u, v);
        let normal = surface.normal(u, v)?;
        let offset = point - foot;
        let aside = (offset - normal * offset.dot(normal)).length();
        (aside <= SQUARE * offset.length() + 1e-15 * (1.0 + point.length())).then_some((u, v))
    });
    let (u, v) = match local {
        Some(found) => found,
        None => surface.project(point).ok()?,
    };
    if matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
        *guess = Some((u, v));
    }
    let foot = surface.eval(u, v);
    let normal = surface.normal(u, v)?;
    let offset = point - foot;
    // Along the normal when the foot is the nearest point; at the rim of
    // a bounded surface (the apex of a cone) it is not, and the whole
    // distance counts.
    let along = offset.dot(normal);
    let aside = (offset - normal * along).length();
    let value = if aside > along.abs() {
        offset.length()
    } else {
        along
    };
    (value.is_finite() && normal.is_finite()).then_some((value, normal))
}

/// [`distance_near`], with the direction the distance grows in where the
/// nearest point lies on the surface's border, not square below the point:
/// from that foot to the point (the unsigned distance's gradient), not the
/// normal there.
fn distance_toward(
    surface: &Surface,
    point: Vec3,
    guess: &mut Option<(f64, f64)>,
) -> Option<(f64, Vec3)> {
    let (value, normal) = distance_near(surface, point, guess)?;
    if !matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
        return Some((value, normal));
    }
    let (u, v) = (*guess)?;
    let offset = point - surface.eval(u, v);
    let along = offset.dot(normal);
    let aside = (offset - normal * along).length();
    if aside > SQUARE * offset.length() + 1e-15 * (1.0 + point.length())
        && aside > along.abs() * 1e-3
    {
        let length = offset.length();
        if length > 0.0 {
            return Some((length, offset * (1.0 / length)));
        }
    }
    Some((value, normal))
}

/// Normals nearer to each other than this sine are one direction: the
/// faces are tangent there, and one of them says nothing the other does
/// not.
const SAME_DIRECTION: f64 = 1e-6;

/// Normals nearer to each other than this sine are nearly one direction:
/// the faces are tangent by construction (a blend and the faces it rolls
/// along, the pieces of one surface), to within what fitting them left;
/// a crease any design means is far steeper.
pub(crate) const TANGENT: f64 = 1e-4;

/// A nearly tangent surface the point lies on within the tolerance (this
/// share of it, all of it: what the check at the end of [`meet`] asks)
/// asks no further step. Two fitted offsets that share a border are each
/// within a quarter of the tolerance of it, so within half of it of each
/// other: a smaller share would take the other as a second direction, and
/// its step, the residual over the sine, sends the point away.
const HELD: f64 = 1.0;

/// The sine below which a surface meeting those before it is taken last in
/// [`meet`]: only once the point lies on them (two fillets of a ring
/// trimmed against each other meet at a fraction of a degree).
const SHALLOW: f64 = 0.05;

/// Most steps of the iteration for one vertex.
const MEET_STEPS: usize = 64;

/// The point nearest `start` where `surfaces` meet, within `tol` of each.
pub(crate) fn meet(surfaces: &[&Surface], start: Vec3, tol: f64) -> Option<Vec3> {
    meet_near(surfaces, start, tol, &mut vec![None; surfaces.len()])
}

/// [`meet`] for a point of an edge rather than a vertex: not put on a seam
/// ([`on_seam`]). Along an edge two of the surfaces may continue each other
/// (the rims on either side of a plate's corner, one an exact cylinder and
/// the other a spline fitted to it): tangent everywhere, they part nowhere,
/// and the rate [`turn_onto_seam`] follows is the fit's noise (2026-11-01:
/// the edge across a sheet's corner on a bend's tangent line, its points
/// moved 40 tolerances along it and no spline through them held).
pub(crate) fn meet_along(surfaces: &[&Surface], start: Vec3, tol: f64) -> Option<Vec3> {
    meet_with(
        surfaces,
        start,
        tol,
        &mut vec![None; surfaces.len()],
        false,
        false,
    )
}

/// [`meet`] next to a rebuilt fillet, from a point foreseen close to where
/// it goes (a vertex started on the new fillet), `sideways`:
/// - where the nearest point of a bounded freeform surface is on its
///   border, not square below the point, the point is drawn toward that
///   border foot (the direction the distance grows in there), not along
///   the normal: two fillets tangent to one face and crossing each other at
///   a fraction of a degree meet that face in a vertex that lies off one
///   fillet's border sideways by the fits' errors over the angle, and
///   along the normal nothing moves it there;
/// - a surface that meets those before it at a shallow angle ([`SHALLOW`])
///   waits until the point lies on them: its step runs along them by its
///   residual over the sine, which from a point away from curved faces
///   throws the point far off.
fn meet_foreseen(surfaces: &[&Surface], start: Vec3, tol: f64) -> Option<Vec3> {
    meet_with(
        surfaces,
        start,
        tol,
        &mut vec![None; surfaces.len()],
        true,
        true,
    )
}

/// [`meet`], with the feet on the freeform surfaces found from `guesses`
/// (one per surface; see [`distance_near`]) and left there.
fn meet_near(
    surfaces: &[&Surface],
    start: Vec3,
    tol: f64,
    guesses: &mut [Option<(f64, f64)>],
) -> Option<Vec3> {
    meet_with(surfaces, start, tol, guesses, false, true)
}

/// [`meet_near`]; with `sideways`, as [`meet_foreseen`] says.
///
/// Where that does not end on every surface, it is tried again for every
/// pair of surfaces tangent where it starts ([`TANGENT`]), with one of the
/// pair left out of the steps: at a vertex where two faces tangent along
/// their edge meet a third (a seam held G1 or G2 ending on a flat face),
/// the pair's normals agree there, so they fix one direction between them
/// and the point is left to slide along the seam; the one whose foot is
/// past its border (the seam is the border of both) asks for steps along
/// the wrong direction, and the iteration runs off (2026-11-01: the fuel
/// tank's crown and side meeting its floor, offset 1.2 mm inward, sent
/// 900 mm away). The one left out is held at the end, as every surface is:
/// the point must lie on it within `tol`, which near the seam the pair's
/// agreement gives.
///
/// With `seam`, a point found where two of the surfaces are tangent is
/// put on their seam ([`on_seam`]): for vertices. The points of an edge
/// found again ([`fitted`]) are held to a plane across it and do not ask
/// for it (most of the calls; the search for pairs alone costs a
/// nearest point on every surface).
fn meet_with(
    surfaces: &[&Surface],
    start: Vec3,
    tol: f64,
    guesses: &mut [Option<(f64, f64)>],
    sideways: bool,
    seam: bool,
) -> Option<Vec3> {
    let settle = |found: Vec3, guesses: &mut [Option<(f64, f64)>]| {
        if seam {
            on_seam(surfaces, found, tol, guesses, sideways)
        } else {
            found
        }
    };
    let kept: Vec<Option<(f64, f64)>> = guesses.to_vec();
    if let Some(found) = meet_steps(surfaces, start, tol, guesses, sideways, None) {
        return Some(settle(found, guesses));
    }
    let pairs = tangent_pairs(surfaces, start, &kept, TANGENT);
    // Bounded: a pair per two surfaces at the vertex (a handful), each try
    // at most `MEET_STEPS` steps and `SLIDES` slides of eight halvings of
    // those; each try is counted against the operation's work bound too, so
    // a search of many vertices ends where its bound says.
    for (i, j) in pairs {
        for silent in [j, i] {
            if craft3d_geom::allowance::pass() {
                guesses.copy_from_slice(&kept);
                return None;
            }
            guesses.copy_from_slice(&kept);
            if let Some(found) = meet_steps(surfaces, start, tol, guesses, sideways, Some(silent)) {
                return Some(settle(found, guesses));
            }
        }
    }
    guesses.copy_from_slice(&kept);
    None
}

/// The pairs of `surfaces` whose normals at `at` are within the sine
/// `within` of each other and point the same way, the feet found from
/// `guesses`.
fn tangent_pairs(
    surfaces: &[&Surface],
    at: Vec3,
    guesses: &[Option<(f64, f64)>],
    within: f64,
) -> Vec<(usize, usize)> {
    pairs_within(&normals_at(surfaces, at, guesses), within, true)
}

/// The normals of `surfaces` at their feet from `at`, found from
/// `guesses`; `None` where a surface has none there.
fn normals_at(
    surfaces: &[&Surface],
    at: Vec3,
    guesses: &[Option<(f64, f64)>],
) -> Vec<Option<Vec3>> {
    surfaces
        .iter()
        .zip(guesses.iter())
        .map(|(surface, guess)| {
            let mut near = *guess;
            distance_near(surface, at, &mut near).map(|(_, n)| n)
        })
        .collect()
}

/// The pairs of `normals` within the sine `within` of each other; with
/// `same_way`, only those that point the same way.
fn pairs_within(normals: &[Option<Vec3>], within: f64, same_way: bool) -> Vec<(usize, usize)> {
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (i, a) in normals.iter().enumerate() {
        for (j, b) in normals.iter().enumerate().skip(i + 1) {
            if let (Some(a), Some(b)) = (a, b)
                && a.cross(*b).length() <= within
                && (!same_way || a.dot(*b) > 0.0)
            {
                pairs.push((i, j));
            }
        }
    }
    pairs
}

/// A vertex `found` where two surfaces tangent along their seam meet a
/// third, moved onto the seam ([`turn_onto_seam`]). Within the tolerance
/// of both is not enough there: the pair's distance grows with the square
/// of the way along the seam, so a point within `tol` of both may lie
/// `√(2 r tol)` along it (`r` the radius the pair part at), far beyond the
/// tolerance (2026-11-01: the fuel tank's tunnel, a plane tangent to a
/// cylinder of 48.8 mm, its shell's vertices at the two ends of their seam
/// 3 µm below and 4 µm above it at a tolerance of 1e-6 mm, and the edge
/// between them dipping past the cylinder's own border, where points
/// beside it could not be told apart). The pair's normals there are apart
/// by up to `√(2 tol / r)` (2e-4 at the tank's nose), beyond [`TANGENT`],
/// so a pair is any two whose normals are within [`SEAM_PAIR`]: where two
/// surfaces cross instead, a point within the tolerance of both lies
/// within the tolerance over the angle of where they cross, and the
/// secant steps either stay there or leave the surfaces and are not
/// taken. Each of the pair is tried as the one the others' curve turns
/// onto; the first that moves the point is kept, if it moves it by more
/// than [`SEAM_KEEP`] of the tolerance.
///
/// The others must meet across each other, in a curve: where two of them
/// are tangent as well (either way round) their curve is not defined
/// better than the pair's own seam, and the rate along it is noise
/// (2026-11-01: a channel's plate, a plane tangent to a cylinder with the
/// two rims at the corner on one plane to rounding; those rims were
/// taken for a pair and the exact corner was moved 26 tolerances along
/// the plane and cylinder's seam). Such a pair is not tried that way
/// round; one surface given twice is no seam at all.
fn on_seam(
    surfaces: &[&Surface],
    found: Vec3,
    tol: f64,
    guesses: &mut [Option<(f64, f64)>],
    sideways: bool,
) -> Vec3 {
    let normals = normals_at(surfaces, found, guesses);
    let touching = pairs_within(&normals, SEAM_PAIR, false);
    for (i, j) in pairs_within(&normals, SEAM_PAIR, true) {
        for silent in [j, i] {
            if touching.iter().any(|&(k, l)| k != silent && l != silent) {
                continue;
            }
            let mut trial = guesses.to_vec();
            if let Some(moved) = turn_onto_seam(surfaces, found, tol, &mut trial, sideways, silent)
            {
                // A vertex the seam is already as good as reached keeps
                // its place, bit for bit.
                if moved.distance(found) <= SEAM_KEEP * tol {
                    return found;
                }
                guesses.copy_from_slice(&trial);
                return moved;
            }
        }
    }
    found
}

/// The sine within which two surfaces' normals at a vertex make them a
/// pair [`on_seam`] puts the vertex onto the seam of.
const SEAM_PAIR: f64 = 1e-2;

/// How far, in tolerances, [`on_seam`] must move a vertex for the move to
/// be taken: closer than this the vertex is on the seam for anything that
/// measures it, and stays as it was found.
const SEAM_KEEP: f64 = 1e-2;

/// How much, in tolerances, the distance to the surface a vertex is
/// turned onto may grow at a step of [`turn_onto_seam`] and the step still
/// count: rounding only (distances are found to a few units in the last
/// place of the coordinates, far below this at any scale the tolerance
/// is meant for). A step that leaves the seam grows it by the way squared
/// over the radius the pair part at (2.6e-9 mm 4.5 µm along the seam of a
/// wrapped glyph's rims), and a vertex moved so is off the edges found on
/// the seam beside it.
const SEAM_RISE: f64 = 1e-4;

/// How many times the farthest a point within the tolerance of both
/// surfaces can lie along their seam [`turn_onto_seam`] may move a vertex.
const SEAM_REACH: f64 = 4.0;

/// Most secant steps of [`turn_onto_seam`].
const SECANTS: usize = 8;

/// The point where the curve the surfaces but `silent` meet in touches
/// `silent`, from `start` on that curve: where its tangent lies in the
/// tangent plane of `silent`. The distance there is a minimum, flat to
/// first order, so a distance within rounding of zero leaves the point
/// `√(2 r ε)` along the seam; the rate at which the distance grows along
/// the curve (`n · t`, of the normals, accurate to rounding) is linear in
/// the way along, and its root is found by secant steps to rounding of
/// the way itself. Taken where it lies on every surface within `tol`, the
/// rate fell and the distance to `silent` did not grow (by more than
/// [`SEAM_RISE`] of `tol`); `None` otherwise, `guesses` then unchanged.
/// A vertex on the seam already stays there: 2026-11-01, the corner of a
/// wrapped glyph where two curved rims are tangent (an oval of four
/// quadratic spans on a cylinder) lay on their seam to 1e-15 mm, and the
/// secant steps took it 1e-3 mm along the plate's front skin, past the
/// end of one rim, where the distance to it (4.4e-7 mm) was still within
/// the tolerance and its rate smaller; the edge across the corner, its
/// points found on the seam, could then not be traced to it.
fn turn_onto_seam(
    surfaces: &[&Surface],
    start: Vec3,
    tol: f64,
    guesses: &mut [Option<(f64, f64)>],
    sideways: bool,
    silent: usize,
) -> Option<Vec3> {
    let surface = surfaces.get(silent)?;
    // The rate along the curve at the point the steps put `at` on it.
    let rate_at = |at: Vec3, trial: &mut [Option<(f64, f64)>]| -> Option<(f64, f64, Vec3, Vec3)> {
        let (on, basis) = meet_iterate(surfaces, at, tol, trial, sideways, Some(silent))?;
        let [first, second] = basis.as_slice() else {
            return None;
        };
        let along = first.cross(*second).normalized()?;
        let mut near = trial.get(silent).copied().flatten();
        let (value, normal) = distance_near(surface, on, &mut near)?;
        let rate = normal.dot(along);
        (rate.is_finite() && value.is_finite()).then_some((rate, value, along, on))
    };
    let mut trial = guesses.to_vec();
    let (r0, v0, a0, p0) = rate_at(start, &mut trial)?;
    let mut best = (r0.abs(), p0, trial.clone());
    let mut last = (r0, a0, p0);
    // The first step: the distance is about `k e²` and its rate `2 k e`, so
    // the seam is `−2 v / r` away; then secant steps on the rate.
    let mut step = if r0 != 0.0 { -2.0 * v0 / r0 } else { 0.0 };
    // How fast the rate grows along the curve, `2k`, from the first secant.
    let mut slope = f64::NAN;
    for _ in 0..SECANTS {
        if !(step.is_finite() && step != 0.0) {
            break;
        }
        // The operation's work bound reached: no move is taken half-way
        // (the vertex keeps its place) and the operation, which asks the
        // bound after its steps, refuses saying so (`allowance`).
        if craft3d_geom::allowance::pass() {
            return None;
        }
        let (r, a, p) = last;
        let Some((r1, v1, a1, p1)) = rate_at(p + a * step, &mut trial) else {
            break;
        };
        // The curve touches `silent` at the seam: the distance there is the
        // least along it, so a point further from `silent` than the start
        // is no seam, whatever its rate (past the border of a bounded
        // spline the distance is to the border and its rate falls too).
        let nearer = v1.abs() <= v0.abs() + SEAM_RISE * tol;
        if nearer && r1.abs() < best.0 {
            best = (r1.abs(), p1, trial.clone());
        }
        let moved = (p1 - p).dot(a);
        last = (r1, a1, p1);
        let change = r1 - r;
        // From the first secant, the longest: the later ones near the
        // root are steps of rounding.
        if slope.is_nan() && moved != 0.0 {
            slope = (change / moved).abs();
        }
        step = if change != 0.0 {
            -r1 * moved / change
        } else {
            0.0
        };
        if r1 == 0.0 {
            break;
        }
    }
    let (rate, point, found) = best;
    // A rate that is not a number never counts as fallen.
    if rate.partial_cmp(&r0.abs()) != Some(std::cmp::Ordering::Less) {
        return None;
    }
    // Within reach of the seam the start lies on: a point within `tol` of
    // both lies at most `√(tol / k)` along it (the distance `k e²`, `2k`
    // the rate's slope the first secant measured), so a move of more than
    // [`SEAM_REACH`] times that went to another place the curve comes
    // near `silent`, not to this seam; and with no slope measured there
    // is no seam to place.
    let reach = SEAM_REACH * (2.0 * tol / slope).sqrt();
    if !(reach.is_finite() && point.distance(start) <= reach) {
        return None;
    }
    let mut check = found.clone();
    let on_all = surfaces
        .iter()
        .zip(check.iter_mut())
        .all(|(s, g)| distance_near(s, point, g).is_some_and(|(value, _)| value.abs() <= tol));
    if !(on_all && point.is_finite()) {
        return None;
    }
    guesses.copy_from_slice(&found);
    Some(point)
}

/// Most slides along the curve where the other surfaces meet, toward the
/// one left out ([`meet_steps`]).
const SLIDES: usize = 32;

/// The iteration of [`meet_with`]. With `silent`, that surface adds no
/// direction to the steps, and once the point lies on the others it slides
/// along the curve where they meet (two of them, the pair's other one and
/// the third face) toward the one left out, by Newton's steps on its
/// distance along that curve, each halved until the distance falls: its
/// distance from the one it is tangent to is that of the seam, which the
/// slide reaches. It is held at the end, as every surface is.
fn meet_steps(
    surfaces: &[&Surface],
    start: Vec3,
    tol: f64,
    guesses: &mut [Option<(f64, f64)>],
    sideways: bool,
    silent: Option<usize>,
) -> Option<Vec3> {
    let (mut point, mut basis) = meet_iterate(surfaces, start, tol, guesses, sideways, silent)?;
    if let Some(left) = silent
        && let Some(surface) = surfaces.get(left)
    {
        let mut foot: Option<(f64, f64)> = None;
        for _ in 0..SLIDES {
            if craft3d_geom::allowance::pass() {
                break;
            }
            let (value, gradient) = distance_toward(surface, point, &mut foot)?;
            if value.abs() <= tol {
                break;
            }
            let [first, second] = basis.as_slice() else {
                break;
            };
            let Some(along) = first.cross(*second).normalized() else {
                break;
            };
            let rate = gradient.dot(along);
            if rate.abs() <= 1e-12 || rate.is_nan() {
                break;
            }
            let mut length = -value / rate;
            let mut better = None;
            for _ in 0..8 {
                let mut trial = guesses.to_vec();
                if let Some((moved, axes)) = meet_iterate(
                    surfaces,
                    point + along * length,
                    tol,
                    &mut trial,
                    sideways,
                    silent,
                ) {
                    let mut near = foot;
                    if let Some((after, _)) = distance_toward(surface, moved, &mut near)
                        && after.abs() < value.abs()
                    {
                        better = Some((moved, axes, trial, near));
                        break;
                    }
                }
                length *= 0.5;
            }
            let Some((moved, axes, trial, near)) = better else {
                break;
            };
            point = moved;
            basis = axes;
            guesses.copy_from_slice(&trial);
            foot = near;
        }
    }
    let on_all = surfaces
        .iter()
        .zip(guesses.iter_mut())
        .all(|(surface, guess)| {
            distance_near(surface, point, guess).is_some_and(|(value, _)| value.abs() <= tol)
        });
    (point.is_finite() && on_all).then_some(point)
}

/// The steps of [`meet_steps`] onto every surface but `silent`, and the
/// directions the last of them was taken along.
fn meet_iterate(
    surfaces: &[&Surface],
    start: Vec3,
    tol: f64,
    guesses: &mut [Option<(f64, f64)>],
    sideways: bool,
    silent: Option<usize>,
) -> Option<(Vec3, Vec<Vec3>)> {
    let mut point = start;
    let mut last: Vec<Vec3> = Vec::new();
    for _ in 0..MEET_STEPS {
        // An orthonormal basis of the normals, and for each surface that
        // adds a direction the step along it that brings the point onto
        // it after the steps along the directions before it (forward
        // substitution: the rows are triangular). A surface whose normal
        // is all but that of one before it adds nothing (tangent faces);
        // one that is only nearly so ([`TANGENT`]) and that the point lies
        // on already, after those steps, within [`HELD`] of the tolerance
        // adds nothing either: the step it would ask for runs along the
        // tangent plane by the residual over the sine, far off for
        // residuals of rounding.
        let mut basis: Vec<Vec3> = Vec::with_capacity(3);
        let mut solved: Vec<f64> = Vec::with_capacity(3);
        for (index, (surface, guess)) in surfaces.iter().zip(guesses.iter_mut()).enumerate() {
            if silent == Some(index) {
                continue;
            }
            let (value, normal) = if sideways {
                distance_toward(surface, point, guess)?
            } else {
                distance_near(surface, point, guess)?
            };
            let mut rest = normal;
            let mut right = -value;
            for (axis, c) in basis.iter().zip(&solved) {
                let along = normal.dot(*axis);
                right -= along * c;
                rest = rest - *axis * along;
            }
            let own = rest.length();
            if own <= SAME_DIRECTION
                || basis.len() >= 3
                || (own <= TANGENT && right.abs() <= HELD * tol)
            {
                continue;
            }
            // Sideways: a surface that meets those before it at a shallow
            // angle waits until the point is on them.
            if sideways && own <= SHALLOW && solved.iter().any(|c| c.abs() > HELD * tol) {
                continue;
            }
            basis.push(rest * (1.0 / own));
            solved.push(right / own);
        }
        let mut step = Vec3::ZERO;
        for (c, axis) in solved.iter().zip(&basis) {
            step += *axis * *c;
        }
        if !step.is_finite() {
            return None;
        }
        point += step;
        last = basis;
        if step.length() <= 1e-15 * (1.0 + point.length()) {
            break;
        }
    }
    point.is_finite().then_some((point, last))
}

/// The angle nearest `near` that is `angle` up to whole turns.
pub(crate) fn unwrapped(angle: f64, near: f64) -> f64 {
    let mut difference = (angle - near) % TAU;
    if difference > PI {
        difference -= TAU;
    } else if difference < -PI {
        difference += TAU;
    }
    near + difference
}

/// The axis a circle on `surface` turns about, when the surface has one:
/// a point of it and its direction. A sphere turns about any line through
/// its centre, so about the one along `along`.
fn axis_of(surface: &Surface, along: Vec3) -> Option<(Vec3, Vec3)> {
    match surface {
        Surface::Cylinder(s) => Some((s.frame().origin(), s.frame().z())),
        Surface::Cone(s) => Some((s.frame().origin(), s.frame().z())),
        Surface::Torus(s) => Some((s.frame().origin(), s.frame().z())),
        Surface::Sphere(s) => Some((s.frame().origin(), along)),
        Surface::Plane(_) | Surface::Bezier(_) | Surface::Nurbs(_) => None,
    }
}

/// Where along an edge it is held against its faces.
const ON_SURFACE_SAMPLES: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

fn lies_on(curve: &Curve3, range: (f64, f64), surfaces: &[&Surface], tol: f64) -> bool {
    ON_SURFACE_SAMPLES.iter().all(|s| {
        let point = curve.eval(range.0 + (range.1 - range.0) * s);
        surfaces
            .iter()
            .all(|surface| distance(surface, point).is_some_and(|(value, _)| value.abs() <= tol))
    })
}

/// What an edge is made into.
enum Remade {
    Edge(Curve3, (f64, f64)),
    Gone,
    OffSurface,
    /// Found again as no closed form, and the faces do not meet all along
    /// it as it ran.
    Lost,
    /// To be found again where its faces meet ([`fitted`]).
    Fit,
}

/// True when `a` is more than `b`, and both are numbers.
fn more(a: f64, b: f64) -> bool {
    a.is_finite() && b.is_finite() && a > b
}

/// The edge with curve `old` over `range` made again from `start` to
/// `end` on `surfaces`, the surfaces of its faces (`moved` says whether
/// one of them is new): a line, a circle, or the same curve between its
/// new ends; [`Remade::Fit`] where it has to be found again where its
/// faces meet ([`fitted`], which the caller calls, or takes the border's
/// iso-curve of a face that keeps its surface).
#[allow(clippy::too_many_arguments)]
fn remake_unfitted(
    old: &Curve3,
    range: (f64, f64),
    start: Vec3,
    end: Vec3,
    closed: bool,
    surfaces: &[&Surface],
    moved: bool,
    tolerance: &Tolerance,
) -> Remade {
    let tol = tolerance.linear;
    let middle = 0.5 * (range.0 + range.1);
    let freeform = surfaces
        .iter()
        .any(|s| matches!(s, Surface::Bezier(_) | Surface::Nurbs(_)));
    // A line or a circle that is not one any more on a freeform face, or a
    // curve of another kind: found again where the surfaces meet.
    let general = |closed_form: Remade| match closed_form {
        Remade::OffSurface if freeform || !matches!(old, Curve3::Line(_) | Curve3::Circle(_)) => {
            Remade::Fit
        }
        other => other,
    };
    general(match old {
        Curve3::Line(_) => {
            let (_, was) = old.derivatives(middle);
            let length = start.distance(end);
            if !more(length, tol) || (end - start).dot(was) <= 0.0 {
                return Remade::Gone;
            }
            let Ok(line) = Line::through(start, end) else {
                return Remade::Gone;
            };
            let curve = Curve3::Line(line);
            if lies_on(&curve, (0.0, length), surfaces, tol) {
                Remade::Edge(curve, (0.0, length))
            } else {
                Remade::OffSurface
            }
        }
        Curve3::Circle(circle) => {
            let along = circle.frame().z();
            let parallel =
                |direction: Vec3| direction.cross(along).length() <= tolerance.angular.max(1e-12);
            let (origin, _) = surfaces
                .iter()
                .filter_map(|surface| axis_of(surface, along))
                .find(|(_, direction)| parallel(*direction))
                .unwrap_or((circle.frame().origin(), along));
            let centre = origin + along * (start - origin).dot(along);
            let radius = start.distance(centre);
            if !more(radius, tol) {
                return Remade::Gone;
            }
            let Some(frame) = circle.frame().at(centre) else {
                return Remade::Gone;
            };
            let Ok(made) = Circle::new(frame, radius) else {
                return Remade::Gone;
            };
            let curve = Curve3::Circle(made);
            let (Ok(at_start), Ok(at_end)) = (curve.project(start), curve.project(end)) else {
                return Remade::Gone;
            };
            let from = unwrapped(at_start, range.0);
            // Angles that did not change stay what they were, bit for bit.
            let from = if (from - range.0).abs() <= 1e-12 {
                range.0
            } else {
                from
            };
            let to = if closed {
                from + (range.1 - range.0)
            } else {
                let to = unwrapped(at_end, range.1);
                if (to - range.1).abs() <= 1e-12 {
                    range.1
                } else {
                    to
                }
            };
            if !more(to, from) || (to - from) * radius <= tol || to - from > TAU + 1e-9 {
                return Remade::Gone;
            }
            if lies_on(&curve, (from, to), surfaces, tol) {
                Remade::Edge(curve, (from, to))
            } else {
                Remade::OffSurface
            }
        }
        Curve3::Ellipse(_) | Curve3::Nurbs(_) => {
            if moved {
                return Remade::Fit;
            }
            // The same curve, between the places its ends moved to.
            let (Ok(at_start), Ok(at_end)) = (old.project(start), old.project(end)) else {
                return Remade::Fit;
            };
            let (from, to) = match old.period() {
                Some(_) if closed => {
                    let from = unwrapped(at_start, range.0);
                    (from, from + (range.1 - range.0))
                }
                Some(_) => (unwrapped(at_start, range.0), unwrapped(at_end, range.1)),
                None => (at_start, at_end),
            };
            if old.eval(from).distance(start) > tol || old.eval(to).distance(end) > tol {
                return Remade::Fit;
            }
            if !more(to, from) {
                return Remade::Gone;
            }
            Remade::Edge(old.clone(), (from, to))
        }
    })
}

/// Samples an edge made again by [`fitted`] starts with, and most it may
/// have; passes of its refinement.
const EDGE_START: usize = 8;
const EDGE_SAMPLES: usize = 4096;
const EDGE_PASSES: usize = 24;

/// How far an edge found again may be from where its surfaces meet, as a
/// share of the tolerance (as the offset of a freeform face is held).
pub(crate) const EDGE_SHARE: f64 = 0.25;

/// A point of an edge found again: its parameter, where it is, and the
/// feet on its surfaces it was found from.
type Sample = (f64, Vec3, Vec<Option<(f64, f64)>>);

/// The edge with curve `old` over `range` found again from `start` to
/// `end` where `surfaces` meet, when it is no longer a curve of a kind
/// made in closed form: every point of the old curve, moved as the ends
/// of the edge moved (in proportion along it), is taken to the nearest
/// point where the surfaces meet ([`meet_near`], each from where the one
/// before it landed), and a cubic is laid through those points at
/// the old curve's parameters, held against them at the middle of every
/// step (and at the quarters at the end) and refined where it does not
/// hold to a share of the tolerance, as a freeform offset is. The ends are
/// the vertices. An edge whose points do not keep their order along it
/// (it turns round or shrinks to nothing) is gone; one whose points do
/// not meet on all its surfaces, or whose fit does not come within the
/// tolerance, is no longer there in the form it had.
///
/// **When that loses the edge, once more from a better guess**
/// (2026-11-14, the kernel set's H-02): an edge that shrinks far more
/// than its ends move (the mitre where two rounds of 2.4 meet, shelled
/// 2: six times smaller, its ends moved 2.8) is guessed badly by its old
/// points moved as its ends moved: they lie far off the new curve, and
/// the planes square to the old curve there cut the new one out of
/// order, so it looked turned round. It is tried again with the old
/// curve carried onto the new ends by the similarity that takes the old
/// chord to the new one ([`Similarity`]), each point held to the plane
/// square to the carried curve; every check is the same (in order along
/// the carried curve, on its surfaces, fitted to a share of the
/// tolerance). An edge found by the first try is never tried again, and
/// an edge with a freeform face is not tried again here at all (a lost
/// edge there sends the operation on to continue the freeform faces past
/// their borders, as before), so what was built before is built the same.
#[allow(clippy::too_many_arguments)]
fn fitted(
    old: &Curve3,
    range: (f64, f64),
    start: Vec3,
    end: Vec3,
    surfaces: &[&Surface],
    tangent: bool,
    sideways: bool,
    tolerance: &Tolerance,
) -> Remade {
    let first = fitted_from(
        old, range, start, end, surfaces, tangent, sideways, None, tolerance,
    );
    // Not with a freeform surface among the edge's: a lost edge there is
    // answered by continuing the freeform faces past their borders and
    // trying again (`offset_face::at`), and a retry here, on the faces not
    // yet continued, could find the edge where that path found it later
    // and so change a body that built before (the review of 2026-11-14).
    let freeform = surfaces
        .iter()
        .any(|s| matches!(s, Surface::Bezier(_) | Surface::Nurbs(_)));
    if freeform
        || !matches!(first, Remade::Gone | Remade::Lost)
        || craft3d_geom::allowance::exhausted()
    {
        return first;
    }
    let Some(carried) = Similarity::onto(old, range, start, end, tolerance.linear) else {
        return first;
    };
    match fitted_from(
        old,
        range,
        start,
        end,
        surfaces,
        tangent,
        sideways,
        Some(&carried),
        tolerance,
    ) {
        found @ Remade::Edge(..) => found,
        _ => first,
    }
}

/// The similarity (a turn, a scale and a shift) that takes the chord of
/// an open edge's old curve onto the chord between its new ends: the
/// shortest turn from the old chord's direction to the new one's. Only
/// for chords longer than the tolerance whose directions are less than a
/// right angle apart: an edge whose ends changed places has turned round,
/// and no guess makes it the same edge.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Similarity {
    from: Vec3,
    to: Vec3,
    scale: f64,
    /// The old chord's direction crossed with the new one's (its length
    /// the sine of the turn), and the cosine of the turn.
    axis: Vec3,
    cosine: f64,
}

impl Similarity {
    fn onto(old: &Curve3, range: (f64, f64), start: Vec3, end: Vec3, tol: f64) -> Option<Self> {
        let (a, b) = (old.eval(range.0), old.eval(range.1));
        let (was, is) = (b - a, end - start);
        if !(more(was.length(), tol) && more(is.length(), tol)) {
            return None;
        }
        let (u, w) = (was.normalized()?, is.normalized()?);
        let cosine = u.dot(w);
        if !(cosine.is_finite() && cosine > 0.0) {
            return None;
        }
        let scale = is.length() / was.length();
        scale.is_finite().then_some(Self {
            from: a,
            to: start,
            scale,
            axis: u.cross(w),
            cosine,
        })
    }

    /// The direction `v` turned (Rodrigues' rule from the cross product and
    /// the cosine: no angle is taken).
    fn turned(&self, v: Vec3) -> Vec3 {
        v * self.cosine + self.axis.cross(v) + self.axis * (self.axis.dot(v) / (1.0 + self.cosine))
    }

    fn point(&self, p: Vec3) -> Vec3 {
        self.to + self.turned(p - self.from) * self.scale
    }
}

/// [`fitted`]'s try: the points guessed from the old curve moved as its
/// ends moved (`carried` none), or carried by a similarity.
#[allow(clippy::too_many_arguments)]
fn fitted_from(
    old: &Curve3,
    range: (f64, f64),
    start: Vec3,
    end: Vec3,
    surfaces: &[&Surface],
    tangent: bool,
    sideways: bool,
    carried: Option<&Similarity>,
    tolerance: &Tolerance,
) -> Remade {
    let tol = tolerance.linear;
    let limit = EDGE_SHARE * tol;
    if !more(range.1, range.0) {
        return Remade::Gone;
    }
    // The exact point at `t`: where the surfaces meet nearest the old
    // curve's point there moved as its ends moved (in proportion along
    // the parameter), so that the points run from the new start to the
    // new end as the old ones ran between the old; found from the guesses
    // of a sample nearby. A closed edge has no ends to run between: its
    // points start where they were (the one vertex's move, carried round
    // a narrow loop, would push the far side the wrong way).
    let closed = start == end && old.eval(range.0).distance(old.eval(range.1)) <= tol;
    let (from, to) = if closed {
        (Vec3::ZERO, Vec3::ZERO)
    } else {
        (start - old.eval(range.0), end - old.eval(range.1))
    };
    let moved = |t: f64| match carried {
        Some(similarity) => similarity.point(old.eval(t)),
        None => {
            let share = (t - range.0) / (range.1 - range.0);
            old.eval(t) + from * (1.0 - share) + to * share
        }
    };
    // Faces tangent all along the edge (their normals one direction but
    // for what fitting left, [`TANGENT`]) meet in it as one surface: the
    // points are found on the first, the others held to it at the end
    // ([`lies_on`]); deciding that point by point would make the points
    // jump where a residual of rounding crosses the line.
    let meeting = if tangent {
        surfaces.get(..1).unwrap_or(surfaces)
    } else {
        surfaces
    };
    // Each point is held to the plane through its start square to the old
    // curve there: where the surfaces meet is a curve, along which a point
    // that lies on them both could be anywhere; the plane makes it one
    // point, so the points are a smooth function of the parameter.
    let exact = |t: f64, guesses: &mut Vec<Option<(f64, f64)>>| {
        let count = meeting.len();
        let from = moved(t);
        let (_, along) = old.derivatives(t);
        let along = match carried {
            Some(similarity) => similarity.turned(along),
            None => along,
        };
        let across = Surface::Plane(Plane::new(Frame::from_z(from, along)?));
        let mut held: Vec<&Surface> = meeting.to_vec();
        held.push(&across);
        let mut feet: Vec<Option<(f64, f64)>> = guesses.get(..count)?.to_vec();
        feet.push(None);
        let point = meet_with(&held, from, tol, &mut feet, sideways, false)?;
        for (slot, foot) in guesses.iter_mut().zip(feet) {
            *slot = foot;
        }
        Some(point)
    };
    let mut params: Vec<f64> = (0..=EDGE_START)
        .map(|k| range.0 + (range.1 - range.0) * k as f64 / EDGE_START as f64)
        .collect();
    if let Some(last) = params.last_mut() {
        *last = range.1;
    }
    let mut guesses: Vec<Vec<Option<(f64, f64)>>> = Vec::with_capacity(params.len());
    let mut points: Vec<Vec3> = Vec::with_capacity(params.len());
    let mut running = vec![None; surfaces.len()];
    for (k, &t) in params.iter().enumerate() {
        let point = if k == 0 {
            // Where the walk starts: the feet of the first vertex.
            let _ = meet_near(surfaces, start, tol, &mut running);
            Some(start)
        } else if k == EDGE_START {
            Some(end)
        } else {
            exact(t, &mut running)
        };
        let Some(point) = point else {
            return Remade::Lost;
        };
        points.push(point);
        guesses.push(running.clone());
    }
    let mut verify = false;
    for _ in 0..EDGE_PASSES {
        // In order along the old curve, and not shrunk to nothing.
        let length: f64 = points
            .windows(2)
            .map(|w| match w {
                [a, b] => a.distance(*b),
                _ => 0.0,
            })
            .sum();
        if !more(length, tol) {
            return Remade::Gone;
        }
        for (pair, t) in points.windows(2).zip(&params) {
            let [a, b] = pair else { continue };
            // In order along the guess the points were held across: the
            // old curve, or the old curve carried.
            let (_, along) = old.derivatives(*t);
            let along = match carried {
                Some(similarity) => similarity.turned(along),
                None => along,
            };
            if (*b - *a).dot(along) < -tol * along.length() {
                return Remade::Gone;
            }
        }
        let Some(curve) = curve_through(&params, &points) else {
            return Remade::Lost;
        };
        let fractions: &[f64] = if verify { &[0.25, 0.5, 0.75] } else { &[0.5] };
        let mut over: Vec<f64> = Vec::with_capacity(params.len());
        let mut found: Vec<Vec<Sample>> = Vec::new();
        for (index, pair) in params.windows(2).enumerate() {
            let [a, b] = pair else { continue };
            let mut worst = 0.0_f64;
            let mut here = Vec::new();
            for f in fractions {
                let t = a + (b - a) * f;
                let mut near = guesses.get(index).cloned().unwrap_or_default();
                let Some(point) = exact(t, &mut near) else {
                    return Remade::Lost;
                };
                let off = curve.eval(t).distance(point) / limit;
                worst = worst.max(if off.is_nan() { f64::INFINITY } else { off });
                here.push((t, point, near));
            }
            over.push(worst);
            found.push(here);
        }
        if over.iter().all(|o| *o <= 1.0) {
            if verify {
                let curve = Curve3::Nurbs(curve);
                return if lies_on(&curve, range, surfaces, tol) {
                    Remade::Edge(curve, range)
                } else {
                    Remade::Lost
                };
            }
            verify = true;
            continue;
        }
        let split = craft3d_geom::offset::steps_to_halve(&over, craft3d_geom::offset::CUBIC_DECAY);
        let mut added: Vec<Sample> = Vec::new();
        for (here, halve) in found.into_iter().zip(split) {
            if !halve {
                continue;
            }
            // The middle of the step, found already.
            if let Some(middle) = here.into_iter().nth(fractions.len() / 2) {
                added.push(middle);
            }
        }
        if params.len() + added.len() > EDGE_SAMPLES {
            return Remade::Lost;
        }
        for (t, point, near) in added {
            let at = params.partition_point(|p| *p < t);
            params.insert(at, t);
            points.insert(at, point);
            guesses.insert(at, near);
        }
    }
    Remade::Lost
}

/// Whether the two faces of an edge of `body` (`faces`) are tangent all
/// along it, their outward normals one direction to [`TANGENT`]: at five
/// points of the edge's curve over `range`.
fn tangent_along(body: &Body, faces: &[FaceId], curve: &Curve3, range: (f64, f64)) -> bool {
    let [a, b] = faces else {
        return false;
    };
    let (Some(a), Some(b)) = (body.face(*a), body.face(*b)) else {
        return false;
    };
    (0..=4).all(|k| {
        let p = curve.eval(range.0 + (range.1 - range.0) * f64::from(k) / 4.0);
        match (a.outward(p), b.outward(p)) {
            (Some(m), Some(n)) => m.dot(n) > 0.0 && m.cross(n).length() <= TANGENT,
            _ => false,
        }
    })
}

/// Points along an edge, in the direction of its curve: enough to tell
/// which way a loop turns and where it lies.
fn sampled(body: &Body, edge: EdgeId) -> Vec<Vec3> {
    let Some(edge) = body.edge(edge) else {
        return Vec::new();
    };
    let steps = match edge.curve {
        Curve3::Line(_) => 1_u32,
        _ => 16,
    };
    (0..=steps)
        .map(|i| {
            let s = f64::from(i) / f64::from(steps);
            edge.curve
                .eval(edge.range.0 + (edge.range.1 - edge.range.0) * s)
        })
        .collect()
}

/// The points of a loop in order, each edge in the loop's direction.
pub(crate) fn loop_points(body: &Body, loop_: craft3d_topo::LoopId) -> Vec<Vec3> {
    let mut points = Vec::new();
    for coedge in body.loop_(loop_).into_iter().flat_map(|l| &l.coedges) {
        let mut along = sampled(body, coedge.edge);
        if coedge.reversed {
            along.reverse();
        }
        points.extend(along);
    }
    points
}

/// The area a closed row of points in the plane of `frame` goes round,
/// counter-clockwise positive seen against the frame's z.
fn signed_area(frame: &Frame, points: &[Vec3]) -> f64 {
    let flat: Vec<(f64, f64)> = points
        .iter()
        .map(|p| {
            let (x, y, _) = frame.local(*p);
            (x, y)
        })
        .collect();
    let mut twice = 0.0;
    for (index, (x0, y0)) in flat.iter().enumerate() {
        let Some((x1, y1)) = flat.get((index + 1) % flat.len().max(1)) else {
            continue;
        };
        twice += x0 * y1 - x1 * y0;
    }
    0.5 * twice
}

/// Whether the loops of the flat face `face` are simple curves that do
/// not meet, one of them around the others. `None` when the face has an
/// edge this cannot tell for (an ellipse, a spline).
fn outline_is_simple(body: &Body, face: FaceId, tolerance: &Tolerance) -> Option<bool> {
    let face = body.face(face)?;
    let Surface::Plane(plane) = &face.surface else {
        return None;
    };
    let frame = plane.frame();
    let flat = |p: Vec3| {
        let (x, y, _) = frame.local(p);
        [x, y]
    };
    let mut curves = Vec::new();
    let mut seen: BTreeSet<EdgeId> = BTreeSet::new();
    for coedge in face
        .loops
        .iter()
        .filter_map(|l| body.loop_(*l))
        .flat_map(|l| &l.coedges)
    {
        if !seen.insert(coedge.edge) {
            // A seam in a flat face: not an outline this can judge.
            return None;
        }
        let edge = body.edge(coedge.edge)?;
        let (start, end) = (body.vertex(edge.start)?.point, body.vertex(edge.end)?.point);
        let curve = match &edge.curve {
            Curve3::Line(_) => Curve2::Line {
                start: flat(start),
                end: flat(end),
            },
            Curve3::Circle(circle) => {
                let centre = flat(circle.frame().origin());
                if edge.start == edge.end {
                    Curve2::Circle {
                        center: centre,
                        radius: circle.radius(),
                    }
                } else if circle.frame().z().dot(frame.z()) > 0.0 {
                    Curve2::Arc {
                        center: centre,
                        start: flat(start),
                        end: flat(end),
                    }
                } else {
                    Curve2::Arc {
                        center: centre,
                        start: flat(end),
                        end: flat(start),
                    }
                }
            }
            Curve3::Ellipse(_) | Curve3::Nurbs(_) => return None,
        };
        curves.push(SketchCurve {
            id: u64::from(coedge.edge.0),
            curve,
        });
    }
    if curves.is_empty() {
        return Some(true);
    }
    Some(match detect_profiles(&curves, tolerance) {
        Ok(profiles) => match profiles.as_slice() {
            [one] => one.holes.len() + 1 == face.loops.len(),
            _ => false,
        },
        Err(_) => false,
    })
}

/// Per loop of a round face about an axis, where along the axis it lies
/// (the mean over its points).
fn heights(body: &Body, face: FaceId) -> Option<Vec<f64>> {
    let face = body.face(face)?;
    let (origin, along) = match &face.surface {
        Surface::Cylinder(s) => (s.frame().origin(), s.frame().z()),
        Surface::Cone(s) => (s.frame().origin(), s.frame().z()),
        Surface::Sphere(s) => (s.frame().origin(), s.frame().z()),
        _ => return None,
    };
    Some(
        face.loops
            .iter()
            .map(|l| {
                let points = loop_points(body, *l);
                let sum: f64 = points.iter().map(|p| (*p - origin).dot(along)).sum();
                sum / (points.len().max(1) as f64)
            })
            .collect(),
    )
}

/// The faces that have an edge among `edges`, ascending.
fn faces_with(uses: &[Vec<FaceId>], edges: &BTreeSet<EdgeId>) -> BTreeSet<FaceId> {
    edges
        .iter()
        .filter_map(|edge| uses.get(edge.0 as usize))
        .flatten()
        .copied()
        .collect()
}

/// `body` with the surfaces of `surfaces` in the place of those of its
/// faces, closed again around them (see the module documentation). The
/// result has the faces, edges and vertices of `body`, each under its
/// own number, and passes `craft3d_topo::check`.
pub(crate) fn resurface(
    body: &Body,
    surfaces: &BTreeMap<FaceId, Surface>,
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, TweakError> {
    resurface_at(body, surfaces, &BTreeSet::new(), operation, tolerance)
}

/// An edge that lies along a border of the parameter range of one of its
/// faces (as the borders of a rolling-ball blend do): the face, and the
/// parameter held along it, u (`u_fixed`) or v, at `value`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Iso {
    pub face: FaceId,
    pub u_fixed: bool,
    pub value: f64,
}

/// [`resurface`] for new surfaces that keep the parameters of the old
/// ones (offsets): an edge of `iso` whose face has a new freeform surface
/// is made again as the same iso-curve of the new surface, between the
/// new vertices, when that lies on its other faces; it lies exactly on
/// the new surface, which a curve found where tangent surfaces meet
/// cannot be told to.
///
/// The faces of `rebuilt` have new surfaces in the parameters of the old
/// ones, point for point as near as the place of the new point goes (a
/// fillet rebuilt between offset faces: the same `u` is the same section):
/// a vertex or an edge on one is looked for from where its old points
/// are on the new surface ([`Foreseen`]), not from where they were.
///
/// `corners` says what becomes of a vertex of four or more faces that the
/// new surfaces no longer meet in ([`Corners`]).
pub(crate) fn resurface_along(
    body: &Body,
    surfaces: &BTreeMap<FaceId, Surface>,
    iso: &BTreeMap<EdgeId, Iso>,
    rebuilt: &BTreeSet<FaceId>,
    corners: Corners,
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, TweakError> {
    resurface_with(
        body,
        surfaces,
        &BTreeSet::new(),
        None,
        iso,
        rebuilt,
        corners,
        operation,
        tolerance,
    )
}

/// Where an old vertex of a face whose new surface keeps its parameters
/// goes: the new surface at the parameters of the point on the old one.
/// Only a vertex is carried so: an edge's points start between its ends
/// moved, as for any edge (where it crosses another face, a point carried
/// along the rebuilt face's normal is off the other face's offset by the
/// distance times what the angle between them leaves).
#[derive(Clone, Copy)]
struct Foreseen<'a> {
    old: &'a Surface,
    new: &'a Surface,
}

impl Foreseen<'_> {
    fn at(&self, point: Vec3) -> Option<Vec3> {
        let (u, v) = self.old.project(point).ok()?;
        let to = self.new.eval(u, v);
        to.is_finite().then_some(to)
    }
}

/// Steps of the search along a border for where a face crosses it.
const BORDER_STEPS: usize = 32;

/// The point of the border `along` of `surface` nearest `near` where the
/// surface among `others` that crosses it most steeply (its normal is
/// furthest from the surface's there, beyond [`TANGENT`]) crosses it, when
/// every one of `others` passes within `tol` of it; `None` when none
/// crosses it.
fn onto_border(
    surface: &craft3d_geom::NurbsSurface,
    along: &Iso,
    near: Vec3,
    others: &[&Surface],
    tol: f64,
) -> Option<Vec3> {
    let (curve, _) = whole_iso(surface, along)?;
    let mut t = curve.project(near).ok()?;
    let own = |t: f64| {
        let (u, v) = if along.u_fixed {
            (along.value, t)
        } else {
            (t, along.value)
        };
        Surface::Nurbs(surface.clone()).normal(u, v)
    };
    let here = own(t)?;
    let steepest = others
        .iter()
        .filter_map(|s| distance(s, near).map(|(_, n)| (s, n.cross(here).length())))
        .filter(|(_, sine)| *sine > TANGENT)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(s, _)| *s)?;
    for _ in 0..BORDER_STEPS {
        let (point, tangent) = curve.derivatives(t);
        let (value, normal) = distance(steepest, point)?;
        let rate = normal.dot(tangent);
        if rate == 0.0 || !rate.is_finite() {
            return None;
        }
        let step = value / rate;
        t -= step;
        if !t.is_finite() {
            return None;
        }
        if (step * tangent.length()).abs() <= 1e-3 * tol {
            break;
        }
    }
    let point = curve.eval(t);
    others
        .iter()
        .all(|s| distance(s, point).is_some_and(|(value, _)| value.abs() <= tol))
        .then_some(point)
}

/// The whole iso-curve of `surface` at `along` and its domain.
fn whole_iso(surface: &craft3d_geom::NurbsSurface, along: &Iso) -> Option<(Curve3, (f64, f64))> {
    let (degree_u, degree_v) = surface.degrees();
    let (count_u, count_v) = surface.counts();
    let points = surface.control_points();
    let weights = surface.weights();
    let at = |i: usize, j: usize| {
        let k = i.checked_mul(count_v)?.checked_add(j)?;
        Some((*points.get(k)?, *weights.get(k)?))
    };
    // Per control point of the iso-curve, the homogeneous sum over the
    // other direction at `value`: two polynomial curves, of the weighted
    // points and of the weights.
    let (lines, inner, knots_inner, degree_inner, knots_outer, degree_outer) = if along.u_fixed {
        (
            count_v,
            count_u,
            surface.knots_u(),
            degree_u,
            surface.knots_v(),
            degree_v,
        )
    } else {
        (
            count_u,
            count_v,
            surface.knots_v(),
            degree_v,
            surface.knots_u(),
            degree_u,
        )
    };
    let mut ctrl = Vec::with_capacity(lines);
    let mut ws = Vec::with_capacity(lines);
    for line in 0..lines {
        let mut weighted = Vec::with_capacity(inner);
        let mut plain = Vec::with_capacity(inner);
        for k in 0..inner {
            let (p, w) = if along.u_fixed {
                at(k, line)?
            } else {
                at(line, k)?
            };
            weighted.push(p * w);
            plain.push(Vec3::new(w, 0.0, 0.0));
        }
        let sum = NurbsCurve::polynomial(degree_inner, knots_inner.to_vec(), weighted).ok()?;
        let weight = NurbsCurve::polynomial(degree_inner, knots_inner.to_vec(), plain).ok()?;
        let w = weight.eval(along.value).x;
        if !(w > 0.0 && w.is_finite()) {
            return None;
        }
        ctrl.push(sum.eval(along.value) * (1.0 / w));
        ws.push(w);
    }
    let curve = Curve3::Nurbs(NurbsCurve::new(degree_outer, knots_outer.to_vec(), ctrl, ws).ok()?);
    let domain = curve.domain();
    Some((curve, domain))
}

/// The iso-curve of `surface` at `along`, as a curve over the range from
/// `start` to `end` (the whole curve, when `closed`), turned round when
/// the parameter runs the other way. `None` when it does not reach them
/// within `tol`.
fn iso_curve(
    surface: &craft3d_geom::NurbsSurface,
    along: &Iso,
    start: Vec3,
    end: Vec3,
    closed: bool,
    tol: f64,
) -> Option<(Curve3, (f64, f64))> {
    let (curve, _) = whole_iso(surface, along)?;
    let (a, b) = curve.domain();
    let (from, to) = if closed {
        (a, b)
    } else {
        (curve.project(start).ok()?, curve.project(end).ok()?)
    };
    let (curve, from, to) = if to < from {
        let Curve3::Nurbs(n) = &curve else {
            return None;
        };
        let turned = Curve3::Nurbs(craft3d_geom::refine::reversed(n).ok()?);
        let (c, d) = turned.domain();
        // Reversal maps t to c + d − t over the same span.
        (turned, c + d - from, c + d - to)
    } else {
        (curve, from, to)
    };
    let holds = curve.eval(from).distance(start) <= tol && curve.eval(to).distance(end) <= tol;
    (holds && more(to, from)).then_some((curve, (from, to)))
}

/// Whether an open edge that ran along the border `along` of `surface`
/// (its curve `old` over `range`) would run along it the other way from
/// `start` to `end`: its ends have changed places along the border, which
/// is an edge turned round ([`EDGE_GONE`]), not the same edge
/// ([`iso_curve`] alone would give the border between the new ends,
/// whichever way it runs: the seam of a vase whose cap went past the cap
/// across from it came out so, 2026-11-14). Not told, and so not said, for
/// a border that closes on itself, where a parameter says nothing of an
/// order, or where one of the four ends does not lie on the border within
/// the tolerance: a vertex found where a plane meets two halves of one
/// turned surface is free round their circle and can come to lie far from
/// the seam (13 off it, 196 degrees round, on a vase whose top was moved to
/// 20 tolerances from its foot); its nearest point on the border says
/// nothing of where the edge ends, and taking it for the end refused right
/// offsets (the review of 2026-11-14). Such an edge is left to the steps
/// that follow, as before.
fn iso_turned(
    surface: &craft3d_geom::NurbsSurface,
    along: &Iso,
    old: &Curve3,
    range: (f64, f64),
    start: Vec3,
    end: Vec3,
    tol: f64,
) -> bool {
    let Some((curve, (a, b))) = whole_iso(surface, along) else {
        return false;
    };
    if curve.eval(a).distance(curve.eval(b)) <= tol {
        return false;
    }
    let at = |p: Vec3| {
        curve
            .project(p)
            .ok()
            .filter(|t| t.is_finite() && curve.eval(*t).distance(p) <= tol)
    };
    let (Some(from_was), Some(to_was), Some(from), Some(to)) = (
        at(old.eval(range.0)),
        at(old.eval(range.1)),
        at(start),
        at(end),
    ) else {
        return false;
    };
    (to_was - from_was) * (to - from) < 0.0
}

/// [`resurface`], with the vertices of `also` found again too, although
/// none of their faces has a new surface: the vertices a change of
/// topology put where they do not lie yet (delete face merges the two
/// ends of a strip into one vertex, which lies where the faces round it
/// meet). The edges that end in them are made again likewise.
pub(crate) fn resurface_at(
    body: &Body,
    surfaces: &BTreeMap<FaceId, Surface>,
    also: &BTreeSet<VertexId>,
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, TweakError> {
    resurface_with(
        body,
        surfaces,
        also,
        None,
        &BTreeMap::new(),
        &BTreeSet::new(),
        Corners::Kept,
        operation,
        tolerance,
    )
}

/// [`resurface`] for faces that all move by one rigid motion `rigid`
/// (their new surfaces are the images of the old ones): a vertex all of
/// whose faces move, and an edge both of whose faces do, are the images
/// of what they were, exactly, whatever their kind; the rest is found
/// again as [`resurface`] finds it.
pub(crate) fn resurface_rigid(
    body: &Body,
    surfaces: &BTreeMap<FaceId, Surface>,
    rigid: &Transform,
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, TweakError> {
    resurface_with(
        body,
        surfaces,
        &BTreeSet::new(),
        Some(rigid),
        &BTreeMap::new(),
        &BTreeSet::new(),
        Corners::Kept,
        operation,
        tolerance,
    )
}

/// What becomes of a vertex of four or more faces that the new surfaces no
/// longer meet in one point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Corners {
    /// Refused, as every operation but the shell's hollow does: its
    /// entities keep their numbers and nothing is added.
    Kept,
    /// Split into the tree of vertices of three faces its moved faces
    /// make, when that is one tree ([`split`]); the new vertices and edges
    /// come after the old ones, and the old vertex is one of them.
    Split,
}

#[allow(clippy::too_many_arguments)]
fn resurface_with(
    body: &Body,
    surfaces: &BTreeMap<FaceId, Surface>,
    also: &BTreeSet<VertexId>,
    rigid: Option<&Transform>,
    iso: &BTreeMap<EdgeId, Iso>,
    rebuilt: &BTreeSet<FaceId>,
    corners: Corners,
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, TweakError> {
    let tol = tolerance.linear;
    // The first rebuilt face of a set of faces, as where its points go.
    let foreseen = |faces: &[FaceId]| -> Option<Foreseen<'_>> {
        let face = *faces.iter().find(|f| rebuilt.contains(f))?;
        Some(Foreseen {
            old: &body.face(face)?.surface,
            new: surfaces.get(&face)?,
        })
    };
    // Whether every one of `faces` moves with the rigid motion.
    let carried = |faces: &[FaceId]| {
        rigid.is_some() && !faces.is_empty() && faces.iter().all(|face| surfaces.contains_key(face))
    };
    let uses = edge_faces(body);
    let mut made = body.clone();
    for (face, surface) in surfaces {
        let Some(slot) = made.faces.get_mut(face.0 as usize) else {
            return Err(TweakError::Unsupported {
                faces: vec![*face],
                edge: None,
                reason: "there is no such face in the body",
            });
        };
        slot.surface = surface.clone();
    }

    // The faces around every vertex, and the vertices on a new surface.
    let mut around: Vec<BTreeSet<FaceId>> = vec![BTreeSet::new(); body.vertices.len()];
    for (index, edge) in body.edges.iter().enumerate() {
        for vertex in [edge.start, edge.end] {
            if let (Some(list), Some(faces)) = (around.get_mut(vertex.0 as usize), uses.get(index))
            {
                list.extend(faces.iter().copied());
            }
        }
    }
    let mut moved_vertices: BTreeSet<VertexId> = BTreeSet::new();
    // Vertices of four or more faces the new surfaces no longer meet in,
    // to be split once the others are placed: where it was, and the error
    // said if it cannot be.
    let mut to_split: Vec<(VertexId, Vec3, TweakError)> = Vec::new();
    for (index, faces) in around.iter().enumerate() {
        let vertex = VertexId(index_u32(index));
        if !(also.contains(&vertex) || faces.iter().any(|face| surfaces.contains_key(face))) {
            continue;
        }
        let Some(was) = body.vertex(vertex).map(|v| v.point) else {
            continue;
        };
        if let Some(motion) = rigid
            && carried(&faces.iter().copied().collect::<Vec<_>>())
        {
            let point = motion.point(was);
            if !point.is_finite() {
                return Err(TweakError::Kernel(KernelError::InvalidSize {
                    name: "motion",
                    value: f64::NAN,
                }));
            }
            if let Some(slot) = made.vertices.get_mut(index) {
                slot.point = point;
            }
            moved_vertices.insert(vertex);
            continue;
        }
        let meeting: Vec<&Surface> = faces
            .iter()
            .filter_map(|face| made.face(*face).map(|f| &f.surface))
            .collect();
        let found =
            match foreseen(&faces.iter().copied().collect::<Vec<_>>()).and_then(|f| f.at(was)) {
                Some(from) => meet_foreseen(&meeting, from, tol),
                None => meet(&meeting, was, tol),
            };
        let Some(point) = found else {
            // Cut short by the work bound (`meet_with` stops when it is
            // spent): say so, not that the faces do not meet.
            if craft3d_geom::allowance::exhausted() {
                return Err(spent());
            }
            let error = if meeting
                .iter()
                .any(|s| matches!(s, Surface::Bezier(_) | Surface::Nurbs(_)))
            {
                TweakError::Unsupported {
                    faces: faces.iter().copied().collect(),
                    edge: None,
                    reason: FREEFORM,
                }
            } else {
                TweakError::Vanishes {
                    faces: faces.iter().copied().collect(),
                    edge: None,
                    reason: NO_MEETING,
                }
            };
            if corners == Corners::Split && faces.len() >= 4 {
                to_split.push((vertex, was, error));
                continue;
            }
            // Refused as before: by the first vertex whose faces do not
            // meet, a deferred one included.
            return Err(match to_split.into_iter().next() {
                Some((_, _, first)) => first,
                None => error,
            });
        };
        if let Some(slot) = made.vertices.get_mut(index) {
            slot.point = point;
        }
        moved_vertices.insert(vertex);
    }

    // A vertex at the end of an edge along a border of a new freeform
    // surface (`iso`), where a face crosses that surface: on that border.
    // Found where the surfaces meet, it is as good as the tangent faces
    // let it be along the border, which is not good enough for the border.
    for (id, along) in iso {
        let Some(Surface::Nurbs(new)) = surfaces.get(&along.face) else {
            continue;
        };
        let Some(edge) = body.edge(*id) else {
            continue;
        };
        for vertex in [edge.start, edge.end] {
            if !moved_vertices.contains(&vertex) {
                continue;
            }
            let Some(faces) = around.get(vertex.0 as usize) else {
                continue;
            };
            let Some(point) = made.vertex(vertex).map(|v| v.point) else {
                continue;
            };
            let crossing: Vec<&Surface> = faces
                .iter()
                .filter(|f| **f != along.face)
                .filter_map(|f| made.face(*f).map(|d| &d.surface))
                .collect();
            if let Some(on) = onto_border(new, along, point, &crossing, tol)
                && let Some(slot) = made.vertices.get_mut(vertex.0 as usize)
            {
                slot.point = on;
            }
        }
    }

    // The vertices the new surfaces no longer meet in, split (the shell's
    // hollow only): the old edges end in the new vertices, the new edges
    // are found again below like the others.
    let mut new_edges: Vec<EdgeId> = Vec::new();
    for (vertex, was, error) in to_split {
        match split::split(body, &mut made, vertex, was, tol) {
            Some(done) => {
                moved_vertices.extend(done.vertices);
                new_edges.extend(done.edges);
            }
            None => return Err(error),
        }
    }
    // Which faces each edge lies between, the new edges' among them (the
    // old body's when nothing was split).
    let uses_made: std::borrow::Cow<'_, [Vec<FaceId>]> = if new_edges.is_empty() {
        std::borrow::Cow::Borrowed(&uses)
    } else {
        std::borrow::Cow::Owned(edge_faces(&made))
    };

    // An edge between faces that keep their surfaces, which ends in a
    // vertex that moved and lies along a border of one of them, does not
    // move: only its end goes elsewhere along it. Its moved end is put on
    // that border where the faces round it cross it; a line or a circle is
    // then made again through its ends as one (`remake_unfitted`), and only
    // a curve that would be fitted becomes the border's iso-curve between
    // its new ends, exactly on the face as it was (below; along the border
    // within the share of the tolerance a fit is held to;
    // `crate::offset_normal::still_iso_edges`, which an offset gives for
    // the faces it continued: their borders before that are no borders of
    // `body`). Found after the split, from the edges' ends as they are in
    // `made` and the faces round them there: an end at a split vertex is
    // one of the new vertices. (No caller splits and keeps a face today:
    // the shell, the one that splits, moves every face of its hollow; read
    // from `made` all the same, so that an edge ending at a split vertex is
    // not left to be fitted.) The surface of such a face is its surface in
    // `body`.
    let mut iso = iso.clone();
    for (index, edge) in made.edges.iter().enumerate().take(body.edges.len()) {
        if !(moved_vertices.contains(&edge.start) || moved_vertices.contains(&edge.end)) {
            continue;
        }
        let id = EdgeId(index_u32(index));
        let Some(faces) = uses_made.get(index) else {
            continue;
        };
        if iso.contains_key(&id)
            || faces.is_empty()
            || faces.iter().any(|face| surfaces.contains_key(face))
        {
            continue;
        }
        if let Some(along) = faces
            .iter()
            .find_map(|face| crate::offset_normal::edge_border(body, *face, id, EDGE_SHARE * tol))
        {
            iso.insert(id, along);
        }
    }
    // The surface an edge of `iso` is the iso-curve of: the new one of its
    // face, or the one it keeps.
    let iso_surface = |face: FaceId| {
        surfaces
            .get(&face)
            .or_else(|| body.face(face).map(|data| &data.surface))
    };
    // The faces round each vertex of `made`.
    let mut around_made: Vec<BTreeSet<FaceId>> = vec![BTreeSet::new(); made.vertices.len()];
    for (index, edge) in made.edges.iter().enumerate() {
        for vertex in [edge.start, edge.end] {
            if let (Some(list), Some(faces)) =
                (around_made.get_mut(vertex.0 as usize), uses_made.get(index))
            {
                list.extend(faces.iter().copied());
            }
        }
    }
    for (id, along) in &iso {
        if surfaces.contains_key(&along.face) {
            continue;
        }
        let Some(Surface::Nurbs(new)) = iso_surface(along.face) else {
            continue;
        };
        let Some(edge) = made.edge(*id) else {
            continue;
        };
        for vertex in [edge.start, edge.end] {
            if !moved_vertices.contains(&vertex) {
                continue;
            }
            let Some(faces) = around_made.get(vertex.0 as usize) else {
                continue;
            };
            let Some(point) = made.vertex(vertex).map(|v| v.point) else {
                continue;
            };
            let crossing: Vec<&Surface> = faces
                .iter()
                .filter(|f| **f != along.face)
                .filter_map(|f| made.face(*f).map(|d| &d.surface))
                .collect();
            if let Some(on) = onto_border(new, along, point, &crossing, tol)
                && let Some(slot) = made.vertices.get_mut(vertex.0 as usize)
            {
                slot.point = on;
            }
        }
    }

    // Every edge that ends in a vertex that moved, on its two surfaces.
    let mut remade: BTreeSet<EdgeId> = BTreeSet::new();
    for (index, edge) in body.edges.iter().enumerate() {
        // Its ends as they are now: an end at a split vertex is one of the
        // new vertices.
        let (Some(start_id), Some(end_id)) = (
            made.edges.get(index).map(|e| e.start),
            made.edges.get(index).map(|e| e.end),
        ) else {
            continue;
        };
        if !(moved_vertices.contains(&start_id) || moved_vertices.contains(&end_id)) {
            continue;
        }
        if craft3d_geom::allowance::pass() {
            return Err(spent());
        }
        let id = EdgeId(index_u32(index));
        let faces: Vec<FaceId> = uses.get(index).cloned().unwrap_or_default();
        let on: Vec<&Surface> = faces
            .iter()
            .filter_map(|face| made.face(*face).map(|f| &f.surface))
            .collect();
        let moved = faces.iter().any(|face| surfaces.contains_key(face));
        let (Some(start), Some(end)) = (
            made.vertex(start_id).map(|v| v.point),
            made.vertex(end_id).map(|v| v.point),
        ) else {
            continue;
        };
        if let Some(motion) = rigid
            && carried(&faces)
        {
            // The image of the edge, with the same parameters.
            let curve = edge.curve.transformed(motion).map_err(KernelError::from)?;
            if let Some(slot) = made.edges.get_mut(index) {
                slot.curve = curve;
            }
            remade.insert(id);
            continue;
        }
        let closed = edge.start == edge.end;
        let fail = |vanishes: bool, reason: &'static str| {
            let (faces, edge) = (faces.clone(), Some(id));
            if vanishes {
                TweakError::Vanishes {
                    faces,
                    edge,
                    reason,
                }
            } else {
                TweakError::Unsupported {
                    faces,
                    edge,
                    reason,
                }
            }
        };
        // An edge along a border of a face that keeps its surface, whose
        // ends have changed places along that border, has turned round.
        // (Along a border of a face with a new surface its old ends do not
        // lie on the new border, and nothing is said.)
        let turned = |along: &Iso, surface: &craft3d_geom::NurbsSurface| {
            !closed && iso_turned(surface, along, &edge.curve, edge.range, start, end, tol)
        };
        // Along a border of a face with a new surface: that border of it.
        if let Some(along) = iso.get(&id).filter(|a| surfaces.contains_key(&a.face))
            && let Some(Surface::Nurbs(new)) = surfaces.get(&along.face)
            && let Some((curve, range)) = iso_curve(new, along, start, end, closed, tol)
            && lies_on(&curve, range, &on, tol)
        {
            if let Some(slot) = made.edges.get_mut(index) {
                slot.curve = curve;
                slot.range = range;
            }
            remade.insert(id);
            continue;
        }
        let tangent = tangent_along(body, &faces, &edge.curve, edge.range);
        let made_again = match remake_unfitted(
            &edge.curve,
            edge.range,
            start,
            end,
            closed,
            &on,
            moved,
            tolerance,
        ) {
            // Where it would be fitted, an edge between faces that keep
            // their surfaces along a border of one of them is that border
            // between its new ends, held to the share of the tolerance a
            // fit is held to; a line, a circle or the old curve between
            // new ends stays what `remake_unfitted` makes it.
            Remade::Fit => match iso
                .get(&id)
                .filter(|a| !surfaces.contains_key(&a.face))
                .and_then(|along| match iso_surface(along.face) {
                    Some(Surface::Nurbs(new)) if turned(along, new) => Some(None),
                    Some(Surface::Nurbs(new)) => {
                        iso_curve(new, along, start, end, closed, EDGE_SHARE * tol)
                            .filter(|(curve, range)| lies_on(curve, *range, &on, EDGE_SHARE * tol))
                            .map(Some)
                    }
                    _ => None,
                }) {
                Some(Some((curve, range))) => Remade::Edge(curve, range),
                // Its ends changed places along the border it runs along.
                Some(None) => Remade::Gone,
                None => fitted(
                    &edge.curve,
                    edge.range,
                    start,
                    end,
                    &on,
                    tangent,
                    faces.iter().any(|f| rebuilt.contains(f)),
                    tolerance,
                ),
            },
            other => other,
        };
        match made_again {
            Remade::Edge(curve, range) => {
                if let Some(slot) = made.edges.get_mut(index) {
                    slot.curve = curve;
                    slot.range = range;
                }
            }
            Remade::Gone => return Err(fail(true, EDGE_GONE)),
            // Far enough, a line or a circle leaves its surfaces because
            // they no longer meet as they did; near enough it never does
            // when the kind is one this builds. The caller tells the two
            // apart by trying a smaller move.
            Remade::OffSurface => return Err(fail(true, OFF_SURFACE)),
            // A search cut short by the work bound says so, not that the
            // edge was lost.
            Remade::Lost if craft3d_geom::allowance::exhausted() => return Err(spent()),
            // `Fit` was answered by the fit above.
            Remade::Lost | Remade::Fit => return Err(fail(true, LOST)),
        }
        remade.insert(id);
    }
    // The new edges of split vertices: between two faces that met only in
    // the vertex, from one new vertex to another; a line where that lies on
    // both faces, else found where they meet from the line as the guess.
    for id in &new_edges {
        if craft3d_geom::allowance::pass() {
            return Err(spent());
        }
        let faces: Vec<FaceId> = uses_made.get(id.0 as usize).cloned().unwrap_or_default();
        let fail = |reason: &'static str| TweakError::Vanishes {
            faces: faces.clone(),
            edge: None,
            reason,
        };
        let on: Vec<&Surface> = faces
            .iter()
            .filter_map(|face| made.face(*face).map(|f| &f.surface))
            .collect();
        let Some(edge) = made.edge(*id) else {
            return Err(fail(LOST));
        };
        let (Some(start), Some(end)) = (
            made.vertex(edge.start).map(|v| v.point),
            made.vertex(edge.end).map(|v| v.point),
        ) else {
            return Err(fail(LOST));
        };
        let (guess, range) = (edge.curve.clone(), edge.range);
        let found = if lies_on(&guess, range, &on, tol) {
            Remade::Edge(guess, range)
        } else {
            fitted(&guess, range, start, end, &on, false, false, tolerance)
        };
        match found {
            Remade::Edge(curve, range) => {
                if let Some(slot) = made.edges.get_mut(id.0 as usize) {
                    slot.curve = curve;
                    slot.range = range;
                }
            }
            Remade::Lost if craft3d_geom::allowance::exhausted() => return Err(spent()),
            Remade::Gone => return Err(fail(EDGE_GONE)),
            Remade::OffSurface | Remade::Lost | Remade::Fit => return Err(fail(LOST)),
        }
        remade.insert(*id);
    }

    // What a face next to a moved edge has to be still.
    for face in faces_with(&uses_made, &remade) {
        let (Some(before), Some(after)) = (body.face(face), made.face(face)) else {
            continue;
        };
        let gone = |reason: &'static str| TweakError::Vanishes {
            faces: vec![face],
            edge: None,
            reason,
        };
        if let (Surface::Plane(was), Surface::Plane(is)) = (&before.surface, &after.surface) {
            for loop_ in &before.loops {
                let area_before = signed_area(was.frame(), &loop_points(body, *loop_));
                let area_after = signed_area(is.frame(), &loop_points(&made, *loop_));
                if area_before * area_after <= 0.0 && area_before != 0.0 {
                    return Err(gone(TURNED));
                }
            }
            if outline_is_simple(&made, face, tolerance) == Some(false)
                && outline_is_simple(body, face, tolerance) == Some(true)
            {
                return Err(gone(OUTLINE));
            }
        }
        if let (Some(was), Some(is)) = (heights(body, face), heights(&made, face)) {
            for (i, (a0, a1)) in was.iter().zip(&is).enumerate() {
                for (b0, b1) in was.iter().zip(&is).skip(i + 1) {
                    let (before, after) = (a0 - b0, a1 - b1);
                    if before.abs() > tol && (after.abs() <= tol || before * after < 0.0) {
                        return Err(gone(RIMS));
                    }
                }
            }
        }
    }

    // Two edges of one face that meet where they did not: a face that
    // moved reached or passed the face that ends a neighbour of it. Before
    // the checker: where the edges lie on each other it would refuse the
    // body as invalid, with nothing to say of how far the face can go.
    let Ok(crossed) = passes::crossed(body, &made, &uses_made, &remade, tolerance) else {
        return Err(spent());
    };
    if let Some((face, edge, other)) = crossed {
        let mut faces = vec![face];
        for id in [edge, other] {
            for next in uses_made.get(id.0 as usize).into_iter().flatten() {
                if !faces.contains(next) {
                    faces.push(*next);
                }
            }
        }
        return Err(TweakError::Vanishes {
            faces,
            edge: Some(edge),
            reason: CROSSED,
        });
    }

    let defects = craft3d_topo::check_remembered(&made, tolerance);
    if !defects.is_empty() {
        return Err(TweakError::Kernel(KernelError::InvalidBody {
            operation,
            defects: defects.into_iter().take(crate::MAX_REPORTED).collect(),
        }));
    }
    if craft3d_geom::allowance::pass() {
        return Err(spent());
    }
    if let Some((face, _)) = buried(&made, &uses_made, &remade, tolerance) {
        return Err(TweakError::Vanishes {
            faces: vec![face],
            edge: None,
            reason: THROUGH,
        });
    }
    // What the faces that moved went through on their way (`passes`): the
    // same question beside the edges at a vertex one of them passed, and
    // beside every stretch of an edge made again that goes through a face
    // not its own; and where a face far from them lay in their way,
    // whether the body still encloses what it did. Counted against the
    // operation's work bound, as `crossed` is: a check that cannot end
    // within it refuses the operation as needing too much work, and never
    // stops without a word.
    if !surfaces.is_empty() {
        let layout = passes::Layout::of(&made);
        let (Ok(swept), Ok(pierced)) = (
            passes::swept(body, &made, &layout, surfaces, tolerance),
            passes::pierced(&made, &layout, &uses_made, &remade, tolerance),
        ) else {
            return Err(spent());
        };
        // Where to ask, and the face that was passed or gone through.
        let mut ask: BTreeMap<EdgeId, Vec<f64>> = BTreeMap::new();
        let mut by: BTreeMap<EdgeId, FaceId> = BTreeMap::new();
        for (id, face) in &swept.edges {
            if remade.contains(id) {
                continue;
            }
            if let Some(edge) = made.edge(*id) {
                ask.insert(*id, vec![0.5 * (edge.range.0 + edge.range.1)]);
                by.insert(*id, *face);
            }
        }
        for (id, (middles, face)) in pierced {
            ask.insert(id, middles);
            by.insert(id, face);
        }
        if !ask.is_empty() {
            if craft3d_geom::allowance::pass() {
                return Err(spent());
            }
            if let Some((face, edge)) = buried_at(&made, &uses_made, &ask, tolerance) {
                let mut faces: Vec<FaceId> = by.get(&edge).copied().into_iter().collect();
                if !faces.contains(&face) {
                    faces.push(face);
                }
                // Found beside a vertex a face passed that may not be the
                // first thing in that face's way (a cone's point has no
                // vertex): a smaller move that passes these checks can
                // still be through it, so the largest that fits is not
                // looked for, and none is said.
                if swept.edges.contains_key(&edge) && !swept.first {
                    return Err(TweakError::Unsupported {
                        faces,
                        edge: None,
                        reason: THROUGH_UNSAID,
                    });
                }
                return Err(TweakError::Vanishes {
                    faces,
                    edge: None,
                    reason: THROUGH,
                });
            }
        }
        if swept.near && passes::inside_out(body, &made, tolerance) {
            return Err(TweakError::Unsupported {
                faces: surfaces.keys().copied().take(crate::MAX_REPORTED).collect(),
                edge: None,
                reason: INSIDE_OUT,
            });
        }
    }
    Ok(made)
}

/// The work bound of the operation was reached ([`craft3d_geom::allowance`]).
fn spent() -> TweakError {
    TweakError::Unsupported {
        faces: Vec::new(),
        edge: None,
        reason: crate::offset_face::SPENT,
    }
}

/// How far inside a face from its edge, and how far off the face, the
/// points lie that are asked about, in tolerances.
const INSIDE_FACE: f64 = 8.0;
const OFF_FACE: f64 = 4.0;
/// How far from a face a point has to lie for the classification to place
/// it, in tolerances (`crate::boolean::contains_points` says nothing of a
/// point within twice the tolerance of the boundary), and the furthest
/// from an edge a point is asked about.
const CLEAR_OF_FACE: f64 = 2.0;
const MOST_INSIDE_FACE: f64 = 1024.0;

/// How far from an edge, along a face, the point just outside the face
/// and the point just inside it are asked about ([`buried`]), in
/// tolerances, given the outward normal of the face across the edge there
/// as its parts `along` the first face (into it, away from the edge) and
/// `off` it (along its outward normal).
///
/// At [`INSIDE_FACE`] and [`OFF_FACE`] the point inside lies behind the
/// face across a convex edge only where the two meet at more than about
/// 27 degrees, and the classification places it only above about 41; below
/// about 13.6 degrees it lies beyond that face by more than the
/// classification's band, in the open, and was found outside: every
/// operation at such an edge was refused as passing through the body (the
/// groove of a cylinder that leaves a face at 8.5 degrees, a face moved
/// within its own plane beside it: the move hunt's case 676, 2026-11-14).
/// Where a point lies that clearly on the wrong side of the face across
/// the edge (the one inside at a convex edge, the one outside at a concave
/// one), it is asked about further from the edge instead, where it is as
/// far behind (or before) that face as it is off its own; where it does
/// not, both points lie where they always did. `None` for a point that
/// would lie further than [`MOST_INSIDE_FACE`] from the edge: not asked.
fn sharp_places(along: f64, off: f64) -> (Option<f64>, Option<f64>) {
    // Convex: the face across the edge looks away from this one.
    let inside = if along < 0.0 && INSIDE_FACE * along - OFF_FACE * off >= CLEAR_OF_FACE {
        Some(OFF_FACE * (off - 1.0) / along)
    } else {
        Some(INSIDE_FACE)
    };
    // Concave: it looks across this one.
    let outside = if along > 0.0 && INSIDE_FACE * along + OFF_FACE * off <= -CLEAR_OF_FACE {
        Some(OFF_FACE * (1.0 - off) / along)
    } else {
        Some(INSIDE_FACE)
    };
    let kept = |at: Option<f64>| at.filter(|a| a.is_finite() && *a <= MOST_INSIDE_FACE);
    (kept(outside), kept(inside))
}

/// A face next to an edge of `remade` that is not skin of `made` any
/// more: just outside it there is material, or just inside it there is
/// none, which is what a body that passes through itself looks like next
/// to the faces that moved. The question is put to the classification of
/// the booleans (a ray from the point), at a point beside the middle of
/// every such edge; a point it cannot tell for says nothing. The face,
/// and the edge it was asked beside.
fn buried(
    made: &Body,
    uses: &[Vec<FaceId>],
    remade: &BTreeSet<EdgeId>,
    tolerance: &Tolerance,
) -> Option<(FaceId, EdgeId)> {
    let middles: BTreeMap<EdgeId, Vec<f64>> = remade
        .iter()
        .filter_map(|id| {
            let edge = made.edge(*id)?;
            Some((*id, vec![0.5 * (edge.range.0 + edge.range.1)]))
        })
        .collect();
    buried_at(made, uses, &middles, tolerance)
}

/// [`buried`], asked beside each edge of `places` at the parameters given
/// with it.
fn buried_at(
    made: &Body,
    uses: &[Vec<FaceId>],
    places: &BTreeMap<EdgeId, Vec<f64>>,
    tolerance: &Tolerance,
) -> Option<(FaceId, EdgeId)> {
    let tol = tolerance.linear;
    let mut asked: Vec<(FaceId, EdgeId, bool)> = Vec::new();
    let mut points: Vec<Vec3> = Vec::new();
    let edges: BTreeSet<EdgeId> = places.keys().copied().collect();
    for id in faces_with(uses, &edges) {
        let Some(face) = made.face(id) else {
            continue;
        };
        let outward = |at: Vec3| {
            let ((u, v), normal) = face.nearest(at).ok()??;
            Some((face.surface.eval(u, v), normal))
        };
        for coedge in face
            .loops
            .iter()
            .filter_map(|l| made.loop_(*l))
            .flat_map(|l| &l.coedges)
        {
            let Some(at) = places.get(&coedge.edge) else {
                continue;
            };
            let Some(edge) = made.edge(coedge.edge) else {
                continue;
            };
            for t in at {
                let (middle, along) = edge.curve.derivatives(*t);
                let along = if coedge.reversed { -along } else { along };
                let Some((_, normal)) = outward(middle) else {
                    continue;
                };
                // The face lies to the left of its edges, seen from outside.
                let Some(into) = normal.cross(along).normalized() else {
                    continue;
                };
                // How far from the edge each of the two points lies: at
                // an edge sharper than the usual place allows, further.
                let beyond = uses
                    .get(coedge.edge.0 as usize)
                    .and_then(|faces| match faces.as_slice() {
                        [a, b] if *a == id && *b != id => Some(*b),
                        [a, b] if *b == id && *a != id => Some(*a),
                        _ => None,
                    })
                    .and_then(|other| made.face(other))
                    .and_then(|other| other.outward(middle));
                let (out_at, in_at) = match beyond {
                    Some(other) => sharp_places(other.dot(into), other.dot(normal)),
                    None => (Some(INSIDE_FACE), Some(INSIDE_FACE)),
                };
                let beside = |at: Option<f64>| outward(middle + into * (at? * tol));
                let outside = beside(out_at);
                let inside = if in_at == out_at {
                    outside
                } else {
                    beside(in_at)
                };
                if let Some((foot, normal)) = outside {
                    points.push(foot + normal * (OFF_FACE * tol));
                    asked.push((id, coedge.edge, false));
                }
                if let Some((foot, normal)) = inside {
                    points.push(foot - normal * (OFF_FACE * tol));
                    asked.push((id, coedge.edge, true));
                }
            }
        }
    }
    let found = crate::boolean::contains_points(made, &points, tolerance).ok()?;
    asked
        .iter()
        .zip(found)
        .find(|((_, _, expected), inside)| inside.is_some_and(|inside| inside != *expected))
        .map(|((face, edge, _), _)| (*face, *edge))
}

/// The vertices on the loops of `face`, ascending, each once.
pub(crate) fn face_vertices(body: &Body, face: FaceId) -> BTreeSet<VertexId> {
    body.face(face)
        .into_iter()
        .flat_map(|f| &f.loops)
        .filter_map(|l| body.loop_(*l))
        .flat_map(|l| &l.coedges)
        .filter_map(|c| body.edge(c.edge))
        .flat_map(|e| [e.start, e.end])
        .collect()
}

#[cfg(test)]
mod sharp_tests {
    use super::*;
    use craft3d_math::Portable;

    /// The two faces of an edge that meet at `degrees` inside the material
    /// (convex below 180): the outward normal of the face across the edge,
    /// as its parts along the first face and off it.
    fn across(degrees: f64) -> (f64, f64) {
        let (sine, cosine) = degrees.to_radians().psin_cos();
        (-sine, -cosine)
    }

    /// Where the point just inside a face and the point just outside it
    /// lie from the edge, by the angle the faces meet at: the usual 8
    /// tolerances wherever the point is not clearly on the wrong side of
    /// the face across the edge (from about 13.7 degrees on, either way
    /// round), further where it is, and there four tolerances clear of
    /// that face; not asked where that is past the cap.
    #[test]
    fn points_beside_a_sharp_edge_are_asked_about_where_the_wedge_holds_them() {
        for degrees in [
            14.0, 27.0, 41.0, 90.0, 135.0, 179.0, 181.0, 270.0, 330.0, 346.0,
        ] {
            let (along, off) = across(degrees);
            assert_eq!(
                sharp_places(along, off),
                (Some(INSIDE_FACE), Some(INSIDE_FACE)),
                "{degrees} degrees"
            );
        }
        // Convex and sharp: the point inside moves out, the one outside
        // stays.
        for degrees in [1.0_f64, 5.0, 8.5, 12.0, 13.5] {
            let (along, off) = across(degrees);
            let (outside, inside) = sharp_places(along, off);
            assert_eq!(outside, Some(INSIDE_FACE), "{degrees} degrees");
            let at = inside.unwrap();
            assert!(at > INSIDE_FACE, "{degrees} degrees: {at}");
            // Behind the face across the edge by as much as it is under
            // its own.
            let behind = at * along - OFF_FACE * off;
            assert!(
                (behind + OFF_FACE).abs() < 1e-9,
                "{degrees} degrees: {behind}"
            );
        }
        // Concave and sharp (the faces fold back to within 13.6 degrees of
        // each other, air between them): the point outside moves out.
        for degrees in [346.5_f64, 352.0, 359.0] {
            let (along, off) = across(degrees);
            let (outside, inside) = sharp_places(along, off);
            assert_eq!(inside, Some(INSIDE_FACE), "{degrees} degrees");
            let at = outside.unwrap();
            assert!(at > INSIDE_FACE, "{degrees} degrees: {at}");
            let before = at * along + OFF_FACE * off;
            assert!(
                (before - OFF_FACE).abs() < 1e-9,
                "{degrees} degrees: {before}"
            );
        }
        // Past the cap: not asked.
        let (along, off) = across(0.1);
        assert_eq!(sharp_places(along, off), (Some(INSIDE_FACE), None));
    }
}

#[cfg(test)]
mod seam_tests {
    use super::*;
    use craft3d_geom::Cylinder;

    /// A vertex where a plane tangent to a cylinder (a tunnel's wall and
    /// its arch) meets a third face is put on the pair's seam, not merely
    /// within the tolerance of both: from a point 5 µm along the seam
    /// (within 2.6e-7 mm of the cylinder, inside a tolerance of 1e-6 mm)
    /// it moved nowhere before (2026-11-01, the fuel tank's shell at its
    /// default tolerance), and the edge between two such vertices dipped
    /// past the cylinder's border.
    #[test]
    fn a_vertex_where_a_plane_tangent_to_a_cylinder_meets_a_third_face_lies_on_their_seam() {
        let (tol, r, z0) = (1e-6, 48.8, 18.4);
        let frame = |origin: Vec3, z: Vec3| Frame::new(origin, z, Vec3::Z).unwrap();
        let cylinder =
            Surface::Cylinder(Cylinder::new(frame(Vec3::new(0.0, 0.0, z0), Vec3::X), r).unwrap());
        let wall = Surface::Plane(Plane::new(frame(Vec3::new(0.0, r, 0.0), Vec3::Y)));
        let end = Surface::Plane(Plane::new(
            Frame::new(Vec3::new(100.0, 0.0, z0), Vec3::new(1.0, 0.0, 0.3), Vec3::Y).unwrap(),
        ));
        // 9 µm along: the pair's normals 1.8e-4 apart, beyond `TANGENT`.
        for dz in [5e-3, -4e-3, 9e-3, 1e-7] {
            let start = Vec3::new(100.0, r, z0 + dz);
            let found = meet(&[&wall, &cylinder, &end], start, tol).unwrap();
            assert!(
                (found.z - z0).abs() <= 1e-9,
                "from {dz:e}: {found:?} is {:e} along the seam",
                found.z - z0
            );
            for s in [&wall, &cylinder, &end] {
                let (d, _) = distance(s, found).unwrap();
                assert!(d.abs() <= tol, "{d:e} off a surface");
            }
        }
    }

    /// A corner where a plane tangent to a cylinder meets one plane given
    /// twice (two rims that continue each other) keeps its place: the
    /// rims are no pair, and the plane and cylinder's curve with one rim
    /// is no curve to slide along (2026-11-01: the exact corner of a
    /// channel's plate at scale 1e-3 was moved 2.6e-8 mm, 26 tolerances,
    /// along the seam, past the traced back edge).
    #[test]
    fn a_corner_on_one_plane_given_twice_keeps_its_place() {
        // The channel's plate at scale 1e-3, at the middle, as the plate
        // asks for its corner: the cylinder's skin (0.3 less half of 0.05,
        // about the axis at y = z = 0.3), the plane's (z = 0.025) and the
        // two rims at the end, x = 0.8, one of them a rounding off it.
        let v = Vec3::new;
        let cylinder = Surface::Cylinder(
            Cylinder::new(
                Frame::new(v(0.0, 3e-4, 3e-4), Vec3::X, -Vec3::Y).unwrap(),
                0.00027499999999999996,
            )
            .unwrap(),
        );
        let floor = Surface::Plane(Plane::new(
            Frame::new(v(4e-4, 7.5e-4, 2.5e-5), -Vec3::Z, Vec3::Y).unwrap(),
        ));
        let rim = Surface::Plane(Plane::new(
            Frame::new(
                v(8e-4, -7.111007552289563e-6, 0.00040633827645874973),
                v(-1.0, 4.176665418462424e-16, 4.176665418462421e-16),
                v(
                    4.1173070568495747e-32,
                    0.7071067811865472,
                    -0.7071067811865477,
                ),
            )
            .unwrap(),
        ));
        let next = Surface::Plane(Plane::new(
            Frame::new(
                v(8e-4, 2e-4, -2.5e-5),
                -Vec3::X,
                v(0.0, 0.9989685402102996, 0.04540766091864999),
            )
            .unwrap(),
        ));
        let (corner, tol) = (v(8e-4, 3e-4, 2.5e-5), 5e-10);
        let start = v(0.0008000000000000001, 0.0003, 2.500000000000001e-5);
        let found = meet(&[&cylinder, &floor, &rim, &next], start, tol).unwrap();
        assert!(
            found.distance(corner) <= 1e-2 * tol,
            "{found:?} moved {:e} from the corner",
            found.distance(corner)
        );
    }
}

#[cfg(test)]
mod similarity_tests {
    #![allow(clippy::unwrap_used)]

    use super::{Remade, Similarity, fitted};
    use craft3d_geom::{Curve3, Cylinder, Ellipse, Line, NurbsSurface, Surface};
    use craft3d_math::{Frame, Tolerance, Vec3};

    /// The kernel set's H-02 mitre, shelled 2: the ellipse where two rounds
    /// of 2.4 meet (one along x, one along y, axes crossing at (37.6, 2.4,
    /// 2.4)), and its new ends on the rounds moved in to 0.4.
    fn mitre() -> (Curve3, (f64, f64), Vec3, Vec3) {
        let centre = Vec3::new(37.6, 2.4, 2.4);
        let frame =
            Frame::new(centre, Vec3::new(1.0, 1.0, 0.0), Vec3::new(1.0, -1.0, 0.0)).unwrap();
        let old = Curve3::Ellipse(Ellipse::new(frame, 2.4 * 2.0_f64.sqrt(), 2.4).unwrap());
        let range = (0.0, std::f64::consts::FRAC_PI_2);
        (
            old,
            range,
            Vec3::new(38.0, 2.0, 2.4),
            Vec3::new(37.6, 2.4, 2.0),
        )
    }

    /// The round along x moved in to 0.4: the cylinder, or its rational
    /// twin (the quarter facing −y to −z, x from 30 to 45).
    fn along_x(spline: bool) -> Surface {
        let axis = Vec3::new(0.0, 2.4, 2.4);
        if !spline {
            let frame = Frame::new(axis, Vec3::X, Vec3::Y).unwrap();
            return Surface::Cylinder(Cylinder::new(frame, 0.4).unwrap());
        }
        let w = 0.5 * 2.0_f64.sqrt();
        let (y0, z0) = (2.4, 2.4);
        let arc = [(y0 - 0.4, z0), (y0 - 0.4, z0 - 0.4), (y0, z0 - 0.4)];
        let mut ctrl = Vec::new();
        for x in [30.0, 45.0] {
            for (y, z) in arc {
                ctrl.push(Vec3::new(x, y, z));
            }
        }
        let weights = vec![1.0, w, 1.0, 1.0, w, 1.0];
        let knots_u = vec![0.0, 0.0, 1.0, 1.0];
        let knots_v = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        Surface::Nurbs(NurbsSurface::new(1, 2, knots_u, knots_v, 2, 3, ctrl, weights).unwrap())
    }

    /// The retry from the chords' similarity runs only where no surface of
    /// the edge is freeform (the review of 2026-11-14): with a spline twin
    /// of a round the edge is left to the old path (continuing the freeform
    /// faces and trying again), exactly as before the retry existed.
    #[test]
    fn the_retry_leaves_an_edge_with_a_freeform_face_to_the_old_path() {
        let tol = Tolerance::new(1e-6, 1e-9).unwrap();
        let (old, range, start, end) = mitre();
        let along_y = Surface::Cylinder(
            Cylinder::new(
                Frame::new(Vec3::new(37.6, 0.0, 2.4), Vec3::Y, Vec3::X).unwrap(),
                0.4,
            )
            .unwrap(),
        );
        // Both rounds closed forms: lost by the first try, found by the
        // retry, on both rounds.
        let a = along_x(false);
        match fitted(&old, range, start, end, &[&a, &along_y], false, false, &tol) {
            Remade::Edge(curve, (t0, t1)) => {
                assert!(curve.eval(t0).distance(start) <= 1e-9);
                assert!(curve.eval(t1).distance(end) <= 1e-9);
                let mid = curve.eval(0.5 * (t0 + t1));
                // In the mitre plane x + y = 40, on the round along y.
                assert!((mid.x + mid.y - 40.0).abs() <= 1e-6, "{mid:?}");
            }
            _ => panic!("the retry did not find the mitre"),
        }
        // With the round along x a spline: what the first try gives.
        let b = along_x(true);
        assert!(matches!(
            fitted(&old, range, start, end, &[&b, &along_y], false, false, &tol),
            Remade::Gone | Remade::Lost
        ));
    }

    fn chord(a: Vec3, b: Vec3) -> Curve3 {
        Curve3::Line(Line::through(a, b).unwrap())
    }

    /// The old chord's ends go to the new ends, directions turn the
    /// shortest way and keep their length (times the scale), and a chord
    /// that turned round or shrank to nothing gives no guess.
    #[test]
    fn the_similarity_takes_the_old_chord_onto_the_new_one() {
        let (a, b) = (Vec3::new(40.0, 0.0, 2.4), Vec3::new(37.6, 2.4, 0.0));
        let old = chord(a, b);
        let range = (0.0, a.distance(b));
        for (start, end) in [
            (Vec3::new(38.0, 2.0, 2.4), Vec3::new(37.6, 2.4, 2.0)),
            (Vec3::new(1.0, -3.0, 7.0), Vec3::new(-2.0, -1.5, 5.0)),
        ] {
            let s = Similarity::onto(&old, range, start, end, 1e-9).unwrap();
            assert!(s.point(a).distance(start) <= 1e-12);
            assert!(s.point(b).distance(end) <= 1e-12);
            let v = Vec3::new(0.3, -0.7, 0.2);
            assert!((s.turned(v).length() - v.length()).abs() <= 1e-12);
            let mid = a + (b - a) * 0.5;
            assert!(s.point(mid).distance(start + (end - start) * 0.5) <= 1e-12);
        }
        // Shrunk about the point where the chord's lines meet: exact.
        let s = Similarity::onto(
            &old,
            range,
            Vec3::new(38.0, 2.0, 2.4),
            Vec3::new(37.6, 2.4, 2.0),
            1e-9,
        )
        .unwrap();
        assert!((s.scale - 1.0 / 6.0).abs() <= 1e-12);
        // Turned round, or nothing left of it: no guess.
        assert!(Similarity::onto(&old, range, b, a, 1e-9).is_none());
        assert!(Similarity::onto(&old, range, a, a, 1e-9).is_none());
        assert!(Similarity::onto(&old, range, a, Vec3::new(f64::NAN, 0.0, 0.0), 1e-9).is_none());
    }
}
