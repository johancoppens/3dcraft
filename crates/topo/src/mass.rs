//! Area, volume and centroid from the exact B-rep, independent of any
//! tessellation: the backbone of the test oracles (docs/testing.md).
//!
//! Method. By the divergence theorem the volume and its first moments are
//! surface integrals over the faces; with the surface's own parametrisation
//! each becomes `∬_D f(u, v) du dv` over the face's parameter domain `D`:
//!
//! - area:    `f = |Sᵤ × Sᵥ|`
//! - volume:  `f = S · (Sᵤ × Sᵥ) / 3`
//! - moments: `f = S_k² (Sᵤ × Sᵥ)_k / 2` for each coordinate `k`
//!
//! `D` is only known through its boundary, the face's loops. Green's theorem
//! turns the domain integral into a boundary one:
//! `∬_D f du dv = -∮ G du`, with `G(u, v) = ∫ f(u, t) dt` from a reference
//! `v₀` to `v`. Both integrals are Gauss–Legendre sums; the boundary is
//! followed along the edge curves, projected onto the surface.
//!
//! On a surface closed around its axis a loop may wind around it (the rim
//! of a cone, the two ends of a cylinder wall). `G` is periodic, so nothing
//! changes, except that a face bounded by a single such loop reaches to the
//! end of the parameter range on one side (a pole or an apex): `v₀` is then
//! that end, where the missing boundary contributes nothing.

use crate::{Body, BodyKind, Coedge, Face};
use craft3d_geom::{Curve3, GeomError, Surface};
use craft3d_math::Vec3;
use craft3d_math::quadrature::Rule;
use std::f64::consts::FRAC_PI_4;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MassProperties {
    /// Total surface area.
    pub area: f64,
    /// Enclosed volume; 0 for a sheet. Negative means the faces point
    /// inward.
    pub volume: f64,
    /// Centre of volume; `None` for a sheet or a zero volume.
    pub centroid: Option<Vec3>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MassError {
    /// A face refers to a loop or edge that does not exist; run `check`.
    Invalid {
        face: usize,
    },
    /// A face is not bounded: no loops on an open surface, or a loop that
    /// winds around a surface with no end on that side.
    UnboundedFace {
        face: usize,
    },
    /// A configuration the integrator does not handle yet.
    Unsupported {
        face: usize,
        what: &'static str,
    },
    Geometry {
        face: usize,
        error: GeomError,
    },
}

impl fmt::Display for MassError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MassError::Invalid { face } => write!(
                f,
                "face {face} refers to a loop or edge that does not exist; check the body first"
            ),
            MassError::UnboundedFace { face } => {
                write!(f, "face {face} is not bounded by its loops")
            }
            MassError::Unsupported { face, what } => {
                write!(f, "face {face}: not supported yet: {what}")
            }
            MassError::Geometry { face, error } => write!(f, "face {face}: {error}"),
        }
    }
}

impl std::error::Error for MassError {}

/// The five integrands at once: area, volume, and the three moments.
#[derive(Clone, Copy, Debug, Default)]
struct Sums {
    area: f64,
    volume: f64,
    moment: Vec3,
}

impl Sums {
    fn add_scaled(&mut self, o: Sums, s: f64) {
        self.area += o.area * s;
        self.volume += o.volume * s;
        self.moment += o.moment * s;
    }
}

/// `f(u, v)` for the five integrals.
fn integrand(surface: &Surface, u: f64, v: f64) -> Sums {
    let (s, su, sv) = surface.derivatives(u, v);
    let n = su.cross(sv);
    Sums {
        area: n.length(),
        volume: s.dot(n) / 3.0,
        moment: Vec3::new(s.x * s.x * n.x, s.y * s.y * n.y, s.z * s.z * n.z) * 0.5,
    }
}

const NODES: usize = 16;
/// Panels for the inner integral along v.
const V_PANELS: usize = 2;

/// `G(u, v) = ∫ f(u, t) dt` from `v_ref` to `v`.
fn inner(surface: &Surface, rule: &Rule, u: f64, v_ref: f64, v: f64) -> Sums {
    let mut sum = Sums::default();
    let width = (v - v_ref) / V_PANELS as f64;
    for k in 0..V_PANELS {
        let start = v_ref + width * k as f64;
        for (t, w) in rule.points(start, start + width) {
            sum.add_scaled(integrand(surface, u, t), w);
        }
    }
    sum
}

/// How many quadrature panels an edge needs: enough that each spans at most
/// an eighth of a turn or one spline span.
fn panels(curve: &Curve3, range: (f64, f64)) -> usize {
    let length = (range.1 - range.0).abs();
    match curve {
        Curve3::Line(_) => 2,
        Curve3::Circle(_) | Curve3::Ellipse(_) => {
            ((length / FRAC_PI_4).ceil() as usize).clamp(1, 64)
        }
        Curve3::Nurbs(c) => (c.control_points().len() * 2).clamp(4, 4096),
    }
}

/// One boundary sample: surface parameters, and how fast they change along
/// the edge, times the quadrature weight and the traversal direction.
struct Sample {
    u: f64,
    v: f64,
    du: f64,
    dv: f64,
}

