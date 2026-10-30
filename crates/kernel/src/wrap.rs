//! Profiles wrapped onto a cylinder or a cone and given a thickness: the
//! solid an embossed (out) or engraved (in) text or outline is
//! (docs/design/text.md › *Wrapped onto a surface*).
//!
//! **The map.** A cylinder and a cone are developable, and their
//! development is known in closed form: the profile's plane is laid on the
//! surface without stretching. On a cylinder of radius `r` the sketch's
//! `(x, y)` is the point at the angle `x / r` round the axis from the
//! frame's x direction and the height `y` along it. On a cone of half
//! angle `α` whose circle at the frame's origin has the radius `r`, the
//! development is a sector about the apex's image: with `ρ₀ = r / sin|α|`
//! (the slant distance from the apex to that circle) and `w = ρ₀ + s y`
//! (`s` the sign of `α`), the point is at the angle `atan2(x, w) / sin|α|`
//! and the slant distance `hypot(x, w)` from the apex. Both are isometries
//! (lengths and areas are kept), so a glyph keeps its arc length and area.
//! Any other surface is refused: its development is not built (owed).
//!
//! **The solid.** Each boundary curve of a profile is mapped point by
//! point and fitted by a cubic B-spline through chord-length samples,
//! doubled until the curve is within a tenth of the tolerance of the
//! mapped points at every midpoint between samples. The fitted loops bound
//! a face on the surface itself (a sheet, one face per profile), and
//! [`crate::plate`] gives it the depth along the outward normal, out of
//! the body for a positive depth and into it for a negative one, with
//! square rims and a lap of five hundredths of the depth (at least ten
//! tolerances) over the surface to the other side: the body to join to
//! the surface's body (raised) or to cut from it (engraved), which then
//! meet it across the surface, never along it, the lap taking nothing
//! either way. On a reversed face (a bore's wall) the sketch's x is
//! turned over, so the text reads from outside the body as in the sketch. The
//! plates of the profiles are disjoint (the profiles neither touch nor
//! cross, and the normals of a cylinder or cone spread them apart outward
//! and inward up to the axis), so they are one body of several lumps.
//!
//! **Names.** As an extrusion's: the skin on the surface is the start cap,
//! the far skin the end cap, a rim the side of its curve (and piece), the
//! edges and vertices by the faces they join.

use crate::KernelError;
use crate::extrude::{OpResult, Origin, Origins, Role};
use crate::plate::{Made, Side, plate};

use crate::profile::{Profile, Segment};
use craft3d_geom::{Curve3, NurbsCurve, Surface};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId};
use std::collections::BTreeMap;
use std::f64::consts::PI;

/// Most samples along one curve.
const MAX_SAMPLES: usize = 4096;

fn refuse(reason: impl Into<String>) -> KernelError {
    KernelError::Plate {
        edge: None,
        reason: reason.into(),
    }
}

/// The development of a cylinder or a cone: sketch `(x, y)` to the
/// surface's `(u, v)`, the sketch's x turned over on a reversed face (one
/// seen from the side the surface's normal points away from, a bore's
/// wall), so that the text reads, seen from outside the body, as it does
/// in the sketch.
#[derive(Clone, Copy, Debug)]
struct Map {
    development: Development,
    /// −1 on a reversed face, else 1.
    flip: f64,
}

impl Map {
    fn uv(&self, p: [f64; 2]) -> Result<(f64, f64), KernelError> {
        self.development.uv([self.flip * p[0], p[1]])
    }
}

#[derive(Clone, Copy, Debug)]
enum Development {
    Cylinder {
        r: f64,
    },
    Cone {
        sin: f64,
        cos: f64,
        sign: f64,
        rho0: f64,
    },
}

impl Development {
    fn of(surface: &Surface) -> Result<Self, KernelError> {
        match surface {
            Surface::Cylinder(c) => Ok(Development::Cylinder { r: c.radius() }),
            Surface::Cone(c) => {
                let a = c.half_angle();
                let (sin, cos) = (a.abs().sin(), a.abs().cos());
                let r = c.radius();
                if !(r > 0.0 && sin > 0.0) {
                    return Err(refuse(
                        "the cone's circle at its frame has no radius: a cone is wrapped from a circle of it, not from its apex",
                    ));
                }
                Ok(Development::Cone {
                    sin,
                    cos,
                    sign: a.signum(),
                    rho0: r / sin,
                })
            }
            other => Err(KernelError::Unsupported {
                operation: "wrap",
                reason: match other {
                    Surface::Plane(_) => "a plane is not wrapped onto: extrude the profile",
                    _ => "wrapping onto a surface other than a cylinder or a cone is not built yet",
                },
                faces: Vec::new(),
            }),
        }
    }

