//! FreeCAD documents (`.FCStd`) read into the bodies of `craft3d-topo`,
//! the way `craft3d-stepread` reads STEP files: the same options, report
//! and result, and the same sewing, edge proofs and checks
//! (`craft3d_stepread::finish`).
//!
//! An `.FCStd` file is a ZIP archive (`craft3d-zipstore`, deflated
//! entries): `Document.xml` lists the objects with their properties
//! (label, visibility, placement, the entry that holds each shape and the
//! objects each uses), `GuiDocument.xml` the views (colours), and each
//! shape is an entry in Open CASCADE's text BRep format
//! (`craft3d-brep`). FreeCAD's unit is the millimetre.
//!
//! # Which objects
//!
//! The document's final shapes, not every step towards them: an object
//! with a shape is imported when it is visible and no other object with a
//! shape uses it through a link (a cut's base and tool, a body's
//! features, a fillet's base: those are steps; an attachment is not a
//! use). A PartDesign body is imported as itself (its tip's shape), a
//! sketch or a wire not at all (no faces). Each solid of the object's
//! shape is one item, named after the object's label (`Label`, then
//! `Label-2`, … for further solids); its shells and faces outside any
//! solid are one sheet. Objects inside an `App::Part` are placed by the
//! part's placement too (the shape holds only the object's own).
//! Colours: the view's `ShapeAppearance` (one material for the whole
//! shape, or one per face), else its `ShapeColor`.
//!
//! # Refused
//!
//! Shapes saved in binary BRep (FreeCAD's option to save them so),
//! curves and surfaces of the kinds `shape` lists as not read, locations
//! that scale or mirror, files over [`MAX_IMPORT_BYTES`], entries larger
//! than [`MAX_ENTRY_BYTES`] once decompressed, and every malformation:
//! each an error that names the object and the shape, never a panic.

mod document;
mod shape;
mod xml;

pub use craft3d_stepread::{ImportError, Imported, ItemKind, MAX_IMPORT_BYTES, Options, Report};

use craft3d_brep::Matrix;
use craft3d_stepread::finish::{Assembled, finish};
use craft3d_topo::BodyKind;
use shape::{Numbering, Parts, Shape, SheetPart, Use};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

/// The largest entry read, once decompressed: 256 MiB.
pub const MAX_ENTRY_BYTES: usize = 256 * 1024 * 1024;

pub(crate) fn err<T>(message: impl Into<String>) -> Result<T, ImportError> {
    Err(ImportError(message.into()))
}

/// Whether bytes look like an `.FCStd` file (a ZIP archive) rather than
/// a STEP file.
pub fn is_fcstd(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

/// Whether bytes look like a text BRep file (`.brep`, `.brp`), which
/// [`parse`] reads as one object.
pub fn is_brep(bytes: &[u8]) -> bool {
    let head = bytes
        .get(..bytes.len().min(256))
        .unwrap_or(&[])
        .trim_ascii_start();
    head.starts_with(b"CASCADE Topology V") || head.starts_with(b"DBRep_DrawableShape")
}

/// Whether [`parse`] reads these bytes (a FreeCAD document or a BRep
/// file).
pub fn reads(bytes: &[u8]) -> bool {
    is_fcstd(bytes) || is_brep(bytes)
}

/// One solid or sheet of the document.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The object's internal name (`Cut`).
    pub object: String,
    /// The object's label (`Plate with hole`).
    pub label: String,
    /// The name for the item: the label, numbered on for further solids
    /// of one object.
    pub name: String,
    pub kind: ItemKind,
    /// The object's colour (sRGB), the most common one where its faces
    /// have several.
    pub color: Option<[u8; 3]>,
}

/// Where one item is in its object's shape.
#[derive(Clone, Copy, Debug)]
enum Part {
    Solid(usize),
    Sheet,
}

struct ObjectShape {
    label: String,
    shape: Shape,
    parts: Parts,
    numbering: Numbering,
    /// One colour per FreeCAD face number (from 1) when the view gives
    /// one per face.
    face_colors: Vec<[u8; 3]>,
}

