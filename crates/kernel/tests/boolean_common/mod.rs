//! Shared by the boolean tests: operands at a scale, and the checks every
//! result gets.

// Test support: a failure here is a failed test.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, BooleanResult, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, MassProperties, check, mass_properties};

/// Model sizes the tests run at; the tolerance scales along.
pub const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

pub fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// The box from `lo` to `hi`, times `k`.
pub fn boxed(k: f64, lo: [f64; 3], hi: [f64; 3]) -> Body {
    let frame = Frame::WORLD
        .at(Vec3::new(lo[0] * k, lo[1] * k, lo[2] * k))
        .unwrap();
    primitives::block(
        &frame,
        (hi[0] - lo[0]) * k,
        (hi[1] - lo[1]) * k,
        (hi[2] - lo[2]) * k,
    )
    .unwrap()
}

/// An upright cylinder standing on `base`, times `k`.
pub fn rod(k: f64, base: [f64; 3], radius: f64, height: f64) -> Body {
    let frame = Frame::WORLD
        .at(Vec3::new(base[0] * k, base[1] * k, base[2] * k))
        .unwrap();
    primitives::cylinder(&frame, radius * k, height * k).unwrap()
}

pub fn mass(body: &Body) -> MassProperties {
    mass_properties(body).unwrap()
}

/// `(vertices, edges, faces, shells)`.
pub fn counts(result: &BooleanResult) -> (usize, usize, usize, usize) {
    let b = &result.body;
    (
        b.vertices.len(),
        b.edges.len(),
        b.faces.len(),
        b.shells.len(),
    )
}

/// The boolean's own check in full ([`craft3d_kernel::boolean::verify`]:
/// volumes, the result's reach, the faces far from the other operand and
/// seeded points), which every result a test makes must pass.
#[track_caller]
pub fn verified(a: &Body, b: &Body, op: BooleanOp, result: &Body, tol: &Tolerance, what: &str) {
    use craft3d_kernel::boolean::{VerifyOptions, verify};
    let found = match verify(a, b, op, result, tol, &VerifyOptions::default()) {
        Ok(found) => found,
        Err(e) => panic!("{what}: {op:?}: the boolean check cannot run: {e}"),
    };
    let violations: Vec<String> = found.violations.iter().map(ToString::to_string).collect();
    assert!(
        violations.is_empty(),
        "{what}: {op:?}: the result breaks what every boolean keeps:\n  {}",
        violations.join("\n  ")
    );
}

/// The boolean, which must succeed, checked again here.
pub fn solve(a: &Body, b: &Body, op: BooleanOp, k: f64) -> BooleanResult {
    let tol = tolerance(k);
    let result = match boolean(a, b, op, &tol) {
        Ok(result) => result,
        Err(e) => panic!("{op:?} at scale {k}: {e}"),
    };
    assert_eq!(check(&result.body, &tol), [], "{op:?} at scale {k}");
    verified(a, b, op, &result.body, &tol, &format!("scale {k}"));
    assert_eq!(result.faces.len(), result.body.faces.len());
    assert_eq!(result.edges.len(), result.body.edges.len());
    assert_eq!(result.vertices.len(), result.body.vertices.len());
    result
}

/// The boolean, which must succeed or be refused as zero thickness
/// (parts of the result that touch along an edge or in a point, refused
/// since 2026-10-15): the result, made without that refusal when it was
/// refused and checked as [`solve`] checks, and where the refusal says the
/// parts touch. A refusal is held to a search of its own
/// ([`shells_touch_at`] on the result before merging); a result that is
/// not refused has no parts that touch by that search.
pub fn solve_parts(a: &Body, b: &Body, op: BooleanOp, k: f64) -> (BooleanResult, Option<Vec3>) {
    use craft3d_kernel::boolean::{BooleanOptions, boolean_with};
    let tol = tolerance(k);
    let unrefused = BooleanOptions {
        refuse_zero_thickness: false,
        ..BooleanOptions::default()
    };
    let imprinted = BooleanOptions {
        merge: false,
        ..unrefused
    };
    let touch = |op: BooleanOp| {
        boolean_with(a, b, op, &tol, &imprinted)
            .ok()
            .and_then(|r| shells_touch_at(&r.body, 2.0 * tol.linear))
    };
    match boolean(a, b, op, &tol) {
        Ok(_) => {
            assert_eq!(touch(op), None, "{op:?} at scale {k}: parts touch");
            (solve(a, b, op, k), None)
        }
        Err(KernelError::ZeroThickness { at, .. }) => {
            let result = match boolean_with(a, b, op, &tol, &unrefused) {
                Ok(result) => result,
                Err(e) => panic!("{op:?} at scale {k}, not refused: {e}"),
            };
            assert_eq!(check(&result.body, &tol), [], "{op:?} at scale {k}");
            verified(
                a,
                b,
                op,
                &result.body,
                &tol,
                &format!("scale {k}, not refused"),
            );
            // Parts that are shells of their own each have the place on
            // their boundary; a shell that touches itself (one part) has
            // it on its boundary at least (where exactly is the test's).
            let parts = if result.body.shells.len() > 1 { 2 } else { 1 };
            assert!(
                on_parts(&result.body, at, &tol) >= parts,
                "{op:?} at scale {k}: refused at {at:?}, which is not on {parts} parts"
            );
            (result, Some(at))
        }
        Err(e) => panic!("{op:?} at scale {k}: {e}"),
    }
}

