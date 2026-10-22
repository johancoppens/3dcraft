//! Questions asked of one solid without changing it, for design
//! verification (docs/design/verification.md): where a line meets its
//! faces, whether a point of a face's surface lies on the face, where a
//! point is. The same pieces the booleans classify with: each face's
//! outline (its loops on its surface) and the exact intersection of a
//! curve with a surface ([`super::hits::curve_hits`]).

use super::hits::curve_hits;
use super::outline::{Side, line_in_box, outward_normal};
use super::section::{self, Solid};
use super::{NOT_SOLID, Operand, Stop, Vertices, classify, geometry};
use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::intersect::CurveIntersection;
use craft3d_geom::{Curve3, Line};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind};

pub use super::Place;

/// Where a point of a face's surface is with respect to the face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnFace {
    /// Inside the face's loops.
    In,
    /// Within the tolerance of its boundary.
    Boundary,
    Out,
}

/// One place where a line meets a face of the solid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineHit {
    /// The face's index in the body.
    pub face: usize,
    /// The line's parameter: the distance from its origin along its unit
    /// direction.
    pub t: f64,
    pub point: Vec3,
    /// The line touches the surface here without crossing it.
    pub tangent: bool,
    /// The point is within the tolerance of the face's boundary (an edge
    /// or a vertex): the faces on the other side of that edge are met at
    /// the same place.
    pub boundary: bool,
    /// The line lies on the face's surface; `t` and `point` are where it
    /// enters the face's box.
    pub along: bool,
    /// The face's outward normal at the point dotted with the line's
    /// direction: negative where the line goes into the material, positive
    /// where it comes out. `None` where there is no normal (an apex, a
    /// line along the surface).
    pub facing: Option<f64>,
}

/// A solid made ready to be asked questions: its faces' outlines are made
/// when first needed and kept for the next question.
pub struct Probe<'a> {
    solid: Solid<'a>,
    tolerance: Tolerance,
}

fn refused(stop: Stop) -> KernelError {
    match stop {
        Stop::Error(error) => error,
        Stop::Unsupported { reason, mut faces } => {
            faces.truncate(MAX_REPORTED);
            KernelError::Unsupported {
                operation: "probe",
                reason,
                faces,
            }
        }
    }
}

impl<'a> Probe<'a> {
    /// Checks `body` (remembered per body, as for an operand of a
    /// boolean) and gets it ready. Sheets are refused: they have no
    /// inside.
    pub fn new(body: &'a Body, tolerance: &Tolerance) -> Result<Self, KernelError> {
        let tol = tolerance.linear;
        if !(tol.is_finite()
            && tol > 0.0
            && tolerance.angular.is_finite()
            && tolerance.angular > 0.0)
        {
            return Err(KernelError::InvalidSize {
                name: "tolerance",
                value: tol,
            });
        }
        if body.kind != BodyKind::Solid {
            return Err(refused(Stop::of(NOT_SOLID)));
        }
        let mut vertices = Vertices::new(0.0, 0.0);
        let solid =
            section::prepare(body, Operand::A, &mut vertices, tolerance, false).map_err(refused)?;
        Ok(Probe {
            solid,
            tolerance: *tolerance,
        })
    }