fn boundary_samples(
    body: &Body,
    face: &Face,
    index: usize,
    rule: &Rule,
) -> Result<Vec<Sample>, MassError> {
    let mut samples = Vec::new();
    for loop_id in &face.loops {
        let loop_ = body
            .loop_(*loop_id)
            .ok_or(MassError::Invalid { face: index })?;
        for &Coedge { edge, reversed } in &loop_.coedges {
            let edge = body.edge(edge).ok_or(MassError::Invalid { face: index })?;
            let (t0, t1) = edge.range;
            let direction = if reversed { -1.0 } else { 1.0 };
            let count = panels(&edge.curve, edge.range);
            let width = (t1 - t0) / count as f64;
            for k in 0..count {
                let start = t0 + width * k as f64;
                for (t, w) in rule.points(start, start + width) {
                    let (point, tangent) = edge.curve.derivatives(t);
                    let (u, v) = face
                        .surface
                        .project(point)
                        .map_err(|error| MassError::Geometry { face: index, error })?;
                    // Solve [Sᵤ Sᵥ] (du, dv) = tangent in the least-squares
                    // sense: the edge lies on the surface, so this is exact
                    // up to the modelling tolerance.
                    let (_, su, sv) = face.surface.derivatives(u, v);
                    let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
                    let det = a * c - b * b;
                    let (ru, rv) = (tangent.dot(su), tangent.dot(sv));
                    // At a pole Sᵤ vanishes and u is undefined; such a
                    // sample has no weight in ∮ G du because G is zero there
                    // only when v_ref is that pole, which is the case this
                    // arises in. Dropping it is the limit.
                    let (du, dv) = if det > 1e-24 * a.max(c) * a.max(c) && det.is_finite() {
                        ((c * ru - b * rv) / det, (a * rv - b * ru) / det)
                    } else if c > 0.0 {
                        (0.0, rv / c)
                    } else {
                        (0.0, 0.0)
                    };
                    samples.push(Sample {
                        u,
                        v,
                        du: du * w * direction,
                        dv: dv * w * direction,
                    });
                }
            }
        }
    }
    Ok(samples)
}

/// `∬ f du dv` over a whole closed surface.
fn whole_surface(surface: &Surface, index: usize, rule: &Rule) -> Result<Sums, MassError> {
    let ((u0, u1), (v0, v1)) = surface.domain();
    if ![u0, u1, v0, v1].iter().all(|x| x.is_finite()) {
        return Err(MassError::UnboundedFace { face: index });
    }
    const U_PANELS: usize = 8;
    let mut sum = Sums::default();
    let width = (u1 - u0) / U_PANELS as f64;
    for k in 0..U_PANELS {
        let start = u0 + width * k as f64;
        for (u, w) in rule.points(start, start + width) {
            sum.add_scaled(inner(surface, rule, u, v0, v1), w);
        }
    }
    Ok(sum)
}

/// `∬_D f du dv` over one face, with `D` oriented by the surface (not by
/// the face's `reversed` flag).
fn face_integrals(body: &Body, face: &Face, index: usize, rule: &Rule) -> Result<Sums, MassError> {
    if face.loops.is_empty() {
        return whole_surface(&face.surface, index, rule);
    }
    let samples = boundary_samples(body, face, index, rule)?;
    // Loops run counter-clockwise around the face seen from outside, which
    // is clockwise in parameter space when the face is reversed.
    let sense = if face.reversed { -1.0 } else { 1.0 };
    let ((_, _), (v_min, v_max)) = face.surface.domain();

    if let Some(period) = face.surface.period_v() {
        let turns = samples.iter().map(|s| s.dv).sum::<f64>() / period;
        if turns.abs() > 0.5 {
            return Err(MassError::Unsupported {
                face: index,
                what: "a loop that winds around the tube of a torus",
            });
        }
    }
    let v_ref = match face.surface.period_u() {
        None => samples.first().map_or(0.0, |s| s.v),
        Some(period) => {
            // Net number of times the boundary goes around the axis, in the
            // surface's counter-clockwise sense.
            let turns = (sense * samples.iter().map(|s| s.du).sum::<f64>() / period).round();
            let end = if turns == 0.0 {
                samples.first().map_or(0.0, |s| s.v)
            } else if turns == 1.0 {
                // The face is on the +v side of a loop running in +u.
                v_max
            } else if turns == -1.0 {
                v_min
            } else {
                return Err(MassError::Unsupported {
                    face: index,
                    what: "a boundary that winds around the axis more than once",
                });
            };
            if face.surface.period_v().is_some() && turns != 0.0 {
                return Err(MassError::UnboundedFace { face: index });
            }
            end
        }
    };
    if !v_ref.is_finite() {
        return Err(MassError::UnboundedFace { face: index });
    }

    let mut sum = Sums::default();
    for s in &samples {
        if s.du != 0.0 {
            sum.add_scaled(inner(&face.surface, rule, s.u, v_ref, s.v), -sense * s.du);
        }
    }
    Ok(sum)
}

/// Area, volume and centroid of `body`. The body should pass
/// [`crate::check`]; on an invalid one the result is meaningless or an
/// error, never a panic.
pub fn mass_properties(body: &Body) -> Result<MassProperties, MassError> {
    let rule = Rule::new(NODES);
    let mut total = Sums::default();
    for (index, face) in body.faces.iter().enumerate() {
        let sums = face_integrals(body, face, index, &rule)?;
        // Area counts whichever way the face points; volume and moments
        // follow the outward normal.
        let outward = if face.reversed { -1.0 } else { 1.0 };
        total.area += sums.area;
        total.volume += sums.volume * outward;
        total.moment += sums.moment * outward;
    }
    if ![total.area, total.volume].iter().all(|x| x.is_finite()) || !total.moment.is_finite() {
        return Err(MassError::Unsupported {
            face: 0,
            what: "geometry whose integrals are not finite",
        });
    }
    let solid = body.kind == BodyKind::Solid;
    Ok(MassProperties {
        area: total.area,
        volume: if solid { total.volume } else { 0.0 },
        centroid: (solid && total.volume != 0.0).then(|| total.moment * (1.0 / total.volume)),
    })
}
