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
pub mod curve_fit;
mod datum;
mod delete_face;
mod developable;
pub use developable::{Developed, placement};
mod derive;
mod draft;
mod entity;
mod extrude;
mod face_ops;
mod fillet;
mod fold;
mod follow;
mod loft;
mod move_face;
mod offset_face;
mod offset_profile;
mod pass;
mod plate;
pub mod progress;
mod query;
mod revolve;
mod selfcheck;
mod sheetmetal;
mod shell;
mod sketch;
mod sketch_pattern;
mod split;
mod sweep;
mod thicken;
mod thread;
pub mod work;

pub use cache::{DEFAULT_BODY_BUDGET, Need, RegenCache};
pub use cause::root_cause;
pub use craft3d_kernel::extrude::{Origin, Origins, Role};
pub use craft3d_kernel::helix::Helix;
pub use craft3d_kernel::offset_profile::{OffsetOrigin, OffsetOutline, OffsetSource};
pub use craft3d_kernel::profile::{Curve2, SketchCurve as PatternCurve};
/// The kernel keeps search trees of freeform surfaces between operations;
/// the engine lets go of those no open document holds.
pub use craft3d_kernel::release_unused as release_unused_trees;
pub use craft3d_kernel::thread::{coarse_pitch, no_coarse_pitch, nominal_diameter};
pub use datum::{Datum, EdgeReference, References, references};
pub use derive::{MAX_PATTERN_FACES, MAX_PATTERN_INSTANCES};
pub use extrude::name_of;
pub use fold::{FlatFold, FoldAnalysis, FoldPattern, fold_analysis, fold_pattern};
pub use follow::{follow_face, follow_face_pieces, newest_body, short_face_name};

/// The feature `feature` as regeneration builds it after `earlier` (the
/// results before it): its faces, edges and a shell's body followed to
/// the newest body of their chain, `None` when nothing is followed. An
/// error is the reason it fails. What a feature inserted into a history
/// takes is read from this ([`craft3d_doc::FeatureKind::grows_from`] of
/// the followed kind).
pub fn followed_feature(
    doc: &craft3d_doc::Document,
    earlier: &[FeatureResult],
    feature: &craft3d_doc::Feature,
) -> Result<Option<craft3d_doc::Feature>, String> {
    follow::feature(doc, earlier, feature)
}
pub use offset_profile::OffsetResult;
pub use pass::{Passed, absent_name, left_out};
pub use query::{ambiguous_cap, edges_between, faces_named, faces_of_edges};
pub use sheetmetal::{SheetMetalPart, sheet_face_name, sheet_metal_part};
pub use sketch::{
    ConstraintResult, ConstraintState, FittedCurve, SketchResult, fit_formula_curve,
    image_places_to_record, record_image_places, settle_sketches, settle_sketches_cached,
};
pub use sketch_pattern::{MAX_PATTERN_CURVES, PatternResult, Placed};
pub use split::{
    SplitPlan, anchors as lump_anchors, shared_by_splits as taken_by_its_splits,
    siblings as splits_of, split_plan,
};

/// The instances of the patterns of a solved sketch, construction curves
/// included, each with whether it is construction: the curves a picture of
/// the sketch draws besides its entities.
pub fn pattern_instances(
    sketch: &craft3d_doc::Sketch,
    solved: &SketchResult,
) -> Result<Vec<(PatternCurve, bool)>, String> {
    sketch_pattern::instance_curves(sketch, solved, true)
}