pub fn assert_empty(a: &Body, b: &Body, op: BooleanOp, k: f64) {
    match boolean(a, b, op, &tolerance(k)) {
        Err(KernelError::EmptyResult { .. }) => {}
        other => panic!("{op:?} at scale {k}: expected an empty result, got {other:?}"),
    }
}

#[track_caller]
pub fn assert_close(actual: f64, expected: f64, what: &str) {
    let slack = 1e-9 * expected.abs().max(actual.abs());
    assert!(
        (actual - expected).abs() <= slack,
        "{what}: {actual} is not {expected}"
    );
}

/// Volume and area, given for scale 1.
#[track_caller]
pub fn assert_mass(result: &BooleanResult, volume: f64, area: f64, k: f64) {
    let m = mass(&result.body);
    assert_close(m.volume, volume * k * k * k, "volume");
    assert_close(m.area, area * k * k, "area");
}

/// SplitMix64: small, seeded, the same on every platform.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `[lo, hi)`.
    pub fn between(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

fn at(k: f64, p: [f64; 3]) -> Vec3 {
    Vec3::new(p[0] * k, p[1] * k, p[2] * k)
}

/// A ball about `center`, times `k`.
pub fn ball(k: f64, center: [f64; 3], radius: f64) -> Body {
    primitives::sphere(&Frame::WORLD.at(at(k, center)).unwrap(), radius * k).unwrap()
}

/// An upright cone standing on `base` with its apex `height` above it (or
/// below it, for a negative height), times `k`.
pub fn spike(k: f64, base: [f64; 3], radius: f64, height: f64) -> Body {
    let frame = if height > 0.0 {
        Frame::WORLD.at(at(k, base)).unwrap()
    } else {
        Frame::new(at(k, base), -Vec3::Z, Vec3::X).unwrap()
    };
    primitives::cone(&frame, radius * k, height.abs() * k).unwrap()
}

/// A ring about the vertical through `center`, times `k`.
pub fn ring(k: f64, center: [f64; 3], major: f64, minor: f64) -> Body {
    primitives::torus(
        &Frame::WORLD.at(at(k, center)).unwrap(),
        major * k,
        minor * k,
    )
    .unwrap()
}

/// A cylinder lying along x from `base`, times `k`.
pub fn log(k: f64, base: [f64; 3], radius: f64, length: f64) -> Body {
    let frame = Frame::new(at(k, base), Vec3::X, Vec3::Y).unwrap();
    primitives::cylinder(&frame, radius * k, length * k).unwrap()
}

/// How many faces of the body lie on each kind of surface, by name.
pub fn kinds(body: &Body) -> std::collections::BTreeMap<&'static str, usize> {
    let mut found = std::collections::BTreeMap::new();
    for face in &body.faces {
        *found.entry(face.surface.kind()).or_default() += 1;
    }
    found
}

/// On how many shells of `body` the point lies: within twice the
/// tolerance of the boundary of that shell alone, by the rays of
/// `contains_points` (a second opinion on where the boolean says parts
/// touch, independent of its search).
pub fn on_parts(body: &Body, at: Vec3, tol: &Tolerance) -> usize {
    (0..body.shells.len())
        .filter(|&s| {
            craft3d_kernel::boolean::contains_points(&shell_alone(body, s), &[at], tol)
                .is_ok_and(|found| found == [None])
        })
        .count()
}

