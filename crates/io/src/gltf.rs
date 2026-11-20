//! Tessellated bodies as the nodes of a glTF file
//! (`craft3d_meshio::gltf`): one node per body, and in it one group of
//! triangles (a primitive of its mesh) per name of its faces and material.
//!
//! **Face groups.** glTF names nodes and meshes but not the primitives of
//! a mesh; the place it gives application data is `extras`. A group holds
//! the triangles of the faces that share a persistent name (the pieces a
//! boolean cut one face into) and a material, and says so in its `extras`:
//! `face`, that name as `document.inspect` lists it under `faceNames`, and
//! `faces`, the indices of those faces in that list. Every triangle of a
//! primitive is of one of those faces, so a reader maps triangles back to
//! the faces 3DCraft refers to by name. A face without a name is a group of
//! its own, with `faces` alone. Groups come in the order of their first
//! face, triangles in the tessellator's order.
//!
//! The tessellator gives each face its own vertices, with the surface's
//! normal at each: a group keeps them (a cylinder shades smooth, and the
//! edges of a box stay sharp, its faces being groups of their own or, when
//! they share a name, having vertices of their own). A triangle with two
//! corners at one place is left out, as welding leaves it out of the other
//! mesh formats ([`crate::to_object`]), so the counts agree.

use crate::IoError;
use craft3d_meshio::gltf::{Extra, Extras, Group, Node};
use craft3d_tess::Mesh;
use std::collections::{BTreeMap, BTreeSet};

/// One body as its glTF node is made.
#[derive(Clone, Debug)]
pub struct GltfBody<'a> {
    pub name: &'a str,
    pub mesh: &'a Mesh,
    /// The persistent name of each face, by face index (`Mesh::face_ids`);
    /// a face past the list, or with an empty name, has none.
    pub face_names: &'a [String],
    /// What each face is made of, an index into the file's materials, by
    /// face index; `None` or past the list: [`GltfBody::material`].
    pub face_materials: &'a [Option<u32>],
    /// What the faces without a material of their own are made of.
    pub material: Option<u32>,
    /// The node's `extras`.
    pub extras: Extras,
}

/// What puts triangles in one group: the faces' name, or the face itself
/// when it has none, and the material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key<'a> {
    Named(&'a str, Option<u32>),
    Face(u32, Option<u32>),
}

/// A group being filled.
struct Filling<'a> {
    name: Option<&'a str>,
    group: Group,
    faces: Vec<u64>,
    seen: BTreeSet<u32>,
    /// The group's vertex for a vertex of the mesh that another group has
    /// in [`node`]'s shared table (a mesh whose faces share vertices).
    others: BTreeMap<u32, u32>,
}

