//! Tessellation of B-rep bodies, edges first: every edge is turned into
//! points once, and every face that uses the edge uses those same points.
//! Faces therefore meet without gaps or T-junctions, and welding the mesh
//! of a valid solid gives a closed manifold (docs/architecture.md §6).
//!
//! Each face gets its own vertices, so it can carry its own normals; shared
//! positions are bit-identical and `Mesh::weld` joins them.
//!
//! Supported so far, everything else is an `Unsupported` error:
//! - edges: lines and circles;
//! - planar faces with one loop (any simple polygon, so also a disc);
//! - cylinder walls between two full circles;
//! - cones with a full circle and an apex;
//! - whole spheres and whole tori.

use crate::{Mesh, TessError, TessParams};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::Vec3;
use craft3d_topo::{Body, Coedge, Face};
use std::f64::consts::{PI, TAU};

fn unsupported<T>(face: usize, what: impl Into<String>) -> Result<T, TessError> {
    Err(TessError::Unsupported {
        face,
        what: what.into(),
    })
}

/// Segments for an arc of `sweep` radians on a circle of `radius` so that
/// no chord is further than the tolerance from the arc: a chord over a
/// half-angle `a` sags by `r (1 - cos a)`.
fn arc_segments(radius: f64, sweep: f64, params: &TessParams) -> Result<u32, TessError> {
    let sag = (params.chordal_tolerance / radius).min(1.0);
    let half_angle = (1.0 - sag).acos();
    let needed = (sweep.abs() / (2.0 * half_angle)).ceil();
    // Enough to stay a recognisable, non-degenerate polygon.
    let minimum = if sweep.abs() >= TAU - 1e-9 { 3.0 } else { 1.0 };
    let needed = needed.max(minimum);
    let limit = f64::from(params.max_segments) * 8.0;
    if !needed.is_finite() || needed > limit {
        return Err(TessError::TooFine {
            patch: 0,
            needed_segments: needed,
            max_segments: params.max_segments,
        });
    }
    // In range by the check above.
    Ok(needed as u32)
}

/// The points of every edge, from its start to its end vertex. The ends
/// are the vertices' own positions, so edges meeting at a vertex agree.
fn edge_points(body: &Body, params: &TessParams) -> Result<Vec<Vec<Vec3>>, TessError> {
    let mut all = Vec::with_capacity(body.edges.len());
    for (index, edge) in body.edges.iter().enumerate() {
        let invalid = TessError::InvalidBody(format!(
            "edge {index} refers to a vertex that does not exist"
        ));
        let (Some(start), Some(end)) = (body.vertex(edge.start), body.vertex(edge.end)) else {
            return Err(invalid);
        };
        let (t0, t1) = edge.range;
        let segments = match &edge.curve {
            Curve3::Line(_) => 1,
            Curve3::Circle(c) => arc_segments(c.radius(), t1 - t0, params)?,
            other => {
                return Err(TessError::Unsupported {
                    face: 0,
                    what: format!(
                        "edge {index} is a {}; only lines and circles can be tessellated yet",
                        other.kind()
                    ),
                });
            }
        };
        let mut points = Vec::with_capacity(segments as usize + 1);
        points.push(start.point);
        for k in 1..segments {
            points.push(
                edge.curve
                    .eval(t0 + (t1 - t0) * f64::from(k) / f64::from(segments)),
            );
        }
        points.push(end.point);
        all.push(points);
    }
    Ok(all)
}

/// The points around one loop, in loop order, without repeating the point
/// where it closes.
fn loop_points(
    body: &Body,
    edges: &[Vec<Vec3>],
    coedges: &[Coedge],
    face: usize,
) -> Result<Vec<Vec3>, TessError> {
    let mut points = Vec::new();
    for coedge in coedges {
        let Some(edge) = edges.get(coedge.edge.0 as usize) else {
            return Err(TessError::InvalidBody(format!(
                "face {face} uses edge {}, which does not exist",
                coedge.edge.0
            )));
        };
        // Each coedge contributes all its points but the last, which is
        // the first of the next coedge.
        let count = edge.len().saturating_sub(1);
        if coedge.reversed {
            points.extend(edge.iter().rev().take(count));
        } else {
            points.extend(edge.iter().take(count));
        }
    }
    let _ = body;
    Ok(points)
}

