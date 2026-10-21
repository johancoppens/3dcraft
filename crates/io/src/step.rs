//! Bodies as a STEP file: the exact B-rep, with units, uncertainty and
//! names, in the entity set of the "automotive design" application
//! protocol (ISO 10303-214) that every mainstream CAD system reads.
//!
//! The exchange structure (Part 21) is `craft3d-step`'s; this module knows
//! which entities to write and what their attributes mean. The entity
//! definitions are those of ISO 10303-42 (clause 4, the geometry schema;
//! clause 5, the topology schema; clause 6, the geometric model schema),
//! and of -41 and -43 (products, representations, contexts, units). The
//! standard's text was not at hand when this was written: each convention
//! below names the schema and the entity whose definition it comes from
//! (the entity definitions are subclauses 4.4, 5.4 and 6.4; their numbers
//! within are not given, rather than given wrong), says how sure the
//! reading is, and was checked against what FreeCAD (an independent
//! kernel) makes of the files: see `cargo xtask freecad` and
//! docs/testing.md, oracle 8, which also says which conventions FreeCAD
//! can tell apart and which it repairs silently.
//!
//! # Mapping
//!
//! | 3DCraft | STEP |
//! |---|---|
//! | body (solid) | one `PRODUCT` with an `ADVANCED_BREP_SHAPE_REPRESENTATION` |
//! | outer shell of a lump | `MANIFOLD_SOLID_BREP`, or `BREP_WITH_VOIDS` when it has cavities |
//! | shell of a cavity | `ORIENTED_CLOSED_SHELL` (`.F.`) of a `CLOSED_SHELL` |
//! | body (sheet) | `MANIFOLD_SURFACE_SHAPE_REPRESENTATION` of a `SHELL_BASED_SURFACE_MODEL` of `OPEN_SHELL`s |
//! | face | `ADVANCED_FACE` with `FACE_OUTER_BOUND` / `FACE_BOUND` |
//! | loop | `EDGE_LOOP` of `ORIENTED_EDGE`s |
//! | edge | `EDGE_CURVE` |
//! | vertex | `VERTEX_POINT` on a `CARTESIAN_POINT` |
//! | plane, cylinder, cone, sphere, torus | `PLANE`, `CYLINDRICAL_SURFACE`, `CONICAL_SURFACE`, `SPHERICAL_SURFACE`, `TOROIDAL_SURFACE` on an `AXIS2_PLACEMENT_3D` |
//! | Bézier patch, NURBS surface | `B_SPLINE_SURFACE_WITH_KNOTS`; rational: the complex instance with `RATIONAL_B_SPLINE_SURFACE` |
//! | line, circle, ellipse | `LINE`, `CIRCLE`, `ELLIPSE` |
//! | NURBS curve | `B_SPLINE_CURVE_WITH_KNOTS`; rational: the complex instance with `RATIONAL_B_SPLINE_CURVE` |
//! | colour of a body, of a face | `STYLED_ITEM` of the solid (or the shell model), of the `ADVANCED_FACE`, with `PRESENTATION_STYLE_ASSIGNMENT` → `SURFACE_STYLE_USAGE` (`.BOTH.`) → `SURFACE_SIDE_STYLE` → `SURFACE_STYLE_FILL_AREA` → `FILL_AREA_STYLE` → `FILL_AREA_STYLE_COLOUR` → `COLOUR_RGB` (0 to 1), all items of a body in one `MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION` in the body's context (the AP214/AP242 presentation resources, as other modellers write them) |
//!
//! # Conventions
//!
//! Each is what Part 42 says as far as remembered, with how sure that is.
//!
//! - **Placement** (geometry schema, `axis2_placement_3d`): `axis` is the local z,
//!   `ref_direction` the local x, y completes a right-handed frame. Both
//!   are always written. *Sure.*
//! - **Surface parametrisation and normal** (geometry schema, `plane`,
//!   `cylindrical_surface`, `conical_surface`, `spherical_surface`,
//!   `toroidal_surface`). Part 42 gives each elementary
//!   surface a parametrisation in its placement (`cylindrical_surface`:
//!   `C + R(cos u x + sin u y) + v z`; `conical_surface`:
//!   `C + (R + v tan α)(cos u x + sin u y) + v z`; `spherical_surface`:
//!   `C + R cos v (cos u x + sin u y) + R sin v z`; `toroidal_surface`:
//!   `C + (R + r cos v)(cos u x + sin u y) + r sin v z`; `plane`:
//!   `C + u x + v y`), and the surface normal is `∂u × ∂v`: away from the
//!   axis or centre, and the placement's z for a plane. These are the
//!   parametrisations of `craft3d-geom`, so frames and radii go over
//!   unchanged. *Sure*, and FreeCAD's volumes would have the wrong sign
//!   or value otherwise.
//! - **Cone** (`conical_surface`): `radius` is the radius in the plane of
//!   the placement, `semi_angle` the angle between axis and surface, and
//!   the radius grows along +z. The standard wants a positive semi-angle
//!   (*fairly sure*; a where-rule asks `radius ≥ 0`), so a cone that
//!   narrows along its frame's z (a negative half-angle here) is written
//!   on the frame seen from the other side (same x, y and z negated): the
//!   same surface with the same normal. `radius` may be zero: the
//!   placement is then at the apex.
//! - **Torus**: `major_radius` then `minor_radius`. *Sure.*
//! - **Angles** (the cone's semi-angle) are in the plane angle unit of the
//!   context: radians or degrees, as the options say.
//! - **Curves** (geometry schema, `line`, `circle`, `ellipse`). `line`: `pnt + t · dir` with a `VECTOR` of magnitude 1,
//!   so the parameter is the distance as it is here. `circle` and
//!   `ellipse`: the angle from the placement's x, counter-clockwise about
//!   its z; `semi_axis_1` is along x. Readers built on a kernel whose
//!   ellipse wants the major axis first exist, so an ellipse with the
//!   longer semi-axis along y is written on a placement turned a quarter
//!   about z: the same curve in the same direction. *Sure* of the
//!   standard; the turn is a courtesy.
//! - **B-spline curve knots** (`b_spline_curve_with_knots`): written on
//!   the interval from 0 to 1, whatever the curve's own knots are (the
//!   kernel's intersection curves have knots that are lengths, so they
//!   grow with the model). A B-spline is the same point set after an
//!   affine change of its knots, and no parameter value of a curve is
//!   written anywhere else in the file, so nothing else changes. *Sure*
//!   of the standard. The reason is a reader: FreeCAD (OpenCascade) calls
//!   a wire "self-intersecting" on a cylinder when a closed spline edge
//!   has knots beyond some 1e4 and the model is 1e3 or larger, and valid
//!   with the same control points on knots from 0 to 1 (measured
//!   2026-10-14: 58 of 312 sleeves with an oil hole invalid before, none
//!   after; supplying parameter-space curves changes nothing, the reader
//!   makes its own). The knots of a B-spline *surface* are written as
//!   they are: scaling them changed no verdict.
//! - **Edge** (topology schema, `edge_curve`): `same_sense` says whether the edge
//!   runs from its start to its end vertex in the direction of increasing
//!   curve parameter. Edges here always do: `.T.`. The edge is the part
//!   of the curve between the vertices; no parameter values are written,
//!   so a closed curve used by an edge with one vertex is the whole curve.
//!   *Sure.*
//! - **Oriented edge** (topology schema, `oriented_edge`): `orientation` `.T.` when the
//!   loop uses the edge from start to end: the opposite of
//!   `Coedge::reversed`. *Sure.*
//! - **Face** (topology schema, `face_surface` and its subtype
//!   `advanced_face`): `same_sense` `.T.` when
//!   the face's normal is the surface's: the opposite of
//!   `Face::reversed`. *Sure.*
//! - **Bounds** (topology schema, `face_bound` and `face`): seen from the side the face normal points
//!   to, a loop runs with the face on its left: outer boundaries
//!   counter-clockwise, holes clockwise. That is the convention of
//!   `craft3d-topo`, so `orientation` is `.T.`. *Sure.*
//! - **Outer bound** (topology schema, `face_outer_bound`): at most one bound of a face may
//!   be of this type, and none need be. It is written for a face with one
//!   loop; for a planar face with several, for the loop that runs
//!   counter-clockwise (exactly one must); on other surfaces for the one
//!   loop that does not go around the surface and has the face inside it,
//!   when no loop goes around. A cylinder wall between two circles has no
//!   outer bound, and gets none. The outer bound comes first in the list.
//!   *Fairly sure* that readers accept a face without one; FreeCAD does.
//! - **Faces without loops** (a whole sphere, a whole torus): a face needs
//!   at least one bound, and the standard gives `vertex_loop` for this
//!   (topology schema: a loop of a single vertex, with the apex of a cone
//!   and a whole sphere as what it is for, as remembered). The vertex is a point of the surface that the body
//!   does not have: on a sphere its north pole (`v = π/2`), on a torus the
//!   point `(u, v) = (0, 0)`. *Fairly sure* for the sphere; for the torus
//!   the standard is silent as far as remembered and this is by analogy.
//! - **Apex of a cone inside a face** (a whole cone): the same
//!   `vertex_loop`, at the apex, as a second bound next to the base
//!   circle. A face whose boundary passes through the apex (a slice of a
//!   cone) has the apex as a vertex of its loop and needs nothing. The
//!   poles of a sphere are not singular points of the surface, only of
//!   its parametrisation, and get nothing either. *Fairly sure.*
//! - **Seams.** Faces on closed surfaces are written as they are here:
//!   without a seam edge. Part 42 does not ask for one. *Sure* of the
//!   standard; that readers cope is what the external check is for.
//! - **Shells and solids** (topology schema, `closed_shell` and
//!   `oriented_closed_shell`; geometric model schema,
//!   `manifold_solid_brep` and `brep_with_voids`): a closed shell's normal points away from the
//!   finite region it encloses. The faces of a cavity here point away
//!   from the material, into the cavity; so a cavity's `CLOSED_SHELL` is
//!   written with every face turned over (`same_sense` and the bounds'
//!   `orientation` negated) and referred to through an
//!   `ORIENTED_CLOSED_SHELL` with orientation `.F.`, which turns it back.
//!   Which shell is a cavity is decided by the sign of the volume it
//!   encloses; which lump a cavity belongs to, when a body has several,
//!   by a point of it being inside the lump's outer shell (the innermost
//!   such lump). *Fairly sure*; it is what other systems' files look
//!   like.
//! - **Units and uncertainty**: lengths are in the unit of the options
//!   (SI with a prefix, or inch and foot as `CONVERSION_BASED_UNIT`s of the
//!   millimetre), and the linear tolerance is the context's
//!   `UNCERTAINTY_MEASURE_WITH_UNIT` (`distance_accuracy_value`).
//! - **Schema**: `AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }` (AP214,
//!   docs/decisions.md D-open-8), with `APPLICATION_PROTOCOL_DEFINITION`
//!   `'international standard'`, `'automotive_design'`, 2000.
//!
//! # Refused
//!
//! With an error that names the body and the face: a body that fails
//! `craft3d_topo::check`; a face without loops on a surface that is not a
//! whole sphere or torus; a planar face without exactly one
//! counter-clockwise loop; a solid whose shells cannot be told apart
//! (cavity or lump) or with a cavity outside every lump; geometry that is
//! not finite.

