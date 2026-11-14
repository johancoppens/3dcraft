//! Whether the loops of a face bound a region of its surface, read in the
//! surface's parameters (`docs/design/names.md` has the faces; the method
//! is in `docs/design/text.md` › *Loops that bound a region*).
//!
//! **What must hold.** A face lies to the left of its loops seen from
//! outside, so in the surface's `(u, v)`, whose orientation is the
//! outward one unless the face is reversed, a face that is a disc with
//! holes has exactly one loop that runs counter-clockwise (its outer
//! loop, enclosing the most area) and every other one clockwise (holes),
//! and no loop crosses itself or another. Holes inside the outer loop are
//! held on planes by [`crate::stranded_loops`]; here, on every other
//! surface, every point of each hole clear of the other loops must be wound
//! once by the outer loop and by no other hole.
//!
//! **What is not held.** Faces this test cannot chart without doubt are
//! passed, the other checks and the tessellator saying what is wrong
//! there: a loop that winds around a closed surface (a band of a
//! cylinder, a torus's handle), a face on a surface closed both ways (a
//! torus: any loop there bounds a region on one side or the other), a
//! loop through or near a pole or an apex (a sphere's pole, a cone's
//! apex, a freeform surface's collapsed border), a freeform surface that
//! closes on itself (its seam is not a period of the parameters), a point
//! that does not project back onto the surface, a loop whose area is not
//! larger than its polyline's error allows to be told from nothing, a
//! face with more than [`MOST_PER_FACE`] points, and a face with a spline
//! edge whose steps are not shown to hide no turn (below) within
//! [`MOST_SPLINE_STEPS`] and [`MOST_PROOF_WORK`]. On a sphere loops that
//! all run clockwise are the sphere less some caps, which is a region.
//!
//! **How.** Each edge is followed in a few steps (a line in one on a
//! plane, else in two; a circle or an ellipse in steps of at most π/8; a
//! spline in eight per edge, or in as many more as it takes for every half
//! step to be shown to hide no turn from the walk, [`spline_steps`]: about
//! the axis of a cylinder, a cone, a sphere or a torus it moves less than a
//! quarter turn, about a torus's tube as well, so that a helix of many
//! turns as one edge is followed round every turn; and in the chart of a
//! plane or of one of those it runs one way, so that the chord from one
//! point to the next goes round no point the edge does not, and a spiral
//! of many turns about a point of the chart is followed round every one;
//! a step that is not shown is not taken, and the face passes), each point
//! projected onto the surface
//! (from the one before on a freeform surface, checked against the point,
//! else from nothing) and moved by whole periods to the one before it.
//! The middle of every step is projected too: how far it is from the
//! chord is the polyline's error. Crossings are proper crossings of two
//! steps whose ends lie on either side of the other step's line by more
//! than the errors of both loops and ten tolerances read in parameters at
//! the smallest scale of the surface over the face (the shortest of
//! `∂u`, `∂v` at the points): loops that touch, or come within that,
//! cross nothing here. Steps are swept in order of their lowest `u`, so a
//! face of `n` steps costs about `n log n` and the pairs that overlap.

use crate::{Body, Coedge, Face, FaceId, LoopId};
use craft3d_geom::{Curve3, CurvePiece, NurbsCurve, Surface};
use craft3d_math::{Portable, Tolerance, Vec3};
use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};
use std::f64::consts::PI;
use std::sync::{Mutex, PoisonError};

/// Most points one face is followed at; a larger face is passed.
pub const MOST_PER_FACE: usize = 1 << 18;
/// Steps of a spline edge (at least: as many more as [`spline_steps`]
/// needs).
const SPLINE_STEPS: usize = 8;
/// Most steps of a spline edge ([`steps_shown`]); an edge that needs more
/// is not followed.
const MOST_SPLINE_STEPS: usize = 1 << 12;
/// Most work that showing the steps of one face's spline edges, and
/// following those that need more than eight, may take, in units
/// ([`step_proofs`]): a point looked at is one, a leg read one (on a
/// plane), a half (round an axis) or two (its box of directions made, about
/// an axis), a Bézier piece made the square of its points, a step of an
/// edge followed in more than eight [`WALK_UNITS`]. A face that needs more
/// is not followed, so it passes, and is counted. Measured (2026-11-14,
/// one thread of the workshop's build server) a unit is 10 to 35 ns, 27
/// over a body of a hundred faces just under the bound, so the most a
/// face's steps can add to its check is 1.3 to 4.6 ms (3 ms a face in
/// that body); a strip of 64 turns between two single edges takes 87 000
/// units, one between edges of half a turn 52 an edge.
pub const MOST_PROOF_WORK: usize = 1 << 17;
/// What a step of an edge followed in more than eight steps counts for
/// against [`MOST_PROOF_WORK`]: two points of the curve and their places
/// on the surface, about a microsecond.
const WALK_UNITS: usize = 32;

thread_local! {
    /// The work showing steps has taken on this thread, and how many faces
    /// were not followed because theirs passed a bound.
    static STEP_PROOFS: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

/// What showing the steps of spline edges has cost on this thread since it
/// started: the units of work ([`MOST_PROOF_WORK`]), and the faces that
/// were not followed (and so passed) because showing their steps passed a
/// bound: the work of a face, or the steps of an edge (one that is not
/// shown within the most steps an edge may take, a curve through the axis
/// among them). For tests and reports: a count of work, never a clock.
pub fn step_proofs() -> (u64, u64) {
    STEP_PROOFS.with(Cell::get)
}

/// Counts `units` of work showing steps against the face's `work`; `None`
/// once the face has taken more than [`MOST_PROOF_WORK`] (counted once: the
/// face is not followed further).
fn spend(work: &mut usize, units: usize) -> Option<()> {
    if *work > MOST_PROOF_WORK {
        return None;
    }
    *work = work.saturating_add(units);
    let over = *work > MOST_PROOF_WORK;
    STEP_PROOFS.with(|c| {
        let (spent, cut) = c.get();
        c.set((
            spent.saturating_add(units as u64),
            cut.saturating_add(u64::from(over)),
        ));
    });
    (!over).then_some(())
}

/// The face is not followed because an edge of it cannot be shown within
/// the steps an edge may take: counted, once a face (its work is put past
/// its bound, so nothing more is counted for it).
fn give_up(work: &mut usize) {
    if *work <= MOST_PROOF_WORK {
        *work = usize::MAX;
        STEP_PROOFS.with(|c| {
            let (spent, cut) = c.get();
            c.set((spent, cut.saturating_add(1)));
        });
    }
}
/// Steps of a line on a curved surface.
const LINE_STEPS: usize = 2;
/// Largest angle of a circle's or an ellipse's step.
const CONIC_STEP: f64 = PI / 8.0;
/// Most pairs of steps looked at for a crossing; a face that needs more
/// is not held to it.
const MOST_PAIRS: usize = 1 << 24;
/// Most steps of a conic.
const MOST_CONIC_STEPS: usize = 64;

/// One loop in the surface's parameters.
pub(crate) struct Ring {
    pub(crate) points: Vec<[f64; 2]>,
    /// Where the loop ends against where it starts, in whole periods (0
    /// for a loop that does not wind around a closed surface).
    pub(crate) winds: [f64; 2],
    /// How far the polyline may be from the curves, in parameters.
    pub(crate) error: f64,
    pub(crate) low: [f64; 2],
    pub(crate) high: [f64; 2],
    /// Where each coedge's points begin in `points`, in the loop's order.
    pub(crate) starts: Vec<usize>,
    /// Whether a half step moved more than a quarter period: the step may
    /// have gone round by more than the chart shows, so how often the loop
    /// goes round, and where along it a point lies, are not to be read off
    /// it (the region rule reads it as before). A spline edge is followed
    /// in steps shown to move less than that about its surface's axis, and
    /// about a torus's tube ([`spline_steps`]); what is left is a conic near
    /// a pole, read by this flag.
    pub(crate) aliased: bool,
}

/// The loops of a face as the region rule followed them, handed to the
/// rule of simple loops (`crate::simple`) so that a face is followed once
/// for both.
pub(crate) struct Followed {
    /// In the order of the face's loops; holes moved by whole periods to
    /// lie over the outer loop where the region rule went that far.
    pub(crate) rings: Vec<Ring>,
    pub(crate) periods: [Option<f64>; 2],
    /// Ten tolerances read in parameters at the smallest scale of the
    /// surface over the face: the reach of a margin.
    pub(crate) reach: f64,
}

/// Why the loops of `face` do not bound a region of its surface, or `None`
/// when they do or this test cannot tell (see the module).
pub fn region_fault(body: &Body, face: &Face, tol: &Tolerance) -> Option<String> {
    region_followed(body, face, tol).0
}

/// [`region_fault`], and the loops as it followed them: given when every
/// loop was followed and the rule found nothing, either to its end or up
/// to loops that wind around a period (bands, which it passes); `None`
/// where it could not chart the face.
pub(crate) fn region_followed(
    body: &Body,
    face: &Face,
    tol: &Tolerance,
) -> (Option<String>, Option<Followed>) {
    if face.loops.is_empty() {
        return (None, None);
    }
    let surface = &face.surface;
    let periods = [surface.period_u(), surface.period_v()];
    if periods.iter().all(Option::is_some) {
        return (None, None);
    }
    let freeform = matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_));
    if freeform
        && (craft3d_geom::closure::closes(surface).iter().any(|c| *c)
            || !craft3d_geom::pole::poles(surface).is_empty())
    {
        return (None, None);
    }
    let near = 10.0 * body.working_tolerance(tol.linear);
    let mut chart = Chart::new(surface, periods, freeform, near);
    let mut rings = Vec::with_capacity(face.loops.len());
    let mut total = 0_usize;
    for id in &face.loops {
        let Some(lp) = body.loop_(*id) else {
            return (None, None);
        };
        // Bands are followed to the end, for the number of times they go
        // around (`crate::simple`); the region rule passes them.
        let Some(ring) = follow(body, &lp.coedges, &mut chart, &mut total, true) else {
            return (None, None);
        };
        rings.push(ring);
    }
    if rings.iter().any(|r| r.winds.iter().any(|w| *w != 0.0)) {
        let reach = near / chart.scale.max(f64::MIN_POSITIVE);
        return (
            None,
            Some(Followed {
                rings,
                periods,
                reach,
            }),
        );
    }
    // Areas with the face's sense: positive for a loop that runs
    // counter-clockwise about the outward normal.
    let sense = if face.reversed { -1.0 } else { 1.0 };
    let mut areas = Vec::with_capacity(rings.len());
    for ring in &rings {
        let area = signed_area(&ring.points) * sense;
        let perimeter = perimeter(&ring.points);
        if !(area.is_finite() && area.abs() > 2.0 * perimeter * ring.error) {
            return (None, None);
        }
        areas.push(area);
    }
    let outer: Vec<usize> = (0..areas.len())
        .filter(|i| areas.get(*i).is_some_and(|a| *a > 0.0))
        .collect();
    let Some(largest) = areas
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .map(|(i, _)| i)
    else {
        return (None, None);
    };
    match outer.as_slice() {
        [] if matches!(surface, Surface::Sphere(_)) => {}
        [] => {
            return (
                Some(format!(
                    "every loop runs clockwise about the outward normal (areas {}): the face would be everything outside them",
                    list(&areas)
                )),
                None,
            );
        }
        [o] if *o == largest => {}
        [o] => {
            return (
                Some(format!(
                    "its counter-clockwise loop {o} encloses less than its hole {largest} (areas {})",
                    list(&areas)
                )),
                None,
            );
        }
        _ => {
            return (
                Some(format!(
                    "{} of its loops run counter-clockwise about the outward normal (areas {}): a face has one outer loop",
                    outer.len(),
                    list(&areas)
                )),
                None,
            );
        }
    }
    // Margin of a crossing, in parameters.
    let errors: Vec<f64> = rings.iter().map(|r| r.error).collect();
    let reach = near / chart.scale.max(f64::MIN_POSITIVE);
    let margin_of = |i: usize, j: usize| {
        let e = |k: usize| errors.get(k).copied().unwrap_or(0.0);
        e(i) + e(j) + reach
    };
    // Holes moved by whole periods to lie over the outer loop.
    if let [o] = outer.as_slice() {
        let o = *o;
        let (Some((lo, hi)), Some(outer_points)) = (
            rings.get(o).map(|r| (r.low, r.high)),
            rings.get(o).map(|r| r.points.clone()),
        ) else {
            return (None, None);
        };
        for (k, ring) in rings.iter_mut().enumerate() {
            if k == o {
                continue;
            }
            for (d, period) in periods.iter().enumerate() {
                let Some(p) = period else { continue };
                let (Some(l), Some(h), Some(rl), Some(rh)) =
                    (lo.get(d), hi.get(d), ring.low.get(d), ring.high.get(d))
                else {
                    continue;
                };
                // The whole periods that bring its middle nearest the
                // outer loop's, or one either side of that when the outer
                // loop winds around its point only there (an outer loop
                // that spans nearly a period).
                let nearest = ((0.5 * (l + h) - 0.5 * (rl + rh)) / p).round();
                let point = ring.points.first().copied();
                let wound_at = |turns: f64| {
                    point.map_or(0, |q| {
                        let mut q = q;
                        if let Some(x) = q.get_mut(d) {
                            *x += turns * p;
                        }
                        winding(q, &outer_points)
                    })
                };
                let turns = [nearest, nearest - 1.0, nearest + 1.0]
                    .into_iter()
                    .find(|t| wound_at(*t) != 0)
                    .unwrap_or(nearest);
                if turns != 0.0 {
                    ring.shift(d, turns * p);
                }
            }
        }
        // Each hole wound once by the outer loop, by no other hole.
        let mut work = 0_usize;
        let widest = (0..rings.len())
            .map(|j| margin_of(j, j))
            .fold(0.0, f64::max)
            * 2.0;
        let grid = Grid::new(&rings, widest);
        if !matches!(surface, Surface::Plane(_)) {
            for (k, ring) in rings.iter().enumerate() {
                if k == o {
                    continue;
                }
                // Every point of the hole clear of the other loops: a hole
                // that runs across a loop's border has points on both
                // sides of it, found here even where it crosses through a
                // point of a polyline (which the crossing test of steps,
                // with its margin, does not see).
                for point in ring.points.iter() {
                    // Only the loops whose boxes (grown by the largest
                    // margin) hold the point can be near it or wind
                    // around it: found in the grid, not by a look at
                    // every loop.
                    let near = grid.at(*point);
                    work = work.saturating_add(near.len().saturating_add(1));
                    let clear = near.iter().all(|&j| {
                        let Some(other) = rings.get(j) else {
                            return true;
                        };
                        j == k || other.box_distance(*point) > margin_of(j, k) || {
                            work = work.saturating_add(other.points.len());
                            distance(*point, &other.points) > margin_of(j, k)
                        }
                    });
                    if work > MOST_PAIRS {
                        return (None, None);
                    }
                    if !clear {
                        continue;
                    }
                    let wound: i64 = near
                        .iter()
                        .filter(|j| **j != k)
                        .filter_map(|j| rings.get(*j))
                        .map(|other| {
                            if other.box_distance(*point) > 0.0 {
                                0
                            } else {
                                work = work.saturating_add(other.points.len());
                                winding(*point, &other.points)
                            }
                        })
                        .sum::<i64>()
                        * if face.reversed { -1 } else { 1 };
                    if wound != 1 {
                        let each: Vec<String> = rings
                            .iter()
                            .map(|other| winding(*point, &other.points).to_string())
                            .collect();
                        return (
                            Some(format!(
                                "its hole, loop {k}, lies outside its outer loop {o} or inside another hole (wound {wound} times at ({:.6}, {:.6}) of the {}; by each loop {}; areas {}; boxes {})",
                                point[0],
                                point[1],
                                surface.kind(),
                                each.join(", "),
                                list(&areas),
                                rings
                                    .iter()
                                    .map(|r| format!(
                                        "[{:.4}, {:.4}]x[{:.4}, {:.4}]",
                                        r.low[0], r.high[0], r.low[1], r.high[1]
                                    ))
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            )),
                            None,
                        );
                    }
                }
            }
        }
    }
    let crossed = crossing(&rings, &margin_of).map(|(i, j)| {
        if i == j {
            format!("its loop {i} crosses itself")
        } else {
            format!("its loops {i} and {j} cross")
        }
    });
    match crossed {
        Some(why) => (Some(why), None),
        None => (
            None,
            Some(Followed {
                rings,
                periods,
                reach,
            }),
        ),
    }
}