/// Twice the signed area of triangle (a, b, c) in the plane.
fn cross2(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// Triangulates a simple polygon by ear clipping. Returns index triples
/// into `polygon`, all with the polygon's own orientation.
fn ear_clip(polygon: &[(f64, f64)]) -> Option<Vec<[usize; 3]>> {
    let n = polygon.len();
    if n < 3 {
        return None;
    }
    let at = |i: usize| polygon.get(i).copied().unwrap_or((0.0, 0.0));
    let area: f64 = (0..n)
        .map(|i| at(i).0 * at((i + 1) % n).1 - at((i + 1) % n).0 * at(i).1)
        .sum();
    if !area.is_finite() || area == 0.0 {
        return None;
    }
    let sign = area.signum();
    let mut remaining: Vec<usize> = (0..n).collect();
    let mut triangles = Vec::with_capacity(n - 2);
    while remaining.len() > 3 {
        let m = remaining.len();
        let pick = |k: usize| remaining.get(k % m).copied().unwrap_or(0);
        let mut clipped = false;
        for k in 0..m {
            let (i, j, l) = (pick(k + m - 1), pick(k), pick(k + 1));
            let (a, b, c) = (at(i), at(j), at(l));
            let turn = cross2(a, b, c) * sign;
            if turn < 0.0 {
                continue; // A reflex corner is not an ear.
            }
            // An ear holds no other vertex of the polygon.
            let blocked = turn > 0.0
                && remaining.iter().any(|&p| {
                    p != i && p != j && p != l && {
                        let q = at(p);
                        cross2(a, b, q) * sign >= 0.0
                            && cross2(b, c, q) * sign >= 0.0
                            && cross2(c, a, q) * sign >= 0.0
                    }
                });
            if blocked {
                continue;
            }
            if turn > 0.0 {
                triangles.push([i, j, l]);
            }
            // A straight corner (turn == 0) is simply dropped.
            remaining.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            return None; // Not a simple polygon.
        }
    }
    if let [i, j, l] = *remaining.as_slice()
        && cross2(at(i), at(j), at(l)) * sign > 0.0
    {
        triangles.push([i, j, l]);
    }
    Some(triangles)
}

/// What a face adds to the mesh: its own vertices and triangles over them.
#[derive(Default)]
struct Patch {
    positions: Vec<Vec3>,
    triangles: Vec<[u32; 3]>,
}

impl Patch {
    fn push(&mut self, p: Vec3) -> u32 {
        // Face sizes are bounded by the segment limits, far below 2^32.
        let index = self.positions.len() as u32;
        self.positions.push(p);
        index
    }

    /// A band of quads between two rings of equal length, both closed.
    fn band(&mut self, lower: &[u32], upper: &[u32]) {
        let n = lower.len();
        for i in 0..n {
            let j = (i + 1) % n;
            if let (Some(&a), Some(&b), Some(&c), Some(&d)) =
                (lower.get(i), lower.get(j), upper.get(j), upper.get(i))
            {
                self.triangles.push([a, b, c]);
                self.triangles.push([a, c, d]);
            }
        }
    }

    /// A fan from a ring to one point.
    fn fan(&mut self, ring: &[u32], tip: u32) {
        let n = ring.len();
        for i in 0..n {
            if let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n)) {
                self.triangles.push([a, b, tip]);
            }
        }
    }
}

/// The points of a loop that is one full circle about the surface's axis,
/// ordered by increasing u and starting at the smallest.
fn ring(surface: &Surface, points: &[Vec3], face: usize) -> Result<Vec<(f64, Vec3)>, TessError> {
    let mut with_u = Vec::with_capacity(points.len());
    for &p in points {
        let (u, _) = surface
            .project(p)
            .map_err(|e| TessError::InvalidBody(format!("face {face}: {e}")))?;
        with_u.push((u, p));
    }
    with_u.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(with_u)
}

