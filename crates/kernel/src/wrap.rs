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
//! [`crate::plate`] gives it the thickness along the surface's normal with
//! square rims: out for a positive depth, in for a negative one. The
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
/// surface's `(u, v)`.
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

/// A segment mapped onto the surface and fitted.
fn mapped_curve(
    segment: &Segment,
    surface: &Surface,
    dev: &Development,
    tol: f64,
) -> Result<NurbsCurve, KernelError> {
    let map = |t: f64| -> Result<Vec3, KernelError> {
        let (u, v) = dev.uv(on_segment(segment, t))?;
        Ok(surface.eval(u, v))
    };
    let mut n = 8usize;
    loop {
        let mut points = Vec::with_capacity(n + 1);
        let mut ts = Vec::with_capacity(n + 1);
        for k in 0..=n {
            let t = k as f64 / n as f64;
            ts.push(t);
            points.push(map(t)?);
        }
        // Chord-length parameters.
        let mut params = vec![0.0];
        let mut total = 0.0;
        for w in points.windows(2) {
            if let [a, b] = w {
                total += (*b - *a).length();
            }
            params.push(total);
        }
        if total.is_nan() || total <= tol {
            return Err(refuse(
                "a curve of the profile is shorter than the tolerance",
            ));
        }
        let params: Vec<f64> = params.iter().map(|p| p / total).collect();
        let values: Vec<Vec<f64>> = points.iter().map(|p| vec![p.x, p.y, p.z]).collect();
        let (knots, ctrl) = craft3d_geom::refine::interpolate(&params, 3, &values)
            .map_err(KernelError::Geometry)?;
        let ctrl: Vec<Vec3> = ctrl
            .iter()
            .map(|c| {
                Vec3::new(
                    *c.first().unwrap_or(&0.0),
                    *c.get(1).unwrap_or(&0.0),
                    *c.get(2).unwrap_or(&0.0),
                )
            })
            .collect();
        let weights = vec![1.0; ctrl.len()];
        let curve = NurbsCurve::new(3, knots, ctrl, weights).map_err(KernelError::Geometry)?;
        // Checked at the middle of every stretch between samples.
        let mut worst: f64 = 0.0;
        for k in 0..n {
            let (Some(t0), Some(t1), Some(p0), Some(p1)) =
                (ts.get(k), ts.get(k + 1), params.get(k), params.get(k + 1))
            else {
                continue;
            };
            let exact = map(0.5 * (t0 + t1))?;
            let fitted = curve.eval(0.5 * (p0 + p1));
            worst = worst.max((exact - fitted).length());
        }
        if worst <= tol * 0.1 {
            return Ok(curve);
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
    dev: &Development,
    tol: f64,
) -> Result<(Body, FaceId, SheetParts), KernelError> {
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
            let curve = mapped_curve(s, surface, dev, tol)?;
            let (Some(&a), Some(&b)) = (starts.get(i), starts.get((i + 1) % n)) else {
                return Err(refuse("a loop of the profile has no segments"));
            };
            let edge = body.add_edge(Curve3::Nurbs(curve), (0.0, 1.0), a, b);
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
        if reversed {
            // Loops run counter-clockwise about the outward normal, which
            // is against the surface's own normal here.
            coedges.reverse();
            for c in &mut coedges {
                c.reversed = true;
            }
        }
        loops.push(body.add_loop(coedges));
    }
    let face = body.add_face(surface.clone(), reversed, loops);
    body.add_shell(vec![face]);
    let tolerance =
        Tolerance::new(tol, 1e-9).ok_or_else(|| refuse("the tolerance is not positive"))?;
    let defects = craft3d_topo::check(&body, &tolerance);
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
    dev: &Development,
    tolerance: &Tolerance,
) -> Result<OpResult, KernelError> {
    let (sheet, _, parts) = sheet(profile, surface, reversed, dev, tolerance.linear)?;
    let side = if depth > 0.0 { Side::Out } else { Side::In };
    let made = plate(
        &sheet,
        &[FaceId(0)],
        depth.abs(),
        side,
        &BTreeMap::new(),
        tolerance,
    )?;
    let body = made.body;
    // The skin on the surface is the start; out, that is the back skin.
    let kinds: Vec<Kind> = made
        .faces
        .iter()
        .map(|m| match (m, side) {
            (Made::Back(_), Side::Out) | (Made::Front(_), Side::In) => Kind::Start,
            (Made::Front(_), _) | (Made::Back(_), _) => Kind::End,
            (Made::Rim(e), _) => match parts.edges.get(e) {
                Some(&(c, p, b, bp)) => Kind::Rim(c, p, b, bp),
                None => Kind::End,
            },
        })
        .collect();
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
    let dev = Development::of(surface)?;
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
