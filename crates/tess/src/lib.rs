//! Tessellator: surfaces to triangle meshes within a chordal tolerance.
//!
//! Two entry points. [`tessellate_patches`] handles untrimmed Bézier
//! patches on a uniform parameter grid, the same grid for every patch;
//! vertices on a seam between two patches that share a whole edge coincide
//! exactly, but are separate entries in the mesh. [`tessellate_body`]
//! handles B-rep bodies, edges first, so that the mesh of a valid solid is
//! watertight (docs/design/tessellator.md); see `body.rs` for what it covers.

mod body;
mod cdt;
mod edge;
mod exact;
mod face;
mod intersections;
mod long;
mod on_surface;
mod remembered;
mod selfcheck;
mod weld;

pub use body::{
    MeshAndEdges, WithinBudget, tess_work, tessellate_body, tessellate_body_and_edges,
    tessellate_body_with_threads, tessellate_edges, tessellate_within_budget,
};
pub use intersections::{
    Crossing, CrossingKind, SelfIntersections, self_intersection_report, self_intersections,
};
pub use on_surface::{OffSurface, off_surfaces};
pub use remembered::{forget_kept_meshes, kept_meshes};
pub use selfcheck::{
    FaceCrossing, FaceRole, SELF_CHECK_WORK, SelfCheck, SelfCheckParams, ShellDefect, ShellFinding,
    Touch, Touched, body_self_intersections, body_self_intersections_near, body_touches_itself,
};
pub use weld::Welded;

use craft3d_geom::BezierPatch;
use craft3d_math::{Bbox3, Vec3};
use std::fmt;

/// A triangle mesh with one face id per triangle, so renderers and exporters
/// can tell which surface a triangle came from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    /// One per position. A zero vector means "no normal here": consumers
    /// fall back to the triangle's own normal.
    pub normals: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
    /// One per triangle.
    pub face_ids: Vec<u32>,
}

impl Mesh {
    /// The three corner positions of triangle `t`.
    pub fn corners(&self, t: usize) -> Option<[Vec3; 3]> {
        let [a, b, c] = *self.triangles.get(t)?;
        Some([
            *self.positions.get(a as usize)?,
            *self.positions.get(b as usize)?,
            *self.positions.get(c as usize)?,
        ])
    }

    pub fn bbox(&self) -> Bbox3 {
        Bbox3::from_points(self.positions.iter().copied())
    }

