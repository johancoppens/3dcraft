//! Sheet bodies (the boolean checklist's item 5): open shells made by
//! `extrude::fill`, held to closed-form areas, Euler–Poincaré with their
//! boundary loops and meshes that are closed except along the boundary;
//! and booleans with a sheet. A sheet trimmed by a solid (intersect keeps
//! what is inside, subtract what is outside) is held to closed-form areas
//! and to `area(S ∩ A) + area(S ∖ A) = area(S)`; a solid cut by a sheet
//! (subtract keeps the side in front of the sheet, intersect the side
//! behind) to closed-form volumes and to `vol(front) + vol(behind) =
//! vol(A)`. Analytic and freeform operands (a ball, a cylinder, the spline
//! cone of the test kit, a sheet bounded by a spline), at 1e-3, 1 and 1e6.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;

use boolean_common::*;
use craft3d_geom::Curve3;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, BooleanResult, boolean};
use craft3d_kernel::extrude::fill;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::solids;
use craft3d_topo::{
    Body, BodyKind, Coedge, Defect, ShellId, boundary_loops, check, mass_properties,
};
use std::collections::BTreeMap;
use std::f64::consts::PI;

fn line(id: u64, start: [f64; 2], end: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

/// A sketch's curves times `k`, filled in the plane `frame`.
fn filled(curves: &[SketchCurve], frame: &Frame, k: f64) -> Body {
    let scale = |p: [f64; 2]| [p[0] * k, p[1] * k];
    let scaled: Vec<SketchCurve> = curves
        .iter()
        .map(|c| SketchCurve {
            id: c.id,
            curve: match &c.curve {
                Curve2::Line { start, end } => Curve2::Line {
                    start: scale(*start),
                    end: scale(*end),
                },
                Curve2::Arc { center, start, end } => Curve2::Arc {
                    center: scale(*center),
                    start: scale(*start),
                    end: scale(*end),
                },
                Curve2::Circle { center, radius } => Curve2::Circle {
                    center: scale(*center),
                    radius: radius * k,
                },
                Curve2::Spline(s) => Curve2::Spline(Spline2 {
                    degree: s.degree,
                    knots: s.knots.clone(),
                    control: s.control.iter().map(|&p| scale(p)).collect(),
                }),
            },
        })
        .collect();
    let tol = tolerance(k);
    let profiles = detect_profiles(&scaled, &tol).unwrap();
    let sheet = fill(&profiles, frame, &tol).unwrap().body;
    assert_eq!(sheet.kind, BodyKind::Sheet);
    assert_eq!(check(&sheet, &tol), [], "a fill is a valid sheet");
    sheet
}

/// The rectangle from `lo` to `hi` in the plane `frame`, times `k`.
fn rectangle(lo: [f64; 2], hi: [f64; 2], frame: &Frame, k: f64) -> Body {
    let corners = [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]];
    let curves: Vec<SketchCurve> = (0..4)
        .map(|i| line(i as u64 + 1, corners[i], corners[(i + 1) % 4]))
        .collect();
    filled(&curves, frame, k)
}

/// The horizontal plane at height `z` (times `k`), its normal up.
fn level(z: f64, k: f64) -> Frame {
    Frame::WORLD.at(Vec3::new(0.0, 0.0, z * k)).unwrap()
}

/// The edges of a body that one face uses (a sheet's boundary).
fn boundary_edges(body: &Body) -> Vec<usize> {
    let mut uses: BTreeMap<u32, usize> = BTreeMap::new();
    for face in &body.faces {
        for &l in &face.loops {
            for c in &body.loops[l.0 as usize].coedges {
                *uses.entry(c.edge.0).or_default() += 1;
            }
        }
    }
    uses.into_iter()
        .filter(|&(_, n)| n == 1)
        .map(|(e, _)| e as usize)
        .collect()
}