/// Whether each of `points` (on the surface of `face`) lies on the face,
/// by the parity of the loops' crossings of a ray from it in the
/// surface's parameters (along v, or along u when v is the one that
/// closes); `None` where the face cannot be charted without doubt, as for
/// [`region_fault`], bands around a closed surface included.
pub fn face_holds(body: &Body, face: &Face, tol: &Tolerance, points: &[Vec3]) -> Option<Vec<bool>> {
    if face.loops.is_empty() {
        return None;
    }
    let surface = &face.surface;
    let periods = [surface.period_u(), surface.period_v()];
    if periods.iter().all(Option::is_some) {
        return None;
    }
    let freeform = matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_));
    if freeform
        && (craft3d_geom::closure::closes(surface).iter().any(|c| *c)
            || !craft3d_geom::pole::poles(surface).is_empty())
    {
        return None;
    }
    let mut chart = Chart::new(
        surface,
        periods,
        freeform,
        10.0 * body.working_tolerance(tol.linear),
    );
    let mut total = 0_usize;
    let mut rings = Vec::with_capacity(face.loops.len());
    for id in &face.loops {
        let lp = body.loop_(*id)?;
        rings.push(follow(body, &lp.coedges, &mut chart, &mut total, true)?);
    }
    // The ray runs along `r`; the other parameter `o` may close.
    let (r, o) = if periods[1].is_some() { (0, 1) } else { (1, 0) };
    // An odd number of loops around the axis: the face reaches a pole on
    // one side. A cone's apex closes it there (a loop at the apex's v,
    // crossed by a ray that passes it); any other pole is not followed.
    let around = rings
        .iter()
        .filter(|ring| ring.winds.iter().any(|w| *w != 0.0))
        .count();
    let apex = match surface {
        _ if around % 2 == 0 => None,
        Surface::Cone(c) if r == 1 => Some(c.apex_v()),
        _ => return None,
    };
    let period = periods.get(o).copied().flatten();
    let mut out = Vec::with_capacity(points.len());
    for p in points {
        chart.last = None;
        let q = chart.at(*p)?;
        let (qr, qo) = (q.get(r).copied()?, q.get(o).copied()?);
        let mut crossings = 0_usize;
        for ring in &rings {
            let n = ring.points.len();
            let (lo, hi) = (ring.low.get(o).copied()?, ring.high.get(o).copied()?);
            let shifts: Vec<f64> = match period {
                Some(per) => {
                    let first = ((lo - qo) / per).floor() as i64 - 1;
                    let last = ((hi - qo) / per).ceil() as i64 + 1;
                    (first..=last).map(|k| qo + k as f64 * per).collect()
                }
                None => vec![qo],
            };
            for at in shifts {
                for i in 0..n {
                    let a = *ring.points.get(i)?;
                    let mut b = *ring.points.get((i + 1) % n)?;
                    if i + 1 == n {
                        b = [b[0] + ring.winds[0], b[1] + ring.winds[1]];
                    }
                    let (ao, bo) = (a.get(o).copied()?, b.get(o).copied()?);
                    if (ao <= at) == (bo <= at) {
                        continue;
                    }
                    let t = (at - ao) / (bo - ao);
                    let (ar, br) = (a.get(r).copied()?, b.get(r).copied()?);
                    if ar + t * (br - ar) > qr {
                        crossings += 1;
                    }
                }
            }
        }
        if apex.is_some_and(|v| v > qr) {
            crossings += 1;
        }
        out.push(crossings % 2 == 1);
    }
    Some(out)
}

/// What is wrong with the loops of a face: they do not bound a region
/// ([`region_fault`]), or one of them is not a simple closed curve
/// (`crate::simple`, asked only of a face whose loops bound a region).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Fault {
    Region(String),
    Loop(LoopId, String),
}

/// Both rules on one face, the loops followed once for both.
fn face_fault(body: &Body, face: &Face, tol: &Tolerance) -> Option<Fault> {
    let (region, followed) = region_followed(body, face, tol);
    if let Some(why) = region {
        return Some(Fault::Region(why));
    }
    crate::simple::loop_fault(body, face, tol, followed.as_ref())
        .map(|(loop_, why)| Fault::Loop(loop_, why))
}

/// Every face of `body` whose loops do not bound a region, with why.
///
/// Faces that passed lately are remembered by everything
/// [`region_fault`] and the rule of simple loops read of them (see
/// [`face_key`]), so a face a boolean or a regeneration carries over
/// unchanged from a body checked before is not followed again: in a row
/// of operations most faces of each result are faces of its operands.
pub fn regionless(body: &Body, tol: &Tolerance) -> Vec<(FaceId, String)> {
    faults(body, tol)
        .into_iter()
        .filter_map(|(face, fault)| match fault {
            Fault::Region(why) => Some((face, why)),
            Fault::Loop(..) => None,
        })
        .collect()
}

/// Every face of `body` with a loop that is not a simple closed curve
/// (`crate::simple`), with the loop and why; a face whose loops do not
/// bound a region ([`regionless`]) is not asked.
pub fn loops_not_simple(body: &Body, tol: &Tolerance) -> Vec<(FaceId, LoopId, String)> {
    faults(body, tol)
        .into_iter()
        .filter_map(|(face, fault)| match fault {
            Fault::Loop(loop_, why) => Some((face, loop_, why)),
            Fault::Region(_) => None,
        })
        .collect()
}

/// [`regionless`] and [`loops_not_simple`] together, each face followed
/// once.
pub(crate) fn faults(body: &Body, tol: &Tolerance) -> Vec<(FaceId, Fault)> {
    let mut found = Vec::new();
    let near = 10.0 * body.working_tolerance(tol.linear);
    for (i, face) in body.faces.iter().enumerate() {
        if let Some(fault) = remembered_fault(body, face, tol, near) {
            found.push((FaceId(u32::try_from(i).unwrap_or(u32::MAX)), fault));
        }
    }
    found
}

/// Faces that passed [`region_fault`] lately, by the hash of their key
/// ([`face_key`]): the key in full (a hit must match it word for word)
/// and the work the check took (`craft3d_geom::allowance`), the oldest
/// first in `order`.
struct PassedFaces {
    by_hash: BTreeMap<u64, (Vec<u64>, u64)>,
    order: VecDeque<u64>,
    words: usize,
}

static PASSED_FACES: Mutex<PassedFaces> = Mutex::new(PassedFaces {
    by_hash: BTreeMap::new(),
    order: VecDeque::new(),
    words: 0,
});
/// Lets go of the faces remembered as passed (`crate::forget_passed`).
pub(crate) fn forget_passed_faces() {
    let mut passed = PASSED_FACES.lock().unwrap_or_else(PoisonError::into_inner);
    passed.by_hash.clear();
    passed.order.clear();
    passed.words = 0;
}

/// How many words all remembered faces are at most (16 MB).
const PASSED_FACE_WORDS: usize = 1 << 21;

/// Everything [`region_fault`] and the rule of simple loops read of `face`
/// in `body`, every bit of every number (the verdict is a function of
/// these alone, `docs/design/topology.md` › *Loops that are simple*): the
/// reach `near` (ten of the body's working tolerances, which also sets how
/// far the rule grows the boxes of edges), the angular tolerance (read by
/// neither rule today; kept so that a rule that reads it cannot be served
/// a verdict made at another), whether the body is a sheet, the surface and
/// its sense, and each loop's coedges as their sense, the edge's curve and
/// its range, the edge's own tolerance (the document's where it has none:
/// two stretches are one curve within the tighter of their edges'), the
/// points of its start and end vertices (which box it), and which earlier
/// coedge of the face uses the same edge (a seam or a spur is one edge used
/// twice, which two edges alike are not; the edges' own numbers are not
/// read, so a face renumbered is the same face). `None` for a face that
/// names a loop, an edge or a vertex the body does not have (checked in
/// full, never remembered).
fn face_key(body: &Body, face: &Face, near: f64, tol: &Tolerance) -> Option<Vec<u64>> {
    use craft3d_math::Bits;
    let mut key = Vec::new();
    near.push_bits(&mut key);
    tol.angular.push_bits(&mut key);
    // A sheet may have a cut, a solid may not (`crate::simple`).
    (body.kind == crate::BodyKind::Sheet).push_bits(&mut key);
    face.surface.push_bits(&mut key);
    face.reversed.push_bits(&mut key);
    face.loops.len().push_bits(&mut key);
    let mut first_use: BTreeMap<crate::EdgeId, usize> = BTreeMap::new();
    for id in &face.loops {
        let lp = body.loop_(*id)?;
        lp.coedges.len().push_bits(&mut key);
        for coedge in &lp.coedges {
            let edge = body.edge(coedge.edge)?;
            coedge.reversed.push_bits(&mut key);
            edge.curve.push_bits(&mut key);
            edge.range.push_bits(&mut key);
            body.edge_tolerance_at(coedge.edge, tol.linear)
                .push_bits(&mut key);
            body.vertex(edge.start)?.point.push_bits(&mut key);
            body.vertex(edge.end)?.point.push_bits(&mut key);
            let count = first_use.len();
            let first = *first_use.entry(coedge.edge).or_insert(count);
            first.push_bits(&mut key);
        }
    }
    Some(key)
}

/// [`face_fault`] of `face`, answered from [`PASSED_FACES`] when the
/// face passed before. A hit charges the work the check took, so an
/// operation counts the same whether the face was remembered or not; a
/// face is remembered only when it passed while work was counted (not
/// cut short by a bound).
fn remembered_fault(body: &Body, face: &Face, tol: &Tolerance, near: f64) -> Option<Fault> {
    use craft3d_geom::allowance;
    let Some(key) = face_key(body, face, near, tol) else {
        return face_fault(body, face, tol);
    };
    let hash = key.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, w| {
        (h ^ w).wrapping_mul(0x0100_0000_01b3)
    });
    {
        let passed = PASSED_FACES.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((kept, units)) = passed.by_hash.get(&hash)
            && *kept == key
        {
            allowance::charge(*units);
            return None;
        }
    }
    let counting = allowance::counting();
    let (fault, units) = allowance::cost_of(|| face_fault(body, face, tol));
    if fault.is_none() && counting && allowance::counting() && key.len() <= PASSED_FACE_WORDS {
        let mut passed = PASSED_FACES.lock().unwrap_or_else(PoisonError::into_inner);
        let words = key.len();
        if let Some((old, _)) = passed.by_hash.insert(hash, (key, units)) {
            passed.words = passed.words.saturating_sub(old.len());
        } else {
            passed.order.push_back(hash);
        }
        passed.words = passed.words.saturating_add(words);
        while passed.words > PASSED_FACE_WORDS {
            let Some(oldest) = passed.order.pop_front() else {
                break;
            };
            if let Some((old, _)) = passed.by_hash.remove(&oldest) {
                passed.words = passed.words.saturating_sub(old.len());
            }
        }
    }
    fault
}