fn plane_face(
    body: &Body,
    edges: &[Vec<Vec3>],
    face: &Face,
    index: usize,
) -> Result<Patch, TessError> {
    let Surface::Plane(plane) = &face.surface else {
        return unsupported(index, "not a plane");
    };
    let [loop_id] = *face.loops.as_slice() else {
        return unsupported(
            index,
            format!(
                "a planar face with {} loops; holes cannot be tessellated yet",
                face.loops.len()
            ),
        );
    };
    let loop_ = body.loop_(loop_id).ok_or_else(|| {
        TessError::InvalidBody(format!("face {index} uses a loop that does not exist"))
    })?;
    let points = loop_points(body, edges, &loop_.coedges, index)?;
    let flat: Vec<(f64, f64)> = points
        .iter()
        .map(|&p| {
            let (x, y, _) = plane.frame().local(p);
            (x, y)
        })
        .collect();
    let Some(triangles) = ear_clip(&flat) else {
        return unsupported(
            index,
            "a planar face whose boundary is not a simple polygon",
        );
    };
    let mut patch = Patch::default();
    for p in points {
        patch.push(p);
    }
    // Polygon indices are positions in `points`, so they fit u32.
    patch.triangles = triangles.into_iter().map(|t| t.map(|k| k as u32)).collect();
    Ok(patch)
}

fn cylinder_face(
    body: &Body,
    edges: &[Vec<Vec3>],
    face: &Face,
    index: usize,
) -> Result<Patch, TessError> {
    let [first, second] = *face.loops.as_slice() else {
        return unsupported(
            index,
            "a cylindrical face that is not the wall between two full circles",
        );
    };
    let mut rings = Vec::new();
    for loop_id in [first, second] {
        let loop_ = body.loop_(loop_id).ok_or_else(|| {
            TessError::InvalidBody(format!("face {index} uses a loop that does not exist"))
        })?;
        rings.push(ring(
            &face.surface,
            &loop_points(body, edges, &loop_.coedges, index)?,
            index,
        )?);
    }
    let (Some(a), Some(b)) = (rings.first(), rings.get(1)) else {
        return unsupported(index, "a cylindrical face without two boundary circles");
    };
    // Both circles have the cylinder's radius, hence the same number of
    // points; they must also sit at the same angles to be joined by quads.
    let aligned = a.len() == b.len()
        && a.len() >= 3
        && a.iter().zip(b).all(|(p, q)| (p.0 - q.0).abs() < 1e-9);
    if !aligned {
        return unsupported(
            index,
            "a cylindrical face whose two boundary circles do not line up",
        );
    }
    let mut patch = Patch::default();
    let lower: Vec<u32> = a.iter().map(|&(_, p)| patch.push(p)).collect();
    let upper: Vec<u32> = b.iter().map(|&(_, p)| patch.push(p)).collect();
    patch.band(&lower, &upper);
    Ok(patch)
}

fn cone_face(
    body: &Body,
    edges: &[Vec<Vec3>],
    face: &Face,
    index: usize,
) -> Result<Patch, TessError> {
    let (Surface::Cone(cone), [loop_id]) = (&face.surface, face.loops.as_slice()) else {
        return unsupported(
            index,
            "a conical face that is not a full circle and an apex",
        );
    };
    let loop_ = body.loop_(*loop_id).ok_or_else(|| {
        TessError::InvalidBody(format!("face {index} uses a loop that does not exist"))
    })?;
    let base = ring(
        &face.surface,
        &loop_points(body, edges, &loop_.coedges, index)?,
        index,
    )?;
    if base.len() < 3 {
        return unsupported(index, "a conical face whose boundary is not a full circle");
    }
    let mut patch = Patch::default();
    let ring: Vec<u32> = base.iter().map(|&(_, p)| patch.push(p)).collect();
    let tip = patch.push(face.surface.eval(0.0, cone.apex_v()));
    patch.fan(&ring, tip);
    Ok(patch)
}

/// A grid over a whole closed surface: `around` columns in u, `along` rows
/// in v. Rows at a pole collapse to one vertex.
fn closed_grid(
    surface: &Surface,
    around: u32,
    along: u32,
    v_range: (f64, f64),
    wrap_v: bool,
) -> Patch {
    let mut patch = Patch::default();
    let rows = if wrap_v { along } else { along + 1 };
    let mut grid: Vec<Vec<u32>> = Vec::with_capacity(rows as usize);
    for j in 0..rows {
        let v = v_range.0 + (v_range.1 - v_range.0) * f64::from(j) / f64::from(along);
        let pole =
            !wrap_v && (j == 0 || j == along) && surface.derivatives(0.0, v).1.length() < 1e-9;
        let row = if pole {
            let tip = patch.push(surface.eval(0.0, v));
            vec![tip; around as usize]
        } else {
            (0..around)
                .map(|i| patch.push(surface.eval(TAU * f64::from(i) / f64::from(around), v)))
                .collect()
        };
        grid.push(row);
    }
    let bands = if wrap_v { rows } else { rows - 1 };
    for j in 0..bands {
        let next = (j + 1) % rows;
        if let (Some(lower), Some(upper)) = (grid.get(j as usize), grid.get(next as usize)) {
            patch.band(lower, upper);
        }
    }
    // Triangles collapsed at a pole have two equal corners.
    patch
        .triangles
        .retain(|[a, b, c]| a != b && b != c && c != a);
    patch
}

