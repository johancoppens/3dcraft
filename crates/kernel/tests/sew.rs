//! The sewing round trip of tolerant modelling (slice 2; the decision of
//! 2026-10-18: sewing "merges topology and measures gaps; it never moves
//! geometry"; `docs/design/topology.md`, Method › *Sewing*).
//!
//! Valid bodies (blocks, a cylinder, a cone, booleans with spheres and
//! tori, a block with a NURBS roof) are exploded into loose faces, each
//! with its own edges and vertices, and each face is moved rigidly by up
//! to k tolerances (as a STEP file from another kernel has its faces: each
//! exact in itself, its neighbours off by a gap), one of its vertices
//! nudged besides. Sewn back (`topo::sew`, at the document's cap):
//!
//! - the same topology: as many faces, edges, vertices and shells, the
//!   same genus, every edge between the same two faces as before;
//! - valid with the tolerances it recorded (`check_capped`), none above
//!   the cap, none for an exploded body that was not moved;
//! - the volume within a bound derived from the gaps;
//! - the same body bit for bit when sewn twice; nothing moved (every
//!   vertex and curve of the result is one of the exploded body's);
//! - at three scales; a face moved beyond the cap is refused (left open,
//!   the check failing, or a gap above the cap named), never sewn.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_testkit::parallel::map_parallel;
use craft3d_testkit::trimmed;
use craft3d_topo::{
    Body, Coedge, EdgeId, FaceId, SewError, VertexId, check_capped, default_loosest, edge_faces,
    genus, mass_properties, sew,
};
use std::collections::BTreeMap;

/// SplitMix64: seeded, the same numbers everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn direction(&mut self) -> Vec3 {
        loop {
            let v = Vec3::new(
                2.0 * self.unit() - 1.0,
                2.0 * self.unit() - 1.0,
                2.0 * self.unit() - 1.0,
            );
            let l = v.length();
            if l > 0.1 && l <= 1.0 {
                return v * (1.0 / l);
            }
        }
    }
}

fn tolerance(scale: f64) -> Tolerance {
    Tolerance {
        linear: 1e-6 * scale,
        angular: 1e-9,
    }
}

fn at(x: f64, y: f64, z: f64, scale: f64) -> Frame {
    Frame::WORLD.at(Vec3::new(x, y, z) * scale).unwrap()
}

const BODIES: usize = 9;

fn body(kind: usize, scale: f64) -> (String, Body) {
    let tol = tolerance(scale);
    let block = || {
        primitives::block(
            &at(0.0, 0.0, 0.0, scale),
            2.0 * scale,
            2.0 * scale,
            2.0 * scale,
        )
        .unwrap()
    };
    let cut = |a: &Body, b: &Body, op| boolean(a, b, op, &tol).unwrap().body;
    match kind {
        0 => ("block".into(), block()),
        1 => (
            "cylinder".into(),
            primitives::cylinder(&at(0.3, 0.2, 0.1, scale), 0.6 * scale, 3.0 * scale).unwrap(),
        ),
        2 => (
            "cone".into(),
            primitives::cone(&at(0.3, 0.2, 0.1, scale), 0.8 * scale, 1.5 * scale).unwrap(),
        ),
        3 => {
            let along_x = Frame::new(Vec3::new(-0.5, 1.0, 0.8) * scale, Vec3::X, Vec3::Y).unwrap();
            let hole = primitives::cylinder(&along_x, 0.4 * scale, 3.0 * scale).unwrap();
            (
                "block - cylinder".into(),
                cut(&block(), &hole, BooleanOp::Subtract),
            )
        }
        4 => {
            let ball = primitives::sphere(&at(2.1, 1.05, 0.95, scale), 0.8 * scale).unwrap();
            (
                "block + sphere".into(),
                cut(&block(), &ball, BooleanOp::Union),
            )
        }
        5 => {
            let ball = primitives::sphere(&at(2.1, 1.05, 0.95, scale), 0.8 * scale).unwrap();
            (
                "block & sphere".into(),
                cut(&block(), &ball, BooleanOp::Intersect),
            )
        }
        6 => {
            let ring =
                primitives::torus(&at(0.95, 1.1, 2.05, scale), 0.7 * scale, 0.25 * scale).unwrap();
            (
                "block - torus".into(),
                cut(&block(), &ring, BooleanOp::Subtract),
            )
        }
        7 => (
            "domed block".into(),
            trimmed::domed_block(
                &at(0.6, 0.5, 0.4, scale),
                2.2 * scale,
                1.1 * scale,
                1.2 * scale,
                true,
            )
            .unwrap()
            .body,
        ),
        _ => {
            let dome = trimmed::domed_block(
                &at(0.6, 0.5, 0.4, scale),
                2.2 * scale,
                1.1 * scale,
                1.2 * scale,
                true,
            )
            .unwrap()
            .body;
            let post =
                primitives::cylinder(&at(1.1, 0.9, -0.5, scale), 0.3 * scale, 3.0 * scale).unwrap();
            (
                "domed block - cylinder".into(),
                cut(&dome, &post, BooleanOp::Subtract),
            )
        }
    }
}