/// The welded mesh of `body` is closed except along its boundary edges:
/// every directed side of a triangle occurs at most once, a side without
/// its reverse lies on a boundary edge of the B-rep (within the chordal
/// tolerance and the edge's own), and those sides add up to the length of
/// the boundary (their chords, within the chordal tolerance's share).
fn closed_but_for_its_boundary(body: &Body, k: f64, what: &str) {
    let chordal = 1e-3 * k;
    let params = TessParams {
        chordal_tolerance: chordal,
        ..TessParams::default()
    };
    let mesh = tessellate_body(body, &params).unwrap().weld().unwrap();
    let mut sides: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for &[a, b, c] in &mesh.triangles {
        for side in [(a, b), (b, c), (c, a)] {
            *sides.entry(side).or_default() += 1;
        }
    }
    assert!(
        sides.values().all(|&n| n == 1),
        "{what}: a side of two triangles runs the same way"
    );
    let boundary = boundary_edges(body);
    let point = |i: u32| {
        let [x, y, z] = mesh.positions[i as usize];
        Vec3::new(x, y, z)
    };
    let mut open = 0.0;
    for &(a, b) in sides.keys() {
        if sides.contains_key(&(b, a)) {
            continue;
        }
        let middle = (point(a) + point(b)) * 0.5;
        let near = boundary.iter().any(|&e| {
            let edge = &body.edges[e];
            let t = edge.curve.project(middle).unwrap();
            edge.curve.eval(t).distance(middle) <= 2.0 * chordal
        });
        assert!(near, "{what}: the mesh is open off the sheet's boundary");
        open += point(a).distance(point(b));
    }
    let length: f64 = boundary
        .iter()
        .map(|&e| curve_length(&body.edges[e].curve, body.edges[e].range))
        .sum();
    assert!(
        (open - length).abs() <= 1e-2 * length,
        "{what}: the mesh is open along {open}, the boundary is {length} long"
    );
    if boundary.is_empty() {
        assert!(mesh.is_closed(), "{what}");
    }
}

fn curve_length(curve: &Curve3, (t0, t1): (f64, f64)) -> f64 {
    let n = 2048;
    (0..n)
        .map(|i| {
            let a = t0 + (t1 - t0) * i as f64 / n as f64;
            let b = t0 + (t1 - t0) * (i + 1) as f64 / n as f64;
            curve.eval(a).distance(curve.eval(b))
        })
        .sum()
}

/// Euler–Poincaré with the boundary, per shell, as the counts give it:
/// `V − E + F − R = 2 − 2g − b`.
fn genus_and_boundary(body: &Body) -> Vec<(i64, usize)> {
    (0..body.shells.len())
        .map(|s| {
            let shell = &body.shells[s];
            let mut vertices = std::collections::BTreeSet::new();
            let mut edges = std::collections::BTreeSet::new();
            let mut rings = 0_i64;
            for &f in &shell.faces {
                let face = &body.faces[f.0 as usize];
                rings += face.loops.len() as i64 - 1;
                for &l in &face.loops {
                    for c in &body.loops[l.0 as usize].coedges {
                        edges.insert(c.edge.0);
                        let e = &body.edges[c.edge.0 as usize];
                        vertices.insert(e.start.0);
                        vertices.insert(e.end.0);
                    }
                }
            }
            let b = boundary_loops(body, ShellId(s as u32)).unwrap();
            let chi = vertices.len() as i64 - edges.len() as i64 + shell.faces.len() as i64 - rings;
            let twice = 2 - b as i64 - chi;
            assert!(
                twice >= 0 && twice % 2 == 0,
                "shell {s}: χ = {chi}, b = {b}"
            );
            (twice / 2, b)
        })
        .collect()
}

/// A square of side 4 about the origin with a round hole of radius 1,
/// and beside it the region under a quadratic spline (a parabola's arch,
/// [`arch_area`]).
fn plate_and_arch() -> Vec<SketchCurve> {
    let mut curves: Vec<SketchCurve> = vec![
        line(1, [-2.0, -2.0], [2.0, -2.0]),
        line(2, [2.0, -2.0], [2.0, 2.0]),
        line(3, [2.0, 2.0], [-2.0, 2.0]),
        line(4, [-2.0, 2.0], [-2.0, -2.0]),
        SketchCurve {
            id: 5,
            curve: Curve2::Circle {
                center: [0.0, 0.0],
                radius: 1.0,
            },
        },
    ];
    curves.push(SketchCurve {
        id: 6,
        curve: Curve2::Spline(Spline2 {
            degree: 2,
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            control: vec![[3.0, 0.0], [5.0, 4.0], [7.0, 0.0]],
        }),
    });
    curves.push(line(7, [7.0, 0.0], [3.0, 0.0]));
    curves
}

