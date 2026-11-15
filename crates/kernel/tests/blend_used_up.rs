//! Blends that use a face up exactly: the borders on a face come to lie on
//! its other edges from end to end, so nothing of it is left. Before
//! 2026-11-13 the face stayed in the body as two edges between the same
//! two vertices, one curve run one way and back: a face of no area, which
//! passed the checker and had the right volume, but could not be meshed,
//! so the self-check refused the feature ("its loops do not bound a
//! region"). Now the face goes and the faces on its two sides share one
//! edge (`blend::rolling::used_up`).
//!
//! Every volume and area here is worked out by hand from the section
//! across the blended edges (a prism's volume is its section's area times
//! its length; its area is twice the section's plus the section's
//! perimeter times the length), at three scales, on the world frame, on a
//! tilted one away from the origin, and on a tilted one far from it.

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
use craft3d_geom::Surface;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::chamfer::{ChamferEdge, ChamferFace, ChamferSize, chamfer};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{FilletEdge, FilletFace, fillet};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{SketchCurve, detect_profiles};
use craft3d_math::{Frame, Portable, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::{Body, EdgeId, FaceId, check};
use std::f64::consts::PI;

/// The frames every case is built on: the world, a tilted one away from
/// the origin, and a tilted one far from it.
fn frames(k: f64) -> Vec<Frame> {
    let mut out = common::frames(k);
    out.push(
        Frame::new(
            Vec3::new(1000.0 * k, -700.0 * k, 300.0 * k),
            Vec3::new(-2.0, 1.0, 4.0),
            Vec3::new(0.3, 1.0, 0.0),
        )
        .unwrap(),
    );
    out
}

/// The box from `lo` with `size`, in `frame`, at scale `k`.
fn block(frame: &Frame, k: f64, lo: [f64; 3], size: [f64; 3]) -> Body {
    let corner = frame.point(lo[0] * k, lo[1] * k, lo[2] * k);
    let at = Frame::new(corner, frame.z(), frame.x()).unwrap();
    primitives::block(&at, size[0] * k, size[1] * k, size[2] * k).unwrap()
}

/// The edge whose middle is nearest `middle` (in `frame`, at scale 1).
fn edge_at(body: &Body, frame: &Frame, k: f64, middle: [f64; 3]) -> EdgeId {
    let target = frame.point(middle[0] * k, middle[1] * k, middle[2] * k);
    let (index, off) = body
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
    assert!(off < 1e-6 * k, "no edge at {middle:?}");
    EdgeId(index as u32)
}

/// The faces that use `edge`, in ascending order.
fn faces_of(body: &Body, edge: EdgeId) -> Vec<FaceId> {
    (0..body.faces.len())
        .map(|i| FaceId(i as u32))
        .filter(|f| {
            body.face(*f)
                .unwrap()
                .loops
                .iter()
                .flat_map(|l| &body.loop_(*l).unwrap().coedges)
                .any(|c| c.edge == edge)
        })
        .collect()
}

/// The face two edges share.
fn shared(body: &Body, a: EdgeId, b: EdgeId) -> FaceId {
    let of_b = faces_of(body, b);
    let both: Vec<FaceId> = faces_of(body, a)
        .into_iter()
        .filter(|f| of_b.contains(f))
        .collect();
    assert_eq!(both.len(), 1);
    both[0]
}

/// Whether two faces of `body` share an edge.
fn neighbours(body: &Body, a: FaceId, b: FaceId) -> bool {
    (0..body.edges.len()).any(|i| {
        let users = faces_of(body, EdgeId(i as u32));
        users.contains(&a) && users.contains(&b)
    })
}

/// What a document asks of a body before it hands it on: valid, meshed
/// watertight with the volume it encloses, and clear of itself on the
/// mesh of the self-check.
fn sound(body: &Body, k: f64, what: &str) {
    let tol = tolerance(k);
    assert_eq!(check(body, &tol), [], "{what}");
    let exact = mass(body);
    for chord in [0.05, 0.005] {
        let params = TessParams {
            chordal_tolerance: chord * k,
            ..TessParams::default()
        };
        let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
        let welded = mesh.weld().unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert!(welded.is_closed(), "{what}: the mesh is not watertight");
        assert!(
            (welded.volume() - exact.volume).abs() <= exact.area * chord * k,
            "{what}: mesh volume {} against {}",
            welded.volume(),
            exact.volume
        );
    }
    let found = body_self_intersections(body, &SelfCheckParams::new(tol.linear))
        .unwrap_or_else(|e| panic!("{what}: the self-check cannot mesh it: {e}"));
    assert!(
        found.crossings.is_empty() && found.certain_crossings == 0,
        "{what}: {} crossings",
        found.crossings.len()
    );
}

/// Volume and area at scale 1, held at scale `k`.
#[track_caller]
fn assert_mass(body: &Body, volume: f64, area: f64, k: f64, what: &str) {
    let m = mass(body);
    assert_close(m.volume, volume * k * k * k, &format!("{what}: volume"));
    assert_close(m.area, area * k * k, &format!("{what}: area"));
}

/// The upright edges of a box `length` × `width` × `height` rounded with
/// half its width: at `ends` of its two ends (0 at x = 0, 1 at x =
/// `length`), the two rounds meet in the middle of the end face and use it
/// up. The section across the uprights is the rectangle with `ends` of its
/// two short sides made half circles: its area is the rectangle's less
/// `(2 − π/2) r²` a half circle (two spandrels of `(1 − π/4) r²`), its
/// perimeter the rectangle's less `(4 − π) r` a half circle (the `4 r` of
/// straight sides round the end become the half circle's `π r`).
fn rounded_ends(length: f64, width: f64, height: f64, ends: &[usize]) {
    let r = width / 2.0;
    let n = ends.len() as f64;
    let section = length * width - n * (2.0 - PI / 2.0) * r * r;
    let perimeter = 2.0 * (length + width) - n * (4.0 - PI) * r;
    let (volume, area) = (section * height, 2.0 * section + perimeter * height);
    for k in SCALES {
        for (index, frame) in frames(k).iter().enumerate() {
            let body = block(frame, k, [0.0, 0.0, 0.0], [length, width, height]);
            let uprights: Vec<[EdgeId; 2]> = ends
                .iter()
                .map(|end| {
                    let x = *end as f64 * length;
                    [
                        edge_at(&body, frame, k, [x, 0.0, height / 2.0]),
                        edge_at(&body, frame, k, [x, width, height / 2.0]),
                    ]
                })
                .collect();
            let used: Vec<FaceId> = uprights
                .iter()
                .map(|[a, b]| shared(&body, *a, *b))
                .collect();
            let mut edges: Vec<FilletEdge> = uprights
                .iter()
                .flatten()
                .map(|edge| FilletEdge {
                    edge: *edge,
                    radius: r * k,
                })
                .collect();
            for turned in [false, true] {
                if turned {
                    edges.reverse();
                }
                let what =
                    format!("{length}x{width}x{height}, scale {k}, frame {index}, turned {turned}");
                let made =
                    fillet(&body, &edges, &tolerance(k)).unwrap_or_else(|e| panic!("{what}: {e}"));
                sound(&made.body, k, &what);
                assert_mass(&made.body, volume, area, k, &what);
                // Four long faces, the rounds, and the ends not rounded.
                assert_eq!(
                    made.body.faces.len(),
                    4 + edges.len() + 2 - ends.len(),
                    "{what}"
                );
                for face in &made.faces {
                    if let FilletFace::Kept(from) = face {
                        assert!(
                            from.iter().all(|f| !used.contains(f)),
                            "{what}: an end is left"
                        );
                    }
                }
                // The two rounds of an end are one cylinder, and meet along
                // the line the end face was used up to.
                let rounds: Vec<FaceId> = (0..made.faces.len())
                    .filter(|i| matches!(made.faces[*i], FilletFace::Fillet(_)))
                    .map(|i| FaceId(i as u32))
                    .collect();
                assert_eq!(rounds.len(), edges.len(), "{what}");
                for a in &rounds {
                    let Surface::Cylinder(c) = &made.body.face(*a).unwrap().surface else {
                        panic!("{what}: a round that is not a cylinder");
                    };
                    assert!((c.radius() - r * k).abs() <= 1e-9 * r * k, "{what}");
                    let met = rounds
                        .iter()
                        .filter(|b| *b != a && neighbours(&made.body, *a, **b))
                        .count();
                    assert_eq!(met, 1, "{what}: a round meets {met} rounds");
                }
            }
        }
    }
}

#[test]
fn two_rounds_of_half_its_width_use_up_each_end_face() {
    rounded_ends(14.0, 6.0, 10.0, &[0, 1]);
}

#[test]
fn two_rounds_of_half_its_width_use_up_one_end_face() {
    rounded_ends(80.0, 20.0, 20.0, &[1]);
}

#[test]
fn two_rounds_use_up_an_end_face_of_spline_surface() {
    // The 14 × 6 × 10 box again, its two ends made of cubic splines: each
    // end of the profile is a straight Bézier (control points at thirds),
    // so the end faces are flat spline surfaces, and the rounds of the
    // uprights between them and the long planes are built in place as
    // fitted spline faces, their borders fitted curves. The two borders
    // on an end coincide only to what the fits are held to, and are
    // proven one curve within the tolerance. The volume is the closed
    // form within what the fitted rounds may be off (their area times
    // the tolerance).
    use craft3d_kernel::profile::{Curve2, Spline2};
    let section = 84.0 - 2.0 * (2.0 - PI / 2.0) * 9.0;
    let perimeter = 40.0 - 2.0 * (4.0 - PI) * 3.0;
    let (volume, area) = (section * 10.0, 2.0 * section + perimeter * 10.0);
    let rounds_area = 4.0 * (PI / 2.0) * 3.0 * 10.0;
    for k in SCALES {
        let tol = tolerance(k);
        let straight = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
            id,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]
                    .iter()
                    .map(|s| {
                        [
                            k * (a[0] + s * (b[0] - a[0])),
                            k * (a[1] + s * (b[1] - a[1])),
                        ]
                    })
                    .collect(),
            }),
        };
        let line = |id: u64, start: [f64; 2], end: [f64; 2]| SketchCurve {
            id,
            curve: Curve2::Line {
                start: [start[0] * k, start[1] * k],
                end: [end[0] * k, end[1] * k],
            },
        };
        let curves = vec![
            line(1, [0.0, 0.0], [14.0, 0.0]),
            straight(2, [14.0, 0.0], [14.0, 6.0]),
            line(3, [14.0, 6.0], [0.0, 6.0]),
            straight(4, [0.0, 6.0], [0.0, 0.0]),
        ];
        let profiles = detect_profiles(&curves, &tol).unwrap();
        for (index, frame) in frames(k).iter().enumerate() {
            let what = format!("scale {k}, frame {index}");
            let body = extrude(&profiles, frame, 0.0, 10.0 * k, &tol).unwrap().body;
            let uprights: Vec<EdgeId> = [[0.0, 0.0], [0.0, 6.0], [14.0, 0.0], [14.0, 6.0]]
                .iter()
                .map(|[x, y]| edge_at(&body, frame, k, [*x, *y, 5.0]))
                .collect();
            let ends = [
                shared(&body, uprights[0], uprights[1]),
                shared(&body, uprights[2], uprights[3]),
            ];
            for end in ends {
                let surface = &body.face(end).unwrap().surface;
                assert!(
                    matches!(surface, Surface::Nurbs(_) | Surface::Bezier(_)),
                    "{what}"
                );
            }
            let edges: Vec<FilletEdge> = uprights
                .iter()
                .map(|edge| FilletEdge {
                    edge: *edge,
                    radius: 3.0 * k,
                })
                .collect();
            let made = fillet(&body, &edges, &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
            sound(&made.body, k, &what);
            let m = mass(&made.body);
            let off = (m.volume - volume * k * k * k).abs();
            assert!(
                off <= rounds_area * k * k * tol.linear,
                "{what}: volume {} off by {off:e}",
                m.volume
            );
            assert!(
                (m.area - area * k * k).abs() <= 1e-6 * area * k * k,
                "{what}: area {}",
                m.area
            );
            assert_eq!(made.body.faces.len(), 8, "{what}");
            for face in &made.faces {
                if let FilletFace::Kept(from) = face {
                    assert!(
                        from.iter().all(|f| !ends.contains(f)),
                        "{what}: an end is left"
                    );
                }
            }
        }
    }
}

