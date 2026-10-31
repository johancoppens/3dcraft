//! Booleans that cut faces on rational NURBS surfaces that close on
//! themselves: a cylinder's wall, a ball, a hemisphere's dome and a torus,
//! each one face on a surface whose control net ends on the row it starts
//! with (around the axis; for the torus around the tube too), as a STEP
//! file brings them, with and without a seam edge, the seam at different
//! places and the turn in either parameter.
//!
//! The oracle is the analytic twin (as in `boolean_rational.rs`): the
//! same solid with its curved face on the cylinder, sphere or torus, which
//! the kernel cuts by closed forms and the corpus has measured against
//! FreeCAD. Every boolean of a twin must give what the same boolean of the
//! analytic solid gives, in volume, area, centroid and shells; besides,
//! the volumes of a twin's own results must satisfy the identities of
//! sets, and `(A − B) ∪ (A ∩ B)` is built and must be A again. At 1e-3, 1
//! and 1e6, in the world's frame and tilted. The tools are placed so that
//! the curves they cut cross the seam (or run through a pole, or around
//! the axis), which is what the boolean could not follow before
//! 2026-10-14 (`boolean::chart`, `Layout`).

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod boolean_common;
mod rational_common;

use boolean_common::*;
use craft3d_geom::{Circle, Curve3, Line, NurbsSurface, Plane, Surface};
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::boolean::boolean;
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::parallel::map_parallel;
use craft3d_topo::{Body, BodyKind, Coedge, check};
use rational_common::{Flat, arc, straight};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::time::Instant;

/// The closed solids, by name.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    /// A cylinder of radius 2 and height 3 standing on the origin.
    Can,
    /// A ball of radius 2 about the origin: poles at both ends.
    Ball,
    /// The upper half of that ball: a pole at one end, a floor.
    Cup,
    /// A torus about the z axis, radii 2 and 0.6: closed both ways.
    Ring,
}

/// Where the seam is, and how the surface is laid out.
#[derive(Clone, Copy, Debug)]
struct Variant {
    /// The angle about the axis where the turn starts.
    from: f64,
    /// For the torus, the angle around the tube where its turn starts.
    tube_from: f64,
    /// The turn about the axis is the surface's second parameter.
    around_v: bool,
    /// The wall of the can is bounded by a seam edge as well.
    seam: bool,
    tilted: bool,
}

/// The profile, times `k`: (distance from the axis, height).
fn profile(shape: Shape, k: f64, tube_from: f64) -> Flat {
    let mut flat = match shape {
        Shape::Can => straight((2.0, 0.0), (2.0, 3.0)),
        Shape::Ball => arc(-FRAC_PI_2, FRAC_PI_2, 0.0, 1.0),
        Shape::Cup => arc(0.0, FRAC_PI_2, 0.0, 1.0),
        Shape::Ring => arc(tube_from, tube_from + TAU, 0.0, 1.0),
    };
    for point in &mut flat.points {
        *point = match shape {
            Shape::Ring => (2.0 + 0.6 * point.0, 0.6 * point.1),
            Shape::Ball | Shape::Cup => (2.0 * point.0, 2.0 * point.1),
            Shape::Can => *point,
        };
        *point = (point.0 * k, point.1 * k);
    }
    flat
}

/// The surface swept by `profile` a full turn about the z axis from the
/// angle `from`: the standard rational quadratic arcs (`rational_common`),
/// four quarters, so that the net ends on the row it starts with. The
/// turn is u over `[0, 1]`, or v when `around_v`.
fn turned(profile: &Flat, from: f64, around_v: bool) -> NurbsSurface {
    let around = arc(from, from + TAU, 0.0, 1.0);
    let (rows, across) = (around.points.len(), profile.points.len());
    let point = |i: usize, j: usize| {
        let (x, y) = around.points[i];
        let (reach, height) = profile.points[j];
        (
            Vec3::new(reach * x, reach * y, height),
            around.weights[i] * profile.weights[j],
        )
    };
    let (mut ctrl, mut weights) = (Vec::new(), Vec::new());
    if around_v {
        for j in 0..across {
            for i in 0..rows {
                let (p, w) = point(i, j);
                ctrl.push(p);
                weights.push(w);
            }
        }
        NurbsSurface::new(
            profile.degree,
            2,
            profile.knots.clone(),
            around.knots,
            across,
            rows,
            ctrl,
            weights,
        )
        .unwrap()
    } else {
        for i in 0..rows {
            for j in 0..across {
                let (p, w) = point(i, j);
                ctrl.push(p);
                weights.push(w);
            }
        }
        NurbsSurface::new(
            2,
            profile.degree,
            around.knots,
            profile.knots.clone(),
            rows,
            across,
            ctrl,
            weights,
        )
        .unwrap()
    }
}