#[test]
fn a_fill_is_a_sheet_with_the_area_of_its_profiles() {
    for k in SCALES {
        let tilted = Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * k,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap();
        let sheet = filled(&plate_and_arch(), &tilted, k);
        assert_eq!(sheet.faces.len(), 2);
        assert_eq!(sheet.shells.len(), 2);
        let m = mass_properties(&sheet).unwrap();
        let expected = (16.0 - PI + arch_area()) * k * k;
        assert!(
            (m.area - expected).abs() <= 1e-12 * expected,
            "scale {k}: area {} for {expected}",
            m.area
        );
        assert_eq!(m.volume, 0.0);
        assert_eq!(m.centroid, None);
        // The plate is a disc with a hole (two boundary loops), the arch a
        // disc (one).
        assert_eq!(genus_and_boundary(&sheet), vec![(0, 2), (0, 1)]);
        // Its normal is the plane's: the outer loop runs anticlockwise
        // seen from there.
        let face = &sheet.faces[0];
        let normal = face.surface.normal(0.0, 0.0).unwrap();
        assert!(normal.dot(tilted.z()) > 0.999 && !face.reversed);
        closed_but_for_its_boundary(&sheet, k, &format!("fill at {k}"));
    }
}

/// A disc on a plane, by hand: one vertex, one circle, one face.
fn disc(k: f64) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let frame = Frame::WORLD;
    let v = body.add_vertex(Vec3::new(k, 0.0, 0.0));
    let c = craft3d_geom::Circle::new(frame, k).unwrap();
    let e = body.add_edge(Curve3::Circle(c), (0.0, std::f64::consts::TAU), v, v);
    let l = body.add_loop(vec![Coedge {
        edge: e,
        reversed: false,
    }]);
    let f = body.add_face(
        craft3d_geom::Surface::Plane(craft3d_geom::Plane::new(frame)),
        false,
        vec![l],
    );
    body.add_shell(vec![f]);
    body
}