    /// `(u, v)` of a sketch point, with `u` kept in (−π, π): a profile that
    /// reaches round to the back of the surface is refused.
    fn uv(&self, p: [f64; 2]) -> Result<(f64, f64), KernelError> {
        let (u, v) = match *self {
            Development::Cylinder { r } => (p[0] / r, p[1]),
            Development::Cone {
                sin,
                cos,
                sign,
                rho0,
            } => {
                let w = rho0 + sign * p[1];
                if w <= 0.0 {
                    return Err(refuse(
                        "the profile reaches the cone's apex: it is wrapped onto the cone only short of it",
                    ));
                }
                let rho = p[0].hypot(w);
                (p[0].atan2(w) / sin, sign * (rho - rho0) * cos)
            }
        };
        if !(u.is_finite() && v.is_finite()) || u.abs() >= PI {
            return Err(refuse(
                "the profile reaches round to the back of the surface (half a turn from its x direction either way): move it nearer x = 0 or make it shorter",
            ));
        }
        Ok((u, v))
    }
}

/// The point of a segment at `t` in [0, 1] along the loop's direction.
fn on_segment(segment: &Segment, t: f64) -> [f64; 2] {
    segment.point_at(if segment.reversed { 1.0 - t } else { t })
}

/// The unit direction a segment leaves its point at `t` (0 or 1) by, along
/// the loop: from a finite difference, coarser where the curve stalls
/// there (a cusp of a glyph), `None` where it does not move.
fn leaving(segment: &Segment, t: f64) -> Option<[f64; 2]> {
    let p = on_segment(segment, t);
    for h in [1e-7, 1e-5, 1e-3, 1e-1] {
        let q = on_segment(segment, if t < 0.5 { h } else { 1.0 - h });
        let d = if t < 0.5 {
            [q[0] - p[0], q[1] - p[1]]
        } else {
            [p[0] - q[0], p[1] - q[1]]
        };
        let l = d[0].hypot(d[1]);
        if l > 0.0 && l.is_finite() {
            return Some([d[0] / l, d[1] / l]);
        }
    }
    None
}

/// How far a wrapped curve runs on past each end of its segment, in
/// multiples of the plate's thickness: twice as far as [`crate::plate`]
/// traces its rims past an edge's ends at a square joint (its margin of
/// twice the thickness), so that the corner's own reach on a curved sheet
/// fits too and the plate traces the curve's own points there, not a
/// polynomial continued far past a short piece (a glyph's piece of a fifth
/// of a millimetre continued by four: 2026-10-30, the r and t of 3DCraft).
/// Past that the plate continues the curve's end span, which is then the
/// run-on itself, a geodesic.
const RUN_ON: f64 = 4.0;

/// How far the plate laps over the surface into the body's side, as a
/// share of the depth (at least ten tolerances).
const OVERLAP: f64 = 0.05;