/// A full circle about the z axis at `height`, starting at `from`.
fn rim(body: &mut Body, reach: f64, height: f64, from: f64) -> craft3d_topo::EdgeId {
    let (sin, cos) = from.psin_cos();
    let vertex = body.add_vertex(Vec3::new(reach * cos, reach * sin, height));
    let frame = Frame::new(
        Vec3::new(0.0, 0.0, height),
        Vec3::Z,
        Vec3::new(cos, sin, 0.0),
    )
    .unwrap();
    body.add_edge(
        Curve3::Circle(Circle::new(frame, reach).unwrap()),
        (0.0, TAU),
        vertex,
        vertex,
    )
}

fn use_of(edge: craft3d_topo::EdgeId, reversed: bool) -> Coedge {
    Coedge { edge, reversed }
}

/// The twin with its curved face on the closed rational surface.
fn twin(shape: Shape, k: f64, v: Variant) -> Body {
    let surface = Surface::Nurbs(turned(&profile(shape, k, v.tube_from), v.from, v.around_v));
    // `∂u × ∂v` points out when u goes around and v up the profile.
    let reversed = v.around_v;
    let mut body = Body::new(BodyKind::Solid);
    match shape {
        Shape::Ball | Shape::Ring => {
            let face = body.add_face(surface, reversed, Vec::new());
            body.add_shell(vec![face]);
        }
        Shape::Can => {
            let lower = rim(&mut body, 2.0 * k, 0.0, v.from);
            let upper = rim(&mut body, 2.0 * k, 3.0 * k, v.from);
            let loops = if v.seam {
                let (start, end) = (body.edges[0].start, body.edges[1].start);
                let (a, b) = (
                    body.vertices[start.0 as usize].point,
                    body.vertices[end.0 as usize].point,
                );
                let line = Curve3::Line(Line::through(a, b).unwrap());
                let seam = body.add_edge(line, (0.0, a.distance(b)), start, end);
                vec![body.add_loop(vec![
                    use_of(lower, false),
                    use_of(seam, false),
                    use_of(upper, true),
                    use_of(seam, true),
                ])]
            } else {
                vec![
                    body.add_loop(vec![use_of(lower, false)]),
                    body.add_loop(vec![use_of(upper, true)]),
                ]
            };
            let wall = body.add_face(surface, reversed, loops);
            let floor_loop = body.add_loop(vec![use_of(lower, true)]);
            let floor = Frame::new(Vec3::ZERO, -Vec3::Z, Vec3::X).unwrap();
            let floor = body.add_face(Surface::Plane(Plane::new(floor)), false, vec![floor_loop]);
            let lid_loop = body.add_loop(vec![use_of(upper, false)]);
            let lid = Frame::new(Vec3::new(0.0, 0.0, 3.0 * k), Vec3::Z, Vec3::X).unwrap();
            let lid = body.add_face(Surface::Plane(Plane::new(lid)), false, vec![lid_loop]);
            body.add_shell(vec![wall, floor, lid]);
        }
        Shape::Cup => {
            let lower = rim(&mut body, 2.0 * k, 0.0, v.from);
            let dome_loop = body.add_loop(vec![use_of(lower, false)]);
            let dome = body.add_face(surface, reversed, vec![dome_loop]);
            let floor_loop = body.add_loop(vec![use_of(lower, true)]);
            let floor = Frame::new(Vec3::ZERO, -Vec3::Z, Vec3::X).unwrap();
            let floor = body.add_face(Surface::Plane(Plane::new(floor)), false, vec![floor_loop]);
            body.add_shell(vec![dome, floor]);
        }
    }
    if v.tilted {
        body = body.transformed(&tilt(k)).unwrap();
    }
    body
}

