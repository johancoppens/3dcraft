//! Each loop of a face must be a simple closed curve on its surface
//! (`Defect::LoopNotSimple`, `docs/design/topology.md` › *Loops that are
//! simple*): it goes round what it encloses once, round a surface that
//! closes at most once, has no edge that goes round its curve more than
//! once, and runs over neither itself nor another loop. Seeded with loops
//! that go round their circle twice (the plate's finding of 2026-11-14,
//! built by hand), run back over themselves, or touch another loop along a
//! stretch, at three scales and turned; the valid twins, the reference
//! solids, seams, slits, poles, thin faces and a face with fifty holes stay
//! clean; hostile bodies give defects, never a panic.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{
    Circle, Curve3, Cylinder, Line, NurbsCurve, NurbsSurface, Plane, Surface, Torus,
};
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_testkit::rings::{
    arc_chain, band, bridged_washer, curved_flange, degrees, disc, disc_with_spur, lid,
    plate_with_hole, spline_disc, spline_disc_back, spline_lid_back, spline_spike, turned,
};
use craft3d_testkit::{solids, trimmed};
use craft3d_topo::{
    Body, BodyKind, Coedge, Defect, EdgeId, FaceId, LoopId, check, loops_not_simple, regionless,
};
use std::f64::consts::{PI, TAU};

fn tol() -> Tolerance {
    Tolerance::default()
}

/// The (face, loop) pairs the check reports as not simple.
fn not_simple(body: &Body) -> Vec<(u32, u32)> {
    not_simple_at(body, &tol())
}

/// The (face, loop) pairs the check at `tol` reports as not simple.
fn not_simple_at(body: &Body, tol: &Tolerance) -> Vec<(u32, u32)> {
    check(body, tol)
        .iter()
        .filter_map(|d| match d {
            Defect::LoopNotSimple { face, loop_, .. } => Some((face.0, loop_.0)),
            _ => None,
        })
        .collect()
}

fn forward(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: false,
    }
}

fn backward(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: true,
    }
}

/// The frames the rows are built in: the world's, and one turned and moved.
fn frames(scale: f64) -> Vec<Frame> {
    vec![
        Frame::WORLD,
        Frame::new(
            Vec3::new(0.3, -1.7, 2.1) * scale,
            Vec3::new(0.2, -0.5, 0.84),
            Vec3::new(0.9, 0.3, 0.1),
        )
        .unwrap(),
    ]
}

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

#[test]
fn loops_that_go_round_twice_are_found_on_every_face() {
    let once = degrees(&[0.0, 120.0, 240.0, 360.0]);
    let twice = degrees(&[0.0, 240.0, 480.0, 720.0]);
    let long = degrees(&[0.0, 120.0, 479.4, 720.0]);
    for scale in SCALES {
        for f in frames(scale) {
            let r = scale;
            // The disc: wall (face 0, loops 0 and 1), floor (face 1, loop
            // 2), lid (face 2, loop 3).
            let both = disc(&f, r, r, &twice, &twice).unwrap();
            assert_eq!(not_simple(&both), vec![(0, 0), (1, 2), (2, 3)], "{scale}");
            let top = disc(&f, r, r, &twice, &once).unwrap();
            assert_eq!(not_simple(&top), vec![(0, 1), (2, 3)], "{scale}");
            let uneven = disc(&f, r, r, &long, &long).unwrap();
            assert_eq!(not_simple(&uneven), vec![(0, 0), (1, 2), (2, 3)], "{scale}");
            // The hole of a plate wound twice: lid (face 0, its hole loop
            // 1), floor (face 1, loop 3), the hole's wall (face 6, loop 8).
            let plate = plate_with_hole(&f, 4.0 * r, r, r, &twice, &twice).unwrap();
            assert_eq!(not_simple(&plate), vec![(0, 1), (1, 3), (6, 8)], "{scale}");
            let sheet = lid(&f, r, &twice).unwrap();
            assert_eq!(not_simple(&sheet), vec![(0, 0)], "{scale}");
            let wall = band(&f, r, r, &twice, &twice).unwrap();
            assert_eq!(not_simple(&wall), vec![(0, 0)], "{scale}");
            let spline = spline_disc(&f, r, r, 2).unwrap();
            assert_eq!(not_simple(&spline), vec![(0, 0), (1, 2), (2, 3)], "{scale}");
            // Their twins that go round once are valid.
            for body in [
                disc(&f, r, r, &once, &once).unwrap(),
                plate_with_hole(&f, 4.0 * r, r, r, &once, &once).unwrap(),
                lid(&f, r, &once).unwrap(),
                band(&f, r, r, &once, &once).unwrap(),
                spline_disc(&f, r, r, 1).unwrap(),
            ] {
                assert_eq!(check(&body, &tol()), vec![], "{scale}");
            }
        }
    }
}

#[test]
fn loops_that_run_back_over_themselves_are_found() {
    let once = degrees(&[0.0, 120.0, 240.0, 360.0]);
    let back = degrees(&[0.0, 359.4, 300.0, 360.0]);
    for scale in SCALES {
        for f in frames(scale) {
            let r = scale;
            // Once round, area and volume right: only the overlap is wrong.
            let body = disc(&f, r, r, &back, &once).unwrap();
            assert_eq!(not_simple(&body), vec![(0, 1), (2, 3)], "{scale}");
            // The first and the last arc run the same way over the stretch
            // the middle one runs back along: no cut, also on a sheet.
            let sheet = lid(&f, r, &back).unwrap();
            assert_eq!(not_simple(&sheet), vec![(0, 0)], "{scale}");
            // The same with spline edges (the sampled test).
            let spline = spline_disc_back(&f, r, r).unwrap();
            assert_eq!(not_simple(&spline), vec![(0, 1), (2, 3)], "{scale}");
            let spline_sheet = spline_lid_back(&f, r).unwrap();
            assert_eq!(not_simple(&spline_sheet), vec![(0, 0)], "{scale}");
            // A spur in a solid's face, one edge out and back, is a cut
            // of the face along an edge it holds on both sides: the region
            // and its measure are the disc's, and a bridge (sheet metal's
            // collars) is the same at one place of a plane. Not refused
            // (it was until the review of 2026-11-14 found the collars).
            let spur = disc_with_spur(&f, r, r).unwrap();
            assert_eq!(check(&spur, &tol()), vec![], "{scale}");
        }
    }
}

