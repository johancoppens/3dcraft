//! Faces whose edges come nearer to each other than the chordal tolerance:
//! the chords of a curved edge, taken on their own, then cross the
//! neighbouring edge, and the polygons no longer bound the face. The
//! edge must be sampled finer there, for both faces that share it.
//!
//! Found by the random booleans (`SELF_PAIR="1000370 0 0.001"`, the union:
//! a planar face refused at a chord of 1e-3 of the body's size and meshed
//! at 1e-2, 1e-4 and 1e-5). Reduced here to two solids built by hand, with
//! closed forms, held to oracle 4 of docs/testing.md at many tolerances.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Curve3, Cylinder, Line, Plane, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{
    Mesh, TessError, TessParams, tessellate_body, tessellate_body_and_edges,
    tessellate_body_with_threads, tessellate_edges,
};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, VertexId, check, mass_properties};
use std::collections::BTreeSet;
use std::f64::consts::{PI, TAU};

fn params(tolerance: f64) -> TessParams {
    TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    }
}

/// A solid with what is known about it in closed form. `face_areas` are
/// the areas of its faces, in face order.
struct Solid {
    name: String,
    body: Body,
    volume: f64,
    face_areas: Vec<f64>,
}

impl Solid {
    fn area(&self) -> f64 {
        self.face_areas.iter().sum()
    }
}

fn line(body: &mut Body, a: VertexId, b: VertexId) -> EdgeId {
    let (p, q) = (
        body.vertices[a.0 as usize].point,
        body.vertices[b.0 as usize].point,
    );
    body.add_edge(
        Curve3::Line(Line::through(p, q).unwrap()),
        (0.0, p.distance(q)),
        a,
        b,
    )
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

/// A disc of radius `k` and height `k / 2` with a notch sawn into it from
/// its far side almost all the way through: the notch's tip, at the angle
/// `tip_at`, stops `gap × k` short of the rim. The rim is one arc of 340°;
/// the two flat faces are left with a neck of that width between the tip
/// and the arc.
fn notched_disc(frame: &Frame, k: f64, gap: f64, tip_at: f64) -> Solid {
    let (r, h) = (k, 0.5 * k);
    let sweep = 170.0_f64.to_radians();
    let mut body = Body::new(BodyKind::Solid);
    let mut levels = Vec::new();
    for z in [0.0, h] {
        let centre = frame.at(frame.point(0.0, 0.0, z)).unwrap();
        let circle = Curve3::Circle(Circle::new(centre, r).unwrap());
        let tip_radius = r * (1.0 - gap);
        let tip = frame.point(tip_radius * tip_at.cos(), tip_radius * tip_at.sin(), z);
        let low = body.add_vertex(circle.eval(-sweep));
        let high = body.add_vertex(circle.eval(sweep));
        let tip = body.add_vertex(tip);
        let arc = body.add_edge(circle, (-sweep, sweep), low, high);
        let in_ = line(&mut body, high, tip);
        let out = line(&mut body, tip, low);
        levels.push(([low, high, tip], [arc, in_, out]));
    }
    let ([low0, high0, tip0], [arc0, in0, out0]) = levels[0];
    let ([low1, high1, tip1], [arc1, in1, out1]) = levels[1];
    let up_low = line(&mut body, low0, low1);
    let up_high = line(&mut body, high0, high1);
    let up_tip = line(&mut body, tip0, tip1);
    let point = |body: &Body, v: VertexId| body.vertices[v.0 as usize].point;

    let bottom = body.add_loop(vec![backward(out0), backward(in0), backward(arc0)]);
    let top = body.add_loop(vec![forward(arc1), forward(in1), forward(out1)]);
    let rim = body.add_loop(vec![
        forward(arc0),
        forward(up_high),
        backward(arc1),
        backward(up_low),
    ]);
    let wall_in = body.add_loop(vec![
        forward(in0),
        forward(up_tip),
        backward(in1),
        backward(up_high),
    ]);
    let wall_out = body.add_loop(vec![
        forward(out0),
        forward(up_low),
        backward(out1),
        backward(up_tip),
    ]);
    // A wall over the line from `a` to `b`, the solid on its left.
    let wall = |body: &Body, a: VertexId, b: VertexId| {
        let (a, b) = (point(body, a), point(body, b));
        let along = b - a;
        Surface::Plane(Plane::new(
            Frame::new(a, along.cross(frame.z()), along).unwrap(),
        ))
    };
    let faces = vec![
        body.add_face(
            Surface::Plane(Plane::new(
                Frame::new(frame.origin(), -frame.z(), frame.x()).unwrap(),
            )),
            false,
            vec![bottom],
        ),
        body.add_face(
            Surface::Plane(Plane::new(frame.at(frame.point(0.0, 0.0, h)).unwrap())),
            false,
            vec![top],
        ),
        body.add_face(
            Surface::Cylinder(Cylinder::new(*frame, r).unwrap()),
            false,
            vec![rim],
        ),
        {
            let surface = wall(&body, high0, tip0);
            body.add_face(surface, false, vec![wall_in])
        },
        {
            let surface = wall(&body, tip0, low0);
            body.add_face(surface, false, vec![wall_out])
        },
    ];
    body.add_shell(faces);

    // The outline's area by Green's theorem, ½ ∮ x dy − y dx: the arc
    // gives ½ r² times its angle, a line from p to q gives ½ p × q.
    let flat = |angle: f64, radius: f64| (radius * angle.cos(), radius * angle.sin());
    let (low, high, tip) = (
        flat(-sweep, r),
        flat(sweep, r),
        flat(tip_at, r * (1.0 - gap)),
    );
    let cross = |p: (f64, f64), q: (f64, f64)| p.0 * q.1 - p.1 * q.0;
    let outline = 0.5 * r * r * 2.0 * sweep + 0.5 * cross(high, tip) + 0.5 * cross(tip, low);
    let length = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).hypot(p.1 - q.1);
    Solid {
        name: format!("a disc with a notch to {gap:e} of its rim, at {tip_at}"),
        body,
        volume: outline * h,
        face_areas: vec![
            outline,
            outline,
            r * 2.0 * sweep * h,
            length(high, tip) * h,
            length(tip, low) * h,
        ],
    }
}