/// The same solid on the analytic surface.
fn analytic(shape: Shape, k: f64, tilted: bool) -> Body {
    let body = match shape {
        Shape::Can => primitives::cylinder(&Frame::WORLD, 2.0 * k, 3.0 * k).unwrap(),
        Shape::Ball => primitives::sphere(&Frame::WORLD, 2.0 * k).unwrap(),
        Shape::Ring => primitives::torus(&Frame::WORLD, 2.0 * k, 0.6 * k).unwrap(),
        Shape::Cup => {
            let ball = primitives::sphere(&Frame::WORLD, 2.0 * k).unwrap();
            let upper = boxed(k, [-3.0, -3.0, 0.0], [3.0, 3.0, 3.0]);
            solve(&ball, &upper, Intersect, k).body
        }
    };
    if tilted {
        body.transformed(&tilt(k)).unwrap()
    } else {
        body
    }
}

/// Volume and area at scale 1, by the closed forms.
fn closed_form(shape: Shape) -> (f64, f64) {
    match shape {
        Shape::Can => (12.0 * PI, 12.0 * PI + 8.0 * PI),
        Shape::Ball => (32.0 * PI / 3.0, 16.0 * PI),
        Shape::Cup => (16.0 * PI / 3.0, 8.0 * PI + 4.0 * PI),
        Shape::Ring => (2.0 * PI * PI * 2.0 * 0.36, 4.0 * PI * PI * 2.0 * 0.6),
    }
}

/// A tilt and a shift that leave nothing along an axis.
fn tilt(k: f64) -> Transform {
    Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
        .unwrap()
        .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * k))
}

/// Tools that cross the seam where it starts at angle 0 (and run past it
/// elsewhere when it starts further round), through a pole, or around
/// the axis.
fn tools(shape: Shape, k: f64) -> Vec<(&'static str, Body)> {
    match shape {
        Shape::Can => vec![
            ("block", boxed(k, [1.2, -0.8, 1.0], [3.0, 0.9, 2.2])),
            ("half", boxed(k, [0.3, -3.0, -1.0], [3.0, 3.0, 4.0])),
            ("log", log(k, [0.6, 0.1, 1.6], 0.5, 3.0)),
            ("ball", ball(k, [1.9, 0.2, 1.4], 0.8)),
            ("spike", spike(k, [1.9, -0.1, 0.6], 0.7, 1.9)),
            ("ring", ring(k, [2.0, 0.05, 1.5], 0.8, 0.3)),
        ],
        Shape::Ball => vec![
            ("cap", boxed(k, [-0.6, -0.5, 1.2], [3.0, 0.6, 3.0])),
            ("log", log(k, [-3.0, 0.1, 0.3], 0.5, 6.0)),
            ("ball", ball(k, [1.9, 0.1, 0.4], 0.7)),
            ("poles", boxed(k, [-3.0, -3.0, -3.0], [3.0, 0.0, 3.0])),
            ("spike", spike(k, [1.6, -0.2, -0.5], 0.6, 1.8)),
        ],
        Shape::Cup => vec![
            ("cap", boxed(k, [-0.6, -0.5, 1.2], [3.0, 0.6, 3.0])),
            ("ball", ball(k, [1.8, 0.1, 0.6], 0.7)),
            ("log", log(k, [-3.0, 0.1, 0.8], 0.5, 6.0)),
            ("half", boxed(k, [-3.0, -3.0, -1.0], [3.0, 0.3, 3.0])),
        ],
        Shape::Ring => vec![
            ("block", boxed(k, [1.5, -0.5, -0.2], [3.0, 0.5, 1.0])),
            ("drum", rod(k, [0.0, 0.0, -1.0], 2.3, 1.3)),
            ("ball", ball(k, [2.5, 0.1, 0.3], 0.5)),
            ("log", log(k, [-3.0, 0.05, 0.1], 0.3, 6.0)),
            ("slice", boxed(k, [0.5, -0.3, -1.0], [3.5, 0.3, 1.0])),
        ],
    }
}

