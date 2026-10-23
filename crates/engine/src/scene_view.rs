//! Which way a viewer looks at what it flies to, and what it lights for a
//! feature: the faces of a scene a selection stands for, the bodies they
//! lie in, the direction from which they are best seen, and the features
//! a scene shows anything of. View-only: nothing here changes a document.

use crate::scene::Scene;
use crate::selection::{SelectionItem, SelectionKind};
use craft3d_doc::PersistentName;
use craft3d_math::{Bbox3, Vec3};
use std::collections::BTreeSet;
use std::ops::Range;

/// How many points of the target [`Scene::best_view`] checks for being
/// hidden by the rest of the model, per direction: those of its largest
/// triangles.
pub const VIEW_SAMPLES: usize = 8;

impl Scene {
    /// The ids in [`Scene::mesh`] of the faces of body `index`.
    pub fn body_face_ids(&self, index: usize) -> Range<u32> {
        let last_id = self
            .mesh
            .face_ids
            .iter()
            .max()
            .map_or(0, |&id| id.saturating_add(1));
        let first = self.bodies.get(index).map_or(0, |b| b.first_face_id);
        let end = self
            .bodies
            .get(index.saturating_add(1))
            .map_or(last_id, |b| b.first_face_id);
        first..end.max(first)
    }

    /// The ids in [`Scene::mesh`] of the faces whose persistent names say
    /// feature `feature` (by id or name) made them, wherever they are now;
    /// ascending. Empty for a feature that made no face of its own (a
    /// boolean, a sketch) and for one the scene does not show.
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
        let mut ids = Vec::new();
        for body in self.bodies.iter() {
            for (k, name) in body.face_names.iter().enumerate() {
                let made = name
                    .parse::<PersistentName>()
                    .is_ok_and(|name| name.root().feature.to_string() == id);
                if made
                    && let Ok(k) = u32::try_from(k)
                    && let Some(face) = body.first_face_id.checked_add(k)
                {
                    ids.push(face);
                }
            }
        }
        ids.sort_unstable();
        ids
    }

    /// The faces `items` stand for, as ids in [`Scene::mesh`], ascending:
    /// a face; the faces a feature made, or its body's when it made none;
    /// every face of a body. Edges, vertices, sketches and datums have
    /// none.
    pub fn items_face_ids(&self, items: &[SelectionItem]) -> Vec<u32> {
        let mut ids = Vec::new();
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
                        ids.extend(self.body_face_ids(index));
                    }
                }
                SelectionKind::Feature | SelectionKind::Sketch | SelectionKind::Datum => {
                    let made = self.feature_face_ids(feature);
                    if made.is_empty() {
                        if let Some(index) = body_of(feature) {
                            ids.extend(self.body_face_ids(index));
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
        for (index, body) in self.bodies.iter().enumerate() {
            let range = self.body_face_ids(index);
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
    /// are best seen, by its index: the most area turned towards it
    /// (the sum over the faces' triangles of area times the cosine to
    /// the direction, where positive), times the share of that area,
    /// over the [`VIEW_SAMPLES`] triangles turned most towards it, whose
    /// middle is not hidden along it by the rest of the model. Each score
    /// is times its weight in `weights` (1 where it has none; a viewer
    /// prefers some directions). Ties go to the earlier
    /// direction. `None` when no face is there or none is seen from any
    /// direction.
    pub fn best_view(&self, ids: &[u32], toward: &[Vec3], weights: &[f64]) -> Option<usize> {
        let mesh = &self.mesh;
        let mut target: Vec<(usize, Vec3, f64)> = Vec::new();
        for (t, id) in mesh.face_ids.iter().enumerate() {
            if ids.binary_search(id).is_ok()
                && let Some([a, b, c]) = mesh.corners(t)
            {
                // Twice the area along the outward normal.
                let normal = (b - a).cross(c - a);
                let area = normal.length() * 0.5;
                if area.is_finite() && area > 0.0 {
                    target.push((t, normal, area));
                }
            }
        }
        if target.is_empty() {
            return None;
        }
        let diagonal = self.bbox.diagonal();
        let reach = if diagonal.is_finite() && diagonal > 0.0 {
            diagonal * 2.0
        } else {
            1.0
        };
        let lift = reach * 1e-7;
        let mut best: Option<(usize, f64)> = None;
        for (index, direction) in toward.iter().enumerate() {
            let Some(d) = direction.normalized() else {
                continue;
            };
            // Each triangle's area seen along `d` (projected).
            let mut turned: Vec<(usize, f64)> = target
                .iter()
                .map(|&(t, normal, _)| (t, (normal.dot(d) * 0.5).max(0.0)))
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
                if !self.blocked(middle + d * lift, d, reach, ids) {
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

    /// Whether a triangle of [`Scene::mesh`] not of the faces `ids` lies
    /// on the ray from `from` along `d` (unit) within `reach`.
    fn blocked(&self, from: Vec3, d: Vec3, reach: f64, ids: &[u32]) -> bool {
        let mesh = &self.mesh;
        mesh.face_ids.iter().enumerate().any(|(t, id)| {
            ids.binary_search(id).is_err()
                && mesh
                    .corners(t)
                    .is_some_and(|[a, b, c]| hits(from, d, reach, a, b, c))
        })
    }

    /// The features the scene shows something of: a face they made, a
    /// body, a sketch or a datum. What a viewer may fly to.
    pub fn shown_features(&self) -> BTreeSet<String> {
        let mut shown = BTreeSet::new();
        for body in self.bodies.iter() {
            shown.insert(body.feature.clone());
            for name in &body.face_names {
                if let Ok(name) = name.parse::<PersistentName>() {
                    shown.insert(name.root().feature.to_string());
                }
            }
        }
        shown.extend(self.sketches.iter().map(|s| s.feature.clone()));
        shown.extend(self.datums.iter().map(|d| d.feature.clone()));
        shown
    }
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
}
