//! What a viewer frames when it flies to a feature or to the selection:
//! the box around the entities a [`Scene`] shows of them, and the feature
//! that made an entity, read from its persistent name. View-only: nothing
//! here changes a document.

use crate::scene::{Scene, SceneDatumShape};
use crate::selection::{SelectionItem, SelectionKind};
use craft3d_math::{Bbox3, Vec3};

impl Scene {
    /// The id (`F3`) of the feature `feature` names, by id or by name.
    fn feature_id(&self, feature: &str) -> Option<&str> {
        self.features
            .iter()
            .find(|f| f.id == feature)
            .or_else(|| self.features.iter().find(|f| f.name == feature))
            .map(|f| f.id.as_str())
    }

    /// Every corner of the triangles of [`Scene::mesh`] whose face ids
    /// are in `ids` (ascending): one pass over the mesh, however many
    /// faces are asked for.
    fn visit_faces(&self, ids: &[u32], visit: &mut dyn FnMut(Vec3)) {
        if ids.is_empty() {
            return;
        }
        for (t, id) in self.mesh.face_ids.iter().enumerate() {
            if ids.binary_search(id).is_ok()
                && let Some(corners) = self.mesh.corners(t)
            {
                corners.into_iter().for_each(&mut *visit);
            }
        }
    }

    /// The points of the mesh of the body of feature `id`; false when
    /// there is none.
    fn visit_body(&self, id: &str, visit: &mut dyn FnMut(Vec3)) -> bool {
        let Some(body) = self.bodies.iter().find(|b| b.feature == id) else {
            return false;
        };
        body.mesh.positions.iter().copied().for_each(visit);
        !body.mesh.positions.is_empty()
    }

    /// The points of the sketch of feature `id`: its curves and points,
    /// or its origin when it has none; false when there is no sketch.
    fn visit_sketch(&self, id: &str, visit: &mut dyn FnMut(Vec3)) -> bool {
        let Some(sketch) = self.sketches.iter().find(|s| s.feature == id) else {
            return false;
        };
        let mut any = false;
        let points = sketch
            .points
            .iter()
            .chain(sketch.polylines.iter().flat_map(|p| p.points.iter()));
        for point in points {
            visit(*point);
            any = true;
        }
        if !any {
            visit(sketch.origin);
        }
        true
    }

    /// The corners of the datum of feature `id`, as it is drawn; false
    /// when there is no such datum.
    fn visit_datum(&self, id: &str, visit: &mut dyn FnMut(Vec3)) -> bool {
        let Some(datum) = self.datums.iter().find(|d| d.feature == id) else {
            return false;
        };
        match datum.shape {
            SceneDatumShape::Plane { center, u, v, .. } => {
                for point in [
                    center + u + v,
                    center + u - v,
                    center - u + v,
                    center - u - v,
                ] {
                    visit(point);
                }
            }
            SceneDatumShape::Axis { from, to } => {
                visit(from);
                visit(to);
            }
            SceneDatumShape::Point(point) => visit(point),
        }
        true
    }

    /// The points that show what feature `feature` (by id or name) made:
    /// the corners of the triangles of the faces whose persistent names
    /// say it made them, wherever they are now (a hole's wall in the
    /// plate it was cut into); else its body's mesh; else its sketch or
    /// its datum. None when the scene shows nothing of it (a feature that
    /// failed, one that made no face of its own and has no body).
    pub fn feature_points(&self, feature: &str, visit: &mut dyn FnMut(Vec3)) {
        let item = SelectionItem {
            kind: SelectionKind::Feature,
            name: String::new(),
            feature: feature.to_string(),
        };
        self.items_points(std::slice::from_ref(&item), visit);
    }

    /// The box around [`Self::feature_points`].
    pub fn feature_box(&self, feature: &str) -> Bbox3 {
        let mut bbox = Bbox3::EMPTY;
        self.feature_points(feature, &mut |p| bbox.add(p));
        bbox
    }

