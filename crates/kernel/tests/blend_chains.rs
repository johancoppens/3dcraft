//! Blends along chains of edges that go on smoothly into each other, whose
//! faces have no closed form (kernel::blend's `rolling`, `chain`): the
//! cut edge of a casing that runs from a cone into the torus of a root
//! fillet and on along a flange.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;

use boolean_common::{SCALES, boxed, mass, tolerance};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::fillet::{FilletEdge, FilletFace, fillet};
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check};

fn edge_near(body: &Body, p: Vec3) -> EdgeId {
    let (i, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let d = (0..=32)
                .map(|j| {
                    let t = e.range.0 + (e.range.1 - e.range.0) * f64::from(j) / 32.0;
                    e.curve.eval(t).distance(p)
                })
                .fold(f64::INFINITY, f64::min);
            (i, d)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    EdgeId(i as u32)
}

fn faces_of(body: &Body, edge: EdgeId) -> Vec<FaceId> {
    (0..body.faces.len())
        .filter(|i| {
            body.faces[*i]
                .loops
                .iter()
                .flat_map(|l| &body.loop_(*l).unwrap().coedges)
                .any(|c| c.edge == edge)
        })
        .map(|i| FaceId(i as u32))
        .collect()
}

fn normal_at(body: &Body, face: FaceId, point: Vec3) -> Vec3 {
    let face = body.face(face).unwrap();
    let (u, v) = face.surface.project(point).unwrap();
    let n = face.surface.normal(u, v).unwrap();
    if face.reversed { -n } else { n }
}

/// The meridian of the casing in the (ρ, y) half-plane: a bottom, the
/// outer cone, the root fillet (an arc of radius `root`), the underside
/// of the flange, its rim, its top and the inner cone back. Returns the
/// polygon's corners with the fillet's tangent points and centre.
struct Meridian {
    /// Bottom inner, bottom outer, fillet start (on the cone), fillet end
    /// (on the flange), flange outer bottom, flange outer top, top inner.
    points: [[f64; 2]; 7],
    centre: [f64; 2],
    root: f64,
}

fn meridian(k: f64) -> Meridian {
    let corner: [f64; 2] = [4.33789, 15.9];
    let low: [f64; 2] = [4.99316, 7.6];
    let root: f64 = 0.06;
    let d1 = {
        let (x, y) = (low[0] - corner[0], low[1] - corner[1]);
        let l = x.hypot(y);
        [x / l, y / l]
    };
    let d2: [f64; 2] = [1.0, 0.0];
    let cos = d1[0] * d2[0] + d1[1] * d2[1];
    let half = 0.5 * cos.acos();
    let along = root / half.tan();
    let bis = {
        let (x, y) = (d1[0] + d2[0], d1[1] + d2[1]);
        let l = x.hypot(y);
        [x / l, y / l]
    };
    let out = root / half.sin();
    let s = |p: [f64; 2]| [p[0] * k, p[1] * k];
    Meridian {
        points: [
            s([4.74316, 7.6]),
            s(low),
            s([corner[0] + d1[0] * along, corner[1] + d1[1] * along]),
            s([corner[0] + d2[0] * along, corner[1] + d2[1] * along]),
            s([5.51513, 15.9]),
            s([5.51513, 16.2]),
            s([4.06421, 16.2]),
        ],
        centre: s([corner[0] + bis[0] * out, corner[1] + bis[1] * out]),
        root: root * k,
    }
}

/// The casing revolved about y, kept where z ≥ 0: two cut faces in the
/// plane z = 0, the one at x > 0 the meridian itself.
fn half_casing(k: f64) -> (Body, Meridian) {
    let m = meridian(k);
    let p = m.points;
    let line = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
        id,
        curve: Curve2::Line { start: a, end: b },
    };
    let curves = vec![
        line(1, p[0], p[1]),
        line(2, p[1], p[2]),
        SketchCurve {
            id: 3,
            curve: Curve2::Arc {
                center: m.centre,
                start: p[3],
                end: p[2],
            },
        },
        line(4, p[3], p[4]),
        line(5, p[4], p[5]),
        line(6, p[5], p[6]),
        line(7, p[6], p[0]),
    ];
    let tol = tolerance(k);
    let profiles = detect_profiles(&curves, &tol).unwrap();
    let ring = revolve(
        &profiles,
        &Frame::WORLD,
        [0.0, 0.0],
        [0.0, 1.0],
        0.0,
        std::f64::consts::TAU,
        &tol,
    )
    .unwrap()
    .body;
    let half = boxed(k, [-10.0, 0.0, 0.0], [10.0, 20.0, 10.0]);
    let body = boolean(&ring, &half, BooleanOp::Intersect, &tol)
        .unwrap()
        .body;
    (body, m)
}

