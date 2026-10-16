//! Whether two solids are clear of each other, touch, or overlap, and
//! where: the interference query (`document.interference`). Read-only: it
//! makes no body but the common volume, and builds on the boolean's own
//! steps, so it refuses exactly what the boolean refuses.

use super::section::{self, Solid};
use super::{
    BooleanOp, BooleanOptions, BooleanResult, Operand, Stop, Vertices, boolean_with, on_the_other,
    outline,
};
use crate::{KernelError, MAX_REPORTED};
use craft3d_math::{Bbox3, Tolerance, Vec3};
use craft3d_topo::{Body, FaceId};
use std::collections::BTreeSet;

/// Most contact points an answer lists.
pub const MAX_CONTACT_POINTS: usize = 16;

/// The highest kind of contact between two solids that touch without
/// overlapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContactKind {
    /// They meet in isolated points: a corner on a face, an edge, or a
    /// corner; a ball on a plane.
    Point,
    /// They meet along curves but share no area: an edge on a face or on
    /// an edge, a cylinder lying on a plane.
    Edge,
    /// They share an area of their boundaries: a face against a face.
    Face,
}

impl ContactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ContactKind::Point => "point",
            ContactKind::Edge => "edge",
            ContactKind::Face => "face",
        }
    }
}

/// What [`interference`] found.
#[derive(Clone, Debug, PartialEq)]
pub enum Interference {
    /// No point in common (no contact within the tolerance).
    Clear,
    /// Contact without common volume. `at` holds up to
    /// [`MAX_CONTACT_POINTS`] places where they meet, in the order they were
    /// found; `faces_a` and `faces_b` the faces of each solid that the
    /// contact is on, ascending.
    Touch {
        kind: ContactKind,
        at: Vec<Vec3>,
        faces_a: Vec<FaceId>,
        faces_b: Vec<FaceId>,
    },
    /// A common volume: the intersection of the two solids. Its faces'
    /// sources (`merged`) name the faces of `a` and `b` that bound it.
    Overlap(Box<BooleanResult>),
}

/// Whether `a` and `b` are clear, touch or overlap.
///
/// Solids whose boxes, grown by twice the tolerance, do not meet are clear
/// at the cost of the two boxes. Otherwise their intersection is computed
/// by [`boolean_with`] (parts of it that touch are allowed): a result is
/// the overlap. When it is empty, the boolean's section (the curves and
/// points in which their faces meet, a corner of one standing on a face of
/// the other, faces on one surface) says whether and how they touch; where
/// faces of both lie on one surface facing each other, their union decides
/// whether they share an area (the two solids become one shell) or only an
/// edge of it. Every refusal of the boolean (contact at the edge of
/// tolerance, geometry not handled) is an error, never "clear".
pub fn interference(
    a: &Body,
    b: &Body,
    tolerance: &Tolerance,
) -> Result<Interference, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0 && tolerance.angular.is_finite() && tolerance.angular > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if !boxes_meet(a, b, 2.0 * tol) {
        return Ok(Interference::Clear);
    }
    let options = BooleanOptions {
        refuse_zero_thickness: false,
        ..BooleanOptions::default()
    };
    match boolean_with(a, b, BooleanOp::Intersect, tolerance, &options) {
        Ok(common) => return Ok(Interference::Overlap(Box::new(common))),
        Err(KernelError::EmptyResult { .. }) => {}
        Err(other) => return Err(other),
    }
    let unsupported = |stop| match stop {
        Stop::Error(error) => error,
        Stop::Unsupported { reason, mut faces } => {
            faces.truncate(MAX_REPORTED);
            KernelError::Unsupported {
                operation: "interference",
                reason,
                faces,
            }
        }
    };
    let found = evidence(a, b, tolerance).map_err(unsupported)?;
    let Some(mut kind) = found.kind else {
        return Ok(Interference::Clear);
    };
    if found.facing {
        // Faces of both on one surface, facing each other: an area in
        // common joins the two into one shell of their union; an edge or a
        // corner in common leaves them two shells.
        let union = boolean_with(a, b, BooleanOp::Union, tolerance, &options)?;
        if joined(&union) {
            kind = ContactKind::Face;
        }
    }
    Ok(Interference::Touch {
        kind,
        at: found.at,
        faces_a: found.faces_a.into_iter().collect(),
        faces_b: found.faces_b.into_iter().collect(),
    })
}

