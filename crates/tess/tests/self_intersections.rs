//! Meshes that pass through themselves (docs/testing.md, oracle 9, at
//! the level of the mesh): seeded defects that must be found, valid
//! meshes in which nothing may be found, and the things a search over a
//! million triangles needs (the same answer on any number of threads, a
//! time that grows with the mesh and not with its square).
//!
//! The bodies the kernel makes are checked in
//! `crates/testkit/tests/self_intersection.rs`.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::{Frame, Transform, Vec3};
use craft3d_tess::{
    Crossing, CrossingKind, Mesh, TessError, TessParams, self_intersection_report,
    self_intersections, tessellate_body, tessellate_body_with_threads, tessellate_patches,
};
use craft3d_testkit::{solids, teapot, trimmed};
use std::collections::BTreeSet;
use std::time::Instant;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

/// A mesh put together by hand. Every triangle gets its own vertices, as
/// the tessellator gives every face its own: triangles are neighbours
/// where their corners are at the same place.
#[derive(Default, Clone)]
struct Soup {
    mesh: Mesh,
}

impl Soup {
    fn triangle(&mut self, a: Vec3, b: Vec3, c: Vec3, face: u32) -> usize {
        let base = self.mesh.positions.len() as u32;
        self.mesh.positions.extend([a, b, c]);
        self.mesh.normals.extend([Vec3::ZERO; 3]);
        self.mesh.triangles.push([base, base + 1, base + 2]);
        self.mesh.face_ids.push(face);
        self.mesh.triangles.len() - 1
    }

    /// `a b c d` counter-clockwise seen from outside.
    fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3, face: u32) {
        self.triangle(a, b, c, face);
        self.triangle(a, c, d, face);
    }

    /// A closed box facing outward, its six sides as faces `face..face + 6`
    /// (-z, +z, -y, +y, -x, +x).
    fn boxed(&mut self, lo: Vec3, hi: Vec3, face: u32) {
        let p = |i: usize| {
            v(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            )
        };
        let sides = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        for (k, [a, b, c, d]) in sides.into_iter().enumerate() {
            self.quad(p(a), p(b), p(c), p(d), face + k as u32);
        }
    }

    /// A closed ball of `2 n (n - 1)` triangles facing outward.
    fn ball(&mut self, centre: Vec3, radius: f64, n: usize, face: u32) {
        let at = |ring: usize, k: usize| {
            let polar = std::f64::consts::PI * ring as f64 / n as f64;
            let around = std::f64::consts::TAU * (k % (2 * n)) as f64 / (2 * n) as f64;
            // The poles and the seam exactly, whatever the sine says.
            if ring == 0 {
                return centre + v(0.0, 0.0, radius);
            }
            if ring == n {
                return centre + v(0.0, 0.0, -radius);
            }
            centre
                + v(
                    radius * polar.sin() * around.cos(),
                    radius * polar.sin() * around.sin(),
                    radius * polar.cos(),
                )
        };
        for ring in 0..n {
            for k in 0..2 * n {
                let (a, b) = (at(ring, k), at(ring, k + 1));
                let (c, d) = (at(ring + 1, k + 1), at(ring + 1, k));
                if ring > 0 {
                    self.triangle(a, c, b, face);
                }
                if ring + 1 < n {
                    self.triangle(a, d, c, face);
                }
            }
        }
    }

    fn scaled(&self, k: f64) -> Mesh {
        let mut mesh = self.mesh.clone();
        for p in &mut mesh.positions {
            *p = *p * k;
        }
        mesh
    }

    fn moved(&self, by: &Transform) -> Mesh {
        let mut mesh = self.mesh.clone();
        for p in &mut mesh.positions {
            *p = by.point(*p);
        }
        mesh
    }
}

/// Everything deeper than rounding.
fn all_crossings(mesh: &Mesh) -> Vec<Crossing> {
    self_intersections(mesh, 0.0, 100_000).unwrap()
}

fn face_pairs(crossings: &[Crossing]) -> BTreeSet<[u32; 2]> {
    crossings
        .iter()
        .map(|c| {
            let [f, g] = c.faces;
            [f.min(g), f.max(g)]
        })
        .collect()
}

/// A turn and a shift with nothing round about them.
fn askew() -> Transform {
    Transform::rotation(v(0.3, -0.7, 0.4), 0.83)
        .unwrap()
        .then(&Transform::translation(v(0.123, -4.56, 7.89)))
}

// ---------------------------------------------------------------- clean

#[test]
fn reference_solids_do_not_cross_themselves() {
    let mut triangles = 0;
    for scale in SCALES {
        let mut all = solids::all(&Frame::WORLD, scale).unwrap();
        all.extend(trimmed::all(&Frame::WORLD, scale).unwrap());
        all.push(trimmed::pipe_elbow(&Frame::WORLD, 3.0 * scale, scale, 1.9).unwrap());
        for solid in &all {
            for chord in [1e-1, 1e-2, 1e-3] {
                let params = TessParams {
                    chordal_tolerance: chord * scale,
                    ..TessParams::default()
                };
                let mesh = tessellate_body(&solid.body, &params).unwrap();
                let report = self_intersection_report(&mesh, 0.0, 16, 0).unwrap();
                assert_eq!(
                    report.crossings,
                    [],
                    "{} at scale {scale}, chord {chord}",
                    solid.name
                );
                assert_eq!(report.degenerate_triangles, 0, "{}", solid.name);
                assert!(!report.truncated);
                triangles += mesh.triangles.len();
            }
        }
    }
    assert!(triangles > 300_000, "{triangles}");
}

