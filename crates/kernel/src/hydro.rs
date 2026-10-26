//! Hydrostatics of closed bodies: what lies below a waterplane, and the
//! waterplane at which bodies float (docs/design/hydrostatics.md).
//!
//! Method. The water is a plane `{p : p · up = level}` and its unit
//! normal `up`. What lies below it is the boolean intersection of each
//! body with a box whose top face lies in that plane and which holds all
//! of the body below it ([`boolean`]): an exact B-rep, so the displaced
//! volume and the centre of buoyancy are its volume and centroid from the
//! exact integrator (`craft3d_topo::mass_properties`, held to 1e-12 of
//! what it sums), and the wetted surface is its area less the waterplane.
//! The waterplane is the faces of that solid that are pieces of the box's
//! top face: flat faces in the plane, bounded by the curves in which the
//! plane meets the body. Its area, first and second moments are Green's
//! theorem in the plane, `A = ∮ a db`, `∫a dA = ∮ a²/2 db`,
//! `∫b dA = −∮ b²/2 da`, `∫a² dA = ∮ a³/3 db`, `∫ab dA = ∮ a²b/2 db`,
//! `∫b² dA = −∮ b³/3 da`, along every coedge of those faces: per edge,
//! panels between the points where its curve is not smooth (the knots of a
//! spline), each summed with the 8- and the 16-node Gauss–Legendre rule
//! and halved until the two agree ([`PANEL_ALLOWANCE`]); no tessellation
//! is integrated anywhere.
//!
//! The waterline for a displacement is the level at which the volume
//! below it is that displacement. The volume grows with the level and
//! its derivative is the waterplane area, so the level is found by
//! Newton's method on that derivative, kept inside a bracket that every
//! step narrows and that falls back on halving where a Newton step would
//! leave it or did not halve the residual; at most [`MAX_STEPS`] steps,
//! after which "did not converge" is an error like any other.
//!
//! Sources: Biran & López-Pulido, *Ship Hydrostatics and Stability*,
//! 2nd ed., Butterworth-Heinemann 2014, ch. 2 (centre of buoyancy,
//! waterplane moments, metacentre BM = I/∇) and ch. 4 (heel, righting arm
//! GZ); Tupper, *Introduction to Naval Architecture*, 5th ed., 2013, ch. 4
//! (the wall-sided formula used as an oracle).

use crate::KernelError;
use crate::boolean::{BooleanOp, FaceSource, Operand, boolean};
use crate::primitives::block;
use craft3d_geom::{Curve3, Surface};
use craft3d_math::quadrature::Rule;
use craft3d_math::{Bbox3, Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, FaceId, MassError, TransformError, mass_properties};
use std::f64::consts::PI;
use std::fmt;

/// Most steps the waterline search takes before it reports that it did
/// not converge. Halving alone narrows a bracket by 2⁻⁶⁴, past the
/// resolution of an `f64`, so a search that reaches it is not short of
/// steps but stuck.
pub const MAX_STEPS: usize = 64;

/// The waterline is found once the volume below it is within this much of
/// the displacement, relative to the displacement. The volume itself is
/// integrated to 1e-12 of what it sums, so this is a hundred times above
/// what the integrals can tell apart; the level is then within the
/// residual over the waterplane area.
pub const SETTLED: f64 = 1e-10;

/// Two Gauss–Legendre sums of a waterplane panel agree when they differ by
/// at most this much of the body's size to the integral's power, in
/// proportion to the panel's share of that size (so that the total over
/// every panel is bounded by this times the perimeter over the size).
pub const PANEL_ALLOWANCE: f64 = 1e-13;

/// Most panels the waterplane of one body may take in all, and most times
/// one panel is halved: an integrand that does not settle within them is
/// an error, not a number.
const MAX_PANELS: usize = 1 << 16;
const MAX_HALVINGS: u32 = 40;

/// The widest first panel along a circle or an ellipse, in its angle:
/// there the 8-node rule is within about 1e-15 of the 16-node one, so most
/// panels settle at once. Only a starting point: every panel is checked.
const ARC_PANEL: f64 = PI / 16.0;

/// How the water lies: the unit normal of its plane (`up`, pointing out of
/// the water) and a unit direction in the plane (`along`, the length of a
/// ship), about which the second moment that gives the transverse
/// metacentre is taken and about which a heel turns the water.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Water {
    pub up: Vec3,
    pub along: Vec3,
}

