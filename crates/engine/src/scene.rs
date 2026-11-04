//! The scene: what a viewer draws for the active document, read-only
//! (docs/design/engine.md, the engine/UI seam).
//!
//! A [`Scene`] is derived data: the tessellated mesh of every body with the
//! feature that made it, every solved sketch as polylines in space, and the
//! feature tree as plain rows. It is rebuilt when the document changes and
//! cached per document and tolerance, so a shell can ask for it every frame.
//! Nothing here can change a document: edits are commands.

use crate::mass::mass_of;
use crate::meshes::{
    BodyFacts, EdgePolylines, Limit, MeshStore, mesh_bytes, points_bytes, text_bytes,
};
use crate::{CommandError, Session, Workspace};
use craft3d_doc::{AngleUnit, Document, Entity, FeatureKind, Geometry, Sketch};
use craft3d_math::BUG;
use craft3d_math::Portable;
use craft3d_math::{Bbox3, Vec3};
use craft3d_regen::{Regen, SketchResult, Status};
use craft3d_tess::{Mesh, TessError, TessParams};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Fewest segments a full circle is drawn with, however coarse the
/// tolerance: a hexagon does not read as a circle.
pub const MIN_CIRCLE_SEGMENTS: usize = 24;

/// Most segments one circle or arc is sampled with. A tolerance that would
/// need more (a chord error below about 3e-7 of the radius) is not met.
pub const MAX_ARC_SEGMENTS: usize = 4096;

/// Scenes kept for one state of a document: one per quality asked for.
const MAX_CACHED: usize = 4;

/// The memory the scenes kept may take by default, in bytes: 1.5 GiB, the
/// scenes of a dozen documents at the display rule's full budget of a
/// million triangles (about 110 MB each), or of hundreds of smaller ones.
pub const DEFAULT_SCENE_BUDGET: usize = 1536 << 20;

/// The quick first picture's chordal tolerance, as a fraction of the
/// diagonal of the box around the model: what a viewport shows at once
/// while the display scene is built ([`Session::set_refine_elsewhere`]),
/// and the coarsest the display rule goes. The box, unlike the model's
/// area, costs nothing to find (measuring the area of a model with spline
/// faces takes seconds).
pub const PREVIEW_RELATIVE_TOLERANCE: f64 = 1e-3;

/// The automatic chordal tolerance, as `file.export` and previews choose
/// it (pictures since 2026-10-15 choose their own,
/// [`Session::picture_scene`]): this fraction of the model's size (the side of a
/// square with the bodies' total area; see [`SceneQuality::Automatic`]).
pub const AUTOMATIC_RELATIVE_TOLERANCE: f64 = 1e-3;

/// The display rule's default chordal tolerance, as a fraction of the
/// model's size: 2e-4, a chord error of a fifth of a pixel on a model that
/// fills a 1020-pixel viewport. Small circles, which a distance alone
/// makes coarse, are kept round by the angle criterion
/// ([`DISPLAY_MAX_ANGLE`]) instead of by a finer distance everywhere
/// (docs/design/tessellator.md, *What a viewer draws*; until 2026-10-14 it
/// was 3e-5 without an angle, three to ten times the triangles).
pub const DISPLAY_RELATIVE_TOLERANCE: f64 = 2e-4;

/// The display rule's default angle criterion, in radians: no chord of an
/// edge turns its tangent, and no side of a triangle turns the surface's
/// normal, by more than 12 degrees (30 chords to a circle), unless it is
/// shorter than the chordal tolerance (`craft3d_tess::ANGLE_FLOOR`). It
/// keeps a small hole round at a chordal tolerance that alone would make
/// it a polygon of a few sides (docs/design/tessellator.md, *What a viewer
/// draws*). The quick first picture has the chordal tolerance alone: it
/// is there to be quick.
pub const DISPLAY_MAX_ANGLE: f64 = DISPLAY_MAX_ANGLE_DEGREES * (std::f64::consts::PI / 180.0);

/// [`DISPLAY_MAX_ANGLE`] in degrees, as `ui.set quality` takes it.
pub const DISPLAY_MAX_ANGLE_DEGREES: f64 = 12.0;

/// The display rule's default triangle budget: the mesh is coarsened
/// until the scene has at most this many triangles, but never beyond the
/// automatic tolerance. A million: the tessellator adds about 1.5 µs per
/// triangle beyond its cost per face (measured 2026-10-13, release, a
/// plate with 1024 holes), so the finer mesh costs at most about 1.5 s
/// per edit, and the GPU viewport draws a million triangles in about
/// 25 ms per frame.
pub const DISPLAY_MAX_TRIANGLES: usize = 1_000_000;

/// Most tessellations the display rule tries before it settles for the
/// automatic tolerance. Every step coarsens the tolerance at least twice,
/// so six steps cover a factor of 64 and the default of 33.
const MAX_DISPLAY_ATTEMPTS: usize = 6;

/// The rungs of the ladder the display rule's and the quick first
/// picture's tolerances stand on ([`on_rung`]): `2^(k/8)` for `k` from 0
/// to 8, so eight rungs to an octave, each 9.05 % coarser than the one
/// below.
const RUNGS: [f64; 9] = [
    1.0,
    1.090_507_732_665_257_7,
    1.189_207_115_002_721,
    1.296_839_554_651_009_6,
    std::f64::consts::SQRT_2,
    1.542_210_825_407_940_7,
    1.681_792_830_507_429,
    1.834_008_086_409_342_4,
    2.0,
];

/// Halfway between the rungs, in the same octave: `2^((k + ½)/8)` for
/// `k` from 0 to 7, what a mantissa is compared with to find its nearest
/// rung.
const BETWEEN_RUNGS: [f64; 8] = [
    1.044_273_782_427_413_8,
    1.138_788_634_756_691_6,
    1.241_857_812_073_484,
    1.354_255_546_936_892_7,
    1.476_826_145_939_499_3,
    1.610_490_331_949_254_3,
    1.756_252_160_373_299_5,
    1.915_206_561_397_147_4,
];

/// The rung of the ladder nearest `tolerance`: `2^(k/8)` document units
/// for the integer `k` that comes nearest, within 4.5 % of `tolerance`
/// either way (the rungs are 9.05 % apart). A tolerance that follows the
/// model's size (the display rule's, the quick first picture's) moves a
/// little with every edit: on the ladder it stands still until the size
/// has moved by about a rung, and the meshes of the bodies the edit did
/// not touch are kept (`meshes.rs`) instead of being made again at a
/// tolerance a tenth of a percent away. Found from the exponent and the
/// mantissa of the number, without `log2` or `exp2`: the same bits on
/// every platform. A tolerance that is not a positive finite number, or
/// is subnormal, is returned as it is.
pub(crate) fn on_rung(tolerance: f64) -> f64 {
    if !(tolerance.is_finite() && tolerance > 0.0) {
        return tolerance;
    }
    let bits = tolerance.to_bits();
    let biased = (bits >> 52) & 0x7ff;
    if biased == 0 {
        return tolerance;
    }
    // The mantissa as a number in [1, 2): the same bits under the
    // exponent of one.
    let mantissa = f64::from_bits((bits & ((1u64 << 52) - 1)) | (1023u64 << 52));
    let k = BETWEEN_RUNGS
        .iter()
        .position(|&between| mantissa < between)
        .unwrap_or(BETWEEN_RUNGS.len());
    let Some(&rung) = RUNGS.get(k) else {
        return tolerance;
    };
    // Times the tolerance's power of two: exact, as the product of a
    // number and a power of two is, unless it leaves the normal range.
    let scale = f64::from_bits(biased << 52);
    let on = rung * scale;
    if on.is_finite() && on > 0.0 && on.to_bits() >> 52 & 0x7ff != 0 {
        on
    } else {
        tolerance
    }
}

/// How finely the bodies and the sketch curves of a scene are sampled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SceneQuality {
    /// The chordal tolerance `file.export` and previews use when none is
    /// given: [`AUTOMATIC_RELATIVE_TOLERANCE`] of the model's size. Without
    /// bodies, the same fraction of the size of the sketches. (Pictures
    /// have their own since 2026-10-15: [`Session::picture_scene`].)
    Automatic,
    /// A chordal tolerance in document units.
    Tolerance(f64),
    /// What a viewport shows: a chordal tolerance of `relative` times the
    /// model's size, coarsened until the scene has at most `max_triangles`
    /// triangles and every body is within what the tessellator can do, but
    /// never beyond the automatic tolerance (the display is never coarser
    /// than `view.render`). The scene says whether it was coarsened
    /// ([`Scene::coarsened`]). With `max_angle` (radians), the angle
    /// criterion of the tessellator beside the chordal tolerance
    /// (`TessParams::max_angle`): [`DISPLAY_MAX_ANGLE`] by default.
    Display {
        relative: f64,
        max_triangles: usize,
        max_angle: Option<f64>,
    },
    /// What a picture is drawn from without a tolerance of its own
    /// (`view.render`, `view.pick`, `view.replay`;
    /// [`Session::picture_scene`] chooses the tolerance): this chordal
    /// tolerance in document units with the display's angle
    /// ([`DISPLAY_MAX_ANGLE`]), coarsened by the display rule's ladder
    /// until the scene has at most [`DISPLAY_MAX_TRIANGLES`] triangles and
    /// every body is within what the tessellator can do
    /// ([`Scene::coarsened`] says so). A document state keeps one such
    /// scene, the latest.
    Picture(f64),
    /// The quick first picture the display rule starts from and a viewer
    /// shows while the display scene is built elsewhere:
    /// [`PREVIEW_RELATIVE_TOLERANCE`] of the diagonal of the model's box
    /// ([`Scene::preview`] is set).
    Preview,
}

impl SceneQuality {
    /// The display rule with its defaults.
    pub fn display() -> Self {
        SceneQuality::Display {
            relative: DISPLAY_RELATIVE_TOLERANCE,
            max_triangles: DISPLAY_MAX_TRIANGLES,
            max_angle: Some(DISPLAY_MAX_ANGLE),
        }
    }

    /// Refuses a quality that is not a request at all: a tolerance or a
    /// fraction that is not a positive number, a budget of nothing.
    fn validate(self) -> Result<(), CommandError> {
        match self {
            SceneQuality::Automatic | SceneQuality::Preview => Ok(()),
            SceneQuality::Tolerance(t) if t.is_finite() && t > 0.0 => Ok(()),
            SceneQuality::Tolerance(_) => Err(CommandError::new(
                "the scene's tolerance must be a positive number",
            )),
            SceneQuality::Picture(t) if t.is_finite() && t > 0.0 => Ok(()),
            SceneQuality::Picture(_) => Err(CommandError::new(
                "the picture's tolerance must be a positive number",
            )),
            SceneQuality::Display { relative, .. } if !(relative.is_finite() && relative > 0.0) => {
                Err(CommandError::new(
                    "the display quality's chordal tolerance must be a positive fraction of the model's size",
                ))
            }
            SceneQuality::Display {
                max_triangles: 0, ..
            } => Err(CommandError::new(
                "the display quality's triangle budget must be at least 1",
            )),
            SceneQuality::Display {
                max_angle: Some(angle),
                ..
            } if !(angle.is_finite() && angle > 0.0 && angle <= std::f64::consts::TAU) => Err(
                CommandError::new("the display quality's angle must be above 0 and at most a turn"),
            ),
            SceneQuality::Display { .. } => Ok(()),
        }
    }

    /// What the cache is keyed by.
    fn key(self) -> SceneKey {
        match self {
            SceneQuality::Automatic => SceneKey::Automatic,
            SceneQuality::Preview => SceneKey::Preview,
            SceneQuality::Tolerance(t) => SceneKey::Tolerance(t.to_bits()),
            SceneQuality::Picture(t) => SceneKey::Picture(t.to_bits()),
            SceneQuality::Display {
                relative,
                max_triangles,
                max_angle,
            } => SceneKey::Display {
                relative: relative.to_bits(),
                max_triangles,
                max_angle: max_angle.map(f64::to_bits),
            },
        }
    }
}

/// A [`SceneQuality`] with its numbers as bits, so it can be compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SceneKey {
    Automatic,
    Tolerance(u64),
    Display {
        relative: u64,
        max_triangles: usize,
        max_angle: Option<u64>,
    },
    /// The quick first picture: one per document state, whatever display
    /// quality it stands in for.
    Preview,
    /// A picture's scene at its tolerance ([`Session::picture_scene`]):
    /// one per document state, the latest asked for.
    Picture(u64),
}

/// One body of the scene.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneBody {
    /// The feature that made the body, as `document.inspect` writes it
    /// (`F3`).
    pub feature: String,
    pub name: String,
    /// Shared with the scenes of other states of the document that have
    /// the body, and with the session's store of meshes (`meshes.rs`): a
    /// scene rebuilt after an edit meshes only the bodies that changed.
    pub mesh: Arc<Mesh>,
    /// The body's face ids in [`Scene::mesh`] start here.
    pub first_face_id: u32,
    /// The body's edges as polylines in space, one per edge in the order
    /// of the B-rep, sampled with the scene's tolerance. They are the
    /// points the mesh has along each edge, so a line drawn through them
    /// lies on the border between the faces it separates. Shared like
    /// [`Self::mesh`].
    pub edges: Arc<EdgePolylines>,
    /// The names of the body's faces, edges and vertices, which faces
    /// each edge lies between and which meet in each vertex, and its
    /// vertices: worked out once per body and shared like
    /// [`Self::mesh`]. Face `k` of the body has id `first_face_id + k`
    /// in [`Scene::mesh`].
    pub facts: Arc<BodyFacts>,
    /// For a body placed as a rigid copy of another (an assembly's
    /// instance at its pose, or at a moment of its motion): the mesh of
    /// the body it was copied from and the rigid motion that takes that
    /// mesh to [`Self::mesh`]. A viewer that has uploaded the source's
    /// mesh moves it instead of uploading the moved one, so a moment of a
    /// motion sends the renderer a few matrices, not new geometry. `None`
    /// for a body meshed itself.
    pub placement: Option<ScenePlacement>,
}

/// Where a placed body's mesh comes from ([`SceneBody::placement`]).
#[derive(Clone, Debug, PartialEq)]
pub struct ScenePlacement {
    /// The mesh of the body it was placed from, shared by every moment
    /// that places that body.
    pub source: Arc<Mesh>,
    /// The rigid motion (no mirror) from `source` to the body's mesh.
    pub pose: craft3d_math::Transform,
}

impl std::ops::Deref for SceneBody {
    type Target = BodyFacts;

    /// A body's facts read as its own fields: `body.face_names`,
    /// `body.edge_faces`, `body.vertices`, …
    fn deref(&self) -> &BodyFacts {
        &self.facts
    }
}

/// What a polyline of a sketch stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchCurve {
    Line,
    Circle,
    Arc,
    Spline,
}

/// One line, circle, arc or spline of a sketch, as points in space. A
/// circle is closed: its last point repeats the first.
#[derive(Clone, Debug, PartialEq)]
pub struct ScenePolyline {
    /// The entity, as `sketch.inspect` writes it (`e7`).
    pub entity: String,
    pub curve: SketchCurve,
    /// Construction geometry: a guide, not part of a profile.
    pub construction: bool,
    pub points: Vec<Vec3>,
}