    /// The points that show `items` (a selection, `select.get`) in the
    /// scene: a body's mesh, a feature's result
    /// ([`Self::feature_points`]), a sketch, a datum, the triangles of a
    /// face, the polyline of an edge, the point of a vertex. The faces of
    /// all items are gathered first and their triangles read in one pass
    /// over the mesh. Items the scene does not show add nothing.
    pub fn items_points(&self, items: &[SelectionItem], visit: &mut dyn FnMut(Vec3)) {
        let mut faces = Vec::new();
        for item in items {
            let feature = item.feature.as_str();
            let id = self.feature_id(feature).unwrap_or(feature);
            match item.kind {
                SelectionKind::Body => {
                    self.visit_body(id, visit);
                }
                SelectionKind::Feature | SelectionKind::Sketch | SelectionKind::Datum => {
                    if self.feature_id(feature).is_none() {
                        continue;
                    }
                    let made = self.feature_face_ids(id);
                    if made.is_empty() {
                        let _ = self.visit_body(id, visit)
                            || self.visit_sketch(id, visit)
                            || self.visit_datum(id, visit);
                    } else {
                        faces.extend(made);
                    }
                }
                SelectionKind::Face => {
                    if let Ok(found) = self.faces_named(Some(feature), &item.name) {
                        faces.extend(found.ids);
                    }
                }
                SelectionKind::Edge => {
                    if let Ok(found) = self.edges_named(Some(feature), Some(&item.name), &[])
                        && let Some(body) = self.bodies.get(found.body_index)
                    {
                        found
                            .edges
                            .iter()
                            .filter_map(|&k| body.edges.get(k))
                            .flatten()
                            .copied()
                            .for_each(&mut *visit);
                    }
                }
                SelectionKind::Vertex => {
                    if let Ok(found) = self.vertices_named(Some(feature), Some(&item.name), &[])
                        && let Some(body) = self.bodies.get(found.body_index)
                    {
                        found
                            .vertices
                            .iter()
                            .filter_map(|&k| body.vertices.get(k))
                            .copied()
                            .for_each(&mut *visit);
                    }
                }
            }
        }
        faces.sort_unstable();
        faces.dedup();
        self.visit_faces(&faces, visit);
    }

    /// The box around [`Self::items_points`]; empty when none of the
    /// items is shown.
    pub fn items_box(&self, items: &[SelectionItem]) -> Bbox3 {
        let mut bbox = Bbox3::EMPTY;
        self.items_points(items, &mut |p| bbox.add(p));
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

    /// Many faces at once are read in one pass and frame what each does
    /// alone: every face of the cut plate, twice over, is the plate.
    #[test]
    fn all_faces_of_a_body_frame_the_body() {
        let mut session = plate(1.0);
        let scene = session.scene(None).unwrap().unwrap();
        let body = scene.bodies.iter().find(|b| b.feature == "F3").unwrap();
        let items: Vec<SelectionItem> = body
            .face_names
            .iter()
            .chain(body.face_names.iter())
            .map(|name| SelectionItem {
                kind: SelectionKind::Face,
                name: name.clone(),
                feature: "F3".to_string(),
            })
            .collect();
        assert_eq!(scene.items_box(&items), body.mesh.bbox());
        let mut points = 0usize;
        scene.items_points(&items, &mut |_| points += 1);
        assert_eq!(points, 3 * body.mesh.triangles.len(), "each triangle once");
        // A feature and the faces it made: the same box.
        let made: Vec<SelectionItem> = body
            .face_names
            .iter()
            .filter(|name| scene.made_by(name).as_deref() == Some("F2"))
            .map(|name| SelectionItem {
                kind: SelectionKind::Face,
                name: name.clone(),
                feature: "F3".to_string(),
            })
            .collect();
        assert_eq!(scene.items_box(&made), scene.feature_box("Tool"));
    }
}