/// A washer of radius `k` and height `k / 2` whose hole, of radius
/// `k / 2`, is off centre: it comes within `gap × k` of the outer wall, in
/// the direction `towards`. All four circles are closed edges with one
/// vertex. With `seam`, the outer wall is one loop that goes up and down
/// a seam edge instead of two loops.
fn eccentric_washer(frame: &Frame, k: f64, gap: f64, towards: f64, seam: bool) -> Solid {
    let (outer, inner, h) = (k, 0.5 * k, 0.5 * k);
    let off = outer - gap * k - inner;
    let mut body = Body::new(BodyKind::Solid);
    let circle = |body: &mut Body, radius: f64, z: f64, shift: f64| {
        let centre = frame.point(shift * towards.cos(), shift * towards.sin(), z);
        let circle = Curve3::Circle(Circle::new(frame.at(centre).unwrap(), radius).unwrap());
        let vertex = body.add_vertex(circle.eval(0.0));
        (body.add_edge(circle, (0.0, TAU), vertex, vertex), vertex)
    };
    let (outer0, outer_v0) = circle(&mut body, outer, 0.0, 0.0);
    let (outer1, outer_v1) = circle(&mut body, outer, h, 0.0);
    let (inner0, _) = circle(&mut body, inner, 0.0, off);
    let (inner1, _) = circle(&mut body, inner, h, off);

    let bottom = vec![
        body.add_loop(vec![backward(outer0)]),
        body.add_loop(vec![forward(inner0)]),
    ];
    let top = vec![
        body.add_loop(vec![forward(outer1)]),
        body.add_loop(vec![backward(inner1)]),
    ];
    let outside = if seam {
        let up = line(&mut body, outer_v0, outer_v1);
        vec![body.add_loop(vec![
            forward(outer0),
            forward(up),
            backward(outer1),
            backward(up),
        ])]
    } else {
        vec![
            body.add_loop(vec![forward(outer0)]),
            body.add_loop(vec![backward(outer1)]),
        ]
    };
    let inside = vec![
        body.add_loop(vec![forward(inner1)]),
        body.add_loop(vec![backward(inner0)]),
    ];
    let hole_axis = frame
        .at(frame.point(off * towards.cos(), off * towards.sin(), 0.0))
        .unwrap();
    let faces = vec![
        body.add_face(
            Surface::Plane(Plane::new(
                Frame::new(frame.origin(), -frame.z(), frame.x()).unwrap(),
            )),
            false,
            bottom,
        ),
        body.add_face(
            Surface::Plane(Plane::new(frame.at(frame.point(0.0, 0.0, h)).unwrap())),
            false,
            top,
        ),
        body.add_face(
            Surface::Cylinder(Cylinder::new(*frame, outer).unwrap()),
            false,
            outside,
        ),
        body.add_face(
            Surface::Cylinder(Cylinder::new(hole_axis, inner).unwrap()),
            true,
            inside,
        ),
    ];
    body.add_shell(faces);
    let ring = PI * (outer * outer - inner * inner);
    Solid {
        name: format!(
            "a washer with its hole {gap:e} from the rim, at {towards}{}",
            if seam { ", with a seam edge" } else { "" }
        ),
        body,
        volume: ring * h,
        face_areas: vec![ring, ring, TAU * outer * h, TAU * inner * h],
    }
}

