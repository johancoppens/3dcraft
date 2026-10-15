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
        let FilletFace::Fillet(of) = source else {
            continue;
        };
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
        assert!(
            crease <= 10.0 * tol.linear / (r * k),
            "a crease of {crease}"
        );
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
        assert!(
            crease <= 10.0 * tol.linear / (r * k),
            "a crease of {crease}"
        );
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
        assert!(
            (removed - apart).abs() <= 1e-9 * before,
            "{removed} against {apart}"
        );
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
/// joint, and an edge with another radius that meets the chain at an
/// angle (with the same radius, the two meet at a corner whose third edge
/// stays sharp, which is built).
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
    // The chain and the upright edge at its start meet at a corner whose
    // third edge stays sharp: built with one radius, refused with two.
    let corner = edge_near(&body, Vec3::new(0.0, 0.0, 0.5 * height));
    let with = |radius: f64| -> Vec<FilletEdge> {
        chain
            .iter()
            .map(|e| FilletEdge {
                edge: *e,
                radius: r,
            })
            .chain([FilletEdge {
                edge: corner,
                radius,
            }])
            .collect()
    };
    let made = fillet(&body, &with(r), &tol).unwrap();
    assert_eq!(check(&made.body, &tol), []);
    let error = fillet(&body, &with(0.5 * r), &tol).unwrap_err().to_string();
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
            .map(|edge| FilletEdge {
                edge,
                radius: r * k,
            })
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
        let FilletFace::Fillet(_) = source else {
            continue;
        };
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
    println!(
        "off the definition by {:e} of the tolerance, crease {crease:e}",
        worst / tol.linear
    );
    assert!(worst <= 2.0 * tol.linear, "{worst} off the rolling ball");
}

/// The nearest point to `p` of the solid cone about z with radius `a` at
/// z = 0 narrowing to its apex at z = 3a.
fn onto_cone(p: Vec3, a: f64) -> Vec3 {
    let rho = p.x.hypot(p.y);
    let (cx, cy) = if rho > 0.0 {
        (p.x / rho, p.y / rho)
    } else {
        (1.0, 0.0)
    };
    if rho <= a - p.z / 3.0 {
        return p;
    }
    let d = [-1.0 / 10f64.sqrt(), 3.0 / 10f64.sqrt()];
    let t = (rho - a) * d[0] + p.z * d[1];
    let apex = 3.0 * a * 10f64.sqrt() / 3.0;
    let t = t.min(apex);
    let (r, z) = (a + d[0] * t, d[1] * t);
    let r = r.max(0.0);
    Vec3::new(cx * r, cy * r, z)
}

/// The top of the half cone: the plane through (0, 0, 1.2) rising by 0.2
/// along x, as (a point, its unit normal out of the body).
fn tilted_top() -> (Vec3, Vec3) {
    (
        Vec3::new(0.0, 0.0, 1.2),
        Vec3::new(-0.2, 0.0, 1.0).normalized().unwrap(),
    )
}

/// The cone of radius 1 and height 3 about z, kept where y ≥ 0 and below
/// the tilted top.
fn tilted_half_cone(k: f64) -> Body {
    let tol = tolerance(k);
    let cone = craft3d_kernel::primitives::cone(&Frame::WORLD, k, 3.0 * k).unwrap();
    let half = boxed(k, [-2.0, 0.0, -1.0], [2.0, 2.0, 4.0]);
    let body = boolean(&cone, &half, BooleanOp::Intersect, &tol)
        .unwrap()
        .body;
    let (p, n) = tilted_top();
    let x = Vec3::new(1.0, 0.0, 0.2).normalized().unwrap();
    let y = n.cross(x);
    let corner = (p - n * 4.0 - x * 3.0 - y * 3.0) * k;
    let frame = Frame::new(corner, n, x).unwrap();
    let below = craft3d_kernel::primitives::block(&frame, 6.0 * k, 6.0 * k, 4.0 * k).unwrap();
    boolean(&body, &below, BooleanOp::Intersect, &tol)
        .unwrap()
        .body
}