/// The three edges of the chain on the cut face at x > 0: the cone's, the
/// fillet's and the flange's.
fn cut_chain(body: &Body, m: &Meridian) -> [EdgeId; 3] {
    let p = m.points;
    let mid = |a: [f64; 2], b: [f64; 2]| Vec3::new(0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1]), 0.0);
    let arc = {
        let (cx, cy) = (m.centre[0], m.centre[1]);
        let (a, b) = ([p[2][0] - cx, p[2][1] - cy], [p[3][0] - cx, p[3][1] - cy]);
        let (x, y) = (a[0] + b[0], a[1] + b[1]);
        let l = x.hypot(y);
        Vec3::new(cx + x / l * m.root, cy + y / l * m.root, 0.0)
    };
    [
        edge_near(body, mid(p[1], p[2])),
        edge_near(body, arc),
        edge_near(body, mid(p[3], p[4])),
    ]
}

/// The largest sine of the angle between the normals across every edge a
/// fillet face shares with a face it runs into smoothly (its borders
/// along the faces it rounds, and the joints between the fillet faces of
/// a chain; told by the normals agreeing to 1e-3), and how many such
/// edges there are. The cuts at the ends are creases by design.
fn creases(body: &Body, faces: &[FilletFace]) -> (f64, usize) {
    let mut worst = 0.0_f64;
    let mut seams = 0;
    for (index, edge) in body.edges.iter().enumerate() {
        let users = faces_of(body, EdgeId(index as u32));
        let [a, b] = users[..] else { continue };
        let fillet = |f: FaceId| matches!(faces[f.0 as usize], FilletFace::Fillet(_));
        if !fillet(a) && !fillet(b) {
            continue;
        }
        let mut here = 0.0_f64;
        let mut same_sense = true;
        for i in 0..17 {
            let s = (f64::from(i) + 0.5) / 17.0;
            let p = edge
                .curve
                .eval(edge.range.0 + s * (edge.range.1 - edge.range.0));
            let (n, m) = (normal_at(body, a, p), normal_at(body, b, p));
            same_sense &= n.dot(m) > 0.0;
            here = here.max(n.cross(m).length());
        }
        if here < 1e-3 {
            assert!(same_sense, "normals of opposite sense across a seam");
            seams += 1;
            worst = worst.max(here);
        }
    }
    (worst, seams)
}