fn frames(scale: f64) -> [Frame; 2] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * scale,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap(),
    ]
}

/// Gaps from a hundredth to a hundred-thousandth of the size, where the
/// edges come near each other at several places along the curved one.
fn solids(frame: &Frame, scale: f64) -> Vec<Solid> {
    vec![
        notched_disc(frame, scale, 1e-3, 0.12),
        notched_disc(frame, scale, 1e-2, -1.9),
        notched_disc(frame, scale, 1e-4, 2.3),
        notched_disc(frame, scale, 1e-5, 0.7),
        eccentric_washer(frame, scale, 1e-3, 0.3, false),
        eccentric_washer(frame, scale, 1e-2, 4.1, false),
        eccentric_washer(frame, scale, 1e-4, 2.0, false),
        eccentric_washer(frame, scale, 1e-3, 5.5, true),
    ]
}

fn bits(mesh: &Mesh) -> Vec<u64> {
    mesh.positions
        .iter()
        .chain(&mesh.normals)
        .flat_map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
        .collect()
}

fn face_area(mesh: &Mesh, face: u32) -> f64 {
    (0..mesh.triangles.len())
        .filter(|&t| mesh.face_ids[t] == face)
        .map(|t| {
            let [a, b, c] = mesh.corners(t).unwrap();
            (b - a).cross(c - a).length() * 0.5
        })
        .sum()
}

#[test]
fn the_bodies_are_valid_and_their_closed_forms_are_theirs() {
    for scale in [1e-3, 1.0, 1e6] {
        // The gaps go down to 1e-5 of the size; the modelling tolerance
        // is well below that.
        let tolerance = Tolerance::new(1e-7 * scale, 1e-9).unwrap();
        for frame in frames(scale) {
            for solid in solids(&frame, scale) {
                let what = format!("{} at scale {scale}", solid.name);
                assert_eq!(check(&solid.body, &tolerance), vec![], "{what}");
                // By another route than the tessellator: the integrals of
                // `craft3d_topo` over the exact faces.
                let mass = mass_properties(&solid.body).unwrap();
                assert!(
                    (mass.volume - solid.volume).abs() <= 1e-9 * solid.volume,
                    "{what}: volume {} vs {}",
                    mass.volume,
                    solid.volume
                );
                assert!(
                    (mass.area - solid.area()).abs() <= 1e-9 * solid.area(),
                    "{what}: area {} vs {}",
                    mass.area,
                    solid.area()
                );
            }
        }
    }
}

