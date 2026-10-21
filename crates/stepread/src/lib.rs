//! Typed reading of STEP files (ISO 10303-21 exchange structure, the
//! B-rep entities of ISO 10303-42 as AP203, AP214 and AP242 use them) into
//! the bodies of `craft3d-topo`.
//!
//! `craft3d-step` knows the exchange structure: records and references.
//! This crate knows which entities make a solid and what their attributes
//! mean, read the same way `craft3d-io`'s export writes them (its module
//! documentation lists every convention and how sure it is). The file is
//! hostile input: every reference, count and number is checked, nothing
//! recurses without a bound, and anything this crate does not read is an
//! error that names the entity (`#12 HYPERBOLA`), never a panic and never
//! a body that silently lacks a face.
//!
//! # What is read
//!
//! - **Solids and sheets**, each one item, in the order of their instances
//!   in the file: `MANIFOLD_SOLID_BREP`, `BREP_WITH_VOIDS` (its voids,
//!   `ORIENTED_CLOSED_SHELL`s, as cavities) and `SHELL_BASED_SURFACE_MODEL`
//!   (its `OPEN_SHELL`s and `CLOSED_SHELL`s as one sheet body).
//! - **Topology**: `CLOSED_SHELL`, `OPEN_SHELL`, `ORIENTED_CLOSED_SHELL`,
//!   `ORIENTED_OPEN_SHELL`, `ADVANCED_FACE` (and its supertype
//!   `FACE_SURFACE`), `ORIENTED_FACE`, `FACE_OUTER_BOUND`, `FACE_BOUND`,
//!   `EDGE_LOOP`, `VERTEX_LOOP` (a whole sphere or torus, a cone's apex:
//!   dropped, as faces here need no such loop), `ORIENTED_EDGE`,
//!   `EDGE_CURVE`, `VERTEX_POINT`.
//! - **Surfaces**: `PLANE`, `CYLINDRICAL_SURFACE`, `CONICAL_SURFACE`,
//!   `SPHERICAL_SURFACE`, `TOROIDAL_SURFACE`, `B_SPLINE_SURFACE_WITH_KNOTS`
//!   (polynomial, or rational as the complex instance with
//!   `RATIONAL_B_SPLINE_SURFACE`), and the swept surfaces that are one of
//!   those kinds: `SURFACE_OF_LINEAR_EXTRUSION` of a line (a plane) or of a
//!   circle along its axis (a cylinder), `SURFACE_OF_REVOLUTION` of a line
//!   (a cylinder, a cone or a plane) or of a circle in a plane through the
//!   axis (a sphere or a ring torus).
//! - **Curves**: `LINE`, `CIRCLE`, `ELLIPSE`, `POLYLINE`,
//!   `B_SPLINE_CURVE_WITH_KNOTS` (polynomial or rational), and
//!   `SURFACE_CURVE`, `SEAM_CURVE`, `INTERSECTION_CURVE` by their 3D curve.
//! - **Placements**: `AXIS2_PLACEMENT_3D`, `AXIS1_PLACEMENT`, `DIRECTION`,
//!   `VECTOR`, `CARTESIAN_POINT`.
//! - **Units** of the representation context the item is in
//!   (`GLOBAL_UNIT_ASSIGNED_CONTEXT`): a `LENGTH_UNIT` that is an `SI_UNIT`
//!   with or without a prefix, or a `CONVERSION_BASED_UNIT` of one (inch,
//!   foot); a `PLANE_ANGLE_UNIT` likewise (radian, degree). Lengths are
//!   converted to the caller's unit by multiplying, which is exact up to
//!   rounding. The context's `UNCERTAINTY_MEASURE_WITH_UNIT` is read as
//!   information and as the distance within which two vertices of the file
//!   are one.
//!
//! # Sewing (topology only)
//!
//! An `EDGE_CURVE` or a `VERTEX_POINT` the file shares between faces is one
//! edge or vertex. Where the file repeats a vertex (two `VERTEX_POINT`s
//! within its uncertainty, or within the caller's tolerance when it states
//! none) they become one vertex, at the place of the first; where two
//! edges used once each join the same two vertices and their midpoints lie
//! within that distance, they become one edge, the first. Geometry is never
//! moved (docs/decisions.md, 2026-10-18): the largest distance merged is
//! measured and reported, and the body is checked against the caller's
//! tolerance. A body whose edges or vertices lie off their faces or curves
//! by more than that is refused, naming the worst gap, until per-entity
//! tolerances exist.
//!
//! # Refused
//!
//! Every entity kind not listed above where a solid needs it (with its
//! instance id and keyword), assemblies that place parts by a
//! transformation that is not the identity (`MAPPED_ITEM`,
//! `REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION`), a context without
//! a length unit, references to the wrong kind of entity, chains of
//! references deeper than a few levels, non-finite or absurd numbers.