#[test]
fn what_a_sheet_must_be() {
    let tol = Tolerance::default();
    let good = disc(1.0);
    assert_eq!(check(&good, &tol), []);
    assert_eq!(boundary_loops(&good, ShellId(0)), Some(1));

    // Its circle used a second time, the same way, by a second loop of
    // another face on the same plane: two faces that disagree about
    // which side is up.
    let mut twice = good.clone();
    let again = twice.add_loop(vec![Coedge {
        edge: craft3d_topo::EdgeId(0),
        reversed: false,
    }]);
    let other = twice.add_face(twice.faces[0].surface.clone(), true, vec![again]);
    twice.shells[0].faces.push(other);
    assert!(
        check(&twice, &tol)
            .iter()
            .any(|d| matches!(d, Defect::EdgeUsedTwiceSameWay { .. })),
        "{:?}",
        check(&twice, &tol)
    );

    // Used three times: no sheet has an edge shared by three faces.
    let mut thrice = twice.clone();
    let third = thrice.add_loop(vec![Coedge {
        edge: craft3d_topo::EdgeId(0),
        reversed: true,
    }]);
    let face = thrice.add_face(thrice.faces[0].surface.clone(), false, vec![third]);
    thrice.shells[0].faces.push(face);
    assert!(
        check(&thrice, &tol)
            .iter()
            .any(|d| matches!(d, Defect::EdgeUses { uses: 3, .. }))
    );

    // An edge no face uses.
    let mut stray = good.clone();
    let v = stray.add_vertex(Vec3::new(5.0, 0.0, 0.0));
    let w = stray.add_vertex(Vec3::new(6.0, 0.0, 0.0));
    stray.add_edge(
        Curve3::Line(
            craft3d_geom::Line::through(Vec3::new(5.0, 0.0, 0.0), Vec3::new(6.0, 0.0, 0.0))
                .unwrap(),
        ),
        (0.0, 1.0),
        v,
        w,
    );
    assert!(
        check(&stray, &tol)
            .iter()
            .any(|d| matches!(d, Defect::EdgeUses { uses: 0, .. }))
    );

    // A square with a triangular hole whose corner is a corner of the
    // square: the boundary touches itself there, a pinch.
    let mut pinched = Body::new(BodyKind::Sheet);
    let p = |x: f64, y: f64| Vec3::new(x, y, 0.0);
    let points = [
        p(0.0, 0.0),
        p(2.0, 0.0),
        p(2.0, 2.0),
        p(0.0, 2.0),
        p(0.5, 1.0),
        p(1.0, 0.5),
    ];
    let ids: Vec<_> = points.iter().map(|&q| pinched.add_vertex(q)).collect();
    let edge = |body: &mut Body, a: usize, b: usize| {
        let line = craft3d_geom::Line::through(points[a], points[b]).unwrap();
        body.add_edge(
            Curve3::Line(line),
            (0.0, points[a].distance(points[b])),
            ids[a],
            ids[b],
        )
    };
    let forward = |edge| Coedge {
        edge,
        reversed: false,
    };
    let outer: Vec<Coedge> = [(0, 1), (1, 2), (2, 3), (3, 0)]
        .iter()
        .map(|&(a, b)| forward(edge(&mut pinched, a, b)))
        .collect();
    let hole: Vec<Coedge> = [(0, 4), (4, 5), (5, 0)]
        .iter()
        .map(|&(a, b)| forward(edge(&mut pinched, a, b)))
        .collect();
    let (outer, hole) = (pinched.add_loop(outer), pinched.add_loop(hole));
    let plane = craft3d_geom::Surface::Plane(craft3d_geom::Plane::new(Frame::WORLD));
    let face = pinched.add_face(plane, false, vec![outer, hole]);
    pinched.add_shell(vec![face]);
    assert!(
        check(&pinched, &tol)
            .iter()
            .any(|d| matches!(d, Defect::BoundaryPinch { ends: 4, .. })),
        "{:?}",
        check(&pinched, &tol)
    );
    // Two discs side by side that share no edge are two shells, each with
    // its boundary.
    let two = craft3d_topo::merge(&[&good, &disc(0.5)]).unwrap().0;
    assert_eq!(genus_and_boundary(&two), vec![(0, 1), (0, 1)]);
}

/// The section of a solid by a horizontal plane sheet, both ways round,
/// with the identity of the areas: the sheet's area is what is inside
/// plus what is outside.
fn trimmed(sheet: &Body, solid: &Body, k: f64, inside: f64, what: &str) {
    let tol = tolerance(k);
    let whole = mass_properties(sheet).unwrap().area;
    let kept = solve(sheet, solid, BooleanOp::Intersect, k);
    let left = solve(sheet, solid, BooleanOp::Subtract, k);
    for (result, name) in [(&kept, "inside"), (&left, "outside")] {
        assert_eq!(result.body.kind, BodyKind::Sheet, "{what}: {name}");
        assert_eq!(check(&result.body, &tol), [], "{what}: {name}");
        genus_and_boundary(&result.body);
        closed_but_for_its_boundary(&result.body, k, &format!("{what}: {name}"));
    }
    let (a_in, a_out) = (mass(&kept.body).area, mass(&left.body).area);
    assert_close(a_in, inside * k * k, &format!("{what}: inside"));
    assert!(
        (a_in + a_out - whole).abs() <= 1e-9 * whole,
        "{what}: {a_in} + {a_out} is not {whole}"
    );
}

