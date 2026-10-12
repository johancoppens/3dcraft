//! Tessellator: surfaces to triangle meshes within a chordal tolerance.
//!
//! This first version handles untrimmed Bézier patches on a uniform parameter
//! grid, the same grid for every patch. Vertices on a seam between two
//! patches that share a whole edge coincide exactly, but are separate entries
//! in the mesh: it has no connectivity across patches yet. Shared edges (and
//! with them watertight solids) arrive with the B-rep tessellator
//! (docs/architecture.md §6).

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
}

impl Default for TessParams {
    fn default() -> Self {
        Self {
            chordal_tolerance: 0.01,
            max_segments: 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TessError {
    InvalidTolerance(f64),
    TooFine {
        patch: usize,
        needed_segments: f64,
        max_segments: u32,
    },
    TooManyVertices,
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

/// Tessellates `patches` into one mesh. Triangle `face_ids` are the patch
/// indices. Triangles are wound so their geometric normal agrees with the
/// patch normal `∂u × ∂v`.
pub fn tessellate_patches(patches: &[BezierPatch], params: &TessParams) -> Result<Mesh, TessError> {
    let tol = params.chordal_tolerance;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(TessError::InvalidTolerance(tol));
    }
    // One segment count for all patches, the finest any of them needs:
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
        for i in 0..side {
            for j in 0..side {
                let u = f64::from(i) / f64::from(n);
                let v = f64::from(j) / f64::from(n);
                mesh.positions.push(patch.eval(u, v));
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

    /// Regression: patches that share an edge were meshed with different
    /// segment counts, leaving T-junctions (pinholes along teapot seams).
    #[test]
    fn patches_sharing_an_edge_get_the_same_vertices_along_it() {
        let curved = dome(1.0);
        // A ruled strip continuing from the dome's v = 1 edge: much flatter.
        let mut net = [[Vec3::ZERO; 4]; 4];
        for (i, row) in net.iter_mut().enumerate() {
            let start = curved.control_point(i, 3).unwrap();
            for (j, p) in row.iter_mut().enumerate() {
                *p = start + Vec3::new(0.0, j as f64 / 3.0, 0.0);
            }
        }
        let strip = BezierPatch::bicubic(net).unwrap();
        let mesh =
            tessellate_patches(&[curved.clone(), strip.clone()], &TessParams::default()).unwrap();
        let bits = |p: &Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
        let on_edge = |face: u32, edge_v: f64, patch: &BezierPatch| {
            let mut pts = std::collections::BTreeSet::new();
            for (tri, &id) in mesh.triangles.iter().zip(&mesh.face_ids) {
                for &k in tri {
                    let p = mesh.positions[k as usize];
                    // Vertices of this face that lie on its v = edge_v curve.
                    let on_curve =
                        (0..=4096).any(|s| patch.eval(f64::from(s) / 4096.0, edge_v) == p);
                    if id == face && on_curve {
                        pts.insert(bits(&p));
                    }
                }
            }
            pts
        };
        let (a, b) = (on_edge(0, 1.0, &curved), on_edge(1, 0.0, &strip));
        assert!(a.len() > 2);
        assert_eq!(a, b, "seam vertices differ: {} vs {}", a.len(), b.len());
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