/// A read document with its items found.
pub struct Parsed {
    objects: Vec<ObjectShape>,
    /// (object, part) of each item.
    at: Vec<(usize, Part)>,
    items: Vec<Item>,
    /// Objects with a shape that were left out, and why.
    skipped: Vec<String>,
}

impl std::fmt::Debug for Parsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Parsed")
            .field("items", &self.items)
            .field("skipped", &self.skipped)
            .finish()
    }
}

/// The control points the faces and edges of one body may hold together
/// (each face and edge holds its own copy).
pub(crate) struct Budget {
    used: usize,
}

impl Budget {
    fn add(&mut self, n: usize) -> Result<(), ImportError> {
        self.used = self.used.saturating_add(n);
        if self.used > craft3d_stepread::MAX_CONTROL_POINT_USES {
            return err(format!(
                "the faces and edges of the body would hold more than {} control points",
                craft3d_stepread::MAX_CONTROL_POINT_USES
            ));
        }
        Ok(())
    }

    pub(crate) fn curve(&mut self, c: &craft3d_brep::Curve) -> Result<(), ImportError> {
        use craft3d_brep::Curve;
        let mut c = c;
        loop {
            match c {
                Curve::BSpline(b) => return self.add(b.poles.len().saturating_mul(2)),
                Curve::Bezier { poles, .. } => return self.add(poles.len()),
                Curve::Trimmed { basis, .. } | Curve::Offset { basis, .. } => c = basis,
                _ => return self.add(1),
            }
        }
    }

    pub(crate) fn surface(&mut self, s: &craft3d_brep::Surface) -> Result<(), ImportError> {
        use craft3d_brep::Surface;
        match s {
            Surface::BSpline(b) => self.add(b.poles.len().saturating_mul(4)),
            Surface::Bezier { poles, .. } => self.add(poles.len()),
            Surface::Extrusion { basis, .. } | Surface::Revolution { basis, .. } => {
                self.curve(basis)?;
                self.add(9)
            }
            Surface::Trimmed { basis, .. } | Surface::Offset { basis, .. } => self.surface(basis),
            _ => self.add(1),
        }
    }
}

/// The most bytes all entries of one document may decompress to
/// together: each entry is decompressed once, however many objects use
/// it.
pub const MAX_INFLATED_BYTES: usize = 2 * MAX_IMPORT_BYTES;

/// The entries of an archive, each decompressed at most once and all
/// within [`MAX_INFLATED_BYTES`].
struct Archive<'a> {
    entries: Vec<craft3d_zipstore::RawEntry<'a>>,
    inflated: BTreeMap<String, Arc<Vec<u8>>>,
    total: usize,
    budget: usize,
}

impl Archive<'_> {
    fn get(&mut self, name: &str) -> Result<Option<Arc<Vec<u8>>>, ImportError> {
        if let Some(data) = self.inflated.get(name) {
            return Ok(Some(Arc::clone(data)));
        }
        let Some(e) = self.entries.iter().find(|e| e.name == name) else {
            return Ok(None);
        };
        if e.size > MAX_ENTRY_BYTES {
            return err(format!(
                "`{name}` is {} bytes once decompressed; at most {MAX_ENTRY_BYTES} are read",
                e.size
            ));
        }
        self.total = self.total.saturating_add(e.size);
        if self.total > self.budget {
            return err(format!(
                "the document's entries decompress to more than {} bytes together",
                self.budget
            ));
        }
        let data = Arc::new(
            e.data()
                .map_err(|e| ImportError(format!("not a readable FreeCAD document: {e}")))?
                .into_owned(),
        );
        self.inflated.insert(name.to_string(), Arc::clone(&data));
        Ok(Some(data))
    }
}

/// Containers whose placement places their children (the children's
/// shapes hold only their own).
fn places_children(type_name: &str) -> bool {
    type_name == "App::Part" || type_name.ends_with("AssemblyObject")
}