use crate::IoError;
use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_meshio::Unit;
use craft3d_step::{Header, Param, Record, StepFile};
use craft3d_topo::{Body, BodyKind, Coedge, Face, FaceId, LoopId, check};
use std::collections::BTreeMap;
use std::f64::consts::FRAC_PI_2;

/// The schema declared in `FILE_SCHEMA`.
pub const SCHEMA: &str = "AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }";

/// The unit plane angles are written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AngleUnit {
    #[default]
    Radian,
    Degree,
}

/// A body to export, with the name it gets in the file and its colours.
#[derive(Clone, Copy, Debug)]
pub struct StepBody<'a> {
    pub name: &'a str,
    pub body: &'a Body,
    /// The body's colour (sRGB), written as a style of its solid.
    pub color: Option<[u8; 3]>,
    /// Empty, or per face of the body (by index): a colour of the face's
    /// own, written as a style of the face where it differs from the
    /// body's.
    pub face_colors: &'a [Option<[u8; 3]>],
}

impl<'a> StepBody<'a> {
    /// A body without colours.
    pub fn new(name: &'a str, body: &'a Body) -> Self {
        Self {
            name,
            body,
            color: None,
            face_colors: &[],
        }
    }
}

/// What a STEP file says besides its bodies.
#[derive(Clone, Debug, PartialEq)]
pub struct StepOptions {
    /// The unit the bodies' coordinates are in.
    pub length_unit: Unit,
    /// The unit angles are written in. Coordinates do not depend on it.
    pub angle_unit: AngleUnit,
    /// The modelling tolerance of the bodies: `linear` (in `length_unit`)
    /// becomes the file's uncertainty, and the bodies are checked with it.
    pub tolerance: Tolerance,
    /// `FILE_NAME.name`: by convention the name of the file.
    pub file_name: String,
    /// `FILE_NAME.time_stamp`, ISO 8601. Empty by default: the same
    /// bodies then give the same bytes whenever they are exported.
    pub time_stamp: String,
    pub author: String,
    pub organization: String,
}

impl Default for StepOptions {
    fn default() -> Self {
        Self {
            length_unit: Unit::Millimeter,
            angle_unit: AngleUnit::Radian,
            tolerance: Tolerance::default(),
            file_name: String::new(),
            time_stamp: String::new(),
            author: String::new(),
            organization: String::new(),
        }
    }
}