/// A flat fan and a flat grid, turned so that no two of their triangles
/// are in one plane to the last bit: neighbours all, and nothing else.
#[test]
fn flat_meshes_askew_have_no_crossings() {
    let mut fan = Soup::default();
    let n = 720;
    let rim = |k: usize| {
        let angle = std::f64::consts::TAU * (k % n) as f64 / n as f64;
        v(3.0 * angle.cos(), 3.0 * angle.sin(), 0.0)
    };
    for k in 0..n {
        fan.triangle(Vec3::ZERO, rim(k), rim(k + 1), 0);
    }
    let mut grid = Soup::default();
    for i in 0..40 {
        for j in 0..40 {
            let p = |a: usize, b: usize| v(a as f64 * 0.1, b as f64 * 0.37, 0.0);
            grid.quad(p(i, j), p(i + 1, j), p(i + 1, j + 1), p(i, j + 1), 0);
        }
    }
    for soup in [&fan, &grid] {
        for scale in SCALES {
            let mut mesh = soup.moved(&askew());
            for p in &mut mesh.positions {
                *p = *p * scale;
            }
            assert_eq!(all_crossings(&mesh), [], "scale {scale}");
        }
        // And flat in a coordinate plane, where everything is exactly so.
        assert_eq!(all_crossings(&soup.mesh), []);
    }
}

#[test]
fn touching_is_not_crossing() {
    for scale in SCALES {
        // Two boxes sharing an edge, and two sharing a corner.
        let mut edge = Soup::default();
        edge.boxed(v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), 0);
        edge.boxed(v(1.0, 1.0, 0.0), v(2.0, 2.0, 1.0), 6);
        assert_eq!(all_crossings(&edge.scaled(scale)), [], "an edge");
        let mut corner = Soup::default();
        corner.boxed(v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), 0);
        corner.boxed(v(1.0, 1.0, 1.0), v(2.0, 2.0, 2.0), 6);
        assert_eq!(all_crossings(&corner.scaled(scale)), [], "a corner");
        // A spike standing with its tip exactly on a plate, and a wedge
        // lying with an edge on it.
        let mut tip = Soup::default();
        tip.boxed(v(-2.0, -2.0, -1.0), v(2.0, 2.0, 0.0), 0);
        spike(&mut tip, v(0.5, 0.25, 0.0), 6);
        assert_eq!(all_crossings(&tip.scaled(scale)), [], "a tip");
        let mut wedge = Soup::default();
        wedge.boxed(v(-2.0, -2.0, -1.0), v(2.0, 2.0, 0.0), 0);
        let (a, b) = (v(-1.0, 0.5, 0.0), v(1.0, 0.25, 0.0));
        wedge.triangle(a, b, v(0.0, 1.0, 1.0), 6);
        wedge.triangle(b, a, v(0.0, -1.0, 1.0), 7);
        assert_eq!(all_crossings(&wedge.scaled(scale)), [], "a wedge");
        // A ball resting on the plate: its lowest vertex is on it.
        let mut ball = Soup::default();
        ball.boxed(v(-2.0, -2.0, -1.0), v(2.0, 2.0, 0.0), 0);
        ball.ball(v(0.25, 0.5, 1.0), 1.0, 24, 6);
        assert_eq!(all_crossings(&ball.scaled(scale)), [], "a ball");
    }
}