fn list(areas: &[f64]) -> String {
    areas
        .iter()
        .map(|a| format!("{a:.3e}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Points of the surface to parameters, each moved by whole periods to the
/// one before.
pub(crate) struct Chart<'a> {
    pub(crate) surface: &'a Surface,
    pub(crate) periods: [Option<f64>; 2],
    pub(crate) freeform: bool,
    pub(crate) near: f64,
    pub(crate) last: Option<[f64; 2]>,
    /// The shortest of `∂u`, `∂v` at the points so far.
    pub(crate) scale: f64,
    /// The work showing the steps of the face's edges has taken
    /// ([`MOST_PROOF_WORK`]).
    pub(crate) work: usize,
}

impl<'a> Chart<'a> {
    pub(crate) fn new(
        surface: &'a Surface,
        periods: [Option<f64>; 2],
        freeform: bool,
        near: f64,
    ) -> Self {
        Chart {
            surface,
            periods,
            freeform,
            near,
            last: None,
            scale: f64::INFINITY,
            work: 0,
        }
    }
}

impl Chart<'_> {
    pub(crate) fn at(&mut self, p: Vec3) -> Option<[f64; 2]> {
        // On a freeform surface from the point before (a few steps) with
        // its derivatives, else from nothing; the derivatives give the
        // point the parameters land on and the chart's scale.
        let walked = match (self.freeform, self.last) {
            (true, Some([u, v])) => self
                .surface
                .project_from_with_derivatives(p, (u, v))
                .ok()
                .filter(|(_, (at, _, _))| at.distance(p) <= self.near),
            _ => None,
        };
        let ((u, v), (at, du, dv)) = match walked {
            Some(found) => found,
            None => {
                let (u, v) = self.surface.project(p).ok()?;
                ((u, v), self.surface.derivatives(u, v))
            }
        };
        let off = at.distance(p);
        if off.is_nan() || off > self.near {
            return None;
        }
        // Poles and apexes: the parameter around them is not defined.
        match self.surface {
            Surface::Sphere(s) if s.radius() * v.pcos() <= self.near => return None,
            Surface::Cone(c) if self.surface.eval(0.0, c.apex_v()).distance(p) <= self.near => {
                return None;
            }
            _ => {}
        }
        let shortest = du.length().min(dv.length());
        if shortest.is_finite() {
            self.scale = self.scale.min(shortest);
        }
        let mut uv = [u, v];
        if let Some(before) = self.last {
            for (d, period) in self.periods.iter().enumerate() {
                if let (Some(p), Some(x), Some(b)) = (period, uv.get_mut(d), before.get(d)) {
                    *x += ((b - *x) / p).round() * p;
                }
            }
        }
        self.last = Some(uv);
        (uv[0].is_finite() && uv[1].is_finite()).then_some(uv)
    }
}

/// A loop as a closed polyline in the chart; `None` where it cannot be
/// followed, or winds around a period and `bands` is false.
pub(crate) fn follow(
    body: &Body,
    coedges: &[Coedge],
    chart: &mut Chart<'_>,
    total: &mut usize,
    bands: bool,
) -> Option<Ring> {
    let planar = matches!(chart.surface, Surface::Plane(_));
    let lay = Lay::of(chart.surface);
    let mut points = Vec::new();
    let mut starts = Vec::with_capacity(coedges.len());
    let mut aliased = false;
    let mut error = 0.0_f64;
    chart.last = None;
    let mut start: Option<[f64; 2]> = None;
    for coedge in coedges {
        starts.push(points.len());
        let edge = body.edge(coedge.edge)?;
        let (t0, t1) = edge.range;
        if !(t0.is_finite() && t1.is_finite() && t1 > t0) {
            return None;
        }
        let steps = match &edge.curve {
            Curve3::Line(_) if planar => 1,
            Curve3::Line(_) => LINE_STEPS,
            Curve3::Circle(_) | Curve3::Ellipse(_) => {
                let n = ((t1 - t0) / CONIC_STEP).ceil();
                if n.is_finite() {
                    (n.max(2.0) as usize).min(MOST_CONIC_STEPS)
                } else {
                    return None;
                }
            }
            Curve3::Nurbs(_) => SPLINE_STEPS,
        };
        // The steps of a spline edge are shown to hide no turn, about the
        // axis of a surface that closes and about any point of the chart,
        // or the loop is not followed.
        let steps = match &edge.curve {
            Curve3::Nurbs(n) => {
                let shown = spline_steps(n, (t0, t1), &lay, &mut chart.work)?;
                if shown > steps {
                    // Following it costs too: counted with the showing.
                    spend(&mut chart.work, shown.saturating_mul(WALK_UNITS))?;
                }
                shown
            }
            _ => steps,
        };
        *total = total.saturating_add(2 * steps);
        if *total > MOST_PER_FACE {
            return None;
        }
        let at = |k: f64| {
            let f = k / steps as f64;
            if coedge.reversed {
                t1 - (t1 - t0) * f
            } else {
                t0 + (t1 - t0) * f
            }
        };
        let mut before = chart.at(edge.curve.eval(at(0.0)))?;
        if start.is_none() {
            start = Some(before);
        }
        for k in 0..steps {
            points.push(before);
            let middle = chart.at(edge.curve.eval(at(k as f64 + 0.5)))?;
            let next = chart.at(edge.curve.eval(at(k as f64 + 1.0)))?;
            for (d, period) in chart.periods.iter().enumerate() {
                if let (Some(p), Some(b), Some(m), Some(n)) =
                    (period, before.get(d), middle.get(d), next.get(d))
                    && ((m - b).abs() > 0.25 * p || (n - m).abs() > 0.25 * p)
                {
                    aliased = true;
                }
            }
            let chord = [0.5 * (before[0] + next[0]), 0.5 * (before[1] + next[1])];
            error = error.max(2.0 * (chord[0] - middle[0]).phypot(chord[1] - middle[1]));
            before = next;
        }
    }
    // Back at its start: a loop that went around a period is a band.
    let (Some(s), Some(end)) = (start, chart.last) else {
        return None;
    };
    let mut winds = [0.0; 2];
    for (d, period) in chart.periods.iter().enumerate() {
        if let (Some(p), Some(a), Some(b), Some(w)) =
            (period, s.get(d), end.get(d), winds.get_mut(d))
            && (a - b).abs() > 0.5 * p
        {
            if !bands {
                return None;
            }
            *w = b - a;
        }
    }
    if points.len() < 3 {
        return None;
    }
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for p in &points {
        low = [low[0].min(p[0]), low[1].min(p[1])];
        high = [high[0].max(p[0]), high[1].max(p[1])];
    }
    Some(Ring {
        points,
        winds,
        error,
        low,
        high,
        starts,
        aliased,
    })
}

/// What the chart of a surface makes of a spline edge, for showing its
/// steps ([`spline_steps`]).
#[derive(Clone, Copy, Debug)]
enum Lay {
    /// A plane: `(u, v)` are a point's coordinates along `x` and `y`, so
    /// the chart's image of a spline is the spline with its control points
    /// read along them.
    Flat { x: Vec3, y: Vec3 },
    /// A surface that closes about the axis through `origin` along the
    /// unit `z` (a cylinder, a cone, a sphere, a torus): `u`, whose period
    /// is a turn, is a point's angle about that axis, also of a point off
    /// the surface (the point of the surface nearest it, which the chart
    /// takes, has its angle); `v` is read as `rise` says.
    About { origin: Vec3, z: Vec3, rise: Rise },
    /// A spline surface: nothing is known of how its chart takes a
    /// direction in space.
    Space,
    /// For the tests of the proof about an axis alone: only the angle
    /// about the axis through `origin` along `z` is shown.
    #[cfg(test)]
    Turning { origin: Vec3, z: Vec3 },
}

/// How `v` is read off a point about an axis ([`Lay::About`]), at the
/// distance `ρ` from the axis and the height `h` along it.
#[derive(Clone, Copy, Debug)]
enum Rise {
    /// A cylinder: `v = h`.
    Height,
    /// A cone of the radius `radius` at `h = 0` that widens by `slope` a
    /// unit of height: the nearest point of its ruling,
    /// `v = ((ρ − radius) slope + h) / (slope² + 1)`, stopped at the apex.
    Slant { slope: f64, radius: f64 },
    /// A sphere: the latitude, `v = atan2(h, ρ)`.
    Latitude,
    /// A torus whose tube's centre circle has the radius `major`: the
    /// angle about the tube, `v = atan2(h, ρ − major)`, whose period is a
    /// turn as well.
    Tube { major: f64 },
}

impl Rise {
    /// Whether a stretch within `reach` keeps to where `v` is read off it
    /// as this says and can be followed: on a cone, to the cone's own side
    /// of its apex, where `v` is the ruling's and not stopped; on a torus,
    /// within less than a quarter turn about the tube, so that the chart,
    /// which moves each point by whole periods to the one before, follows
    /// the edge round the tube as it does round the axis (the box of the
    /// stretch's distances from the axis and its heights lies in a fan
    /// about the tube's centre narrower than that, and so clear of it).
    fn keeps(&self, reach: &Reach) -> bool {
        let (near, far) = reach.off;
        let (below, above) = reach.height;
        match self {
            Rise::Height | Rise::Latitude => true,
            Rise::Slant { slope, radius } => {
                let ruling = if *slope > 0.0 { below } else { above };
                (near * slope + ruling) * slope + radius > 0.0
            }
            Rise::Tube { major } => {
                let mut fan = Fan::within(QUARTER_TURN);
                [
                    (near - major, below),
                    (near - major, above),
                    (far - major, below),
                    (far - major, above),
                ]
                .into_iter()
                .all(|corner| fan.add(corner))
            }
        }
    }
}

impl Lay {
    fn of(surface: &Surface) -> Lay {
        let about = |frame: &craft3d_math::Frame, rise: Rise| Lay::About {
            origin: frame.origin(),
            z: frame.z(),
            rise,
        };
        match surface {
            Surface::Plane(p) => Lay::Flat {
                x: p.frame().x(),
                y: p.frame().y(),
            },
            Surface::Cylinder(c) => about(c.frame(), Rise::Height),
            Surface::Cone(c) => about(
                c.frame(),
                Rise::Slant {
                    slope: c.half_angle().ptan(),
                    radius: c.radius(),
                },
            ),
            Surface::Sphere(s) => about(s.frame(), Rise::Latitude),
            Surface::Torus(t) => about(t.frame(), Rise::Tube { major: t.radii().0 }),
            Surface::Bezier(_) | Surface::Nurbs(_) => Lay::Space,
        }
    }

    /// Whether a stretch within `reach` about the axis can be followed in
    /// this chart ([`Rise::keeps`]).
    fn keeps(&self, reach: &Reach) -> bool {
        match self {
            Lay::About { rise, .. } => rise.keeps(reach),
            Lay::Flat { .. } | Lay::Space => true,
            #[cfg(test)]
            Lay::Turning { .. } => true,
        }
    }

    /// The axis the surface closes about, as a point on it and its unit
    /// direction.
    fn axis(&self) -> Option<(Vec3, Vec3)> {
        match self {
            Lay::About { origin, z, .. } => Some((*origin, *z)),
            Lay::Flat { .. } | Lay::Space => None,
            #[cfg(test)]
            Lay::Turning { origin, z } => Some((*origin, *z)),
        }
    }
}

/// The steps an edge over `(t0, t1)` is followed in: the fewest
/// `base · 2^k`, `most` at the most, such that `shown` holds of every half
/// step, `shown(a, b, work)` saying whether the edge over `(a, b)` is shown
/// to hide no turn there (`None`: the face's work is spent, [`spend`]).
/// What `shown` holds of a stretch it holds of every part of it, so each of
/// the `2 · base` half steps is halved only until it is shown, and the
/// deepest it needed sets the steps. `None` where a half step cannot be
/// shown within `most` steps or the work is spent: the loop cannot be
/// followed without doubt.
fn steps_shown(
    base: usize,
    most: usize,
    (t0, t1): (f64, f64),
    work: &mut usize,
    mut shown: impl FnMut(f64, f64, &mut usize) -> Option<bool>,
) -> Option<usize> {
    let width = t1 - t0;
    let halves = base.checked_mul(2).filter(|h| *h > 0)?;
    let deepest = (most / base).max(1).ilog2();
    let mut level = 0_u32;
    // Each half step of `base` halved until it is shown: (the stretch's
    // index at its level, the level). At most `halves · 2^(deepest + 1)`.
    let mut open: Vec<(usize, u32)> = (0..halves).map(|i| (i, 0)).collect();
    while let Some((i, at)) = open.pop() {
        let parts = halves.checked_shl(at)?;
        let end = |k: usize| t0 + width * (k as f64 / parts as f64);
        let (a, b) = (end(i), end(i + 1));
        spend(work, 1)?;
        if shown(a, b, work)? {
            level = level.max(at);
        } else if at < deepest {
            open.push((2 * i + 1, at + 1));
            open.push((2 * i, at + 1));
        } else {
            give_up(work);
            return None;
        }
    }
    base.checked_shl(level)
}

/// The steps a spline edge over `(t0, t1)` is followed in on a surface
/// whose chart is `lay`: the fewest `SPLINE_STEPS · 2^k` such that over
/// each half step the edge is shown to hide no turn from the walk
/// ([`steps_shown`]), which reads the edge at the ends and the middle of
/// each step and takes the chord between them for it. Two things are shown
/// of a half step.
///
/// **About an axis** ([`Lay::About`]) the curve's angle about it moves by
/// less than a quarter turn, so the chart, which moves each point by whole
/// periods to the one before, unwraps the loop as it goes round: a helix of
/// many turns as one edge is followed round every turn, where eight steps
/// of it read as a few turns the other way. Shown by the first of: (1) the
/// convex hull of the control points of the knot spans the half step meets
/// in a wedge about the axis narrower than a quarter turn ([`Wedge`]); (2)
/// for a polynomial spline, its speed bound ([`speed_bound`]) times the half
/// step's width, over the least distance from the axis the stretch can have
/// (its middle's less half that length: the distance from a line changes no
/// faster than the point moves), less than a quarter turn (one evaluation
/// of the curve, counted against an operation's work, where (1) does not
/// show it); (3) the Bézier pieces of the spans the half step meets, each
/// made from its own span alone and cut to the stretch ([`Pieces`]), in
/// such a wedge (of a stretch that meets no more than [`MANY_SPANS`] spans:
/// a longer one not shown by (1) or (2) is halved).
///
/// **In the chart** the edge's image runs one way: its direction keeps
/// within a fan narrower than a half turn ([`ONE_WAY`], [`one_way`]), read
/// off the legs of the same control points, (1), or of the cut pieces, (3):
/// the direction of a spline with positive weights is, at each of its
/// points, a sum of the legs between the control points of its span with
/// weights that are not negative, and the chart takes directions in space
/// to directions of its own by a linear map at each point ([`Lay`]). The
/// image and its chord then cross every line square to the fan's middle
/// once each, so together they go round no point more than once, and only
/// round points that lie between them: the chord hides no turn about any
/// point of the chart, and what lies between the two is what the walk
/// measures at the step's middle. On a spline surface ([`Lay::Space`]),
/// whose chart is not known, the edge is held to running one way along its
/// stretch's own chord in space, and only as a reason not to follow it: an
/// edge so shown within `SPLINE_STEPS` is followed in them, as every spline
/// edge was before, and one that is not is not followed.
///
/// (1) and (2) are read only of a stretch inside the curve's domain; past
/// it (3) cuts the first or the last piece as far as the stretch goes, the
/// curve being that piece continued there. Each looks at the points of a
/// stretch only until they are too wide, and bounds the movement over the
/// whole stretch, so over any part of it. The whole edge is looked at
/// first, by its control points alone, (1): an edge shown at one look,
/// which is nearly every edge, is followed in `SPLINE_STEPS` at the points
/// it always was, and that look, a pass over the edge's control points as
/// the key a face is remembered by makes one, is not counted against the
/// face's work (it stops at the first point or leg that does not fit).
/// `None` where it cannot be
/// shown within [`MOST_SPLINE_STEPS`] (a curve through or very near the
/// axis, a helix of thousands of turns, an edge that turns back on itself
/// at a point no halving reaches) or within the face's work: the loop
/// cannot be followed without doubt.
fn spline_steps(
    curve: &NurbsCurve,
    range: (f64, f64),
    lay: &Lay,
    work: &mut usize,
) -> Option<usize> {
    let (d0, d1) = curve.domain();
    // How far rounding can put the curve outside the hull of points made
    // for it, and so how far from the axis such points must keep
    // ([`Wedge`]) and how long a leg must be to have a direction
    // ([`one_way`]): none for a curve with a weight that is not positive.
    let clear = HULL_CLEAR * hull_rounding(curve)?;
    let axis = lay.axis();
    // (1) The control points whose basis functions are not zero over
    // `(a, b)`, found without inserting a knot.
    let (knots, ctrl, degree) = (curve.knots(), curve.control_points(), curve.degree());
    let span = |t: f64| {
        knots
            .partition_point(|k| *k <= t)
            .saturating_sub(1)
            .clamp(degree, ctrl.len().saturating_sub(1))
    };
    let spanned = |a: f64, b: f64| ctrl.get(span(a).saturating_sub(degree)..=span(b));
    // (2) The angle moves at most at the speed over the distance from the
    // axis: what the stretch keeps within, about its middle, and whether
    // it is shown there to go round the axis one way. The speed's bound
    // is made once, where (1) first fails; the bound on how fast the
    // direction changes span by span, each once (`made` counts the legs
    // read for them).
    let speed = std::cell::OnceCell::new();
    let bends: std::cell::RefCell<BTreeMap<usize, Option<f64>>> = Default::default();
    let bend_over = |a: f64, b: f64, made: &mut usize| -> Option<f64> {
        // The spans the stretch lies in, up to the one it ends in (not one
        // that only starts where it ends).
        let first = span(a);
        let last = knots
            .partition_point(|k| *k < b)
            .saturating_sub(1)
            .clamp(first, ctrl.len().saturating_sub(1));
        if last - first > MANY_SPANS {
            return None;
        }
        let mut most = 0.0_f64;
        let mut before: Option<usize> = None;
        for k in first..=last {
            if !matches!((knots.get(k), knots.get(k + 1)), (Some(x), Some(y)) if y > x) {
                continue;
            }
            // Between two spans the direction is one only where fewer
            // knots than the degree lie on each other.
            if before.is_some_and(|j| k - j >= degree) {
                return None;
            }
            before = Some(k);
            let bend = *bends.borrow_mut().entry(k).or_insert_with(|| {
                *made += degree;
                bend_bound(curve, k)
            });
            most = most.max(bend?);
        }
        before.map(|_| most)
    };
    let slow = |a: f64, b: f64, made: &mut usize| -> Option<(Reach, bool)> {
        let (origin, z) = axis?;
        let speed = (*speed.get_or_init(|| speed_bound(curve)))?;
        let reach = (b - a) * speed;
        let (at, going) = curve.derivatives(0.5 * (a + b));
        let d = at - origin;
        let height = d.dot(z);
        let across = d - z * height;
        let off = across.length();
        let low = off - 0.5 * reach;
        if !(low > clear && reach < QUARTER_TURN * low) {
            return None;
        }
        // Half the stretch either side of its middle: at most half the
        // angle each way, less than an eighth of a turn.
        let (sin, cos) = (0.5 * reach / low).psin_cos();
        let middle = across * (1.0 / off);
        let on = z.cross(middle);
        // Round the axis one way: the curve's direction at its middle
        // runs round the axis there by more than what its part away from
        // the axis can take from that as the point goes round by up to
        // half the angle, and what the direction itself can change by
        // over half the stretch (its second derivative's bound).
        let within = Reach {
            low: middle * cos - on * sin,
            high: middle * cos + on * sin,
            off: (low, off + 0.5 * reach),
            height: (height - 0.5 * reach, height + 0.5 * reach),
        };
        let round = lay.keeps(&within)
            && bend_over(a, b, made).is_some_and(|bend| {
                let lost = going.dot(middle).abs() * sin + bend * 0.5 * (b - a);
                going.dot(on).abs() * cos - lost > ROUND_CLEAR * speed
            });
        Some((within, round))
    };
    // (3) The Bézier pieces, each made once where (1) and (2) do not show
    // it, and the points of a stretch's cut pieces.
    let mut pieces = Pieces::of(curve);
    let mut held = Held::default();
    // The last run of control points (the first and the last of them)
    // found too wide: for a wedge about the axis, or on a plane for their
    // legs to lie in a fan. A stretch whose spans' control points hold all
    // of that run is too wide as well, and they are not looked at again:
    // each half of a stretch found too wide from its start is not read
    // from that start a second time.
    let mut wide: Option<(usize, usize)> = None;
    // Whether the edge over `(a, b)` is shown; `look` for the one look at
    // the whole edge, by its control points alone and not counted.
    let mut shown = |a: f64, b: f64, look: bool, work: &mut usize| -> Option<bool> {
        let charge = |work: &mut usize, units: usize| {
            if look { Some(()) } else { spend(work, units) }
        };
        if a >= d0 && b <= d1 {
            // The control points first: they cost no evaluation of the
            // curve (each counted against an operation's work,
            // `geom::allowance`).
            if let Some(points) = spanned(a, b) {
                let first = span(a).saturating_sub(degree);
                let known = wide.is_some_and(|(from, to)| {
                    first <= from && to < first.saturating_add(points.len())
                });
                let reach = match axis {
                    // On a plane the legs are what can be too wide.
                    None if known => None,
                    Some(axis) => {
                        let mut wedge = Wedge::about(axis, QUARTER_TURN);
                        let within = if known {
                            0
                        } else {
                            let within =
                                points.iter().take_while(|p| wedge.add(**p, clear)).count();
                            charge(work, within)?;
                            if within < points.len() {
                                wide = Some((first, first.saturating_add(within)));
                            }
                            within
                        };
                        let narrow = within > 0 && within == points.len();
                        match narrow.then(|| wedge.reach()).flatten() {
                            Some(reach) => Some(Some(reach)),
                            None if look => None,
                            None => {
                                let mut made = 0;
                                let slow = slow(a, b, &mut made);
                                charge(work, made)?;
                                match slow {
                                    // Shown to go round one way: the
                                    // image keeps to one side of a line
                                    // of constant `u`, whatever `v` does.
                                    Some((_, true)) => return Some(true),
                                    Some((reach, false)) => Some(Some(reach)),
                                    None => None,
                                }
                            }
                        }
                    }
                    None => Some(None),
                };
                if let Some(reach) = reach {
                    let (one, legs) =
                        one_way(lay, points, &[(points.len(), clear)], reach.as_ref());
                    charge(work, legs)?;
                    if one {
                        return Some(true);
                    }
                    // The legs up to the one that did not fit, on a plane.
                    if axis.is_none() && matches!(lay, Lay::Flat { .. }) {
                        wide = Some((first, first.saturating_add(legs)));
                    }
                }
            }
            // Over many spans the pieces say little more than the control
            // points did, and making them all would cost more than the
            // walk: halved first.
            if look || span(b).saturating_sub(span(a)) > MANY_SPANS {
                return Some(false);
            }
        }
        if look {
            return Some(false);
        }
        held.clear();
        let mut wedge = axis.map(|axis| Wedge::about(axis, QUARTER_TURN));
        let taken = pieces.each_piece((a, b), work, |points, growth| {
            let clear = clear * growth;
            if let Some(wedge) = wedge.as_mut()
                && !points.iter().all(|p| wedge.add(*p, clear))
            {
                return false;
            }
            held.take(points, clear);
            true
        })?;
        if !taken {
            return Some(false);
        }
        let reach = match &wedge {
            Some(wedge) => match wedge.reach() {
                Some(reach) => Some(reach),
                None => return Some(false),
            },
            None => None,
        };
        let (one, legs) = one_way(lay, &held.points, &held.runs, reach.as_ref());
        spend(work, legs)?;
        Some(one)
    };
    if shown(range.0, range.1, true, work)? {
        return Some(SPLINE_STEPS);
    }
    // Nothing shows how the chart of a spline surface takes an edge: an
    // edge that needs more steps than it always had is not followed.
    let most = match lay {
        Lay::Space => SPLINE_STEPS,
        Lay::Flat { .. } | Lay::About { .. } => MOST_SPLINE_STEPS,
        #[cfg(test)]
        Lay::Turning { .. } => MOST_SPLINE_STEPS,
    };
    steps_shown(SPLINE_STEPS, most, range, work, |a, b, work| {
        shown(a, b, false, work)
    })
}

/// The widest fan the directions of a half step of a spline edge may keep
/// within in the chart ([`one_way`]): three eighths of a turn, as computed.
/// A leg has its direction to an eighth of a radian ([`LEG_CLEAR`]), so a
/// fan found that narrow is narrower than a half turn, which is what the
/// proof needs: all its directions on one side of a line.
const ONE_WAY: f64 = 0.75 * PI;

/// How many times the rounding of a curve's hull points ([`hull_rounding`],
/// times [`HULL_CLEAR`] already) a leg between two of them must be long for
/// its direction to be read: each end is right to an eighth of that, so the
/// leg's direction to an eighth of a radian.
const LEG_CLEAR: f64 = 2.0;

/// What a stretch of a spline edge keeps within about an axis
/// ([`Lay::About`]): its angle between the unit vectors `low` and `high`
/// across the axis (from `low` anticlockwise to `high`, less than a quarter
/// turn), its distance from the axis and its height along it from the
/// axis's origin between the two of `off` and of `height`.
#[derive(Clone, Copy, Debug)]
struct Reach {
    low: Vec3,
    high: Vec3,
    off: (f64, f64),
    height: (f64, f64),
}

/// The points of a stretch's cut Bézier pieces, piece by piece: where each
/// piece's points end in `points`, with how far a point of it must keep
/// clear (its rounding grown where the piece is continued past its span).
#[derive(Default)]
struct Held {
    points: Vec<Vec3>,
    runs: Vec<(usize, f64)>,
}

impl Held {
    fn clear(&mut self) {
        self.points.clear();
        self.runs.clear();
    }

    fn take(&mut self, points: &[Vec3], clear: f64) {
        self.points.extend_from_slice(points);
        self.runs.push((self.points.len(), clear));
    }
}

/// Directions in a plane taken one by one, held to a fan narrower than
/// `most` (radians, less than a half turn): all of them then lie on one
/// side of a line through the origin, and so does every sum of them with
/// weights that are not negative. No arc tangent is taken: the two
/// directions furthest round are kept, and a new one is told inside or
/// outside them by a cross product with each.
struct Fan {
    /// The cosine of the widest the fan may be.
    most: f64,
    /// The directions furthest round clockwise and anticlockwise (not of
    /// unit length); the second is never clockwise of the first.
    ends: Option<((f64, f64), (f64, f64))>,
}

impl Fan {
    fn within(most: f64) -> Self {
        Fan {
            most: most.pcos(),
            ends: None,
        }
    }

    /// Takes the direction `t`: whether the directions so far keep within
    /// the fan. Not for a direction of no length or one that is not a
    /// number.
    fn add(&mut self, t: (f64, f64)) -> bool {
        let size = t.0 * t.0 + t.1 * t.1;
        if !(size > 0.0 && size.is_finite()) {
            return false;
        }
        let cross = |a: (f64, f64), b: (f64, f64)| a.0 * b.1 - a.1 * b.0;
        let dot = |a: (f64, f64), b: (f64, f64)| a.0 * b.0 + a.1 * b.1;
        let Some((low, high)) = self.ends else {
            self.ends = Some((t, t));
            return true;
        };
        // Anticlockwise of the low end and clockwise of the high one, it
        // lies between them; clockwise of the low end and, by less than a
        // half turn, of the high one, it is the new low end, and likewise
        // the other way; else it lies across from the fan.
        let (from_low, to_high) = (cross(low, t), cross(t, high));
        let (low, high) = if from_low >= 0.0 && to_high >= 0.0 {
            // Along both ends: a fan of one direction, and `t` along it
            // or straight against it.
            let against = dot(low, t).partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater);
            if from_low == 0.0 && to_high == 0.0 && against {
                return false;
            }
            (low, high)
        } else if from_low < 0.0 && to_high > 0.0 {
            (t, high)
        } else if to_high < 0.0 && from_low > 0.0 {
            (low, t)
        } else {
            return false;
        };
        self.ends = Some((low, high));
        // The angle between the ends under the most, by its cosine and
        // their sizes' squares (no root).
        let between = dot(low, high);
        let sizes = dot(low, low) * dot(high, high);
        let squares = self.most * self.most * sizes;
        if self.most >= 0.0 {
            between > 0.0 && between * between > squares
        } else {
            between >= 0.0 || between * between < squares
        }
    }
}