/// The variants of a shape's twin that are cut.
fn variants(shape: Shape) -> Vec<Variant> {
    let v = |from, tube_from, around_v, seam, tilted| Variant {
        from,
        tube_from,
        around_v,
        seam,
        tilted,
    };
    match shape {
        Shape::Can => vec![
            v(0.0, 0.0, false, false, false),
            v(0.0, 0.0, true, true, false),
            v(0.0, 0.0, false, true, true),
            v(2.5, 0.0, true, false, true),
        ],
        Shape::Ring => vec![
            v(0.0, 0.0, false, false, false),
            v(0.0, 1.0, true, false, true),
            v(2.5, 3.0, false, false, true),
        ],
        Shape::Ball | Shape::Cup => vec![
            v(0.0, 0.0, false, false, false),
            v(0.0, 0.0, true, false, true),
            v(2.5, 0.0, false, false, true),
        ],
    }
}

#[track_caller]
fn near(actual: f64, expected: f64, rel: f64, what: &str) -> f64 {
    let off = (actual - expected).abs() / expected.abs().max(actual.abs());
    assert!(
        off <= rel,
        "{what}: {actual} is not {expected} (off by {off:e})"
    );
    off
}

/// The mesh of a body is closed and encloses its volume.
fn meshes(body: &Body, k: f64) -> Result<(), String> {
    let exact = mass(body);
    let chord = 5e-3 * k;
    let params = TessParams {
        chordal_tolerance: chord,
        ..TessParams::default()
    };
    let mesh = tessellate_body(body, &params).map_err(|e| e.to_string())?;
    let welded = mesh.weld().map_err(|e| format!("{e:?}"))?;
    if !welded.is_closed() {
        return Err("not watertight".into());
    }
    if (welded.volume() - exact.volume).abs() > exact.area * chord {
        return Err(format!(
            "mesh volume {} against {}",
            welded.volume(),
            exact.volume
        ));
    }
    Ok(())
}

#[test]
fn the_closed_twins_are_valid_and_measure_as_the_closed_forms() {
    for k in SCALES {
        let tol = tolerance(k);
        for shape in [Shape::Can, Shape::Ball, Shape::Cup, Shape::Ring] {
            let (volume, area) = closed_form(shape);
            for v in variants(shape) {
                let what = format!("{shape:?} {v:?} at {k}");
                let body = twin(shape, k, v);
                assert_eq!(check(&body, &tol), [], "{what}");
                let m = mass(&body);
                near(m.volume, volume * k * k * k, 1e-9, &what);
                near(m.area, area * k * k, 1e-9, &what);
                // The mesh is a second opinion, from another crate: what
                // it declines is listed, not failed here.
                if let Err(e) = meshes(&body, k) {
                    println!("not meshed: {what}: {e}");
                }
            }
        }
    }
}

/// One case: a twin, a tool, a scale.
struct Case {
    shape: Shape,
    variant: Variant,
    tool: &'static str,
    k: f64,
}

/// What a case found: the worst relative differences in volume and area
/// against the analytic twin, and the meshes that failed.
struct Found {
    volume: f64,
    area: f64,
    identity: f64,
    unmeshed: Vec<String>,
    seconds: f64,
    /// Results the measure declined, held to the analytic ones by sampled
    /// membership.
    sampled: usize,
}