/// What went into a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct StepSummary {
    /// Products: one per body.
    pub bodies: usize,
    /// `MANIFOLD_SOLID_BREP`s, `BREP_WITH_VOIDS` included.
    pub solids: usize,
    /// Cavities: the `ORIENTED_CLOSED_SHELL`s.
    pub voids: usize,
    /// `CLOSED_SHELL`s and `OPEN_SHELL`s.
    pub shells: usize,
    pub faces: usize,
    pub edges: usize,
    /// `VERTEX_POINT`s, those of vertex loops included.
    pub vertices: usize,
    /// `VERTEX_LOOP`s: faces without loops, and cone apexes inside a face.
    pub vertex_loops: usize,
    /// Entity instances in the data section.
    pub instances: usize,
}

fn unsupported(body: &str, what: String) -> IoError {
    IoError::Unsupported {
        body: body.to_string(),
        what,
    }
}

/// `-0.0` as `0.0`: the same number, and a file without `-0.`.
fn tidy(x: f64) -> f64 {
    x + 0.0
}

fn text(s: &str) -> Param {
    Param::string(s)
}

fn empty() -> Param {
    Param::String(String::new())
}

/// The distinct knots of a knot vector with how often each occurs.
fn knot_multiplicities(knots: &[f64]) -> (Vec<i64>, Vec<f64>) {
    let mut multiplicities: Vec<i64> = Vec::new();
    let mut distinct: Vec<f64> = Vec::new();
    for &knot in knots {
        match (distinct.last(), multiplicities.last_mut()) {
            (Some(&last), Some(count)) if last == knot => *count += 1,
            _ => {
                distinct.push(knot);
                multiplicities.push(1);
            }
        }
    }
    (multiplicities, distinct)
}

/// The knots of a curve on the interval from 0 to 1: the same curve, with
/// its parameter running over a range that does not depend on the size of
/// the model. Knots that cannot be mapped (no span, not finite, or two
/// distinct knots that would fall together) are returned as they are.
fn unit_interval(knots: Vec<f64>) -> Vec<f64> {
    let (Some(&first), Some(&last)) = (knots.first(), knots.last()) else {
        return knots;
    };
    let span = last - first;
    if !(span.is_finite() && span > 0.0) {
        return knots;
    }
    let mapped: Vec<f64> = knots.iter().map(|&knot| (knot - first) / span).collect();
    let increasing = mapped.iter().zip(mapped.iter().skip(1)).all(|(a, b)| a < b);
    if increasing && mapped.iter().all(|knot| knot.is_finite()) {
        mapped
    } else {
        knots
    }
}

fn integers(values: &[i64]) -> Param {
    Param::List(values.iter().map(|&v| Param::Integer(v)).collect())
}

fn is_rational(weights: &[f64]) -> bool {
    weights
        .first()
        .is_some_and(|&first| weights.iter().any(|&w| w != first))
}

/// Points along a loop, in the loop's direction: `per_edge` per coedge,
/// from its start (included) towards its end (not included).
fn loop_samples(body: &Body, loop_: LoopId, per_edge: u32) -> Option<Vec<Vec3>> {
    let mut out = Vec::new();
    for coedge in &body.loop_(loop_)?.coedges {
        let edge = body.edge(coedge.edge)?;
        let (t0, t1) = edge.range;
        for k in 0..per_edge {
            let s = f64::from(k) / f64::from(per_edge);
            let t = if coedge.reversed {
                t1 + (t0 - t1) * s
            } else {
                t0 + (t1 - t0) * s
            };
            out.push(edge.curve.eval(t));
        }
    }
    Some(out)
}

/// Twice the signed area of a closed polygon.
fn shoelace(points: &[(f64, f64)]) -> f64 {
    let mut sum = 0.0;
    for (i, &(x0, y0)) in points.iter().enumerate() {
        let Some(&(x1, y1)) = points.get(i + 1).or(points.first()) else {
            break;
        };
        sum += x0 * y1 - x1 * y0;
    }
    sum
}

/// A loop in the parameter plane of a surface that closes on itself: how
/// often it goes around in u and in v, and twice its signed area there
/// (which means something only when it goes around in neither).
struct ParameterLoop {
    wind_u: i64,
    wind_v: i64,
    area: f64,
}

fn parameter_loop(surface: &Surface, points: &[Vec3]) -> Option<ParameterLoop> {
    let (period_u, period_v) = (surface.period_u(), surface.period_v());
    // The representative of `raw` nearest to `previous`.
    let near = |raw: f64, previous: f64, period: Option<f64>| match period {
        Some(p) => {
            let mut delta = (raw - previous).rem_euclid(p);
            if delta > p / 2.0 {
                delta -= p;
            }
            previous + delta
        }
        None => raw,
    };
    let mut unrolled: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    for &point in points {
        let (u, v) = surface.project(point).ok()?;
        let (u, v) = match unrolled.last() {
            Some(&(pu, pv)) => (near(u, pu, period_u), near(v, pv, period_v)),
            None => (u, v),
        };
        if !(u.is_finite() && v.is_finite()) {
            return None;
        }
        unrolled.push((u, v));
    }
    let (&(u0, v0), &(u_last, v_last)) = (unrolled.first()?, unrolled.last()?);
    // Where the first point is when arrived at from the last.
    let (u_end, v_end) = (near(u0, u_last, period_u), near(v0, v_last, period_v));
    let turns = |from: f64, to: f64, period: Option<f64>| -> Option<i64> {
        let Some(p) = period else { return Some(0) };
        let turns = ((to - from) / p).round();
        // A loop goes around a handful of times at most.
        (turns.abs() <= 1e6).then_some(turns as i64)
    };
    Some(ParameterLoop {
        wind_u: turns(u0, u_end, period_u)?,
        wind_v: turns(v0, v_end, period_v)?,
        area: shoelace(&unrolled),
    })
}

/// How a face is bounded in the file.
struct Bounds {
    /// The loop written as `FACE_OUTER_BOUND`, if any.
    outer: Option<LoopId>,
    /// A point for a `VERTEX_LOOP`, if the face needs one.
    vertex_loop: Option<Vec3>,
}

/// Samples per edge when a loop's sense or winding is measured.
const SAMPLES: u32 = 16;