    pub fn body(&self) -> &'a Body {
        self.solid.body
    }

    /// Where `point`, a point on or near the surface of face `face`, is
    /// with respect to the face's loops.
    pub fn on_face(&self, face: usize, point: Vec3) -> Result<OnFace, KernelError> {
        let outline = self.solid.outline(face).map_err(refused)?;
        let named = [self.solid.name(face)];
        match outline
            .side(point, self.tolerance.linear)
            .map_err(|r| refused(Stop::of(r).at(&named)))?
        {
            Side::In => Ok(OnFace::In),
            Side::On => Ok(OnFace::Boundary),
            Side::Out => Ok(OnFace::Out),
        }
    }

    /// The outward normal of face `face` at `point` of its surface.
    pub fn normal(&self, face: usize, point: Vec3) -> Result<Option<Vec3>, KernelError> {
        let outline = self.solid.outline(face).map_err(refused)?;
        Ok(outward_normal(outline.sheet(), outline.reversed, point))
    }

    /// Where `point` is with respect to the solid: inside, outside, on its
    /// boundary (within twice the tolerance) or unknown (every ray tried
    /// grazed the solid).
    pub fn place(&self, point: Vec3) -> Result<Place, KernelError> {
        match classify::inside(&self.solid, point, &self.tolerance) {
            Ok(true) => Ok(Place::Inside),
            Ok(false) => Ok(Place::Outside),
            Err(Stop::Unsupported { reason, .. }) if reason == super::ON_A_FACE => {
                Ok(Place::OnBoundary)
            }
            Err(Stop::Unsupported { .. }) => Ok(Place::Unknown),
            Err(stop) => Err(refused(stop)),
        }
    }

    /// Every place where the line `origin + t·direction` (the direction is
    /// made a unit vector) meets a face of the solid for `t` in `[from,
    /// to]` (`to` may be infinite: the faces' boxes bound it), in
    /// ascending `t` and then face order. A line that crosses an edge
    /// meets each face along it there, marked `boundary`.
    pub fn line_hits(
        &self,
        origin: Vec3,
        direction: Vec3,
        from: f64,
        to: f64,
    ) -> Result<Vec<LineHit>, KernelError> {
        let tol = self.tolerance.linear;
        if from.is_nan() || to.is_nan() || from > to {
            return Err(KernelError::InvalidSize {
                name: "line range",
                value: from,
            });
        }
        let line = Line::new(origin, direction).map_err(KernelError::Geometry)?;
        let (origin, direction) = (line.origin(), line.direction());
        let ray = Curve3::Line(line);
        let solid = &self.solid;
        let reach = solid.candidates(|b| {
            line_in_box(origin, direction, b, 2.0 * tol)
                .is_some_and(|(enter, leave)| !(leave < from || enter > to))
        });
        let mut found = Vec::new();
        for face in reach {
            let Some(box_) = solid.boxes.get(face) else {
                continue;
            };
            let Some((enter, leave)) = line_in_box(origin, direction, box_, 2.0 * tol) else {
                continue;
            };
            let lo = enter.max(from);
            let hi = leave.min(to);
            if !(lo.is_finite() && hi.is_finite()) || lo > hi {
                continue;
            }
            // A little past both ends, so that neither end is a place
            // where the line is found near the surface without crossing.
            let pad = 16.0 * tol + 0.01 * (hi - lo);
            let outline = solid.outline(face).map_err(refused)?;
            let named = [solid.name(face)];
            let hits =
                match curve_hits(&ray, (lo - pad, hi + pad), outline.sheet(), &self.tolerance)
                    .map_err(|e| refused(geometry(e, &named)))?
                {
                    CurveIntersection::OnSurface => {
                        // The line lies on the surface: met where a point of it
                        // within the box is on the face.
                        for t in [lo, 0.5 * (lo + hi), hi] {
                            let point = origin + direction * t;
                            if outline
                                .side(point, tol)
                                .map_err(|r| refused(Stop::of(r).at(&named)))?
                                != Side::Out
                            {
                                found.push(LineHit {
                                    face,
                                    t,
                                    point,
                                    tangent: true,
                                    boundary: false,
                                    along: true,
                                    facing: None,
                                });
                                break;
                            }
                        }
                        continue;
                    }
                    CurveIntersection::Hits(hits) => hits,
                };
            for hit in hits {
                if hit.t < from - tol || hit.t > to + tol || !box_.contains(hit.point, 2.0 * tol) {
                    continue;
                }
                let side = outline
                    .side(hit.point, tol)
                    .map_err(|r| refused(Stop::of(r).at(&named)))?;
                if side == Side::Out {
                    continue;
                }
                let facing = outward_normal(outline.sheet(), outline.reversed, hit.point)
                    .map(|n| n.dot(direction))
                    .filter(|d| d.is_finite());
                found.push(LineHit {
                    face,
                    t: hit.t,
                    point: hit.point,
                    tangent: hit.tangent,
                    boundary: side == Side::On,
                    along: false,
                    facing,
                });
            }
        }
        found.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.face.cmp(&b.face)));
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Frame;

    #[test]
    fn a_line_through_a_block_meets_two_faces_at_the_exact_distances() {
        let block = crate::primitives::block(&Frame::WORLD, 2.0, 3.0, 4.0).unwrap();
        let probe = Probe::new(&block, &Tolerance::default()).unwrap();
        let hits = probe
            .line_hits(
                Vec3::new(1.0, 1.5, -10.0),
                Vec3::new(0.0, 0.0, 1.0),
                0.0,
                f64::INFINITY,
            )
            .unwrap();
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!((hits[0].t - 10.0).abs() < 1e-12);
        assert!((hits[1].t - 14.0).abs() < 1e-12);
        assert!(hits[0].facing.unwrap() < 0.0 && hits[1].facing.unwrap() > 0.0);
        assert_eq!(
            probe.place(Vec3::new(1.0, 1.0, 1.0)).unwrap(),
            Place::Inside
        );
        assert_eq!(
            probe.place(Vec3::new(5.0, 1.0, 1.0)).unwrap(),
            Place::Outside
        );
        // Behind the origin nothing is met.
        let none = probe
            .line_hits(
                Vec3::new(1.0, 1.5, 10.0),
                Vec3::new(0.0, 0.0, 1.0),
                0.0,
                f64::INFINITY,
            )
            .unwrap();
        assert!(none.is_empty());
    }
}
