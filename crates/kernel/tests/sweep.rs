//! Sweeps: a profile carried along a path.
//!
//! The oracles are closed forms. Along lines, arcs and mitred corners the
//! volume of a sweep is the area of the profile times the length its
//! centre of area travels, and the area of its sides the length of the
//! profile's boundary times the length the centre of that boundary
//! travels (Pappus for the bends; for a prism cut by slanted planes the
//! same holds, since the length of each of its lines is linear in where
//! the line is). The lengths are written out in the tests from the path
//! as drawn, the areas and centres from the profile as drawn (for a
//! spline: by quadrature here). A second oracle is the union, by the
//! boolean, of an extrusion, a revolution and an extrusion built apart.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{arc, circle, line, polygon, rectangle, scale_curves};
use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::{Role, extrude};
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_kernel::sweep::{Swept, sweep};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::{Body, check, genus, mass_properties};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

type P = [f64; 2];

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tol() -> Tolerance {
    Tolerance::default()
}

/// The plane the profiles are drawn on: through the world's origin,
/// facing −y, with the sketch's x along the world's x and its y along the
/// world's z. A path in the world's xy plane that leaves the origin
/// along +y is square to it, and passes through it from the front.
fn front() -> Frame {
    Frame::new(Vec3::ZERO, -Vec3::Y, Vec3::X).unwrap()
}

/// The same plane seen from the other side: the sketch's y is the
/// world's −z, and the path passes through it from behind.
fn back() -> Frame {
    Frame::new(Vec3::ZERO, Vec3::Y, Vec3::X).unwrap()
}

fn profile(curves: &[SketchCurve]) -> Profile {
    let profiles = detect_profiles(curves, &tol()).unwrap();
    assert_eq!(profiles.len(), 1);
    profiles[0].clone()
}

fn knots(degree: usize, count: usize) -> Vec<f64> {
    let spans = count - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend(vec![1.0; degree + 1]);
    knots
}

fn spline(id: u64, degree: usize, control: &[P]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree,
            knots: knots(degree, control.len()),
            control: control.to_vec(),
        }),
    }
}

/// Line, quarter bend to the left, line: from the origin 10 along +y, a
/// quarter turn of radius 5 about (−5, 10), then 12 along −x. Ids 101,
/// 102, 103.
fn bent(k: f64) -> Vec<SketchCurve> {
    scale_curves(
        &[
            line(101, [0.0, 0.0], [0.0, 10.0]),
            arc(102, [-5.0, 10.0], [0.0, 10.0], [-5.0, 15.0]),
            line(103, [-5.0, 15.0], [-17.0, 15.0]),
        ],
        k,
    )
}

/// What the centre of a point of the profile travels along `bent`, when
/// it is `a` to the left of the path.
fn bent_length(a: f64) -> f64 {
    10.0 + (5.0 - a) * FRAC_PI_2 + 12.0
}

fn kinds(swept: &Swept, role: Role) -> Vec<&'static str> {
    swept
        .body
        .faces
        .iter()
        .zip(&swept.faces)
        .filter(|(_, origin)| origin.role == role)
        .map(|(face, _)| face.surface.kind())
        .collect()
}

fn count(kinds: &[&str], kind: &str) -> usize {
    kinds.iter().filter(|k| **k == kind).count()
}

/// The volume of the mesh at the chordal tolerance `chord`; the mesh has
/// to be closed.
fn mesh_volume(body: &Body, chord: f64) -> f64 {
    let mesh = tessellate_body(
        body,
        &TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        },
    )
    .unwrap();
    let welded = mesh.weld().unwrap();
    assert!(welded.is_closed(), "the mesh is not watertight at {chord}");
    welded.volume()
}

/// Everything a good sweep has to be: valid, measurable with its outside
/// out, not passing through itself, watertight as a mesh at three
/// tolerances with the mesh's volume tending to the measured one. The
/// tolerances are parts of `thickness`, the size of the profile: a pipe
/// is long and thin, and a chord that is a hundredth of its length is as
/// wide as the pipe. Returns (volume, area, centre of volume).
fn sound_with(swept: &Swept, thickness: f64, tolerance: &Tolerance) -> (f64, f64, Vec3) {
    let body = &swept.body;
    assert_eq!(check(body, tolerance), vec![]);
    assert_eq!(body.faces.len(), swept.faces.len());
    let mut names: Vec<_> = swept.faces.iter().collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), swept.faces.len(), "two faces with one origin");
    let mass = mass_properties(body).unwrap();
    assert!(mass.volume > 0.0, "inside out: {}", mass.volume);
    let found = body_self_intersections(body, &SelfCheckParams::new(tolerance.linear)).unwrap();
    assert!(found.is_clean(), "{found}");
    for fine in [1e-1, 1e-2, 1e-3] {
        let now = (mesh_volume(body, fine * thickness) - mass.volume).abs() / mass.volume;
        assert!(now < 4.0 * fine, "mesh volume off by {now} at {fine}");
    }
    (mass.volume, mass.area, mass.centroid.unwrap())
}

fn sound(swept: &Swept, thickness: f64) -> (f64, f64, Vec3) {
    sound_with(swept, thickness, &tol())
}

#[track_caller]
fn close(found: f64, expected: f64, relative: f64) {
    assert!(
        (found - expected).abs() <= relative * expected.abs(),
        "found {found}, expected {expected} (off by {:e})",
        (found - expected).abs() / expected.abs()
    );
}

