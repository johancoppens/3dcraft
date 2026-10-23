//! What a viewer frames when it flies to a feature or to the selection:
//! the box around the entities a [`Scene`] shows of them, and the feature
//! that made an entity, read from its persistent name. View-only: nothing
//! here changes a document.

use crate::scene::{Scene, SceneDatumShape};
use crate::selection::{SelectionItem, SelectionKind};
use craft3d_doc::PersistentName;
use craft3d_math::Bbox3;

impl Scene {
    /// The id (`F3`) of the feature `feature` names, by id or by name.
    fn feature_id(&self, feature: &str) -> Option<&str> {
        self.features
            .iter()
            .find(|f| f.id == feature)
            .or_else(|| self.features.iter().find(|f| f.name == feature))
            .map(|f| f.id.as_str())
    }

    /// Where the feature of id `id` is in the history; `None` for one the
    /// scene does not list.
    fn feature_place(&self, id: &str) -> Option<usize> {
        self.features.iter().position(|f| f.id == id)
    }

    /// The feature (by id, `F3`) that made the face, edge or vertex
    /// named `name`: the feature at the end of its chain of copies
    /// (`F4:copy(0)/F2:face(2)` was made by `F2`). An edge or a vertex
    /// named by the faces it lies on was made by the latest of the
    /// features that made those faces, in the order of the history: it
    /// came to be with the last of them. `None` for text that is no
    /// persistent name.
    pub fn made_by(&self, name: &str) -> Option<String> {
        let name: PersistentName = name.parse().ok()?;
        let root = name.root();
        if root.role.is_by_faces() {
            let latest = root
                .faces()
                .iter()
                .map(|face| face.root().feature.to_string())
                .filter_map(|id| self.feature_place(&id).map(|place| (place, id)))
                .max_by_key(|(place, _)| *place);
            if let Some((_, id)) = latest {
                return Some(id);
            }
        }
        Some(root.feature.to_string())
    }

    /// The box around the faces of [`Scene::mesh`] whose ids are in
    /// `ids` (ascending).
    fn faces_box(&self, ids: &[u32]) -> Bbox3 {
        let mut bbox = Bbox3::EMPTY;
        if ids.is_empty() {
            return bbox;
        }
        for (t, id) in self.mesh.face_ids.iter().enumerate() {
            if ids.binary_search(id).is_ok()
                && let Some(corners) = self.mesh.corners(t)
            {
                for corner in corners {
                    bbox.add(corner);
                }
            }
        }
        bbox
    }

