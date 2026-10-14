//! The scene: what a viewer draws for the active document, read-only
//! (docs/architecture.md §8.5, the engine/UI seam).
//!
//! A [`Scene`] is derived data: the tessellated mesh of every body with the
//! feature that made it, every solved sketch as polylines in space, and the
//! feature tree as plain rows. It is rebuilt when the document changes and
//! cached per document and tolerance, so a shell can ask for it every frame.
//! Nothing here can change a document: edits are commands.

use crate::mass::mass_of;
use crate::{CommandError, Session, Workspace};
use craft3d_doc::{AngleUnit, Document, Entity, FeatureKind, Geometry, Sketch};
use craft3d_math::{Bbox3, Vec3};
use craft3d_regen::{Regen, SketchResult, Status};
use craft3d_tess::{Mesh, TessError, TessParams, tessellate_body_and_edges};
use std::sync::Arc;

/// Fewest segments a full circle is drawn with, however coarse the
/// tolerance: a hexagon does not read as a circle.
pub const MIN_CIRCLE_SEGMENTS: usize = 24;

/// Most segments one circle or arc is sampled with. A tolerance that would
/// need more (a chord error below about 3e-7 of the radius) is not met.
pub const MAX_ARC_SEGMENTS: usize = 4096;

/// Scenes kept for one document: one per quality asked for.
const MAX_CACHED: usize = 4;

/// The automatic chordal tolerance, as `view.render`, `file.export` and
/// previews choose it: this fraction of the model's size (the side of a
/// square with the bodies' total area; see [`SceneQuality::Automatic`]).
pub const AUTOMATIC_RELATIVE_TOLERANCE: f64 = 1e-3;

/// The display rule's default chordal tolerance, as a fraction of the
/// model's size: 3e-5, a chord error of a thirtieth of a pixel on a model
/// that fills a 1020-pixel viewport, so a circle stays round when the
/// developer zooms in on a detail (docs/architecture.md §6, *What a viewer
/// draws*).
pub const DISPLAY_RELATIVE_TOLERANCE: f64 = 3e-5;

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

/// How finely the bodies and the sketch curves of a scene are sampled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SceneQuality {
    /// The chordal tolerance `view.render`, `file.export` and previews use
    /// when none is given: [`AUTOMATIC_RELATIVE_TOLERANCE`] of the model's
    /// size. Without bodies, the same fraction of the size of the sketches.
    Automatic,
    /// A chordal tolerance in document units.
    Tolerance(f64),
    /// What a viewport shows: a chordal tolerance of `relative` times the
    /// model's size, coarsened until the scene has at most `max_triangles`
    /// triangles and every body is within what the tessellator can do, but
    /// never beyond the automatic tolerance (the display is never coarser
    /// than `view.render`). The scene says whether it was coarsened
    /// ([`Scene::coarsened`]).
    Display { relative: f64, max_triangles: usize },
}

impl SceneQuality {
    /// The display rule with its defaults.
    pub fn display() -> Self {
        SceneQuality::Display {
            relative: DISPLAY_RELATIVE_TOLERANCE,
            max_triangles: DISPLAY_MAX_TRIANGLES,
        }
    }