/// The placement of the containers (`App::Part`, also through groups
/// inside one) around an object, outermost first, multiplied out.
fn outer_placement(
    objects: &[document::Object],
    parents: &BTreeMap<&str, &str>,
    name: &str,
) -> Matrix {
    let mut chain = Vec::new();
    let mut seen = BTreeSet::new();
    let mut at = name;
    while let Some(&parent) = parents.get(at) {
        if !seen.insert(parent) || seen.len() > 64 {
            break;
        }
        if let Some(p) = objects
            .iter()
            .find(|o| o.name == parent && places_children(&o.type_name))
            .and_then(|o| o.placement)
        {
            chain.push(p);
        }
        at = parent;
    }
    let mut m = craft3d_brep::IDENTITY;
    for p in chain.iter().rev() {
        m = craft3d_brep::multiply(&m, p);
    }
    m
}

/// Reads a document (or a plain BRep file, [`is_brep`]) and finds its
/// items.
pub fn parse(bytes: &[u8]) -> Result<Parsed, ImportError> {
    parse_within(bytes, MAX_INFLATED_BYTES)
}

/// [`parse`] with the entries decompressing to at most `budget` bytes.
fn parse_within(bytes: &[u8], budget: usize) -> Result<Parsed, ImportError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return err(format!(
            "the file is {} bytes; at most {MAX_IMPORT_BYTES} are read",
            bytes.len()
        ));
    }
    if is_brep(bytes) {
        return parse_brep(bytes);
    }
    let mut archive = Archive {
        entries: craft3d_zipstore::read(bytes)
            .map_err(|e| ImportError(format!("not a readable FreeCAD document: {e}")))?,
        inflated: BTreeMap::new(),
        total: 0,
        budget,
    };
    let Some(doc) = archive.get("Document.xml")? else {
        return err("not a FreeCAD document: the archive has no Document.xml");
    };
    let text = std::str::from_utf8(&doc)
        .map_err(|_| ImportError("Document.xml is not UTF-8 text".to_string()))?;
    let root = xml::parse(text).map_err(|e| ImportError(format!("Document.xml: {e}")))?;
    let objects =
        document::objects(&root).map_err(|e| ImportError(format!("Document.xml: {e}")))?;
    drop(root);
    let views = match archive.get("GuiDocument.xml")? {
        Some(gui) => match std::str::from_utf8(&gui)
            .ok()
            .and_then(|t| xml::parse(t).ok())
        {
            Some(root) => document::views(&root),
            // The views only colour; a damaged one costs the colours.
            None => BTreeMap::new(),
        },
        None => BTreeMap::new(),
    };
    let visible = |o: &document::Object| {
        o.visible
            .or_else(|| views.get(&o.name).and_then(|v| v.visible))
            .unwrap_or(true)
    };
    // What consumes what: only a visible object with a shape consumes (an
    // App::Part groups, it does not consume; a hidden cut leaves its
    // operands to be what is seen).
    let mut used_by: BTreeMap<&str, &str> = BTreeMap::new();
    for o in &objects {
        if o.shape_file.is_some() && visible(o) {
            for u in &o.uses {
                if u != &o.name {
                    used_by.entry(u.as_str()).or_insert(o.label.as_str());
                }
            }
        }
    }
    let mut parents: BTreeMap<&str, &str> = BTreeMap::new();
    for o in &objects {
        for child in &o.group {
            if child != &o.name {
                parents.entry(child.as_str()).or_insert(o.name.as_str());
            }
        }
    }
    let mut parsed = Parsed {
        objects: Vec::new(),
        at: Vec::new(),
        items: Vec::new(),
        skipped: Vec::new(),
    };
    // Each shape entry read once, however many objects use it; the walks
    // over all objects' shapes within one bound.
    let mut breps: BTreeMap<String, Arc<craft3d_brep::Brep>> = BTreeMap::new();
    let mut walks = shape::MAX_USES.saturating_mul(2);
    for o in &objects {
        let Some(file) = &o.shape_file else {
            continue;
        };
        if let Some(by) = used_by.get(o.name.as_str()) {
            parsed
                .skipped
                .push(format!("`{}` ({}): used by `{by}`", o.label, o.name));
            continue;
        }
        let view = views.get(&o.name);
        if !visible(o) {
            parsed
                .skipped
                .push(format!("`{}` ({}): hidden", o.label, o.name));
            continue;
        }
        let brep = match breps.get(file) {
            Some(b) => Arc::clone(b),
            None => {
                let Some(bytes) = archive.get(file)? else {
                    return err(format!(
                        "`{}` ({}): its shape `{file}` is not in the archive",
                        o.label, o.name
                    ));
                };
                if !file.ends_with(".brp") && !bytes.trim_ascii_start().starts_with(b"CASCADE") {
                    return err(format!(
                        "`{}` ({}): its shape `{file}` is in binary BRep (FreeCAD's option to save shapes as binary), which is not read; save the document with that option off",
                        o.label, o.name
                    ));
                }
                let b = Arc::new(craft3d_brep::read(&bytes).map_err(|e| {
                    ImportError(format!("`{}` ({}), `{file}`: {e}", o.label, o.name))
                })?);
                breps.insert(file.clone(), Arc::clone(&b));
                b
            }
        };
        let outer = outer_placement(&objects, &parents, &o.name);
        let shape = Shape::new(brep, outer);
        let at = |e: ImportError| ImportError(format!("`{}` ({}): {}", o.label, o.name, e.0));
        let parts = shape.parts(&mut walks).map_err(at)?;
        if parts.solids.is_empty() && parts.sheet.is_empty() {
            parsed
                .skipped
                .push(format!("`{}` ({}): no faces", o.label, o.name));
            continue;
        }
        let numbering = shape.numbering(&mut walks).map_err(at)?;
        let colors = view
            .and_then(|v| v.materials.as_ref())
            .map(|f| archive.get(f))
            .transpose()?
            .flatten()
            .map(|b| document::diffuse_colors(&b))
            .unwrap_or_default();
        let mut color = match colors.as_slice() {
            [] => None,
            [one] => Some(*one),
            many => most_common(many),
        };
        if color.is_none()
            && let Some(packed) = view.and_then(|v| v.color)
        {
            color = Some([
                (packed >> 24) as u8,
                ((packed >> 16) & 0xff) as u8,
                ((packed >> 8) & 0xff) as u8,
            ]);
        }
        let index = parsed.objects.len();
        for k in 0..parts.solids.len() {
            parsed.at.push((index, Part::Solid(k)));
            parsed.items.push(Item {
                object: o.name.clone(),
                label: o.label.clone(),
                name: if k == 0 {
                    o.label.clone()
                } else {
                    format!("{}-{}", o.label, k + 1)
                },
                kind: ItemKind::Solid,
                color,
            });
        }
        if !parts.sheet.is_empty() {
            parsed.at.push((index, Part::Sheet));
            parsed.items.push(Item {
                object: o.name.clone(),
                label: o.label.clone(),
                name: if parts.solids.is_empty() {
                    o.label.clone()
                } else {
                    format!("{}-sheet", o.label)
                },
                kind: ItemKind::Sheet,
                color,
            });
        }
        parsed.objects.push(ObjectShape {
            label: o.label.clone(),
            shape,
            parts,
            numbering,
            face_colors: if colors.len() > 1 { colors } else { Vec::new() },
        });
    }
    Ok(parsed)
}