    /// The box around what feature `feature` (by id or name) made, as
    /// the scene shows it: the faces whose persistent names say it made
    /// them, wherever they are now (a hole's wall in the plate it was cut
    /// into); else its body; else its sketch or its datum. Empty when the
    /// scene shows nothing of it (a feature that failed, one that made no
    /// face of its own).
    pub fn feature_box(&self, feature: &str) -> Bbox3 {
        let Some(id) = self.feature_id(feature) else {
            return Bbox3::EMPTY;
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
        let faces = self.faces_box(&ids);
        if !faces.is_empty() {
            return faces;
        }
        self.body_box(id)
            .or_else(|| self.sketch_box(id))
            .or_else(|| self.datum_box(id))
            .unwrap_or(Bbox3::EMPTY)
    }

    /// The box around the body of feature `id`.
    fn body_box(&self, id: &str) -> Option<Bbox3> {
        let body = self.bodies.iter().find(|b| b.feature == id)?;
        Some(body.mesh.bbox()).filter(|b| !b.is_empty())
    }

    /// The box around the sketch of feature `id`: its curves and points.
    fn sketch_box(&self, id: &str) -> Option<Bbox3> {
        let sketch = self.sketches.iter().find(|s| s.feature == id)?;
        let mut bbox = Bbox3::from_points(sketch.points.iter().copied());
        for polyline in &sketch.polylines {
            for point in &polyline.points {
                bbox.add(*point);
            }
        }
        if bbox.is_empty() {
            bbox.add(sketch.origin);
        }
        Some(bbox)
    }

    /// The box around the datum of feature `id`, as it is drawn.
    fn datum_box(&self, id: &str) -> Option<Bbox3> {
        let datum = self.datums.iter().find(|d| d.feature == id)?;
        let points = match datum.shape {
            SceneDatumShape::Plane { center, u, v } => {
                vec![
                    center + u + v,
                    center + u - v,
                    center - u + v,
                    center - u - v,
                ]
            }
            SceneDatumShape::Axis { from, to } => vec![from, to],
            SceneDatumShape::Point(point) => vec![point],
        };
        Some(Bbox3::from_points(points))
    }

    /// The box around what `items` (a selection, `select.get`) are in the
    /// scene: a body's mesh, a feature's result ([`Self::feature_box`]),
    /// a sketch, a datum, the triangles of a face, the polyline of an
    /// edge, the point of a vertex. Items the scene does not show add
    /// nothing; empty when none of them is shown.
    pub fn items_box(&self, items: &[SelectionItem]) -> Bbox3 {
        let mut bbox = Bbox3::EMPTY;
        let mut add = |found: Bbox3| {
            if let Some((lo, hi)) = found.corners() {
                bbox.add(lo);
                bbox.add(hi);
            }
        };
        for item in items {
            let feature = item.feature.as_str();
            match item.kind {
                SelectionKind::Body => {
                    let id = self.feature_id(feature).unwrap_or(feature);
                    add(self.body_box(id).unwrap_or(Bbox3::EMPTY));
                }
                SelectionKind::Feature | SelectionKind::Sketch | SelectionKind::Datum => {
                    add(self.feature_box(feature));
                }
                SelectionKind::Face => {
                    if let Ok(found) = self.faces_named(Some(feature), &item.name) {
                        add(self.faces_box(&found.ids));
                    }
                }
                SelectionKind::Edge => {
                    if let Ok(found) = self.edges_named(Some(feature), Some(&item.name), &[])
                        && let Some(body) = self.bodies.get(found.body_index)
                    {
                        let points = found
                            .edges
                            .iter()
                            .filter_map(|&k| body.edges.get(k))
                            .flatten()
                            .copied();
                        add(Bbox3::from_points(points));
                    }
                }
                SelectionKind::Vertex => {
                    if let Ok(found) = self.vertices_named(Some(feature), Some(&item.name), &[])
                        && let Some(body) = self.bodies.get(found.body_index)
                    {
                        let points = found
                            .vertices
                            .iter()
                            .filter_map(|&k| body.vertices.get(k))
                            .copied();
                        add(Bbox3::from_points(points));
                    }
                }
            }
        }
        bbox
    }
}

#[cfg(test)]
mod tests {
    use crate::{SelectionItem, SelectionKind, Session, Workspace};
    use serde_json::json;

    fn run(session: &mut Session, id: &str, params: serde_json::Value) {
        if let Err(e) = session.execute(id, &params) {
            panic!("{id}: {e}");
        }
    }

