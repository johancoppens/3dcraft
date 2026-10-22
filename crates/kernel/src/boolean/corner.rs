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
//! ([`refine`]), found by Newton's method from the hit, when that point is
//! within the tolerance of the edge. Every face pair that asks where the
//! edge meets the surface asks it of the same edge and surface, and gets
//! the same point.

use super::outline::{Sheet, boxes_meet, locate};
use super::section::Solid;
use craft3d_math::Vec3;
use craft3d_topo::EdgeId;

/// How far a hit must be from where the three surfaces meet, in
/// tolerances, to be moved there. Closer than that it is merged with any
/// other place found there (half the tolerance) with room to spare, and is
/// left as it was found.
const MOVE: f64 = 0.25;

/// How far, in tolerances, the point where the three surfaces meet may be
/// from the hit: further than that it is not the place the hit stands for.
const REACH: f64 = 256.0;

/// The most Newton steps.
const STEPS: usize = 32;

/// The point near `start` where the three surfaces meet: Newton's method
/// on the planes tangent to each surface at the point of it nearest to
/// the current guess. `None` when the surfaces' normals there are too
/// close to dependent to tell the point, or the steps do not settle.
pub(super) fn meet(sheets: [Sheet; 3], start: Vec3, tol: f64) -> Option<Vec3> {
    let mut p = start;
    for _ in 0..STEPS {
        let mut normals = [Vec3::ZERO; 3];
        let mut offsets = [0.0; 3];
        for ((sheet, normal), offset) in sheets.iter().zip(&mut normals).zip(&mut offsets) {
            let (u, v) = sheet.project(p)?;
            let q = sheet.surface.eval(u, v);
            let n = sheet.surface.normal(u, v)?;
            *normal = n;
            *offset = (q - p).dot(n);
        }
        let [n0, n1, n2] = normals;
        let [b0, b1, b2] = offsets;
        let det = n0.dot(n1.cross(n2));
        if !(det.is_finite() && det.abs() > 1e-9) {
            return None;
        }
        let step = (n1.cross(n2) * b0 + n2.cross(n0) * b1 + n0.cross(n1) * b2) * (1.0 / det);
        p += step;
        if !p.is_finite() || p.distance(start) > REACH * tol {
            return None;
        }
        if step.length() <= 1e-6 * tol {
            return Some(p);
        }
    }
    None
}

/// Where edge `id` of `own`, on a spline, meets `surface`, given `hit`,
/// the point where its spline does: the point where the surfaces of the
/// edge's two faces and `surface` meet, when the hit is more than
/// [`MOVE`] tolerances from it and that point is within the tolerance of
/// the edge; otherwise `None`, and the hit stands.
pub(super) fn refine(own: &Solid, id: EdgeId, surface: Sheet, hit: Vec3, tol: f64) -> Option<Vec3> {
    let edge = own.body.edge(id)?;
    // The faces on either side of the edge.
    let edge_box = own.edge_box(id);
    let mut faces = Vec::with_capacity(2);
    for face in own.candidates(|b| boxes_meet(b, &edge_box, tol)) {
        let f = own.body.faces.get(face)?;
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
        return None;
    };
    let (a, b) = (own.outline(*first).ok()?, own.outline(*second).ok()?);
    let point = meet([a.sheet(), b.sheet(), surface], hit, tol)?;
    if point.distance(hit) <= MOVE * tol {
        return None;
    }
    locate(&edge.curve, edge.range, point, tol)?;
    Some(point)
}