    /// Refuses a quality that is not a request at all: a tolerance or a
    /// fraction that is not a positive number, a budget of nothing.
    fn validate(self) -> Result<(), CommandError> {
        match self {
            SceneQuality::Automatic => Ok(()),
            SceneQuality::Tolerance(t) if t.is_finite() && t > 0.0 => Ok(()),
            SceneQuality::Tolerance(_) => Err(CommandError::new(
                "the scene's tolerance must be a positive number",
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
            SceneQuality::Display { .. } => Ok(()),
        }
    }

    /// What the cache is keyed by.
    fn key(self) -> SceneKey {
        match self {
            SceneQuality::Automatic => SceneKey::Automatic,
            SceneQuality::Tolerance(t) => SceneKey::Tolerance(t.to_bits()),
            SceneQuality::Display {
                relative,
                max_triangles,
            } => SceneKey::Display {
                relative: relative.to_bits(),
                max_triangles,
            },
        }
    }
}

/// A [`SceneQuality`] with its numbers as bits, so it can be compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SceneKey {
    Automatic,
    Tolerance(u64),
    Display { relative: u64, max_triangles: usize },
}

/// One body of the scene.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneBody {
    /// The feature that made the body, as `document.inspect` writes it
    /// (`F3`).
    pub feature: String,
    pub name: String,
    pub mesh: Mesh,
    /// The body's face ids in [`Scene::mesh`] start here.
    pub first_face_id: u32,
    /// The body's edges as polylines in space, one per edge in the order
    /// of the B-rep, sampled with the scene's tolerance. They are the
    /// points the mesh has along each edge, so a line drawn through them
    /// lies on the border between the faces it separates.
    pub edges: Vec<Vec<Vec3>>,
    /// The persistent name of every face, as `document.inspect` lists them
    /// under `faceNames`: face `k` of the body has id `first_face_id + k`
    /// in [`Scene::mesh`]. Empty when the body's faces have no names.
    pub face_names: Vec<String>,
    /// Per face, the other names it answers to (`faceAliases` of
    /// `document.inspect`). Empty when no face of the body has any.
    pub face_aliases: Vec<Vec<String>>,
    /// The persistent name of every edge, in the order of [`Self::edges`],
    /// as `document.edges` gives it under `name`. Empty when the body's
    /// edges have no names of their own (after a boolean): an edge is then
    /// referred to by the faces it lies between.
    pub edge_names: Vec<String>,
    /// Per edge, the faces it lies between, as their places among the
    /// faces of the body, ascending: two for an edge of a solid, one for a
    /// seam. What `document.edges` lists under `faces`.
    pub edge_faces: Vec<Vec<usize>>,
    /// The body's vertices, in the order of the B-rep.
    pub vertices: Vec<Vec3>,
    /// The persistent name of every vertex, in the order of
    /// [`Self::vertices`], as `document.edges` gives it under `vertices`.
    /// Empty when the body's vertices have no names of their own (after a
    /// boolean): a vertex is then referred to by the faces that meet in it.
    pub vertex_names: Vec<String>,
    /// Per vertex, the faces that meet in it (the faces of the edges that
    /// end there), as their places among the faces of the body, ascending.
    pub vertex_faces: Vec<Vec<usize>>,
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
    Plane {
        center: Vec3,
        u: Vec3,
        v: Vec3,
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
    RolledBack,
    Failed,
}

impl FeatureState {
    /// The word `document.inspect` uses.
    pub fn name(self) -> &'static str {
        match self {
            FeatureState::Ok => "ok",
            FeatureState::Suppressed => "suppressed",
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
}

/// Everything a viewer draws for one state of the active document.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// See [`Session::scene_revision`].
    pub revision: u64,
    /// The chordal tolerance the bodies and the sketch curves were sampled
    /// with, in document units.
    pub tolerance: f64,
    /// True when the display rule ([`SceneQuality::Display`]) had to
    /// coarsen its tolerance to stay within its triangle budget or within
    /// what the tessellator can do. False for the other qualities.
    pub coarsened: bool,
    /// In history order.
    pub bodies: Vec<SceneBody>,
    /// All bodies as one mesh for a renderer; face ids keep counting across
    /// bodies, as in `view.render`.
    pub mesh: Mesh,
    pub sketches: Vec<SceneSketch>,
    /// The datums that are placed, in history order. They are not in
    /// [`Scene::bbox`]: they are sized to it.
    pub datums: Vec<SceneDatum>,
    pub features: Vec<SceneFeature>,
    /// Bodies that could not be tessellated, as sentences.
    pub problems: Vec<String>,
    /// The box around the meshes and the sketch geometry.
    pub bbox: Bbox3,
}

impl Scene {
    pub fn triangles(&self) -> usize {
        self.mesh.triangles.len()
    }
}

/// The active document as a status bar shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentStatus {
    /// Its index among the open documents.
    pub index: usize,
    /// How many documents are open.
    pub open: usize,
    /// The path it was opened from or last saved to.
    pub path: Option<String>,
    /// True when it has changes that are not on disk.
    pub dirty: bool,
    pub length_unit: &'static str,
    pub angle_unit: &'static str,
}

