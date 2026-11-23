//! What lies on one side of a plane: the volume and centroid of the part
//! of closed bodies below a plane, the area, centroid and second moments
//! of their section by it, and the offset of a plane at which the volume
//! below it is a given volume (docs/design/sections-and-levels.md).
//!
//! Method. The plane is `{p : p · normal = offset}`; "below" is the side
//! the normal points away from. What lies below it is the boolean
//! intersection of each body with a box whose top face lies in that plane
//! and which holds all of the body below it ([`boolean`]): an exact B-rep,
//! so the volume and the centroid below are its volume and centroid from
//! the exact integrator (`craft3d_topo::mass_properties`, held to 1e-12 of
//! what it sums), and the surface below is its area less the section. The
//! section is the faces of that solid that are pieces of the box's top
//! face: flat faces in the plane, bounded by the curves in which the plane
//! meets the bodies. Its area, first and second moments are Green's
//! theorem in the plane, `A = ∮ a db`, `∫a dA = ∮ a²/2 db`,
//! `∫b dA = −∮ b²/2 da`, `∫a² dA = ∮ a³/3 db`, `∫ab dA = ∮ a²b/2 db`,
//! `∫b² dA = −∮ b³/3 da`, along every coedge of those faces: per edge,
//! panels between the points where its curve is not smooth (the knots of a
//! spline), each summed with the 8- and the 16-node Gauss–Legendre rule
//! and halved until the two agree ([`PANEL_ALLOWANCE`]); no tessellation
//! is integrated anywhere. A section whose panels cannot agree that
//! closely because its points are rounded more than that (one small
//! against its distance from the origin) is summed again with the
//! rounding of its coordinates allowed for ([`ROUNDING_ULPS`]), unless
//! an edge of it lies on a rational spline whose weights spread more than
//! twofold (`held_sums`).
//!
//! The offset for a volume is where the volume below the plane is that
//! volume. The volume grows with the offset and its derivative is the
//! section's area, so the offset is found by Newton's method on that
//! derivative, kept inside a bracket that every step narrows and that
//! falls back on halving where a Newton step would leave it or did not
//! halve the residual; at most [`MAX_STEPS`] steps, after which "did not
//! converge" is an error like any other.

use crate::KernelError;
use crate::boolean::probe::{OnFace, Probe};
use crate::boolean::{BooleanOp, FaceSource, Operand, boolean};
use crate::primitives::block;
use craft3d_geom::support::{CurveNets, Piece, SurfaceNets, curve_piece, surface_piece};
use craft3d_geom::{Curve3, RationalBezierPatch, Surface};
use craft3d_math::Portable;
use craft3d_math::quadrature::Rule;
use craft3d_math::{Bbox3, Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, BodyKind, FaceId, MassError, TransformError, mass_properties};
use std::cell::{Cell, OnceCell};
use std::collections::BinaryHeap;
use std::f64::consts::PI;
use std::fmt;

mod float;
pub mod torsion;
pub use float::{
    ARM_SETTLED, ARM_UNSETTLED, Equilibrium, Floating, LEAVE, MAX_ITERATIONS, MAX_LEVELS, MAX_TURN,
    Principal, Stability, floating,
};

/// Most steps the search for an offset takes before it reports that it
/// did not converge. Halving alone narrows a bracket by 2⁻⁶⁴, past the
/// resolution of an `f64`, so a search that reaches it is not short of
/// steps but stuck.
pub const MAX_STEPS: usize = 64;

/// The offset is found once the volume below it is within this much of
/// the volume asked for, relative to it. The volume itself is integrated
/// to 1e-12 of what it sums, so this is a hundred times above what the
/// integrals can tell apart; the offset is then within the residual over
/// the section's area.
pub const SETTLED: f64 = 1e-10;

/// Where the bracket can no longer be narrowed, the offset found is
/// still the answer when the volume is within this much (relative) of
/// the one asked for: a thousand times [`SETTLED`], the rounding of an
/// offset next to a steep section. Further off it is "did not converge".
pub const UNSETTLED: f64 = 1e-7;

/// Where in the bracket (as shares of it, from its low end) the level
/// search tries next after an offset the boolean refuses, in turn while
/// they are refused: its middle, then its quarters and an eighth. An
/// offset is refused where the plane meets a body in a configuration the
/// boolean cannot cut (a face in the plane, a tangency), which happens at
/// isolated offsets; points spread over the bracket step around them
/// (each further than 1e-9 of the bracket from those refused: nearer is
/// the same offset up to rounding), and refused at all four after one
/// refusal the last refusal is the answer.
const AROUND: [f64; 4] = [0.5, 0.25, 0.75, 0.375];

/// Two Gauss–Legendre sums of a section's panel agree when they differ by
/// at most this much of the bodies' size to the integral's power, in
/// proportion to the panel's share of that size (so that the total over
/// every panel is bounded by this times the perimeter over the size).
pub const PANEL_ALLOWANCE: f64 = 1e-13;

/// Most panels the section of one body may take in all, and most times
/// one panel is halved: an integrand that does not settle within them is
/// an error, not a number.
const MAX_PANELS: usize = 1 << 16;
const MAX_HALVINGS: u32 = 40;

/// How many units in the last place a point of an edge, as it is
/// computed, is taken to be off by, of the numbers it is made from: the
/// coordinates of its circle's or ellipse's centre and its radius, of its
/// line's origin, of its spline's control points (by the spread of its
/// weights), and its parameter times its speed. Each of those is a few
/// roundings, so this is a bound with room. The rounding a panel's two
/// sums take on from their points is far below [`PANEL_ALLOWANCE`] of the
/// section's size for a section near the origin, and more than that for
/// one about a thousand times smaller than its distance from the origin
/// and beyond (halving a panel halves both): there the sums never agree
/// to that alone, and the section is summed again with this rounding
/// allowed for (`Held::Rounding`).
pub const ROUNDING_ULPS: f64 = 16.0;

/// The most a rational spline's heaviest weight may be of its lightest
/// for a section with an edge on it to be summed with the rounding of its
/// coordinates allowed for ([`held_sums`]). A panel's two sums tell its
/// error only where the 16-node one is far nearer the integral than the
/// 8-node one. A rational curve's integrands have the poles of its
/// denominator, the sum of its weights, and where the weights spread
/// widely those lie near the span (a conic of weights 1, w, 1 within
/// about `1/(2w)` of its parameter beyond its ends, where it turns): both
/// rules then miss alike, their sums agree on one with most of the
/// integral missing, and the rounding's allowance, wider than the strict
/// one, takes it (docs/design/sections-and-levels.md, 2026-11-23). Within
/// a spread of 2 the poles of such a conic lie 0.37 of its span or more
/// beyond it, so on every panel the 16-node sum's error is some 1e-8 of
/// the 8-node one's, whatever the allowance: the sum taken is the
/// integral to rounding at every distance from the origin (held to the
/// closed form out to 1e10 times the size, as it is and raised to
/// degrees 3 and 6, which keeps its poles). A rational span of degree n
/// of that spread has them only outside |t| + |1 - t| = 3^(1/n) (rho 1.30
/// at degree 32), where the pair does not bound the error: there the rule
/// rests on measurement (`far_tests.rs`). A circle as a spline spreads
/// 1.41, a hyperbola's arc the boolean builds at most 1.13.
const ROUNDING_SPREAD: f64 = 2.0;

/// The words of the two ways a section's panels fail to settle, by which
/// [`held_sums`] tells them from every other refusal of the section.
const TOO_MANY_PANELS: &str = "its edges take more panels than a section may";
const NOT_SETTLED: &str = "an edge along which the section's integrals do not settle";

/// A section's panels that do not settle even with the rounding of their
/// coordinates allowed for.
const NOT_SETTLED_ROUNDED: &str = "its edges' integrals do not settle even when held only to the rounding of their coordinates (where the section is small against its distance from the origin, moving the bodies nearer it may let them settle)";

/// What the second pass ([`Held::Rounding`]) says of an edge on a
/// rational spline whose weights spread more than [`ROUNDING_SPREAD`]:
/// never the answer, [`held_sums`] gives the strict pass's answer or its
/// refusal in its own words for it.
const WIDE_SPREAD: &str = "an edge on a rational spline whose weights spread too widely to be held only to the rounding of its coordinates";

/// The widest first panel along a circle or an ellipse, in its angle:
/// there the 8-node rule is within about 1e-15 of the 16-node one, so most
/// panels settle at once. Only a starting point: every panel is checked.
const ARC_PANEL: f64 = PI / 16.0;

/// The direction of a cutting plane: its unit normal (pointing away from
/// what is below it) and a unit direction in it, the section's x axis,
/// about which (and about `y_axis`) the section's moments are given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutPlane {
    pub normal: Vec3,
    pub x_axis: Vec3,
}

