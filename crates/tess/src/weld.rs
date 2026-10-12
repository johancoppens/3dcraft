//! Connectivity for a mesh whose faces each carry their own vertices.

use crate::Mesh;
use std::collections::BTreeMap;

/// A mesh with vertices at bit-identical positions merged, and the
/// triangles that collapse by it dropped. Normals are not kept: a merged
/// vertex on a crease has several.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Welded {
    pub positions: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

impl Mesh {
    /// Merges vertices at bit-identical positions. The tessellator computes
    /// every edge once, so faces that share an edge share its points
    /// exactly and no distance tolerance is needed. `None` when a triangle
    /// refers to a vertex that does not exist; the value is the triangle.
    pub fn weld(&self) -> Result<Welded, usize> {
        let mut index_of: BTreeMap<[u64; 3], u32> = BTreeMap::new();
        let mut positions = Vec::new();
        let mut remap = Vec::with_capacity(self.positions.len());
        for p in &self.positions {
            // -0.0 and 0.0 are the same place.
            let key = [p.x + 0.0, p.y + 0.0, p.z + 0.0].map(f64::to_bits);
            // Indices are u32 in the mesh already, so this cannot overflow.
            let next = positions.len() as u32;
            let index = *index_of.entry(key).or_insert_with(|| {
                positions.push([p.x, p.y, p.z]);
                next
            });
            remap.push(index);
        }
        let mut triangles = Vec::with_capacity(self.triangles.len());
        for (t, tri) in self.triangles.iter().enumerate() {
            let [a, b, c] = tri.map(|k| remap.get(k as usize).copied());
            let (Some(a), Some(b), Some(c)) = (a, b, c) else {
                return Err(t);
            };
            if a != b && b != c && c != a {
                triangles.push([a, b, c]);
            }
        }
        Ok(Welded {
            positions,
            triangles,
        })
    }
}

impl Welded {
    /// True when the triangles form a closed, consistently oriented
    /// manifold: every directed edge occurs once, and so does its reverse.
    pub fn is_closed(&self) -> bool {
        let mut edges: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for &[a, b, c] in &self.triangles {
            for edge in [(a, b), (b, c), (c, a)] {
                *edges.entry(edge).or_insert(0) += 1;
            }
        }
        !edges.is_empty()
            && edges
                .iter()
                .all(|(&(a, b), &n)| n == 1 && edges.get(&(b, a)) == Some(&1))
    }

    /// Signed volume by the divergence theorem: positive when the triangles
    /// face outward. Only meaningful for a closed mesh.
    pub fn volume(&self) -> f64 {
        self.triangles
            .iter()
            .filter_map(|t| {
                let [a, b, c] = t.map(|k| self.positions.get(k as usize).copied());
                let (a, b, c) = (a?, b?, c?);
                let [ax, ay, az] = a;
                let [bx, by, bz] = b;
                let [cx, cy, cz] = c;
                Some(
                    (ax * (by * cz - bz * cy) - ay * (bx * cz - bz * cx)
                        + az * (bx * cy - by * cx))
                        / 6.0,
                )
            })
            .sum()
    }
}