/// A plain BRep file: one object, `shape`, without colour.
fn parse_brep(bytes: &[u8]) -> Result<Parsed, ImportError> {
    let brep = craft3d_brep::read(bytes).map_err(|e| ImportError(e.to_string()))?;
    let shape = Shape::new(Arc::new(brep), craft3d_brep::IDENTITY);
    let mut walks = shape::MAX_USES;
    let parts = shape.parts(&mut walks)?;
    let numbering = shape.numbering(&mut walks)?;
    let mut parsed = Parsed {
        objects: Vec::new(),
        at: Vec::new(),
        items: Vec::new(),
        skipped: Vec::new(),
    };
    let label = "shape".to_string();
    for k in 0..parts.solids.len() {
        parsed.at.push((0, Part::Solid(k)));
        parsed.items.push(Item {
            object: label.clone(),
            label: label.clone(),
            name: if k == 0 {
                label.clone()
            } else {
                format!("{label}-{}", k + 1)
            },
            kind: ItemKind::Solid,
            color: None,
        });
    }
    if !parts.sheet.is_empty() {
        parsed.at.push((0, Part::Sheet));
        parsed.items.push(Item {
            object: label.clone(),
            label: label.clone(),
            name: format!("{label}-sheet"),
            kind: ItemKind::Sheet,
            color: None,
        });
    }
    parsed.objects.push(ObjectShape {
        label,
        shape,
        parts,
        numbering,
        face_colors: Vec::new(),
    });
    Ok(parsed)
}

