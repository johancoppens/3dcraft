//! Sheet bodies (the boolean checklist's item 5): open shells made by
//! `extrude::fill`, held to closed-form areas, Euler–Poincaré with their
//! boundary loops and meshes that are closed except along the boundary;
//! and booleans with a sheet. A sheet trimmed by a solid (intersect keeps
//! what is inside, subtract what is outside) is held to closed-form areas
//! and to `area(S ∩ A) + area(S ∖ A) = area(S)`; a solid cut by a sheet
//! (subtract keeps the side in front of the sheet, intersect the side
//! behind) to closed-form volumes and to `vol(front) + vol(behind) =
//! vol(A)`. A piece of the sheet lying on a face of the solid counts as
//! inside the solid (decided 2026-10-18): the identities hold with it, the
//! trim keeps it for intersect, and the cut keeps the solid's face there
//! as two solids keep a face they share. Analytic and freeform operands
//! (a ball, a cylinder, a hemisphere, the spline cone of the test kit, a
//! sheet bounded by a spline, a sheet that is a freeform face), at 1e-3,
//! 1 and 1e6, and turned off-axis.

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
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
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

    // A closed shell is a solid's: marked as a sheet it is refused (a
    // sheet has no inside; this one would), and so is a boolean with it.
    let mut closed = boxed(1.0, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    closed.kind = BodyKind::Sheet;
    assert_eq!(
        check(&closed, &tol),
        [Defect::SheetClosed { shell: ShellId(0) }]
    );
    let other = boxed(1.0, [0.5, 0.5, 0.5], [2.0, 2.0, 2.0]);
    for op in [BooleanOp::Subtract, BooleanOp::Intersect] {
        assert!(matches!(
            boolean(&closed, &other, op, &tol),
            Err(KernelError::InvalidBody { .. })
        ));
    }
    // So is a whole sphere, which has no edge to tell it by: trimmed by a
    // ball it would otherwise be kept whole, a closed "sheet".
    let mut sphere = solids::sphere(&Frame::WORLD, 1.0).unwrap().body;
    sphere.kind = BodyKind::Sheet;
    assert_eq!(
        check(&sphere, &tol),
        [Defect::SheetClosed { shell: ShellId(0) }]
    );
    let ball = solids::sphere(&Frame::WORLD, 2.0).unwrap().body;
    for op in [BooleanOp::Subtract, BooleanOp::Intersect] {
        for (a, b) in [(&sphere, &ball), (&ball, &sphere)] {
            assert!(matches!(
                boolean(a, b, op, &tol),
                Err(KernelError::InvalidBody { .. })
            ));
        }
    }
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

/// One face of `body` as a sheet of its own.
fn face_as_sheet(body: &Body, face: usize) -> Body {
    faces_as_sheet(body, &[face])
}

/// Faces of `body` as one sheet, sharing the edges they share there.
fn faces_as_sheet(body: &Body, faces: &[usize]) -> Body {
    use craft3d_topo::VertexId;
    let mut out = Body::new(BodyKind::Sheet);
    let mut vertices: BTreeMap<u32, VertexId> = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let mut made = Vec::new();
    for &face in faces {
        let source = &body.faces[face];
        let mut loops = Vec::new();
        for &l in &source.loops {
            let mut coedges = Vec::new();
            for c in &body.loops[l.0 as usize].coedges {
                let edge = *edges.entry(c.edge.0).or_insert_with(|| {
                    let e = &body.edges[c.edge.0 as usize];
                    let mut vertex = |v: VertexId| {
                        *vertices
                            .entry(v.0)
                            .or_insert_with(|| out.add_vertex(body.vertices[v.0 as usize].point))
                    };
                    let (start, end) = (vertex(e.start), vertex(e.end));
                    out.add_edge(e.curve.clone(), e.range, start, end)
                });
                coedges.push(Coedge {
                    edge,
                    reversed: c.reversed,
                });
            }
            loops.push(out.add_loop(coedges));
        }
        made.push(out.add_face(source.surface.clone(), source.reversed, loops));
    }
    out.add_shell(made);
    out
}

/// A piece of the sheet lying on a face of the solid counts as inside
/// the solid (decided 2026-10-18): a sheet in the plane of a face of the
/// solid, reaching across the part of the solid that rises above it, is
/// trimmed to the face plus the section, whichever way it looks, and
/// cuts the solid there into what is above and the rest, the identities
/// of areas and volumes holding. A step (plane faces), a round boss, a
/// hemisphere and the spline cone (its side a freeform face) on a plate,
/// and a sheet that is the cone's own side, on the cone of a solid that
/// swallows half of it. At three scales, as built and turned off-axis.
#[test]
fn a_piece_of_the_sheet_on_a_face_of_the_solid_is_inside() {
    let turn = Transform::rotation(Vec3::new(0.3, -0.7, 0.5), 0.9).unwrap();
    for k in SCALES {
        for turned in [false, true] {
            let what = |name: &str| {
                if turned {
                    format!("{name}, turned")
                } else {
                    name.to_string()
                }
            };
            let place = |body: &Body| -> Body {
                if turned {
                    body.transformed(&turn).unwrap()
                } else {
                    body.clone()
                }
            };
            // The plate is 4 × 3 × 1 from (−2, −1, 0): 12 on top of it.
            let plate = boxed(k, [-2.0, -1.0, 0.0], [2.0, 2.0, 1.0]);
            let wide = rectangle([-10.0, -10.0], [10.0, 10.0], &level(1.0, k), k);
            let facing_down = Frame::new(Vec3::new(0.0, 0.0, k), -Vec3::Z, Vec3::X).unwrap();
            let wide_down = rectangle([-10.0, -10.0], [10.0, 10.0], &facing_down, k);
            // What rises above the plate: a block of 4 × 1.5 × 1 along its
            // back (a step), a boss of radius 0.5, a hemisphere of radius
            // 1 and the spline cone of radius 1 and height 2 (its apex
            // up), each standing on the plate's top.
            let step = solve(
                &plate,
                &boxed(k, [-2.0, 0.5, 1.0], [2.0, 2.0, 2.0]),
                BooleanOp::Union,
                k,
            )
            .body;
            let bossed = solve(
                &plate,
                &rod(k, [0.0, 0.5, 1.0], 0.5, 1.0),
                BooleanOp::Union,
                k,
            )
            .body;
            let dome = solids::hemisphere(&Frame::WORLD.at(Vec3::new(0.0, 0.5 * k, k)).unwrap(), k)
                .unwrap()
                .body;
            let domed = solve(&plate, &dome, BooleanOp::Union, k).body;
            // The cone's frame: its y axis (the axis of the cone, apex at
            // the origin) is the frame's z × x = y × x, down.
            let cone = solids::spline_cone(
                &Frame::new(Vec3::new(0.0, 0.5 * k, 3.0 * k), Vec3::Y, Vec3::X).unwrap(),
                k,
                2.0 * k,
            )
            .unwrap()
            .body;
            let coned = solve(&plate, &cone, BooleanOp::Union, k).body;
            for (solid, above, name) in [
                (&step, 6.0, "step"),
                (&bossed, PI * 0.25, "boss"),
                (&domed, 2.0 * PI / 3.0, "dome"),
                (&coned, 2.0 * PI / 3.0, "spline cone"),
            ] {
                let solid = place(solid);
                assert_close(mass(&solid).volume, (12.0 + above) * k * k * k, name);
                for (sheet, front, way) in [(&wide, above, "up"), (&wide_down, 12.0, "down")] {
                    let sheet = place(sheet);
                    let what = what(&format!("{name}, facing {way}"));
                    // Trimmed: the plate's top plus the section, 12.
                    trimmed(&sheet, &solid, k, 12.0, &what);
                    // Cut: subtract keeps the side the sheet faces.
                    let [ahead, behind] = cut(&solid, &sheet, k, front, &what);
                    // One cap at most on each piece (the plate's top, on
                    // the piece that holds the plate, is the solid's own
                    // face, merged with the cap's where both look up).
                    for (r, piece) in [(&ahead, "ahead"), (&behind, "behind")] {
                        let caps = r
                            .faces
                            .iter()
                            .filter(|s| s.operand == craft3d_kernel::boolean::Operand::B)
                            .count();
                        assert!(caps <= 1, "{what}: {piece}: {caps} caps");
                    }
                }
            }
            // The cone's side is two half-turns about y = 0.5; the half
            // towards +y as a sheet, trimmed by a solid that swallows the
            // cone up to y = 0.8 (the other half and both seams with it):
            // the part on the solid's own cone face counts as inside,
            // with the part within the solid: all of it, half of
            // π r √(r² + h²); nothing is outside.
            let side = (0..cone.faces.len())
                .filter(|&f| {
                    matches!(
                        cone.faces[f].surface,
                        craft3d_geom::Surface::Nurbs(_) | craft3d_geom::Surface::Bezier(_)
                    )
                })
                .max_by(|&f, &g| {
                    let towards = |f: usize| {
                        let mesh = tessellate_body(
                            &face_as_sheet(&cone, f),
                            &TessParams {
                                chordal_tolerance: 1e-2 * k,
                                ..TessParams::default()
                            },
                        )
                        .unwrap();
                        mesh.positions.iter().map(|p| p.y).sum::<f64>()
                    };
                    towards(f).total_cmp(&towards(g))
                })
                .unwrap();
            let skin = place(&face_as_sheet(&cone, side));
            let swallowed = solve(
                &coned,
                &boxed(k, [-3.0, -1.5, -0.5], [3.0, 0.8, 4.0]),
                BooleanOp::Union,
                k,
            )
            .body;
            let swallowed = place(&swallowed);
            let kept = solve(&skin, &swallowed, BooleanOp::Intersect, k);
            let tol = tolerance(k);
            assert_eq!(check(&kept.body, &tol), [], "{}", what("skin"));
            closed_but_for_its_boundary(&kept.body, k, &what("skin"));
            assert_close(
                mass(&kept.body).area,
                0.5 * PI * 5.0_f64.sqrt() * k * k,
                &what("skin, inside"),
            );
            assert_empty(&skin, &swallowed, BooleanOp::Subtract, k);
            // A sheet that ends on the plate's top, around the boss: the
            // top beyond it is reached by no cut, so the cut is refused;
            // trimmed, the disc is inside altogether.
            let disc = filled(
                &[SketchCurve {
                    id: 1,
                    curve: Curve2::Circle {
                        center: [0.0, 0.5],
                        radius: 1.2,
                    },
                }],
                &level(1.0, k),
                k,
            );
            let (disc, bossed) = (place(&disc), place(&bossed));
            for op in [BooleanOp::Subtract, BooleanOp::Intersect] {
                refused(
                    boolean(&bossed, &disc, op, &tol),
                    "does not cross",
                    &what("disc"),
                );
            }
            let inside = solve(&disc, &bossed, BooleanOp::Intersect, k);
            assert_eq!(check(&inside.body, &tol), [], "{}", what("disc"));
            closed_but_for_its_boundary(&inside.body, k, &what("disc"));
            assert_close(
                mass(&inside.body).area,
                PI * 1.44 * k * k,
                &what("disc, inside"),
            );
            assert_empty(&disc, &bossed, BooleanOp::Subtract, k);
        }
    }
}

/// The area of the arch `arch` holds above y = 0: its rim is the
/// quadratic Bézier from (1, −1) over (2, 6) to (3, −1), `x = 1 + 2t`,
/// `y = 2.5 − 14u²` with `u = t − ½`, so above y = 0 for `|u| ≤ a`,
/// `a = √(5/28)`, and the area there is `∫ y dx = 2 ∫ (2.5 − 14u²) du`
/// over `[−a, a]`, `10a − (56/3) a³`.
fn arch_above_zero() -> f64 {
    let a = (5.0_f64 / 28.0).sqrt();
    10.0 * a - 56.0 / 3.0 * a * a * a
}

/// A sheet whose rim enters a face of the solid from outside it and
/// leaves it again: the face is divided along the rim from where it
/// enters to where it leaves (a boundary edge of the sheet has no second
/// face to imprint it, and the rim and the face's edges lie on one
/// surface, so no surface crossing finds where they cross). Before this
/// was found, the face stayed whole and one sample point said for all of
/// it whether it was on the sheet: cuts were refused as passing through
/// nothing or as tangent, or broke the result (an edge used once), or
/// were right by luck. (Trims divide the sheet by the solid's other
/// faces and were right; they are held to the same closed forms.) A
/// block 4 × 3 × 1 and sheets in its top's plane over part of the top,
/// both mirror images, a rim slanted across the top and a rim that is a
/// spline arch crossing one edge twice; the block with a boss; a sheet
/// folded along the top, its other face through the block; a half tube
/// on the side of a rod (a cylinder: rim lines that cross the rod's
/// bottom circle); and a piece of the side of the spline cone on the
/// cone's frustum (a freeform face, a rim that crosses the frustum's
/// top). At three scales, as built and turned off-axis, sheets facing up
/// and down.
#[test]
fn a_sheet_partly_on_a_face_of_the_solid_divides_it_where_its_rim_crosses() {
    let turn = Transform::rotation(Vec3::new(0.3, -0.7, 0.5), 0.9).unwrap();
    for k in SCALES {
        let tol = tolerance(k);
        for turned in [false, true] {
            let what = |name: &str| {
                if turned {
                    format!("{name}, turned, at {k}")
                } else {
                    format!("{name}, at {k}")
                }
            };
            let place = |body: &Body| -> Body {
                if turned {
                    body.transformed(&turn).unwrap()
                } else {
                    body.clone()
                }
            };
            let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 1.0]);
            let bossed = solve(
                &block,
                &rod(k, [3.0, 1.5, 1.0], 0.5, 1.0),
                BooleanOp::Union,
                k,
            )
            .body;
            let up = level(1.0, k);
            let down = Frame::new(Vec3::new(0.0, 0.0, k), -Vec3::Z, Vec3::X).unwrap();
            // The sheets in the top's plane, each facing up and down
            // (facing down, the plane's x is the world's and its y the
            // world's −y: the rectangle's y range is turned over).
            let both = |lo: [f64; 2], hi: [f64; 2]| {
                [
                    rectangle(lo, hi, &up, k),
                    rectangle([lo[0], -hi[1]], [hi[0], -lo[1]], &down, k),
                ]
            };
            let slanted = |frame: &Frame, flip: f64| {
                let p = |x: f64, y: f64| [x, flip * y];
                let corners = [p(2.0, -1.0), p(9.0, -1.0), p(9.0, 4.0), p(1.0, 4.0)];
                let curves: Vec<SketchCurve> = (0..4)
                    .map(|i| line(i as u64 + 1, corners[i], corners[(i + 1) % 4]))
                    .collect();
                filled(&curves, frame, k)
            };
            let arch = |frame: &Frame, flip: f64| {
                filled(
                    &[
                        SketchCurve {
                            id: 1,
                            curve: Curve2::Spline(Spline2 {
                                degree: 2,
                                knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                                control: vec![[1.0, -flip], [2.0, 6.0 * flip], [3.0, -flip]],
                            }),
                        },
                        line(2, [3.0, -flip], [1.0, -flip]),
                    ],
                    frame,
                    k,
                )
            };
            let mut cases: Vec<(Body, &Body, f64, &str)> = Vec::new();
            for (lo, hi, inside, name) in [
                ([-4.0, -1.0], [2.0, 4.0], 6.0, "left half"),
                ([2.0, -1.0], [8.0, 4.0], 6.0, "right half"),
                ([0.5, -1.0], [8.0, 4.0], 10.5, "from x = 0.5"),
                ([2.0, 0.5], [8.0, 2.5], 4.0, "a strip from inside"),
            ] {
                for sheet in both(lo, hi) {
                    cases.push((sheet, &block, inside, name));
                }
            }
            // The rim from (2, −1) to (1, 4), x = 2 − (y + 1)/5, crosses
            // the top from (1.8, 0) to (1.2, 3): inside, ∫ (4 − x) dy.
            cases.push((slanted(&up, 1.0), &block, 7.5, "slanted rim"));
            cases.push((slanted(&down, -1.0), &block, 7.5, "slanted rim"));
            cases.push((arch(&up, 1.0), &block, arch_above_zero(), "spline arch"));
            cases.push((arch(&down, -1.0), &block, arch_above_zero(), "spline arch"));
            // With the boss: beside it, and over it (the top less the
            // boss's foot, and the section of the boss, 6 in all).
            for (lo, hi, name) in [
                ([-4.0, -1.0], [2.0, 4.0], "left half, boss"),
                ([2.0, -1.0], [8.0, 4.0], "right half, over the boss"),
            ] {
                for sheet in both(lo, hi) {
                    cases.push((sheet, &bossed, 6.0, name));
                }
            }
            for (sheet, solid, inside, name) in &cases {
                let (sheet, solid) = (place(sheet), place(solid));
                trimmed(&sheet, &solid, k, *inside, &what(name));
            }
            // Cut by them: the sheet reaches no further than the top, so
            // all of the block is behind a sheet facing up and in front
            // of one facing down, as for a sheet over the whole top;
            // over the boss the boss is on the other side.
            // (Beside the boss, the sheet crosses none of it: the boss is
            // on the block's side, whole.)
            for (lo, hi, solid, volume, name) in [
                ([-4.0, -1.0], [2.0, 4.0], &block, 12.0, "left half"),
                ([2.0, -1.0], [8.0, 4.0], &block, 12.0, "right half"),
                ([0.5, -1.0], [8.0, 4.0], &block, 12.0, "from x = 0.5"),
                ([2.0, 0.5], [8.0, 2.5], &block, 12.0, "a strip from inside"),
                (
                    [-4.0, -1.0],
                    [2.0, 4.0],
                    &bossed,
                    12.0 + 0.25 * PI,
                    "left half, boss",
                ),
            ] {
                let [facing_up, facing_down] = both(lo, hi);
                let (solid, facing_up, facing_down) =
                    (place(solid), place(&facing_up), place(&facing_down));
                let volume = volume * k * k * k;
                assert_empty(&solid, &facing_up, BooleanOp::Subtract, k);
                let behind = solve(&solid, &facing_up, BooleanOp::Intersect, k);
                assert_eq!(check(&behind.body, &tol), [], "{}", what(name));
                assert_close(mass(&behind.body).volume, volume, &what(name));
                assert_empty(&solid, &facing_down, BooleanOp::Intersect, k);
                let ahead = solve(&solid, &facing_down, BooleanOp::Subtract, k);
                assert_eq!(check(&ahead.body, &tol), [], "{}", what(name));
                assert_close(mass(&ahead.body).volume, volume, &what(name));
            }
            let [over_up, over_down] = both([2.0, -1.0], [8.0, 4.0]);
            let bossed_here = place(&bossed);
            cut(
                &bossed_here,
                &place(&over_up),
                k,
                0.25 * PI,
                &what("over the boss, up"),
            );
            cut(
                &bossed_here,
                &place(&over_down),
                k,
                12.0,
                &what("over the boss, down"),
            );

            // A sheet folded along x = 2, z = 1: the top (facing up) and
            // the side x = 2 (facing +x) of the block from (−2, −1, −1)
            // to (2, 4, 1). Its top lies on the block's top over
            // [0, 2] × [0, 3], its side passes through the block. Trimmed:
            // 6 on the top and 3 of the side inside, of 30. Cut: the side
            // divides the block in two halves of 6, and along the fold
            // the block's top beyond the sheet continues the sheet's top
            // (told nothing there, but by the side), not a tangent cut.
            let folder = boxed(k, [-2.0, -1.0, -1.0], [2.0, 4.0, 1.0]);
            let where_ = |f: usize| {
                let face = &folder.faces[f];
                let mut sum = Vec3::ZERO;
                let mut n = 0.0;
                for &l in &face.loops {
                    for c in &folder.loops[l.0 as usize].coedges {
                        let e = &folder.edges[c.edge.0 as usize];
                        sum += folder.vertices[e.start.0 as usize].point;
                        n += 1.0;
                    }
                }
                sum * (1.0 / n)
            };
            let top = (0..folder.faces.len())
                .find(|&f| (where_(f).z - k).abs() < 1e-9 * k)
                .unwrap();
            let side = (0..folder.faces.len())
                .find(|&f| (where_(f).x - 2.0 * k).abs() < 1e-9 * k)
                .unwrap();
            let folded = place(&faces_as_sheet(&folder, &[top, side]));
            assert_eq!(check(&folded, &tol), [], "{}", what("folded"));
            trimmed(&folded, &place(&block), k, 9.0, &what("folded"));
            cut(&place(&block), &folded, k, 6.0, &what("folded"));

            // A half tube (radius 0.5, z from −0.5 to 0.5, x ≥ 0) on the
            // side of a rod (radius 0.5, z from 0 to 1): its rim lines at
            // x = 0 cross the rod's bottom circle. Inside, the upper half
            // of it: π r h = π/4; outside as much.
            let long = solve(
                &rod(k, [0.0, 0.0, -0.5], 0.5, 1.0),
                &boxed(k, [0.0, -1.0, -1.0], [1.0, 1.0, 1.0]),
                BooleanOp::Intersect,
                k,
            )
            .body;
            let wall = (0..long.faces.len())
                .find(|&f| matches!(long.faces[f].surface, craft3d_geom::Surface::Cylinder(_)))
                .unwrap();
            let half = face_as_sheet(&long, wall);
            assert_eq!(check(&half, &tol), []);
            assert_close(mass(&half).area, 0.5 * PI * k * k, "the half tube");
            let short = rod(k, [0.0, 0.0, 0.0], 0.5, 1.0);
            trimmed(
                &place(&half),
                &place(&short),
                k,
                0.25 * PI,
                &what("half tube"),
            );

            // The side of the spline cone (radius 1, height 2, its apex
            // at z = 3 and its base at z = 1) beyond y = 0.3, on the cone
            // cut off at z = 2: its rim, a hyperbola up to z = 2.4,
            // crosses the frustum's top. Inside: the cone's area over
            // its plan is √5 times the plan's, and the plan is the
            // segment beyond y = 0.3 of the base's circle less that of
            // the top's (radius ½). The rim and the frustum's top are
            // sections of a freeform surface, within the tolerance of
            // the exact curves: the area to within the tolerance times
            // their length (the identity of the areas is exact, both
            // sides share the curves).
            let cone = solids::spline_cone(
                &Frame::new(Vec3::new(0.0, 0.0, 3.0 * k), Vec3::Y, Vec3::X).unwrap(),
                k,
                2.0 * k,
            )
            .unwrap()
            .body;
            let side = (0..cone.faces.len())
                .filter(|&f| {
                    matches!(
                        cone.faces[f].surface,
                        craft3d_geom::Surface::Nurbs(_) | craft3d_geom::Surface::Bezier(_)
                    )
                })
                .max_by(|&f, &g| {
                    let towards = |f: usize| {
                        let mesh = tessellate_body(
                            &face_as_sheet(&cone, f),
                            &TessParams {
                                chordal_tolerance: 1e-2 * k,
                                ..TessParams::default()
                            },
                        )
                        .unwrap();
                        mesh.positions.iter().map(|p| p.y).sum::<f64>()
                    };
                    towards(f).total_cmp(&towards(g))
                })
                .unwrap();
            // (Beyond y = 0.3, clear of the other half, which meets it
            // tangentially along the seams.)
            let skin = place(
                &solve(
                    &face_as_sheet(&cone, side),
                    &boxed(k, [-2.0, 0.3, 0.0], [2.0, 2.0, 4.0]),
                    BooleanOp::Intersect,
                    k,
                )
                .body,
            );
            let frustum = place(
                &solve(
                    &cone,
                    &boxed(k, [-2.0, 0.2, 0.0], [2.0, 2.0, 2.0]),
                    BooleanOp::Intersect,
                    k,
                )
                .body,
            );
            let whole = mass(&skin).area;
            let kept = solve(&skin, &frustum, BooleanOp::Intersect, k);
            let left = solve(&skin, &frustum, BooleanOp::Subtract, k);
            for (result, name) in [(&kept, "inside"), (&left, "outside")] {
                assert_eq!(result.body.kind, BodyKind::Sheet);
                assert_eq!(check(&result.body, &tol), [], "{}", what(name));
                genus_and_boundary(&result.body);
                closed_but_for_its_boundary(&result.body, k, &what(&format!("skin {name}")));
            }
            let (a_in, a_out) = (mass(&kept.body).area, mass(&left.body).area);
            let segment = |r: f64, d: f64| r * r * (d / r).acos() - d * (r * r - d * d).sqrt();
            let exact = 5.0_f64.sqrt() * (segment(1.0, 0.3) - segment(0.5, 0.3)) * k * k;
            assert!(
                (a_in - exact).abs() <= tol.linear * PI * k,
                "{}: {a_in} for {exact}",
                what("skin inside")
            );
            assert!(
                (a_in + a_out - whole).abs() <= tol.linear * PI * k,
                "{}: {a_in} + {a_out} is not {whole}",
                what("skin")
            );
            // Cut by them (each facing out of its solid, and reaching
            // across none): all of the solid is behind.
            for (solid, sheet, name) in [
                (place(&short), place(&half), "half tube"),
                (frustum, skin, "skin"),
            ] {
                let whole = mass(&solid).volume;
                assert_empty(&solid, &sheet, BooleanOp::Subtract, k);
                let behind = solve(&solid, &sheet, BooleanOp::Intersect, k);
                assert_eq!(check(&behind.body, &tol), [], "{}", what(name));
                assert_close(mass(&behind.body).volume, whole, &what(name));
            }
        }
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
        // On a face of the block: the piece on it counts as inside
        // (2026-10-18), so trimmed by the block the sheet keeps the top
        // for intersect and the rest for subtract; as a tool facing up
        // it has the whole block behind it (nothing in front).
        let on_top = rectangle([-10.0, -10.0], [10.0, 10.0], &level(2.0, k), k);
        assert_empty(&block, &on_top, BooleanOp::Subtract, k);
        let behind = solve(&block, &on_top, BooleanOp::Intersect, k);
        assert_mass(&behind, 24.0, 52.0, k);
        assert_eq!(behind.body.faces.len(), 6, "on top: the block as it was");
        trimmed(&on_top, &block, k, 12.0, "on top, trimmed");
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
        // A ring cut once, by a half-sheet whose edge lies in its hole:
        // the material on the two sides of the cut is one.
        let flat = ring(k, [0.0, 0.0, 0.0], 3.0, 1.0);
        let half_plane = Frame::new(Vec3::ZERO, Vec3::Y, Vec3::X).unwrap();
        let once = rectangle([0.0, -5.0], [10.0, 5.0], &half_plane, k);
        for op in [BooleanOp::Subtract, BooleanOp::Intersect] {
            refused(
                boolean(&flat, &once, op, &tol),
                "does not divide",
                "ring cut once",
            );
        }
        // A sheet has no inside to classify points by.
        assert!(craft3d_kernel::boolean::contains_points(&big, &[Vec3::ZERO], &tol).is_err());
        assert!(craft3d_kernel::boolean::interference(&big, &block, &tol).is_err());
    }
}

