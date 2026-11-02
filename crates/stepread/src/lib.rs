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
//!   `SPHERICAL_SURFACE`, `TOROIDAL_SURFACE` (a spindle or horn torus too)
//!   and `DEGENERATE_TOROIDAL_SURFACE`, each face on one sheet
//!   (`spindle`), `B_SPLINE_SURFACE_WITH_KNOTS`
//!   (polynomial, or rational as the complex instance with
//!   `RATIONAL_B_SPLINE_SURFACE`) and the B-splines whose knots their type
//!   implies (`UNIFORM_SURFACE`, `QUASI_UNIFORM_SURFACE`,
//!   `BEZIER_SURFACE`); the swept surfaces: `SURFACE_OF_LINEAR_EXTRUSION`
//!   of a line (a plane) or of a circle along its axis (a cylinder),
//!   `SURFACE_OF_REVOLUTION` of a line (a cylinder, a cone or a plane) or
//!   of a circle in a plane through the axis (a sphere or a ring torus),
//!   and of any other curve read as the exact B-spline surface it is
//!   (2026-10-21, slice 2); `OFFSET_SURFACE` of a plane, cylinder, cone,
//!   sphere or torus (the same kind again).
//! - **Curves**: `LINE`, `CIRCLE`, `ELLIPSE`, `POLYLINE`,
//!   `B_SPLINE_CURVE_WITH_KNOTS` (polynomial or rational), `UNIFORM_CURVE`,
//!   `QUASI_UNIFORM_CURVE`, `BEZIER_CURVE`, and `SURFACE_CURVE`,
//!   `SEAM_CURVE`, `INTERSECTION_CURVE` by their 3D curve.
//! - **Assemblies** (`assembly`): every placement of a solid by
//!   `NEXT_ASSEMBLY_USAGE_OCCURRENCE` with `CONTEXT_DEPENDENT_SHAPE_
//!   REPRESENTATION` and a representation relationship with an
//!   `ITEM_DEFINED_TRANSFORMATION` or a `CARTESIAN_TRANSFORMATION_
//!   OPERATOR_3D`, or by `MAPPED_ITEM`, composed through nested
//!   assemblies: one placed body each ([`Parsed::placed`]).
//! - **Colours** (`style`): a solid's `STYLED_ITEM` colour, else its
//!   faces' most common one ([`Imported::color`]).
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
//! instance id and keyword), placements that scale or mirror, assemblies
//! that contain themselves or are nested or multiplied past their limits,
//! a context without a length unit, references to the wrong kind of
//! entity, chains of references deeper than a few levels, non-finite or
//! absurd numbers.

mod assembly;
mod budget;
mod diagnose;
pub mod finish;
pub mod geometry;
pub mod spindle;
mod style;
pub mod swept;
mod topology;
mod units;

pub use assembly::{MAX_DEPTH, MAX_PLACEMENTS, Placed};
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
    /// Edges the file does not join that were sewn to another within the
    /// document's cap (`topo::sew`, after the file's own sharing), the
    /// vertices that went into others doing so, and the largest gap
    /// between two edges sewn (their ends and places inside).
    pub sewn_edges: usize,
    pub sewn_vertices: usize,
    pub sewn_gap: f64,
    /// Edges of the file split where shorter free edges of other faces
    /// run along them (a T-junction, `topo::sew`): each piece then sewn
    /// to one of them.
    pub split_edges: usize,
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

/// One placed item as a body.
#[derive(Clone, Debug)]
pub struct Imported {
    pub body: Body,
    pub name: String,
    /// Where an assembly places it ([`Placed::path`]): empty when none
    /// does.
    pub path: String,
    /// The colour the file gives the solid (sRGB): its own style, else
    /// the colour most of its styled faces have (a body has one colour
    /// here).
    pub color: Option<[u8; 3]>,
    pub report: Report,
    /// Per face, edge and vertex of the body (by index): the instance id
    /// of the `ADVANCED_FACE`, `EDGE_CURVE` or `VERTEX_POINT` it is.
    pub face_ids: Vec<u64>,
    pub edge_ids: Vec<u64>,
    pub vertex_ids: Vec<u64>,
}