mod budget;
mod geometry;
mod topology;
mod units;

pub use budget::MAX_CONTROL_POINT_USES;

use craft3d_math::Tolerance;
use craft3d_step::{Entity, Instance, Param, ReadLimits, Record, StepFile};
use craft3d_topo::Body;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

/// Largest STEP file that is read: 128 MiB. A document holds the file it
/// imported, and a `.3dcraft` file is at most 256 MiB.
pub const MAX_IMPORT_BYTES: usize = 128 * 1024 * 1024;

/// Why a file, or one solid of it, could not be read. The message names
/// the entity (`#12 HYPERBOLA`) wherever there is one.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportError(pub String);

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ImportError {}

pub(crate) fn err<T>(message: impl Into<String>) -> Result<T, ImportError> {
    Err(ImportError(message.into()))
}

/// What an item of the file becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Solid,
    Sheet,
}

/// One solid or sheet of the file.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The instance id of its `MANIFOLD_SOLID_BREP`, `BREP_WITH_VOIDS` or
    /// `SHELL_BASED_SURFACE_MODEL`.
    pub id: u64,
    pub kind: ItemKind,
    /// The name the file gives it (the item's own, else that of the
    /// product whose shape it is), possibly empty.
    pub name: String,
}

/// What the caller wants the bodies in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// How many metres one unit of the bodies is (0.001 for millimetres).
    pub metres_per_unit: f64,
    /// The modelling tolerance, in that unit: the bodies are checked with
    /// it.
    pub tolerance: Tolerance,
    /// The cap on any entity's own tolerance, in that unit (the
    /// document's `loosest`): an edge or vertex of the file that lies off
    /// its faces or edges by more than the tolerance gets what was proven
    /// as its own tolerance, up to this; beyond it the body is refused.
    pub loosest: f64,
}

/// How one item was read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// The file's length unit, as the file names it (`MILLI METRE`,
    /// `INCH`).
    pub length_unit: String,
    /// Metres per unit of the file.
    pub metres_per_file_unit: f64,
    /// What a file length was multiplied by.
    pub scale: f64,
    /// The file's uncertainty, in the caller's unit, if it states one.
    pub uncertainty: Option<f64>,
    /// Vertices of the file that became one with an earlier one, and the
    /// largest distance between two that did.
    pub merged_vertices: usize,
    pub vertex_gap: f64,
    /// Edges of the file that became one with an earlier one, and the
    /// largest distance between the midpoints of two that did.
    pub merged_edges: usize,
    pub edge_gap: f64,
    /// Shells whose faces were turned over because the volume they
    /// enclose had the wrong sign (a cavity given as a plain shell).
    pub turned_shells: usize,
    /// Edges and vertices that lie off their faces or edges by more than
    /// the tolerance, and so have a tolerance of their own, and the
    /// largest such tolerance (0 when there is none).
    pub loose_edges: usize,
    /// Edges no proof within the cap was found for, accepted because 1025
    /// places along them all lie within the tolerance of their faces (an
    /// arc through a sphere's pole, where the prover gives out).
    pub unproven_edges: usize,
    pub loose_vertices: usize,
    pub largest_tolerance: f64,
}

/// One item as a body.
#[derive(Clone, Debug)]
pub struct Imported {
    pub body: Body,
    pub name: String,
    pub report: Report,
    /// Per face, edge and vertex of the body (by index): the instance id
    /// of the `ADVANCED_FACE`, `EDGE_CURVE` or `VERTEX_POINT` it is.
    pub face_ids: Vec<u64>,
    pub edge_ids: Vec<u64>,
    pub vertex_ids: Vec<u64>,
}

/// A parsed file with its items found.
#[derive(Debug)]
pub struct Parsed {
    file: StepFile,
    /// Where each instance id is in `file.data`.
    positions: BTreeMap<u64, usize>,
    items: Vec<Item>,
    /// The representation context each item is in.
    contexts: BTreeMap<u64, u64>,
}

/// Reads a file and finds its solids and sheets.
pub fn parse(bytes: &[u8]) -> Result<Parsed, ImportError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return err(format!(
            "the file is {} bytes; at most {MAX_IMPORT_BYTES} are read",
            bytes.len()
        ));
    }
    let limits = ReadLimits {
        max_bytes: MAX_IMPORT_BYTES,
        ..ReadLimits::default()
    };
    let file = craft3d_step::read_with(bytes, &limits)
        .map_err(|e| ImportError(format!("not a readable STEP file: {e}")))?;
    let mut positions = BTreeMap::new();
    for (at, instance) in file.data.iter().enumerate() {
        positions.insert(instance.id, at);
    }
    let mut parsed = Parsed {
        file,
        positions,
        items: Vec::new(),
        contexts: BTreeMap::new(),
    };
    parsed.find_items()?;
    budget::check(&parsed)?;
    Ok(parsed)
}

