//! The review of the fuzzer's timeouts of 2026-11-25 (`check/fuzz-timeouts`
//! 963416fb): whether two faces lie on one another, told from the samples'
//! feet first ([`feet`]), held to [`coincide`] as it was on `main` ed93bafc.
//! That version is kept here word for word as the reference
//! (`reference_coincide`, `reference_sample`); the only thing added to it
//! is that it hands out how many samples it counted each way, for the
//! lemma the change rests on: a sample `sample` counts is one `feet` says
//! can count, so fewer than [`EVIDENCE`] that can count leave no answer
//! but "no evidence".
//!
//! Over many pairs of faces, each pair asked on fresh operands each time
//! (a cover's answers depend on what it was asked before): hexahedra over
//! one bicubic skin (as in `tests/boolean_coincide.rs`) stacked on it,
//! side by side on it, lifted off it by parts of the tolerance around a
//! quarter of it, on a surface that leaves it inside the faces, on the
//! skin given with its parameters exchanged or reversed, and on the skin
//! with a bump `c (u + s v − m)²` that touches it along a line, inside both
//! faces, straight or slanting, and along a border of either (the case of
//! the fuzzer's fillets), with `c` swept so that the samples that can count
//! pass the evidence asked for; vases turned about their axis (a seam,
//! moved) and grown by parts of the tolerance; the fillet of a ring cut by
//! a cylinder against that cylinder. At 1e-3, 1 and 1e6, and turned.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_range_loop,
    clippy::type_complexity,
    clippy::assign_op_pattern,
    clippy::large_enum_variant
)]

use super::*;
use crate::boolean::section::prepare;
use crate::boolean::{BooleanOp, Operand, Vertices, boolean};
use crate::fillet::{FilletEdge, fillet};
use crate::primitives;
use crate::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use crate::revolve::revolve;
use craft3d_geom::{Curve3, Line, NurbsCurve, NurbsSurface, Plane};
use craft3d_math::{Frame, Portable, Tolerance, Transform};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, VertexId, check};
use std::collections::BTreeMap;
use std::f64::consts::TAU;

// ---------------------------------------------------------------------
// The reference: `coincide` and `sample` as on `main` ed93bafc.
// ---------------------------------------------------------------------

/// `coincide` of `main`, word for word, with what `reference_sample`
/// counted each way (0: the face of `a` sampled, 1: the face of `b`).
fn reference_coincide(
    a: &Solid,
    fa: usize,
    b: &Solid,
    fb: usize,
    tol: f64,
    counted: &mut Vec<(usize, usize)>,
) -> Result<Coincidence, Stop> {
    let (oa, ob) = (a.outline(fa)?, b.outline(fb)?);
    if !(is_freeform(&oa.surface) || is_freeform(&ob.surface)) {
        return Ok(Coincidence::Unknown);
    }
    let mut same = None;
    let mut evidence = 0_usize;
    let mut partly = false;
    for (way, ((own, of_own), (other, of_other))) in [((a, fa), (b, fb)), ((b, fb), (a, fa))]
        .into_iter()
        .enumerate()
    {
        if !is_freeform(&own.outline(of_own)?.surface) {
            continue;
        }
        let mut count = 0_usize;
        let found = reference_sample((own, of_own), (other, of_other), tol, &mut count);
        counted.push((way, count));
        match found {
            Found::Over { agree, count } => {
                if same.is_some_and(|s| s != agree) {
                    return Ok(Coincidence::Unknown);
                }
                same = Some(agree);
                evidence += count;
            }
            Found::Partly => partly = true,
            Found::Nothing => {}
            Found::Unknown => return Ok(Coincidence::Unknown),
        }
    }
    Ok(match (partly, same) {
        (true, _) => Coincidence::Partly,
        (false, Some(agree)) if evidence >= EVIDENCE => Coincidence::Faces(agree),
        _ => Coincidence::Unknown,
    })
}