/// The distance in the meridian half-plane from the point (ρ, y) to the
/// wall `wall` of the casing continued: 0 the cone's line, 1 the root
/// fillet's circle, 2 the flange's line.
fn to_wall(m: &Meridian, wall: usize, rho: f64, y: f64) -> f64 {
    let p = m.points;
    match wall {
        0 => {
            let (a, b) = (p[1], p[2]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            ((rho - a[0]) * dy - (y - a[1]) * dx).abs() / dx.hypot(dy)
        }
        1 => ((rho - m.centre[0]).hypot(y - m.centre[1]) - m.root).abs(),
        _ => (y - p[3][1]).abs(),
    }
}

/// Every point of every fillet face (sampled over its surface, which runs
/// on past the face where it is cut at an end: still the same family of
/// balls) is the radius from the centre of a ball that touches the cut
/// plane and the wall of its edge: the centre, the radius in along the
/// face's normal, is the radius from both.
fn by_definition(made: &craft3d_kernel::fillet::FilletResult, m: &Meridian, r: f64) -> f64 {
    let mut worst = 0.0_f64;
    let mut seen = 0;
    for (index, source) in made.faces.iter().enumerate() {
        let FilletFace::Fillet(of) = source else { continue };
        let wall = of[0];
        let face = &made.body.faces[index];
        let craft3d_geom::Surface::Nurbs(surface) = &face.surface else {
            panic!("a fillet face that is not a spline surface");
        };
        let ((u0, u1), (v0, v1)) = surface.domain();
        for i in 0..=24 {
            for j in 0..=8 {
                let u = u0 + (u1 - u0) * f64::from(i) / 24.0;
                let v = v0 + (v1 - v0) * f64::from(j) / 8.0;
                let (q, du, dv) = surface.derivatives(u, v);
                let n = du.cross(dv).normalized().unwrap();
                let n = if face.reversed { -n } else { n };
                let c = q - n * r;
                let off_plane = (c.z - r).abs();
                let off_wall = (to_wall(m, wall, c.x.hypot(c.z), c.y) - r).abs();
                worst = worst.max(off_plane).max(off_wall);
            }
        }
        seen += 1;
    }
    assert_eq!(seen, 3, "a fillet face per edge of the chain");
    worst
}

/// The casing's cut edge runs from the cone into the torus of the root
/// fillet and on along the underside of the flange: one fillet along the
/// three, a face per edge, the faces at the two joints changing from the
/// cone to the torus and from the torus to the flange.
#[test]
fn the_cut_edge_of_a_casing_through_its_root_fillet() {
    for k in SCALES {
        let tol = tolerance(k);
        let (body, m) = half_casing(k);
        assert_eq!(check(&body, &tol), []);
        let chain = cut_chain(&body, &m);
        let r = 0.05 * k;
        let edges: Vec<FilletEdge> = chain
            .iter()
            .map(|e| FilletEdge {
                edge: *e,
                radius: r,
            })
            .collect();
        let made = match fillet(&body, &edges, &tol) {
            Ok(m) => m,
            Err(e) => panic!("scale {k}: {e}"),
        };
        assert_eq!(check(&made.body, &tol), []);
        let off = by_definition(&made, &m, r);
        let (crease, seams) = creases(&made.body, &made.faces);
        let removed = mass(&body).volume - mass(&made.body).volume;
        println!(
            "scale {k}: off the definition {:e}, largest crease {crease:e} over {seams} seams, removed {:e}",
            off / tol.linear,
            removed / k.powi(3)
        );
        // Three faces, each tangent to the cut plane and to its wall, and
        // to the next face across the two joints.
        assert_eq!(seams, 8);
        assert!(off <= 2.0 * tol.linear, "{off} off the rolling ball");
        assert!(crease <= 10.0 * tol.linear / r, "a crease of {crease}");
        // A quarter disc's corner along the chain, about 9.5 long.
        let corner = (1.0 - std::f64::consts::FRAC_PI_4) * 0.05 * 0.05;
        assert!(removed > 9.0 * corner * k.powi(3) && removed < 10.0 * corner * k.powi(3));
    }
}

/// A cubic Bézier in the plane.
#[derive(Clone, Copy)]
struct Bez([[f64; 2]; 4]);

impl Bez {
    fn d(&self, t: f64) -> [f64; 2] {
        let s = 1.0 - t;
        let p = self.0;
        let c = [3.0 * s * s, 6.0 * s * t, 3.0 * t * t];
        let q = |i: usize, k: usize| p[i + 1][k] - p[i][k];
        [
            c[0] * q(0, 0) + c[1] * q(1, 0) + c[2] * q(2, 0),
            c[0] * q(0, 1) + c[1] * q(1, 1) + c[2] * q(2, 1),
        ]
    }

    /// Its length, by Gauss-Legendre over 64 pieces.
    fn length(&self) -> f64 {
        let nodes = [
            (-0.906_179_845_938_664, 0.236_926_885_056_189),
            (-0.538_469_310_105_683, 0.478_628_670_499_366),
            (0.0, 0.568_888_888_888_889),
            (0.538_469_310_105_683, 0.478_628_670_499_366),
            (0.906_179_845_938_664, 0.236_926_885_056_189),
        ];
        let n = 64;
        let mut sum = 0.0;
        for i in 0..n {
            let (a, b) = (f64::from(i) / f64::from(n), f64::from(i + 1) / f64::from(n));
            for (x, w) in nodes {
                let d = self.d(0.5 * (a + b) + 0.5 * (b - a) * x);
                sum += 0.5 * (b - a) * w * d[0].hypot(d[1]);
            }
        }
        sum
    }

    fn scaled(&self, k: f64) -> Bez {
        Bez(self.0.map(|p| [p[0] * k, p[1] * k]))
    }
}

/// What a fillet of radius `r` along the rim of the cap of a prism whose
/// walls stand square on it takes away, when the rim is `length` long and
/// turns by `turning` (counter-clockwise seen from outside the cap, the
/// material to the left): in every plane across the rim the corner of a
/// square of side r less a quarter of the disc, swept along the rim; a
/// strip at distance w in from the wall is shortened by the factor
/// (1 − κw) where the rim curves by κ, so the volume is the area times the
/// length less the section's first moment about the wall times the
/// turning (Pappus-Guldinus along a plane curve). The ends, if any, have
/// to be square to the rim.
fn swept_corner(r: f64, length: f64, turning: f64) -> f64 {
    let area = r * r * (1.0 - std::f64::consts::FRAC_PI_4);
    let moment = r * r * r * (5.0 / 6.0 - std::f64::consts::FRAC_PI_4);
    area * length - moment * turning
}

fn bezier(id: u64, b: &Bez) -> SketchCurve {
    use craft3d_kernel::profile::Spline2;
    SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control: b.0.to_vec(),
        }),
    }
}

