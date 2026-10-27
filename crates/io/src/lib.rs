//! Import and export orchestration.
//!
//! 3DCraft meshes to STL, 3MF and OBJ through `craft3d-meshio`, and bodies
//! (the exact B-rep) to STEP through `craft3d-step`: see [`step`]. The
//! outline of a sketch or of a flat face goes to DXF and SVG through
//! `craft3d-drawing`: see [`drawing`].

pub mod drawing;
pub mod step;

pub use drawing::{
    DrawingFormat, FaceAxes, FaceOutline, FittedSpline, SketchOutline, encode_drawing,
    encode_drawing_stroked, face_frame, face_outline, face_outline_in, sketch_outline,
};
pub use step::{AngleUnit, StepBody, StepOptions, StepSummary, export_step, step_model};

use craft3d_meshio::{self as meshio, MeshError, Model, Object, ObjectKind, Unit};
use craft3d_tess::Mesh;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshFormat {
    StlBinary,
    StlAscii,
    ThreeMf,
    Obj,
}

impl MeshFormat {
    /// The format for a file extension (without the dot, any case). `stl`
    /// means binary STL.
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "stl" => Some(MeshFormat::StlBinary),
            "3mf" => Some(MeshFormat::ThreeMf),
            "obj" => Some(MeshFormat::Obj),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum IoError {
    /// The mesh's arrays do not fit together.
    InvalidMesh {
        triangle: usize,
    },
    Format(MeshError),
    /// There is nothing to export.
    NoBodies,
    /// A body fails the validity check: the first few defects, and how
    /// many there are.
    InvalidBody {
        body: String,
        defects: Vec<String>,
        total: usize,
    },
    /// Something the export does not handle; `body` is empty when it is
    /// not about one body.
    Unsupported {
        body: String,
        what: String,
    },
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IoError::InvalidMesh { triangle } => {
                write!(
                    f,
                    "mesh is inconsistent: triangle {triangle} refers to a vertex that does not exist"
                )
            }
            IoError::Format(e) => write!(f, "{e}"),
            IoError::NoBodies => write!(f, "there are no bodies to export"),
            IoError::InvalidBody {
                body,
                defects,
                total,
            } => write!(
                f,
                "body `{body}` is not valid ({total} defects): {}",
                defects.join("; ")
            ),
            IoError::Unsupported { body, what } if body.is_empty() => write!(f, "{what}"),
            IoError::Unsupported { body, what } => write!(f, "body `{body}`: {what}"),
        }
    }
}

impl std::error::Error for IoError {}

impl From<MeshError> for IoError {
    fn from(e: MeshError) -> Self {
        IoError::Format(e)
    }
}

/// Converts a tessellated mesh to an exchange object.
///
/// Vertices at bit-identical positions are merged (`Mesh::weld`), which
/// joins faces along the edges the tessellator made exact; triangles that
/// collapse by it are dropped. Vertex normals are not carried over: none of
/// the formats needs them. The object is a [`ObjectKind::Solid`] only when
/// the merged mesh is closed and consistently oriented.
pub fn to_object(mesh: &Mesh, name: &str) -> Result<Object, IoError> {
    let welded = mesh
        .weld()
        .map_err(|triangle| IoError::InvalidMesh { triangle })?;
    let kind = if welded.is_closed() {
        ObjectKind::Solid
    } else {
        ObjectKind::Surface
    };
    Ok(Object {
        name: name.to_string(),
        kind,
        positions: welded.positions,
        triangles: welded.triangles,
        ..Object::default()
    })
}

/// [`to_object`] with a material per triangle: `face_material` says what
/// the face of each triangle (`Mesh::face_ids`) is made of, an index into
/// the model's materials, or `None` for the object's own
/// (`Object::material`, which the caller sets).
pub fn to_object_with_materials(
    mesh: &Mesh,
    name: &str,
    face_material: impl Fn(u32) -> Option<u32>,
) -> Result<Object, IoError> {
    let mut object = to_object(mesh, name)?;
    // Welding drops the triangles with two corners at one place: the same
    // ones are left out here, so the lists stay in step.
    let place = |k: u32| {
        mesh.positions
            .get(k as usize)
            .map(|p| [p.x + 0.0, p.y + 0.0, p.z + 0.0].map(f64::to_bits))
    };
    let mut materials = Vec::with_capacity(object.triangles.len());
    for (tri, &face) in mesh.triangles.iter().zip(&mesh.face_ids) {
        let [a, b, c] = tri.map(place);
        if a != b && b != c && c != a {
            materials.push(face_material(face));
        }
    }
    if materials.len() == object.triangles.len() && materials.iter().any(Option::is_some) {
        object.triangle_materials = materials;
    }
    Ok(object)
}