/// `body` as loose faces: each face with its own copies of its edges and
/// vertices (a seam the face uses twice stays one edge of it), the shells
/// holding the same faces; each face moved rigidly by `moves[f]` and
/// `nudge` moving one vertex of a face (face, its k-th vertex, by).
fn exploded(body: &Body, moves: &[Vec3], nudge: Option<(usize, usize, Vec3)>) -> Body {
    let mut out = Body::new(body.kind);
    for (f, face) in body.faces.iter().enumerate() {
        let t = Transform::translation(moves[f]);
        let mut vertices: BTreeMap<VertexId, VertexId> = BTreeMap::new();
        let mut edges: BTreeMap<EdgeId, EdgeId> = BTreeMap::new();
        let mut loops = Vec::new();
        for l in &face.loops {
            let mut coedges = Vec::new();
            for c in &body.loop_(*l).unwrap().coedges {
                let edge = match edges.get(&c.edge) {
                    Some(e) => *e,
                    None => {
                        let e = body.edge(c.edge).unwrap();
                        let mut vertex = |v: VertexId| {
                            *vertices.entry(v).or_insert_with(|| {
                                out.add_vertex(t.point(body.vertex(v).unwrap().point))
                            })
                        };
                        let (a, b) = (vertex(e.start), vertex(e.end));
                        let ne = out.add_edge(e.curve.transformed(&t).unwrap(), e.range, a, b);
                        edges.insert(c.edge, ne);
                        ne
                    }
                };
                coedges.push(Coedge {
                    edge,
                    reversed: c.reversed,
                });
            }
            loops.push(out.add_loop(coedges));
        }
        if let Some((nf, k, by)) = nudge
            && nf == f
            && let Some(v) = vertices.values().nth(k % vertices.len().max(1))
        {
            out.vertices[v.0 as usize].point += by;
        }
        out.add_face(face.surface.transformed(&t).unwrap(), face.reversed, loops);
    }
    for shell in &body.shells {
        out.add_shell(shell.faces.clone());
    }
    out
}

/// Per edge, the sorted pair of faces that use it, as a sorted list.
fn adjacency(body: &Body) -> Vec<Vec<FaceId>> {
    let mut out: Vec<Vec<FaceId>> = edge_faces(body)
        .into_iter()
        .map(|mut f| {
            f.sort();
            f
        })
        .collect();
    out.sort();
    out
}

/// Total edge length (64 chords each, a little short of it).
fn edge_length(body: &Body) -> f64 {
    body.edges
        .iter()
        .map(|e| {
            let (t0, t1) = e.range;
            (0..64)
                .map(|k| {
                    let t = |k: f64| t0 + (t1 - t0) * k / 64.0;
                    e.curve
                        .eval(t(f64::from(k)))
                        .distance(e.curve.eval(t(f64::from(k + 1))))
                })
                .sum::<f64>()
        })
        .sum::<f64>()
        * 1.1
}

#[derive(Debug, Default)]
struct Tally {
    cases: usize,
    sewn: usize,
    loose: usize,
    joined: usize,
    worst_gap_ratio: f64,
    worst_volume_ratio: f64,
    largest_tolerance_in_tols: f64,
    failures: Vec<String>,
}

