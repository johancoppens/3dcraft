//! Which way a viewer looks at what it flies to, and what it lights for a
//! feature: the feature that made a face, the faces of a scene a selection
//! stands for, the bodies they lie in, the direction from which they are
//! best seen, and the features a scene shows anything of. View-only:
//! nothing here changes a document.

use crate::scene::Scene;
use crate::selection::{SelectionItem, SelectionKind};
use craft3d_doc::{MAX_NAME_DEPTH, NameRole, PersistentName};
use craft3d_math::{Bbox3, Vec3};
use std::collections::BTreeSet;
use std::ops::Range;

/// How many points of the target [`Scene::best_view`] checks for being
/// hidden, per direction: the middles of its triangles turned most
/// towards it.
pub const VIEW_SAMPLES: usize = 8;

/// The most triangles [`Scene::best_view`] tests rays against, in all:
/// past it, what is left untested counts as not hiding the target, so a
/// flight on a model of millions of triangles stays quick.
pub const VIEW_RAY_BUDGET: usize = 4_000_000;

/// Triangles [`Scene::best_view`] keeps a box for together: a ray that
/// misses the box skips them all.
const VIEW_CHUNK: usize = 64;

/// The kinds of feature that make instances of a body: a face of an
/// instance other than the first (`copy(k)`, k not 0) was made by the
/// feature, not by the seed.
pub const COPYING_KINDS: [&str; 3] = ["pattern", "mirror", "transform"];

impl Scene {
    /// One past the highest face id in [`Scene::mesh`].
    fn face_id_end(&self) -> u32 {
        self.mesh
            .face_ids
            .iter()
            .max()
            .map_or(0, |&id| id.saturating_add(1))
    }

    /// The ids in [`Scene::mesh`] of the faces of body `index`.
    pub fn body_face_ids(&self, index: usize) -> Range<u32> {
        self.body_face_ids_to(index, self.face_id_end())
    }

    /// [`Self::body_face_ids`] with the end of the ids worked out once.
    fn body_face_ids_to(&self, index: usize, end_of_ids: u32) -> Range<u32> {
        let first = self.bodies.get(index).map_or(0, |b| b.first_face_id);
        let end = self
            .bodies
            .get(index.saturating_add(1))
            .map_or(end_of_ids, |b| b.first_face_id);
        first..end.max(first)
    }

    /// The ids (`F9`) of the features that make instances
    /// ([`COPYING_KINDS`]).
    fn copying(&self) -> BTreeSet<&str> {
        self.features
            .iter()
            .filter(|f| COPYING_KINDS.contains(&f.kind))
            .map(|f| f.id.as_str())
            .collect()
    }

    /// The feature that made the entity named `name`: the feature of the
    /// outermost link that makes an instance (`F9:copy(2)/F2:face(1)`, a
    /// face of the third instance of pattern `F9`, was made by `F9`),
    /// else the feature at the end of its chain of copies
    /// (`F4:copy(0)/F2:face(2)` and the seed's `F9:copy(0)/F2:face(1)`
    /// were made by `F2`).
    fn maker_of(name: &PersistentName, copying: &BTreeSet<&str>) -> String {
        let mut link = name;
        for _ in 0..MAX_NAME_DEPTH {
            if link.role == NameRole::Copy && link.sources.iter().any(|&k| k != 0) {
                let id = link.feature.to_string();
                if copying.contains(id.as_str()) {
                    return id;
                }
            }
            match &link.of {
                Some(next) => link = next,
                None => break,
            }
        }
        name.root().feature.to_string()
    }

    /// The feature (by id, `F3`) that made the face, edge or vertex
    /// named `name` (see [`Self::maker_of`]). An edge or a vertex named
    /// by the faces it lies on was made by the latest of the features
    /// that made those faces, in the order of the history: it came to be
    /// with the last of them. `None` for text that is no persistent name.
    pub fn made_by(&self, name: &str) -> Option<String> {
        let name: PersistentName = name.parse().ok()?;
        let copying = self.copying();
        let root = name.root();
        if root.role.is_by_faces() {
            let place = |id: &str| self.features.iter().position(|f| f.id == id);
            let latest = root
                .faces()
                .iter()
                .map(|face| Self::maker_of(face, &copying))
                .filter_map(|id| place(&id).map(|at| (at, id)))
                .max_by_key(|(at, _)| *at);
            if let Some((_, id)) = latest {
                return Some(id);
            }
        }
        Some(Self::maker_of(&name, &copying))
    }