use craft3d_doc::{
    Document, Expr, Feature, FeatureId, FeatureKind, NameRole, Orientation, PersistentName,
};
use craft3d_expr::{Dimension, Evaluation, Quantity};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::Body;
use entity::Entities;
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
    /// Switched off by its condition (`when` is 0): behaves exactly as
    /// `Suppressed`.
    Inactive,
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
/// (docs/design/names.md).
///
/// A face that a boolean merged from faces of both operands (two flush
/// blocks united: their tops become one face) is named after the first
/// contributor in the kernel's order and carries the names of the others
/// as *aliases*: a reference by any of them resolves to it
/// (docs/design/names.md).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Names {
    pub faces: Vec<PersistentName>,
    /// Per face that has them (by its index in `faces`), the other names
    /// it answers to, in the kernel's order. Empty for a body no boolean
    /// merged faces in.
    pub aliases: BTreeMap<usize, Vec<PersistentName>>,
    /// The names of the edges and of the vertices, and what each kept
    /// from the body it came from, in the form that takes the least room
    /// (`entity::Entities`): a name is made when it is asked for
    /// ([`Names::edge_name`]).
    entities: Entities,
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
    /// Names given outright: a primitive's, a sweep's, each entity by
    /// where it came from.
    pub fn listed(
        faces: Vec<PersistentName>,
        edges: Vec<PersistentName>,
        vertices: Vec<PersistentName>,
    ) -> Self {
        Self {
            faces,
            aliases: BTreeMap::new(),
            entities: Entities::Listed { edges, vertices },
        }
    }

    /// Faces named `faces`, and the edges and vertices of `body` named by
    /// the faces they lie on (`entity::by_faces`), keeping nothing from
    /// other bodies.
    pub(crate) fn by_faces(
        feature: FeatureId,
        body: &Body,
        faces: Vec<PersistentName>,
        tolerance: f64,
    ) -> Result<Self, String> {
        let made = entity::by_faces(feature, body, &faces, tolerance)?;
        Ok(Self {
            faces,
            aliases: BTreeMap::new(),
            entities: Entities::ByFaces(Box::new(made)),
        })
    }

    /// The names of a body whose entities are, instance by instance,
    /// copies of the entities of other bodies (a transform, a mirror, a
    /// pattern whose instances stay apart; an offset face, which keeps
    /// every entity under its number): for `(instance, source)` of
    /// `instances`, in order, every entity of the body `source` names, in
    /// its order, as the copy of what it was. What an entity answered to
    /// besides its own name, its copy answers to as a copy. The names of
    /// the faces are made now (a name too long to write is an error, as
    /// for any copy); the edges and vertices stay as `source` has them
    /// and are copied when asked for.
    pub(crate) fn copied(
        feature: FeatureId,
        instances: Vec<(u64, Arc<Names>)>,
    ) -> Result<Self, String> {
        let mut out = Names::default();
        let mut edge_starts = Vec::with_capacity(instances.len().saturating_add(1));
        let mut vertex_starts = Vec::with_capacity(instances.len().saturating_add(1));
        let (mut edges, mut vertices) = (0usize, 0usize);
        for (instance, source) in &instances {
            derive::copied_aliases(
                feature,
                *instance,
                &source.aliases,
                out.faces.len(),
                &mut out.aliases,
            );
            derive::copied(feature, *instance, &source.faces, &mut out.faces)?;
            edge_starts.push(edges);
            vertex_starts.push(vertices);
            edges = edges.saturating_add(source.edge_count());
            vertices = vertices.saturating_add(source.vertex_count());
        }
        edge_starts.push(edges);
        vertex_starts.push(vertices);
        out.entities = Entities::Copied {
            feature,
            instances,
            edge_starts,
            vertex_starts,
        };
        // A copy of an edge's or a vertex's name is held to the caps of a
        // name as the faces' are, as when every copy was written out.
        for name in out.edge_names().chain(out.vertex_names()) {
            name.check().map_err(too_long)?;
        }
        Ok(out)
    }

    /// The names of a body whose entities are some of the entities of
    /// the body `source` names (one lump of it, taken by
    /// [`craft3d_topo::shells_alone`]): each the copy, in instance 0 of
    /// `feature`, of the entity of the source it is (`taken` says which),
    /// answering as a copy to what that entity answered to besides its
    /// own name. The names of the faces are made now; the edges and
    /// vertices stay as `source` has them and are copied when asked for.
    pub(crate) fn picked(
        feature: FeatureId,
        source: Arc<Names>,
        taken: &craft3d_topo::Taken,
    ) -> Result<Self, String> {
        let mut out = Names::default();
        for (index, face) in taken.faces.iter().enumerate() {
            let there = face.0 as usize;
            let Some(name) = source.faces.get(there) else {
                return Err(format!(
                    "the body it is made from has no face {there} to name"
                ));
            };
            let copy = PersistentName::copy(feature, 0, name.clone());
            copy.check().map_err(too_long)?;
            out.faces.push(copy);
            let aliases = copies(feature, 0, source.aliases_of(there).to_vec());
            if !aliases.is_empty() {
                out.aliases.insert(index, aliases);
            }
        }
        out.entities = Entities::Picked {
            feature,
            source,
            edges: taken.edges.iter().map(|e| e.0 as usize).collect(),
            vertices: taken.vertices.iter().map(|v| v.0 as usize).collect(),
        };
        for name in out.edge_names().chain(out.vertex_names()) {
            name.check().map_err(too_long)?;
        }
        Ok(out)
    }

    /// These names with those of the edges and the vertices let go of
    /// (`Entities::Dropped`): the faces' names and aliases stay, which is
    /// what following a name through the bodies it went into needs.
    pub fn without_entities(&self) -> Self {
        Self {
            faces: self.faces.clone(),
            aliases: self.aliases.clone(),
            entities: Entities::Dropped {
                edges: self.edge_count(),
                vertices: self.vertex_count(),
            },
        }
    }

    /// False when the names of the edges and the vertices were let go of.
    pub fn has_entities(&self) -> bool {
        !matches!(self.entities, Entities::Dropped { .. })
    }

    /// The listed names of the edges and the vertices, to change, for a
    /// body whose names were given outright ([`Names::listed`]); `None`
    /// for one named otherwise.
    pub(crate) fn listed_mut(
        &mut self,
    ) -> Option<(&mut Vec<PersistentName>, &mut Vec<PersistentName>)> {
        match &mut self.entities {
            Entities::Listed { edges, vertices } => Some((edges, vertices)),
            _ => None,
        }
    }

    /// For entity `index` of a body of copies: the instance it is in, the
    /// names of that instance's source and its index there. `starts` is
    /// where each instance's entities start, with the total at the end.
    fn copied_at<'a>(
        feature: FeatureId,
        instances: &'a [(u64, Arc<Names>)],
        starts: &[usize],
        index: usize,
    ) -> Option<(FeatureId, u64, &'a Arc<Names>, usize)> {
        let which = starts
            .partition_point(|start| *start <= index)
            .checked_sub(1)?;
        let (instance, source) = instances.get(which)?;
        let start = *starts.get(which)?;
        (index < *starts.get(which.saturating_add(1))?)
            .then(|| (feature, *instance, source, index.saturating_sub(start)))
    }

    /// The entity of a body of copies called `name`: in the instance the
    /// name says (the first number of a copy is its instance), the one
    /// whose name copied is `name`.
    fn copied_named(
        feature: FeatureId,
        instances: &[(u64, Arc<Names>)],
        starts: &[usize],
        name: &PersistentName,
        name_of: fn(&Names, usize) -> Option<PersistentName>,
    ) -> Option<usize> {
        if name.feature != feature || name.role != NameRole::Copy {
            return None;
        }
        let wanted = *name.sources.first()?;
        for (which, (instance, source)) in instances.iter().enumerate() {
            if *instance != wanted {
                continue;
            }
            let start = *starts.get(which)?;
            let count = starts.get(which.saturating_add(1))?.saturating_sub(start);
            if let Some(there) = (0..count).find(|there| {
                name_of(source, *there)
                    .is_some_and(|own| PersistentName::copy(feature, *instance, own) == *name)
            }) {
                return Some(start.saturating_add(there));
            }
        }
        None
    }

    pub fn edge_count(&self) -> usize {
        match &self.entities {
            Entities::Listed { edges, .. } => edges.len(),
            Entities::Dropped { edges, .. } => *edges,
            Entities::ByFaces(by) => by.edge_count(),
            Entities::Copied { edge_starts, .. } => edge_starts.last().copied().unwrap_or(0),
            Entities::Picked { edges, .. } => edges.len(),
        }
    }

    pub fn vertex_count(&self) -> usize {
        match &self.entities {
            Entities::Listed { vertices, .. } => vertices.len(),
            Entities::Dropped { vertices, .. } => *vertices,
            Entities::ByFaces(by) => by.vertex_count(),
            Entities::Copied { vertex_starts, .. } => vertex_starts.last().copied().unwrap_or(0),
            Entities::Picked { vertices, .. } => vertices.len(),
        }
    }

    /// The name of edge `edge` (by its index in the body), made from
    /// what is stored.
    pub fn edge_name(&self, edge: usize) -> Option<PersistentName> {
        match &self.entities {
            Entities::Listed { edges, .. } => edges.get(edge).cloned(),
            Entities::Dropped { .. } => None,
            Entities::ByFaces(by) => by.edge(&self.faces, edge),
            Entities::Copied {
                feature,
                instances,
                edge_starts,
                ..
            } => {
                let (feature, instance, source, there) =
                    Self::copied_at(*feature, instances, edge_starts, edge)?;
                Some(PersistentName::copy(
                    feature,
                    instance,
                    source.edge_name(there)?,
                ))
            }
            Entities::Picked {
                feature,
                source,
                edges,
                ..
            } => Some(PersistentName::copy(
                *feature,
                0,
                source.edge_name(*edges.get(edge)?)?,
            )),
        }
    }

    /// The name of vertex `vertex`.
    pub fn vertex_name(&self, vertex: usize) -> Option<PersistentName> {
        match &self.entities {
            Entities::Listed { vertices, .. } => vertices.get(vertex).cloned(),
            Entities::Dropped { .. } => None,
            Entities::ByFaces(by) => by.vertex(&self.faces, vertex),
            Entities::Copied {
                feature,
                instances,
                vertex_starts,
                ..
            } => {
                let (feature, instance, source, there) =
                    Self::copied_at(*feature, instances, vertex_starts, vertex)?;
                Some(PersistentName::copy(
                    feature,
                    instance,
                    source.vertex_name(there)?,
                ))
            }
            Entities::Picked {
                feature,
                source,
                vertices,
                ..
            } => Some(PersistentName::copy(
                *feature,
                0,
                source.vertex_name(*vertices.get(vertex)?)?,
            )),
        }
    }

    /// The names of every edge, in the body's order: for a body of
    /// copies instance by instance, each the source's names copied.
    pub fn edge_names(&self) -> Box<dyn Iterator<Item = PersistentName> + '_> {
        match &self.entities {
            Entities::Copied {
                feature, instances, ..
            } => Box::new(instances.iter().flat_map(move |(instance, source)| {
                source
                    .edge_names()
                    .map(move |name| PersistentName::copy(*feature, *instance, name))
            })),
            _ => Box::new((0..self.edge_count()).filter_map(|edge| self.edge_name(edge))),
        }
    }

    /// The names of every vertex, in the body's order.
    pub fn vertex_names(&self) -> Box<dyn Iterator<Item = PersistentName> + '_> {
        match &self.entities {
            Entities::Copied {
                feature, instances, ..
            } => Box::new(instances.iter().flat_map(move |(instance, source)| {
                source
                    .vertex_names()
                    .map(move |name| PersistentName::copy(*feature, *instance, name))
            })),
            _ => Box::new((0..self.vertex_count()).filter_map(|vertex| self.vertex_name(vertex))),
        }
    }

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

    /// True when face `face` is called `name`, by its own name or an alias,
    /// or `name` is the short form of a cap (`F2:startCap()`) and the face
    /// is a cap of that role of that feature ([`PersistentName::shortens`]).
    pub fn face_answers_to(&self, face: usize, name: &PersistentName) -> bool {
        if name.is_short_cap() {
            return self.names_of(face).any(|n| name.shortens(n));
        }
        self.names_of(face).any(|n| n == name)
    }

    /// The other names edge `edge` answers to besides its own: the names
    /// it kept from the bodies it came from.
    pub fn edge_aliases_of(&self, edge: usize) -> Vec<PersistentName> {
        match &self.entities {
            Entities::Listed { .. } | Entities::Dropped { .. } => Vec::new(),
            Entities::ByFaces(by) => by.edge_aliases.of(edge).to_vec(),
            Entities::Copied {
                feature,
                instances,
                edge_starts,
                ..
            } => Self::copied_at(*feature, instances, edge_starts, edge)
                .map(|(feature, instance, source, there)| {
                    copies(feature, instance, source.edge_aliases_of(there))
                })
                .unwrap_or_default(),
            Entities::Picked {
                feature,
                source,
                edges,
                ..
            } => edges
                .get(edge)
                .map(|&there| copies(*feature, 0, source.edge_aliases_of(there)))
                .unwrap_or_default(),
        }
    }

    /// The other names vertex `vertex` answers to besides its own.
    pub fn vertex_aliases_of(&self, vertex: usize) -> Vec<PersistentName> {
        match &self.entities {
            Entities::Listed { .. } | Entities::Dropped { .. } => Vec::new(),
            Entities::ByFaces(by) => by.vertex_aliases.of(vertex).to_vec(),
            Entities::Copied {
                feature,
                instances,
                vertex_starts,
                ..
            } => Self::copied_at(*feature, instances, vertex_starts, vertex)
                .map(|(feature, instance, source, there)| {
                    copies(feature, instance, source.vertex_aliases_of(there))
                })
                .unwrap_or_default(),
            Entities::Picked {
                feature,
                source,
                vertices,
                ..
            } => vertices
                .get(vertex)
                .map(|&there| copies(*feature, 0, source.vertex_aliases_of(there)))
                .unwrap_or_default(),
        }
    }

    /// Every name edge `edge` answers to: its own first.
    pub fn edge_names_of(&self, edge: usize) -> Vec<PersistentName> {
        let mut names: Vec<PersistentName> = self.edge_name(edge).into_iter().collect();
        names.extend(self.edge_aliases_of(edge));
        names
    }

    /// Every name vertex `vertex` answers to: its own first.
    pub fn vertex_names_of(&self, vertex: usize) -> Vec<PersistentName> {
        let mut names: Vec<PersistentName> = self.vertex_name(vertex).into_iter().collect();
        names.extend(self.vertex_aliases_of(vertex));
        names
    }

    /// The names edge `edge` answers to that a primitive or a sweep gave
    /// (its own when it is one, and those it kept): what an entity made
    /// from it keeps. A name by faces is not among them: what an edge
    /// was called by its faces in a body in between says nothing the
    /// faces of the new body do not say, and the lists would grow with
    /// every boolean.
    pub(crate) fn edge_kept_names(&self, edge: usize) -> Vec<PersistentName> {
        match &self.entities {
            Entities::Dropped { .. } => Vec::new(),
            Entities::Listed { edges, .. } => edges
                .get(edge)
                .filter(|name| !name.root().role.is_by_faces())
                .cloned()
                .into_iter()
                .collect(),
            Entities::ByFaces(by) => by.edge_aliases.of(edge).to_vec(),
            Entities::Copied {
                feature,
                instances,
                edge_starts,
                ..
            } => Self::copied_at(*feature, instances, edge_starts, edge)
                .map(|(feature, instance, source, there)| {
                    copies(feature, instance, source.edge_kept_names(there))
                })
                .unwrap_or_default(),
            Entities::Picked {
                feature,
                source,
                edges,
                ..
            } => edges
                .get(edge)
                .map(|&there| copies(*feature, 0, source.edge_kept_names(there)))
                .unwrap_or_default(),
        }
    }

    /// The names vertex `vertex` answers to that a primitive or a sweep
    /// gave ([`Names::edge_kept_names`]).
    pub(crate) fn vertex_kept_names(&self, vertex: usize) -> Vec<PersistentName> {
        match &self.entities {
            Entities::Dropped { .. } => Vec::new(),
            Entities::Listed { vertices, .. } => vertices
                .get(vertex)
                .filter(|name| !name.root().role.is_by_faces())
                .cloned()
                .into_iter()
                .collect(),
            Entities::ByFaces(by) => by.vertex_aliases.of(vertex).to_vec(),
            Entities::Copied {
                feature,
                instances,
                vertex_starts,
                ..
            } => Self::copied_at(*feature, instances, vertex_starts, vertex)
                .map(|(feature, instance, source, there)| {
                    copies(feature, instance, source.vertex_kept_names(there))
                })
                .unwrap_or_default(),
            Entities::Picked {
                feature,
                source,
                vertices,
                ..
            } => vertices
                .get(vertex)
                .map(|&there| copies(*feature, 0, source.vertex_kept_names(there)))
                .unwrap_or_default(),
        }
    }

    /// The edge called `name`, by its own name or an alias: its index.
    pub fn edge_named(&self, name: &PersistentName) -> Option<usize> {
        let own = match &self.entities {
            Entities::Listed { edges, .. } => edges.iter().position(|own| own == name),
            Entities::Dropped { .. } => None,
            Entities::ByFaces(by) => by.edge_named(&self.faces, name),
            Entities::Copied {
                feature,
                instances,
                edge_starts,
                ..
            } => Self::copied_named(*feature, instances, edge_starts, name, Names::edge_name),
            Entities::Picked { edges, .. } => {
                (0..edges.len()).find(|&edge| self.edge_name(edge).as_ref() == Some(name))
            }
        };
        own.or_else(|| {
            (0..self.edge_count()).find(|edge| self.edge_aliases_of(*edge).contains(name))
        })
    }

    /// The vertex called `name`, by its own name or an alias: its index.
    pub fn vertex_named(&self, name: &PersistentName) -> Option<usize> {
        let own = match &self.entities {
            Entities::Listed { vertices, .. } => vertices.iter().position(|own| own == name),
            Entities::Dropped { .. } => None,
            Entities::ByFaces(by) => by.vertex_named(&self.faces, name),
            Entities::Copied {
                feature,
                instances,
                vertex_starts,
                ..
            } => Self::copied_named(*feature, instances, vertex_starts, name, Names::vertex_name),
            Entities::Picked { vertices, .. } => {
                (0..vertices.len()).find(|&vertex| self.vertex_name(vertex).as_ref() == Some(name))
            }
        };
        own.or_else(|| {
            (0..self.vertex_count()).find(|vertex| self.vertex_aliases_of(*vertex).contains(name))
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
    ) -> Result<(), String> {
        let mut made = entity::by_faces(feature, body, &self.faces, tolerance)?;
        let edges = entity::kept_names(feature, kept_edges, |instance, edge| {
            source(instance).map_or_else(Vec::new, |names| names.edge_kept_names(edge))
        });
        let vertices = entity::kept_names(feature, kept_vertices, |instance, vertex| {
            source(instance).map_or_else(Vec::new, |names| names.vertex_kept_names(vertex))
        });
        made.edge_aliases = entity::Aliases::from_map(&edges, body.edges.len())?;
        made.vertex_aliases = entity::Aliases::from_map(&vertices, body.vertices.len())?;
        self.entities = Entities::ByFaces(Box::new(made));
        Ok(())
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
                let copy = PersistentName::copy(feature, instance, name.clone());
                copy.check().map_err(too_long)?;
                if !all.contains(&copy) {
                    all.push(copy);
                }
                for alias in there.aliases_of(face) {
                    let copy = PersistentName::copy(feature, instance, alias.clone());
                    if copy.check().is_ok() {
                        inherited.push(copy);
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
        Self::listed(
            named(NameRole::Face, body.faces.len()),
            named(NameRole::Edge, body.edges.len()),
            named(NameRole::Vertex, body.vertices.len()),
        )
    }

    /// The names an extrusion (or another operation that reports where
    /// its entities came from) gives the entities of feature `feature`.
    pub fn of_origins(feature: FeatureId, origins: &Origins) -> Self {
        let named = |list: &[Origin]| list.iter().map(|o| name_of(feature, o)).collect();
        Self::listed(
            named(&origins.faces),
            named(&origins.edges),
            named(&origins.vertices),
        )
    }
}

/// `names` as copies in instance `instance` of `feature`, those that can
/// be written.
fn copies(feature: FeatureId, instance: u64, names: Vec<PersistentName>) -> Vec<PersistentName> {
    names
        .into_iter()
        .map(|name| PersistentName::copy(feature, instance, name))
        .filter(|copy| copy.check().is_ok())
        .collect()
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
    /// The designation, as given or as found from the diameter given or
    /// the face's: `M10`.
    pub size: String,
    /// The major and the minor diameter of the basic profile, in model
    /// units.
    pub major: f64,
    pub minor: f64,
    pub pitch: f64,
    /// The pitch was given; otherwise it is the coarse pitch of ISO 261
    /// for the size, looked up at every regeneration.
    pub pitch_given: bool,
    /// How far it goes at full depth.
    pub length: f64,
    pub left_handed: bool,
    /// Only marked on the face; the body is not changed.
    pub cosmetic: bool,
    /// How far the faces of the groove were found to lie from the true
    /// sweep, at most; 0 for a cosmetic thread.
    pub deviation: f64,
    /// An internal thread: the face is a hole (its material is outside
    /// it), tapped; otherwise an external one, on a shank.
    pub internal: bool,
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
    /// For a developable or an unfold that was built: what it made beside
    /// its body (`developable::Developed`).
    pub developed: Option<Developed>,
    /// For an offset of the profiles of a sketch that was built: its
    /// outline.
    pub offset: Option<Arc<OffsetResult>>,
    /// The later feature this feature's body went into. The body is then
    /// no longer one of the document's bodies; it stays here because names
    /// are resolved against it.
    pub consumed_by: Option<FeatureId>,
    /// For a feature whose faces or edges were given by the names of an
    /// earlier body that had gone into later features: the feature as it
    /// was built, its names followed to the newest body (`follow`). `None`
    /// when the feature was built as it stands in the document.
    pub followed: Option<Arc<Feature>>,
    /// For a feature switched off (suppressed or inactive), and for one
    /// that is on but reads an operand that is absent: what it passes on
    /// to the features after it (`pass`). Its `body` and `names` are then
    /// the body passed on, when it is one. `None` for a feature built as
    /// usual, and for the kinds that pass nothing on when off (a sketch, a
    /// datum, an offset of profiles, a split).
    pub passed: Option<Passed>,
}

impl FeatureResult {
    /// True when the result holds what it stands for: the feature was
    /// built (`Ok`, with or without a body), or it is off and passes a
    /// body on. A body of such a result that the cache let go of is
    /// derived again like any other.
    pub fn is_built(&self) -> bool {
        match &self.passed {
            Some(Passed::Body(_)) => true,
            Some(Passed::Nothing(_) | Passed::Broken(_)) => false,
            None => self.status == Status::Ok,
        }
    }

    /// The feature whose body this one passes on unchanged (`pass`): its
    /// body is made from that body alone, in whichever instance its names
    /// are copies in. `None` for a feature built as usual.
    pub fn passes_on(&self) -> Option<FeatureId> {
        match &self.passed {
            Some(Passed::Body(source)) => Some(*source),
            _ => None,
        }
    }

    /// True for a feature switched off, by the user or by its condition.
    pub fn is_off(&self) -> bool {
        matches!(self.status, Status::Suppressed | Status::Inactive)
    }

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
    length_or_number(doc, variables, name, expr).map(|(value, _)| value)
}

/// [`length`], and whether the expression was a length (true) or a plain
/// number (false).
fn length_or_number(
    doc: &Document,
    variables: &Evaluation,
    name: &str,
    expr: &Expr,
) -> Result<(f64, bool), String> {
    let quantity = variables
        .evaluate(expr.source())
        .map_err(|e| format!("{name}: `{}`: {e}", expr.source()))?;
    if quantity.dimension == Dimension::LENGTH {
        Ok((to_document(quantity, doc).value, true))
    } else if quantity.dimension.is_dimensionless() {
        Ok((quantity.value, false))
    } else {
        Err(format!(
            "{name}: `{}` is {}, but a length is needed{}",
            expr.source(),
            quantity.dimension.described(),
            quantity.dimension.unit_hint(expr.source())
        ))
    }
}

/// Names what a feature made from the curves of sketch `sketch` after
/// those curves: the kernel's numbers for curves of nested patterns become
/// their names' numbers (`e3[2][1]`, [`sketch_pattern::CurveIds`]), in
/// every name of `names`; and the caps of `roles` whose loop holds every
/// instance of a curve of a pattern have the run written once, `e7[*]`
/// (`e3[*][1]` for a cell), so the name of the cap does not change with
/// the count.
pub(crate) fn collapse_runs(
    doc: &Document,
    earlier: &[FeatureResult],
    sketch: FeatureId,
    names: &mut [PersistentName],
    roles: &[NameRole],
) {
    let Some(FeatureKind::Sketch(drawn)) = doc.feature(sketch).map(|f| &f.kind) else {
        return;
    };
    if drawn.patterns.is_empty() {
        return;
    }
    let Some(solved) = earlier
        .iter()
        .find(|r| r.id == sketch)
        .and_then(|r| r.sketch.as_ref())
    else {
        return;
    };
    if !solved.curve_ids.is_empty() {
        for name in names.iter_mut() {
            // The link the kernel's numbers are in: under a piece, if any.
            let mut link = name;
            while link.role == NameRole::Piece {
                let Some(next) = link.of.as_mut().map(Arc::make_mut) else {
                    break;
                };
                link = next;
            }
            link.sources = solved.curve_ids.expand(&link.sources);
        }
    }
    if roles.is_empty() {
        return;
    }
    let runs = sketch_pattern::full_runs(drawn, solved);
    for face in names {
        if roles.contains(&face.role)
            && let Some(sources) = sketch_pattern::collapse(&face.sources, &runs)
        {
            face.sources = sources;
        }
    }
}

/// Evaluates a plain number: a count, a number of turns, a condition.
fn number(variables: &Evaluation, name: &str, expr: &Expr) -> Result<f64, String> {
    let quantity = variables
        .evaluate(expr.source())
        .map_err(|e| format!("{name}: `{}`: {e}", expr.source()))?;
    if quantity.dimension.is_dimensionless() {
        Ok(quantity.value)
    } else {
        Err(format!(
            "{name}: `{}` is {}, but a plain number is needed{}",
            expr.source(),
            quantity.dimension.described(),
            quantity.dimension.unit_hint(expr.source())
        ))
    }
}

/// Evaluates an angle, in radians. A bare number is taken in the
/// document's angle unit.
pub(crate) fn angle(
    doc: &Document,
    variables: &Evaluation,
    name: &str,
    expr: &Expr,
) -> Result<f64, String> {
    let quantity = variables
        .evaluate(expr.source())
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
            "{name}: `{}` is {}, but an angle is needed{}",
            expr.source(),
            quantity.dimension.described(),
            quantity.dimension.unit_hint(expr.source())
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
            let axis = derive::direction(doc, variables, "axis", &orientation.axis)?;
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
        FeatureKind::OffsetProfile(_) => return Err("an offset has no body".to_string()),
        FeatureKind::Fill(_) => return Err("a fill needs its sketch".to_string()),
        FeatureKind::Fold(_) => return Err("a fold needs its sketch".to_string()),
        FeatureKind::SheetMetal(_) => {
            return Err("a sheet-metal part needs its sketch".to_string());
        }
        // Built in `features`, which knows the sketch's result.
        FeatureKind::Extrude(_) => return Err("an extrusion needs its sketch".to_string()),
        FeatureKind::Revolve(_) => return Err("a revolution needs its sketch".to_string()),
        FeatureKind::Loft(_) => return Err("a loft needs its sketches".to_string()),
        FeatureKind::Sweep(_) => return Err("a sweep needs its sketches".to_string()),
        FeatureKind::Developable(_) => {
            return Err("a developable needs the curves it runs between".to_string());
        }
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
        | FeatureKind::MoveFace(_)
        | FeatureKind::Thicken(_)
        | FeatureKind::Plate(_)
        | FeatureKind::Draft(_)
        | FeatureKind::Shell(_)
        | FeatureKind::DeleteFace(_)
        | FeatureKind::Thread(_)
        | FeatureKind::Split(_)
        | FeatureKind::Unfold(_) => {
            return Err(format!(
                "a {} needs the body it is made from",
                feature.kind.type_name()
            ));
        }
        FeatureKind::Import(import) => return imported(doc, feature.id, import),
    }
    .map_err(|e| e.to_string())?;

    checked(doc, body)
}

/// Metres per unit of length.
pub fn metres_per_unit(unit: craft3d_doc::LengthUnit) -> f64 {
    use craft3d_doc::LengthUnit;
    match unit {
        LengthUnit::Micrometre => 1e-6,
        LengthUnit::Millimetre => 1e-3,
        LengthUnit::Centimetre => 1e-2,
        LengthUnit::Metre => 1.0,
        LengthUnit::Inch => 0.0254,
        LengthUnit::Foot => 0.3048,
    }
}

/// One solid or sheet of an imported STEP file, read from the bytes the
/// document holds, in the document's unit and checked with its tolerance
/// (`craft3d-stepread`, docs/design/step-import.md). The file is parsed
/// once for all the features that read solids of it.
///
/// A file is not a body this kernel made, so the whole body is looked at
/// for places where it passes through or touches itself, as a loft's is
/// (docs/design/regeneration.md › *Never through itself*).
fn imported(
    doc: &Document,
    feature: FeatureId,
    import: &craft3d_doc::Import,
) -> Result<Body, String> {
    let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    let file = &import.file_name;
    let parsed = craft3d_stepread::parse_shared(import.data.bytes(), &import.data.sha256())
        .map_err(|e| format!("`{file}`: {e}"))?;
    let options = craft3d_stepread::Options {
        metres_per_unit: metres_per_unit(doc.units.length),
        tolerance,
        loosest: doc.tolerance.loosest_linear(),
    };
    let index = usize::try_from(import.solid).unwrap_or(usize::MAX);
    let read = parsed
        .build(index, &options)
        .map_err(|e| format!("`{file}`: {e}"))?;
    let body = checked(doc, read.body)?;
    let names = Names::numbered(feature, &body);
    selfcheck::crossing(&body, &names.faces, tolerance.linear).refused(
        "data",
        "imported body",
        "the file's solid is not a valid solid; correct it where it was made",
    )?;
    Ok(body)
}

/// Never hand on an invalid body (docs/design/kernel.md).
fn checked(doc: &Document, mut body: Body) -> Result<Body, String> {
    let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    // Remembered: a body the operation that made it checked already
    // (every boolean does) is not checked again.
    // Held to the document's cap on any entity's own tolerance.
    let cap = doc.tolerance.loosest_linear();
    match craft3d_topo::check_remembered_capped(&body, &tolerance, cap).first() {
        None => {
            // Kept as long as the document is open, and as it was built
            // its vectors hold the room they grew by: a third of a body
            // (2026-10-19, docs/design/regeneration.md).
            body.vertices.shrink_to_fit();
            body.edges.shrink_to_fit();
            body.faces.shrink_to_fit();
            body.loops.shrink_to_fit();
            body.shells.shrink_to_fit();
            for l in &mut body.loops {
                l.coedges.shrink_to_fit();
            }
            for face in &mut body.faces {
                face.loops.shrink_to_fit();
            }
            for shell in &mut body.shells {
                shell.faces.shrink_to_fit();
            }
            Ok(body)
        }
        Some(defect) => Err(format!("the result is not a valid body: {defect}")),
    }
}

/// Why a feature fails when a name it would give its entities is beyond
/// the caps of a name (`PersistentName::check`). A copy of a copy is one
/// link, so this takes a body that was the tool, or a later instance, of
/// thousands of features in a row.
pub(crate) fn too_long(reason: &str) -> String {
    format!(
        "a name this feature would give cannot be written ({reason}): the body went in as a tool or a later instance too many times in a row"
    )
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
    regenerate_cached_needing(doc, cache, Need::Bodies)
}

/// [`regenerate_cached`], with the bodies `need` asks for derived again
/// where the cache had let go of them to stay within its budget
/// ([`RegenCache::set_body_budget`]): with [`Need::All`] the result is
/// that of [`regenerate`] body for body.
/// The variables as the features are built with them: those of the
/// document, or none when its table is invalid (a hand-edited file), when
/// every feature that uses one fails. What regeneration and every reader
/// that evaluates a feature's expression the same way (`split_plan`) use.
pub(crate) fn feature_variables(doc: &Document) -> Evaluation {
    match doc.variable_table() {
        Ok(table) => table.evaluate(),
        Err(_) => craft3d_expr::VariableTable::new().evaluate(),
    }
}

pub fn regenerate_cached_needing(doc: &Document, cache: &mut RegenCache, need: Need) -> Regen {
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
                features: features(doc, &feature_variables(doc), cache, need),
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
        features: features(doc, &evaluation, cache, need),
    }
}

/// About what `body` takes in memory: its entities, and the control nets
/// and knots of its freeform surfaces and curves. What the cache counts
/// against its budget of bodies ([`RegenCache::set_body_budget`]).
pub fn body_bytes(body: &Body) -> usize {
    use craft3d_geom::{Curve3, Surface};
    use craft3d_math::Bits;
    use std::mem::size_of;
    let mut words = Vec::new();
    let mut bytes = body
        .vertices
        .len()
        .saturating_mul(size_of::<craft3d_topo::Vertex>())
        .saturating_add(
            body.edges
                .len()
                .saturating_mul(size_of::<craft3d_topo::Edge>()),
        )
        .saturating_add(
            body.faces
                .len()
                .saturating_mul(size_of::<craft3d_topo::Face>()),
        )
        .saturating_add(
            body.loops
                .len()
                .saturating_mul(size_of::<craft3d_topo::Loop>()),
        )
        .saturating_add(
            body.shells
                .len()
                .saturating_mul(size_of::<craft3d_topo::Shell>()),
        );
    for l in &body.loops {
        bytes = bytes.saturating_add(
            l.coedges
                .len()
                .saturating_mul(size_of::<craft3d_topo::Coedge>()),
        );
    }
    for shell in &body.shells {
        bytes = bytes.saturating_add(
            shell
                .faces
                .len()
                .saturating_mul(size_of::<craft3d_topo::FaceId>()),
        );
    }
    for face in &body.faces {
        bytes = bytes.saturating_add(
            face.loops
                .len()
                .saturating_mul(size_of::<craft3d_topo::LoopId>()),
        );
        if matches!(face.surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
            words.clear();
            face.surface.push_bits(&mut words);
            bytes = bytes.saturating_add(words.len().saturating_mul(size_of::<u64>()));
        }
    }
    for edge in &body.edges {
        if matches!(edge.curve, Curve3::Nurbs(_)) {
            words.clear();
            edge.curve.push_bits(&mut words);
            bytes = bytes.saturating_add(words.len().saturating_mul(size_of::<u64>()));
        }
    }
    bytes
}

fn features(
    doc: &Document,
    variables: &Evaluation,
    cache: &mut RegenCache,
    need: Need,
) -> Vec<FeatureResult> {
    cache::features(
        doc,
        variables,
        cache,
        need,
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
    let mut developed = None;
    let mut outline = None;
    let mut passed = None;
    let switch = if rolled_back {
        Ok(None)
    } else {
        pass::switch(variables, feature)
    };
    let (status, body) = if rolled_back {
        (Status::RolledBack, None)
    } else if let Err(reason) = &switch {
        (Status::Failed(reason.clone()), None)
    } else if let Ok(Some(off)) = switch {
        // Not built: what it changes goes on unchanged (`pass`).
        let why = pass::off_reason(feature, &off);
        match pass::switched_off(doc, feature, &why, earlier) {
            Some(through) => {
                names = through.names;
                instances = through.instances;
                passed = Some(through.passed);
                (off, through.body)
            }
            None => (off, None),
        }
    } else if let Some(through) = pass::operand_absent(doc, feature, earlier) {
        // On, with an operand that is off: identity on what it changes.
        match through.passed {
            pass::Passed::Broken(reason) => (Status::Failed(reason), None),
            other => {
                names = through.names;
                instances = through.instances;
                passed = Some(other);
                (Status::Ok, through.body)
            }
        }
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
    } else if let FeatureKind::OffsetProfile(offset) = &feature.kind {
        match offset_profile::build(doc, variables, feature.id, offset, earlier) {
            Ok(made) => {
                outline = Some(Arc::new(made));
                (Status::Ok, None)
            }
            Err(reason) => (Status::Failed(reason), None),
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
    } else if let FeatureKind::Extrude(_) | FeatureKind::Revolve(_) | FeatureKind::Fill(_) =
        &feature.kind
    {
        let swept = match &feature.kind {
            FeatureKind::Revolve(revolution) => revolve::build(doc, variables, revolution, earlier),
            FeatureKind::Extrude(extrusion) => extrude::build(doc, variables, extrusion, earlier),
            FeatureKind::Fill(fill) => extrude::fill(doc, fill, earlier),
            _ => Err("not a sweep".to_string()),
        };
        let up_to = matches!(
            &feature.kind,
            FeatureKind::Extrude(e) if matches!(e.end, craft3d_doc::ExtrudeEnd::UpToFace { .. })
        );
        let made = swept
            .and_then(|(body, origins)| checked(doc, body).map(|body| (body, origins)))
            .and_then(|(body, origins)| {
                let mut named = Names::of_origins(feature.id, &origins);
                // A cap whose loop holds a whole run of a sketch pattern
                // is named after the run once (`e7[*]`).
                let sketch = match &feature.kind {
                    FeatureKind::Revolve(r) => Some(r.sketch),
                    FeatureKind::Extrude(e) => Some(e.sketch),
                    FeatureKind::Fill(f) => Some(f.sketch),
                    _ => None,
                };
                if let Some(sketch) = sketch {
                    collapse_runs(
                        doc,
                        earlier,
                        sketch,
                        &mut named.faces,
                        &[NameRole::StartCap, NameRole::EndCap],
                    );
                    // Swept from an offset: named after its curves.
                    let caps = [NameRole::StartCap, NameRole::EndCap];
                    offset_profile::rename(doc, earlier, sketch, &mut named.faces, &caps)?;
                    if let Some((edges, vertices)) = named.listed_mut() {
                        collapse_runs(doc, earlier, sketch, edges, &[]);
                        collapse_runs(doc, earlier, sketch, vertices, &[]);
                        offset_profile::rename(doc, earlier, sketch, edges, &caps)?;
                        offset_profile::rename(doc, earlier, sketch, vertices, &caps)?;
                    }
                }
                if up_to {
                    extrude::not_through_itself(&body, &origins, &named, doc.tolerance.linear)?;
                }
                Ok((body, named))
            });
        match made {
            Ok((body, named)) => {
                names = Some(Arc::new(named));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::SheetMetal(made) = &feature.kind {
        match sheetmetal::build(doc, variables, feature.id, made, earlier).and_then(
            |(body, made_names, part)| checked(doc, body).map(|body| (body, made_names, part)),
        ) {
            Ok((body, made_names, part)) => {
                names = Some(Arc::new(made_names));
                developed = Some(Developed::SheetMetal(part));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::Fold(folding) = &feature.kind {
        match fold::build(doc, variables, feature.id, folding, earlier).and_then(
            |(body, made_names, flat)| checked(doc, body).map(|body| (body, made_names, flat)),
        ) {
            Ok((body, made_names, flat)) => {
                names = Some(Arc::new(made_names));
                developed = flat.map(|flat| Developed::FlatFold(Arc::new(flat)));
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
    } else if let FeatureKind::Developable(sheet) = &feature.kind {
        match developable::build_developable(doc, feature.id, sheet, earlier)
            .and_then(|(body, names, report)| checked(doc, body).map(|body| (body, names, report)))
        {
            Ok((body, made_names, report)) => {
                names = Some(Arc::new(made_names));
                developed = Some(report);
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::Unfold(pattern) = &feature.kind {
        match developable::build_unfold(doc, variables, feature.id, pattern, earlier)
            .and_then(|(body, names, dev)| checked(doc, body).map(|body| (body, names, dev)))
        {
            Ok((body, made_names, dev)) => {
                names = Some(Arc::new(made_names));
                developed = Some(Developed::Pattern(dev));
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
    } else if let FeatureKind::Thicken(layer) = &feature.kind {
        match thicken::build(doc, variables, feature.id, layer, earlier)
            .and_then(|(body, names)| checked(doc, body).map(|body| (body, names)))
        {
            Ok((body, made_names)) => {
                names = made_names.map(Arc::new);
                // Instance 0 lies on or along the faces of the source,
                // 1 under them; neither moves as a whole.
                instances = Some(Arc::new(vec![Transform::IDENTITY; 2]));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::Plate(made) = &feature.kind {
        match plate::build(doc, variables, feature.id, made, earlier)
            .and_then(|(body, names)| checked(doc, body).map(|body| (body, names)))
        {
            Ok((body, made_names)) => {
                names = made_names.map(Arc::new);
                // Instance 0 is the front skin, 1 the back; neither moves
                // as a whole.
                instances = Some(Arc::new(vec![Transform::IDENTITY; 2]));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::Draft(tilted) = &feature.kind {
        match draft::build(doc, variables, feature.id, tilted, earlier)
            .and_then(|(body, names)| checked(doc, body).map(|body| (body, names)))
        {
            Ok((body, made_names)) => {
                names = made_names.map(Arc::new);
                // The body as a whole does not move; a face that turned
                // is found where it lies now (`sketch::resolve_face`).
                instances = Some(Arc::new(vec![Transform::IDENTITY]));
                (Status::Ok, Some(Arc::new(body)))
            }
            Err(reason) => (Status::Failed(reason), None),
        }
    } else if let FeatureKind::MoveFace(moving) = &feature.kind {
        match move_face::build(doc, variables, feature.id, moving, earlier)
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
    } else if let FeatureKind::Split(split) = &feature.kind {
        match split::build(doc, variables, feature, split, earlier)
            .and_then(|(body, names)| checked(doc, body).map(|body| (body, names)))
        {
            Ok((body, made_names)) => {
                names = made_names.map(Arc::new);
                // One instance, the lump where it lies.
                instances = Some(Arc::new(vec![Transform::IDENTITY]));
                taken = Some(split.source);
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
            developed,
            offset: outline,
            consumed_by: None,
            followed: None,
            passed,
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

    /// Regeneration holds every body to the document's cap on any
    /// entity's own tolerance, both ways: a larger cap lets a looser
    /// entity through, a smaller one refuses one the default would take.
    #[test]
    fn bodies_are_held_to_the_documents_cap() {
        let frame = craft3d_math::Frame::WORLD;
        let block = craft3d_kernel::primitives::block(&frame, 1.0, 1.0, 1.0).unwrap();
        let mut doc = Document::new(Units::default());
        // 5e-3 is above the default cap (1e-3 at 1e-6).
        let mut loose = block.clone();
        loose
            .set_edge_tolerance(craft3d_topo::EdgeId(0), 5e-3)
            .unwrap();
        assert!(checked(&doc, loose.clone()).is_err());
        doc.tolerance.loosest = Some(1e-2);
        assert!(checked(&doc, loose).is_ok());
        // 5e-4 is within the default cap, above a cap of 1e-4.
        let mut looser = block;
        looser
            .set_edge_tolerance(craft3d_topo::EdgeId(0), 5e-4)
            .unwrap();
        doc.tolerance.loosest = None;
        assert!(checked(&doc, looser.clone()).is_ok());
        doc.tolerance.loosest = Some(1e-4);
        let refused = checked(&doc, looser).unwrap_err();
        assert!(refused.contains("cap"), "{refused}");
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
            (["0 mm", "0", "0 in"], "0", "which has no length"),
            (["1 deg", "0", "0"], "0", "but a length is needed"),
            // A plain number beside a length would be read in the
            // document's unit: refused, whichever way round.
            (["1", "1 mm", "0"], "0", "mixes lengths and plain numbers"),
            (
                ["0 mm", "0.5", "2 in"],
                "0",
                "mixes lengths and plain numbers",
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

    /// An axis is read as a position is: plain numbers or lengths, from
    /// expressions, so a part can stay square to a wall whose direction
    /// depends on parameters. Only where it points counts.
    #[test]
    fn an_axis_may_be_lengths_or_numbers_from_expressions() {
        let built = |axis: [&str; 3]| {
            let (regen, _) = single(FeatureKind::Cylinder(Cylinder {
                position: exprs(["0", "0", "0"]),
                radius: Expr::new("1"),
                height: Expr::new("4"),
                orientation: turned(axis, "0"),
            }));
            assert!(regen.is_ok(), "{axis:?}: {:?}", regen.features[0].status);
            let body = regen.features[0].body.as_ref().unwrap();
            mass_properties(body).unwrap().centroid.unwrap()
        };
        let along_x = built(["1", "0", "0"]);
        assert!(
            along_x.distance(Vec3::new(2.0, 0.0, 0.0)) < 1e-10,
            "{along_x:?}"
        );
        for axis in [
            ["30 mm", "0 mm", "0"],
            ["3 cm", "0", "0"],
            ["cos(0 deg)", "sin(0 deg)", "0"],
            ["2 * 5 mm", "0", "0 mm"],
        ] {
            let centroid = built(axis);
            assert!(centroid.distance(along_x) < 1e-10, "{axis:?}: {centroid:?}");
        }
        let slanted = built(["3 mm", "4 mm", "0"]);
        assert!(
            slanted.distance(Vec3::new(1.2, 1.6, 0.0)) < 1e-10,
            "{slanted:?}"
        );
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
