//! Regeneration: evaluates a document's variables and feature history into
//! bodies. The document is the recipe; what this returns is derived and can
//! be recomputed at any time (AGENTS.md rule 2).
//!
//! A failure is data, not an exception: a variable or feature that cannot
//! be evaluated gets a status that says why, and everything that does not
//! depend on it is still built.
//!
//! Sketches are solved here too ([`sketch`]): a sketch feature has no body,
//! its result is the solved sketch placed in space. Reference geometry
//! ([`datum`]) has no body either: its result is where the datum lies.
//!
//! A feature can make its body out of the body of an earlier one
//! ([`derive`]: transform, mirror, pattern). The earlier feature's result
//! stays, marked as taken: its body is no longer one of the document's.
//!
//! Regeneration with a [`RegenCache`] rebuilds only the features an edit
//! touches, and gives exactly what a full regeneration gives ([`cache`]).

mod blend;
mod boolean;
mod cache;
mod cause;
mod chamfer;
mod datum;
mod delete_face;
mod derive;
mod entity;
mod extrude;
mod face_ops;
mod fillet;
mod follow;
mod loft;
mod offset_face;
pub mod progress;
mod query;
mod revolve;
mod selfcheck;
mod shell;
mod sketch;
mod sweep;
mod thread;

pub use cache::RegenCache;
pub use cause::root_cause;
pub use craft3d_kernel::extrude::{Origin, Origins, Role};
pub use craft3d_kernel::helix::Helix;
pub use craft3d_kernel::thread::{coarse_pitch, nominal_diameter};
pub use datum::{Datum, EdgeReference, References, references};
pub use derive::{MAX_PATTERN_FACES, MAX_PATTERN_INSTANCES};
pub use extrude::name_of;
pub use follow::{newest_body, short_face_name};
pub use query::{edges_between, faces_named, faces_of_edges};
pub use sketch::{
    ConstraintResult, ConstraintState, SketchResult, settle_sketches, settle_sketches_cached,
};

use craft3d_doc::{
    Document, Expr, Feature, FeatureId, FeatureKind, MAX_NAME_DEPTH, NameRole, Orientation,
    PersistentName,
};
use craft3d_expr::{Dimension, Evaluation, Quantity};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::Body;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The value of a variable, in the units the document shows.
#[derive(Clone, Debug, PartialEq)]
pub struct VariableResult {
    pub name: String,
    /// The expression as written.
    pub expr: String,
    pub value: Result<Value, String>,
}

/// A number with its dimension. Lengths are in the document's length unit
/// (areas in its square, and so on); angles in radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Value {
    pub value: f64,
    pub dimension: Dimension,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Ok,
    /// Switched off by the user.
    Suppressed,
    /// After the rollback marker.
    RolledBack,
    /// Could not be built; the text names the parameter and the reason.
    Failed(String),
}

/// What every entity of a body is called: `faces[i]` is the name of face
/// `FaceId(i)`, and so on. A name belongs to one entity, except that the
/// pieces a boolean cut a face into share the face's name: every edge has
/// a name no other edge of the body has, and every vertex one no other
/// vertex has.
///
/// The edges and vertices of a body a boolean made, or a pattern that
/// joined its instances, are named by the faces they lie on
/// (`F7:between[a,b]`, `F7:corner[a,b,c]`); one that an operand already
/// had also answers to the name it had there, as an *alias*
/// (docs/architecture.md §4.1).
///
/// A face that a boolean merged from faces of both operands (two flush
/// blocks united: their tops become one face) is named after the first
/// contributor in the kernel's order and carries the names of the others
/// as *aliases*: a reference by any of them resolves to it
/// (docs/architecture.md §4.1).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Names {
    pub faces: Vec<PersistentName>,
    /// Per face that has them (by its index in `faces`), the other names
    /// it answers to, in the kernel's order. Empty for a body no boolean
    /// merged faces in.
    pub aliases: BTreeMap<usize, Vec<PersistentName>>,
    pub edges: Vec<PersistentName>,
    pub vertices: Vec<PersistentName>,
    /// Per edge that has them (by its index in `edges`), the other names
    /// it answers to: the names it had in the bodies it came from. No two
    /// edges answer to one name.
    pub edge_aliases: BTreeMap<usize, Vec<PersistentName>>,
    /// The same per vertex, by its index in `vertices`.
    pub vertex_aliases: BTreeMap<usize, Vec<PersistentName>>,
}

/// For every edge or vertex of a body the kernel put together: the
/// instance and the entity of that instance's source it is, when the
/// kernel says it is one.
pub(crate) type Kept = Vec<Option<(u64, usize)>>;

/// Per face of a body made from the bodies of other features, the
/// (instance, face of the source) pairs it has a piece of: one for a face
/// that is a piece of one face, more for a face the kernel merged from
/// several, the one it is named after first.
pub(crate) type Contributors = Vec<Vec<(u64, usize)>>;

impl Names {
    /// The other names face `face` answers to besides `faces[face]`.
    pub fn aliases_of(&self, face: usize) -> &[PersistentName] {
        self.aliases.get(&face).map_or(&[], Vec::as_slice)
    }

    /// Every name face `face` answers to: its own first, then its aliases.
    pub fn names_of(&self, face: usize) -> impl Iterator<Item = &PersistentName> {
        self.faces
            .get(face)
            .into_iter()
            .chain(self.aliases_of(face))
    }

    /// Every name any face answers to, face by face.
    pub fn all_face_names(&self) -> impl Iterator<Item = &PersistentName> {
        (0..self.faces.len()).flat_map(|face| self.names_of(face))
    }

    /// True when face `face` is called `name`, by its own name or an alias.
    pub fn face_answers_to(&self, face: usize, name: &PersistentName) -> bool {
        self.names_of(face).any(|n| n == name)
    }