/// The entity of an imported body with the largest tolerance of its own,
/// for reports: `"edge"` or `"vertex"`, the file's instance id, the
/// tolerance, and for a vertex whether an edge that ends at it is as
/// loose (its tolerance raised to the edge's) rather than measured from
/// its edges' ends.
#[derive(Clone, Debug, PartialEq)]
pub struct Loosest {
    pub kind: &'static str,
    pub step: u64,
    pub tolerance: f64,
    pub from_edge: bool,
    /// For a vertex: how far the ends of its edges are from it, and the
    /// largest tolerance of an edge that ends at it.
    pub ends: f64,
    pub edge: f64,
    /// For a vertex: the loosest edge that ends at it (its file id) and
    /// how far that edge lies off its faces at 1025 places.
    pub edge_step: u64,
    pub edge_gap: f64,
}

/// How far `edge` lies off the surfaces of its faces at 1025 places, each
/// foot the nearer of the projection and the foot walked from the place
/// before: for reports.
fn sampled_gap(body: &craft3d_topo::Body, edge: craft3d_topo::EdgeId) -> f64 {
    let Some(e) = body.edge(edge) else {
        return f64::NAN;
    };
    let mut worst = 0.0_f64;
    for face in &body.faces {
        let uses = face.loops.iter().any(|l| {
            body.loop_(*l)
                .is_some_and(|l| l.coedges.iter().any(|c| c.edge == edge))
        });
        if !uses {
            continue;
        }
        let mut last: Option<(f64, f64)> = None;
        for k in 0..=1024 {
            let t = e.range.0 + (e.range.1 - e.range.0) * f64::from(k) / 1024.0;
            let p = e.curve.eval(t);
            let mut best: Option<(f64, (f64, f64))> = None;
            for at in [
                last.and_then(|l| face.surface.project_from(p, l).ok()),
                face.surface.project(p).ok(),
            ]
            .into_iter()
            .flatten()
            {
                let d = face.surface.eval(at.0, at.1).distance(p);
                if d.is_finite() && best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, at));
                }
            }
            if let Some((d, at)) = best {
                worst = worst.max(d);
                last = Some(at);
            }
        }
    }
    worst
}

impl Imported {
    /// The entity with the largest tolerance of its own (an edge before a
    /// vertex as loose), `None` for an exact body.
    pub fn loosest(&self) -> Option<Loosest> {
        let id = |ids: &[u64], k: u32| ids.get(k as usize).copied().unwrap_or(0);
        let mut best: Option<Loosest> = None;
        for (e, t) in self.body.tolerances.edges() {
            if best.as_ref().is_none_or(|b| t > b.tolerance) {
                best = Some(Loosest {
                    kind: "edge",
                    step: id(&self.edge_ids, e.0),
                    tolerance: t,
                    from_edge: false,
                    ends: 0.0,
                    edge: t,
                    edge_step: id(&self.edge_ids, e.0),
                    edge_gap: sampled_gap(&self.body, e),
                });
            }
        }
        for (v, t) in self.body.tolerances.vertices() {
            if best.as_ref().is_none_or(|b| t > b.tolerance) {
                let from_edge = self.body.edges.iter().enumerate().any(|(k, e)| {
                    (e.start == v || e.end == v)
                        && u32::try_from(k).is_ok_and(|k| {
                            self.body.edge_tolerance(craft3d_topo::EdgeId(k)) == Some(t)
                        })
                });
                let point = self.body.vertex(v).map(|x| x.point);
                let (mut ends, mut edge) = (0.0_f64, 0.0_f64);
                let mut loosest_edge: Option<craft3d_topo::EdgeId> = None;
                for (k, e) in self.body.edges.iter().enumerate() {
                    for (end, at) in [(e.start, e.range.0), (e.end, e.range.1)] {
                        if end == v
                            && let Some(p) = point
                        {
                            ends = ends.max(e.curve.eval(at).distance(p));
                            let own = u32::try_from(k)
                                .ok()
                                .and_then(|k| self.body.edge_tolerance(craft3d_topo::EdgeId(k)));
                            if own.unwrap_or(0.0) > edge {
                                edge = own.unwrap_or(0.0);
                                loosest_edge = u32::try_from(k).ok().map(craft3d_topo::EdgeId);
                            }
                        }
                    }
                }
                best = Some(Loosest {
                    kind: "vertex",
                    step: id(&self.vertex_ids, v.0),
                    tolerance: t,
                    from_edge,
                    ends,
                    edge,
                    edge_step: loosest_edge.map_or(0, |e| id(&self.edge_ids, e.0)),
                    edge_gap: loosest_edge.map_or(f64::NAN, |e| sampled_gap(&self.body, e)),
                });
            }
        }
        best
    }
}