fn most_common(colors: &[[u8; 3]]) -> Option<[u8; 3]> {
    let mut counts: BTreeMap<[u8; 3], usize> = BTreeMap::new();
    for c in colors {
        *counts.entry(*c).or_insert(0) += 1;
    }
    let best = counts.values().copied().max()?;
    // The first of equals in face order.
    colors
        .iter()
        .copied()
        .find(|c| counts.get(c) == Some(&best))
}

/// The documents [`parse_shared`] parsed last, by key, the most recent
/// first.
static SHARED: Mutex<Vec<(String, usize, Arc<Parsed>)>> = Mutex::new(Vec::new());

/// [`parse`], kept for the next calls with the same key (the hash of the
/// bytes): one document's import features each read one item of it, and
/// it is parsed once for all of them. A few are kept, the least recently
/// used let go first.
pub fn parse_shared(bytes: &[u8], key: &str) -> Result<Arc<Parsed>, ImportError> {
    {
        let mut held = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = held.iter().position(|(k, _, _)| k == key) {
            let entry = held.remove(at);
            let parsed = Arc::clone(&entry.2);
            held.insert(0, entry);
            return Ok(parsed);
        }
    }
    let parsed = Arc::new(parse(bytes)?);
    let mut held = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
    held.insert(0, (key.to_string(), bytes.len(), Arc::clone(&parsed)));
    let mut total = 0usize;
    let mut keep = 0;
    for (_, size, _) in held.iter() {
        total = total.saturating_add(*size);
        if keep >= 4 || (keep > 0 && total > MAX_IMPORT_BYTES) {
            break;
        }
        keep += 1;
    }
    held.truncate(keep.max(1));
    Ok(parsed)
}

impl Parsed {
    /// The solids and sheets, in the order of the document's objects.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Objects with a shape left out, and why (hidden).
    pub fn skipped(&self) -> &[String] {
        &self.skipped
    }

    /// The colour of each face of item `index`'s body (by the numbers
    /// [`Imported::face_ids`] gives: FreeCAD's face numbers), where the
    /// view gives one per face; empty otherwise.
    pub fn face_colors(&self, index: usize, face_ids: &[u64]) -> Vec<Option<[u8; 3]>> {
        let Some(object) = self.at.get(index).and_then(|(o, _)| self.objects.get(*o)) else {
            return Vec::new();
        };
        if object.face_colors.is_empty() {
            return vec![None; face_ids.len()];
        }
        face_ids
            .iter()
            .map(|&k| {
                usize::try_from(k)
                    .ok()
                    .and_then(|k| k.checked_sub(1))
                    .and_then(|k| object.face_colors.get(k).copied())
            })
            .collect()
    }

