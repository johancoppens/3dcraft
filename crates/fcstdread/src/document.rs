//! What FreeCAD's `Document.xml` and `GuiDocument.xml` say of the objects
//! of a document, as far as an import needs it: name, type, label,
//! visibility, placement, the file that holds the shape, the objects it
//! uses, and the colour its view gives it.
//!
//! Read from files FreeCAD writes (format described by the files
//! themselves: `<Objects>` lists every object with its type,
//! `<ObjectData>` its properties, each `<Property name type>` holding one
//! element with the value).

use crate::xml::Element;
use craft3d_brep::Matrix;
use std::collections::BTreeMap;

/// One object of the document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Object {
    pub name: String,
    /// `Part::Box`, `PartDesign::Body`, `App::Part`, …
    pub type_name: String,
    pub label: String,
    /// The `Visibility` property, where the file has one.
    pub visible: Option<bool>,
    /// The `Placement` property as a matrix.
    pub placement: Option<Matrix>,
    /// The ZIP entry that holds the `Shape` property, if any.
    pub shape_file: Option<String>,
    /// Objects it consumes through its link properties ([`CONSUMES`]): a
    /// cut its base and tool, a body its features.
    pub uses: Vec<String>,
    /// The objects of its `Group` (an `App::Part`'s children).
    pub group: Vec<String>,
}

/// The colour a view gives an object: one material per face, or one for
/// all, and the entry that holds them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub visible: Option<bool>,
    /// `ShapeAppearance`'s `MaterialList file`.
    pub materials: Option<String>,
    /// `ShapeColor`, packed `0xRRGGBBAA` (older files).
    pub color: Option<u32>,
}

/// Link properties through which an object consumes others, so that they
/// are steps towards it rather than results of their own: a boolean's
/// `Base` and `Tool` (or `Shapes`), a compound's `Links`, a feature's
/// `Source`, `Base` or `BaseFeature`, a body's `Group` and `Tip`, a loft's
/// or a sweep's `Sections` and `Spine`, a pattern's `Originals`. Links that
/// only refer (a sketch's `ExternalGeometry`, an attachment, a pad's
/// `UpToFace`, a revolution's `ReferenceAxis`) consume nothing.
const CONSUMES: [&str; 16] = [
    "Base",
    "Tool",
    "Tools",
    "Shapes",
    "Links",
    "Group",
    "BaseFeature",
    "Tip",
    "Source",
    "Sources",
    "Objects",
    "Profile",
    "Sections",
    "Spine",
    "Original",
    "Originals",
];

/// A rotation `(x, y, z, w)` and a translation as a matrix.
pub fn placement_matrix(q: [f64; 4], t: [f64; 3]) -> Option<Matrix> {
    let [x, y, z, w] = q;
    let n = (x * x + y * y + z * z + w * w).sqrt();
    if !(n.is_finite() && n > 1e-12) || t.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let (x, y, z, w) = (x / n, y / n, z / n, w / n);
    Some([
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
            t[0],
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
            t[1],
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
            t[2],
        ],
    ])
}

fn number(e: &Element, name: &str) -> Option<f64> {
    e.attribute(name)?.trim().parse::<f64>().ok()
}

fn placement(e: &Element) -> Option<Matrix> {
    let p = e.child("PropertyPlacement")?;
    let t = [number(p, "Px")?, number(p, "Py")?, number(p, "Pz")?];
    let q = [
        number(p, "Q0")?,
        number(p, "Q1")?,
        number(p, "Q2")?,
        number(p, "Q3")?,
    ];
    placement_matrix(q, t)
}