/// Whether a spline edge runs one way in the chart `lay` over a stretch,
/// and the legs looked at. `points` are points made for the stretch, in
/// runs (`runs`: where each run ends in `points`, and how far a point of it
/// must keep clear): the control points of the spans the stretch meets, one
/// run, or of its Bézier pieces cut to it, a run each. The curve over the
/// stretch lies in their hull, and its direction at each of its points is a
/// sum, with weights that are not negative, of the legs from each point of
/// its span's run to the next (for a polynomial spline its derivative is a
/// spline on those legs; for a rational one with positive weights the
/// derivative is a sum over pairs of control points `i < j` of `Pj − Pi`
/// times `Ni Nj' − Ni' Nj`, which is not negative, the basis being totally
/// positive: de Boor, *A Practical Guide to Splines*, ch. XIII; for a Bézier
/// piece `Bi Bj' − Bi' Bj = Bi Bj (j − i) / (t (1 − t))`). The chart takes a
/// direction `w` at a point to a direction of its own by a linear map there
/// ([`Lay`]):
///
/// - a plane: `(w·x, w·y)`, the same everywhere;
/// - about an axis, at the angle `φ`, the distance `ρ` and the height `h`:
///   `u' = w·e(φ) / ρ` with `e` the unit vector round the axis there, and
///   `v'` as [`Rise`] has it, from `w·z` and `w·r(φ)`, `r` the unit vector
///   away from the axis. Over the stretch's `reach` each leg's image lies
///   in a box of such directions (the ranges of `w·e` and `w·r` over the
///   angles, the ends and a turning point between them; of `ρ` and `h`),
///   whose four corners stand for it;
///
/// and the image of the edge runs one way where all of those lie in a fan
/// narrower than a half turn ([`Fan`], [`ONE_WAY`]): its direction is then
/// a sum of them, on one side of a line all along. About an axis the legs
/// are first held to running round it one way (`w·e` of one sign at both
/// ends of the reach, so all along it: `u` moves one way), which costs two
/// products a leg and is all a helix needs; the boxes are made where that
/// fails. A leg of no length says
/// nothing and is passed over; a shorter one than [`LEG_CLEAR`] times its
/// clearance has no direction that can be read, and the stretch is not
/// shown. On a spline surface ([`Lay::Space`]) the legs are held to running
/// forward along the chord from the first point to the last, in space.
fn one_way(
    lay: &Lay,
    points: &[Vec3],
    runs: &[(usize, f64)],
    reach: Option<&Reach>,
) -> (bool, usize) {
    // Each leg with the clearance of its run.
    let legs = || {
        let mut starts = 0_usize;
        runs.iter().flat_map(move |&(end, clear)| {
            let run = points.get(starts..end).unwrap_or(&[]);
            starts = end;
            run.windows(2).filter_map(move |pair| match pair {
                [a, b] => Some((*b - *a, LEG_CLEAR * clear)),
                _ => None,
            })
        })
    };
    let mut looked = 0_usize;
    let nothing = |w: Vec3| w.x == 0.0 && w.y == 0.0 && w.z == 0.0;
    let longer = |a: f64, b: f64| a.partial_cmp(&b) == Some(std::cmp::Ordering::Greater);
    match lay {
        Lay::Flat { x, y } => {
            let mut fan = Fan::within(ONE_WAY);
            for (w, clear) in legs() {
                looked += 1;
                if nothing(w) {
                    continue;
                }
                let t = (w.dot(*x), w.dot(*y));
                if !(longer(t.0 * t.0 + t.1 * t.1, clear * clear) && fan.add(t)) {
                    return (false, looked);
                }
            }
            (true, looked)
        }
        Lay::About { z, rise, .. } => {
            let Some(reach) = reach else {
                return (false, 0);
            };
            let (near, far) = reach.off;
            let (below, above) = reach.height;
            if !(near > 0.0 && far.is_finite() && below.is_finite() && above.is_finite()) {
                return (false, 0);
            }
            // Where `v` cannot be read off the stretch (past a cone's
            // apex, too far round a torus's tube) nothing is shown.
            if !rise.keeps(reach) {
                return (false, 0);
            }
            let norm = match rise {
                Rise::Slant { slope, .. } => 1.0 / (slope * slope + 1.0).sqrt(),
                Rise::Height | Rise::Latitude | Rise::Tube { .. } => 1.0,
            };
            let (round_low, round_high) = (z.cross(reach.low), z.cross(reach.high));
            // First the angle about the axis alone, which is what a helix
            // or any edge that goes round takes: where every leg runs
            // round the axis the same way at both ends of the reach, it
            // does all along it (the ends are less than a quarter turn
            // apart), so `u` moves one way and the image keeps to one
            // side of a line of constant `u`. Two products a leg, less
            // than half of what a point looked at costs.
            let mut way = 0_i8;
            let mut round = true;
            for (w, clear) in legs() {
                looked += 1;
                if nothing(w) {
                    continue;
                }
                let (e0, e1) = (w.dot(round_low), w.dot(round_high));
                let now = if longer(e0, clear) && longer(e1, clear) {
                    1
                } else if longer(-e0, clear) && longer(-e1, clear) {
                    -1
                } else {
                    0
                };
                if now == 0 || (way != 0 && way != now) {
                    round = false;
                    break;
                }
                way = now;
            }
            looked = looked.div_ceil(2);
            if round {
                return (true, looked);
            }
            let widest = far / near;
            // The rate of a latitude, or of the angle about a tube, is over
            // the square of the distance from the centre it is taken about
            // (the sphere's, the tube's): between these, over the box of the
            // stretch's distances from the axis and its heights.
            let (inside, outside) = match rise {
                Rise::Tube { major } => (near - major, far - major),
                Rise::Height | Rise::Slant { .. } | Rise::Latitude => (near, far),
            };
            let span = |low: f64, high: f64| {
                let least = if low <= 0.0 && high >= 0.0 {
                    0.0
                } else {
                    low.abs().min(high.abs())
                };
                (least, low.abs().max(high.abs()))
            };
            let ((least_off, most_off), (least, most)) =
                (span(inside, outside), span(below, above));
            let (inner, outer) = (
                least_off * least_off + least * least,
                most_off * most_off + most * most,
            );
            let mut fan = Fan::within(ONE_WAY);
            for (w, clear) in legs() {
                looked += ABOUT_UNITS;
                if nothing(w) {
                    continue;
                }
                if !longer(w.length_squared(), clear * clear) {
                    return (false, looked);
                }
                let (e0, e1) = (w.dot(round_low), w.dot(round_high));
                let (r0, r1) = (w.dot(reach.low), w.dot(reach.high));
                let along = w.dot(*z);
                // Across the axis the leg is as long as this; `w·e` and
                // `w·r` reach it, either way, where the other one changes
                // its sign between the ends.
                let across = (e0 * e0 + r0 * r0).sqrt();
                let ranged = |a: f64, b: f64, turns: bool| {
                    let (mut low, mut high) = (a.min(b), a.max(b));
                    if turns {
                        if a >= 0.0 || b >= 0.0 {
                            high = high.max(across);
                        }
                        if a <= 0.0 || b <= 0.0 {
                            low = low.min(-across);
                        }
                    }
                    (low, high)
                };
                let round = ranged(e0, e1, r0 * r1 <= 0.0);
                let away = ranged(r0, r1, e0 * e1 <= 0.0);
                // `u'`, times the most distance from the axis: a length.
                let u = (round.0.min(round.0 * widest), round.1.max(round.1 * widest));
                // `v'`, as a length as well.
                let v = match rise {
                    Rise::Height => (along, along),
                    Rise::Slant { slope, .. } => {
                        let (a, b) = (slope * away.0, slope * away.1);
                        ((a.min(b) + along) * norm, (a.max(b) + along) * norm)
                    }
                    Rise::Latitude | Rise::Tube { .. } => {
                        // `(ρ w·z − h w·r) / (ρ² + h²)`, `ρ` from the axis
                        // or, about a tube, from its centre circle.
                        let (a, b) = (inside * along, outside * along);
                        let lifts = [
                            below * away.0,
                            below * away.1,
                            above * away.0,
                            above * away.1,
                        ];
                        let lift_low = lifts.iter().copied().fold(f64::INFINITY, f64::min);
                        let lift_high = lifts.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                        let (low, high) = (a.min(b) - lift_high, a.max(b) - lift_low);
                        let scale = outer.sqrt();
                        (
                            (low / inner).min(low / outer) * scale,
                            (high / inner).max(high / outer) * scale,
                        )
                    }
                };
                for t in [(u.0, v.0), (u.0, v.1), (u.1, v.0), (u.1, v.1)] {
                    if !fan.add(t) {
                        return (false, looked);
                    }
                }
            }
            (true, looked)
        }
        #[cfg(test)]
        Lay::Turning { .. } => (true, 0),
        Lay::Space => {
            let (Some(first), Some(last)) = (points.first(), points.last()) else {
                return (false, 0);
            };
            let chord = *last - *first;
            let length = chord.length();
            if !(length.is_finite() && length > 0.0) {
                return (false, 0);
            }
            for (w, clear) in legs() {
                looked += 1;
                if nothing(w) {
                    continue;
                }
                if !longer(w.dot(chord), clear * length) {
                    return (false, looked);
                }
            }
            (true, looked)
        }
    }
}

/// What a leg whose box of directions is made about an axis counts for
/// against [`MOST_PROOF_WORK`] ([`one_way`]): its five components and the
/// four corners of its box, about two points looked at.
const ABOUT_UNITS: usize = 2;

/// How many times the rounding of a curve's hull points the points of a
/// hull must keep from the axis ([`Wedge`]): the angle of a point about
/// the axis is then right to an eighth of a radian, 7°, and a wedge found
/// narrower than a quarter turn is narrower than 105°, well under the half
/// turn at which the chart would unwrap a step the wrong way.
const HULL_CLEAR: f64 = 8.0;

/// How much the rounding of a Bézier piece may grow where it is continued
/// past its span ([`growth`]) for the cut to be read: 2²⁰, which leaves a
/// millionth of a unit in the last place times the curve's size. Read off
/// a measurement of 5,883 splines (review of 2026-11-14): continued two
/// span widths past, a piece of degree 8 (a growth of 5⁸, 4e5) holds its
/// curve to 7e-12 of the cut's reach, one of degree 13 (5¹³, 1e9) to 7e-9,
/// of degree 21 to 1.5e-3, of degree 32 not at all.
const MOST_GROWTH: f64 = 1_048_576.0;

/// An upper bound on how far rounding can put a spline outside the convex
/// hull of the points made for a stretch of it (its control points, or
/// those of its Bézier pieces cut to the stretch inside their spans): 16
/// units in the last place of its largest coordinate for each control
/// point of a span (64 for a cubic: a piece is made by as many rounds of
/// blends as its degree, and so is a point of the curve), times the ratio
/// of its largest weight to its least (a rational spline is a quotient,
/// and loses that many figures). Measured on 5,883 splines (review of
/// 2026-11-14): the curve at most 2.4e-14 of its largest coordinate
/// outside the hull of a cut piece at degree 21 (this gives 7.8e-14), and
/// 8.6e-8 of its size for weights of 1e-6 to 1e6 at degree 5 (2e-2).
/// `None` for a weight that is not positive and finite: no hull of control
/// points holds such a curve.
fn hull_rounding(curve: &NurbsCurve) -> Option<f64> {
    let (mut least, mut most) = (f64::INFINITY, 0.0_f64);
    for w in curve.weights() {
        if !(w.is_finite() && *w > 0.0) {
            return None;
        }
        least = least.min(*w);
        most = most.max(*w);
    }
    let size = curve.control_points().iter().fold(0.0_f64, |m, p| {
        m.max(p.x.abs()).max(p.y.abs()).max(p.z.abs())
    });
    let points = (curve.degree() + 1) as f64;
    let rounding = 16.0 * points * f64::EPSILON * (most / least) * size;
    rounding.is_finite().then_some(rounding)
}

/// By how much the rounding of a Bézier piece of `degree` over `(lo, hi)`
/// grows when it is cut to `(from, to)` by de Casteljau's algorithm: 1
/// inside its span; past it, each of the `degree` rounds of blends by a
/// factor outside `[0, 1]` multiplies an error by the sum of the sizes of
/// its two weights, `1 + 2 s` for a cut `s` span widths before the span,
/// `2 s − 1` for one ending at `s` times the rest of it.
fn growth((lo, hi): (f64, f64), (from, to): (f64, f64), degree: usize) -> f64 {
    let rounds = i32::try_from(degree).unwrap_or(i32::MAX);
    let mut growth = 1.0_f64;
    if from < lo {
        growth *= (1.0 + 2.0 * (lo - from) / (hi - lo)).ppowi(rounds);
    }
    if to > hi {
        growth *= (2.0 * (to - from) / (hi - from) - 1.0).ppowi(rounds);
    }
    growth
}