impl CutPlane {
    /// `normal` and `x_axis` made unit; refused when either is zero or not
    /// finite, or when `x_axis` is not square to `normal` (within 1e-9 of
    /// the cosine).
    pub fn new(normal: Vec3, x_axis: Vec3) -> Result<CutPlane, CutError> {
        let normal = normal
            .normalized()
            .filter(|v| v.is_finite())
            .ok_or(CutError::InvalidPlane("the normal is zero or not finite"))?;
        let x_axis = x_axis
            .normalized()
            .filter(|v| v.is_finite())
            .ok_or(CutError::InvalidPlane("the x axis is zero or not finite"))?;
        if normal.dot(x_axis).abs() > 1e-9 {
            return Err(CutError::InvalidPlane(
                "the x axis must be square to the normal (it lies in the plane)",
            ));
        }
        Ok(CutPlane { normal, x_axis })
    }

    /// The section's y axis, `normal × x_axis`.
    pub fn y_axis(&self) -> Vec3 {
        self.normal.cross(self.x_axis)
    }
}

/// The section of the bodies by the plane: its area, centroid, and second
/// moments about axes through that centroid (`i_x = ∫ y² dA` about the
/// axis along `x_axis`, `i_y = ∫ x² dA` about the one along `y_axis`,
/// `product = ∫ x y dA`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Section {
    pub area: f64,
    pub centroid: Option<Vec3>,
    pub i_x: f64,
    pub i_y: f64,
    pub product: f64,
}

impl Section {
    /// The principal second moments (largest first) and the unit
    /// direction, in the plane, of the axis about which the moment is the
    /// largest. `None` for a section of no area. Where the two are equal
    /// (a circle) every axis is principal and the x axis is given.
    pub fn principal(&self, plane: &CutPlane) -> Option<(f64, f64, Vec3)> {
        if self.area <= 0.0 {
            return None;
        }
        let mean = 0.5 * (self.i_x + self.i_y);
        let half = 0.5 * (self.i_x - self.i_y);
        let radius = half.phypot(self.product);
        let (major, minor) = (mean + radius, mean - radius);
        // The moment about an axis at angle θ from x is
        // i_x cos²θ + i_y sin²θ − 2 product sin θ cos θ; it is largest at
        // 2θ = atan2(−product, half).
        let theta = 0.5 * (-self.product).patan2(half);
        let (s, c) = theta.psin_cos();
        let direction = plane.x_axis * c + plane.y_axis() * s;
        Some((major, minor, direction))
    }
}

/// What lies below the plane at one offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BelowPlane {
    /// The plane is `{p : p · normal = offset}`.
    pub offset: f64,
    /// The volume of the bodies below the plane.
    pub volume: f64,
    /// The centroid of that volume; `None` when nothing is below.
    pub centroid: Option<Vec3>,
    /// The area of the bodies' surface below the plane (the section not
    /// counted).
    pub surface: f64,
    pub section: Section,
}

/// The offset at which the volume below the plane is a given volume, and
/// what lies below it there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    pub below: BelowPlane,
    /// How many offsets were measured to find it.
    pub steps: usize,
    /// `|volume(offset) − volume asked for|`.
    pub residual: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CutError {
    InvalidPlane(&'static str),
    /// The offset or the volume asked for is not a finite number.
    NotFinite(&'static str),
    NoBodies,
    /// `body` (by its place in the list) is not a closed solid: a sheet
    /// has no inside.
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
    /// The intersection with the half-space below the plane failed.
    Kernel {
        body: usize,
        offset: f64,
        error: KernelError,
    },
    Mass {
        body: usize,
        error: MassError,
    },
    /// The section of `body` could not be measured.
    Section {
        body: usize,
        what: &'static str,
    },
    /// The boolean found nothing below the plane, but `body` may reach
    /// `depth` below it (more than the tolerance), by a bound that holds
    /// every point of it: an answer of nothing could be wrong, so it is an
    /// error.
    LostBelow {
        body: usize,
        offset: f64,
        depth: f64,
    },
    /// The volume asked for is not strictly between 0 and the bodies'
    /// whole volume `total`, so no plane leaves it below.
    OutOfRange {
        volume: f64,
        total: f64,
    },
    NotConverged {
        steps: usize,
        low: f64,
        high: f64,
        residual: f64,
    },
    /// The search for a floating equilibrium ([`floating`]) did not
    /// settle: after `steps`, the centre of gravity still `arm` off the
    /// normal through the centroid below, at the plane of `normal`.
    NoEquilibrium {
        steps: usize,
        arm: f64,
        normal: Vec3,
    },
    /// The torsion of the section could not be solved for ([`torsion`]).
    Torsion(String),
}

impl fmt::Display for CutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CutError::InvalidPlane(why) => write!(f, "the plane: {why}"),
            CutError::NotFinite(what) => write!(f, "{what} is not a finite number"),
            CutError::NoBodies => write!(f, "there are no bodies to cut"),
            CutError::NotSolid { body } => write!(
                f,
                "body {body} is not a closed solid (a sheet has no inside); close it first"
            ),
            CutError::NoVolume { body, volume } => write!(
                f,
                "body {body} encloses no volume ({volume:e}); its faces may point inward"
            ),
            CutError::Boxing { body, error } => {
                write!(f, "body {body} cannot be boxed: {error}")
            }
            CutError::Kernel {
                body,
                offset,
                error,
            } => write!(
                f,
                "the part of body {body} below the plane at offset {offset} cannot be cut off: {error}"
            ),
            CutError::Mass { body, error } => {
                write!(f, "body {body} cannot be measured: {error}")
            }
            CutError::Section { body, what } => {
                write!(f, "the section of body {body}: {what}")
            }
            CutError::LostBelow {
                body,
                offset,
                depth,
            } => write!(
                f,
                "nothing of body {body} was found below the plane at offset {offset}, yet it may reach {depth} below it: not measured"
            ),
            CutError::OutOfRange { volume, total } => write!(
                f,
                "a volume of {volume} is outside (0, {total}), the bodies' whole volume: no plane leaves it below"
            ),
            CutError::NotConverged {
                steps,
                low,
                high,
                residual,
            } => write!(
                f,
                "the offset did not converge in {steps} steps: it lies between {low} and {high}, the volume still {residual} off"
            ),
            CutError::NoEquilibrium { steps, arm, normal } => write!(
                f,
                "the floating equilibrium did not converge in {steps} steps: at the normal [{}, {}, {}] the centre of gravity is still {arm} off the normal through the centroid below",
                normal.x, normal.y, normal.z
            ),
            CutError::Torsion(why) => write!(f, "the torsion of the section: {why}"),
        }
    }
}

impl std::error::Error for CutError {}

/// The extent of `bodies` along `direction` (unit): a range that
/// certainly holds every point of them, and their box.
pub fn extent(bodies: &[&Body], direction: Vec3) -> Result<(f64, f64, Bbox3), CutError> {
    let prepared = prepare(bodies, direction)?;
    let mut all = Bbox3::EMPTY;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for one in &prepared {
        lo = lo.min(one.lo);
        hi = hi.max(one.hi);
        if let Some((a, c)) = one.bbox.corners() {
            all.add(a);
            all.add(c);
        }
    }
    Ok((lo, hi, all))
}

/// A body with what is measured of it once for every plane of one
/// direction: its extent along the direction, its box, and (when asked
/// for) its whole volume, first moment and area.
struct Prepared<'b> {
    index: usize,
    body: &'b Body,
    lo: f64,
    hi: f64,
    bbox: Bbox3,
    whole: OnceCell<Result<(f64, Vec3, f64), CutError>>,
    /// The body made ready to tell whether a point of a face's surface is
    /// in the face (made when first needed: `None` where it cannot be).
    probe: OnceCell<Option<Probe<'b>>>,
    /// Whether the panels of this body's section did not settle to
    /// [`PANEL_ALLOWANCE`] at some offset of this call. That section
    /// refused the call before 2026-11-22 (a level search ends at such a
    /// refusal), so nothing this call gives was given before, and its
    /// later sections are held to the rounding of their coordinates at
    /// once ([`held_sums`]) instead of spending every panel first.
    rounded: Cell<bool>,
    /// Whether a section of this body that does not settle strictly may
    /// be summed again with the rounding of its coordinates allowed for
    /// (true but in a floating search after a first search that settled
    /// strictly: [`level_held`]); where not, the strict refusal stands.
    may_round: bool,
}

impl<'b> Prepared<'b> {
    fn whole(&self) -> Result<(f64, Vec3, f64), CutError> {
        self.whole
            .get_or_init(|| whole(self.index, self.body))
            .clone()
    }