impl Water {
    /// `up` and `along` made unit; refused when either is zero or not
    /// finite, or when `along` is not square to `up` (within 1e-9 of the
    /// cosine).
    pub fn new(up: Vec3, along: Vec3) -> Result<Water, HydroError> {
        let up = up
            .normalized()
            .filter(|v| v.is_finite())
            .ok_or(HydroError::InvalidWater("up is zero or not finite"))?;
        let along = along
            .normalized()
            .filter(|v| v.is_finite())
            .ok_or(HydroError::InvalidWater("along is zero or not finite"))?;
        if up.dot(along).abs() > 1e-9 {
            return Err(HydroError::InvalidWater(
                "along must be square to up (it lies in the waterplane)",
            ));
        }
        Ok(Water { up, along })
    }

    /// The third direction, `up × along`: across the length, in the
    /// waterplane.
    pub fn across(&self) -> Vec3 {
        self.up.cross(self.along)
    }

    /// The water turned by `angle` (radians) about `along`, right-handed:
    /// a body heeled by `-angle` in still water. A positive angle tips
    /// `up` towards `-across`, so the side of the body towards `across`
    /// goes down.
    pub fn heeled(&self, angle: f64) -> Result<Water, HydroError> {
        if !angle.is_finite() {
            return Err(HydroError::InvalidWater("the heel is not finite"));
        }
        let (s, c) = angle.sin_cos();
        // Rodrigues about `along`, which is square to `up`.
        let up = self.up * c + self.along.cross(self.up) * s;
        Water::new(up, self.along)
    }
}

/// The waterplane: the area of the section of the bodies by the water's
/// plane, its centroid, and its second moments about axes through that
/// centroid (`i_along` about the axis along `along`, the one the
/// transverse metacentre follows; `i_across` about the axis along
/// `across`; `product` = ∫ a b dA, a along and b across).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waterplane {
    pub area: f64,
    pub centroid: Option<Vec3>,
    pub i_along: f64,
    pub i_across: f64,
    pub product: f64,
}

/// What lies below the water at one level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Immersion {
    /// The plane is `{p : p · up = level}`.
    pub level: f64,
    /// The displaced volume (∇).
    pub volume: f64,
    /// The centre of buoyancy, the centroid of the displaced volume;
    /// `None` when nothing is below the water.
    pub buoyancy: Option<Vec3>,
    /// The area of the bodies' surface below the water.
    pub wetted: f64,
    pub waterplane: Waterplane,
}

/// The level at which the bodies displace a given volume, and what lies
/// below it there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Floating {
    pub immersion: Immersion,
    /// How many levels were measured to find it.
    pub steps: usize,
    /// `|∇(level) − displacement|`.
    pub residual: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HydroError {
    InvalidWater(&'static str),
    /// The level or the displacement asked for is not a finite number.
    NotFinite(&'static str),
    NoBodies,
    /// `body` (by its place in the list) is not a closed solid: a sheet
    /// holds no water out.
    NotSolid {
        body: usize,
    },
    /// `body` encloses no volume, or its faces point inward.
    NoVolume {
        body: usize,
        volume: f64,
    },
    Boxing {
        body: usize,
        error: TransformError,
    },
    /// The intersection with the half-space below the water failed.
    Kernel {
        body: usize,
        level: f64,
        error: KernelError,
    },
    Mass {
        body: usize,
        error: MassError,
    },
    /// The waterplane of `body` could not be measured.
    Waterplane {
        body: usize,
        what: &'static str,
    },
    /// No weight to float: a displacement of zero or less.
    NoDisplacement {
        needed: f64,
    },
    /// Wholly under water the bodies displace `most`, no more than the
    /// `needed` displacement: they sink (or, equal, hover).
    Sinks {
        most: f64,
        needed: f64,
    },
    NotConverged {
        steps: usize,
        low: f64,
        high: f64,
        residual: f64,
    },
}

impl fmt::Display for HydroError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HydroError::InvalidWater(why) => write!(f, "the water: {why}"),
            HydroError::NotFinite(what) => write!(f, "{what} is not a finite number"),
            HydroError::NoBodies => write!(f, "there are no bodies to float"),
            HydroError::NotSolid { body } => write!(
                f,
                "body {body} is not a closed solid (a sheet holds no water out); close it first"
            ),
            HydroError::NoVolume { body, volume } => write!(
                f,
                "body {body} encloses no volume ({volume:e}); its faces may point inward"
            ),
            HydroError::Boxing { body, error } => {
                write!(f, "body {body} cannot be boxed: {error}")
            }
            HydroError::Kernel { body, level, error } => write!(
                f,
                "the part of body {body} below the water at level {level} cannot be cut off: {error}"
            ),
            HydroError::Mass { body, error } => {
                write!(f, "body {body} cannot be measured: {error}")
            }
            HydroError::Waterplane { body, what } => {
                write!(f, "the waterplane of body {body}: {what}")
            }
            HydroError::NoDisplacement { needed } => write!(
                f,
                "nothing to float: the displacement asked for is {needed}, not more than zero (no weight)"
            ),
            HydroError::Sinks { most, needed } => write!(
                f,
                "the bodies sink: wholly under water they displace {most}, and the weight needs {needed}"
            ),
            HydroError::NotConverged {
                steps,
                low,
                high,
                residual,
            } => write!(
                f,
                "the waterline did not converge in {steps} steps: it lies between levels {low} and {high}, the volume still {residual} off"
            ),
        }
    }
}