/// The prism of `curves` in `frame` from 0 to `height`.
fn prism(curves: &[SketchCurve], frame: &Frame, height: f64, k: f64) -> Body {
    use craft3d_kernel::extrude::extrude;
    let tol = tolerance(k);
    let profiles = detect_profiles(curves, &tol).unwrap();
    extrude(&profiles, frame, 0.0, height, &tol).unwrap().body
}

/// The edges of the top cap's rim near the given points of the outline.
fn rim(body: &Body, frame: &Frame, height: f64, points: &[[f64; 2]]) -> Vec<EdgeId> {
    points
        .iter()
        .map(|p| edge_near(body, frame.point(p[0], p[1], height)))
        .collect()
}

/// A smooth closed outline of a line, a half circle and a Bézier, all
/// tangent where they meet, extruded: its top rim is one chain of three
/// edges between the cap and three walls (a plane, a cylinder, a spline
/// surface), each joint a place where the wall changes across a smooth
/// edge. Rounded, it loses exactly the swept corner over the whole rim,
/// which turns once round.
#[test]
fn the_rim_of_a_smooth_outline_all_round() {
    let (a, b) = (1.0, 1.0);
    let s = Bez([[a, b], [a - 3.0, b], [-a - 3.0, -b], [-a, -b]]);
    let r = 0.1;
    let height = 2.0;
    let length = 2.0 * a + std::f64::consts::PI * b + s.length();
    let tilted = Frame::new(
        Vec3::new(0.3, -0.2, 0.1),
        Vec3::new(0.2, -0.3, 1.0),
        Vec3::new(1.0, 0.4, 0.0),
    )
    .unwrap();
    for (k, frame) in SCALES
        .iter()
        .map(|k| (*k, Frame::WORLD))
        .chain([(1.0, tilted)])
    {
        let tol = tolerance(k);
        let frame = frame.at(frame.origin() * k).unwrap();
        let curves = vec![
            SketchCurve {
                id: 1,
                curve: Curve2::Line {
                    start: [-a * k, -b * k],
                    end: [a * k, -b * k],
                },
            },
            SketchCurve {
                id: 2,
                curve: Curve2::Arc {
                    center: [a * k, 0.0],
                    start: [a * k, -b * k],
                    end: [a * k, b * k],
                },
            },
            bezier(3, &s.scaled(k)),
        ];
        let body = prism(&curves, &frame, height * k, k);
        let chain = rim(
            &body,
            &frame,
            height * k,
            &[[0.0, -b * k], [(a + b) * k, 0.0], [-2.25 * k, 0.0]],
        );
        let edges: Vec<FilletEdge> = chain
            .iter()
            .map(|e| FilletEdge {
                edge: *e,
                radius: r * k,
            })
            .collect();
        let made = match fillet(&body, &edges, &tol) {
            Ok(m) => m,
            Err(e) => panic!("scale {k}: {e}"),
        };
        assert_eq!(check(&made.body, &tol), []);
        let removed = mass(&body).volume - mass(&made.body).volume;
        let expected = swept_corner(r * k, length * k, std::f64::consts::TAU);
        let (crease, seams) = creases(&made.body, &made.faces);
        println!(
            "scale {k}: removed {removed:e}, expected {expected:e} (off by {:e} of the tolerance times the fillet's area), crease {crease:e} over {seams}",
            (removed - expected) / (tol.linear * length * k * r * k * std::f64::consts::FRAC_PI_2)
        );
        // Three faces with two borders each, and three joints.
        assert_eq!(seams, 9);
        assert!(crease <= 10.0 * tol.linear / (r * k), "a crease of {crease}");
        // The fillet faces are within a quarter of the tolerance of the
        // true surface, whose area is the quarter cylinder's along the
        // rim, about.
        let bound = 0.25 * tol.linear * length * k * r * k * std::f64::consts::FRAC_PI_2;
        assert!(
            (removed - expected).abs() <= bound,
            "removed {removed} instead of {expected}, more than {bound} off"
        );
    }
}