    /// The other names edge `edge` answers to besides `edges[edge]`.
    pub fn edge_aliases_of(&self, edge: usize) -> &[PersistentName] {
        self.edge_aliases.get(&edge).map_or(&[], Vec::as_slice)
    }

    /// The other names vertex `vertex` answers to besides
    /// `vertices[vertex]`.
    pub fn vertex_aliases_of(&self, vertex: usize) -> &[PersistentName] {
        self.vertex_aliases.get(&vertex).map_or(&[], Vec::as_slice)
    }

    /// Every name edge `edge` answers to: its own first.
    pub fn edge_names_of(&self, edge: usize) -> impl Iterator<Item = &PersistentName> {
        self.edges
            .get(edge)
            .into_iter()
            .chain(self.edge_aliases_of(edge))
    }

    /// Every name vertex `vertex` answers to: its own first.
    pub fn vertex_names_of(&self, vertex: usize) -> impl Iterator<Item = &PersistentName> {
        self.vertices
            .get(vertex)
            .into_iter()
            .chain(self.vertex_aliases_of(vertex))
    }

    /// The edge called `name`, by its own name or an alias: its index.
    pub fn edge_named(&self, name: &PersistentName) -> Option<usize> {
        self.edges.iter().position(|own| own == name).or_else(|| {
            self.edge_aliases
                .iter()
                .find(|(_, aliases)| aliases.contains(name))
                .map(|(index, _)| *index)
        })
    }

    /// The vertex called `name`, by its own name or an alias: its index.
    pub fn vertex_named(&self, name: &PersistentName) -> Option<usize> {
        self.vertices
            .iter()
            .position(|own| own == name)
            .or_else(|| {
                self.vertex_aliases
                    .iter()
                    .find(|(_, aliases)| aliases.contains(name))
                    .map(|(index, _)| *index)
            })
    }

    /// Names the edges and vertices of `body`, whose faces these names
    /// are of, by the faces they lie on, and gives each the names it kept
    /// from the body it came from: `source(instance)` is what the
    /// entities of that instance's source are called.
    pub(crate) fn name_entities<'a>(
        &mut self,
        feature: FeatureId,
        body: &Body,
        tolerance: f64,
        kept_edges: &Kept,
        kept_vertices: &Kept,
        source: impl Fn(u64) -> Option<&'a Names>,
    ) {
        let made = entity::by_faces(feature, body, &self.faces, tolerance);
        self.edges = made.edges;
        self.vertices = made.vertices;
        self.edge_aliases = entity::kept_names(feature, kept_edges, |instance, edge| {
            source(instance)
                .into_iter()
                .flat_map(move |names| names.edge_names_of(edge))
        });
        self.vertex_aliases = entity::kept_names(feature, kept_vertices, |instance, vertex| {
            source(instance)
                .into_iter()
                .flat_map(move |names| names.vertex_names_of(vertex))
        });
    }

    /// The names of faces that are copies of the faces of an earlier body:
    /// `contributors[i]` lists, for face `i`, the (instance, face of the
    /// source) pairs it has a piece of, the one it is named after first.
    /// `source(instance)` gives what the entities of that instance's
    /// source are called. Edges and vertices are named after
    /// the faces, by [`Names::name_entities`].
    pub(crate) fn copied_faces<'a>(
        feature: FeatureId,
        contributors: &Contributors,
        source: impl Fn(u64) -> Option<&'a Names>,
    ) -> Result<Option<Self>, String> {
        let mut names = Names {
            faces: Vec::with_capacity(contributors.len()),
            ..Names::default()
        };
        for (index, sources) in contributors.iter().enumerate() {
            let mut all = Vec::with_capacity(sources.len());
            // What the contributors answered to besides their own names
            // (a face an earlier boolean merged): the face answers to
            // those too, after the names of the contributors themselves.
            let mut inherited = Vec::new();
            for &(instance, face) in sources {
                // A face is named only when the body it came from names
                // its faces.
                let Some(there) = source(instance) else {
                    return Ok(None);
                };
                let Some(name) = there.faces.get(face) else {
                    return Ok(None);
                };
                if name.depth() >= MAX_NAME_DEPTH {
                    return Err(format!(
                        "more than {MAX_NAME_DEPTH} features in a row each make their body from the one before"
                    ));
                }
                let copy = PersistentName::copy(feature, instance, name.clone());
                if !all.contains(&copy) {
                    all.push(copy);
                }
                for alias in there.aliases_of(face) {
                    if alias.depth() < MAX_NAME_DEPTH {
                        inherited.push(PersistentName::copy(feature, instance, alias.clone()));
                    }
                }
            }
            for alias in inherited {
                if !all.contains(&alias) {
                    all.push(alias);
                }
            }
            let mut rest = all.into_iter();
            let Some(own) = rest.next() else {
                return Err("the kernel gave a face without a source".to_string());
            };
            names.faces.push(own);
            let aliases: Vec<PersistentName> = rest.collect();
            if !aliases.is_empty() {
                names.aliases.insert(index, aliases);
            }
        }
        Ok(Some(names))
    }

    /// The names of the entities of a primitive: each by its number. A
    /// primitive is built in a fixed order (`craft3d_kernel::primitives`),
    /// so for it the number is where the entity came from.
    pub fn numbered(feature: FeatureId, body: &Body) -> Self {
        let named = |role, count: usize| {
            (0..count as u64)
                .map(|i| PersistentName::new(feature, role, vec![i]))
                .collect()
        };
        Self {
            faces: named(NameRole::Face, body.faces.len()),
            aliases: BTreeMap::new(),
            edges: named(NameRole::Edge, body.edges.len()),
            vertices: named(NameRole::Vertex, body.vertices.len()),
            ..Self::default()
        }
    }

    /// The names an extrusion (or another operation that reports where
    /// its entities came from) gives the entities of feature `feature`.
    pub fn of_origins(feature: FeatureId, origins: &Origins) -> Self {
        let named = |list: &[Origin]| list.iter().map(|o| name_of(feature, o)).collect();
        Self {
            faces: named(&origins.faces),
            aliases: BTreeMap::new(),
            edges: named(&origins.edges),
            vertices: named(&origins.vertices),
            ..Self::default()
        }
    }
}