impl std::error::Error for HydroError {}

/// The extent of `bodies` along `up`: a range that certainly holds every
/// point of them (from their boxes, `Body::bbox`), and their box.
pub fn extent(bodies: &[&Body], up: Vec3) -> Result<(f64, f64, Bbox3), HydroError> {
    let mut all = Bbox3::EMPTY;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for (index, body) in bodies.iter().enumerate() {
        let (l, h, b) = body_extent(index, body, up)?;
        lo = lo.min(l);
        hi = hi.max(h);
        if let Some((a, c)) = b.corners() {
            all.add(a);
            all.add(c);
        }
    }
    if bodies.is_empty() {
        return Err(HydroError::NoBodies);
    }
    Ok((lo, hi, all))
}

fn body_extent(index: usize, body: &Body, up: Vec3) -> Result<(f64, f64, Bbox3), HydroError> {
    let bbox = body
        .bbox()
        .map_err(|error| HydroError::Boxing { body: index, error })?;
    let Some((a, b)) = bbox.corners() else {
        return Err(HydroError::NoVolume {
            body: index,
            volume: 0.0,
        });
    };
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for k in 0..8_u32 {
        let corner = Vec3::new(
            if k & 1 == 0 { a.x } else { b.x },
            if k & 2 == 0 { a.y } else { b.y },
            if k & 4 == 0 { a.z } else { b.z },
        );
        let h = corner.dot(up);
        lo = lo.min(h);
        hi = hi.max(h);
    }
    if !(lo.is_finite() && hi.is_finite()) {
        return Err(HydroError::NotFinite("the bodies' box"));
    }
    Ok((lo, hi, bbox))
}

/// What one body has below the water: its volume, the first moment of the
/// volume about the origin, its wetted area, and the six plane integrals
/// of its waterplane about the common point `o` (area, ∫a, ∫b, ∫a², ∫ab,
/// ∫b²).
#[derive(Clone, Copy, Debug, Default)]
struct Below {
    volume: f64,
    moment: Vec3,
    wetted: f64,
    plane: [f64; 6],
}

/// What lies below the water at `level` (the plane `{p : p · water.up =
/// level}`) of each of `bodies`, summed. Bodies that overlap are counted
/// once each, as separate bodies are: give bodies that do not.
pub fn immersion(
    bodies: &[&Body],
    water: &Water,
    level: f64,
    tolerance: &Tolerance,
) -> Result<Immersion, HydroError> {
    if !level.is_finite() {
        return Err(HydroError::NotFinite("the level"));
    }
    let (_, _, all) = extent(bodies, water.up)?;
    immersion_in(bodies, water, level, tolerance, &all)
}