/// `sample` of `main`, word for word, with the samples it counted added
/// to `counted` where it counts them.
fn reference_sample(
    (solid, face): (&Solid, usize),
    (twin, of_twin): (&Solid, usize),
    tol: f64,
    counted: &mut usize,
) -> Found {
    let (Ok(own), Ok(other), Some(other_box)) = (
        solid.outline(face),
        twin.outline(of_twin),
        twin.boxes.get(of_twin),
    ) else {
        return Found::Unknown;
    };
    let Some(((ulo, uhi), (vlo, vhi))) = trimmed(own) else {
        return Found::Unknown;
    };
    let us = stations(&knots(&own.surface, true), ulo, uhi);
    let vs = stations(&knots(&own.surface, false), vlo, vhi);
    if us.len().saturating_mul(vs.len()) > MOST {
        return Found::Unknown;
    }
    let mut rows: Vec<Row> = Vec::with_capacity(us.len());
    let mut agree: Option<bool> = None;
    let (mut count, mut apart) = (0_usize, false);
    for &u in &us {
        let mut row = Vec::with_capacity(vs.len());
        for &v in &vs {
            row.push(f64::NAN);
            let p = own.surface.eval(u, v);
            if !(p.is_finite() && other_box.contains(p, 2.0 * tol)) {
                continue;
            }
            let Some((pu, pv)) = other.sheet().project(p) else {
                return Found::Unknown;
            };
            let q = other.surface.eval(pu, pv);
            let d = p.distance(q);
            if !d.is_finite() {
                return Found::Unknown;
            }
            let near = d <= 0.25 * tol;
            if !twin.may_hold(of_twin, q, tol) {
                continue;
            }
            let over = match other.side(q, tol) {
                Ok(Side::In) => match own.side(p, tol) {
                    Ok(Side::In) => true,
                    Ok(_) => false,
                    Err(_) => return Found::Unknown,
                },
                Ok(_) => false,
                Err(_) => return Found::Unknown,
            };
            if !over {
                continue;
            }
            if !near {
                apart = true;
                continue;
            }
            if let Some(last) = row.last_mut() {
                *last = d;
            }
            count += 1;
            *counted += 1;
            let (Some(n), Some(m)) = (own.surface.normal(u, v), other.surface.normal(pu, pv))
            else {
                return Found::Unknown;
            };
            let here = n.dot(m) > 0.0;
            if agree.is_some_and(|a| a != here) {
                return Found::Unknown;
            }
            agree = Some(here);
        }
        rows.push(row);
    }
    if apart {
        return if count >= EVIDENCE {
            Found::Partly
        } else {
            Found::Unknown
        };
    }
    let Some(agree) = agree else {
        return Found::Nothing;
    };
    if !between(&rows, tol) {
        return Found::Unknown;
    }
    Found::Over { agree, count }
}

// ---------------------------------------------------------------------
// The faces: hexahedra over one skin (`tests/boolean_coincide.rs`).
// ---------------------------------------------------------------------

const L: f64 = 4.0;
const W: f64 = 3.0;
const H: f64 = 0.05;
/// The tolerance at scale 1.
const TOL: f64 = 1e-6;

const KNOTS_U: [f64; 11] = [0.0, 0.0, 0.0, 0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 1.0];
const KNOTS_V: [f64; 10] = [0.0, 0.0, 0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0];

type Rect = ((f64, f64), (f64, f64));

const RA: Rect = ((0.1, 0.6), (0.15, 0.85));
const RB: Rect = ((0.4, 0.9), (0.3, 0.7));
/// The first plate, reaching past u = 0.75, where a surface parted at
/// u = 0.55 leaves the skin.
const RW: Rect = ((0.1, 0.95), (0.15, 0.85));

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(TOL * k, 1e-9).unwrap()
}

/// A bump added to the skin, at scale 1: `c (u + slant v − m)²`.
#[derive(Clone, Copy)]
struct Bump {
    c: f64,
    m: f64,
    slant: f64,
}

/// The skin at scale `k`, lifted by `lift` (scale 1), with `bump` added
/// exactly: a quadratic is a cubic B-spline whose coefficients are its
/// blossom at the knots (`u²`: the mean of the products of two of the
/// three; `u v`: the product of the Greville abscissae).
fn skin(k: f64, lift: f64, bump: Option<Bump>) -> NurbsSurface {
    let greville = |knots: &[f64], i: usize| (knots[i + 1] + knots[i + 2] + knots[i + 3]) / 3.0;
    let square = |knots: &[f64], i: usize| {
        let (a, b, c) = (knots[i + 1], knots[i + 2], knots[i + 3]);
        (a * b + a * c + b * c) / 3.0
    };
    let (nu, nv) = (7, 6);
    let mut ctrl = Vec::new();
    for i in 0..nu {
        for j in 0..nv {
            let (fi, fj) = (i as f64, j as f64);
            let z = 0.8
                * (0.6 + 0.25 * (1.7 * fi + 0.9 * fj).psin() + 0.1 * (2.3 * fj - 0.4 * fi).pcos());
            let (gu, gv) = (greville(&KNOTS_U, i), greville(&KNOTS_V, j));
            let extra = bump.map_or(0.0, |Bump { c, m, slant }| {
                c * (square(&KNOTS_U, i) + slant * slant * square(&KNOTS_V, j) + m * m
                    - 2.0 * m * gu
                    - 2.0 * m * slant * gv
                    + 2.0 * slant * gu * gv)
            });
            ctrl.push(Vec3::new(L * gu * k, W * gv * k, (z + lift + extra) * k));
        }
    }
    NurbsSurface::new(
        3,
        3,
        KNOTS_U.to_vec(),
        KNOTS_V.to_vec(),
        nu,
        nv,
        ctrl,
        vec![1.0; nu * nv],
    )
    .unwrap()
}

