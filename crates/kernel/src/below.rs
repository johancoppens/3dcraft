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
//! is integrated anywhere.
//!
//! The offset for a volume is where the volume below the plane is that
//! volume. The volume grows with the offset and its derivative is the
//! section's area, so the offset is found by Newton's method on that
//! derivative, kept inside a bracket that every step narrows and that
//! falls back on halving where a Newton step would leave it or did not
//! halve the residual; at most [`MAX_STEPS`] steps, after which "did not
//! converge" is an error like any other.

use crate::KernelError;
use crate::boolean::{BooleanOp, FaceSource, Operand, boolean};
use crate::primitives::block;
use craft3d_geom::{Curve3, RationalBezierPatch, Surface};
use craft3d_math::quadrature::Rule;
use craft3d_math::{Bbox3, Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, BodyKind, FaceId, MassError, TransformError, mass_properties};
use std::cell::OnceCell;
use std::collections::BinaryHeap;
use std::f64::consts::PI;
use std::fmt;

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
        let radius = half.hypot(self.product);
        let (major, minor) = (mean + radius, mean - radius);
        // The moment about an axis at angle θ from x is
        // i_x cos²θ + i_y sin²θ − 2 product sin θ cos θ; it is largest at
        // 2θ = atan2(−product, half).
        let theta = 0.5 * (-self.product).atan2(half);
        let (s, c) = theta.sin_cos();
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
}

