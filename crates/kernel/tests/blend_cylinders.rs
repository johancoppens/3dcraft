//! Fillets and chamfers along straight edges next to a cylinder (the cut
//! edges of a halved rod, the sharp edge where the two arcs of a lens
//! meet, the foot of a half-round boss), against closed forms. Such an
//! edge is a ruling of the cylinder, so the blend is the same in every
//! plane across it: the test works the section out in 2D on its own (the
//! ball's centre from the two curves moved by the radius, the area by
//! Green's theorem over lines and arcs) and holds volume, area, counts and
//! tangency of the result against it, at three scales and in a tilted
//! frame.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod common;

use boolean_common::{SCALES, assert_close, mass, tolerance};
use common::{arc, frames, line, scale_curves};
use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::chamfer::{ChamferEdge, ChamferSize, chamfer};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{FilletEdge, FilletFace, FilletResult, fillet};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{SketchCurve, detect_profiles};
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check};
use std::f64::consts::PI;

type P = [f64; 2];

/// A piece of the boundary of a region in the plane.
enum Piece {
    Line(P, P),
    /// About `centre` with `radius`, from angle `from` to angle `to`
    /// (counter-clockwise when `to > from`).
    Arc {
        centre: P,
        radius: f64,
        from: f64,
        to: f64,
    },
}

/// The area of a region bounded counter-clockwise by `pieces` (Green's
/// theorem: half the integral of x dy − y dx along the boundary).
fn area(pieces: &[Piece]) -> f64 {
    let mut twice = 0.0;
    for piece in pieces {
        twice += match *piece {
            Piece::Line(p, q) => p[0] * q[1] - p[1] * q[0],
            Piece::Arc {
                centre: c,
                radius: r,
                from: a,
                to: b,
            } => r * r * (b - a) + r * (c[0] * (b.sin() - a.sin()) - c[1] * (b.cos() - a.cos())),
        };
    }
    0.5 * twice
}

fn angle(from: P, to: P) -> f64 {
    (to[1] - from[1]).atan2(to[0] - from[0])
}

/// The profile extruded from 0 to `height` in `frame`, at scale `k`.
fn prism(frame: &Frame, k: f64, curves: &[SketchCurve], height: f64) -> Body {
    let curves = scale_curves(curves, k);
    let profiles = detect_profiles(&curves, &tolerance(k)).unwrap();
    extrude(&profiles, frame, 0.0, height * k, &tolerance(k))
        .unwrap()
        .body
}