/// The surface cut down to `rect` and continued over the whole domain
/// again as the functions its end spans are.
fn parted(s: &NurbsSurface, rect: Rect, k: f64) -> NurbsSurface {
    s.restricted(rect.0, rect.1)
        .unwrap()
        .extended((0.0, 1.0), (0.0, 1.0), 1e-9 * k)
        .unwrap()
}

/// The same surface with u and v exchanged.
fn transposed(s: &NurbsSurface) -> NurbsSurface {
    let (du, dv) = s.degrees();
    let (nu, nv) = s.counts();
    let (c, w) = (s.control_points(), s.weights());
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for j in 0..nv {
        for i in 0..nu {
            ctrl.push(c[i * nv + j]);
            weights.push(w[i * nv + j]);
        }
    }
    NurbsSurface::new(
        dv,
        du,
        s.knots_v().to_vec(),
        s.knots_u().to_vec(),
        nv,
        nu,
        ctrl,
        weights,
    )
    .unwrap()
}

/// The same surface with u reversed.
fn reversed_u(s: &NurbsSurface) -> NurbsSurface {
    let (du, dv) = s.degrees();
    let (nu, nv) = s.counts();
    let (c, w) = (s.control_points(), s.weights());
    let ku = s.knots_u();
    let (a, b) = (ku[0], ku[ku.len() - 1]);
    let knots: Vec<f64> = ku.iter().rev().map(|t| a + b - t).collect();
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for i in (0..nu).rev() {
        for j in 0..nv {
            ctrl.push(c[i * nv + j]);
            weights.push(w[i * nv + j]);
        }
    }
    NurbsSurface::new(du, dv, knots, s.knots_v().to_vec(), nu, nv, ctrl, weights).unwrap()
}

/// A floor or a roof: the plane z = c (scale 1), or a face on `surface`,
/// which equals `base` over the rectangle (its corners and borders are
/// `base`'s).
#[derive(Clone)]
enum Level {
    Plane(f64),
    Skin {
        base: NurbsSurface,
        surface: NurbsSurface,
    },
}

impl Level {
    fn on(base: NurbsSurface) -> Level {
        Level::Skin {
            surface: base.clone(),
            base,
        }
    }

    fn point(&self, k: f64, u: f64, v: f64) -> Vec3 {
        match self {
            Level::Plane(c) => Vec3::new(L * u * k, W * v * k, c * k),
            Level::Skin { base, .. } => base.eval(u, v),
        }
    }

    /// The curve of the border of `rect` along u at `v` (`along_u`) or
    /// along v at `u`, from its lower end to its upper one, with its
    /// range.
    fn border(&self, k: f64, rect: Rect, along_u: bool, at: f64) -> (Curve3, (f64, f64)) {
        let ((ua, ub), (va, vb)) = rect;
        match self {
            Level::Plane(_) => {
                let (p, q) = if along_u {
                    (self.point(k, ua, at), self.point(k, ub, at))
                } else {
                    (self.point(k, at, va), self.point(k, at, vb))
                };
                (
                    Curve3::Line(Line::through(p, q).unwrap()),
                    (0.0, p.distance(q)),
                )
            }
            Level::Skin { base, .. } => {
                let piece = base.restricted(rect.0, rect.1).unwrap();
                let (nu, nv) = piece.counts();
                let c = piece.control_points();
                let (curve, range) = if along_u {
                    let j = if at == va { 0 } else { nv - 1 };
                    let pts: Vec<Vec3> = (0..nu).map(|i| c[i * nv + j]).collect();
                    (
                        NurbsCurve::polynomial(3, piece.knots_u().to_vec(), pts).unwrap(),
                        (ua, ub),
                    )
                } else {
                    let i = if at == ua { 0 } else { nu - 1 };
                    let pts: Vec<Vec3> = (0..nv).map(|j| c[i * nv + j]).collect();
                    (
                        NurbsCurve::polynomial(3, piece.knots_v().to_vec(), pts).unwrap(),
                        (va, vb),
                    )
                };
                (Curve3::Nurbs(curve), range)
            }
        }
    }