/// One solved sketch, placed in space.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneSketch {
    pub feature: String,
    pub name: String,
    /// The sketch frame: a point of the sketch at (x, y) is at
    /// `origin + x_axis * x + y_axis * y`.
    pub origin: Vec3,
    pub x_axis: Vec3,
    pub y_axis: Vec3,
    pub normal: Vec3,
    /// In the order of the sketch's entities.
    pub polylines: Vec<ScenePolyline>,
    /// Every point entity, in space.
    pub points: Vec<Vec3>,
}

/// Where a datum is drawn: a plane as a square, an axis as a segment, both
/// sized to the model, and a point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SceneDatumShape {
    /// The square `center ± u ± v` in the datum plane: centred on the foot
    /// of the model's centre on the plane, `u` and `v` along the plane's
    /// own axes, each [`DATUM_PLANE_EXTENT`] of the model's diagonal long.
    ///
    /// `origin` and `normal` are the plane as regeneration placed it (the
    /// datum's own origin and its unit normal, bit for bit): what a
    /// section on the datum plane lies on when the bodies are not at
    /// hand (`Session::section_plane` under regeneration elsewhere).
    Plane {
        center: Vec3,
        u: Vec3,
        v: Vec3,
        origin: Vec3,
        normal: Vec3,
    },
    /// The part of the axis around the foot of the model's centre on it,
    /// [`DATUM_AXIS_EXTENT`] of the model's diagonal to either side.
    Axis {
        from: Vec3,
        to: Vec3,
    },
    Point(Vec3),
}

/// A datum plane is drawn as a square whose half side is this fraction of
/// the model's diagonal: a square a little larger than the model's longest
/// side for a flat part, which marks the plane without hiding the model
/// behind a tint.
pub const DATUM_PLANE_EXTENT: f64 = 0.4;

/// A datum axis is drawn as a segment whose half length is this fraction
/// of the model's diagonal: more than half, so it runs through the model's
/// extent wherever the model lies along it.
pub const DATUM_AXIS_EXTENT: f64 = 0.6;

/// A datum that is placed (built, not suppressed, not rolled back), as a
/// viewer draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneDatum {
    pub feature: String,
    pub name: String,
    pub shape: SceneDatumShape,
}

/// How a feature came out of regeneration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureState {
    Ok,
    Suppressed,
    /// Switched off by its condition (`when` is 0).
    Inactive,
    RolledBack,
    Failed,
}

impl FeatureState {
    /// The word `document.inspect` uses.
    pub fn name(self) -> &'static str {
        match self {
            FeatureState::Ok => "ok",
            FeatureState::Suppressed => "suppressed",
            FeatureState::Inactive => "inactive",
            FeatureState::RolledBack => "rolledBack",
            FeatureState::Failed => "failed",
        }
    }
}

/// One row of the feature tree.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneFeature {
    pub id: String,
    pub name: String,
    /// `block`, `cylinder`, `sketch`, …
    pub kind: &'static str,
    pub state: FeatureState,
    /// Why the feature failed.
    pub error: Option<String>,
    /// For a sketch: the degrees of freedom left, or `None` when it could
    /// not be solved.
    pub dof: Option<usize>,
    pub is_sketch: bool,
    /// When it failed because something it is made from failed: the id of
    /// the feature at the start of that chain (`F3`), the one to fix, as
    /// `document.inspect` gives it under `causedBy`.
    pub caused_by: Option<String>,
    /// Hidden in the view (`feature.visible`): its body, sketch or datum
    /// is not in the scene, though it is built.
    pub hidden: bool,
    /// Built, with something of its own to draw: a body that no later
    /// feature took, a sketch, a datum. What showing a hidden feature
    /// brings into view; a viewer offers to fly to it by this.
    pub draws: bool,
    /// The condition that switches it on (`feature.when`), as written.
    pub when: Option<String>,
    /// What it passes on instead of a body of its own and what it leaves
    /// out, in a sentence ("Passes on the body of Joined unchanged."):
    /// for a feature that is off, or on with nothing (or not everything)
    /// to work on. `None` for one built as usual.
    pub passes: Option<String>,
}

/// Everything a viewer draws for one state of the active document.
///
/// The bulky parts (the meshes) are shared by `Arc`, so a copy of a scene
/// under another revision, as a document that becomes active again gets,
/// costs nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// See [`Session::scene_revision`].
    pub revision: u64,
    /// The chordal tolerance the bodies and the sketch curves were sampled
    /// with, in document units.
    pub tolerance: f64,
    /// The angle criterion the bodies were meshed with besides the
    /// tolerance (radians), if any: the display rule's.
    pub max_angle: Option<f64>,
    /// True when the display rule ([`SceneQuality::Display`]) had to
    /// coarsen its tolerance to stay within its triangle budget or within
    /// what the tessellator can do, when a body of the quick first
    /// picture was coarsened to stay within [`DISPLAY_MAX_TRIANGLES`] on
    /// its own, or when a body of any scene was too fine for the
    /// tessellator at the tolerance asked for and was meshed coarser (the
    /// tolerance is then the one asked for; the body's is coarser).
    pub coarsened: bool,
    /// True for the quick first picture ([`PREVIEW_RELATIVE_TOLERANCE`])
    /// that stands in for the display scene while that is built elsewhere
    /// ([`Session::set_refine_elsewhere`]): a viewer says it is refining.
    pub preview: bool,
    /// In history order.
    pub bodies: Arc<Vec<SceneBody>>,
    /// All bodies as one mesh for a renderer; face ids keep counting across
    /// bodies, as in `view.render`.
    pub mesh: Arc<Mesh>,
    /// The faces of [`Scene::mesh`] that have a colour of their own (a
    /// body's appearance, `body.appearance`); the others are drawn in the
    /// theme's colour.
    pub face_colors: Arc<craft3d_render::FaceColors>,
    pub sketches: Vec<SceneSketch>,
    /// The datums that are placed, in history order. They are not in
    /// [`Scene::bbox`]: they are sized to it.
    pub datums: Vec<SceneDatum>,
    pub features: Vec<SceneFeature>,
    /// Bodies that could not be tessellated, as sentences.
    pub problems: Vec<String>,
    /// What the window says without the picture being wrong, as
    /// sentences: a face colour whose face is gone (it colours nothing).
    /// A picture is drawn all the same (`view.render`).
    pub notices: Vec<String>,
    /// The box around the meshes and the sketch geometry.
    pub bbox: Bbox3,
    /// The box around the bodies from their B-rep, whatever the tolerance:
    /// what pictures are framed on (`view.render`, `view.pick`), so that
    /// the camera and the picture's tolerance do not depend on the mesh.
    pub bodies_box: Bbox3,
    /// The box a window frames the model on ([`Scene::frame_box`]).
    pub frame: Bbox3,
}

impl Scene {
    /// The box a window frames the model on ([`frame_of`]): the box
    /// around the sketches as drawn when there is none (a scene read
    /// from an older form).
    pub fn frame_box(&self) -> Bbox3 {
        if self.frame.is_empty() {
            self.bbox
        } else {
            self.frame
        }
    }

    /// How the bodies were sampled: the chordal tolerance and the angle
    /// criterion. Two scenes of one revision with the same sampling have
    /// the same meshes; a viewer that keeps what it uploaded keys it by
    /// this.
    pub fn sampling(&self) -> (f64, Option<f64>) {
        (self.tolerance, self.max_angle)
    }

    pub fn triangles(&self) -> usize {
        self.mesh.triangles.len()
    }

    /// About how many bytes the scene holds: its meshes, edges, points and
    /// names. What the memory budget of the scenes kept counts
    /// ([`Session::set_scene_budget`]).
    pub fn bytes(&self) -> usize {
        let (points, text) = (points_bytes, text_bytes);
        let mut bytes = std::mem::size_of::<Scene>().saturating_add(mesh_bytes(&self.mesh));
        for body in self.bodies.iter() {
            bytes = bytes
                .saturating_add(mesh_bytes(&body.mesh))
                .saturating_add(points(&body.edges))
                .saturating_add(
                    body.vertices
                        .len()
                        .saturating_mul(std::mem::size_of::<Vec3>()),
                )
                .saturating_add(text(&body.face_names))
                .saturating_add(text(&body.edge_names))
                .saturating_add(text(&body.vertex_names));
        }
        for sketch in &self.sketches {
            for polyline in &sketch.polylines {
                bytes = bytes.saturating_add(
                    polyline
                        .points
                        .len()
                        .saturating_mul(std::mem::size_of::<Vec3>()),
                );
            }
        }
        bytes
    }
}

/// The active document as a status bar shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentStatus {
    /// Its index among the open documents.
    pub index: usize,
    /// Which open document it is, within the process: never reused, and
    /// kept while documents before it close and its index moves. A
    /// window keys what it remembers per document (the view of a tab) by
    /// it.
    pub id: u64,
    /// How many documents are open.
    pub open: usize,
    /// The path it was opened from or last saved to.
    pub path: Option<String>,
    /// That file on disk, as an absolute path, whichever root `path` is
    /// relative to; `None` for a document in memory (the browser), a new
    /// one, and a recovered one until it is saved. What the window's list
    /// of recent files keeps.
    pub file: Option<String>,
    /// True when it has changes that are not on disk.
    pub dirty: bool,
    /// True when `file.close` refuses to close it without `discard`:
    /// unsaved changes worth keeping (it has a file, or features).
    pub close_asks: bool,
    pub length_unit: &'static str,
    pub angle_unit: &'static str,
    /// A published Jig opened to be used (`.3dcraftjig`): only its
    /// parameters change, and Save saves it as a model.
    pub jig_in_use: bool,
    /// The name of the step `edit.undo` would undo (`Set teeth`), `None`
    /// when there is nothing to undo: what an Edit menu says.
    pub undo: Option<String>,
    /// The name of the step `edit.redo` would redo, `None` when there is
    /// nothing to redo.
    pub redo: Option<String>,
}

/// A scene, or why it could not be built.
type Built = Result<Arc<Scene>, CommandError>;

/// The scenes of one state of a document, by the quality asked for, with
/// the bytes each holds ([`Scene::bytes`]).
type Scenes = Vec<(SceneKey, Built, usize)>;

/// The scenes of one state of a document.
#[derive(Clone)]
struct Kept {
    /// Holding the `Arc` keeps its address from being reused by another
    /// document.
    doc: Arc<Document>,
    scenes: Scenes,
    /// When this state was last the active one ([`SceneCache::tick`]):
    /// the least recently shown go first when the budget is exceeded.
    shown: u64,
}

impl Kept {
    fn bytes(&self) -> usize {
        self.scenes
            .iter()
            .fold(0usize, |n, (_, _, bytes)| n.saturating_add(*bytes))
    }
}

/// The scenes of every open document (and of the earlier states of each
/// that its undo history still holds), within a memory budget: making a
/// document active again (`file.activate`, a tab), or undoing an edit,
/// shows its scenes again instead of meshing it anew.
#[derive(Clone)]
pub(crate) struct SceneCache {
    /// The document the revision belongs to.
    seen: Option<Arc<Document>>,
    revision: u64,
    /// Keyed by the document state (the `Arc`), and within it by the
    /// quality that was asked for. A scene that could not be built is
    /// remembered as its error, so a viewer asking every frame does not
    /// build it every frame.
    kept: Vec<Kept>,
    /// Counts the changes of the active document, for [`Kept::shown`].
    tick: u64,
    /// Most bytes of scenes kept ([`DEFAULT_SCENE_BUDGET`]). The active
    /// document's scenes are kept whatever they hold.
    budget: usize,
    /// See [`Session::set_refine_elsewhere`].
    pub(crate) refine_elsewhere: bool,
    /// The active document as it is, while its view is scrubbed to an
    /// earlier state (`view.scrub`): its scenes are kept like the shown
    /// state's, so going back to the end shows them at once.
    pinned: Option<Arc<Document>>,
    /// The meshes of the bodies, by body and sampling, shared by every
    /// scene built for the session (`meshes.rs`): a scene rebuilt after
    /// an edit meshes only the bodies the edit changed. Within the same
    /// budget as the scenes.
    pub(crate) meshes: MeshStore,
}

impl Default for SceneCache {
    fn default() -> Self {
        Self {
            seen: None,
            revision: 0,
            kept: Vec::new(),
            tick: 0,
            budget: DEFAULT_SCENE_BUDGET,
            refine_elsewhere: false,
            pinned: None,
            meshes: MeshStore::new(DEFAULT_SCENE_BUDGET),
        }
    }
}

impl SceneCache {
    /// Bumps the revision when `current` is not the document seen last.
    fn observe(&mut self, current: Option<&Arc<Document>>) -> u64 {
        let same = match (&self.seen, current) {
            (None, None) => true,
            (Some(seen), Some(current)) => Arc::ptr_eq(seen, current),
            _ => false,
        };
        if !same {
            self.seen = current.cloned();
            self.revision = self.revision.saturating_add(1);
            self.tick = self.tick.saturating_add(1);
            let tick = self.tick;
            if let Some(kept) = self.active_mut() {
                kept.shown = tick;
            }
            self.meshes.forget_gone();
        }
        // States nobody else holds any more (a closed document, an edit
        // undone and then overwritten) cannot be shown again. Checked
        // every time: a few counts per state kept.
        let seen = &self.seen;
        self.kept.retain(|kept| {
            let ours = 1 + usize::from(seen.as_ref().is_some_and(|s| Arc::ptr_eq(s, &kept.doc)));
            Arc::strong_count(&kept.doc) > ours
        });
        self.revision
    }

    fn active_mut(&mut self) -> Option<&mut Kept> {
        let seen = self.seen.as_ref()?;
        self.kept
            .iter_mut()
            .find(|kept| Arc::ptr_eq(&kept.doc, seen))
    }

    /// The scene of the active document for `key`, under the current
    /// revision.
    fn lookup(&mut self, key: SceneKey) -> Option<Built> {
        let revision = self.revision;
        let kept = self.active_mut()?;
        let (_, built, _) = kept.scenes.iter_mut().find(|(k, ..)| *k == key)?;
        if let Ok(scene) = built
            && scene.revision != revision
        {
            // The same scene under the revision it is shown at now: the
            // meshes are shared, not copied.
            *scene = Arc::new(Scene {
                revision,
                ..(**scene).clone()
            });
        }
        Some(built.clone())
    }

    /// True when the active document has a scene (or an error) for `key`.
    fn has(&mut self, key: SceneKey) -> bool {
        self.active_mut()
            .is_some_and(|kept| kept.scenes.iter().any(|(k, ..)| *k == key))
    }