/// Two bodies hold the same points: 4000 seeded points of the box the
/// solids are in, classified by ray casting (`boolean::contains_points`,
/// which does not go through the measure). A point within the tolerance
/// of a boundary is classified either way, and a random point is that
/// close with a chance of about a millionth; one disagreement is allowed.
fn same_points(body: &Body, expected: &Body, k: f64, tilted: bool, what: &str) {
    let mut rng = Rng(0x5eed_c105_ed00);
    let points: Vec<Vec3> = (0..4000)
        .map(|_| {
            let p = Vec3::new(
                rng.between(-3.2, 3.2),
                rng.between(-3.2, 3.2),
                rng.between(-1.2, 3.2),
            ) * k;
            if tilted { tilt(k).point(p) } else { p }
        })
        .collect();
    let tol = tolerance(k);
    let ours = craft3d_kernel::boolean::contains_points(body, &points, &tol).unwrap();
    let theirs = craft3d_kernel::boolean::contains_points(expected, &points, &tol).unwrap();
    let decided = ours
        .iter()
        .zip(&theirs)
        .filter(|(a, b)| a.is_some() && b.is_some())
        .count();
    let differ = ours
        .iter()
        .zip(&theirs)
        .filter(|(a, b)| a.is_some() && b.is_some() && a != b)
        .count();
    let inside = theirs.iter().filter(|x| **x == Some(true)).count();
    assert!(
        decided >= 3900 && differ <= 1 && inside > 100,
        "{what}: {differ} of {decided} points differ ({inside} inside)"
    );
}

fn run(case: &Case) -> Found {
    let (k, v) = (case.k, case.variant);
    let tol = tolerance(k);
    let (_, mut tool) = tools(case.shape, k)
        .into_iter()
        .find(|(name, _)| *name == case.tool)
        .unwrap();
    if v.tilted {
        tool = tool.transformed(&tilt(k)).unwrap();
    }
    let a = twin(case.shape, k, v);
    let plain = analytic(case.shape, k, v.tilted);
    let mut found = Found {
        volume: 0.0,
        area: 0.0,
        identity: 0.0,
        unmeshed: Vec::new(),
        seconds: 0.0,
        sampled: 0,
    };
    let mut volumes = Vec::new();
    for op in [Union, Intersect, Subtract] {
        let what = format!("{:?} {v:?} and {} at {k}, {op:?}", case.shape, case.tool);
        let expected = solve(&plain, &tool, op, k);
        let started = Instant::now();
        let result = match boolean(&a, &tool, op, &tol) {
            Ok(result) => result,
            Err(e) => panic!("{what}: {e}"),
        };
        found.seconds += started.elapsed().as_secs_f64();
        assert_eq!(check(&result.body, &tol), [], "{what}");
        let want = mass(&expected.body);
        let got = match craft3d_topo::mass_properties(&result.body) {
            Ok(got) => got,
            Err(e) => {
                // The measure declines a boundary through both poles of a
                // spline surface that also crosses its seam (a limitation
                // of `topo`, named there): the body is held to the
                // analytic one by sampled membership instead.
                assert!(
                    e.to_string().contains("two poles"),
                    "{what}: not measured: {e}"
                );
                same_points(&result.body, &expected.body, k, v.tilted, &what);
                found.sampled += 1;
                volumes.push((result, None));
                continue;
            }
        };
        found.volume = found.volume.max(near(
            got.volume,
            want.volume,
            1e-6,
            &format!("{what}: volume"),
        ));
        found.area = found
            .area
            .max(near(got.area, want.area, 1e-6, &format!("{what}: area")));
        assert!(
            got.centroid.unwrap().distance(want.centroid.unwrap()) <= 1e-5 * k,
            "{what}: centroid"
        );
        assert_eq!(
            result.body.shells.len(),
            expected.body.shells.len(),
            "{what}: shells"
        );
        if let Err(e) = meshes(&result.body, k) {
            found.unmeshed.push(format!("{what}: {e}"));
        }
        volumes.push((result, Some(got.volume)));
    }
    // The identities of sets, by volume: (A − B) + (A ∩ B) = A and
    // (A ∪ B) + (A ∩ B) = A + B.
    let (va, vb) = (mass(&a).volume, mass(&tool).volume);
    if let (Some(union), Some(common), Some(rest)) = (volumes[0].1, volumes[1].1, volumes[2].1) {
        found.identity = found
            .identity
            .max(near(rest + common, va, 1e-7, "(A − B) + (A ∩ B)"))
            .max(near(union + common, va + vb, 1e-7, "(A ∪ B) + (A ∩ B)"));
    }
    // And A put together again from its two parts, which meet in faces on
    // the tool's surface and are cut from the closed face.
    let what = format!(
        "{:?} {v:?} and {} at {k}: (A − B) ∪ (A ∩ B)",
        case.shape, case.tool
    );
    let again = match boolean(&volumes[2].0.body, &volumes[1].0.body, Union, &tol) {
        Ok(again) => again,
        Err(e) => panic!("{what}: {e}"),
    };
    assert_eq!(check(&again.body, &tol), [], "{what}");
    found.identity = found
        .identity
        .max(near(mass(&again.body).volume, va, 1e-7, &what));
    found
}