    /// Total area of all triangles.
    pub fn area(&self) -> f64 {
        (0..self.triangles.len())
            .filter_map(|t| self.corners(t))
            .map(|[a, b, c]| (b - a).cross(c - a).length() * 0.5)
            .sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TessParams {
    /// Largest allowed distance between the mesh and the surface, in model
    /// units.
    pub chordal_tolerance: f64,
    /// Upper limit on grid segments per patch direction. A tolerance that
    /// needs more is an error, never a silent coarser mesh.
    pub max_segments: u32,
    /// A triangle budget, or `None` for no limit (the default). A mesh
    /// that would have more triangles is not made:
    /// [`TessError::OverBudget`], as soon as the faces done so far show
    /// it, without tessellating the rest. A mesh within the budget is the
    /// very mesh made without one. For a viewer that tries a fine
    /// tolerance first and a coarser one when that is too much.
    pub max_triangles: Option<usize>,
    /// An angle criterion beside the chordal tolerance, in radians, or
    /// `None` for none (the default: the mesh is then what the chordal
    /// tolerance alone makes, bit for bit). With it, no chord of an edge
    /// turns the curve's tangent by more than this between its ends, and
    /// no triangle of a curved face has corners whose surface normals are
    /// further apart than this: a small hole stays round at a chordal
    /// tolerance that alone would make it a polygon of a few sides. A
    /// chord or a side shorter than the chordal tolerance
    /// ([`ANGLE_FLOOR`] times it) is not held to the angle, so the angle
    /// never asks for more than a fine tolerance would, and stays free
    /// of any length of its own. For display meshes; export keeps the
    /// chordal tolerance alone unless asked.
    pub max_angle: Option<f64>,
}

/// Chords and sides of triangles shorter than this many chordal
/// tolerances are not held to [`TessParams::max_angle`]: an edge around a
/// radius below about `ANGLE_FLOOR / max_angle` tolerances is as round as
/// the tolerance makes it, and refinement for the angle ends where the
/// tolerance would have ended it.
pub const ANGLE_FLOOR: f64 = 1.0;

impl Default for TessParams {
    fn default() -> Self {
        Self {
            chordal_tolerance: 0.01,
            max_segments: 1024,
            max_triangles: None,
            max_angle: None,
        }
    }
}

impl TessParams {
    /// Refuses parameters that are no request: a tolerance that is not a
    /// positive number, an angle that is not one or is beyond a turn.
    pub(crate) fn validate(&self) -> Result<(), TessError> {
        let tol = self.chordal_tolerance;
        if !(tol.is_finite() && tol > 0.0) {
            return Err(TessError::InvalidTolerance(tol));
        }
        if let Some(angle) = self.max_angle
            && !(angle.is_finite() && angle > 0.0 && angle <= std::f64::consts::TAU)
        {
            return Err(TessError::InvalidAngle(angle));
        }
        Ok(())
    }

    /// The angle a chord of `length` is held to, if any: none below the
    /// floor.
    pub(crate) fn angle_for(&self, length: f64) -> Option<f64> {
        self.max_angle
            .filter(|_| length >= ANGLE_FLOOR * self.chordal_tolerance)
    }
}

/// Whether a turn is beyond the angle criterion. A millionth of the angle
/// is rounding: an arc cut in chords of exactly the angle turns by it to
/// the last bits either way, and a face must not ask to cut its edges
/// again for that.
pub(crate) fn turns_beyond(turn: f64, angle: f64) -> bool {
    turn > angle * (1.0 + 1e-6)
}

/// The angle between two vectors, in radians, or `None` when either has
/// no direction.
pub(crate) fn angle_between(a: Vec3, b: Vec3) -> Option<f64> {
    let cross = a.cross(b).length();
    let dot = a.dot(b);
    let angle = cross.atan2(dot);
    (a.length_squared() > 0.0 && b.length_squared() > 0.0 && angle.is_finite()).then_some(angle)
}

#[derive(Clone, Debug, PartialEq)]
pub enum TessError {
    InvalidTolerance(f64),
    /// `TessParams::max_angle` is not an angle in radians above zero and
    /// at most a turn.
    InvalidAngle(f64),
    /// The tolerance asks for more points than `TessParams::max_segments`
    /// allows. `patch` is the index of the patch, the face or (when the
    /// edges are being turned into points) the edge. `needed_segments` is
    /// infinite when the tolerance is finer than the face's coordinates
    /// can resolve (no point fits between the corners of a triangle that
    /// is still as large as the tolerance): there is no count to give.
    TooFine {
        patch: usize,
        needed_segments: f64,
        max_segments: u32,
    },
    TooManyVertices,
    /// The mesh would have more triangles than
    /// [`TessParams::max_triangles`]: the first `faces` faces, in face
    /// order, make `triangles` already, and the rest was not tessellated.
    /// The numbers depend on the body and the parameters only, not on the
    /// number of threads or on which of them was first.
    OverBudget {
        triangles: usize,
        faces: usize,
        max_triangles: usize,
    },
    /// The body refers to something that does not exist; run the checker.
    InvalidBody(String),
    /// A face or edge this tessellator cannot handle yet.
    Unsupported {
        face: usize,
        what: String,
    },
    /// A face that cannot be tessellated as it is given: its loops do not
    /// close, do not bound a region, or cross.
    Face {
        face: usize,
        reason: String,
    },
    /// The tessellator itself went wrong (a bug); nothing was produced.
    Internal(String),
    /// The work bound of the operation the mesh is made for
    /// (`craft3d_geom::allowance`) was reached before the mesh was done.
    Spent,
}

impl fmt::Display for TessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TessError::InvalidTolerance(t) => {
                write!(
                    f,
                    "chordal tolerance must be a positive finite number, got {t}"
                )
            }
            TessError::InvalidAngle(a) => write!(
                f,
                "the angle criterion must be an angle above 0 and at most a turn (in radians), \
                 got {a}"
            ),
            TessError::TooFine {
                patch,
                needed_segments,
                ..
            } if !needed_segments.is_finite() => write!(
                f,
                "patch {patch} asks for a tolerance finer than its coordinates can resolve; use \
                 a larger tolerance"
            ),
            TessError::TooFine {
                patch,
                needed_segments,
                max_segments,
            } => write!(
                f,
                "patch {patch} needs about {needed_segments:.0} segments per side for this \
                 tolerance, above the limit of {max_segments}; use a larger tolerance"
            ),
            TessError::TooManyVertices => write!(f, "mesh would exceed 2^32 vertices"),
            TessError::OverBudget {
                triangles,
                faces,
                max_triangles,
            } => write!(
                f,
                "the mesh has {triangles} triangles after {faces} faces, over the budget of \
                 {max_triangles}; use a coarser tolerance"
            ),
            TessError::InvalidBody(what) => write!(f, "invalid body: {what}"),
            TessError::Unsupported { face, what } => {
                write!(f, "face {face} cannot be tessellated yet: {what}")
            }
            TessError::Face { face, reason } => {
                write!(f, "face {face} cannot be tessellated: {reason}")
            }
            TessError::Internal(what) => write!(f, "internal tessellator error: {what}"),
            TessError::Spent => write!(
                f,
                "the mesh needs more work than the operation it is made for is allowed"
            ),
        }
    }
}