fn run_case(kind: usize, scale: f64, k: f64, seed: u64) -> Tally {
    let mut tally = Tally::default();
    let tol = tolerance(scale);
    let cap = default_loosest(tol.linear);
    let (name, original) = body(kind, scale);
    let what = format!("{name} at {scale:e}, k = {k}, seed {seed}");
    let mut rng = Rng(seed ^ (kind as u64) << 32 ^ k.to_bits());
    let delta = k * tol.linear;
    let moves: Vec<Vec3> = (0..original.faces.len())
        .map(|_| rng.direction() * (delta * rng.unit()))
        .collect();
    let nudge = (k > 0.0).then(|| {
        (
            (rng.next() % original.faces.len() as u64) as usize,
            (rng.next() % 4) as usize,
            rng.direction() * (0.5 * delta),
        )
    });
    let loose = exploded(&original, &moves, nudge);
    tally.cases += 1;
    let sewn = match sew(&loose, &tol, cap, cap) {
        Ok(s) => s,
        Err(e) => {
            tally.failures.push(format!("{what}: refused: {e}"));
            return tally;
        }
    };
    let again = sew(&loose, &tol, cap, cap).unwrap();
    if format!("{:?}", again.body) != format!("{:?}", sewn.body) {
        tally.failures.push(format!("{what}: not deterministic"));
    }
    let b = &sewn.body;
    if !sewn.open.is_empty() {
        tally
            .failures
            .push(format!("{what}: {} edges left open", sewn.open.len()));
        return tally;
    }
    let counts = |x: &Body| {
        (
            x.faces.len(),
            x.edges.len(),
            x.vertices.len(),
            x.shells.len(),
        )
    };
    if counts(b) != counts(&original) {
        tally.failures.push(format!(
            "{what}: counts {:?} against {:?}",
            counts(b),
            counts(&original)
        ));
        return tally;
    }
    if genus(b) != genus(&original) {
        tally.failures.push(format!("{what}: genus"));
    }
    if adjacency(b) != adjacency(&original) {
        tally
            .failures
            .push(format!("{what}: another face adjacency"));
    }
    let defects = check_capped(b, &tol, cap);
    if !defects.is_empty() {
        tally.failures.push(format!(
            "{what}: invalid: {:?}",
            &defects[..defects.len().min(3)]
        ));
        return tally;
    }
    // Nothing moved: every point and curve is the exploded body's.
    for v in &b.vertices {
        if !loose.vertices.iter().any(|w| w.point == v.point) {
            tally.failures.push(format!("{what}: a vertex moved"));
        }
    }
    for e in &b.edges {
        if !loose
            .edges
            .iter()
            .any(|x| x.curve == e.curve && x.range == e.range)
        {
            tally.failures.push(format!("{what}: an edge moved"));
        }
    }
    let w = b.working_tolerance(tol.linear);
    if sewn.largest_gap > 0.0 {
        tally.worst_gap_ratio = tally.worst_gap_ratio.max(sewn.largest_gap / w);
    }
    if w > cap {
        tally.failures.push(format!("{what}: above the cap"));
    }
    tally.largest_tolerance_in_tols = w / tol.linear;
    if b.is_tolerant() {
        tally.loose += 1;
    }
    if k == 0.0 && b.is_tolerant() {
        tally
            .failures
            .push(format!("{what}: an exact explosion sewn loose"));
    }
    tally.joined += sewn.joined;
    // The volume, within a slab of W over every face and a strip of W
    // along every edge as far as the body reaches.
    let reference = mass_properties(&original).unwrap();
    match mass_properties(b) {
        Ok(m) => {
            let bbox = original.bbox().unwrap();
            let (lo, hi) = bbox.corners().unwrap();
            let diagonal = lo.distance(hi);
            let wide = w.max(2.0 * delta);
            let bound = 2.0 * wide * reference.area
                + wide * edge_length(&original) * diagonal / 3.0
                + 1e-9 * reference.volume.abs();
            let off = (m.volume - reference.volume).abs();
            tally.worst_volume_ratio = tally.worst_volume_ratio.max(off / bound);
            if off > bound {
                tally.failures.push(format!(
                    "{what}: volume {} against {}, bound {bound:e}",
                    m.volume, reference.volume
                ));
            }
        }
        Err(e) => tally.failures.push(format!("{what}: not measured: {e}")),
    }
    tally.sewn += 1;
    tally
}

#[test]
fn loose_faces_sew_back_into_the_body_they_came_from() {
    let mut cases = Vec::new();
    for kind in 0..BODIES {
        for scale in [1e-3, 1.0, 1e6] {
            for k in [0.0, 3.0, 30.0, 300.0] {
                for seed in [1u64, 2] {
                    cases.push((kind, scale, k, seed));
                }
            }
        }
    }
    let tallies = map_parallel(cases, |(kind, scale, k, seed)| {
        run_case(kind, scale, k, seed)
    });
    let mut all = Tally::default();
    for t in tallies {
        all.cases += t.cases;
        all.sewn += t.sewn;
        all.loose += t.loose;
        all.joined += t.joined;
        all.worst_gap_ratio = all.worst_gap_ratio.max(t.worst_gap_ratio);
        all.worst_volume_ratio = all.worst_volume_ratio.max(t.worst_volume_ratio);
        all.largest_tolerance_in_tols = all
            .largest_tolerance_in_tols
            .max(t.largest_tolerance_in_tols);
        all.failures.extend(t.failures);
    }
    println!(
        "sewing round trip: {} cases, {} sewn back ({} loose), {} pairs joined, largest tolerance {:.1} tolerances, worst gap {:.3} of the working tolerance, worst volume {:.3} of its bound",
        all.cases,
        all.sewn,
        all.loose,
        all.joined,
        all.largest_tolerance_in_tols,
        all.worst_gap_ratio,
        all.worst_volume_ratio
    );
    for f in all.failures.iter().take(20) {
        println!("  {f}");
    }
    assert!(all.failures.is_empty(), "{} failures", all.failures.len());
    assert_eq!(all.sewn, all.cases);
}