fn refusal(result: Result<Swept, KernelError>) -> String {
    match result {
        Ok(_) => panic!("should have been refused"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn a_round_pipe_along_line_bend_line_is_a_cylinder_a_torus_and_a_cylinder() {
    for k in SCALES {
        let (r, length) = (1.0 * k, bent_length(0.0) * k);
        let pipe = sweep(
            &profile(&[circle(1, [0.0, 0.0], r)]),
            &front(),
            &bent(k),
            &Frame::WORLD,
            &tol(),
        )
        .unwrap();
        assert!(!pipe.closed);
        assert_eq!(pipe.deviation, 0.0);
        close(pipe.length, length, 1e-14);
        assert_eq!(kinds(&pipe, Role::Side), ["cylinder", "torus", "cylinder"]);
        assert_eq!(kinds(&pipe, Role::StartCap), ["plane"]);
        assert_eq!(kinds(&pipe, Role::EndCap), ["plane"]);
        // No seam along the pipe: three circles and nothing else.
        assert_eq!(pipe.body.edges.len(), 4);
        assert_eq!(pipe.body.vertices.len(), 4);
        let (volume, area, centre) = sound(&pipe, k);
        close(volume, PI * r * r * length, 1e-12);
        close(area, TAU * r * length + TAU * r * r, 1e-12);
        assert!(centre.z.abs() <= 1e-12 * k, "{centre:?}");
        // Named after the profile's curve and the path's.
        let sides: Vec<_> = pipe
            .faces
            .iter()
            .filter(|o| o.role == Role::Side)
            .map(|o| (o.sources.clone(), o.piece))
            .collect();
        assert_eq!(
            sides,
            [(vec![1, 101], 0), (vec![1, 102], 0), (vec![1, 103], 0)]
        );
    }
}

#[test]
fn a_square_pipe_has_one_face_for_each_side_parallel_to_the_paths_plane() {
    for k in SCALES {
        let square = profile(&scale_curves(&rectangle(1, 1, (-1.0, 1.0), (-1.0, 1.0)), k));
        for plane in [front(), back()] {
            let pipe = sweep(&square, &plane, &bent(k), &Frame::WORLD, &tol()).unwrap();
            let sides = kinds(&pipe, Role::Side);
            // Top and bottom: one plane each along the whole path. The
            // inside and the outside of the bend: plane, cylinder, plane.
            assert_eq!(sides.len(), 8, "{sides:?}");
            assert_eq!(count(&sides, "plane"), 6);
            assert_eq!(count(&sides, "cylinder"), 2);
            let whole: Vec<_> = pipe
                .faces
                .iter()
                .filter(|o| o.role == Role::Side && o.sources.len() == 1)
                .map(|o| o.sources[0])
                .collect();
            // The rectangle's first and third side are parallel to x.
            assert_eq!(whole.len(), 2, "{:?}", pipe.faces);
            assert!(whole.contains(&1) && whole.contains(&3), "{whole:?}");
            let (volume, area, _) = sound(&pipe, k);
            let length = bent_length(0.0) * k;
            close(volume, 4.0 * k * k * length, 1e-12);
            close(area, 8.0 * k * length + 8.0 * k * k, 1e-12);
        }
    }
}

#[test]
fn a_profile_off_the_path_with_a_hole_follows_pappus() {
    // A rectangle 3 by 2 with a round hole, well to the right of the
    // path (the world's +x, which is `a < 0`), seen from the front and
    // from behind.
    for k in SCALES {
        let mut curves = rectangle(1, 1, (2.0, 5.0), (-1.0, 1.0));
        curves.push(circle(9, [3.0, 0.0], 0.5));
        let tube = profile(&scale_curves(&curves, k));
        assert_eq!(tube.holes.len(), 1);
        for plane in [front(), back()] {
            let swept = sweep(&tube, &plane, &bent(k), &Frame::WORLD, &tol()).unwrap();
            let sides = kinds(&swept, Role::Side);
            assert_eq!(count(&sides, "torus"), 1, "{sides:?}");
            assert_eq!(count(&sides, "cylinder"), 4, "{sides:?}");
            let (volume, area, _) = sound(&swept, k);
            // Area times the travel of the centre of area, for the
            // rectangle less the hole; the centre of the rectangle is at
            // a = −3.5, that of the hole at a = −3.
            let (outer, hole) = (6.0, PI * 0.25);
            let expected = outer * bent_length(-3.5) - hole * bent_length(-3.0);
            close(volume, expected * k * k * k, 1e-12);
            let sides = 10.0 * bent_length(-3.5) + PI * bent_length(-3.0);
            close(area, (sides + 2.0 * (outer - hole)) * k * k, 1e-12);
        }
    }
}

/// A closed profile with a spline in it: a base with a bulge on top. Ids
/// 1 (the base line), 2 and 4 (the sides), 3 (the spline).
fn bulge(k: f64) -> Vec<SketchCurve> {
    scale_curves(
        &[
            line(1, [-1.0, -0.5], [1.5, -0.5]),
            line(2, [1.5, -0.5], [1.5, 0.25]),
            spline(
                3,
                3,
                &[
                    [1.5, 0.25],
                    [1.0, 1.25],
                    [0.25, 0.25],
                    [-0.5, 1.0],
                    [-1.0, 0.5],
                ],
            ),
            line(4, [-1.0, 0.5], [-1.0, -0.5]),
        ],
        k,
    )
}

/// Area, centre of area, length and centre of length of a closed loop
/// given as a polygon fine enough to stand for it.
fn polygon_measures(points: &[P]) -> (f64, P, f64, P) {
    let (mut area, mut mx, mut my) = (0.0, 0.0, 0.0);
    let (mut length, mut lx, mut ly) = (0.0, 0.0, 0.0);
    for (i, p) in points.iter().enumerate() {
        let q = points[(i + 1) % points.len()];
        let w = p[0] * q[1] - q[0] * p[1];
        area += w / 2.0;
        mx += (p[0] + q[0]) * w / 6.0;
        my += (p[1] + q[1]) * w / 6.0;
        let l = (q[0] - p[0]).hypot(q[1] - p[1]);
        length += l;
        lx += (p[0] + q[0]) / 2.0 * l;
        ly += (p[1] + q[1]) / 2.0 * l;
    }
    (
        area,
        [mx / area, my / area],
        length,
        [lx / length, ly / length],
    )
}

/// The boundary of `bulge` as a polygon: the spline by de Boor at many
/// parameters (Richardson would do better; 200 000 chords do).
fn bulge_outline(k: f64) -> Vec<P> {
    let control: [P; 5] = [
        [1.5, 0.25],
        [1.0, 1.25],
        [0.25, 0.25],
        [-0.5, 1.0],
        [-1.0, 0.5],
    ];
    let kn = knots(3, 5);
    let eval = |t: f64| -> P {
        // de Boor, degree 3.
        let span = if t >= 1.0 { 4 } else { 3 + (t * 2.0) as usize };
        let mut d: Vec<P> = (0..=3).map(|j| control[span - 3 + j]).collect();
        for r in 1..=3 {
            for j in (r..=3).rev() {
                let i = span - 3 + j;
                let alpha = (t - kn[i]) / (kn[i + 3 - r + 1] - kn[i]);
                d[j] = [
                    (1.0 - alpha) * d[j - 1][0] + alpha * d[j][0],
                    (1.0 - alpha) * d[j - 1][1] + alpha * d[j][1],
                ];
            }
        }
        d[3]
    };
    let mut points = vec![[-1.0, -0.5], [1.5, -0.5]];
    let n = 200_000;
    points.extend((0..=n).map(|i| eval(i as f64 / n as f64)));
    points.iter().map(|p| [p[0] * k, p[1] * k]).collect()
}

#[test]
fn a_profile_with_a_spline_follows_pappus_along_the_bend() {
    for k in SCALES {
        let swept = sweep(
            &profile(&bulge(k)),
            &front(),
            &bent(k),
            &Frame::WORLD,
            &tol(),
        )
        .unwrap();
        let sides = kinds(&swept, Role::Side);
        // The base is one plane; each side a plane, a cylinder, a plane;
        // the spline three spline surfaces, the middle one rational.
        assert_eq!(sides.len(), 10, "{sides:?}");
        assert_eq!(count(&sides, "nurbs surface"), 3);
        assert_eq!(count(&sides, "cylinder"), 2);
        let (volume, area, _) = sound(&swept, k);
        let (a, centre, length, middle) = polygon_measures(&bulge_outline(1.0));
        // On the front plane the sketch's x is the world's x: a = −x.
        close(volume, a * bent_length(-centre[0]) * k * k * k, 1e-9);
        close(
            area,
            (length * bent_length(-middle[0]) + 2.0 * a) * k * k,
            1e-9,
        );
    }
}

/// The same body as `sweep` along `bent`, made of an extrusion, a
/// revolution and an extrusion joined by the boolean. The profile is
/// drawn on `back` (the plane an extrusion along +y starts from).
fn joined(curves: &[SketchCurve], k: f64, tol: &Tolerance) -> Body {
    let shape = [profile(curves)];
    let at = |x: f64, y: f64| Vec3::new(x * k, y * k, 0.0);
    let first = extrude(&shape, &back(), 0.0, 10.0 * k, tol).unwrap().body;
    // About the line through (−5, 10) along the world's z, which in the
    // sketch is at x = −5 and runs along −y.
    let turning = back().at(at(0.0, 10.0)).unwrap();
    let bend = revolve(
        &shape,
        &turning,
        [-5.0 * k, 0.0],
        [0.0, -1.0],
        0.0,
        FRAC_PI_2,
        tol,
    )
    .unwrap()
    .body;
    // After the bend the profile faces −x, and its x is the world's y.
    let last = Frame::new(at(-5.0, 15.0), -Vec3::X, Vec3::Y).unwrap();
    let second = extrude(&shape, &last, 0.0, 12.0 * k, tol).unwrap().body;
    let elbow = boolean(&first, &bend, BooleanOp::Union, tol).unwrap().body;
    boolean(&elbow, &second, BooleanOp::Union, tol)
        .unwrap()
        .body
}

#[test]
fn a_sweep_is_the_union_of_its_pieces_with_no_more_faces() {
    for k in SCALES {
        // The boolean wants a tolerance that rounding allows at the size
        // of the body: 1e-6 of a million is not one.
        let tol = Tolerance::new(1e-6 * k.max(1.0), 1e-9).unwrap();
        // (Not the round pipe: the boolean refuses a cylinder joined to
        // the torus that continues it; `a_round_pipe_is_the_union_of_its_pieces`.)
        let cases: [(&str, Vec<SketchCurve>, usize); 2] = [
            (
                "square",
                scale_curves(&rectangle(1, 1, (-1.0, 1.0), (-1.0, 1.0)), k),
                10,
            ),
            ("bulge", bulge(k), 12),
        ];
        for (name, curves, faces) in cases {
            let swept = sweep(&profile(&curves), &back(), &bent(k), &Frame::WORLD, &tol)
                .unwrap_or_else(|e| panic!("{name} at {k}: {e}"));
            let pieces = joined(&curves, k, &tol);
            let (ours, theirs) = (
                mass_properties(&swept.body).unwrap(),
                mass_properties(&pieces).unwrap(),
            );
            close(ours.volume, theirs.volume, 1e-9);
            close(ours.area, theirs.area, 1e-9);
            let apart = ours.centroid.unwrap().distance(theirs.centroid.unwrap());
            assert!(apart <= 1e-9 * 20.0 * k, "{name} at {k}: centres {apart}");
            // The boolean merges the faces of its operands that lie on
            // one plane, so the union has as few faces as the sweep, not
            // more: the sweep never had the seams.
            assert_eq!(swept.body.faces.len(), faces, "{name} at {k}");
            assert_eq!(pieces.faces.len(), faces, "{name} at {k}: the union");
        }
    }
}

/// Two lines that meet at a corner where the path turns left through
/// `turn`: 10 along +y, then 8 on. Ids 101, 102.
fn cornered(turn: f64, k: f64) -> Vec<SketchCurve> {
    let (s, c) = turn.sin_cos();
    scale_curves(
        &[
            line(101, [0.0, 0.0], [0.0, 10.0]),
            line(102, [0.0, 10.0], [-8.0 * s, 10.0 + 8.0 * c]),
        ],
        k,
    )
}

#[test]
fn a_corner_between_two_lines_is_mitred_on_the_bisecting_plane() {
    for k in SCALES {
        for degrees in [90.0_f64, 60.0, 120.0, -90.0, 30.0] {
            let turn = degrees.to_radians();
            // What a line of the profile at `a` to the left of the path
            // travels: the two legs less what the mitre takes of each.
            let travel = |a: f64| 18.0 - 2.0 * a * (turn / 2.0).tan();

            // A rectangle off the path: 1.5 wide from a = −1 to 0.5 (the
            // world's x from −0.5 to 1), 1 high.
            let block = profile(&scale_curves(&rectangle(1, 1, (-0.5, 1.0), (-0.5, 0.5)), k));
            let swept = sweep(&block, &front(), &cornered(turn, k), &Frame::WORLD, &tol())
                .unwrap_or_else(|e| panic!("{degrees}° at {k}: {e}"));
            let sides = kinds(&swept, Role::Side);
            // Top and bottom once, the two sides once per leg.
            assert_eq!(sides, ["plane"; 6], "{degrees}°");
            let (volume, area, _) = sound(&swept, k);
            close(volume, 1.5 * travel(-0.25) * k * k * k, 1e-12);
            // The sides: each line of the boundary times what its middle
            // travels; then the two ends.
            let lateral = 1.5 * 2.0 * travel(-0.25) + travel(-1.0) + travel(0.5);
            close(area, (lateral + 3.0) * k * k, 1e-12);

            // A cylinder cut by the slanted plane: its volume is its
            // cross-section times its length along the axis.
            let rod = profile(&[circle(1, [0.25 * k, 0.0], 0.5 * k)]);
            let swept = sweep(&rod, &front(), &cornered(turn, k), &Frame::WORLD, &tol())
                .unwrap_or_else(|e| panic!("{degrees}° at {k}: {e}"));
            assert_eq!(kinds(&swept, Role::Side), ["cylinder", "cylinder"]);
            let joint = swept
                .body
                .edges
                .iter()
                .filter(|e| e.curve.kind() == "ellipse")
                .count();
            assert_eq!(joint, 1, "the two cylinders meet in one ellipse");
            let (volume, area, _) = sound(&swept, k);
            close(volume, PI * 0.25 * travel(-0.25) * k * k * k, 1e-12);
            close(area, (PI * travel(-0.25) + 2.0 * PI * 0.25) * k * k, 1e-12);
        }
    }
}

#[test]
fn a_spline_profile_goes_round_a_mitre() {
    let turn = 70.0_f64.to_radians();
    let swept = sweep(
        &profile(&bulge(1.0)),
        &front(),
        &cornered(turn, 1.0),
        &Frame::WORLD,
        &tol(),
    )
    .unwrap();
    let (volume, area, _) = sound(&swept, 1.0);
    let travel = |a: f64| 18.0 - 2.0 * a * (turn / 2.0).tan();
    let (a, centre, length, middle) = polygon_measures(&bulge_outline(1.0));
    close(volume, a * travel(-centre[0]), 1e-9);
    close(area, length * travel(-middle[0]) + 2.0 * a, 1e-9);
}

/// A rectangle 20 by 12 with corners rounded at radius 3, walked
/// counter-clockwise from the middle of its bottom side. Ids 101 to 108.
/// The profile's plane (`front`) crosses its bottom side at the origin…
/// which runs along x: so this one is drawn turned, its "bottom" side
/// along +y through the origin.
fn rounded_rectangle(k: f64) -> Vec<SketchCurve> {
    // In a frame where the first side runs along +x from (−7, 0) to
    // (7, 0); then turned a quarter turn so that it runs along +y.
    let turn = |p: P| [-p[1], p[0]];
    let straight = |id, a: P, b: P| line(id, turn(a), turn(b));
    let round = |id, c: P, a: P, b: P| arc(id, turn(c), turn(a), turn(b));
    scale_curves(
        &[
            straight(101, [-7.0, 0.0], [7.0, 0.0]),
            round(102, [7.0, 3.0], [7.0, 0.0], [10.0, 3.0]),
            straight(103, [10.0, 3.0], [10.0, 9.0]),
            round(104, [7.0, 9.0], [10.0, 9.0], [7.0, 12.0]),
            straight(105, [7.0, 12.0], [-7.0, 12.0]),
            round(106, [-7.0, 9.0], [-7.0, 12.0], [-10.0, 9.0]),
            straight(107, [-10.0, 9.0], [-10.0, 3.0]),
            round(108, [-7.0, 3.0], [-10.0, 3.0], [-7.0, 0.0]),
        ],
        k,
    )
}

#[test]
fn a_gasket_round_a_rounded_rectangle_is_a_ring() {
    for k in SCALES {
        let r = 0.8 * k;
        let gasket = sweep(
            &profile(&[circle(1, [0.0, 0.0], r)]),
            &front(),
            &rounded_rectangle(k),
            &Frame::WORLD,
            &tol(),
        )
        .unwrap();
        assert!(gasket.closed);
        let sides = kinds(&gasket, Role::Side);
        assert_eq!(sides.len(), gasket.body.faces.len(), "no caps");
        assert_eq!(count(&sides, "cylinder"), 4);
        assert_eq!(count(&sides, "torus"), 4);
        assert_eq!(genus(&gasket.body), Some(1));
        let (volume, area, _) = sound(&gasket, k);
        let length = (2.0 * 14.0 + 2.0 * 6.0 + TAU * 3.0) * k;
        close(gasket.length, length, 1e-14);
        close(volume, PI * r * r * length, 1e-12);
        close(area, TAU * r * length, 1e-12);
    }
}

#[test]
fn a_frame_with_four_mitres_and_a_ring_round_a_circle() {
    for k in SCALES {
        // A picture frame: a rectangle 16 by 10 walked counter-clockwise
        // (the inside is to the left), a moulding 2 wide and 1 high whose
        // outer edge is on the path.
        let path = scale_curves(
            &polygon(
                101,
                1,
                &[[0.0, -5.0], [0.0, 5.0], [-16.0, 5.0], [-16.0, -5.0]],
            ),
            k,
        );
        let moulding = profile(&scale_curves(&rectangle(1, 1, (-2.0, 0.0), (0.0, 1.0)), k));
        let frame = sweep(&moulding, &front(), &path, &Frame::WORLD, &tol()).unwrap();
        assert!(frame.closed);
        let sides = kinds(&frame, Role::Side);
        // Top and bottom once (each with a hole), inside and outside once
        // per leg.
        assert_eq!(sides, ["plane"; 10]);
        let rings = frame
            .body
            .faces
            .iter()
            .filter(|f| f.loops.len() == 2)
            .count();
        assert_eq!(rings, 2);
        assert_eq!(genus(&frame.body), Some(1));
        let (volume, area, _) = sound(&frame, k);
        // 16 by 10 outside, 12 by 6 inside, 1 high.
        close(volume, (160.0 - 72.0) * k * k * k, 1e-12);
        close(area, (2.0 * 88.0 + 52.0 + 36.0) * k * k, 1e-12);

        // A square bar round a circle of radius 6: Pappus.
        let ring = sweep(
            &profile(&scale_curves(&rectangle(1, 1, (-1.0, 1.0), (-1.0, 1.0)), k)),
            &front(),
            &[circle(101, [-6.0 * k, 0.0], 6.0 * k)],
            &Frame::WORLD,
            &tol(),
        )
        .unwrap();
        assert!(ring.closed);
        let sides = kinds(&ring, Role::Side);
        // Top and bottom once; inside and outside in two halves.
        assert_eq!(count(&sides, "plane"), 2, "{sides:?}");
        assert_eq!(count(&sides, "cylinder"), 4, "{sides:?}");
        assert_eq!(genus(&ring.body), Some(1));
        let (volume, area, _) = sound(&ring, k);
        close(volume, 4.0 * TAU * 6.0 * k * k * k, 1e-12);
        close(area, 8.0 * TAU * 6.0 * k * k, 1e-12);
        let halves: Vec<_> = ring
            .faces
            .iter()
            .filter(|o| o.sources == [2, 101])
            .map(|o| o.piece)
            .collect();
        assert_eq!(halves, [0, 1]);
    }
}

#[test]
fn a_hollow_profile_round_a_closed_path_has_a_cavity_that_goes_round() {
    let tube = profile(&[circle(1, [0.0, 0.0], 1.0), circle(2, [0.0, 0.0], 0.6)]);
    let ring = sweep(
        &tube,
        &front(),
        &rounded_rectangle(1.0),
        &Frame::WORLD,
        &tol(),
    )
    .unwrap();
    assert_eq!(ring.body.shells.len(), 2);
    let (volume, _, _) = sound(&ring, 1.0);
    let length = 2.0 * 14.0 + 2.0 * 6.0 + TAU * 3.0;
    close(volume, PI * (1.0 - 0.36) * length, 1e-12);
}

#[test]
fn the_path_may_be_drawn_in_any_order_and_direction() {
    // The bend drawn backwards and listed out of order, the second line
    // from its far end: the same body.
    let straight = sweep(
        &profile(&rectangle(1, 1, (-1.0, 1.0), (-1.0, 1.0))),
        &front(),
        &bent(1.0),
        &Frame::WORLD,
        &tol(),
    )
    .unwrap();
    let shuffled = [
        line(103, [-17.0, 15.0], [-5.0, 15.0]),
        line(101, [0.0, 10.0], [0.0, 0.0]),
        arc(102, [-5.0, 10.0], [0.0, 10.0], [-5.0, 15.0]),
    ];
    let again = sweep(
        &profile(&rectangle(1, 1, (-1.0, 1.0), (-1.0, 1.0))),
        &front(),
        &shuffled,
        &Frame::WORLD,
        &tol(),
    )
    .unwrap();
    let (a, b) = (
        mass_properties(&straight.body).unwrap(),
        mass_properties(&again.body).unwrap(),
    );
    close(a.volume, b.volume, 1e-13);
    close(a.area, b.area, 1e-13);
    assert_eq!(straight.faces, again.faces);
}

#[test]
fn what_cannot_be_swept_is_refused_with_the_place() {
    let square = profile(&rectangle(1, 1, (-1.0, 1.0), (-1.0, 1.0)));
    let go = |path: &[SketchCurve]| sweep(&square, &front(), path, &Frame::WORLD, &tol());

    // A mitre too sharp: the second leg is shorter than the mitres at its
    // two ends take on the inside.
    let zigzag = [
        line(101, [0.0, 0.0], [0.0, 10.0]),
        line(102, [0.0, 10.0], [-1.0, 10.0]),
        line(103, [-1.0, 10.0], [-1.0, 0.0]),
    ];
    let why = refusal(go(&zigzag));
    assert!(why.contains("too sharp") && why.contains("e102"), "{why}");
    assert!(matches!(
        go(&zigzag),
        Err(KernelError::Sweep { curves, .. }) if curves.contains(&102)
    ));

    // A bend tighter than the profile is wide on the inside.
    let tight = [
        line(101, [0.0, 0.0], [0.0, 10.0]),
        arc(102, [-0.75, 10.0], [0.0, 10.0], [-0.75, 10.75]),
    ];
    let why = refusal(go(&tight));
    assert!(why.contains("too tight") && why.contains("e102"), "{why}");
    // The same bend the other way round has the profile's other side on
    // its inside: just as tight.
    let tight_right = [
        line(101, [0.0, 0.0], [0.0, 10.0]),
        arc(102, [0.75, 10.0], [0.75, 10.75], [0.0, 10.0]),
    ];
    let why = refusal(go(&tight_right));
    assert!(why.contains("too tight") && why.contains("e102"), "{why}");
    // With room to spare on the inside it is swept.
    let wide = [
        line(101, [0.0, 0.0], [0.0, 10.0]),
        arc(102, [-1.5, 10.0], [0.0, 10.0], [-1.5, 11.5]),
    ];
    sound(&go(&wide).unwrap(), 1.0);

    // A path that does not start near the profile.
    let away = [line(101, [0.0, 3.0], [0.0, 10.0])];
    let why = refusal(go(&away));
    assert!(why.contains("does not start on the plane"), "{why}");
    // One that leaves it at a slant.
    let slanted = [line(101, [0.0, 0.0], [1.0, 10.0])];
    let why = refusal(go(&slanted));
    assert!(why.contains("right angle"), "{why}");

    // A branch, and a path in two parts.
    let branch = [
        line(101, [0.0, 0.0], [0.0, 10.0]),
        line(102, [0.0, 10.0], [-5.0, 15.0]),
        line(103, [0.0, 10.0], [5.0, 15.0]),
    ];
    let why = refusal(go(&branch));
    assert!(why.contains("branches") && why.contains("e103"), "{why}");
    let apart = [
        line(101, [0.0, 0.0], [0.0, 10.0]),
        line(102, [3.0, 12.0], [5.0, 15.0]),
    ];
    let why = refusal(go(&apart));
    assert!(why.contains("more than one part"), "{why}");

    // A corner at an arc is not a mitre.
    let kinked = [
        line(101, [0.0, 0.0], [0.0, 10.0]),
        arc(
            102,
            [-5.0, 12.0],
            [0.0, 10.0],
            [-5.0, 12.0 + 29.0_f64.sqrt()],
        ),
    ];
    let why = refusal(go(&kinked));
    assert!(why.contains("corner") && why.contains("e102"), "{why}");

    // A path that turns straight back.
    let back = [
        line(101, [0.0, 0.0], [0.0, 10.0]),
        line(102, [0.0, 10.0], [0.0, 4.0]),
    ];
    let why = refusal(go(&back));
    assert!(why.contains("turns back"), "{why}");

    // Nothing to follow, and a tolerance that is none.
    assert!(go(&[]).is_err());
    let bad = Tolerance {
        linear: 0.0,
        angular: 1e-9,
    };
    assert!(sweep(&square, &front(), &bent(1.0), &Frame::WORLD, &bad).is_err());
    assert!(
        sweep(
            &square,
            &front(),
            &[line(101, [0.0, 0.0], [0.0, f64::NAN])],
            &Frame::WORLD,
            &tol()
        )
        .is_err()
    );
}

#[test]
fn a_sweep_that_comes_back_through_itself_is_valid_in_form_and_found_by_the_self_check() {
    // A path that crosses its own start: a loop of three quarter turns
    // and a line back across the first leg.
    let path = [
        line(101, [0.0, 0.0], [0.0, 10.0]),
        arc(102, [-3.0, 10.0], [0.0, 10.0], [-3.0, 13.0]),
        arc(103, [-3.0, 10.0], [-3.0, 13.0], [-6.0, 10.0]),
        arc(104, [-3.0, 10.0], [-6.0, 10.0], [-3.0, 7.0]),
        line(105, [-3.0, 7.0], [6.0, 7.0]),
    ];
    let swept = sweep(
        &profile(&[circle(1, [0.0, 0.0], 1.0)]),
        &front(),
        &path,
        &Frame::WORLD,
        &tol(),
    )
    .unwrap();
    assert_eq!(check(&swept.body, &tol()), vec![]);
    let found = body_self_intersections(&swept.body, &SelfCheckParams::new(tol().linear)).unwrap();
    assert!(!found.is_clean());
    assert!(found.crossings.iter().any(|c| c.certain), "{found}");
}

#[test]
fn a_sweep_is_the_same_every_time() {
    let make = || {
        sweep(
            &profile(&bulge(1.0)),
            &front(),
            &rounded_rectangle(1.0),
            &Frame::WORLD,
            &tol(),
        )
        .unwrap()
    };
    assert_eq!(make(), make());
}

#[test]
fn faces_on_one_surface_kind_are_what_the_table_says() {
    // A slanted line of the profile round a bend is a cone; an arc whose
    // circle would reach the axis a rational surface, not a torus.
    let wedge = profile(&polygon(1, 1, &[[-1.0, -1.0], [1.0, -1.0], [0.0, 1.0]]));
    let swept = sweep(&wedge, &front(), &bent(1.0), &Frame::WORLD, &tol()).unwrap();
    let sides = kinds(&swept, Role::Side);
    assert_eq!(count(&sides, "cone"), 2, "{sides:?}");
    let (volume, _, _) = sound(&swept, 1.0);
    // The centre of the triangle is on the path's plane at a = 0.
    close(volume, 2.0 * bent_length(0.0), 1e-12);

    // A sliver between a line and a short arc of a big circle whose
    // centre is far to the left, beyond the axis of the bend: its torus
    // would be a spindle.
    let lens = profile(&[
        line(1, [1.0, 1.0], [1.0, -1.0]),
        arc(2, [-9.0, 0.0], [1.0, -1.0], [1.0, 1.0]),
    ]);
    let swept = sweep(&lens, &front(), &bent(1.0), &Frame::WORLD, &tol()).unwrap();
    let sides = kinds(&swept, Role::Side);
    assert_eq!(count(&sides, "nurbs surface"), 1, "{sides:?}");
    assert_eq!(count(&sides, "torus"), 0, "{sides:?}");
    assert!(
        swept.body.faces.iter().any(
            |f| matches!(&f.surface, Surface::Nurbs(s) if s.weights().iter().any(|w| *w != 1.0))
        )
    );
    // The sliver is 0.05 thick.
    let (volume, _, _) = sound(&swept, 0.05);
    // A segment of a circle of radius √101 with half-angle asin(1/√101):
    // its area, and its centre measured from the circle's.
    let radius = 101.0_f64.sqrt();
    let half = (1.0 / radius).asin();
    let area = 101.0 * half - 10.0;
    let centre =
        -9.0 + 4.0 * radius * half.sin().powi(3) / (3.0 * (2.0 * half - (2.0 * half).sin()));
    close(volume, area * bent_length(-centre), 1e-9);
}

/// The mesh of the sliver of `faces_on_one_surface_kind_are_what_the_table_says`
/// at a chord of a quarter of a unit, five times the thickness of the
/// sliver, is closed: it was left open before a chord of a curved edge that
/// equals another edge's chord was cut in two (the tessellator, 2026-10-14).
#[test]
fn the_mesh_of_a_sliver_is_closed_at_a_coarse_chord() {
    let lens = profile(&[
        line(1, [1.0, 1.0], [1.0, -1.0]),
        arc(2, [-9.0, 0.0], [1.0, -1.0], [1.0, 1.0]),
    ]);
    let swept = sweep(&lens, &front(), &bent(1.0), &Frame::WORLD, &tol()).unwrap();
    mesh_volume(&swept.body, 0.25);
}

/// The union of an extruded cylinder and the quarter torus that carries
/// it on: the two faces are tangent along the circle they share, and the
/// boolean refuses ("surfaces that touch along a curve ... without a
/// closed form"). It is the oracle a round pipe would have had.
#[test]
#[ignore = "the boolean refuses a cylinder joined to the torus that continues it tangentially"]
fn a_round_pipe_is_the_union_of_its_pieces() {
    let curves = [circle(1, [0.0, 0.0], 1.0)];
    let pieces = joined(&curves, 1.0, &tol());
    let swept = sweep(
        &profile(&curves),
        &back(),
        &bent(1.0),
        &Frame::WORLD,
        &tol(),
    )
    .unwrap();
    let (ours, theirs) = (
        mass_properties(&swept.body).unwrap(),
        mass_properties(&pieces).unwrap(),
    );
    close(ours.volume, theirs.volume, 1e-9);
}

/// A B-spline of the test's own: de Boor.
fn de_boor(degree: usize, knots: &[f64], control: &[P], t: f64) -> P {
    let n = control.len();
    let mut span = degree;
    while span + 1 < n && t >= knots[span + 1] {
        span += 1;
    }
    let mut d: Vec<P> = (0..=degree).map(|j| control[span - degree + j]).collect();
    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = span - degree + j;
            let alpha = (t - knots[i]) / (knots[i + degree - r + 1] - knots[i]);
            d[j] = [
                (1.0 - alpha) * d[j - 1][0] + alpha * d[j][0],
                (1.0 - alpha) * d[j - 1][1] + alpha * d[j][1],
            ];
        }
    }
    d[degree]
}

/// An S through six control points that leaves the origin along +y: a
/// cubic spline of three spans. Id 101.
const WAVE: [P; 6] = [
    [0.0, 0.0],
    [0.0, 4.0],
    [-3.0, 7.0],
    [-3.0, 11.0],
    [1.0, 14.0],
    [1.0, 18.0],
];

fn wave(k: f64) -> Vec<SketchCurve> {
    scale_curves(&[spline(101, 3, &WAVE)], k)
}

/// `WAVE` as a polygon of many chords, in the world's xy plane.
fn wave_polygon(k: f64, chords: usize) -> Vec<Vec3> {
    let kn = knots(3, WAVE.len());
    (0..=chords)
        .map(|i| {
            let p = de_boor(3, &kn, &WAVE, i as f64 / chords as f64);
            Vec3::new(p[0] * k, p[1] * k, 0.0)
        })
        .collect()
}

fn polygon_length(points: &[Vec3]) -> f64 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// The distance from `p` to the polygon.
fn polygon_distance(points: &[Vec3], p: Vec3) -> f64 {
    points
        .windows(2)
        .map(|w| {
            let d = w[1] - w[0];
            let t = ((p - w[0]).dot(d) / d.length_squared()).clamp(0.0, 1.0);
            p.distance(w[0] + d * t)
        })
        .fold(f64::INFINITY, f64::min)
}

/// A tolerance that suits a body of size `k`: the default up to a unit,
/// and as fine in proportion above it. Along a spline the sweep is an
/// approximation within the tolerance, and one part in 10¹² is not to be
/// had.
fn tolerance_at(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k.max(1.0), 1e-9).unwrap()
}