/// An open outline: a line, a half circle and a Bézier, tangent where they
/// meet, closed by a straight back that is square to both ends of that
/// chain. Its top rim from corner to corner is one chain; the fillet ends
/// in the back wall, which lies in the plane across the chain at both
/// ends, so it loses exactly the swept corner, turning by half a turn.
#[test]
fn a_chain_from_corner_to_corner() {
    let (l1, rr, h) = (2.0, 1.0, 3.0);
    let s = Bez([[l1, 2.0 * rr], [1.2, 2.0 * rr], [0.8, h], [0.0, h]]);
    let r = 0.1;
    let height = 1.5;
    let length = l1 + std::f64::consts::PI * rr + s.length();
    for k in SCALES {
        let tol = tolerance(k);
        let frame = Frame::WORLD;
        let line = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
            id,
            curve: Curve2::Line {
                start: [a[0] * k, a[1] * k],
                end: [b[0] * k, b[1] * k],
            },
        };
        let curves = vec![
            line(1, [0.0, 0.0], [l1, 0.0]),
            SketchCurve {
                id: 2,
                curve: Curve2::Arc {
                    center: [l1 * k, rr * k],
                    start: [l1 * k, 0.0],
                    end: [l1 * k, 2.0 * rr * k],
                },
            },
            bezier(3, &s.scaled(k)),
            line(4, [0.0, h], [0.0, 0.0]),
        ];
        let body = prism(&curves, &frame, height * k, k);
        let chain = rim(
            &body,
            &frame,
            height * k,
            &[[0.5 * l1 * k, 0.0], [(l1 + rr) * k, rr * k], [k, 2.5 * k]],
        );
        let edges: Vec<FilletEdge> = chain
            .iter()
            .map(|e| FilletEdge {
                edge: *e,
                radius: r * k,
            })
            .collect();
        let made = match fillet(&body, &edges, &tol) {
            Ok(m) => m,
            Err(e) => panic!("scale {k}: {e}"),
        };
        assert_eq!(check(&made.body, &tol), []);
        let removed = mass(&body).volume - mass(&made.body).volume;
        let expected = swept_corner(r * k, length * k, std::f64::consts::PI);
        let (crease, seams) = creases(&made.body, &made.faces);
        println!(
            "scale {k}: removed {removed:e}, expected {expected:e} (off by {:e} of the tolerance times the fillet's area), crease {crease:e} over {seams}",
            (removed - expected) / (tol.linear * length * k * r * k * std::f64::consts::FRAC_PI_2)
        );
        assert_eq!(seams, 8);
        assert!(crease <= 10.0 * tol.linear / (r * k), "a crease of {crease}");
        // The fillet faces are within a quarter of the tolerance of the
        // true surface, whose area is the quarter cylinder's along the
        // rim, about.
        let bound = 0.25 * tol.linear * length * k * r * k * std::f64::consts::FRAC_PI_2;
        assert!(
            (removed - expected).abs() <= bound,
            "removed {removed} instead of {expected}, more than {bound} off"
        );
    }
}

/// A freeform chain and an edge of the closed forms in one fillet: the
/// rim of the open outline above and the bottom edge of its back, far
/// from it. What they take away together is what each takes on its own,
/// and every fillet face says which edge it is of.
#[test]
fn a_chain_and_a_straight_edge_in_one_fillet() {
    let (l1, rr, h) = (2.0, 1.0, 3.0);
    let s = Bez([[l1, 2.0 * rr], [1.2, 2.0 * rr], [0.8, h], [0.0, h]]);
    let (r, height) = (0.1, 1.5);
    for k in SCALES {
        let tol = tolerance(k);
        let frame = Frame::WORLD;
        let line = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
            id,
            curve: Curve2::Line {
                start: [a[0] * k, a[1] * k],
                end: [b[0] * k, b[1] * k],
            },
        };
        let curves = vec![
            line(1, [0.0, 0.0], [l1, 0.0]),
            SketchCurve {
                id: 2,
                curve: Curve2::Arc {
                    center: [l1 * k, rr * k],
                    start: [l1 * k, 0.0],
                    end: [l1 * k, 2.0 * rr * k],
                },
            },
            bezier(3, &s.scaled(k)),
            line(4, [0.0, h], [0.0, 0.0]),
        ];
        let body = prism(&curves, &frame, height * k, k);
        let chain = rim(
            &body,
            &frame,
            height * k,
            &[[0.5 * l1 * k, 0.0], [(l1 + rr) * k, rr * k], [k, 2.5 * k]],
        );
        let back = edge_near(&body, Vec3::new(0.0, 0.5 * h * k, 0.0));
        let all: Vec<FilletEdge> = chain
            .iter()
            .chain([&back])
            .map(|e| FilletEdge {
                edge: *e,
                radius: r * k,
            })
            .collect();
        let made = match fillet(&body, &all, &tol) {
            Ok(m) => m,
            Err(e) => panic!("scale {k}: {e}"),
        };
        assert_eq!(check(&made.body, &tol), []);
        let alone = |edges: &[FilletEdge]| mass(&fillet(&body, edges, &tol).unwrap().body).volume;
        let before = mass(&body).volume;
        let removed = before - mass(&made.body).volume;
        let apart = (before - alone(&all[..3])) + (before - alone(&all[3..]));
        println!("scale {k}: removed {removed:e}, apart {apart:e}");
        assert!((removed - apart).abs() <= 1e-9 * before, "{removed} against {apart}");
        let mut of: Vec<Vec<usize>> = made
            .faces
            .iter()
            .filter_map(|f| match f {
                FilletFace::Fillet(of) => Some(of.clone()),
                _ => None,
            })
            .collect();
        of.sort();
        assert_eq!(of, [vec![0], vec![1], vec![2], vec![3]]);
        assert_eq!(made.faces.len(), made.body.faces.len());
        assert_eq!(made.edges.len(), made.body.edges.len());
        assert_eq!(made.vertices.len(), made.body.vertices.len());
    }
}

