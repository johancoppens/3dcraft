//! The scene: what a viewer draws for the active document, read-only
//! (docs/architecture.md §8.5, the engine/UI seam).
//!
//! A [`Scene`] is derived data: the tessellated mesh of every body with the
//! feature that made it, every solved sketch as polylines in space, and the
//! feature tree as plain rows. It is rebuilt when the document changes and
//! cached per document and tolerance, so a shell can ask for it every frame.
//! Nothing here can change a document: edits are commands.

use crate::{CommandError, Session, Workspace};
use craft3d_doc::{AngleUnit, Document, Entity, FeatureKind, Geometry, Sketch};
use craft3d_math::{Bbox3, Vec3};
use craft3d_regen::{Regen, SketchResult, Status};
use craft3d_tess::{Mesh, TessParams, tessellate_body_and_edges};
use craft3d_topo::mass_properties;
use std::sync::Arc;

/// Fewest segments a full circle is drawn with, however coarse the
/// tolerance: a hexagon does not read as a circle.
pub const MIN_CIRCLE_SEGMENTS: usize = 24;

/// Most segments one circle or arc is sampled with. A tolerance that would
/// need more (a chord error below about 3e-7 of the radius) is not met.
pub const MAX_ARC_SEGMENTS: usize = 4096;

/// Scenes kept for one document: one per tolerance asked for.
const MAX_CACHED: usize = 4;

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
}

/// What a polyline of a sketch stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchCurve {
    Line,
    Circle,
    Arc,
}

/// One line, circle or arc of a sketch, as points in space. A circle is
/// closed: its last point repeats the first.
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
}

/// Everything a viewer draws for one state of the active document.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// See [`Session::scene_revision`].
    pub revision: u64,
    /// The chordal tolerance the bodies and the sketch curves were sampled
    /// with, in document units.
    pub tolerance: f64,
    /// In history order.
    pub bodies: Vec<SceneBody>,
    /// All bodies as one mesh for a renderer; face ids keep counting across
    /// bodies, as in `view.render`.
    pub mesh: Mesh,
    pub sketches: Vec<SceneSketch>,
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

/// The scenes of the document last looked at.
#[derive(Clone, Default)]
pub(crate) struct SceneCache {
    /// The document the revision and the scenes belong to. Holding the
    /// `Arc` keeps its address from being reused by another document.
    seen: Option<Arc<Document>>,
    revision: u64,
    /// Keyed by the tolerance that was asked for (`None`: automatic). A
    /// scene that could not be built is remembered as its error, so a
    /// viewer asking every frame does not build it every frame.
    scenes: Vec<(Option<u64>, Built)>,
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
            self.scenes.clear();
        }
        self.revision
    }
}

/// The default chordal tolerance, as `view.render` and `file.export` choose
/// it: a thousandth of the side of a square with the bodies' total area.
/// Without bodies: a thousandth of the size of the sketches.
fn automatic_tolerance(regen: &Regen, doc: &Document) -> f64 {
    let area: f64 = regen
        .bodies()
        .filter_map(|(_, body)| mass_properties(body).ok())
        .map(|m| m.area)
        .sum();
    if area.is_finite() && area > 0.0 {
        return area.sqrt() * 1e-3;
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
                Geometry::Line { .. } | Geometry::Arc { .. } => {}
            }
        }
    }
    let size = extent.diagonal();
    if size.is_finite() && size > 0.0 {
        size * 1e-3
    } else {
        1e-3
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

fn build(doc: &Document, regen: &Regen, revision: u64, tolerance: f64) -> Scene {
    #[cfg(test)]
    tests::building();
    let tess = TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    };
    let mut scene = Scene {
        revision,
        tolerance,
        bodies: Vec::new(),
        mesh: Mesh::default(),
        sketches: Vec::new(),
        features: Vec::new(),
        problems: Vec::new(),
        bbox: Bbox3::EMPTY,
    };
    for feature in &doc.features {
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
        let Some(body) = outcome.and_then(|o| o.own_body()) else {
            continue;
        };
        // The mesh and the edges in one go: the boundaries of the faces
        // are worked out once.
        let (mesh, edges) = match tessellate_body_and_edges(body, &tess) {
            Ok(both) => match both.mesh {
                Ok(mesh) => (mesh, both.edges),
                Err(e) => {
                    scene.problems.push(format!("{}: {e}", feature.name));
                    continue;
                }
            },
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
        scene.bodies.push(SceneBody {
            feature: feature.id.to_string(),
            name: feature.name.clone(),
            mesh,
            first_face_id,
            edges,
        });
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
    scene
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
    /// picks the one `view.render` uses. A tolerance that is not a positive
    /// number is an error.
    ///
    /// Cached: asking again for the same tolerance while the document has
    /// not changed returns the same `Arc`.
    pub fn scene(&mut self, tolerance: Option<f64>) -> Result<Option<Arc<Scene>>, CommandError> {
        if let Some(t) = tolerance
            && !(t.is_finite() && t > 0.0)
        {
            return Err(CommandError::new(
                "the scene's tolerance must be a positive number",
            ));
        }
        let revision = self.scene_revision();
        let key = tolerance.map(f64::to_bits);
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
            let tolerance = tolerance.unwrap_or_else(|| automatic_tolerance(&regen, doc));
            build(doc, &regen, revision, tolerance)
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
