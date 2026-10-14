//! STL, 3MF and OBJ writers with their own mesh model.
//!
//! Standalone: depends on no workspace crate (docs/architecture.md §3)
//! except the equally standalone `craft3d-zipstore`, the ZIP container of
//! 3MF. The mapping from 3DCraft meshes lives in `craft3d-io`.
//!
//! Sources: the 3D Systems StereoLithography interface description (1989)
//! for STL; the 3MF Core Specification and the Open Packaging Conventions it
//! builds on (a ZIP container) for 3MF; the Wavefront OBJ format description.

use std::fmt::{self, Write as _};

/// Length unit of the coordinates. Only 3MF records it; STL and OBJ have no
/// notion of units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Unit {
    Micron,
    #[default]
    Millimeter,
    Centimeter,
    Inch,
    Foot,
    Meter,
}

impl Unit {
    /// The unit's name in a 3MF `<model unit="…">` attribute.
    pub fn as_3mf(self) -> &'static str {
        match self {
            Unit::Micron => "micron",
            Unit::Millimeter => "millimeter",
            Unit::Centimeter => "centimeter",
            Unit::Inch => "inch",
            Unit::Foot => "foot",
            Unit::Meter => "meter",
        }
    }
}

/// Whether an object encloses a volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ObjectKind {
    /// A closed, consistently oriented mesh.
    #[default]
    Solid,
    /// An open mesh: a sheet, or surfaces that do not close up.
    Surface,
}

/// One indexed triangle mesh.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Object {
    pub name: String,
    pub kind: ObjectKind,
    pub positions: Vec<[f64; 3]>,
    /// Counter-clockwise seen from outside.
    pub triangles: Vec<[u32; 3]>,
    /// The object's material, an index into [`Model::materials`]: what a
    /// triangle without one of its own is made of. `None`: none (written
    /// without a colour), unless a triangle has one (see
    /// [`Object::triangle_materials`]).
    pub material: Option<u32>,
    /// Empty, or one entry per triangle: the triangle's own material, an
    /// index into [`Model::materials`], or `None` for the object's. In
    /// 3MF an object whose triangles have materials needs one itself: the
    /// first triangle's stands in when it has none.
    pub triangle_materials: Vec<Option<u32>>,
}

/// A material by its name and colour (sRGB): a `<base>` of the 3MF base
/// material group, which slicers and viewers show as the colour of what
/// is made of it. STL and OBJ files have no colours and leave it out.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Material {
    pub name: String,
    pub color: [u8; 3],
}

impl Material {
    /// The colour as 3MF writes it: `#RRGGBB`.
    pub fn display_color(&self) -> String {
        let [r, g, b] = self.color;
        format!("#{r:02X}{g:02X}{b:02X}")
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Model {
    pub unit: Unit,
    pub objects: Vec<Object>,
    /// What the objects and triangles are made of, by index.
    pub materials: Vec<Material>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MeshError {
    /// A triangle refers to a vertex that does not exist.
    IndexOutOfRange {
        object: usize,
        triangle: usize,
    },
    NonFinitePosition {
        object: usize,
        vertex: usize,
    },
    /// A coordinate does not fit the format's 32-bit floats (binary STL).
    CoordinateTooLarge {
        object: usize,
        vertex: usize,
    },
    /// More triangles, vertices or bytes than the format can count.
    TooLarge(&'static str),
    /// A material index past [`Model::materials`], or a list of triangle
    /// materials that is not one per triangle.
    BadMaterial {
        object: usize,
    },
}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MeshError::IndexOutOfRange { object, triangle } => write!(
                f,
                "object {object}: triangle {triangle} refers to a vertex that does not exist"
            ),
            MeshError::NonFinitePosition { object, vertex } => {
                write!(
                    f,
                    "object {object}: vertex {vertex} is not finite (NaN or infinity)"
                )
            }
            MeshError::CoordinateTooLarge { object, vertex } => write!(
                f,
                "object {object}: vertex {vertex} does not fit a 32-bit float, which binary STL requires"
            ),
            MeshError::TooLarge(what) => write!(f, "too large for this format: {what}"),
            MeshError::BadMaterial { object } => write!(
                f,
                "object {object}: a material that does not exist, or triangle materials that are not one per triangle"
            ),
        }
    }
}

impl std::error::Error for MeshError {}

impl Model {
    /// Checks what every writer relies on: finite positions and triangle
    /// indices in range.
    pub fn validate(&self) -> Result<(), MeshError> {
        for (object, o) in self.objects.iter().enumerate() {
            if let Some(vertex) = o
                .positions
                .iter()
                .position(|p| p.iter().any(|c| !c.is_finite()))
            {
                return Err(MeshError::NonFinitePosition { object, vertex });
            }
            let count = o.positions.len();
            if let Some(triangle) = o
                .triangles
                .iter()
                .position(|t| t.iter().any(|&k| k as usize >= count))
            {
                return Err(MeshError::IndexOutOfRange { object, triangle });
            }
            let known = |m: &Option<u32>| m.is_none_or(|m| (m as usize) < self.materials.len());
            if !known(&o.material)
                || !o.triangle_materials.iter().all(known)
                || !(o.triangle_materials.is_empty()
                    || o.triangle_materials.len() == o.triangles.len())
            {
                return Err(MeshError::BadMaterial { object });
            }
        }
        Ok(())
    }
}

/// The triangles of `o` as three corner positions each, in order. Triangles
/// with an index out of range are skipped; [`Model::validate`] rejects them
/// before any writer gets here.
fn corners(o: &Object) -> impl Iterator<Item = [[f64; 3]; 3]> + '_ {
    o.triangles.iter().filter_map(|&[a, b, c]| {
        Some([
            *o.positions.get(a as usize)?,
            *o.positions.get(b as usize)?,
            *o.positions.get(c as usize)?,
        ])
    })
}

