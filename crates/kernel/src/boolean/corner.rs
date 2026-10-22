//! Where an approximated edge meets a surface at a shallow angle.
//!
//! An edge on a spline that stands for the curve in which two surfaces
//! meet (made by an earlier boolean) is only within the tolerance of that
//! curve. Where it runs onto a third surface at a shallow angle, the point
//! where the spline itself meets that surface ([`super::hits`]) slides
//! along it: a spline a fraction `e` of the tolerance off its surfaces,
//! meeting the third surface at an angle `a`, meets it up to `e / sin a`
//! from where the three surfaces meet. The third surface meets the edge's
//! own faces in curves of their own that go through where the three meet,
//! not through that point: where two of the surfaces are nearly tangent (a
//! tube in a tube, their walls crossing at a few degrees), such a curve
//! passes several tolerances from it, the edge is not cut on it, and a
//! piece of the curve runs on across a hole of the face
//! (`kernel/tests/boolean_tube_clusters.rs`).
//!
//! So such a hit is taken to the point where the three surfaces meet
//! ([`refine`]), found by Newton's method from the hit, when that point
//! lies on all three surfaces (measured, not only by the Newton step), is
//! within the tolerance of the edge, and is on the edge strictly between
//! the hits of the edge on the surface on either side of this one (two
//! hits never become one place, nor change order). How far it may be
//! from the hit is bounded by the slide the angle explains (the hit's
//! distance from the edge's own surfaces over the sine of the angle at
//! which the edge meets the surface), up to [`CAP`] tolerances: a hit
//! that would have to slide further, and is not within [`MOVE`]
//! tolerances of where it stands for, is refused as too shallow an angle
//! ([`SHALLOW`]). Every face pair that asks where the edge meets the
//! surface asks it of the same edge and surface, and gets the same point.

use super::Reason;
use super::outline::{Sheet, boxes_meet, locate, surface_distance};
use super::section::Solid;
use craft3d_geom::Curve3;
use craft3d_math::Vec3;
use craft3d_topo::EdgeId;

/// How far a hit must be from where the three surfaces meet, in
/// tolerances, to be moved there. Closer than that it is merged with any
/// other place found there (half the tolerance) with room to spare, and is
/// left as it was found.
const MOVE: f64 = 0.25;

/// How far, in tolerances, the point where the three surfaces meet may
/// always be from the hit; further only as far as the angle explains
/// (four times the slide it allows), never further than [`CAP`].
const REACH: f64 = 256.0;

/// The furthest, in tolerances, a hit is ever moved: a slide longer than
/// this is refused ([`SHALLOW`]).
const CAP: f64 = 65536.0;

/// The surfaces' unit normals must span at least this volume (the
/// determinant of the three, a measure free of scale) for the point
/// where they meet to be told.
const SPAN: f64 = 1e-10;

/// The most Newton steps.
const STEPS: usize = 32;

pub(super) const SHALLOW: Reason = "an edge on a spline meets a surface at an angle so shallow that where it meets it cannot be placed within the tolerance (the surfaces it lies between are nearly tangent to that surface there)";

