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
        /// For [`ContactKind::Face`], the area shared; 0 otherwise.
        area: f64,
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
/// whether they share an area (the boundary of the union is smaller than
/// both boundaries by twice that area) or only an edge of it; a face's
/// orientation counts, so a pin in a hole and a body in a cavity face
/// the walls they lie against. Every refusal of the boolean (contact at the edge of
/// tolerance, geometry not handled) is an error, never "clear".
pub fn interference(
    a: &Body,
    b: &Body,
    tolerance: &Tolerance,
) -> Result<Interference, KernelError> {
    super::freeform::holding(|| interference_held(a, b, tolerance, true))
}

/// The common volume of `a` and `b`, if they have one: the first step of
/// [`interference`] alone, for a question that needs no more than that
/// (whether bodies overlap, and by how much). `None` when they have no
/// volume in common, whether they touch or are apart: the intersection is
/// empty (parts of it that touch are allowed). Solids whose boxes, grown
/// by twice the tolerance, do not meet cost only the two boxes. What the
/// boolean refuses is an error, as in [`interference`]; the contact that
/// [`interference`] would go on to classify (and the union it may build
/// for that) is not looked at, so a touch that only that union refuses
/// is no error here.
pub fn common_volume(
    a: &Body,
    b: &Body,
    tolerance: &Tolerance,
) -> Result<Option<BooleanResult>, KernelError> {
    super::freeform::holding(|| common_volume_held(a, b, tolerance))
}

/// [`common_volume`], its trees held to a budget (by the caller).
fn common_volume_held(
    a: &Body,
    b: &Body,
    tolerance: &Tolerance,
) -> Result<Option<BooleanResult>, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0 && tolerance.angular.is_finite() && tolerance.angular > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if a.kind != craft3d_topo::BodyKind::Solid || b.kind != craft3d_topo::BodyKind::Solid {
        return Err(KernelError::Unsupported {
            operation: "interference",
            reason: super::NOT_SOLID,
            faces: Vec::new(),
        });
    }
    if !boxes_meet(a, b, 2.0 * tol) {
        return Ok(None);
    }
    match boolean_with(a, b, BooleanOp::Intersect, tolerance, &touching()) {
        Ok(common) => Ok(Some(common)),
        Err(KernelError::EmptyResult { .. }) => Ok(None),
        Err(other) => Err(other),
    }
}

/// The boolean's options for these queries: parts that touch are allowed.
fn touching() -> BooleanOptions {
    BooleanOptions {
        refuse_zero_thickness: false,
        ..BooleanOptions::default()
    }
}

/// Whether `a` and `b` are clear, touch or overlap, and nothing more:
/// [`interference`] without the area two faces on one surface share,
/// which takes the union of the two and the boundaries of all three
/// integrated (for a body of spline faces, most of the time of the
/// whole). The answer is the one [`interference`] gives, its kind of
/// touch aside; a touch whose shared area only that union could measure,
/// and which the union refuses, is a touch here rather than an error.
pub fn contact(a: &Body, b: &Body, tolerance: &Tolerance) -> Result<Contact, KernelError> {
    super::freeform::holding(|| {
        Ok(match interference_held(a, b, tolerance, false)? {
            Interference::Clear => Contact::Clear,
            Interference::Touch { .. } => Contact::Touch,
            Interference::Overlap(_) => Contact::Overlap,
        })
    })
}

/// What [`contact`] tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contact {
    Clear,
    Touch,
    Overlap,
}