/// Unit normal of a triangle by the right-hand rule; zero for a degenerate
/// triangle, as STL readers expect.
fn facet_normal([a, b, c]: [[f64; 3]; 3]) -> [f64; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len.is_finite() && len > 0.0 {
        n.map(|c| c / len)
    } else {
        [0.0; 3]
    }
}

/// A name that is safe on one line of a text format.
fn single_line(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Binary STL: an 80-byte header, a triangle count, then 50 bytes per
/// triangle (normal, three corners, attribute word), all little-endian.
/// Objects are concatenated: STL has no notion of separate objects.
pub fn write_stl_binary(model: &Model) -> Result<Vec<u8>, MeshError> {
    model.validate()?;
    let count: usize = model.objects.iter().map(|o| o.triangles.len()).sum();
    let count = u32::try_from(count)
        .map_err(|_| MeshError::TooLarge("more than 2^32 - 1 triangles in an STL file"))?;
    for (object, o) in model.objects.iter().enumerate() {
        let limit = f64::from(f32::MAX);
        if let Some(vertex) = o
            .positions
            .iter()
            .position(|p| p.iter().any(|c| c.abs() > limit))
        {
            return Err(MeshError::CoordinateTooLarge { object, vertex });
        }
    }
    let mut out = Vec::with_capacity(84 + count as usize * 50);
    // Must not begin with "solid", which marks the ASCII variant.
    let mut header = [b' '; 80];
    for (slot, byte) in header.iter_mut().zip(b"3DCraft binary STL") {
        *slot = *byte;
    }
    out.extend_from_slice(&header);
    out.extend_from_slice(&count.to_le_bytes());
    for tri in model.objects.iter().flat_map(corners) {
        for v in std::iter::once(facet_normal(tri)).chain(tri) {
            for c in v {
                // Range-checked above; rounding to f32 is the format.
                out.extend_from_slice(&(c as f32).to_le_bytes());
            }
        }
        out.extend_from_slice(&0_u16.to_le_bytes());
    }
    Ok(out)
}

/// ASCII STL, one `solid` block per object.
pub fn write_stl_ascii(model: &Model) -> Result<String, MeshError> {
    model.validate()?;
    let mut out = String::new();
    for o in &model.objects {
        let name = single_line(&o.name);
        // Writing to a String cannot fail.
        let _ = writeln!(out, "solid {name}");
        for tri in corners(o) {
            let n = facet_normal(tri);
            let _ = writeln!(out, "  facet normal {} {} {}", n[0], n[1], n[2]);
            let _ = writeln!(out, "    outer loop");
            for v in tri {
                let _ = writeln!(out, "      vertex {} {} {}", v[0], v[1], v[2]);
            }
            let _ = writeln!(out, "    endloop");
            let _ = writeln!(out, "  endfacet");
        }
        let _ = writeln!(out, "endsolid {name}");
    }
    Ok(out)
}

/// Wavefront OBJ: one `o` block per object, vertex indices counted from 1
/// across the whole file.
pub fn write_obj(model: &Model) -> Result<String, MeshError> {
    model.validate()?;
    let mut out = String::from("# 3DCraft\n");
    let mut offset: u64 = 1;
    for o in &model.objects {
        let _ = writeln!(out, "o {}", single_line(&o.name));
        for p in &o.positions {
            let _ = writeln!(out, "v {} {} {}", p[0], p[1], p[2]);
        }
        for t in &o.triangles {
            let [a, b, c] = t.map(|k| u64::from(k) + offset);
            let _ = writeln!(out, "f {a} {b} {c}");
        }
        offset += o.positions.len() as u64;
    }
    Ok(out)
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 forbids most control characters outright.
            c if c.is_control() && !matches!(c, '\t' | '\n' | '\r') => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

const CONTENT_TYPES: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
    "<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
    "<Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>",
    "</Types>\n"
);

const RELS: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    "<Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" ",
    "Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>",
    "</Relationships>\n"
);