/// A sheet may have a cut: a slit into a face, two edges whose sides run
/// over each other opposite ways (origami builds them,
/// `docs/design/origami.md` › *Slits and slots*), or a hole whose side
/// lies on the outer loop's. The same face in a solid is refused, also
/// right after it passed in the sheet (the faces that passed are
/// remembered by a key that holds whether the body is a sheet: without it
/// the solid passed here). One edge used twice (a spur) is not judged in
/// either.
#[test]
fn a_sheet_may_have_a_cut_and_a_solid_may_not() {
    for scale in SCALES {
        for f in frames(scale) {
            let mut cuts = vec![
                spline_spike(&f, scale, true).unwrap(),
                square_with_touching_hole(&f, scale),
            ];
            cuts.push(square_with_spur(&f, scale, false));
            let mut spur = square_with_spur(&f, scale, true);
            assert_eq!(not_simple(&spur), vec![], "{scale}");
            spur.kind = BodyKind::Solid;
            assert_eq!(not_simple(&spur), vec![], "{scale}");
            for sheet in cuts {
                assert_eq!(not_simple(&sheet), vec![], "{scale}");
                let mut solid = sheet.clone();
                solid.kind = BodyKind::Solid;
                assert_eq!(not_simple(&solid), vec![(0, 0)], "{scale}");
            }
            assert_eq!(
                check(&spline_spike(&f, scale, false).unwrap(), &tol()),
                vec![]
            );
        }
    }
}

#[test]
fn a_hole_along_its_outer_loop_and_an_edge_round_twice_are_found() {
    for scale in SCALES {
        for f in frames(scale) {
            let mut touching = square_with_touching_hole(&f, scale);
            touching.kind = BodyKind::Solid;
            assert_eq!(not_simple(&touching), vec![(0, 0)], "{scale}");
            let message = check(&touching, &tol())
                .iter()
                .map(ToString::to_string)
                .collect::<String>();
            assert!(message.contains("runs over loop 1"), "{message}");
            // One edge that runs round its circle twice.
            let mut body = Body::new(BodyKind::Sheet);
            let v = body.add_vertex(f.point(scale, 0.0, 0.0));
            let circle = Curve3::Circle(Circle::new(f, scale).unwrap());
            let e = body.add_edge(circle, (0.0, 2.0 * TAU), v, v);
            let lp = body.add_loop(vec![forward(e)]);
            let face = body.add_face(Surface::Plane(Plane::new(f)), false, vec![lp]);
            body.add_shell(vec![face]);
            let found = check(&body, &tol());
            assert_eq!(not_simple(&body), vec![(0, 0)], "{scale}: {found:?}");
            assert!(
                found[0]
                    .to_string()
                    .contains("goes round its circle 2.0000 times")
            );
        }
    }
}

#[test]
fn a_loop_twice_round_a_torus_is_found_and_once_is_not() {
    let once = degrees(&[0.0, 120.0, 240.0, 360.0]);
    let twice = degrees(&[0.0, 240.0, 480.0, 720.0]);
    for scale in SCALES {
        for f in frames(scale) {
            let good = torus_band(&f, scale, &once);
            assert_eq!(check(&good, &tol()), vec![], "{scale}");
            let bad = torus_band(&f, scale, &twice);
            assert_eq!(not_simple(&bad), vec![(0, 0)], "{scale}");
            let message = check(&bad, &tol())[0].to_string();
            assert!(message.contains("2 times about its axis"), "{message}");
        }
    }
}

/// A face between one and ten tolerances wide is thin, not degenerate:
/// two of its edges that far apart are two curves (the boolean corpus's
/// slivers of the `near` cases, refused when ten tolerances counted as
/// one curve, Heavy corpus 43548965835). Within the tolerance they are one
/// curve, and a solid's face so narrow along a stretch has no width there.
#[test]
fn a_face_wider_than_the_tolerance_is_not_two_edges_on_each_other() {
    let t = tol().linear;
    for scale in SCALES {
        for f in frames(scale) {
            for (width, refused) in [(3.0 * t, false), (9.0 * t, false), (0.5 * t, true)] {
                let mut strip = strip(&f, 10.0 * scale, width);
                strip.kind = BodyKind::Solid;
                assert_eq!(
                    !not_simple(&strip).is_empty(),
                    refused,
                    "{width:e} at {scale}"
                );
                let mut held = square_with_hole_near_its_side(&f, scale, width);
                held.kind = BodyKind::Solid;
                assert_eq!(
                    !not_simple(&held).is_empty(),
                    refused,
                    "{width:e} at {scale}"
                );
            }
        }
    }
}

/// A face that ends in a sharp tip: near it its two edges keep within
/// the tolerance of each other over the tolerance over the tip's angle
/// (here 0.1, at 1e-5 rad), but part there at that angle; two edges on
/// each other run parallel. (The crescents of a cone on a tilted drill,
/// `crates/testkit/tests/tess_faces.rs`, were refused as a loop running
/// over itself until the rule asked for that, Check 43550380946; this
/// face was too.)
#[test]
fn a_sharp_tip_is_not_two_edges_on_each_other() {
    for f in frames(1.0) {
        assert_eq!(not_simple(&sharp_tip(&f)), vec![]);
    }
}

#[test]
fn a_seam_of_a_spline_surface_not_known_to_close_is_a_seam() {
    let t = tol().linear;
    for delta in [0.0, 1e-8, 0.5 * t] {
        for reversed in [false, true] {
            let body = spline_wall_cylinder(&rational_cylinder(delta), reversed, None);
            assert_eq!(not_simple(&body), vec![], "{delta:e} {reversed}");
            if !reversed {
                assert_eq!(check(&body, &tol()), vec![], "{delta:e}");
            }
        }
        // A spur (one edge up and back) is not judged; two edges up and
        // back on each other are refused where the face can be charted,
        // found at one place by the walk: on the surface `closes` finds
        // closed too, since it is charted with its domain as its period
        // (2026-11-14; before, it was not charted and not held there).
        let spur = spline_wall_cylinder(&rational_cylinder(delta), false, Some(true));
        assert_eq!(not_simple(&spur), vec![], "{delta:e}");
        let spike = spline_wall_cylinder(&rational_cylinder(delta), false, Some(false));
        assert_eq!(not_simple(&spike), vec![(0, 0)], "{delta:e}");
    }
    let (surface, rim) = periodic_cylinder();
    assert_eq!(craft3d_geom::closure::closes(&surface), [false, false]);
    let body = periodic_wall_cylinder(&surface, &rim);
    assert_eq!(check(&body, &tol()), vec![]);
}