    /// Keeps `built` for `doc`, then lets the least recently shown states
    /// go until the scenes kept are within the budget.
    fn store(&mut self, doc: &Arc<Document>, key: SceneKey, built: Built) {
        let bytes = built.as_ref().map_or(0, |scene| scene.bytes());
        let at = match self.kept.iter().position(|k| Arc::ptr_eq(&k.doc, doc)) {
            Some(at) => at,
            None => {
                self.kept.push(Kept {
                    doc: Arc::clone(doc),
                    scenes: Vec::new(),
                    shown: self.tick,
                });
                self.kept.len().saturating_sub(1)
            }
        };
        if let Some(kept) = self.kept.get_mut(at) {
            // A state keeps one picture's scene, the latest: pictures of
            // other sizes do not push the viewer's scenes out.
            let picture = matches!(key, SceneKey::Picture(_));
            kept.scenes
                .retain(|(k, ..)| *k != key && !(picture && matches!(k, SceneKey::Picture(_))));
            if kept.scenes.len() >= MAX_CACHED {
                // The oldest goes, whether it was a scene or a failure;
                // the oldest that is not the viewer's (its display scene
                // and its quick first picture) when there is one, so that
                // pictures of other tolerances do not push those out.
                let viewer =
                    |k: &SceneKey| matches!(k, SceneKey::Display { .. } | SceneKey::Preview);
                let oldest = kept
                    .scenes
                    .iter()
                    .position(|(k, ..)| !viewer(k))
                    .unwrap_or(0);
                if oldest < kept.scenes.len() {
                    drop(kept.scenes.remove(oldest));
                }
            }
            kept.scenes.push((key, built, bytes));
        }
        self.evict();
    }

    /// Lets whole states of documents go, least recently shown first,
    /// until the scenes kept are within the budget. The active document's
    /// are never let go.
    fn evict(&mut self) {
        let mut total = self
            .kept
            .iter()
            .fold(0usize, |n, kept| n.saturating_add(kept.bytes()));
        while total > self.budget {
            let seen = self.seen.clone();
            let pinned = self.pinned.clone();
            let oldest = self
                .kept
                .iter()
                .enumerate()
                .filter(|(_, kept)| !seen.as_ref().is_some_and(|s| Arc::ptr_eq(s, &kept.doc)))
                .filter(|(_, kept)| !pinned.as_ref().is_some_and(|p| Arc::ptr_eq(p, &kept.doc)))
                .min_by_key(|(_, kept)| kept.shown)
                .map(|(at, _)| at);
            let Some(at) = oldest else {
                break;
            };
            let gone = self.kept.remove(at);
            total = total.saturating_sub(gone.bytes());
        }
    }

    fn bytes(&self) -> usize {
        self.kept
            .iter()
            .fold(0usize, |n, kept| n.saturating_add(kept.bytes()))
    }
}

/// The size of the model that the relative tolerances are fractions of:
/// the side of a square with the bodies' total area. Without bodies, the
/// diagonal of the box around the sketches' points and circles; without
/// those, one unit.
fn model_size(regen: &Regen, doc: &Document) -> f64 {
    // A body placed as a rigid copy (an assembly's instance) has its
    // source's area: the source is measured, once, not each moment's copy.
    let measured: Vec<std::sync::Arc<craft3d_topo::Body>> = regen
        .bodies()
        .map(|(_, body)| crate::meshes::measured_as(body))
        .collect();
    let bodies: Vec<&std::sync::Arc<craft3d_topo::Body>> = measured.iter().collect();
    crate::mass::measure_ahead(&bodies);
    let area: f64 = measured
        .iter()
        .filter_map(|body| mass_of(body).ok())
        .map(|m| m.area)
        .sum();
    if area.is_finite() && area > 0.0 {
        return area.sqrt();
    }
    let mut extent = Bbox3::EMPTY;
    for (feature, solved) in sketches(regen, doc) {
        let FeatureKind::Sketch(sketch) = &feature.kind else {
            continue;
        };
        for entity in &sketch.entities {
            match entity.geometry {
                Geometry::Point { .. } | Geometry::Image { .. } => {
                    if let Some((x, y)) = solved.point(entity.id) {
                        extent.add(solved.to_world(x, y));
                    }
                }
                Geometry::Circle { center, .. } => {
                    if let (Some((x, y)), Some(r)) =
                        (solved.point(center), solved.radius(entity.id))
                    {
                        extent.add(solved.to_world(x - r, y - r));
                        extent.add(solved.to_world(x + r, y + r));
                    }
                }
                // The control points of a spline are points of the sketch,
                // counted above, and hold the curve between them.
                Geometry::Line { .. } | Geometry::Arc { .. } | Geometry::Spline { .. } => {}
                // A fitted curve lies within its control points.
                Geometry::Formula { .. } | Geometry::Interpolated { .. } => {
                    for p in solved
                        .curve(entity.id)
                        .map_or(&[][..], |c| &c.spline.control)
                    {
                        extent.add(solved.to_world(p[0], p[1]));
                    }
                }
                // A text lies within the box of its curves' control points.
                Geometry::Text(_) => {
                    if let Some([x0, y0, x1, y1]) = solved.text(entity.id).and_then(|t| t.ink) {
                        extent.add(solved.to_world(x0, y0));
                        extent.add(solved.to_world(x1, y1));
                    }
                }
            }
        }
    }
    let size = extent.diagonal();
    if size.is_finite() && size > 0.0 {
        size
    } else {
        1.0
    }
}

/// The size the quick first picture's tolerance is a fraction of: the
/// diagonal of the box around the bodies ([`bodies_box`], which is found
/// from the geometry without measuring anything), or without bodies the
/// size [`model_size`] gives (from the sketches, which costs nothing).
fn preview_size(regen: &Regen, doc: &Document) -> f64 {
    let extent = bodies_box(doc, regen);
    let size = extent.diagonal();
    if size.is_finite() && size > 0.0 {
        size
    } else {
        model_size(regen, doc)
    }
}

/// The share of a pixel a picture's chordal tolerance is when the pixel
/// is the coarser of the two ([`picture_tolerance`]): a chord error of half
/// a pixel cannot be seen.
pub const PICTURE_PIXEL_SHARE: f64 = 0.5;

/// `frame` with its corners on a grid of a power of two near a billionth
/// of its size. Cosmetic: the B-rep box of a cylinder or a sphere reaches
/// its round extremes to within a few units in the last place
/// (19.999999999999574 for 20), which a window would show in its camera's
/// numbers; on the grid it is the round number. Scaling by a power of two
/// and rounding are exact, so the grid moves nothing else.
fn snapped(frame: Bbox3) -> Bbox3 {
    let Some((low, high)) = frame.corners() else {
        return frame;
    };
    let fine = frame.diagonal() * 1e-9;
    if !(fine.is_finite() && fine > f64::MIN_POSITIVE) {
        return frame;
    }
    // The largest power of two not above it: its sign and exponent bits.
    let step = f64::from_bits(fine.to_bits() & 0xFFF0_0000_0000_0000);
    let snap = |v: f64| {
        let on = (v / step).round() * step;
        if on.is_finite() { on } else { v }
    };
    let snap3 = |p: Vec3| Vec3::new(snap(p.x), snap(p.y), snap(p.z));
    Bbox3::from_corners(snap3(low), snap3(high))
}

/// The box a window frames the model on, the same at every sampling of
/// the scene where it can be: each body's box from its B-rep (exact for
/// planes, cylinders, cones, spheres and tori, `Body::held_bbox`), and,
/// for a body with a freeform face (whose B-rep box is its surfaces'
/// control nets, much larger than a trimmed face) or whose box cannot be
/// found, its mesh's box widened by the scene's chordal tolerance (that
/// body alone may move the camera between samplings); with every sketch
/// and helix sampled at a tolerance of the model's own size (not the
/// scene's), widened by it, so a circle or an arc counts whole.
fn frame_of(doc: &Document, regen: &Regen, scene: &Scene) -> Bbox3 {
    let mut frame = Bbox3::EMPTY;
    let take = |frame: &mut Bbox3, b: Bbox3, pad: f64| {
        if let Some((low, high)) = b.corners() {
            let pad = if pad.is_finite() && pad > 0.0 {
                pad
            } else {
                0.0
            };
            frame.add(low - Vec3::new(pad, pad, pad));
            frame.add(high + Vec3::new(pad, pad, pad));
        }
    };
    for (id, body) in regen.bodies().filter(|(id, _)| !doc.is_hidden(*id)) {
        let freeform = body.faces.iter().any(|f| f.surface.is_freeform());
        match held_box(body).filter(|_| !freeform) {
            Some(held) => take(&mut frame, held, 0.0),
            None => {
                let name = id.to_string();
                if let Some(drawn) = scene.bodies.iter().find(|b| b.feature == name) {
                    take(&mut frame, drawn.mesh.bbox(), scene.tolerance);
                }
            }
        }
    }
    let size = if scene.bodies_box.is_empty() {
        doc.tolerance.linear * 1e6
    } else {
        scene.bodies_box.diagonal()
    };
    let sampling = size * 1e-3;
    if sampling.is_finite() && sampling > 0.0 {
        for feature in doc.features.iter().filter(|f| !doc.is_hidden(f.id)) {
            let Some(outcome) = regen.feature(feature.id) else {
                continue;
            };
            let drawn = match (&feature.kind, outcome.sketch.as_deref(), &outcome.status) {
                (FeatureKind::Sketch(sketch), Some(solved), Status::Ok | Status::Failed(_)) => {
                    Some(scene_sketch(feature, sketch, solved, sampling))
                }
                _ => match (outcome.datum, &outcome.status) {
                    (Some(craft3d_regen::Datum::Helix(helix)), Status::Ok) => {
                        scene_helix(feature, &helix, sampling)
                    }
                    _ => None,
                },
            };
            if let Some(drawn) = drawn {
                // A line's ends and the points are exact; a curve's samples
                // lie on it, and it reaches past them by the sampling at
                // most.
                for polyline in &drawn.polylines {
                    let points = Bbox3::from_points(polyline.points.iter().copied());
                    let pad = if polyline.curve == SketchCurve::Line {
                        0.0
                    } else {
                        sampling
                    };
                    take(&mut frame, points, pad);
                }
                take(
                    &mut frame,
                    Bbox3::from_points(drawn.points.iter().copied()),
                    0.0,
                );
            }
        }
    }
    if frame.is_empty() {
        return frame;
    }
    snapped(frame)
}

/// The box around the bodies of `regen` that are not hidden in the view
/// (`feature.visible`), from their B-rep (a face on a
/// sphere or torus by what it holds, `Body::held_bbox`): empty without
/// bodies.
pub(crate) fn bodies_box(doc: &Document, regen: &Regen) -> Bbox3 {
    let mut extent = Bbox3::EMPTY;
    for (_, body) in regen.bodies().filter(|(id, _)| !doc.is_hidden(*id)) {
        if let Some((low, high)) = held_box(body).and_then(|b| b.corners()) {
            extent.add(low);
            extent.add(high);
        }
    }
    extent
}

/// The most bodies whose boxes [`held_box`] keeps.
const HELD_BOXES: usize = 4096;

/// `Body::held_bbox` of a body, worked out once per body: a picture asks
/// for the box of the bodies three or four times (its size, its tolerance,
/// its scene), and regeneration hands on the same `Arc` for every body an
/// edit did not touch, so a body is known by its `Arc` for as long as it
/// lives (a `Weak` that still upgrades to it). `None` for a body whose box
/// cannot be found.
fn held_box(body: &Arc<craft3d_topo::Body>) -> Option<Bbox3> {
    type Known = Vec<(std::sync::Weak<craft3d_topo::Body>, Option<Bbox3>)>;
    static KNOWN: std::sync::Mutex<Known> = std::sync::Mutex::new(Vec::new());
    {
        let known = KNOWN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, found)) = known
            .iter()
            // A `Weak` keeps its allocation, so no other body can be
            // where a known one was.
            .find(|(weak, _)| std::ptr::eq(weak.as_ptr(), Arc::as_ptr(body)))
        {
            return *found;
        }
    }
    // Outside the lock: this is the work the memo saves.
    let found = body.held_bbox().ok();
    let mut known = KNOWN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    known.retain(|(weak, _)| weak.strong_count() > 0);
    if known.len() >= HELD_BOXES {
        known.clear();
    }
    known.push((Arc::downgrade(body), found));
    found
}

/// The chordal tolerance of a picture of `doc` as `regen` has it. The
/// floor is the display rule's fraction ([`DISPLAY_RELATIVE_TOLERANCE`])
/// of the diagonal of the box around the bodies (of the sketches without
/// any); above it the tolerance is the coarsest of the floor times a power
/// of two, `floor · 2^⌊log2(footprint / floor)⌋`, that is at most
/// `footprint`, [`PICTURE_PIXEL_SHARE`] of the model units a pixel covers
/// (`pixel`, asked with the box around the bodies): so between a quarter
/// and half a pixel. The rungs make pictures of nearly the same framing
/// (a picture a few pixels larger) share one tolerance, and so one scene;
/// twice the pixels is one rung finer.
pub(crate) fn picture_tolerance(
    regen: &Regen,
    doc: &Document,
    pixel: impl FnOnce(&Bbox3) -> Option<f64>,
) -> f64 {
    let floor = preview_size(regen, doc) * DISPLAY_RELATIVE_TOLERANCE;
    let model = bodies_box(doc, regen);
    let Some(footprint) = pixel(&model).map(|p| p * PICTURE_PIXEL_SHARE) else {
        return floor;
    };
    let ratio = footprint / floor;
    if !(ratio.is_finite() && ratio >= 2.0) {
        return floor;
    }
    // ⌊log2⌋ of a ratio of at least 2 and at most f64::MAX: 1 to 1023.
    let mut rung = ratio.plog2().floor().clamp(1.0, 1023.0) as i32;
    // `log2` may round across an integer: put the rung right, exactly
    // (the rungs are the floor times powers of two).
    if floor * 2f64.ppowi(rung) > footprint {
        rung -= 1;
    } else if rung < 1023 && floor * 2f64.ppowi(rung + 1) <= footprint {
        rung += 1;
    }
    let tolerance = floor * 2f64.ppowi(rung);
    if tolerance.is_finite() {
        tolerance
    } else {
        floor
    }
}

/// The model units one pixel of a picture `height` pixels high covers at
/// the point of `model` nearest the camera (where the picture shows the
/// model largest): the height of the view there over the pixels. 0 when
/// the camera is inside the box; `None` for an empty box or a camera
/// without a size.
pub fn pixel_footprint(camera: &craft3d_render::Camera, height: u32, model: &Bbox3) -> Option<f64> {
    let (low, high) = model.corners()?;
    let pixels = f64::from(height.max(1));
    let seen = match camera.projection {
        craft3d_render::Projection::Orthographic { half_height } => 2.0 * half_height,
        craft3d_render::Projection::Perspective { fov_y_deg } => {
            let eye = camera.eye;
            // (`max` then `min`, not `clamp`, which panics on a NaN.)
            let nearest = Vec3::new(
                eye.x.max(low.x).min(high.x),
                eye.y.max(low.y).min(high.y),
                eye.z.max(low.z).min(high.z),
            );
            2.0 * eye.distance(nearest) * (fov_y_deg.to_radians() * 0.5).ptan()
        }
    };
    let footprint = seen / pixels;
    (footprint.is_finite() && footprint >= 0.0).then_some(footprint)
}

/// The area of a mesh: the sum of its triangles' areas, in order.
fn mesh_area(mesh: &Mesh) -> f64 {
    (0..mesh.triangles.len())
        .filter_map(|t| mesh.corners(t))
        .map(|[a, b, c]| 0.5 * (b - a).cross(c - a).length())
        .sum()
}