/// What a chain does not take is refused with the reason, never built
/// wrong: radii that differ along it, a chamfer whose wall changes at a
/// joint, and an edge that meets the chain at an angle.
#[test]
fn what_a_chain_refuses() {
    use craft3d_kernel::chamfer::{ChamferEdge, ChamferSize, chamfer};
    let (l1, rr, h) = (2.0, 1.0, 3.0);
    let s = Bez([[l1, 2.0 * rr], [1.2, 2.0 * rr], [0.8, h], [0.0, h]]);
    let (r, height) = (0.1, 1.5);
    let tol = tolerance(1.0);
    let line = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
        id,
        curve: Curve2::Line { start: a, end: b },
    };
    let curves = vec![
        line(1, [0.0, 0.0], [l1, 0.0]),
        SketchCurve {
            id: 2,
            curve: Curve2::Arc {
                center: [l1, rr],
                start: [l1, 0.0],
                end: [l1, 2.0 * rr],
            },
        },
        bezier(3, &s),
        line(4, [0.0, h], [0.0, 0.0]),
    ];
    let body = prism(&curves, &Frame::WORLD, height, 1.0);
    let chain = rim(
        &body,
        &Frame::WORLD,
        height,
        &[[0.5 * l1, 0.0], [l1 + rr, rr], [1.0, 2.5]],
    );
    let radii = [r, r, 0.5 * r];
    let edges: Vec<FilletEdge> = chain
        .iter()
        .zip(radii)
        .map(|(e, radius)| FilletEdge { edge: *e, radius })
        .collect();
    let error = fillet(&body, &edges, &tol).unwrap_err().to_string();
    assert!(error.contains("different radii"), "{error}");
    let edges: Vec<ChamferEdge> = chain
        .iter()
        .map(|e| ChamferEdge {
            edge: *e,
            first: faces_of(&body, *e)[0],
            size: ChamferSize::Distances {
                first: r,
                second: r,
            },
        })
        .collect();
    let error = chamfer(&body, &edges, &tol).unwrap_err().to_string();
    assert!(error.contains("one of the faces changes"), "{error}");
    let corner = edge_near(&body, Vec3::new(0.0, 0.0, 0.5 * height));
    let edges: Vec<FilletEdge> = chain
        .iter()
        .chain([&corner])
        .map(|e| FilletEdge {
            edge: *e,
            radius: r,
        })
        .collect();
    let error = fillet(&body, &edges, &tol).unwrap_err().to_string();
    assert!(error.contains("meet in a vertex"), "{error}");
}