    /// Every face of the scene with the feature that made it, by id in
    /// [`Scene::mesh`], ascending.
    fn face_makers(&self) -> Vec<(u32, String)> {
        let copying = self.copying();
        let mut out = Vec::new();
        for body in self.bodies.iter() {
            for (k, name) in body.face_names.iter().enumerate() {
                let Ok(name) = name.parse::<PersistentName>() else {
                    continue;
                };
                if let Ok(k) = u32::try_from(k)
                    && let Some(face) = body.first_face_id.checked_add(k)
                {
                    out.push((face, Self::maker_of(&name, &copying)));
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// The ids in [`Scene::mesh`] of the faces feature `feature` (by id
    /// or name) made, wherever they are now ([`Self::made_by`]: the
    /// instances of a pattern are the pattern's, the seed's own faces the
    /// seed's); ascending. Empty for a feature that made no face of its
    /// own (a boolean, a sketch) and for one the scene does not show.
    pub fn feature_face_ids(&self, feature: &str) -> Vec<u32> {
        let id = self
            .features
            .iter()
            .find(|f| f.id == feature)
            .or_else(|| self.features.iter().find(|f| f.name == feature))
            .map(|f| f.id.clone());
        let Some(id) = id else {
            return Vec::new();
        };
        self.face_makers()
            .into_iter()
            .filter(|(_, maker)| *maker == id)
            .map(|(face, _)| face)
            .collect()
    }

    /// The faces `items` stand for, as ids in [`Scene::mesh`], ascending:
    /// a face; the faces a feature made, or its body's when it made none;
    /// every face of a body. Edges, vertices, sketches and datums have
    /// none.
    pub fn items_face_ids(&self, items: &[SelectionItem]) -> Vec<u32> {
        let mut ids = Vec::new();
        let end_of_ids = self.face_id_end();
        let body_of = |feature: &str| {
            self.bodies
                .iter()
                .position(|b| b.feature == feature)
                .or_else(|| {
                    let id = &self.features.iter().find(|f| f.name == feature)?.id;
                    self.bodies.iter().position(|b| &b.feature == id)
                })
        };
        for item in items {
            let feature = item.feature.as_str();
            match item.kind {
                SelectionKind::Body => {
                    if let Some(index) = body_of(feature) {
                        ids.extend(self.body_face_ids_to(index, end_of_ids));
                    }
                }
                SelectionKind::Feature | SelectionKind::Sketch | SelectionKind::Datum => {
                    let made = self.feature_face_ids(feature);
                    if made.is_empty() {
                        if let Some(index) = body_of(feature) {
                            ids.extend(self.body_face_ids_to(index, end_of_ids));
                        }
                    } else {
                        ids.extend(made);
                    }
                }
                SelectionKind::Face => {
                    if let Ok(found) = self.faces_named(Some(feature), &item.name) {
                        ids.extend(found.ids);
                    }
                }
                SelectionKind::Edge | SelectionKind::Vertex => {}
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// The box around the bodies that hold any of the faces `ids`
    /// (ascending): what a feature belongs to, for the context a flight
    /// keeps around it.
    pub fn bodies_box_of(&self, ids: &[u32]) -> Bbox3 {
        let mut bbox = Bbox3::EMPTY;
        let end_of_ids = self.face_id_end();
        for (index, body) in self.bodies.iter().enumerate() {
            let range = self.body_face_ids_to(index, end_of_ids);
            let holds = ids.binary_search(&range.start).map_or_else(
                |at| ids.get(at).is_some_and(|id| range.contains(id)),
                |_| true,
            );
            if holds && let Some((lo, hi)) = body.mesh.bbox().corners() {
                bbox.add(lo);
                bbox.add(hi);
            }
        }
        bbox
    }

    /// Of the directions `toward` (from the model towards the eye, any
    /// length but zero), the one from which the faces `ids` (ascending)
    /// are best seen, by its index: the most area turned towards it (the
    /// sum over the faces' triangles of area times the cosine to the
    /// direction, where positive), times the share of that area, over the
    /// [`VIEW_SAMPLES`] triangles turned most towards it, whose middle is
    /// not hidden along it by the rest of the model. Each score is times
    /// its weight in `weights` (1 where it has none; a viewer prefers some
    /// directions). Ties go to the earlier direction. `None` when no face
    /// is there or none is seen from any direction.
    ///
    /// The rays are tested against boxes of [`VIEW_CHUNK`] triangles
    /// first, and against at most [`VIEW_RAY_BUDGET`] triangles in all:
    /// past that, what is left counts as open.
    pub fn best_view(&self, ids: &[u32], toward: &[Vec3], weights: &[f64]) -> Option<usize> {
        let mesh = &self.mesh;
        // Which triangles are the target's, worked out once.
        let target_mask: Vec<bool> = mesh
            .face_ids
            .iter()
            .map(|id| ids.binary_search(id).is_ok())
            .collect();
        let mut target: Vec<(usize, Vec3)> = Vec::new();
        for (t, _) in target_mask.iter().enumerate().filter(|(_, mine)| **mine) {
            if let Some([a, b, c]) = mesh.corners(t) {
                // Twice the area along the outward normal.
                let normal = (b - a).cross(c - a);
                if normal.length().is_finite() && normal.length() > 0.0 {
                    target.push((t, normal));
                }
            }
        }
        if target.is_empty() {
            return None;
        }
        let blockers = Blockers::new(self, &target_mask);
        let diagonal = self.bbox.diagonal();
        let reach = if diagonal.is_finite() && diagonal > 0.0 {
            diagonal * 2.0
        } else {
            1.0
        };
        let lift = reach * 1e-7;
        let mut budget = VIEW_RAY_BUDGET;
        let mut best: Option<(usize, f64)> = None;
        for (index, direction) in toward.iter().enumerate() {
            let Some(d) = direction.normalized() else {
                continue;
            };
            // Each triangle's area seen along `d` (projected).
            let mut turned: Vec<(usize, f64)> = target
                .iter()
                .map(|&(t, normal)| (t, (normal.dot(d) * 0.5).max(0.0)))
                .filter(|(_, seen)| seen.is_finite() && *seen > 0.0)
                .collect();
            let facing: f64 = turned.iter().map(|(_, seen)| seen).sum();
            if !(facing.is_finite() && facing > 0.0) {
                continue;
            }
            // Of the triangles turned most towards it, how much is not
            // hidden by the rest of the model.
            turned.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
            let (mut sampled, mut open) = (0.0, 0.0);
            for &(t, seen) in turned.iter().take(VIEW_SAMPLES) {
                let Some([a, b, c]) = mesh.corners(t) else {
                    continue;
                };
                let middle = (a + b + c) * (1.0 / 3.0);
                sampled += seen;
                if !blockers.hit(middle + d * lift, d, reach, &mut budget) {
                    open += seen;
                }
            }
            let visible = if sampled > 0.0 { open / sampled } else { 0.0 };
            let weight = weights
                .get(index)
                .copied()
                .filter(|w| w.is_finite() && *w > 0.0);
            let score = facing * weight.unwrap_or(1.0) * (visible + 0.001);
            if best.is_none_or(|(_, top)| score > top) {
                best = Some((index, score));
            }
        }
        best.map(|(index, _)| index)
    }

    /// The features the scene shows something of: a face they made
    /// ([`Self::made_by`]), a body, a sketch or a datum. What a viewer
    /// may fly to.
    pub fn shown_features(&self) -> BTreeSet<String> {
        let mut shown: BTreeSet<String> = self
            .face_makers()
            .into_iter()
            .map(|(_, maker)| maker)
            .collect();
        shown.extend(self.bodies.iter().map(|b| b.feature.clone()));
        shown.extend(self.sketches.iter().map(|s| s.feature.clone()));
        shown.extend(self.datums.iter().map(|d| d.feature.clone()));
        shown
    }
}

/// The triangles that may hide a target, in chunks with a box each.
struct Blockers<'a> {
    scene: &'a Scene,
    /// Per chunk: its first triangle, one past its last, and its box
    /// (empty when every triangle in it is the target's).
    chunks: Vec<(usize, usize, Bbox3)>,
    target: &'a [bool],
}

impl<'a> Blockers<'a> {
    fn new(scene: &'a Scene, target: &'a [bool]) -> Self {
        let count = scene.mesh.triangles.len();
        let mut chunks = Vec::with_capacity(count / VIEW_CHUNK + 1);
        let mut first = 0;
        while first < count {
            let end = first.saturating_add(VIEW_CHUNK).min(count);
            let mut bbox = Bbox3::EMPTY;
            for t in first..end {
                if target.get(t).copied().unwrap_or(false) {
                    continue;
                }
                if let Some(corners) = scene.mesh.corners(t) {
                    corners.into_iter().for_each(|p| bbox.add(p));
                }
            }
            chunks.push((first, end, bbox));
            first = end;
        }
        Self {
            scene,
            chunks,
            target,
        }
    }

    /// Whether a triangle not of the target lies on the ray from `from`
    /// along `d` (unit) within `reach`; false once `budget` triangle
    /// tests are spent.
    fn hit(&self, from: Vec3, d: Vec3, reach: f64, budget: &mut usize) -> bool {
        for &(first, end, bbox) in &self.chunks {
            if !ray_meets_box(from, d, reach, &bbox) {
                continue;
            }
            for t in first..end {
                if self.target.get(t).copied().unwrap_or(true) {
                    continue;
                }
                if *budget == 0 {
                    return false;
                }
                *budget -= 1;
                if self
                    .scene
                    .mesh
                    .corners(t)
                    .is_some_and(|[a, b, c]| hits(from, d, reach, a, b, c))
                {
                    return true;
                }
            }
        }
        false
    }
}

/// Whether the ray from `from` along `d` meets `bbox` within `reach`
/// (slabs); an empty box never.
fn ray_meets_box(from: Vec3, d: Vec3, reach: f64, bbox: &Bbox3) -> bool {
    let Some((lo, hi)) = bbox.corners() else {
        return false;
    };
    let (mut near, mut far) = (0.0_f64, reach);
    for (o, dir, l, h) in [
        (from.x, d.x, lo.x, hi.x),
        (from.y, d.y, lo.y, hi.y),
        (from.z, d.z, lo.z, hi.z),
    ] {
        if dir.abs() < 1e-300 {
            if o < l || o > h {
                return false;
            }
            continue;
        }
        let (a, b) = ((l - o) / dir, (h - o) / dir);
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        near = near.max(a);
        far = far.min(b);
        if near > far || near.is_nan() || far.is_nan() {
            return false;
        }
    }
    true
}

/// Whether the ray from `from` along `d` meets the triangle `a b c` at a
/// distance in (0, `reach`] (Möller and Trumbore).
fn hits(from: Vec3, d: Vec3, reach: f64, a: Vec3, b: Vec3, c: Vec3) -> bool {
    let (e1, e2) = (b - a, c - a);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if !det.is_finite() || det.abs() < 1e-300 {
        return false;
    }
    let inv = 1.0 / det;
    let s = from - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return false;
    }
    let t = e2.dot(q) * inv;
    t > 0.0 && t <= reach
}

#[cfg(test)]
mod tests {
    use crate::{SelectionItem, SelectionKind, Session, Workspace};
    use craft3d_math::Vec3;
    use serde_json::json;

    fn run(session: &mut Session, id: &str, params: serde_json::Value) {
        if let Err(e) = session.execute(id, &params) {
            panic!("{id}: {e}");
        }
    }

    /// A big plate with a small round post on top, filleted where it
    /// meets the plate, at `scale`.
    fn post(scale: f64) -> Session {
        let s = |v: f64| v * scale;
        let mut session = Session::new(Workspace::none());
        run(&mut session, "file.new", json!({"units": "mm"}));
        run(
            &mut session,
            "feature.block",
            json!({"name": "Plate", "size": [s(400.0), s(300.0), s(20.0)]}),
        );
        run(
            &mut session,
            "feature.cylinder",
            json!({"name": "Post", "position": [s(200.0), s(150.0), s(20.0)],
                   "radius": s(10.0), "height": s(30.0)}),
        );
        run(
            &mut session,
            "feature.boolean",
            json!({"name": "Joined", "op": "union", "target": "Plate", "tool": "Post"}),
        );
        let scene = session.scene(None).unwrap().unwrap();
        let body = scene.bodies.iter().find(|b| b.feature == "F3").unwrap();
        // The edge where the post meets the plate's top, by its two faces.
        let edge: Vec<String> = body
            .edge_faces
            .iter()
            .map(|faces| {
                faces
                    .iter()
                    .filter_map(|&f| body.face_names.get(f).cloned())
                    .collect::<Vec<String>>()
            })
            .find(|faces| {
                let makers: Vec<Option<String>> = faces.iter().map(|f| scene.made_by(f)).collect();
                makers.contains(&Some("F1".to_string())) && makers.contains(&Some("F2".to_string()))
            })
            .unwrap();
        run(
            &mut session,
            "feature.fillet",
            json!({"name": "Round", "edges": [edge], "radius": s(3.0)}),
        );
        session
    }

    #[test]
    fn a_fillet_on_top_is_seen_from_above_and_lies_in_its_body() {
        for scale in [1e-3, 1.0, 1e3] {
            let mut session = post(scale);
            let scene = session.scene(None).unwrap().unwrap();
            let round = scene.feature_face_ids("Round");
            assert!(!round.is_empty(), "the fillet made faces");
            let items = [SelectionItem {
                kind: SelectionKind::Feature,
                name: String::new(),
                feature: "Round".to_string(),
            }];
            assert_eq!(scene.items_face_ids(&items), round);
            // The body it lies in: the whole plate.
            let (lo, hi) = scene.bodies_box_of(&round).corners().unwrap();
            assert!(
                (hi.x - lo.x - 400.0 * scale).abs() < 1e-6 * scale,
                "{lo:?} {hi:?}"
            );
            // From below, the plate hides it; from above it is seen.
            let iso = |up: f64, x: f64, y: f64| Vec3::new(x, y, up * 0.8);
            let toward = [
                iso(-1.0, 1.0, -1.0),
                iso(-1.0, -1.0, 1.0),
                iso(1.0, 1.0, -1.0),
                iso(1.0, -1.0, -1.0),
            ];
            let best = scene.best_view(&round, &toward, &[]).unwrap();
            assert!(best >= 2, "{scale}: {best}");
            // Nothing to look at: none.
            assert_eq!(scene.best_view(&[], &toward, &[]), None);
            assert_eq!(scene.best_view(&round, &[], &[]), None);
            assert_eq!(
                scene.best_view(&round, &[Vec3::new(0.0, 0.0, 0.0)], &[]),
                None
            );
            // What may be flown to.
            let shown = scene.shown_features();
            for id in ["F1", "F2", "F4"] {
                assert!(shown.contains(id), "{id}: {shown:?}");
            }
            assert!(!shown.contains("F3"), "the union made no face: {shown:?}");
        }
    }

    /// A pattern made its instances; the seed keeps only its own faces.
    #[test]
    fn a_pattern_made_its_instances_and_the_seed_its_own_faces() {
        let mut session = Session::new(Workspace::none());
        run(&mut session, "file.new", json!({"units": "mm"}));
        run(
            &mut session,
            "feature.block",
            json!({"name": "Fin", "size": [2, 10, 10]}),
        );
        run(
            &mut session,
            "feature.pattern",
            json!({"name": "Fins", "source": "Fin", "count": 4, "step": [5, 0, 0]}),
        );
        let scene = session.scene(None).unwrap().unwrap();
        let seed = scene.feature_face_ids("Fin");
        let pattern = scene.feature_face_ids("Fins");
        assert_eq!(seed.len(), 6, "the seed's own block");
        assert_eq!(pattern.len(), 18, "three instances of six faces");
        assert!(seed.iter().all(|id| !pattern.contains(id)));
        // The seed's box is the first fin, the pattern's the other three.
        let items = |feature: &str| {
            [SelectionItem {
                kind: SelectionKind::Feature,
                name: String::new(),
                feature: feature.to_string(),
            }]
        };
        let (lo, hi) = scene.items_box(&items("Fin")).corners().unwrap();
        assert!(
            lo.x.abs() < 1e-9 && (hi.x - 2.0).abs() < 1e-9,
            "{lo:?} {hi:?}"
        );
        let (lo, hi) = scene.items_box(&items("Fins")).corners().unwrap();
        assert!(
            (lo.x - 5.0).abs() < 1e-9 && (hi.x - 17.0).abs() < 1e-9,
            "{lo:?} {hi:?}"
        );
        let body = &scene.bodies[0];
        let made: Vec<String> = body
            .face_names
            .iter()
            .filter_map(|name| scene.made_by(name))
            .collect();
        assert_eq!(made.iter().filter(|m| *m == "F1").count(), 6);
        assert_eq!(made.iter().filter(|m| *m == "F2").count(), 18);
    }
}