/// The sketch features that have a solution, in history order.
fn sketches<'a>(
    regen: &'a Regen,
    doc: &'a Document,
) -> impl Iterator<Item = (&'a craft3d_doc::Feature, &'a SketchResult)> {
    doc.features.iter().filter_map(|feature| {
        let solved = regen.feature(feature.id)?.sketch.as_deref()?;
        Some((feature, solved))
    })
}

/// How many chords a sweep of `sweep` radians on a circle of radius `r`
/// needs so that no chord is further than `tolerance` from the circle.
///
/// A chord over the angle θ is `r·(1 − cos(θ/2))` away from the circle at
/// its middle, so θ = 2·acos(1 − tolerance/r).
pub fn arc_segments(r: f64, sweep: f64, tolerance: f64) -> usize {
    let turn = std::f64::consts::TAU;
    let sweep = if sweep.is_finite() {
        sweep.abs().min(turn)
    } else {
        turn
    };
    // At least the share of a 24-gon this sweep covers, and one chord.
    let least = ((MIN_CIRCLE_SEGMENTS as f64 * sweep / turn).ceil() as usize).max(1);
    if !(r.is_finite() && r > 0.0 && tolerance.is_finite() && tolerance > 0.0) || tolerance >= r {
        return least;
    }
    let step = 2.0 * (1.0 - tolerance / r).pacos();
    if !(step.is_finite() && step > 0.0) {
        return MAX_ARC_SEGMENTS;
    }
    let needed = (sweep / step).ceil();
    if needed >= MAX_ARC_SEGMENTS as f64 {
        MAX_ARC_SEGMENTS
    } else {
        // Finite, non-negative and below the cap.
        (needed as usize).max(least)
    }
}

/// Points on the circle about (cx, cy) from angle `from`, counter-clockwise
/// over `sweep` radians: `segments + 1` of them.
fn sample_arc(
    solved: &SketchResult,
    (cx, cy): (f64, f64),
    r: f64,
    from: f64,
    sweep: f64,
    tolerance: f64,
) -> Vec<Vec3> {
    let segments = arc_segments(r, sweep, tolerance);
    (0..=segments)
        .map(|k| {
            let angle = from + sweep * (k as f64 / segments as f64);
            solved.to_world(cx + r * angle.pcos(), cy + r * angle.psin())
        })
        .collect()
}

fn polyline(solved: &SketchResult, entity: &Entity, tolerance: f64) -> Option<ScenePolyline> {
    let (curve, points) = match entity.geometry {
        // A text's curves are drawn by `text_polylines`.
        Geometry::Point { .. } | Geometry::Image { .. } | Geometry::Text(_) => return None,
        Geometry::Formula { .. } | Geometry::Interpolated { .. } => {
            let fitted = solved.curve(entity.id)?;
            let points = craft3d_regen::curve_fit::polyline(&fitted.spline, tolerance)
                .into_iter()
                .map(|[x, y]| solved.to_world(x, y))
                .collect();
            (SketchCurve::Spline, points)
        }
        Geometry::Spline {
            degree,
            ref control,
        } => {
            let control: Vec<(f64, f64)> = control
                .iter()
                .map(|id| solved.point(*id))
                .collect::<Option<_>>()?;
            let curve = crate::sketch_spline_cmds::SplineCurve::new(degree, &control)?;
            let points = curve
                .polyline(tolerance)
                .into_iter()
                .map(|(x, y)| solved.to_world(x, y))
                .collect();
            (SketchCurve::Spline, points)
        }
        Geometry::Line { start, end } => {
            let (a, b) = (solved.point(start)?, solved.point(end)?);
            (
                SketchCurve::Line,
                vec![solved.to_world(a.0, a.1), solved.to_world(b.0, b.1)],
            )
        }
        Geometry::Circle { center, .. } => {
            let r = solved.radius(entity.id)?;
            let mut points = sample_arc(
                solved,
                solved.point(center)?,
                r,
                0.0,
                std::f64::consts::TAU,
                tolerance,
            );
            // Closed exactly, not within rounding of cos(2π).
            let first = points.first().copied();
            if let (Some(first), Some(last)) = (first, points.last_mut()) {
                *last = first;
            }
            (SketchCurve::Circle, points)
        }
        Geometry::Arc { center, start, end } => {
            let (c, a, b) = (
                solved.point(center)?,
                solved.point(start)?,
                solved.point(end)?,
            );
            let r = (a.0 - c.0).phypot(a.1 - c.1);
            let from = (a.1 - c.1).patan2(a.0 - c.0);
            let to = (b.1 - c.1).patan2(b.0 - c.0);
            // Counter-clockwise from start to end; coincident ends are a
            // full turn.
            let mut sweep = (to - from).rem_euclid(std::f64::consts::TAU);
            if sweep <= 0.0 {
                sweep = std::f64::consts::TAU;
            }
            let mut points = sample_arc(solved, c, r, from, sweep, tolerance);
            // The arc ends on its end point, whatever the rounding.
            if let Some(last) = points.last_mut() {
                *last = solved.to_world(b.0, b.1);
            }
            (SketchCurve::Arc, points)
        }
    };
    points.iter().all(|p| p.is_finite()).then(|| ScenePolyline {
        entity: entity.id.to_string(),
        curve,
        construction: entity.construction,
        points,
    })
}

/// An instance of a sketch pattern as a polyline, named as the sketch
/// calls it (`e7[3]`).
fn instance_polyline(
    solved: &SketchResult,
    made: &craft3d_regen::PatternCurve,
    construction: bool,
    tolerance: f64,
) -> Option<ScenePolyline> {
    use craft3d_regen::Curve2;
    let world = |[x, y]: [f64; 2]| solved.to_world(x, y);
    let (curve, points) = match &made.curve {
        Curve2::Line { start, end } => (SketchCurve::Line, vec![world(*start), world(*end)]),
        Curve2::Circle { center, radius } => {
            let mut points = sample_arc(
                solved,
                (center[0], center[1]),
                *radius,
                0.0,
                std::f64::consts::TAU,
                tolerance,
            );
            let first = points.first().copied();
            if let (Some(first), Some(last)) = (first, points.last_mut()) {
                *last = first;
            }
            (SketchCurve::Circle, points)
        }
        Curve2::Arc { center, start, end } => {
            let (c, a, b) = (center, start, end);
            let r = (a[0] - c[0]).phypot(a[1] - c[1]);
            let from = (a[1] - c[1]).patan2(a[0] - c[0]);
            let to = (b[1] - c[1]).patan2(b[0] - c[0]);
            let mut sweep = (to - from).rem_euclid(std::f64::consts::TAU);
            if sweep <= 0.0 {
                sweep = std::f64::consts::TAU;
            }
            let mut points = sample_arc(solved, (c[0], c[1]), r, from, sweep, tolerance);
            if let Some(last) = points.last_mut() {
                *last = world(*b);
            }
            (SketchCurve::Arc, points)
        }
        Curve2::Spline(spline) => {
            let control: Vec<(f64, f64)> = spline.control.iter().map(|p| (p[0], p[1])).collect();
            let degree = u32::try_from(spline.degree).ok()?;
            let curve = crate::sketch_spline_cmds::SplineCurve::new(degree, &control)?;
            let points = curve
                .polyline(tolerance)
                .into_iter()
                .map(|(x, y)| solved.to_world(x, y))
                .collect();
            (SketchCurve::Spline, points)
        }
    };
    points.iter().all(|p| p.is_finite()).then(|| ScenePolyline {
        entity: solved.curve_name(made.id),
        curve,
        construction,
        points,
    })
}

/// The curves of the texts of a sketch, each named as the sketch calls it
/// (`e7.glyph(3,U+0041).contour(1)`).
fn text_polylines(solved: &SketchResult, tolerance: f64) -> Vec<ScenePolyline> {
    let mut out = Vec::new();
    for text in &solved.texts {
        for curve in &text.curves {
            let points: Vec<Vec3> = craft3d_regen::curve_fit::polyline(&curve.spline, tolerance)
                .into_iter()
                .map(|[x, y]| solved.to_world(x, y))
                .collect();
            if points.len() >= 2 && points.iter().all(|p| p.is_finite()) {
                out.push(ScenePolyline {
                    entity: curve.name.to_string(),
                    curve: SketchCurve::Spline,
                    construction: text.construction,
                    points,
                });
            }
        }
    }
    out
}

fn scene_sketch(
    feature: &craft3d_doc::Feature,
    sketch: &Sketch,
    solved: &SketchResult,
    tolerance: f64,
) -> SceneSketch {
    let mut polylines: Vec<ScenePolyline> = sketch
        .entities
        .iter()
        .filter_map(|entity| polyline(solved, entity, tolerance))
        .collect();
    polylines.extend(text_polylines(solved, tolerance));
    // The instances of the sketch's patterns, as the curves they are.
    if let Ok(instances) = craft3d_regen::pattern_instances(sketch, solved) {
        polylines.extend(instances.iter().filter_map(|(curve, construction)| {
            instance_polyline(solved, curve, *construction, tolerance)
        }));
    }
    let points = sketch
        .entities
        .iter()
        .filter(|entity| matches!(entity.geometry, Geometry::Point { .. }))
        .filter_map(|entity| solved.point(entity.id))
        .map(|(x, y)| solved.to_world(x, y))
        .filter(|p| p.is_finite())
        .collect();
    SceneSketch {
        feature: feature.id.to_string(),
        name: feature.name.clone(),
        origin: solved.origin,
        x_axis: solved.x_axis,
        y_axis: solved.y_axis,
        normal: solved.normal,
        polylines,
        points,
    }
}

/// Most points a helix is drawn with.
const MAX_HELIX_POINTS: f64 = 65_536.0;

/// A datum helix as a curve: a sketch of one spline on the plane square
/// to its axis through its start, with points close enough that no chord
/// is further than `tolerance` from the helix.
fn scene_helix(
    feature: &craft3d_doc::Feature,
    helix: &craft3d_regen::Helix,
    tolerance: f64,
) -> Option<SceneSketch> {
    let step = if tolerance < helix.radius {
        2.0 * (1.0 - tolerance / helix.radius).pacos()
    } else {
        std::f64::consts::FRAC_PI_2
    };
    let count = (helix.angle() / step.clamp(1e-6, std::f64::consts::FRAC_PI_2))
        .ceil()
        .clamp(1.0, MAX_HELIX_POINTS);
    if !count.is_finite() {
        return None;
    }
    let count = count as usize;
    let points: Vec<Vec3> = (0..=count)
        .map(|i| helix.point(helix.angle() * i as f64 / count as f64))
        .collect();
    let y_axis = helix.axis.cross(helix.start);
    points.iter().all(|p| p.is_finite()).then(|| SceneSketch {
        feature: feature.id.to_string(),
        name: feature.name.clone(),
        origin: helix.origin,
        x_axis: helix.start,
        y_axis,
        normal: helix.axis,
        polylines: vec![ScenePolyline {
            entity: "helix".to_string(),
            curve: SketchCurve::Spline,
            construction: false,
            points,
        }],
        points: Vec::new(),
    })
}

/// Why a build with a triangle budget stopped before its last body.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stop {
    /// The triangles so far are over the budget, with `faces_done` of the
    /// model's `faces_total` faces meshed.
    OverBudget {
        triangles: usize,
        faces_done: usize,
        faces_total: usize,
    },
    /// A body needs more segments on a patch than the tessellator allows
    /// at this tolerance.
    TooFine { needed: f64, max: u32 },
    /// The scene is not wanted any more ([`Refinement::run`]).
    Cancelled,
}

/// How a scene is sampled: see [`build`].
#[derive(Clone, Copy, Debug)]
struct Sampling {
    tolerance: f64,
    angle: Option<f64>,
    budget: Option<usize>,
    per_body: Option<usize>,
}

impl Sampling {
    /// At `tolerance` with the chordal tolerance alone and no budget: the
    /// scenes of a given tolerance and of the automatic one.
    fn plain(tolerance: f64) -> Self {
        Self {
            tolerance,
            angle: None,
            budget: None,
            per_body: None,
        }
    }
}