/// Rule 4's reach is the tighter of the two edges' own tolerances, not the
/// body's largest: a slab 500 tolerances thick with one edge loosened to
/// 900 tolerances, and a plate 0.4 thick in a document of tolerance 1e-3
/// with an edge loose to 0.5, are right and pass (each was refused on the
/// faces that do not hold the loose edge, and on the one that does; review
/// of 2026-11-14). An overlap on a loose body is still caught, and a face
/// whose two sides are both looser than its width is one curve; the face
/// cache tells the two apart (its key holds the edges' own tolerances).
#[test]
fn a_loose_edge_does_not_make_other_edges_one_curve() {
    let f = Frame::WORLD;
    for loose in [9e-4, 4e-4] {
        let mut slab = solids::block(&f, 10.0, 10.0, 5e-4).unwrap().body;
        let e = (0..slab.edges.len())
            .find(|&i| (slab.edges[i].range.1 - slab.edges[i].range.0 - 10.0).abs() < 1e-9)
            .unwrap();
        slab.set_edge_tolerance(EdgeId(e as u32), loose).unwrap();
        assert_eq!(check(&slab, &tol()), vec![], "{loose:e}");
    }
    let coarse = Tolerance::new(1e-3, 1e-9).unwrap();
    let mut plate = solids::block(&f, 100.0, 50.0, 0.4).unwrap().body;
    plate.set_edge_tolerance(EdgeId(0), 0.5).unwrap();
    assert_eq!(check(&plate, &coarse), vec![]);
    // An overlap with one of its arcs loose, and with an edge elsewhere
    // loose: still caught.
    let once = degrees(&[0.0, 120.0, 240.0, 360.0]);
    let back = degrees(&[0.0, 359.4, 300.0, 360.0]);
    for which in [3, 4, 0] {
        let mut body = disc(&f, 1.0, 1.0, &back, &once).unwrap();
        body.set_edge_tolerance(EdgeId(which), 1e-4).unwrap();
        assert_eq!(not_simple(&body), vec![(0, 1), (2, 3)], "edge {which}");
    }
    // A strip 5e-4 wide (as a solid's face): its sides are two curves at
    // the document's tolerance, one curve when both are looser than 5e-4,
    // checked one after the other.
    let mut strip = strip(&f, 10.0, 5e-4);
    strip.kind = BodyKind::Solid;
    assert_eq!(not_simple(&strip), vec![]);
    strip.set_edge_tolerance(EdgeId(0), 9e-4).unwrap();
    assert_eq!(not_simple(&strip), vec![], "one side loose");
    strip.set_edge_tolerance(EdgeId(2), 9e-4).unwrap();
    assert_eq!(not_simple(&strip), vec![(0, 0)], "both sides loose");
}

/// The rule reads no angle: a sharp tip passes, and two stretches of one
/// curve are caught, at every angular tolerance a document may set, in
/// either order (the face cache's key holds the angular tolerance too).
#[test]
fn every_angular_tolerance_gives_the_same_verdicts() {
    let once = degrees(&[0.0, 120.0, 240.0, 360.0]);
    let back = degrees(&[0.0, 359.4, 300.0, 360.0]);
    let overlap = disc(&Frame::WORLD, 1.0, 1.0, &back, &once).unwrap();
    let spike = spline_spike(&Frame::WORLD, 1.0, true).unwrap();
    let mut spike_solid = spike.clone();
    spike_solid.kind = BodyKind::Solid;
    let tip = sharp_tip(&Frame::WORLD);
    let mut angles = vec![1e-9, 1e-6, 2.6e-5, 1e-4, 1e-3];
    for _ in 0..2 {
        for &angular in &angles {
            let t = Tolerance::new(tol().linear, angular).unwrap();
            assert_eq!(not_simple_at(&tip, &t), vec![], "tip at {angular:e}");
            assert_eq!(
                not_simple_at(&overlap, &t),
                vec![(0, 1), (2, 3)],
                "{angular:e}"
            );
            assert_eq!(not_simple_at(&spike_solid, &t), vec![(0, 0)], "{angular:e}");
            assert_eq!(not_simple_at(&spike, &t), vec![], "a cut at {angular:e}");
        }
        angles.reverse();
    }
}

/// A seam on a cylinder of radius up to 7.5e5 in a document whose
/// tolerance scales with it: its two uses are a period apart in the angle,
/// a short way in parameters and the circumference on the surface. Two
/// places are told apart along each parameter at its speed (read in one
/// "reach" for both, ten tolerances over the axis's unit speed, the period
/// of 2π was within it: the plates and sheet-metal parts at 1e6 were
/// refused, Check 43556657265).
#[test]
fn a_seam_of_a_large_cylinder_at_a_coarse_tolerance_is_a_seam() {
    for r in [1e-3, 1.0, 7.5e5] {
        let coarse = Tolerance::new(1e-6 * r, 1e-9).unwrap();
        for f in frames(r) {
            assert_eq!(check(&seamed_cylinder(&f, r), &coarse), vec![], "{r}");
        }
    }
}

/// Faces that close on themselves round a hole, made by sheet metal: a
/// collar bent from a round hole's edge and a pan's rim bent up from a
/// disc, whose flat ends are annuli written as one loop with a bridge
/// edge used twice at one place of their plane (refused as "runs along
/// edge 18 and back over it" by the rule until the gate of 2026-11-14,
/// 43560145677, found them in `sheetmetal_curved`, `format_versions` and
/// the FreeCAD step); and a washer written the same way by hand. Right
/// bodies, at three scales.
#[test]
fn faces_closed_round_a_hole_with_a_bridge_are_valid() {
    for k in SCALES {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for pan in [false, true] {
            let body = curved_flange(k, pan).unwrap_or_else(|e| panic!("{k} {pan}: {e}"));
            assert_eq!(check(&body, &tol), vec![], "{k} {pan}");
        }
        for f in frames(k) {
            let washer = bridged_washer(&f, k).unwrap();
            assert_eq!(check(&washer, &tol), vec![], "{k}");
            assert_eq!(craft3d_topo::genus(&washer), Some(1));
        }
    }
}

#[test]
fn valid_faces_stay_valid() {
    for scale in SCALES {
        for f in frames(scale) {
            let r = scale;
            // A full circle as one edge; circles of two, three and sixty arcs.
            assert_eq!(
                check(&solids::cylinder(&f, r, r).unwrap().body, &tol()),
                vec![]
            );
            for n in [2, 3, 60] {
                let path: Vec<f64> = (0..=n).map(|k| TAU * k as f64 / n as f64).collect();
                let body = disc(&f, r, r, &path, &path).unwrap();
                assert_eq!(check(&body, &tol()), vec![], "{n} arcs at {scale}");
                let plate = plate_with_hole(&f, 4.0 * r, r, r, &path, &path).unwrap();
                assert_eq!(check(&plate, &tol()), vec![], "{n} arcs at {scale}");
            }
            // A seam, used twice by the wall's one loop.
            assert_eq!(check(&seamed_cylinder(&f, r), &tol()), vec![], "{scale}");
            // A slit: two edges alike at either side of it, a period apart.
            assert_eq!(check(&slit_sheet(&f, r), &tol()), vec![], "{scale}");
            // Fifty holes, nested in their plate.
            assert_eq!(
                check(&perforated(&f, scale, 10, 5), &tol()),
                vec![],
                "{scale}"
            );
            // Faces narrower than the polyline's error: a C of 3 % of its
            // radius and a lune 0.2 across between circles of radius 2.
            assert_eq!(check(&thin_c(&f, scale), &tol()), vec![], "{scale}");
            assert_eq!(check(&lune(&f, scale), &tol()), vec![], "{scale}");
            // A side split in two along one line.
            assert_eq!(check(&split_square(&f, scale), &tol()), vec![], "{scale}");
            // Poles, apexes, spline edges and faces, ellipses, tori.
            let mut all = solids::all(&f, scale).unwrap();
            all.extend(trimmed::all(&f, scale).unwrap());
            all.push(solids::spline_cone(&f, scale, 2.0 * scale).unwrap());
            for reference in all {
                assert_eq!(
                    check(&reference.body, &tol()),
                    vec![],
                    "{} at {scale}",
                    reference.name
                );
            }
        }
    }
}