/// The prism of `height` over the closed outline `curves` (given at
/// scale 1), in `frame`, at scale `k`.
fn prism(frame: &Frame, k: f64, curves: &[SketchCurve], height: f64) -> Body {
    let tol = tolerance(k);
    let profiles = detect_profiles(&common::scale_curves(curves, k), &tol).unwrap();
    extrude(&profiles, frame, 0.0, height * k, &tol)
        .unwrap()
        .body
}

#[test]
fn two_rounds_use_up_a_curved_end_face() {
    // A slab 6 wide and 10 tall whose end is a cylinder: the outline runs
    // from (0, 0) to (14, 0), along the arc of radius 5 about (10, 3) to
    // (14, 6) (its apex at (15, 3)), back to (0, 6). The two uprights at
    // the end rounded 3: the ball touches the side 3 in from it and the
    // arc's circle from inside, its centre 5 − 3 = 2 from (10, 3), at
    // (12, 3): both rounds are that one circle, and they touch the end at
    // its apex only, so it is used up. The section is the rectangle 12 × 6
    // and the half disc of radius 3 about (12, 3); its perimeter
    // 12 + 12 + 6 + 3π. The faces: top, bottom, back, the two sides and
    // the two rounds (one cylinder).
    use common::{arc, line};
    let outline = [
        line(1, [0.0, 0.0], [14.0, 0.0]),
        arc(2, [10.0, 3.0], [14.0, 0.0], [14.0, 6.0]),
        line(3, [14.0, 6.0], [0.0, 6.0]),
        line(4, [0.0, 6.0], [0.0, 0.0]),
    ];
    let section = 72.0 + 4.5 * PI;
    let perimeter = 30.0 + 3.0 * PI;
    for k in SCALES {
        for (index, frame) in frames(k).iter().enumerate() {
            let what = format!("scale {k}, frame {index}");
            let body = prism(frame, k, &outline, 10.0);
            let uprights = [
                edge_at(&body, frame, k, [14.0, 0.0, 5.0]),
                edge_at(&body, frame, k, [14.0, 6.0, 5.0]),
            ];
            let end = shared(&body, uprights[0], uprights[1]);
            assert!(matches!(
                body.face(end).unwrap().surface,
                Surface::Cylinder(_)
            ));
            let edges: Vec<FilletEdge> = uprights
                .iter()
                .map(|edge| FilletEdge {
                    edge: *edge,
                    radius: 3.0 * k,
                })
                .collect();
            let made =
                fillet(&body, &edges, &tolerance(k)).unwrap_or_else(|e| panic!("{what}: {e}"));
            sound(&made.body, k, &what);
            assert_mass(
                &made.body,
                section * 10.0,
                2.0 * section + perimeter * 10.0,
                k,
                &what,
            );
            assert_eq!(made.body.faces.len(), 7, "{what}");
            let rounds: Vec<FaceId> = (0..made.faces.len())
                .filter(|i| matches!(made.faces[*i], FilletFace::Fillet(_)))
                .map(|i| FaceId(i as u32))
                .collect();
            assert_eq!(rounds.len(), 2, "{what}");
            assert!(neighbours(&made.body, rounds[0], rounds[1]), "{what}");
            for face in &made.faces {
                if let FilletFace::Kept(from) = face {
                    assert!(!from.contains(&end), "{what}: the end is left");
                }
            }
        }
    }
}