/// Everything oracle 4 asks of one mesh.
fn assert_good_mesh(solid: &Solid, mesh: &Mesh, scale: f64, tolerance: f64, what: &str) {
    assert_eq!(mesh.normals.len(), mesh.positions.len(), "{what}");
    assert_eq!(mesh.face_ids.len(), mesh.triangles.len(), "{what}");
    let welded = mesh.weld().unwrap();
    assert!(welded.is_closed(), "{what}: not a closed manifold");
    for (t, &face) in mesh.face_ids.iter().enumerate() {
        let face = &solid.body.faces[face as usize];
        let [a, b, c] = mesh.corners(t).unwrap();
        let distance = |p: Vec3| {
            let (u, v) = face.surface.project(p).unwrap();
            (
                face.surface.eval(u, v).distance(p),
                face.surface.normal(u, v),
            )
        };
        for p in [a, b, c] {
            assert!(distance(p).0 <= 1e-9 * scale, "{what}: vertex off its face");
        }
        let centroid = (a + b + c) * (1.0 / 3.0);
        let (off, normal) = distance(centroid);
        assert!(
            off <= tolerance * (1.0 + 1e-9),
            "{what}: centroid {off:e} from the surface"
        );
        let outward = normal.unwrap() * if face.reversed { -1.0 } else { 1.0 };
        assert!(
            (b - a).cross(c - a).dot(outward) > 0.0,
            "{what}: triangle {t} faces inward"
        );
    }
    // Within `tolerance` of the surface everywhere, so the volume is off
    // by at most area × tolerance.
    let volume = welded.volume();
    assert!(
        (volume - solid.volume).abs() <= solid.area() * tolerance,
        "{what}: volume {volume} vs {}",
        solid.volume
    );
    // No face is larger than it is: flat faces lose what the chords cut
    // off (and gain it around a hole, never beyond the wall's tolerance),
    // curved ones are inscribed.
    for (f, exact) in solid.face_areas.iter().enumerate() {
        let area = face_area(mesh, f as u32);
        assert!(
            (area - exact).abs() <= 4.0 * tolerance * solid.area() / scale,
            "{what}: face {f} has area {area}, not {exact}"
        );
    }
}

/// Forty tolerances from a tenth to a hundred-thousandth of the size:
/// whether a chord crosses the neighbouring edge depends on where the
/// chords happen to fall, so one tolerance proves little. None may be
/// refused, and each mesh is a good one.
#[test]
fn no_tolerance_is_refused() {
    let mut refused = Vec::new();
    let mut meshed = 0;
    for scale in [1e-3, 1.0, 1e6] {
        let frame = if scale == 1.0 {
            Frame::WORLD
        } else {
            frames(scale)[1]
        };
        for solid in solids(&frame, scale) {
            for step in 0..40 {
                let rel = 10.0_f64.powf(-1.0 - 4.0 * f64::from(step) / 39.0);
                let tolerance = rel * scale;
                let what = format!("{} at scale {scale}, tolerance {rel:.3e}", solid.name);
                match tessellate_body(&solid.body, &params(tolerance)) {
                    Ok(mesh) => {
                        assert_good_mesh(&solid, &mesh, scale, tolerance, &what);
                        meshed += 1;
                    }
                    Err(e) => refused.push(format!("{what}: {e}")),
                }
            }
        }
    }
    println!("{meshed} meshed, {} refused", refused.len());
    assert!(
        refused.is_empty(),
        "{} refused: {refused:#?}",
        refused.len()
    );
}

#[test]
fn volume_and_face_areas_converge_to_the_closed_forms() {
    for solid in solids(&Frame::WORLD, 1.0) {
        let mut previous = f64::INFINITY;
        let mut last = Vec::new();
        for tolerance in [1e-1, 1e-2, 1e-3, 1e-4, 1e-5] {
            let mesh = tessellate_body(&solid.body, &params(tolerance)).unwrap();
            let volume_error = (mesh.weld().unwrap().volume() - solid.volume).abs() / solid.volume;
            last = solid
                .face_areas
                .iter()
                .enumerate()
                .map(|(f, exact)| (face_area(&mesh, f as u32) - exact).abs() / exact)
                .collect();
            println!(
                "{:60} tol {tolerance:e}: {:6} triangles, volume error {volume_error:.2e}",
                solid.name,
                mesh.triangles.len()
            );
            assert!(volume_error <= previous, "{}", solid.name);
            previous = volume_error;
        }
        // A chord error of 1e-5 on a size of 1.
        assert!(previous < 1e-4, "{}: {previous}", solid.name);
        assert!(last.iter().all(|e| *e < 1e-4), "{}: {last:?}", solid.name);
    }
}