/// Builds the scene at `tolerance`. With a `budget`, the build stops at the
/// first body with which the scene has more triangles than that, or that
/// is too fine for the tessellator, and says so; without one it never
/// stops, and a body the tessellator refuses is a problem of the scene.
///
/// The budget is the scene's: the bodies are tessellated one after the
/// other, in feature order, and each is given what the bodies before it
/// left (`TessParams::max_triangles`), so the body that goes over stops
/// in the tessellator, at the first faces that show it, and is not
/// tessellated in full first. A scene that is within the budget is the
/// scene built without one, and where a build stops and what it reports
/// depend on the document and the tolerance only, not on the number of
/// threads: the tessellator's promise, kept here by the fixed order.
///
/// Without a `budget`, a body the tessellator cannot do at the tolerance
/// because it would need more segments on a patch than it allows
/// (`TessError::TooFine`) is meshed at the finest coarser tolerance it can
/// do (`craft3d_tess::tessellate_within_budget`), and the scene is
/// [`Scene::coarsened`]. With `per_body` too, every body on its own is
/// kept within that many triangles the same way: what the quick first
/// picture does, so that one large body cannot make it slow. A body within
/// both is the body meshed at the tolerance.
///
/// `angle` is the tessellator's angle criterion, if any.
///
/// With `cancel`, the build stops before the next body once it is set.
///
/// Every body's mesh comes from `meshes`, which keeps it: a body meshed
/// so before (for an earlier state of the document, or for another
/// scene) is not meshed again, and the scene is the same either way.
fn build(
    doc: &Document,
    regen: &Regen,
    revision: u64,
    sampling: Sampling,
    cancel: Option<&AtomicBool>,
    meshes: &MeshStore,
) -> (Scene, Option<Stop>) {
    #[cfg(test)]
    tests::building();
    let Sampling {
        tolerance,
        angle,
        budget,
        per_body,
    } = sampling;
    let tess = TessParams {
        chordal_tolerance: tolerance,
        max_angle: angle,
        ..TessParams::default()
    };
    let mut scene = Scene {
        revision,
        tolerance,
        max_angle: angle,
        coarsened: false,
        preview: false,
        bodies: Arc::new(Vec::new()),
        mesh: Arc::new(Mesh::default()),
        face_colors: Arc::new(craft3d_render::FaceColors::new()),
        sketches: Vec::new(),
        datums: Vec::new(),
        features: Vec::new(),
        problems: Vec::new(),
        notices: Vec::new(),
        bbox: Bbox3::EMPTY,
        bodies_box: bodies_box(doc, regen),
        frame: Bbox3::EMPTY,
    };
    // How much of the model is done, for the estimate when the budget is
    // exceeded: by faces, which every body has and which cost nothing to
    // count.
    let faces_total: usize = if budget.is_some() {
        doc.features
            .iter()
            .filter(|f| !doc.is_hidden(f.id))
            .filter_map(|f| regen.feature(f.id).and_then(|o| o.own_body()))
            .fold(0, |n, body| n.saturating_add(body.faces.len()))
    } else {
        0
    };
    let mut faces_done: usize = 0;
    // The face ids of the next body start here: one past the highest id
    // in the scene's mesh so far.
    let mut next_face_id: u32 = 0;
    let steps = doc.features.len();
    let values = crate::appearance::Values::new(doc);
    for (index, feature) in doc.features.iter().enumerate() {
        let outcome = regen.feature(feature.id);
        let (state, error) = match outcome.map(|o| &o.status) {
            Some(Status::Ok) => (FeatureState::Ok, None),
            Some(Status::Suppressed) => (FeatureState::Suppressed, None),
            Some(Status::Inactive) => (FeatureState::Inactive, None),
            Some(Status::RolledBack) => (FeatureState::RolledBack, None),
            Some(Status::Failed(reason)) => (FeatureState::Failed, Some(reason.clone())),
            None => (
                FeatureState::Failed,
                Some("the feature was not regenerated".to_string()),
            ),
        };
        let solved = outcome.and_then(|o| o.sketch.as_deref());
        scene.features.push(SceneFeature {
            id: feature.id.to_string(),
            name: feature.name.clone(),
            kind: feature.kind.type_name(),
            state,
            error,
            dof: solved.and_then(|s| s.dof),
            is_sketch: matches!(feature.kind, FeatureKind::Sketch(_)),
            caused_by: craft3d_regen::root_cause(doc, &regen.features, feature.id)
                .map(|root| root.to_string()),
            hidden: doc.is_hidden(feature.id),
            when: feature.when.as_ref().map(|when| when.source().to_string()),
            passes: outcome.and_then(|outcome| {
                crate::commands::passes_text(doc, &regen.features, feature, outcome)
            }),
            // A feature switched off that passes a body on draws it.
            draws: outcome.is_some_and(craft3d_regen::FeatureResult::is_built)
                && (outcome.and_then(|o| o.own_body()).is_some()
                    || solved.is_some()
                    || outcome.and_then(|o| o.datum).is_some()),
        });
        // Hidden in the view: a row of the tree, and nothing drawn.
        if doc.is_hidden(feature.id) {
            continue;
        }
        // A suppressed or rolled-back sketch is not drawn, like its body
        // would not be.
        if let (FeatureKind::Sketch(sketch), Some(solved)) = (&feature.kind, solved)
            && matches!(state, FeatureState::Ok | FeatureState::Failed)
        {
            scene
                .sketches
                .push(scene_sketch(feature, sketch, solved, tolerance));
        }
        // A helix is drawn as a curve, like a spline of a sketch.
        if let (Some(craft3d_regen::Datum::Helix(helix)), FeatureState::Ok) =
            (outcome.and_then(|o| o.datum), state)
            && let Some(drawn) = scene_helix(feature, &helix, tolerance)
        {
            scene.sketches.push(drawn);
        }
        let Some(body) = outcome.and_then(|o| o.own_body()) else {
            continue;
        };
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return (scene, Some(Stop::Cancelled));
        }
        // The mesh and the edges in one go: the boundaries of the faces
        // are worked out once.
        craft3d_regen::progress::report(&craft3d_regen::progress::Progress {
            phase: "meshing",
            feature: Some(feature.id.to_string()),
            name: Some(feature.name.clone()),
            step: index.saturating_add(1),
            steps,
        });
        // What the body is meshed within. Without the scene's budget a
        // body too fine for the tessellator at this tolerance is
        // coarsened until it is not, and the scene says so (lamp T2,
        // 2026-10-15: a picture refused for it); within `per_body`
        // triangles too, when that is asked for. A body that needs
        // neither is the body meshed at the tolerance. With the scene's
        // budget, the body gets what the bodies before it left.
        let limit = match budget {
            None => per_body.map_or(Limit::None, Limit::Own),
            Some(max) => Limit::Left(max.saturating_sub(scene.mesh.triangles.len())),
        };
        // The mesh and the edges with the angle criterion `max_angle`, and
        // whether the tolerance was coarsened for the body.
        let mesh_with = |max_angle: Option<f64>| {
            let meshed = meshes
                .mesh(body, &TessParams { max_angle, ..tess }, limit)
                .map(|meshed| (meshed.mesh, meshed.edges, meshed.coarsened, meshed.placed));
            #[cfg(test)]
            let meshed = tests::tessellated(meshed, max_angle.is_some());
            meshed
        };
        let meshed = match mesh_with(angle) {
            // The angle is a wish: a body that cannot be meshed with it is
            // drawn without it, as `view.render` draws it, rather than not
            // at all. (Not a body over the budget, nor one a coarser
            // tolerance is about to be tried for.)
            Err(error)
                if angle.is_some()
                    && !matches!(error, TessError::OverBudget { .. })
                    && !(budget.is_some() && matches!(error, TessError::TooFine { .. })) =>
            {
                mesh_with(None).map_err(|_| error)
            }
            other => other,
        }
        .map(|(mesh, edges, coarsened, placed)| {
            scene.coarsened |= coarsened;
            (mesh, edges, placed)
        });
        let (mesh, edges, placed) = match meshed {
            Ok(both) => both,
            Err(TessError::OverBudget {
                triangles, faces, ..
            }) if budget.is_some() => {
                // The scene so far and the first faces of this body.
                let triangles = scene.mesh.triangles.len().saturating_add(triangles);
                return (
                    scene,
                    Some(Stop::OverBudget {
                        triangles,
                        faces_done: faces_done.saturating_add(faces),
                        faces_total,
                    }),
                );
            }
            Err(TessError::TooFine {
                needed_segments,
                max_segments,
                ..
            }) if budget.is_some() => {
                // A coarser tolerance will do: the caller's business.
                return (
                    scene,
                    Some(Stop::TooFine {
                        needed: needed_segments,
                        max: max_segments,
                    }),
                );
            }
            Err(e) => {
                scene.problems.push(format!("{}: {e}", feature.name));
                continue;
            }
        };
        // A fold folded flat lies in one plane: its layers are drawn apart
        // by their order (display only, fold_cmds::layered_mesh).
        let (mesh, edges, placed) = match outcome.and_then(|o| o.developed.as_ref()) {
            Some(craft3d_regen::Developed::FlatFold(flat)) => {
                let (m, e) = crate::fold_cmds::layered_mesh(&mesh, &edges, body, &flat.offsets);
                (Arc::new(m), Arc::new(e), None)
            }
            _ => (mesh, edges, placed),
        };
        let Ok(vertex_base) = u32::try_from(scene.mesh.positions.len()) else {
            scene
                .problems
                .push(format!("{}: the scene has too many vertices", feature.name));
            continue;
        };
        let first_face_id = next_face_id;
        if let Some(highest) = mesh.face_ids.iter().copied().max() {
            next_face_id = first_face_id.saturating_add(highest).saturating_add(1);
        }
        // The scene's own mesh while it is built: `make_mut` copies nothing.
        let all = Arc::make_mut(&mut scene.mesh);
        all.positions.extend(&mesh.positions);
        all.normals.extend(&mesh.normals);
        // (The body's own mesh and edges are shared with the store and
        // with the scenes of other states that have the body.)
        all.triangles.extend(
            mesh.triangles
                .iter()
                .map(|t| t.map(|k| k.saturating_add(vertex_base))),
        );
        all.face_ids.extend(
            mesh.face_ids
                .iter()
                .map(|f| f.saturating_add(first_face_id)),
        );
        let facts = meshes.facts(body, outcome.and_then(|o| o.names.as_ref()));
        crate::appearance::color_faces(
            Arc::make_mut(&mut scene.face_colors),
            &values,
            regen,
            feature.id,
            first_face_id,
            body.faces.len(),
        );
        Arc::make_mut(&mut scene.bodies).push(SceneBody {
            feature: feature.id.to_string(),
            name: feature.name.clone(),
            mesh,
            first_face_id,
            edges,
            facts,
            placement: placed.map(|(source, pose)| ScenePlacement { source, pose }),
        });
        faces_done = faces_done.saturating_add(body.faces.len());
        let triangles = scene.mesh.triangles.len();
        if let Some(max) = budget
            && triangles > max
        {
            return (
                scene,
                Some(Stop::OverBudget {
                    triangles,
                    faces_done,
                    faces_total,
                }),
            );
        }
    }
    let mut bbox = scene.mesh.bbox();
    for sketch in &scene.sketches {
        for point in sketch
            .polylines
            .iter()
            .flat_map(|p| &p.points)
            .chain(&sketch.points)
        {
            bbox.add(*point);
        }
    }
    scene.bbox = bbox;
    scene.frame = frame_of(doc, regen, &scene);
    scene.datums = scene_datums(doc, regen, &bbox);
    // A face colour whose face an edit took away colours nothing (never
    // another face); said here so the window shows it. It is no problem
    // of the picture, which is drawn without it.
    for lost in crate::appearance::lost_faces(doc, regen) {
        let made = doc
            .feature(lost.feature)
            .map_or_else(|| lost.feature.to_string(), |f| f.name.clone());
        scene.notices.push(format!(
            "the colour of face {} of `{made}` colours nothing: the face is gone (undo brings it back; face.appearance with clear takes the colour away)",
            lost.face
        ));
    }
    (scene, None)
}

/// The placed datums of `doc`, sized to `bbox` (the model without them).
/// Without a model they are sized to themselves, and to one document
/// unit when that has no size either.
fn scene_datums(doc: &Document, regen: &Regen, bbox: &Bbox3) -> Vec<SceneDatum> {
    let placed: Vec<(&craft3d_doc::Feature, craft3d_regen::Datum)> = doc
        .features
        .iter()
        .filter_map(|feature| {
            let outcome = regen.feature(feature.id)?;
            match (&outcome.status, outcome.datum) {
                (Status::Ok, Some(datum)) if !doc.is_hidden(feature.id) => Some((feature, datum)),
                _ => None,
            }
        })
        .collect();
    let extent = if bbox.is_empty() {
        Bbox3::from_points(placed.iter().map(|(_, datum)| match datum {
            craft3d_regen::Datum::Plane(frame) => frame.origin(),
            craft3d_regen::Datum::Axis { origin, .. } => *origin,
            craft3d_regen::Datum::Point(point) => *point,
            craft3d_regen::Datum::Helix(helix) => helix.origin,
        }))
    } else {
        *bbox
    };
    let diagonal = extent.diagonal();
    let size = if diagonal.is_finite() && diagonal > 0.0 {
        diagonal
    } else {
        1.0
    };
    let (plane_half, axis_half) = (size * DATUM_PLANE_EXTENT, size * DATUM_AXIS_EXTENT);
    let centre = extent.center();
    let finite = |points: &[Vec3]| points.iter().all(|p| p.is_finite());
    placed
        .into_iter()
        .filter_map(|(feature, datum)| {
            let shape = match datum {
                craft3d_regen::Datum::Plane(frame) => {
                    let (origin, normal) = (frame.origin(), frame.z());
                    let c = centre.unwrap_or(origin);
                    let center = c - normal * (c - origin).dot(normal);
                    let (u, v) = (frame.x() * plane_half, frame.y() * plane_half);
                    finite(&[center, u, v, origin, normal]).then_some(SceneDatumShape::Plane {
                        center,
                        u,
                        v,
                        origin,
                        normal,
                    })?
                }
                craft3d_regen::Datum::Axis { origin, direction } => {
                    let c = centre.unwrap_or(origin);
                    let foot = origin + direction * (c - origin).dot(direction);
                    let (from, to) = (foot - direction * axis_half, foot + direction * axis_half);
                    finite(&[from, to]).then_some(SceneDatumShape::Axis { from, to })?
                }
                craft3d_regen::Datum::Point(point) => {
                    finite(&[point]).then_some(SceneDatumShape::Point(point))?
                }
                // Drawn with the sketches, as a curve.
                craft3d_regen::Datum::Helix(_) => return None,
            };
            Some(SceneDatum {
                feature: feature.id.to_string(),
                name: feature.name.clone(),
                shape,
            })
        })
        .collect()
}

/// How much coarser the next rung of a ladder is after a build that
/// stopped with `stop` under a budget of `max_triangles`: by the estimated
/// excess (the triangles so far over the share of the faces done, against
/// the budget), or for a body too fine for the tessellator by the square
/// of how far over the limit it is, at least twice and at most 64 times.
/// `None` when the build was cancelled.
fn coarser_by(stop: &Stop, max_triangles: usize) -> Option<f64> {
    let factor = match *stop {
        Stop::Cancelled => return None,
        Stop::OverBudget {
            triangles,
            faces_done,
            faces_total,
        } => {
            let share = if faces_total > 0 {
                (faces_done as f64 / faces_total as f64).clamp(1e-3, 1.0)
            } else {
                1.0
            };
            (triangles as f64 / share) / (max_triangles.max(1) as f64)
        }
        // Segments on a patch grow as the inverse square root of the
        // tolerance; a tenth more for the rounding of the count.
        Stop::TooFine { needed, max } => (needed / f64::from(max.max(1)) * 1.1).ppowi(2),
    };
    Some(if factor.is_finite() {
        factor.clamp(2.0, 64.0)
    } else {
        4.0
    })
}

/// A picture's scene of `doc` at the chordal tolerance `fine` with the
/// display's angle ([`SceneQuality::Picture`]): the display rule's ladder
/// ([`build_for_display`]) with a budget of [`DISPLAY_MAX_TRIANGLES`] for
/// the whole scene and no cap, climbed until the scene is within it. Each
/// rung is at least twice as coarse, so the climb ends within the range of
/// an `f64`; a scene that is over the budget at every tolerance (more
/// faces than the budget has triangles) is that partial scene with a
/// problem that says so, which a picture refuses. [`Scene::coarsened`]
/// when the tolerance or a body's had to be coarser than `fine`.
pub(crate) fn picture_scene_of(
    doc: &Document,
    regen: &Regen,
    revision: u64,
    fine: f64,
    meshes: &MeshStore,
) -> Scene {
    let mut tolerance = fine;
    loop {
        let sampling = Sampling {
            tolerance,
            angle: Some(DISPLAY_MAX_ANGLE),
            budget: Some(DISPLAY_MAX_TRIANGLES),
            per_body: None,
        };
        let (mut scene, stop) = build(doc, regen, revision, sampling, None, meshes);
        let Some(stop) = stop else {
            return Scene {
                coarsened: scene.coarsened || tolerance > fine,
                ..scene
            };
        };
        let Some(factor) = coarser_by(&stop, DISPLAY_MAX_TRIANGLES) else {
            return scene;
        };
        // Each rung at least twice as coarse, on the ladder's rungs (the
        // estimate moves with every edit; `on_rung`): still above the
        // tolerance, as a rung's nearest neighbour is within 4.5 %.
        let next = on_rung(tolerance * factor);
        if !(next.is_finite() && next > tolerance) {
            scene.problems.push(format!(
                "the picture has more than {DISPLAY_MAX_TRIANGLES} triangles at any chordal tolerance"
            ));
            return scene;
        }
        tolerance = next;
    }
}

/// What the display rule asks for ([`build_for_display`]): the chordal
/// tolerance it starts from, its triangle budget and its angle.
#[derive(Clone, Copy, Debug)]
struct DisplayRule {
    fine: f64,
    max_triangles: usize,
    angle: Option<f64>,
}