fn immersion_in(
    bodies: &[&Body],
    water: &Water,
    level: f64,
    tolerance: &Tolerance,
    all: &Bbox3,
) -> Result<Immersion, HydroError> {
    let center = all.center().unwrap_or(Vec3::ZERO);
    let size = (0.5 * all.diagonal()).max(f64::MIN_POSITIVE);
    // The point the waterplane's moments are taken about: in the plane,
    // over the middle of the bodies, so a small waterplane far from the
    // origin loses nothing to cancellation.
    let o = center + water.up * (level - center.dot(water.up));
    let mut total = Below::default();
    for (index, body) in bodies.iter().enumerate() {
        let below = below_one(index, body, water, level, tolerance, (o, size))?;
        total.volume += below.volume;
        total.moment += below.moment;
        total.wetted += below.wetted;
        for (sum, part) in total.plane.iter_mut().zip(below.plane) {
            *sum += part;
        }
    }
    let buoyancy = (total.volume > 0.0).then(|| total.moment * (1.0 / total.volume));
    let [area, sa, sb, iaa, iab, ibb] = total.plane;
    let waterplane = if area > 0.0 {
        let (ca, cb) = (sa / area, sb / area);
        Waterplane {
            area,
            centroid: Some(o + water.along * ca + water.across() * cb),
            i_along: ibb - area * cb * cb,
            i_across: iaa - area * ca * ca,
            product: iab - area * ca * cb,
        }
    } else {
        Waterplane {
            area: 0.0,
            centroid: None,
            i_along: 0.0,
            i_across: 0.0,
            product: 0.0,
        }
    };
    Ok(Immersion {
        level,
        volume: total.volume,
        buoyancy,
        wetted: total.wetted,
        waterplane,
    })
}

/// The whole of a solid body, measured.
fn whole(index: usize, body: &Body) -> Result<(f64, Vec3, f64), HydroError> {
    if body.kind != BodyKind::Solid {
        return Err(HydroError::NotSolid { body: index });
    }
    let props = mass_properties(body).map_err(|error| HydroError::Mass { body: index, error })?;
    let centroid = props.centroid.filter(|_| props.volume > 0.0);
    let Some(centroid) = centroid else {
        return Err(HydroError::NoVolume {
            body: index,
            volume: props.volume,
        });
    };
    Ok((props.volume, centroid * props.volume, props.area))
}

fn below_one(
    index: usize,
    body: &Body,
    water: &Water,
    level: f64,
    tolerance: &Tolerance,
    (o, size): (Vec3, f64),
) -> Result<Below, HydroError> {
    if body.kind != BodyKind::Solid {
        return Err(HydroError::NotSolid { body: index });
    }
    let (lo, hi, bbox) = body_extent(index, body, water.up)?;
    if level <= lo {
        return Ok(Below::default());
    }
    if level >= hi {
        let (volume, moment, wetted) = whole(index, body)?;
        return Ok(Below {
            volume,
            moment,
            wetted,
            plane: [0.0; 6],
        });
    }
    // A box with its top face in the plane that holds all of the body
    // below it: wider than the body's box on every side, and deeper.
    let center = bbox.center().unwrap_or(Vec3::ZERO);
    let r = 0.5 * bbox.diagonal();
    let margin = 0.25 * r + 10.0 * tolerance.linear;
    let half = r + margin;
    let depth = level - (center.dot(water.up) - r) + margin;
    let q = center + water.up * (level - center.dot(water.up));
    let corner = q - water.along * half - water.across() * half - water.up * depth;
    let frame = Frame::new(corner, water.up, water.along).ok_or(HydroError::InvalidWater(
        "the directions do not make a frame",
    ))?;
    let tool =
        block(&frame, 2.0 * half, 2.0 * half, depth).map_err(|error| HydroError::Kernel {
            body: index,
            level,
            error,
        })?;
    let cut = match boolean(body, &tool, BooleanOp::Intersect, tolerance) {
        Ok(cut) => cut,
        Err(KernelError::EmptyResult { .. }) => return Ok(Below::default()),
        Err(error) => {
            return Err(HydroError::Kernel {
                body: index,
                level,
                error,
            });
        }
    };
    // The block's faces are -z, +z, -y, +y, -x, +x of its frame: the top
    // one, in the water's plane, is the second.
    let top = FaceSource {
        operand: Operand::B,
        face: FaceId(1),
    };
    let mut caps = Vec::new();
    for (face, sources) in cut.merged.iter().enumerate() {
        if sources.contains(&top) {
            caps.push(face);
        } else if sources.iter().any(|s| s.operand == Operand::B) {
            return Err(HydroError::Waterplane {
                body: index,
                what: "the box cut off more than what lies below the water (a side of it is in the result)",
            });
        }
    }
    let props =
        mass_properties(&cut.body).map_err(|error| HydroError::Mass { body: index, error })?;
    let Some(centroid) = props.centroid.filter(|_| props.volume > 0.0) else {
        return Ok(Below::default());
    };
    let mut plane = [0.0; 6];
    let mut budget = MAX_PANELS;
    let rule = (Rule::new(8), Rule::new(16));
    for face in caps {
        let sums = cap_sums(&cut.body, face, water, (o, size), &rule, &mut budget)
            .map_err(|what| HydroError::Waterplane { body: index, what })?;
        for (sum, part) in plane.iter_mut().zip(sums) {
            *sum += part;
        }
    }
    let [cap_area, ..] = plane;
    Ok(Below {
        volume: props.volume,
        moment: centroid * props.volume,
        wetted: (props.area - cap_area).max(0.0),
        plane,
    })
}