impl std::error::Error for TessError {}

/// Grid segments per direction so that the piecewise-linear mesh of `patch`
/// stays within `tolerance` of the surface.
///
/// Bound from Filip, Magedson & Markot, "Surface algorithms using bounds on
/// derivatives" (CAGD 3, 1986): linear interpolation over a right triangle
/// with legs `h` in parameter space deviates by at most
/// `(Muu + 2 Muv + Mvv) h² / 8`.
fn segments_for(patch: &BezierPatch, index: usize, params: &TessParams) -> Result<u32, TessError> {
    let (muu, muv, mvv) = patch.second_derivative_bounds();
    let m = muu + 2.0 * muv + mvv;
    let needed = (m / (8.0 * params.chordal_tolerance)).sqrt().ceil();
    if !needed.is_finite() || needed > f64::from(params.max_segments) {
        return Err(TessError::TooFine {
            patch: index,
            needed_segments: needed,
            max_segments: params.max_segments,
        });
    }
    // In range by the check above; at least one segment.
    Ok((needed as u32).max(1))
}

/// One boundary edge of a patch, prepared so that every patch sharing the
/// edge computes the same vertices along it, bit for bit.
struct BoundaryEdge {
    /// The edge's control points, in an order that does not depend on which
    /// way the patch runs along the edge.
    ctrl: Vec<Vec3>,
    /// True when that order is the reverse of the patch's own.
    reversed: bool,
    /// The point the edge is collapsed to, when all control points coincide.
    collapsed: Option<Vec3>,
}

impl BoundaryEdge {
    /// `ctrl` in the patch's parameter order along the edge.
    fn new(ctrl: Vec<Vec3>) -> Self {
        let key = |p: &Vec3| [p.x, p.y, p.z];
        // Control points are finite, so the comparison is total.
        let reversed = ctrl.iter().rev().map(key).lt(ctrl.iter().map(key));
        let collapsed = ctrl
            .first()
            .copied()
            .filter(|first| ctrl.iter().all(|p| p == first));
        let ctrl = if reversed {
            ctrl.into_iter().rev().collect()
        } else {
            ctrl
        };
        Self {
            ctrl,
            reversed,
            collapsed,
        }
    }

    /// Vertex `k` of `n` segments along the edge, counted in the patch's
    /// direction. Evaluated by de Casteljau on the edge's own control
    /// points in their canonical order, so the result depends on nothing
    /// else of the patch; on a collapsed edge it is the control point
    /// itself, where evaluating would scatter the vertices by rounding.
    fn vertex(&self, k: u32, n: u32) -> Vec3 {
        if let Some(point) = self.collapsed {
            return point;
        }
        let k = if self.reversed { n - k.min(n) } else { k };
        let t = f64::from(k) / f64::from(n);
        let mut points = self.ctrl.clone();
        while points.len() > 1 {
            points = points
                .iter()
                .zip(points.iter().skip(1))
                .map(|(a, b)| a.lerp(*b, t))
                .collect();
        }
        points.first().copied().unwrap_or(Vec3::ZERO)
    }
}

