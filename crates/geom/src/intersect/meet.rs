//! `meet_surfaces`: the intersection of any two surfaces inside a box.

// `!(x > 0.0)` is deliberate throughout: unlike `x <= 0.0` it is also true
// for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::clip;
use super::exact::{self, Closed};
use super::field::{Chart, Field};
use super::march::Marcher;
use crate::{Curve3, GeomError, Surface};
use craft3d_math::{Bbox3, Frame, Tolerance, Vec3};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

/// One curve along which two surfaces meet.
#[derive(Clone, Debug, PartialEq)]
pub struct MeetingCurve {
    pub curve: Curve3,
    /// The part of `curve` that is meant: `(t0, t1)` with `t0 < t1`; for a
    /// closed curve one full period. On a circle or ellipse `t1` may exceed
    /// `2π` (a piece across the angle 0).
    pub range: (f64, f64),
    pub closed: bool,
    /// True when the surfaces touch along it without crossing (a cylinder
    /// lying on a plane).
    pub tangent: bool,
    /// True when `curve` is the exact intersection (line, circle, ellipse,
    /// exact rational NURBS conic); false when it is an approximation, and
    /// then `deviation` bounds its distance to both surfaces.
    pub exact: bool,
    pub deviation: f64,
}

/// The curves and points in which two surfaces meet.
///
/// Curves do not cross each other except at their ends: where two branches
/// of the intersection cross (the surfaces are tangent there), every arm is
/// a curve of its own that ends on the crossing point.
#[derive(Clone, Debug, PartialEq)]
pub struct Meeting {
    /// In a deterministic order.
    pub curves: Vec<MeetingCurve>,
    /// Isolated points where the surfaces touch (a sphere resting on a
    /// plane) and no curve passes.
    pub points: Vec<Vec3>,
}

/// How two surfaces meet inside a box.
#[derive(Clone, Debug, PartialEq)]
pub enum SurfaceMeeting {
    /// They do not come within the linear tolerance of each other there.
    None,
    /// They are the same surface. `same_normal` says whether their normals
    /// agree where they coincide.
    Coincident {
        same_normal: bool,
    },
    Meets(Meeting),
}

fn rank(s: &Surface) -> u8 {
    match s {
        Surface::Plane(_) => 0,
        Surface::Cylinder(_) => 1,
        Surface::Cone(_) => 2,
        Surface::Sphere(_) => 3,
        Surface::Torus(_) => 4,
        Surface::Bezier(_) => 5,
        Surface::Nurbs(_) => 6,
    }
}

/// The numbers that define a surface, for a total order on surfaces.
fn key(s: &Surface) -> Vec<f64> {
    fn vec(out: &mut Vec<f64>, v: Vec3) {
        out.extend([v.x, v.y, v.z]);
    }
    fn frame(out: &mut Vec<f64>, f: &Frame) {
        for v in [f.origin(), f.x(), f.y(), f.z()] {
            vec(out, v);
        }
    }
    let mut out = Vec::new();
    match s {
        Surface::Plane(p) => frame(&mut out, p.frame()),
        Surface::Cylinder(c) => {
            frame(&mut out, c.frame());
            out.push(c.radius());
        }
        Surface::Cone(c) => {
            frame(&mut out, c.frame());
            out.extend([c.radius(), c.half_angle()]);
        }
        Surface::Sphere(sp) => {
            frame(&mut out, sp.frame());
            out.push(sp.radius());
        }
        Surface::Torus(t) => {
            frame(&mut out, t.frame());
            out.extend([t.radii().0, t.radii().1]);
        }
        Surface::Bezier(b) => {
            out.extend([b.degree_u() as f64, b.degree_v() as f64]);
            for p in b.control_points() {
                vec(&mut out, *p);
            }
        }
        Surface::Nurbs(n) => {
            out.extend([n.degrees().0 as f64, n.degrees().1 as f64]);
            out.extend(n.knots_u());
            out.extend(n.knots_v());
            for p in n.control_points() {
                vec(&mut out, *p);
            }
            out.extend(n.weights());
        }
    }
    out
}