/// Shell `shell` of `body` as a body of its own.
pub fn shell_alone(body: &Body, shell: usize) -> Body {
    use craft3d_topo::{Coedge, VertexId};
    use std::collections::BTreeMap;
    let mut out = Body::new(body.kind);
    let mut vertices: BTreeMap<u32, VertexId> = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let mut faces = Vec::new();
    for &f in &body.shells[shell].faces {
        let face = &body.faces[f.0 as usize];
        let mut loops = Vec::new();
        for &l in &face.loops {
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
        faces.push(out.add_face(face.surface.clone(), face.reversed, loops));
    }
    out.add_shell(faces);
    out
}

/// Where two shells of `body` touch: a vertex of one within `near` of a
/// vertex of another, or of a point of an edge of another (sampled at 257
/// points, then narrowed by ternary search on the distance), or on the
/// boundary of another (`contains_points` of that shell alone, by rays,
/// says it is within twice the tolerance of it). Written apart from
/// `craft3d_topo::touches` and the boolean's own search, by brute force
/// over every pair, as a second opinion on the boolean's zero-thickness
/// refusals.
pub fn shells_touch_at(body: &Body, near: f64) -> Option<Vec3> {
    if let Some(at) = shells_touch_at_entities(body, near) {
        return Some(at);
    }
    if body.shells.len() < 2 {
        return None;
    }
    let tol = Tolerance::new(0.5 * near, 1e-9).unwrap();
    for (s, _) in body.shells.iter().enumerate() {
        let alone = shell_alone(body, s);
        let own: std::collections::BTreeSet<[u64; 3]> = alone
            .vertices
            .iter()
            .map(|v| {
                [
                    v.point.x.to_bits(),
                    v.point.y.to_bits(),
                    v.point.z.to_bits(),
                ]
            })
            .collect();
        let others: Vec<Vec3> = body
            .vertices
            .iter()
            .map(|v| v.point)
            .filter(|p| !own.contains(&[p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]))
            .collect();
        if let Ok(found) = craft3d_kernel::boolean::contains_points(&alone, &others, &tol)
            && let Some(at) = others
                .iter()
                .zip(found)
                .find(|(_, inside)| inside.is_none())
                .map(|(p, _)| *p)
        {
            return Some(at);
        }
    }
    None
}

fn shells_touch_at_entities(body: &Body, near: f64) -> Option<Vec3> {
    let mut vertex_shell = vec![usize::MAX; body.vertices.len()];
    let mut edge_shell = vec![usize::MAX; body.edges.len()];
    for (s, shell) in body.shells.iter().enumerate() {
        for &f in &shell.faces {
            for &l in &body.faces[f.0 as usize].loops {
                for c in &body.loops[l.0 as usize].coedges {
                    let e = &body.edges[c.edge.0 as usize];
                    edge_shell[c.edge.0 as usize] = s;
                    vertex_shell[e.start.0 as usize] = s;
                    vertex_shell[e.end.0 as usize] = s;
                }
            }
        }
    }
    for (v, vertex) in body.vertices.iter().enumerate() {
        let s = vertex_shell[v];
        if s == usize::MAX {
            continue;
        }
        for (w, other) in body.vertices.iter().enumerate() {
            let t = vertex_shell[w];
            if t != usize::MAX && t != s && vertex.point.distance(other.point) <= near {
                return Some(vertex.point);
            }
        }
        for (e, edge) in body.edges.iter().enumerate() {
            let t = edge_shell[e];
            if t == usize::MAX || t == s {
                continue;
            }
            let (t0, t1) = edge.range;
            let step = (t1 - t0) / 256.0;
            let distance = |u: f64| edge.curve.eval(u).distance(vertex.point);
            let best = (0..=256)
                .map(|i| t0 + step * i as f64)
                .min_by(|a, b| distance(*a).total_cmp(&distance(*b)))
                .unwrap();
            let (mut lo, mut hi) = ((best - step).max(t0), (best + step).min(t1));
            for _ in 0..100 {
                let (a, b) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
                if distance(a) < distance(b) {
                    hi = b;
                } else {
                    lo = a;
                }
            }
            if distance(0.5 * (lo + hi)) <= near {
                return Some(vertex.point);
            }
        }
    }
    None
}