#[test]
fn a_pipe_along_a_spline_lies_within_the_tolerance_of_the_true_sweep() {
    for k in SCALES {
        let tolerance = tolerance_at(k);
        let r = 0.5 * k;
        let pipe = sweep(
            &profile(&[circle(1, [0.0, 0.0], r)]),
            &front(),
            &wave(k),
            &Frame::WORLD,
            &tolerance,
        )
        .unwrap();
        // What it says of itself.
        assert!(
            pipe.deviation > 0.0 && pipe.deviation <= tolerance.linear / 2.0,
            "{}",
            pipe.deviation
        );
        assert!(pipe.stations >= 6);
        let centre_line = wave_polygon(k, 20_000);
        let length = polygon_length(&wave_polygon(k, 100_000));
        close(pipe.length, length, 1e-8);
        // The circle in four quarters, each one face along the spline.
        let sides = kinds(&pipe, Role::Side);
        assert_eq!(sides, ["nurbs surface"; 4]);

        // Measured here: every point of a side is at the pipe's radius
        // from the centre line.
        let mut worst: f64 = 0.0;
        for face in pipe.body.faces.iter().take(4) {
            let ((u0, u1), (v0, v1)) = face.surface.domain();
            for i in 0..=6 {
                for j in 0..=24 {
                    let (u, v) = (
                        u0 + (u1 - u0) * i as f64 / 6.0,
                        v0 + (v1 - v0) * j as f64 / 24.0,
                    );
                    let p = face.surface.eval(u, v);
                    worst = worst.max((polygon_distance(&centre_line, p) - r).abs());
                }
            }
        }
        // (The polygon itself is within 1e-7 of a unit of the spline.)
        assert!(
            worst <= tolerance.linear / 2.0 + 1e-7 * k,
            "{worst} at scale {k}; the sweep says {}",
            pipe.deviation
        );

        // A profile centred on a path in a plane sweeps its area times
        // the length of the path, whatever the path. The surfaces are
        // within `deviation` of the true ones, so the volume is within
        // the area of the sides times that: 2 π r · length · deviation.
        let (volume, area, _) = sound_with(&pipe, k, &tolerance);
        let exact = PI * r * r * length;
        let bound = TAU * r * length * pipe.deviation + 1e-8 * exact;
        assert!(
            (volume - exact).abs() <= bound,
            "volume {volume}, exact {exact}, allowed {bound}"
        );
        // The area changes with the surface's distance from the true one
        // by its curvature, which is at most that of the profile: 1/r.
        close(
            area,
            TAU * r * length + 2.0 * PI * r * r,
            2.0 * pipe.deviation / r + 1e-7,
        );
    }
}