fn bounds(
    body: &Body,
    face_id: FaceId,
    face: &Face,
    tolerance: &Tolerance,
    name: &str,
) -> Result<Bounds, IoError> {
    let index = face_id.0;
    let missing = || {
        unsupported(
            name,
            format!("face {index} refers to a loop or edge that does not exist"),
        )
    };
    if face.loops.is_empty() {
        let point = match &face.surface {
            Surface::Sphere(_) => face.surface.eval(0.0, FRAC_PI_2),
            Surface::Torus(_) => face.surface.eval(0.0, 0.0),
            other => {
                return Err(unsupported(
                    name,
                    format!(
                        "face {index} has no boundary on a {}: only a whole sphere or torus can be a face without loops",
                        other.kind()
                    ),
                ));
            }
        };
        return Ok(Bounds {
            outer: None,
            vertex_loop: Some(point),
        });
    }
    let mut vertex_loop = None;
    if let Surface::Cone(cone) = &face.surface {
        let apex = cone.frame().point(0.0, 0.0, cone.apex_v());
        let mut on_boundary = false;
        let mut around = 0_i64;
        for &loop_ in &face.loops {
            for coedge in &body.loop_(loop_).ok_or_else(missing)?.coedges {
                let (start, _) = body.coedge_ends(*coedge).ok_or_else(missing)?;
                let point = body.vertex(start).ok_or_else(missing)?.point;
                if point.distance(apex) <= 4.0 * tolerance.linear {
                    on_boundary = true;
                }
            }
        }
        if !on_boundary {
            for &loop_ in &face.loops {
                let points = loop_samples(body, loop_, SAMPLES).ok_or_else(missing)?;
                let Some(found) = parameter_loop(&face.surface, &points) else {
                    return Err(unsupported(
                        name,
                        format!(
                            "face {index}: a loop on the cone could not be followed around its axis"
                        ),
                    ));
                };
                around += found.wind_u;
            }
            // A cone ends at its apex only: a boundary that goes around the
            // axis once in total leaves the apex inside the face.
            if around != 0 {
                vertex_loop = Some(apex);
            }
        }
    }
    let outer = match (&face.surface, face.loops.as_slice()) {
        (_, [only]) => Some(*only),
        (Surface::Plane(plane), loops) => {
            let mut outer = Vec::new();
            for &loop_ in loops {
                let points = loop_samples(body, loop_, SAMPLES).ok_or_else(missing)?;
                let flat: Vec<(f64, f64)> = points
                    .iter()
                    .map(|&p| {
                        let (x, y, _) = plane.frame().local(p);
                        (x, y)
                    })
                    .collect();
                let area = shoelace(&flat);
                if (area > 0.0) != face.reversed && area != 0.0 {
                    outer.push(loop_);
                }
            }
            match outer.as_slice() {
                [one] => Some(*one),
                _ => {
                    return Err(unsupported(
                        name,
                        format!(
                            "planar face {index} has {} counter-clockwise loops among its {}; it must have exactly one outer boundary",
                            outer.len(),
                            loops.len()
                        ),
                    ));
                }
            }
        }
        (surface, loops) => {
            let mut candidates = Vec::new();
            let mut winds = false;
            for &loop_ in loops {
                let points = loop_samples(body, loop_, SAMPLES).ok_or_else(missing)?;
                match parameter_loop(surface, &points) {
                    Some(found) if found.wind_u == 0 && found.wind_v == 0 => {
                        if (found.area > 0.0) != face.reversed && found.area != 0.0 {
                            candidates.push(loop_);
                        }
                    }
                    // Around the surface, or not measurable (through a
                    // pole): then no loop is called the outer one.
                    _ => winds = true,
                }
            }
            match (winds, candidates.as_slice()) {
                (false, [one]) => Some(*one),
                _ => None,
            }
        }
    };
    Ok(Bounds { outer, vertex_loop })
}

/// The shells of a solid body sorted into lumps: `(outer, cavities)`, by
/// shell index, in the order of the outer shells (`craft3d_kernel::lumps`:
/// which shells lie inside which decides; the sign of the volume each
/// shell encloses, where it can be told from zero, must agree, and when
/// every sign can be read and one shell faces out it is the one part
/// without a ray).
fn lumps(
    body: &Body,
    tolerance: &Tolerance,
    name: &str,
) -> Result<Vec<(usize, Vec<usize>)>, IoError> {
    if let [_] = body.shells.as_slice() {
        // One shell is the outside; a shell that is inside out would be a
        // solid of negative volume, which is not for the export to judge.
        return Ok(vec![(0, Vec::new())]);
    }
    let failed = |e: craft3d_kernel::KernelError| {
        unsupported(
            name,
            format!("cannot sort its shells into parts and cavities: {e}"),
        )
    };
    // The faces measured once, summed per shell.
    let volumes = craft3d_topo::shell_volumes(body);
    let outer = craft3d_kernel::lumps::outer_shells(body, &volumes, tolerance).map_err(failed)?;
    // Every shell's volume has a sign to read, and one faces outward.
    if outer.iter().all(Option::is_some)
        && let [only] = outer
            .iter()
            .enumerate()
            .filter(|(_, o)| **o == Some(true))
            .map(|(s, _)| s)
            .collect::<Vec<_>>()
            .as_slice()
    {
        // One part: every other shell is one of its cavities, without a
        // ray (what lies where is `document.check`'s to judge).
        let cavities = (0..outer.len()).filter(|s| s != only).collect();
        return Ok(vec![(*only, cavities)]);
    }
    let found = craft3d_kernel::lumps::lumps_measured(body, tolerance, &volumes).map_err(failed)?;
    Ok(found
        .into_iter()
        .map(|lump| {
            (
                lump.outer.0 as usize,
                lump.cavities.iter().map(|c| c.0 as usize).collect(),
            )
        })
        .collect())
}

struct Writer<'a> {
    file: StepFile,
    options: &'a StepOptions,
    summary: StepSummary,
}

/// What of one body has been written: ids by index.
#[derive(Default)]
struct Written {
    vertices: BTreeMap<u32, u64>,
    edges: BTreeMap<u32, u64>,
    /// The `ADVANCED_FACE` of each face written, by face index.
    faces: BTreeMap<u32, u64>,
}