/// On a surface curved in both directions a grid cell sags in u and in v
/// at once, and inside a triangle the two add up. Giving each direction
/// half the tolerance keeps their sum within it.
fn doubly_curved(params: &TessParams) -> TessParams {
    TessParams {
        chordal_tolerance: params.chordal_tolerance / 2.0,
        ..*params
    }
}

fn face_patch(
    body: &Body,
    edges: &[Vec<Vec3>],
    face: &Face,
    index: usize,
    params: &TessParams,
) -> Result<Patch, TessError> {
    match &face.surface {
        Surface::Plane(_) => plane_face(body, edges, face, index),
        Surface::Cylinder(_) => cylinder_face(body, edges, face, index),
        Surface::Cone(_) => cone_face(body, edges, face, index),
        Surface::Sphere(s) if face.loops.is_empty() => {
            let params = &doubly_curved(params);
            let around = arc_segments(s.radius(), TAU, params)?.max(4);
            Ok(closed_grid(
                &face.surface,
                around,
                around.div_ceil(2),
                (-PI / 2.0, PI / 2.0),
                false,
            ))
        }
        Surface::Torus(t) if face.loops.is_empty() => {
            let params = &doubly_curved(params);
            let (major, minor) = t.radii();
            let around = arc_segments(major + minor, TAU, params)?.max(4);
            let tube = arc_segments(minor, TAU, params)?.max(4);
            Ok(closed_grid(&face.surface, around, tube, (0.0, TAU), true))
        }
        other => unsupported(index, format!("a trimmed {} face", other.kind())),
    }
}