#[test]
fn a_square_bar_along_a_spline_keeps_its_flat_sides_flat() {
    let bar = profile(&rectangle(1, 1, (-0.6, 0.6), (-0.4, 0.4)));
    let swept = sweep(&bar, &front(), &wave(1.0), &Frame::WORLD, &tol()).unwrap();
    let sides = kinds(&swept, Role::Side);
    // Top and bottom are planes, exactly; the two walls follow the path.
    assert_eq!(count(&sides, "plane"), 2, "{sides:?}");
    assert_eq!(count(&sides, "nurbs surface"), 2, "{sides:?}");
    let (volume, area, _) = sound(&swept, 1.0);
    let length = polygon_length(&wave_polygon(1.0, 100_000));
    let exact = 1.2 * 0.8 * length;
    assert!(
        (volume - exact).abs() <= 4.0 * length * swept.deviation + 1e-8 * exact,
        "volume {volume}, exact {exact}"
    );
    // The walls are offsets of the path, as long as it together (what one
    // gains in a bend the other loses); top and bottom are 1.2 wide.
    close(
        area,
        2.0 * 1.2 * length + 2.0 * 0.8 * length + 2.0 * 0.96,
        1e-6,
    );
}

#[test]
fn lines_and_a_spline_in_one_path_share_the_profile_where_they_meet() {
    // 4 along +y, the wave moved up by 4, and 3 on along +y.
    let moved: Vec<P> = WAVE.iter().map(|p| [p[0], p[1] + 4.0]).collect();
    let path = [
        line(101, [0.0, 0.0], [0.0, 4.0]),
        spline(102, 3, &moved),
        line(103, [1.0, 22.0], [1.0, 25.0]),
    ];
    let swept = sweep(
        &profile(&bulge(0.5)),
        &front(),
        &path,
        &Frame::WORLD,
        &tol(),
    )
    .unwrap();
    assert!(swept.deviation > 0.0 && swept.deviation <= 5e-7);
    // The base is one plane along all three; the sides and the spline
    // are one face per piece of the path.
    assert_eq!(swept.body.faces.len(), 1 + 3 * 3 + 2);
    let (volume, _, _) = sound(&swept, 0.5);
    // Not centred on the path: a profile whose centre is `a` to the left
    // sweeps its area times (length − a × the angle the path turns
    // through in all), and this path ends in the direction it starts in.
    let (area, _, boundary, _) = polygon_measures(&bulge_outline(0.5));
    let length = 7.0 + polygon_length(&wave_polygon(1.0, 100_000));
    let exact = area * length;
    assert!(
        (volume - exact).abs() <= boundary * length * swept.deviation + 1e-8 * exact,
        "volume {volume}, exact {exact}"
    );
}