#[test]
fn on_a_shallow_edge_a_vertex_within_the_band_of_the_border_is_not_a_way_in() {
    // A wedge 5 tall whose blunt tip is a face `w` wide, the sides leaving
    // it at 10° (the faces turn by θ = 10° at its two edges), both edges
    // rounded 10. A ball of radius r touches each face r tan(θ/2) from the
    // edge, so the tip is used up when w = 2 r tan(θ/2). Whether the
    // boundary leads into what the blend takes is told inside by more than
    // the tolerance in the measure's own terms: how much nearer than the
    // radius the ball's centre comes to the other face, which along the
    // tip face is the tolerance over sin θ (5.8 tolerances here). Narrower
    // than that by half the tolerance (along the face): used up within the
    // tolerance, built (refused before 2026-11-13, when the sign alone
    // read the vertex as inside); by 3 tolerances (in the band, more than
    // the tolerance along the face) and by 20 (past it): the blend is not
    // taken as reaching on, and it does not fit: refused as too large.
    // The section: the quadrilateral less two spandrels r² (tan(θ/2) − θ/2).
    let theta = PI / 18.0;
    let r = 10.0;
    let exact = 2.0 * r * (0.5 * theta).ptan();
    let spandrel = r * r * ((0.5 * theta).ptan() - 0.5 * theta);
    for k in SCALES {
        let tol = tolerance(k);
        for (index, frame) in frames(k).iter().enumerate() {
            for (narrower, builds) in [(0.0, true), (0.5, true), (3.0, false), (20.0, false)] {
                let what = format!("scale {k}, frame {index}, {narrower} tolerances narrower");
                // At scale 1, then scaled with the outline.
                let w = exact - narrower * tol.linear / k;
                let (p1, p2) = ([0.0, -w / 2.0], [0.0, w / 2.0]);
                let out = 40.0;
                let q1 = [p1[0] - out * theta.psin(), p1[1] - out * theta.pcos()];
                let q2 = [p2[0] - out * theta.psin(), p2[1] + out * theta.pcos()];
                let corners = [p1, p2, q2, q1];
                let outline: Vec<SketchCurve> = (0..4)
                    .map(|i| common::line(i as u64 + 1, corners[i], corners[(i + 1) % 4]))
                    .collect();
                let body = prism(frame, k, &outline, 5.0);
                let edges = [p1, p2].map(|p| FilletEdge {
                    edge: edge_at(&body, frame, k, [p[0], p[1], 2.5]),
                    radius: r * k,
                });
                let made = fillet(&body, &edges, &tol);
                if !builds {
                    assert!(made.is_err(), "{what}");
                    continue;
                }
                let made = made.unwrap_or_else(|e| panic!("{what}: {e}"));
                sound(&made.body, k, &what);
                // Top, bottom, back, two sides and the two rounds.
                assert_eq!(made.body.faces.len(), 7, "{what}");
                let quad = 0.5
                    * (0..4)
                        .map(|i| {
                            let (a, b) = (corners[i], corners[(i + 1) % 4]);
                            a[0] * b[1] - b[0] * a[1]
                        })
                        .sum::<f64>()
                        .abs();
                let want = (quad - 2.0 * spandrel) * 5.0 * k * k * k;
                let m = mass(&made.body);
                // Within what the second round's border moved onto the
                // first by the tip's shortfall can change: that length
                // times the round's radius and the prism's height.
                let shortfall = narrower * tol.linear * (r * k) * (5.0 * k);
                assert!(
                    (m.volume - want).abs() <= (1e-9 * want).max(shortfall),
                    "{what}: {} against {want}",
                    m.volume
                );
            }
        }
    }
}