/// The files [`parse_shared`] parsed last, by key, the most recent first.
static SHARED: Mutex<Vec<(String, usize, Arc<Parsed>)>> = Mutex::new(Vec::new());

/// How many parsed files [`parse_shared`] keeps, and how many bytes of
/// files at most (a parsed file takes several times its bytes).
const SHARED_FILES: usize = 4;
const SHARED_BYTES: usize = MAX_IMPORT_BYTES;

/// [`parse`], kept for the next calls with the same key (the hash of the
/// bytes): one document's import features each read one solid of the same
/// file, and the file is parsed once for all of them. A few files are
/// kept, the least recently used let go first, within [`SHARED_BYTES`].
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
        if keep >= SHARED_FILES || (keep > 0 && total > SHARED_BYTES) {
            break;
        }
        keep += 1;
    }
    held.truncate(keep.max(1));
    Ok(parsed)
}

/// Lets go of the files [`parse_shared`] keeps.
pub fn forget_shared() {
    let mut held = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
    held.clear();
}

/// The keywords of an instance, for messages: `LINE`, or
/// `(B_SPLINE_CURVE B_SPLINE_CURVE_WITH_KNOTS …)` for a complex one.
pub(crate) fn keywords(entity: &Entity) -> String {
    match entity {
        Entity::Simple(record) => record.keyword.clone(),
        Entity::Complex(records) => {
            let names: Vec<&str> = records.iter().map(|r| r.keyword.as_str()).collect();
            format!("({})", names.join(" "))
        }
    }
}