    /// Item `index` as a body in the caller's units. The numbers of its
    /// faces, edges and vertices ([`Imported::face_ids`], …) are
    /// FreeCAD's (`Face3` is 3), so messages name them as FreeCAD does.
    pub fn build(&self, index: usize, options: &Options) -> Result<Imported, ImportError> {
        let (Some(&(o, part)), Some(item)) = (self.at.get(index), self.items.get(index)) else {
            return err(format!(
                "the document has {} solids and sheets; there is no number {index}",
                self.items.len()
            ));
        };
        let Some(object) = self.objects.get(o) else {
            return err("an item without its object");
        };
        if !(options.metres_per_unit.is_finite() && options.metres_per_unit > 0.0) {
            return err("the unit to read into must be a positive length");
        }
        if craft3d_math::Tolerance::new(options.tolerance.linear, options.tolerance.angular)
            .is_none()
        {
            return err("the tolerance must be positive and finite");
        }
        if !(options.loosest.is_finite() && options.loosest >= options.tolerance.linear) {
            return err(
                "the cap on an entity's tolerance must be finite and at least the tolerance",
            );
        }
        // FreeCAD's unit is the millimetre.
        let scale = 0.001 / options.metres_per_unit;
        let merge = options.tolerance.linear;
        let mut report = Report {
            length_unit: "millimetre".to_string(),
            metres_per_file_unit: 0.001,
            scale,
            ..Report::default()
        };
        let named = |e: ImportError| ImportError(format!("`{}`: {}", item.name, e.0));
        let mut budget = Budget { used: 0 };
        let kind = match part {
            Part::Solid(_) => BodyKind::Solid,
            Part::Sheet => BodyKind::Sheet,
        };
        let mut b = shape::Builder::new(
            &object.shape,
            &object.numbering,
            kind,
            scale,
            merge,
            &mut budget,
        );
        match part {
            Part::Solid(k) => {
                let solid: &Use = object
                    .parts
                    .solids
                    .get(k)
                    .ok_or_else(|| ImportError("a solid is missing".to_string()))?;
                for s in object.shape.children(solid).map_err(named)? {
                    let t = object.shape.brep.shape(s.number);
                    if matches!(t.map(|t| &t.kind), Some(craft3d_brep::Kind::Shell)) {
                        b.shell(&s).map_err(named)?;
                    }
                }
            }
            Part::Sheet => {
                let mut loose = Vec::new();
                for p in &object.parts.sheet {
                    match p {
                        SheetPart::Shell(s) => b.shell(s).map_err(named)?,
                        SheetPart::Face(f) => loose.push(*f),
                    }
                }
                if !loose.is_empty() {
                    let first = b.body.faces.len();
                    for f in &loose {
                        b.face(f).map_err(named)?;
                    }
                    b.close_shell(first).map_err(named)?;
                }
            }
        }
        b.compact();
        let shape::Builder {
            body,
            face_ids,
            edge_ids,
            vertex_ids,
            ..
        } = b;
        if body.shells.is_empty() {
            return err(format!("`{}`: the solid has no shells", item.name));
        }
        // The outer shell is the one that reaches farthest; the others
        // are cavities (Open CASCADE lists them in any order).
        let cavities = cavities(&body);
        let mut read = finish(
            Assembled {
                body,
                cavities,
                face_ids,
                edge_ids,
                vertex_ids,
            },
            item.name.clone(),
            merge,
            options,
            &mut report,
        )?;
        read.color = item.color;
        let _ = &object.label;
        Ok(read)
    }
}

/// Per shell whether it is a cavity: every shell but the one whose
/// vertices span the largest box.
fn cavities(body: &craft3d_topo::Body) -> Vec<bool> {
    let span = |shell: &craft3d_topo::Shell| {
        let (mut lo, mut hi) = (
            craft3d_math::Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
            craft3d_math::Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
        );
        for f in &shell.faces {
            let Some(face) = body.faces.get(f.0 as usize) else {
                continue;
            };
            for l in &face.loops {
                let Some(l) = body.loops.get(l.0 as usize) else {
                    continue;
                };
                for c in &l.coedges {
                    let Some(e) = body.edges.get(c.edge.0 as usize) else {
                        continue;
                    };
                    for v in [e.start, e.end] {
                        if let Some(v) = body.vertices.get(v.0 as usize) {
                            lo = lo.min(v.point);
                            hi = hi.max(v.point);
                        }
                    }
                }
            }
        }
        let d = hi - lo;
        if d.is_finite() { d.length() } else { 0.0 }
    };
    let spans: Vec<f64> = body.shells.iter().map(span).collect();
    let outer = spans
        .iter()
        .enumerate()
        .fold((0usize, f64::NEG_INFINITY), |best, (i, &s)| {
            if s > best.1 { (i, s) } else { best }
        })
        .0;
    (0..spans.len()).map(|i| i != outer).collect()
}

#[cfg(test)]
mod tests;