/// The point near `start`, no further than `reach`, where the three
/// surfaces meet: Newton's method on the planes tangent to each surface
/// at the point of it nearest to the current guess, until the guess is
/// within a millionth of the tolerance of the three tangent planes or a
/// step that small, and accepted only when the point is measured within
/// a quarter of the tolerance of each surface. `None` when the surfaces' normals there are too close to
/// dependent to tell the point, or the steps do not settle.
pub(super) fn meet(sheets: [Sheet; 3], start: Vec3, reach: f64, tol: f64) -> Option<Vec3> {
    // On all three surfaces, the point is where it stays: measured, since
    // a small step only says the tangent planes go through it (a nearest
    // point clamped at a border, or a local one, is no foot of the point).
    let on = |p: Vec3| {
        sheets
            .iter()
            .all(|&sheet| surface_distance(sheet, p) <= 0.25 * tol)
    };
    let mut p = start;
    for _ in 0..STEPS {
        let mut normals = [Vec3::ZERO; 3];
        let mut offsets = [0.0; 3];
        for ((sheet, normal), offset) in sheets.iter().zip(&mut normals).zip(&mut offsets) {
            let (u, v) = sheet.project(p)?;
            let q = sheet.surface.eval(u, v);
            let n = sheet.surface.normal(u, v)?.normalized()?;
            *normal = n;
            *offset = (q - p).dot(n);
        }
        // Within a millionth of the tolerance of the three tangent planes:
        // what is left is rounding, which a step only magnifies where the
        // surfaces are close to tangent.
        if offsets.iter().all(|o| o.abs() <= 1e-6 * tol) {
            return on(p).then_some(p);
        }
        let [n0, n1, n2] = normals;
        let [b0, b1, b2] = offsets;
        let det = n0.dot(n1.cross(n2));
        if !(det.is_finite() && det.abs() > SPAN) {
            return None;
        }
        let step = (n1.cross(n2) * b0 + n2.cross(n0) * b1 + n0.cross(n1) * b2) * (1.0 / det);
        p += step;
        if !p.is_finite() || p.distance(start) > reach {
            return None;
        }
        if step.length() <= 1e-6 * tol {
            return on(p).then_some(p);
        }
    }
    None
}

/// Where edge `id` of `own`, on a spline, meets `surface`, given `hit`,
/// the point where its spline does, at parameter `at` of the edge
/// (`touch` when it only touches the surface there: such a hit stands),
/// and
/// `between`, the parameters of the hits of the edge on the surface on
/// either side of it (or the ends of the edge): the point where the
/// surfaces of the edge's two faces and `surface` meet, when the hit is
/// more than [`MOVE`] tolerances from it, and that point is within the
/// tolerance of the edge strictly between those parameters; `Ok(None)`
/// when the hit stands; [`SHALLOW`] when it would have to slide further
/// than [`CAP`] tolerances.
pub(super) fn refine(
    own: &Solid,
    id: EdgeId,
    surface: Sheet,
    (hit, at, touch): (Vec3, f64, bool),
    between: (f64, f64),
    tol: f64,
) -> Result<Option<Vec3>, Reason> {
    // Where the edge only touches the surface, the three surfaces meet
    // in no point of their own: they are tangent there, every point of a
    // band along the edge is as near all three, and Newton creeps along
    // it to wherever it stops (48.0 or 48.7 tolerances from the hit by
    // two stopping rules, at a rolling fillet's end on a cone,
    // `fillet_contact.rs`). The hit, where the edge comes closest, is the
    // place.
    if touch {
        return Ok(None);
    }
    let Some(edge) = own.body.edge(id) else {
        return Ok(None);
    };
    // The faces on either side of the edge.
    let edge_box = own.edge_box(id);
    let mut faces = Vec::with_capacity(2);
    for face in own.candidates(|b| boxes_meet(b, &edge_box, tol)) {
        let Some(f) = own.body.faces.get(face) else {
            return Ok(None);
        };
        let borders = f
            .loops
            .iter()
            .filter_map(|&l| own.body.loop_(l))
            .any(|l| l.coedges.iter().any(|c| c.edge == id));
        if borders {
            faces.push(face);
        }
    }
    let [first, second] = faces.as_slice() else {
        return Ok(None);
    };
    let (Ok(a), Ok(b)) = (own.outline(*first), own.outline(*second)) else {
        return Ok(None);
    };
    let (a, b) = (a.sheet(), b.sheet());
    // How far the hit can have slid: its distance from the edge's own
    // surfaces over the sine of the angle at which the edge meets the
    // surface.
    let off = surface_distance(a, hit).max(surface_distance(b, hit));
    let slide = match slide(&edge.curve, at, surface, off) {
        Some(slide) => slide,
        None => return Ok(None),
    };
    let reach = (REACH * tol).max(4.0 * slide);
    if reach > CAP * tol {
        return Err(SHALLOW);
    }
    let Some(point) = meet([a, b, surface], hit, reach, tol) else {
        return Ok(None);
    };
    if point.distance(hit) <= MOVE * tol {
        return Ok(None);
    }
    let Some(t) = locate(&edge.curve, edge.range, point, tol) else {
        return Ok(None);
    };
    let (lo, hi) = (between.0.min(between.1), between.0.max(between.1));
    if !(t > lo && t < hi) {
        return Ok(None);
    }
    Ok(Some(point))
}