/// The Bézier pieces of a spline edge, each made when a stretch first meets
/// its span and from that span alone (`NurbsCurve::bezier_piece`), so that
/// showing an edge's steps costs about its control points, where the pieces
/// of the whole curve (`NurbsCurve::bezier_pieces`) cost their square.
struct Pieces<'a> {
    curve: &'a NurbsCurve,
    made: BTreeMap<usize, Option<CurvePiece>>,
}

impl<'a> Pieces<'a> {
    fn of(curve: &'a NurbsCurve) -> Self {
        Pieces {
            curve,
            made: BTreeMap::new(),
        }
    }

    /// Hands `take` points whose convex hull holds the curve over `(a, b)`,
    /// piece by piece until it refuses one, each piece's with how much its
    /// rounding has grown ([`growth`]; 1 inside the domain): the control
    /// points of the Bézier piece of each span the stretch meets, cut to
    /// the stretch by de Casteljau's algorithm
    /// (`craft3d_geom::piece_points_over`), the first and the last piece of
    /// the curve continued past its domain as far as the stretch goes (as
    /// `NurbsCurve::eval` continues them). `Some(true)` when `take` took
    /// them all, `Some(false)` when it refused one or no such points are
    /// known: a stretch past the end of a domain that ends on a span of no
    /// width (knots not clamped, the end knot repeated: `eval` continues
    /// that empty span's polynomial there, not the last piece's), a piece
    /// continued so far that its rounding grows past [`MOST_GROWTH`], a
    /// rational piece continued to where a weight is not positive, a
    /// stretch wholly past the domain's end. `None` when the face's work
    /// is spent.
    fn each_piece(
        &mut self,
        (a, b): (f64, f64),
        work: &mut usize,
        mut take: impl FnMut(&[Vec3], f64) -> bool,
    ) -> Option<bool> {
        let curve = self.curve;
        let (d0, d1) = curve.domain();
        let (knots, degree) = (curve.knots(), curve.degree());
        // Past the domain's end `eval` continues the span that ends
        // there, which is the last piece only if that span has a width.
        let count = curve.control_points().len();
        let ends_wide = matches!(
            (knots.get(count.wrapping_sub(1)), knots.get(count)),
            (Some(x), Some(y)) if y > x
        );
        if b > d1 && !ends_wide {
            return Some(false);
        }
        let (first, last) = (curve.span_at(a), curve.span_at(b));
        let mut any = false;
        for k in first..=last {
            let (Some(&lo), Some(&hi)) = (knots.get(k), knots.get(k + 1)) else {
                return Some(false);
            };
            if hi.partial_cmp(&lo) != Some(std::cmp::Ordering::Greater) {
                continue;
            }
            // Within its span; past the domain's ends for the first and
            // the last span of the curve.
            let from = if lo <= d0 { a } else { a.max(lo) };
            let to = if hi >= d1 { b } else { b.min(hi) };
            if to.partial_cmp(&from) == Some(std::cmp::Ordering::Less) {
                continue;
            }
            if let std::collections::btree_map::Entry::Vacant(slot) = self.made.entry(k) {
                spend(work, (degree + 1).saturating_mul(degree + 1))?;
                slot.insert(curve.bezier_piece(k));
            }
            let Some(Some(piece)) = self.made.get(&k) else {
                return Some(false);
            };
            let growth = growth((lo, hi), (from, to), degree);
            if growth.partial_cmp(&MOST_GROWTH) != Some(std::cmp::Ordering::Less) {
                return Some(false);
            }
            let Some(points) = craft3d_geom::piece_points_over(piece, (from, to)) else {
                return Some(false);
            };
            spend(work, points.len())?;
            any |= !points.is_empty();
            if !take(&points, growth) {
                return Some(false);
            }
        }
        Some(any)
    }
}

/// Spans of a spline past which a stretch inside its domain is held by
/// the spans' control points alone, not by their Bézier pieces cut to it:
/// over many spans the two differ by little (the control points of the
/// spans at its two ends), and the pieces of every span of a long edge
/// cost more than following it.
const MANY_SPANS: usize = 16;

/// An upper bound on a polynomial spline's speed over its domain (its
/// weights all alike): the longest of its derivative's control points
/// `p (P[i+1] - P[i]) / (u[i+p+1] - u[i+1])` (*The NURBS Book* §3.3), whose
/// hull holds the derivative. `None` for a rational spline, or where a
/// derivative control point is not defined (`p + 1` equal knots inside: the
/// curve may jump there).
fn speed_bound(curve: &NurbsCurve) -> Option<f64> {
    let weights = curve.weights();
    let first = *weights.first()?;
    if weights.iter().any(|w| *w != first) {
        return None;
    }
    let (knots, ctrl, p) = (curve.knots(), curve.control_points(), curve.degree());
    let mut most = 0.0_f64;
    for (i, pair) in ctrl.windows(2).enumerate() {
        let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let lo = *knots.get(i + 1)?;
        let hi = *knots.get(i + p + 1)?;
        let gap = hi - lo;
        if !(gap.is_finite() && gap > 0.0) {
            return None;
        }
        most = most.max(p as f64 * (*b - *a).length() / gap);
    }
    most.is_finite().then_some(most)
}

/// An upper bound on how fast a polynomial spline's direction and speed
/// change over its knot span `k` (the length of its second derivative
/// there): the longest of the control points of its derivative's own
/// derivative that reach into the span,
/// `(p − 1) (Q[j+1] − Q[j]) / (u[j+p+1] − u[j+2])` for `j` from `k − p` to
/// `k − 2`, `Q[j] = p (P[j+1] − P[j]) / (u[j+p+1] − u[j+1])` the
/// derivative's (whose hull holds it; each of those gaps holds the span, so
/// none is empty for a span that has a width). 0 for a spline of degree 1,
/// a straight line over a span. `None` for a span the curve does not have
/// or a number that is none. For a spline whose weights are all alike
/// ([`speed_bound`]).
fn bend_bound(curve: &NurbsCurve, k: usize) -> Option<f64> {
    let (knots, ctrl, p) = (curve.knots(), curve.control_points(), curve.degree());
    if k >= ctrl.len() {
        return None;
    }
    let first = k.checked_sub(p)?;
    let rate = |j: usize| -> Option<Vec3> {
        let gap = *knots.get(j + p + 1)? - *knots.get(j + 1)?;
        (gap.is_finite() && gap > 0.0)
            .then(|| Some((*ctrl.get(j + 1)? - *ctrl.get(j)?) * (p as f64 / gap)))
            .flatten()
    };
    let mut most = 0.0_f64;
    let mut before = rate(first)?;
    for j in first..k.saturating_sub(1) {
        let gap = *knots.get(j + p + 1)? - *knots.get(j + 2)?;
        if !(gap.is_finite() && gap > 0.0) {
            return None;
        }
        let next = rate(j + 1)?;
        most = most.max((p as f64 - 1.0) * (next - before).length() / gap);
        before = next;
    }
    most.is_finite().then_some(most)
}

/// How far above nothing, as a share of the spline's speed bound, the rate
/// at which a stretch goes round its axis must be shown to keep
/// ([`spline_steps`], (2)): well above the rounding of a derivative.
const ROUND_CLEAR: f64 = 1e-9;

/// The angle about its axis the stretch of a spline edge over a half step
/// stays within ([`spline_steps`]): a quarter of the period, which is what
/// the walk reads as a half step that may have gone round unseen
/// ([`Ring::aliased`]).
const QUARTER_TURN: f64 = 0.5 * PI;

/// Points taken one by one, held to lying in a wedge about an axis
/// narrower than `most` (radians, a quarter turn at most), none on the
/// axis: their convex hull then keeps clear of the axis (every point lies
/// in one open half plane through it) and every point of the hull lies
/// within that wedge, so the angle about the axis moves by less than `most`
/// along a curve in it. Each point is taken with how far it must keep from
/// the axis: [`HULL_CLEAR`] times what rounding can have moved it, so that
/// a curve a rounding outside the hull of the points as computed still
/// turns about the axis as they say.
struct Wedge {
    origin: Vec3,
    z: Vec3,
    /// The cosine of the widest the wedge may be (not below 0: a quarter
    /// turn at most).
    most: f64,
    /// Across the axis: towards the first point, and a quarter turn on.
    frame: Option<(Vec3, Vec3)>,
    /// The points furthest round either way, as they lie across the axis
    /// in that frame (not of unit length).
    low: (f64, f64),
    high: (f64, f64),
    /// The squares of the least and the most distance of a point from the
    /// axis, and the least and the most height along it from its origin.
    off: (f64, f64),
    height: (f64, f64),
}

impl Wedge {
    fn about((origin, z): (Vec3, Vec3), most: f64) -> Self {
        Wedge {
            origin,
            z,
            most: most.pcos().max(0.0),
            frame: None,
            low: (1.0, 0.0),
            high: (1.0, 0.0),
            off: (f64::INFINITY, 0.0),
            height: (f64::INFINITY, f64::NEG_INFINITY),
        }
    }

    /// What the hull of the points taken keeps within ([`Reach`]), for a
    /// wedge that every point was taken into: the two ends of the wedge;
    /// no nearer the axis than the nearest point reaches along the wedge's
    /// middle, which is at least its distance times the cosine of half the
    /// wedge (a point of a hull is at least as far from the axis as it
    /// reaches along any one direction across it, and reaches at least as
    /// far along it as the least of the hull's own points), and no further
    /// than the furthest point (the distance from a line is largest at a
    /// corner of a hull); between the lowest and the highest point along
    /// the axis.
    fn reach(&self) -> Option<Reach> {
        let (x, y) = self.frame?;
        let unit = |(c, s): (f64, f64)| {
            let size = (c * c + s * s).sqrt();
            (size.is_finite() && size > 0.0).then(|| x * (c / size) + y * (s / size))
        };
        let (low, high) = (unit(self.low)?, unit(self.high)?);
        let half = (0.5 * (1.0 + low.dot(high).clamp(-1.0, 1.0))).sqrt();
        Some(Reach {
            low,
            high,
            off: (self.off.0.sqrt() * half, self.off.1.sqrt()),
            height: self.height,
        })
    }

    /// Takes `p`, which must keep more than `clear` from the axis: whether
    /// the points so far lie in such a wedge. No arc tangent is taken. A
    /// point must lie within a quarter turn of both ends of the wedge so
    /// far (a dot product each: a point further round, one straight across
    /// the axis among them, is no point of a wedge that narrow); such a
    /// point outside the wedge lies to the left of its last point or to
    /// the right of its first (a cross product each); and the wedge is
    /// narrow while the cosine of the angle between those two is above the
    /// cosine of the most (by their squares: no root, no division).
    fn add(&mut self, p: Vec3, clear: f64) -> bool {
        let d = p - self.origin;
        let height = d.dot(self.z);
        let q = d - self.z * height;
        let first = self.frame.is_none();
        let (x, y) = match self.frame {
            Some(frame) => frame,
            None => {
                let length = q.length();
                if !(length.is_finite() && length > 0.0) {
                    return false;
                }
                let x = q * (1.0 / length);
                let frame = (x, self.z.cross(x));
                self.frame = Some(frame);
                frame
            }
        };
        let (c, s) = (q.dot(x), q.dot(y));
        let off = c * c + s * s;
        // Not `<=`: a number that is none is refused too.
        if !(off > 0.0 && off > clear * clear && off.is_finite()) {
            return false;
        }
        self.off = (self.off.0.min(off), self.off.1.max(off));
        self.height = (self.height.0.min(height), self.height.1.max(height));
        if first {
            self.low = (c, s);
            self.high = (c, s);
        } else {
            // Within a quarter turn of both ends, or it is no point of the
            // wedge: the cross products below say on which side of an end
            // a point lies only within a half turn of it, and are both
            // nothing for a point straight across the axis from points
            // that lie on one ray.
            let near_low = self.low.0 * c + self.low.1 * s > 0.0;
            let near_high = self.high.0 * c + self.high.1 * s > 0.0;
            if !(near_low && near_high) {
                return false;
            }
            if self.high.0 * s - self.high.1 * c > 0.0 {
                self.high = (c, s);
            } else if self.low.0 * s - self.low.1 * c < 0.0 {
                self.low = (c, s);
            }
        }
        // The cosine of the angle between the two ends above that of the
        // most: by their squares, both being positive.
        let dot = self.low.0 * self.high.0 + self.low.1 * self.high.1;
        let sizes = (self.low.0 * self.low.0 + self.low.1 * self.low.1)
            * (self.high.0 * self.high.0 + self.high.1 * self.high.1);
        dot > 0.0 && dot * dot > self.most * self.most * sizes
    }
}

/// Whether `points` lie in a wedge about `axis` narrower than `most`
/// ([`Wedge`]); not for none.
#[cfg(test)]
fn narrow(points: &[Vec3], axis: (Vec3, Vec3), most: f64) -> bool {
    let mut wedge = Wedge::about(axis, most);
    !points.is_empty() && points.iter().all(|p| wedge.add(*p, 0.0))
}

impl Ring {
    /// How far `p` is from the box (0 inside it).
    fn box_distance(&self, p: [f64; 2]) -> f64 {
        let dx = (self.low[0] - p[0]).max(p[0] - self.high[0]).max(0.0);
        let dy = (self.low[1] - p[1]).max(p[1] - self.high[1]).max(0.0);
        dx.phypot(dy)
    }

    fn shift(&mut self, d: usize, by: f64) {
        for p in &mut self.points {
            if let Some(x) = p.get_mut(d) {
                *x += by;
            }
        }
        if let (Some(l), Some(h)) = (self.low.get_mut(d), self.high.get_mut(d)) {
            *l += by;
            *h += by;
        }
    }
}

pub(crate) fn signed_area(points: &[[f64; 2]]) -> f64 {
    let Some(o) = points.first() else { return 0.0 };
    let n = points.len();
    (0..n)
        .filter_map(|i| {
            let (a, b) = (points.get(i)?, points.get((i + 1) % n)?);
            Some((a[0] - o[0]) * (b[1] - o[1]) - (b[0] - o[0]) * (a[1] - o[1]))
        })
        .sum::<f64>()
        * 0.5
}

fn perimeter(points: &[[f64; 2]]) -> f64 {
    let n = points.len();
    (0..n)
        .filter_map(|i| {
            let (a, b) = (points.get(i)?, points.get((i + 1) % n)?);
            Some((b[0] - a[0]).phypot(b[1] - a[1]))
        })
        .sum()
}

/// The distance from `p` to the closed polyline `points`.
pub(crate) fn distance(p: [f64; 2], points: &[[f64; 2]]) -> f64 {
    let n = points.len();
    (0..n)
        .filter_map(|i| {
            let (a, b) = (points.get(i)?, points.get((i + 1) % n)?);
            Some(to_segment(p, *a, *b))
        })
        .fold(f64::INFINITY, f64::min)
}

fn to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length2 = dx * dx + dy * dy;
    let t = if length2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a[0] + t * dx - p[0]).phypot(a[1] + t * dy - p[1])
}

/// How many times the closed polyline `points` winds around `p`
/// anticlockwise.
fn winding(p: [f64; 2], points: &[[f64; 2]]) -> i64 {
    let n = points.len();
    let mut w = 0_i64;
    for i in 0..n {
        let (Some(a), Some(b)) = (points.get(i), points.get((i + 1) % n)) else {
            continue;
        };
        let side = (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]);
        if a[1] <= p[1] {
            if b[1] > p[1] && side > 0.0 {
                w += 1;
            }
        } else if b[1] <= p[1] && side < 0.0 {
            w -= 1;
        }
    }
    w
}

/// Signed distance of `p` from the line through `a` and `b` (left
/// positive), or `None` for a step of no length.
fn side(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> Option<f64> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx.phypot(dy);
    (length > 0.0).then(|| (dx * (p[1] - a[1]) - dy * (p[0] - a[0])) / length)
}

/// Whether the steps `a b` and `c d` cross properly, each one's ends on
/// either side of the other's line by more than `margin`.
fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2], margin: f64) -> bool {
    let apart = |x: Option<f64>, y: Option<f64>| match (x, y) {
        (Some(x), Some(y)) => (x > margin && y < -margin) || (x < -margin && y > margin),
        _ => false,
    };
    apart(side(a, b, c), side(a, b, d)) && apart(side(c, d, a), side(c, d, b))
}