    fn probe(&self, tolerance: &Tolerance) -> Option<&Probe<'b>> {
        self.probe
            .get_or_init(|| Probe::new(self.body, tolerance).ok())
            .as_ref()
    }
}

fn prepare<'b>(bodies: &[&'b Body], direction: Vec3) -> Result<Vec<Prepared<'b>>, CutError> {
    if bodies.is_empty() {
        return Err(CutError::NoBodies);
    }
    bodies
        .iter()
        .enumerate()
        .map(|(index, body)| {
            if body.kind != BodyKind::Solid {
                return Err(CutError::NotSolid { body: index });
            }
            let (lo, hi, bbox) = body_extent(index, body, direction)?;
            Ok(Prepared {
                index,
                body,
                lo,
                hi,
                bbox,
                whole: OnceCell::new(),
                probe: OnceCell::new(),
                rounded: Cell::new(false),
                may_round: true,
            })
        })
        .collect()
}

/// The extent of a body along `direction`: the z range of the held box
/// (`Body::held_bbox`, tight for planes, cylinders, cones, spheres and
/// tori with lines, circles and ellipses) of the body turned so that
/// `direction` is z, and the body's own box.
fn body_extent(index: usize, body: &Body, direction: Vec3) -> Result<(f64, f64, Bbox3), CutError> {
    let boxing = |error| CutError::Boxing { body: index, error };
    let bbox = body.bbox().map_err(boxing)?;
    if bbox.corners().is_none() {
        return Err(CutError::NoVolume {
            body: index,
            volume: 0.0,
        });
    }
    let turned = body.transformed(&onto_z(direction)?).map_err(boxing)?;
    let held = turned.held_bbox().map_err(boxing)?;
    let Some((a, b)) = held.corners() else {
        return Err(CutError::NoVolume {
            body: index,
            volume: 0.0,
        });
    };
    let (lo, hi) = (a.z, b.z);
    if !(lo.is_finite() && hi.is_finite()) {
        return Err(CutError::NotFinite("the bodies' box"));
    }
    Ok((lo, hi, bbox))
}

/// A rotation that turns the unit `direction` onto z: about their common
/// normal `k = direction × z / |direction × z|`, by the angle between
/// them read from its sine and cosine together (`Vec3::angle_to`). The
/// turned z of a point is then its height along `direction` to rounding:
/// the rotation's last row is `(−k.y s, k.x s, c)` (`k.z` is 0), the
/// sine and cosine of `atan2(|direction × z|, direction · z)`, which are
/// that cross product's length and that dot to a few units in the last
/// place, near 0, near π (to the last place of π there) and between, so
/// the row is `direction` to rounding. The `acos` of the dot alone was
/// off by all of the angle `θ` wherever its cosine rounds to ±1 (any
/// direction within 1.05e-8 rad of z or of −z) and by up to about
/// `5.5e-17 / θ` beyond, so a body's extent and floor were read along an
/// axis tilted that much from `direction`: 5e-6 off on a box 1000 long
/// at 1e-8 rad (2026-11-14). Along the axes the angle is the very
/// `acos` it was (`π/2`, exact z and −z by the branches below), so the
/// turn is the same to the bit. Along z: no turn; against it, where the
/// normal is zero: half a turn about x.
fn onto_z(direction: Vec3) -> Result<Transform, CutError> {
    let z = Vec3::new(0.0, 0.0, 1.0);
    let axis = direction.cross(z);
    match axis.normalized() {
        // Turning `direction` onto z about their common normal.
        Some(axis) if axis.is_finite() => Transform::rotation(axis, direction.angle_to(z)),
        // Along z already, or against it.
        _ if direction.dot(z) > 0.0 => Some(Transform::IDENTITY),
        _ => Transform::rotation(Vec3::new(1.0, 0.0, 0.0), PI),
    }
    .ok_or(CutError::InvalidPlane("the normal gives no rotation"))
}

/// What lies below the plane `{p : p · side.normal = offset}` of each of
/// `bodies`, summed. Bodies that overlap are counted once each, as
/// separate bodies are: give bodies that do not.
pub fn below(
    bodies: &[&Body],
    side: &CutPlane,
    offset: f64,
    tolerance: &Tolerance,
) -> Result<BelowPlane, CutError> {
    if !offset.is_finite() {
        return Err(CutError::NotFinite("the offset"));
    }
    let prepared = prepare(bodies, side.normal)?;
    below_in(&prepared, side, offset, tolerance)
}

/// One body's part below the plane: its volume, the first moment of that
/// volume about the origin, its area (the section's faces counted), and
/// the solid with the faces of it that make the section.
struct Part {
    volume: f64,
    moment: Vec3,
    area: f64,
    cut: Option<(Body, Vec<usize>)>,
}

fn below_in(
    bodies: &[Prepared<'_>],
    side: &CutPlane,
    offset: f64,
    tolerance: &Tolerance,
) -> Result<BelowPlane, CutError> {
    below_parts(bodies, side, offset, tolerance).map(|(found, _)| found)
}

/// [`below_in`], with each body's part below the plane, whose section
/// faces [`torsion`] meshes.
fn below_parts(
    bodies: &[Prepared<'_>],
    side: &CutPlane,
    offset: f64,
    tolerance: &Tolerance,
) -> Result<(BelowPlane, Vec<Part>), CutError> {
    let mut parts = Vec::with_capacity(bodies.len());
    for one in bodies {
        parts.push(part_one(one, side, offset, tolerance)?);
    }
    let (mut volume, mut moment, mut area) = (0.0, Vec3::ZERO, 0.0);
    // The section's own extent: the boxes of its faces.
    let mut extent = Bbox3::EMPTY;
    for (index, part) in parts.iter().enumerate() {
        volume += part.volume;
        moment += part.moment;
        area += part.area;
        if let Some((body, caps)) = &part.cut {
            for &face in caps {
                let face_id = FaceId(u32::try_from(face).unwrap_or(u32::MAX));
                let b = body
                    .face_bbox(face_id)
                    .map_err(|error| CutError::Boxing { body: index, error })?;
                if let Some((a, c)) = b.corners() {
                    extent.add(a);
                    extent.add(c);
                }
            }
        }
    }
    let centroid = (volume > 0.0).then(|| moment * (1.0 / volume));
    let section = section_of(bodies, &parts, side, offset, &extent)?;
    Ok((
        BelowPlane {
            offset,
            volume,
            centroid,
            surface: (area - section.area).max(0.0),
            section,
        },
        parts,
    ))
}

/// How a panel's two sums are held to each other ([`edge_sums`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    /// To [`PANEL_ALLOWANCE`] of the section's size and 64 ulps of their
    /// terms, as every section was until 2026-11-22.
    Strict,
    /// To that and to the rounding of the points they are made of
    /// ([`ROUNDING_ULPS`]), for a section whose panels do not settle
    /// strictly.
    Rounding,
}

/// The section's integrals over every part's section faces about `o`,
/// held to the section's own size, each body's as [`held_sums`] says.
fn sums_about(
    bodies: &[Prepared<'_>],
    parts: &[Part],
    side: &CutPlane,
    o: Vec3,
    size: f64,
) -> Result<[f64; 6], CutError> {
    let rule = (Rule::new(8), Rule::new(16));
    let mut total = [0.0; 6];
    for (index, part) in parts.iter().enumerate() {
        let Some((body, caps)) = &part.cut else {
            continue;
        };
        let one = bodies.get(index);
        let faces = held_sums(
            |held| faces_sums(body, caps, side, (o, size), &rule, held),
            one.is_none_or(|one| one.may_round),
            one.map(|one| &one.rounded),
        )
        .map_err(|what| CutError::Section { body: index, what })?;
        for sums in faces {
            for (sum, part) in total.iter_mut().zip(sums) {
                *sum += part;
            }
        }
    }
    Ok(total)
}

/// Whether `what` is one of the two ways a section's panels fail to
/// settle: they run out, or one is halved as far as it goes.
fn unsettled(what: &str) -> bool {
    what == TOO_MANY_PANELS || what == NOT_SETTLED
}

/// A body's section summed by `sum`: strictly first (`Held::Strict`, as
/// every section was summed before 2026-11-22, operation for operation
/// and in the same order, so every section that settled then is the same
/// to the bit); only where its panels do not settle so ([`unsettled`]) and
/// `may_round`, again with the rounding of its points allowed for
/// (`Held::Rounding`), and from then on in the call at once (`rounded`,
/// the body's [`Prepared::rounded`], set). A section summed again refused
/// before, as did the call it is part of. Every other refusal of the
/// strict pass, and every one where `may_round` is false, is the answer
/// as it was, in its words. A section with an edge on a rational spline
/// whose weights spread more than [`ROUNDING_SPREAD`] is not summed with
/// the rounding allowed for ([`WIDE_SPREAD`]): the strict pass's sums,
/// or its refusal in its words, stand.
fn held_sums<T>(
    sum: impl Fn(Held) -> Result<T, &'static str>,
    may_round: bool,
    rounded: Option<&Cell<bool>>,
) -> Result<T, &'static str> {
    let strict = if may_round && rounded.is_some_and(Cell::get) {
        None
    } else {
        match sum(Held::Strict) {
            Err(what) if may_round && unsettled(what) => Some(what),
            other => return other,
        }
    };
    if let Some(rounded) = rounded {
        rounded.set(true);
    }
    match sum(Held::Rounding) {
        Err(what) if unsettled(what) => Err(NOT_SETTLED_ROUNDED),
        Err(what) if what == WIDE_SPREAD => match strict {
            Some(what) => Err(what),
            None => sum(Held::Strict),
        },
        other => other,
    }
}

/// The six integrals of each of a body's section faces `caps`, in their
/// order, all of them taking one budget of panels. Held to the rounding,
/// a section with an edge on a rational spline of wide spread is
/// [`WIDE_SPREAD`] before any panel is summed.
fn faces_sums(
    body: &Body,
    caps: &[usize],
    side: &CutPlane,
    about: (Vec3, f64),
    rule: &(Rule, Rule),
    held: Held,
) -> Result<Vec<[f64; 6]>, &'static str> {
    if held == Held::Rounding
        && caps
            .iter()
            .filter_map(|&face| body.faces.get(face))
            .flat_map(|face| face.loops.iter())
            .filter_map(|&id| body.loop_(id))
            .flat_map(|lp| lp.coedges.iter())
            .filter_map(|coedge| body.edge(coedge.edge))
            .any(|edge| spreads_widely(&edge.curve))
    {
        return Err(WIDE_SPREAD);
    }
    let mut budget = MAX_PANELS;
    caps.iter()
        .map(|&face| cap_sums(body, face, side, about, rule, &mut budget, held))
        .collect()
}