/// The straight edge whose middle is nearest `(p, height / 2)`.
fn edge_at(body: &Body, frame: &Frame, k: f64, p: P, height: f64) -> EdgeId {
    let target = frame.point(p[0] * k, p[1] * k, 0.5 * height * k);
    let (index, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            (
                i,
                e.curve.eval(0.5 * (e.range.0 + e.range.1)).distance(target),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    EdgeId(index as u32)
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

/// Every fillet face runs into both neighbours of its edge without a
/// crease: the normals agree at points along every edge they share.
fn assert_smooth(original: &Body, edges: &[FilletEdge], made: &FilletResult) {
    let body = &made.body;
    let mut found = 0;
    for (index, edge) in body.edges.iter().enumerate() {
        let users = faces_of(body, EdgeId(index as u32));
        let [a, b] = users[..] else { continue };
        let neighbours = |fillet: FaceId, other: FaceId| {
            let FilletFace::Fillet(requests) = &made.faces[fillet.0 as usize] else {
                return false;
            };
            let FilletFace::Kept(kept) = &made.faces[other.0 as usize] else {
                return false;
            };
            requests.iter().any(|r| {
                faces_of(original, edges[*r].edge)
                    .iter()
                    .any(|f| kept.contains(f))
            })
        };
        if !(neighbours(a, b) || neighbours(b, a)) {
            continue;
        }
        found += 1;
        for i in 0..9 {
            let s = (f64::from(i) + 0.5) / 9.0;
            let p = edge
                .curve
                .eval(edge.range.0 + s * (edge.range.1 - edge.range.0));
            let (n, m) = (normal_at(body, a, p), normal_at(body, b, p));
            assert!(n.dot(m) > 0.0);
            let sine = n.cross(m).length();
            assert!(sine <= 1e-9, "a crease of {sine} along a fillet's seam");
        }
    }
    assert_eq!(found, 2 * edges.len(), "seams");
}

fn round(body: &Body, frame: &Frame, k: f64, p: P, height: f64, r: f64) -> FilletEdge {
    FilletEdge {
        edge: edge_at(body, frame, k, p, height),
        radius: r * k,
    }
}

/// A half disc of radius `big` (the arc above the x axis, then the
/// diameter), as a sketch.
fn half_disc(big: f64) -> Vec<SketchCurve> {
    vec![
        arc(1, [0.0, 0.0], [big, 0.0], [-big, 0.0]),
        line(2, [-big, 0.0], [big, 0.0]),
    ]
}

/// What a round of radius `r` takes out of the corner of a half disc of
/// radius `big` at `(big, 0)`, and how much of the boundary it takes away
/// along the diameter and along the arc, and the length of its own arc.
fn half_disc_corner(big: f64, r: f64) -> (f64, f64, f64, f64) {
    // The ball is `r` above the diameter and `big - r` from the centre.
    let mx = ((big - r) * (big - r) - r * r).sqrt();
    let m = [mx, r];
    let psi = angle([0.0, 0.0], m);
    let foot_arc = [big * psi.cos(), big * psi.sin()];
    let removed = area(&[
        Piece::Line([mx, 0.0], [big, 0.0]),
        Piece::Arc {
            centre: [0.0, 0.0],
            radius: big,
            from: 0.0,
            to: psi,
        },
        Piece::Arc {
            centre: m,
            radius: r,
            from: angle(m, foot_arc),
            to: -PI / 2.0,
        },
    ]);
    // The same region in the closed form worked out by hand.
    let by_hand = big * big * psi / 2.0 - mx * r / 2.0 - r * r * (PI / 2.0 + psi) / 2.0;
    assert!((removed - by_hand).abs() <= 1e-12 * big * big);
    (removed, big - mx, big * psi, r * (PI / 2.0 + psi))
}

#[test]
fn the_cut_edges_of_a_halved_rod_are_rounded() {
    let (big, height) = (1.0, 3.0);
    for k in SCALES {
        for frame in frames(k) {
            for (r, both) in [(0.2, false), (0.2, true), (0.45, true)] {
                let body = prism(&frame, k, &half_disc(big), height);
                let mut edges = vec![round(&body, &frame, k, [big, 0.0], height, r)];
                if both {
                    edges.push(round(&body, &frame, k, [-big, 0.0], height, r));
                }
                let made = match fillet(&body, &edges, &tolerance(k)) {
                    Ok(made) => made,
                    Err(e) => panic!("scale {k}, r {r}: {e}"),
                };
                assert_eq!(check(&made.body, &tolerance(k)), []);
                let (removed, flat, curved, own) = half_disc_corner(big, r);
                let n = if both { 2.0 } else { 1.0 };
                let section = PI * big * big / 2.0 - n * removed;
                let perimeter = PI * big + 2.0 * big + n * (own - flat - curved);
                let m = mass(&made.body);
                assert_close(m.volume, section * height * k * k * k, "volume");
                assert_close(m.area, (2.0 * section + perimeter * height) * k * k, "area");
                let fillets: Vec<usize> = (0..made.faces.len())
                    .filter(|i| matches!(made.faces[*i], FilletFace::Fillet(_)))
                    .collect();
                assert_eq!(fillets.len(), edges.len());
                assert_eq!(made.body.faces.len(), 4 + edges.len());
                for face in fillets {
                    let Surface::Cylinder(cylinder) = &made.body.faces[face].surface else {
                        panic!("the fillet face is a cylinder");
                    };
                    assert_close(cylinder.radius(), r * k, "radius");
                    assert!(cylinder.frame().z().cross(frame.z()).length() < 1e-12);
                }
                assert_smooth(&body, &edges, &made);
            }
        }
    }
}

/// The halved rod of the finding (a rod less a block, the cut along the
/// rod's axis), rounded along one cut edge: the same as the extruded half
/// disc.
#[test]
fn a_rod_halved_by_a_boolean() {
    for k in SCALES {
        let tol = tolerance(k);
        let rod = primitives::cylinder(&Frame::WORLD, 10.0 * k, 20.0 * k).unwrap();
        let block = primitives::block(
            &Frame::WORLD
                .at(Vec3::new(0.0, -20.0 * k, -5.0 * k))
                .unwrap(),
            20.0 * k,
            40.0 * k,
            30.0 * k,
        )
        .unwrap();
        let half = boolean(&rod, &block, BooleanOp::Subtract, &tol)
            .unwrap()
            .body;
        // The cut lies in x = 0; the half that is left has x < 0. Its
        // edge at y = 10 seen from x < 0 is the corner (r, 0) of a half
        // disc turned by a quarter.
        let frame = Frame::new(Vec3::ZERO, Vec3::Z, Vec3::Y).unwrap();
        let edge = edge_at(&half, &frame, k, [10.0, 0.0], 20.0);
        let edges = [FilletEdge {
            edge,
            radius: 1.0 * k,
        }];
        let made = fillet(&half, &edges, &tol).unwrap();
        assert_eq!(check(&made.body, &tol), []);
        let (removed, ..) = half_disc_corner(10.0, 1.0);
        assert_close(
            mass(&made.body).volume,
            (PI * 50.0 - removed) * 20.0 * k * k * k,
            "volume",
        );
        assert_smooth(&half, &edges, &made);
    }
}

/// A lens: the arcs of two circles of radius `big` about `(0, ∓c)`,
/// meeting in `(±a, 0)`.
fn lens(big: f64, c: f64) -> (Vec<SketchCurve>, f64) {
    let a = (big * big - c * c).sqrt();
    (
        vec![
            arc(1, [0.0, -c], [a, 0.0], [-a, 0.0]),
            arc(2, [0.0, c], [-a, 0.0], [a, 0.0]),
        ],
        a,
    )
}

#[test]
fn the_tip_of_a_lens_is_rounded() {
    let (big, c, height) = (5.0, 4.0, 2.0);
    for k in SCALES {
        for frame in frames(k) {
            let (curves, a) = lens(big, c);
            let body = prism(&frame, k, &curves, height);
            let r = 0.3;
            let edges = [round(&body, &frame, k, [a, 0.0], height, r)];
            let made = match fillet(&body, &edges, &tolerance(k)) {
                Ok(made) => made,
                Err(e) => panic!("scale {k}: {e}"),
            };
            assert_eq!(check(&made.body, &tolerance(k)), []);
            // The ball sits on the x axis, `big - r` from both centres.
            let mx = ((big - r) * (big - r) - c * c).sqrt();
            let m = [mx, 0.0];
            let lower = [0.0, -c];
            let from = angle(lower, [a, 0.0]);
            let to = angle(lower, m);
            let half = area(&[
                Piece::Line([mx + r, 0.0], [a, 0.0]),
                Piece::Arc {
                    centre: lower,
                    radius: big,
                    from,
                    to,
                },
                Piece::Arc {
                    centre: m,
                    radius: r,
                    from: to,
                    to: 0.0,
                },
            ]);
            let removed = 2.0 * half;
            let theta = (c / big).acos();
            let lens_area = 2.0 * (big * big * theta - c * a);
            let perimeter = 4.0 * big * theta - 2.0 * big * (to - from) + 2.0 * r * to;
            let section = lens_area - removed;
            let got = mass(&made.body);
            assert_close(got.volume, section * height * k * k * k, "volume");
            assert_close(
                got.area,
                (2.0 * section + perimeter * height) * k * k,
                "area",
            );
            assert_eq!(made.body.faces.len(), 5);
            assert_smooth(&body, &edges, &made);
        }
    }
}

#[test]
fn the_foot_of_a_half_round_boss_gains_material() {
    // A plate with a half disc standing on its top: the two edges where
    // the arc meets the top are concave.
    let (big, half_width, depth, height) = (1.0, 3.0, 1.0, 2.0);
    let curves = vec![
        line(1, [-half_width, -depth], [half_width, -depth]),
        line(2, [half_width, -depth], [half_width, 0.0]),
        line(3, [half_width, 0.0], [big, 0.0]),
        arc(4, [0.0, 0.0], [big, 0.0], [-big, 0.0]),
        line(5, [-big, 0.0], [-half_width, 0.0]),
        line(6, [-half_width, 0.0], [-half_width, -depth]),
    ];
    for k in SCALES {
        for frame in frames(k) {
            let body = prism(&frame, k, &curves, height);
            let r = 0.25;
            let edges = [
                round(&body, &frame, k, [big, 0.0], height, r),
                round(&body, &frame, k, [-big, 0.0], height, r),
            ];
            let made = match fillet(&body, &edges, &tolerance(k)) {
                Ok(made) => made,
                Err(e) => panic!("scale {k}: {e}"),
            };
            assert_eq!(check(&made.body, &tolerance(k)), []);
            // The ball is `r` above the top and `big + r` from the centre.
            let mx = ((big + r) * (big + r) - r * r).sqrt();
            let m = [mx, r];
            let psi = angle([0.0, 0.0], m);
            let added = area(&[
                Piece::Line([big, 0.0], [mx, 0.0]),
                // Clockwise round the ball, from below it to the foot on
                // the arc, which lies towards the centre.
                Piece::Arc {
                    centre: m,
                    radius: r,
                    from: -PI / 2.0,
                    to: psi - PI,
                },
                Piece::Arc {
                    centre: [0.0, 0.0],
                    radius: big,
                    from: psi,
                    to: 0.0,
                },
            ]);
            assert!(added > 0.0);
            let section = 2.0 * half_width * depth + PI * big * big / 2.0 + 2.0 * added;
            let own = r * (PI / 2.0 - psi);
            let perimeter = 2.0 * half_width
                + 2.0 * depth
                + 2.0 * (half_width - big)
                + PI * big
                + 2.0 * (own - (mx - big) - big * psi);
            let got = mass(&made.body);
            assert_close(got.volume, section * height * k * k * k, "volume");
            assert_close(
                got.area,
                (2.0 * section + perimeter * height) * k * k,
                "area",
            );
            assert_smooth(&body, &edges, &made);
        }
    }
}

#[test]
fn a_halved_rod_is_chamfered_along_the_diameter_and_the_arc() {
    let (big, height) = (1.0, 3.0);
    for k in SCALES {
        for frame in frames(k) {
            let body = prism(&frame, k, &half_disc(big), height);
            let edge = edge_at(&body, &frame, k, [big, 0.0], height);
            // The flat face first: 0.3 along the diameter, 0.2 along the
            // arc.
            let flat = faces_of(&body, edge)
                .into_iter()
                .find(|f| matches!(body.faces[f.0 as usize].surface, Surface::Plane(_)))
                .unwrap();
            let (d1, d2) = (0.3, 0.2);
            let made = chamfer(
                &body,
                &[ChamferEdge {
                    edge,
                    first: flat,
                    size: ChamferSize::Distances {
                        first: d1 * k,
                        second: d2 * k,
                    },
                }],
                &tolerance(k),
            )
            .unwrap();
            assert_eq!(check(&made.body, &tolerance(k)), []);
            let phi = d2 / big;
            let foot = [big * phi.cos(), big * phi.sin()];
            let removed = area(&[
                Piece::Line([big - d1, 0.0], [big, 0.0]),
                Piece::Arc {
                    centre: [0.0, 0.0],
                    radius: big,
                    from: 0.0,
                    to: phi,
                },
                Piece::Line(foot, [big - d1, 0.0]),
            ]);
            let chord = (foot[0] - (big - d1)).hypot(foot[1]);
            let section = PI * big * big / 2.0 - removed;
            let perimeter = PI * big + 2.0 * big - d1 - d2 + chord;
            let got = mass(&made.body);
            assert_close(got.volume, section * height * k * k * k, "volume");
            assert_close(
                got.area,
                (2.0 * section + perimeter * height) * k * k,
                "area",
            );
            assert_eq!(made.body.faces.len(), 5);
        }
    }
}

#[test]
fn a_radius_the_rod_cannot_hold_is_refused() {
    let k = 1.0;
    let body = prism(&Frame::WORLD, k, &half_disc(1.0), 3.0);
    let edges = [round(&body, &Frame::WORLD, k, [1.0, 0.0], 3.0, 0.9)];
    match fillet(&body, &edges, &tolerance(k)) {
        Err(KernelError::FilletTooLarge { .. } | KernelError::Fillet { .. }) => {}
        other => panic!("{other:?}"),
    }
}

/// A cambered blade of two arcs (through `(5, 1.5)` and `(5, 0.5)` over
/// the chord from `(0, 0)` to `(10, 0)`), both rims of its tip rounded:
/// the two tori run out into each other at the sharp leading and trailing
/// edges, which stay sharp (finding F1 of the turbofan showcase; the
/// boolean refused it until 2026-10-14).
#[test]
fn both_tip_edges_of_a_cambered_blade() {
    let upper = (25.0 + 2.25) / 3.0;
    let lower = 25.0 + 0.25;
    let curves = vec![
        arc(1, [5.0, 1.5 - upper], [10.0, 0.0], [0.0, 0.0]),
        arc(2, [5.0, 0.5 - lower], [10.0, 0.0], [0.0, 0.0]),
    ];
    let height = 10.0;
    for k in SCALES {
        for frame in frames(k) {
            let tol = tolerance(k);
            let body = prism(&frame, k, &curves, height);
            let rim = |p: P| {
                // The edge of the top through `p`: the nearest by samples.
                let target = frame.point(p[0] * k, p[1] * k, height * k);
                let (index, _) = body
                    .edges
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        let d = (0..=32)
                            .map(|j| {
                                let t = e.range.0 + (e.range.1 - e.range.0) * f64::from(j) / 32.0;
                                e.curve.eval(t).distance(target)
                            })
                            .fold(f64::INFINITY, f64::min);
                        (i, d)
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .unwrap();
                EdgeId(index as u32)
            };
            let (top, bottom) = (rim([5.0, 1.5]), rim([5.0, 0.5]));
            for r in [0.05, 0.2, 0.3] {
                let edge = |e: EdgeId| FilletEdge {
                    edge: e,
                    radius: r * k,
                };
                let volume = |edges: &[FilletEdge]| {
                    let made = match fillet(&body, edges, &tol) {
                        Ok(made) => made,
                        Err(e) => panic!("scale {k}, r {r}: {e}"),
                    };
                    assert_eq!(check(&made.body, &tol), []);
                    assert_smooth(&body, edges, &made);
                    assert_eq!(made.body.faces.len(), 4 + edges.len());
                    mass(&made.body).volume
                };
                let whole = mass(&body).volume;
                let one = volume(&[edge(top)]);
                let other = volume(&[edge(bottom)]);
                let both = volume(&[edge(top), edge(bottom)]);
                let turned = volume(&[edge(bottom), edge(top)]);
                // What the two take away overlaps near the tips only.
                assert!(both < one.min(other));
                assert!(both > one + other - whole);
                if std::env::var_os("BLEND_NUMBERS").is_some() {
                    println!("k {k} r {r}: {whole} {one} {other} {both}");
                }
                // The order of the edges does not change the solid, to
                // within what the tolerance allows (the boolean's curves
                // where the tori meet are approximations).
                let slack = tol.linear * mass(&body).area;
                assert!((both - turned).abs() <= slack);
            }
        }
    }
}

/// The foot of a boss whose axis stands `d` outside a wall, rounded with
/// a radius whose touch line on the boss crosses the cylinder's seam
/// (engine-block finding E2): the result must not depend on where the
/// seam lies.
#[test]
fn a_boss_root_fillet_does_not_depend_on_the_seam() {
    // The wall: the block below y = 0, from x = -4 to 4 and y = -3.
    // The boss: a rod of radius `big` about (0, d), as high as the block.
    let (big, d, r, height) = (1.0_f64, 0.3_f64, 0.5_f64, 2.0_f64);
    let ex = (big * big - d * d).sqrt();
    // The ball is `r` off the wall and `big + r` from the axis.
    let mx = ((big + r) * (big + r) - (r - d) * (r - d)).sqrt();
    let m = [mx, r];
    let psi = angle([0.0, d], m);
    let phi = angle([0.0, d], [ex, 0.0]);
    // The touch line on the boss lies past its widest point, where a
    // seam at 0 or 180 degrees runs.
    assert!(psi > 0.0 && phi < 0.0);
    let added = area(&[
        Piece::Line([ex, 0.0], [mx, 0.0]),
        Piece::Arc {
            centre: m,
            radius: r,
            from: -PI / 2.0,
            to: psi - PI,
        },
        Piece::Arc {
            centre: [0.0, d],
            radius: big,
            from: psi,
            to: phi,
        },
    ]);
    assert!(added > 0.0);
    // The disc's part above the wall: the side of the chord at distance
    // d below the centre that holds the centre.
    let cap = big * big * (PI - (d / big).acos()) + d * ex;
    let section = 8.0 * 3.0 + cap + 2.0 * added;
    for k in SCALES {
        for frame in frames(k) {
            let mut volumes = Vec::new();
            for degrees in [0.0_f64, 45.0, 90.0, 180.0, 270.0] {
                let turn = degrees.to_radians();
                let wall_frame = frame.at(frame.point(-4.0 * k, -3.0 * k, 0.0)).unwrap();
                let wall = primitives::block(&wall_frame, 8.0 * k, 3.0 * k, height * k).unwrap();
                let boss_frame = Frame::new(
                    frame.point(0.0, d * k, 0.0),
                    frame.z(),
                    frame.vector(turn.cos(), turn.sin(), 0.0),
                )
                .unwrap();
                let boss = primitives::cylinder(&boss_frame, big * k, height * k).unwrap();
                let body = boolean(&wall, &boss, BooleanOp::Union, &tolerance(k))
                    .unwrap()
                    .body;
                let edges = [
                    round(&body, &frame, k, [ex, 0.0], height, r),
                    round(&body, &frame, k, [-ex, 0.0], height, r),
                ];
                let made = match fillet(&body, &edges, &tolerance(k)) {
                    Ok(made) => made,
                    Err(e) => panic!("scale {k}, seam at {degrees} degrees: {e}"),
                };
                assert_eq!(check(&made.body, &tolerance(k)), [], "seam {degrees}");
                assert_smooth(&body, &edges, &made);
                let got = mass(&made.body).volume;
                assert_close(got, section * height * k * k * k, "volume");
                volumes.push(got);
            }
            for v in &volumes {
                assert_close(*v, volumes[0], "volume against the seam at 0 degrees");
            }
        }
    }
}