/// The boundary edges u = 0, u = 1, v = 0 and v = 1 of `patch`.
fn boundary_edges(patch: &BezierPatch) -> [BoundaryEdge; 4] {
    let (m, n) = (patch.degree_u(), patch.degree_v());
    let along_v = |i: usize| (0..=n).filter_map(|j| patch.control_point(i, j)).collect();
    let along_u = |j: usize| (0..=m).filter_map(|i| patch.control_point(i, j)).collect();
    [
        BoundaryEdge::new(along_v(0)),
        BoundaryEdge::new(along_v(m)),
        BoundaryEdge::new(along_u(0)),
        BoundaryEdge::new(along_u(n)),
    ]
}

/// Tessellates `patches` into one mesh. Triangle `face_ids` are the patch
/// indices. Triangles are wound so their geometric normal agrees with the
/// patch normal `∂u × ∂v`.
pub fn tessellate_patches(patches: &[BezierPatch], params: &TessParams) -> Result<Mesh, TessError> {
    params.validate()?;
    // One segment count for all patches, the finest any of them needs, and
    // boundary vertices computed from the edge alone (`BoundaryEdge`):
    // patches that share a whole edge then get the same vertices along it,
    // bit for bit, so the seams have no T-junctions. (Wasteful on flat
    // patches; the B-rep tessellator will discretise each edge once instead.)
    let mut n = 1;
    for (index, patch) in patches.iter().enumerate() {
        n = n.max(segments_for(patch, index, params)?);
    }
    let mut mesh = Mesh::default();
    for (index, patch) in patches.iter().enumerate() {
        let face_id = u32::try_from(index).map_err(|_| TessError::TooManyVertices)?;
        let side = n + 1;
        let base = u32::try_from(mesh.positions.len()).map_err(|_| TessError::TooManyVertices)?;
        // side² ≤ (max_segments + 1)², checked against the index range here.
        let end = u64::from(base) + u64::from(side) * u64::from(side);
        if end > u64::from(u32::MAX) {
            return Err(TessError::TooManyVertices);
        }
        let [u0, u1, v0, v1] = boundary_edges(patch);
        for i in 0..side {
            for j in 0..side {
                let u = f64::from(i) / f64::from(n);
                let v = f64::from(j) / f64::from(n);
                // Boundary vertices come from the edge alone, so patches
                // sharing it agree exactly; corners are control points.
                let position = if i == 0 {
                    u0.vertex(j, n)
                } else if i == n {
                    u1.vertex(j, n)
                } else if j == 0 {
                    v0.vertex(i, n)
                } else if j == n {
                    v1.vertex(i, n)
                } else {
                    patch.eval(u, v)
                };
                mesh.positions.push(position);
                mesh.normals.push(patch.normal(u, v).unwrap_or(Vec3::ZERO));
            }
        }
        for i in 0..n {
            for j in 0..n {
                let a = base + i * side + j;
                let b = a + side; // (i + 1, j)
                let c = b + 1; // (i + 1, j + 1)
                let d = a + 1; // (i, j + 1)
                for tri in [[a, b, c], [a, c, d]] {
                    // Skip triangles collapsed to a line or point (the apex
                    // of a patch with a degenerate edge).
                    let degenerate = match tri.map(|k| mesh.positions.get(k as usize)) {
                        [Some(&p), Some(&q), Some(&r)] => {
                            (q - p).cross(r - p).length_squared() == 0.0
                        }
                        _ => true,
                    };
                    if !degenerate {
                        mesh.triangles.push(tri);
                        mesh.face_ids.push(face_id);
                    }
                }
            }
        }
        // Patches are made one after the other: the first that takes the
        // mesh over the budget ends it.
        if let Some(max_triangles) = params.max_triangles
            && mesh.triangles.len() > max_triangles
        {
            return Err(TessError::OverBudget {
                triangles: mesh.triangles.len(),
                faces: index.saturating_add(1),
                max_triangles,
            });
        }
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_square(size: f64) -> BezierPatch {
        let mut net = [[Vec3::ZERO; 4]; 4];
        for (i, row) in net.iter_mut().enumerate() {
            for (j, p) in row.iter_mut().enumerate() {
                *p = Vec3::new(i as f64 * size / 3.0, j as f64 * size / 3.0, 0.0);
            }
        }
        BezierPatch::bicubic(net).unwrap()
    }

    /// One eighth of a "sphere-like" dome: strongly curved in both directions.
    fn dome(scale: f64) -> BezierPatch {
        let mut net = [[Vec3::ZERO; 4]; 4];
        for (i, row) in net.iter_mut().enumerate() {
            for (j, p) in row.iter_mut().enumerate() {
                let (x, y) = (i as f64 / 3.0, j as f64 / 3.0);
                *p = Vec3::new(x, y, 1.0 - x * x - 0.5 * y * y + 0.3 * x * y) * scale;
            }
        }
        BezierPatch::bicubic(net).unwrap()
    }

    #[test]
    fn a_flat_patch_needs_one_segment_and_has_exact_area() {
        let mesh = tessellate_patches(&[flat_square(2.0)], &TessParams::default()).unwrap();
        assert_eq!(mesh.positions.len(), 4);
        assert_eq!(mesh.triangles.len(), 2);
        assert_eq!(mesh.face_ids, vec![0, 0]);
        assert!((mesh.area() - 4.0).abs() < 1e-12);
        for n in &mesh.normals {
            assert!(n.distance(Vec3::Z) < 1e-12);
        }
    }

    #[test]
    fn winding_agrees_with_the_patch_normal() {
        let patch = dome(1.0);
        let mesh =
            tessellate_patches(std::slice::from_ref(&patch), &TessParams::default()).unwrap();
        for t in 0..mesh.triangles.len() {
            let [a, b, c] = mesh.corners(t).unwrap();
            let geometric = (b - a).cross(c - a).normalized().unwrap();
            let [ia, _, _] = mesh.triangles[t];
            assert!(geometric.dot(mesh.normals[ia as usize]) > 0.5);
        }
    }

    /// Oracle 4 (docs/testing.md): the mesh stays within the chordal
    /// tolerance, at several tolerances and model scales.
    #[test]
    fn deviation_is_within_tolerance() {
        for scale in [1e-3, 1.0, 1e4] {
            for rel_tol in [1e-2, 1e-3, 1e-4] {
                let patch = dome(scale);
                let tol = rel_tol * scale;
                let params = TessParams {
                    chordal_tolerance: tol,
                    ..TessParams::default()
                };
                let mesh = tessellate_patches(std::slice::from_ref(&patch), &params).unwrap();
                let n = ((mesh.positions.len() as f64).sqrt() - 1.0).round();
                let mut worst = 0.0_f64;
                for i in 0..n as u32 {
                    for j in 0..n as u32 {
                        // Compare each cell's two triangle centroids with the
                        // surface at the same parameters (an upper bound on
                        // the true distance).
                        let at = |di: f64, dj: f64| {
                            patch.eval((f64::from(i) + di) / n, (f64::from(j) + dj) / n)
                        };
                        let (a, b, c, d) = (at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0));
                        let lower = (a + b + c) * (1.0 / 3.0);
                        let upper = (a + c + d) * (1.0 / 3.0);
                        worst = worst.max(lower.distance(at(2.0 / 3.0, 1.0 / 3.0)));
                        worst = worst.max(upper.distance(at(1.0 / 3.0, 2.0 / 3.0)));
                    }
                }
                assert!(worst <= tol, "scale {scale} tol {tol}: deviation {worst}");
                // The bound must not be absurdly pessimistic either.
                assert!(
                    worst > tol / 50.0,
                    "scale {scale} tol {tol}: deviation only {worst}"
                );
            }
        }
    }

    #[test]
    fn area_converges_as_the_tolerance_shrinks() {
        let patch = dome(1.0);
        let area_at = |tol: f64| {
            let params = TessParams {
                chordal_tolerance: tol,
                ..TessParams::default()
            };
            tessellate_patches(std::slice::from_ref(&patch), &params)
                .unwrap()
                .area()
        };
        let (coarse, mid, fine) = (area_at(1e-2), area_at(1e-4), area_at(1e-6));
        assert!((mid - fine).abs() < (coarse - fine).abs());
        assert!((mid - fine).abs() < 1e-3);
    }

    #[test]
    fn collapsed_edges_produce_no_degenerate_triangles() {
        let apex = Vec3::new(0.0, 0.0, 1.0);
        let mut net = [[apex; 4]; 4];
        for (i, row) in net.iter_mut().enumerate().skip(1) {
            for (j, p) in row.iter_mut().enumerate() {
                let r = i as f64;
                *p = Vec3::new(r, (j as f64 - 1.5) * r, 1.0 - r * r * 0.2);
            }
        }
        let patch = BezierPatch::bicubic(net).unwrap();
        let mesh = tessellate_patches(&[patch], &TessParams::default()).unwrap();
        assert!(!mesh.triangles.is_empty());
        for t in 0..mesh.triangles.len() {
            let [a, b, c] = mesh.corners(t).unwrap();
            assert!((b - a).cross(c - a).length() > 0.0);
        }
    }

    /// Regression: along a collapsed edge the vertices came out a rounding
    /// error apart instead of identical, leaving slivers of area ~1e-16 and
    /// near-duplicate points (FreeCAD merged 67 more points than we did when
    /// it read the teapot).
    #[test]
    fn a_collapsed_edge_becomes_one_exact_point() {
        let apex = Vec3::new(0.1, 0.2, 3.15);
        let mut net = [[apex; 4]; 4];
        for (i, row) in net.iter_mut().enumerate() {
            for (j, p) in row.iter_mut().enumerate().skip(1) {
                let r = j as f64 * 0.7;
                *p = apex + Vec3::new(r * (i as f64 - 1.5), r, -0.3 * r * r);
            }
        }
        // The v = 0 edge (j = 0 for every i) is the apex.
        let patch = BezierPatch::bicubic(net).unwrap();
        let params = TessParams {
            chordal_tolerance: 1e-3,
            ..TessParams::default()
        };
        let mesh = tessellate_patches(&[patch], &params).unwrap();
        let near: Vec<&Vec3> = mesh
            .positions
            .iter()
            .filter(|p| p.distance(apex) < 1e-9)
            .collect();
        assert!(near.len() > 4);
        assert!(
            near.iter().all(|p| **p == apex),
            "apex vertices differ: {near:?}"
        );
        for t in 0..mesh.triangles.len() {
            let [a, b, c] = mesh.corners(t).unwrap();
            assert!((b - a).cross(c - a).length() > 1e-12, "sliver triangle {t}");
        }
    }

    /// Regression: patches that share an edge were meshed with different
    /// segment counts, leaving T-junctions (pinholes along teapot seams); and
    /// an edge shared in opposite parameter directions (the teapot's body
    /// and bottom) evaluated to points a rounding error apart.
    #[test]
    fn patches_sharing_an_edge_get_the_same_vertices_along_it() {
        let curved = dome(1.0);
        for reversed in [false, true] {
            // A ruled strip continuing from the dome's v = 1 edge: much
            // flatter, and optionally running the other way along the edge.
            let mut net = [[Vec3::ZERO; 4]; 4];
            for (i, row) in net.iter_mut().enumerate() {
                let along = if reversed { 3 - i } else { i };
                let start = curved.control_point(along, 3).unwrap();
                for (j, p) in row.iter_mut().enumerate() {
                    *p = start + Vec3::new(0.0, j as f64 / 3.0, 0.0);
                }
            }
            let strip = BezierPatch::bicubic(net).unwrap();
            let mesh =
                tessellate_patches(&[curved.clone(), strip], &TessParams::default()).unwrap();
            let side = ((mesh.positions.len() / 2) as f64).sqrt() as usize;
            let bits = |p: &Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
            // Dome: vertices (i, j = last). Strip: vertices (i, j = 0).
            let dome_edge: std::collections::BTreeSet<_> = (0..side)
                .map(|i| bits(&mesh.positions[i * side + side - 1]))
                .collect();
            let strip_edge: std::collections::BTreeSet<_> = (0..side)
                .map(|i| bits(&mesh.positions[side * side + i * side]))
                .collect();
            assert!(side > 3 && dome_edge.len() == side);
            assert_eq!(dome_edge, strip_edge, "reversed = {reversed}");
        }
    }

    #[test]
    fn bad_parameters_are_errors() {
        let patch = dome(1.0);
        for tol in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let params = TessParams {
                chordal_tolerance: tol,
                ..TessParams::default()
            };
            assert!(matches!(
                tessellate_patches(std::slice::from_ref(&patch), &params),
                Err(TessError::InvalidTolerance(_))
            ));
        }
        let too_fine = TessParams {
            chordal_tolerance: 1e-300,
            max_segments: 64,
            ..TessParams::default()
        };
        let err = tessellate_patches(&[patch], &too_fine).unwrap_err();
        assert!(matches!(err, TessError::TooFine { patch: 0, .. }));
        assert!(err.to_string().contains("larger tolerance"));
        assert_eq!(
            tessellate_patches(&[], &TessParams::default()).unwrap(),
            Mesh::default()
        );
    }
}