/// A number: a real, an integer written without a point (lenient), or a
/// typed measure (`LENGTH_MEASURE(2.5)`).
pub(crate) fn real(param: &Param) -> Option<f64> {
    match param {
        Param::Real(x) => Some(*x),
        Param::Integer(i) => Some(*i as f64),
        Param::Typed(_, inner) => match inner.as_ref() {
            Param::Real(x) => Some(*x),
            Param::Integer(i) => Some(*i as f64),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn boolean(param: &Param) -> Option<bool> {
    match param.as_enum()? {
        "T" => Some(true),
        "F" => Some(false),
        _ => None,
    }
}

impl Parsed {
    /// The solids and sheets, in file order.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn header(&self) -> Option<craft3d_step::Header> {
        self.file.header_values()
    }

    pub(crate) fn instance(&self, id: u64) -> Result<&Instance, ImportError> {
        self.positions
            .get(&id)
            .and_then(|&at| self.file.data.get(at))
            .ok_or_else(|| ImportError(format!("#{id} does not exist")))
    }

    /// The record `keyword` of instance `id`; an error naming what the
    /// instance is when it has none.
    pub(crate) fn record(&self, id: u64, keyword: &str) -> Result<&Record, ImportError> {
        let instance = self.instance(id)?;
        instance.entity.record(keyword).ok_or_else(|| {
            ImportError(format!(
                "#{id} is a {}, where a {keyword} is needed",
                keywords(&instance.entity)
            ))
        })
    }

    pub(crate) fn describe(&self, id: u64) -> String {
        match self.instance(id) {
            Ok(instance) => format!("#{id} {}", keywords(&instance.entity)),
            Err(_) => format!("#{id}"),
        }
    }

    fn find_items(&mut self) -> Result<(), ImportError> {
        let mut items = Vec::new();
        let mut contexts = BTreeMap::new();
        let mut product_names: BTreeMap<u64, String> = BTreeMap::new();
        for instance in &self.file.data {
            let entity = &instance.entity;
            if entity.is("MAPPED_ITEM") {
                return err(format!(
                    "#{} MAPPED_ITEM: the file places parts in an assembly, which is not read yet",
                    instance.id
                ));
            }
            if entity.is("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION") {
                self.identity_transformation(instance)?;
            }
            let kind = if entity.is("MANIFOLD_SOLID_BREP") || entity.is("BREP_WITH_VOIDS") {
                Some(ItemKind::Solid)
            } else if entity.is("SHELL_BASED_SURFACE_MODEL") {
                Some(ItemKind::Sheet)
            } else {
                None
            };
            if let Some(kind) = kind {
                let name = entity
                    .records()
                    .iter()
                    .find_map(|r| r.params.first().and_then(Param::as_str))
                    .unwrap_or("")
                    .to_string();
                items.push(Item {
                    id: instance.id,
                    kind,
                    name,
                });
            }
            for record in entity.records() {
                if !record.keyword.ends_with("REPRESENTATION") {
                    continue;
                }
                let (Some(list), Some(context)) = (
                    record.params.get(1).and_then(Param::as_list),
                    record.params.get(2).and_then(Param::as_ref),
                ) else {
                    continue;
                };
                for item in list.iter().filter_map(Param::as_ref) {
                    contexts.entry(item).or_insert(context);
                }
                // The product a shape representation belongs to names its
                // items when they have no name of their own.
                if let Some(name) = record.params.first().and_then(Param::as_str)
                    && !name.is_empty()
                {
                    for item in list.iter().filter_map(Param::as_ref) {
                        product_names
                            .entry(item)
                            .or_insert_with(|| name.to_string());
                    }
                }
            }
        }
        for item in &mut items {
            if item.name.is_empty()
                && let Some(name) = product_names.get(&item.id)
            {
                item.name = name.clone();
            }
        }
        self.items = items;
        self.contexts = contexts;
        Ok(())
    }

    /// A placement of a part by a transformation: read only when it is
    /// the identity (both placements the same frame), else refused.
    fn identity_transformation(&self, instance: &Instance) -> Result<(), ImportError> {
        let id = instance.id;
        let refused = || {
            ImportError(format!(
                "#{id} REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION: the file places parts in an assembly, which is not read yet"
            ))
        };
        let record = instance
            .entity
            .record("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION")
            .ok_or_else(refused)?;
        let transformation = record
            .params
            .first()
            .and_then(Param::as_ref)
            .ok_or_else(refused)?;
        let transformation = self
            .record(transformation, "ITEM_DEFINED_TRANSFORMATION")
            .map_err(|_| refused())?;
        let (Some(a), Some(b)) = (
            transformation.params.get(2).and_then(Param::as_ref),
            transformation.params.get(3).and_then(Param::as_ref),
        ) else {
            return Err(refused());
        };
        let one = geometry::Reader::new(self, 1.0, 1.0);
        let (fa, fb) = (one.placement(a)?, one.placement(b)?);
        let same = |p: craft3d_math::Vec3, q: craft3d_math::Vec3| (p - q).length() <= 1e-12;
        if same(fa.origin(), fb.origin()) && same(fa.x(), fb.x()) && same(fa.z(), fb.z()) {
            Ok(())
        } else {
            Err(refused())
        }
    }

    /// Item `index` (of [`Parsed::items`]) as a body in the caller's units.
    pub fn build(&self, index: usize, options: &Options) -> Result<Imported, ImportError> {
        let item = self.items.get(index).ok_or_else(|| {
            ImportError(format!(
                "the file has {} solids and sheets; there is no number {index}",
                self.items.len()
            ))
        })?;
        if !(options.metres_per_unit.is_finite() && options.metres_per_unit > 0.0) {
            return err("the unit to read into must be a positive length");
        }
        if Tolerance::new(options.tolerance.linear, options.tolerance.angular).is_none() {
            return err("the tolerance must be positive and finite");
        }
        if !(options.loosest.is_finite() && options.loosest >= options.tolerance.linear) {
            return err(
                "the cap on an entity's tolerance must be finite and at least the tolerance",
            );
        }
        let context = match self.contexts.get(&item.id) {
            Some(&context) => context,
            None => self.only_context().ok_or_else(|| {
                ImportError(format!(
                    "{} is in no representation, so its units are unknown",
                    self.describe(item.id)
                ))
            })?,
        };
        let units = units::read(self, context)?;
        let scale = units.metres_per_length / options.metres_per_unit;
        if !(scale.is_finite() && scale > 0.0) {
            return err(format!(
                "a length of the file ({} m) cannot be converted to the document's unit",
                units.metres_per_length
            ));
        }
        let mut report = Report {
            length_unit: units.length_name.clone(),
            metres_per_file_unit: units.metres_per_length,
            scale,
            uncertainty: units
                .uncertainty_metres
                .map(|m| m / options.metres_per_unit),
            ..Report::default()
        };
        let reader = geometry::Reader::new(self, scale, units.radians_per_angle);
        topology::build(&reader, item, options, &mut report)
    }

    /// The one context with units in the file, when there is exactly one.
    fn only_context(&self) -> Option<u64> {
        let mut found = self
            .file
            .data
            .iter()
            .filter(|i| i.entity.is("GLOBAL_UNIT_ASSIGNED_CONTEXT"))
            .map(|i| i.id);
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }
}

#[cfg(test)]
mod tests;