/// The section in two passes, each about a point near what it sums so
/// that nothing is lost to cancellation: the area and the first moments
/// about the middle of the section's box, which give the centroid; then
/// the second moments about the centroid itself. Both are held to the
/// section's own size (its box), not to the bodies'.
fn section_of(
    bodies: &[Prepared<'_>],
    parts: &[Part],
    side: &CutPlane,
    offset: f64,
    extent: &Bbox3,
) -> Result<Section, CutError> {
    let empty = Section {
        area: 0.0,
        centroid: None,
        i_x: 0.0,
        i_y: 0.0,
        product: 0.0,
    };
    let Some(middle) = extent.center() else {
        return Ok(empty);
    };
    let size = (0.5 * extent.diagonal()).max(f64::MIN_POSITIVE);
    let onto = |p: Vec3| p + side.normal * (offset - p.dot(side.normal));
    let o = onto(middle);
    let [area, sa, sb, ..] = sums_about(bodies, parts, side, o, size)?;
    if area.is_nan() || area <= 0.0 {
        return Ok(empty);
    }
    let centroid = o + side.x_axis * (sa / area) + side.y_axis() * (sb / area);
    let [_, sa, sb, iaa, iab, ibb] = sums_about(bodies, parts, side, centroid, size)?;
    // What is left of the first moments about the centroid is rounding;
    // the parallel-axis rule takes it out of the second.
    let (ca, cb) = (sa / area, sb / area);
    Ok(Section {
        area,
        centroid: Some(centroid),
        i_x: ibb - area * cb * cb,
        i_y: iaa - area * ca * ca,
        product: iab - area * ca * cb,
    })
}

/// The whole of a solid body, measured.
fn whole(index: usize, body: &Body) -> Result<(f64, Vec3, f64), CutError> {
    if body.kind != BodyKind::Solid {
        return Err(CutError::NotSolid { body: index });
    }
    let props = mass_properties(body).map_err(|error| CutError::Mass { body: index, error })?;
    let centroid = props.centroid.filter(|_| props.volume > 0.0);
    let Some(centroid) = centroid else {
        return Err(CutError::NoVolume {
            body: index,
            volume: props.volume,
        });
    };
    Ok((props.volume, centroid * props.volume, props.area))
}

/// Nothing found below the plane, which is right only when no point of
/// the body lies more than the tolerance below it. The vertices alone do
/// not tell (a sphere, a torus or a closed revolved spline has none, or
/// none where it is lowest), so this is decided from a bound that holds
/// every point of the body ([`floor`]): nothing below when that bound is
/// within the tolerance of the plane, else the error, since an answer of
/// nothing could then be wrong.
fn nothing_below(
    one: &Prepared<'_>,
    side: &CutPlane,
    offset: f64,
    tolerance: &Tolerance,
) -> Result<Part, CutError> {
    let enough = offset - tolerance.linear;
    let lowest = floor(one, side.normal, enough, tolerance)?;
    // `floor` refuses a bound that is not a number.
    if lowest < enough {
        return Err(CutError::LostBelow {
            body: one.index,
            offset,
            depth: offset - lowest,
        });
    }
    Ok(Part {
        volume: 0.0,
        moment: Vec3::ZERO,
        area: 0.0,
        cut: None,
    })
}

/// Most times the Bezier patches of one freeform face are split in two
/// while its least along a direction is closed in on ([`surface_floor`]).
const MAX_FLOOR_SPLITS: usize = 1 << 14;

/// A number no point of the body lies below along `normal` (unit): the
/// least, over its faces, of a bound for each. An analytic face is bound
/// by its held box (`Body::held_face_bbox`) in a frame whose z is the
/// normal, which is tight where the face reaches its extreme. A freeform
/// face is bound by what of its surface lies in its trim, with its edges
/// ([`face_floor`]), or where that cannot be made by its whole surface
/// ([`surface_floor`]), which holds the face; either closed in on until
/// the bound reaches `enough` or is within a hundredth of the tolerance of
/// a point of the face (of the surface, for the whole surface). Edges and
/// vertices lie on the faces (to the tolerance the caller allows for), so
/// a closed solid's faces hold all of it. A bound that cannot be made is
/// an error, never a guess.
fn floor(
    one: &Prepared<'_>,
    normal: Vec3,
    enough: f64,
    tolerance: &Tolerance,
) -> Result<f64, CutError> {
    let linear = tolerance.linear;
    let boxing = |error| CutError::Boxing {
        body: one.index,
        error,
    };
    let turned = one.body.transformed(&onto_z(normal)?).map_err(boxing)?;
    let mut lowest = f64::INFINITY;
    for (index, face) in one.body.faces.iter().enumerate() {
        let bound = if face.surface.is_freeform() {
            let trimmed = (!face.loops.is_empty())
                .then(|| one.probe(tolerance))
                .flatten()
                .and_then(|probe| {
                    // How far a face's edges may be from its surface: the
                    // modelling tolerance, or an edge's own larger one.
                    let slack = linear + one.body.tolerances.largest().unwrap_or(0.0);
                    face_floor(probe, index, normal, enough, 0.01 * linear, slack)
                });
            trimmed
                .or_else(|| surface_floor(&face.surface, normal, enough, 0.01 * linear))
                .ok_or(CutError::Section {
                    body: one.index,
                    what: "a freeform face whose least along the normal cannot be bounded",
                })?
        } else {
            let face_id = FaceId(u32::try_from(index).unwrap_or(u32::MAX));
            let held = turned.held_face_bbox(face_id).map_err(boxing)?;
            held.corners().map_or(f64::INFINITY, |(a, _)| a.z)
        };
        if bound.is_nan() {
            return Err(CutError::NotFinite("a bound of a face along the normal"));
        }
        lowest = lowest.min(bound);
    }
    Ok(lowest)
}

/// How deep an edge is halved to tell whether it may meet a box.
const EDGE_DEPTH: u32 = 14;

/// An edge of a face in [`face_floor`]: its curve, made ready to be cut.
struct Bounding<'a> {
    curve: &'a Curve3,
    nets: CurveNets,
    range: (f64, f64),
    bbox: Bbox3,
}

/// What [`face_floor`] cuts: a box of the surface's parameters, or a range
/// of the parameter of one of the face's edges (by its place in the list).
#[derive(Clone, Copy, Debug)]
enum Region {
    Surface((f64, f64), (f64, f64)),
    Edge(usize, (f64, f64)),
}

/// A region waiting in [`face_floor`]'s heap: the least bound first, the
/// first made of equals.
struct Waiting {
    bound: f64,
    seq: u64,
    region: Region,
}