#[test]
fn a_spline_that_bends_tighter_than_the_profile_reaches_is_refused_with_the_place() {
    // A hairpin: the radius of curvature at its top is well under 1.
    let hairpin = [spline(
        101,
        3,
        &[[0.0, 0.0], [0.0, 6.0], [-2.0, 6.0], [-2.0, 0.0]],
    )];
    let wide = profile(&rectangle(1, 1, (-1.0, 1.0), (-0.2, 0.2)));
    let why = refusal(sweep(&wide, &front(), &hairpin, &Frame::WORLD, &tol()));
    assert!(
        why.contains("bends too tightly") && why.contains("e101") && why.contains("radius"),
        "{why}"
    );
    // A wire thin enough goes round it.
    let wire = profile(&[circle(1, [0.0, 0.0], 0.1)]);
    let swept = sweep(&wire, &front(), &hairpin, &Frame::WORLD, &tol()).unwrap();
    sound(&swept, 0.1);
    // A corner between a line and a spline is refused like one at an arc.
    let kinked = [
        line(101, [0.0, 0.0], [0.0, 4.0]),
        spline(102, 3, &[[0.0, 4.0], [2.0, 6.0], [2.0, 9.0], [0.0, 11.0]]),
    ];
    let why = refusal(sweep(&wire, &front(), &kinked, &Frame::WORLD, &tol()));
    assert!(why.contains("corner") && why.contains("e102"), "{why}");
    // A tolerance no row of stations reaches.
    let fine = Tolerance::new(1e-15, 1e-9).unwrap();
    let why = refusal(sweep(&wire, &front(), &wave(1.0), &Frame::WORLD, &fine));
    assert!(
        why.contains("cannot be brought within the tolerance"),
        "{why}"
    );
}

