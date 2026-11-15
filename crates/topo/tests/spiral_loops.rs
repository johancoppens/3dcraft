//! A thin strip that spirals about a point of its surface's chart is a
//! right face however many turns one of its edges makes
//! (`docs/design/topology.md` › *A strip that spirals about a point of the
//! chart*): until 2026-11-14 a spline edge was followed in eight steps
//! whatever it did, and a side of 16 turns in one edge, two whole turns a
//! step, read as a straight line. And what the walk still misreads, each
//! pinned as it is today so that a fix turns it round: wrong strips passed
//! unjudged, wavy ribbons whose steps end on their middle line, a step
//! whose halves each run one way going round a point.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "common/spirals.rs"]
mod spirals;

use craft3d_topo::{Body, Defect, check, forget_passed, mass_properties};
use spirals::{Strip, grounds, spiral_strip, tolerance, turned};

/// What the check says of the loops of a body's faces.
fn loop_defects(body: &Body, k: f64) -> Vec<String> {
    forget_passed();
    check(body, &tolerance(k))
        .iter()
        .filter(|d| {
            matches!(
                d,
                Defect::LoopsBoundNoRegion { .. } | Defect::LoopNotSimple { .. }
            )
        })
        .map(ToString::to_string)
        .collect()
}

/// The smallest strip found refused: on a plane, the spiral from radius
/// 0.45 to 1.65 in 16 turns, the strip 0.35 of the pitch wide, its outer
/// side one edge and its inner side cut into quarter turns ("every loop
/// runs clockwise about the outward normal"), or the other way about ("it
/// winds 2 times about the point …"). Right by its measure: the area is the
/// closed form's.
#[test]
fn the_smallest_spiral_strip_is_a_face() {
    let (_, plane, at, area) = grounds(1.0).swap_remove(0);
    for cut in [(0, 4), (4, 0), (0, 0), (4, 4)] {
        let strip = Strip {
            cut,
            ..Strip::of(16.0)
        };
        let body = spiral_strip(&plane, at.as_ref(), &strip, 1e-6).unwrap();
        let want = strip.chart_area() * area.unwrap();
        let got = mass_properties(&body).unwrap().area;
        assert!(
            (got / want - 1.0).abs() < 1e-8,
            "cut {cut:?}: area {got} against {want}"
        );
        assert_eq!(
            check(&body, &tolerance(1.0)),
            Vec::new(),
            "cut {cut:?}: a right strip"
        );
    }
}