/// The two surfaces in an order that does not depend on the order they were
/// given in, so that `meet(a, b)` and `meet(b, a)` are the same computation.
fn ordered<'a>(a: &'a Surface, b: &'a Surface) -> (&'a Surface, &'a Surface) {
    let order = rank(a).cmp(&rank(b)).then_with(|| {
        let (ka, kb) = (key(a), key(b));
        ka.iter()
            .zip(&kb)
            .map(|(x, y)| x.total_cmp(y))
            .find(|o| *o != Ordering::Equal)
            .unwrap_or_else(|| ka.len().cmp(&kb.len()))
    });
    if order == Ordering::Greater {
        (b, a)
    } else {
        (a, b)
    }
}

fn check(within: &Bbox3, tol: &Tolerance) -> Result<(), GeomError> {
    if !(tol.linear.is_finite() && tol.linear > 0.0 && tol.angular.is_finite() && tol.angular > 0.0)
    {
        return Err(GeomError::InvalidParameter(
            "a tolerance must be positive and finite",
        ));
    }
    match within.corners() {
        Some((lo, hi)) if lo.is_finite() && hi.is_finite() => Ok(()),
        _ => Err(GeomError::InvalidParameter(
            "the box to intersect in is empty or not finite",
        )),
    }
}

fn finish(closed: Closed, within: &Bbox3, tol: &Tolerance) -> Result<SurfaceMeeting, GeomError> {
    Ok(match closed {
        Closed::None => SurfaceMeeting::None,
        Closed::Coincident { same_normal } => SurfaceMeeting::Coincident { same_normal },
        Closed::Meets { curves, points } => {
            let mut clipped = Vec::new();
            for curve in &curves {
                clipped.extend(clip::clip(curve, within, tol)?);
            }
            let points: Vec<Vec3> = points
                .into_iter()
                .filter(|p| within.contains(*p, tol.linear))
                .collect();
            if clipped.is_empty() && points.is_empty() {
                SurfaceMeeting::None
            } else {
                SurfaceMeeting::Meets(Meeting {
                    curves: clipped,
                    points,
                })
            }
        }
    })
}

/// Which of two surfaces is walked in its parameters: freeform surfaces
/// first (the other is then a distance in closed form), then the kinds
/// whose parametrisation has no pole or apex.
///
/// Last of all a freeform surface with a pole at a corner of its
/// parameters (two adjacent borders one point): there both its
/// parameters stall, and the distance to a surface through the pole
/// vanishes to second order at the corner (as `s t`), a saddle of the
/// walked function just outside the chart, around which no cell is
/// simple down to the floor. Measured against instead, it is a surface
/// like any other there.
fn chart_rank(s: &Surface) -> u8 {
    match s {
        Surface::Bezier(_) | Surface::Nurbs(_) => {
            let poles = crate::pole::poles(s);
            let corner = poles.iter().any(|a| a.across_v()) && poles.iter().any(|a| !a.across_v());
            if corner { 6 } else { 0 }
        }
        Surface::Plane(_) => 1,
        Surface::Cylinder(_) => 2,
        Surface::Torus(_) => 3,
        Surface::Sphere(_) => 4,
        Surface::Cone(_) => 5,
    }
}

fn numeric(
    a: &Surface,
    b: &Surface,
    within: &Bbox3,
    tol: &Tolerance,
) -> Result<SurfaceMeeting, GeomError> {
    let (walked, other) = if chart_rank(b) < chart_rank(a) {
        (b, a)
    } else {
        (a, b)
    };
    let field = Field::new(other)?;
    let back = Field::new(walked)?;
    // One lying on the other (a patch of a sphere on that sphere).
    if let Some(same_normal) = Marcher::lies_on(walked, &field, within, tol)
        .or_else(|| Marcher::lies_on(other, &back, within, tol))
    {
        return Ok(SurfaceMeeting::Coincident { same_normal });
    }
    let march = |bounded: bool| -> Result<Option<Meeting>, GeomError> {
        let (field, back) = (Field::new(other)?, Field::new(walked)?);
        let Some(chart) = Chart::new(walked, within, tol, &field)? else {
            return Ok(None);
        };
        let mut marcher = Marcher::new(chart, field, within, tol)?;
        if bounded {
            marcher = marcher.bounded();
        }
        // A freeform surface the other is measured against has borders
        // that are edges: where they cross the walked surface a piece of
        // the curve may end. (Without a chart of it, nothing is added.)
        if matches!(other, Surface::Bezier(_) | Surface::Nurbs(_))
            && let Ok(Some(edges)) = Chart::new(other, within, tol, &back)
        {
            marcher = marcher.with_field_chart(edges);
        }
        marcher.solve(within).map(Some)
    };
    // Where the cells ran out, once more with the bound that leaves out
    // the cells of surfaces side by side (`Marcher::bounded`).
    let meeting = match march(false) {
        Err(e) if e == super::march::too_many() => march(true)?,
        other => other?,
    };
    let Some(meeting) = meeting else {
        return Ok(SurfaceMeeting::None);
    };
    Ok(if meeting.curves.is_empty() && meeting.points.is_empty() {
        SurfaceMeeting::None
    } else {
        SurfaceMeeting::Meets(meeting)
    })
}