/// What a sweep says of itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SweepReport {
    /// The path closes on itself: the body is a ring without ends.
    pub closed: bool,
    /// The length of the path, in model units.
    pub length: f64,
    /// How far the faces along splines of the path were found to lie
    /// from the true sweep, at most; 0 along lines and arcs, where the
    /// faces are exact. Never more than half the linear tolerance.
    pub deviation: f64,
    /// The most stations a spline of the path was followed through.
    pub stations: usize,
}

/// What a thread says of itself: the face is marked with it.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadReport {
    /// The designation, as given: `M10`.
    pub size: String,
    /// The major and the minor diameter of the basic profile, in model
    /// units.
    pub major: f64,
    pub minor: f64,
    pub pitch: f64,
    /// How far it goes at full depth.
    pub length: f64,
    pub left_handed: bool,
    /// Only marked on the face; the body is not changed.
    pub cosmetic: bool,
    /// How far the faces of the groove were found to lie from the true
    /// sweep, at most; 0 for a cosmetic thread.
    pub deviation: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureResult {
    pub id: FeatureId,
    pub status: Status,
    /// The body this feature made, when it is `Ok`.
    pub body: Option<Arc<Body>>,
    /// For a sketch: how it solved. Present for a failed sketch too, unless
    /// its plane could not be evaluated.
    pub sketch: Option<Arc<SketchResult>>,
    /// For reference geometry that was placed: where it lies.
    pub datum: Option<Datum>,
    /// What each face, edge and vertex of `body` is called by the features
    /// that come after, for features that name what they make (not the
    /// primitives yet).
    pub names: Option<Arc<Names>>,
    /// For a feature made from another feature's body: the maps from that
    /// body onto each instance of this one (one map, unless a pattern).
    pub instances: Option<Arc<Vec<Transform>>>,
    /// For a sweep that was built: how its path was followed.
    pub sweep: Option<SweepReport>,
    /// For a thread that was built: what it is.
    pub thread: Option<ThreadReport>,
    /// The later feature this feature's body went into. The body is then
    /// no longer one of the document's bodies; it stays here because names
    /// are resolved against it.
    pub consumed_by: Option<FeatureId>,
    /// For a feature whose faces or edges were given by the names of an
    /// earlier body that had gone into later features: the feature as it
    /// was built, its names followed to the newest body (`follow`). `None`
    /// when the feature was built as it stands in the document.
    pub followed: Option<Arc<Feature>>,
}

impl FeatureResult {
    /// The body, when it is this feature's still: not taken by a later
    /// feature.
    pub fn own_body(&self) -> Option<&Arc<Body>> {
        match self.consumed_by {
            None => self.body.as_ref(),
            Some(_) => None,
        }
    }
}

/// Everything derived from one document.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Regen {
    /// In evaluation order (dependencies first).
    pub variables: Vec<VariableResult>,
    /// In history order, one per feature.
    pub features: Vec<FeatureResult>,
}

impl Regen {
    /// The bodies that exist, with the feature that made each, in history
    /// order. A body that went into a later feature is not one of them.
    pub fn bodies(&self) -> impl Iterator<Item = (FeatureId, &Arc<Body>)> {
        self.features
            .iter()
            .filter_map(|f| Some((f.id, f.own_body()?)))
    }

    /// True when no variable and no feature failed.
    pub fn is_ok(&self) -> bool {
        self.variables.iter().all(|v| v.value.is_ok())
            && self
                .features
                .iter()
                .all(|f| !matches!(f.status, Status::Failed(_)))
    }

    pub fn feature(&self, id: FeatureId) -> Option<&FeatureResult> {
        self.features.iter().find(|f| f.id == id)
    }
}

/// The factor from the expression engine's base length (millimetres) to
/// the document's length unit, per power of length.
fn to_document(quantity: Quantity, doc: &Document) -> Value {
    let per_mm = Quantity::new(1.0, Dimension::LENGTH)
        .to_unit(doc.units.length.as_expr_unit())
        .unwrap_or(1.0);
    Value {
        value: quantity.value * per_mm.powi(quantity.dimension.length),
        dimension: quantity.dimension,
    }
}

/// Evaluates one length parameter, in the document's unit. A bare number
/// is taken to be in that unit, as when typing a dimension in a CAD
/// system.
fn length(doc: &Document, variables: &Evaluation, name: &str, expr: &Expr) -> Result<f64, String> {
    let quantity = craft3d_expr::evaluate(expr.source(), |variable| variables.value(variable))
        .map_err(|e| format!("{name}: `{}`: {e}", expr.source()))?;
    if quantity.dimension == Dimension::LENGTH {
        Ok(to_document(quantity, doc).value)
    } else if quantity.dimension.is_dimensionless() {
        Ok(quantity.value)
    } else {
        Err(format!(
            "{name}: `{}` is a {}, but a length is needed",
            expr.source(),
            quantity.dimension
        ))
    }
}

/// Evaluates a plain number: a direction component.
fn number(variables: &Evaluation, name: &str, expr: &Expr) -> Result<f64, String> {
    let quantity = craft3d_expr::evaluate(expr.source(), |variable| variables.value(variable))
        .map_err(|e| format!("{name}: `{}`: {e}", expr.source()))?;
    if quantity.dimension.is_dimensionless() {
        Ok(quantity.value)
    } else {
        Err(format!(
            "{name}: `{}` is a {}, but a plain number is needed",
            expr.source(),
            quantity.dimension
        ))
    }
}