#[test]
fn a_face_a_little_wider_than_its_rounds_stays_and_one_narrower_is_refused() {
    // 14 × 6 × 10 with the four uprights rounded: 2.9 leaves each end a
    // strip 0.2 wide; 3.1 does not fit. A guard on the step, not a proof
    // of it: it passes without the change of 2026-11-13 as well.
    for k in SCALES {
        let body = block(&Frame::WORLD, k, [0.0, 0.0, 0.0], [14.0, 6.0, 10.0]);
        let uprights: Vec<EdgeId> = [[0.0, 0.0], [0.0, 6.0], [14.0, 0.0], [14.0, 6.0]]
            .iter()
            .map(|[x, y]| edge_at(&body, &Frame::WORLD, k, [*x, *y, 5.0]))
            .collect();
        let at = |r: f64| -> Vec<FilletEdge> {
            uprights
                .iter()
                .map(|edge| FilletEdge {
                    edge: *edge,
                    radius: r * k,
                })
                .collect()
        };
        let r = 2.9;
        let made = fillet(&body, &at(r), &tolerance(k)).unwrap();
        sound(&made.body, k, "2.9");
        let section = 84.0 - 4.0 * (1.0 - PI / 4.0) * r * r;
        let perimeter = 40.0 - 4.0 * (2.0 - PI / 2.0) * r;
        assert_mass(
            &made.body,
            section * 10.0,
            2.0 * section + perimeter * 10.0,
            k,
            "2.9",
        );
        assert_eq!(made.body.faces.len(), 10);
        assert!(fillet(&body, &at(3.1), &tolerance(k)).is_err());
    }
}