    /// The face's surface and whether it is reversed for the outward
    /// normal `outward`.
    fn face(&self, k: f64, rect: Rect, outward: Vec3) -> (Surface, bool) {
        match self {
            Level::Plane(c) => {
                let frame = Frame::new(Vec3::new(0.0, 0.0, c * k), outward, Vec3::X).unwrap();
                (Surface::Plane(Plane::new(frame)), false)
            }
            Level::Skin { surface, .. } => {
                let s = Surface::Nurbs(surface.clone());
                let ((ua, ub), (va, vb)) = rect;
                let middle = self.point(k, 0.5 * (ua + ub), 0.5 * (va + vb));
                let (u, v) = s.project(middle).unwrap();
                let n = s.normal(u, v).unwrap();
                (s, n.dot(outward) < 0.0)
            }
        }
    }
}

/// The hexahedron from `floor` up to `roof` over `rect`, or `None` where
/// the checker refuses it. Faces: floor 0, roof 1, walls 2 to 5.
fn hexahedron(k: f64, floor: &Level, roof: &Level, rect: Rect) -> Option<Body> {
    let ((ua, ub), (va, vb)) = rect;
    let mut body = Body::new(BodyKind::Solid);
    let levels = [floor, roof];
    let us = [ua, ub];
    let vs = [va, vb];
    let mut corner: BTreeMap<(usize, usize, usize), VertexId> = BTreeMap::new();
    for iu in 0..2 {
        for iv in 0..2 {
            for iz in 0..2 {
                let p = levels[iz].point(k, us[iu], vs[iv]);
                corner.insert((iu, iv, iz), body.add_vertex(p));
            }
        }
    }
    type Key = ((usize, usize, usize), (usize, usize, usize));
    let mut edges: BTreeMap<Key, EdgeId> = BTreeMap::new();
    for iz in 0..2 {
        for iv in 0..2 {
            let (curve, range) = levels[iz].border(k, rect, true, vs[iv]);
            let (a, b) = ((0, iv, iz), (1, iv, iz));
            let e = body.add_edge(curve, range, corner[&a], corner[&b]);
            edges.insert((a, b), e);
        }
        for iu in 0..2 {
            let (curve, range) = levels[iz].border(k, rect, false, us[iu]);
            let (a, b) = ((iu, 0, iz), (iu, 1, iz));
            let e = body.add_edge(curve, range, corner[&a], corner[&b]);
            edges.insert((a, b), e);
        }
    }
    for iu in 0..2 {
        for iv in 0..2 {
            let (a, b) = ((iu, iv, 0), (iu, iv, 1));
            let (p, q) = (
                levels[0].point(k, us[iu], vs[iv]),
                levels[1].point(k, us[iu], vs[iv]),
            );
            let line = Curve3::Line(Line::through(p, q).unwrap());
            let e = body.add_edge(line, (0.0, p.distance(q)), corner[&a], corner[&b]);
            edges.insert((a, b), e);
        }
    }
    let point = |c: (usize, usize, usize)| levels[c.2].point(k, us[c.0], vs[c.1]);
    let mut faces = Vec::new();
    let quads: [([(usize, usize, usize); 4], Vec3); 6] = [
        ([(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)], -Vec3::Z),
        ([(0, 0, 1), (1, 0, 1), (1, 1, 1), (0, 1, 1)], Vec3::Z),
        ([(0, 0, 0), (0, 1, 0), (0, 1, 1), (0, 0, 1)], -Vec3::X),
        ([(1, 0, 0), (1, 1, 0), (1, 1, 1), (1, 0, 1)], Vec3::X),
        ([(0, 0, 0), (1, 0, 0), (1, 0, 1), (0, 0, 1)], -Vec3::Y),
        ([(0, 1, 0), (1, 1, 0), (1, 1, 1), (0, 1, 1)], Vec3::Y),
    ];
    for (index, (quad, outward)) in quads.iter().enumerate() {
        let mut cycle = quad.to_vec();
        let mut newell = Vec3::ZERO;
        for i in 0..4 {
            let (p, q) = (point(cycle[i]), point(cycle[(i + 1) % 4]));
            newell = newell + p.cross(q);
        }
        if newell.dot(*outward) < 0.0 {
            cycle.reverse();
        }
        let coedges: Vec<Coedge> = (0..4)
            .map(|i| {
                let (a, b) = (cycle[i], cycle[(i + 1) % 4]);
                match edges.get(&(a, b)) {
                    Some(&edge) => Coedge {
                        edge,
                        reversed: false,
                    },
                    None => Coedge {
                        edge: edges[&(b, a)],
                        reversed: true,
                    },
                }
            })
            .collect();
        let l = body.add_loop(coedges);
        let (surface, reversed) = match index {
            0 => floor.face(k, rect, *outward),
            1 => roof.face(k, rect, *outward),
            _ => {
                let origin = point(quad[0]);
                let hint = if outward.x != 0.0 { Vec3::Y } else { Vec3::X };
                let frame = Frame::new(origin, *outward, hint).unwrap();
                (Surface::Plane(Plane::new(frame)), false)
            }
        };
        faces.push(body.add_face(surface, reversed, vec![l]));
    }
    body.add_shell(faces);
    check(&body, &tolerance(k)).is_empty().then_some(body)
}

