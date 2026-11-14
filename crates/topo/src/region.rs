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
//! closes on itself both ways, a point that does not project back onto the
//! surface, a loop whose area is not
//! larger than its polyline's error allows to be told from nothing, and a
//! face with more than [`MOST_PER_FACE`] points. On a sphere loops that
//! all run clockwise are the sphere less some caps, which is a region. A
//! freeform surface that closes one way on clamped knots is charted with
//! its domain's width that way as a period ([`periods_of`]), as a cylinder
//! is.
//!
//! **How.** Each edge is followed in a few steps (a line in one on a
//! plane, else in two; a circle or an ellipse in steps of at most π/8; a
//! spline in eight per edge, and on a surface that closes about an axis in
//! as many more as its control points show it needs to move less than a
//! quarter turn about the axis from each point to the next, [`spline_steps`],
//! so that a helix of many turns as one edge is followed round every turn),
//! each point projected onto the surface
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
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Portable, Tolerance, Vec3};
use std::collections::{BTreeMap, VecDeque};
use std::f64::consts::PI;
use std::sync::{Mutex, PoisonError};

/// Most points one face is followed at; a larger face is passed.
pub const MOST_PER_FACE: usize = 1 << 18;
/// Steps of a spline edge (at least: on a surface that closes about an
/// axis, as many more as [`spline_steps`] needs).
const SPLINE_STEPS: usize = 8;
/// Most steps of a spline edge on a surface that closes about an axis; an
/// edge that needs more is not followed.
const MOST_SPLINE_STEPS: usize = 1 << 14;
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
    /// it (the region rule reads it as before). A spline edge on a surface
    /// that closes about an axis is followed in steps shown to move less
    /// than that about it ([`spline_steps`]); what is left is the angle
    /// about a torus's tube and a conic near a pole.
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
    let freeform = matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_));
    if freeform && !craft3d_geom::pole::poles(surface).is_empty() {
        return (None, None);
    }
    let periods = periods_of(surface);
    if periods.iter().all(Option::is_some) {
        return (None, None);
    }
    let near = 10.0 * body.working_tolerance(tol.linear);
    let mut chart = Chart {
        surface,
        periods,
        freeform,
        near,
        last: None,
        scale: f64::INFINITY,
    };
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

/// The periods of a surface's parameters as the chart moves points by
/// them: the analytic kinds' own (`Surface::period_u`), and for a spline
/// surface that closes on itself on clamped knots (`craft3d_geom::closure::
/// closes`: its control net ends on the row it starts with, the same curve)
/// the width of its domain that way, across which it closes, as the
/// measure takes it (`mass.rs`, `spline_closure`). A loop over the seam is
/// then followed over it, and one that goes round is a band, as on a
/// cylinder. A spline surface that closes only within the tolerance, or on
/// knots that are not clamped, has none (charted as one that does not
/// close).
pub(crate) fn periods_of(surface: &Surface) -> [Option<f64>; 2] {
    if !matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
        return [surface.period_u(), surface.period_v()];
    }
    let [closes_u, closes_v] = craft3d_geom::closure::closes(surface);
    let ((u0, u1), (v0, v1)) = surface.domain();
    let width = |closes: bool, a: f64, b: f64| {
        let w = b - a;
        (closes && w.is_finite() && w > 0.0).then_some(w)
    };
    [width(closes_u, u0, u1), width(closes_v, v0, v1)]
}

/// Whether each of `points` (on the surface of `face`) lies on the face,
/// by the parity of the loops' crossings of a ray from it in the
/// surface's parameters (along v, or along u when v is the one that
/// closes); `None` where the face cannot be charted without doubt, as for
/// [`region_fault`], bands around a closed surface included, and on every
/// freeform surface that closes (not charted with a period here: its ray's
/// parity across the seam is owed).
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
    let mut chart = Chart {
        surface,
        periods,
        freeform,
        near: 10.0 * body.working_tolerance(tol.linear),
        last: None,
        scale: f64::INFINITY,
    };
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
    let axis = axis_of(chart.surface);
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
            Curve3::Nurbs(n) => match axis {
                Some(axis) => spline_steps(n, (t0, t1), axis)?,
                None => SPLINE_STEPS,
            },
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