fn boolean(e: &Element) -> Option<bool> {
    match e.child("Bool")?.attribute("value")? {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// The objects of `Document.xml`, in the order of its `<Objects>` list.
pub fn objects(root: &Element) -> Result<Vec<Object>, String> {
    if root.name != "Document" {
        return Err(format!(
            "Document.xml's root is `{}`, not `Document`",
            root.name.chars().take(32).collect::<String>()
        ));
    }
    let mut out: Vec<Object> = Vec::new();
    let mut at: BTreeMap<String, usize> = BTreeMap::new();
    if let Some(list) = root.child("Objects") {
        for o in list.named("Object") {
            let Some(name) = o.attribute("name") else {
                continue;
            };
            if at.contains_key(name) {
                return Err(format!("two objects are named `{name}`"));
            }
            at.insert(name.to_string(), out.len());
            out.push(Object {
                name: name.to_string(),
                type_name: o.attribute("type").unwrap_or("").to_string(),
                label: name.to_string(),
                ..Object::default()
            });
        }
    }
    let Some(data) = root.child("ObjectData") else {
        return Ok(out);
    };
    for o in data.named("Object") {
        let Some(object) = o
            .attribute("name")
            .and_then(|n| at.get(n))
            .and_then(|&i| out.get_mut(i))
        else {
            continue;
        };
        let Some(properties) = o.child("Properties") else {
            continue;
        };
        for p in properties.named("Property") {
            let name = p.attribute("name").unwrap_or("");
            let kind = p.attribute("type").unwrap_or("");
            match name {
                "Label" => {
                    if let Some(v) = p.child("String").and_then(|s| s.attribute("value")) {
                        object.label = v.to_string();
                    }
                }
                "Visibility" => object.visible = boolean(p),
                "Placement" if kind == "App::PropertyPlacement" => {
                    object.placement = placement(p);
                }
                "Shape" if kind.contains("PropertyPartShape") => {
                    object.shape_file = p
                        .child("Part")
                        .and_then(|e| e.attribute("file"))
                        .filter(|f| !f.is_empty())
                        .map(str::to_string);
                }
                _ => {}
            }
            if kind.starts_with("App::PropertyLink")
                && (CONSUMES.contains(&name) || name == "Group")
            {
                let mut targets = Vec::new();
                for e in p.descendants() {
                    for key in ["value", "obj"] {
                        if let Some(v) = e.attribute(key)
                            && (e.name == "Link" || e.name == "LinkSub")
                            && !v.is_empty()
                        {
                            targets.push(v.to_string());
                        }
                    }
                }
                if name == "Group" {
                    object.group.extend(targets.iter().cloned());
                }
                if CONSUMES.contains(&name) {
                    object.uses.extend(targets);
                }
            }
        }
    }
    Ok(out)
}

/// The views of `GuiDocument.xml`, by object name.
pub fn views(root: &Element) -> BTreeMap<String, View> {
    let mut out = BTreeMap::new();
    let Some(data) = root.child("ViewProviderData") else {
        return out;
    };
    for v in data.named("ViewProvider") {
        let Some(name) = v.attribute("name") else {
            continue;
        };
        let mut view = View::default();
        if let Some(properties) = v.child("Properties") {
            for p in properties.named("Property") {
                match p.attribute("name").unwrap_or("") {
                    "Visibility" => view.visible = boolean(p),
                    "ShapeAppearance" => {
                        view.materials = p
                            .child("MaterialList")
                            .and_then(|m| m.attribute("file"))
                            .map(str::to_string);
                    }
                    "ShapeColor" => {
                        view.color = p
                            .child("PropertyColor")
                            .and_then(|c| c.attribute("value"))
                            .and_then(|v| v.trim().parse::<u32>().ok());
                    }
                    _ => {}
                }
            }
        }
        out.insert(name.to_string(), view);
    }
    out
}

/// The diffuse colour of each material of a `MaterialList` entry (count,
/// then per material ambient, diffuse, specular and emissive colours as
/// packed little-endian `0xRRGGBBAA`, shininess, transparency and, in
/// later versions, a few more fields after them). Only as many materials
/// as can be read for certain: the fields after the first material's
/// colours vary with the version, so a list of several is read only when
/// its size says how long each one is.
pub fn diffuse_colors(bytes: &[u8]) -> Vec<[u8; 3]> {
    let u32_at = |at: usize| -> Option<u32> {
        let b = bytes.get(at..at.checked_add(4)?)?;
        Some(u32::from_le_bytes([
            *b.first()?,
            *b.get(1)?,
            *b.get(2)?,
            *b.get(3)?,
        ]))
    };
    let Some(count) = u32_at(0).and_then(|c| usize::try_from(c).ok()) else {
        return Vec::new();
    };
    if count == 0 || count > bytes.len() / 4 {
        return Vec::new();
    }
    let rgb = |packed: u32| {
        [
            (packed >> 24) as u8,
            ((packed >> 16) & 0xff) as u8,
            ((packed >> 8) & 0xff) as u8,
        ]
    };
    // Each material takes the same number of bytes when no strings follow
    // with lengths of their own (FreeCAD writes empty ones).
    let each = (bytes.len() - 4) / count;
    if each < 8 || !(bytes.len() - 4).is_multiple_of(count) {
        return u32_at(8).map(rgb).into_iter().take(1).collect();
    }
    (0..count)
        .map_while(|k| u32_at(4 + k * each + 4).map(rgb))
        .collect()
}