/// The distance from `p` to the centres of balls of radius `r` behind the
/// top and the cone (`cone` true) or behind the top and the plane y = 0,
/// by Dykstra's alternating projections onto the two convex sets.
fn to_top_centres(p: Vec3, r: f64, cone: bool) -> f64 {
    let (q, n) = tilted_top();
    let top = |x: Vec3| {
        let d = n.dot(x - q) + r;
        if d > 0.0 { x - n * d } else { x }
    };
    let other = |x: Vec3| {
        if cone {
            onto_cone(x, 1.0 - r * 10f64.sqrt() / 3.0)
        } else {
            Vec3::new(x.x, x.y.max(r), x.z)
        }
    };
    let (mut x, mut pp, mut qq) = (p, Vec3::ZERO, Vec3::ZERO);
    for _ in 0..400 {
        let y0 = x + pp;
        let y = top(y0);
        pp = y0 - y;
        let x0 = y + qq;
        x = other(x0);
        qq = x0 - x;
    }
    p.distance(x)
}

/// A half cone cut by a tilted plane: its top is bounded by an arc of an
/// ellipse next to the cone and a line in the cut plane, which meet at the
/// two cut edges. Rounding both top edges and leaving the cut edges sharp,
/// the two fillets meet at each end in the curve they share: a point of
/// the body is kept exactly when a ball behind the top and the cone holds
/// it and a ball behind the top and the cut plane holds it.
#[test]
fn two_fillets_that_meet_where_a_third_edge_stays_sharp() {
    use craft3d_kernel::boolean::contains_points;
    let r = 0.1;
    for k in SCALES {
        let tol = tolerance(k);
        let body = tilted_half_cone(k);
        assert_eq!(check(&body, &tol), []);
        let rim = edge_near(&body, Vec3::new(0.0, 0.6 * k, 1.2 * k));
        let line = edge_near(&body, Vec3::new(0.0, 0.0, 1.2 * k));
        let made = match fillet(
            &body,
            &[
                FilletEdge {
                    edge: rim,
                    radius: r * k,
                },
                FilletEdge {
                    edge: line,
                    radius: r * k,
                },
            ],
            &tol,
        ) {
            Ok(m) => m,
            Err(e) => panic!("scale {k}: {e}"),
        };
        assert_eq!(check(&made.body, &tol), []);
        let deep = craft3d_tess::body_self_intersections(
            &made.body,
            &craft3d_tess::SelfCheckParams::new(tol.linear),
        )
        .unwrap();
        assert_eq!(deep.certain_crossings, 0, "{deep:?}");
        assert!(deep.shells.is_empty(), "{:?}", deep.shells);
        // The fillets are tangent to the faces they round; where they run
        // out into each other and into the cut plane, there are creases.
        let (crease, seams) = creases(&made.body, &made.faces);
        assert_eq!(seams, 4);
        assert!(
            crease <= 10.0 * tol.linear / (r * k),
            "a crease of {crease}"
        );
        let (q, n) = tilted_top();
        let n_grid = 15;
        let margin = 2e-4;
        let mut points = Vec::new();
        let mut expected = Vec::new();
        for i in 0..n_grid {
            for j in 0..n_grid {
                for l in 0..n_grid {
                    let f = |m: usize, lo: f64, hi: f64| {
                        lo + (hi - lo) * (m as f64 + 0.37) / n_grid as f64
                    };
                    let p = Vec3::new(f(i, -0.75, 0.75), f(j, -0.02, 0.3), f(l, 1.0, 1.4));
                    let cone = (1.0 - p.z / 3.0 - p.x.hypot(p.y)) * 3.0 / 10f64.sqrt();
                    let top = -n.dot(p - q);
                    let in_body = p.y > 0.0 && cone > 0.0 && top > 0.0;
                    let near_body = [p.y, cone, top].iter().any(|d| d.abs() < margin);
                    let (da, db) = (to_top_centres(p, r, true), to_top_centres(p, r, false));
                    if near_body || (da - r).abs() < margin || (db - r).abs() < margin {
                        continue;
                    }
                    points.push(p * k);
                    expected.push(in_body && da <= r && db <= r);
                }
            }
        }
        let found = contains_points(&made.body, &points, &tol).unwrap();
        let mut undecided = 0;
        let mut removed = 0;
        for ((p, want), got) in points.iter().zip(&expected).zip(&found) {
            match got {
                Some(got) => assert_eq!(got, want, "at {p:?}, scale {k}"),
                None => undecided += 1,
            }
            let q = *p * (1.0 / k);
            let cone = (1.0 - q.z / 3.0 - q.x.hypot(q.y)) * 3.0 / 10f64.sqrt();
            if q.y > 0.0 && cone > 0.0 && -n.dot(q - tilted_top().0) > 0.0 && !want {
                removed += 1;
            }
        }
        println!(
            "scale {k}: {} points, {removed} taken away, {undecided} undecided",
            points.len()
        );
        assert!(undecided * 100 < points.len(), "{undecided} undecided");
        assert!(
            removed > 50,
            "only {removed} points in what the fillets take away"
        );
    }
}