/// A solid cut by a sheet both ways, with the identity of the volumes:
/// the solid's volume is what is in front plus what is behind.
fn cut(solid: &Body, sheet: &Body, k: f64, front: f64, what: &str) -> [BooleanResult; 2] {
    let tol = tolerance(k);
    let whole = mass(solid).volume;
    let ahead = solve(solid, sheet, BooleanOp::Subtract, k);
    let behind = solve(solid, sheet, BooleanOp::Intersect, k);
    for (result, name) in [(&ahead, "in front"), (&behind, "behind")] {
        assert_eq!(result.body.kind, BodyKind::Solid, "{what}: {name}");
        assert_eq!(check(&result.body, &tol), [], "{what}: {name}");
        let mesh = tessellate_body(
            &result.body,
            &TessParams {
                chordal_tolerance: 1e-3 * k,
                ..TessParams::default()
            },
        )
        .unwrap()
        .weld()
        .unwrap();
        assert!(mesh.is_closed(), "{what}: {name}: the mesh is not closed");
        let v = mass(&result.body).volume;
        assert!(
            (mesh.volume() - v).abs() <= 1e-2 * v,
            "{what}: {name}: the mesh encloses {} for {v}",
            mesh.volume()
        );
    }
    let (v_front, v_back) = (mass(&ahead.body).volume, mass(&behind.body).volume);
    assert_close(v_front, front * k * k * k, &format!("{what}: in front"));
    assert!(
        (v_front + v_back - whole).abs() <= 1e-9 * whole,
        "{what}: {v_front} + {v_back} is not {whole}"
    );
    [ahead, behind]
}

#[test]
fn a_sheet_trimmed_by_solids() {
    for k in SCALES {
        let sheet = rectangle([-10.0, -10.0], [10.0, 10.0], &level(1.0, k), k);
        // A block of 4 × 3 × 2: its section is 12.
        trimmed(
            &sheet,
            &boxed(k, [-2.0, -1.0, 0.0], [2.0, 2.0, 2.0]),
            k,
            12.0,
            "block",
        );
        // A rod of radius 2 standing on the floor.
        trimmed(
            &sheet,
            &rod(k, [1.0, 0.0, 0.0], 2.0, 3.0),
            k,
            4.0 * PI,
            "rod",
        );
        // A ball of radius 2 whose centre is 0.5 below the plane.
        trimmed(
            &sheet,
            &ball(k, [0.0, 0.0, 1.5], 2.0),
            k,
            PI * (4.0 - 0.25),
            "ball",
        );
        // The spline cone, apex at the origin, up the y axis, radius 3 at
        // height 4: cut by the plane y = 1 its section is a disc of 0.75.
        let cone = solids::spline_cone(
            &Frame::WORLD.at(Vec3::new(0.0, 0.0, 0.0)).unwrap(),
            3.0 * k,
            4.0 * k,
        )
        .unwrap()
        .body;
        let upright = Frame::new(Vec3::new(0.0, k, 0.0), Vec3::Y, Vec3::X).unwrap();
        let wall = rectangle([-10.0, -10.0], [10.0, 10.0], &upright, k);
        trimmed(&wall, &cone, k, PI * 0.75 * 0.75, "spline cone");
        // A sheet bounded by a spline: the arch, standing in the plane
        // x = 0 (sketch x along world y, sketch y up), trimmed by a block
        // that holds the part of it below z = 1.
        let upright = Frame::new(Vec3::ZERO, Vec3::X, Vec3::Y).unwrap();
        let arch = filled(&plate_and_arch()[5..], &upright, k);
        trimmed(
            &arch,
            &boxed(k, [-1.0, -1.0, -1.0], [1.0, 9.0, 1.0]),
            k,
            arch_area() - arch_above(1.0),
            "spline sheet",
        );
    }
}

/// The arch of `plate_and_arch`: the quadratic Bézier (3, 0), (5, 4),
/// (7, 0) is x = 3 + 4t, y = 8t(1 − t), so y = 2 − (x − 5)²/2 over x in
/// [3, 7], under which lie 16/3 (two thirds of its control triangle).
fn arch_area() -> f64 {
    16.0 / 3.0
}

/// The area of the arch above the line y = h: ∫ (2 − u²/2 − h) du over
/// |u| ≤ √(2(2 − h)).
fn arch_above(h: f64) -> f64 {
    let w = (2.0 * (2.0 - h)).sqrt();
    2.0 * ((2.0 - h) * w - w * w * w / 6.0)
}