/// The boxes of the two bodies, grown by `margin`, meet. A body without a
/// box (not finite) is taken to meet everything, so that the boolean
/// decides and reports it.
fn boxes_meet(a: &Body, b: &Body, margin: f64) -> bool {
    let corners = |body: &Body| body.bbox().ok().as_ref().and_then(Bbox3::corners);
    let (Some((alo, ahi)), Some((blo, bhi))) = (corners(a), corners(b)) else {
        return true;
    };
    alo.x <= bhi.x + margin
        && blo.x <= ahi.x + margin
        && alo.y <= bhi.y + margin
        && blo.y <= ahi.y + margin
        && alo.z <= bhi.z + margin
        && blo.z <= ahi.z + margin
}

/// Some shell of the result has faces of both operands.
fn joined(result: &BooleanResult) -> bool {
    result.body.shells.iter().any(|shell| {
        let (mut from_a, mut from_b) = (false, false);
        for face in &shell.faces {
            for source in result.merged.get(face.0 as usize).into_iter().flatten() {
                match source.operand {
                    Operand::A => from_a = true,
                    Operand::B => from_b = true,
                }
            }
        }
        from_a && from_b
    })
}

/// Where two solids that have no volume in common meet.
#[derive(Default)]
struct Evidence {
    /// The highest kind seen short of a shared area: `Edge` for a curve
    /// of positive length, `Point` for points only, `None` for nothing.
    kind: Option<ContactKind>,
    /// Faces of both lie on one surface with opposite normals.
    facing: bool,
    at: Vec<Vec3>,
    faces_a: BTreeSet<FaceId>,
    faces_b: BTreeSet<FaceId>,
}

impl Evidence {
    fn raise(&mut self, kind: ContactKind) {
        self.kind = Some(self.kind.map_or(kind, |k| k.max(kind)));
    }

    fn point(&mut self, p: Vec3, tol: f64) {
        if p.is_finite()
            && self.at.len() < MAX_CONTACT_POINTS
            && !self.at.iter().any(|q| q.distance(p) <= tol)
        {
            self.at.push(p);
        }
    }
}

/// The boolean's section of `a` and `b`, read for contact.
fn evidence(a: &Body, b: &Body, tolerance: &Tolerance) -> Result<Evidence, Stop> {
    let tol = tolerance.linear;
    let mut vertices = Vertices::new(0.5 * tol, 2.0 * tol);
    // Both passed the check in the intersection just made.
    let sa = section::prepare(a, Operand::A, &mut vertices, tolerance, true)?;
    let sb = section::prepare(b, Operand::B, &mut vertices, tolerance, true)?;
    let sections = section::section(&sa, &sb, &mut vertices, tolerance)?;
    let mut found = Evidence::default();
    let mut points: Vec<Vec3> = Vec::new();
    for stretch in &sections.stretches {
        found.faces_a.insert(sa.name(stretch.fa).1);
        found.faces_b.insert(sb.name(stretch.fb).1);
        let (t0, t1) = stretch.range;
        let ends = [
            stretch.curve.eval(t0),
            stretch.curve.eval(0.5 * (t0 + t1)),
            stretch.curve.eval(t1),
        ];
        let long = ends[0].distance(ends[1]) > 2.0 * tol || ends[1].distance(ends[2]) > 2.0 * tol;
        found.raise(if long {
            ContactKind::Edge
        } else {
            ContactKind::Point
        });
        for p in ends {
            found.point(p, tol);
        }
    }
    for (fa, list) in &sections.coincident_a {
        for &(fb, agree) in list {
            if !agree {
                found.facing = true;
                found.faces_a.insert(sa.name(*fa).1);
                found.faces_b.insert(sb.name(fb).1);
            }
        }
    }
    // Where vertices of both meet, or a vertex of one lies on an edge or a
    // face of the other.
    points.extend(
        vertices
            .points
            .iter()
            .zip(&vertices.meetings)
            .filter(|(_, m)| {
                let a = m.operands & 1 != 0 || m.a_edge.is_some() || m.a_face.is_some();
                let b = m.operands & 2 != 0 || m.b_edge.is_some() || m.b_face.is_some();
                a && b
            })
            .map(|(p, _)| *p),
    );
    points.extend_from_slice(&sections.touches);
    points.extend(on_the_other(&sa, &sb, tol));
    points.extend(on_the_other(&sb, &sa, tol));
    for p in points {
        let (on_a, on_b) = (faces_at(&sa, p, tol), faces_at(&sb, p, tol));
        if on_a.is_empty() || on_b.is_empty() {
            continue;
        }
        found.raise(ContactKind::Point);
        found.point(p, tol);
        found.faces_a.extend(on_a);
        found.faces_b.extend(on_b);
    }
    Ok(found)
}