/// A cubic Bézier of a blade section, turned by `angle` and scaled by `s`.
fn section_curve(id: u64, control: [[f64; 2]; 4], s: f64, angle: f64, k: f64) -> SketchCurve {
    use craft3d_kernel::profile::Spline2;
    let (sin, cos) = angle.sin_cos();
    SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control: control
                .iter()
                .map(|p| {
                    let (x, y) = (p[0] * s, p[1] * s);
                    [(x * cos - y * sin) * k, (x * sin + y * cos) * k]
                })
                .collect(),
        }),
    }
}

/// The suction and the pressure side of the blade's sections, from the
/// leading edge to the trailing edge and back: cambered, thin, with sharp
/// edges (a wedge of about 17° at the leading edge).
const SUCTION: [[f64; 2]; 4] = [[-1.0, 0.0], [-0.5, 0.3], [0.4, 0.3], [1.0, 0.0]];
const PRESSURE: [[f64; 2]; 4] = [[1.0, 0.0], [0.4, 0.12], [-0.5, 0.12], [-1.0, 0.0]];

/// A blade lofted through three such sections, smaller and turned further
/// towards the tip at z = 3.
fn lofted_blade(k: f64) -> Body {
    use craft3d_kernel::loft::{Options, Section as LoftSection, loft};
    let tol = tolerance(k);
    let levels = [(0.0, 1.0, 0.0), (1.5, 0.9, 0.15), (3.0, 0.8, 0.3)];
    let sections: Vec<LoftSection> = levels
        .iter()
        .map(|(z, s, angle)| {
            let curves = [
                section_curve(1, SUCTION, *s, *angle, k),
                section_curve(2, PRESSURE, *s, *angle, k),
            ];
            let profiles = detect_profiles(&curves, &tol).unwrap();
            LoftSection {
                plane: Frame::WORLD.at(Vec3::new(0.0, 0.0, z * k)).unwrap(),
                outline: profiles[0].outer.clone(),
                start: None,
            }
        })
        .collect();
    loft(&sections, &Options::default(), &tol).unwrap().body
}

/// The point of a section's Bézier at `t`, turned and scaled as the tip's.
fn at_tip(control: [[f64; 2]; 4], t: f64) -> [f64; 2] {
    let s = 1.0 - t;
    let b = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
    let (x, y) = (0..4).fold((0.0, 0.0), |(x, y), i| {
        (
            x + b[i] * control[i][0] * 0.8,
            y + b[i] * control[i][1] * 0.8,
        )
    });
    let (sin, cos) = 0.3_f64.sin_cos();
    [x * cos - y * sin, x * sin + y * cos]
}