#[test]
fn two_chamfers_that_meet_along_the_middle_of_a_face_use_it_up() {
    // A box 40 × 20 × 5: both long edges of its front face (y = 0, 5 tall)
    // chamfered 2.5, which meet along the face's middle line. Each takes a
    // right triangle of legs 2.5 times 40: V = 4000 − 2 × 125. The section
    // across x is the rectangle 20 × 5 less two such triangles,
    // 100 − 6.25, with the front side's 5 replaced by two hypotenuses
    // 2.5 √2 and the top and bottom 2.5 shorter each: its perimeter is
    // 50 − 5 − 5 + 5 √2.
    let section = 100.0 - 6.25;
    let perimeter = 40.0 + 5.0 * 2.0_f64.sqrt();
    let (volume, area) = (40.0 * section, 2.0 * section + 40.0 * perimeter);
    assert_close(volume, 3750.0, "volume");
    for k in SCALES {
        for (index, frame) in frames(k).iter().enumerate() {
            let body = block(frame, k, [0.0, 0.0, 0.0], [40.0, 20.0, 5.0]);
            let top = edge_at(&body, frame, k, [20.0, 0.0, 5.0]);
            let bottom = edge_at(&body, frame, k, [20.0, 0.0, 0.0]);
            let front = shared(&body, top, bottom);
            for (distances, first_on) in [((2.5, 2.5), 0), ((2.5, 2.5), 1)] {
                let what = format!("scale {k}, frame {index}, first face {first_on}");
                let edges: Vec<ChamferEdge> = [top, bottom]
                    .iter()
                    .map(|edge| ChamferEdge {
                        edge: *edge,
                        first: faces_of(&body, *edge)[first_on],
                        size: ChamferSize::Distances {
                            first: distances.0 * k,
                            second: distances.1 * k,
                        },
                    })
                    .collect();
                let made =
                    chamfer(&body, &edges, &tolerance(k)).unwrap_or_else(|e| panic!("{what}: {e}"));
                sound(&made.body, k, &what);
                assert_mass(&made.body, volume, area, k, &what);
                // Top, bottom, back, two ends and two chamfers.
                assert_eq!(made.body.faces.len(), 7, "{what}");
                for face in &made.faces {
                    if let ChamferFace::Kept(from) = face {
                        assert!(!from.contains(&front), "{what}: the front is left");
                    }
                }
                // The two chamfers meet along the ridge at half height.
                let flats: Vec<FaceId> = (0..made.faces.len())
                    .filter(|i| matches!(made.faces[*i], ChamferFace::Chamfer(_)))
                    .map(|i| FaceId(i as u32))
                    .collect();
                assert_eq!(flats.len(), 2, "{what}");
                assert!(neighbours(&made.body, flats[0], flats[1]), "{what}");
                let ridge = edge_at(&made.body, frame, k, [20.0, 0.0, 2.5]);
                let mut users = faces_of(&made.body, ridge);
                users.sort();
                assert_eq!(users, flats, "{what}");
            }
        }
    }
}