/// The axis a surface closes about, as a point on it and its unit
/// direction: where `u`, whose period is a turn, is the angle about the
/// frame's z axis (a cylinder, a cone, a sphere, a torus), so the point of
/// the surface nearest a point off it, which the chart takes, has the
/// point's own angle about that axis. `None` for every other surface.
fn axis_of(surface: &Surface) -> Option<(Vec3, Vec3)> {
    let frame = match surface {
        Surface::Cylinder(c) => c.frame(),
        Surface::Cone(c) => c.frame(),
        Surface::Sphere(s) => s.frame(),
        Surface::Torus(t) => t.frame(),
        Surface::Plane(_) | Surface::Bezier(_) | Surface::Nurbs(_) => return None,
    };
    Some((frame.origin(), frame.z()))
}

/// The steps a spline edge over `(t0, t1)` is followed in on a surface
/// that closes about `axis` (`u` the angle about it, [`axis_of`]): the
/// fewest `SPLINE_STEPS · 2^k` such that the curve's angle about the axis is
/// shown to move by less than a quarter turn over each half step. Then the
/// chart, which moves each point by whole periods to the one before,
/// unwraps the loop as it goes round: a helix of many turns as one edge is
/// followed round every turn, where eight steps of it read as a few turns
/// the other way (`docs/design/topology.md` › *Loops that bound a region*).
/// Shown by the first of: (1) the convex hull of the control points of the
/// knot spans the half step meets in a wedge about the axis narrower than a
/// quarter turn ([`narrow`]); (2) for a polynomial spline, its speed bound
/// ([`speed_bound`]) times the half step's width, over the least distance
/// from the axis the stretch can have (its middle's less half that length:
/// the distance from a line changes no faster than the point moves), less
/// than a quarter turn (one evaluation of the curve, counted against an
/// operation's work, where (1) does not show it); (3) the hull of the
/// curve's Bézier pieces cut to it (`craft3d_geom::hull_points_in`, made
/// once, only where needed) in such a wedge. Each
/// bounds the angle's movement over the whole stretch, so over any part of
/// it: each of the sixteen half steps of eight is halved only until it is
/// shown, and the deepest it needed sets the steps. `SPLINE_STEPS` where
/// nothing of this can be read (a weight that is not positive and finite, a
/// range past the curve's domain): followed as before. `None` where it
/// cannot be shown within [`MOST_SPLINE_STEPS`] (a curve through or very
/// near the axis, a helix of thousands of turns): the loop cannot be
/// followed without doubt.
fn spline_steps(
    curve: &craft3d_geom::NurbsCurve,
    (t0, t1): (f64, f64),
    axis: (Vec3, Vec3),
) -> Option<usize> {
    let (d0, d1) = curve.domain();
    let positive = curve.weights().iter().all(|w| w.is_finite() && *w > 0.0);
    if !(positive && t0 >= d0 && t1 <= d1) {
        return Some(SPLINE_STEPS);
    }
    let (origin, z) = axis;
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
    // axis.
    let speed = speed_bound(curve);
    let slow = |a: f64, b: f64| {
        let Some(speed) = speed else {
            return false;
        };
        let reach = (b - a) * speed;
        let d = curve.eval(0.5 * (a + b)) - origin;
        let low = (d - z * d.dot(z)).length() - 0.5 * reach;
        low > 0.0 && reach < QUARTER_TURN * low
    };
    // (3) The Bézier pieces, made once where (1) and (2) do not show it.
    let mut pieces = None;
    let width = t1 - t0;
    let halves = 2 * SPLINE_STEPS;
    let deepest = (MOST_SPLINE_STEPS / SPLINE_STEPS).ilog2();
    let mut level = 0_u32;
    // Each half step of `SPLINE_STEPS` halved until it is shown: (the
    // stretch's index at its level, the level).
    let mut open: Vec<(usize, u32)> = (0..halves).map(|i| (i, 0)).collect();
    while let Some((i, at)) = open.pop() {
        let parts = halves.checked_shl(at)?;
        let end = |k: usize| t0 + width * (k as f64 / parts as f64);
        let (a, b) = (end(i), end(i + 1));
        // The control points first: they cost no evaluation of the curve
        // (each counted against an operation's work, `geom::allowance`).
        let shown = spanned(a, b).is_some_and(|points| narrow(points, axis, QUARTER_TURN))
            || slow(a, b)
            || {
                let pieces = pieces.get_or_insert_with(|| curve.bezier_pieces());
                craft3d_geom::hull_points_in(pieces, (a, b))
                    .is_some_and(|points| narrow(&points, axis, QUARTER_TURN))
            };
        if shown {
            level = level.max(at);
        } else if at < deepest {
            open.push((2 * i + 1, at + 1));
            open.push((2 * i, at + 1));
        } else {
            return None;
        }
    }
    SPLINE_STEPS.checked_shl(level)
}