#[test]
fn hostile_bodies_give_defects_not_a_panic() {
    let f = Frame::WORLD;
    let once = degrees(&[0.0, 120.0, 240.0, 360.0]);
    let mut bodies = Vec::new();
    // A vertex that is not finite.
    let mut b = lid(&f, 1.0, &once).unwrap();
    b.vertices[0].point = Vec3::new(f64::NAN, 0.0, 0.0);
    bodies.push(b);
    // A range that is not finite, and one of no length.
    let mut b = lid(&f, 1.0, &once).unwrap();
    b.edges[1].range = (f64::NAN, 1.0);
    bodies.push(b);
    let mut b = lid(&f, 1.0, &once).unwrap();
    b.edges[1].range = (1.0, 1.0 + 1e-300);
    bodies.push(b);
    // A range of a thousand turns.
    let mut b = lid(&f, 1.0, &once).unwrap();
    b.edges[1].range = (0.0, 1000.0 * TAU);
    bodies.push(b);
    // An empty loop; a loop of an edge that does not exist; a face of a
    // loop that does not exist.
    let mut b = lid(&f, 1.0, &once).unwrap();
    b.loops[0].coedges.clear();
    bodies.push(b);
    let mut b = lid(&f, 1.0, &once).unwrap();
    b.loops[0].coedges.push(forward(EdgeId(99)));
    bodies.push(b);
    let mut b = lid(&f, 1.0, &once).unwrap();
    b.faces[0].loops.push(LoopId(99));
    bodies.push(b);
    // An infinite line's range.
    let mut b = square_with_spur(&f, 1.0, true);
    b.edges[0].range = (f64::NEG_INFINITY, f64::INFINITY);
    bodies.push(b);
    for body in &bodies {
        let defects = check(body, &tol());
        assert!(!defects.is_empty(), "{body:?}");
        // The rules asked directly, past the checks before them.
        let _ = loops_not_simple(body, &tol());
        let _ = regionless(body, &tol());
    }
    // An empty body, and a face with no loops.
    let _ = loops_not_simple(&Body::new(BodyKind::Sheet), &tol());
    let mut b = Body::new(BodyKind::Sheet);
    let face = b.add_face(Surface::Plane(Plane::new(f)), false, vec![]);
    b.add_shell(vec![face]);
    let _ = check(&b, &tol());
    assert!(loops_not_simple(&b, &tol()).is_empty());
}

#[test]
fn the_faults_name_the_face_and_the_loop() {
    let twice = degrees(&[0.0, 240.0, 480.0, 720.0]);
    let body = disc(&Frame::WORLD, 1.0, 1.0, &twice, &twice).unwrap();
    let found = loops_not_simple(&body, &tol());
    assert_eq!(
        found.iter().map(|(f, l, _)| (*f, *l)).collect::<Vec<_>>(),
        vec![
            (FaceId(0), LoopId(0)),
            (FaceId(1), LoopId(2)),
            (FaceId(2), LoopId(3))
        ]
    );
    let lid_defect = check(&body, &tol())
        .into_iter()
        .find(|d| {
            matches!(
                d,
                Defect::LoopNotSimple {
                    face: FaceId(2),
                    ..
                }
            )
        })
        .unwrap();
    let message = lid_defect.to_string();
    assert!(
        message.starts_with(
            "loop 3 of face 2 is not a simple closed curve: it winds 2 times about the point"
        ),
        "{message}"
    );
    // Checked again (the faces that passed are remembered): the same.
    assert_eq!(loops_not_simple(&body, &tol()), found);
}

// Builders.

fn line(body: &mut Body, a: craft3d_topo::VertexId, b: craft3d_topo::VertexId) -> EdgeId {
    let (pa, pb) = (
        body.vertices[a.0 as usize].point,
        body.vertices[b.0 as usize].point,
    );
    body.add_edge(
        Curve3::Line(Line::through(pa, pb).unwrap()),
        (0.0, pa.distance(pb)),
        a,
        b,
    )
}

/// A plane sheet: the square 4 × 4 with a spike from the middle of its
/// right side 2 out and back, one edge walked twice (`same_edge`) or two
/// edges alike.
fn square_with_spur(f: &Frame, s: f64, same_edge: bool) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let at = |x: f64, y: f64| f.point(x * s, y * s, 0.0);
    let v: Vec<_> = [
        (0.0, 0.0),
        (4.0, 0.0),
        (4.0, 2.0),
        (6.0, 2.0),
        (4.0, 4.0),
        (0.0, 4.0),
    ]
    .iter()
    .map(|&(x, y)| body.add_vertex(at(x, y)))
    .collect();
    let e01 = line(&mut body, v[0], v[1]);
    let e12 = line(&mut body, v[1], v[2]);
    let out = line(&mut body, v[2], v[3]);
    let back = if same_edge {
        backward(out)
    } else {
        forward(line(&mut body, v[3], v[2]))
    };
    let e24 = line(&mut body, v[2], v[4]);
    let e45 = line(&mut body, v[4], v[5]);
    let e50 = line(&mut body, v[5], v[0]);
    let lp = body.add_loop(vec![
        forward(e01),
        forward(e12),
        forward(out),
        back,
        forward(e24),
        forward(e45),
        forward(e50),
    ]);
    let face = body.add_face(Surface::Plane(Plane::new(*f)), false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// A plane sheet: the square 4 × 4 with a square hole 1 × 1 whose lower
/// side lies on the square's (an edge of its own).
fn square_with_touching_hole(f: &Frame, s: f64) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let at = |x: f64, y: f64| f.point(x * s, y * s, 0.0);
    let o: Vec<_> = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]
        .iter()
        .map(|&(x, y)| body.add_vertex(at(x, y)))
        .collect();
    let h: Vec<_> = [(1.0, 0.0), (1.0, 1.0), (2.0, 1.0), (2.0, 0.0)]
        .iter()
        .map(|&(x, y)| body.add_vertex(at(x, y)))
        .collect();
    let outer: Vec<_> = (0..4)
        .map(|k| forward(line(&mut body, o[k], o[(k + 1) % 4])))
        .collect();
    let hole: Vec<_> = (0..4)
        .map(|k| forward(line(&mut body, h[k], h[(k + 1) % 4])))
        .collect();
    let loops = vec![body.add_loop(outer), body.add_loop(hole)];
    let face = body.add_face(Surface::Plane(Plane::new(*f)), false, loops);
    body.add_shell(vec![face]);
    body
}