/// Evaluates an angle, in radians. A bare number is taken in the
/// document's angle unit.
fn angle(doc: &Document, variables: &Evaluation, name: &str, expr: &Expr) -> Result<f64, String> {
    let quantity = craft3d_expr::evaluate(expr.source(), |variable| variables.value(variable))
        .map_err(|e| format!("{name}: `{}`: {e}", expr.source()))?;
    if quantity.dimension == Dimension::ANGLE {
        Ok(quantity.value)
    } else if quantity.dimension.is_dimensionless() {
        Ok(match doc.units.angle {
            craft3d_doc::AngleUnit::Degree => quantity.value.to_radians(),
            craft3d_doc::AngleUnit::Radian => quantity.value,
        })
    } else {
        Err(format!(
            "{name}: `{}` is a {}, but an angle is needed",
            expr.source(),
            quantity.dimension
        ))
    }
}

fn build(doc: &Document, variables: &Evaluation, feature: &Feature) -> Result<Body, String> {
    let get = |name: &str, expr: &Expr| length(doc, variables, name, expr);
    let point = |exprs: &[Expr; 3]| -> Result<Vec3, String> {
        let [x, y, z] = exprs;
        Ok(Vec3::new(
            get("position.x", x)?,
            get("position.y", y)?,
            get("position.z", z)?,
        ))
    };
    // The frame of a primitive: its z axis along `axis`, then turned by
    // `angle` about it. With the default orientation this is the world
    // frame moved to `position`.
    let placed =
        |position: &[Expr; 3], orientation: Option<&Orientation>| -> Result<Frame, String> {
            let origin = point(position)?;
            let Some(orientation) = orientation else {
                return Frame::new(origin, Vec3::Z, Vec3::X)
                    .ok_or("position is not finite".to_string());
            };
            let [ax, ay, az] = &orientation.axis;
            let axis = Vec3::new(
                number(variables, "axis.x", ax)?,
                number(variables, "axis.y", ay)?,
                number(variables, "axis.z", az)?,
            );
            let turn = angle(doc, variables, "angle", &orientation.angle)?;
            let upright = Frame::from_z(origin, axis).ok_or_else(|| {
                format!(
                    "axis must be a direction, but it is ({}, {}, {})",
                    axis.x, axis.y, axis.z
                )
            })?;
            let x = upright.x() * turn.cos() + upright.y() * turn.sin();
            Frame::new(origin, axis, x).ok_or("orientation is not finite".to_string())
        };
    // Sizes are checked here, where the parameter has the name the user
    // knows it by; the kernel would report its own argument name.
    let size = |name: &str, expr: &Expr| -> Result<f64, String> {
        let value = get(name, expr)?;
        if value > 0.0 {
            Ok(value)
        } else {
            Err(format!(
                "{name} must be a positive length, but `{}` is {value}",
                expr.source()
            ))
        }
    };
    let body = match &feature.kind {
        FeatureKind::Block(b) => {
            let [dx, dy, dz] = &b.size;
            let frame = placed(&b.position, Some(&b.orientation))?;
            primitives::block(&frame, size("size.x", dx)?, size("size.y", dy)?, size("size.z", dz)?)
        }
        FeatureKind::Cylinder(c) => {
            let frame = placed(&c.position, Some(&c.orientation))?;
            primitives::cylinder(&frame, size("radius", &c.radius)?, size("height", &c.height)?)
        }
        FeatureKind::Sphere(s) => {
            let frame = placed(&s.position, None)?;
            primitives::sphere(&frame, size("radius", &s.radius)?)
        }
        FeatureKind::Cone(c) => {
            let frame = placed(&c.position, Some(&c.orientation))?;
            primitives::cone(&frame, size("radius", &c.radius)?, size("height", &c.height)?)
        }
        FeatureKind::Torus(t) => {
            let frame = placed(&t.position, Some(&t.orientation))?;
            let (major, minor) = (size("major", &t.major)?, size("minor", &t.minor)?);
            if major <= minor {
                return Err(format!(
                    "major ({major}) must be larger than minor ({minor}): the tube would reach the axis"
                ));
            }
            primitives::torus(&frame, major, minor)
        }
        // Solved in `features`; a sketch is not a body.
        FeatureKind::Sketch(_) => return Err("a sketch has no body".to_string()),
        // Built in `features`, which knows the sketch's result.
        FeatureKind::Extrude(_) => return Err("an extrusion needs its sketch".to_string()),
        FeatureKind::Revolve(_) => return Err("a revolution needs its sketch".to_string()),
        FeatureKind::Loft(_) => return Err("a loft needs its sketches".to_string()),
        FeatureKind::Sweep(_) => return Err("a sweep needs its sketches".to_string()),
        // Placed in `features`; reference geometry is not a body.
        FeatureKind::DatumPlane(_)
        | FeatureKind::DatumAxis(_)
        | FeatureKind::DatumPoint(_)
        | FeatureKind::DatumHelix(_) => {
            return Err("a datum has no body".to_string());
        }
        // Built in `features`, which knows the earlier bodies.
        FeatureKind::Transform(_)
        | FeatureKind::Mirror(_)
        | FeatureKind::Pattern(_)
        | FeatureKind::Boolean(_)
        | FeatureKind::Chamfer(_)
        | FeatureKind::Fillet(_)
        | FeatureKind::OffsetFace(_)
        | FeatureKind::Shell(_)
        | FeatureKind::DeleteFace(_)
        | FeatureKind::Thread(_) => {
            return Err(format!(
                "a {} needs the body it is made from",
                feature.kind.type_name()
            ));
        }
    }
    .map_err(|e| e.to_string())?;

    checked(doc, body)
}