/// A scene, or why it could not be built.
type Built = Result<Arc<Scene>, CommandError>;

/// The scenes of one document, by the quality asked for.
type Scenes = Vec<(SceneKey, Built)>;

/// The scenes of the document last looked at.
#[derive(Clone, Default)]
pub(crate) struct SceneCache {
    /// The document the revision and the scenes belong to. Holding the
    /// `Arc` keeps its address from being reused by another document.
    seen: Option<Arc<Document>>,
    revision: u64,
    /// Keyed by the quality that was asked for. A scene that could not be
    /// built is remembered as its error, so a viewer asking every frame
    /// does not build it every frame.
    scenes: Scenes,
    /// The scenes of documents looked at before, newest last: making one
    /// of them active again (`file.activate`) shows its scenes again
    /// instead of meshing it anew, as long as it has not changed (the same
    /// `Arc`).
    parked: Vec<(Arc<Document>, Scenes)>,
}

/// Most other documents whose scenes are kept.
const MAX_PARKED: usize = 2;

impl SceneCache {
    /// Bumps the revision when `current` is not the document seen last.
    fn observe(&mut self, current: Option<&Arc<Document>>) -> u64 {
        let same = match (&self.seen, current) {
            (None, None) => true,
            (Some(seen), Some(current)) => Arc::ptr_eq(seen, current),
            _ => false,
        };
        if !same {
            let left = std::mem::take(&mut self.scenes);
            if let Some(seen) = self.seen.take()
                && !left.is_empty()
            {
                if self.parked.len() >= MAX_PARKED {
                    drop(self.parked.remove(0));
                }
                self.parked.push((seen, left));
            }
            self.seen = current.cloned();
            self.revision = self.revision.saturating_add(1);
            let revision = self.revision;
            if let Some(current) = current
                && let Some(at) = self
                    .parked
                    .iter()
                    .position(|(doc, _)| Arc::ptr_eq(doc, current))
            {
                let (_, scenes) = self.parked.remove(at);
                // The same scenes under the revision they are shown at now.
                self.scenes = scenes
                    .into_iter()
                    .map(|(key, built)| {
                        let built = built.map(|scene| {
                            Arc::new(Scene {
                                revision,
                                ..(*scene).clone()
                            })
                        });
                        (key, built)
                    })
                    .collect();
            }
        }
        self.revision
    }
}

/// The size of the model that the relative tolerances are fractions of:
/// the side of a square with the bodies' total area. Without bodies, the
/// diagonal of the box around the sketches' points and circles; without
/// those, one unit.
fn model_size(regen: &Regen, doc: &Document) -> f64 {
    let area: f64 = regen
        .bodies()
        .filter_map(|(_, body)| mass_of(body).ok())
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
                Geometry::Point { .. } => {
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
    let step = 2.0 * (1.0 - tolerance / r).acos();
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
            solved.to_world(cx + r * angle.cos(), cy + r * angle.sin())
        })
        .collect()
}