/// A sheet of a torus about the frame's z (major 3 s, minor s) between
/// the circles about the axis at tube angles −0.5 and 0.5, both walked
/// along `path`.
fn torus_band(f: &Frame, s: f64, path: &[f64]) -> Body {
    let (major, minor) = (3.0 * s, s);
    let mut body = Body::new(BodyKind::Sheet);
    let rim = |body: &mut Body, v: f64| {
        let (sv, cv) = v.psin_cos();
        let frame = f.at(f.point(0.0, 0.0, minor * sv)).unwrap();
        arc_chain(body, &frame, major + minor * cv, path).unwrap()
    };
    let low = rim(&mut body, -0.5);
    let high = rim(&mut body, 0.5);
    let loops = vec![body.add_loop(low), body.add_loop(turned(&high))];
    let face = body.add_face(
        Surface::Torus(Torus::new(*f, major, minor).unwrap()),
        false,
        loops,
    );
    body.add_shell(vec![face]);
    body
}

/// A cylinder whose wall is one loop with a seam: the bottom circle, the
/// seam up, the top circle back, the seam down.
fn seamed_cylinder(f: &Frame, r: f64) -> Body {
    let mut body = Body::new(BodyKind::Solid);
    let top_frame = f.at(f.point(0.0, 0.0, r)).unwrap();
    let vb = body.add_vertex(f.point(r, 0.0, 0.0));
    let vt = body.add_vertex(top_frame.point(r, 0.0, 0.0));
    let bottom = body.add_edge(
        Curve3::Circle(Circle::new(*f, r).unwrap()),
        (0.0, TAU),
        vb,
        vb,
    );
    let top = body.add_edge(
        Curve3::Circle(Circle::new(top_frame, r).unwrap()),
        (0.0, TAU),
        vt,
        vt,
    );
    let seam = line(&mut body, vb, vt);
    let wall_loop = body.add_loop(vec![
        forward(bottom),
        forward(seam),
        backward(top),
        backward(seam),
    ]);
    let wall = body.add_face(
        Surface::Cylinder(Cylinder::new(*f, r).unwrap()),
        false,
        vec![wall_loop],
    );
    let floor_loop = body.add_loop(vec![backward(bottom)]);
    let floor = body.add_face(
        Surface::Plane(Plane::new(f.reversed_at(f.origin()).unwrap())),
        false,
        vec![floor_loop],
    );
    let lid_loop = body.add_loop(vec![forward(top)]);
    let lid = body.add_face(Surface::Plane(Plane::new(top_frame)), false, vec![lid_loop]);
    body.add_shell(vec![wall, floor, lid]);
    body
}

/// A sheet of a whole turn of a cylinder slit along a line: the two sides
/// of the slit are edges alike between vertices at the same points, a
/// period apart in the face's parameters.
fn slit_sheet(f: &Frame, r: f64) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let top_frame = f.at(f.point(0.0, 0.0, r)).unwrap();
    let b0 = body.add_vertex(f.point(r, 0.0, 0.0));
    let b1 = body.add_vertex(f.point(r, 0.0, 0.0));
    let t0 = body.add_vertex(top_frame.point(r, 0.0, 0.0));
    let t1 = body.add_vertex(top_frame.point(r, 0.0, 0.0));
    let bottom = body.add_edge(
        Curve3::Circle(Circle::new(*f, r).unwrap()),
        (0.0, TAU),
        b0,
        b1,
    );
    let top = body.add_edge(
        Curve3::Circle(Circle::new(top_frame, r).unwrap()),
        (0.0, TAU),
        t0,
        t1,
    );
    let far = line(&mut body, b1, t1);
    let near = line(&mut body, b0, t0);
    let lp = body.add_loop(vec![
        forward(bottom),
        forward(far),
        backward(top),
        backward(near),
    ]);
    let face = body.add_face(
        Surface::Cylinder(Cylinder::new(*f, r).unwrap()),
        false,
        vec![lp],
    );
    body.add_shell(vec![face]);
    body
}

/// A plane sheet `2 nx × 2 ny` (times `s`) with a round hole of radius
/// 0.4 in every cell of 2 × 2.
fn perforated(f: &Frame, s: f64, nx: usize, ny: usize) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let (w, h) = (2.0 * nx as f64, 2.0 * ny as f64);
    let at = |x: f64, y: f64| f.point(x * s, y * s, 0.0);
    let o: Vec<_> = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
        .iter()
        .map(|&(x, y)| body.add_vertex(at(x, y)))
        .collect();
    let outer: Vec<_> = (0..4)
        .map(|k| forward(line(&mut body, o[k], o[(k + 1) % 4])))
        .collect();
    let mut loops = vec![body.add_loop(outer)];
    for i in 0..nx {
        for j in 0..ny {
            let c = at(1.0 + 2.0 * i as f64, 1.0 + 2.0 * j as f64);
            let frame = f.at(c).unwrap();
            let v = body.add_vertex(frame.point(0.4 * s, 0.0, 0.0));
            let e = body.add_edge(
                Curve3::Circle(Circle::new(frame, 0.4 * s).unwrap()),
                (0.0, TAU),
                v,
                v,
            );
            loops.push(body.add_loop(vec![backward(e)]));
        }
    }
    let face = body.add_face(Surface::Plane(Plane::new(*f)), false, loops);
    body.add_shell(vec![face]);
    body
}