/// The strips the review found refused, and their neighbours: 15 to 33
/// turns, either side cut into quarter turns, on a plane, a cylinder, a
/// cone and a sphere, at three scales, in the world and turned a thousand
/// sizes off. None has a fault of its loops; on the plane and the cylinder
/// each measures its closed form (those of 16 turns also where they are
/// turned and far off).
#[test]
fn spiral_strips_with_a_side_cut_are_faces() {
    let mut refused = Vec::new();
    let mut count = 0;
    for k in [1e-3, 1.0, 1e6] {
        for (name, surface, at, area) in grounds(k) {
            for turns in [15.0, 16.0, 16.5, 17.0, 32.0, 33.0] {
                for cut in [(4, 0), (0, 4)] {
                    let strip = Strip {
                        cut,
                        ..Strip::of(turns)
                    };
                    let body = spiral_strip(&surface, at.as_ref(), &strip, 1e-6 * k).unwrap();
                    if let Some(area) = area {
                        let want = strip.chart_area() * area;
                        let got = mass_properties(&body).unwrap().area;
                        assert!(
                            (got / want - 1.0).abs() < 1e-7,
                            "{name} at {k}, {turns} turns, cut {cut:?}: area {got} against {want}"
                        );
                    }
                    for (place, body) in [
                        ("world", body.clone()),
                        ("turned", body.transformed(&turned(k, 1e3)).unwrap()),
                    ] {
                        count += 1;
                        // The closed form far off and turned as well.
                        if let (Some(area), "turned", true) = (area, place, turns == 16.0) {
                            let want = strip.chart_area() * area;
                            let got = mass_properties(&body).unwrap().area;
                            assert!(
                                (got / want - 1.0).abs() < 1e-7,
                                "{name} at {k} turned, cut {cut:?}: area {got} against {want}"
                            );
                        }
                        let found = loop_defects(&body, k);
                        if !found.is_empty() {
                            refused.push(format!(
                                "{name} at {k} {place}, {turns} turns, cut {cut:?}: {}",
                                found.join("; ")
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        refused.is_empty(),
        "{} of {count} right strips refused:\n{}",
        refused.len(),
        refused.join("\n")
    );
}

/// A strip that is wrong is refused as it was, where its sides are in
/// edges short enough to be followed closely (both cut into quarter
/// turns): turned over (the face flipped, its loop kept: everything
/// outside the strip) by the region rule, and wider than its pitch, so
/// that it lies over itself, by the rule of simple loops. At three scales
/// on a plane, at one on a cylinder and a cone, in the world and turned.
#[test]
fn a_spiral_strip_turned_over_or_over_itself_is_refused() {
    for k in [1e-3, 1.0, 1e6] {
        for (name, surface, at, _) in grounds(k) {
            if name == "sphere" || (name != "plane" && k != 1.0) {
                continue;
            }
            for (strip, what) in [
                (
                    Strip {
                        cut: (4, 4),
                        flipped: true,
                        ..Strip::of(2.0)
                    },
                    "do not bound a region",
                ),
                (
                    Strip {
                        cut: (4, 4),
                        flipped: true,
                        ..Strip::of(5.0)
                    },
                    "do not bound a region",
                ),
                (
                    Strip {
                        cut: (4, 4),
                        share: 1.5,
                        ..Strip::of(3.0)
                    },
                    "is not a simple closed curve",
                ),
            ] {
                let body = spiral_strip(&surface, at.as_ref(), &strip, 1e-6 * k).unwrap();
                for body in [body.clone(), body.transformed(&turned(k, 1e3)).unwrap()] {
                    let found = loop_defects(&body, k);
                    assert!(
                        found.len() == 1 && found[0].contains(what),
                        "{name} at {k}, {strip:?}: {found:?}"
                    );
                }
            }
        }
    }
}

/// A side of `turns` turns as one cubic on control points along the
/// spiral, eight a turn, `off` from the middle spiral.
fn wound(turns: usize, off: f64) -> craft3d_geom::NurbsCurve {
    use craft3d_math::Portable;
    let count = 8 * turns + 1;
    let points: Vec<craft3d_math::Vec3> = (0..count)
        .map(|i| {
            let t = std::f64::consts::TAU * i as f64 / 8.0;
            let r = 1.0 + 0.01 * i as f64 / 8.0 + off;
            let (s, c) = t.psin_cos();
            craft3d_math::Vec3::new(r * c, r * s, 0.0)
        })
        .collect();
    let spans = count - 3;
    let mut knots = vec![0.0; 3];
    knots.extend((0..=spans).map(|j| j as f64 / spans as f64));
    knots.extend([1.0; 3]);
    craft3d_geom::NurbsCurve::polynomial(3, knots, points).unwrap()
}

/// A strip on the plane between two sides of `turns` turns, each one edge
/// ([`wound`]), `width` apart, with lines across its ends.
fn wound_strip(turns: usize, width: f64) -> Body {
    use craft3d_geom::{Curve3, Line, Plane, Surface};
    use craft3d_topo::{BodyKind, Coedge};
    let (outer, inner) = (wound(turns, 0.5 * width), wound(turns, -0.5 * width));
    let mut body = Body::new(BodyKind::Sheet);
    let ends = [
        outer.eval(0.0),
        outer.eval(1.0),
        inner.eval(1.0),
        inner.eval(0.0),
    ];
    let v: Vec<_> = ends.iter().map(|p| body.add_vertex(*p)).collect();
    let out = body.add_edge(Curve3::Nurbs(outer), (0.0, 1.0), v[0], v[1]);
    let back = body.add_edge(Curve3::Nurbs(inner), (0.0, 1.0), v[3], v[2]);
    let mut across = |a: usize, b: usize| {
        let line = Line::through(ends[a], ends[b]).unwrap();
        body.add_edge(
            Curve3::Line(line),
            (0.0, ends[a].distance(ends[b])),
            v[a],
            v[b],
        )
    };
    let (end, start) = (across(1, 2), across(3, 0));
    let forward = |edge| Coedge {
        edge,
        reversed: false,
    };
    let lp = body.add_loop(vec![
        forward(out),
        forward(end),
        Coedge {
            edge: back,
            reversed: true,
        },
        forward(start),
    ]);
    let plane = Surface::Plane(Plane::new(craft3d_math::Frame::WORLD));
    let face = body.add_face(plane, false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// Hostile strips get an answer, never a panic or a hang: a spiral of
/// three thousand turns between two single edges is not followed (it
/// passes, and is counted, for work that stays within the bound of a
/// face); a strip narrower than the tolerance, and one whose numbers are
/// not numbers, are given their defects.
#[test]
fn hostile_spirals_get_an_answer() {
    use craft3d_topo::{MOST_PROOF_WORK, step_proofs};
    let long = wound_strip(3000, 0.004);
    forget_passed();
    let (work, cut) = step_proofs();
    assert_eq!(loop_defects(&long, 1.0), Vec::<String>::new());
    let (work_now, cut_now) = step_proofs();
    assert_eq!(cut_now - cut, 1, "the face is counted as not followed");
    assert!(
        work_now - work <= 2 * MOST_PROOF_WORK as u64,
        "{}",
        work_now - work
    );
    // A strip a thousandth of the tolerance wide: an answer either way.
    let thin = wound_strip(3, 1e-9);
    let _ = check(&thin, &tolerance(1.0));
    // Numbers that are none: a vertex, an edge's range.
    for spoil in 0..3 {
        let mut body = wound_strip(3, 0.004);
        match spoil {
            0 => body.vertices[0].point.x = f64::NAN,
            1 => body.edges[0].range = (0.0, f64::NAN),
            _ => body.edges[1].range = (f64::NEG_INFINITY, f64::INFINITY),
        }
        assert!(!check(&body, &tolerance(1.0)).is_empty(), "spoilt {spoil}");
    }
}

/// **Known misses**: a wrong strip whose outer side is cut into quarter
/// turns and whose inner side is one edge, turned over (the face flipped,
/// its loop kept: everything outside the strip), at 15, 17, 31 and 33
/// turns, passes, not judged. Its single side is followed a quarter turn a
/// half step, too coarsely for the strip's area to be told from what the
/// walk measures of its error. Until 2026-11-14 that side was followed in
/// eight steps, and these strips were refused by their area ("every loop
/// runs clockwise about the outward normal"), rightly, while their right
/// twins passed (the review of 2026-11-15: 40 such strips on a plane, a
/// cylinder and a cone, and 3 about a cone's apex). Listed so that a fix
/// (`docs/design/topology.md` › *A strip that spirals about a point of the
/// chart*, Owed) turns them round: on a plane at three scales, on a
/// cylinder and a cone at one, in the world and turned.
#[test]
fn wrong_spiral_strips_with_the_outer_side_cut_pass_and_the_misses_are_known() {
    let mut judged = Vec::new();
    for k in [1e-3, 1.0, 1e6] {
        for (name, surface, at, _) in grounds(k) {
            if name == "sphere" || (name != "plane" && k != 1.0) {
                continue;
            }
            for turns in [15.0, 17.0, 31.0, 33.0] {
                let strip = Strip {
                    cut: (4, 0),
                    flipped: true,
                    ..Strip::of(turns)
                };
                let body = spiral_strip(&surface, at.as_ref(), &strip, 1e-6 * k).unwrap();
                for (place, body) in [
                    ("world", body.clone()),
                    ("turned", body.transformed(&turned(k, 1e3)).unwrap()),
                ] {
                    let found = loop_defects(&body, k);
                    if !found.is_empty() {
                        judged.push(format!("{name} at {k} {place}, {turns} turns: {found:?}"));
                    }
                }
            }
        }
    }
    assert!(
        judged.is_empty(),
        "known misses now refused (take them off the list):\n{}",
        judged.join("\n")
    );
}

/// A ribbon of the chart `at` between the wave `y = a (sin φ + 0.3 sin 3φ)`,
/// `φ` running `waves` periods over `x` from −0.6 to 0.6, and the same wave
/// lifted by half its amplitude `a` (`slope` over the wave's angular
/// rate, at most 0.05): the lower side one cubic through 64 points a wave,
/// the upper side one cubic alike cut into `pieces` edges over equal parts
/// of its range, short cubics across the ends. Its loop runs
/// counter-clockwise in the chart; `flipped` turns the face over, its loop
/// kept. With eight or sixteen waves the ends and the middles of the lower
/// side's eight steps all lie where it crosses its middle line. Also the
/// gap between the sides in the chart.
fn wave_ribbon(
    surface: &craft3d_geom::Surface,
    at: &dyn Fn(f64, f64) -> craft3d_math::Vec3,
    waves: f64,
    slope: f64,
    pieces: usize,
    flipped: bool,
    tol: f64,
) -> (Body, f64) {
    use craft3d_geom::Curve3;
    use craft3d_math::Portable;
    use craft3d_topo::{BodyKind, Coedge};
    let rate = std::f64::consts::TAU * waves / 1.2;
    let a = (slope / rate).min(0.05);
    let gap = 0.5 * a;
    let low = move |x: f64| {
        let phi = rate * (x + 0.6);
        a * (phi.psin() + 0.3 * (3.0 * phi).psin())
    };
    let side = |lift: f64| {
        let f = |s: f64| {
            let x = -0.6 + 1.2 * s;
            at(x, low(x) + lift)
        };
        spirals::fitted(
            &f,
            (0.0, 1.0),
            (64.0 * waves) as usize,
            3,
            false,
            0.05 * tol,
        )
        .unwrap()
    };
    let (lower, upper) = (side(0.0), side(gap));
    let across = |x: f64, up: bool| {
        let f = |s: f64| at(x, low(x) + gap * if up { s } else { 1.0 - s });
        spirals::fitted(&f, (0.0, 1.0), 8, 3, false, 0.05 * tol).unwrap()
    };
    let mut body = Body::new(BodyKind::Sheet);
    let (al, bl) = (
        body.add_vertex(lower.eval(0.0)),
        body.add_vertex(lower.eval(1.0)),
    );
    let mut ups = vec![body.add_vertex(upper.eval(0.0))];
    for i in 1..pieces {
        ups.push(body.add_vertex(upper.eval(i as f64 / pieces as f64)));
    }
    ups.push(body.add_vertex(upper.eval(1.0)));
    let bottom = body.add_edge(Curve3::Nurbs(lower), (0.0, 1.0), al, bl);
    let right = body.add_edge(
        Curve3::Nurbs(across(0.6, true)),
        (0.0, 1.0),
        bl,
        ups[pieces],
    );
    let tops: Vec<_> = (0..pieces)
        .map(|i| {
            let range = (i as f64 / pieces as f64, (i + 1) as f64 / pieces as f64);
            body.add_edge(Curve3::Nurbs(upper.clone()), range, ups[i], ups[i + 1])
        })
        .collect();
    let left = body.add_edge(Curve3::Nurbs(across(-0.6, false)), (0.0, 1.0), ups[0], al);
    let co = |edge, reversed| Coedge { edge, reversed };
    let mut coedges = vec![co(bottom, false), co(right, false)];
    coedges.extend(tops.iter().rev().map(|e| co(*e, true)));
    coedges.push(co(left, false));
    let lp = body.add_loop(coedges);
    let face = body.add_face(surface.clone(), flipped, vec![lp]);
    body.add_shell(vec![face]);
    (body, gap)
}

/// Wavy ribbons whose lower side's eight steps end, and are measured, where
/// it crosses its middle line (the review of 2026-11-15): the walk reads
/// that side as a straight line, its error as nothing, while it lies
/// further from its chords than the ribbon is wide.
///
/// - Right, its sides' slopes up to 1.9 in the chart, the upper side in
///   four edges a wave: **a known false alarm** on a plane (three scales),
///   a cylinder and a sphere, refused as on `main` and in 0.1.x ("its loop
///   0 crosses itself": the lower side's chords cross the upper side),
///   while it measures its closed form. The halves of the lower side's
///   steps are shown to run one way, so it is followed in its eight steps;
///   that what lies between a step and its chord is measured only at the
///   middle is what misreads it (`docs/design/topology.md` › *A strip that
///   spirals about a point of the chart*, Owed). Listed so that a fix
///   turns them round.
/// - Right, slopes up to 5.7, on a plane: passes (the lower side's halves
///   are not one way at eight steps, so it is followed in more).
/// - Wrong, slopes up to 5.7, both sides single edges, turned over:
///   **known misses** on a plane, passed unjudged (both sides followed in
///   more steps, the area cannot be told from their error), where until
///   2026-11-14 they were refused by their area, rightly.
#[test]
fn wavy_ribbons_whose_steps_end_on_their_middle_line() {
    /// A surface, the map from the chart to it, and the factor from the
    /// chart's area to the surface's where that is a constant.
    type WaveChart = (
        craft3d_geom::Surface,
        Box<dyn Fn(f64, f64) -> craft3d_math::Vec3>,
        Option<f64>,
    );
    use craft3d_geom::{Cylinder, Plane, Sphere, Surface};
    use craft3d_math::{Frame, Portable, Vec3};
    let mut wrong = Vec::new();
    for k in [1e-3, 1.0, 1e6] {
        let plane: WaveChart = (
            Surface::Plane(Plane::new(Frame::WORLD)),
            Box::new(move |x, y| Vec3::new(2.0 * k * x, 2.0 * k * y, 0.0)),
            Some(4.0 * k * k),
        );
        let mut charts = vec![("plane", plane)];
        if k == 1.0 {
            charts.push((
                "cylinder",
                (
                    Surface::Cylinder(Cylinder::new(Frame::WORLD, 1.3).unwrap()),
                    Box::new(|x: f64, y: f64| {
                        let (s, c) = x.psin_cos();
                        Vec3::new(1.3 * c, 1.3 * s, 1.3 * y)
                    }),
                    Some(1.69),
                ),
            ));
            charts.push((
                "sphere",
                (
                    Surface::Sphere(Sphere::new(Frame::WORLD, 1.7).unwrap()),
                    Box::new(|x: f64, y: f64| {
                        let (su, cu) = x.psin_cos();
                        let (sv, cv) = (0.2 + y).psin_cos();
                        Vec3::new(1.7 * cv * cu, 1.7 * cv * su, 1.7 * sv)
                    }),
                    None,
                ),
            ));
        }
        for (name, (surface, at, scale)) in &charts {
            for waves in [8.0, 16.0] {
                let mut cases = vec![(1.0, 4, false, "refused")];
                if *name == "plane" {
                    cases.push((3.0, 4, false, "passes"));
                    cases.push((3.0, 1, true, "passes"));
                }
                for (slope, a_wave, flipped, want) in cases {
                    let pieces = if a_wave == 1 {
                        1
                    } else {
                        a_wave * waves as usize
                    };
                    let (body, gap) = wave_ribbon(
                        surface,
                        at.as_ref(),
                        waves,
                        slope,
                        pieces,
                        flipped,
                        1e-6 * k,
                    );
                    if let (Some(scale), false) = (scale, flipped) {
                        let want = 1.2 * gap * scale;
                        let got = mass_properties(&body).unwrap().area;
                        assert!(
                            (got / want - 1.0).abs() < 1e-7,
                            "{name} at {k}, {waves} waves, slope {slope}: area {got} against {want}"
                        );
                    }
                    let found = loop_defects(&body, k);
                    let said = match found.as_slice() {
                        [] => "passes",
                        [one] if one.contains("its loop 0 crosses itself") => "refused",
                        _ => "something else",
                    };
                    if said != want {
                        wrong.push(format!(
                            "{name} at {k}, {waves} waves, slope {slope}, {pieces} edges above, flipped {flipped}: {said}, not {want}: {found:?}"
                        ));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// **A known miss**: a step whose two halves each run one way can still go
/// round a point twice with the chord of the whole step while its middle
/// lies on that chord (the review of 2026-11-15). The lower side of a strip
/// on a plane is a spline of degree 1 whose eight steps each run from
/// `(0, 0)` by `(1.0713, -0.2204)` to `(1, 0)`, its middle, and by
/// `(0.9439, -0.4633)` to `(2, 0)` (a tenth of that across), each half's
/// legs within 120.6°, under the three eighths of a turn the rule allows:
/// the walk measures the middle on the chord, an error of nothing, while
/// the step with its chord winds twice about `(0.0994, -0.0163)`. The edge
/// crosses itself in every step, so the face is wrong; it passes, as it
/// did before the one-way rule. The rule shows that the chord of a half
/// step hides no turn, not that the chord of a step does
/// (`docs/design/topology.md` › *A strip that spirals about a point of the
/// chart*, Owed).
#[test]
fn a_step_whose_halves_each_run_one_way_can_go_round_a_point_unseen() {
    use craft3d_geom::{Curve3, NurbsCurve, Plane, Surface};
    use craft3d_math::{Frame, Vec3};
    use craft3d_topo::{BodyKind, Coedge};
    let step = [
        (0.0, 0.0),
        (1.071_276_613_925_536_3, -0.220_360_541_398_313_18),
        (1.0, 0.0),
        (0.943_896_263_786_737, -0.463_289_763_797_715_8),
        (2.0, 0.0),
    ];
    let mut points = vec![Vec3::new(0.0, 0.0, 0.0)];
    let mut knots = vec![0.0, 0.0];
    for s in 0..8 {
        for (i, (x, y)) in step.iter().enumerate().skip(1) {
            points.push(Vec3::new(0.1 * (2.0 * s as f64 + x), 0.1 * y, 0.0));
            knots.push((s as f64 + i as f64 / 4.0) / 8.0);
        }
    }
    // The last knot of the last step is the end of the domain.
    knots.pop();
    knots.extend([1.0, 1.0]);
    let side = NurbsCurve::polynomial(1, knots, points.clone()).unwrap();
    // Degree 1: the curve is its control polygon, which crosses itself in
    // each step (the leg out of the step's start and the leg out of its
    // middle).
    let cross = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| {
        let side =
            |p: Vec3, q: Vec3, r: Vec3| (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
        side(a, b, c) * side(a, b, d) < 0.0 && side(c, d, a) * side(c, d, b) < 0.0
    };
    for s in 0..8 {
        let at = |i: usize| points[4 * s + i];
        assert!(
            cross(at(0), at(1), at(2), at(3)),
            "step {s} does not cross itself"
        );
        // Its middle at the middle of its chord.
        let middle = side.eval((s as f64 + 0.5) / 8.0);
        assert!(middle.distance((at(0) + at(4)) * 0.5) < 1e-12);
    }
    let mut body = Body::new(BodyKind::Sheet);
    let (a, b) = (points[0], *points.last().unwrap());
    let corners = [
        a,
        b,
        b + Vec3::new(0.0, 0.2, 0.0),
        a + Vec3::new(0.0, 0.2, 0.0),
    ];
    let v: Vec<_> = corners.iter().map(|p| body.add_vertex(*p)).collect();
    let line = |p: Vec3, q: Vec3| {
        Curve3::Nurbs(NurbsCurve::polynomial(1, vec![0.0, 0.0, 1.0, 1.0], vec![p, q]).unwrap())
    };
    let mut edges = vec![body.add_edge(Curve3::Nurbs(side), (0.0, 1.0), v[0], v[1])];
    for i in 1..4 {
        let j = (i + 1) % 4;
        edges.push(body.add_edge(line(corners[i], corners[j]), (0.0, 1.0), v[i], v[j]));
    }
    let lp = body.add_loop(
        edges
            .iter()
            .map(|edge| Coedge {
                edge: *edge,
                reversed: false,
            })
            .collect(),
    );
    let face = body.add_face(Surface::Plane(Plane::new(Frame::WORLD)), false, vec![lp]);
    body.add_shell(vec![face]);
    assert_eq!(
        loop_defects(&body, 1.0),
        Vec::<String>::new(),
        "a known miss now refused (take it off the list)"
    );
}