/// Everything of the intersection of `a` and `b` that lies inside `within`.
///
/// Curves are clipped to the box; a curve that leaves it gives one
/// [`MeetingCurve`] per piece inside. The box is what makes unbounded
/// intersections (the hyperbola of a plane and a cone, crossing cylinders)
/// finite. The result does not depend on the order of `a` and `b`.
///
/// Pairs with a closed form (see docs/design/geometry.md) give exact
/// curves; every other pair is intersected numerically, and fails with
/// [`GeomError::NoConvergence`] or [`GeomError::Unsupported`] rather than
/// return a curve that is incomplete or out of tolerance.
pub fn meet_surfaces(
    a: &Surface,
    b: &Surface,
    within: &Bbox3,
    tolerance: &Tolerance,
) -> Result<SurfaceMeeting, GeomError> {
    check(within, tolerance)?;
    let (a, b) = ordered(a, b);
    match exact::closed_form(a, b, tolerance)? {
        Some(closed) => finish(closed, within, tolerance),
        None => remembered(a, b, within, tolerance),
    }
}

/// The numeric intersections made lately, by what they were asked for
/// (written out in full, so that two questions are the same only when
/// every number of them is, bit for bit), each with how much it holds,
/// the work marching it took (`crate::allowance`) and when it was last
/// used; and the questions by when they were last used, to let go of the
/// one asked longest ago first.
///
/// **Sized for big booleans** (2026-10-21). It held 64 intersections and
/// a quarter of a million points, looked up one by one. A union of bodies
/// of 106 and 670 faces, all near one another (the instances of a pattern
/// of rings), intersects 65 000 pairs of faces, 9800 of them numerically;
/// the union, difference and intersection of the same two bodies, and the
/// same boolean regenerated, ask the same questions, and marched every one
/// of them again (16 s each in the test profile, the fuzzer's seed
/// 20830834). Now they are found by their question in a map and kept by
/// what they weigh, [`SHELF_SIZE`] questions, [`SHELF_POINTS`] points of
/// curves and [`SHELF_BYTES`] of questions at most, and the same boolean
/// asked again takes 6 to 8 s.
struct Shelf {
    by_key: BTreeMap<Arc<str>, Kept>,
    by_use: BTreeMap<u64, Arc<str>>,
    clock: u64,
    points: usize,
    bytes: usize,
}

struct Kept {
    result: Result<SurfaceMeeting, GeomError>,
    weight: usize,
    units: u64,
    used: u64,
}

static SHELF: Mutex<Shelf> = Mutex::new(Shelf {
    by_key: BTreeMap::new(),
    by_use: BTreeMap::new(),
    clock: 0,
    points: 0,
    bytes: 0,
});
/// How many intersections are kept, how many points of their curves all
/// of them hold at most, and how many bytes their questions take at most
/// (a question names both surfaces in full: a spline's every control
/// point). The points are what weighs: the union of 106 and 670 faces
/// above keeps 4600 intersections, 3.6 million points of their curves
/// (a couple of hundred MB at most, for the whole shelf). Fewer than its
/// questions is no help when a boolean is asked again: they come in the
/// same order, and each would be let go of before it is asked again. In
/// the browser a sixteenth of that (a few tens of MB at most), as the
/// projections' cache in `surface.rs`: a page has a quarter GB in all.
#[cfg(not(target_arch = "wasm32"))]
const SHELF_SIZE: usize = 1 << 17;
#[cfg(not(target_arch = "wasm32"))]
const SHELF_POINTS: usize = 1 << 22;
#[cfg(not(target_arch = "wasm32"))]
const SHELF_BYTES: usize = 64 << 20;
#[cfg(target_arch = "wasm32")]
const SHELF_SIZE: usize = 1 << 13;
#[cfg(target_arch = "wasm32")]
const SHELF_POINTS: usize = 1 << 18;
#[cfg(target_arch = "wasm32")]
const SHELF_BYTES: usize = 4 << 20;