#[test]
fn solids_cut_by_a_sheet() {
    for k in SCALES {
        let sheet = rectangle([-10.0, -10.0], [10.0, 10.0], &level(1.5, k), k);
        // A block of 4 × 3 × 2: 1.5 below the sheet, 0.5 above.
        let [ahead, behind] = cut(
            &boxed(k, [-2.0, -1.0, 0.0], [2.0, 2.0, 2.0]),
            &sheet,
            k,
            6.0,
            "block",
        );
        // Each piece has a cap in the plane: in front its outward normal
        // points down, behind up.
        let caps = |r: &BooleanResult| {
            r.faces
                .iter()
                .filter(|s| s.operand == craft3d_kernel::boolean::Operand::B)
                .count()
        };
        assert_eq!((caps(&ahead), caps(&behind)), (1, 1));
        assert_mass(&ahead, 6.0, 2.0 * 12.0 + 14.0 * 0.5, k);
        // A rod of radius 2 and height 3: 1.5 of it in front.
        cut(
            &rod(k, [1.0, 0.0, 0.0], 2.0, 3.0),
            &sheet,
            k,
            6.0 * PI,
            "rod",
        );
        // A ball of radius 2 whose centre is 0.5 below the sheet: the cap
        // above it is 1.5 high.
        let h: f64 = 1.5;
        cut(
            &ball(k, [0.0, 0.0, 1.0], 2.0),
            &sheet,
            k,
            PI * h * h * (6.0 - h) / 3.0,
            "ball",
        );
        // The spline cone (apex at the origin, up the y axis, radius 3 at
        // height 4) cut at y = 1 by a sheet facing down the axis: in
        // front of it the tip, a cone of radius 0.75 and height 1.
        let cone = solids::spline_cone(&Frame::WORLD, 3.0 * k, 4.0 * k)
            .unwrap()
            .body;
        let facing_down = Frame::new(Vec3::new(0.0, k, 0.0), -Vec3::Y, Vec3::X).unwrap();
        let wall = rectangle([-10.0, -10.0], [10.0, 10.0], &facing_down, k);
        cut(&cone, &wall, k, PI * 0.75 * 0.75 / 3.0, "spline cone");
        // A tilted sheet through a block: the volumes still add up (the
        // closed form: the plane z = 1 + 0.25 x through the block from x =
        // −2 to 2 leaves 4 · 3 · (2 − 1) above it on average).
        let tilted = Frame::new(
            Vec3::new(0.0, 0.0, k),
            Vec3::new(-0.25, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 0.25),
        )
        .unwrap();
        let slope = rectangle([-10.0, -10.0], [10.0, 10.0], &tilted, k);
        cut(
            &boxed(k, [-2.0, -1.0, 0.0], [2.0, 2.0, 2.0]),
            &slope,
            k,
            12.0,
            "tilted",
        );
        // A ring lying flat about z = 1.5, halved: each half is π² R r².
        let [half, _] = cut(
            &ring(k, [0.0, 0.0, 1.5], 3.0, 1.0),
            &sheet,
            k,
            PI * PI * 3.0,
            "ring",
        );
        // Its cap is an annulus: two loops.
        let cap = half
            .faces
            .iter()
            .position(|s| s.operand == craft3d_kernel::boolean::Operand::B)
            .unwrap();
        assert_eq!(half.body.faces[cap].loops.len(), 2);
        // A block of 10 with a cavity of 4 in its middle, cut through the
        // cavity at 1.5 above the block's middle: the cavity opens into
        // the cap, which gets a hole.
        let hollow = solve(
            &boxed(k, [-5.0, -5.0, -3.5], [5.0, 5.0, 6.5]),
            &boxed(k, [-2.0, -2.0, -0.5], [2.0, 2.0, 3.5]),
            BooleanOp::Subtract,
            k,
        )
        .body;
        let [ahead, behind] = cut(&hollow, &sheet, k, 100.0 * 5.0 - 16.0 * 2.0, "hollow");
        for r in [&ahead, &behind] {
            assert_eq!(r.body.shells.len(), 1, "the cavity is opened");
        }
        // A cavity the sheet does not reach: its side is not told.
        let low = solve(
            &boxed(k, [-5.0, -5.0, -3.5], [5.0, 5.0, 6.5]),
            &boxed(k, [-2.0, -2.0, -2.5], [2.0, 2.0, 0.5]),
            BooleanOp::Subtract,
            k,
        )
        .body;
        refused(
            boolean(&low, &sheet, BooleanOp::Subtract, &tolerance(k)),
            "does not cross",
            "low cavity",
        );
    }
}

