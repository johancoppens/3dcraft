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