/// The tip of a lofted blade rounded all round: the two rims of its tip
/// face meet at the leading and at the trailing edge, which stay sharp.
/// Every fillet face, where it is a face of the result (its mesh), is made
/// of balls that touch the tip and the side of its edge, from inside;
/// the body passes the checks, through itself nowhere, and the fillets are
/// tangent to the faces they round.
#[test]
fn the_tip_of_a_lofted_blade_rounded_all_round() {
    use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
    let r = 0.02;
    for k in SCALES {
        let tol = tolerance(k);
        let body = lofted_blade(k);
        assert_eq!(check(&body, &tol), []);
        let rims: Vec<EdgeId> = [SUCTION, PRESSURE]
            .iter()
            .map(|c| {
                let p = at_tip(*c, 0.5);
                edge_near(&body, Vec3::new(p[0] * k, p[1] * k, 3.0 * k))
            })
            .collect();
        let edges: Vec<FilletEdge> = rims
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
        let deep = body_self_intersections(&made.body, &SelfCheckParams::new(tol.linear)).unwrap();
        assert_eq!(deep.certain_crossings, 0, "{deep:?}");
        let (crease, seams) = creases(&made.body, &made.faces);
        assert_eq!(seams, 4);
        assert!(
            crease <= 10.0 * tol.linear / (r * k),
            "a crease of {crease}"
        );
        // Each fillet's own side of the blade: the face its edge had
        // besides the tip.
        let own: Vec<&craft3d_geom::Surface> = rims
            .iter()
            .map(|e| {
                let side = faces_of(&body, *e)
                    .into_iter()
                    .find(|f| {
                        matches!(
                            body.faces[f.0 as usize].surface,
                            craft3d_geom::Surface::Nurbs(_)
                        )
                    })
                    .unwrap();
                &made.body.faces[side.0 as usize].surface
            })
            .collect();
        let params = TessParams {
            chordal_tolerance: 1e-3 * k,
            ..TessParams::default()
        };
        let mesh = tessellate_body(&made.body, &params).unwrap();
        let mut worst = 0.0_f64;
        let mut seen = 0;
        for (t, face) in mesh.face_ids.iter().enumerate() {
            let face = *face as usize;
            if !matches!(made.faces[face], FilletFace::Fillet(_)) {
                continue;
            }
            let FilletFace::Fillet(of) = &made.faces[face] else {
                continue;
            };
            let side = own[of[0]];
            let f = &made.body.faces[face];
            for q in mesh.corners(t).unwrap() {
                let (u, v) = f.surface.project(q).unwrap();
                let n = f.surface.normal(u, v).unwrap();
                let n = if f.reversed { -n } else { n };
                let c = f.surface.eval(u, v) - n * (r * k);
                let to_tip = (3.0 * k - c.z - r * k).abs();
                let to_side = {
                    let (a, b) = side.project(c).unwrap();
                    side.eval(a, b).distance(c)
                };
                worst = worst.max(to_tip).max((to_side - r * k).abs());
                seen += 1;
            }
        }
        println!(
            "scale {k}: {seen} mesh points of the fillets, off the definition by {:e} of the tolerance, crease {crease:e}",
            worst / tol.linear
        );
        assert!(seen > 100);
        assert!(worst <= 4.0 * tol.linear, "{worst} off the rolling ball");
    }
}