/// One body whose two parts meet face on face in one place and along an
/// edge in another: an L lying flat, and a bar on top of it that covers
/// the short leg and runs beside the long one. Along `y = 1, z = 1` from
/// `x = 1` to `x = 3` four faces meet, two and two in one plane, with
/// material in two opposite quarters: the top of the long leg and the
/// underside of the bar, the side of the leg and the side of the bar.
/// Nothing passes through anything there.
///
/// Each of the four lies exactly in the wall of the column over the
/// next, and the surface runs from one to the one opposite around the
/// end of the edge at `(1, 1, 1)`, where the leg's side turns the corner
/// under the bar. With the column a hair wider than its triangle all
/// round, that was "through": depth 1, the height of the leg, between
/// faces that touch. (Found by the long random run of the booleans: seed
/// 1000453 on the grid, a slot on an L.) The bar is longer than the leg,
/// so that the edge has other ends in the two parts, as it had there.
fn bar_on_an_l() -> Soup {
    let mut soup = Soup::default();
    let mut face = |points: &[[f64; 3]], outward: [f64; 3], id: u32| {
        let p: Vec<Vec3> = points.iter().map(|&[x, y, z]| v(x, y, z)).collect();
        let outward = v(outward[0], outward[1], outward[2]);
        for k in 1..p.len() - 1 {
            let (a, mut b, mut c) = (p[0], p[k], p[k + 1]);
            let turn = (b - a).cross(c - a).dot(outward);
            assert!(turn != 0.0, "a triangle without area in face {id}");
            if turn < 0.0 {
                (b, c) = (c, b);
            }
            soup.triangle(a, b, c, id);
        }
    };
    let (down, up) = ([0.0, 0.0, -1.0], [0.0, 0.0, 1.0]);
    let (left, right) = ([-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    let (front, back) = ([0.0, -1.0, 0.0], [0.0, 1.0, 0.0]);
    let q = |a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3]| [a, b, c, d];
    // The L: [0, 3] x [0, 1] and [0, 1] x [1, 2], from z = 0 to 1.
    face(
        &q(
            [0.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [3.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ),
        down,
        0,
    );
    face(
        &q(
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
        ),
        down,
        0,
    );
    // Its top, where the bar is not.
    let top = [
        [0.0, 0.0, 1.0],
        [3.0, 0.0, 1.0],
        [3.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ];
    face(&top, up, 1);
    face(
        &q(
            [0.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [3.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ),
        front,
        2,
    );
    face(
        &q(
            [3.0, 0.0, 0.0],
            [3.0, 1.0, 0.0],
            [3.0, 1.0, 1.0],
            [3.0, 0.0, 1.0],
        ),
        right,
        3,
    );
    // The side of the long leg, and the inner corner under the bar.
    face(
        &q(
            [3.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 1.0, 1.0],
            [3.0, 1.0, 1.0],
        ),
        back,
        4,
    );
    face(
        &q(
            [1.0, 1.0, 0.0],
            [1.0, 2.0, 0.0],
            [1.0, 2.0, 1.0],
            [1.0, 1.0, 1.0],
        ),
        right,
        5,
    );
    face(
        &q(
            [1.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
            [0.0, 2.0, 1.0],
            [1.0, 2.0, 1.0],
        ),
        back,
        6,
    );
    face(
        &q(
            [0.0, 0.0, 0.0],
            [0.0, 2.0, 0.0],
            [0.0, 2.0, 1.0],
            [0.0, 0.0, 1.0],
        ),
        left,
        7,
    );
    // The bar: [0, 3.5] x [1, 2], from z = 1 to 2.
    let far = [
        [0.0, 2.0, 2.0],
        [3.5, 2.0, 2.0],
        [3.5, 2.0, 1.0],
        [1.0, 2.0, 1.0],
        [0.0, 2.0, 1.0],
    ];
    face(&far, back, 6);
    face(
        &q(
            [0.0, 1.0, 1.0],
            [0.0, 2.0, 1.0],
            [0.0, 2.0, 2.0],
            [0.0, 1.0, 2.0],
        ),
        left,
        7,
    );
    face(
        &q(
            [0.0, 1.0, 2.0],
            [3.5, 1.0, 2.0],
            [3.5, 2.0, 2.0],
            [0.0, 2.0, 2.0],
        ),
        up,
        8,
    );
    // Its underside beside the L, and its side over the long leg.
    face(
        &q(
            [1.0, 1.0, 1.0],
            [1.0, 2.0, 1.0],
            [3.5, 2.0, 1.0],
            [3.5, 1.0, 1.0],
        ),
        down,
        9,
    );
    let near = [
        [0.0, 1.0, 2.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
        [3.5, 1.0, 1.0],
        [3.5, 1.0, 2.0],
    ];
    face(&near, front, 10);
    face(
        &q(
            [3.5, 1.0, 1.0],
            [3.5, 2.0, 1.0],
            [3.5, 2.0, 2.0],
            [3.5, 1.0, 2.0],
        ),
        right,
        11,
    );
    soup
}

#[test]
fn parts_of_one_body_that_meet_along_an_edge_touch() {
    let soup = bar_on_an_l();
    // The mesh is what it is meant to be: it encloses the L and the bar.
    let volume: f64 = (0..soup.mesh.triangles.len())
        .map(|t| {
            let [a, b, c] = soup.mesh.corners(t).unwrap();
            a.dot(b.cross(c)) / 6.0
        })
        .sum();
    assert!((volume - (4.0 + 3.5)).abs() < 1e-12, "{volume}");
    for scale in SCALES {
        assert_eq!(all_crossings(&soup.scaled(scale)), [], "scale {scale}");
        for turn in [0.83, 1.7, 2.9, 4.1] {
            let by = Transform::rotation(v(0.3, -0.7, 0.4), turn)
                .unwrap()
                .then(&Transform::translation(v(0.123, -4.56, 7.89)));
            let mut mesh = soup.moved(&by);
            for q in &mut mesh.positions {
                *q = *q * scale;
            }
            // Turned, the four faces are a rounding error out of their
            // planes: what is found is as deep as that, not as the leg.
            let deepest = all_crossings(&mesh)
                .iter()
                .map(|c| c.depth)
                .fold(0.0, f64::max);
            assert!(
                deepest < 1e-9 * scale,
                "scale {scale}, turned {turn}: {deepest}"
            );
        }
    }
    // The same bar pushed a tenth into the leg is a crossing, about a
    // tenth deep.
    let mut pushed = bar_on_an_l();
    let ids = pushed.mesh.face_ids.clone();
    for (corners, id) in pushed.mesh.positions.chunks_mut(3).zip(ids) {
        if id >= 8 {
            for p in corners {
                *p += v(0.0, -0.1, -0.1);
            }
        }
    }
    let deepest = all_crossings(&pushed.mesh)
        .iter()
        .map(|c| c.depth)
        .fold(0.0, f64::max);
    assert!(deepest >= 0.099, "{deepest}");
}

// ------------------------------------------------------- seeded defects

/// A spike of three faces with its tip at `tip`, pointing down.
fn spike(soup: &mut Soup, tip: Vec3, face: u32) {
    let base = [v(0.1, 0.0, 1.0), v(-0.05, 0.08, 1.0), v(-0.05, -0.08, 1.0)].map(|p| tip + p);
    for k in 0..3 {
        soup.triangle(tip, base[(k + 1) % 3], base[k], face + k as u32);
    }
}

/// (a) Two faces crossing: two closed boxes through each other.
#[test]
fn boxes_through_each_other_are_found() {
    for scale in SCALES {
        let mut soup = Soup::default();
        soup.boxed(v(0.0, 0.0, 0.0), v(4.0, 3.0, 2.0), 0);
        soup.boxed(v(1.0, 1.0, 1.0), v(6.0, 5.0, 4.0), 6);
        let mesh = soup.scaled(scale);
        let found = all_crossings(&mesh);
        // The faces of the first box that reach into the second (+z, +y,
        // +x) against the faces of the second that reach into the first
        // (-z, -y, -x): each of the six edges of the common corner is
        // where two of them cross.
        assert_eq!(
            face_pairs(&found),
            BTreeSet::from([[1, 8], [1, 10], [3, 6], [3, 10], [5, 6], [5, 8]]),
            "scale {scale}"
        );
        for c in &found {
            assert_eq!(c.kind, CrossingKind::Crossing);
            // The boxes reach one unit or more into each other.
            assert!(c.depth >= 0.999 * scale, "{c:?}");
            // On the surface of the common block from (1,1,1) to (4,3,2).
            let p = c.at * (1.0 / scale);
            let inside = |x: f64, lo: f64, hi: f64| x > lo - 1e-9 && x < hi + 1e-9;
            assert!(
                inside(p.x, 1.0, 4.0) && inside(p.y, 1.0, 3.0) && inside(p.z, 1.0, 2.0),
                "{c:?}"
            );
            assert!(c.triangles[0] < c.triangles[1]);
        }
        // A tolerance below the depth finds the same faces; one above it
        // finds nothing.
        let deep = self_intersections(&mesh, 0.25 * scale, 1000).unwrap();
        assert_eq!(face_pairs(&deep), face_pairs(&found), "scale {scale}");
        assert_eq!(self_intersections(&mesh, 1.5 * scale, 1000).unwrap(), []);

        // Apart: nothing.
        let mut apart = Soup::default();
        apart.boxed(v(0.0, 0.0, 0.0), v(4.0, 3.0, 2.0), 0);
        apart.boxed(v(4.5, 0.0, 0.0), v(6.0, 5.0, 4.0), 6);
        assert_eq!(all_crossings(&apart.scaled(scale)), []);
    }
}

/// A wall standing exactly on the diagonal along which a square is cut
/// into triangles: neither triangle of the square is on both sides of the
/// wall, the square is. And the same a hair askew, where each triangle
/// crosses the wall by next to nothing and the square by a lot.
#[test]
fn a_crossing_along_an_edge_of_the_triangulation_is_found() {
    for scale in SCALES {
        for skew in [0.0, 1e-12, -1e-12, 1e-6, 1e-2] {
            let mut soup = Soup::default();
            // Cut along the diagonal from (0, 0) to (2, 2).
            soup.quad(
                v(0.0, 0.0, 0.0),
                v(2.0, 0.0, 0.0),
                v(2.0, 2.0, 0.0),
                v(0.0, 2.0, 0.0),
                0,
            );
            // The wall in the plane x = y, turned by `skew` about the
            // middle of the square.
            let p = |t: f64, z: f64| v(t + skew * (t - 1.0), t - skew * (t - 1.0), z);
            soup.quad(p(0.5, -1.0), p(1.5, -1.0), p(1.5, 1.0), p(0.5, 1.0), 1);
            let what = format!("scale {scale}, skew {skew}");
            let found = all_crossings(&soup.scaled(scale));
            assert!(!found.is_empty(), "{what}");
            assert_eq!(face_pairs(&found), BTreeSet::from([[0, 1]]), "{what}");
            // The wall reaches 1 above and below the square; the square
            // half its diagonal or more to either side of the wall.
            let deepest = found.iter().map(|c| c.depth).fold(0.0, f64::max);
            assert!(deepest > 0.5 * scale, "{what}: {deepest}");
            // The same turned out of the coordinate planes, where rounding
            // puts the wall a hair to one side of the diagonal or the
            // other.
            if skew == 0.0 {
                for turn in [0.83, 1.7, 2.9, 4.1] {
                    let by = Transform::rotation(v(0.3, -0.7, 0.4), turn)
                        .unwrap()
                        .then(&Transform::translation(v(0.123, -4.56, 7.89)));
                    let mut mesh = soup.moved(&by);
                    for q in &mut mesh.positions {
                        *q = *q * scale;
                    }
                    let found = all_crossings(&mesh);
                    let deepest = found.iter().map(|c| c.depth).fold(0.0, f64::max);
                    assert!(deepest > 0.5 * scale, "{what}, turned {turn}: {deepest}");
                }
            }
            // The wall beside the square instead: nothing.
            let mut beside = soup.clone();
            for q in beside.mesh.positions.iter_mut().skip(6) {
                *q += v(0.0, 0.0, 1.5);
            }
            assert_eq!(all_crossings(&beside.scaled(scale)), [], "{what}");
            // Standing on the square without going through: contact.
            let mut standing = soup.clone();
            for q in standing.mesh.positions.iter_mut().skip(6) {
                *q += v(0.0, 0.0, 1.0);
            }
            assert_eq!(all_crossings(&standing.scaled(scale)), [], "{what}");
        }
    }
}

/// (b) A sheet folded back through itself: through a triangle it shares
/// nothing with, through one it shares a corner with, and flat onto the
/// one it shares an edge with.
#[test]
fn a_fold_through_a_neighbour_is_found() {
    for scale in SCALES {
        // Three strips in a row; the third comes back down through the
        // first.
        let mut strip = Soup::default();
        let row = |x: f64, z: f64| (v(x, 0.0, z), v(x, 1.0, z));
        let (a0, a1) = row(0.0, 0.0);
        let (b0, b1) = row(1.0, 0.0);
        let (c0, c1) = row(2.0, 0.5);
        let (d0, d1) = row(-0.5, -0.5);
        strip.quad(a0, b0, b1, a1, 0);
        strip.quad(b0, c0, c1, b1, 0);
        strip.quad(c0, d0, d1, c1, 0);
        let found = all_crossings(&strip.scaled(scale));
        assert!(!found.is_empty(), "scale {scale}");
        for c in &found {
            // First strip (triangles 0, 1) against the third (4, 5).
            assert!(c.triangles[0] < 2 && c.triangles[1] >= 4, "{c:?}");
            assert_eq!(c.kind, CrossingKind::Crossing);
            // It crosses z = 0 at x = 0.75, a quarter from the first
            // strip's end, and goes 0.5 down.
            assert!((c.at.x / scale - 0.75).abs() < 1e-9, "{c:?}");
            // Where it crosses the first strip the third goes from 0.1
            // above it to 0.3 below.
            assert!(c.depth > 0.05 * scale && c.depth < 0.11 * scale, "{c:?}");
        }

        // Two triangles of a fan, one turned through the other.
        let mut fan = Soup::default();
        let hub = Vec3::ZERO;
        fan.triangle(hub, v(4.0, 0.0, 0.0), v(0.0, 4.0, 0.0), 0);
        fan.triangle(hub, v(2.0, 2.0, 1.0), v(2.0, 2.0, -1.0), 0);
        let found = all_crossings(&fan.scaled(scale));
        assert_eq!(found.len(), 1, "scale {scale}");
        assert_eq!(found[0].kind, CrossingKind::Crossing);
        // Turned away instead: neighbours that meet in the hub only.
        let mut open = Soup::default();
        open.triangle(hub, v(4.0, 0.0, 0.0), v(0.0, 4.0, 0.0), 0);
        open.triangle(hub, v(-2.0, -2.0, 1.0), v(-2.0, -2.0, -1.0), 0);
        assert_eq!(all_crossings(&open.scaled(scale)), []);

        // Folded flat over a shared edge.
        let mut flat = Soup::default();
        let (p, q) = (v(0.0, 0.0, 0.0), v(2.0, 0.0, 0.0));
        flat.triangle(p, q, v(1.0, 2.0, 0.0), 0);
        flat.triangle(q, p, v(1.5, 1.0, 0.0), 0);
        let found = all_crossings(&flat.scaled(scale));
        assert_eq!(found.len(), 1, "scale {scale}");
        assert_eq!(found[0].kind, CrossingKind::CoplanarOverlap);
        // Unfolded: fine.
        let mut unfolded = Soup::default();
        unfolded.triangle(p, q, v(1.0, 2.0, 0.0), 0);
        unfolded.triangle(q, p, v(1.5, -1.0, 0.0), 0);
        assert_eq!(all_crossings(&unfolded.scaled(scale)), []);
        // Folded, but not flat: they share the edge and nothing else.
        let mut up = Soup::default();
        up.triangle(p, q, v(1.0, 2.0, 0.0), 0);
        up.triangle(q, p, v(1.5, 1.0, 1e-3), 0);
        assert_eq!(all_crossings(&up.scaled(scale)), []);
    }
}

/// (f) Faces in one plane that share area, facing the same way and
/// facing each other; in a coordinate plane and in a slanted one.
#[test]
fn overlaps_in_one_plane_are_found() {
    for scale in SCALES {
        for facing in [true, false] {
            for slanted in [false, true] {
                // Slanted: the plane z = x, whose points are still exact.
                let p = |x: f64, y: f64| v(x, y, if slanted { x } else { 0.0 });
                let mut soup = Soup::default();
                soup.quad(p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0), 0);
                if facing {
                    soup.quad(p(1.0, 1.0), p(1.0, 3.0), p(3.0, 3.0), p(3.0, 1.0), 1);
                } else {
                    soup.quad(p(1.0, 1.0), p(3.0, 1.0), p(3.0, 3.0), p(1.0, 3.0), 1);
                }
                let found = all_crossings(&soup.scaled(scale));
                let what = format!("scale {scale}, facing {facing}, slanted {slanted}");
                assert!(!found.is_empty(), "{what}");
                for c in &found {
                    assert_eq!(c.kind, CrossingKind::CoplanarOverlap, "{what}");
                    assert_eq!(c.faces, [0, 1], "{what}");
                    assert!(c.depth > 0.0, "{what}");
                    // In the common square from (1,1) to (2,2).
                    let at = c.at * (1.0 / scale);
                    assert!(
                        at.x > 1.0 && at.x < 2.0 && at.y > 1.0 && at.y < 2.0,
                        "{c:?}"
                    );
                }
                // The common square is one unit wide, so no part of it is
                // wider than half a unit; the halves the triangles cut it
                // into are 0.29 wide (legs of 1: twice the area, 1, over
                // the perimeter, 2 + sqrt 2) or wider where it is slanted.
                let deepest = found.iter().map(|c| c.depth).fold(0.0, f64::max);
                assert!(
                    deepest > 0.29 * scale && deepest <= 0.5 * scale,
                    "{what}: {deepest}"
                );

                // Side by side in the plane: an edge in common, no area.
                let mut beside = Soup::default();
                beside.quad(p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0), 0);
                beside.quad(p(2.0, 0.5), p(4.0, 0.5), p(4.0, 1.5), p(2.0, 1.5), 1);
                assert_eq!(all_crossings(&beside.scaled(scale)), [], "{what}");
            }
        }
        // A limit of the search on a bare mesh, pinned here so that it is
        // not forgotten: the same two squares turned out of the
        // coordinate planes are no longer in one plane to the last bit.
        // They then weave through each other by a rounding error, which
        // is not a crossing deeper than rounding and not an overlap
        // either. `body_self_intersections` knows the modelling
        // tolerance of the body and does catch it
        // (crates/testkit/tests/self_intersection.rs).
        let mut soup = Soup::default();
        let p = |x: f64, y: f64| v(x, y, 0.0);
        soup.quad(p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0), 0);
        soup.quad(p(1.0, 1.0), p(3.0, 1.0), p(3.0, 3.0), p(1.0, 3.0), 1);
        let mut askew_mesh = soup.moved(&askew());
        for q in &mut askew_mesh.positions {
            *q = *q * scale;
        }
        assert_eq!(all_crossings(&askew_mesh), [], "scale {scale}");

        // The same face twice.
        let mut twice = Soup::default();
        for face in 0..2 {
            twice.triangle(v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0), face);
        }
        let found = all_crossings(&twice.scaled(scale));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, CrossingKind::CoplanarOverlap);
    }
}

/// (g) A spike through a plate, by a little more and a little less than
/// the tolerance.
#[test]
fn a_spike_is_found_when_it_is_deeper_than_the_tolerance() {
    for scale in SCALES {
        for tolerance in [1e-2, 1e-4, 1e-6] {
            let depth_of = |through: f64| {
                let mut soup = Soup::default();
                soup.boxed(v(-2.0, -2.0, -1.0), v(2.0, 2.0, 0.0), 0);
                spike(&mut soup, v(0.5, 0.25, -through), 6);
                let found =
                    self_intersections(&soup.scaled(scale), tolerance * scale, 100).unwrap();
                for c in &found {
                    // The plate's top against the spike.
                    assert_eq!(c.faces[0], 1, "{c:?}");
                    assert!(c.faces[1] >= 6, "{c:?}");
                }
                found.iter().map(|c| c.depth / scale).fold(0.0, f64::max)
            };
            let what = format!("scale {scale}, tolerance {tolerance}");
            // Within a tenth of the tolerance on either side.
            let above = depth_of(1.1 * tolerance);
            assert!(
                (above - 1.1 * tolerance).abs() < 1e-6 * tolerance,
                "{what}: a depth of {above}"
            );
            assert_eq!(depth_of(0.9 * tolerance), 0.0, "{what}");
            // Far through: found, with a depth of at least four times the
            // tolerance (beyond which it is a lower bound).
            assert!(depth_of(0.5) >= 4.0 * tolerance, "{what}");
        }
    }
}

/// The handle and the spout of the Utah teapot go into its body, which
/// has no holes for them: a real mesh with real crossings.
#[test]
fn the_teapot_crosses_itself_where_it_is_known_to() {
    let patches = teapot::teapot().unwrap();
    let params = TessParams {
        chordal_tolerance: 1e-2,
        ..TessParams::default()
    };
    let mesh = tessellate_patches(&patches, &params).unwrap();
    let report = self_intersection_report(&mesh, 1e-2, 100_000, 0).unwrap();
    assert!(!report.truncated);
    let pairs = face_pairs(&report.crossings);
    // Order of the patches: rim (4), body (8), lid (8), bottom (4),
    // handle (4), spout (4).
    assert!(!pairs.is_empty());
    for [f, g] in &pairs {
        assert!((4..12).contains(f), "patch {f} is not of the body");
        assert!((24..32).contains(g), "patch {g} is not handle or spout");
    }
    let touched: BTreeSet<u32> = pairs.iter().map(|p| p[1]).collect();
    // Both halves of the handle at both ends, and of the spout.
    for patch in 24..30 {
        assert!(touched.contains(&patch), "patch {patch}: {touched:?}");
    }
}

// --------------------------------------------------------- the machinery

/// Möller and Trumbore's ray against a triangle, in plain `f64`: the
/// segment `p q` meets the triangle `t`. Good enough for triangles in
/// general position, and written without a look at the code under test.
fn segment_hits(p: Vec3, q: Vec3, t: [Vec3; 3]) -> bool {
    let (e1, e2, d) = (t[1] - t[0], t[2] - t[0], q - p);
    let h = d.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1e-300 {
        return false;
    }
    let s = p - t[0];
    let u = s.dot(h) / det;
    let w = s.cross(e1);
    let (vv, along) = (d.dot(w) / det, e2.dot(w) / det);
    u >= 0.0 && vv >= 0.0 && u + vv <= 1.0 && (0.0..=1.0).contains(&along)
}

fn brute_force(mesh: &Mesh) -> BTreeSet<[usize; 2]> {
    let n = mesh.triangles.len();
    let mut out = BTreeSet::new();
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (mesh.corners(i).unwrap(), mesh.corners(j).unwrap());
            let edges =
                |x: [Vec3; 3], y: [Vec3; 3]| (0..3).any(|k| segment_hits(x[k], x[(k + 1) % 3], y));
            if edges(a, b) || edges(b, a) {
                out.insert([i, j]);
            }
        }
    }
    out
}

struct Rng(u64);

impl Rng {
    fn unit(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn point(&mut self, size: f64) -> Vec3 {
        v(self.unit() * size, self.unit() * size, self.unit() * size)
    }
}

fn random_soup(seed: u64, count: usize, spread: f64) -> Mesh {
    let mut rng = Rng(seed);
    let mut soup = Soup::default();
    for k in 0..count {
        let at = rng.point(spread);
        soup.triangle(
            at + rng.point(1.0),
            at + rng.point(1.0),
            at + rng.point(1.0),
            k as u32,
        );
    }
    soup.mesh
}

/// Loose triangles thrown about: the search, with its hierarchy and its
/// exact predicates, finds the pairs that testing every pair with plain
/// arithmetic finds.
#[test]
fn a_soup_of_triangles_matches_brute_force() {
    let mut total = 0;
    for seed in 1..=6 {
        for scale in SCALES {
            let mut mesh = random_soup(seed, 400, 4.0);
            for p in &mut mesh.positions {
                *p = *p * scale;
            }
            let found: BTreeSet<[usize; 2]> =
                all_crossings(&mesh).iter().map(|c| c.triangles).collect();
            let expected = brute_force(&mesh);
            assert_eq!(found, expected, "seed {seed}, scale {scale}");
            total += expected.len();
        }
    }
    // The test means something only when a good many pairs cross.
    assert!(total > 2000, "{total}");
}

#[test]
fn the_result_does_not_depend_on_the_number_of_threads() {
    // Two balls through each other, and a soup: many crossings, and more
    // triangles than a thread takes at a time.
    let mut balls = Soup::default();
    balls.ball(v(0.0, 0.0, 0.0), 1.0, 48, 0);
    balls.ball(v(0.7, 0.3, 0.2), 1.0, 40, 1);
    let soup = random_soup(9, 6000, 9.0);
    for (mesh, limit) in [(&balls.mesh, 100_000), (&soup, 100_000), (&soup, 37)] {
        assert!(mesh.triangles.len() > 5000);
        let one = self_intersection_report(mesh, 0.0, limit, 1).unwrap();
        assert!(one.crossings.len() >= 37, "{}", one.crossings.len());
        assert_eq!(one.truncated, limit == 37);
        for threads in [2, 3, 5, 16, 0] {
            let many = self_intersection_report(mesh, 0.0, limit, threads).unwrap();
            // Bit for bit: `Crossing` compares its numbers exactly.
            assert_eq!(many, one, "{threads} threads");
        }
        // In triangle order.
        let order: Vec<[usize; 2]> = one.crossings.iter().map(|c| c.triangles).collect();
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(order, sorted);
    }
    // The balls cross in a ring: every crossing is between the two.
    let found = all_crossings(&balls.mesh);
    assert_eq!(face_pairs(&found), BTreeSet::from([[0, 1]]));
}

#[test]
fn triangles_without_area_are_counted_and_left_out() {
    let mut soup = Soup::default();
    soup.boxed(v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), 0);
    // Through the box, but no triangles: a point, a needle with two
    // corners at one place, three corners on one line.
    let p = v(0.5, 0.5, -1.0);
    soup.triangle(p, p, p, 9);
    soup.triangle(p, v(0.5, 0.5, 2.0), p, 9);
    soup.triangle(p, v(0.5, 0.5, 0.5), v(0.5, 0.5, 2.0), 9);
    let report = self_intersection_report(&soup.mesh, 0.0, 10, 1).unwrap();
    assert_eq!(report.degenerate_triangles, 3);
    assert_eq!(report.crossings, []);
    // Nothing but such triangles, and nothing at all.
    let mut needles = Soup::default();
    needles.triangle(p, p, v(1.0, 2.0, 3.0), 0);
    let report = self_intersection_report(&needles.mesh, 0.0, 10, 0).unwrap();
    assert_eq!(
        (report.degenerate_triangles, report.crossings.len()),
        (1, 0)
    );
    assert_eq!(self_intersections(&Mesh::default(), 0.0, 10).unwrap(), []);
}

#[test]
fn the_limit_caps_the_result() {
    let mut soup = Soup::default();
    soup.boxed(v(0.0, 0.0, 0.0), v(4.0, 3.0, 2.0), 0);
    soup.boxed(v(1.0, 1.0, 1.0), v(6.0, 5.0, 4.0), 6);
    let all = all_crossings(&soup.mesh);
    assert!(all.len() > 4);
    for limit in [0, 1, 4] {
        let report = self_intersection_report(&soup.mesh, 0.0, limit, 0).unwrap();
        assert_eq!(report.crossings, all[..limit]);
        assert!(report.truncated);
    }
    let report = self_intersection_report(&soup.mesh, 0.0, all.len(), 0).unwrap();
    assert!(!report.truncated);
    // A great many triangles all through each other: the answer is cut
    // off, and it does not take the time of looking at every pair twice.
    let mut pile = Soup::default();
    for k in 0..3000 {
        let z = k as f64 * 1e-4;
        pile.triangle(v(0.0, 0.0, z), v(1.0, 0.0, 1.0 - z), v(0.0, 1.0, 0.5), 0);
    }
    let report = self_intersection_report(&pile.mesh, 0.0, 5, 0).unwrap();
    assert_eq!(report.crossings.len(), 5);
    assert!(report.truncated);
}

#[test]
fn bad_input_is_an_error_not_a_crash() {
    let mut soup = Soup::default();
    soup.boxed(v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), 0);
    let good = soup.mesh;
    for tolerance in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(
            self_intersections(&good, tolerance, 10),
            Err(TessError::InvalidTolerance(_))
        ));
    }
    let mut dangling = good.clone();
    dangling.triangles[3] = [0, 1, 999_999];
    let error = self_intersections(&dangling, 0.0, 10).unwrap_err();
    assert!(error.to_string().contains("triangle 3"), "{error}");
    for bad in [f64::NAN, f64::INFINITY, -f64::INFINITY, 1e200] {
        let mut mesh = good.clone();
        mesh.positions[7].y = bad;
        let error = self_intersections(&mesh, 0.0, 10).unwrap_err();
        assert!(error.to_string().contains("vertex 7"), "{error}");
    }
    let mut short = good.clone();
    short.face_ids.pop();
    assert!(self_intersections(&short, 0.0, 10).is_err());
    // Coordinates at the edge of what a number can hold: an answer or an
    // error, never a panic.
    for size in [1e-300, 1e-160, 1e99] {
        let mut soup = Soup::default();
        soup.boxed(v(0.0, 0.0, 0.0), v(4.0, 3.0, 2.0), 0);
        soup.boxed(v(1.0, 1.0, 1.0), v(6.0, 5.0, 4.0), 6);
        let _ = self_intersections(&soup.scaled(size), 0.0, 10);
    }
}