/// Never hand on an invalid body (docs/architecture.md §5.3).
fn checked(doc: &Document, body: Body) -> Result<Body, String> {
    let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    match craft3d_topo::check(&body, &tolerance).first() {
        None => Ok(body),
        Some(defect) => Err(format!("the result is not a valid body: {defect}")),
    }
}

/// What to say of a feature that a reference needs and that comes later
/// in the history than the feature being built (the one after `earlier`).
/// Moving it before is the remedy, unless it is itself made from the
/// feature being built: then no order builds both, and the reason says
/// so instead of sending the user round in a circle.
pub(crate) fn comes_after(
    doc: &Document,
    earlier: &[FeatureResult],
    later: FeatureId,
) -> &'static str {
    let this = doc.features.get(earlier.len()).map(|f| f.id);
    if this == Some(later) {
        return "is this feature itself; point the reference at something else";
    }
    let circle = match (this, doc.feature(later)) {
        (Some(this), Some(later)) => cache::dependencies(doc, later).contains(&this),
        _ => false,
    };
    if circle {
        "comes after this feature in the history and is itself made from this feature, directly or through others: the two cannot both be built; point one of them at something else"
    } else {
        "comes after this feature in the history; move it before (feature.move)"
    }
}

/// Regenerates the whole document, from nothing.
pub fn regenerate(doc: &Document) -> Regen {
    regenerate_cached(doc, &mut RegenCache::default())
}

/// Regenerates the document, rebuilding only the features whose result
/// can differ from what `cache` remembers of the last regeneration. The
/// result is exactly that of [`regenerate`].
pub fn regenerate_cached(doc: &Document, cache: &mut RegenCache) -> Regen {
    let evaluation = match doc.variable_table() {
        Ok(table) => table.evaluate(),
        // An invalid table (a hand-edited file): every variable fails the
        // same way, and so does every feature that uses one.
        Err(error) => {
            let message = error.to_string();
            let variables = doc
                .variables
                .iter()
                .map(|v| VariableResult {
                    name: v.name.clone(),
                    expr: v.expr.source().to_string(),
                    value: Err(message.clone()),
                })
                .collect();
            return Regen {
                variables,
                features: features(doc, &craft3d_expr::VariableTable::new().evaluate(), cache),
            };
        }
    };
    let variables = evaluation
        .entries()
        .iter()
        .map(|entry| VariableResult {
            name: entry.name.clone(),
            expr: doc
                .variable(&entry.name)
                .map(|v| v.expr.source().to_string())
                .unwrap_or_default(),
            value: match &entry.result {
                Ok(quantity) => Ok(to_document(*quantity, doc)),
                Err(error) => Err(error.to_string()),
            },
        })
        .collect();
    Regen {
        variables,
        features: features(doc, &evaluation, cache),
    }
}

fn features(doc: &Document, variables: &Evaluation, cache: &mut RegenCache) -> Vec<FeatureResult> {
    cache::features(
        doc,
        variables,
        cache,
        |feature, rolled_back, earlier, cache| {
            build_feature(doc, variables, feature, rolled_back, earlier, cache)
        },
    )
}