/// A segment mapped onto the surface and fitted, run on past its ends
/// along its tangents in the development (a geodesic of the surface) by
/// up to `run_on`, shortened where that would leave the development; and
/// the curve's parameters at the segment's ends.
fn mapped_curve(
    segment: &Segment,
    surface: &Surface,
    dev: &Map,
    tol: f64,
    run_on: f64,
) -> Result<(NurbsCurve, (f64, f64)), KernelError> {
    let ends = [on_segment(segment, 0.0), on_segment(segment, 1.0)];
    // The run-on of each end: shortened (halved) until every point of it
    // lies in the development.
    let mut runs = [0.0_f64; 2];
    let mut ways = [[0.0_f64; 2]; 2];
    for (k, (run, way)) in runs.iter_mut().zip(ways.iter_mut()).enumerate() {
        let Some(d) = leaving(segment, if k == 0 { 0.0 } else { 1.0 }) else {
            continue;
        };
        // Back along the curve at the start, on along it at the end.
        *way = if k == 0 { [-d[0], -d[1]] } else { d };
        let end = ends.get(k).copied().unwrap_or([0.0, 0.0]);
        let mut length = run_on;
        for _ in 0..12 {
            let fits = (1..=8).all(|j| {
                let s = length * f64::from(j) / 8.0;
                dev.uv([end[0] + way[0] * s, end[1] + way[1] * s]).is_ok()
            });
            if fits {
                *run = length;
                break;
            }
            length *= 0.5;
        }
    }
    // A point of the run: `x` in [−1, 0) on before the start, [0, 1] the
    // segment, (1, 2] on past the end.
    let sketch_at = |x: f64| -> [f64; 2] {
        if x < 0.0 {
            let s = -x * runs[0];
            [ends[0][0] + ways[0][0] * s, ends[0][1] + ways[0][1] * s]
        } else if x > 1.0 {
            let s = (x - 1.0) * runs[1];
            [ends[1][0] + ways[1][0] * s, ends[1][1] + ways[1][1] * s]
        } else {
            on_segment(segment, x)
        }
    };
    let map = |x: f64| -> Result<Vec3, KernelError> {
        let (u, v) = dev.uv(sketch_at(x))?;
        Ok(surface.eval(u, v))
    };
    let (before, after) = (runs[0] > 0.0, runs[1] > 0.0);
    let mut n = 8usize;
    loop {
        // The parts: the run-on before, the segment, the run-on after,
        // each fitted on its own (the run-on meets the segment along its
        // tangent, not in its curvature) and joined at knots of full
        // multiplicity less one, through the point they share.
        let half = n / 2;
        let mut parts: Vec<Vec<f64>> = Vec::with_capacity(3);
        if before {
            parts.push((0..=half).map(|k| -1.0 + k as f64 / half as f64).collect());
        }
        parts.push((0..=n).map(|k| k as f64 / n as f64).collect());
        if after {
            parts.push((0..=half).map(|k| 1.0 + k as f64 / half as f64).collect());
        }
        let mut sampled: Vec<(Vec<f64>, Vec<Vec3>)> = Vec::with_capacity(parts.len());
        let mut total = 0.0;
        let mut lengths = Vec::with_capacity(parts.len());
        for xs in &parts {
            let mut points = Vec::with_capacity(xs.len());
            for x in xs {
                points.push(map(*x)?);
            }
            let mut params = vec![total];
            for w in points.windows(2) {
                if let [a, b] = w {
                    total += (*b - *a).length();
                }
                params.push(total);
            }
            lengths.push(
                params.last().copied().unwrap_or(0.0) - params.first().copied().unwrap_or(0.0),
            );
            sampled.push((params, points));
        }
        if total.is_nan() || total <= tol || lengths.iter().any(|l| l.is_nan() || *l <= 0.0) {
            return Err(refuse(
                "a curve of the profile is shorter than the tolerance",
            ));
        }
        let mut knots: Vec<f64> = Vec::new();
        let mut ctrl: Vec<Vec3> = Vec::new();
        let mut range = (0.0, 1.0);
        for (k, (params, points)) in sampled.iter_mut().enumerate() {
            for p in params.iter_mut() {
                *p /= total;
            }
            if k == usize::from(before) {
                range = (
                    params.first().copied().unwrap_or(0.0),
                    params.last().copied().unwrap_or(1.0),
                );
            }
            let values: Vec<Vec<f64>> = points.iter().map(|p| vec![p.x, p.y, p.z]).collect();
            let (own_knots, own_ctrl) = craft3d_geom::refine::interpolate(params, 3, &values)
                .map_err(KernelError::Geometry)?;
            let own_ctrl = own_ctrl.iter().map(|c| {
                Vec3::new(
                    *c.first().unwrap_or(&0.0),
                    *c.get(1).unwrap_or(&0.0),
                    *c.get(2).unwrap_or(&0.0),
                )
            });
            if knots.is_empty() {
                knots.extend_from_slice(&own_knots);
                ctrl.extend(own_ctrl);
            } else {
                // The joint once less than the degree plus one: the shared
                // point is the last control point of the one and the first
                // of the other.
                knots.pop();
                knots.extend(own_knots.iter().skip(4));
                ctrl.extend(own_ctrl.skip(1));
            }
        }
        let weights = vec![1.0; ctrl.len()];
        let curve = NurbsCurve::new(3, knots, ctrl, weights).map_err(KernelError::Geometry)?;
        // Checked at the middle of every stretch between samples.
        let mut worst: f64 = 0.0;
        for ((params, _), xs) in sampled.iter().zip(&parts) {
            for k in 0..xs.len().saturating_sub(1) {
                let (Some(x0), Some(x1), Some(p0), Some(p1)) =
                    (xs.get(k), xs.get(k + 1), params.get(k), params.get(k + 1))
                else {
                    continue;
                };
                let exact = map(0.5 * (x0 + x1))?;
                let fitted = curve.eval(0.5 * (p0 + p1));
                worst = worst.max((exact - fitted).length());
            }
        }
        if worst <= tol * 0.1 {
            return Ok((curve, range));
        }
        if n * 2 > MAX_SAMPLES {
            return Err(refuse(format!(
                "a curve of the profile could not be fitted on the surface within a tenth of the tolerance (off by {worst:e} at {n} samples)"
            )));
        }
        n *= 2;
    }
}