    /// A plate with a hole cut into it and a sketch above it, at `scale`.
    fn plate(scale: f64) -> Session {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(
            &mut s,
            "feature.block",
            json!({"name": "Plate", "size": [100.0 * scale, 60.0 * scale, 10.0 * scale]}),
        );
        run(
            &mut s,
            "feature.cylinder",
            json!({"name": "Tool", "position": [20.0 * scale, 30.0 * scale, -5.0 * scale],
                   "radius": 5.0 * scale, "height": 20.0 * scale}),
        );
        run(
            &mut s,
            "feature.boolean",
            json!({"name": "Hole", "op": "subtract", "target": "Plate", "tool": "Tool"}),
        );
        run(
            &mut s,
            "sketch.create",
            json!({"name": "Above", "offset": 50.0 * scale}),
        );
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Above", "closed": true,
                   "points": [[0, 0], [10.0 * scale, 0], [10.0 * scale, 10.0 * scale]]}),
        );
        s
    }

    #[test]
    fn a_feature_is_framed_by_what_it_made_and_entities_by_their_creator() {
        for scale in [1e-3, 1.0, 1e6] {
            let mut session = plate(scale);
            let scene = session.scene(None).unwrap().unwrap();
            // Within the mesh's chords of the circle.
            let near = |a: f64, b: f64| (a - b).abs() <= 5e-3 * scale * 100.0;
            // The plate's own faces: the whole plate.
            let (lo, hi) = scene.feature_box("F1").corners().unwrap();
            assert!(
                near(lo.x, 0.0) && near(hi.x, 100.0 * scale),
                "{scale}: {lo:?} {hi:?}"
            );
            assert!(near(hi.z, 10.0 * scale));
            // The tool's faces left in the plate: the hole's wall, a
            // cylinder of radius 5 about (20, 30), as high as the plate.
            let (lo, hi) = scene.feature_box("Tool").corners().unwrap();
            assert!(
                near(lo.x, 15.0 * scale) && near(hi.x, 25.0 * scale),
                "{lo:?} {hi:?}"
            );
            assert!(near(lo.y, 25.0 * scale) && near(hi.y, 35.0 * scale));
            assert!(near(lo.z, 0.0) && near(hi.z, 10.0 * scale));
            // The subtraction made no face of its own: its body.
            let (lo, hi) = scene.feature_box("Hole").corners().unwrap();
            assert!(near(lo.x, 0.0) && near(hi.x, 100.0 * scale));
            // The sketch: its triangle, at its height.
            let (lo, hi) = scene.feature_box("Above").corners().unwrap();
            assert!(near(lo.z, 50.0 * scale) && near(hi.z, 50.0 * scale));
            assert!(near(hi.x, 10.0 * scale));
            // Unknown features frame nothing.
            assert!(scene.feature_box("F99").is_empty());
            assert!(scene.feature_box("").is_empty());

            // Every face of the plate's body was made by F1 or F2.
            let body = scene.bodies.iter().find(|b| b.feature == "F3").unwrap();
            let makers: std::collections::BTreeSet<String> = body
                .face_names
                .iter()
                .filter_map(|name| scene.made_by(name))
                .collect();
            assert_eq!(
                makers.into_iter().collect::<Vec<_>>(),
                ["F1", "F2"],
                "{scale}"
            );
            // An edge between the top and the hole's wall came with the
            // wall.
            let edge = body
                .edge_names
                .iter()
                .find(|name| {
                    let faces: Vec<&String> = body
                        .edge_faces
                        .get(body.edge_names.iter().position(|n| n == *name).unwrap())
                        .unwrap()
                        .iter()
                        .filter_map(|&f| body.face_names.get(f))
                        .collect();
                    faces
                        .iter()
                        .any(|f| scene.made_by(f).as_deref() == Some("F1"))
                        && faces
                            .iter()
                            .any(|f| scene.made_by(f).as_deref() == Some("F2"))
                })
                .unwrap();
            assert_eq!(scene.made_by(edge).as_deref(), Some("F2"), "{edge}");
            assert_eq!(scene.made_by("not a name"), None);

            // A selection: a face of the wall, framed by itself.
            let wall = body
                .face_names
                .iter()
                .find(|name| scene.made_by(name).as_deref() == Some("F2"))
                .unwrap();
            let items = [SelectionItem {
                kind: SelectionKind::Face,
                name: wall.clone(),
                feature: "F3".to_string(),
            }];
            let (lo, hi) = scene.items_box(&items).corners().unwrap();
            assert!(near(lo.x, 15.0 * scale) && near(hi.x, 25.0 * scale));
            let items = [SelectionItem {
                kind: SelectionKind::Edge,
                name: edge.clone(),
                feature: "F3".to_string(),
            }];
            let (lo, hi) = scene.items_box(&items).corners().unwrap();
            assert!(near(lo.x, 15.0 * scale) && near(hi.x, 25.0 * scale));
            assert!(near(hi.z - lo.z, 0.0), "the top rim: {lo:?} {hi:?}");
            let items = [SelectionItem {
                kind: SelectionKind::Body,
                name: "Hole".to_string(),
                feature: "F3".to_string(),
            }];
            let (lo, hi) = scene.items_box(&items).corners().unwrap();
            assert!(near(lo.x, 0.0) && near(hi.x, 100.0 * scale));
            // Nothing the scene shows: empty.
            let items = [SelectionItem {
                kind: SelectionKind::Face,
                name: "F9:face(0)".to_string(),
                feature: "F9".to_string(),
            }];
            assert!(scene.items_box(&items).is_empty());
            assert!(scene.items_box(&[]).is_empty());
        }
    }
}