/// A trim that would leave a sheet touching itself in a point is refused
/// as zero thickness, as a solid touching itself is: a square with a round
/// hole less a block whose side is tangent to the hole, and a plane
/// tangent to the inside of a ring's tube less the ring (its section a
/// figure of eight).
#[test]
fn a_sheet_that_would_touch_itself_is_refused() {
    for k in SCALES {
        let tol = tolerance(k);
        let holed = filled(&plate_and_arch()[..5], &level(1.0, k), k);
        // The block's side x = 1 touches the hole (radius 1) at (1, 0).
        let block = boxed(k, [1.0, -5.0, 0.0], [5.0, 5.0, 2.0]);
        // A ring about z (R 3, r 1); the plane x = 2 touches its tube's
        // inside at (2, 0, 0).
        let ring = ring(k, [0.0, 0.0, 0.0], 3.0, 1.0);
        let side = Frame::new(Vec3::new(2.0 * k, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap();
        let wall = rectangle([-10.0, -10.0], [10.0, 10.0], &side, k);
        // (What the block holds of the square touches the hole in one
        // point of its edge only: a plain rectangle, which is made.)
        let inside = solve(&holed, &block, BooleanOp::Intersect, k);
        assert_close(mass(&inside.body).area, 4.0 * k * k, "inside the block");
        for (sheet, solid, what, ops) in [
            (&holed, &block, "tangent block", &[BooleanOp::Subtract][..]),
            (
                &wall,
                &ring,
                "ring",
                &[BooleanOp::Subtract, BooleanOp::Intersect][..],
            ),
        ] {
            for &op in ops {
                match boolean(sheet, solid, op, &tol) {
                    Err(KernelError::ZeroThickness { .. }) => {}
                    Err(other) => panic!("{what} {op:?} at {k}: {other}"),
                    Ok(r) => panic!(
                        "{what} {op:?} at {k}: a sheet that touches itself was made ({:?})",
                        check(&r.body, &tol)
                    ),
                }
            }
        }
    }
}

/// The check of a cut sees a result that kept the wrong side (its cut
/// face looks the wrong way) and a result that reaches outside the solid.
#[test]
fn the_check_of_a_cut_sees_the_wrong_side() {
    use craft3d_kernel::boolean::{VerifyOptions, verify};
    for k in SCALES {
        let tol = tolerance(k);
        let block = boxed(k, [-2.0, -1.0, 0.0], [2.0, 2.0, 2.0]);
        let sheet = rectangle([-10.0, -10.0], [10.0, 10.0], &level(1.5, k), k);
        let front = solve(&block, &sheet, BooleanOp::Subtract, k).body;
        let wrong = verify(
            &block,
            &sheet,
            BooleanOp::Intersect,
            &front,
            &tol,
            &VerifyOptions::default(),
        )
        .unwrap();
        assert!(
            !wrong.violations.is_empty(),
            "the front kept for an intersect passes"
        );
        let outside = boxed(k, [5.0, 5.0, 5.0], [6.0, 6.0, 6.0]);
        let options = VerifyOptions {
            volumes: false,
            points: 64,
        };
        let away = verify(
            &block,
            &sheet,
            BooleanOp::Subtract,
            &outside,
            &tol,
            &options,
        )
        .unwrap();
        assert!(away.points > 0);
        let right = verify(&block, &sheet, BooleanOp::Subtract, &front, &tol, &options).unwrap();
        assert!(right.is_ok(), "{:?}", right.violations);
        // A result that fills the box of a ball it was cut from: its
        // points in the corners of the box are outside the ball.
        let round = ball(k, [0.0, 0.0, 1.0], 2.0);
        let filled_box = boxed(k, [-2.0, -2.0, -1.0], [2.0, 2.0, 3.0]);
        let seen = verify(
            &round,
            &sheet,
            BooleanOp::Subtract,
            &filled_box,
            &tol,
            &options,
        )
        .unwrap();
        assert!(
            seen.violations
                .iter()
                .any(|v| matches!(v, craft3d_kernel::boolean::Violation::Membership { .. })),
            "{:?}",
            seen.violations
        );
        assert!(!away.violations.is_empty());
    }
}