/// The glTF node of a body (see the module's documentation): an error
/// when the mesh's arrays do not fit together (a triangle that refers to a
/// vertex that does not exist, or a face id missing), naming the triangle.
pub fn node(body: GltfBody<'_>) -> Result<Node, IoError> {
    let mesh = body.mesh;
    if mesh.face_ids.len() != mesh.triangles.len() {
        return Err(IoError::InvalidMesh {
            triangle: mesh.face_ids.len().min(mesh.triangles.len()),
        });
    }
    let mut keys: BTreeMap<Key, usize> = BTreeMap::new();
    let mut filling: Vec<Filling> = Vec::new();
    // The group and its vertex of each vertex of the mesh, for the first
    // group that uses it (the only one, when faces have vertices of their
    // own, as the tessellator makes them).
    let mut shared: Vec<Option<(usize, u32)>> = vec![None; mesh.positions.len()];
    let place = |p: craft3d_math::Vec3| [p.x + 0.0, p.y + 0.0, p.z + 0.0].map(f64::to_bits);
    for (t, (triangle, &face)) in mesh.triangles.iter().zip(&mesh.face_ids).enumerate() {
        let [Some(a), Some(b), Some(c)] = triangle.map(|k| mesh.positions.get(k as usize).copied())
        else {
            return Err(IoError::InvalidMesh { triangle: t });
        };
        let [a, b, c] = [a, b, c].map(place);
        if a == b || b == c || c == a {
            continue;
        }
        let material = body
            .face_materials
            .get(face as usize)
            .copied()
            .flatten()
            .or(body.material);
        let name = body
            .face_names
            .get(face as usize)
            .map(String::as_str)
            .filter(|n| !n.is_empty());
        let key = match name {
            Some(name) => Key::Named(name, material),
            None => Key::Face(face, material),
        };
        let at = match keys.get(&key) {
            Some(&at) => at,
            None => {
                keys.insert(key, filling.len());
                filling.push(Filling {
                    name,
                    group: Group {
                        material,
                        ..Group::default()
                    },
                    faces: Vec::new(),
                    seen: BTreeSet::new(),
                    others: BTreeMap::new(),
                });
                filling.len() - 1
            }
        };
        let Some(fill) = filling.get_mut(at) else {
            return Err(IoError::InvalidMesh { triangle: t });
        };
        if fill.seen.insert(face) {
            fill.faces.push(u64::from(face));
        }
        let mut corners = [0_u32; 3];
        for (corner, &k) in corners.iter_mut().zip(triangle) {
            let known = match shared.get(k as usize).copied().flatten() {
                Some((group, vertex)) if group == at => Some(vertex),
                _ => fill.others.get(&k).copied(),
            };
            *corner = match known {
                Some(vertex) => vertex,
                None => {
                    let vertex = u32::try_from(fill.group.positions.len())
                        .map_err(|_| IoError::InvalidMesh { triangle: t })?;
                    let (Some(p), n) =
                        (mesh.positions.get(k as usize), mesh.normals.get(k as usize))
                    else {
                        return Err(IoError::InvalidMesh { triangle: t });
                    };
                    fill.group.positions.push([p.x, p.y, p.z]);
                    // No normal: the writer takes the triangles' own.
                    fill.group
                        .normals
                        .push(n.map_or([0.0; 3], |n| [n.x, n.y, n.z]));
                    match shared.get_mut(k as usize) {
                        Some(slot) if slot.is_none() => *slot = Some((at, vertex)),
                        _ => {
                            fill.others.insert(k, vertex);
                        }
                    }
                    vertex
                }
            };
        }
        fill.group.triangles.push(corners);
    }
    let groups = filling
        .into_iter()
        .map(|fill| {
            let mut group = fill.group;
            if let Some(name) = fill.name {
                group
                    .extras
                    .push(("face".to_string(), Extra::Text(name.to_string())));
            }
            group
                .extras
                .push(("faces".to_string(), Extra::Numbers(fill.faces)));
            group
        })
        .collect();
    Ok(Node {
        name: body.name.to_string(),
        extras: body.extras,
        groups,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::cube;
    use craft3d_math::Vec3;

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|k| format!("F1:face({k})")).collect()
    }

    #[test]
    fn a_group_per_face_name_and_material_with_the_faces_in_extras() {
        let mesh = cube(2.0);
        // Faces 2 and 3 share a name; face 5 has none; face 1 is green.
        let mut faces = names(6);
        faces[3] = faces[2].clone();
        faces[5] = String::new();
        let materials = [None, Some(1), None, None];
        let node = node(GltfBody {
            name: "cube",
            mesh: &mesh,
            face_names: &faces,
            face_materials: &materials,
            material: Some(0),
            extras: vec![("feature".into(), Extra::Text("F1".into()))],
        })
        .unwrap();
        assert_eq!(node.name, "cube");
        let summary: Vec<(Extras, Option<u32>, usize, usize)> = node
            .groups
            .iter()
            .map(|g| {
                (
                    g.extras.clone(),
                    g.material,
                    g.positions.len(),
                    g.triangles.len(),
                )
            })
            .collect();
        let named = |name: &str, faces: Vec<u64>| {
            vec![
                ("face".to_string(), Extra::Text(name.to_string())),
                ("faces".to_string(), Extra::Numbers(faces)),
            ]
        };
        assert_eq!(
            summary,
            [
                (named("F1:face(0)", vec![0]), Some(0), 4, 2),
                (named("F1:face(1)", vec![1]), Some(1), 4, 2),
                (named("F1:face(2)", vec![2, 3]), Some(0), 8, 4),
                (named("F1:face(4)", vec![4]), Some(0), 4, 2),
                (
                    vec![("faces".to_string(), Extra::Numbers(vec![5]))],
                    Some(0),
                    4,
                    2
                ),
            ]
        );
        // Every triangle's corners are the mesh's, in its order.
        let group = &node.groups[2];
        for (t, triangle) in group.triangles.iter().enumerate() {
            let source = mesh.triangles[4 + t];
            for (corner, k) in triangle.iter().zip(source) {
                let p = mesh.positions[k as usize];
                assert_eq!(group.positions[*corner as usize], [p.x, p.y, p.z]);
            }
        }
    }

    #[test]
    fn triangles_welding_drops_are_left_out_and_bad_meshes_are_errors() {
        let mut mesh = cube(1.0);
        let k = mesh.positions.len() as u32;
        mesh.positions
            .extend([Vec3::ZERO, Vec3::new(-0.0, 0.0, 0.0), Vec3::X]);
        mesh.normals.extend([Vec3::Z; 3]);
        mesh.triangles.push([k, k + 1, k + 2]);
        mesh.face_ids.push(6);
        fn body(mesh: &Mesh) -> GltfBody<'_> {
            GltfBody {
                name: "c",
                mesh,
                face_names: &[],
                face_materials: &[],
                material: None,
                extras: Vec::new(),
            }
        }
        let made = node(body(&mesh)).unwrap();
        let triangles: usize = made.groups.iter().map(|g| g.triangles.len()).sum();
        assert_eq!(triangles, 12);
        assert_eq!(made.groups.len(), 6);
        assert_eq!(
            triangles,
            crate::to_object(&mesh, "c").unwrap().triangles.len()
        );
        let mut bad = cube(1.0);
        bad.triangles[5] = [0, 1, 999];
        assert_eq!(
            node(body(&bad)).unwrap_err(),
            IoError::InvalidMesh { triangle: 5 }
        );
        let mut short = cube(1.0);
        short.face_ids.pop();
        assert_eq!(
            node(body(&short)).unwrap_err(),
            IoError::InvalidMesh { triangle: 11 }
        );
    }

    #[test]
    fn faces_that_share_vertices_get_their_own_in_each_group() {
        // A mesh whose two triangles share an edge but are two faces.
        let mut mesh = Mesh::default();
        for p in [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 0.0)] {
            mesh.positions.push(p);
            mesh.normals.push(Vec3::Z);
        }
        mesh.triangles = vec![[0, 1, 2], [2, 1, 3]];
        mesh.face_ids = vec![0, 1];
        let made = node(GltfBody {
            name: "two",
            mesh: &mesh,
            face_names: &names(2),
            face_materials: &[],
            material: None,
            extras: Vec::new(),
        })
        .unwrap();
        assert_eq!(made.groups.len(), 2);
        for group in &made.groups {
            assert_eq!(group.positions.len(), 3);
            assert_eq!(group.normals, [[0.0, 0.0, 1.0]; 3]);
            assert_eq!(group.triangles, [[0, 1, 2]]);
        }
        assert_eq!(made.groups[1].positions[0], [0.0, 1.0, 0.0]);
    }
}