/// The engine block's finding E6: a vertical boss on a horizontal one
/// whose root into a sloped flare is filleted; the foot of the vertical
/// boss runs over the flare, the root fillet, the horizontal boss, the
/// root fillet again and back: a closed chain of five edges whose base
/// face changes at four joints, all concave.
fn distributor_boss(k: f64) -> Body {
    use craft3d_kernel::extrude::extrude;
    let tol = tolerance(k);
    let points: [[f64; 2]; 12] = [
        [-63.0, 220.0],
        [-63.0, 110.0],
        [-98.0, 45.0],
        [-98.0, -55.0],
        [-110.0, -55.0],
        [-110.0, -70.0],
        [110.0, -70.0],
        [110.0, -55.0],
        [98.0, -55.0],
        [98.0, 45.0],
        [63.0, 110.0],
        [63.0, 220.0],
    ];
    let curves: Vec<SketchCurve> = (0..12)
        .map(|i| SketchCurve {
            id: i as u64 + 1,
            curve: Curve2::Line {
                start: [points[i][0] * k, points[i][1] * k],
                end: [points[(i + 1) % 12][0] * k, points[(i + 1) % 12][1] * k],
            },
        })
        .collect();
    let yz = Frame::new(Vec3::new(-200.0 * k, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap();
    let profiles = detect_profiles(&curves, &tol).unwrap();
    let block = extrude(&profiles, &yz, 0.0, 400.0 * k, &tol).unwrap().body;
    let along_x = Frame::new(Vec3::new(-40.0 * k, 78.0 * k, 82.0 * k), Vec3::X, Vec3::Y).unwrap();
    let boss = craft3d_kernel::primitives::cylinder(&along_x, 22.0 * k, 240.0 * k).unwrap();
    let body = boolean(&block, &boss, BooleanOp::Union, &tol).unwrap().body;
    // The flare from (98, 45) to (63, 110) meets the boss in two lines.
    let (a, b): ([f64; 2], [f64; 2]) = ([98.0, 45.0], [63.0, 110.0]);
    let (dy, dz) = (b[0] - a[0], b[1] - a[1]);
    let roots: Vec<[f64; 2]> = {
        // a + t (b − a) on the circle about (78, 82) of radius 22.
        let (py, pz) = (a[0] - 78.0, a[1] - 82.0);
        let qa = dy * dy + dz * dz;
        let qb = 2.0 * (py * dy + pz * dz);
        let qc = py * py + pz * pz - 22.0 * 22.0;
        let d = (qb * qb - 4.0 * qa * qc).sqrt();
        [(-qb - d) / (2.0 * qa), (-qb + d) / (2.0 * qa)]
            .iter()
            .map(|t| [a[0] + t * dy, a[1] + t * dz])
            .collect()
    };
    let roots: Vec<EdgeId> = roots
        .iter()
        .map(|p| edge_near(&body, Vec3::new(100.0 * k, p[0] * k, p[1] * k)))
        .collect();
    let root: Vec<FilletEdge> = roots
        .iter()
        .map(|e| FilletEdge {
            edge: *e,
            radius: 4.0 * k,
        })
        .collect();
    let body = fillet(&body, &root, &tol).unwrap().body;
    let up = Frame::WORLD
        .at(Vec3::new(-20.0 * k, 82.0 * k, 82.0 * k))
        .unwrap();
    let distributor = craft3d_kernel::primitives::cylinder(&up, 17.0 * k, 50.0 * k).unwrap();
    boolean(&body, &distributor, BooleanOp::Union, &tol)
        .unwrap()
        .body
}

/// The edges of `body` where the vertical boss of [`distributor_boss`]
/// stands on the rest: on its wall and below its top.
fn foot(body: &Body, k: f64) -> Vec<EdgeId> {
    (0..body.edges.len())
        .filter(|i| {
            let e = &body.edges[*i];
            (0..=8).all(|j| {
                let t = e.range.0 + (e.range.1 - e.range.0) * f64::from(j) / 8.0;
                let p = e.curve.eval(t) * (1.0 / k);
                ((p.x + 20.0).hypot(p.y - 82.0) - 17.0).abs() < 1e-6 && p.z < 131.0
            })
        })
        .map(|i| EdgeId(i as u32))
        .collect()
}

/// The distance from the point (y, z) of the block's cross-section to
/// the base the vertical boss stands on near its foot (all of it runs
/// along x): the flare above the upper root fillet, that fillet, and the
/// horizontal boss beyond it, each where it is a face.
fn to_base(y: f64, z: f64) -> f64 {
    let (a, b) = ([98.0_f64, 45.0_f64], [63.0_f64, 110.0_f64]);
    let along = {
        let (dy, dz) = (b[0] - a[0], b[1] - a[1]);
        let l = dy.hypot(dz);
        [dy / l, dz / l]
    };
    // Out of the material: away from the block's inside.
    let n = [along[1], -along[0]];
    let h = [78.0_f64, 82.0_f64];
    // The root fillet's centre: 4 out from the flare, 26 from the boss's
    // axis; the upper of the two.
    let (px, pz) = (a[0] + 4.0 * n[0] - h[0], a[1] + 4.0 * n[1] - h[1]);
    let qb = 2.0 * (px * along[0] + pz * along[1]);
    let qc = px * px + pz * pz - 26.0 * 26.0;
    let t = (-qb + (qb * qb - 4.0 * qc).sqrt()) / 2.0;
    let o = [h[0] + px + t * along[0], h[1] + pz + t * along[1]];
    let on_line = [o[0] - 4.0 * n[0], o[1] - 4.0 * n[1]];
    let on_boss = [
        h[0] + 22.0 * (o[0] - h[0]) / 26.0,
        h[1] + 22.0 * (o[1] - h[1]) / 26.0,
    ];
    let cross = |u: [f64; 2], v: [f64; 2]| u[0] * v[1] - u[1] * v[0];
    let mut best = f64::INFINITY;
    // The flare, beyond the fillet's tangent line along it.
    let along_from = (y - on_line[0]) * along[0] + (z - on_line[1]) * along[1];
    if along_from >= -1e-9 {
        best = best.min(((y - a[0]) * n[0] + (z - a[1]) * n[1]).abs());
    }
    // The fillet, between its two tangent points as seen from its centre.
    let to = [y - o[0], z - o[1]];
    let (from_line, from_boss) = (
        [on_line[0] - o[0], on_line[1] - o[1]],
        [on_boss[0] - o[0], on_boss[1] - o[1]],
    );
    if cross(from_line, to) * cross(from_line, from_boss) >= -1e-9
        && cross(to, from_boss) * cross(from_line, from_boss) >= -1e-9
    {
        best = best.min((4.0 - to[0].hypot(to[1])).abs());
    }
    // The boss, on the far side of the fillet's tangent point.
    let to_h = [y - h[0], z - h[1]];
    let tangent_h = [on_boss[0] - h[0], on_boss[1] - h[1]];
    if cross(tangent_h, to_h) * cross(tangent_h, [on_line[0] - h[0], on_line[1] - h[1]]) <= 1e-9 {
        best = best.min((to_h[0].hypot(to_h[1]) - 22.0).abs());
    }
    best
}

/// The foot of the distributor boss (finding E6 of the engine block, in
/// its first form, without the block's waist round above the flare): a
/// closed chain of five edges, the base face changing at four joints (flare
/// to root fillet to horizontal boss and back) and the boss's wall the
/// other side throughout, all concave. Every fillet face is made of balls
/// that touch the wall and the base (the centre the radius from both, in
/// the cross-section of the base, which runs along x), tangent across all
/// fifteen seams. A radius whose border would leave the narrow strip of
/// flare above the root fillet is refused with the reason.
#[test]
fn the_foot_of_a_boss_over_a_root_fillet() {
    use craft3d_tess::{TessParams, tessellate_body};
    let k = 1.0;
    let tol = tolerance(k);
    let body = distributor_boss(k);
    assert_eq!(check(&body, &tol), []);
    let at = |r: f64| -> Vec<FilletEdge> {
        foot(&body, k)
            .into_iter()
            .map(|edge| FilletEdge { edge, radius: r * k })
            .collect()
    };
    assert_eq!(at(1.0).len(), 5, "the foot's edges");
    let error = fillet(&body, &at(3.0), &tol).unwrap_err().to_string();
    assert!(error.contains("runs off a face beside the edge"), "{error}");
    let r = 1.0;
    let made = match fillet(&body, &at(r), &tol) {
        Ok(m) => m,
        Err(e) => panic!("{e}"),
    };
    assert_eq!(check(&made.body, &tol), []);
    assert!(tessellate_body(&made.body, &TessParams::default()).is_ok());
    let (crease, seams) = creases(&made.body, &made.faces);
    assert_eq!(seams, 15);
    assert!(crease <= 10.0 * tol.linear / r, "a crease of {crease}");
    let mut worst = 0.0_f64;
    let mut faces = 0;
    for (index, source) in made.faces.iter().enumerate() {
        let FilletFace::Fillet(_) = source else { continue };
        faces += 1;
        let face = &made.body.faces[index];
        let craft3d_geom::Surface::Nurbs(surface) = &face.surface else {
            panic!("a fillet face that is not a spline surface");
        };
        let ((u0, u1), (v0, v1)) = surface.domain();
        for i in 0..=24 {
            for j in 0..=8 {
                let u = u0 + (u1 - u0) * f64::from(i) / 24.0;
                let v = v0 + (v1 - v0) * f64::from(j) / 8.0;
                let (q, du, dv) = surface.derivatives(u, v);
                let n = du.cross(dv).normalized().unwrap();
                let n = if face.reversed { -n } else { n };
                // Concave: the ball is outside, along the outward normal.
                let c = q + n * r;
                let to_wall = ((c.x + 20.0).hypot(c.y - 82.0) - 17.0 - r).abs();
                let to_base = (to_base(c.y, c.z) - r).abs();
                worst = worst.max(to_wall).max(to_base);
            }
        }
    }
    assert_eq!(faces, 5);
    println!("off the definition by {:e} of the tolerance, crease {crease:e}", worst / tol.linear);
    assert!(worst <= 2.0 * tol.linear, "{worst} off the rolling ball");
}