/// The same corners inside out: the tilted half cone taken out of a block
/// as a pocket open at the bottom, its ceiling's two edges rounded (all
/// edges at the two corners concave). What the fillets add is the mirror
/// of what they took away from the half cone: a point of the pocket stays
/// empty exactly when a ball inside the pocket below the ceiling and
/// beside the cone holds it and one below the ceiling and beside the cut
/// plane holds it.
#[test]
fn two_fillets_that_meet_in_a_pocket() {
    use craft3d_kernel::boolean::contains_points;
    let r = 0.1;
    for k in [1e-3, 1.0] {
        let tol = tolerance(k);
        let pocket = tilted_half_cone(k);
        let block = boxed(k, [-1.5, -0.5, 0.9], [1.5, 1.5, 2.0]);
        let body = boolean(&block, &pocket, BooleanOp::Subtract, &tol)
            .unwrap()
            .body;
        assert_eq!(check(&body, &tol), []);
        let rim = edge_near(&body, Vec3::new(0.0, 0.6 * k, 1.2 * k));
        let line = edge_near(&body, Vec3::new(0.0, 0.0, 1.2 * k));
        let made = match fillet(
            &body,
            &[
                FilletEdge {
                    edge: rim,
                    radius: r * k,
                },
                FilletEdge {
                    edge: line,
                    radius: r * k,
                },
            ],
            &tol,
        ) {
            Ok(m) => m,
            Err(e) => panic!("scale {k}: {e}"),
        };
        assert_eq!(check(&made.body, &tol), []);
        let deep = craft3d_tess::body_self_intersections(
            &made.body,
            &craft3d_tess::SelfCheckParams::new(tol.linear),
        )
        .unwrap();
        assert_eq!(deep.certain_crossings, 0, "{deep:?}");
        let (crease, seams) = creases(&made.body, &made.faces);
        assert_eq!(seams, 4);
        assert!(
            crease <= 10.0 * tol.linear / (r * k),
            "a crease of {crease}"
        );
        let (q, n) = tilted_top();
        let n_grid = 15;
        let margin = 2e-4;
        let mut points = Vec::new();
        let mut expected = Vec::new();
        for i in 0..n_grid {
            for j in 0..n_grid {
                for l in 0..n_grid {
                    let f = |m: usize, lo: f64, hi: f64| {
                        lo + (hi - lo) * (m as f64 + 0.37) / n_grid as f64
                    };
                    let p = Vec3::new(f(i, -0.75, 0.75), f(j, -0.02, 0.3), f(l, 1.0, 1.4));
                    let cone = (1.0 - p.z / 3.0 - p.x.hypot(p.y)) * 3.0 / 10f64.sqrt();
                    let top = -n.dot(p - q);
                    let in_pocket = p.y > 0.0 && cone > 0.0 && top > 0.0;
                    let near_pocket = [p.y, cone, top].iter().any(|d| d.abs() < margin);
                    let (da, db) = (to_top_centres(p, r, true), to_top_centres(p, r, false));
                    if near_pocket || (da - r).abs() < margin || (db - r).abs() < margin {
                        continue;
                    }
                    points.push(p * k);
                    // In the block (its bounds are far off): solid unless the
                    // pocket still holds the point empty.
                    expected.push(!(in_pocket && da <= r && db <= r));
                }
            }
        }
        let found = contains_points(&made.body, &points, &tol).unwrap();
        let mut undecided = 0;
        let mut added = 0;
        for ((p, want), got) in points.iter().zip(&expected).zip(&found) {
            match got {
                Some(got) => assert_eq!(got, want, "at {p:?}, scale {k}"),
                None => undecided += 1,
            }
            let q = *p * (1.0 / k);
            let cone = (1.0 - q.z / 3.0 - q.x.hypot(q.y)) * 3.0 / 10f64.sqrt();
            if q.y > 0.0 && cone > 0.0 && -n.dot(q - tilted_top().0) > 0.0 && *want {
                added += 1;
            }
        }
        println!(
            "scale {k}: {} points, {added} filled in, {undecided} undecided",
            points.len()
        );
        assert!(undecided * 100 < points.len(), "{undecided} undecided");
        assert!(added > 50, "only {added} points in what the fillets add");
    }
}

/// The top view of a wall: a Bézier arch over (-1, 0) to (1, 0), the
/// material below it, and a side line leaving its right end at the angle
/// `phi` (radians) to the arch, the material to its right.
struct ArchCorner {
    arch: Bez,
    /// The side line's direction from (1, 0), a unit vector.
    side: [f64; 2],
}