/// The `3D/3dmodel.model` part of a 3MF package.
pub fn model_xml_3mf(model: &Model) -> Result<String, MeshError> {
    model.validate()?;
    let mut out = String::new();
    let _ = writeln!(out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(
        out,
        "<model unit=\"{}\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">",
        model.unit.as_3mf()
    );
    let _ = writeln!(out, " <resources>");
    // The base material group, when there are materials: its id follows
    // the objects' (ids are unique among the resources).
    let group = model.objects.len().saturating_add(1);
    if !model.materials.is_empty() {
        let _ = writeln!(out, "  <basematerials id=\"{group}\">");
        for material in &model.materials {
            let _ = writeln!(
                out,
                "   <base name=\"{}\" displaycolor=\"{}\"/>",
                xml_escape(&material.name),
                material.display_color()
            );
        }
        let _ = writeln!(out, "  </basematerials>");
    }
    for (index, o) in model.objects.iter().enumerate() {
        let kind = match o.kind {
            ObjectKind::Solid => "model",
            ObjectKind::Surface => "surface",
        };
        // The object's own material, or the first triangle's for an
        // object whose triangles have them (3MF wants one on the object).
        let default = o
            .material
            .or_else(|| o.triangle_materials.iter().find_map(|m| *m));
        let property = default
            .map(|m| format!(" pid=\"{group}\" pindex=\"{m}\""))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  <object id=\"{}\" type=\"{kind}\" name=\"{}\"{property}>",
            index + 1,
            xml_escape(&o.name)
        );
        let _ = writeln!(out, "   <mesh>");
        let _ = writeln!(out, "    <vertices>");
        for p in &o.positions {
            let _ = writeln!(
                out,
                "     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>",
                p[0], p[1], p[2]
            );
        }
        let _ = writeln!(out, "    </vertices>");
        let _ = writeln!(out, "    <triangles>");
        for (t, [a, b, c]) in o.triangles.iter().enumerate() {
            match o.triangle_materials.get(t).copied().flatten() {
                Some(m) if Some(m) != default => {
                    let _ = writeln!(
                        out,
                        "     <triangle v1=\"{a}\" v2=\"{b}\" v3=\"{c}\" pid=\"{group}\" p1=\"{m}\"/>"
                    );
                }
                _ => {
                    let _ = writeln!(out, "     <triangle v1=\"{a}\" v2=\"{b}\" v3=\"{c}\"/>");
                }
            }
        }
        let _ = writeln!(out, "    </triangles>");
        let _ = writeln!(out, "   </mesh>");
        let _ = writeln!(out, "  </object>");
    }
    let _ = writeln!(out, " </resources>");
    let _ = writeln!(out, " <build>");
    for index in 0..model.objects.len() {
        let _ = writeln!(out, "  <item objectid=\"{}\"/>", index + 1);
    }
    let _ = writeln!(out, " </build>");
    let _ = writeln!(out, "</model>");
    Ok(out)
}