/// The display rule: the scene at `fine`, or at the coarsest tolerance
/// between `fine` and `cap` (the quick first picture's) that keeps the
/// scene within `max_triangles` triangles and within the tessellator's
/// reach. Each attempt that stops early says how far over it was, and the
/// next tolerance is coarser by that ratio (triangles grow as the inverse
/// of the tolerance at worst, so the estimate errs towards another
/// attempt, never towards a coarser mesh than needed), at least twice as
/// coarse and at most 64 times, on the ladder's rungs ([`on_rung`]). At or
/// beyond `cap` no budget applies, and
/// the scene is the first picture itself: the same tolerance gives the
/// same mesh. `None` when `cancel` was set meanwhile.
fn build_for_display(
    doc: &Document,
    regen: &Regen,
    preview: &Scene,
    rule: DisplayRule,
    cancel: &AtomicBool,
    meshes: &MeshStore,
) -> Option<Scene> {
    let DisplayRule {
        fine,
        max_triangles,
        angle,
    } = rule;
    let (revision, cap) = (preview.revision, preview.tolerance);
    let mut tolerance = fine;
    let mut attempts = 0;
    while tolerance < cap && attempts < MAX_DISPLAY_ATTEMPTS {
        attempts += 1;
        let sampling = Sampling {
            tolerance,
            angle,
            budget: Some(max_triangles),
            per_body: None,
        };
        let (scene, stop) = build(doc, regen, revision, sampling, Some(cancel), meshes);
        let Some(stop) = stop else {
            return Some(Scene {
                coarsened: tolerance > fine,
                ..scene
            });
        };
        let factor = coarser_by(&stop, max_triangles)?;
        // On the ladder's rung, like the first attempt (`on_rung`): the
        // estimate moves a little with every edit, and a model over the
        // budget would otherwise be meshed again whole at every step.
        tolerance = on_rung((tolerance * factor).min(cap));
    }
    // At the cap, out of attempts below it, or asked for something
    // coarser from the start: no budget.
    let tolerance = if tolerance.is_finite() && tolerance >= cap {
        tolerance
    } else {
        cap
    };
    let same_angle = angle.map(f64::to_bits) == preview.max_angle.map(f64::to_bits);
    let scene = if tolerance.to_bits() == cap.to_bits() && same_angle {
        // The first picture is that scene already.
        Scene {
            preview: false,
            ..preview.clone()
        }
    } else {
        // No budget, but each body within the display's default on its
        // own, as in the first picture.
        let sampling = Sampling {
            tolerance,
            angle,
            budget: None,
            per_body: Some(DISPLAY_MAX_TRIANGLES),
        };
        match build(doc, regen, revision, sampling, Some(cancel), meshes) {
            (_, Some(Stop::Cancelled)) => return None,
            (scene, _) => scene,
        }
    };
    Some(Scene {
        coarsened: tolerance > fine || scene.coarsened,
        ..scene
    })
}

/// The message of a panic caught while building a scene.
fn internal_error(payload: &(dyn std::any::Any + Send)) -> CommandError {
    let detail = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no details".to_string());
    CommandError::new(format!("the scene could not be built: {BUG}: {detail}"))
}

/// The display scene of one state of a document, to be built away from
/// the session (on a thread of its own) while a viewer shows the quick
/// first picture: [`Session::refinement`] gives it, [`Self::run`] builds
/// it, [`Session::accept_refined`] takes the result back. It holds the
/// document state and its regeneration, both shared, so the session can
/// go on running commands meanwhile.
pub struct Refinement {
    doc: Arc<Document>,
    regen: Arc<Regen>,
    key: SceneKey,
    relative: f64,
    max_triangles: usize,
    max_angle: Option<f64>,
    preview: Arc<Scene>,
    /// The size the display tolerance is a fraction of, when it is not
    /// the first picture's own: a state the view is scrubbed to is meshed
    /// as the document as it is ([`Session::scrubbed_sizes`]).
    size: Option<f64>,
    /// The session's store of meshes, shared: what the refinement meshes
    /// serves the scenes built on the engine thread, and the other way.
    meshes: MeshStore,
}

/// What [`Refinement::run`] built.
pub struct Refined {
    doc: Arc<Document>,
    key: SceneKey,
    /// `None` when it was cancelled.
    built: Option<Built>,
}

impl Refined {
    /// True when the refinement was cancelled before it was done.
    pub fn cancelled(&self) -> bool {
        self.built.is_none()
    }
}

impl Refinement {
    /// Builds the display scene: the display rule ([`SceneQuality::Display`])
    /// with the model's size taken from the area of the first picture's
    /// mesh (the exact area takes seconds to measure on spline faces; the
    /// mesh's is within its chordal tolerance of it), the tolerance on
    /// the ladder's nearest rung ([`on_rung`]: so that an edit that moves
    /// the size a little leaves the tolerance, and the meshes of the
    /// bodies it did not touch, as they were), and the first picture's
    /// tolerance as the coarsest it goes. Stops before the next body, or
    /// the next attempt, once `cancel` is set. Never panics: a panic in
    /// the kernel becomes the scene's error.
    pub fn run(&self, cancel: &AtomicBool) -> Refined {
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let area = mesh_area(&self.preview.mesh);
            let size = match self.size {
                Some(size) => size,
                None if area.is_finite() && area > 0.0 => area.sqrt(),
                None => model_size(&self.regen, &self.doc),
            };
            build_for_display(
                &self.doc,
                &self.regen,
                &self.preview,
                DisplayRule {
                    fine: on_rung(size * self.relative),
                    max_triangles: self.max_triangles,
                    angle: self.max_angle,
                },
                cancel,
                &self.meshes,
            )
        }));
        let built = match built {
            Ok(Some(scene)) => Some(Ok(Arc::new(scene))),
            Ok(None) => None,
            Err(payload) => Some(Err(internal_error(payload.as_ref()))),
        };
        Refined {
            doc: Arc::clone(&self.doc),
            key: self.key,
            built,
        }
    }

    /// True while `session` still wants this scene: its document state is
    /// the active one, and its display scene is not there yet.
    pub fn wanted(&self, session: &mut Session) -> bool {
        let active = session
            .documents
            .get_mut(session.active)
            .map(crate::OpenDocument::shown);
        session.scene_revision();
        active.is_some_and(|doc| Arc::ptr_eq(&doc, &self.doc)) && !session.scene.has(self.key)
    }
}

impl Session {
    /// A number that changes exactly when the active document does: after a
    /// command that edited it, an undo or a redo, and when another document
    /// becomes the active one. Commands that only read, and commands that
    /// fail, leave it alone.
    ///
    /// It counts the changes this function has noticed, so two edits
    /// between two calls move it by one. Scrubbing the view through the
    /// history (`view.scrub`) changes the state shown, and moves it too.
    pub fn scene_revision(&mut self) -> u64 {
        let open = self.documents.get_mut(self.active);
        let base = open.as_ref().map(|open| Arc::clone(open.history.current()));
        let current = open.map(crate::OpenDocument::shown);
        self.scene.pinned = base.filter(|base| {
            current
                .as_ref()
                .is_some_and(|current| !Arc::ptr_eq(base, current))
        });
        self.scene.observe(current.as_ref())
    }

    /// The scene of the active document, or `None` when no document is
    /// open. `tolerance` is the chordal tolerance in document units; `None`
    /// picks the one `view.render` uses ([`SceneQuality::Automatic`]). A
    /// tolerance that is not a positive number is an error.
    ///
    /// Cached: asking again for the same tolerance while the document has
    /// not changed returns the same `Arc`.
    pub fn scene(&mut self, tolerance: Option<f64>) -> Result<Option<Arc<Scene>>, CommandError> {
        self.scene_with(tolerance.map_or(SceneQuality::Automatic, SceneQuality::Tolerance))
    }

    /// The scene a picture of the active document is drawn from
    /// (`view.render`, `view.pick`, `view.replay`, MCP's
    /// `doc_render_preview`). At `tolerance` when one is given. Otherwise
    /// [`SceneQuality::Picture`] at the picture's own tolerance
    /// ([`picture_tolerance`]: between a quarter and half a pixel in model
    /// units where the picture shows the model, never finer than the
    /// display rule's tolerance), with the display's angle, coarsened by
    /// the display's ladder until the scene has at most
    /// [`DISPLAY_MAX_TRIANGLES`] triangles. `pixel` is given the box around
    /// the bodies (the one pictures are framed on, [`Scene::bodies_box`])
    /// and says how many model units a pixel of the picture covers at its
    /// nearest point ([`pixel_footprint`]); `None` leaves the display
    /// tolerance. A function of the document and the picture's parameters
    /// alone: no scene the window keeps is drawn instead (lamp T2,
    /// 2026-10-15: the display scene of the lamp took minutes for one
    /// picture). No scene refuses a body that is too fine for the
    /// tessellator: it is coarsened, and the scene says so
    /// ([`Scene::coarsened`]).
    pub fn picture_scene(
        &mut self,
        tolerance: Option<f64>,
        pixel: impl FnOnce(&Bbox3) -> Option<f64>,
    ) -> Result<Option<Arc<Scene>>, CommandError> {
        if let Some(tolerance) = tolerance {
            return self.scene(Some(tolerance));
        }
        self.scene_revision();
        let Some(open) = self.documents.get_mut(self.active) else {
            return Ok(None);
        };
        let doc = open.shown();
        let regen = open.shown_regen();
        let tolerance = picture_tolerance(&regen, &doc, pixel);
        self.scene_with(SceneQuality::Picture(tolerance))
    }

    /// Keeps the error of a panic in [`Session::prepare_some`] as the
    /// active document's first picture and its scene at `quality`: a
    /// viewer then shows the error and asks for nothing again until the
    /// document changes (a panic in the work would come back every frame).
    pub(crate) fn fail_scenes(
        &mut self,
        quality: SceneQuality,
        payload: &(dyn std::any::Any + Send),
    ) {
        self.scene_revision();
        let Some(open) = self.documents.get_mut(self.active) else {
            return;
        };
        let doc = open.shown();
        let error = internal_error(payload);
        self.scene
            .store(&doc, SceneKey::Preview, Err(error.clone()));
        if quality.validate().is_ok() {
            self.scene.store(&doc, quality.key(), Err(error));
        }
    }