impl ArchCorner {
    fn new(phi: f64) -> ArchCorner {
        let arch = Bez([[-1.0, 0.0], [-1.0 / 3.0, 0.5], [1.0 / 3.0, 0.5], [1.0, 0.0]]);
        // Back along the arch from its end, turned on by `phi`.
        let back = [-0.8_f64, 0.6_f64];
        let (s, c) = phi.sin_cos();
        let side = [back[0] * c - back[1] * s, back[0] * s + back[1] * c];
        ArchCorner { arch, side }
    }

    fn eval(&self, t: f64) -> [f64; 2] {
        let p = self.arch.0;
        let s = 1.0 - t;
        let b = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
        [
            (0..4).map(|i| b[i] * p[i][0]).sum(),
            (0..4).map(|i| b[i] * p[i][1]).sum(),
        ]
    }

    fn dd(&self, t: f64) -> [f64; 2] {
        let p = self.arch.0;
        let s = 1.0 - t;
        let c = |k: usize| {
            6.0 * (s * (p[2][k] - 2.0 * p[1][k] + p[0][k])
                + t * (p[3][k] - 2.0 * p[2][k] + p[1][k]))
        };
        [c(0), c(1)]
    }

    /// The nearest point of the arch (continued past its ends as the cubic
    /// it is) to `q`: its parameter, and the signed distance of `q`,
    /// positive below the arch (in the material).
    fn to_arch(&self, q: [f64; 2]) -> (f64, f64) {
        let mut t = (0..=64)
            .map(|i| -0.3 + 1.6 * f64::from(i) / 64.0)
            .min_by(|a, b| {
                let (pa, pb) = (self.eval(*a), self.eval(*b));
                let da = (pa[0] - q[0]).hypot(pa[1] - q[1]);
                let db = (pb[0] - q[0]).hypot(pb[1] - q[1]);
                da.total_cmp(&db)
            })
            .unwrap();
        for _ in 0..50 {
            let (c, d, e) = (self.eval(t), self.arch.d(t), self.dd(t));
            let r = [c[0] - q[0], c[1] - q[1]];
            let f = r[0] * d[0] + r[1] * d[1];
            let df = d[0] * d[0] + d[1] * d[1] + r[0] * e[0] + r[1] * e[1];
            t -= f / df;
        }
        let (c, d) = (self.eval(t), self.arch.d(t));
        let len = d[0].hypot(d[1]);
        // Right of the direction of travel is below the arch.
        let n = [d[1] / len, -d[0] / len];
        (t, (q[0] - c[0]) * n[0] + (q[1] - c[1]) * n[1])
    }

    /// The point `r` in from the arch at `t`.
    fn inset(&self, t: f64, r: f64) -> [f64; 2] {
        let (c, d) = (self.eval(t), self.arch.d(t));
        let len = d[0].hypot(d[1]);
        [c[0] + r * d[1] / len, c[1] - r * d[0] / len]
    }

    /// The signed distance of `q` from the side line, positive in the
    /// material.
    fn to_side(&self, q: [f64; 2]) -> f64 {
        let (dx, dy) = (q[0] - 1.0, q[1]);
        -(self.side[0] * dy - self.side[1] * dx)
    }