/// A face moved by twice the cap is never sewn to its neighbours: it is
/// left open (and the check refuses the solid) or its gap above the cap is
/// named.
#[test]
fn a_face_moved_beyond_the_cap_is_refused() {
    for scale in [1e-3, 1.0, 1e6] {
        let tol = tolerance(scale);
        let cap = default_loosest(tol.linear);
        for kind in [0, 3, 7] {
            let (name, original) = body(kind, scale);
            let mut moves = vec![Vec3::ZERO; original.faces.len()];
            moves[1] = Vec3::new(0.6, 0.0, 0.8) * (2.0 * cap);
            let loose = exploded(&original, &moves, None);
            match sew(&loose, &tol, cap, cap) {
                Ok(sewn) => {
                    let (edge, gap) = sewn.worst_open().expect("an open edge");
                    let gap = gap.expect("a nearest edge");
                    assert!(
                        gap > cap,
                        "{name} at {scale:e}: edge {edge:?} nearest {gap:e}"
                    );
                    assert!(!check_capped(&sewn.body, &tol, cap).is_empty());
                }
                Err(SewError::Gap { measured, .. }) => {
                    assert!(measured.is_none_or(|m| m > cap), "{name}: {measured:?}");
                }
                Err(other) => panic!("{name} at {scale:e}: {other}"),
            }
        }
    }
}