#[test]
fn output_does_not_depend_on_the_number_of_threads() {
    for solid in solids(&frames(1.0)[1], 1.0) {
        for tolerance in [0.1, 0.03, 4e-3, 1e-3] {
            let reference =
                tessellate_body_with_threads(&solid.body, &params(tolerance), 1).unwrap();
            for threads in [1, 2, 8, 0] {
                let mesh =
                    tessellate_body_with_threads(&solid.body, &params(tolerance), threads).unwrap();
                let what = format!("{} at {tolerance}: {threads} threads", solid.name);
                assert_eq!(bits(&mesh), bits(&reference), "{what}");
                assert_eq!(mesh, reference, "{what}");
            }
            assert_eq!(
                tessellate_edges(&solid.body, &params(tolerance)).unwrap(),
                tessellate_edges(&solid.body, &params(tolerance)).unwrap(),
                "{}",
                solid.name
            );
        }
    }
}

/// `tessellate_edges` gives the points the mesh has along each edge, also
/// where an edge was sampled finer for the sake of one of its faces: its
/// points are corners of the mesh, and the mesh has no other corner on a
/// curved edge.
#[test]
fn the_edge_polylines_are_the_borders_of_the_mesh() {
    let key = |p: Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    for solid in solids(&Frame::WORLD, 1.0) {
        for tolerance in [0.1, 0.03, 4e-3, 1e-3] {
            let what = format!("{} at {tolerance}", solid.name);
            let mesh = tessellate_body(&solid.body, &params(tolerance)).unwrap();
            let corners: BTreeSet<[u64; 3]> = mesh.positions.iter().map(|&p| key(p)).collect();
            let edges = tessellate_edges(&solid.body, &params(tolerance)).unwrap();
            assert_eq!(edges.len(), solid.body.edges.len(), "{what}");
            // Both in one go are the same two things.
            let both = tessellate_body_and_edges(&solid.body, &params(tolerance)).unwrap();
            assert_eq!(both.mesh.as_ref(), Ok(&mesh), "{what}");
            assert_eq!(both.edges, edges, "{what}");
            for (polyline, edge) in edges.iter().zip(&solid.body.edges) {
                for &p in polyline {
                    assert!(
                        corners.contains(&key(p)),
                        "{what}: an edge point not in the mesh"
                    );
                }
                let Curve3::Circle(circle) = &edge.curve else {
                    // A straight edge is its own chord: cutting it would
                    // cure nothing, and it is left alone.
                    assert_eq!(polyline.len(), 2, "{what}: a straight edge was cut");
                    continue;
                };
                // The corners of the mesh on this circle (the bodies have
                // no two edges on one circle).
                let centre = circle.frame().origin();
                let on_circle = mesh
                    .positions
                    .iter()
                    .filter(|p| {
                        let (x, y, z) = circle.frame().local(**p);
                        let _ = centre;
                        z.abs() <= 1e-12 && (x.hypot(y) - circle.radius()).abs() <= 1e-12
                    })
                    .map(|&p| key(p))
                    .collect::<BTreeSet<_>>();
                let of_edge: BTreeSet<[u64; 3]> = polyline.iter().map(|&p| key(p)).collect();
                assert_eq!(on_circle, of_edge, "{what}: the mesh and the edge differ");
                // Still within the tolerance of the circle, chord by chord.
                for pair in polyline.windows(2) {
                    let middle = pair[0].lerp(pair[1], 0.5);
                    let (x, y, _) = circle.frame().local(middle);
                    assert!(
                        circle.radius() - x.hypot(y) <= tolerance * (1.0 + 1e-9),
                        "{what}: a chord sags more than the tolerance"
                    );
                }
            }
        }
    }
}