    /// The distance from `q` to the centres of balls of radius `r` behind
    /// the arch (`side` false) or behind the arch and the side line, in
    /// the top view: the material eroded by `r`, a convex set (the arch is
    /// concave, the side straight).
    fn to_centres(&self, q: [f64; 2], r: f64, side: bool) -> f64 {
        let in_arch = |p: [f64; 2]| self.to_arch(p).1 >= r - 1e-12;
        let in_side = |p: [f64; 2]| self.to_side(p) >= r - 1e-12;
        let onto_arch = |p: [f64; 2]| {
            let (t, d) = self.to_arch(p);
            if d >= r { p } else { self.inset(t, r) }
        };
        let onto_side = |p: [f64; 2]| {
            let d = self.to_side(p);
            if d >= r {
                p
            } else {
                // The side's inward normal: its direction turned clockwise.
                let n = [self.side[1], -self.side[0]];
                [p[0] + n[0] * (r - d), p[1] + n[1] * (r - d)]
            }
        };
        let dist = |p: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]);
        if !side {
            return dist(onto_arch(q));
        }
        if in_arch(q) && in_side(q) {
            return 0.0;
        }
        let mut best = f64::INFINITY;
        if !in_arch(q) {
            let p = onto_arch(q);
            if in_side(p) {
                best = best.min(dist(p));
            }
        }
        if !in_side(q) {
            let p = onto_side(q);
            if in_arch(p) {
                best = best.min(dist(p));
            }
        }
        if best.is_finite() {
            return best;
        }
        // The corner of the eroded set: the point `r` in from the arch
        // that is `r` in from the side, by bisection along the arch.
        let g = |t: f64| self.to_side(self.inset(t, r)) - r;
        let (mut lo, mut hi) = (0.3, 1.3);
        assert!(g(lo) > 0.0 && g(hi) < 0.0);
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            if g(mid) > 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        dist(self.inset(0.5 * (lo + hi), r))
    }
}