/// Tessellates every face of `body`. Triangle `face_ids` are face indices;
/// triangles face outward. The body should pass `craft3d_topo::check`.
pub fn tessellate_body(body: &Body, params: &TessParams) -> Result<Mesh, TessError> {
    let tol = params.chordal_tolerance;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(TessError::InvalidTolerance(tol));
    }
    let edges = edge_points(body, params)?;
    let mut mesh = Mesh::default();
    for (index, face) in body.faces.iter().enumerate() {
        let patch = face_patch(body, &edges, face, index, params)?;
        let face_id = u32::try_from(index).map_err(|_| TessError::TooManyVertices)?;
        let base = u32::try_from(mesh.positions.len()).map_err(|_| TessError::TooManyVertices)?;
        if u64::from(base) + patch.positions.len() as u64 > u64::from(u32::MAX) {
            return Err(TessError::TooManyVertices);
        }
        let outward = if face.reversed { -1.0 } else { 1.0 };
        let mut normals = Vec::with_capacity(patch.positions.len());
        for &p in &patch.positions {
            let normal = face
                .surface
                .project(p)
                .ok()
                .and_then(|(u, v)| face.surface.normal(u, v));
            normals.push(normal.map_or(Vec3::ZERO, |n| n * outward));
        }
        // Make the triangles face outward: compare one of them with the
        // surface normal at its corners.
        let flip = patch.triangles.iter().find_map(|&[a, b, c]| {
            let corner = |k: u32| patch.positions.get(k as usize).copied();
            let (pa, pb, pc) = (corner(a)?, corner(b)?, corner(c)?);
            let geometric = (pb - pa).cross(pc - pa);
            let reference = [a, b, c]
                .iter()
                .filter_map(|&k| normals.get(k as usize))
                .fold(Vec3::ZERO, |s, &n| s + n);
            let agreement = geometric.dot(reference);
            (agreement != 0.0).then_some(agreement < 0.0)
        });
        for [a, b, c] in patch.triangles {
            let (b, c) = if flip == Some(true) { (c, b) } else { (b, c) };
            mesh.triangles.push([base + a, base + b, base + c]);
            mesh.face_ids.push(face_id);
        }
        mesh.positions.extend(patch.positions);
        mesh.normals.extend(normals);
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(polygon: &[(f64, f64)], triangles: &[[usize; 3]]) -> f64 {
        triangles
            .iter()
            .map(|&[a, b, c]| cross2(polygon[a], polygon[b], polygon[c]) / 2.0)
            .sum()
    }

    #[test]
    fn ear_clipping_covers_convex_and_concave_polygons_exactly() {
        let square = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        let tris = ear_clip(&square).unwrap();
        assert_eq!(tris.len(), 2);
        assert_eq!(area(&square, &tris), 4.0);
        // An L shape: concave at (1, 1).
        let ell = [
            (0.0, 0.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (1.0, 2.0),
            (0.0, 2.0),
        ];
        let tris = ear_clip(&ell).unwrap();
        assert_eq!(tris.len(), 4);
        assert_eq!(area(&ell, &tris), 3.0);
        assert!(
            tris.iter()
                .all(|&[a, b, c]| cross2(ell[a], ell[b], ell[c]) > 0.0)
        );
        // Clockwise input keeps its orientation: negative area, same size.
        let clockwise: Vec<_> = ell.iter().rev().copied().collect();
        let tris = ear_clip(&clockwise).unwrap();
        assert_eq!(area(&clockwise, &tris), -3.0);
        // A comb with deep notches.
        let comb = [
            (0.0, 0.0),
            (7.0, 0.0),
            (7.0, 3.0),
            (6.0, 3.0),
            (6.0, 1.0),
            (5.0, 1.0),
            (5.0, 3.0),
            (4.0, 3.0),
            (4.0, 1.0),
            (3.0, 1.0),
            (3.0, 3.0),
            (2.0, 3.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (1.0, 3.0),
            (0.0, 3.0),
        ];
        let tris = ear_clip(&comb).unwrap();
        assert!((area(&comb, &tris) - (7.0 + 4.0 * 2.0)).abs() < 1e-12);
    }

    #[test]
    fn ear_clipping_drops_straight_corners_and_rejects_non_polygons() {
        let with_midpoints = [
            (0.0, 0.0),
            (1.0, 0.0),
            (2.0, 0.0),
            (2.0, 2.0),
            (1.0, 2.0),
            (0.0, 2.0),
        ];
        let tris = ear_clip(&with_midpoints).unwrap();
        assert_eq!(area(&with_midpoints, &tris), 4.0);
        assert!(ear_clip(&[]).is_none());
        assert!(ear_clip(&[(0.0, 0.0), (1.0, 0.0)]).is_none());
        assert!(
            ear_clip(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)]).is_none(),
            "zero area"
        );
        assert!(ear_clip(&[(0.0, 0.0), (f64::NAN, 0.0), (1.0, 1.0)]).is_none());
    }

    #[test]
    fn arc_segments_keep_the_chord_within_tolerance() {
        for (radius, tol) in [(1.0, 0.1), (1.0, 1e-3), (250.0, 0.01), (1e-3, 1e-6)] {
            let params = TessParams {
                chordal_tolerance: tol,
                ..TessParams::default()
            };
            let n = arc_segments(radius, TAU, &params).unwrap();
            let sag = |n: u32| radius * (1.0 - (PI / f64::from(n)).cos());
            assert!(
                sag(n) <= tol * (1.0 + 1e-12),
                "r {radius} tol {tol}: n = {n}"
            );
            // One segment fewer would not do (unless at the minimum).
            assert!(
                n == 3 || sag(n - 1) > tol,
                "r {radius} tol {tol}: n = {n} is more than needed"
            );
        }
        // A tolerance larger than the radius still gives a polygon.
        let coarse = TessParams {
            chordal_tolerance: 10.0,
            ..TessParams::default()
        };
        assert_eq!(arc_segments(1.0, TAU, &coarse).unwrap(), 3);
        let too_fine = TessParams {
            chordal_tolerance: 1e-15,
            max_segments: 16,
        };
        assert!(matches!(
            arc_segments(1.0, TAU, &too_fine),
            Err(TessError::TooFine { .. })
        ));
    }
}