/// An upper bound on a polynomial spline's speed over its domain (its
/// weights all alike): the longest of its derivative's control points
/// `p (P[i+1] - P[i]) / (u[i+p+1] - u[i+1])` (*The NURBS Book* §3.3), whose
/// hull holds the derivative. `None` for a rational spline, or where a
/// derivative control point is not defined (`p + 1` equal knots inside: the
/// curve may jump there).
fn speed_bound(curve: &craft3d_geom::NurbsCurve) -> Option<f64> {
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

/// The angle about its axis the stretch of a spline edge over a half step
/// stays within ([`spline_steps`]): a quarter of the period, which is what
/// the walk reads as a half step that may have gone round unseen
/// ([`Ring::aliased`]).
const QUARTER_TURN: f64 = 0.5 * PI;

/// Whether `points` lie in a wedge about `axis` narrower than `most`
/// (radians, less than a half turn), none on the axis: their convex hull
/// then keeps clear of the axis (every point lies in one open half plane
/// through it) and every point of the hull lies within that wedge, so the
/// angle about the axis moves by less than `most` along a curve in it.
fn narrow(points: &[Vec3], (origin, z): (Vec3, Vec3), most: f64) -> bool {
    let across = |p: Vec3| {
        let d = p - origin;
        d - z * d.dot(z)
    };
    let Some(first) = points.first().map(|p| across(*p)) else {
        return false;
    };
    let length = first.length();
    if !(length.is_finite() && length > 0.0) {
        return false;
    }
    let x = first * (1.0 / length);
    let y = z.cross(x);
    let (mut low, mut high) = (0.0_f64, 0.0_f64);
    for p in points {
        let q = across(*p);
        let (c, s) = (q.dot(x), q.dot(y));
        let off = c.phypot(s);
        if off.is_nan() || off <= 0.0 {
            return false;
        }
        let angle = s.patan2(c);
        if !angle.is_finite() {
            return false;
        }
        low = low.min(angle);
        high = high.max(angle);
        if high - low >= most {
            return false;
        }
    }
    true
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
    use craft3d_geom::NurbsCurve;
    use craft3d_math::Frame;
    use std::f64::consts::{FRAC_PI_2, TAU};

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
                let steps = spline_steps(&curve, range, (f.origin(), f.z())).unwrap();
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
        assert_eq!(
            spline_steps(&curve, curve.domain(), (f.origin(), f.z())),
            None
        );
        // Its first fifth keeps clear of the axis: followed in eight steps.
        assert_eq!(
            spline_steps(&curve, (0.0, 0.2), (f.origin(), f.z())),
            Some(SPLINE_STEPS)
        );
        // A range past the curve's domain has no hull of control points:
        // followed in eight steps, as before.
        assert_eq!(
            spline_steps(&curve, (-0.5, 1.0), (f.origin(), f.z())),
            Some(SPLINE_STEPS)
        );
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
}