/// A vase about the z axis (a spline profile turned a full turn from
/// `from`, so its seam is there), its radii grown by `grow` (scale 1).
fn vase(k: f64, grow: f64, from: f64) -> Option<Body> {
    let tolerance = tolerance(k);
    let (r0, r1, h) = (8.0 + grow, 6.0 + grow, 20.0);
    let line = |id: u64, s: [f64; 2], e: [f64; 2]| SketchCurve {
        id,
        curve: Curve2::Line {
            start: [s[0] * k, s[1] * k],
            end: [e[0] * k, e[1] * k],
        },
    };
    let mid = 0.5 * (r0 + r1);
    let control: Vec<[f64; 2]> = [
        [r0, 0.0],
        [r0 + 3.0, h / 3.0],
        [mid - 2.0, 2.0 * h / 3.0],
        [r1, h],
    ]
    .into_iter()
    .map(|[x, y]| [x * k, y * k])
    .collect();
    let curves = [
        line(1, [0.0, 0.0], [r0, 0.0]),
        SketchCurve {
            id: 2,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control,
            }),
        },
        line(3, [r1, h], [0.0, h]),
        line(4, [0.0, h], [0.0, 0.0]),
    ];
    let profiles = detect_profiles(&curves, &tolerance).ok()?;
    let plane = Frame::new(Vec3::ZERO, -Vec3::Y, Vec3::X)?;
    let made = revolve(
        &profiles,
        &plane,
        [0.0, 0.0],
        [0.0, 1.0],
        from,
        from + TAU,
        &tolerance,
    )
    .ok()?;
    Some(made.body)
}

/// The faces of `body` that use `edge`.
fn users(body: &Body, edge: EdgeId) -> Vec<usize> {
    (0..body.faces.len())
        .filter(|&f| {
            body.faces[f]
                .loops
                .iter()
                .filter_map(|&l| body.loop_(l))
                .flat_map(|l| &l.coedges)
                .any(|c| c.edge == edge)
        })
        .collect()
}

/// A cylinder, and a ring through its wall (its axis leaning `lean` from
/// the x axis towards z) less the cylinder, filleted with `radius` along
/// the first edge between the ring and the cut: the fillet's border on
/// the cut lies on the cylinder, tangent to it.
fn filleted_cut(k: f64, lean: f64, radius: f64) -> Option<(Body, Body)> {
    let tolerance = tolerance(k);
    let cylinder = primitives::cylinder(
        &Frame::new(Vec3::new(0.0, 0.0, -4.0 * k), Vec3::Z, Vec3::X)?,
        2.0 * k,
        8.0 * k,
    )
    .ok()?;
    let (s, c) = lean.psin_cos();
    let ring = primitives::torus(
        &Frame::new(Vec3::new(0.0, 0.3 * k, 0.0), Vec3::new(c, 0.0, s), Vec3::Y)?,
        3.0 * k,
        k,
    )
    .ok()?;
    let cut = boolean(&ring, &cylinder, BooleanOp::Subtract, &tolerance)
        .ok()?
        .body;
    let edge = (0..cut.edges.len()).find_map(|e| {
        let edge = EdgeId(u32::try_from(e).ok()?);
        let faces = users(&cut, edge);
        let on_ring = faces
            .iter()
            .any(|&f| matches!(cut.faces[f].surface, Surface::Torus(_)));
        let on_wall = faces
            .iter()
            .any(|&f| matches!(cut.faces[f].surface, Surface::Cylinder(_)));
        (on_ring && on_wall).then_some(edge)
    })?;
    let edges = [FilletEdge {
        edge,
        radius: radius * k,
    }];
    let filleted = fillet(&cut, &edges, &tolerance).ok()?.body;
    Some((cylinder, filleted))
}