impl PartialEq for Waiting {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for Waiting {}

impl PartialOrd for Waiting {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Waiting {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .bound
            .total_cmp(&self.bound)
            .then(other.seq.cmp(&self.seq))
    }
}

/// Whether two boxes certainly do not meet.
fn apart(a: &Bbox3, b: &Bbox3) -> bool {
    let (Some((alo, ahi)), Some((blo, bhi))) = (a.corners(), b.corners()) else {
        return false;
    };
    alo.x > bhi.x
        || blo.x > ahi.x
        || alo.y > bhi.y
        || blo.y > ahi.y
        || alo.z > bhi.z
        || blo.z > ahi.z
}

/// Whether the edge's piece over `t` may meet the box `b`: its enclosure's
/// box halved down to `depth` (true where it cannot be told).
fn edge_meets(edge: &Bounding<'_>, t: (f64, f64), b: &Bbox3, depth: u32) -> bool {
    if t == edge.range && apart(&edge.bbox, b) {
        return false;
    }
    let Some(piece) = curve_piece(edge.curve, &edge.nets, t) else {
        return true;
    };
    if apart(&piece.bbox(), b) {
        return false;
    }
    let m = 0.5 * (t.0 + t.1);
    if depth == 0 || !(m > t.0 && m < t.1) || b.contains(edge.curve.eval(m), 0.0) {
        return true;
    }
    edge_meets(edge, (t.0, m), b, depth - 1) || edge_meets(edge, (m, t.1), b, depth - 1)
}

/// How far the heights along `normal` of five points of `at` over `[0, 1]`
/// spread.
fn spread(at: impl Fn(f64) -> Vec3, normal: Vec3) -> f64 {
    let (lo, hi) = (0..=4)
        .map(|k| at(f64::from(k) / 4.0).dot(normal))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), h| {
            (lo.min(h), hi.max(h))
        });
    (hi - lo).max(0.0)
}

/// A number no point of the trimmed freeform face `face` of the probe's
/// body lies below along `normal` (unit), from the part of its surface in
/// its trim and from its edges. Branch and bound over pieces of both
/// (`craft3d_geom::support`: a piece's least along a direction is proven,
/// from the convex hull of its Bézier net): a piece of the surface over a
/// box of its parameters is connected, so one that meets none of the
/// face's edges (its box grown by `slack`, how far an edge may be from
/// the surface: the face's outline is its edges' curves seen on the
/// surface, so each of its points is within `slack` of a point of an
/// edge) is all in the face or all out of it, told by its middle, and
/// dropped when out; every other piece is kept in full
/// (it holds every point of the face it covers). The region with the
/// least bound is cut in two (a piece of the surface across the parameter
/// along which it spreads more along the normal; an edge's range in
/// half) until that bound reaches `enough`, or comes within `resolution`
/// of the least of the face's points seen so far (points of its edges,
/// and middles of pieces that are in the face), or [`MAX_FLOOR_SPLITS`]
/// are spent, where the bound left is still a true one. So the bound
/// closes in on the face's least, on its boundary or at a point inside
/// it, whatever the surface does outside its trim. `None` where a piece
/// cannot be made (the caller then bounds the whole surface).
fn face_floor(
    probe: &Probe<'_>,
    face: usize,
    normal: Vec3,
    enough: f64,
    resolution: f64,
    slack: f64,
) -> Option<f64> {
    let body = probe.body();
    let shape = body.faces.get(face)?;
    let surface = &shape.surface;
    let nets = SurfaceNets::of(surface).ok()?;
    let mut edges = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for &loop_id in &shape.loops {
        for coedge in &body.loop_(loop_id)?.coedges {
            if !seen.insert(coedge.edge.0) {
                continue;
            }
            let edge = body.edge(coedge.edge)?;
            let curve_nets = CurveNets::of(&edge.curve);
            let piece = curve_piece(&edge.curve, &curve_nets, edge.range)?;
            edges.push(Bounding {
                curve: &edge.curve,
                nets: curve_nets,
                range: edge.range,
                bbox: piece.bbox(),
            });
        }
    }
    if edges.is_empty() {
        return None;
    }
    let mut open = BinaryHeap::new();
    let mut least_point = f64::INFINITY;
    let mut seq = 0_u64;
    let mut push =
        |region: Region, open: &mut BinaryHeap<Waiting>, least_point: &mut f64| -> Option<()> {
            let mut seen_point = |p: Vec3| {
                let h = p.dot(normal);
                if h.is_finite() {
                    *least_point = least_point.min(h);
                }
            };
            let piece: Piece = match region {
                Region::Edge(index, t) => {
                    let edge = edges.get(index)?;
                    let piece = curve_piece(edge.curve, &edge.nets, t)?;
                    for s in [t.0, 0.5 * (t.0 + t.1), t.1] {
                        seen_point(edge.curve.eval(s));
                    }
                    piece
                }
                Region::Surface(u, v) => {
                    let piece = surface_piece(surface, &nets, u, v)?;
                    let middle = surface.eval(0.5 * (u.0 + u.1), 0.5 * (v.0 + v.1));
                    if !middle.is_finite() {
                        return None;
                    }
                    let side = probe.on_face(face, middle).ok();
                    if side == Some(OnFace::In) {
                        seen_point(middle);
                    }
                    if side == Some(OnFace::Out) {
                        let bbox = piece.bbox();
                        let reach = match bbox.corners() {
                            Some((lo, hi)) => {
                                let d = Vec3::new(slack, slack, slack);
                                Bbox3::from_points([lo - d, hi + d])
                            }
                            None => return None,
                        };
                        if !edges
                            .iter()
                            .any(|edge| edge_meets(edge, edge.range, &reach, EDGE_DEPTH))
                        {
                            // Wholly out of the face: nothing of it is lost.
                            return Some(());
                        }
                    }
                    piece
                }
            };
            let bound = piece.least(normal);
            if !bound.is_finite() {
                return None;
            }
            seq += 1;
            open.push(Waiting { bound, seq, region });
            Some(())
        };
    for (index, edge) in edges.iter().enumerate() {
        push(Region::Edge(index, edge.range), &mut open, &mut least_point)?;
    }
    let (u, v) = surface.domain();
    push(Region::Surface(u, v), &mut open, &mut least_point)?;
    for _ in 0..MAX_FLOOR_SPLITS {
        let lowest = open.pop()?;
        if lowest.bound >= enough || least_point - lowest.bound <= resolution {
            return Some(lowest.bound);
        }
        let halves = match lowest.region {
            Region::Edge(index, (a, b)) => {
                let m = 0.5 * (a + b);
                (m > a && m < b)
                    .then_some([Region::Edge(index, (a, m)), Region::Edge(index, (m, b))])
            }
            Region::Surface((u0, u1), (v0, v1)) => {
                let (um, vm) = (0.5 * (u0 + u1), 0.5 * (v0 + v1));
                let along_u = |s: f64| surface.eval(u0 + (u1 - u0) * s, vm);
                let along_v = |s: f64| surface.eval(um, v0 + (v1 - v0) * s);
                let (su, sv) = (spread(along_u, normal), spread(along_v, normal));
                let cut_u = su > sv
                    || (su == sv
                        && (along_u(1.0) - along_u(0.0)).length()
                            >= (along_v(1.0) - along_v(0.0)).length());
                if cut_u {
                    (um > u0 && um < u1).then_some([
                        Region::Surface((u0, um), (v0, v1)),
                        Region::Surface((um, u1), (v0, v1)),
                    ])
                } else {
                    (vm > v0 && vm < v1).then_some([
                        Region::Surface((u0, u1), (v0, vm)),
                        Region::Surface((u0, u1), (vm, v1)),
                    ])
                }
            }
        };
        // A region as small as an f64 makes it: its bound, the least
        // left, is the answer.
        let Some(halves) = halves else {
            return Some(lowest.bound);
        };
        for half in halves {
            push(half, &mut open, &mut least_point)?;
        }
    }
    open.peek().map(|lowest| lowest.bound)
}