fn polyline(solved: &SketchResult, entity: &Entity, tolerance: f64) -> Option<ScenePolyline> {
    let (curve, points) = match entity.geometry {
        Geometry::Point { .. } => return None,
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
            let r = (a.0 - c.0).hypot(a.1 - c.1);
            let from = (a.1 - c.1).atan2(a.0 - c.0);
            let to = (b.1 - c.1).atan2(b.0 - c.0);
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

fn scene_sketch(
    feature: &craft3d_doc::Feature,
    sketch: &Sketch,
    solved: &SketchResult,
    tolerance: f64,
) -> SceneSketch {
    let polylines = sketch
        .entities
        .iter()
        .filter_map(|entity| polyline(solved, entity, tolerance))
        .collect();
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
        2.0 * (1.0 - tolerance / helix.radius).acos()
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
fn build(
    doc: &Document,
    regen: &Regen,
    revision: u64,
    tolerance: f64,
    budget: Option<usize>,
) -> (Scene, Option<Stop>) {
    #[cfg(test)]
    tests::building();
    let tess = TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    };
    let mut scene = Scene {
        revision,
        tolerance,
        coarsened: false,
        bodies: Vec::new(),
        mesh: Mesh::default(),
        sketches: Vec::new(),
        datums: Vec::new(),
        features: Vec::new(),
        problems: Vec::new(),
        bbox: Bbox3::EMPTY,
    };
    // How much of the model is done, for the estimate when the budget is
    // exceeded: by faces, which every body has and which cost nothing to
    // count.
    let faces_total: usize = if budget.is_some() {
        doc.features
            .iter()
            .filter_map(|f| regen.feature(f.id).and_then(|o| o.own_body()))
            .fold(0, |n, body| n.saturating_add(body.faces.len()))
    } else {
        0
    };
    let mut faces_done: usize = 0;
    let steps = doc.features.len();
    for (index, feature) in doc.features.iter().enumerate() {
        let outcome = regen.feature(feature.id);
        let (state, error) = match outcome.map(|o| &o.status) {
            Some(Status::Ok) => (FeatureState::Ok, None),
            Some(Status::Suppressed) => (FeatureState::Suppressed, None),
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
        });
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
        // The mesh and the edges in one go: the boundaries of the faces
        // are worked out once.
        craft3d_regen::progress::report(&craft3d_regen::progress::Progress {
            phase: "meshing",
            feature: Some(feature.id.to_string()),
            name: Some(feature.name.clone()),
            step: index.saturating_add(1),
            steps,
        });
        let left = budget.map(|max| max.saturating_sub(scene.mesh.triangles.len()));
        let tess = TessParams {
            max_triangles: left,
            ..tess
        };
        let meshed = tessellate_body_and_edges(body, &tess)
            .and_then(|both| both.mesh.map(|mesh| (mesh, both.edges)));
        #[cfg(test)]
        let meshed = tests::tessellated(meshed);
        let (mesh, edges) = match meshed {
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
        let Ok(vertex_base) = u32::try_from(scene.mesh.positions.len()) else {
            scene
                .problems
                .push(format!("{}: the scene has too many vertices", feature.name));
            continue;
        };
        let first_face_id = scene
            .mesh
            .face_ids
            .iter()
            .copied()
            .max()
            .map_or(0, |m| m.saturating_add(1));
        scene.mesh.positions.extend(&mesh.positions);
        scene.mesh.normals.extend(&mesh.normals);
        scene.mesh.triangles.extend(
            mesh.triangles
                .iter()
                .map(|t| t.map(|k| k.saturating_add(vertex_base))),
        );
        scene.mesh.face_ids.extend(
            mesh.face_ids
                .iter()
                .map(|f| f.saturating_add(first_face_id)),
        );
        let names = outcome.and_then(|o| o.names.as_ref());
        let face_names: Vec<String> = names
            .map(|names| names.faces.iter().map(ToString::to_string).collect())
            .unwrap_or_default();
        let face_aliases: Vec<Vec<String>> = names
            .filter(|names| !names.aliases.is_empty())
            .map(|names| {
                (0..names.faces.len())
                    .map(|face| {
                        names
                            .aliases_of(face)
                            .iter()
                            .map(ToString::to_string)
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_default();
        let edge_names: Vec<String> = names
            .map(|names| names.edges.iter().map(ToString::to_string).collect())
            .unwrap_or_default();
        let edge_faces: Vec<Vec<usize>> = craft3d_regen::faces_of_edges(body)
            .into_iter()
            .map(|faces| faces.into_iter().map(|face| face.0 as usize).collect())
            .collect();
        let vertices: Vec<Vec3> = body.vertices.iter().map(|v| v.point).collect();
        let vertex_names: Vec<String> = names
            .map(|names| names.vertices.iter().map(ToString::to_string).collect())
            .unwrap_or_default();
        let mut vertex_faces: Vec<Vec<usize>> = vec![Vec::new(); vertices.len()];
        for (edge, faces) in body.edges.iter().zip(&edge_faces) {
            for end in [edge.start, edge.end] {
                if let Some(list) = vertex_faces.get_mut(end.0 as usize) {
                    list.extend(
                        faces
                            .iter()
                            .filter(|face| !list.contains(face))
                            .copied()
                            .collect::<Vec<_>>(),
                    );
                }
            }
        }
        for list in &mut vertex_faces {
            list.sort_unstable();
        }
        scene.bodies.push(SceneBody {
            feature: feature.id.to_string(),
            name: feature.name.clone(),
            mesh,
            first_face_id,
            edges,
            face_names,
            face_aliases,
            edge_names,
            edge_faces,
            vertices,
            vertex_names,
            vertex_faces,
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
    scene.datums = scene_datums(doc, regen, &bbox);
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
                (Status::Ok, Some(datum)) => Some((feature, datum)),
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
                    finite(&[center, u, v]).then_some(SceneDatumShape::Plane { center, u, v })?
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

/// The display rule: the scene at `fine`, or at the coarsest tolerance
/// between `fine` and `automatic` that keeps the scene within
/// `max_triangles` triangles and within the tessellator's reach. Each
/// attempt that stops early says how far over it was, and the next
/// tolerance is coarser by that ratio (triangles grow as the inverse of
/// the tolerance at worst, so the estimate errs towards another attempt,
/// never towards a coarser mesh than needed), at least twice as coarse
/// and at most 64 times. At or beyond `automatic` no budget applies: the
/// display is never coarser than `view.render`.
fn build_for_display(
    doc: &Document,
    regen: &Regen,
    revision: u64,
    fine: f64,
    automatic: f64,
    max_triangles: usize,
) -> Scene {
    let mut tolerance = fine;
    let mut attempts = 0;
    while tolerance < automatic && attempts < MAX_DISPLAY_ATTEMPTS {
        attempts += 1;
        let (scene, stop) = build(doc, regen, revision, tolerance, Some(max_triangles));
        let Some(stop) = stop else {
            return Scene {
                coarsened: tolerance > fine,
                ..scene
            };
        };
        let factor = match stop {
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
                (triangles as f64 / share) / (max_triangles as f64)
            }
            // Segments on a patch grow as the inverse square root of the
            // tolerance; a tenth more for the rounding of the count.
            Stop::TooFine { needed, max } => (needed / f64::from(max.max(1)) * 1.1).powi(2),
        };
        let factor = if factor.is_finite() {
            factor.clamp(2.0, 64.0)
        } else {
            4.0
        };
        tolerance = (tolerance * factor).min(automatic);
    }
    // At the automatic tolerance, out of attempts below it, or asked for
    // something coarser from the start: no budget.
    let tolerance = if tolerance.is_finite() && tolerance >= automatic {
        tolerance
    } else {
        automatic
    };
    let (scene, _) = build(doc, regen, revision, tolerance, None);
    Scene {
        coarsened: tolerance > fine,
        ..scene
    }
}

impl Session {
    /// A number that changes exactly when the active document does: after a
    /// command that edited it, an undo or a redo, and when another document
    /// becomes the active one. Commands that only read, and commands that
    /// fail, leave it alone.
    ///
    /// It counts the changes this function has noticed, so two edits
    /// between two calls move it by one.
    pub fn scene_revision(&mut self) -> u64 {
        let current = self
            .documents
            .get(self.active)
            .map(|open| open.history.current());
        self.scene.observe(current)
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

    /// [`Self::scene`] for any [`SceneQuality`]: what a viewport asks for
    /// with [`SceneQuality::Display`]. Cached per quality: asking again for
    /// the same quality while the document has not changed returns the
    /// same `Arc`.
    pub fn scene_with(
        &mut self,
        quality: SceneQuality,
    ) -> Result<Option<Arc<Scene>>, CommandError> {
        quality.validate()?;
        let revision = self.scene_revision();
        let key = quality.key();
        if let Some((_, scene)) = self.scene.scenes.iter().find(|(k, _)| *k == key) {
            return scene.clone().map(Some);
        }
        let Some(open) = self.documents.get_mut(self.active) else {
            return Ok(None);
        };
        // Regeneration and tessellation run here, outside any command: a
        // panic in them must not take the viewer down (AGENTS.md,
        // last-resort guard). The document is not touched either way.
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let regen = open.regen();
            let doc = open.history.current();
            craft3d_regen::progress::report(&craft3d_regen::progress::Progress {
                phase: "measuring",
                feature: None,
                name: None,
                step: 0,
                steps: 0,
            });
            let size = model_size(&regen, doc);
            let automatic = size * AUTOMATIC_RELATIVE_TOLERANCE;
            match quality {
                SceneQuality::Automatic => build(doc, &regen, revision, automatic, None).0,
                SceneQuality::Tolerance(t) => build(doc, &regen, revision, t, None).0,
                SceneQuality::Display {
                    relative,
                    max_triangles,
                } => build_for_display(
                    doc,
                    &regen,
                    revision,
                    size * relative,
                    automatic,
                    max_triangles,
                ),
            }
        }));
        let scene = built.map(Arc::new).map_err(|payload| {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "no details".to_string());
            CommandError::new(format!(
                "the scene could not be built: internal error (a bug in 3DCraft, please report it): {detail}"
            ))
        });
        if self.scene.scenes.len() >= MAX_CACHED {
            // The oldest goes; whether it was a scene or a failure.
            drop(self.scene.scenes.remove(0));
        }
        self.scene.scenes.push((key, scene.clone()));
        scene.map(Some)
    }

    /// True when [`Self::scene_with`] for `quality` would answer at once:
    /// the scene is built (or known to fail), no document is open, or the
    /// quality is refused. A host that keeps a window drawing builds the
    /// scene elsewhere when this is false.
    pub fn scene_ready(&mut self, quality: SceneQuality) -> bool {
        if quality.validate().is_err() || !self.has_document() {
            return true;
        }
        self.scene_revision();
        let key = quality.key();
        self.scene.scenes.iter().any(|(k, _)| *k == key)
    }

    /// The active document as a status bar shows it.
    pub fn document_status(&self) -> Option<DocumentStatus> {
        let open = self.documents.get(self.active)?;
        let doc = open.history.current();
        Some(DocumentStatus {
            index: self.active,
            open: self.documents.len(),
            path: open.path.clone(),
            dirty: open.dirty(),
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
    }

    /// Called with what the tessellator made of each body: replaced by the
    /// error a test asked for.
    pub(super) fn tessellated<T>(meshed: Result<T, TessError>) -> Result<T, TessError> {
        match TESSELLATION_FAILS.with_borrow(Clone::clone) {
            Some(error) => Err(error),
            None => meshed,
        }
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
        assert_eq!(BUILDS.get(), builds + 1, "built once");
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

    /// The closed form: n chords over a sweep are each `sweep / n` wide, and
    /// a chord of width θ is r·(1 − cos(θ/2)) from the circle.
    fn sagitta(r: f64, sweep: f64, n: usize) -> f64 {
        r * (1.0 - (sweep / n as f64 / 2.0).cos())
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