fn refused(result: Result<BooleanResult, KernelError>, wanted: &str, what: &str) {
    match result {
        Err(KernelError::Unsupported { reason, .. }) => {
            assert!(reason.contains(wanted), "{what}: refused as `{reason}`")
        }
        other => panic!(
            "{what}: expected a refusal ({wanted}), got {:?}",
            other.map(|r| r.body.faces.len())
        ),
    }
}

#[test]
fn what_a_sheet_cannot_do_is_refused() {
    for k in SCALES {
        let tol = tolerance(k);
        let block = boxed(k, [-2.0, -1.0, 0.0], [2.0, 2.0, 2.0]);
        let big = rectangle([-10.0, -10.0], [10.0, 10.0], &level(1.0, k), k);
        // A sheet that ends inside the block.
        let small = rectangle([-1.0, -0.5], [1.0, 0.5], &level(1.0, k), k);
        refused(
            boolean(&block, &small, BooleanOp::Subtract, &tol),
            "ends inside",
            "small",
        );
        // A sheet that reaches in from one side and ends inside.
        let half = rectangle([0.0, -10.0], [10.0, 10.0], &level(1.0, k), k);
        refused(
            boolean(&block, &half, BooleanOp::Intersect, &tol),
            "ends inside",
            "half",
        );
        // Missing the block altogether.
        let away = rectangle([-10.0, -10.0], [10.0, 10.0], &level(5.0, k), k);
        refused(
            boolean(&block, &away, BooleanOp::Subtract, &tol),
            "does not pass",
            "away",
        );
        // Trimmed by a block it misses: nothing inside, all of it outside.
        assert!(matches!(
            boolean(&away, &block, BooleanOp::Intersect, &tol),
            Err(KernelError::EmptyResult { .. })
        ));
        let outside = solve(&away, &block, BooleanOp::Subtract, k);
        assert_close(mass(&outside.body).area, 400.0 * k * k, "all outside");
        // On a face of the block.
        let on_top = rectangle([-10.0, -10.0], [10.0, 10.0], &level(2.0, k), k);
        for op in [BooleanOp::Subtract, BooleanOp::Intersect] {
            refused(
                boolean(&block, &on_top, op, &tol),
                "lies on a face",
                "on top",
            );
            refused(
                boolean(&on_top, &block, op, &tol),
                "lies on a face",
                "on top, trimmed",
            );
        }
        // Union with a sheet, two sheets.
        refused(
            boolean(&block, &big, BooleanOp::Union, &tol),
            "union of a sheet",
            "union",
        );
        refused(
            boolean(&big, &block, BooleanOp::Union, &tol),
            "union of a sheet",
            "union",
        );
        for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
            refused(boolean(&big, &small, op, &tol), "two sheets", "two sheets");
        }
        // A solid of two lumps of which the sheet crosses one.
        let two = craft3d_topo::merge(&[&block, &boxed(k, [5.0, 5.0, 5.0], [6.0, 6.0, 6.0])])
            .unwrap()
            .0;
        refused(
            boolean(&two, &big, BooleanOp::Subtract, &tol),
            "does not cross",
            "two lumps",
        );
        // A sheet has no inside to classify points by.
        assert!(craft3d_kernel::boolean::contains_points(&big, &[Vec3::ZERO], &tol).is_err());
        assert!(craft3d_kernel::boolean::interference(&big, &block, &tol).is_err());
    }
}