/// What the sheet's entities were made from, for the names.
struct SheetParts {
    /// Per edge of the sheet: the segment's curve id, its piece, and the
    /// curve id of the segment before it in its loop.
    edges: BTreeMap<EdgeId, (u64, u32, u64, u32)>,
    /// The outer loop's curve ids, ascending.
    outer: Vec<u64>,
}

/// One profile as a sheet face on the surface.
fn sheet(
    profile: &Profile,
    surface: &Surface,
    reversed: bool,
    dev: &Map,
    tolerance: &Tolerance,
    run_on: f64,
) -> Result<(Body, FaceId, SheetParts), KernelError> {
    let tol = tolerance.linear;
    let mut body = Body::new(BodyKind::Sheet);
    let mut parts = SheetParts {
        edges: BTreeMap::new(),
        outer: Vec::new(),
    };
    let mut loops = Vec::new();
    for (k, l) in std::iter::once(&profile.outer)
        .chain(profile.holes.iter())
        .enumerate()
    {
        if k == 0 {
            parts.outer = l.segments.iter().map(|s| s.source).collect();
            parts.outer.sort_unstable();
            parts.outer.dedup();
        }
        let n = l.segments.len();
        // The vertex at the start of each segment.
        let mut starts: Vec<VertexId> = Vec::with_capacity(n);
        for s in &l.segments {
            let (u, v) = dev.uv(on_segment(s, 0.0))?;
            starts.push(body.add_vertex(surface.eval(u, v)));
        }
        let mut coedges = Vec::with_capacity(n);
        for (i, s) in l.segments.iter().enumerate() {
            let (curve, range) = mapped_curve(s, surface, dev, tol, run_on)?;
            let (Some(&a), Some(&b)) = (starts.get(i), starts.get((i + 1) % n)) else {
                return Err(refuse("a loop of the profile has no segments"));
            };
            let edge = body.add_edge(Curve3::Nurbs(curve), range, a, b);
            let before = l
                .segments
                .get((i + n - 1) % n)
                .map_or((s.source, s.piece), |p| (p.source, p.piece));
            parts
                .edges
                .insert(edge, (s.source, s.piece, before.0, before.1));
            coedges.push(Coedge {
                edge,
                reversed: false,
            });
        }
        // On a reversed face the sketch's x is turned over (`Map`), so a
        // loop counter-clockwise in the sketch runs clockwise about the
        // surface's normal: counter-clockwise about the outward normal,
        // as a face's loops do, either way.
        loops.push(body.add_loop(coedges));
    }
    let face = body.add_face(surface.clone(), reversed, loops);
    body.add_shell(vec![face]);
    let defects = craft3d_topo::check(&body, tolerance);
    if let Some(defect) = defects.first() {
        return Err(refuse(format!(
            "the profile does not lie on the surface as one face: {defect:?}"
        )));
    }
    Ok((body, face, parts))
}

/// The faces that use each edge of `body`.
fn faces_of_edges(body: &Body) -> BTreeMap<EdgeId, Vec<FaceId>> {
    let mut out: BTreeMap<EdgeId, Vec<FaceId>> = BTreeMap::new();
    for (i, face) in body.faces.iter().enumerate() {
        let id = FaceId(u32::try_from(i).unwrap_or(u32::MAX));
        for l in &face.loops {
            if let Some(lp) = body.loop_(*l) {
                for c in &lp.coedges {
                    out.entry(c.edge).or_default().push(id);
                }
            }
        }
    }
    out
}

/// What a face of a plate is, for the names.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Start,
    End,
    Rim(u64, u32, u64, u32),
}

