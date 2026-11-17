//! Import and export orchestration.
//!
//! 3DCraft meshes to STL, 3MF and OBJ through `craft3d-meshio`, and bodies
//! (the exact B-rep) to STEP through `craft3d-step`: see [`step`]. The
//! outline of a sketch or of a flat face goes to DXF and SVG through
//! `craft3d-drawing`: see [`drawing`].

pub mod cut;
pub mod drawing;
pub mod step;
pub mod step_assembly;

pub use drawing::{
    DrawingFormat, FaceAxes, FaceOutline, FittedSpline, SketchOutline, encode_drawing,
    encode_drawing_stroked, face_frame, face_outline, face_outline_in, sketch_outline,
};
pub use step::{AngleUnit, StepBody, StepOptions, StepSummary, export_step, step_model};
pub use step_assembly::{StepOccurrence, step_assembly};

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

/// What an object's mesh is once its coordinates are rounded to single
/// precision, as a binary STL file holds them ([`in_single_precision`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SinglePrecision {
    /// Points that fall on another point of the object once rounded.
    pub merged: usize,
    /// Triangles with two corners on one point once rounded: a line, of
    /// no area.
    pub folded: usize,
    /// Whether the rounded triangles, those folded to a line left out, are
    /// still a closed surface: every side used once each way, the
    /// triangles round every corner one fan, and some triangle left.
    pub closed: bool,
}