/// The time grows with the mesh, not with its square: four times the
/// triangles may not take sixteen times as long. (A loose bound that a
/// busy machine does not break; the real numbers are in the ignored test
/// below.)
#[test]
fn the_time_does_not_grow_with_the_square() {
    let time = |n: usize| {
        let mut soup = Soup::default();
        soup.ball(Vec3::ZERO, 1.0, n, 0);
        let mut best = f64::INFINITY;
        for _ in 0..3 {
            let start = Instant::now();
            let report = self_intersection_report(&soup.mesh, 0.0, 10, 1).unwrap();
            best = best.min(start.elapsed().as_secs_f64());
            assert_eq!(report.crossings, []);
        }
        (soup.mesh.triangles.len(), best)
    };
    let (small, t_small) = time(60);
    let (large, t_large) = time(120);
    assert!(large > 3 * small);
    assert!(
        t_large < 10.0 * t_small + 0.05,
        "{small} triangles took {t_small} s, {large} took {t_large} s"
    );
}

/// A million triangles of a valid body, in release:
/// `cargo test --release -p craft3d-tess --test self_intersections -- --ignored --nocapture`
///
/// The budget: 5 s on one thread and 2 s on all of them, for a search
/// that finds nothing (the usual case). Measured on the development
/// machine (16 threads): see the dev log.
#[test]
#[ignore = "a million triangles; run in release with --ignored --nocapture"]
fn a_million_triangles() {
    let torus = solids::torus(&Frame::WORLD, 3.0, 1.0).unwrap();
    let params = TessParams {
        chordal_tolerance: 3.6e-5,
        max_segments: 8192,
    };
    let start = Instant::now();
    let mesh = tessellate_body_with_threads(&torus.body, &params, 0).unwrap();
    println!(
        "tessellated a torus: {} triangles in {:.2} s",
        mesh.triangles.len(),
        start.elapsed().as_secs_f64()
    );
    assert!(mesh.triangles.len() >= 1_000_000);
    let time = |threads: usize| {
        let mut best = f64::INFINITY;
        for _ in 0..3 {
            let start = Instant::now();
            let report = self_intersection_report(&mesh, 0.0, 100, threads).unwrap();
            best = best.min(start.elapsed().as_secs_f64());
            assert_eq!(report.crossings, []);
            assert_eq!(report.degenerate_triangles, 0);
        }
        best
    };
    let (single, auto) = (time(1), time(0));
    println!("search: {single:.2} s on one thread, {auto:.2} s on all");
    assert!(single < 5.0, "{single} s on one thread");
    assert!(auto < 2.0, "{auto} s on all threads");

    // And with something to find: the same torus twice, the second a
    // little aside, a million triangles each.
    let mut both = mesh.clone();
    let base = both.positions.len() as u32;
    both.positions
        .extend(mesh.positions.iter().map(|&p| p + v(0.5, 0.25, 0.125)));
    both.normals.extend(mesh.normals.iter().copied());
    both.triangles
        .extend(mesh.triangles.iter().map(|t| t.map(|k| k + base)));
    both.face_ids.extend(mesh.face_ids.iter().map(|_| 1));
    let start = Instant::now();
    let report = self_intersection_report(&both, 1e-3, 1000, 0).unwrap();
    println!(
        "two tori through each other: {} triangles, {} crossings{} in {:.2} s",
        both.triangles.len(),
        report.crossings.len(),
        if report.truncated { " (cut off)" } else { "" },
        start.elapsed().as_secs_f64()
    );
    assert_eq!(report.crossings.len(), 1000);
    assert!(report.crossings.iter().all(|c| c.faces == [0, 1]));
}