impl Writer<'_> {
    fn add(&mut self, keyword: &str, params: Vec<Param>) -> u64 {
        self.file.add(keyword, params)
    }

    fn point(&mut self, p: Vec3) -> u64 {
        self.add(
            "CARTESIAN_POINT",
            vec![empty(), Param::reals([tidy(p.x), tidy(p.y), tidy(p.z)])],
        )
    }

    fn direction(&mut self, d: Vec3) -> u64 {
        self.add(
            "DIRECTION",
            vec![empty(), Param::reals([tidy(d.x), tidy(d.y), tidy(d.z)])],
        )
    }

    fn placement(&mut self, origin: Vec3, z: Vec3, x: Vec3) -> u64 {
        let location = self.point(origin);
        let axis = self.direction(z);
        let reference = self.direction(x);
        self.add(
            "AXIS2_PLACEMENT_3D",
            vec![
                empty(),
                Param::Ref(location),
                Param::Ref(axis),
                Param::Ref(reference),
            ],
        )
    }

    fn frame(&mut self, frame: &Frame) -> u64 {
        self.placement(frame.origin(), frame.z(), frame.x())
    }

    fn angle(&self, radians: f64) -> f64 {
        match self.options.angle_unit {
            AngleUnit::Radian => radians,
            AngleUnit::Degree => radians.to_degrees(),
        }
    }

    fn nurbs_curve(&mut self, curve: &NurbsCurve) -> u64 {
        let points: Vec<u64> = curve
            .control_points()
            .iter()
            .map(|&p| self.point(p))
            .collect();
        let (multiplicities, knots) = knot_multiplicities(curve.knots());
        // No parameter of a curve is written anywhere else (an edge is the
        // part of its curve between its vertices), so the curve's own
        // range is free to choose: see "B-spline curve knots" above.
        let knots = unit_interval(knots);
        let degree = Param::Integer(i64::try_from(curve.degree()).unwrap_or(i64::MAX));
        let unknown = || Param::enumeration("U");
        let unspecified = || Param::enumeration("UNSPECIFIED");
        if is_rational(curve.weights()) {
            self.file.add_complex(vec![
                Record::new("BOUNDED_CURVE", vec![]),
                Record::new(
                    "B_SPLINE_CURVE",
                    vec![
                        degree,
                        Param::refs(points),
                        unspecified(),
                        unknown(),
                        unknown(),
                    ],
                ),
                Record::new(
                    "B_SPLINE_CURVE_WITH_KNOTS",
                    vec![
                        integers(&multiplicities),
                        Param::reals(knots),
                        unspecified(),
                    ],
                ),
                Record::new("CURVE", vec![]),
                Record::new("GEOMETRIC_REPRESENTATION_ITEM", vec![]),
                Record::new(
                    "RATIONAL_B_SPLINE_CURVE",
                    vec![Param::reals(curve.weights().iter().copied())],
                ),
                Record::new("REPRESENTATION_ITEM", vec![empty()]),
            ])
        } else {
            self.add(
                "B_SPLINE_CURVE_WITH_KNOTS",
                vec![
                    empty(),
                    degree,
                    Param::refs(points),
                    unspecified(),
                    unknown(),
                    unknown(),
                    integers(&multiplicities),
                    Param::reals(knots),
                    unspecified(),
                ],
            )
        }
    }

    fn curve(&mut self, curve: &Curve3) -> u64 {
        match curve {
            Curve3::Line(line) => {
                let point = self.point(line.origin());
                let direction = self.direction(line.direction());
                let vector = self.add(
                    "VECTOR",
                    vec![empty(), Param::Ref(direction), Param::Real(1.0)],
                );
                self.add("LINE", vec![empty(), Param::Ref(point), Param::Ref(vector)])
            }
            Curve3::Circle(circle) => {
                let placement = self.frame(circle.frame());
                self.add(
                    "CIRCLE",
                    vec![empty(), Param::Ref(placement), Param::Real(circle.radius())],
                )
            }
            Curve3::Ellipse(ellipse) => {
                let (a, b) = ellipse.semi_axes();
                let frame = ellipse.frame();
                // The longer semi-axis first: turned a quarter about z,
                // x' = y and y' = -x, the same curve run the same way.
                let (placement, first, second) = if a >= b {
                    (self.frame(frame), a, b)
                } else {
                    (self.placement(frame.origin(), frame.z(), frame.y()), b, a)
                };
                self.add(
                    "ELLIPSE",
                    vec![
                        empty(),
                        Param::Ref(placement),
                        Param::Real(first),
                        Param::Real(second),
                    ],
                )
            }
            Curve3::Nurbs(nurbs) => self.nurbs_curve(nurbs),
        }
    }

    fn nurbs_surface(&mut self, surface: &NurbsSurface) -> u64 {
        let (_, count_v) = surface.counts();
        let ids: Vec<u64> = surface
            .control_points()
            .iter()
            .map(|&p| self.point(p))
            .collect();
        // One list per step in u, each along v.
        let rows = |values: Vec<Param>| {
            Param::List(
                values
                    .chunks(count_v.max(1))
                    .map(|row| Param::List(row.to_vec()))
                    .collect(),
            )
        };
        let net = rows(ids.into_iter().map(Param::Ref).collect());
        let (degree_u, degree_v) = surface.degrees();
        let degree = |d: usize| Param::Integer(i64::try_from(d).unwrap_or(i64::MAX));
        let (mult_u, knots_u) = knot_multiplicities(surface.knots_u());
        let (mult_v, knots_v) = knot_multiplicities(surface.knots_v());
        let unknown = || Param::enumeration("U");
        let unspecified = || Param::enumeration("UNSPECIFIED");
        if is_rational(surface.weights()) {
            let weights = rows(surface.weights().iter().map(|&w| Param::Real(w)).collect());
            self.file.add_complex(vec![
                Record::new("BOUNDED_SURFACE", vec![]),
                Record::new(
                    "B_SPLINE_SURFACE",
                    vec![
                        degree(degree_u),
                        degree(degree_v),
                        net,
                        unspecified(),
                        unknown(),
                        unknown(),
                        unknown(),
                    ],
                ),
                Record::new(
                    "B_SPLINE_SURFACE_WITH_KNOTS",
                    vec![
                        integers(&mult_u),
                        integers(&mult_v),
                        Param::reals(knots_u),
                        Param::reals(knots_v),
                        unspecified(),
                    ],
                ),
                Record::new("GEOMETRIC_REPRESENTATION_ITEM", vec![]),
                Record::new("RATIONAL_B_SPLINE_SURFACE", vec![weights]),
                Record::new("REPRESENTATION_ITEM", vec![empty()]),
                Record::new("SURFACE", vec![]),
            ])
        } else {
            self.add(
                "B_SPLINE_SURFACE_WITH_KNOTS",
                vec![
                    empty(),
                    degree(degree_u),
                    degree(degree_v),
                    net,
                    unspecified(),
                    unknown(),
                    unknown(),
                    unknown(),
                    integers(&mult_u),
                    integers(&mult_v),
                    Param::reals(knots_u),
                    Param::reals(knots_v),
                    unspecified(),
                ],
            )
        }
    }

    fn surface(&mut self, surface: &Surface, name: &str) -> Result<u64, IoError> {
        Ok(match surface {
            Surface::Plane(plane) => {
                let placement = self.frame(plane.frame());
                self.add("PLANE", vec![empty(), Param::Ref(placement)])
            }
            Surface::Cylinder(cylinder) => {
                let placement = self.frame(cylinder.frame());
                self.add(
                    "CYLINDRICAL_SURFACE",
                    vec![
                        empty(),
                        Param::Ref(placement),
                        Param::Real(cylinder.radius()),
                    ],
                )
            }
            Surface::Cone(cone) => {
                let frame = cone.frame();
                // A positive semi-angle: a cone that narrows along z is
                // written on the frame seen from the other side.
                let placement = if cone.half_angle() > 0.0 {
                    self.frame(frame)
                } else {
                    self.placement(frame.origin(), -frame.z(), frame.x())
                };
                let semi_angle = self.angle(cone.half_angle().abs());
                self.add(
                    "CONICAL_SURFACE",
                    vec![
                        empty(),
                        Param::Ref(placement),
                        Param::Real(cone.radius()),
                        Param::Real(semi_angle),
                    ],
                )
            }
            Surface::Sphere(sphere) => {
                let placement = self.frame(sphere.frame());
                self.add(
                    "SPHERICAL_SURFACE",
                    vec![empty(), Param::Ref(placement), Param::Real(sphere.radius())],
                )
            }
            Surface::Torus(torus) => {
                let placement = self.frame(torus.frame());
                let (major, minor) = torus.radii();
                self.add(
                    "TOROIDAL_SURFACE",
                    vec![
                        empty(),
                        Param::Ref(placement),
                        Param::Real(major),
                        Param::Real(minor),
                    ],
                )
            }
            Surface::Bezier(patch) => {
                let nurbs = NurbsSurface::from_bezier(patch).map_err(|e| {
                    unsupported(name, format!("a Bézier patch as a B-spline surface: {e}"))
                })?;
                self.nurbs_surface(&nurbs)
            }
            Surface::Nurbs(nurbs) => self.nurbs_surface(nurbs),
        })
    }

    fn vertex_point(&mut self, point: Vec3) -> u64 {
        let point = self.point(point);
        self.summary.vertices += 1;
        self.add("VERTEX_POINT", vec![empty(), Param::Ref(point)])
    }

    fn vertex(
        &mut self,
        body: &Body,
        index: u32,
        written: &mut Written,
        name: &str,
    ) -> Result<u64, IoError> {
        if let Some(&id) = written.vertices.get(&index) {
            return Ok(id);
        }
        let vertex = body
            .vertices
            .get(index as usize)
            .ok_or_else(|| unsupported(name, format!("vertex {index} does not exist")))?;
        let id = self.vertex_point(vertex.point);
        written.vertices.insert(index, id);
        Ok(id)
    }

    fn edge(
        &mut self,
        body: &Body,
        index: u32,
        written: &mut Written,
        name: &str,
    ) -> Result<u64, IoError> {
        if let Some(&id) = written.edges.get(&index) {
            return Ok(id);
        }
        let edge = body
            .edges
            .get(index as usize)
            .ok_or_else(|| unsupported(name, format!("edge {index} does not exist")))?;
        let start = self.vertex(body, edge.start.0, written, name)?;
        let end = self.vertex(body, edge.end.0, written, name)?;
        let curve = self.curve(&edge.curve);
        self.summary.edges += 1;
        // Edges here run from start to end with the curve's parameter.
        let id = self.add(
            "EDGE_CURVE",
            vec![
                empty(),
                Param::Ref(start),
                Param::Ref(end),
                Param::Ref(curve),
                Param::boolean(true),
            ],
        );
        written.edges.insert(index, id);
        Ok(id)
    }

    fn edge_loop(
        &mut self,
        body: &Body,
        loop_: LoopId,
        written: &mut Written,
        name: &str,
    ) -> Result<u64, IoError> {
        let coedges: &[Coedge] = &body
            .loop_(loop_)
            .ok_or_else(|| unsupported(name, format!("loop {} does not exist", loop_.0)))?
            .coedges;
        let mut oriented = Vec::with_capacity(coedges.len());
        for coedge in coedges {
            let edge = self.edge(body, coedge.edge.0, written, name)?;
            oriented.push(self.add(
                "ORIENTED_EDGE",
                vec![
                    empty(),
                    Param::Derived,
                    Param::Derived,
                    Param::Ref(edge),
                    Param::boolean(!coedge.reversed),
                ],
            ));
        }
        Ok(self.add("EDGE_LOOP", vec![empty(), Param::refs(oriented)]))
    }

    /// One face. `turned` writes it facing the other way (for the shell of
    /// a cavity): `same_sense` and the orientation of its bounds negated.
    fn face(
        &mut self,
        body: &Body,
        face_id: FaceId,
        turned: bool,
        written: &mut Written,
        name: &str,
    ) -> Result<u64, IoError> {
        let face = body
            .face(face_id)
            .ok_or_else(|| unsupported(name, format!("face {} does not exist", face_id.0)))?;
        let plan = bounds(body, face_id, face, &self.options.tolerance, name)?;
        // The outer bound first.
        let mut order: Vec<LoopId> = Vec::with_capacity(face.loops.len());
        order.extend(plan.outer);
        order.extend(
            face.loops
                .iter()
                .copied()
                .filter(|l| Some(*l) != plan.outer),
        );
        let mut bound_ids = Vec::new();
        for loop_ in order {
            let edge_loop = self.edge_loop(body, loop_, written, name)?;
            let keyword = if Some(loop_) == plan.outer {
                "FACE_OUTER_BOUND"
            } else {
                "FACE_BOUND"
            };
            bound_ids.push(self.add(
                keyword,
                vec![empty(), Param::Ref(edge_loop), Param::boolean(!turned)],
            ));
        }
        if let Some(point) = plan.vertex_loop {
            let vertex = self.vertex_point(point);
            let vertex_loop = self.add("VERTEX_LOOP", vec![empty(), Param::Ref(vertex)]);
            self.summary.vertex_loops += 1;
            bound_ids.push(self.add(
                "FACE_BOUND",
                vec![empty(), Param::Ref(vertex_loop), Param::boolean(true)],
            ));
        }
        let surface = self.surface(&face.surface, name)?;
        self.summary.faces += 1;
        Ok(self.add(
            "ADVANCED_FACE",
            vec![
                empty(),
                Param::refs(bound_ids),
                Param::Ref(surface),
                Param::boolean(face.reversed == turned),
            ],
        ))
    }

    /// The colours of a body: a `STYLED_ITEM` per solid (or shell model)
    /// in the body's colour and per face whose colour differs from it, in
    /// one presentation representation. One style per colour.
    fn styles(&mut self, item: &StepBody<'_>, solids: &[u64], written: &Written, context: u64) {
        let mut styles: BTreeMap<[u8; 3], u64> = BTreeMap::new();
        let mut style = |writer: &mut Self, color: [u8; 3]| -> u64 {
            if let Some(&id) = styles.get(&color) {
                return id;
            }
            let [r, g, b] = color.map(|c| Param::Real(f64::from(c) / 255.0));
            let rgb = writer.add("COLOUR_RGB", vec![empty(), r, g, b]);
            let fill = writer.add("FILL_AREA_STYLE_COLOUR", vec![empty(), Param::Ref(rgb)]);
            let area = writer.add("FILL_AREA_STYLE", vec![empty(), Param::refs([fill])]);
            let surface = writer.add("SURFACE_STYLE_FILL_AREA", vec![Param::Ref(area)]);
            let side = writer.add("SURFACE_SIDE_STYLE", vec![empty(), Param::refs([surface])]);
            let usage = writer.add(
                "SURFACE_STYLE_USAGE",
                vec![Param::enumeration("BOTH"), Param::Ref(side)],
            );
            let assignment =
                writer.add("PRESENTATION_STYLE_ASSIGNMENT", vec![Param::refs([usage])]);
            styles.insert(color, assignment);
            assignment
        };
        let mut styled = Vec::new();
        if let Some(color) = item.color {
            for &solid in solids {
                let assignment = style(self, color);
                styled.push(self.add(
                    "STYLED_ITEM",
                    vec![text("color"), Param::refs([assignment]), Param::Ref(solid)],
                ));
            }
        }
        for (&face, &id) in &written.faces {
            let own = item.face_colors.get(face as usize).copied().flatten();
            if let Some(color) = own.filter(|&c| Some(c) != item.color) {
                let assignment = style(self, color);
                styled.push(self.add(
                    "STYLED_ITEM",
                    vec![text("color"), Param::refs([assignment]), Param::Ref(id)],
                ));
            }
        }
        if !styled.is_empty() {
            self.add(
                "MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION",
                vec![empty(), Param::refs(styled), Param::Ref(context)],
            );
        }
    }

    fn shell(
        &mut self,
        body: &Body,
        shell: usize,
        keyword: &str,
        turned: bool,
        written: &mut Written,
        name: &str,
    ) -> Result<u64, IoError> {
        let faces = &body
            .shells
            .get(shell)
            .ok_or_else(|| unsupported(name, format!("shell {shell} does not exist")))?
            .faces;
        let mut ids = Vec::with_capacity(faces.len());
        for &face in faces {
            let id = self.face(body, face, turned, written, name)?;
            written.faces.insert(face.0, id);
            ids.push(id);
        }
        self.summary.shells += 1;
        Ok(self.add(keyword, vec![empty(), Param::refs(ids)]))
    }

    /// The length unit, the plane angle unit, the solid angle unit and the
    /// uncertainty, as one representation context.
    fn context(&mut self) -> u64 {
        let si = |prefix: Option<&str>, name: &str| {
            Record::new(
                "SI_UNIT",
                vec![
                    prefix.map_or(Param::Omitted, Param::enumeration),
                    Param::enumeration(name),
                ],
            )
        };
        let si_length = |writer: &mut Self, prefix: Option<&str>| {
            writer.file.add_complex(vec![
                Record::new("LENGTH_UNIT", vec![]),
                Record::new("NAMED_UNIT", vec![Param::Derived]),
                si(prefix, "METRE"),
            ])
        };
        let exponents = |writer: &mut Self, length: f64| {
            let mut values = vec![Param::Real(length)];
            values.extend(std::iter::repeat_n(Param::Real(0.0), 6));
            writer.add("DIMENSIONAL_EXPONENTS", values)
        };
        // An inch or a foot is so many millimetres.
        let converted = |writer: &mut Self, unit_name: &str, millimetres: f64| {
            let millimetre = si_length(writer, Some("MILLI"));
            let measure = writer.add(
                "LENGTH_MEASURE_WITH_UNIT",
                vec![
                    Param::typed("LENGTH_MEASURE", Param::Real(millimetres)),
                    Param::Ref(millimetre),
                ],
            );
            let dimensions = exponents(writer, 1.0);
            writer.file.add_complex(vec![
                Record::new(
                    "CONVERSION_BASED_UNIT",
                    vec![text(unit_name), Param::Ref(measure)],
                ),
                Record::new("LENGTH_UNIT", vec![]),
                Record::new("NAMED_UNIT", vec![Param::Ref(dimensions)]),
            ])
        };
        let length = match self.options.length_unit {
            Unit::Micron => si_length(self, Some("MICRO")),
            Unit::Millimeter => si_length(self, Some("MILLI")),
            Unit::Centimeter => si_length(self, Some("CENTI")),
            Unit::Meter => si_length(self, None),
            Unit::Inch => converted(self, "INCH", 25.4),
            Unit::Foot => converted(self, "FOOT", 304.8),
        };
        let radian = self.file.add_complex(vec![
            Record::new("NAMED_UNIT", vec![Param::Derived]),
            Record::new("PLANE_ANGLE_UNIT", vec![]),
            si(None, "RADIAN"),
        ]);
        let angle = match self.options.angle_unit {
            AngleUnit::Radian => radian,
            AngleUnit::Degree => {
                let measure = self.add(
                    "PLANE_ANGLE_MEASURE_WITH_UNIT",
                    vec![
                        Param::typed("PLANE_ANGLE_MEASURE", Param::Real(1.0_f64.to_radians())),
                        Param::Ref(radian),
                    ],
                );
                let dimensions = exponents(self, 0.0);
                self.file.add_complex(vec![
                    Record::new(
                        "CONVERSION_BASED_UNIT",
                        vec![text("DEGREE"), Param::Ref(measure)],
                    ),
                    Record::new("NAMED_UNIT", vec![Param::Ref(dimensions)]),
                    Record::new("PLANE_ANGLE_UNIT", vec![]),
                ])
            }
        };
        let solid_angle = self.file.add_complex(vec![
            Record::new("NAMED_UNIT", vec![Param::Derived]),
            si(None, "STERADIAN"),
            Record::new("SOLID_ANGLE_UNIT", vec![]),
        ]);
        let uncertainty = self.add(
            "UNCERTAINTY_MEASURE_WITH_UNIT",
            vec![
                Param::typed("LENGTH_MEASURE", Param::Real(self.options.tolerance.linear)),
                Param::Ref(length),
                text("distance_accuracy_value"),
                text("modelling tolerance"),
            ],
        );
        self.file.add_complex(vec![
            Record::new("GEOMETRIC_REPRESENTATION_CONTEXT", vec![Param::Integer(3)]),
            Record::new(
                "GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT",
                vec![Param::refs([uncertainty])],
            ),
            Record::new(
                "GLOBAL_UNIT_ASSIGNED_CONTEXT",
                vec![Param::refs([length, angle, solid_angle])],
            ),
            Record::new("REPRESENTATION_CONTEXT", vec![text("3D"), text("")]),
        ])
    }

    fn body(
        &mut self,
        item: &StepBody<'_>,
        name: &str,
        product_context: u64,
        definition_context: u64,
        context: u64,
    ) -> Result<(), IoError> {
        let body = item.body;
        let defects = check(body, &self.options.tolerance);
        if !defects.is_empty() {
            let total = defects.len();
            return Err(IoError::InvalidBody {
                body: name.to_string(),
                defects: defects.iter().take(8).map(ToString::to_string).collect(),
                total,
            });
        }
        let mut written = Written::default();
        let mut items = Vec::new();
        let representation = match body.kind {
            BodyKind::Solid => {
                for (outer, voids) in lumps(body, &self.options.tolerance, name)? {
                    let outer =
                        self.shell(body, outer, "CLOSED_SHELL", false, &mut written, name)?;
                    self.summary.solids += 1;
                    if voids.is_empty() {
                        items.push(
                            self.add("MANIFOLD_SOLID_BREP", vec![text(name), Param::Ref(outer)]),
                        );
                        continue;
                    }
                    let mut oriented = Vec::new();
                    for void in voids {
                        let shell =
                            self.shell(body, void, "CLOSED_SHELL", true, &mut written, name)?;
                        self.summary.voids += 1;
                        oriented.push(self.add(
                            "ORIENTED_CLOSED_SHELL",
                            vec![
                                empty(),
                                Param::Derived,
                                Param::Ref(shell),
                                Param::boolean(false),
                            ],
                        ));
                    }
                    items.push(self.add(
                        "BREP_WITH_VOIDS",
                        vec![text(name), Param::Ref(outer), Param::refs(oriented)],
                    ));
                }
                "ADVANCED_BREP_SHAPE_REPRESENTATION"
            }
            BodyKind::Sheet => {
                let mut shells = Vec::new();
                for shell in 0..body.shells.len() {
                    shells.push(self.shell(
                        body,
                        shell,
                        "OPEN_SHELL",
                        false,
                        &mut written,
                        name,
                    )?);
                }
                items.push(self.add(
                    "SHELL_BASED_SURFACE_MODEL",
                    vec![text(name), Param::refs(shells)],
                ));
                "MANIFOLD_SURFACE_SHAPE_REPRESENTATION"
            }
        };
        if items.is_empty() {
            return Err(unsupported(name, "the body has no shells".to_string()));
        }
        self.styles(item, &items, &written, context);
        let representation = self.add(
            representation,
            vec![text(name), Param::refs(items), Param::Ref(context)],
        );
        let product = self.add(
            "PRODUCT",
            vec![
                text(name),
                text(name),
                empty(),
                Param::refs([product_context]),
            ],
        );
        let formation = self.add(
            "PRODUCT_DEFINITION_FORMATION",
            vec![empty(), empty(), Param::Ref(product)],
        );
        let definition = self.add(
            "PRODUCT_DEFINITION",
            vec![
                text("design"),
                empty(),
                Param::Ref(formation),
                Param::Ref(definition_context),
            ],
        );
        let shape = self.add(
            "PRODUCT_DEFINITION_SHAPE",
            vec![empty(), empty(), Param::Ref(definition)],
        );
        self.add(
            "SHAPE_DEFINITION_REPRESENTATION",
            vec![Param::Ref(shape), Param::Ref(representation)],
        );
        self.add(
            "PRODUCT_RELATED_PRODUCT_CATEGORY",
            vec![text("part"), Param::Omitted, Param::refs([product])],
        );
        self.summary.bodies += 1;
        Ok(())
    }
}