impl Prepared<'_> {
    fn whole(&self) -> Result<(f64, Vec3, f64), CutError> {
        self.whole
            .get_or_init(|| whole(self.index, self.body))
            .clone()
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

/// A rotation that turns the unit `direction` onto z.
fn onto_z(direction: Vec3) -> Result<Transform, CutError> {
    let z = Vec3::new(0.0, 0.0, 1.0);
    let cos = direction.dot(z).clamp(-1.0, 1.0);
    let axis = direction.cross(z);
    match axis.normalized() {
        // Turning `direction` onto z about their common normal.
        Some(axis) if axis.is_finite() => Transform::rotation(axis, cos.acos()),
        // Along z already, or against it.
        _ if cos > 0.0 => Some(Transform::IDENTITY),
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
    let section = section_of(&parts, side, offset, &extent)?;
    Ok(BelowPlane {
        offset,
        volume,
        centroid,
        surface: (area - section.area).max(0.0),
        section,
    })
}

/// The section's integrals over every part's section faces about `o`,
/// held to the section's own size.
fn sums_about(parts: &[Part], side: &CutPlane, o: Vec3, size: f64) -> Result<[f64; 6], CutError> {
    let rule = (Rule::new(8), Rule::new(16));
    let mut total = [0.0; 6];
    for (index, part) in parts.iter().enumerate() {
        let Some((body, caps)) = &part.cut else {
            continue;
        };
        let mut budget = MAX_PANELS;
        for &face in caps {
            let sums = cap_sums(body, face, side, (o, size), &rule, &mut budget)
                .map_err(|what| CutError::Section { body: index, what })?;
            for (sum, part) in total.iter_mut().zip(sums) {
                *sum += part;
            }
        }
    }
    Ok(total)
}

/// The section in two passes, each about a point near what it sums so
/// that nothing is lost to cancellation: the area and the first moments
/// about the middle of the section's box, which give the centroid; then
/// the second moments about the centroid itself. Both are held to the
/// section's own size (its box), not to the bodies'.
fn section_of(
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
    let [area, sa, sb, ..] = sums_about(parts, side, o, size)?;
    if area.is_nan() || area <= 0.0 {
        return Ok(empty);
    }
    let centroid = o + side.x_axis * (sa / area) + side.y_axis() * (sb / area);
    let [_, sa, sb, iaa, iab, ibb] = sums_about(parts, side, centroid, size)?;
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
    let lowest = floor(one, side.normal, enough, tolerance.linear)?;
    if !(lowest >= enough) {
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
/// face is bound by its surface ([`surface_floor`]), which holds the face,
/// closed in on until the bound reaches `enough` or is within a hundredth
/// of `linear` of a point of the surface. Edges and vertices lie on the
/// faces (to the tolerance the caller allows for), so a closed solid's
/// faces hold all of it. A bound that cannot be made is an error, never a
/// guess.
fn floor(one: &Prepared<'_>, normal: Vec3, enough: f64, linear: f64) -> Result<f64, CutError> {
    let boxing = |error| CutError::Boxing {
        body: one.index,
        error,
    };
    let turned = one.body.transformed(&onto_z(normal)?).map_err(boxing)?;
    let mut lowest = f64::INFINITY;
    for (index, face) in one.body.faces.iter().enumerate() {
        let bound = if face.surface.is_freeform() {
            surface_floor(&face.surface, normal, enough, 0.01 * linear).ok_or(
                CutError::Section {
                    body: one.index,
                    what: "a freeform face whose least along the normal cannot be bounded",
                },
            )?
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
) -> Result<[f64; 6], &'static str> {
    let face = body
        .faces
        .get(face)
        .ok_or("a face that is not in the body")?;
    let Surface::Plane(plane) = &face.surface else {
        return Err("a face in the plane that is not flat");
    };
    // The loops run counter-clockwise about the surface's normal; the
    // face's outward normal is that normal, or the other way round when
    // the face is reversed. The cap of what is below faces along the normal.
    let normal = plane.frame().z();
    let facing = normal.dot(side.normal);
    let outward = if face.reversed { -facing } else { facing };
    if outward.is_nan() || outward <= 0.5 {
        return Err("a face in the plane that does not face away from what is below");
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
            let along = edge_sums(&edge.curve, edge.range, side, (o, size), rule, budget)?;
            let sign = if coedge.reversed { -1.0 } else { 1.0 };
            for (sum, part) in sums.iter_mut().zip(along) {
                *sum += sign * orientation * part;
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

/// The six line integrals of [`cap_sums`] along `curve` from `range.0` to
/// `range.1`.
fn edge_sums(
    curve: &Curve3,
    (t0, t1): (f64, f64),
    side: &CutPlane,
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
        return Err("an edge with more knot spans than a section takes");
    }
    let across = side.y_axis();
    let integrand = |t: f64| -> ([f64; 6], f64) {
        let (p, d) = curve.derivatives(t);
        let r = p - o;
        let (a, b) = (r.dot(side.x_axis), r.dot(across));
        let (da, db) = (d.dot(side.x_axis), d.dot(across));
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
            .ok_or("its edges take more panels than a section may")?;
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
            return Err("an edge along which the section's integrals do not settle");
        }
        let mid = 0.5 * (a + b);
        if !(mid > a && mid < b) {
            return Err("an edge along which the section's integrals do not settle");
        }
        open.push((mid, b, halvings + 1));
        open.push((a, mid, halvings + 1));
    }
    if !total.iter().all(|x| x.is_finite()) {
        return Err("an edge whose integrals are not finite");
    }
    Ok(total)
}

/// The offset of a plane square to `side.normal` at which the volume of
/// `bodies` below it is `volume`, found as the module documentation says,
/// and what lies below the plane there. An offset the boolean refuses
/// (a face of a body lying in the plane where a Newton step landed) is
/// stepped around by halving the bracket instead; one refused where the
/// halving lands is the error.
pub fn level_for(
    bodies: &[&Body],
    side: &CutPlane,
    volume: f64,
    tolerance: &Tolerance,
) -> Result<Level, CutError> {
    if !volume.is_finite() {
        return Err(CutError::NotFinite("the volume"));
    }
    let prepared = prepare(bodies, side.normal)?;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut most = 0.0;
    for one in &prepared {
        lo = lo.min(one.lo);
        hi = hi.max(one.hi);
        most += one.whole()?.0;
    }
    search((lo, hi), most, volume, |offset| {
        below_in(&prepared, side, offset, tolerance)
    })
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
    let mut halving_now = false;
    for step in 1..=MAX_STEPS {
        let found = match measure(offset) {
            Ok(found) => found,
            Err(error @ CutError::Kernel { .. }) => {
                let halving = 0.5 * (lo + hi);
                if !halving_now && halving > lo && halving < hi && halving != offset {
                    offset = halving;
                    halving_now = true;
                    continue;
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        };
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
        halving_now = !use_newton;
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