fn cases(shape: Shape) -> Vec<Case> {
    let mut all = Vec::new();
    for k in SCALES {
        for variant in variants(shape) {
            for (tool, _) in tools(shape, k) {
                all.push(Case {
                    shape,
                    variant,
                    tool,
                    k,
                });
            }
        }
    }
    all
}

fn against_the_analytic_twin(shape: Shape) {
    let all = cases(shape);
    let count = all.len();
    let found = map_parallel(all, |case| run(&case));
    let worst = |pick: fn(&Found) -> f64| found.iter().map(pick).fold(0.0, f64::max);
    let unmeshed: Vec<&String> = found.iter().flat_map(|f| &f.unmeshed).collect();
    println!(
        "{shape:?}: {count} cases, {} booleans of the twin in {:.1} s; worst off the analytic twin: volume {:.1e}, area {:.1e}; identities {:.1e}; {} results held by sampled points (not measured); {} results not meshed",
        3 * count,
        found.iter().map(|f| f.seconds).sum::<f64>(),
        worst(|f| f.volume),
        worst(|f| f.area),
        worst(|f| f.identity),
        found.iter().map(|f| f.sampled).sum::<usize>(),
        unmeshed.len()
    );
    for line in &unmeshed {
        println!("  not meshed: {line}");
    }
}

#[test]
fn a_closed_rational_can_against_every_kind_of_tool() {
    against_the_analytic_twin(Shape::Can);
}

#[test]
fn a_closed_rational_ball_against_every_kind_of_tool() {
    against_the_analytic_twin(Shape::Ball);
}

#[test]
fn a_closed_rational_cup_against_every_kind_of_tool() {
    against_the_analytic_twin(Shape::Cup);
}

#[test]
fn a_closed_rational_ring_against_every_kind_of_tool() {
    against_the_analytic_twin(Shape::Ring);
}

/// `TWIN_CASE="shape variant tool k"` (the shape as `Shape` names it, the
/// variant an index into `variants`, the tool by its name in `tools`)
/// with `--ignored`: one case, its three booleans and the union of the
/// parts, with the volumes printed.
#[test]
#[ignore]
fn one_case() {
    let spec = std::env::var("TWIN_CASE").unwrap_or_else(|_| "Can 0 block 1".into());
    let parts: Vec<&str> = spec.split_whitespace().collect();
    let shape = match parts[0] {
        "Can" => Shape::Can,
        "Ball" => Shape::Ball,
        "Cup" => Shape::Cup,
        _ => Shape::Ring,
    };
    let variant = variants(shape)[parts[1].parse::<usize>().unwrap()];
    let k: f64 = parts[3].parse().unwrap();
    let a = twin(shape, k, variant);
    let (_, mut b) = tools(shape, k)
        .into_iter()
        .find(|(name, _)| *name == parts[2])
        .unwrap();
    if variant.tilted {
        b = b.transformed(&tilt(k)).unwrap();
    }
    let volume = |body: &Body| craft3d_topo::mass_properties(body).map(|m| m.volume);
    let mut results = Vec::new();
    for op in [Union, Intersect, Subtract] {
        match boolean(&a, &b, op, &tolerance(k)) {
            Ok(r) => {
                println!("{op:?}: {:?}", volume(&r.body));
                results.push(r.body);
            }
            Err(e) => println!("{op:?}: {e}"),
        }
    }
    if let [_, common, rest] = results.as_slice() {
        match boolean(rest, common, Union, &tolerance(k)) {
            Ok(r) => println!("(A − B) ∪ (A ∩ B): {:?}", volume(&r.body)),
            Err(e) => println!("(A − B) ∪ (A ∩ B): {e}"),
        }
    }
}
