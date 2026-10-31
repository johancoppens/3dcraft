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
use craft3d_math::Portable;
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
            // their boundary; a shell that touches itself has it on its
            // boundary and touches itself there by a witness of its own.
            if result.body.shells.len() > 1 {
                assert!(
                    on_parts(&result.body, at, &tol) >= 2,
                    "{op:?} at scale {k}: refused at {at:?}, which is not on two parts"
                );
            } else {
                assert!(
                    on_parts(&result.body, at, &tol) >= 1
                        && touches_itself_at(&result.body, at, &tol),
                    "{op:?} at scale {k}: refused at {at:?}, where the shell does not touch itself"
                );
            }
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

/// How the material of `body` lies around `at` on the sphere of `radius`
/// about it: the number of pieces of material and of space
/// (`(material, space)`), from `contains_points` at 3000 points evenly
/// over the sphere. A point counts for its kind when all its neighbours
/// (within one and a half times the mean spacing, about 3.3°) are of that
/// kind, and two counted points of a kind are joined when they are
/// neighbours: two pieces that meet in a point of the sphere (where a
/// line along which two parts touch goes through it) are not joined
/// through it, and a band wider than about 7° is kept. Points the rays
/// cannot tell (on the boundary) count for nothing.
pub fn sectors_at(body: &Body, at: Vec3, tol: &Tolerance, radius: f64) -> (usize, usize) {
    const N: usize = 3000;
    let golden = std::f64::consts::PI * (3.0 - 5f64.sqrt());
    let directions: Vec<Vec3> = (0..N)
        .map(|i| {
            let z = 1.0 - 2.0 * (i as f64 + 0.5) / N as f64;
            let r = (1.0 - z * z).sqrt();
            let a = golden * i as f64;
            Vec3::new(r * a.pcos(), r * a.psin(), z)
        })
        .collect();
    let points: Vec<Vec3> = directions.iter().map(|d| at + *d * radius).collect();
    let inside = craft3d_kernel::boolean::contains_points(body, &points, tol).unwrap();
    let spacing = (4.0 * std::f64::consts::PI / N as f64).sqrt();
    // Neighbours by a sort along z: only points within the reach in z.
    let reach = 1.5 * spacing;
    let neighbours: Vec<Vec<usize>> = (0..N)
        .map(|i| {
            (0..N)
                .filter(|&j| {
                    j != i
                        && (directions[i].z - directions[j].z).abs() < reach
                        && directions[i].distance(directions[j]) < reach
                })
                .collect()
        })
        .collect();
    let core: Vec<Option<bool>> = (0..N)
        .map(|i| {
            inside[i].filter(|&kind| {
                neighbours[i]
                    .iter()
                    .all(|&j| inside[j].is_none_or(|other| other == kind))
            })
        })
        .collect();
    let mut parent: Vec<usize> = (0..N).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..N {
        for &j in &neighbours[i] {
            if core[i].is_some() && core[i] == core[j] {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    let mut count = |kind: bool| {
        let mut roots: Vec<usize> = (0..N)
            .filter(|&i| core[i] == Some(kind))
            .map(|i| root(&mut parent, i))
            .collect();
        roots.sort_unstable();
        roots.dedup();
        roots.len()
    };
    (count(true), count(false))
}

fn diagonal(body: &Body) -> f64 {
    body.bbox()
        .ok()
        .and_then(|b| b.corners())
        .map_or(0.0, |(lo, hi)| lo.distance(hi))
}

/// Two pieces of material about `at` on a sphere a thousandth of the
/// body's box across (200 tolerances at least) and on one of 75
/// tolerances ([`sectors_at`]): two parts that meet there, seen near and a
/// little further off, so that a rib or a thin plate nearby that only one
/// sphere reaches is not taken for one.
pub fn material_pieces_at(body: &Body, at: Vec3, tol: &Tolerance) -> bool {
    let far = (1e-3 * diagonal(body)).max(200.0 * tol.linear);
    let near = 75.0 * tol.linear;
    sectors_at(body, at, tol, far).0 > 1 && sectors_at(body, at, tol, near).0 > 1
}

/// The regression net a refusal as zero thickness within one shell is
/// held to, on the body made without the refusal: it touches itself at
/// (or within a thousandth of its box of) `at`, by `craft3d_topo::touches`
/// (a vertex on a vertex or an edge of the same shell, what regeneration
/// holds every body to), by two pieces of material about it at two radii
/// ([`material_pieces_at`]), or by two faces through it that share
/// nothing there ([`walls_meet_at`]). None of the three asks the boolean;
/// each is a look at a sample of the body, not a proof.
pub fn touches_itself_at(body: &Body, at: Vec3, tol: &Tolerance) -> bool {
    let near = (1e-3 * diagonal(body)).max(100.0 * tol.linear);
    let by_vertex =
        craft3d_topo::touches(body, 2.0 * tol.linear, 1024, craft3d_topo::TouchScope::All)
            .is_ok_and(|found| found.iter().any(|t| t.at.distance(at) <= near));
    by_vertex || material_pieces_at(body, at, tol) || walls_meet_at(body, at, tol)
}

/// The point of triangle `[a, b, c]` nearest `p` (Ericson, *Real-Time
/// Collision Detection*, 5.1.5).
fn nearest_on_triangle(p: Vec3, [a, b, c]: [Vec3; 3]) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

/// The point of edge `e` nearest `p`, with the curve's tangent there (in
/// the edge's own direction): the projection inside the edge's range, or
/// the nearer end.
fn nearest_on_edge(body: &Body, e: u32, p: Vec3) -> (Vec3, Vec3) {
    let edge = &body.edges[e as usize];
    let (t0, t1) = edge.range;
    let mut best = t0;
    let mut candidates = vec![t0, t1];
    if let Ok(t) = edge.curve.project(p) {
        let t = match edge.curve.period() {
            Some(period) if period > 0.0 => t - ((t - t0) / period).floor() * period,
            _ => t,
        };
        if t >= t0 && t <= t1 {
            candidates.push(t);
        }
    }
    // And a row of samples, for a projection that found another branch.
    candidates.extend((1..64).map(|i| t0 + (t1 - t0) * i as f64 / 64.0));
    let mut nearest = f64::INFINITY;
    for t in candidates {
        let d = edge.curve.eval(t).distance(p);
        if d < nearest {
            nearest = d;
            best = t;
        }
    }
    let (point, tangent) = edge.curve.derivatives(best);
    (point, tangent)
}

/// Whether `q` (a point of face `f`'s surface) lies inside face `f` or on
/// its boundary: within four tolerances of a boundary edge, on; else
/// inside when the fine mesh of the face comes within its chord of `q`
/// and, near the boundary (within twice the chord), `q` is on the face's
/// side of its nearest boundary edge (the face lies to the left of its
/// loops seen down its outward normal).
fn on_face(
    body: &Body,
    mesh: &craft3d_tess::Mesh,
    chord: f64,
    f: usize,
    q: Vec3,
    tol: f64,
) -> bool {
    let face = &body.faces[f];
    let mut nearest: Option<(f64, Vec3, Vec3)> = None;
    for &l in &face.loops {
        for c in &body.loops[l.0 as usize].coedges {
            let (e, t) = nearest_on_edge(body, c.edge.0, q);
            let t = if c.reversed { -t } else { t };
            let d = e.distance(q);
            if nearest.is_none_or(|(best, _, _)| d < best) {
                nearest = Some((d, e, t));
            }
        }
    }
    if let Some((d, _, _)) = nearest
        && d <= 4.0 * tol
    {
        return true;
    }
    let meshed = (0..mesh.triangles.len()).any(|t| {
        mesh.face_ids[t] as usize == f
            && mesh
                .corners(t)
                .is_some_and(|tri| nearest_on_triangle(q, tri).distance(q) <= chord + 4.0 * tol)
    });
    if !meshed {
        return false;
    }
    let Some((d, e, t)) = nearest else {
        return true;
    };
    if d > 2.0 * chord {
        return true;
    }
    let Ok((u, v)) = face.surface.project(e) else {
        return false;
    };
    let Some(n) = face.surface.normal(u, v) else {
        return false;
    };
    let n = if face.reversed { -n } else { n };
    (q - e).dot(n.cross(t)) > 0.0
}

/// Two faces of `body` through `at` (each surface within twice the
/// tolerance of it, the nearest point of the surface inside the face or
/// on its boundary, [`on_face`], the two nearest points within twice the
/// tolerance of each other) that share no vertex and no edge there,
/// with material just past each of them: two walls of the body that meet
/// where nothing joins them, the parts behind them touching (not a wall
/// that thins to nothing between them). A shared
/// vertex joins within four tolerances, a shared edge within four
/// tolerances over sin(θ/2) (θ the angle between the faces at its nearest
/// point, so that a sharp wedge or groove is one), and no further than a
/// tenth of the smaller face's box. A plain look at the B-rep and a mesh
/// of it, nothing of the boolean's: the net for parts that touch
/// tangentially (a rounded end against a flat wall), whose gap is too
/// thin for [`sectors_at`] to see and shares no vertex for
/// `craft3d_topo::touches`.
pub fn walls_meet_at(body: &Body, at: Vec3, tol: &Tolerance) -> bool {
    let t = tol.linear;
    let chord = (1e-4 * diagonal(body)).max(10.0 * t);
    let params = craft3d_tess::TessParams {
        chordal_tolerance: chord,
        ..craft3d_tess::TessParams::default()
    };
    let Ok(mesh) = craft3d_tess::tessellate_body(body, &params) else {
        return false;
    };
    let at_face: Vec<(usize, Vec3)> = (0..body.faces.len())
        .filter_map(|f| {
            let surface = &body.faces[f].surface;
            let (u, v) = surface.project(at).ok()?;
            let q = surface.eval(u, v);
            (q.distance(at) <= 2.0 * t && on_face(body, &mesh, chord, f, q, t)).then_some((f, q))
        })
        .collect();
    let parts = |f: usize| {
        let mut vertices = std::collections::BTreeSet::new();
        let mut edges = std::collections::BTreeSet::new();
        for &l in &body.faces[f].loops {
            for c in &body.loops[l.0 as usize].coedges {
                edges.insert(c.edge.0);
                let e = &body.edges[c.edge.0 as usize];
                vertices.insert(e.start.0);
                vertices.insert(e.end.0);
            }
        }
        (vertices, edges)
    };
    let face_size = |f: usize| {
        body.face_bbox(craft3d_topo::FaceId(f as u32))
            .ok()
            .and_then(|b| b.corners())
            .map_or(0.0, |(lo, hi)| lo.distance(hi))
    };
    let outward = |f: usize, x: Vec3| {
        let face = &body.faces[f];
        let (u, v) = face.surface.project(x).ok()?;
        let n = face.surface.normal(u, v)?;
        Some(if face.reversed { -n } else { n })
    };
    // Just past face `f` (twenty tolerances out along its outward normal,
    // and as far again to the sides) there is material: the other part.
    // Where the material between two walls thins to nothing (a hole whose
    // wall touches a plate's side), past each wall is space.
    let beyond = |f: usize| {
        let Some(n) = outward(f, at) else {
            return false;
        };
        let Some(frame) = Frame::from_z(at, n) else {
            return false;
        };
        let step = 20.0 * t;
        let points: Vec<Vec3> = std::iter::once(Vec3::new(0.0, 0.0, 0.0))
            .chain((0..8).map(|k| {
                let a = k as f64 * std::f64::consts::FRAC_PI_4;
                frame.x() * a.pcos() + frame.y() * a.psin()
            }))
            .map(|side| at + (n + side) * step)
            .collect();
        craft3d_kernel::boolean::contains_points(body, &points, tol)
            .is_ok_and(|found| found.contains(&Some(true)))
    };
    for (i, &(f, qf)) in at_face.iter().enumerate() {
        let (fv, fe) = parts(f);
        for &(g, qg) in at_face.iter().skip(i + 1) {
            // No further apart than the tolerance can tell.
            if qf.distance(qg) > 2.0 * t {
                continue;
            }
            let (gv, ge) = parts(g);
            let most = (0.1 * face_size(f).min(face_size(g))).max(4.0 * t);
            let by_vertex = fv
                .intersection(&gv)
                .any(|&v| body.vertices[v as usize].point.distance(at) <= 4.0 * t);
            let by_edge = fe.intersection(&ge).any(|&e| {
                let (q, _) = nearest_on_edge(body, e, at);
                let margin = match (outward(f, q), outward(g, q)) {
                    (Some(nf), Some(ng)) => {
                        let half = (0.5 * (1.0 + nf.dot(ng))).max(0.0).sqrt();
                        if half > 0.0 { 4.0 * t / half } else { most }
                    }
                    _ => most,
                };
                q.distance(at) <= margin.max(4.0 * t).min(most)
            });
            if !by_vertex && !by_edge && beyond(f) && beyond(g) {
                return true;
            }
        }
    }
    false
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