/// A 3MF package: a ZIP container with the content types, the root
/// relationship and the model part. Entries are stored uncompressed.
pub fn write_3mf(model: &Model) -> Result<Vec<u8>, MeshError> {
    let xml = model_xml_3mf(model)?;
    craft3d_zipstore::write_stored(&[
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", RELS.as_bytes()),
        ("3D/3dmodel.model", xml.as_bytes()),
    ])
    // The names are fixed and valid, so size is the only way this fails.
    .map_err(|_| MeshError::TooLarge("a 3MF package of 4 GiB or more (ZIP64 is not supported)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The corner tetrahedron: volume 1/6, outward-facing triangles.
    fn tetrahedron() -> Object {
        Object {
            name: "tetra".into(),
            kind: ObjectKind::Solid,
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            triangles: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
            ..Object::default()
        }
    }

    fn model(objects: Vec<Object>) -> Model {
        Model {
            unit: Unit::Millimeter,
            objects,
            materials: Vec::new(),
        }
    }

    fn one_triangle() -> Object {
        Object {
            name: "t".into(),
            kind: ObjectKind::Surface,
            positions: vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 0.5, 0.0]],
            triangles: vec![[0, 1, 2]],
            ..Object::default()
        }
    }

    /// Signed volume by the divergence theorem: positive for outward
    /// orientation.
    fn volume(tris: &[[[f64; 3]; 3]]) -> f64 {
        tris.iter()
            .map(|[a, b, c]| {
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            })
            .sum()
    }

    /// A minimal binary STL reader, for parse-back only.
    fn read_stl_binary(bytes: &[u8]) -> Vec<([f32; 3], [[f64; 3]; 3])> {
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
        assert_eq!(bytes.len(), 84 + count * 50);
        bytes[84..]
            .chunks(50)
            .map(|rec| {
                let f = |i: usize| f32::from_le_bytes(rec[i * 4..i * 4 + 4].try_into().unwrap());
                assert_eq!(&rec[48..50], &[0, 0]);
                let v = |k: usize| [f64::from(f(k)), f64::from(f(k + 1)), f64::from(f(k + 2))];
                ([f(0), f(1), f(2)], [v(3), v(6), v(9)])
            })
            .collect()
    }

    #[test]
    fn binary_stl_parses_back_to_the_same_solid() {
        let bytes = write_stl_binary(&model(vec![tetrahedron()])).unwrap();
        assert_eq!(bytes.len(), 84 + 4 * 50);
        assert!(!bytes.starts_with(b"solid"));
        let facets = read_stl_binary(&bytes);
        let tris: Vec<_> = facets.iter().map(|f| f.1).collect();
        assert!((volume(&tris) - 1.0 / 6.0).abs() < 1e-12);
        // Stored normals are unit length and outward: the base faces -z.
        assert_eq!(facets[0].0, [0.0, 0.0, -1.0]);
        let s = 1.0 / 3.0_f32.sqrt();
        for (got, want) in facets[3].0.iter().zip([s, s, s]) {
            assert!((got - want).abs() < 1e-6);
        }
    }

    #[test]
    fn ascii_stl_is_exactly_as_specified() {
        let text = write_stl_ascii(&model(vec![one_triangle()])).unwrap();
        let expected = "\
solid t
  facet normal 0 0 1
    outer loop
      vertex 0 0 0
      vertex 2 0 0
      vertex 0 0.5 0
    endloop
  endfacet
endsolid t
";
        assert_eq!(text, expected);
    }

    #[test]
    fn obj_counts_indices_across_objects_from_one() {
        let text = write_obj(&model(vec![one_triangle(), one_triangle()])).unwrap();
        let expected = "\
# 3DCraft
o t
v 0 0 0
v 2 0 0
v 0 0.5 0
f 1 2 3
o t
v 0 0 0
v 2 0 0
v 0 0.5 0
f 4 5 6
";
        assert_eq!(text, expected);
    }

    #[test]
    fn model_xml_is_exactly_as_specified() {
        let mut m = model(vec![one_triangle()]);
        m.unit = Unit::Inch;
        m.objects[0].name = "a<b & \"c\"\u{1}".into();
        let expected = "\
<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<model unit=\"inch\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">
 <resources>
  <object id=\"1\" type=\"surface\" name=\"a&lt;b &amp; &quot;c&quot; \">
   <mesh>
    <vertices>
     <vertex x=\"0\" y=\"0\" z=\"0\"/>
     <vertex x=\"2\" y=\"0\" z=\"0\"/>
     <vertex x=\"0\" y=\"0.5\" z=\"0\"/>
    </vertices>
    <triangles>
     <triangle v1=\"0\" v2=\"1\" v3=\"2\"/>
    </triangles>
   </mesh>
  </object>
 </resources>
 <build>
  <item objectid=\"1\"/>
 </build>
</model>
";
        assert_eq!(model_xml_3mf(&m).unwrap(), expected);
        assert!(
            model_xml_3mf(&model(vec![tetrahedron()]))
                .unwrap()
                .contains("type=\"model\"")
        );
    }

    #[test]
    fn a_3mf_package_has_the_three_required_parts() {
        let m = model(vec![tetrahedron()]);
        let bytes = write_3mf(&m).unwrap();
        let entries = craft3d_zipstore::read_stored(&bytes).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name).collect();
        assert_eq!(
            names,
            ["[Content_Types].xml", "_rels/.rels", "3D/3dmodel.model"]
        );
        assert_eq!(entries[0].data, CONTENT_TYPES.as_bytes());
        assert_eq!(entries[1].data, RELS.as_bytes());
        assert_eq!(entries[2].data, model_xml_3mf(&m).unwrap().as_bytes());
        // The relationship points at the model part, whose extension has a
        // registered content type.
        assert!(RELS.contains("Target=\"/3D/3dmodel.model\""));
        assert!(CONTENT_TYPES.contains("Extension=\"model\""));
    }

    #[test]
    fn numbers_round_trip_through_text() {
        let awkward = [
            0.1 + 0.2,
            -1.0e-7,
            123_456_789.123_456_79,
            f64::MIN_POSITIVE,
        ];
        let o = Object {
            name: String::new(),
            kind: ObjectKind::Surface,
            positions: vec![
                [awkward[0], awkward[1], awkward[2]],
                [awkward[3], 0.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            triangles: vec![[0, 1, 2]],
            ..Object::default()
        };
        let text = write_obj(&model(vec![o])).unwrap();
        let parsed: Vec<f64> = text
            .lines()
            .filter(|l| l.starts_with("v "))
            .flat_map(|l| l.split(' ').skip(1).map(|n| n.parse::<f64>().unwrap()))
            .collect();
        assert_eq!(&parsed[..4], &awkward);
        // No exponent notation, which not every OBJ or XML reader accepts.
        assert!(!text.contains('e') && !text.contains('E'));
    }

    #[test]
    fn bad_models_are_errors_in_every_writer() {
        let mut bad_index = tetrahedron();
        bad_index.triangles[2] = [0, 1, 4];
        let mut nan = tetrahedron();
        nan.positions[1][2] = f64::NAN;
        let cases = [
            (
                bad_index,
                MeshError::IndexOutOfRange {
                    object: 1,
                    triangle: 2,
                },
            ),
            (
                nan,
                MeshError::NonFinitePosition {
                    object: 1,
                    vertex: 1,
                },
            ),
        ];
        for (bad, expected) in cases {
            let m = model(vec![tetrahedron(), bad]);
            assert_eq!(write_stl_binary(&m), Err(expected.clone()));
            assert_eq!(write_stl_ascii(&m), Err(expected.clone()));
            assert_eq!(write_obj(&m), Err(expected.clone()));
            assert_eq!(write_3mf(&m), Err(expected));
        }
        let mut huge = tetrahedron();
        huge.positions[3] = [0.0, 0.0, 1e39];
        let m = model(vec![huge]);
        assert_eq!(
            write_stl_binary(&m),
            Err(MeshError::CoordinateTooLarge {
                object: 0,
                vertex: 3
            })
        );
        // The text formats carry f64 and accept it.
        assert!(write_obj(&m).is_ok() && write_3mf(&m).is_ok() && write_stl_ascii(&m).is_ok());
    }

    #[test]
    fn an_empty_model_is_a_valid_empty_file() {
        let m = model(vec![]);
        assert_eq!(write_stl_binary(&m).unwrap().len(), 84);
        assert_eq!(write_stl_ascii(&m).unwrap(), "");
        assert_eq!(write_obj(&m).unwrap(), "# 3DCraft\n");
        assert_eq!(
            craft3d_zipstore::read_stored(&write_3mf(&m).unwrap())
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn names_cannot_break_out_of_their_line() {
        let mut o = one_triangle();
        o.name = "evil\nendsolid\r\nf 1 1 1".into();
        let m = model(vec![o]);
        assert_eq!(write_stl_ascii(&m).unwrap().lines().count(), 9);
        assert_eq!(write_obj(&m).unwrap().lines().count(), 6);
    }

    /// The value of attribute `name` in an XML start tag `tag`.
    fn attribute(tag: &str, name: &str) -> Option<String> {
        let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
        let end = tag[start..].find('"')? + start;
        Some(tag[start..end].to_string())
    }

    #[test]
    fn materials_are_written_as_3mf_base_materials_and_read_back() {
        let mut red = tetrahedron();
        red.name = "red".into();
        red.material = Some(1);
        let mut mixed = tetrahedron();
        mixed.name = "mixed".into();
        // Triangle 2 of its own; the rest the object's.
        mixed.material = Some(0);
        mixed.triangle_materials = vec![None, None, Some(1), Some(0)];
        let mut tool_only = tetrahedron();
        tool_only.name = "only some".into();
        tool_only.triangle_materials = vec![None, Some(1), None, None];
        let plain = tetrahedron();
        let m = Model {
            unit: Unit::Millimeter,
            objects: vec![red, mixed, tool_only, plain],
            materials: vec![
                Material {
                    name: "Steel & co".into(),
                    color: [128, 130, 135],
                },
                Material {
                    name: "Red".into(),
                    color: [200, 30, 4],
                },
            ],
        };
        let xml = model_xml_3mf(&m).unwrap();
        // Read back: the base materials, then each object's and
        // triangle's colour.
        let bases: Vec<(String, String)> = xml
            .lines()
            .filter(|l| l.trim_start().starts_with("<base "))
            .map(|l| {
                (
                    attribute(l, "name").unwrap(),
                    attribute(l, "displaycolor").unwrap(),
                )
            })
            .collect();
        assert_eq!(
            bases,
            [
                ("Steel &amp; co".to_string(), "#808287".to_string()),
                ("Red".to_string(), "#C81E04".to_string())
            ]
        );
        assert!(xml.contains("<basematerials id=\"5\">"), "{xml}");
        let mut objects = Vec::new();
        let mut current: Option<(String, Option<String>, Vec<Option<String>>)> = None;
        for line in xml.lines().map(str::trim_start) {
            if line.starts_with("<object ") {
                if let Some(pid) = attribute(line, "pid") {
                    assert_eq!(pid, "5");
                }
                current = Some((
                    attribute(line, "name").unwrap(),
                    attribute(line, "pindex"),
                    Vec::new(),
                ));
            } else if line.starts_with("<triangle ") {
                let p1 = attribute(line, "p1");
                if p1.is_some() {
                    assert_eq!(attribute(line, "pid").as_deref(), Some("5"));
                }
                current.as_mut().unwrap().2.push(p1);
            } else if line.starts_with("</object>") {
                objects.push(current.take().unwrap());
            }
        }
        let s = |x: &str| Some(x.to_string());
        assert_eq!(
            objects,
            [
                ("red".into(), s("1"), vec![None; 4]),
                ("mixed".into(), s("0"), vec![None, None, s("1"), None]),
                ("only some".into(), s("1"), vec![None; 4]),
                ("tetra".into(), None, vec![None; 4]),
            ]
        );
        // The other formats are written as before.
        assert_eq!(
            write_obj(&m).unwrap(),
            write_obj(&Model {
                materials: Vec::new(),
                objects: m
                    .objects
                    .iter()
                    .map(|o| Object {
                        material: None,
                        triangle_materials: Vec::new(),
                        ..o.clone()
                    })
                    .collect(),
                ..m.clone()
            })
            .unwrap()
        );
        // A material that does not exist is an error.
        let mut bad = m.clone();
        bad.objects[0].material = Some(2);
        assert_eq!(
            model_xml_3mf(&bad),
            Err(MeshError::BadMaterial { object: 0 })
        );
        let mut short = m;
        short.objects[1].triangle_materials.pop();
        assert_eq!(write_3mf(&short), Err(MeshError::BadMaterial { object: 1 }));
    }
}