/// The top rim of a wall standing on a Bézier arch and the edge where the
/// wall meets a flat side, rounded with one radius: two freeform fillets
/// that meet at the top corner, the side's top edge left sharp (the
/// impeller's finding F3: the cap and the end of a blade-like wall). The
/// side meets the arch at angles each side of a right angle and close to
/// it, where the two fillets reach the side's top edge in one point or
/// close to each other (one of them then runs on along the other's face
/// for a few hundredths of the radius, which the meet had taken for one
/// point). A point near the corner is kept exactly when a ball behind the
/// cap and the arch holds it and a ball behind the arch and the side
/// holds it; every mesh point of the fillets lies where the larger of the
/// two distances is the radius.
#[test]
fn two_fillets_that_meet_at_the_corner_of_a_spline_wall() {
    use craft3d_kernel::boolean::contains_points;
    use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
    let (r, h) = (0.15, 1.0);
    for degrees in [80.0_f64, 89.5, 90.0, 100.0] {
        let corner = ArchCorner::new(degrees.to_radians());
        let q = [1.0 + 1.2 * corner.side[0], 1.2 * corner.side[1]];
        for k in SCALES {
            let tol = tolerance(k);
            let frame = Frame::new(
                Vec3::new(0.3, -0.2, 0.5) * k,
                Vec3::new(0.2, 0.3, 1.0),
                Vec3::new(1.0, 0.0, -0.2),
            )
            .unwrap();
            let line = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
                id,
                curve: Curve2::Line {
                    start: [a[0] * k, a[1] * k],
                    end: [b[0] * k, b[1] * k],
                },
            };
            let curves = vec![
                bezier(1, &corner.arch.scaled(k)),
                line(2, [1.0, 0.0], q),
                line(3, q, [-1.0, q[1]]),
                line(4, [-1.0, q[1]], [-1.0, 0.0]),
            ];
            let body = prism(&curves, &frame, h * k, k);
            assert_eq!(check(&body, &tol), []);
            let rim = edge_near(&body, frame.point(0.0, 0.375 * k, h * k));
            let end = edge_near(&body, frame.point(k, 0.0, 0.5 * h * k));
            let made = match fillet(
                &body,
                &[
                    FilletEdge {
                        edge: rim,
                        radius: r * k,
                    },
                    FilletEdge {
                        edge: end,
                        radius: r * k,
                    },
                ],
                &tol,
            ) {
                Ok(m) => m,
                Err(e) => panic!("{degrees}°, scale {k}: {e}"),
            };
            assert_eq!(check(&made.body, &tol), [], "{degrees}°, scale {k}");
            let deep =
                body_self_intersections(&made.body, &SelfCheckParams::new(tol.linear)).unwrap();
            assert_eq!(deep.certain_crossings, 0, "{deep:?}");
            let (crease, seams) = creases(&made.body, &made.faces);
            assert_eq!(seams, 4, "{degrees}°, scale {k}");
            assert!(
                crease <= 10.0 * tol.linear / (r * k),
                "a crease of {crease}"
            );
            // The distances, in the top view's units, of a point given in
            // the frame's.
            let local = |p: Vec3| {
                let (x, y, z) = frame.local(p);
                ([x / k, y / k], z / k)
            };
            let to_a = |xy: [f64; 2], z: f64| {
                let dz = (z - (h - r)).max(0.0);
                dz.hypot(corner.to_centres(xy, r, false))
            };
            let to_b = |xy: [f64; 2]| corner.to_centres(xy, r, true);
            // Points near the corner.
            let n_grid = 14;
            let margin = 2e-4;
            let mut points = Vec::new();
            let mut expected = Vec::new();
            let mut inside = Vec::new();
            for i in 0..n_grid {
                for j in 0..n_grid {
                    for l in 0..n_grid {
                        let f = |m: usize, lo: f64, hi: f64| {
                            lo + (hi - lo) * (m as f64 + 0.37) / n_grid as f64
                        };
                        let xy = [f(i, 0.55, 1.05), f(j, -0.45, 0.05)];
                        let z = f(l, h - 0.45, h + 0.02);
                        let (sa, ss, sz) = (corner.to_arch(xy).1, corner.to_side(xy), h - z);
                        let in_body = sa > 0.0 && ss > 0.0 && sz > 0.0;
                        let (da, db) = (to_a(xy, z), to_b(xy));
                        let near = [sa, ss, sz, da - r, db - r]
                            .iter()
                            .any(|d| d.abs() < margin);
                        if near {
                            continue;
                        }
                        points.push(frame.point(xy[0] * k, xy[1] * k, z * k));
                        expected.push(in_body && da <= r && db <= r);
                        inside.push(in_body);
                    }
                }
            }
            let found = contains_points(&made.body, &points, &tol).unwrap();
            let mut undecided = 0;
            let mut removed = 0;
            for (((p, want), got), in_body) in points.iter().zip(&expected).zip(&found).zip(&inside)
            {
                match got {
                    Some(got) => assert_eq!(got, want, "at {p:?}, {degrees}°, scale {k}"),
                    None => undecided += 1,
                }
                if *in_body && !want {
                    removed += 1;
                }
            }
            assert!(undecided * 100 < points.len(), "{undecided} undecided");
            assert!(removed > 50, "only {removed} points taken away");
            // Every mesh point of the fillets near the corner is on the
            // boundary of what both keep.
            let params = TessParams {
                chordal_tolerance: 1e-3 * k,
                ..TessParams::default()
            };
            let mesh = tessellate_body(&made.body, &params).unwrap();
            let mut worst = 0.0_f64;
            let mut seen = 0;
            for (t, face) in mesh.face_ids.iter().enumerate() {
                if !matches!(made.faces[*face as usize], FilletFace::Fillet(_)) {
                    continue;
                }
                for p in mesh.corners(t).unwrap() {
                    let (xy, z) = local(p);
                    // Away from the other ends of the two edges.
                    if xy[0] < 0.5 || z < 0.3 {
                        continue;
                    }
                    let g = to_a(xy, z).max(to_b(xy)) - r;
                    worst = worst.max(g.abs());
                    seen += 1;
                }
            }
            println!(
                "{degrees}°, scale {k}: {} points, {removed} taken away, {undecided} undecided; {seen} mesh points of the fillets off the definition by {:e} of the tolerance",
                points.len(),
                worst * k / tol.linear
            );
            assert!(seen > 100);
            assert!(
                worst * k <= 4.0 * tol.linear,
                "{} off the rolling balls",
                worst * k
            );
        }
    }
}