/// The work one placed solid or sheet may take to read
/// (`craft3d_geom::allowance` units: about one per freeform evaluation,
/// one to three million a second): past it the body is refused, naming
/// the stage and the entity where the work ran out (2026-11-02: ABC
/// models of tens of thousands of edges took minutes to prove).
pub const IMPORT_WORK: u64 = 40_000_000;

/// A parsed file with its items found.
#[derive(Debug)]
pub struct Parsed {
    file: StepFile,
    /// Where each instance id is in `file.data`.
    positions: BTreeMap<u64, usize>,
    items: Vec<Item>,
    /// Every placement of an item (`assembly`): the items' first
    /// placements in item order, then the further ones.
    placed: Vec<Placed>,
    /// The representation context each item is in.
    contexts: BTreeMap<u64, u64>,
    /// The colour of each styled item (`style`), by instance id.
    colors: BTreeMap<u64, [u8; 3]>,
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
        placed: Vec::new(),
        contexts: BTreeMap::new(),
        colors: BTreeMap::new(),
    };
    parsed.find_items()?;
    parsed.placed = parsed.find_placements()?;
    budget::check(&parsed)?;
    parsed.colors = style::colors(&parsed);
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
    /// The colour each of `faces` (instance ids of `ADVANCED_FACE`s, as
    /// [`Read::face_ids`] lists a body's) is styled with in the file
    /// itself, in their order: `None` for a face without a style of its
    /// own (it has its solid's).
    pub fn face_colors(&self, faces: &[u64]) -> Vec<Option<[u8; 3]>> {
        faces
            .iter()
            .map(|face| self.colors.get(face).copied())
            .collect()
    }

    /// The solids and sheets, in file order.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Every placed solid or sheet: each item once where no assembly
    /// places it, else once per placement. The first `items().len()` are
    /// the items in their order at their first placement (so a file
    /// without assemblies has its items here, in the same positions),
    /// then the further placements in file order. [`Parsed::build`] takes
    /// an index into this.
    pub fn placed(&self) -> &[Placed] {
        &self.placed
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

    /// Placed item `index` (of [`Parsed::placed`]) as a body in the
    /// caller's units, where its assembly places it.
    pub fn build(&self, index: usize, options: &Options) -> Result<Imported, ImportError> {
        self.build_within(index, options, IMPORT_WORK)
    }

    /// [`Parsed::build`] with `work` in place of [`IMPORT_WORK`].
    pub fn build_within(
        &self,
        index: usize,
        options: &Options,
        work: u64,
    ) -> Result<Imported, ImportError> {
        let placed = self.placed.get(index).ok_or_else(|| {
            ImportError(format!(
                "the file has {} placed solids and sheets; there is no number {index}",
                self.placed.len()
            ))
        })?;
        let item = self.items.get(placed.item).ok_or_else(|| {
            ImportError(format!(
                "the file has {} solids and sheets; there is no number {}",
                self.items.len(),
                placed.item
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
        // Bounded work (`craft3d_geom::allowance`, counted, so the same
        // file gives the same answer on every machine): a body whose
        // proofs would take minutes is refused naming where the work went.
        let mut read = craft3d_geom::allowance::within(work, || {
            topology::build(&reader, item, options, &mut report)
        })?;
        read.color = style::color_of(self, item.id, &read.face_ids);
        if !placed.steps.is_empty() {
            let transform = self.placement_transform(placed, options)?;
            // A placement where the solid already is (each step's frames
            // the same within 1e-12 in the file's units, as slice 1
            // accepted it) leaves the body as read, bit for bit: a
            // document saved before assemblies were read regenerates the
            // same bodies.
            if !self.placement_is_identity(placed)? {
                read.body = read.body.transformed(&transform).map_err(|e| {
                    ImportError(format!("`{}` placed at {}: {e}", read.name, placed.path))
                })?;
            }
            read.path = placed.path.clone();
        }
        Ok(read)
    }

    /// The one context with units in the file, when there is exactly one.
    pub(crate) fn only_context(&self) -> Option<u64> {
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
mod spindle_tests;
#[cfg(test)]
mod tests;