/// Builds one feature from the results of the ones before it. Also
/// returns the earlier feature whose body it took, if any.
fn build_feature(
    doc: &Document,
    variables: &Evaluation,
    feature: &Feature,
    rolled_back: bool,
    earlier: &[FeatureResult],
    cache: &mut RegenCache,
) -> (FeatureResult, Option<FeatureId>) {
    let mut solved = None;
    let mut names = None;
    let mut instances = None;
    let mut taken = None;
    let mut placed = None;
    let mut followed = None;
    let mut threaded = None;
    let (status, body) = if rolled_back {
        (Status::RolledBack, None)
    } else if feature.suppressed {
        (Status::Suppressed, None)
    } else if let FeatureKind::Sketch(sketch) = &feature.kind {
        // Where a sketch lies does not change how it solves: it is solved
        // flat (and that is remembered), then put in its place.
        match sketch::place(doc, variables, sketch, earlier) {
            Err(reason) => (Status::Failed(reason), None),
            Ok(placement) => {
                let (status, flat) = cache.solve(feature.id, doc, variables, sketch);
                solved = flat.map(|flat| {
                    Arc::new(SketchResult {
                        origin: placement.origin,
                        x_axis: placement.x_axis,
                        y_axis: placement.y_axis,
                        normal: placement.normal,
                        ..flat
                    })
                });
                (status, None)
            }
        }
    } else if let FeatureKind::DatumPlane(plane) = &feature.kind {
        match datum::plane(doc, variables, plane, earlier) {
            Ok(frame) => {
                placed = Some(Datum::Plane(frame));
                (Status::Ok, None)
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::DatumPoint(point) = &feature.kind {
        match datum::point(doc, variables, point, earlier) {
            Ok(at) => {
                placed = Some(Datum::Point(at));
                (Status::Ok, None)
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::DatumHelix(helix) = &feature.kind {
        match datum::helix(doc, variables, helix, earlier) {
            Ok(helix) => {
                placed = Some(Datum::Helix(helix));
                (Status::Ok, None)
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::DatumAxis(axis) = &feature.kind {
        match datum::axis(doc, variables, axis, earlier) {
            Ok(line) => {
                placed = Some(Datum::Axis {
                    origin: line.origin,
                    direction: line.direction,
                });
                (Status::Ok, None)
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::Extrude(_) | FeatureKind::Revolve(_) = &feature.kind {
        let swept = match &feature.kind {
            FeatureKind::Revolve(revolution) => revolve::build(doc, variables, revolution, earlier),
            FeatureKind::Extrude(extrusion) => extrude::build(doc, variables, extrusion, earlier),
            _ => Err("not a sweep".to_string()),
        };
        match swept.and_then(|(body, origins)| checked(doc, body).map(|body| (body, origins))) {
            Ok((body, origins)) => {
                names = Some(Arc::new(Names::of_origins(feature.id, &origins)));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::Loft(lofted) = &feature.kind {
        match loft::build(doc, feature.id, lofted, earlier)
            .and_then(|(body, names)| checked(doc, body).map(|body| (body, names)))
        {
            Ok((body, made_names)) => {
                names = Some(Arc::new(made_names));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::Sweep(swept) = &feature.kind {
        match sweep::build(doc, feature.id, swept, earlier)
            .and_then(|(body, names, report)| checked(doc, body).map(|body| (body, names, report)))
        {
            Ok((body, made_names, report)) => {
                names = Some(Arc::new(made_names));
                followed = Some(report);
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::Boolean(boolean) = &feature.kind {
        match boolean::build(doc, feature.id, boolean, earlier)
            .and_then(|(body, names)| checked(doc, body).map(|body| (body, names)))
        {
            Ok((body, made_names)) => {
                names = made_names.map(Arc::new);
                // Neither operand moves: a face of the result lies where
                // the face it came from lies.
                instances = Some(Arc::new(vec![Transform::IDENTITY; 2]));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::Chamfer(chamfer) = &feature.kind {
        match chamfer::build(doc, variables, feature.id, chamfer, earlier)
            .and_then(|(body, names)| checked(doc, body).map(|body| (body, names)))
        {
            Ok((body, made_names)) => {
                names = made_names.map(Arc::new);
                // The body does not move: a face lies where the face it
                // came from lies.
                instances = Some(Arc::new(vec![Transform::IDENTITY]));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::Fillet(fillet) = &feature.kind {
        match fillet::build(doc, variables, feature.id, fillet, earlier)
            .and_then(|(body, names)| checked(doc, body).map(|body| (body, names)))
        {
            Ok((body, made_names)) => {
                names = made_names.map(Arc::new);
                // The body does not move: a face lies where the face it
                // came from lies.
                instances = Some(Arc::new(vec![Transform::IDENTITY]));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::OffsetFace(offset) = &feature.kind {
        match offset_face::build(doc, variables, feature.id, offset, earlier)
            .and_then(|(body, names)| checked(doc, body).map(|body| (body, names)))
        {
            Ok((body, made_names)) => {
                names = made_names.map(Arc::new);
                // The body as a whole does not move; a face that did is
                // found where it lies now (`sketch::resolve_face`).
                instances = Some(Arc::new(vec![Transform::IDENTITY]));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::DeleteFace(deletion) = &feature.kind {
        match delete_face::build(doc, feature.id, deletion, earlier)
            .and_then(|(body, names)| checked(doc, body).map(|body| (body, names)))
        {
            Ok((body, made_names)) => {
                names = made_names.map(Arc::new);
                // Nothing moves: a face lies where the face it is a copy
                // of lies, on the surface it had.
                instances = Some(Arc::new(vec![Transform::IDENTITY]));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::Thread(cut) = &feature.kind {
        match thread::build(doc, variables, feature.id, cut, earlier)
            .and_then(|(body, names, report)| checked(doc, body).map(|body| (body, names, report)))
        {
            Ok((body, made_names, report)) => {
                names = made_names.map(Arc::new);
                threaded = Some(report);
                // Neither the body nor the groove moves.
                instances = Some(Arc::new(vec![Transform::IDENTITY; 2]));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::Shell(hollowed) = &feature.kind {
        match shell::build(doc, variables, feature.id, hollowed, earlier)
            .and_then(|(body, names)| checked(doc, body).map(|body| (body, names)))
        {
            Ok((body, made_names)) => {
                names = made_names.map(Arc::new);
                // Instance 0 is the skin, 1 the hollow; neither moves as
                // a whole (`sketch::resolve_face` finds a face of the
                // hollow where it lies).
                instances = Some(Arc::new(vec![Transform::IDENTITY; 2]));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let Some((source, keep)) = feature.kind.source() {
        match derive::build(doc, variables, feature, earlier).and_then(|made| {
            let body = checked(doc, made.body)?;
            Ok((body, made.names, made.instances))
        }) {
            Ok((body, made_names, maps)) => {
                names = made_names.map(Arc::new);
                instances = Some(Arc::new(maps));
                if !keep {
                    taken = Some(source);
                }
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else {
        match build(doc, variables, feature) {
            Ok(body) => {
                names = Some(Arc::new(Names::numbered(feature.id, &body)));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    };
    (
        FeatureResult {
            id: feature.id,
            status,
            body,
            sketch: solved,
            datum: placed,
            names,
            instances,
            sweep: followed,
            thread: threaded,
            consumed_by: None,
            followed: None,
        },
        taken,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_doc::{Block, Cylinder, LengthUnit, Sphere, Units};
    use craft3d_topo::mass_properties;
    use std::f64::consts::PI;

    fn exprs(values: [&str; 3]) -> [Expr; 3] {
        values.map(Expr::new)
    }

    fn sample(units: Units) -> Document {
        let mut doc = Document::new(units);
        doc.set_variable("width", "40 mm").unwrap();
        doc.set_variable("half", "width / 2").unwrap();
        doc.add_feature(
            None,
            FeatureKind::Block(Block {
                position: exprs(["0", "0", "0"]),
                size: exprs(["width", "half", "10 mm"]),
                orientation: Default::default(),
            }),
        )
        .unwrap();
        doc.add_feature(
            Some("Pin"),
            FeatureKind::Cylinder(Cylinder {
                position: exprs(["half", "10 mm", "10 mm"]),
                radius: Expr::new("width / 8"),
                height: Expr::new("2 cm"),
                orientation: Default::default(),
            }),
        )
        .unwrap();
        doc.add_feature(
            Some("Ball"),
            FeatureKind::Sphere(Sphere {
                position: exprs(["0 mm", "0 mm", "50 mm"]),
                radius: Expr::new("half / 2"),
            }),
        )
        .unwrap();
        doc
    }

    fn volumes(regen: &Regen) -> Vec<f64> {
        regen
            .bodies()
            .map(|(_, body)| mass_properties(body).unwrap().volume)
            .collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-12 * b.abs()
    }

    #[test]
    fn features_become_bodies_of_the_expected_size_and_place() {
        let regen = regenerate(&sample(Units::default()));
        assert!(regen.is_ok());
        let v = volumes(&regen);
        assert!(close(v[0], 40.0 * 20.0 * 10.0));
        assert!(close(v[1], PI * 25.0 * 20.0));
        assert!(close(v[2], 4.0 / 3.0 * PI * 1000.0));
        let centroid = |i: usize| {
            mass_properties(regen.features[i].body.as_ref().unwrap())
                .unwrap()
                .centroid
                .unwrap()
        };
        assert!(centroid(0).distance(Vec3::new(20.0, 10.0, 5.0)) < 1e-10);
        assert!(centroid(1).distance(Vec3::new(20.0, 10.0, 20.0)) < 1e-10);
        assert!(centroid(2).distance(Vec3::new(0.0, 0.0, 50.0)) < 1e-10);
        // Variables come back in the document's unit, dependencies first.
        let half = regen.variables.iter().find(|v| v.name == "half").unwrap();
        assert_eq!(
            (half.expr.as_str(), half.value.clone().unwrap().value),
            ("width / 2", 20.0)
        );
    }

    #[test]
    fn the_same_recipe_in_inches_gives_the_same_part_in_inches() {
        let units = Units {
            length: LengthUnit::Inch,
            ..Units::default()
        };
        let regen = regenerate(&sample(units));
        assert!(regen.is_ok());
        let per_inch = 25.4_f64;
        assert!(close(
            volumes(&regen)[0],
            40.0 * 20.0 * 10.0 / per_inch.powi(3)
        ));
        let width = regen.variables.iter().find(|v| v.name == "width").unwrap();
        assert!(close(width.value.clone().unwrap().value, 40.0 / per_inch));
        // A bare number is in the document's unit: 1 means one inch here.
        let mut doc = Document::new(units);
        doc.add_feature(
            None,
            FeatureKind::Block(Block {
                position: exprs(["0", "0", "0"]),
                size: exprs(["1", "25.4 mm", "1 in"]),
                orientation: Default::default(),
            }),
        )
        .unwrap();
        assert!(close(volumes(&regenerate(&doc))[0], 1.0));
    }

    #[test]
    fn changing_a_variable_changes_the_parts_that_use_it() {
        let mut doc = sample(Units::default());
        let before = volumes(&regenerate(&doc));
        doc.set_variable("width", "80 mm").unwrap();
        let after = volumes(&regenerate(&doc));
        assert!(
            close(after[0], before[0] * 4.0),
            "block: width and half both doubled"
        );
        assert!(close(after[1], before[1] * 4.0), "pin: radius doubled");
        assert!(close(after[2], before[2] * 8.0), "ball: radius doubled");
    }

    #[test]
    fn a_failure_stays_local_and_says_what_is_wrong() {
        let mut doc = sample(Units::default());
        doc.set_variable("half", "width / 0").unwrap();
        let regen = regenerate(&doc);
        assert!(!regen.is_ok());
        let half = regen.variables.iter().find(|v| v.name == "half").unwrap();
        assert!(
            half.value.as_ref().unwrap_err().contains("zero"),
            "{:?}",
            half.value
        );
        assert!(
            regen
                .variables
                .iter()
                .find(|v| v.name == "width")
                .unwrap()
                .value
                .is_ok()
        );
        let Status::Failed(reason) = &regen.features[0].status else {
            panic!("the block uses `half`")
        };
        assert!(reason.starts_with("size.y: `half`"), "{reason}");
        assert!(regen.features[0].body.is_none());
        // Only the features that use `half` fail.
        doc.features[2].kind = FeatureKind::Sphere(Sphere {
            position: exprs(["0", "0", "0"]),
            radius: Expr::new("width"),
        });
        let regen = regenerate(&doc);
        assert_eq!(regen.features[2].status, Status::Ok);
        assert_eq!(regen.bodies().count(), 1);
    }

    #[test]
    fn bad_parameters_are_failures_with_the_parameter_named() {
        let cases = [
            ("0 mm", "size.x must be"),
            ("-5 mm", "size.x must be"),
            ("30 deg", "but a length is needed"),
            ("1 mm * 1 mm", "but a length is needed"),
            ("nope", "unknown"),
            ("1 +", "size.x"),
            ("1e300 * 1e300", "size.x"),
        ];
        for (source, expected) in cases {
            let mut doc = Document::new(Units::default());
            doc.add_feature(
                None,
                FeatureKind::Block(Block {
                    position: exprs(["0", "0", "0"]),
                    size: exprs([source, "1", "1"]),
                    orientation: Default::default(),
                }),
            )
            .unwrap();
            let regen = regenerate(&doc);
            let Status::Failed(reason) = &regen.features[0].status else {
                panic!("`{source}` should fail")
            };
            assert!(reason.contains(expected), "`{source}`: {reason}");
        }
    }

    fn turned(axis: [&str; 3], angle: &str) -> Orientation {
        Orientation {
            axis: exprs(axis),
            angle: Expr::new(angle),
        }
    }

    fn single(kind: FeatureKind) -> (Regen, Document) {
        let mut doc = Document::new(Units::default());
        doc.add_feature(None, kind).unwrap();
        (regenerate(&doc), doc)
    }

    #[test]
    fn cones_and_tori_have_their_closed_form_volumes() {
        let (regen, _) = single(FeatureKind::Cone(craft3d_doc::Cone {
            position: exprs(["1", "2", "3"]),
            radius: Expr::new("2 mm"),
            height: Expr::new("6 mm"),
            orientation: Orientation::default(),
        }));
        let mass = mass_properties(regen.features[0].body.as_ref().unwrap()).unwrap();
        assert!(close(mass.volume, PI * 4.0 * 6.0 / 3.0));
        // The centroid of a cone is a quarter of the way up.
        assert!(mass.centroid.unwrap().distance(Vec3::new(1.0, 2.0, 4.5)) < 1e-10);

        let (regen, _) = single(FeatureKind::Torus(craft3d_doc::Torus {
            position: exprs(["0", "0", "0"]),
            major: Expr::new("5 mm"),
            minor: Expr::new("1 mm"),
            orientation: Orientation::default(),
        }));
        assert!(close(volumes(&regen)[0], 2.0 * PI * PI * 5.0));
        let (regen, _) = single(FeatureKind::Torus(craft3d_doc::Torus {
            position: exprs(["0", "0", "0"]),
            major: Expr::new("1 mm"),
            minor: Expr::new("1 mm"),
            orientation: Orientation::default(),
        }));
        let Status::Failed(reason) = &regen.features[0].status else {
            panic!()
        };
        assert!(
            reason.contains("major (1) must be larger than minor (1)"),
            "{reason}"
        );
    }

    #[test]
    fn orientation_turns_a_primitive_without_changing_its_size() {
        let cylinder = |orientation| {
            FeatureKind::Cylinder(Cylinder {
                position: exprs(["10 mm", "0", "0"]),
                radius: Expr::new("1 mm"),
                height: Expr::new("8 mm"),
                orientation,
            })
        };
        // Lying along +x: the centroid is 4 mm along x from the base.
        let (regen, _) = single(cylinder(turned(["1", "0", "0"], "0")));
        let mass = mass_properties(regen.features[0].body.as_ref().unwrap()).unwrap();
        assert!(close(mass.volume, PI * 8.0));
        assert!(mass.centroid.unwrap().distance(Vec3::new(14.0, 0.0, 0.0)) < 1e-10);
        // Along a diagonal, with an axis of any length.
        let (regen, _) = single(cylinder(turned(["0", "3", "3"], "45 deg")));
        let s = 4.0 / 2.0_f64.sqrt();
        let mass = mass_properties(regen.features[0].body.as_ref().unwrap()).unwrap();
        assert!(mass.centroid.unwrap().distance(Vec3::new(10.0, s, s)) < 1e-10);

        // A block turned a quarter about z: its far corner moves from
        // (+x, +y) to (-y side): centroid of a 4 x 2 x 1 block at (2, 1, .5)
        // goes to (-1, 2, .5).
        let block = |angle: &str| {
            FeatureKind::Block(Block {
                position: exprs(["0", "0", "0"]),
                size: exprs(["4", "2", "1"]),
                orientation: turned(["0", "0", "1"], angle),
            })
        };
        for (angle, expected) in [
            ("0", Vec3::new(2.0, 1.0, 0.5)),
            ("90 deg", Vec3::new(-1.0, 2.0, 0.5)),
            ("90", Vec3::new(-1.0, 2.0, 0.5)),
            ("pi / 2 * 1 rad", Vec3::new(-1.0, 2.0, 0.5)),
            ("180 deg", Vec3::new(-2.0, -1.0, 0.5)),
        ] {
            let (regen, _) = single(block(angle));
            let mass = mass_properties(regen.features[0].body.as_ref().unwrap()).unwrap();
            assert!(close(mass.volume, 8.0), "angle {angle}");
            assert!(
                mass.centroid.unwrap().distance(expected) < 1e-10,
                "angle {angle}: {:?}",
                mass.centroid
            );
        }
    }

    #[test]
    fn a_bad_orientation_is_a_failure_that_names_the_parameter() {
        for (axis, angle, expected) in [
            (["0", "0", "0"], "0", "axis must be a direction"),
            (
                ["1 mm", "0", "0"],
                "0",
                "axis.x: `1 mm` is a length, but a plain number is needed",
            ),
            (["0", "nope", "1"], "0", "axis.y"),
            (
                ["0", "0", "1"],
                "5 mm",
                "angle: `5 mm` is a length, but an angle is needed",
            ),
            (["0", "0", "1"], "1 / 0", "angle"),
        ] {
            let (regen, _) = single(FeatureKind::Cylinder(Cylinder {
                position: exprs(["0", "0", "0"]),
                radius: Expr::new("1"),
                height: Expr::new("1"),
                orientation: turned(axis, angle),
            }));
            let Status::Failed(reason) = &regen.features[0].status else {
                panic!("axis {axis:?} angle {angle} should fail")
            };
            assert!(reason.contains(expected), "{reason}");
        }
    }

    #[test]
    fn suppressed_and_rolled_back_features_make_no_bodies() {
        let mut doc = sample(Units::default());
        doc.feature_mut(FeatureId(2)).unwrap().suppressed = true;
        let regen = regenerate(&doc);
        assert_eq!(regen.features[1].status, Status::Suppressed);
        assert_eq!(
            regen.bodies().map(|(id, _)| id.0).collect::<Vec<_>>(),
            [1, 3]
        );
        assert!(regen.is_ok());

        doc.set_rollback(Some(FeatureId(1))).unwrap();
        let regen = regenerate(&doc);
        let statuses: Vec<_> = regen.features.iter().map(|f| f.status.clone()).collect();
        assert_eq!(
            statuses,
            [Status::Ok, Status::RolledBack, Status::RolledBack]
        );
        assert_eq!(regen.feature(FeatureId(3)).unwrap().body, None);
    }

    #[test]
    fn regeneration_is_deterministic() {
        let doc = sample(Units::default());
        assert_eq!(regenerate(&doc), regenerate(&doc));
        assert_eq!(
            regenerate(&Document::new(Units::default())),
            Regen::default()
        );
    }
}