// ---------------------------------------------------------------------
// The comparison.
// ---------------------------------------------------------------------

/// The two operands prepared afresh, each with its own vertices.
fn solids<'a>(a: &'a Body, b: &'a Body, tolerance: &Tolerance) -> Option<(Solid<'a>, Solid<'a>)> {
    let tol = tolerance.linear;
    let mut va = Vertices::new(0.5 * tol, 2.0 * tol);
    let mut vb = Vertices::new(0.5 * tol, 2.0 * tol);
    let sa = prepare(a, Operand::A, &mut va, tolerance, true).ok()?;
    let sb = prepare(b, Operand::B, &mut vb, tolerance, true).ok()?;
    Some((sa, sb))
}

/// How many samples `feet` says can count, each way `coincide` samples
/// the pair (0: the face of `a`, 1: that of `b`); `None` where a way's
/// feet are not found.
fn possible(a: &Solid, fa: usize, b: &Solid, fb: usize, tol: f64) -> Vec<(usize, Option<usize>)> {
    let mut ways = Vec::new();
    for (way, ((own, of_own), (other, of_other))) in [((a, fa), (b, fb)), ((b, fb), (a, fa))]
        .into_iter()
        .enumerate()
    {
        let Ok(outline) = own.outline(of_own) else {
            continue;
        };
        if is_freeform(&outline.surface) {
            ways.push((
                way,
                feet((own, of_own), (other, of_other), tol).map(|f| f.possible),
            ));
        }
    }
    ways
}

#[derive(Default)]
struct Tally {
    pairs: usize,
    unprepared: usize,
    not_built: Vec<String>,
    mismatches: Vec<String>,
    lemma: Vec<String>,
    same: usize,
    opposite: usize,
    partly: usize,
    early: usize,
    late: usize,
    stopped: usize,
    /// Pairs whose samples that can count, both ways together, are
    /// 8 to 15, and 16 to 31.
    below: usize,
    above: usize,
    old_units: u64,
    new_units: u64,
    /// (the change's ledger, `main`'s, the pair) where the change asks
    /// more.
    dearer: Vec<(u64, u64, String)>,
}

/// Every pair of a face of `a` and a face of `b` of which one is
/// freeform, asked both ways.
fn compare(tally: &mut Tally, what: &str, a: &Body, b: &Body, tolerance: &Tolerance) {
    let tol = tolerance.linear;
    for fa in 0..a.faces.len() {
        for fb in 0..b.faces.len() {
            if !(is_freeform(&a.faces[fa].surface) || is_freeform(&b.faces[fb].surface)) {
                continue;
            }
            let (Some((na, nb)), Some((ra, rb)), Some((pa, pb))) = (
                solids(a, b, tolerance),
                solids(a, b, tolerance),
                solids(a, b, tolerance),
            ) else {
                tally.unprepared += 1;
                continue;
            };
            tally.pairs += 1;
            let label = format!("{what}, faces {fa} and {fb}");
            let (new, new_units) =
                craft3d_geom::allowance::measured_asked(|| coincide(&na, fa, &nb, fb, tol));
            let mut counted = Vec::new();
            let (old, old_units) = craft3d_geom::allowance::measured_asked(|| {
                reference_coincide(&ra, fa, &rb, fb, tol, &mut counted)
            });
            let ways = possible(&pa, fa, &pb, fb, tol);
            let (new_text, old_text) = (format!("{new:?}"), format!("{old:?}"));
            if new_text != old_text {
                tally
                    .mismatches
                    .push(format!("{label}: {new_text}, where main says {old_text}"));
            }
            for &(way, count) in &counted {
                if let Some(&(_, Some(most))) = ways.iter().find(|(w, _)| *w == way)
                    && count > most
                {
                    tally.lemma.push(format!(
                        "{label}: main counted {count} samples way {way}, feet says at most {most}"
                    ));
                }
            }
            let total = ways
                .iter()
                .try_fold(0_usize, |sum, &(_, p)| p.map(|p| sum + p));
            match (&new, total) {
                (Ok(Coincidence::Faces(true)), _) => tally.same += 1,
                (Ok(Coincidence::Faces(false)), _) => tally.opposite += 1,
                (Ok(Coincidence::Partly), _) => tally.partly += 1,
                (Ok(Coincidence::Unknown), Some(t)) if t < EVIDENCE => tally.early += 1,
                (Ok(Coincidence::Unknown), _) => tally.late += 1,
                (Err(_), _) => tally.stopped += 1,
            }
            if let Some(t) = total {
                if (8..EVIDENCE).contains(&t) {
                    tally.below += 1;
                } else if (EVIDENCE..2 * EVIDENCE).contains(&t) {
                    tally.above += 1;
                }
            }
            tally.old_units += old_units;
            tally.new_units += new_units;
            if new_units > old_units {
                tally.dearer.push((new_units, old_units, label));
            }
        }
    }
}

