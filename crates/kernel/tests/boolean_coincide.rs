//! Booleans of solids whose freeform faces lie on one another over a
//! region, though their surfaces part beyond the faces (2026-10-28, the
//! sea kayak's plates: docs/design/booleans.md › *Faces that lie on one
//! another over a region*).
//!
//! Every solid here is a hexahedron over a rectangle of the parameters of
//! one bicubic B-spline skin `S(u, v) = (L u, W v, z(u, v))` (its x and y
//! control coordinates are the Greville abscissae, so x and y are linear
//! in u and v and the iso-curves lie in the planes x = const and
//! y = const, which are the walls). Its floor and roof are each either a
//! plane z = const or the skin lifted by a height, and a face on the skin
//! is given one of several surfaces that all equal the skin over the
//! face: the skin itself, the skin cut down to the face and continued
//! past it as the functions its end spans are (*parted*: beyond the face
//! it leaves the skin, as a plate's rim traced past its ends does), the
//! same with its parameters exchanged or one of them reversed. So two
//! faces lie on one another wherever one is over the other, while their
//! surfaces, in the box of the two faces, do not.
//!
//! Closed forms: a slab between the skin and the skin lifted by `h` over
//! a rectangle `R` of the parameters holds `h L W |R|` exactly; a block
//! from a plane `z = c` up to the skin holds `L W ∫∫_R (z − c)`, a
//! polynomial on each span integrated exactly by Gauss–Legendre. At
//! 1e-3, 1 and 1e6, with the tolerance scaled.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_range_loop,
    clippy::type_complexity,
    clippy::assign_op_pattern
)]

mod boolean_common;