/// A number no point of a freeform surface lies below along `normal`:
/// branch and bound over its rational Bezier patches (positive weights,
/// so each patch lies in the convex hull of its control points, and the
/// least of `p · normal` over them bounds it), splitting the patch with
/// the least bound in two until that bound reaches `enough`, or comes
/// within `resolution` of the least of the surface's points evaluated so
/// far (patch corners and middles: the least is then known), or the work
/// is spent ([`MAX_FLOOR_SPLITS`]: the bound left is still a true one,
/// only further below). A patch is split across the direction of its
/// parameters along which its net spreads more along the normal (a
/// surface turned about the normal does not spread along its turn, so
/// only its other direction is cut). `None` for a surface that is not
/// freeform or has no finite patches.
fn surface_floor(surface: &Surface, normal: Vec3, enough: f64, resolution: f64) -> Option<f64> {
    let patches: Vec<RationalBezierPatch> = match surface {
        Surface::Bezier(patch) => vec![RationalBezierPatch::from_bezier(patch)],
        Surface::Nurbs(nurbs) => nurbs
            .bezier_spans()
            .ok()?
            .into_iter()
            .map(|span| span.patch)
            .collect(),
        _ => return None,
    };
    let mut open = BinaryHeap::new();
    let mut least_point = f64::INFINITY;
    let mut seq = 0_u64;
    let mut push = |patch: RationalBezierPatch,
                    open: &mut BinaryHeap<Lowest>,
                    least_point: &mut f64|
     -> Option<()> {
        let bound = patch
            .points()
            .map(|p| p.dot(normal))
            .fold(f64::INFINITY, f64::min);
        if !bound.is_finite() {
            return None;
        }
        for (s, t) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.5, 0.5)] {
            let reached = patch.eval(s, t).dot(normal);
            if reached.is_finite() {
                *least_point = least_point.min(reached);
            }
        }
        seq += 1;
        open.push(Lowest { bound, seq, patch });
        Some(())
    };
    for patch in patches {
        push(patch, &mut open, &mut least_point)?;
    }
    for _ in 0..MAX_FLOOR_SPLITS {
        let lowest = open.pop()?;
        if lowest.bound >= enough || least_point - lowest.bound <= resolution {
            return Some(lowest.bound);
        }
        let (a, b) = if spreads_along_u(&lowest.patch, normal) {
            lowest.patch.split_u(0.5)
        } else {
            lowest.patch.split_v(0.5)
        };
        push(a, &mut open, &mut least_point)?;
        push(b, &mut open, &mut least_point)?;
    }
    open.peek().map(|lowest| lowest.bound)
}

/// Whether the net of `patch` spreads more along `normal` across its
/// rows (along u) than along them (along v).
fn spreads_along_u(patch: &RationalBezierPatch, normal: Vec3) -> bool {
    let (du, dv) = patch.degrees();
    let at = |i: usize, j: usize| {
        patch
            .control_point(i * (dv + 1) + j)
            .map_or(0.0, |p| p.dot(normal))
    };
    let spread = |values: &mut dyn Iterator<Item = f64>| {
        let (lo, hi) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
            (lo.min(x), hi.max(x))
        });
        (hi - lo).max(0.0)
    };
    let along_u = (0..=dv)
        .map(|j| spread(&mut (0..=du).map(|i| at(i, j))))
        .fold(0.0, f64::max);
    let along_v = (0..=du)
        .map(|i| spread(&mut (0..=dv).map(|j| at(i, j))))
        .fold(0.0, f64::max);
    along_u > along_v
}

/// A patch waiting in [`surface_floor`]'s heap: the least bound first,
/// the first made of equals.
struct Lowest {
    bound: f64,
    seq: u64,
    patch: RationalBezierPatch,
}

impl PartialEq for Lowest {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for Lowest {}

impl PartialOrd for Lowest {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Lowest {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // The heap pops its greatest: the least bound, then the oldest.
        other
            .bound
            .total_cmp(&self.bound)
            .then(other.seq.cmp(&self.seq))
    }
}

fn part_one(
    one: &Prepared<'_>,
    side: &CutPlane,
    offset: f64,
    tolerance: &Tolerance,
) -> Result<Part, CutError> {
    let index = one.index;
    if offset <= one.lo {
        return Ok(Part {
            volume: 0.0,
            moment: Vec3::ZERO,
            area: 0.0,
            cut: None,
        });
    }
    if offset >= one.hi {
        let (volume, moment, area) = one.whole()?;
        return Ok(Part {
            volume,
            moment,
            area,
            cut: None,
        });
    }
    // A box with its top face in the plane that holds all of the body
    // below it: wider than the body's box on every side, and deeper.
    let center = one.bbox.center().unwrap_or(Vec3::ZERO);
    let r = 0.5 * one.bbox.diagonal();
    let margin = 0.25 * r + 10.0 * tolerance.linear;
    let half = r + margin;
    let depth = offset - (center.dot(side.normal) - r) + margin;
    let q = center + side.normal * (offset - center.dot(side.normal));
    let corner = q - side.x_axis * half - side.y_axis() * half - side.normal * depth;
    let frame = Frame::new(corner, side.normal, side.x_axis)
        .ok_or(CutError::InvalidPlane("the directions do not make a frame"))?;
    let tool = block(&frame, 2.0 * half, 2.0 * half, depth).map_err(|error| CutError::Kernel {
        body: index,
        offset,
        error,
    })?;
    let cut = match boolean(one.body, &tool, BooleanOp::Intersect, tolerance) {
        Ok(cut) => cut,
        Err(KernelError::EmptyResult { .. }) => {
            return nothing_below(one, side, offset, tolerance);
        }
        Err(error) => {
            return Err(CutError::Kernel {
                body: index,
                offset,
                error,
            });
        }
    };
    // The block's faces are -z, +z, -y, +y, -x, +x of its frame: the top
    // one, in the plane, is the second.
    let top = FaceSource {
        operand: Operand::B,
        face: FaceId(1),
    };
    let mut caps = Vec::new();
    for (face, sources) in cut.merged.iter().enumerate() {
        if sources.contains(&top) {
            caps.push(face);
        } else if sources.iter().any(|s| s.operand == Operand::B) {
            return Err(CutError::Section {
                body: index,
                what: "the box cut off more than what lies below the plane (a side of it is in the result)",
            });
        }
    }
    let props =
        mass_properties(&cut.body).map_err(|error| CutError::Mass { body: index, error })?;
    let Some(centroid) = props.centroid.filter(|_| props.volume > 0.0) else {
        return nothing_below(one, side, offset, tolerance);
    };
    Ok(Part {
        volume: props.volume,
        moment: centroid * props.volume,
        area: props.area,
        cut: Some((cut.body, caps)),
    })
}

/// The six plane integrals (area, ∫a, ∫b, ∫a², ∫ab, ∫b², with a along
/// `side.x_axis` and b along `side.y_axis()` from `o`) of a flat face of
/// `body` in the plane whose outward normal is `side.normal`.
fn cap_sums(
    body: &Body,
    face: usize,
    side: &CutPlane,
    (o, size): (Vec3, f64),
    rule: &(Rule, Rule),
    budget: &mut usize,
    held: Held,
) -> Result<[f64; 6], &'static str> {
    let face = body
        .faces
        .get(face)
        .ok_or("a face that is not in the body")?;
    let Surface::Plane(plane) = &face.surface else {
        return Err("a face in the plane that is not flat");
    };
    // The face's outward normal is its surface's normal, or the other way
    // round when the face is reversed; the cap of what is below faces
    // along the plane's normal. Its loops run counter-clockwise about its
    // outward normal seen from outside (`craft3d_topo`'s convention, which
    // already takes `reversed` into account), so about the plane's normal,
    // and Green's theorem in (a, b), whose a × b is that normal, sums them
    // as they run: no sign from the surface's own normal. (That sign was
    // taken until 2026-11-21, which turned the sums of a reversed face in
    // the plane negative: a pocket's floor, a notch's, a shell's inner
    // wall, a mirrored face, all refused as enclosing no area.)
    let normal = plane.frame().z();
    let facing = normal.dot(side.normal);
    let outward = if face.reversed { -facing } else { facing };
    if outward.is_nan() || outward <= 0.5 {
        return Err("a face in the plane that does not face away from what is below");
    }
    let mut sums = [0.0; 6];
    for &loop_id in &face.loops {
        let lp = body
            .loop_(loop_id)
            .ok_or("a loop that is not in the body")?;
        for coedge in &lp.coedges {
            let edge = body
                .edge(coedge.edge)
                .ok_or("an edge that is not in the body")?;
            let along = edge_sums(&edge.curve, edge.range, side, (o, size), rule, budget, held)?;
            let sign = if coedge.reversed { -1.0 } else { 1.0 };
            for (sum, part) in sums.iter_mut().zip(along) {
                *sum += sign * part;
            }
        }
    }
    let [area, ..] = sums;
    if area.is_nan() || area <= 0.0 {
        return Err("a face in the plane whose loops enclose no area");
    }
    Ok(sums)
}

/// The powers of the length in each plane integral.
const POWERS: [i32; 6] = [2, 3, 3, 4, 4, 4];

/// What the points of a curve are computed from, for the rounding they
/// carry ([`ROUNDING_ULPS`]): `far`, the size of the numbers a point is
/// made from (its centre's distance from the origin and its radius, its
/// line's origin's distance, its spline's farthest control point times
/// the spread of its weights), and `rational`, the degree of a spline with
/// weights other than 1, whose derivative is made from weighted control
/// points and so carries their rounding over its parameter's stretch
/// (zero for every other curve, whose derivative is rounded as a share of
/// itself).
#[derive(Clone, Copy, Debug)]
struct Reach {
    far: f64,
    rational: f64,
}