/// How many points an intersection holds, roughly: what it costs to keep.
fn weight(result: &Result<SurfaceMeeting, GeomError>) -> usize {
    match result {
        Ok(SurfaceMeeting::Meets(meeting)) => meeting
            .curves
            .iter()
            .map(|c| match &c.curve {
                Curve3::Nurbs(n) => n.control_points().len().saturating_add(1),
                _ => 1,
            })
            .fold(
                meeting.points.len().saturating_add(1),
                usize::saturating_add,
            ),
        _ => 1,
    }
}

/// [`numeric`], remembered. The numeric intersection is a function of the
/// two surfaces, the box and the tolerance alone, and the most expensive
/// step of a boolean: the union, difference and intersection of the same
/// two bodies (a boolean tried one way and then another, the three of a
/// test) ask it the same questions, as does a boolean done again on
/// operands that did not change. The answer from here is the one it would
/// give, refusals included.
fn remembered(
    a: &Surface,
    b: &Surface,
    within: &Bbox3,
    tol: &Tolerance,
) -> Result<SurfaceMeeting, GeomError> {
    let key = format!("{a:?}|{b:?}|{within:?}|{tol:?}");
    // Charged by the work the march took, once per operation under a
    // bound, whether it is marched now or found on the shelf
    // (`crate::allowance::first_time`): the march is counted the first
    // time, its count kept with it, and a hit charges that count.
    let thing = crate::allowance::number_of(&key);
    let first = crate::allowance::first_time(thing);
    if let Some((result, units)) = shelf_find(&key) {
        if first {
            crate::allowance::charge(units);
        }
        return result;
    }
    let (result, units) = if first {
        crate::allowance::measured(|| numeric(a, b, within, tol))
    } else {
        // Paid for already in this operation.
        (
            crate::allowance::uncounted(|| numeric(a, b, within, tol)),
            0,
        )
    };
    if first && crate::allowance::counting() {
        shelf_put(key, &result, units);
    }
    result
}

/// The intersection asked for by `key`, with the work it took, from the
/// shelf.
fn shelf_find(key: &str) -> Option<(Result<SurfaceMeeting, GeomError>, u64)> {
    let mut guard = SHELF.lock().unwrap_or_else(PoisonError::into_inner);
    let shelf = &mut *guard;
    shelf.clock = shelf.clock.wrapping_add(1);
    let clock = shelf.clock;
    let kept = shelf.by_key.get_mut(key)?;
    let found = (kept.result.clone(), kept.units);
    let before = kept.used;
    kept.used = clock;
    if let Some(name) = shelf.by_use.remove(&before) {
        shelf.by_use.insert(clock, name);
    }
    Some(found)
}

/// Puts `result` on the shelf with the work it took, letting go of those
/// asked for longest ago while there is not room.
fn shelf_put(key: String, result: &Result<SurfaceMeeting, GeomError>, units: u64) {
    let weight = weight(result);
    let bytes = key.len();
    if weight > SHELF_POINTS || bytes > SHELF_BYTES {
        return;
    }
    let mut guard = SHELF.lock().unwrap_or_else(PoisonError::into_inner);
    let shelf = &mut *guard;
    if shelf.by_key.contains_key(key.as_str()) {
        return;
    }
    while !shelf.by_key.is_empty()
        && (shelf.by_key.len() >= SHELF_SIZE
            || shelf.points.saturating_add(weight) > SHELF_POINTS
            || shelf.bytes.saturating_add(bytes) > SHELF_BYTES)
    {
        let Some((_, oldest)) = shelf.by_use.pop_first() else {
            break;
        };
        if let Some(gone) = shelf.by_key.remove(&oldest) {
            shelf.points = shelf.points.saturating_sub(gone.weight);
            shelf.bytes = shelf.bytes.saturating_sub(oldest.len());
        }
    }
    shelf.clock = shelf.clock.wrapping_add(1);
    let used = shelf.clock;
    shelf.points = shelf.points.saturating_add(weight);
    shelf.bytes = shelf.bytes.saturating_add(bytes);
    let key: Arc<str> = Arc::from(key);
    shelf.by_use.insert(used, Arc::clone(&key));
    shelf.by_key.insert(
        key,
        Kept {
            result: result.clone(),
            weight,
            units,
            used,
        },
    );
}