#[test]
fn a_chamfer_that_takes_a_whole_wall_measures_the_same_on_every_frame() {
    // A step: the outline (0, 0), (40, 0), (40, 10), (28, 10), (28, 20),
    // (0, 20), 40 tall. The inside edge at (28, 10), between the lower top
    // and the wall 10 high, chamfered `d` along the lower top and 10 up
    // the wall: the chamfer fills the triangle of legs `d` and 10 and takes
    // the whole wall, so it meets the upper top along the wall's top edge.
    // Before `used_up` (2026-11-13) the wall stayed as a face of no area:
    // the checker passed it and the measure took it on the world frame and
    // refused it on tilted ones ("face 3 is not bounded by its loops": the
    // sign of an area that is rounding; found by a review on 2026-11-14 on
    // the released line, with 1 and 5 along the top at scales 1 and 1e6).
    // The section is the step's 680 and the triangle; its perimeter is the
    // step's 120 less `d` of the lower top and the wall's 10, plus the
    // hypotenuse.
    use common::line;
    let outline = [
        line(1, [0.0, 0.0], [40.0, 0.0]),
        line(2, [40.0, 0.0], [40.0, 10.0]),
        line(3, [40.0, 10.0], [28.0, 10.0]),
        line(4, [28.0, 10.0], [28.0, 20.0]),
        line(5, [28.0, 20.0], [0.0, 20.0]),
        line(6, [0.0, 20.0], [0.0, 0.0]),
    ];
    for k in SCALES {
        for (index, frame) in frames(k).iter().enumerate() {
            let body = prism(frame, k, &outline, 40.0);
            let inside = edge_at(&body, frame, k, [28.0, 10.0, 20.0]);
            let above = edge_at(&body, frame, k, [28.0, 20.0, 20.0]);
            let wall = shared(&body, inside, above);
            let lower = *faces_of(&body, inside)
                .iter()
                .find(|f| **f != wall)
                .unwrap();
            for d in [1.0, 5.0] {
                let section = 680.0 + 5.0 * d;
                let perimeter = 110.0 - d + d.phypot(10.0);
                for first_on_wall in [false, true] {
                    let what = format!(
                        "scale {k}, frame {index}, {d} along the top, the wall first {first_on_wall}"
                    );
                    let (first, size) = if first_on_wall {
                        (
                            wall,
                            ChamferSize::Distances {
                                first: 10.0 * k,
                                second: d * k,
                            },
                        )
                    } else {
                        (
                            lower,
                            ChamferSize::Distances {
                                first: d * k,
                                second: 10.0 * k,
                            },
                        )
                    };
                    let edges = [ChamferEdge {
                        edge: inside,
                        first,
                        size,
                    }];
                    let made = chamfer(&body, &edges, &tolerance(k))
                        .unwrap_or_else(|e| panic!("{what}: {e}"));
                    sound(&made.body, k, &what);
                    assert_mass(
                        &made.body,
                        40.0 * section,
                        2.0 * section + 40.0 * perimeter,
                        k,
                        &what,
                    );
                    // The step's eight faces less the wall, and the chamfer.
                    assert_eq!(made.body.faces.len(), 8, "{what}");
                    for face in &made.faces {
                        if let ChamferFace::Kept(from) = face {
                            assert!(!from.contains(&wall), "{what}: the wall is left");
                        }
                    }
                    // A chamfer a little short of the wall's top leaves the
                    // wall, as a strip.
                    let short = [ChamferEdge {
                        edge: inside,
                        first: lower,
                        size: ChamferSize::Distances {
                            first: d * k,
                            second: 9.999 * k,
                        },
                    }];
                    let made = chamfer(&body, &short, &tolerance(k))
                        .unwrap_or_else(|e| panic!("{what}, short: {e}"));
                    sound(&made.body, k, &format!("{what}, short"));
                    assert_eq!(made.body.faces.len(), 9, "{what}, short");
                    assert_close(
                        mass(&made.body).volume,
                        40.0 * (680.0 + 0.5 * d * 9.999) * k * k * k,
                        &format!("{what}, short: volume"),
                    );
                }
            }
        }
    }
}