/// `off` over the sine of the angle at which the curve, at `t`, meets the
/// surface; infinite where it runs along it. `None` without a tangent or
/// a normal there.
fn slide(curve: &Curve3, t: f64, surface: Sheet, off: f64) -> Option<f64> {
    let (p, tangent) = curve.derivatives(t);
    let tangent = tangent.normalized()?;
    let (u, v) = surface.project(p)?;
    let normal = surface.surface.normal(u, v)?.normalized()?;
    let sine = tangent.dot(normal).abs();
    if !off.is_finite() {
        return None;
    }
    Some(if off <= 0.0 {
        0.0
    } else if sine > 0.0 {
        off / sine
    } else {
        f64::INFINITY
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_geom::{Cylinder, Plane, Surface};
    use craft3d_math::Frame;

    fn plane(origin: Vec3, normal: Vec3) -> Surface {
        let hint = if normal.x.abs() < 0.9 {
            Vec3::X
        } else {
            Vec3::Y
        };
        Surface::Plane(Plane::new(Frame::new(origin, normal, hint).unwrap()))
    }

    /// Three planes meet in their point; a cylinder and two planes in the
    /// point on the cylinder; planes too close to dependent, or a point
    /// out of reach, give nothing.
    #[test]
    fn three_surfaces_meet_where_all_three_are() {
        let tol = 1e-6;
        let target = Vec3::new(1.0, 2.0, 3.0);
        let planes = [
            plane(target, Vec3::X),
            plane(target, Vec3::Y),
            plane(target, Vec3::Z),
        ];
        let [a, b, c] = &planes;
        let sheets = [Sheet::bare(a), Sheet::bare(b), Sheet::bare(c)];
        let found = meet(sheets, Vec3::new(1.0, 2.0, 2.999), 1.0, tol).unwrap();
        assert!(found.distance(target) < 1e-12);
        // Out of reach.
        assert!(meet(sheets, Vec3::ZERO, 1.0, tol).is_none());

        // The cylinder of radius 2 about z, the planes x = 1 and z = 5:
        // they meet at (1, √3, 5) near the start.
        let cylinder = Surface::Cylinder(Cylinder::new(Frame::WORLD, 2.0).unwrap());
        let x1 = plane(Vec3::new(1.0, 0.0, 0.0), Vec3::X);
        let z5 = plane(Vec3::new(0.0, 0.0, 5.0), Vec3::Z);
        let sheets = [Sheet::bare(&cylinder), Sheet::bare(&x1), Sheet::bare(&z5)];
        let found = meet(sheets, Vec3::new(1.01, 1.7, 5.0), 1.0, tol).unwrap();
        assert!(found.distance(Vec3::new(1.0, 3.0_f64.sqrt(), 5.0)) < 1e-12);
        // Two planes 1e-11 rad apart and a third across them, from off the
        // point: too close to dependent to tell where they meet.
        // Two planes 1e-11 rad apart and a third across them.
        let tilted = plane(target, Vec3::new(1.0, 1e-11, 0.0));
        let sheets = [Sheet::bare(a), Sheet::bare(&tilted), Sheet::bare(c)];
        assert!(meet(sheets, target + Vec3::new(0.0, 0.5, 0.0), 1.0, tol).is_none());
    }
}