/// `a` and `b` turned by `turn` (when given) and compared, or noted as
/// not built.
fn run(
    tally: &mut Tally,
    what: &str,
    a: Option<Body>,
    b: Option<Body>,
    turn: Option<&Transform>,
    tolerance: &Tolerance,
) {
    let place = |body: Option<Body>| match (body, turn) {
        (Some(body), Some(t)) => body.transformed(t).ok(),
        (body, None) => body,
        (None, Some(_)) => None,
    };
    match (place(a), place(b)) {
        (Some(a), Some(b)) => compare(tally, what, &a, &b, tolerance),
        _ => tally.not_built.push(what.to_string()),
    }
}

/// The answer from the feet first is `main`'s answer for every pair, and
/// no sample `main` counted is one `feet` says cannot count.
#[test]
#[ignore = "the review's comparison with main's coincide, minutes in release: cargo test --release -p craft3d-kernel --lib -- --ignored early_review --nocapture"]
fn the_answer_from_the_feet_first_is_mains_for_every_pair_of_faces() {
    let started = std::time::Instant::now();
    let mut tally = Tally::default();
    for (k, turned) in [(1e-3, false), (1.0, false), (1e6, false), (1.0, true)] {
        let tolerance = tolerance(k);
        let turn = turned.then(|| {
            Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 0.7)
                .unwrap()
                .then(&Transform::translation(Vec3::new(
                    5.0 * k,
                    -3.0 * k,
                    2.0 * k,
                )))
        });
        let turn = turn.as_ref();
        let at = format!("at {k}{}", if turned { ", turned" } else { "" });
        let one = |lift: f64| skin(k, lift, None);
        let below = Level::Plane(-0.5);
        let above = Level::Plane(2.0);

        // Two plates stacked on one skin, each face on the skin parted
        // past it: they touch over the rectangle both cover.
        run(
            &mut tally,
            &format!("stacked {at}"),
            hexahedron(
                k,
                &below,
                &Level::Skin {
                    base: one(H),
                    surface: parted(&one(H), RA, k),
                },
                RA,
            ),
            hexahedron(
                k,
                &Level::Skin {
                    base: one(H),
                    surface: parted(&one(H), RB, k),
                },
                &above,
                RB,
            ),
            turn,
            &tolerance,
        );
        // Two plates side by side on one skin, the second's floor on it
        // with u and v exchanged.
        run(
            &mut tally,
            &format!("side by side {at}"),
            hexahedron(k, &Level::on(one(0.0)), &above, RA),
            hexahedron(
                k,
                &Level::Skin {
                    base: one(0.0),
                    surface: transposed(&one(0.0)),
                },
                &Level::Plane(1.5),
                RB,
            ),
            turn,
            &tolerance,
        );
        // On the skin with its parameters exchanged after parting, or u
        // reversed.
        for (name, surface) in [
            ("transposed", transposed(&parted(&one(H), RB, k))),
            ("reversed", reversed_u(&one(H))),
        ] {
            run(
                &mut tally,
                &format!("{name} {at}"),
                hexahedron(k, &below, &Level::on(one(H)), RA),
                hexahedron(
                    k,
                    &Level::Skin {
                        base: one(H),
                        surface,
                    },
                    &above,
                    RB,
                ),
                turn,
                &tolerance,
            );
        }
        // Lifted off the skin by parts of the tolerance about a quarter
        // of it (the normal distance is less, by the slope).
        for part in [0.05, 0.2, 0.24, 0.26, 0.3, 0.45, 0.55, 1.0, 2.5, 10.0] {
            run(
                &mut tally,
                &format!("lifted by {part} of the tolerance {at}"),
                hexahedron(k, &below, &Level::on(one(H)), RA),
                hexahedron(k, &Level::on(one(H + part * TOL)), &above, RB),
                turn,
                &tolerance,
            );
        }
        // On a surface that leaves the skin inside both faces: parted at
        // u = 0.55, the same polynomial up to the knot 0.75, apart after.
        run(
            &mut tally,
            &format!("parted inside {at}"),
            hexahedron(k, &below, &Level::on(one(H)), RW),
            hexahedron(
                k,
                &Level::on(parted(&one(H), ((0.4, 0.55), (0.4, 0.6)), k)),
                &above,
                RB,
            ),
            turn,
            &tolerance,
        );
        // Touching the skin along a line: slanting across both faces,
        // straight across them, and along a border of either.
        let mut bumps = Vec::new();
        for c in [
            1e-7, 1e-6, 1e-5, 1e-4, 1e-3, 4e-3, 1e-2, 1.5e-2, 2e-2, 3e-2, 4e-2, 5e-2, 7e-2, 0.1,
            0.15, 0.3, 1.0,
        ] {
            bumps.push(Bump {
                c,
                m: 0.85,
                slant: 0.7,
            });
        }
        for c in [1e-6, 1e-4, 1e-2, 1.0] {
            bumps.push(Bump {
                c,
                m: 0.5,
                slant: 0.0,
            });
        }
        for c in [1e-4, 1e-2] {
            for m in [0.4, 0.6] {
                bumps.push(Bump { c, m, slant: 0.0 });
            }
        }
        for bump in bumps {
            let Bump { c, m, slant } = bump;
            run(
                &mut tally,
                &format!("touching along u + {slant} v = {m}, bent by {c}, {at}"),
                hexahedron(k, &below, &Level::on(one(H)), RA),
                hexahedron(k, &Level::on(skin(k, H, Some(bump))), &above, RB),
                turn,
                &tolerance,
            );
        }
        // Vases: the seam turned a radian, and grown by parts of the
        // tolerance.
        for (name, grow, from) in [
            ("seam turned", 0.0, 1.0),
            ("grown by a tenth", 0.1 * TOL, 0.0),
            ("grown by 0.24", 0.24 * TOL, 0.0),
            ("grown by 0.3", 0.3 * TOL, 0.0),
            ("grown by one tolerance", TOL, 0.0),
        ] {
            run(
                &mut tally,
                &format!("vase, {name}, {at}"),
                vase(k, 0.0, 0.0),
                vase(k, grow, from),
                turn,
                &tolerance,
            );
        }
        // The fillet of a ring cut by a cylinder against the cylinder.
        for (lean, radius) in [(0.0, 0.2), (0.35, 0.3)] {
            let (a, b) = filleted_cut(k, lean, radius).unzip();
            run(
                &mut tally,
                &format!("fillet {radius} of a ring leaning {lean}, {at}"),
                a,
                b,
                turn,
                &tolerance,
            );
        }
    }
    tally
        .dearer
        .sort_by_key(|(new, old, _)| std::cmp::Reverse(new - old));
    eprintln!(
        "{} pairs in {:.1} s ({} not prepared; not built: {:?})",
        tally.pairs,
        started.elapsed().as_secs_f64(),
        tally.unprepared,
        tally.not_built
    );
    eprintln!(
        "answers: faces alike {}, faces opposite {}, partly {}, no evidence from the feet {}, no evidence after the samples {}, stopped {}",
        tally.same, tally.opposite, tally.partly, tally.early, tally.late, tally.stopped
    );
    eprintln!(
        "samples that can count: 8 to 15 in {} pairs, 16 to 31 in {}",
        tally.below, tally.above
    );
    eprintln!(
        "ledger: main {} units, the change {} units; the change asks more in {} pairs, the most: {:?}",
        tally.old_units,
        tally.new_units,
        tally.dearer.len(),
        tally.dearer.iter().take(5).collect::<Vec<_>>()
    );
    for line in tally.mismatches.iter().chain(&tally.lemma) {
        eprintln!("{line}");
    }
    assert!(tally.mismatches.is_empty(), "{:?}", tally.mismatches);
    assert!(tally.lemma.is_empty(), "{:?}", tally.lemma);
    // Not a vacuous comparison: each answer comes up.
    assert!(tally.same + tally.opposite > 0, "no pair lies on another");
    assert!(tally.partly > 0, "no pair lies on another over part");
    assert!(tally.early > 0, "no pair answered from the feet");
}