/// Encodes a model as a file of the given format.
pub fn encode(model: &Model, format: MeshFormat) -> Result<Vec<u8>, IoError> {
    Ok(match format {
        MeshFormat::StlBinary => meshio::write_stl_binary(model)?,
        MeshFormat::StlAscii => meshio::write_stl_ascii(model)?.into_bytes(),
        MeshFormat::ThreeMf => meshio::write_3mf(model)?,
        MeshFormat::Obj => meshio::write_obj(model)?.into_bytes(),
    })
}

/// Encodes `mesh` as a file of the given format. `unit` is recorded where
/// the format can (3MF); coordinates are written as they are.
pub fn export_mesh(
    mesh: &Mesh,
    name: &str,
    unit: Unit,
    format: MeshFormat,
) -> Result<Vec<u8>, IoError> {
    let model = Model {
        unit,
        objects: vec![to_object(mesh, name)?],
        materials: Vec::new(),
    };
    encode(&model, format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Vec3;
    use craft3d_tess::{TessParams, Welded, tessellate_patches};
    use craft3d_testkit::teapot::teapot;

    /// An axis-aligned cube of the given side with one corner at the origin,
    /// as six separate outward-facing quads: 24 vertices, 12 triangles.
    pub(crate) fn cube(side: f64) -> Mesh {
        let s = side;
        let faces = [
            [[0.0, 0.0, 0.0], [0.0, s, 0.0], [s, s, 0.0], [s, 0.0, 0.0]], // -z
            [[0.0, 0.0, s], [s, 0.0, s], [s, s, s], [0.0, s, s]],         // +z
            [[0.0, 0.0, 0.0], [s, 0.0, 0.0], [s, 0.0, s], [0.0, 0.0, s]], // -y
            [[0.0, s, 0.0], [0.0, s, s], [s, s, s], [s, s, 0.0]],         // +y
            [[0.0, 0.0, 0.0], [0.0, 0.0, s], [0.0, s, s], [0.0, s, 0.0]], // -x
            [[s, 0.0, 0.0], [s, s, 0.0], [s, s, s], [s, 0.0, s]],         // +x
        ];
        let mut mesh = Mesh::default();
        for (face, quad) in faces.iter().enumerate() {
            let base = mesh.positions.len() as u32;
            for [x, y, z] in quad {
                mesh.positions.push(Vec3::new(*x, *y, *z));
                mesh.normals.push(Vec3::ZERO);
            }
            mesh.triangles.push([base, base + 1, base + 2]);
            mesh.triangles.push([base, base + 2, base + 3]);
            mesh.face_ids.extend([face as u32; 2]);
        }
        mesh
    }

    fn signed_volume(o: &Object) -> f64 {
        o.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|k| o.positions[k as usize]);
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            })
            .sum()
    }

    #[test]
    fn a_cube_of_separate_faces_welds_into_a_closed_solid() {
        let object = to_object(&cube(2.0), "cube").unwrap();
        assert_eq!(object.positions.len(), 8);
        assert_eq!(object.triangles.len(), 12);
        assert_eq!(object.kind, ObjectKind::Solid);
        // Outward orientation: positive volume, side³ up to the rounding of
        // twelve terms divided by six.
        assert!((signed_volume(&object) - 8.0).abs() < 1e-12);
    }

    fn is_closed(triangles: &[[u32; 3]]) -> bool {
        Welded {
            positions: Vec::new(),
            triangles: triangles.to_vec(),
        }
        .is_closed()
    }

    #[test]
    fn closedness_needs_every_edge_matched_in_the_opposite_direction() {
        let tetra = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
        assert!(is_closed(&tetra));
        assert!(!is_closed(&tetra[..3]), "a missing face leaves open edges");
        let mut flipped = tetra;
        flipped[3] = [1, 3, 2];
        assert!(!is_closed(&flipped), "one face wound the wrong way");
        let mut doubled = tetra.to_vec();
        doubled.push([1, 2, 3]);
        assert!(!is_closed(&doubled), "an edge used three times");
        assert!(!is_closed(&[]));
        let mut open = cube(1.0);
        open.triangles.truncate(10);
        assert_eq!(to_object(&open, "open").unwrap().kind, ObjectKind::Surface);
    }

    #[test]
    fn the_teapot_welds_along_its_seams_but_stays_open() {
        let mesh = tessellate_patches(&teapot().unwrap(), &TessParams::default()).unwrap();
        let object = to_object(&mesh, "teapot").unwrap();
        // Seams and the collapsed lid and bottom centres merge.
        assert!(object.positions.len() < mesh.positions.len());
        assert!(object.triangles.len() <= mesh.triangles.len());
        // The dataset has a gap between lid and rim and an open spout.
        assert_eq!(object.kind, ObjectKind::Surface);
        // Welding moves no vertex: the bounding box is unchanged.
        let (lo, hi) = mesh.bbox().corners().unwrap();
        for (axis, (min, max)) in [(lo.x, hi.x), (lo.y, hi.y), (lo.z, hi.z)]
            .into_iter()
            .enumerate()
        {
            let values = object.positions.iter().map(|p| p[axis]);
            assert_eq!(values.clone().fold(f64::INFINITY, f64::min), min);
            assert_eq!(values.fold(f64::NEG_INFINITY, f64::max), max);
        }
    }

    #[test]
    fn negative_zero_welds_with_zero() {
        let mut mesh = Mesh::default();
        for p in [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::X,
            Vec3::Y,
            Vec3::new(-0.0, 0.0, -0.0),
            Vec3::Y,
            Vec3::Z,
        ] {
            mesh.positions.push(p);
            mesh.normals.push(Vec3::ZERO);
        }
        mesh.triangles = vec![[0, 1, 2], [3, 4, 5]];
        mesh.face_ids = vec![0, 1];
        assert_eq!(to_object(&mesh, "").unwrap().positions.len(), 4);
    }

    #[test]
    fn every_format_exports_and_is_reproducible() {
        let mesh = cube(1.0);
        for (format, magic) in [
            (MeshFormat::StlBinary, &b"3DCraft binary STL"[..]),
            (MeshFormat::StlAscii, b"solid cube"),
            (MeshFormat::ThreeMf, b"PK\x03\x04"),
            (MeshFormat::Obj, b"# 3DCraft\no cube"),
        ] {
            let bytes = export_mesh(&mesh, "cube", Unit::Millimeter, format).unwrap();
            assert!(bytes.starts_with(magic), "{format:?}");
            assert_eq!(
                bytes,
                export_mesh(&mesh, "cube", Unit::Millimeter, format).unwrap()
            );
        }
        assert_eq!(
            MeshFormat::from_extension("STL"),
            Some(MeshFormat::StlBinary)
        );
        assert_eq!(MeshFormat::from_extension("3mf"), Some(MeshFormat::ThreeMf));
        assert_eq!(MeshFormat::from_extension("Obj"), Some(MeshFormat::Obj));
        assert_eq!(MeshFormat::from_extension("step"), None);
    }

    #[test]
    fn bad_meshes_are_errors() {
        let mut bad = cube(1.0);
        bad.triangles[5] = [0, 1, 999];
        for format in [
            MeshFormat::StlBinary,
            MeshFormat::StlAscii,
            MeshFormat::ThreeMf,
            MeshFormat::Obj,
        ] {
            assert_eq!(
                export_mesh(&bad, "bad", Unit::Millimeter, format),
                Err(IoError::InvalidMesh { triangle: 5 })
            );
        }
        let mut nan = cube(1.0);
        nan.positions[2].x = f64::NAN;
        assert!(matches!(
            export_mesh(&nan, "nan", Unit::Millimeter, MeshFormat::Obj),
            Err(IoError::Format(MeshError::NonFinitePosition { .. }))
        ));
    }

    #[test]
    fn triangle_materials_follow_the_faces_and_skip_what_welding_drops() {
        let mut mesh = cube(2.0);
        // A degenerate triangle of a face of its own, before the last face's
        // triangles: welding drops it, and its material with it.
        let k = u32::try_from(mesh.positions.len()).unwrap();
        mesh.positions.extend([Vec3::ZERO, Vec3::ZERO, Vec3::X]);
        mesh.normals.extend([Vec3::Z; 3]);
        mesh.triangles.insert(0, [k, k + 1, k + 2]);
        mesh.face_ids.insert(0, 99);
        let top = mesh.face_ids[1 + 2];
        let object = to_object_with_materials(&mesh, "cube", |face| match face {
            99 => Some(5),
            f if f == top => Some(1),
            _ => None,
        })
        .unwrap();
        assert_eq!(object.triangles.len(), 12);
        assert_eq!(object.triangle_materials.len(), 12);
        let kept: Vec<u32> = mesh.face_ids[1..].to_vec();
        for (material, face) in object.triangle_materials.iter().zip(kept) {
            assert_eq!(*material, (face == top).then_some(1));
        }
        // No material anywhere: no list.
        let plain = to_object_with_materials(&mesh, "cube", |_| None).unwrap();
        assert!(plain.triangle_materials.is_empty());
        assert_eq!(plain, to_object(&mesh, "cube").unwrap());
    }
}