/// Lets go of every intersection on the shelf (each is found again, the
/// same, when asked for), and says how many there were.
pub(super) fn forget_shelf() -> usize {
    let mut guard = SHELF.lock().unwrap_or_else(PoisonError::into_inner);
    let kept = guard.by_key.len();
    *guard = Shelf {
        by_key: BTreeMap::new(),
        by_use: BTreeMap::new(),
        clock: guard.clock,
        points: 0,
        bytes: 0,
    };
    kept
}

/// How many intersections the shelf holds.
pub(super) fn shelf_len() -> usize {
    SHELF
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .by_key
        .len()
}

/// [`meet_surfaces`] without its closed forms: the numeric marcher on any
/// pair. For tests that hold the marcher against the exact results.
#[doc(hidden)]
pub fn meet_surfaces_numeric(
    a: &Surface,
    b: &Surface,
    within: &Bbox3,
    tolerance: &Tolerance,
) -> Result<SurfaceMeeting, GeomError> {
    check(within, tolerance)?;
    let (a, b) = ordered(a, b);
    numeric(a, b, within, tolerance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BezierPatch, Sphere};

    #[test]
    fn the_shelf_keeps_the_questions_of_a_big_boolean_and_lets_go_of_the_oldest() {
        // (Keys of their own: the shelf is shared with the other tests.)
        let key = |n: usize| format!("shelf test {n}");
        let none: Result<SurfaceMeeting, GeomError> = Ok(SurfaceMeeting::None);
        // A boolean of a few hundred faces a side asks tens of thousands
        // of questions; the shelf held 64.
        for n in 0..20_000 {
            shelf_put(key(n), &none, n as u64);
        }
        for n in (0..20_000).step_by(997) {
            let (found, units) = shelf_find(&key(n)).unwrap();
            assert!(matches!(found, Ok(SurfaceMeeting::None)));
            assert_eq!(units, n as u64);
        }
        // Full: the ones asked for longest ago go first.
        for n in 20_000..20_000 + SHELF_SIZE {
            shelf_put(key(n), &none, 0);
        }
        assert!(shelf_find(&key(1)).is_none());
        assert!(shelf_find(&key(20_000 + SHELF_SIZE - 1)).is_some());
        let shelf = SHELF.lock().unwrap();
        assert!(shelf.by_key.len() <= SHELF_SIZE);
        assert_eq!(shelf.by_key.len(), shelf.by_use.len());
        assert!(shelf.points <= SHELF_POINTS && shelf.bytes <= SHELF_BYTES);
    }

    #[test]
    fn a_numeric_intersection_asked_again_is_the_one_made_the_first_time() {
        let mut ctrl = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                let (x, y) = (f64::from(i), f64::from(j));
                ctrl.push(Vec3::new(x, y, 0.4 * (1.3 * x - 0.7 * y).sin()));
            }
        }
        let roof = Surface::Bezier(BezierPatch::new(3, 3, ctrl).unwrap());
        let ball = Surface::Sphere(
            Sphere::new(Frame::WORLD.at(Vec3::new(1.4, 1.6, 0.3)).unwrap(), 0.9).unwrap(),
        );
        let within = Bbox3::from_points([Vec3::new(-1.0, -1.0, -2.0), Vec3::new(4.0, 4.0, 2.0)]);
        let tol = Tolerance::default();
        let made = meet_surfaces_numeric(&roof, &ball, &within, &tol);
        assert!(matches!(&made, Ok(SurfaceMeeting::Meets(m)) if !m.curves.is_empty()));
        for (a, b) in [(&roof, &ball), (&ball, &roof), (&roof, &ball)] {
            let asked = meet_surfaces(a, b, &within, &tol);
            assert_eq!(format!("{asked:?}"), format!("{made:?}"));
        }
        // Another box is another question.
        let smaller = Bbox3::from_points([Vec3::new(-1.0, -1.0, -2.0), Vec3::new(1.6, 4.0, 2.0)]);
        let other = meet_surfaces(&roof, &ball, &smaller, &tol);
        assert_eq!(
            format!("{other:?}"),
            format!("{:?}", meet_surfaces_numeric(&roof, &ball, &smaller, &tol))
        );
        assert_ne!(format!("{other:?}"), format!("{made:?}"));
    }
}