/// The plate of one profile, with the origin of every entity.
fn plate_of(
    profile: &Profile,
    surface: &Surface,
    reversed: bool,
    depth: f64,
    dev: &Map,
    tolerance: &Tolerance,
) -> Result<OpResult, KernelError> {
    // The plate goes the depth to one side of the surface (out of the
    // body for a positive depth, into it for a negative one) and laps
    // over the surface into the body's side by a little, so that a union
    // or a difference with the body the face is of meets it across the
    // surface, never along it (a skin on the face itself makes slivers
    // where the rims meet the face): the lap of a raised text lies in the
    // body's material, that of an engraved one outside it, and neither
    // changes what the boolean makes.
    let lap = (OVERLAP * depth.abs()).max(10.0 * tolerance.linear);
    let levels = if depth > 0.0 {
        (-lap, depth)
    } else {
        (depth, lap)
    };
    let thickness = levels.1 - levels.0;
    let run_on = RUN_ON * thickness;
    let (sheet, _, parts) = sheet(profile, surface, reversed, dev, tolerance, run_on)?;
    let made = plate(
        &sheet,
        &[FaceId(0)],
        thickness,
        Side::Levels(levels.0, levels.1),
        &BTreeMap::new(),
        tolerance,
    )?;
    let body = made.body;
    // The end cap is the far skin: the front (outside) for a positive
    // depth, the back for a negative one; the other is the start.
    let out = depth > 0.0;
    let kinds: Vec<Kind> = made
        .faces
        .iter()
        .map(|m| match m {
            Made::Front(_) if out => Ok(Kind::End),
            Made::Back(_) if !out => Ok(Kind::End),
            Made::Front(_) | Made::Back(_) => Ok(Kind::Start),
            Made::Rim(e) => match parts.edges.get(e) {
                Some(&(c, p, b, bp)) => Ok(Kind::Rim(c, p, b, bp)),
                None => Err(refuse(
                    "a rim of the plate runs along no curve of the profile (a kernel bug)",
                )),
            },
        })
        .collect::<Result<_, _>>()?;
    let kind = |f: FaceId| {
        kinds
            .get(usize::try_from(f.0).unwrap_or(usize::MAX))
            .copied()
    };
    let faces: Vec<Origin> = kinds
        .iter()
        .map(|k| match k {
            Kind::Start => Origin::new(Role::StartCap, &parts.outer),
            Kind::End => Origin::new(Role::EndCap, &parts.outer),
            Kind::Rim(c, p, ..) => Origin::of_piece(Role::Side, &[*c], *p),
        })
        .collect();
    let around = faces_of_edges(&body);
    let mut edges = Vec::with_capacity(body.edges.len());
    for i in 0..body.edges.len() {
        let id = EdgeId(u32::try_from(i).unwrap_or(u32::MAX));
        let fs: Vec<Kind> = around
            .get(&id)
            .map(|v| v.iter().filter_map(|f| kind(*f)).collect())
            .unwrap_or_default();
        let rims: Vec<(u64, u32, u64, u32)> = fs
            .iter()
            .filter_map(|k| match k {
                Kind::Rim(c, p, b, bp) => Some((*c, *p, *b, *bp)),
                _ => None,
            })
            .collect();
        let origin = match (
            fs.contains(&Kind::Start),
            fs.contains(&Kind::End),
            rims.as_slice(),
        ) {
            (true, _, [r, ..]) => Origin::of_piece(Role::StartEdge, &[r.0], r.1),
            (_, true, [r, ..]) => Origin::of_piece(Role::EndEdge, &[r.0], r.1),
            (_, _, [a, b]) => {
                // The rim the loop arrives by, then the one it leaves by.
                let (arrive, leave) = if b.2 == a.0 && b.3 == a.1 {
                    (a, b)
                } else {
                    (b, a)
                };
                Origin::of_piece(Role::LateralEdge, &[arrive.0, leave.0], leave.1)
            }
            (_, _, [a]) => Origin::of_piece(Role::LateralEdge, &[a.0, a.0], a.1),
            _ => return Err(refuse("an edge of the plate lies on no rim")),
        };
        edges.push(origin);
    }
    // A vertex: on the start or the end skin, named by the lateral edge
    // through it (or the rim's curve twice, for a loop of one curve).
    let mut vertices = vec![None; body.vertices.len()];
    for (i, e) in body.edges.iter().enumerate() {
        let Some(origin) = edges.get(i) else { continue };
        for v in [e.start, e.end] {
            let Some(slot) = vertices.get_mut(usize::try_from(v.0).unwrap_or(usize::MAX)) else {
                continue;
            };
            let cap = match origin.role {
                Role::StartEdge => Some(Role::StartVertex),
                Role::EndEdge => Some(Role::EndVertex),
                _ => None,
            };
            match (origin.role, cap, slot.as_ref()) {
                (Role::LateralEdge, _, _) => {}
                (_, Some(role), None) => {
                    let c = origin.sources.first().copied().unwrap_or(0);
                    *slot = Some(Origin::of_piece(role, &[c, c], origin.piece));
                }
                _ => {}
            }
        }
    }
    // Where a lateral edge meets a vertex, it names it (the curves it
    // joins), on the side its other edges say.
    for (i, e) in body.edges.iter().enumerate() {
        let Some(origin) = edges.get(i) else { continue };
        if origin.role != Role::LateralEdge {
            continue;
        }
        for v in [e.start, e.end] {
            if let Some(Some(slot)) = vertices.get_mut(usize::try_from(v.0).unwrap_or(usize::MAX)) {
                *slot = Origin::of_piece(slot.role, &origin.sources, origin.piece);
            }
        }
    }
    let vertices = vertices
        .into_iter()
        .map(|v| v.ok_or_else(|| refuse("a vertex of the plate is on neither skin")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OpResult {
        body,
        origins: Origins {
            faces,
            edges,
            vertices,
        },
    })
}

/// Bodies that do not meet, as one body of their lumps, their origins in
/// the same order.
fn disjoint(parts: Vec<OpResult>) -> Result<OpResult, KernelError> {
    let mut body = Body::new(BodyKind::Solid);
    let mut origins = Origins {
        faces: Vec::new(),
        edges: Vec::new(),
        vertices: Vec::new(),
    };
    for part in parts {
        let id = |n: usize| u32::try_from(n).map_err(|_| refuse("too many entities"));
        let (dv, de, dl, df) = (
            id(body.vertices.len())?,
            id(body.edges.len())?,
            id(body.loops.len())?,
            id(body.faces.len())?,
        );
        let p = part.body;
        body.vertices.extend(p.vertices.iter().cloned());
        for e in &p.edges {
            let mut e = e.clone();
            e.start = VertexId(e.start.0 + dv);
            e.end = VertexId(e.end.0 + dv);
            body.edges.push(e);
        }
        for l in &p.loops {
            let mut l = l.clone();
            for c in &mut l.coedges {
                c.edge = EdgeId(c.edge.0 + de);
            }
            body.loops.push(l);
        }
        for f in &p.faces {
            let mut f = f.clone();
            for l in &mut f.loops {
                *l = craft3d_topo::LoopId(l.0 + dl);
            }
            body.faces.push(f);
        }
        for s in &p.shells {
            let mut s = s.clone();
            for f in &mut s.faces {
                *f = FaceId(f.0 + df);
            }
            body.shells.push(s);
        }
        for (v, t) in p.tolerances.vertices() {
            body.set_vertex_tolerance(VertexId(v.0 + dv), t)
                .map_err(|e| refuse(format!("{e:?}")))?;
        }
        for (e, t) in p.tolerances.edges() {
            body.set_edge_tolerance(EdgeId(e.0 + de), t)
                .map_err(|e| refuse(format!("{e:?}")))?;
        }
        origins.faces.extend(part.origins.faces);
        origins.edges.extend(part.origins.edges);
        origins.vertices.extend(part.origins.vertices);
    }
    Ok(OpResult { body, origins })
}

/// The profiles (in sketch coordinates, the development's) wrapped onto
/// `surface` (the surface of a face of a body, `reversed` as that face is)
/// and given `depth` along its outward normal: out when positive, in when
/// negative.
pub fn wrap(
    profiles: &[Profile],
    surface: &Surface,
    reversed: bool,
    depth: f64,
    tolerance: &Tolerance,
) -> Result<OpResult, KernelError> {
    if !(depth.is_finite() && depth.abs() > tolerance.linear) {
        return Err(KernelError::InvalidSize {
            name: "depth",
            value: depth,
        });
    }
    let dev = Map {
        development: Development::of(surface)?,
        flip: if reversed { -1.0 } else { 1.0 },
    };
    if profiles.is_empty() {
        return Err(refuse("there is no profile to wrap"));
    }
    let mut parts = Vec::with_capacity(profiles.len());
    for p in profiles {
        parts.push(plate_of(p, surface, reversed, depth, &dev, tolerance)?);
    }
    let made = disjoint(parts)?;
    let defects = craft3d_topo::check(&made.body, tolerance);
    if let Some(d) = defects.first() {
        return Err(refuse(format!("the wrapped solid is not valid: {d:?}")));
    }
    Ok(made)
}