/// The six plane integrals (area, ∫a, ∫b, ∫a², ∫ab, ∫b², with a along
/// `water.along` and b along `water.across()` from `o`) of a flat face of
/// `body` in the water's plane whose outward normal is `water.up`.
fn cap_sums(
    body: &Body,
    face: usize,
    water: &Water,
    (o, size): (Vec3, f64),
    rule: &(Rule, Rule),
    budget: &mut usize,
) -> Result<[f64; 6], &'static str> {
    let face = body
        .faces
        .get(face)
        .ok_or("a face that is not in the body")?;
    let Surface::Plane(plane) = &face.surface else {
        return Err("a face in the water's plane that is not flat");
    };
    // The loops run counter-clockwise about the surface's normal; the
    // face's outward normal is that normal, or the other way round when
    // the face is reversed. The cap of what is below the water faces up.
    let normal = plane.frame().z();
    let facing = normal.dot(water.up);
    let outward = if face.reversed { -facing } else { facing };
    if outward.is_nan() || outward <= 0.5 {
        return Err("a face in the water's plane that does not face up out of the water");
    }
    let orientation = if facing > 0.0 { 1.0 } else { -1.0 };
    let mut sums = [0.0; 6];
    for &loop_id in &face.loops {
        let lp = body
            .loop_(loop_id)
            .ok_or("a loop that is not in the body")?;
        for coedge in &lp.coedges {
            let edge = body
                .edge(coedge.edge)
                .ok_or("an edge that is not in the body")?;
            let along = edge_sums(&edge.curve, edge.range, water, (o, size), rule, budget)?;
            let sign = if coedge.reversed { -1.0 } else { 1.0 };
            for (sum, part) in sums.iter_mut().zip(along) {
                *sum += sign * orientation * part;
            }
        }
    }
    let [area, ..] = sums;
    if area.is_nan() || area <= 0.0 {
        return Err("a face in the water's plane whose loops enclose no area");
    }
    Ok(sums)
}

/// The powers of the length in each plane integral.
const POWERS: [i32; 6] = [2, 3, 3, 4, 4, 4];