/// The first pair of loops (or a loop and itself) whose steps cross.
fn crossing(rings: &[Ring], margin_of: &dyn Fn(usize, usize) -> f64) -> Option<(usize, usize)> {
    // Every step: (lowest u, highest u, lowest v, highest v, ring, index).
    let mut steps: Vec<(f64, f64, f64, f64, usize, usize)> = Vec::new();
    for (r, ring) in rings.iter().enumerate() {
        let n = ring.points.len();
        for i in 0..n {
            let (Some(a), Some(b)) = (ring.points.get(i), ring.points.get((i + 1) % n)) else {
                continue;
            };
            steps.push((
                a[0].min(b[0]),
                a[0].max(b[0]),
                a[1].min(b[1]),
                a[1].max(b[1]),
                r,
                i,
            ));
        }
    }
    let largest_margin = (0..rings.len())
        .map(|i| margin_of(i, i))
        .fold(0.0, f64::max)
        * 2.0;
    let point = |r: usize, i: usize| -> Option<([f64; 2], [f64; 2])> {
        let ring = rings.get(r)?;
        let n = ring.points.len();
        Some((*ring.points.get(i)?, *ring.points.get((i + 1) % n)?))
    };
    // Bands across v, each step in every band it reaches (with the
    // margin), each band swept along u: steps far apart in v are never
    // paired, so a face of many holes in rows and columns (a perforated
    // tube) costs about its steps, not their square.
    let (v_low, v_high) = steps
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), s| {
            (l.min(s.2), h.max(s.3))
        });
    let count = ((steps.len() as f64).sqrt().ceil() as usize).clamp(1, 512);
    let height = (v_high - v_low) / count as f64;
    let band_of = |v: f64| {
        let f = (v - v_low) / height;
        if f.is_finite() && f > 0.0 {
            (f as usize).min(count - 1)
        } else {
            0
        }
    };
    let mut bands: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (k, st) in steps.iter().enumerate() {
        for b in band_of(st.2 - largest_margin)..=band_of(st.3 + largest_margin) {
            if let Some(band) = bands.get_mut(b) {
                band.push(k);
            }
        }
    }
    let mut work = 0_usize;
    for band in &mut bands {
        band.sort_by(|x, y| {
            let (Some(x), Some(y)) = (steps.get(*x), steps.get(*y)) else {
                return std::cmp::Ordering::Equal;
            };
            x.0.total_cmp(&y.0).then(x.4.cmp(&y.4)).then(x.5.cmp(&y.5))
        });
        let mut active: Vec<(f64, usize, usize)> = Vec::new();
        for k in band.iter() {
            let Some(&(low, high, _, _, r, i)) = steps.get(*k) else {
                continue;
            };
            active.retain(|(h, _, _)| *h + largest_margin >= low);
            let (a, b) = point(r, i)?;
            work = work.saturating_add(active.len().saturating_add(1));
            if work > MOST_PAIRS {
                return None;
            }
            for &(_, s, j) in &active {
                if s == r {
                    let n = rings.get(r).map_or(0, |x| x.points.len());
                    // Steps that meet at a point of the loop.
                    if j == i || (j + 1) % n == i || (i + 1) % n == j {
                        continue;
                    }
                }
                let (c, d) = point(s, j)?;
                if cross(a, b, c, d, margin_of(r, s)) {
                    return Some((s.min(r), s.max(r)));
                }
            }
            active.push((high, r, i));
        }
    }
    None
}

/// The loops by the cells of a grid over their boxes (grown by a margin):
/// the loops whose boxes hold a point are among those of its cell.
struct Grid {
    low: [f64; 2],
    size: [f64; 2],
    n: usize,
    cells: Vec<Vec<usize>>,
    /// Every loop, for a point outside the grid.
    all: Vec<usize>,
}