/// The operations on one body (fillet, chamfer, offset, move, draft,
/// delete, shell, thicken) on sewn bodies, loose at every edge: each
/// result is valid with the tolerances it carries, none looser than the
/// body given, with the volume of the same operation on the exact body
/// within a bound derived from them; or the operation refuses. Never an
/// invalid body, never a table dropped.
#[test]
fn operations_on_sewn_bodies_carry_their_tolerances_or_refuse() {
    use craft3d_kernel::chamfer::{ChamferEdge, ChamferSize, chamfer};
    use craft3d_kernel::delete_face::delete_faces;
    use craft3d_kernel::draft::{Draft, draft};
    use craft3d_kernel::fillet::{FilletEdge, fillet};
    use craft3d_kernel::move_face::{Motion, move_faces};
    use craft3d_kernel::offset_face::{FaceOffset, offset_faces};
    use craft3d_kernel::shell::shell;
    use craft3d_kernel::thicken::{Side, thicken};

    #[derive(Default)]
    struct Count {
        right: usize,
        refused: usize,
        loose: usize,
        exact_refused: usize,
    }
    let mut counts: BTreeMap<&'static str, Count> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut refusals: BTreeMap<String, usize> = BTreeMap::new();
    for scale in [1e-3, 1.0, 1e6] {
        let tol = tolerance(scale);
        let cap = default_loosest(tol.linear);
        for kind in [0usize, 3] {
            for k in [3.0_f64, 30.0, 300.0] {
                let (name, original) = body(kind, scale);
                let mut rng = Rng(11 ^ k.to_bits() ^ kind as u64);
                let delta = k * tol.linear;
                let moves: Vec<Vec3> = (0..original.faces.len())
                    .map(|_| rng.direction() * (delta * rng.unit()))
                    .collect();
                let sewn = sew(&exploded(&original, &moves, None), &tol, cap, cap)
                    .unwrap()
                    .body;
                assert!(sewn.is_tolerant());
                let w_in = sewn.working_tolerance(tol.linear);
                // The sewn edge between the faces of the original's edge.
                let pairs = edge_faces(&original);
                let sewn_pairs = edge_faces(&sewn);
                let edge_in = |e: EdgeId| {
                    let mut want = pairs[e.0 as usize].clone();
                    want.sort();
                    EdgeId(
                        sewn_pairs
                            .iter()
                            .position(|f| {
                                let mut f = f.clone();
                                f.sort();
                                f == want
                            })
                            .unwrap() as u32,
                    )
                };
                // A plane face along z (for the draft) and one that is not.
                let along_z = (0..original.faces.len())
                    .find(|&f| {
                        let face = &original.faces[f];
                        let p = original.vertices[0].point;
                        let (u, v) = face.surface.project(p).unwrap();
                        matches!(face.surface, craft3d_geom::Surface::Plane(_))
                            && face.surface.normal(u, v).unwrap().z.abs() < 1e-9
                    })
                    .map(|f| FaceId(f as u32));
                let flat = (0..original.faces.len())
                    .find(|&f| matches!(original.faces[f].surface, craft3d_geom::Surface::Plane(_)))
                    .map(|f| FaceId(f as u32))
                    .unwrap();
                let curved = (0..original.faces.len())
                    .find(|&f| {
                        !matches!(original.faces[f].surface, craft3d_geom::Surface::Plane(_))
                    })
                    .map(|f| FaceId(f as u32));
                let straight = (0..original.edges.len())
                    .find(|&e| matches!(original.edges[e].curve, craft3d_geom::Curve3::Line(_)))
                    .map(|e| EdgeId(e as u32))
                    .unwrap();
                let first_of = |e: EdgeId, b: &Body| edge_faces(b)[e.0 as usize][0];
                type Op<'a> =
                    Box<dyn Fn(&Body, &dyn Fn(EdgeId) -> EdgeId) -> Result<Body, String> + 'a>;
                let mut ops: Vec<(&'static str, Op)> = vec![
                    (
                        "fillet",
                        Box::new(|b: &Body, map: &dyn Fn(EdgeId) -> EdgeId| {
                            fillet(
                                b,
                                &[FilletEdge {
                                    edge: map(straight),
                                    radius: 0.2 * scale,
                                }],
                                &tol,
                            )
                            .map(|r| r.body)
                            .map_err(|e| e.to_string())
                        }),
                    ),
                    (
                        "chamfer",
                        Box::new(|b: &Body, map: &dyn Fn(EdgeId) -> EdgeId| {
                            let e = map(straight);
                            chamfer(
                                b,
                                &[ChamferEdge {
                                    edge: e,
                                    first: first_of(e, b),
                                    size: ChamferSize::Distances {
                                        first: 0.2 * scale,
                                        second: 0.3 * scale,
                                    },
                                }],
                                &tol,
                            )
                            .map(|r| r.body)
                            .map_err(|e| e.to_string())
                        }),
                    ),
                    (
                        "offset",
                        Box::new(|b: &Body, _: &dyn Fn(EdgeId) -> EdgeId| {
                            offset_faces(
                                b,
                                &[FaceOffset {
                                    face: flat,
                                    distance: 0.1 * scale,
                                }],
                                &tol,
                            )
                            .map(|r| r.body)
                            .map_err(|e| e.to_string())
                        }),
                    ),
                    (
                        "move",
                        Box::new(|b: &Body, _: &dyn Fn(EdgeId) -> EdgeId| {
                            let n = {
                                let face = &b.faces[flat.0 as usize];
                                let p = b.vertices[0].point;
                                let (u, v) = face.surface.project(p).unwrap();
                                face.surface.normal(u, v).unwrap()
                                    * if face.reversed { -1.0 } else { 1.0 }
                            };
                            move_faces(
                                b,
                                &[flat],
                                &Motion {
                                    translate: n * (0.15 * scale),
                                    pivot: Vec3::ZERO,
                                    axis: Vec3::Z,
                                    angle: 0.0,
                                },
                                &tol,
                            )
                            .map(|r| r.body)
                            .map_err(|e| e.to_string())
                        }),
                    ),
                    (
                        "shell",
                        Box::new(|b: &Body, _: &dyn Fn(EdgeId) -> EdgeId| {
                            shell(b, 0.2 * scale, &[flat], &tol)
                                .map(|r| r.body)
                                .map_err(|e| e.to_string())
                        }),
                    ),
                    (
                        "thicken",
                        Box::new(|b: &Body, _: &dyn Fn(EdgeId) -> EdgeId| {
                            thicken(b, &[flat], 0.1 * scale, Side::Outside, &tol)
                                .map(|r| r.body)
                                .map_err(|e| e.to_string())
                        }),
                    ),
                ];
                if let Some(face) = along_z {
                    ops.push((
                        "draft",
                        Box::new(move |b: &Body, _: &dyn Fn(EdgeId) -> EdgeId| {
                            draft(
                                b,
                                &Draft {
                                    faces: vec![face],
                                    point: Vec3::ZERO,
                                    pull: Vec3::Z,
                                    angle: 0.05,
                                },
                                &tol,
                            )
                            .map(|r| r.body)
                            .map_err(|e| e.to_string())
                        }),
                    ));
                }
                if let Some(face) = curved {
                    ops.push((
                        "delete",
                        Box::new(move |b: &Body, _: &dyn Fn(EdgeId) -> EdgeId| {
                            delete_faces(b, &[face], &tol)
                                .map(|r| r.body)
                                .map_err(|e| e.to_string())
                        }),
                    ));
                }
                for (op_name, op) in &ops {
                    let what = format!("{op_name} on {name} at {scale:e}, k = {k}");
                    let count = counts.entry(op_name).or_default();
                    let reference = match op(&original, &|e| e) {
                        Ok(r) => r,
                        Err(_) => {
                            count.exact_refused += 1;
                            continue;
                        }
                    };
                    match op(&sewn, &edge_in) {
                        Ok(result) => {
                            let defects = check_capped(&result, &tol, cap);
                            if !defects.is_empty() {
                                failures.push(format!("{what}: invalid {defects:?}"));
                                continue;
                            }
                            let w = result.working_tolerance(tol.linear);
                            if w > w_in * (1.0 + 1e-12) {
                                failures.push(format!(
                                    "{what}: looser ({w:e}) than its body ({w_in:e})"
                                ));
                            }
                            if result.is_tolerant() {
                                count.loose += 1;
                            }
                            let r = mass_properties(&reference).unwrap();
                            let (lo, hi) = reference.bbox().unwrap().corners().unwrap();
                            let wide = w_in.max(2.0 * delta);
                            let bound = 2.0 * wide * r.area
                                + wide * edge_length(&reference) * lo.distance(hi) / 3.0
                                + 1e-9 * r.volume.abs();
                            match mass_properties(&result) {
                                Ok(m) if (m.volume - r.volume).abs() <= bound => count.right += 1,
                                Ok(m) => failures.push(format!(
                                    "{what}: volume {} against {}, bound {bound:e}",
                                    m.volume, r.volume
                                )),
                                Err(e) => failures.push(format!("{what}: not measured: {e}")),
                            }
                        }
                        Err(e) => {
                            count.refused += 1;
                            *refusals
                                .entry(format!(
                                    "{op_name}: {}",
                                    e.chars().take(90).collect::<String>()
                                ))
                                .or_default() += 1;
                        }
                    }
                }
            }
        }
    }
    for (op, c) in &counts {
        println!(
            "{op:8}: {} right ({} carrying a table), {} refused, {} refused on the exact body too",
            c.right, c.loose, c.refused, c.exact_refused
        );
    }
    for (r, n) in &refusals {
        println!("  {n} × {r}");
    }
    for f in failures.iter().take(20) {
        println!("  {f}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
    for op in ["fillet", "chamfer", "offset", "shell"] {
        assert!(counts[op].right > 0, "{op} never worked on a loose body");
    }
}

/// A fan of 3000 loose triangles round one point (each with its own
/// vertices, a sheet): every spoke is sewn to its neighbour's, the rim
/// stays open, and the many edges ending at the hub cost a bounded
/// number of looks.
#[test]
fn a_fan_of_thousands_of_loose_facets_is_sewn_within_its_bounds() {
    use craft3d_geom::{Curve3, Line, Plane, Surface};
    let tol = tolerance(1.0);
    let cap = default_loosest(tol.linear);
    let n = 3000;
    let mut body = Body::new(craft3d_topo::BodyKind::Sheet);
    let rim = |i: usize| {
        let a = std::f64::consts::TAU * (i % n) as f64 / n as f64;
        Vec3::new(a.cos(), a.sin(), 0.0)
    };
    for i in 0..n {
        let pts = [Vec3::ZERO, rim(i), rim(i + 1)];
        let v: Vec<VertexId> = pts.iter().map(|p| body.add_vertex(*p)).collect();
        let mut coedges = Vec::new();
        for k in 0..3 {
            let (a, b) = (pts[k], pts[(k + 1) % 3]);
            let line = Line::through(a, b).unwrap();
            let e = body.add_edge(
                Curve3::Line(line),
                (0.0, a.distance(b)),
                v[k],
                v[(k + 1) % 3],
            );
            coedges.push(Coedge {
                edge: e,
                reversed: false,
            });
        }
        let l = body.add_loop(coedges);
        let f = body.add_face(Surface::Plane(Plane::new(Frame::WORLD)), false, vec![l]);
        body.add_shell(vec![f]);
    }
    let started = std::time::Instant::now();
    let sewn = sew(&body, &tol, cap, cap).unwrap();
    println!("fan of {n}: sewn in {:?}", started.elapsed());
    assert_eq!(sewn.joined, n);
    assert_eq!(sewn.open.len(), n);
    assert_eq!(sewn.body.shells.len(), 1);
    assert_eq!(sewn.body.vertices.len(), n + 1);
    assert!(check_capped(&sewn.body, &tol, cap).is_empty());
}

/// Faces that already share their edges come back as they are, bit for
/// bit, and so does an exact body.
#[test]
fn a_sewn_body_is_left_as_it_is() {
    let tol = tolerance(1.0);
    let cap = default_loosest(tol.linear);
    for kind in 0..BODIES {
        let (name, original) = body(kind, 1.0);
        let sewn = sew(&original, &tol, cap, cap).unwrap();
        assert_eq!(
            format!("{:?}", sewn.body),
            format!("{original:?}"),
            "{name}"
        );
        assert_eq!(sewn.joined, 0);
    }
}

/// A slot narrower than the sewing distance between two walls is not
/// sewn shut: each wall's own edges are nearer each other than the
/// slot's walls are.
#[test]
fn a_slot_narrower_than_the_distance_stays_open() {
    let tol = tolerance(1.0);
    let cap = default_loosest(tol.linear);
    // Two blocks 0.5 caps apart, each exploded and perturbed by a tenth
    // of a cap: sewn, they are two shells again, not one.
    let a = primitives::block(&at(0.0, 0.0, 0.0, 1.0), 1.0, 1.0, 1.0).unwrap();
    let b = primitives::block(
        &Frame::WORLD
            .at(Vec3::new(1.0 + 0.5 * cap, 0.0, 0.0))
            .unwrap(),
        1.0,
        1.0,
        1.0,
    )
    .unwrap();
    let both = craft3d_topo::merge(&[&a, &b]).unwrap().0;
    let mut rng = Rng(7);
    let moves: Vec<Vec3> = (0..both.faces.len())
        .map(|_| rng.direction() * (0.1 * cap * rng.unit()))
        .collect();
    let loose = exploded(&both, &moves, None);
    let sewn = sew(&loose, &tol, cap, cap).unwrap();
    assert!(sewn.open.is_empty());
    assert_eq!(sewn.body.shells.len(), 2);
    assert_eq!(adjacency(&sewn.body), adjacency(&both));
    // Each shell alone (as `body.split` takes them) keeps the tolerances
    // of its entities, and is valid with them.
    for s in 0..2 {
        let alone = craft3d_topo::shell_alone(&sewn.body, craft3d_topo::ShellId(s)).unwrap();
        assert!(alone.is_tolerant());
        assert!(check_capped(&alone, &tol, cap).is_empty());
    }
}

/// Two unit blocks with a slot of `w` caps between them, every face moved
/// by up to `m` caps: no edge of the result joins a face of one block to a
/// face of the other. Where an edge could be sewn across the slot as well
/// as to its own neighbour (the slot narrower than the gaps), it is left
/// open and the solid is refused, never sewn shut.
fn slot_case(w: f64, m: f64, seed: u64, scale: f64) -> (bool, usize) {
    let tol = tolerance(scale);
    let cap = default_loosest(tol.linear);
    let a = primitives::block(&at(0.0, 0.0, 0.0, scale), scale, scale, scale).unwrap();
    let b = primitives::block(
        &Frame::WORLD
            .at(Vec3::new(scale + w * cap, 0.0, 0.0))
            .unwrap(),
        scale,
        scale,
        scale,
    )
    .unwrap();
    let both = craft3d_topo::merge(&[&a, &b]).unwrap().0;
    let mut rng = Rng(seed);
    let moves: Vec<Vec3> = (0..both.faces.len())
        .map(|_| rng.direction() * (m * cap * rng.unit()))
        .collect();
    let loose = exploded(&both, &moves, None);
    let half = a.faces.len() as u32;
    match sew(&loose, &tol, cap, cap) {
        Ok(sewn) => {
            for faces in edge_faces(&sewn.body) {
                let sides: std::collections::BTreeSet<bool> =
                    faces.iter().map(|f| f.0 < half).collect();
                assert_eq!(
                    sides.len(),
                    1,
                    "slot {w} caps, moves {m}, seed {seed}, scale {scale:e}: sewn across"
                );
            }
            (sewn.open.is_empty(), sewn.body.shells.len())
        }
        Err(_) => (false, 0),
    }
}

#[test]
fn a_slot_narrower_than_the_gaps_is_never_sewn_shut() {
    let mut closed = 0;
    for scale in [1e-3, 1.0, 1e6] {
        for w in [0.3, 0.5, 0.9] {
            for seed in 1..=8 {
                let (all, shells) = slot_case(w, 0.4, seed, scale);
                if all {
                    assert_eq!(shells, 2);
                    closed += 1;
                }
            }
        }
    }
    println!("slots: {closed} of 72 sewn into their two blocks, the rest left open");
}

/// An edge of one face replaced by a polyline with the same ends that
/// meets the line at its even points and bulges 0.2 into its face between
/// them (two hundred caps): its seven inner places lie on the line, but
/// sewing proves the whole of both edges and refuses.
#[test]
fn an_edge_that_bulges_between_its_samples_is_not_sewn_exact() {
    use craft3d_geom::{Curve3, NurbsCurve};
    for scale in [1e-3, 1.0, 1e6] {
        let tol = tolerance(scale);
        let cap = default_loosest(tol.linear);
        let block = primitives::block(
            &at(0.0, 0.0, 0.0, scale),
            2.0 * scale,
            2.0 * scale,
            2.0 * scale,
        )
        .unwrap();
        let mut loose = exploded(&block, &vec![Vec3::ZERO; block.faces.len()], None);
        // The first edge of the top face (+z, face 1), bulging towards
        // the face's middle.
        let top = &loose.faces[1];
        let e = loose.loops[top.loops[0].0 as usize].coedges[0].edge;
        let edge = loose.edges[e.0 as usize].clone();
        let (p0, p1) = (edge.curve.eval(edge.range.0), edge.curve.eval(edge.range.1));
        let middle = Vec3::new(scale, scale, 2.0 * scale);
        let mid_edge = p0.lerp(p1, 0.5);
        let inward = (middle - mid_edge).normalized().unwrap() * (0.2 * scale);
        let points: Vec<Vec3> = (0..=16)
            .map(|j| {
                let p = p0.lerp(p1, f64::from(j) / 16.0);
                if j % 2 == 1 { p + inward } else { p }
            })
            .collect();
        let mut knots = vec![0.0, 0.0];
        knots.extend((1..16).map(f64::from));
        knots.extend([16.0, 16.0]);
        loose.edges[e.0 as usize].curve =
            Curve3::Nurbs(NurbsCurve::polynomial(1, knots, points).unwrap());
        loose.edges[e.0 as usize].range = (0.0, 16.0);
        match sew(&loose, &tol, cap, cap) {
            Err(SewError::Gap { .. }) => {}
            Ok(sewn) => panic!(
                "scale {scale:e}: sewn with a largest tolerance of {:e}",
                sewn.body.working_tolerance(tol.linear)
            ),
            Err(other) => panic!("scale {scale:e}: {other}"),
        }
    }
}

/// A face moved in its own plane by 0.9 caps: its edges along the motion
/// slide along their neighbours', the others move off them. Sewn, every
/// gap is recorded (the result is loose by at least the motion), and the
/// body is valid with it.
#[test]
fn a_face_moved_in_its_own_plane_is_sewn_with_the_gap_recorded() {
    for scale in [1e-3, 1.0, 1e6] {
        let tol = tolerance(scale);
        let cap = default_loosest(tol.linear);
        let block = primitives::block(
            &at(0.0, 0.0, 0.0, scale),
            2.0 * scale,
            2.0 * scale,
            2.0 * scale,
        )
        .unwrap();
        for (face, along) in [(4, Vec3::Y), (4, Vec3::Z), (5, Vec3::Y), (5, Vec3::Z)] {
            let mut moves = vec![Vec3::ZERO; block.faces.len()];
            moves[face] = along * (0.9 * cap);
            let loose = exploded(&block, &moves, None);
            match sew(&loose, &tol, cap, cap) {
                Ok(sewn) => {
                    let w = sewn.body.working_tolerance(tol.linear);
                    assert!(
                        w >= 0.9 * cap,
                        "face {face} along {along:?} at {scale:e}: largest tolerance {w:e}"
                    );
                    assert!(check_capped(&sewn.body, &tol, cap).is_empty());
                }
                Err(SewError::Gap { .. }) => {}
                Err(other) => panic!("{other}"),
            }
        }
    }
}