#[test]
fn a_closed_spline_is_a_path_and_a_polyline_a_row_of_mitres() {
    // A rounded square as one closed spline of degree 2 (its control
    // polygon a square, doubled at the middles so that it is smooth): it
    // passes through the origin along +y.
    let ring = [spline(
        101,
        2,
        &[
            [0.0, 0.0],
            [0.0, 5.0],
            [-5.0, 5.0],
            [-10.0, 5.0],
            [-10.0, 0.0],
            [-10.0, -5.0],
            [-5.0, -5.0],
            [0.0, -5.0],
            [0.0, 0.0],
        ],
    )];
    let swept = sweep(
        &profile(&[circle(1, [0.0, 0.0], 0.5)]),
        &front(),
        &ring,
        &Frame::WORLD,
        &tol(),
    )
    .unwrap();
    assert!(swept.closed);
    assert_eq!(genus(&swept.body), Some(1));
    let (volume, _, _) = sound(&swept, 0.5);
    close(volume, PI * 0.25 * swept.length, 1e-5);

    // A polyline (a spline of degree 1) is its lines, mitred.
    let zigzag = [spline(
        101,
        1,
        &[[0.0, 0.0], [0.0, 6.0], [-4.0, 9.0], [-4.0, 15.0]],
    )];
    let swept = sweep(
        &profile(&rectangle(1, 1, (-0.5, 0.5), (-0.5, 0.5))),
        &front(),
        &zigzag,
        &Frame::WORLD,
        &tol(),
    )
    .unwrap();
    assert_eq!(swept.deviation, 0.0);
    assert_eq!(kinds(&swept, Role::Side), ["plane"; 8]);
    let (volume, _, _) = sound(&swept, 1.0);
    close(volume, 6.0 + 5.0 + 6.0, 1e-12);
    // The three faces one side of the profile makes along the one curve
    // of the path are told apart by their number.
    let pieces: Vec<_> = swept
        .faces
        .iter()
        .filter(|o| o.sources == [2, 101])
        .map(|o| o.piece)
        .collect();
    assert_eq!(pieces, [0, 1, 2]);
}