impl Grid {
    fn new(rings: &[Ring], margin: f64) -> Grid {
        let mut low = [f64::INFINITY; 2];
        let mut high = [f64::NEG_INFINITY; 2];
        for r in rings {
            low = [low[0].min(r.low[0] - margin), low[1].min(r.low[1] - margin)];
            high = [
                high[0].max(r.high[0] + margin),
                high[1].max(r.high[1] + margin),
            ];
        }
        let n = ((rings.len() as f64).sqrt().ceil() as usize).clamp(1, 256);
        let size = [
            ((high[0] - low[0]) / n as f64).max(f64::MIN_POSITIVE),
            ((high[1] - low[1]) / n as f64).max(f64::MIN_POSITIVE),
        ];
        let mut grid = Grid {
            low,
            size,
            n,
            cells: vec![Vec::new(); n * n],
            all: (0..rings.len()).collect(),
        };
        if !(size[0].is_finite() && size[1].is_finite()) {
            grid.n = 0;
            return grid;
        }
        for (j, r) in rings.iter().enumerate() {
            let a = grid.cell([r.low[0] - margin, r.low[1] - margin]);
            let b = grid.cell([r.high[0] + margin, r.high[1] + margin]);
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    if let Some(cell) = grid.cells.get_mut(x * n + y) {
                        cell.push(j);
                    }
                }
            }
        }
        grid
    }

    fn cell(&self, p: [f64; 2]) -> [usize; 2] {
        let at = |x: f64, low: f64, size: f64| {
            let f = (x - low) / size;
            if f.is_finite() && f > 0.0 {
                (f as usize).min(self.n.saturating_sub(1))
            } else {
                0
            }
        };
        let [px, py] = p;
        let ([lx, ly], [sx, sy]) = (self.low, self.size);
        [at(px, lx, sx), at(py, ly, sy)]
    }

    fn at(&self, p: [f64; 2]) -> &[usize] {
        if self.n == 0 {
            return &self.all;
        }
        let [x, y] = self.cell(p);
        self.cells
            .get(x * self.n + y)
            .map_or(self.all.as_slice(), Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use craft3d_math::Frame;
    use std::f64::consts::{FRAC_PI_2, TAU};

    /// A chart of which only the angle about `axis` is shown, for the tests
    /// of that proof alone.
    fn turning((origin, z): (Vec3, Vec3)) -> Lay {
        Lay::Turning { origin, z }
    }

    fn tilted() -> Frame {
        Frame::new(
            Vec3::new(3.0, -1.0, 2.0),
            Vec3::new(0.2, -0.5, 0.84),
            Vec3::new(0.9, 0.3, 0.1),
        )
        .unwrap()
    }

    /// The point at `angle` about `frame`'s z axis, `radius` from it and
    /// `height` along it.
    fn about(frame: &Frame, radius: f64, angle: f64, height: f64) -> Vec3 {
        let (s, c) = angle.psin_cos();
        frame.origin() + frame.x() * (radius * c) + frame.y() * (radius * s) + frame.z() * height
    }

    /// A curve going `quarters` quarter turns about `frame`'s z axis at
    /// `radius`, rising `rise` a turn: rational quadratic quarter arcs, so
    /// it lies on the cylinder of that radius exactly.
    fn winding(frame: &Frame, radius: f64, quarters: usize, rise: f64) -> NurbsCurve {
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        let mut knots = vec![0.0; 3];
        let h = |a: f64| rise * a / TAU;
        for k in 0..quarters {
            let (a0, a1) = (k as f64 * FRAC_PI_2, (k + 1) as f64 * FRAC_PI_2);
            if k == 0 {
                ctrl.push(about(frame, radius, a0, h(a0)));
                weights.push(1.0);
            }
            let middle = 0.5 * (a0 + a1);
            ctrl.push(about(frame, radius * 2.0_f64.sqrt(), middle, h(middle)));
            weights.push(0.5 * 2.0_f64.sqrt());
            ctrl.push(about(frame, radius, a1, h(a1)));
            weights.push(1.0);
            let knot = (k + 1) as f64;
            if k + 1 < quarters {
                knots.extend([knot, knot]);
            } else {
                knots.extend([knot; 3]);
            }
        }
        NurbsCurve::new(2, knots, ctrl, weights).unwrap()
    }

    /// The angle about `frame`'s z axis swept along the curve over `(a, b)`,
    /// sampled finely and unwrapped step by step.
    fn swept(curve: &NurbsCurve, frame: &Frame, (a, b): (f64, f64)) -> f64 {
        let angle = |t: f64| {
            let d = curve.eval(t) - frame.origin();
            d.dot(frame.y()).patan2(d.dot(frame.x()))
        };
        let (mut low, mut high, mut at, mut last) = (0.0_f64, 0.0_f64, 0.0_f64, angle(a));
        for k in 1..=256 {
            let now = angle(a + (b - a) * k as f64 / 256.0);
            at += (now - last + PI).rem_euclid(TAU) - PI;
            last = now;
            low = low.min(at);
            high = high.max(at);
        }
        high - low
    }

    #[test]
    fn points_in_a_wedge_about_the_axis_narrower_than_a_quarter_turn_are_narrow() {
        let f = tilted();
        let axis = (f.origin(), f.z());
        let at = |degrees: &[f64]| -> Vec<Vec3> {
            degrees
                .iter()
                .enumerate()
                .map(|(k, d)| about(&f, 1.0 + k as f64, d.to_radians(), k as f64))
                .collect()
        };
        assert!(narrow(&at(&[10.0, 50.0, 89.0]), axis, QUARTER_TURN));
        // Across the half turn, where the angle's reading jumps.
        assert!(narrow(&at(&[170.0, -170.0, 175.0]), axis, QUARTER_TURN));
        assert!(!narrow(&at(&[0.0, 91.0]), axis, QUARTER_TURN));
        assert!(!narrow(&at(&[0.0, 120.0, 240.0]), axis, QUARTER_TURN));
        // A point on the axis, a point that is not a number, none.
        let mut on = at(&[0.0, 10.0]);
        on.push(f.origin() + f.z() * 5.0);
        assert!(!narrow(&on, axis, QUARTER_TURN));
        assert!(!narrow(
            &[Vec3::new(f64::NAN, 0.0, 0.0), about(&f, 1.0, 0.1, 0.0)],
            axis,
            QUARTER_TURN
        ));
        assert!(!narrow(&[], axis, QUARTER_TURN));
    }

    /// The steps a spline edge going round an axis is followed in move less
    /// than a quarter turn from each point followed to the next (each half
    /// step's stretch sampled finely), in a frame turned and moved, at three
    /// scales; one turn keeps the eight steps it always had.
    #[test]
    fn a_spline_going_round_is_followed_less_than_a_quarter_turn_a_step() {
        let f = tilted();
        for scale in [1e-3, 1.0, 1e6] {
            for quarters in [1, 4, 5, 37, 400] {
                let curve = winding(&f, scale, quarters, 0.7 * scale);
                let range = curve.domain();
                let steps =
                    spline_steps(&curve, range, &turning((f.origin(), f.z())), &mut 0).unwrap();
                if quarters <= 4 {
                    assert_eq!(steps, SPLINE_STEPS, "{quarters}");
                }
                let halves = 2 * steps;
                for k in 0..halves {
                    let end = |j: usize| range.0 + (range.1 - range.0) * (j as f64 / halves as f64);
                    let sweep = swept(&curve, &f, (end(k), end(k + 1)));
                    assert!(
                        sweep < QUARTER_TURN,
                        "{quarters} quarter turns at {scale}: half step {k} of {halves} sweeps {sweep}"
                    );
                }
            }
        }
    }

    /// A spline through the axis cannot be shown to move less than a
    /// quarter turn a step however fine the steps: it is not followed. (It
    /// crosses the axis at `√2 − 1`, where no halving of its range ends.)
    #[test]
    fn a_spline_through_the_axis_is_not_followed() {
        let f = tilted();
        let curve = NurbsCurve::polynomial(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![
                about(&f, 1.0, 0.0, 0.0),
                f.origin() + f.z() * 0.5,
                about(&f, 2.0, PI, 1.0),
            ],
        )
        .unwrap();
        let axis = (f.origin(), f.z());
        assert_eq!(
            spline_steps(&curve, curve.domain(), &turning(axis), &mut 0),
            None
        );
        // Its first fifth keeps clear of the axis: followed in eight steps.
        assert_eq!(
            spline_steps(&curve, (0.0, 0.2), &turning(axis), &mut 0),
            Some(SPLINE_STEPS)
        );
        // A range past the curve's domain is the first and the last piece
        // continued: from before its start to its first fifth it keeps
        // clear of the axis, to its end it goes through it. (Until
        // 2026-11-14 such a range was followed in eight steps whatever the
        // curve did there, as every spline edge was before.)
        assert_eq!(
            spline_steps(&curve, (-0.1, 0.2), &turning(axis), &mut 0),
            Some(SPLINE_STEPS)
        );
        assert_eq!(
            spline_steps(&curve, (-0.5, 1.0), &turning(axis), &mut 0),
            None
        );
    }

    /// A cubic through a helix of `turns` turns of radius 1 and pitch 1
    /// about `frame`'s z at `count` points evenly along it.
    fn fitted_helix(frame: &Frame, turns: f64, count: usize) -> NurbsCurve {
        let params: Vec<f64> = (0..count).map(|i| i as f64 / (count - 1) as f64).collect();
        let values: Vec<Vec<f64>> = params
            .iter()
            .map(|s| {
                let p = about(frame, 1.0, TAU * turns * s, turns * s);
                vec![p.x, p.y, p.z]
            })
            .collect();
        let (knots, ctrl) = craft3d_geom::refine::interpolate(&params, 3, &values).unwrap();
        let points = ctrl.iter().map(|c| Vec3::new(c[0], c[1], c[2])).collect();
        NurbsCurve::polynomial(3, knots, points).unwrap()
    }

    /// Showing the steps of a spline edge costs about its control points,
    /// not their square (until 2026-11-14 the Bézier pieces of the whole
    /// curve were made where a stretch was not shown by its control points,
    /// `n²`: 49 s for an edge of 16384): counted in points looked at, never
    /// timed. A helix of 64 turns through 1024, 4096 and 16384 points is
    /// shown in the same steps, for work that grows no faster than the
    /// points do; so is a rational one of 256 quarter turns, which needs
    /// its Bézier pieces cut.
    #[test]
    fn showing_the_steps_of_a_spline_edge_costs_about_its_control_points() {
        let f = tilted();
        let axis = (f.origin(), f.z());
        let mut costs = Vec::new();
        for count in [1024, 4096, 16384] {
            let curve = fitted_helix(&f, 64.0, count);
            let mut work = 0;
            let steps = spline_steps(&curve, curve.domain(), &turning(axis), &mut work).unwrap();
            assert_eq!(steps, 256, "{count}");
            costs.push((count, work));
        }
        for pair in costs.windows(2) {
            let ((n0, w0), (n1, w1)) = (pair[0], pair[1]);
            assert!(
                w1 * n0 <= 2 * w0 * n1,
                "{n0} points cost {w0}, {n1} cost {w1}: more than twice as much a point"
            );
        }
        println!("costs {costs:?}");
        assert!(costs.iter().all(|(n, w)| *w <= 32 * n), "{costs:?}");
        let rational = winding(&f, 1.0, 256, 1.0);
        let mut work = 0;
        let steps = spline_steps(&rational, rational.domain(), &turning(axis), &mut work).unwrap();
        assert!((256..=1024).contains(&steps), "{steps}");
        println!("rational {work} {steps}");
        assert!(work <= 64 * 513, "{work}");
    }

    /// A face whose steps cost more to show than [`MOST_PROOF_WORK`] is not
    /// followed, and counted ([`step_proofs`]): here an edge handed what is
    /// left of a face's work.
    #[test]
    fn an_edge_past_the_work_of_its_face_is_not_followed_and_is_counted() {
        let f = tilted();
        let axis = (f.origin(), f.z());
        let curve = fitted_helix(&f, 64.0, 4096);
        let (spent, cut) = step_proofs();
        let mut work = 0;
        assert_eq!(
            spline_steps(&curve, curve.domain(), &turning(axis), &mut work),
            Some(256)
        );
        let (now, still) = step_proofs();
        assert_eq!(now - spent, work as u64);
        assert_eq!(still, cut);
        // The same edge with less left than it took.
        let mut left = MOST_PROOF_WORK - work / 2;
        assert_eq!(
            spline_steps(&curve, curve.domain(), &turning(axis), &mut left),
            None
        );
        assert!(left > MOST_PROOF_WORK);
        assert_eq!(step_proofs().1, cut + 1);
    }

    /// Past the end of a domain that ends on a span of no width (knots not
    /// clamped, the end knot repeated: degree 2 on 0, 0, 0, 1, 1, 2, 3)
    /// `eval` continues that empty span's polynomial, which is not the last
    /// piece's: a tenth past the end the two are a quarter of a unit apart
    /// on a curve three units long, and the curve is outside the box of the
    /// last piece cut that far. A stretch past such an end is not shown, so
    /// the edge is not followed; until 2026-11-14 (the second review) it
    /// was shown by the last piece continued, in eight steps. Inside the
    /// domain, and past its start (where `eval` does continue the first
    /// piece), it keeps its eight steps.
    #[test]
    fn a_stretch_past_a_domain_that_ends_on_an_empty_span_is_not_shown() {
        // Far from the curve, so that every stretch of it is narrow.
        let axis = (Vec3::new(1.5, -30.0, 0.0), Vec3::new(0.0, 0.0, 1.0));
        let points = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 2.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(3.0, 0.5, 4.0),
        ];
        let open =
            NurbsCurve::polynomial(2, vec![0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 3.0], points).unwrap();
        assert_eq!(open.domain(), (0.0, 1.0));
        // What `eval` gives past the end is not the last piece continued.
        let last = open.bezier_piece(open.span_at(1.1)).unwrap();
        assert_eq!(last.0, (0.0, 1.0));
        let cut = craft3d_geom::piece_points_over(&last, (0.75, 1.1)).unwrap();
        let there = open.eval(1.1);
        let end = cut.last().copied().unwrap();
        assert!(there.distance(end) > 0.2, "{there:?} {end:?}");
        // So no stretch past that end is shown, however short.
        assert_eq!(
            spline_steps(&open, (0.1, 1.1), &turning(axis), &mut 0),
            None
        );
        assert_eq!(
            spline_steps(&open, (0.1, 1.0 + 1e-9), &turning(axis), &mut 0),
            None
        );
        assert_eq!(
            spline_steps(&open, (0.1, 1.0), &turning(axis), &mut 0),
            Some(SPLINE_STEPS)
        );
        assert_eq!(
            spline_steps(&open, (-1e-9, 1.0), &turning(axis), &mut 0),
            Some(SPLINE_STEPS)
        );
        // Counted as a face not followed for a bound.
        let cut = step_proofs().1;
        assert_eq!(
            spline_steps(&open, (0.1, 1.1), &turning(axis), &mut 0),
            None
        );
        assert_eq!(step_proofs().1, cut + 1);
    }

    /// A piece is continued past its curve's domain only as far as rounding
    /// lets its cut be read ([`MOST_GROWTH`]): an edge of degree 21 that
    /// runs one span width past its curve is not shown (each of the 21
    /// rounds of the cut triples an error: 3²¹, 1e10), where until
    /// 2026-11-14 (the second review) it was, by points that no longer
    /// held the curve; a billionth past, or a whole width past at degree
    /// 5, it is.
    #[test]
    fn a_piece_is_continued_past_its_domain_only_as_far_as_its_rounding_allows() {
        let f = tilted();
        let axis = (f.origin(), f.z());
        // One span: points along a twentieth of a turn at radius 3.
        let arc = |degree: usize| {
            let points: Vec<Vec3> = (0..=degree)
                .map(|k| about(&f, 3.0, 0.3 * k as f64 / degree as f64, 0.1 * k as f64))
                .collect();
            let mut knots = vec![0.0; degree + 1];
            knots.extend(std::iter::repeat_n(1.0, degree + 1));
            NurbsCurve::polynomial(degree, knots, points).unwrap()
        };
        let high = arc(21);
        assert_eq!(
            spline_steps(&high, (-1.0, 1.0), &turning(axis), &mut 0),
            None
        );
        assert_eq!(
            spline_steps(&high, (0.0, 2.0), &turning(axis), &mut 0),
            None
        );
        assert_eq!(
            spline_steps(&high, (-1e-9, 1.0 + 1e-9), &turning(axis), &mut 0),
            Some(SPLINE_STEPS)
        );
        assert_eq!(
            spline_steps(&high, (0.0, 1.0), &turning(axis), &mut 0),
            Some(SPLINE_STEPS)
        );
        let low = arc(5);
        assert!(spline_steps(&low, (-1.0, 1.0), &turning(axis), &mut 0).is_some());
        // The growth itself: none inside the span, the sum of the sizes of
        // a blend's two weights to the degree past it.
        assert_eq!(growth((0.0, 1.0), (0.25, 0.75), 21), 1.0);
        assert_eq!(growth((0.0, 1.0), (-1.0, 0.5), 2), 9.0);
        assert_eq!(growth((2.0, 4.0), (3.0, 6.0), 3), 125.0);
        assert!(growth((0.0, 1.0), (-1.0, 1.0), 21) > MOST_GROWTH);
        assert!(growth((0.0, 1.0), (-1e-9, 1.0), 21) < 1.0 + 1e-6);
    }

    /// The points of a hull keep clear of the axis by what rounding can
    /// have moved them ([`hull_rounding`], [`HULL_CLEAR`]): a rational edge
    /// whose weights span twelve orders is shown only a hundredth of its
    /// size and more from the axis, an even one as near as rounding goes.
    #[test]
    fn a_hull_keeps_clear_of_the_axis_by_its_rounding() {
        let f = tilted();
        let axis = (f.origin(), f.z());
        let size = f
            .origin()
            .x
            .abs()
            .max(f.origin().y.abs())
            .max(f.origin().z.abs());
        // A short arc at `radius`, its middle weight `weight`.
        let arc = |radius: f64, weight: f64| {
            NurbsCurve::new(
                2,
                vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                vec![
                    about(&f, radius, 0.0, 0.0),
                    about(&f, radius, 0.1, 0.0),
                    about(&f, radius, 0.2, 0.0),
                ],
                vec![1e-6, weight, 1e-6],
            )
            .unwrap()
        };
        let even = arc(1e-9, 1e-6);
        let rounding = hull_rounding(&even).unwrap();
        assert!(rounding > 0.0 && rounding < 1e-12 * size.max(1.0) * 64.0);
        assert_eq!(
            spline_steps(&even, even.domain(), &turning(axis), &mut 0),
            Some(SPLINE_STEPS)
        );
        let uneven = arc(1e-9, 1e6);
        let rounding = hull_rounding(&uneven).unwrap();
        assert!(rounding > 1e-3 && rounding < 1e-1, "{rounding}");
        assert_eq!(
            spline_steps(&uneven, uneven.domain(), &turning(axis), &mut 0),
            None
        );
        let clear = arc(4.0, 1e6);
        assert_eq!(
            spline_steps(&clear, clear.domain(), &turning(axis), &mut 0),
            Some(SPLINE_STEPS)
        );
        // The wedge itself.
        let mut wedge = Wedge::about(axis, QUARTER_TURN);
        assert!(wedge.add(about(&f, 1.0, 0.0, 0.0), 0.5));
        assert!(!wedge.add(about(&f, 0.4, 0.1, 0.0), 0.5));
    }

    /// A point straight across the axis from the points taken so far is no
    /// point of a wedge narrower than a quarter turn. While those lie on
    /// one ray from the axis the two cross products that tell on which
    /// side of the wedge's ends a point lies are both nothing for it, so
    /// between 342a11ca and this test the wedge took it and read as narrow,
    /// and a curve through the axis was shown: the cubic through 27 points
    /// of a great circle of the sphere of radius 5 at the world's origin,
    /// in the plane y = 0, from 0.6 radians above the equator over the
    /// pole to 0.9 above it on the other side, was followed in eight steps
    /// about the world's z. It is not shown, and counted.
    #[test]
    fn a_point_across_the_axis_is_no_point_of_the_wedge() {
        let axis = (Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0));
        let mut wedge = Wedge::about(axis, QUARTER_TURN);
        assert!(wedge.add(Vec3::new(1.0, 0.0, 0.0), 0.0));
        assert!(wedge.add(Vec3::new(2.0, 0.0, 0.5), 0.0));
        assert!(!wedge.add(Vec3::new(-1.0, 0.0, 0.7), 0.0));
        // More than a quarter turn from the one end, not from the other.
        let mut wedge = Wedge::about(axis, QUARTER_TURN);
        assert!(wedge.add(Vec3::new(1.0, 0.0, 0.0), 0.0));
        assert!(wedge.add(Vec3::new(1.0, 1.0, 0.0), 0.0));
        assert!(!wedge.add(Vec3::new(-0.1, 1.0, 0.0), 0.0));
        let count = 27;
        let params: Vec<f64> = (0..count).map(|i| i as f64 / (count - 1) as f64).collect();
        let values: Vec<Vec<f64>> = params
            .iter()
            .map(|t| {
                let (sin, cos) = (0.6 + (PI - 1.5) * t).psin_cos();
                vec![5.0 * cos, 0.0, 5.0 * sin]
            })
            .collect();
        let (knots, ctrl) = craft3d_geom::refine::interpolate(&params, 3, &values).unwrap();
        let points: Vec<Vec3> = ctrl.iter().map(|c| Vec3::new(c[0], c[1], c[2])).collect();
        assert!(points.iter().all(|q| q.y == 0.0));
        let over = NurbsCurve::polynomial(3, knots, points).unwrap();
        let cut = step_proofs().1;
        assert_eq!(
            spline_steps(&over, over.domain(), &turning(axis), &mut 0),
            None
        );
        assert_eq!(step_proofs().1, cut + 1);
        // Either side of the pole it is shown as it was.
        assert_eq!(
            spline_steps(&over, (0.0, 0.4), &turning(axis), &mut 0),
            Some(SPLINE_STEPS)
        );
        assert_eq!(
            spline_steps(&over, (0.6, 1.0), &turning(axis), &mut 0),
            Some(SPLINE_STEPS)
        );
    }

    /// An edge that is not shown within the most steps an edge may take is
    /// counted as a face not followed for a bound ([`step_proofs`]), once a
    /// face: here the spline through the axis, twice on one face's work.
    #[test]
    fn an_edge_not_shown_within_its_steps_is_counted_once_a_face() {
        let f = tilted();
        let axis = (f.origin(), f.z());
        let curve = NurbsCurve::polynomial(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![
                about(&f, 1.0, 0.0, 0.0),
                f.origin() + f.z() * 0.5,
                about(&f, 2.0, PI, 1.0),
            ],
        )
        .unwrap();
        let cut = step_proofs().1;
        let mut work = 0;
        assert_eq!(
            spline_steps(&curve, curve.domain(), &turning(axis), &mut work),
            None
        );
        assert_eq!(step_proofs().1, cut + 1);
        assert_eq!(
            spline_steps(&curve, curve.domain(), &turning(axis), &mut work),
            None
        );
        assert_eq!(step_proofs().1, cut + 1);
    }

    /// The speed bound of a polynomial spline is above its speed sampled
    /// finely; a rational spline, or one that may jump at a knot, has none.
    #[test]
    fn a_polynomial_spline_moves_no_faster_than_its_speed_bound() {
        let points = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 2.0, 0.5),
            Vec3::new(3.0, -1.0, 2.0),
            Vec3::new(4.0, 4.0, -1.0),
            Vec3::new(6.0, 0.0, 0.0),
            Vec3::new(7.0, 1.0, 3.0),
        ];
        let knots = vec![0.0, 0.0, 0.0, 0.0, 0.3, 0.45, 1.0, 1.0, 1.0, 1.0];
        let curve = NurbsCurve::polynomial(3, knots, points.clone()).unwrap();
        let bound = speed_bound(&curve).unwrap();
        let fastest = (0..=4096)
            .map(|k| curve.derivatives(k as f64 / 4096.0).1.length())
            .fold(0.0_f64, f64::max);
        assert!(
            fastest <= bound && bound < 4.0 * fastest,
            "{fastest} {bound}"
        );
        let rational = winding(&tilted(), 1.0, 4, 1.0);
        assert_eq!(speed_bound(&rational), None);
        // Four equal knots inside a cubic: the curve may jump there.
        let jumps = NurbsCurve::polynomial(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0],
            [points.clone(), points[..2].to_vec()].concat(),
        )
        .unwrap();
        assert_eq!(speed_bound(&jumps), None);
    }

    /// The bound on how fast a polynomial spline's direction changes over
    /// a knot span is above its second derivative there, sampled (as the
    /// change of its first over a short way): on 300 splines of degrees 1
    /// to 7, their knots clamped or not and repeated.
    #[test]
    fn a_polynomial_splines_direction_changes_no_faster_than_its_bound() {
        let mut spans = 0;
        for seed in 0..300_u64 {
            let degree = 1 + (seed % 7) as usize;
            let curve = wandering(seed, degree, degree + 2 + (seed % 9) as usize, false);
            let knots = curve.knots();
            let (d0, d1) = curve.domain();
            for k in degree..curve.control_points().len() {
                let (lo, hi) = (knots[k], knots[k + 1]);
                if !(hi > lo && lo >= d0 && hi <= d1) {
                    assert!(hi <= lo || bend_bound(&curve, k).is_some());
                    continue;
                }
                let bound = bend_bound(&curve, k).unwrap();
                spans += 1;
                let h = 1e-4 * (hi - lo);
                let mut fastest = 0.0_f64;
                for j in 1..64 {
                    let t = lo + (hi - lo) * j as f64 / 64.0;
                    let change = curve.derivatives(t + h).1 - curve.derivatives(t - h).1;
                    fastest = fastest.max(change.length() / (2.0 * h));
                }
                assert!(
                    fastest <= bound * (1.0 + 1e-6) + 1e-6,
                    "seed {seed}, degree {degree}, span {k}: {fastest} above {bound}"
                );
            }
            assert_eq!(bend_bound(&curve, 0), None);
            assert_eq!(bend_bound(&curve, usize::MAX), None);
        }
        assert!(spans > 1000, "{spans}");
    }

    fn turn(degrees: f64, length: f64) -> (f64, f64) {
        let (s, c) = degrees.to_radians().psin_cos();
        (length * c, length * s)
    }

    /// Whether the directions lie in a fan narrower than `most`, taken in
    /// this order.
    fn fanned(directions: &[(f64, f64)], most: f64) -> bool {
        let mut fan = Fan::within(most);
        directions.iter().all(|t| fan.add(*t))
    }

    /// A fan takes directions that keep within its most, in any order and
    /// across the half turn where an angle's reading jumps, and none across
    /// from it, of no length or that is not a number.
    #[test]
    fn directions_within_less_than_a_half_turn_are_a_fan() {
        let at = |degrees: &[f64]| -> Vec<(f64, f64)> {
            degrees
                .iter()
                .enumerate()
                .map(|(k, d)| turn(*d, 1.0 + 3.0 * k as f64))
                .collect()
        };
        assert!(fanned(&at(&[10.0, 100.0, 144.0, 60.0]), ONE_WAY));
        assert!(fanned(&at(&[144.0, 10.0, 60.0, 100.0]), ONE_WAY));
        assert!(fanned(&at(&[170.0, -170.0, -100.0, 175.0]), ONE_WAY));
        assert!(!fanned(&at(&[10.0, 146.0]), ONE_WAY));
        assert!(!fanned(&at(&[146.0, 10.0]), ONE_WAY));
        assert!(!fanned(&at(&[0.0, 100.0, 200.0]), ONE_WAY));
        assert!(!fanned(&at(&[0.0, 120.0, 240.0]), ONE_WAY));
        // Straight against a fan of one direction, and against a fan's end.
        assert!(!fanned(&[(1.0, 0.0), (-2.0, 0.0)], ONE_WAY));
        assert!(!fanned(&[(1.0, 0.0), (3.0, 0.0), (-2.0, 0.0)], ONE_WAY));
        assert!(!fanned(&at(&[0.0, 90.0, 180.0]), ONE_WAY));
        assert!(fanned(&[(1.0, 0.0), (3.0, 0.0), (0.5, 0.0)], ONE_WAY));
        // Up to just under a half turn where that is asked.
        assert!(fanned(&at(&[0.0, 90.0, 179.0]), 0.999 * PI));
        assert!(!fanned(&at(&[0.0, 90.0, 181.0]), 0.999 * PI));
        assert!(!fanned(&[(1.0, 0.0), (0.0, 0.0)], ONE_WAY));
        assert!(!fanned(&[(0.0, 0.0)], ONE_WAY));
        assert!(!fanned(&[(1.0, 0.0), (f64::NAN, 1.0)], ONE_WAY));
        assert!(!fanned(&[(f64::INFINITY, 1.0)], ONE_WAY));
        // Every order of the same directions gives the same answer.
        let mut seed = 7_u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1_u64 << 53) as f64
        };
        for _ in 0..2000 {
            let width = 200.0 * next();
            let start = 360.0 * next();
            let mut directions: Vec<(f64, f64)> = (0..6)
                .map(|k| {
                    let share = if k == 0 {
                        0.0
                    } else if k == 1 {
                        1.0
                    } else {
                        next()
                    };
                    turn(start + width * share, 0.1 + 10.0 * next())
                })
                .collect();
            let want = width < 134.9;
            if (width - 135.0).abs() < 0.1 {
                continue;
            }
            for _ in 0..6 {
                let k = (next() * 6.0) as usize % 6;
                directions.swap(0, k);
                assert_eq!(fanned(&directions, ONE_WAY), want, "{width} {directions:?}");
            }
        }
    }

    /// A deterministic spline for the tests: `count` control points that
    /// wander, of `degree`, rational where asked (weights from a tenth to
    /// ten), its knots clamped or not and some of them repeated.
    fn wandering(seed: u64, degree: usize, count: usize, rational: bool) -> NurbsCurve {
        let mut state = seed;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 11) as f64 / (1_u64 << 53) as f64
        };
        let mut at = Vec3::new(0.0, 0.0, 0.0);
        let mut heading = 6.0 * next();
        let ctrl: Vec<Vec3> = (0..count)
            .map(|_| {
                heading += 1.6 * (next() - 0.5);
                let (s, c) = heading.psin_cos();
                at += Vec3::new(c, s, 0.3 * (next() - 0.5)) * (0.2 + next());
                at
            })
            .collect();
        let weights: Vec<f64> = (0..count)
            .map(|_| {
                if rational {
                    (4.6 * (next() - 0.5)).pexp()
                } else {
                    1.0
                }
            })
            .collect();
        let clamped = next() < 0.5;
        let mut knots = Vec::new();
        let mut knot = 0.0;
        for k in 0..count + degree + 1 {
            let end = k <= degree || k >= count;
            if !(end && clamped) && next() > 0.15 {
                knot += 0.1 + next();
            }
            if k == count {
                knot += 1.0;
            }
            knots.push(knot);
        }
        NurbsCurve::new(degree, knots, ctrl, weights).unwrap()
    }

    /// What the one-way rule rests on: over a stretch, the direction of a
    /// spline with positive weights is a sum of the legs between the
    /// control points of the spans the stretch meets, and between the
    /// points of its Bézier pieces cut to it, with weights that are not
    /// negative. So where those legs lie in a fan, the curve's direction,
    /// sampled finely, lies in it too: on 600 splines of degrees 1 to 7,
    /// rational and not, clamped and not, with repeated knots.
    #[test]
    fn a_splines_direction_lies_in_the_fan_of_its_legs() {
        let (x, y) = (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
        let flat = |w: Vec3| (w.dot(x), w.dot(y));
        let cross = |a: (f64, f64), b: (f64, f64)| a.0 * b.1 - a.1 * b.0;
        let (mut fans, mut sampled) = (0, 0);
        for seed in 0..600_u64 {
            let degree = 1 + (seed % 7) as usize;
            let curve = wandering(
                seed,
                degree,
                degree + 2 + (seed % 9) as usize,
                seed % 2 == 1,
            );
            let (d0, d1) = curve.domain();
            let (knots, ctrl) = (curve.knots(), curve.control_points());
            let span = |t: f64| {
                knots
                    .partition_point(|k| *k <= t)
                    .saturating_sub(1)
                    .clamp(degree, ctrl.len() - 1)
            };
            let mut pieces = Pieces::of(&curve);
            for part in 0..16 {
                let a = d0 + (d1 - d0) * part as f64 / 16.0;
                let b = d0 + (d1 - d0) * (part + 1) as f64 / 16.0;
                // The control points of the spans the stretch meets hold
                // it whole; the Bézier piece of a span, cut to the stretch,
                // holds the stretch's part in that span.
                let mut held = vec![((a, b), ctrl[span(a) - degree..=span(b)].to_vec())];
                for k in curve.span_at(a)..=curve.span_at(b) {
                    let within = (a.max(knots[k]), b.min(knots[k + 1]));
                    if within.1 > within.0 {
                        let mut first = None;
                        pieces.each_piece(within, &mut 0, |points, _| {
                            first = Some(points.to_vec());
                            false
                        });
                        held.extend(first.map(|points| (within, points)));
                    }
                }
                for (within, run) in held {
                    let mut fan = Fan::within(0.999 * PI);
                    let legs: Vec<(f64, f64)> = run
                        .windows(2)
                        .map(|p| flat(p[1] - p[0]))
                        .filter(|t| t.0 != 0.0 || t.1 != 0.0)
                        .collect();
                    if legs.is_empty() || !legs.iter().all(|t| fan.add(*t)) {
                        continue;
                    }
                    let (low, high) = fan.ends.unwrap();
                    fans += 1;
                    for k in 0..64 {
                        let t = within.0 + (within.1 - within.0) * (k as f64 + 0.5) / 64.0;
                        let d = flat(curve.derivatives(t).1);
                        let size = d.0.phypot(d.1);
                        if size == 0.0 {
                            continue;
                        }
                        sampled += 1;
                        let slack = 1e-9 * size;
                        assert!(
                            cross(low, d) >= -slack * low.0.phypot(low.1)
                                && cross(d, high) >= -slack * high.0.phypot(high.1),
                            "seed {seed}, degree {degree}, stretch {part}: the direction {d:?} at {t} leaves the fan {low:?} to {high:?}"
                        );
                    }
                }
            }
        }
        assert!(
            fans > 1500 && sampled > 60_000,
            "{fans} fans, {sampled} directions"
        );
    }

    /// A cubic through `count` points of a spiral that goes `turns` times
    /// round the point `(0, 0)` of a chart, from the radius 0.15 to 0.55,
    /// `at` taking the chart's coordinates to space.
    fn fitted_spiral(at: &dyn Fn(f64, f64) -> Vec3, turns: f64, count: usize) -> NurbsCurve {
        let params: Vec<f64> = (0..count).map(|i| i as f64 / (count - 1) as f64).collect();
        let values: Vec<Vec<f64>> = params
            .iter()
            .map(|s| {
                let (sin, cos) = (TAU * turns * s).psin_cos();
                let r = 0.15 + 0.4 * s;
                let p = at(r * cos, r * sin);
                vec![p.x, p.y, p.z]
            })
            .collect();
        let (knots, ctrl) = craft3d_geom::refine::interpolate(&params, 3, &values).unwrap();
        let points = ctrl.iter().map(|c| Vec3::new(c[0], c[1], c[2])).collect();
        NurbsCurve::polynomial(3, knots, points).unwrap()
    }

    /// A plane, a cylinder, a cone, a sphere and a torus of size `k` in
    /// `frame`, each with the map from a chart about `(0, 0)` to it, and
    /// the lengths a unit of its `u` and of its `v` are on it there.
    #[allow(clippy::type_complexity)]
    fn charted(frame: Frame, k: f64) -> Vec<(Surface, Box<dyn Fn(f64, f64) -> Vec3>, (f64, f64))> {
        use craft3d_geom::{Cone, Cylinder, Plane, Sphere, Torus};
        let slope = 0.4_f64.ptan();
        vec![
            (
                Surface::Plane(Plane::new(frame)),
                Box::new(move |x, y| frame.point(3.0 * k * x, 3.0 * k * y, 0.0)),
                (1.0, 1.0),
            ),
            (
                Surface::Cylinder(Cylinder::new(frame, k).unwrap()),
                Box::new(move |x, y| about(&frame, k, x, k * y)),
                (k, 1.0),
            ),
            (
                Surface::Cone(Cone::new(frame, 2.0 * k, 0.4).unwrap()),
                Box::new(move |x, y| about(&frame, 2.0 * k + k * y * slope, x, k * y)),
                (2.0 * k, 1.0),
            ),
            (
                Surface::Sphere(Sphere::new(frame, 2.0 * k).unwrap()),
                Box::new(move |x, y| {
                    let (s, c) = y.psin_cos();
                    about(&frame, 2.0 * k * c, x, 2.0 * k * s)
                }),
                (2.0 * k, 2.0 * k),
            ),
            (
                Surface::Torus(Torus::new(frame, 3.0 * k, k).unwrap()),
                Box::new(move |x, y| {
                    let (s, c) = y.psin_cos();
                    about(&frame, 3.0 * k + k * c, 0.5 * x, k * s)
                }),
                (4.0 * k, k),
            ),
        ]
    }

    /// The directions an edge's image in the chart of `surface` takes over
    /// `(a, b)`, from one of 64 points of it to the next (`u` unwrapped to
    /// the point before), `u` and `v` as the lengths `scale` gives a unit
    /// of each.
    fn chart_directions(
        surface: &Surface,
        scale: (f64, f64),
        curve: &NurbsCurve,
        (a, b): (f64, f64),
    ) -> Vec<(f64, f64)> {
        let mut last: Option<(f64, f64)> = None;
        let mut out = Vec::new();
        for k in 0..=64 {
            let (u, v) = surface
                .project(curve.eval(a + (b - a) * k as f64 / 64.0))
                .unwrap();
            let (mut u, mut v) = (u, v);
            if let Some((lu, lv)) = last {
                if let Some(period) = surface.period_u() {
                    u += ((lu - u) / period).round() * period;
                }
                if let Some(period) = surface.period_v() {
                    v += ((lv - v) / period).round() * period;
                }
                out.push(((u - lu) * scale.0, (v - lv) * scale.1));
            }
            last = Some((u, v));
        }
        out
    }

    /// A spline edge that spirals about a point of the chart, on a plane, a
    /// cylinder, a cone, a sphere and a torus, turned and moved, at three
    /// scales, is
    /// followed in steps over each half of which its image in the chart
    /// runs one way (sampled finely, its directions lie in less than a half
    /// turn), so no chord of the walk hides a turn: 16 turns in 32 steps or
    /// more, where eight read it as a straight line. Up to four turns it
    /// keeps the eight steps it always had. Through 64 points a turn, which
    /// the control points show, and through 8, whose spans are too wide for
    /// that: shown by the speed and by the cut pieces.
    #[test]
    fn a_spline_that_spirals_in_the_chart_runs_one_way_each_half_step() {
        for (k, a_turn) in [(1e-3, 64.0), (1.0, 64.0), (1e6, 64.0), (1.0, 8.0)] {
            for (surface, at, scale) in charted(tilted(), k) {
                let lay = Lay::of(&surface);
                for turns in [0.25, 1.0, 4.0, 16.0, 40.0] {
                    let count = ((a_turn * turns) as usize).max(16);
                    let curve = fitted_spiral(at.as_ref(), turns, count);
                    let range = curve.domain();
                    let steps = spline_steps(&curve, range, &lay, &mut 0).unwrap();
                    let kind = surface.kind();
                    // A half step of three eighths of a turn at the most,
                    // so more than four thirds of a step a turn.
                    let least = (turns * 4.0 / 3.0) as usize;
                    if a_turn < 64.0 || kind == "torus" {
                        assert!(steps >= least, "{kind}, {turns} turns: {steps} steps");
                    } else if turns <= 4.0 {
                        assert_eq!(steps, SPLINE_STEPS, "{kind} at {k}, {turns} turns");
                    } else {
                        assert!(
                            (least..=4 * least).contains(&steps),
                            "{kind} at {k}, {turns} turns: {steps} steps"
                        );
                    }
                    let halves = 2 * steps;
                    for h in 0..halves {
                        let end =
                            |j: usize| range.0 + (range.1 - range.0) * (j as f64 / halves as f64);
                        let directions =
                            chart_directions(&surface, scale, &curve, (end(h), end(h + 1)));
                        assert!(
                            fanned(&directions, 0.999 * PI),
                            "{kind} at {k}, {turns} turns: half step {h} of {halves} turns back"
                        );
                    }
                }
            }
        }
    }

    /// A curve that winds about a torus's tube as it goes round its axis
    /// (`p` times round the axis and `q` about the tube, a cubic through 64
    /// points a turn of the faster) is followed in steps each half of
    /// which moves less than a quarter turn about the axis and about the
    /// tube (sampled finely, each unwrapped from point to point): what the
    /// walk needs to follow it round both ways. Three scales, two tori.
    #[test]
    fn a_spline_on_a_torus_moves_less_than_a_quarter_turn_about_its_tube_a_half_step() {
        let f = tilted();
        for k in [1e-3, 1.0, 1e6] {
            for (major, minor) in [(3.0 * k, k), (2.0 * k, 0.5 * k)] {
                let torus = Surface::Torus(craft3d_geom::Torus::new(f, major, minor).unwrap());
                let lay = Lay::of(&torus);
                for (p, q) in [
                    (1.0_f64, 0.0_f64),
                    (0.0, 1.0),
                    (3.0, 13.0),
                    (13.0, 3.0),
                    (2.0, 2.0),
                    (1.0, 40.0),
                ] {
                    let count = 64 * (p.max(q) as usize) + 1;
                    let params: Vec<f64> =
                        (0..count).map(|i| i as f64 / (count - 1) as f64).collect();
                    let values: Vec<Vec<f64>> = params
                        .iter()
                        .map(|s| {
                            let (sv, cv) = (TAU * q * s + 0.3).psin_cos();
                            let at = about(&f, major + minor * cv, TAU * p * s, minor * sv);
                            vec![at.x, at.y, at.z]
                        })
                        .collect();
                    let (knots, ctrl) =
                        craft3d_geom::refine::interpolate(&params, 3, &values).unwrap();
                    let points = ctrl.iter().map(|c| Vec3::new(c[0], c[1], c[2])).collect();
                    let curve = NurbsCurve::polynomial(3, knots, points).unwrap();
                    let range = curve.domain();
                    let steps = spline_steps(&curve, range, &lay, &mut 0).unwrap();
                    let halves = 2 * steps;
                    for h in 0..halves {
                        let end =
                            |j: usize| range.0 + (range.1 - range.0) * (j as f64 / halves as f64);
                        let moved =
                            chart_directions(&torus, (1.0, 1.0), &curve, (end(h), end(h + 1)));
                        for d in 0..2 {
                            let (mut at, mut low, mut high) = (0.0_f64, 0.0_f64, 0.0_f64);
                            for step in &moved {
                                at += if d == 0 { step.0 } else { step.1 };
                                low = low.min(at);
                                high = high.max(at);
                            }
                            assert!(
                                high - low < QUARTER_TURN,
                                "({p}, {q}) at {k}, radii {major} and {minor}: half step {h} of {halves} moves {} about {}",
                                high - low,
                                if d == 0 { "the axis" } else { "the tube" }
                            );
                        }
                    }
                }
            }
        }
    }

    /// Showing that an edge runs one way in the chart takes no more steps
    /// of a helix than showing its angle about the axis does: a helix is a
    /// straight line in a cylinder's chart. (The steps of the helices of
    /// `showing_the_steps_of_a_spline_edge_costs_about_its_control_points`,
    /// on a cylinder's own chart.) And little more work: a strip of 64
    /// turns between two cubics through 16384 points each, the finest a
    /// fit may be, is still judged within the work of a face
    /// (`MOST_PROOF_WORK`: both sides shown, and their 256 steps each
    /// followed), as it was while only the angle was shown.
    #[test]
    fn a_helix_keeps_the_steps_its_angle_needs() {
        let f = tilted();
        let axis = (f.origin(), f.z());
        let cylinder = Lay::of(&Surface::Cylinder(
            craft3d_geom::Cylinder::new(f, 1.0).unwrap(),
        ));
        let mut curves = vec![
            fitted_helix(&f, 64.0, 1024),
            fitted_helix(&f, 64.0, 4096),
            fitted_helix(&f, 10.0, 400),
            fitted_helix(&f, 1.0, 40),
        ];
        curves.extend([1, 4, 5, 37, 256].map(|quarters| winding(&f, 1.0, quarters, 0.7)));
        for curve in &curves {
            let range = curve.domain();
            let (mut turned, mut charted) = (0, 0);
            let about = spline_steps(curve, range, &turning(axis), &mut turned).unwrap();
            let on = spline_steps(curve, range, &cylinder, &mut charted).unwrap();
            assert_eq!(on, about, "{} control points", curve.control_points().len());
            assert!(
                charted <= 4 * turned + 64,
                "{} control points: {charted} units against {turned}",
                curve.control_points().len()
            );
        }
        let finest = fitted_helix(&f, 64.0, 16384);
        let mut work = 0;
        let steps = spline_steps(&finest, finest.domain(), &cylinder, &mut work).unwrap();
        assert_eq!(steps, 256);
        assert!(
            2 * (work + steps * WALK_UNITS) < MOST_PROOF_WORK,
            "{work} units a side"
        );
    }

    /// Nothing shows how the chart of a spline surface takes an edge, so
    /// there an edge is only held to running forward in space over each
    /// half of its eight steps: one that does is followed in them, one that
    /// does not (a spiral of 16 turns) is not followed and is counted. An
    /// edge that turns back at a point no halving reaches (a hairpin at a
    /// knot) is not followed on a plane either; a corner of a third of a
    /// turn is.
    #[test]
    fn an_edge_that_cannot_be_shown_to_run_one_way_is_not_followed() {
        let f = tilted();
        let flat = Lay::of(&Surface::Plane(craft3d_geom::Plane::new(f)));
        let on_plane = |x: f64, y: f64| f.point(x, y, 0.0);
        let gentle = fitted_spiral(&on_plane, 0.25, 16);
        let wound = fitted_spiral(&on_plane, 16.0, 1024);
        assert_eq!(
            spline_steps(&gentle, gentle.domain(), &Lay::Space, &mut 0),
            Some(SPLINE_STEPS)
        );
        let cut = step_proofs().1;
        let mut work = 0;
        assert_eq!(
            spline_steps(&wound, wound.domain(), &Lay::Space, &mut work),
            None
        );
        assert_eq!(step_proofs().1, cut + 1);
        assert!(work > MOST_PROOF_WORK);
        assert_eq!(
            spline_steps(&wound, wound.domain(), &flat, &mut 0),
            Some(32)
        );
        // Corners at a knot of a spline of degree 1.
        let cornered = |degrees: f64| {
            let (s, c) = degrees.to_radians().psin_cos();
            NurbsCurve::polynomial(
                1,
                vec![0.0, 0.0, 0.37, 1.0, 1.0],
                vec![
                    on_plane(0.0, 0.0),
                    on_plane(1.0, 0.0),
                    on_plane(1.0 + 2.0 * c, 2.0 * s),
                ],
            )
            .unwrap()
        };
        let corner = cornered(120.0);
        assert_eq!(
            spline_steps(&corner, corner.domain(), &flat, &mut 0),
            Some(SPLINE_STEPS)
        );
        let hairpin = cornered(179.0);
        let cut = step_proofs().1;
        assert_eq!(
            spline_steps(&hairpin, hairpin.domain(), &flat, &mut 0),
            None
        );
        assert_eq!(step_proofs().1, cut + 1);
        // Control points that lie on each other say nothing of a
        // direction: passed over.
        let doubled = NurbsCurve::polynomial(
            2,
            vec![0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0],
            vec![
                on_plane(0.0, 0.0),
                on_plane(1.0, 0.5),
                on_plane(1.0, 0.5),
                on_plane(2.0, 0.0),
            ],
        )
        .unwrap();
        assert_eq!(
            spline_steps(&doubled, doubled.domain(), &flat, &mut 0),
            Some(SPLINE_STEPS)
        );
        // Numbers too large to multiply: not shown, no panic.
        let huge = NurbsCurve::polynomial(
            1,
            vec![0.0, 0.0, 0.5, 1.0, 1.0],
            vec![
                Vec3::new(-1e200, 0.0, 0.0),
                Vec3::new(1e200, 1e200, 0.0),
                Vec3::new(1e200, -1e200, 1e200),
            ],
        )
        .unwrap();
        for lay in [flat, Lay::Space, turning((f.origin(), f.z()))] {
            assert_eq!(spline_steps(&huge, huge.domain(), &lay, &mut 0), None);
        }
    }
}