/// [`interference`], its trees held to a budget
/// ([`super::freeform::holding`]); without `shared_area` the area that
/// faces on one surface share is not measured ([`contact`]).
fn interference_held(
    a: &Body,
    b: &Body,
    tolerance: &Tolerance,
    shared_area: bool,
) -> Result<Interference, KernelError> {
    // The checks of the tolerance and of the kinds, the box test and the
    // intersection: a common volume is the overlap.
    if let Some(common) = common_volume_held(a, b, tolerance)? {
        return Ok(Interference::Overlap(Box::new(common)));
    }
    let tol = tolerance.linear;
    if !boxes_meet(a, b, 2.0 * tol) {
        return Ok(Interference::Clear);
    }
    let options = touching();
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
    let mut area = 0.0;
    if found.facing && shared_area {
        // Faces of both on one surface, facing each other. Where they share
        // an area, that area is inside the union: its boundary is smaller
        // than both boundaries together by twice the area. That holds for a
        // block against a block, a pin that fills a hole and a body that
        // fills a cavity alike; an edge or a corner in common takes no
        // area away.
        let union = boolean_with(a, b, BooleanOp::Union, tolerance, &options)?;
        let measure = |body: &Body| {
            craft3d_topo::mass_properties(body)
                .map(|m| m.area)
                .ok()
                .filter(|area| area.is_finite())
                .ok_or(KernelError::Unsupported {
                    operation: "interference",
                    reason: "the boundary of a body or of the union could not be measured",
                    faces: Vec::new(),
                })
        };
        let both = measure(a)? + measure(b)?;
        let shared = 0.5 * (both - measure(&union.body)?);
        if shared > (tol * both.sqrt()).max(1e-9 * both) {
            kind = ContactKind::Face;
            area = shared;
        }
    }
    Ok(Interference::Touch {
        kind,
        area,
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
        for &(fb, same_normal) in list {
            // Their outward normals, not their surfaces', point opposite
            // ways (a reversed face is a hole's wall or a cavity's).
            let reversed =
                |solid: &Solid, f: usize| solid.body.faces.get(f).is_some_and(|face| face.reversed);
            if same_normal != (reversed(&sa, *fa) == reversed(&sb, fb)) {
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
            let Interference::Touch { area, .. } = &found else {
                unreachable!()
            };
            assert!((area - s * s).abs() <= 1e-9 * s * s, "scale {s}: {area}");
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

    /// `common_volume` answers what `interference` answers about volume:
    /// the same common volume for an overlap (closed form), none for a
    /// face, an edge or a corner in common, a pin lying on a plate, a ball
    /// on it, boxes that meet around solids that do not, and apart; at
    /// three scales. Its refusals are the boolean's (a sheet is not a
    /// solid, a tolerance that is not positive).
    #[test]
    fn the_common_volume_alone_agrees_with_interference() {
        for s in [1e-3, 1.0, 1e4] {
            let tolerance = Tolerance {
                linear: 1e-7 * s,
                ..Tolerance::default()
            };
            let v = |x: f64, y: f64, z: f64| Vec3::new(x * s, y * s, z * s);
            let a = block(Vec3::ZERO, 2.0 * s, 2.0 * s, 2.0 * s);
            let b = block(v(1.0, 0.5, 0.0), 2.0 * s, 2.0 * s, 2.0 * s);
            let common = common_volume(&a, &b, &tolerance).unwrap().unwrap();
            let want = 3.0 * s * s * s;
            assert!((volume(&common) - want).abs() <= 1e-9 * want, "scale {s}");
            let plate = block(Vec3::ZERO, 4.0 * s, 4.0 * s, 0.5 * s);
            let up = Frame::WORLD.at(v(2.0, 2.0, -1.0)).unwrap();
            let pin = primitives::cylinder(&up, 0.5 * s, 3.0 * s).unwrap();
            let common = common_volume(&plate, &pin, &tolerance).unwrap().unwrap();
            let want = std::f64::consts::PI * 0.25 * 0.5 * s * s * s;
            assert!((volume(&common) - want).abs() <= 1e-7 * want, "scale {s}");
            let lying = Frame::new(v(0.5, 2.0, 1.0), Vec3::X, Vec3::Y).unwrap();
            let none = [
                (a.clone(), block(v(2.0, 0.5, 0.5), s, s, s)),
                (a.clone(), block(v(2.0, 2.0, 0.0), s, s, 2.0 * s)),
                (a.clone(), block(v(2.0, 2.0, 2.0), s, s, s)),
                (a.clone(), block(v(3.0, 0.0, 0.0), s, s, s)),
                (
                    plate.clone(),
                    primitives::cylinder(&lying, 0.5 * s, 3.0 * s).unwrap(),
                ),
                (
                    plate.clone(),
                    primitives::sphere(&Frame::WORLD.at(v(2.0, 2.0, 1.5)).unwrap(), s).unwrap(),
                ),
                (
                    plate.clone(),
                    primitives::sphere(&Frame::WORLD.at(v(-0.7, -0.7, 1.2)).unwrap(), s).unwrap(),
                ),
            ];
            for (n, (x, y)) in none.iter().enumerate() {
                assert!(
                    common_volume(x, y, &tolerance).unwrap().is_none(),
                    "case {n} at scale {s}"
                );
                assert!(
                    !matches!(
                        interference(x, y, &tolerance).unwrap(),
                        Interference::Overlap(_)
                    ),
                    "case {n} at scale {s}"
                );
            }
            let bad = Tolerance {
                linear: 0.0,
                ..tolerance
            };
            assert!(common_volume(&a, &b, &bad).is_err());
        }
    }

    /// `contact` answers clear, touch or overlap as `interference` does,
    /// for a face, an edge and a corner in common, a block in a pocket it
    /// fills (faces on one surface facing each other, whose shared area
    /// `interference` measures with a union), an overlap and apart; at
    /// three scales. Its refusals are the boolean's.
    #[test]
    fn contact_alone_agrees_with_interference() {
        for s in [1e-3, 1.0, 1e4] {
            let tolerance = Tolerance {
                linear: 1e-7 * s,
                ..Tolerance::default()
            };
            let v = |x: f64, y: f64, z: f64| Vec3::new(x * s, y * s, z * s);
            let a = block(Vec3::ZERO, 2.0 * s, 2.0 * s, 2.0 * s);
            let pocket = crate::boolean::boolean(
                &a,
                &block(v(0.5, 0.5, 1.0), s, s, 2.0 * s),
                crate::boolean::BooleanOp::Subtract,
                &tolerance,
            )
            .unwrap()
            .body;
            let cases = [
                (
                    a.clone(),
                    block(v(1.0, 0.5, 0.0), 2.0 * s, 2.0 * s, 2.0 * s),
                    Contact::Overlap,
                ),
                (a.clone(), block(v(2.0, 0.5, 0.5), s, s, s), Contact::Touch),
                (
                    a.clone(),
                    block(v(2.0, 2.0, 0.0), s, s, 2.0 * s),
                    Contact::Touch,
                ),
                (a.clone(), block(v(2.0, 2.0, 2.0), s, s, s), Contact::Touch),
                (pocket, block(v(0.5, 0.5, 1.0), s, s, s), Contact::Touch),
                (a.clone(), block(v(3.0, 0.0, 0.0), s, s, s), Contact::Clear),
            ];
            for (n, (x, y, want)) in cases.iter().enumerate() {
                let got = contact(x, y, &tolerance).unwrap();
                assert_eq!(got, *want, "case {n} at scale {s}");
                let full = match interference(x, y, &tolerance).unwrap() {
                    Interference::Clear => Contact::Clear,
                    Interference::Touch { .. } => Contact::Touch,
                    Interference::Overlap(_) => Contact::Overlap,
                };
                assert_eq!(got, full, "case {n} at scale {s}");
            }
            let bad = Tolerance {
                linear: 0.0,
                ..tolerance
            };
            assert!(contact(&a, &a, &bad).is_err());
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

    fn cut(a: &Body, b: &Body) -> Body {
        boolean_with(
            a,
            b,
            BooleanOp::Subtract,
            &Tolerance::default(),
            &BooleanOptions::default(),
        )
        .unwrap()
        .body
    }

    /// Contact with reversed faces: a pin that fits a hole exactly, a block
    /// flush against an inner wall of an open box, and a body that fills a
    /// closed cavity, each sharing an area of known size.
    #[test]
    fn walls_of_holes_and_cavities_are_faced() {
        for s in [1e-3, 1.0, 1e4] {
            let tolerance = Tolerance {
                linear: 1e-7 * s,
                ..Tolerance::default()
            };
            let v = |x: f64, y: f64, z: f64| Vec3::new(x * s, y * s, z * s);
            let face_area = |found: &Interference| match found {
                Interference::Touch {
                    kind: ContactKind::Face,
                    area,
                    ..
                } => *area,
                other => panic!("scale {s}: not a face contact: {other:?}"),
            };
            let at = |p: Vec3| Frame::WORLD.at(p).unwrap();
            // A plate 1 thick with a hole of radius 2, and a pin of radius 2
            // through it: the hole's wall, 2 pi r t.
            let plate = block(Vec3::ZERO, 10.0 * s, 10.0 * s, s);
            let drill = primitives::cylinder(&at(v(5.0, 5.0, -1.0)), 2.0 * s, 3.0 * s).unwrap();
            let holed = cut(&plate, &drill);
            let pin = primitives::cylinder(&at(v(5.0, 5.0, -2.0)), 2.0 * s, 5.0 * s).unwrap();
            let found = interference(&holed, &pin, &tolerance).unwrap();
            let want = 2.0 * std::f64::consts::PI * 2.0 * s * s;
            let area = face_area(&found);
            assert!((area - want).abs() <= 1e-6 * want, "scale {s}: {area}");
            // An open box, walls 1 thick, and a block against an inner wall.
            let outer = block(Vec3::ZERO, 10.0 * s, 10.0 * s, 6.0 * s);
            let inside = block(v(1.0, 1.0, 1.0), 8.0 * s, 8.0 * s, 6.0 * s);
            let open = cut(&outer, &inside);
            let flush = block(v(1.0, 2.0, 2.0), 2.0 * s, 2.0 * s, 2.0 * s);
            let area = face_area(&interference(&open, &flush, &tolerance).unwrap());
            assert!(
                (area - 4.0 * s * s).abs() <= 1e-9 * s * s,
                "scale {s}: {area}"
            );
            // A closed cavity and the body that fills it: all six walls.
            let cavity = block(v(2.0, 2.0, 2.0), 6.0 * s, 6.0 * s, 6.0 * s);
            let hollow = cut(&block(Vec3::ZERO, 10.0 * s, 10.0 * s, 10.0 * s), &cavity);
            let area = face_area(&interference(&hollow, &cavity, &tolerance).unwrap());
            assert!(
                (area - 216.0 * s * s).abs() <= 1e-9 * 216.0 * s * s,
                "scale {s}: {area}"
            );
        }
    }
}