/// Whether `curve` is a rational spline whose heaviest weight is more
/// than [`ROUNDING_SPREAD`] times its lightest (or whose weights are not
/// all positive): one whose section is not held to the rounding of its
/// coordinates ([`held_sums`]).
fn spreads_widely(curve: &Curve3) -> bool {
    let Curve3::Nurbs(spline) = curve else {
        return false;
    };
    let weights = spline.weights();
    let light = weights.iter().copied().fold(f64::INFINITY, f64::min);
    let heavy = weights.iter().copied().fold(0.0, f64::max);
    !(light > 0.0 && heavy <= ROUNDING_SPREAD * light)
}

impl Reach {
    /// [`WIDE_SPREAD`] for a rational spline that [`spreads_widely`], and
    /// an error where it is not a finite number.
    fn of(curve: &Curve3) -> Result<Reach, &'static str> {
        if spreads_widely(curve) {
            return Err(WIDE_SPREAD);
        }
        let reach = match curve {
            Curve3::Line(line) => Reach {
                far: line.origin().length(),
                rational: 0.0,
            },
            Curve3::Circle(circle) => Reach {
                far: circle.frame().origin().length() + circle.radius(),
                rational: 0.0,
            },
            Curve3::Ellipse(ellipse) => {
                let (a, b) = ellipse.semi_axes();
                Reach {
                    far: ellipse.frame().origin().length() + a.max(b),
                    rational: 0.0,
                }
            }
            Curve3::Nurbs(spline) => {
                let weights = spline.weights();
                let light = weights.iter().copied().fold(f64::INFINITY, f64::min);
                let heavy = weights.iter().copied().fold(0.0, f64::max);
                let far = spline
                    .control_points()
                    .iter()
                    .map(|p| p.length())
                    .fold(0.0, f64::max);
                let rational = if weights.iter().all(|&w| w == 1.0) {
                    0.0
                } else {
                    spline.degree() as f64
                };
                Reach {
                    far: far * (heavy / light),
                    rational,
                }
            }
        };
        if reach.far.is_finite() && reach.rational.is_finite() {
            Ok(reach)
        } else {
            Err("an edge whose coordinates are not finite")
        }
    }

    /// How much rounding each of the six integrands of [`edge_sums`] may
    /// carry at the point at `t` (moving at `speed`, in a stretch of the
    /// parameter `span` long between two breaks) whose offsets are `a`
    /// and `b` and whose derivative's components are `da` and `db`: their
    /// first-order change when the point moves by `u` along each axis and
    /// the derivative by `ud`, each [`ROUNDING_ULPS`] of what it is made
    /// from (the point: the curve's `far` and its parameter times its
    /// speed; the derivative: itself, and a rational spline's `far` over
    /// the stretch).
    fn carried(
        &self,
        t: f64,
        speed: f64,
        span: f64,
        (a, b, da, db): (f64, f64, f64, f64),
    ) -> [f64; 6] {
        let ulps = ROUNDING_ULPS * f64::EPSILON;
        let u = ulps * (self.far + t.abs() * speed);
        let ud = ulps * (speed + self.far * self.rational / span);
        let (a, b, da, db) = (a.abs(), b.abs(), da.abs(), db.abs());
        [
            u * db + a * ud,
            a * u * db + 0.5 * a * a * ud,
            b * u * da + 0.5 * b * b * ud,
            a * a * u * db + a * a * a / 3.0 * ud,
            (a * b + 0.5 * a * a) * u * db + 0.5 * a * a * b * ud,
            b * b * u * da + b * b * b / 3.0 * ud,
        ]
    }
}

/// The six line integrals of [`cap_sums`] along `curve` from `range.0` to
/// `range.1`, each panel's two sums held to each other as `held` says.
fn edge_sums(
    curve: &Curve3,
    (t0, t1): (f64, f64),
    side: &CutPlane,
    (o, size): (Vec3, f64),
    rule: &(Rule, Rule),
    budget: &mut usize,
    held: Held,
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
        return Err("an edge with more knot spans than a section takes");
    }
    // What the curve's points are made from, where their rounding is
    // allowed for.
    let reach = match held {
        Held::Strict => None,
        Held::Rounding => Some(Reach::of(curve)?),
    };
    let across = side.y_axis();
    // The six integrands at `t`, the speed there and, held to the
    // rounding, how much rounding each may carry (`span`: the stretch of
    // the parameter between the two breaks the point lies between).
    let integrand = |t: f64, span: f64| -> ([f64; 6], f64, [f64; 6]) {
        let (p, d) = curve.derivatives(t);
        let r = p - o;
        let (a, b) = (r.dot(side.x_axis), r.dot(across));
        let (da, db) = (d.dot(side.x_axis), d.dot(across));
        let speed = d.length();
        let carried = reach.map_or([0.0; 6], |reach| {
            reach.carried(t, speed, span, (a, b, da, db))
        });
        (
            [
                a * db,
                0.5 * a * a * db,
                -0.5 * b * b * da,
                a * a * a / 3.0 * db,
                0.5 * a * a * b * db,
                -b * b * b / 3.0 * da,
            ],
            speed,
            carried,
        )
    };
    type Summed = ([f64; 6], [f64; 6], f64, [f64; 6]);
    let sum_on = |rule: &Rule, a: f64, b: f64, span: f64| -> Summed {
        let mut sums = [0.0; 6];
        let mut magnitude = [0.0; 6];
        let mut rounding = [0.0; 6];
        let mut length = 0.0;
        for (t, w) in rule.points(a, b) {
            let (f, speed, carried) = integrand(t, span);
            for k in 0..6 {
                let (Some(s), Some(m), Some(v)) = (sums.get_mut(k), magnitude.get_mut(k), f.get(k))
                else {
                    continue;
                };
                *s += w * v;
                *m += (w * v).abs();
            }
            for (sum, c) in rounding.iter_mut().zip(carried) {
                *sum += w * c;
            }
            length += w * speed;
        }
        (sums, magnitude, length, rounding)
    };
    let mut total = [0.0; 6];
    let mut open: Vec<(f64, f64, u32, f64)> = breaks
        .windows(2)
        .filter_map(|w| match w {
            [a, b] if b > a => Some((*a, *b, 0, *b - *a)),
            _ => None,
        })
        .rev()
        .collect();
    while let Some((a, b, halvings, span)) = open.pop() {
        *budget = budget.checked_sub(1).ok_or(TOO_MANY_PANELS)?;
        let (coarse, _, _, coarse_rounding) = sum_on(&rule.0, a, b, span);
        let (fine, magnitude, length, fine_rounding) = sum_on(&rule.1, a, b, span);
        let share = (length / size).max(1e-9);
        let settled = (0..6).all(|k| {
            let (Some(c), Some(f), Some(m), Some(p), Some(rc), Some(rf)) = (
                coarse.get(k),
                fine.get(k),
                magnitude.get(k),
                POWERS.get(k),
                coarse_rounding.get(k),
                fine_rounding.get(k),
            ) else {
                return false;
            };
            let allowed = PANEL_ALLOWANCE * size.ppowi(*p) * share + 64.0 * f64::EPSILON * m;
            // The rounding each of the two sums may carry, on top: none
            // when held strictly, so the test is the one of before
            // 2026-11-22 to the bit.
            let allowed = match held {
                Held::Strict => allowed,
                Held::Rounding => allowed + rc + rf,
            };
            (f - c).abs() <= allowed
        });
        if settled {
            for (sum, part) in total.iter_mut().zip(fine) {
                *sum += part;
            }
            continue;
        }
        if halvings >= MAX_HALVINGS {
            return Err(NOT_SETTLED);
        }
        let mid = 0.5 * (a + b);
        if !(mid > a && mid < b) {
            return Err(NOT_SETTLED);
        }
        open.push((mid, b, halvings + 1, span));
        open.push((a, mid, halvings + 1, span));
    }
    if !total.iter().all(|x| x.is_finite()) {
        return Err("an edge whose integrals are not finite");
    }
    Ok(total)
}

/// Where the section of the bodies by a plane begins and ends along a
/// direction ([`section_extent`]): `lo` is no more than the least of
/// `p · along` over the section and `hi` no less than the greatest, each
/// within a hundredth of the tolerance of a point of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SectionExtent {
    pub lo: f64,
    pub hi: f64,
}