    /// For a host with one thread ([`Session::prepare_some`]): builds the
    /// quick first picture of the active document ([`SceneQuality::Preview`])
    /// a body at a time, until `stop` says so after a body. True when it
    /// is there (built now or before); false when it stopped first, with
    /// the bodies meshed so far kept in the store of meshes, so the next
    /// call goes on from there. The picture is the one
    /// [`Self::scene_with`] builds in one go.
    pub(crate) fn preview_some(&mut self, stop: impl Fn() -> bool + 'static) -> bool {
        let revision = self.scene_revision();
        if self.scene.has(SceneKey::Preview) {
            return true;
        }
        let meshes = self.scene.meshes.clone();
        let Some(open) = self.documents.get_mut(self.active) else {
            return true;
        };
        let doc = open.shown();
        let regen = open.shown_regen();
        let cancel = Arc::new(AtomicBool::new(false));
        let for_sink = Arc::clone(&cancel);
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            craft3d_regen::progress::with_sink(
                // Asked as each body is about to be meshed: that one is
                // meshed, the build stops before the next.
                move |_| {
                    if stop() {
                        for_sink.store(true, Ordering::Relaxed);
                    }
                },
                || {
                    let sampling = Sampling {
                        tolerance: on_rung(preview_size(&regen, &doc) * PREVIEW_RELATIVE_TOLERANCE),
                        angle: None,
                        budget: None,
                        per_body: Some(DISPLAY_MAX_TRIANGLES),
                    };
                    build(&doc, &regen, revision, sampling, Some(&cancel), &meshes)
                },
            )
        }));
        let scene = match built {
            Ok((_, Some(Stop::Cancelled))) => return false,
            Ok((scene, _)) => Ok(Arc::new(Scene {
                preview: true,
                ..scene
            })),
            Err(payload) => Err(internal_error(payload.as_ref())),
        };
        self.scene.store(&doc, SceneKey::Preview, scene);
        true
    }

    /// [`Self::scene`] for any [`SceneQuality`]: what a viewport asks for
    /// with [`SceneQuality::Display`]. Cached per quality: asking again for
    /// the same quality while the document has not changed returns the
    /// same `Arc`.
    ///
    /// For the display rule the quick first picture is built first
    /// ([`PREVIEW_RELATIVE_TOLERANCE`]); when the display scene is built
    /// elsewhere ([`Self::set_refine_elsewhere`]) that picture is the
    /// answer until the display scene is accepted, and it says so
    /// ([`Scene::preview`]).
    pub fn scene_with(
        &mut self,
        quality: SceneQuality,
    ) -> Result<Option<Arc<Scene>>, CommandError> {
        quality.validate()?;
        let revision = self.scene_revision();
        let key = quality.key();
        if let Some(built) = self.scene.lookup(key) {
            return built.map(Some);
        }
        let meshes = self.scene.meshes.clone();
        let (relative, max_triangles, max_angle) = match quality {
            SceneQuality::Display {
                relative,
                max_triangles,
                max_angle,
            } => (relative, max_triangles, max_angle),
            SceneQuality::Preview => {
                let sizes = self.scrubbed_sizes();
                return self.build_and_keep(key, |doc, regen| {
                    let size = sizes.map_or_else(|| preview_size(regen, doc), |s| s.preview);
                    let sampling = Sampling {
                        tolerance: on_rung(size * PREVIEW_RELATIVE_TOLERANCE),
                        angle: None,
                        budget: None,
                        per_body: Some(DISPLAY_MAX_TRIANGLES),
                    };
                    Scene {
                        preview: true,
                        ..build(doc, regen, revision, sampling, None, &meshes).0
                    }
                });
            }
            SceneQuality::Picture(tolerance) => {
                return self.build_and_keep(key, |doc, regen| {
                    picture_scene_of(doc, regen, revision, tolerance, &meshes)
                });
            }
            SceneQuality::Automatic | SceneQuality::Tolerance(_) => {
                return self.build_and_keep(key, |doc, regen| {
                    let tolerance = match quality {
                        SceneQuality::Tolerance(t) => t,
                        _ => model_size(regen, doc) * AUTOMATIC_RELATIVE_TOLERANCE,
                    };
                    build(
                        doc,
                        regen,
                        revision,
                        Sampling::plain(tolerance),
                        None,
                        &meshes,
                    )
                    .0
                });
            }
        };
        let Some(preview) = self.scene_with(SceneQuality::Preview)? else {
            return Ok(None);
        };
        if self.scene.refine_elsewhere {
            return Ok(Some(preview));
        }
        let Some(job) = self.refinement_from(key, relative, max_triangles, max_angle, preview)
        else {
            return Ok(None);
        };
        let refined = job.run(&AtomicBool::new(false));
        self.accept_refined(refined);
        self.scene.lookup(key).transpose()
    }

    /// Builds a scene of the active document with `make`, guarded, and
    /// keeps it (or its error) under `key`.
    fn build_and_keep(
        &mut self,
        key: SceneKey,
        make: impl FnOnce(&Document, &Regen) -> Scene,
    ) -> Result<Option<Arc<Scene>>, CommandError> {
        let Some(open) = self.documents.get_mut(self.active) else {
            return Ok(None);
        };
        let doc = open.shown();
        // Regeneration and tessellation run here, outside any command: a
        // panic in them must not take the viewer down (AGENTS.md,
        // last-resort guard). The document is not touched either way.
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let regen = open.shown_regen();
            craft3d_regen::progress::report(&craft3d_regen::progress::Progress {
                phase: "measuring",
                feature: None,
                name: None,
                step: 0,
                steps: 0,
            });
            make(&doc, &regen)
        }));
        let scene = built
            .map(Arc::new)
            .map_err(|payload| internal_error(payload.as_ref()));
        self.scene.store(&doc, key, scene.clone());
        scene.map(Some)
    }

    fn refinement_from(
        &mut self,
        key: SceneKey,
        relative: f64,
        max_triangles: usize,
        max_angle: Option<f64>,
        preview: Arc<Scene>,
    ) -> Option<Refinement> {
        let meshes = self.scene.meshes.clone();
        let size = self.scrubbed_sizes().map(|s| s.display);
        let open = self.documents.get_mut(self.active)?;
        Some(Refinement {
            size,
            doc: open.shown(),
            regen: open.shown_regen(),
            key,
            relative,
            max_triangles,
            max_angle,
            preview,
            meshes,
        })
    }

    /// While the view of the active document is scrubbed to an earlier
    /// state of its history (`view.scrub`): the sizes of the document as
    /// it is, which the tolerances of the viewer's scenes of that state
    /// are fractions of instead of its own. Every state of a history is
    /// then meshed as finely as the end of it, so a body a step shares
    /// with other steps or with the end keeps its mesh (the store of
    /// meshes finds it), and the picture does not grow coarser or finer
    /// as the model grows while the player plays. Only once the document
    /// as it is has its own first picture (the window showed it), and not
    /// while states are built elsewhere (a browser's worker meshes them at
    /// their own sizes).
    /// `None` when the view shows the document as it is (or its motion,
    /// or an assembly).
    pub(crate) fn scrubbed_sizes(&mut self) -> Option<crate::scrub::ShownSizes> {
        // A state built elsewhere is meshed there, at its own sizes: the
        // same rule on both sides, so the two scenes are the same bits.
        if self.regen_elsewhere() {
            return None;
        }
        let open = self.documents.get_mut(self.active)?;
        let current = Arc::clone(open.history.current());
        let shown = open.shown();
        let scrubbed = open.scrubbed();
        if !Arc::ptr_eq(&shown, &scrubbed) || Arc::ptr_eq(&scrubbed, &current) {
            return None;
        }
        if let Some((of, sizes)) = &open.scrub.sizes
            && Arc::ptr_eq(of, &current)
        {
            return Some(*sizes);
        }
        let regen = open.regen();
        let preview = preview_size(&regen, &current);
        let found = self
            .scene
            .kept
            .iter()
            .find(|kept| Arc::ptr_eq(&kept.doc, &current))
            .and_then(|kept| {
                kept.scenes
                    .iter()
                    .find(|(key, ..)| *key == SceneKey::Preview)
                    .and_then(|(_, built, _)| built.as_ref().ok().cloned())
            });
        // Never built here for this (a whole first picture, on the
        // thread that asked): without the document's own first picture
        // the state is meshed at its own sizes, as before, until there is
        // one.
        let picture = found?;
        let area = mesh_area(&picture.mesh);
        let display = if area.is_finite() && area > 0.0 {
            area.sqrt()
        } else {
            model_size(&regen, &current)
        };
        let sizes = crate::scrub::ShownSizes { preview, display };
        let open = self.documents.get_mut(self.active)?;
        open.scrub.sizes = Some((current, sizes));
        Some(sizes)
    }

    /// The display scene of the active document for `quality`, to be
    /// built elsewhere: `Some` when the quick first picture is there and
    /// the display scene is not (nor known to fail). A shell that keeps
    /// its window drawing runs it on a thread of its own and hands the
    /// result to [`Self::accept_refined`].
    pub fn refinement(&mut self, quality: SceneQuality) -> Option<Refinement> {
        let SceneQuality::Display {
            relative,
            max_triangles,
            max_angle,
        } = quality
        else {
            return None;
        };
        quality.validate().ok()?;
        self.scene_revision();
        let key = quality.key();
        if self.scene.has(key) {
            return None;
        }
        let preview = self.scene.lookup(SceneKey::Preview)?.ok()?;
        self.refinement_from(key, relative, max_triangles, max_angle, preview)
    }

    /// Keeps the scene a [`Refinement`] built, for the document state it
    /// was built for (also when another document, or another state, is
    /// active by now: it is shown when that state is active again). True
    /// when it is now the active document's. A cancelled one is dropped.
    pub fn accept_refined(&mut self, refined: Refined) -> bool {
        let Refined { doc, key, built } = refined;
        let Some(built) = built else {
            return false;
        };
        self.scene_revision();
        self.scene.store(&doc, key, built);
        self.scene
            .seen
            .as_ref()
            .is_some_and(|seen| Arc::ptr_eq(seen, &doc))
    }

    /// True when the scenes kept for the document state `doc` have one
    /// (or its error) for `quality` (`regen_elsewhere.rs`).
    /// A scene kept for the document state `doc`, at any quality: what a
    /// datum plane of that state is taken from without regenerating it
    /// (each scene carries the datum planes as regeneration placed them).
    /// `None` when none is kept; the error of one that failed.
    pub(crate) fn kept_scene_of(&self, doc: &Arc<Document>) -> Option<Result<Arc<Scene>, String>> {
        let kept = self
            .scene
            .kept
            .iter()
            .find(|kept| Arc::ptr_eq(&kept.doc, doc))?;
        let mut built = kept.scenes.iter().map(|(_, built, _)| built);
        let first = built.next()?;
        Some(
            std::iter::once(first)
                .chain(built)
                .find_map(|b| b.as_ref().ok().cloned())
                .ok_or_else(|| match first {
                    Err(e) => e.message().to_string(),
                    Ok(_) => String::new(),
                }),
        )
    }

    pub(crate) fn has_scene_of(&self, doc: &Arc<Document>, quality: SceneQuality) -> bool {
        let key = quality.key();
        self.scene
            .kept
            .iter()
            .any(|kept| Arc::ptr_eq(&kept.doc, doc) && kept.scenes.iter().any(|(k, ..)| *k == key))
    }

    /// Keeps `built` as the scene of the document state `doc` for
    /// `quality`, as one built here would be kept (`regen_elsewhere.rs`).
    pub(crate) fn keep_scene_of(
        &mut self,
        doc: &Arc<Document>,
        quality: SceneQuality,
        built: Built,
    ) {
        self.scene_revision();
        self.scene.store(doc, quality.key(), built);
    }

    /// The revision the scenes of the active document are shown at now,
    /// as [`Self::scene_revision`] last counted it.
    pub(crate) fn scene_revision_now(&self) -> u64 {
        self.scene.revision
    }

    /// The most bytes of scenes kept ([`Self::set_scene_budget`]).
    pub(crate) fn scene_budget(&self) -> usize {
        self.scene.budget
    }

    /// Says that the display scene is built elsewhere, by a host that runs
    /// [`Self::refinement`] on a thread of its own (the window), so that
    /// [`Self::scene_with`] answers the display rule with the quick first
    /// picture until then. Off by default: the display scene is built
    /// where it is asked for.
    pub fn set_refine_elsewhere(&mut self, elsewhere: bool) {
        self.scene.refine_elsewhere = elsewhere;
    }

    /// The most bytes of scenes kept for the open documents and their
    /// earlier states ([`DEFAULT_SCENE_BUDGET`]); the least recently shown
    /// go first. The active document's scenes are kept whatever they hold.
    /// The meshes of the bodies are kept within a budget of the same size
    /// of their own ([`Self::mesh_bytes`]); as the kept scenes hold their
    /// meshes from that store, the two overlap, and the memory is at most
    /// their sum: twice `bytes`.
    pub fn set_scene_budget(&mut self, bytes: usize) {
        self.scene.budget = bytes;
        self.scene.evict();
        self.scene.meshes.set_budget(bytes);
    }

    /// The bytes of the scenes kept now ([`Scene::bytes`], which counts
    /// the mesh of a body once per scene that has it).
    pub fn scene_bytes(&self) -> usize {
        self.scene.bytes()
    }

    /// The bytes of the meshes of bodies kept now, by body and sampling,
    /// which the scenes share: a scene rebuilt after an edit meshes only
    /// the bodies the edit changed.
    pub fn mesh_bytes(&self) -> usize {
        self.scene.meshes.bytes()
    }

    /// True when [`Self::scene_with`] for `quality` would answer at once:
    /// the scene is built (or known to fail), no document is open, or the
    /// quality is refused; for the display rule while the display scene is
    /// built elsewhere, when the quick first picture is there. A host that
    /// keeps a window drawing builds the scene elsewhere when this is
    /// false.
    pub fn scene_ready(&mut self, quality: SceneQuality) -> bool {
        if quality.validate().is_err() || !self.has_document() {
            return true;
        }
        self.scene_revision();
        let key = quality.key();
        self.scene.has(key)
            || (self.scene.refine_elsewhere
                && matches!(quality, SceneQuality::Display { .. })
                && self.scene.has(SceneKey::Preview))
    }

    /// Millimetres per length unit of the active document (1 without
    /// one): the scale a camera lens needs (`look.dof`).
    pub fn length_mm(&self) -> f64 {
        self.document().map_or(1.0, |open| {
            open.history.current().units.length.as_expr_unit().factor()
        })
    }

    /// The active document as a status bar shows it.
    pub fn document_status(&self) -> Option<DocumentStatus> {
        self.status_of(self.active)
    }

    /// Every open document as a status bar shows it, in the order of
    /// `file.list` (`index` is the `document` number `file.activate`
    /// takes): what a row of tabs shows. Reads only; runs no command.
    pub fn document_statuses(&self) -> Vec<DocumentStatus> {
        (0..self.documents.len())
            .filter_map(|index| self.status_of(index))
            .collect()
    }

    fn status_of(&self, index: usize) -> Option<DocumentStatus> {
        let open = self.documents.get(index)?;
        let doc = open.history.current();
        Some(DocumentStatus {
            index,
            id: open.id,
            open: self.documents.len(),
            path: open.path.clone(),
            file: open.file.clone(),
            dirty: open.dirty(),
            close_asks: open.close_asks(),
            jig_in_use: open.jig_in_use,
            undo: open.history.undo_labels().last().map(str::to_string),
            redo: open.history.redo_labels().next().map(str::to_string),
            length_unit: doc.units.length.symbol(),
            angle_unit: match doc.units.angle {
                AngleUnit::Degree => "deg",
                AngleUnit::Radian => "rad",
            },
        })
    }

    /// The rules for touching files this session runs under.
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Replaces the rules for touching files. For a shell that opens the
    /// file its user named before it hands the session to automation with
    /// narrower roots.
    pub fn set_workspace(&mut self, workspace: Workspace) {
        self.workspace = workspace;
    }

    /// Writes `bytes` to `path` under the write root, atomically: for a
    /// shell's own output files (a screenshot), which are not a command's.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn write_output(&self, path: &str, bytes: &[u8]) -> Result<(), CommandError> {
        if self.writes_to_memory() {
            return Err(CommandError::new(
                "this session keeps its files in memory; leave `path` out to get the data in the reply",
            ));
        }
        let resolved = self.workspace.resolve_write(path)?;
        crate::workspace::write_atomic(&resolved, bytes)
    }

    /// There are no files to write in a browser.
    #[cfg(target_arch = "wasm32")]
    pub fn write_output(&self, _path: &str, _bytes: &[u8]) -> Result<(), CommandError> {
        Err(CommandError::new(
            "files cannot be written here; leave `path` out to get the data in the reply",
        ))
    }
}