/// The faces of `solid` that `p` lies on: within twice the tolerance of
/// the surface and not outside the boundary.
fn faces_at(solid: &Solid, p: Vec3, tol: f64) -> Vec<FaceId> {
    let near = 2.0 * tol;
    (0..solid.faces())
        .filter(|&f| solid.boxes.get(f).is_some_and(|b| b.contains(p, near)))
        .filter(|&f| {
            solid.outline(f).is_ok_and(|outline| {
                outline.surface.project(p).is_ok_and(|(u, v)| {
                    let foot = outline.surface.eval(u, v);
                    foot.distance(p) <= near
                        && matches!(
                            outline.side(foot, tol),
                            Ok(outline::Side::In | outline::Side::On)
                        )
                })
            })
        })
        .map(|f| solid.name(f).1)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::unreachable)]

    use super::*;
    use crate::primitives;
    use craft3d_math::Frame;

    fn block(at: Vec3, x: f64, y: f64, z: f64) -> Body {
        primitives::block(&Frame::WORLD.at(at).unwrap(), x, y, z).unwrap()
    }

    fn volume(common: &BooleanResult) -> f64 {
        craft3d_topo::mass_properties(&common.body).unwrap().volume
    }

    fn touch_kind(found: &Interference) -> ContactKind {
        match found {
            Interference::Touch { kind, .. } => *kind,
            other => panic!("not a touch: {other:?}"),
        }
    }

    /// Closed forms for boxes at three scales: overlap volume, a face, an
    /// edge and a corner in common, and apart.
    #[test]
    fn boxes_overlap_touch_or_are_clear() {
        for s in [1e-3, 1.0, 1e4] {
            let tolerance = Tolerance {
                linear: 1e-7 * s,
                ..Tolerance::default()
            };
            let a = block(Vec3::ZERO, 2.0 * s, 2.0 * s, 2.0 * s);
            let v = |x: f64, y: f64, z: f64| Vec3::new(x * s, y * s, z * s);
            // Overlap: [1,2] x [0.5,2] x [0,2] = 1 * 1.5 * 2.
            let b = block(v(1.0, 0.5, 0.0), 2.0 * s, 2.0 * s, 2.0 * s);
            let Interference::Overlap(common) = interference(&a, &b, &tolerance).unwrap() else {
                panic!("expected an overlap at scale {s}");
            };
            let want = 3.0 * s * s * s;
            assert!((volume(&common) - want).abs() <= 1e-9 * want, "scale {s}");
            // A face against a face, offset sideways.
            let face = block(v(2.0, 0.5, 0.5), 1.0 * s, 1.0 * s, 1.0 * s);
            let found = interference(&a, &face, &tolerance).unwrap();
            assert_eq!(touch_kind(&found), ContactKind::Face, "scale {s}");
            // An edge on an edge.
            let edge = block(v(2.0, 2.0, 0.0), 1.0 * s, 1.0 * s, 2.0 * s);
            let found = interference(&a, &edge, &tolerance).unwrap();
            assert_eq!(touch_kind(&found), ContactKind::Edge, "scale {s}");
            // A corner on a corner.
            let corner = block(v(2.0, 2.0, 2.0), 1.0 * s, 1.0 * s, 1.0 * s);
            let found = interference(&a, &corner, &tolerance).unwrap();
            assert_eq!(touch_kind(&found), ContactKind::Point, "scale {s}");
            let Interference::Touch { at, .. } = &found else {
                unreachable!()
            };
            assert!(at.iter().any(|p| p.distance(v(2.0, 2.0, 2.0)) <= 1e-9 * s));
            // Apart.
            let apart = block(v(3.0, 0.0, 0.0), 1.0 * s, 1.0 * s, 1.0 * s);
            assert_eq!(
                interference(&a, &apart, &tolerance).unwrap(),
                Interference::Clear
            );
        }
    }

    /// A plate and curved or tilted solids on it, at three scales: a
    /// cylinder through it (overlap pi r^2 t), lying on it (along a line),
    /// a ball on it (a point), a tilted block on an edge and on a corner.
    #[test]
    fn curved_and_tilted_solids_on_a_plate() {
        for s in [1e-3, 1.0, 1e4] {
            let tolerance = Tolerance {
                linear: 1e-7 * s,
                ..Tolerance::default()
            };
            let v = |x: f64, y: f64, z: f64| Vec3::new(x * s, y * s, z * s);
            let plate = block(Vec3::ZERO, 4.0 * s, 4.0 * s, 0.5 * s);
            let up = Frame::WORLD.at(v(2.0, 2.0, -1.0)).unwrap();
            let pin = primitives::cylinder(&up, 0.5 * s, 3.0 * s).unwrap();
            let Interference::Overlap(common) = interference(&plate, &pin, &tolerance).unwrap()
            else {
                panic!("expected an overlap at scale {s}");
            };
            let want = std::f64::consts::PI * 0.25 * 0.5 * s * s * s;
            assert!((volume(&common) - want).abs() <= 1e-7 * want, "scale {s}");
            let lying = Frame::new(v(0.5, 2.0, 1.0), Vec3::X, Vec3::Y).unwrap();
            let log = primitives::cylinder(&lying, 0.5 * s, 3.0 * s).unwrap();
            let found = interference(&plate, &log, &tolerance).unwrap();
            assert_eq!(touch_kind(&found), ContactKind::Edge, "scale {s}");
            let ball = primitives::sphere(&Frame::WORLD.at(v(2.0, 2.0, 1.5)).unwrap(), s).unwrap();
            let found = interference(&plate, &ball, &tolerance).unwrap();
            assert_eq!(touch_kind(&found), ContactKind::Point, "scale {s}");
            // Boxes overlap, the solids do not.
            let near =
                primitives::sphere(&Frame::WORLD.at(v(-0.7, -0.7, 1.2)).unwrap(), s).unwrap();
            assert_eq!(
                interference(&plate, &near, &tolerance).unwrap(),
                Interference::Clear,
                "scale {s}"
            );
            let ridge = Frame::new(v(1.0, 1.0, 0.5), Vec3::new(0.0, -1.0, 1.0), Vec3::X).unwrap();
            let roof = primitives::block(&ridge, 2.0 * s, s, s).unwrap();
            let found = interference(&plate, &roof, &tolerance).unwrap();
            assert_eq!(touch_kind(&found), ContactKind::Edge, "scale {s}");
            let (c, h) = ((2.0_f64 / 3.0).sqrt(), 3.0_f64.sqrt().recip());
            let tip = Frame::new(
                v(2.0, 2.0, 0.5),
                Vec3::new(-0.5 * c, -0.75_f64.sqrt() * c, h),
                Vec3::new(c, 0.0, h),
            )
            .unwrap();
            assert!(tip.x().z > 0.0 && tip.y().z > 0.0 && tip.z().z > 0.0);
            let cube = primitives::block(&tip, s, s, s).unwrap();
            let found = interference(&plate, &cube, &tolerance).unwrap();
            assert_eq!(touch_kind(&found), ContactKind::Point, "scale {s}");
            let Interference::Touch { faces_a, at, .. } = &found else {
                unreachable!()
            };
            assert_eq!(faces_a.len(), 1, "only the top of the plate");
            assert!(at.iter().any(|p| p.distance(v(2.0, 2.0, 0.5)) <= 1e-9 * s));
        }
    }
}