/// The bodies as a STEP file model, with what went into it.
///
/// Every body is checked with the options' tolerance first; an invalid
/// body is an error, never a file. Bodies with the same name get a number
/// after it, and a body without a name is called `Body`.
pub fn step_model(
    bodies: &[StepBody<'_>],
    options: &StepOptions,
) -> Result<(StepFile, StepSummary), IoError> {
    if bodies.is_empty() {
        return Err(IoError::NoBodies);
    }
    let tolerance = options.tolerance;
    if Tolerance::new(tolerance.linear, tolerance.angular).is_none() {
        return Err(IoError::Unsupported {
            body: String::new(),
            what: "the tolerance must be positive and finite".to_string(),
        });
    }
    let list = |value: &str| {
        if value.is_empty() {
            Vec::new()
        } else {
            vec![value.to_string()]
        }
    };
    let header = Header {
        description: vec!["3DCraft model".to_string()],
        implementation_level: "2;1".to_string(),
        name: options.file_name.clone(),
        time_stamp: options.time_stamp.clone(),
        author: list(&options.author),
        organization: list(&options.organization),
        preprocessor_version: format!("craft3d-step {}", env!("CARGO_PKG_VERSION")),
        originating_system: "3DCraft".to_string(),
        authorization: String::new(),
        schemas: vec![SCHEMA.to_string()],
    };
    let mut writer = Writer {
        file: StepFile::new(&header),
        options,
        summary: StepSummary::default(),
    };
    let application = writer.add(
        "APPLICATION_CONTEXT",
        vec![text("core data for automotive mechanical design processes")],
    );
    writer.add(
        "APPLICATION_PROTOCOL_DEFINITION",
        vec![
            text("international standard"),
            text("automotive_design"),
            Param::Integer(2000),
            Param::Ref(application),
        ],
    );
    let product_context = writer.add(
        "PRODUCT_CONTEXT",
        vec![empty(), Param::Ref(application), text("mechanical")],
    );
    let definition_context = writer.add(
        "PRODUCT_DEFINITION_CONTEXT",
        vec![
            text("part definition"),
            Param::Ref(application),
            text("design"),
        ],
    );
    let context = writer.context();
    let mut used: BTreeMap<String, usize> = BTreeMap::new();
    for item in bodies {
        let base = if item.name.is_empty() {
            "Body"
        } else {
            item.name
        };
        let count = used.entry(base.to_string()).or_insert(0);
        *count += 1;
        let name = if *count == 1 {
            base.to_string()
        } else {
            format!("{base} ({count})")
        };
        writer.body(item, &name, product_context, definition_context, context)?;
    }
    writer.summary.instances = writer.file.data.len();
    Ok((writer.file, writer.summary))
}

/// The bodies as the bytes of a STEP file (ASCII).
pub fn export_step(
    bodies: &[StepBody<'_>],
    options: &StepOptions,
) -> Result<(Vec<u8>, StepSummary), IoError> {
    let (file, summary) = step_model(bodies, options)?;
    let text = craft3d_step::write(&file).map_err(|e| IoError::Unsupported {
        body: String::new(),
        what: e.to_string(),
    })?;
    Ok((text.into_bytes(), summary))
}

/// Whether a file extension (without the dot, any case) means STEP.
pub fn is_step_extension(extension: &str) -> bool {
    matches!(extension.to_ascii_lowercase().as_str(), "step" | "stp")
}

#[cfg(test)]
mod tests {
    use super::unit_interval;

    #[test]
    fn knots_go_to_the_unit_interval_or_stay_as_they_are() {
        assert_eq!(unit_interval(vec![2.0, 5.0, 12.0]), [0.0, 0.3, 1.0]);
        assert_eq!(unit_interval(vec![-4.0e7, 0.0, 4.0e7]), [0.0, 0.5, 1.0]);
        // On the interval already: bit for bit.
        let thirds = vec![0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];
        assert_eq!(unit_interval(thirds.clone()), thirds);
        // Nothing to map onto, or nothing finite: as they are.
        assert_eq!(unit_interval(vec![]), Vec::<f64>::new());
        assert_eq!(unit_interval(vec![3.0]), [3.0]);
        assert_eq!(unit_interval(vec![3.0, 3.0]), [3.0, 3.0]);
        assert_eq!(unit_interval(vec![5.0, 3.0]), [5.0, 3.0]);
        let endless = unit_interval(vec![0.0, 1.0, f64::INFINITY]);
        assert_eq!(endless.get(1), Some(&1.0));
        assert_eq!(
            unit_interval(vec![-f64::MAX, 0.0, f64::MAX]).first(),
            Some(&-f64::MAX)
        );
        let not_a_number = unit_interval(vec![0.0, f64::NAN, 1.0]);
        assert!(not_a_number.get(1).is_some_and(|k| k.is_nan()));
        // Two knots that would fall together stay apart.
        let close = vec![0.0, 5e-324, 1e300];
        assert_eq!(unit_interval(close.clone()), close);
    }
}