/// A plane sheet: a C between arcs of radius s and 0.97 s about the
/// origin, from 30° to 330°.
fn thin_c(f: &Frame, s: f64) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let (a0, a1) = (PI / 6.0, 11.0 * PI / 6.0);
    let at = |r: f64, a: f64| {
        let (sa, ca) = a.psin_cos();
        f.point(r * ca, r * sa, 0.0)
    };
    let (ro, ri) = (s, 0.97 * s);
    let v: Vec<_> = [(ro, a0), (ro, a1), (ri, a1), (ri, a0)]
        .iter()
        .map(|&(r, a)| body.add_vertex(at(r, a)))
        .collect();
    let outer = body.add_edge(
        Curve3::Circle(Circle::new(*f, ro).unwrap()),
        (a0, a1),
        v[0],
        v[1],
    );
    let end = line(&mut body, v[1], v[2]);
    let inner = body.add_edge(
        Curve3::Circle(Circle::new(*f, ri).unwrap()),
        (a0, a1),
        v[3],
        v[2],
    );
    let start = line(&mut body, v[3], v[0]);
    let lp = body.add_loop(vec![
        forward(outer),
        forward(end),
        backward(inner),
        forward(start),
    ]);
    let face = body.add_face(Surface::Plane(Plane::new(*f)), false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// A plane sheet: the lune inside the circle of radius 2 s about the
/// origin and outside the one of the same radius about (0, 0.2 s).
fn lune(f: &Frame, s: f64) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let r = 2.0 * s;
    let x = (r * r - (0.1 * s) * (0.1 * s)).sqrt();
    let (left, right) = (f.point(-x, 0.1 * s, 0.0), f.point(x, 0.1 * s, 0.0));
    let (vl, vr) = (body.add_vertex(left), body.add_vertex(right));
    let tilt = (0.05_f64).pasin();
    // The lower arc of the first circle, counter-clockwise from the left
    // corner to the right one.
    let low = body.add_edge(
        Curve3::Circle(Circle::new(*f, r).unwrap()),
        (PI - tilt, TAU + tilt),
        vl,
        vr,
    );
    // The lower arc of the second, walked back from the right corner.
    let other = f.at(f.point(0.0, 0.2 * s, 0.0)).unwrap();
    let high = body.add_edge(
        Curve3::Circle(Circle::new(other, r).unwrap()),
        (PI + tilt, TAU - tilt),
        vl,
        vr,
    );
    let lp = body.add_loop(vec![forward(low), backward(high)]);
    let face = body.add_face(Surface::Plane(Plane::new(*f)), false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// A plane sheet: the unit square times 4 with its lower side split in
/// two along one line.
fn split_square(f: &Frame, s: f64) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let at = |x: f64, y: f64| f.point(x * s, y * s, 0.0);
    let v: Vec<_> = [(0.0, 0.0), (1.5, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]
        .iter()
        .map(|&(x, y)| body.add_vertex(at(x, y)))
        .collect();
    let coedges: Vec<_> = (0..5)
        .map(|k| forward(line(&mut body, v[k], v[(k + 1) % 5])))
        .collect();
    let lp = body.add_loop(coedges);
    let face = body.add_face(Surface::Plane(Plane::new(*f)), false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// A plane sheet: the rectangle `length × width` from the frame's origin.
fn strip(f: &Frame, length: f64, width: f64) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let v: Vec<_> = [(0.0, 0.0), (length, 0.0), (length, width), (0.0, width)]
        .iter()
        .map(|&(x, y)| body.add_vertex(f.point(x, y, 0.0)))
        .collect();
    let coedges: Vec<_> = (0..4)
        .map(|k| forward(line(&mut body, v[k], v[(k + 1) % 4])))
        .collect();
    let lp = body.add_loop(coedges);
    let face = body.add_face(Surface::Plane(Plane::new(*f)), false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// A plane sheet: the square 4 × 4 (times `s`) with a square hole 1 × 1
/// whose lower side lies `gap` above the square's.
fn square_with_hole_near_its_side(f: &Frame, s: f64, gap: f64) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let at = |x: f64, y: f64| f.point(x * s, y * s, 0.0);
    let o: Vec<_> = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]
        .iter()
        .map(|&(x, y)| body.add_vertex(at(x, y)))
        .collect();
    let h: Vec<_> = [(1.0, 0.0), (1.0, 1.0), (2.0, 1.0), (2.0, 0.0)]
        .iter()
        .map(|&(x, y)| body.add_vertex(at(x, y) + f.vector(0.0, gap, 0.0)))
        .collect();
    let outer: Vec<_> = (0..4)
        .map(|k| forward(line(&mut body, o[k], o[(k + 1) % 4])))
        .collect();
    let hole: Vec<_> = (0..4)
        .map(|k| forward(line(&mut body, h[k], h[(k + 1) % 4])))
        .collect();
    let loops = vec![body.add_loop(outer), body.add_loop(hole)];
    let face = body.add_face(Surface::Plane(Plane::new(*f)), false, loops);
    body.add_shell(vec![face]);
    body
}

/// A solid face that ends in a tip of 1e-5 rad at the frame's origin: the
/// last edge runs from a vertex on the long side 1e-2 from the tip back to
/// it, within 1e-7 (a tenth of the tolerance) of the first edge's line all
/// along, by a gap that grows from nothing at the tip.
fn sharp_tip(f: &Frame) -> Body {
    let angle = 1e-5;
    let mut body = Body::new(BodyKind::Solid);
    let at = |x: f64, y: f64| f.point(x, y, 0.0);
    let v: Vec<_> = [(0.0, 0.0), (1.0, 0.0), (1.0, angle), (1e-2, 1e-2 * angle)]
        .iter()
        .map(|&(x, y)| body.add_vertex(at(x, y)))
        .collect();
    let coedges: Vec<_> = (0..4)
        .map(|k| forward(line(&mut body, v[k], v[(k + 1) % 4])))
        .collect();
    let lp = body.add_loop(coedges);
    let face = body.add_face(Surface::Plane(Plane::new(*f)), false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// The rational quadratic cylinder of radius 1 and height 1 about z (a
/// net of 9 × 2), its last row (the seam's, the first one's again) moved
/// by `delta` along y.
fn rational_cylinder(delta: f64) -> Surface {
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let ring = [
        (1.0, 0.0, 1.0),
        (1.0, 1.0, h),
        (0.0, 1.0, 1.0),
        (-1.0, 1.0, h),
        (-1.0, 0.0, 1.0),
        (-1.0, -1.0, h),
        (0.0, -1.0, 1.0),
        (1.0, -1.0, h),
        (1.0, 0.0, 1.0),
    ];
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (i, (x, y, w)) in ring.iter().enumerate() {
        let shift = if i == 8 { delta } else { 0.0 };
        for z in [0.0, 1.0] {
            ctrl.push(Vec3::new(*x, *y + shift, z));
            weights.push(*w);
        }
    }
    Surface::Nurbs(
        NurbsSurface::new(
            2,
            1,
            vec![
                0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
            ],
            vec![0.0, 0.0, 1.0, 1.0],
            9,
            2,
            ctrl,
            weights,
        )
        .unwrap(),
    )
}

/// A solid whose wall is `surface` (a cylinder of radius 1 and height 1
/// about z, closing along u at x = 1, y = 0), its loop the bottom circle,
/// one seam edge up, the top circle back and the seam down (as a STEP
/// file's seam curve gives it); with `spur`, the bottom is two half
/// circles and the loop also runs up a ruling from (-1, 0, 0) to half the
/// height and back, along the same edge (`Some(true)`) or along a second
/// edge alike (`Some(false)`).
fn spline_wall_cylinder(surface: &Surface, reversed: bool, spur: Option<bool>) -> Body {
    let f = Frame::WORLD;
    let mut body = Body::new(BodyKind::Solid);
    let top_frame = f.at(f.point(0.0, 0.0, 1.0)).unwrap();
    let vb = body.add_vertex(f.point(1.0, 0.0, 0.0));
    let vt = body.add_vertex(top_frame.point(1.0, 0.0, 0.0));
    let circle = Curve3::Circle(Circle::new(f, 1.0).unwrap());
    let mut bottom = Vec::new();
    let mut extra = Vec::new();
    if let Some(same_edge) = spur {
        let vh = body.add_vertex(f.point(-1.0, 0.0, 0.0));
        let tip = body.add_vertex(f.point(-1.0, 0.0, 0.5));
        bottom.push(body.add_edge(circle.clone(), (0.0, PI), vb, vh));
        let up = line(&mut body, vh, tip);
        let down = if same_edge {
            backward(up)
        } else {
            forward(line(&mut body, tip, vh))
        };
        extra = vec![forward(up), down];
        bottom.push(body.add_edge(circle, (PI, TAU), vh, vb));
    } else {
        bottom.push(body.add_edge(circle, (0.0, TAU), vb, vb));
    }
    let top = body.add_edge(
        Curve3::Circle(Circle::new(top_frame, 1.0).unwrap()),
        (0.0, TAU),
        vt,
        vt,
    );
    let seam = line(&mut body, vb, vt);
    let mut wall = vec![forward(bottom[0])];
    wall.extend(extra);
    wall.extend(bottom.iter().skip(1).map(|e| forward(*e)));
    wall.extend([forward(seam), backward(top), backward(seam)]);
    let wall_loop = body.add_loop(wall);
    let wall = body.add_face(surface.clone(), reversed, vec![wall_loop]);
    let floor_loop = body.add_loop(bottom.iter().rev().map(|e| backward(*e)).collect());
    let floor = body.add_face(
        Surface::Plane(Plane::new(f.reversed_at(f.origin()).unwrap())),
        false,
        vec![floor_loop],
    );
    let lid_loop = body.add_loop(vec![forward(top)]);
    let lid = body.add_face(Surface::Plane(Plane::new(top_frame)), false, vec![lid_loop]);
    body.add_shell(vec![wall, floor, lid]);
    body
}

/// A closed quadratic B-spline cylinder on knots that are not clamped
/// (uniform, eight control points round and the first two again: it
/// closes, which `closure::closes` does not see), height 1, and its rim
/// at height 0 as a spline curve on the same knots.
fn periodic_cylinder() -> (Surface, NurbsCurve) {
    let round: Vec<(f64, f64)> = (0..8)
        .map(|k| {
            let (s, c) = (TAU * k as f64 / 8.0).psin_cos();
            (1.1 * c, 1.1 * s)
        })
        .collect();
    let wrapped: Vec<(f64, f64)> = round.iter().chain(round.iter().take(2)).copied().collect();
    let knots: Vec<f64> = (0..=12).map(f64::from).collect();
    let mut ctrl = Vec::new();
    for (x, y) in &wrapped {
        for z in [0.0, 1.0] {
            ctrl.push(Vec3::new(*x, *y, z));
        }
    }
    let n = wrapped.len();
    let surface = NurbsSurface::new(
        2,
        1,
        knots.clone(),
        vec![0.0, 0.0, 1.0, 1.0],
        n,
        2,
        ctrl,
        vec![1.0; 2 * n],
    )
    .unwrap();
    let rim = NurbsCurve::polynomial(
        2,
        knots,
        wrapped
            .iter()
            .map(|(x, y)| Vec3::new(*x, *y, 0.0))
            .collect(),
    )
    .unwrap();
    (Surface::Nurbs(surface), rim)
}

/// A solid whose wall is the unclamped cylinder, with a seam edge used
/// twice, its floor and lid bounded by its rims.
fn periodic_wall_cylinder(surface: &Surface, rim: &NurbsCurve) -> Body {
    let f = Frame::WORLD;
    let mut body = Body::new(BodyKind::Solid);
    let (t0, t1) = rim.domain();
    let lift = |c: &NurbsCurve, z: f64| {
        NurbsCurve::polynomial(
            c.degree(),
            c.knots().to_vec(),
            c.control_points()
                .iter()
                .map(|p| *p + Vec3::new(0.0, 0.0, z))
                .collect(),
        )
        .unwrap()
    };
    let top_rim = lift(rim, 1.0);
    let vb = body.add_vertex(rim.eval(t0));
    let vt = body.add_vertex(top_rim.eval(t0));
    let bottom = body.add_edge(Curve3::Nurbs(rim.clone()), (t0, t1), vb, vb);
    let top = body.add_edge(Curve3::Nurbs(top_rim), (t0, t1), vt, vt);
    let seam = line(&mut body, vb, vt);
    let wall_loop = body.add_loop(vec![
        forward(bottom),
        forward(seam),
        backward(top),
        backward(seam),
    ]);
    let wall = body.add_face(surface.clone(), false, vec![wall_loop]);
    let floor_loop = body.add_loop(vec![backward(bottom)]);
    let floor = body.add_face(
        Surface::Plane(Plane::new(f.reversed_at(f.origin()).unwrap())),
        false,
        vec![floor_loop],
    );
    let top_frame = f.at(f.point(0.0, 0.0, 1.0)).unwrap();
    let lid_loop = body.add_loop(vec![forward(top)]);
    let lid = body.add_face(Surface::Plane(Plane::new(top_frame)), false, vec![lid_loop]);
    body.add_shell(vec![wall, floor, lid]);
    body
}

/// `body` with its face `face` moved onto `surface` (the same points, other
/// parameters), turned over where `flip` (a surface whose normal points the
/// other way).
fn on_surface(mut body: Body, face: usize, (surface, flip): (&Surface, bool)) -> Body {
    body.faces[face].surface = surface.clone();
    body.faces[face].reversed ^= flip;
    body
}

/// `surface`, a spline surface, with its parameters swapped (the same
/// points: its normal points the other way).
fn transposed(surface: &Surface) -> Surface {
    let Surface::Nurbs(s) = surface else {
        return surface.clone();
    };
    let (degree_u, degree_v) = s.degrees();
    let (count_u, count_v) = s.counts();
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for j in 0..count_v {
        for i in 0..count_u {
            ctrl.push(s.control_points()[i * count_v + j]);
            weights.push(s.weights()[i * count_v + j]);
        }
    }
    Surface::Nurbs(
        NurbsSurface::new(
            degree_v,
            degree_u,
            s.knots_v().to_vec(),
            s.knots_u().to_vec(),
            count_v,
            count_u,
            ctrl,
            weights,
        )
        .unwrap(),
    )
}

/// `surface`, a spline surface, with its control points scaled by `scale`
/// about the origin (the same knots and weights).
fn scaled(surface: &Surface, scale: f64) -> Surface {
    let Surface::Nurbs(s) = surface else {
        return surface.clone();
    };
    let (degree_u, degree_v) = s.degrees();
    let (count_u, count_v) = s.counts();
    Surface::Nurbs(
        NurbsSurface::new(
            degree_u,
            degree_v,
            s.knots_u().to_vec(),
            s.knots_v().to_vec(),
            count_u,
            count_v,
            s.control_points().iter().map(|p| *p * scale).collect(),
            s.weights().to_vec(),
        )
        .unwrap(),
    )
}

/// A patch of the cylinder of radius `r` about z from 270° round to 450°
/// (across the seam at 0°, where the loop has no edge) and from height
/// `0.2 r` to `0.8 r`, on `surface`: an arc at the low height forward, a
/// ruling up at 90°, the high arc back, a ruling down at 270°; the right way
/// round unless `reversed`.
fn patch_across_the_seam(surface: &Surface, r: f64, reversed: bool) -> Body {
    let f = Frame::WORLD;
    let mut body = Body::new(BodyKind::Sheet);
    let low_frame = f.at(f.point(0.0, 0.0, 0.2 * r)).unwrap();
    let high_frame = f.at(f.point(0.0, 0.0, 0.8 * r)).unwrap();
    let (a, b) = (1.5 * PI, 2.5 * PI);
    let at = |frame: &Frame, t: f64| {
        let (s, c) = t.psin_cos();
        frame.point(r * c, r * s, 0.0)
    };
    let v = [
        body.add_vertex(at(&low_frame, a)),
        body.add_vertex(at(&low_frame, b)),
        body.add_vertex(at(&high_frame, b)),
        body.add_vertex(at(&high_frame, a)),
    ];
    let low = body.add_edge(
        Curve3::Circle(Circle::new(low_frame, r).unwrap()),
        (a, b),
        v[0],
        v[1],
    );
    let up = line(&mut body, v[1], v[2]);
    let high = body.add_edge(
        Curve3::Circle(Circle::new(high_frame, r).unwrap()),
        (a, b),
        v[3],
        v[2],
    );
    let down = line(&mut body, v[3], v[0]);
    let lp = body.add_loop(vec![
        forward(low),
        forward(up),
        backward(high),
        forward(down),
    ]);
    let face = body.add_face(surface.clone(), reversed, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// Loops on a spline surface that closes on clamped knots, which
/// `closure::closes` finds closed (a rational NURBS cylinder of radius `r`
/// and height `r` about z, as a STEP file brings one: the same points as
/// the analytic cylinder's, other parameters; and the same with its
/// parameters swapped, closed in v), are held as on the analytic cylinder:
/// its domain's width is the chart's period (`region::periods_of`,
/// `docs/design/topology.md` › *A spline surface that closes, charted*).
/// Until 2026-11-14 such a face was not charted at all: a band whose two
/// loops go round twice passed, a disc's wall did (its lid and floor, planes,
/// were refused), and a patch across the seam turned the wrong way round
/// passed; each is refused now, by the same rule as on the cylinder. Right
/// faces pass: a band and a disc once round, the patch across the seam (no
/// edge there), a wall with a seam edge used twice (as a STEP file's seam
/// gives it). Its twin that closes within the tolerance but not to
/// `closes`'s rounding (the seam's row moved by 1e-8) is charted as a
/// surface that does not close and is still not held to them: the band
/// gone round twice passes there (owed). Three scales, in the world and
/// turned and moved.
#[test]
fn loops_on_a_closed_spline_surface_are_held_as_on_a_cylinder() {
    let once = degrees(&[0.0, 120.0, 240.0, 360.0]);
    let twice = degrees(&[0.0, 240.0, 480.0, 720.0]);
    let w = Frame::WORLD;
    for r in SCALES {
        let closed = scaled(&rational_cylinder(0.0), r);
        assert_eq!(craft3d_geom::closure::closes(&closed), [true, false]);
        let closed_v = transposed(&closed);
        assert_eq!(craft3d_geom::closure::closes(&closed_v), [false, true]);
        // The seam's row 1e-8 off, within the tolerance; at 1e6 that is below
        // `closes`'s rounding of the net (1e-9 of its size), which then finds
        // it closed: no twin there.
        let near = scaled(&rational_cylinder(1e-8 / r), r);
        let near = (r <= 1.0).then_some(near);
        if let Some(near) = &near {
            assert_eq!(craft3d_geom::closure::closes(near), [false, false]);
        }
        let cylinder = Surface::Cylinder(Cylinder::new(w, r).unwrap());
        let band_of = |s: (&Surface, bool), arcs: &[f64]| {
            on_surface(band(&w, r, r, arcs, arcs).unwrap(), 0, s)
        };
        let disc_of = |s: (&Surface, bool), arcs: &[f64]| {
            on_surface(disc(&w, r, r, arcs, arcs).unwrap(), 0, s)
        };
        let patch_of = |(s, flip): (&Surface, bool), reversed: bool| {
            patch_across_the_seam(s, r, reversed ^ flip)
        };
        let charted = [(&cylinder, false), (&closed, false), (&closed_v, true)];
        for place in frames(r) {
            let moved = |body: Body| {
                body.transformed(&craft3d_math::Transform::from_frame(&place))
                    .unwrap()
            };
            let every = charted
                .iter()
                .copied()
                .chain(near.as_ref().map(|s| (s, false)));
            for surface in every {
                let kind = format!("{r}: {:?}", craft3d_geom::closure::closes(surface.0));
                let mut right = vec![
                    band_of(surface, &once),
                    disc_of(surface, &once),
                    patch_of(surface, false),
                ];
                if !surface.1 && !matches!(surface.0, Surface::Cylinder(_)) && r == 1.0 {
                    right.push(spline_wall_cylinder(surface.0, false, None));
                }
                for body in right {
                    assert_eq!(check(&moved(body), &tol()), vec![], "{kind}");
                }
            }
            for surface in charted {
                let kind = format!("{r}: {:?}", craft3d_geom::closure::closes(surface.0));
                assert_eq!(
                    not_simple(&moved(band_of(surface, &twice))),
                    vec![(0, 0)],
                    "{kind}"
                );
                assert_eq!(
                    not_simple(&moved(disc_of(surface, &twice))),
                    vec![(0, 0), (1, 2), (2, 3)],
                    "{kind}"
                );
                let wrong = regionless(&moved(patch_of(surface, true)), &tol());
                assert!(
                    wrong
                        .iter()
                        .any(|(_, why)| why.contains("every loop runs clockwise")),
                    "{kind}: {wrong:?}"
                );
            }
            // Not known to close: not held (a pass, never a refusal).
            if let Some(near) = &near {
                assert_eq!(
                    check(&moved(band_of((near, false), &twice)), &tol()),
                    vec![]
                );
            }
        }
    }
}