use boolean_common::{SCALES, assert_empty, mass, solve, tolerance};
use craft3d_geom::intersect::{SurfaceMeeting, meet_surfaces};
use craft3d_geom::{Curve3, Line, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_math::{Bbox3, Frame, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId, check};
use std::collections::BTreeMap;

const L: f64 = 4.0;
const W: f64 = 3.0;

const KNOTS_U: [f64; 11] = [0.0, 0.0, 0.0, 0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 1.0];
const KNOTS_V: [f64; 10] = [0.0, 0.0, 0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0];

/// The skin at scale `k`, lifted by `lift` (scale 1).
fn skin(k: f64, lift: f64) -> NurbsSurface {
    let greville = |knots: &[f64], i: usize| (knots[i + 1] + knots[i + 2] + knots[i + 3]) / 3.0;
    let (nu, nv) = (7, 6);
    let mut ctrl = Vec::new();
    for i in 0..nu {
        for j in 0..nv {
            let (fi, fj) = (i as f64, j as f64);
            let z = 0.8
                * (0.6 + 0.25 * (1.7 * fi + 0.9 * fj).sin() + 0.1 * (2.3 * fj - 0.4 * fi).cos());
            ctrl.push(Vec3::new(
                L * greville(&KNOTS_U, i) * k,
                W * greville(&KNOTS_V, j) * k,
                (z + lift) * k,
            ));
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

type Rect = ((f64, f64), (f64, f64));

/// The skin cut down to `rect` and continued over the whole domain again
/// as the functions its end spans are: equal to the skin over `rect`,
/// apart from it past the next knots.
fn parted(s: &NurbsSurface, rect: Rect, k: f64) -> NurbsSurface {
    s.restricted(rect.0, rect.1)
        .unwrap()
        .extended((0.0, 1.0), (0.0, 1.0), 1e-9 * k)
        .unwrap()
}

/// The same surface with u and v exchanged: `T(v, u) = S(u, v)`.
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

/// The same surface with u reversed: `R(a + b − u, v) = S(u, v)`.
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

/// A floor or a roof: the plane z = c (scale 1), or the skin lifted by a
/// height, its face on `surface` (which equals the lifted skin over the
/// rectangle).
#[derive(Clone)]
enum Level {
    Plane(f64),
    Skin { lift: f64, surface: NurbsSurface },
}

impl Level {
    fn on(k: f64, lift: f64) -> Level {
        Level::Skin {
            lift,
            surface: skin(k, lift),
        }
    }

    fn point(&self, k: f64, u: f64, v: f64) -> Vec3 {
        match self {
            Level::Plane(c) => Vec3::new(L * u * k, W * v * k, c * k),
            Level::Skin { lift, .. } => skin(k, *lift).eval(u, v),
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
            Level::Skin { lift, .. } => {
                // The skin cut down to the rectangle has clamped knots at
                // its ends: its border rows are the border curves, with
                // the skin's own parameters.
                let piece = skin(k, *lift).restricted(rect.0, rect.1).unwrap();
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

/// The hexahedron from `floor` up to `roof` over `rect`. Faces: floor 0,
/// roof 1, walls at u low 2, u high 3, v low 4, v high 5.
fn hexahedron(k: f64, floor: &Level, roof: &Level, rect: Rect) -> Body {
    let ((ua, ub), (va, vb)) = rect;
    let mut body = Body::new(BodyKind::Solid);
    let levels = [floor, roof];
    let us = [ua, ub];
    let vs = [va, vb];
    // Corners by (iu, iv, iz).
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
        // Counter-clockwise seen from outside (Newell's normal along the
        // outward one).
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
    let tol = tolerance(k);
    assert_eq!(check(&body, &tol), [], "a hexahedron at scale {k}");
    body
}

fn area(r: Rect) -> f64 {
    (r.0.1 - r.0.0) * (r.1.1 - r.1.0)
}

fn common(a: Rect, b: Rect) -> Rect {
    (
        (a.0.0.max(b.0.0), a.0.1.min(b.0.1)),
        (a.1.0.max(b.1.0), a.1.1.min(b.1.1)),
    )
}

/// `∫∫_rect (z(u, v) − c) du dv` of the skin (scale 1), exactly: three
/// Gauss–Legendre points per piece between the knots (a cubic in each).
fn under_skin(rect: Rect, c: f64) -> f64 {
    let s = skin(1.0, 0.0);
    let pieces = |(lo, hi): (f64, f64), knots: &[f64]| {
        let mut cuts = vec![lo];
        cuts.extend(knots.iter().copied().filter(|&t| t > lo && t < hi));
        cuts.push(hi);
        cuts.dedup();
        cuts
    };
    let g = (0.6_f64).sqrt();
    let nodes = [(-g, 5.0 / 9.0), (0.0, 8.0 / 9.0), (g, 5.0 / 9.0)];
    let (cu, cv) = (pieces(rect.0, &KNOTS_U), pieces(rect.1, &KNOTS_V));
    let mut sum = 0.0;
    for pu in cu.windows(2) {
        for pv in cv.windows(2) {
            let (hu, mu) = (0.5 * (pu[1] - pu[0]), 0.5 * (pu[1] + pu[0]));
            let (hv, mv) = (0.5 * (pv[1] - pv[0]), 0.5 * (pv[1] + pv[0]));
            for (x, wx) in nodes {
                for (y, wy) in nodes {
                    let z = s.eval(mu + hu * x, mv + hv * y).z;
                    sum += hu * hv * wx * wy * (z - c);
                }
            }
        }
    }
    sum
}

#[track_caller]
fn close(found: f64, expected: f64, k: f64, what: &str) {
    let slack = 1e-8 * expected.abs().max(k * k * k);
    assert!(
        (found - expected).abs() <= slack,
        "{what} at scale {k}: {found} is not {expected}"
    );
}

/// The box of face `f` of `body`.
fn face_box(body: &Body, f: u32) -> Bbox3 {
    body.face_bbox(FaceId(f)).unwrap()
}

/// That the two faces' surfaces, in the box both faces span, are not
/// found to be one surface: the case this file is about.
#[track_caller]
fn surfaces_part(a: &Body, fa: u32, b: &Body, fb: u32, k: f64) {
    let mut both = face_box(a, fa);
    let (lo, hi) = face_box(b, fb).corners().unwrap();
    both.add(lo);
    both.add(hi);
    let (sa, sb) = (&a.faces[fa as usize].surface, &b.faces[fb as usize].surface);
    let met = meet_surfaces(sa, sb, &both, &tolerance(k));
    assert!(
        !matches!(met, Ok(SurfaceMeeting::Coincident { .. })),
        "the surfaces are found to be one at scale {k}"
    );
}

const RA: Rect = ((0.1, 0.6), (0.15, 0.85));
const RB: Rect = ((0.4, 0.9), (0.3, 0.7));
const H: f64 = 0.05;

/// Two plates on one skin, one on top of the other where they overlap,
/// the upper one's floor and the lower one's roof each on the lifted skin
/// parted past its own face: they touch over the rectangle both cover.
/// The intersection is empty, the union holds both, the difference is the
/// target.
#[test]
fn plates_stacked_on_one_skin_touch() {
    for k in SCALES {
        let a = hexahedron(
            k,
            &Level::on(k, 0.0),
            &Level::Skin {
                lift: H,
                surface: parted(&skin(k, H), RA, k),
            },
            RA,
        );
        let b = hexahedron(
            k,
            &Level::Skin {
                lift: H,
                surface: parted(&skin(k, H), RB, k),
            },
            &Level::on(k, 2.0 * H),
            RB,
        );
        surfaces_part(&a, 1, &b, 0, k);
        let (va, vb) = (H * L * W * area(RA), H * L * W * area(RB));
        let k3 = k * k * k;
        close(mass(&a).volume, va * k3, k, "the lower plate");
        close(mass(&b).volume, vb * k3, k, "the upper plate");
        assert_empty(&a, &b, Intersect, k);
        assert_empty(&b, &a, Intersect, k);
        let union = solve(&a, &b, Union, k);
        assert_eq!(union.body.shells.len(), 1, "one solid at scale {k}");
        close(mass(&union.body).volume, (va + vb) * k3, k, "the union");
        let less = solve(&a, &b, Subtract, k);
        close(
            mass(&less.body).volume,
            va * k3,
            k,
            "the lower less the upper",
        );
        let less = solve(&b, &a, Subtract, k);
        close(
            mass(&less.body).volume,
            vb * k3,
            k,
            "the upper less the lower",
        );
    }
}

/// Two plates side by side on one skin that overlap over a known
/// rectangle, the second one's floor and roof parted past it: the
/// intersection is the slab over the common rectangle, the union over
/// both, the difference over what the first has alone.
#[test]
fn plates_side_by_side_on_one_skin_overlap() {
    for k in SCALES {
        let a = hexahedron(k, &Level::on(k, 0.0), &Level::on(k, H), RA);
        let b = hexahedron(
            k,
            &Level::Skin {
                lift: 0.0,
                surface: parted(&skin(k, 0.0), RB, k),
            },
            &Level::Skin {
                lift: H,
                surface: transposed(&parted(&skin(k, H), RB, k)),
            },
            RB,
        );
        surfaces_part(&a, 0, &b, 0, k);
        surfaces_part(&a, 1, &b, 1, k);
        let c = common(RA, RB);
        let slab = H * L * W;
        let k3 = k * k * k;
        for (x, y) in [(&a, &b), (&b, &a)] {
            let both = solve(x, y, Intersect, k);
            close(
                mass(&both.body).volume,
                slab * area(c) * k3,
                k,
                "the intersection",
            );
            let union = solve(x, y, Union, k);
            close(
                mass(&union.body).volume,
                slab * (area(RA) + area(RB) - area(c)) * k3,
                k,
                "the union",
            );
        }
        let less = solve(&a, &b, Subtract, k);
        close(
            mass(&less.body).volume,
            slab * (area(RA) - area(c)) * k3,
            k,
            "A less B",
        );
        let less = solve(&b, &a, Subtract, k);
        close(
            mass(&less.body).volume,
            slab * (area(RB) - area(c)) * k3,
            k,
            "B less A",
        );
    }
}

/// A block with the skin for its roof, and a second block on a higher
/// floor whose roof is the skin parted past it with u reversed, over a
/// rectangle that overlaps the first: the roofs share the common part.
/// Volumes from the skin's integral.
#[test]
fn blocks_that_share_part_of_a_freeform_roof() {
    for k in SCALES {
        let a = hexahedron(k, &Level::Plane(-0.5), &Level::on(k, 0.0), RA);
        let b = hexahedron(
            k,
            &Level::Plane(-0.3),
            &Level::Skin {
                lift: 0.0,
                surface: reversed_u(&parted(&skin(k, 0.0), RB, k)),
            },
            RB,
        );
        surfaces_part(&a, 1, &b, 1, k);
        let c = common(RA, RB);
        let lw = L * W;
        let (va, vb) = (lw * under_skin(RA, -0.5), lw * under_skin(RB, -0.3));
        let vc = lw * under_skin(c, -0.3);
        let k3 = k * k * k;
        close(mass(&a).volume, va * k3, k, "A");
        close(mass(&b).volume, vb * k3, k, "B");
        let both = solve(&a, &b, Intersect, k);
        close(mass(&both.body).volume, vc * k3, k, "the intersection");
        let union = solve(&a, &b, Union, k);
        close(
            mass(&union.body).volume,
            (va + vb - vc) * k3,
            k,
            "the union",
        );
        let less = solve(&a, &b, Subtract, k);
        close(mass(&less.body).volume, (va - vc) * k3, k, "A less B");
        let less = solve(&b, &a, Subtract, k);
        close(mass(&less.body).volume, (vb - vc) * k3, k, "B less A");
    }
}

/// The same skin in two other parametrisations (u and v exchanged; u
/// reversed), whole: the roofs are one surface in the box, as before.
#[test]
fn blocks_under_one_skin_in_two_parametrisations() {
    for k in SCALES {
        let a = hexahedron(k, &Level::Plane(-0.5), &Level::on(k, 0.0), RA);
        for surface in [transposed(&skin(k, 0.0)), reversed_u(&skin(k, 0.0))] {
            let b = hexahedron(
                k,
                &Level::Plane(-0.3),
                &Level::Skin { lift: 0.0, surface },
                RB,
            );
            let c = common(RA, RB);
            let lw = L * W;
            let (va, vb) = (lw * under_skin(RA, -0.5), lw * under_skin(RB, -0.3));
            let vc = lw * under_skin(c, -0.3);
            let k3 = k * k * k;
            let both = solve(&a, &b, Intersect, k);
            close(mass(&both.body).volume, vc * k3, k, "the intersection");
            let union = solve(&a, &b, Union, k);
            close(
                mass(&union.body).volume,
                (va + vb - vc) * k3,
                k,
                "the union",
            );
            let less = solve(&a, &b, Subtract, k);
            close(mass(&less.body).volume, (va - vc) * k3, k, "A less B");
        }
    }
}

/// Faces that lie on one another over part of where one is over the
/// other and leave each other inside both: a second block whose roof is
/// the skin over one half of the common rectangle and bends away from it
/// over the other (a roof cut down to its first spans and continued
/// past them, where the skin goes on in other spans). Refused with the
/// reason, never guessed.
#[test]
fn faces_that_part_inside_both_are_refused() {
    for k in SCALES {
        let a = hexahedron(k, &Level::Plane(-0.5), &Level::on(k, 0.0), RA);
        // Equal to the skin up to u = 0.25 (the first span), continued
        // past it by that span's cubic, over a block from 0.2 to 0.55.
        let bent = parted(&skin(k, 0.0), ((0.0, 0.25), (0.0, 1.0)), k);
        let rect = ((0.2, 0.55), (0.3, 0.7));
        let b = bent_block(k, &bent, rect);
        let found = craft3d_kernel::boolean::boolean(&a, &b, Intersect, &tolerance(k));
        match found {
            Err(e) => assert!(
                e.to_string().contains("lie on one another over part"),
                "at scale {k}: {e}"
            ),
            Ok(r) => panic!("at scale {k}: built, volume {}", mass(&r.body).volume),
        }
    }
}

/// A block from z = −0.3 up to `roof` (a surface with x and y linear in
/// u and v, as the skin's) over `rect`, its edges iso-curves of `roof`.
fn bent_block(k: f64, roof: &NurbsSurface, rect: Rect) -> Body {
    block_on(k, roof, rect, -0.3)
}

/// A block between the plane z = `plane` (scale 1) and `roof` (a surface
/// with x and y linear in u and v, as the skin's, above or below the
/// plane) over `rect`, its edges iso-curves of `roof`.
fn block_on(k: f64, roof: &NurbsSurface, rect: Rect, plane: f64) -> Body {
    // (`hexahedron` takes its edges from the skin, which the bent roof
    // leaves.)
    let ((ua, ub), (va, vb)) = rect;
    let piece = roof.restricted(rect.0, rect.1).unwrap();
    let (nu, nv) = piece.counts();
    let c = piece.control_points().to_vec();
    let wt = piece.weights().to_vec();
    let mut body = Body::new(BodyKind::Solid);
    let floor = |u: f64, v: f64| Vec3::new(L * u * k, W * v * k, plane * k);
    let top = |u: f64, v: f64| roof.eval(u, v);
    let us = [ua, ub];
    let vs = [va, vb];
    let mut corner = BTreeMap::new();
    for iu in 0..2 {
        for iv in 0..2 {
            corner.insert((iu, iv, 0), body.add_vertex(floor(us[iu], vs[iv])));
            corner.insert((iu, iv, 1), body.add_vertex(top(us[iu], vs[iv])));
        }
    }
    let mut edges: BTreeMap<((usize, usize, usize), (usize, usize, usize)), EdgeId> =
        BTreeMap::new();
    let line = |p: Vec3, q: Vec3| {
        (
            Curve3::Line(Line::through(p, q).unwrap()),
            (0.0, p.distance(q)),
        )
    };
    for iv in 0..2 {
        let (curve, range) = line(floor(ua, vs[iv]), floor(ub, vs[iv]));
        edges.insert(
            ((0, iv, 0), (1, iv, 0)),
            body.add_edge(curve, range, corner[&(0, iv, 0)], corner[&(1, iv, 0)]),
        );
        let j = if iv == 0 { 0 } else { nv - 1 };
        let pts: Vec<Vec3> = (0..nu).map(|i| c[i * nv + j]).collect();
        let ws: Vec<f64> = (0..nu).map(|i| wt[i * nv + j]).collect();
        let curve = Curve3::Nurbs(NurbsCurve::new(3, piece.knots_u().to_vec(), pts, ws).unwrap());
        edges.insert(
            ((0, iv, 1), (1, iv, 1)),
            body.add_edge(curve, (ua, ub), corner[&(0, iv, 1)], corner[&(1, iv, 1)]),
        );
    }
    for iu in 0..2 {
        let (curve, range) = line(floor(us[iu], va), floor(us[iu], vb));
        edges.insert(
            ((iu, 0, 0), (iu, 1, 0)),
            body.add_edge(curve, range, corner[&(iu, 0, 0)], corner[&(iu, 1, 0)]),
        );
        let i = if iu == 0 { 0 } else { nu - 1 };
        let pts: Vec<Vec3> = (0..nv).map(|j| c[i * nv + j]).collect();
        let ws: Vec<f64> = (0..nv).map(|j| wt[i * nv + j]).collect();
        let curve = Curve3::Nurbs(NurbsCurve::new(3, piece.knots_v().to_vec(), pts, ws).unwrap());
        edges.insert(
            ((iu, 0, 1), (iu, 1, 1)),
            body.add_edge(curve, (va, vb), corner[&(iu, 0, 1)], corner[&(iu, 1, 1)]),
        );
    }
    for iu in 0..2 {
        for iv in 0..2 {
            let (curve, range) = line(floor(us[iu], vs[iv]), top(us[iu], vs[iv]));
            edges.insert(
                ((iu, iv, 0), (iu, iv, 1)),
                body.add_edge(curve, range, corner[&(iu, iv, 0)], corner[&(iu, iv, 1)]),
            );
        }
    }
    let point = |c: (usize, usize, usize)| {
        if c.2 == 0 {
            floor(us[c.0], vs[c.1])
        } else {
            top(us[c.0], vs[c.1])
        }
    };
    let middle = roof.eval(0.5 * (ua + ub), 0.5 * (va + vb));
    let up = if middle.z > plane * k {
        Vec3::Z
    } else {
        -Vec3::Z
    };
    let quads: [([(usize, usize, usize); 4], Vec3); 6] = [
        ([(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)], -up),
        ([(0, 0, 1), (1, 0, 1), (1, 1, 1), (0, 1, 1)], up),
        ([(0, 0, 0), (0, 1, 0), (0, 1, 1), (0, 0, 1)], -Vec3::X),
        ([(1, 0, 0), (1, 1, 0), (1, 1, 1), (1, 0, 1)], Vec3::X),
        ([(0, 0, 0), (1, 0, 0), (1, 0, 1), (0, 0, 1)], -Vec3::Y),
        ([(0, 1, 0), (1, 1, 0), (1, 1, 1), (0, 1, 1)], Vec3::Y),
    ];
    let mut faces = Vec::new();
    for (index, (quad, outward)) in quads.iter().enumerate() {
        let mut cycle = quad.to_vec();
        let mut newell = Vec3::ZERO;
        for i in 0..4 {
            newell = newell + point(cycle[i]).cross(point(cycle[(i + 1) % 4]));
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
            0 => {
                let frame = Frame::new(Vec3::new(0.0, 0.0, plane * k), *outward, Vec3::X).unwrap();
                (Surface::Plane(Plane::new(frame)), false)
            }
            1 => {
                let surface = Surface::Nurbs(roof.clone());
                let n = surface.normal(0.5 * (ua + ub), 0.5 * (va + vb)).unwrap();
                (surface, n.dot(*outward) < 0.0)
            }
            _ => {
                let hint = if outward.x != 0.0 { Vec3::Y } else { Vec3::X };
                let frame = Frame::new(point(quad[0]), *outward, hint).unwrap();
                (Surface::Plane(Plane::new(frame)), false)
            }
        };
        faces.push(body.add_face(surface, reversed, vec![l]));
    }
    body.add_shell(faces);
    assert_eq!(
        check(&body, &tolerance(k)),
        [],
        "the bent block at scale {k}"
    );
    body
}

/// A flat bicubic B-spline over [0, 1]² at z = 0 with `count` control
/// points each way (x and y at the Greville abscissae), the middle one
/// lifted by `lift` (scale 1) and given the weight `weight`.
fn flat(k: f64, count: usize, lift: f64, weight: f64) -> NurbsSurface {
    let spans = count - 3;
    let mut knots = vec![0.0; 4];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend([1.0; 4]);
    let greville = |i: usize| (knots[i + 1] + knots[i + 2] + knots[i + 3]) / 3.0;
    let middle = count / 2;
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for i in 0..count {
        for j in 0..count {
            let spike = (i, j) == (middle, middle);
            let z = if spike { lift } else { 0.0 };
            ctrl.push(Vec3::new(L * greville(i) * k, W * greville(j) * k, z * k));
            weights.push(if spike { weight } else { 1.0 });
        }
    }
    NurbsSurface::new(
        3,
        3,
        knots.clone(),
        knots.clone(),
        count,
        count,
        ctrl,
        weights,
    )
    .unwrap()
}

/// A block whose floor is a flat rational patch with a narrow spike (the
/// middle control point lifted by ten tolerances with a weight of 10⁴:
/// in space the patch is pulled into a spike at that point, a dent in the
/// block ten tolerances high), standing on a block with a flat top: a
/// plane, or a coarse flat patch (one span). Away from the spike the two
/// lie on one another; at the spike they are apart, and the dent is a
/// cavity of the union. That must be refused, or told apart (the union
/// keeps the cavity, the intersection is empty), never taken for faces
/// on one another (the review's negative case).
#[test]
fn a_narrow_spike_on_a_face_is_not_taken_for_coincidence() {
    for k in SCALES {
        let tol = tolerance(k);
        let spike = flat(k, 11, 10.0 * 1e-6, 1e4);
        // The spike's support is u, v in [1/4, 3/4]: the block's walls,
        // at 0.15 and 0.85, are clear of it and planar.
        let upper = block_on(k, &spike, ((0.15, 0.85), (0.15, 0.85)), 0.3);
        let below = [
            hexahedron(
                k,
                &Level::Plane(-0.5),
                &Level::Plane(0.0),
                ((0.1, 0.9), (0.1, 0.9)),
            ),
            block_on(k, &flat(k, 4, 0.0, 1.0), ((0.1, 0.9), (0.1, 0.9)), -0.5),
        ];
        for (n, lower) in below.iter().enumerate() {
            let what = format!("at scale {k}, lower block {n}");
            match craft3d_kernel::boolean::boolean(lower, &upper, Union, &tol) {
                Ok(r) => assert!(
                    r.body.shells.len() >= 2,
                    "{what}: the union lost the dent ({} shells)",
                    r.body.shells.len()
                ),
                Err(e) => eprintln!("{what}: union refused: {e}"),
            }
            match craft3d_kernel::boolean::boolean(lower, &upper, Intersect, &tol) {
                Ok(r) => panic!("{what}: an intersection of volume {}", mass(&r.body).volume),
                Err(e) => eprintln!("{what}: intersection: {e}"),
            }
        }
    }
}