/// The finer sampling is local: at a tolerance fifty times the gap the
/// rim of the notched disc gets a few points more near the notch's tip,
/// not a rim sampled fifty times finer.
#[test]
fn an_edge_is_sampled_finer_only_where_it_must_be() {
    let near = notched_disc(&Frame::WORLD, 1.0, 1e-3, 0.12);
    // The same disc with the notch's tip far from the rim: nothing to do.
    let far = notched_disc(&Frame::WORLD, 1.0, 0.5, 0.12);
    for tolerance in [0.05, 0.02] {
        let plain = tessellate_edges(&far.body, &params(tolerance)).unwrap()[0].len();
        let refined = tessellate_edges(&near.body, &params(tolerance)).unwrap();
        let (bottom, top) = (&refined[0], &refined[3]);
        assert!(bottom.len() > plain, "at {tolerance}: nothing was refined");
        assert!(
            bottom.len() <= plain + 12,
            "at {tolerance}: {} points where {plain} would do but for the notch",
            bottom.len()
        );
        // Both rims alike: the wall between them is ruled.
        assert_eq!(bottom.len(), top.len());
        // The points added are near the tip, at the angle 0.12.
        let added: Vec<f64> = bottom
            .iter()
            .map(|p| p.y.atan2(p.x))
            .filter(|angle| {
                !tessellate_edges(&far.body, &params(tolerance)).unwrap()[0]
                    .iter()
                    .any(|q| (q.y.atan2(q.x) - angle).abs() < 1e-9)
            })
            .collect();
        assert_eq!(added.len(), bottom.len() - plain);
        let chord = TAU / plain as f64;
        assert!(
            added.iter().all(|angle| (angle - 0.12).abs() <= chord),
            "at {tolerance}: points were added at {added:?}"
        );
    }
}

/// Edges that really cross stay an error: sampling them finer does not
/// uncross them, and the tessellator gives up after a bounded number of
/// attempts with the error it had.
#[test]
fn curved_edges_that_really_cross_are_still_refused() {
    // A bow tie whose crossing sides are arcs of radius 10.
    let corners =
        [(0.0, 2.0), (4.0, 0.0), (4.0, 4.0), (0.0, 0.0)].map(|(x, y)| Vec3::new(x, y, 0.0));
    let mut body = Body::new(BodyKind::Sheet);
    let vertices: Vec<VertexId> = corners.iter().map(|&p| body.add_vertex(p)).collect();
    let mut coedges = Vec::new();
    for i in 0..4 {
        let j = (i + 1) % 4;
        let (a, b) = (corners[i], corners[j]);
        let edge = if i % 2 == 0 {
            // The arc from a to b, counter-clockwise about a centre on
            // the left of the chord.
            let radius = 10.0_f64;
            let half = 0.5 * a.distance(b);
            let left = Vec3::Z.cross(b - a).normalized().unwrap();
            let centre = a.lerp(b, 0.5) + left * (radius * radius - half * half).sqrt();
            let circle =
                Circle::new(Frame::new(centre, Vec3::Z, a - centre).unwrap(), radius).unwrap();
            let sweep = 2.0 * (half / radius).asin();
            body.add_edge(
                Curve3::Circle(circle),
                (0.0, sweep),
                vertices[i],
                vertices[j],
            )
        } else {
            line(&mut body, vertices[i], vertices[j])
        };
        coedges.push(forward(edge));
    }
    let loop_ = body.add_loop(coedges);
    let face = body.add_face(Surface::Plane(Plane::new(Frame::WORLD)), false, vec![loop_]);
    body.add_shell(vec![face]);
    for tolerance in [0.5, 1e-2, 1e-4] {
        for threads in [1, 4] {
            let error =
                tessellate_body_with_threads(&body, &params(tolerance), threads).unwrap_err();
            assert!(
                matches!(error, TessError::Face { face: 0, .. }),
                "at {tolerance}: {error}"
            );
            assert!(error.to_string().contains("crosses itself"), "{error}");
        }
        // The edges are still given, whatever becomes of the face.
        let both = tessellate_body_and_edges(&body, &params(tolerance)).unwrap();
        assert!(matches!(both.mesh, Err(TessError::Face { face: 0, .. })));
        assert_eq!(
            both.edges,
            tessellate_edges(&body, &params(tolerance)).unwrap()
        );
        assert_eq!(
            tessellate_edges(&body, &params(tolerance)).unwrap().len(),
            4
        );
    }
}