/// The six line integrals of [`cap_sums`] along `curve` from `range.0` to
/// `range.1`.
fn edge_sums(
    curve: &Curve3,
    (t0, t1): (f64, f64),
    water: &Water,
    (o, size): (Vec3, f64),
    rule: &(Rule, Rule),
    budget: &mut usize,
) -> Result<[f64; 6], &'static str> {
    if !(t0.is_finite() && t1.is_finite() && t0 <= t1) {
        return Err("an edge whose parameter range is not finite");
    }
    // Where the curve is smooth in between: the knots of a spline, and
    // short stretches of the angle of a circle or an ellipse (only where
    // the panels start; every panel is checked).
    let mut breaks = vec![t0];
    match curve {
        Curve3::Line(_) => {}
        Curve3::Circle(_) | Curve3::Ellipse(_) => {
            let n = ((t1 - t0) / ARC_PANEL).ceil().clamp(1.0, 1024.0) as usize;
            for k in 1..n {
                breaks.push(t0 + (t1 - t0) * (k as f64 / n as f64));
            }
        }
        Curve3::Nurbs(spline) => {
            for &knot in spline.knots() {
                if knot > t0 && knot < t1 && breaks.last().is_some_and(|&last| knot > last) {
                    breaks.push(knot);
                }
            }
        }
    }
    breaks.push(t1);
    if breaks.len() > MAX_PANELS {
        return Err("an edge with more knot spans than a waterplane takes");
    }
    let across = water.across();
    let integrand = |t: f64| -> ([f64; 6], f64) {
        let (p, d) = curve.derivatives(t);
        let r = p - o;
        let (a, b) = (r.dot(water.along), r.dot(across));
        let (da, db) = (d.dot(water.along), d.dot(across));
        (
            [
                a * db,
                0.5 * a * a * db,
                -0.5 * b * b * da,
                a * a * a / 3.0 * db,
                0.5 * a * a * b * db,
                -b * b * b / 3.0 * da,
            ],
            d.length(),
        )
    };
    let sum_on = |rule: &Rule, a: f64, b: f64| -> ([f64; 6], [f64; 6], f64) {
        let mut sums = [0.0; 6];
        let mut magnitude = [0.0; 6];
        let mut length = 0.0;
        for (t, w) in rule.points(a, b) {
            let (f, speed) = integrand(t);
            for k in 0..6 {
                let (Some(s), Some(m), Some(v)) = (sums.get_mut(k), magnitude.get_mut(k), f.get(k))
                else {
                    continue;
                };
                *s += w * v;
                *m += (w * v).abs();
            }
            length += w * speed;
        }
        (sums, magnitude, length)
    };
    let mut total = [0.0; 6];
    let mut open: Vec<(f64, f64, u32)> = breaks
        .windows(2)
        .filter_map(|w| match w {
            [a, b] if b > a => Some((*a, *b, 0)),
            _ => None,
        })
        .rev()
        .collect();
    while let Some((a, b, halvings)) = open.pop() {
        *budget = budget
            .checked_sub(1)
            .ok_or("its edges take more panels than a waterplane may")?;
        let (coarse, _, _) = sum_on(&rule.0, a, b);
        let (fine, magnitude, length) = sum_on(&rule.1, a, b);
        let share = (length / size).max(1e-9);
        let settled = (0..6).all(|k| {
            let (Some(c), Some(f), Some(m), Some(p)) =
                (coarse.get(k), fine.get(k), magnitude.get(k), POWERS.get(k))
            else {
                return false;
            };
            let allowed = PANEL_ALLOWANCE * size.powi(*p) * share + 64.0 * f64::EPSILON * m;
            (f - c).abs() <= allowed
        });
        if settled {
            for (sum, part) in total.iter_mut().zip(fine) {
                *sum += part;
            }
            continue;
        }
        if halvings >= MAX_HALVINGS {
            return Err("an edge along which the waterplane's integrals do not settle");
        }
        let mid = 0.5 * (a + b);
        if !(mid > a && mid < b) {
            return Err("an edge along which the waterplane's integrals do not settle");
        }
        open.push((mid, b, halvings + 1));
        open.push((a, mid, halvings + 1));
    }
    if !total.iter().all(|x| x.is_finite()) {
        return Err("an edge whose integrals are not finite");
    }
    Ok(total)
}

/// The level at which `bodies` displace `volume`, found as the module
/// documentation says, and what lies below the water there.
pub fn float(
    bodies: &[&Body],
    water: &Water,
    volume: f64,
    tolerance: &Tolerance,
) -> Result<Floating, HydroError> {
    if !volume.is_finite() {
        return Err(HydroError::NotFinite("the displacement"));
    }
    if volume <= 0.0 {
        return Err(HydroError::NoDisplacement { needed: volume });
    }
    let (mut lo, mut hi, all) = extent(bodies, water.up)?;
    let mut most = 0.0;
    for (index, body) in bodies.iter().enumerate() {
        most += whole(index, body)?.0;
    }
    if volume >= most {
        return Err(HydroError::Sinks {
            most,
            needed: volume,
        });
    }
    let mut level = lo + (hi - lo) * (volume / most);
    let mut last_residual = f64::INFINITY;
    for step in 1..=MAX_STEPS {
        let found = immersion_in(bodies, water, level, tolerance, &all)?;
        let residual = found.volume - volume;
        if residual.abs() <= SETTLED * volume {
            return Ok(Floating {
                immersion: found,
                steps: step,
                residual: residual.abs(),
            });
        }
        if residual < 0.0 {
            lo = level;
        } else {
            hi = level;
        }
        let area = found.waterplane.area;
        let newton = level - residual / area;
        let halving = 0.5 * (lo + hi);
        let next = if area > 0.0
            && newton.is_finite()
            && newton > lo
            && newton < hi
            && residual.abs() <= 0.5 * last_residual
        {
            newton
        } else {
            halving
        };
        last_residual = residual.abs();
        if !(next > lo && next < hi) {
            // The bracket is as narrow as an f64 makes it.
            return Ok(Floating {
                immersion: found,
                steps: step,
                residual: residual.abs(),
            });
        }
        level = next;
    }
    Err(HydroError::NotConverged {
        steps: MAX_STEPS,
        low: lo,
        high: hi,
        residual: last_residual,
    })
}

#[cfg(test)]
mod tests;