/// Standard base64 with padding, as commands return images in JSON.
pub fn base64(bytes: &[u8]) -> String {
    crate::base64::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Portable;
    use std::cell::Cell;

    thread_local! {
        /// Makes the next builds on this thread panic.
        static FALL_OVER: Cell<bool> = const { Cell::new(false) };
        /// How many scenes this thread has started to build.
        static BUILDS: Cell<u32> = const { Cell::new(0) };
        /// Makes every body fail to tessellate on this thread, with this
        /// error.
        static TESSELLATION_FAILS: std::cell::RefCell<Option<TessError>> =
            const { std::cell::RefCell::new(None) };
        /// Makes every body fail to tessellate with the angle criterion.
        static ANGLE_FAILS: Cell<bool> = const { Cell::new(false) };
    }

    /// Called with what the tessellator made of each body: replaced by the
    /// error a test asked for.
    pub(super) fn tessellated<T>(
        meshed: Result<T, TessError>,
        with_angle: bool,
    ) -> Result<T, TessError> {
        match TESSELLATION_FAILS.with_borrow(Clone::clone) {
            Some(error) => Err(error),
            None if with_angle && ANGLE_FAILS.get() => Err(TessError::Face {
                face: 0,
                reason: "the angle criterion fails here, on purpose".into(),
            }),
            None => meshed,
        }
    }

    /// A body the angle criterion cannot be met on is drawn without it,
    /// in the display scene and in the quick first picture's fall-back
    /// alike, rather than not at all; nothing else changes.
    #[test]
    fn a_body_that_cannot_be_meshed_with_the_angle_is_drawn_without_it() {
        let mut session = Session::new(Workspace::none());
        run(&mut session, "file.new", serde_json::json!({}));
        run(
            &mut session,
            "feature.cylinder",
            serde_json::json!({"radius": 3, "height": 5}),
        );
        let quality = SceneQuality::Display {
            relative: DISPLAY_RELATIVE_TOLERANCE,
            max_triangles: DISPLAY_MAX_TRIANGLES,
            max_angle: None,
        };
        let plain = session.scene_with(quality).unwrap().unwrap();
        assert!(plain.problems.is_empty() && !plain.mesh.triangles.is_empty());
        let mut session = Session::new(Workspace::none());
        run(&mut session, "file.new", serde_json::json!({}));
        run(
            &mut session,
            "feature.cylinder",
            serde_json::json!({"radius": 3, "height": 5}),
        );
        ANGLE_FAILS.set(true);
        let scene = session.scene_with(SceneQuality::display());
        ANGLE_FAILS.set(false);
        let scene = scene.unwrap().unwrap();
        assert!(scene.problems.is_empty(), "{:?}", scene.problems);
        assert_eq!(scene.max_angle, Some(DISPLAY_MAX_ANGLE));
        // The mesh of the scene without the angle, at the same tolerance.
        assert_eq!(scene.tolerance.to_bits(), plain.tolerance.to_bits());
        assert_eq!(*scene.mesh, *plain.mesh);
    }

    /// The display rule tries coarser tolerances only for what a coarser
    /// tolerance cures (a triangle budget exceeded, a patch too fine for
    /// the tessellator), and each such try ends as soon as that is
    /// certain. A body that fails in any other way, also a triangulation
    /// that ran out of its work budget (`craft3d_tess`, a face that does
    /// not converge), is a problem of the scene after one build: its cost
    /// is not paid again for every attempt.
    #[test]
    fn the_display_rule_does_not_repeat_a_failure_coarser_tolerances_do_not_cure() {
        let mut session = Session::new(Workspace::none());
        let run = |session: &mut Session, id: &str, params: serde_json::Value| {
            session.execute(id, &params).unwrap()
        };
        run(&mut session, "file.new", serde_json::json!({}));
        run(
            &mut session,
            "feature.sphere",
            serde_json::json!({"radius": 2}),
        );
        let slow = TessError::Face {
            face: 0,
            reason: "its triangulation did not converge".into(),
        };
        TESSELLATION_FAILS.set(Some(slow));
        let builds = BUILDS.get();
        let scene = session
            .scene_with(SceneQuality::display())
            .unwrap()
            .unwrap();
        // Twice: the quick first picture, then the display scene once
        // (coarse first, then sharp, 2026-10-14); not once per attempt.
        assert_eq!(BUILDS.get(), builds + 2, "built once per stage");
        assert!(scene.bodies.is_empty() && !scene.coarsened);
        assert!(
            scene.problems.len() == 1 && scene.problems[0].contains("did not converge"),
            "{:?}",
            scene.problems
        );

        // What a coarser tolerance cures is tried again, a few times at
        // most, and then shown at the automatic tolerance.
        run(
            &mut session,
            "feature.sphere",
            serde_json::json!({"radius": 3}),
        );
        TESSELLATION_FAILS.set(Some(TessError::TooFine {
            patch: 0,
            needed_segments: 2048.0,
            max_segments: 1024,
        }));
        let builds = BUILDS.get();
        let scene = session
            .scene_with(SceneQuality::display())
            .unwrap()
            .unwrap();
        let tries = BUILDS.get() - builds;
        assert!(
            (2..=MAX_DISPLAY_ATTEMPTS as u32 + 1).contains(&tries),
            "{tries} builds"
        );
        assert!(scene.coarsened && scene.bodies.is_empty());
        TESSELLATION_FAILS.set(None);
    }

    /// Called at the start of every build: counts it, and panics when a
    /// test asked for that.
    pub(super) fn building() {
        BUILDS.set(BUILDS.get() + 1);
        if FALL_OVER.get() {
            panic!("the tessellator fell over, on purpose");
        }
    }

    /// A panic in regeneration or tessellation while a viewer asks for the
    /// scene is outside any command, so nothing else would catch it.
    #[test]
    fn a_panic_while_building_is_an_error_that_is_not_retried_every_frame() {
        let mut session = Session::new(Workspace::none());
        let run = |session: &mut Session, id: &str, params: serde_json::Value| {
            session.execute(id, &params).unwrap()
        };
        run(&mut session, "file.new", serde_json::json!({}));
        run(
            &mut session,
            "feature.sphere",
            serde_json::json!({"radius": 2}),
        );
        assert_eq!(session.scene(None).unwrap().unwrap().bodies.len(), 1);
        run(
            &mut session,
            "feature.sphere",
            serde_json::json!({"radius": 3}),
        );

        FALL_OVER.set(true);
        let builds = BUILDS.get();
        let error = session.scene(None).unwrap_err();
        assert!(
            error.message().contains("internal error")
                && error
                    .message()
                    .contains("the tessellator fell over, on purpose"),
            "{error}"
        );
        // A viewer asks again every frame: the same answer, without
        // building again.
        for _ in 0..5 {
            assert_eq!(session.scene(None).unwrap_err(), error);
        }
        assert_eq!(BUILDS.get(), builds + 1);
        // Another tolerance is another scene, and fails on its own.
        assert!(session.scene(Some(0.5)).is_err());
        assert_eq!(BUILDS.get(), builds + 2);

        // The document was kept and commands still run.
        let measured = run(&mut session, "document.measure", serde_json::json!({}));
        assert_eq!(measured["bodies"].as_array().unwrap().len(), 2);
        // The failure belongs to that state of the document: the next edit
        // gets a fresh attempt.
        FALL_OVER.set(false);
        assert!(session.scene(None).is_err(), "still the remembered failure");
        run(
            &mut session,
            "feature.sphere",
            serde_json::json!({"radius": 1}),
        );
        assert_eq!(session.scene(None).unwrap().unwrap().bodies.len(), 3);
    }

    fn run(session: &mut Session, id: &str, params: serde_json::Value) {
        session.execute(id, &params).unwrap();
    }

    /// `count` open documents, each a different solid.
    fn documents(count: usize) -> Session {
        let mut session = Session::new(Workspace::none());
        for k in 0..count {
            run(&mut session, "file.new", serde_json::json!({}));
            run(
                &mut session,
                "feature.cylinder",
                serde_json::json!({"radius": 1.0 + k as f64, "height": 3}),
            );
        }
        session
    }

    fn activate(session: &mut Session, document: usize) {
        run(
            session,
            "file.activate",
            serde_json::json!({ "document": document }),
        );
    }

    /// The developer's report: with four tabs, switching sometimes showed
    /// the model at once and sometimes meshed it again (two other
    /// documents were kept). Within the budget every open document's
    /// scenes are kept: switching among six, back and forth, builds
    /// nothing, and shows the very meshes it showed before.
    #[test]
    fn switching_among_six_open_documents_never_meshes_again_within_the_budget() {
        let mut session = documents(6);
        let mut shown = Vec::new();
        for document in 0..6 {
            activate(&mut session, document);
            shown.push(
                session
                    .scene_with(SceneQuality::display())
                    .unwrap()
                    .unwrap(),
            );
        }
        let builds = BUILDS.get();
        for round in 0..4 {
            for document in [3, 0, 5, 1, 4, 2] {
                activate(&mut session, (document + round) % 6);
                let again = session
                    .scene_with(SceneQuality::display())
                    .unwrap()
                    .unwrap();
                let before = &shown[(document + round) % 6];
                assert!(Arc::ptr_eq(&again.mesh, &before.mesh));
                assert!(Arc::ptr_eq(&again.bodies, &before.bodies));
                assert!(again.revision > before.revision);
            }
        }
        assert_eq!(BUILDS.get(), builds, "a scene was built again");
        assert!(session.scene_bytes() > 0);
    }

    /// Over the budget, the least recently shown documents' scenes go
    /// first, and the active document's are kept whatever they hold.
    #[test]
    fn over_the_budget_the_least_recently_shown_go_first() {
        let mut session = documents(3);
        for document in [0, 1, 2] {
            activate(&mut session, document);
            session.scene_with(SceneQuality::display()).unwrap();
        }
        let all = session.scene_bytes();
        // Room for about two documents' scenes: the first one shown goes.
        session.set_scene_budget(all * 3 / 4);
        assert!(session.scene_bytes() <= all * 3 / 4);
        let builds = BUILDS.get();
        activate(&mut session, 1);
        session.scene_with(SceneQuality::display()).unwrap();
        assert_eq!(BUILDS.get(), builds, "document 1 was kept");
        activate(&mut session, 0);
        session.scene_with(SceneQuality::display()).unwrap();
        assert!(BUILDS.get() > builds, "document 0 was let go");
        // A budget of nothing keeps the active document's scenes only.
        session.set_scene_budget(0);
        let builds = BUILDS.get();
        session.scene_with(SceneQuality::display()).unwrap();
        assert_eq!(BUILDS.get(), builds);
        activate(&mut session, 2);
        session.scene_with(SceneQuality::display()).unwrap();
        assert!(BUILDS.get() > builds);
    }

    /// A closed document's scenes are let go at once, budget or not.
    #[test]
    fn a_closed_documents_scenes_are_let_go() {
        let mut session = documents(2);
        session.scene_with(SceneQuality::display()).unwrap();
        activate(&mut session, 0);
        session.scene_with(SceneQuality::display()).unwrap();
        activate(&mut session, 1);
        let both = session.scene_bytes();
        run(
            &mut session,
            "file.close",
            serde_json::json!({"discard": true}),
        );
        session.scene_with(SceneQuality::display()).unwrap();
        assert!(session.scene_bytes() < both);
    }

    /// Coarse first, then sharp: with the display scene built elsewhere,
    /// the display rule answers with the quick first picture, and the
    /// refinement, run away from the session, replaces it at the same
    /// revision. The same as building it in place, bit for bit.
    #[test]
    fn the_first_picture_is_refined_elsewhere_into_the_display_scene() {
        let mut session = documents(1);
        run(
            &mut session,
            "feature.sphere",
            serde_json::json!({"radius": 2, "position": [5, 0, 0]}),
        );
        let mut in_place = session.clone();
        let sharp_in_place = in_place
            .scene_with(SceneQuality::display())
            .unwrap()
            .unwrap();

        session.set_refine_elsewhere(true);
        assert!(!session.scene_ready(SceneQuality::display()));
        let first = session
            .scene_with(SceneQuality::display())
            .unwrap()
            .unwrap();
        assert!(first.preview);
        assert!(session.scene_ready(SceneQuality::display()));
        assert!(first.triangles() < sharp_in_place.triangles());
        // Asked again: the same first picture, nothing built.
        let builds = BUILDS.get();
        let again = session
            .scene_with(SceneQuality::display())
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(BUILDS.get(), builds);

        let job = session.refinement(SceneQuality::display()).unwrap();
        assert!(job.wanted(&mut session));
        // Built on another thread while the session stays here.
        let refined = std::thread::spawn(move || job.run(&AtomicBool::new(false)))
            .join()
            .unwrap();
        assert!(!refined.cancelled());
        assert!(session.accept_refined(refined));
        let sharp = session
            .scene_with(SceneQuality::display())
            .unwrap()
            .unwrap();
        assert!(!sharp.preview);
        assert_eq!(sharp.revision, first.revision);
        assert_eq!(sharp.tolerance, sharp_in_place.tolerance);
        assert_eq!(*sharp.mesh, *sharp_in_place.mesh);
        assert_eq!(*sharp.bodies, *sharp_in_place.bodies);
        assert!(session.refinement(SceneQuality::display()).is_none());
    }

    /// A refinement that is cancelled keeps nothing; one that finishes
    /// after an edit is kept for the state it was built for, and shown
    /// without building when an undo makes that state active again.
    #[test]
    fn a_refinement_is_cancelled_or_kept_for_its_own_state() {
        let mut session = documents(1);
        session.set_refine_elsewhere(true);
        session.scene_with(SceneQuality::display()).unwrap();
        let job = session.refinement(SceneQuality::display()).unwrap();
        let cancelled = job.run(&AtomicBool::new(true));
        assert!(cancelled.cancelled());
        assert!(!session.accept_refined(cancelled));
        assert!(session.refinement(SceneQuality::display()).is_some());

        run(
            &mut session,
            "feature.block",
            serde_json::json!({"size": [1, 1, 1], "position": [9, 0, 0]}),
        );
        assert!(!job.wanted(&mut session));
        let late = job.run(&AtomicBool::new(false));
        assert!(!session.accept_refined(late));
        // The edited state shows its own first picture.
        assert!(
            session
                .scene_with(SceneQuality::display())
                .unwrap()
                .unwrap()
                .preview
        );
        run(&mut session, "edit.undo", serde_json::json!({}));
        let builds = BUILDS.get();
        let shown = session
            .scene_with(SceneQuality::display())
            .unwrap()
            .unwrap();
        assert!(!shown.preview);
        assert_eq!(BUILDS.get(), builds);
    }

    /// The ladder: every rung is `2^(k/8)` times a power of two, the
    /// nearest rung is within 4.5 % either way, a number on a rung stays
    /// where it is, and a number that is no tolerance is left alone.
    #[test]
    fn the_ladder_has_eight_rungs_to_an_octave() {
        let ratio = 2f64.ppowf(0.125);
        for k in -40..40 {
            let rung = ratio.ppowi(k);
            let on = on_rung(rung);
            assert!((on / rung - 1.0).abs() < 1e-12, "{rung} -> {on}");
            // Within the octave the rung is exact: the one `RUNGS` holds.
            let (octave, step) = (k.div_euclid(8), k.rem_euclid(8) as usize);
            assert_eq!(on, RUNGS[step] * 2f64.ppowi(octave));
            // Up to just under half a rung away, the same rung.
            for fraction in [-0.49, -0.25, 0.25, 0.49] {
                let near = rung * ratio.ppowf(fraction);
                assert_eq!(on_rung(near), on, "{near} near {rung}");
            }
        }
        for t in [1e-9, 3.7e-3, 0.0147, 0.25, 1.0, 1.99, 2.0, 12345.678, 1e9] {
            let on = on_rung(t);
            assert!((on / t - 1.0).abs() <= 0.0443, "{t} -> {on}");
            assert_eq!(on_rung(on), on, "on a rung already");
        }
        assert_eq!(on_rung(1.0), 1.0);
        assert_eq!(on_rung(2.0), 2.0);
        assert_eq!(on_rung(0.5), 0.5);
        for not in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::MIN_POSITIVE / 2.0] {
            let same = on_rung(not);
            assert!(same.to_bits() == not.to_bits(), "{not} -> {same}");
        }
        assert_eq!(on_rung(f64::MAX), f64::MAX, "no rung above the range");
    }

    /// The closed form: n chords over a sweep are each `sweep / n` wide, and
    /// a chord of width θ is r·(1 − cos(θ/2)) from the circle.
    fn sagitta(r: f64, sweep: f64, n: usize) -> f64 {
        r * (1.0 - (sweep / n as f64 / 2.0).pcos())
    }

    #[test]
    fn arc_segments_meet_the_tolerance_and_are_not_wasteful() {
        let turn = std::f64::consts::TAU;
        for r in [1e-3, 1.0, 25.0, 1e3, 1e6] {
            for relative in [1e-2, 1e-3, 1e-5] {
                for sweep in [turn, turn / 2.0, 0.3] {
                    let tolerance = r * relative;
                    let n = arc_segments(r, sweep, tolerance);
                    assert!(
                        sagitta(r, sweep, n) <= tolerance * (1.0 + 1e-9),
                        "r {r} tol {tolerance} sweep {sweep}: {n} segments"
                    );
                    // One chord fewer would not do, unless the floor for
                    // looks decided the count.
                    let least = ((24.0 * sweep / turn).ceil() as usize).max(1);
                    if n > least {
                        assert!(
                            sagitta(r, sweep, n - 1) > tolerance,
                            "r {r} tol {tolerance} sweep {sweep}: {n} is one too many"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn arc_segments_survive_hostile_numbers() {
        let turn = std::f64::consts::TAU;
        for r in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-300, 1e300] {
            for tolerance in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-300, 1e300] {
                for sweep in [0.0, turn, f64::NAN, f64::INFINITY, -3.0] {
                    let n = arc_segments(r, sweep, tolerance);
                    assert!(
                        (1..=MAX_ARC_SEGMENTS).contains(&n),
                        "{r} {tolerance} {sweep}"
                    );
                }
            }
        }
        // A coarse tolerance still draws a circle, not a triangle.
        assert_eq!(arc_segments(1.0, turn, 10.0), MIN_CIRCLE_SEGMENTS);
        assert_eq!(arc_segments(1.0, turn / 4.0, 10.0), 6);
        // A tolerance nobody can meet is capped.
        assert_eq!(arc_segments(1e6, turn, 1e-9), MAX_ARC_SEGMENTS);
    }
}