/// The object's mesh as a program reads it from a file in single
/// precision (binary STL): its points rounded to `f32` and those that
/// round to one place taken for one, as such a program welds them. A
/// mesh closed in double precision (`ObjectKind::Solid`) can have points
/// closer than single precision tells apart (a part far from the origin,
/// or a surface that turns more sharply than the angle of the mesh can
/// follow with points as far apart as that). Where two such points are
/// the ends of a side, the side shrinks to a point and its two triangles
/// to lines: those bound nothing (each one's two other sides cancel), a
/// reader's repair drops them first, and the rest can still be a closed
/// surface, which is counted without them. Where they are not (two points
/// with no side between them, the two sides of a seam), the file is no
/// closed surface; this says which.
pub fn in_single_precision(object: &Object) -> SinglePrecision {
    let rounded: Vec<[u32; 3]> = object
        .positions
        .iter()
        // −0 as 0, as a reader compares the numbers.
        .map(|p| p.map(|c| ((c as f32) + 0.0).to_bits()))
        .collect();
    let mut order: Vec<([u32; 3], usize)> = rounded.iter().copied().zip(0..).collect();
    order.sort_unstable();
    let mut index = vec![0_usize; rounded.len()];
    let mut distinct = 0;
    for (k, &(place, i)) in order.iter().enumerate() {
        let new = k == 0 || order.get(k - 1).is_none_or(|&(before, _)| before != place);
        if new {
            distinct += 1;
        }
        if let Some(slot) = index.get_mut(i) {
            *slot = distinct - 1;
        }
    }
    let merged = rounded.len() - distinct;
    let closed_before = object.kind == ObjectKind::Solid;
    if merged == 0 {
        return SinglePrecision {
            merged,
            folded: 0,
            closed: closed_before,
        };
    }
    let corner = |k: u32| index.get(k as usize).copied().unwrap_or(usize::MAX);
    let mut folded = 0;
    let mut sides: std::collections::BTreeMap<(usize, usize), u32> = Default::default();
    let mut round: std::collections::BTreeMap<(usize, usize), usize> = Default::default();
    for t in &object.triangles {
        let [a, b, c] = t.map(corner);
        if a == b || b == c || c == a {
            folded += 1;
            continue;
        }
        for (from, to, other) in [(a, b, c), (b, c, a), (c, a, b)] {
            *sides.entry((from, to)).or_insert(0) += 1;
            round.insert((from, to), other);
        }
    }
    let once = sides
        .iter()
        .all(|(&(a, b), &n)| n == 1 && sides.get(&(b, a)) == Some(&1));
    // Round each corner: from one neighbour to the next, as many steps as
    // it has triangles, back where it started.
    let mut fans: std::collections::BTreeMap<usize, Vec<(usize, usize)>> = Default::default();
    for (&(corner, from), &to) in &round {
        fans.entry(corner).or_default().push((from, to));
    }
    let one_fan = |steps: &Vec<(usize, usize)>| {
        let next: std::collections::BTreeMap<usize, usize> = steps.iter().copied().collect();
        let Some(&(start, _)) = steps.first() else {
            return true;
        };
        let (mut here, mut count) = (start, 0);
        while let Some(&to) = next.get(&here) {
            here = to;
            count += 1;
            if here == start || count > steps.len() {
                break;
            }
        }
        count == steps.len() && here == start
    };
    // The triangles folded to a line left out, as a reader's repair drops
    // them: the rest closed, and something left.
    let closed =
        closed_before && folded < object.triangles.len() && once && fans.values().all(one_fan);
    SinglePrecision {
        merged,
        folded,
        closed,
    }
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
    fn points_single_precision_cannot_tell_apart_fold_triangles_or_open_the_file() {
        // The cube a thousand units from the origin, closed both ways.
        let mut far = cube(2.0);
        for p in &mut far.positions {
            *p += Vec3::new(1000.0, 0.0, 0.0);
        }
        let object = to_object(&far, "cube").unwrap();
        assert_eq!(object.kind, ObjectKind::Solid);
        assert_eq!(
            in_single_precision(&object),
            SinglePrecision {
                merged: 0,
                folded: 0,
                closed: true
            }
        );
        // Its top made a fan round a point just inside one corner:
        // closed in double precision; in single precision, where a unit in
        // the last place is 6e-5 in x and 2e-7 in y there, that point is
        // the corner, and the two triangles of the fan on it are lines, of
        // no area: left out, the rest is the cube's top in two triangles,
        // and the file a closed surface.
        let mut cut = object.clone();
        cut.triangles
            .retain(|t| !t.iter().all(|&k| cut.positions[k as usize][2] == 2.0));
        assert_eq!(cut.triangles.len(), 10);
        let open = cut.clone();
        let corner = |x: f64, y: f64| {
            cut.positions
                .iter()
                .position(|&p| p == [x, y, 2.0])
                .unwrap() as u32
        };
        let square = [
            corner(1000.0, 0.0),
            corner(1002.0, 0.0),
            corner(1002.0, 2.0),
            corner(1000.0, 2.0),
        ];
        let middle = cut.positions.len() as u32;
        cut.positions.push([1002.0 - 1e-6, 2.0 - 1e-8, 2.0]);
        for i in 0..4 {
            cut.triangles.push([square[i], square[(i + 1) % 4], middle]);
        }
        let welded = craft3d_tess::Welded {
            positions: cut.positions.clone(),
            triangles: cut.triangles.clone(),
        };
        assert!(welded.is_closed(), "the fan in double precision");
        assert_eq!(
            in_single_precision(&cut),
            SinglePrecision {
                merged: 1,
                folded: 2,
                closed: true
            }
        );
        // Its top made of two fans instead, round two points either side
        // of its diagonal from (1000, 0) to (1002, 2), closer together than
        // single precision tells apart there (6e-5 in x, 6e-8 in y at
        // (1001, 1)), with no side between them: in a file one point, from
        // which the sides to the diagonal's ends are used twice each way.
        // No triangle folds, and the file is no closed surface.
        let mut two = open;
        let [s0, s1, s2, s3] = square;
        let below = two.positions.len() as u32;
        let above = below + 1;
        two.positions.push([1001.0 + 1e-6, 1.0 - 1e-9, 2.0]);
        two.positions.push([1001.0 - 1e-6, 1.0 + 1e-9, 2.0]);
        two.triangles.extend([
            [s0, s1, below],
            [s1, s2, below],
            [s2, s0, below],
            [s2, s3, above],
            [s3, s0, above],
            [s0, s2, above],
        ]);
        let welded = craft3d_tess::Welded {
            positions: two.positions.clone(),
            triangles: two.triangles.clone(),
        };
        assert!(welded.is_closed(), "the two fans in double precision");
        assert_eq!(
            in_single_precision(&two),
            SinglePrecision {
                merged: 1,
                folded: 0,
                closed: false
            }
        );
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