#[test]
fn a_round_as_long_as_both_faces_it_runs_along_uses_them_up() {
    // An L: a foot 40 × 20 × 5 and an upright 5 × 20 × 40, 20 deep, joined
    // (V0 = 4000 + 4000 − 500 = 7500). The inside faces run 35 from the
    // bend; a round of 35 in the bend is tangent to them at their far
    // edges, so it uses both up and meets the legs' ends there, square.
    // It adds the spandrel (1 − π/4) 35² along the 20: the section across
    // y has the area 375 + (1 − π/4) 1225 and the perimeter 40 + 5 +
    // 35 π / 2 + 5 + 40 (the two inside sides replaced by the quarter
    // circle).
    let section = 375.0 + (1.0 - PI / 4.0) * 1225.0;
    let perimeter = 90.0 + 35.0 * PI / 2.0;
    let (volume, area) = (20.0 * section, 2.0 * section + 20.0 * perimeter);
    for k in SCALES {
        for (index, frame) in frames(k).iter().enumerate() {
            let what = format!("scale {k}, frame {index}");
            let foot = block(frame, k, [0.0, 0.0, 0.0], [40.0, 20.0, 5.0]);
            let upright = block(frame, k, [0.0, 0.0, 0.0], [5.0, 20.0, 40.0]);
            let body = boolean(&foot, &upright, BooleanOp::Union, &tolerance(k))
                .unwrap()
                .body;
            let bend = edge_at(&body, frame, k, [5.0, 10.0, 5.0]);
            let used = faces_of(&body, bend);
            let ends = [
                shared(
                    &body,
                    edge_at(&body, frame, k, [40.0, 10.0, 5.0]),
                    edge_at(&body, frame, k, [40.0, 10.0, 0.0]),
                ),
                shared(
                    &body,
                    edge_at(&body, frame, k, [2.5, 0.0, 40.0]),
                    edge_at(&body, frame, k, [2.5, 20.0, 40.0]),
                ),
            ];
            let made = fillet(
                &body,
                &[FilletEdge {
                    edge: bend,
                    radius: 35.0 * k,
                }],
                &tolerance(k),
            )
            .unwrap_or_else(|e| panic!("{what}: {e}"));
            sound(&made.body, k, &what);
            assert_mass(&made.body, volume, area, k, &what);
            assert_eq!(made.body.faces.len(), 7, "{what}");
            let mut round = None;
            for (i, face) in made.faces.iter().enumerate() {
                match face {
                    FilletFace::Kept(from) => {
                        assert!(
                            from.iter().all(|f| !used.contains(f)),
                            "{what}: an inside face is left"
                        );
                    }
                    FilletFace::Fillet(_) => round = Some(FaceId(i as u32)),
                    FilletFace::Corner(_) => panic!("{what}: a corner"),
                }
            }
            // The round meets each leg's end face, where the face it used
            // up was.
            let round = round.unwrap();
            for end in ends {
                let now = made
                    .faces
                    .iter()
                    .position(|f| matches!(f, FilletFace::Kept(from) if from.contains(&end)))
                    .unwrap();
                assert!(neighbours(&made.body, round, FaceId(now as u32)), "{what}");
            }
        }
    }
}