/// Where the section of `bodies` by the plane `{p : p · side.normal =
/// offset}` begins and ends along the unit direction `along`, or `None`
/// when the plane does not cut them (the length of a waterline, the span
/// of a cut along an axis). The section is the flat faces of what lies
/// below the plane that are pieces of the plane ([`below`]); a flat face
/// lies within its edges, so its extent along any direction is theirs.
/// Each edge's least along `along` and along `−along` is closed in on by
/// branch and bound over pieces of its curve (each piece's least is a
/// proof, from the convex hull of its Bézier net) until it is within a
/// hundredth of the tolerance of a point of the edge.
pub fn section_extent(
    bodies: &[&Body],
    side: &CutPlane,
    offset: f64,
    along: Vec3,
    tolerance: &Tolerance,
) -> Result<Option<SectionExtent>, CutError> {
    if !offset.is_finite() {
        return Err(CutError::NotFinite("the offset"));
    }
    let along = along
        .normalized()
        .filter(|d| d.is_finite())
        .ok_or(CutError::InvalidPlane("the direction has no length"))?;
    let prepared = prepare(bodies, side.normal)?;
    let resolution = 0.01 * tolerance.linear;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for one in &prepared {
        let part = part_one(one, side, offset, tolerance)?;
        let Some((body, caps)) = &part.cut else {
            continue;
        };
        let mut seen = std::collections::BTreeSet::new();
        for &face in caps {
            let shape = body.faces.get(face).ok_or(CutError::Section {
                body: one.index,
                what: "a face that is not in the body",
            })?;
            for &loop_id in &shape.loops {
                let lost = CutError::Section {
                    body: one.index,
                    what: "a loop or an edge that is not in the body",
                };
                let coedges = &body.loop_(loop_id).ok_or(lost.clone())?.coedges;
                for coedge in coedges {
                    if !seen.insert(coedge.edge.0) {
                        continue;
                    }
                    let edge = body.edge(coedge.edge).ok_or(lost.clone())?;
                    let unbounded = CutError::Section {
                        body: one.index,
                        what: "an edge whose extent along the direction cannot be bounded",
                    };
                    let least = curve_least(&edge.curve, edge.range, along, resolution)
                        .ok_or(unbounded.clone())?;
                    let most = curve_least(&edge.curve, edge.range, -along, resolution)
                        .ok_or(unbounded)?;
                    lo = lo.min(least);
                    hi = hi.max(-most);
                }
            }
        }
    }
    if !(lo.is_finite() && hi.is_finite()) {
        return Ok(None);
    }
    Ok(Some(SectionExtent { lo, hi }))
}

/// No more than the least of `curve(t) · d` over `range`, within
/// `resolution` of the value at a point of it: branch and bound over
/// pieces of the curve (`craft3d_geom::support::curve_piece`), the piece
/// with the least bound halved until that bound is within `resolution` of
/// the least point seen, or [`MAX_FLOOR_SPLITS`] are spent (the bound
/// left is still a true one). `None` where a piece cannot be made or a
/// bound is not a number.
fn curve_least(curve: &Curve3, range: (f64, f64), d: Vec3, resolution: f64) -> Option<f64> {
    let nets = CurveNets::of(curve);
    let mut open = BinaryHeap::new();
    let mut least_point = f64::INFINITY;
    let mut seq = 0_u64;
    let mut push = |t: (f64, f64), open: &mut BinaryHeap<Waiting>, least_point: &mut f64| {
        let piece = curve_piece(curve, &nets, t)?;
        for s in [t.0, 0.5 * (t.0 + t.1), t.1] {
            let h = curve.eval(s).dot(d);
            if h.is_finite() {
                *least_point = least_point.min(h);
            }
        }
        let bound = piece.least(d);
        if !bound.is_finite() {
            return None;
        }
        seq += 1;
        open.push(Waiting {
            bound,
            seq,
            region: Region::Edge(0, t),
        });
        Some(())
    };
    push(range, &mut open, &mut least_point)?;
    for _ in 0..MAX_FLOOR_SPLITS {
        let lowest = open.pop()?;
        let Region::Edge(_, (a, b)) = lowest.region else {
            return None;
        };
        let m = 0.5 * (a + b);
        if least_point - lowest.bound <= resolution || !(m > a && m < b) {
            return Some(lowest.bound.min(least_point));
        }
        push((a, m), &mut open, &mut least_point)?;
        push((m, b), &mut open, &mut least_point)?;
    }
    open.pop().map(|w| w.bound.min(least_point))
}

/// The offset of a plane square to `side.normal` at which the volume of
/// `bodies` below it is `volume`, found as the module documentation says,
/// and what lies below the plane there. An offset the boolean refuses
/// (a face of a body lying in the plane where a step landed) is stepped
/// around: the next offset is the first of the points of the bracket in
/// [`AROUND`] not yet tried since the last offset measured; refused at
/// all of them, it is the error.
pub fn level_for(
    bodies: &[&Body],
    side: &CutPlane,
    volume: f64,
    tolerance: &Tolerance,
) -> Result<Level, CutError> {
    let mut rounded = false;
    level_held(bodies, side, volume, tolerance, true, &mut rounded)
}

/// [`level_for`], a section that does not settle strictly summed again
/// with the rounding of its coordinates allowed for only where
/// `may_round` ([`held_sums`]; where not, its strict refusal ends the
/// search as it did before 2026-11-22); `rounded` is set where some
/// section was (so before 2026-11-22 the search was refused, whatever it
/// ends in now). For the searches of [`floating`].
fn level_held(
    bodies: &[&Body],
    side: &CutPlane,
    volume: f64,
    tolerance: &Tolerance,
    may_round: bool,
    rounded: &mut bool,
) -> Result<Level, CutError> {
    if !volume.is_finite() {
        return Err(CutError::NotFinite("the volume"));
    }
    let mut prepared = prepare(bodies, side.normal)?;
    for one in &mut prepared {
        one.may_round = may_round;
    }
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut most = 0.0;
    for one in &prepared {
        lo = lo.min(one.lo);
        hi = hi.max(one.hi);
        most += one.whole()?.0;
    }
    let found = search((lo, hi), most, volume, |offset| {
        below_in(&prepared, side, offset, tolerance)
    });
    *rounded |= prepared.iter().any(|one| one.rounded.get());
    found
}

/// The search of [`level_for`] between the offsets `lo` and `hi` that
/// bracket the bodies, whose whole volume is `most`, with `measure` what
/// lies below the plane at an offset.
fn search(
    (mut lo, mut hi): (f64, f64),
    most: f64,
    volume: f64,
    mut measure: impl FnMut(f64) -> Result<BelowPlane, CutError>,
) -> Result<Level, CutError> {
    if volume <= 0.0 || volume >= most {
        return Err(CutError::OutOfRange {
            volume,
            total: most,
        });
    }
    let mut offset = lo + (hi - lo) * (volume / most);
    let mut last_residual = f64::INFINITY;
    // The offsets refused since the last one measured, and how far along
    // `AROUND` the next one to try is.
    let mut refused: Vec<f64> = Vec::new();
    let mut around = 0;
    for step in 1..=MAX_STEPS {
        let found = match measure(offset) {
            Ok(found) => found,
            Err(error @ CutError::Kernel { .. }) => {
                refused.push(offset);
                let next = AROUND
                    .iter()
                    .enumerate()
                    .skip(around)
                    .map(|(k, share)| (k, lo + (hi - lo) * share))
                    // Not one refused already, up to rounding.
                    .find(|&(_, x)| {
                        x > lo
                            && x < hi
                            && refused.iter().all(|&r| (x - r).abs() > 1e-9 * (hi - lo))
                    });
                let Some((k, x)) = next else {
                    return Err(error);
                };
                around = k + 1;
                offset = x;
                continue;
            }
            Err(error) => return Err(error),
        };
        refused.clear();
        around = 0;
        let residual = found.volume - volume;
        if residual.abs() <= SETTLED * volume {
            return Ok(Level {
                below: found,
                steps: step,
                residual: residual.abs(),
            });
        }
        if residual < 0.0 {
            lo = offset;
        } else {
            hi = offset;
        }
        let area = found.section.area;
        let newton = offset - residual / area;
        let halving = 0.5 * (lo + hi);
        let use_newton = area > 0.0
            && newton.is_finite()
            && newton > lo
            && newton < hi
            && residual.abs() <= 0.5 * last_residual;
        let next = if use_newton { newton } else { halving };
        last_residual = residual.abs();
        if !(next > lo && next < hi) {
            // The bracket is as narrow as an f64 makes it: an answer only
            // when the volume is near what was asked for, else the volume
            // jumps there (which a closed body's does not).
            if residual.abs() <= UNSETTLED * volume {
                return Ok(Level {
                    below: found,
                    steps: step,
                    residual: residual.abs(),
                });
            }
            return Err(CutError::NotConverged {
                steps: step,
                low: lo,
                high: hi,
                residual: residual.abs(),
            });
        }
        offset = next;
    }
    Err(CutError::NotConverged {
        steps: MAX_STEPS,
        low: lo,
        high: hi,
        residual: last_residual,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod far_tests;
