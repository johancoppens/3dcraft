//! The `ui.*` methods of the control channel (docs/control-protocol.md):
//! what an agent uses to look at the window it is driving, frame the camera
//! and caption a step. They read and change view state only; the document
//! is changed with commands.
//!
//! This is the body of the methods; the transport (JSON lines over TCP, the
//! token) belongs to the application.

use crate::shell::Shell;
use crate::viewport::{self, MAX_SIDE, MIN_SIDE, Quality};
use craft3d_engine::Session;
use serde_json::{Map, Value, json};

/// A method, as the protocol's `methods` lists it.
pub struct UiMethod {
    pub name: &'static str,
    pub params: &'static str,
    pub summary: &'static str,
}

/// Every `ui.*` method, in a fixed order.
pub const UI_METHODS: &[UiMethod] = &[
    UiMethod {
        name: "ui.get",
        params: "{}",
        summary: "Returns the UI state (camera, panels, selectedFeature, notice, quality), autosave and recovery as recovery.list gives them but without looking at the disk again (recovery: the documents a crashed 3DCraft left behind, when a document was last autosaved, the autosave problem if there is one), the viewport size, what the scene holds (bodies, sketches, triangles, the chordal tolerance it was meshed with, whether the display quality was coarsened to fit its triangle budget, and refining: true while the viewport shows the quick first picture and the display-quality mesh is still being built), the feature tree rows, which renderer the viewport uses (gpu or cpu, the adapter, whether the GPU draws directly on the window's device and with how many samples, why it fell back), how the viewport was last rendered, and section: where the section plane of state.section lies while it is on (point, normal, into the half that is cut away, and range, the offsets between which the plane passes through the bodies), or lost, why the section was switched off when its plane no longer exists (a face or a datum plane that is gone); null when it is off. tabs: the open documents as the row of tabs above the viewport shows them, in the order of file.list (document: the number file.activate takes, name: the file's name without its extension or Untitled n, path, dirty: unsaved changes, marked on the tab, active, asksToClose: closing it from its tab asks Save, Don't Save or Cancel first, as file.close refuses it without discard); closing: that question while the window asks it ({document, name, path}), else null. A click on a tab runs file.activate, its close button file.close.",
    },
    UiMethod {
        name: "ui.set",
        params: r#"{"view": "iso|front|back|right|left|top|bottom", "camera": {"target": [x, y, z], "distance": number, "yaw": degrees, "pitch": degrees, "projection": "perspective|orthographic", "fovY": degrees, "autoFit": boolean}, "panels": {"tree": boolean, "status": boolean, "recovery": boolean}, "selectedFeature": id or null, "notice": string, "quality": {"chordal": fraction of the model's size = 3e-5, "maxTriangles": integer = 1000000}, "highlight": {"feature": id or name, "face": face name} or null, "highlightEdge": {"feature": id or name, "edge": edge name, "between": [face name, face name]} or null, "highlightVertex": {"feature": id or name, "vertex": vertex name, "corner": [face name, ...]} or null, "transparency": number from 0 to 0.9 = 0, "section": {"on": boolean, "plane": "xy|yz|zx" or a datum plane or a face name, "feature": id or name, "point": [x, y, z], "normal": [x, y, z], "offset": number, "flip": boolean}, "show": {"datums": boolean = true, "sketches": boolean = true}}"#,
        summary: "Changes the fields given and leaves the rest. An unknown field or a wrong type is an error, and then nothing has changed. Numbers out of range are clamped. quality is how finely the viewport's mesh follows the model: a chordal tolerance of chordal times the model's size, coarsened until the scene has at most maxTriangles triangles but never beyond the automatic tolerance view.render uses. highlight is the face shown highlighted, highlightEdge the edge and highlightVertex the vertex (ui.highlight sets them too, and says whether they exist); transparency is how see-through the bodies are drawn (0: opaque; up to 0.9): the front surface is blended over the one behind it, so that inner faces, holes and cavities show, and the hidden parts of edges are drawn dimmed; ui.screenshot shows it too, and a pick still gives the front-most face. section cuts the viewport's picture open with a plane, to look inside a shell, a pocket or a cavity; it is a view, the document does not change. The plane is plane (xy, yz or zx, a datum plane by its feature's id or name, or a flat face by its persistent name, with feature to say which body) or point and normal, moved by offset along its normal; what lies on the side the normal points to is not drawn (a face's normal points out of its body, so a negative offset cuts into it) and flip keeps the other half. Where the plane passes through a body the cut is closed with a flat cap in a colour of its own, edges end at the plane and the outline of the cut is drawn as edges. Setting plane replaces point and normal and the other way round; on with no plane at all is xy. A plane that means nothing (an unknown name, a face that is not flat, a normal without direction) is an error and changes nothing; ui.get says under section where the plane lies and between which offsets it passes through the model. ui.screenshot and ui.pick follow the section: a pick gives what is seen, never a face that is cut away, and on the cap kind is section. show says what the viewport draws besides the bodies and their edges: datums false hides the datum planes, axes and points, sketches false the sketches (their lines and points); the toggles over the viewport set the same fields, and ui.screenshot follows them. picked, the last click, cannot be set.",
    },
    UiMethod {
        name: "ui.pick",
        params: r#"{"x": number, "y": number, "width": integer = the viewport's, "height": integer = the viewport's, "radius": number = the click's (4 points), "select": boolean = false}"#,
        summary: "Says what the viewport shows at a position, as the command view.pick does, for the viewport's mesh and camera and cut open by the viewport's section when that is on (then kind is section where the cap of the section is in front, with section: {feature, body, point, depth}, the body that is cut there and the point of the plane): kind (vertex when a vertex is near, otherwise edge when an edge is near, otherwise face), the face under the position (the body as feature and name, the face's persistent name as document.inspect lists it under faceNames, with faceIndex and aliases, and the point in space) and, when its nearest point in sight is within radius pixels, edge: {feature, body, name, index, faces, point, depth, distance}, the edge as document.edges lists it (name is its own persistent name or null, faces the names of the faces it lies between), or vertex, with the same fields, when a vertex in sight is within radius pixels (faces: the faces that meet in it; index: its index under vertices of document.edges). x and y are in pixels from the top-left corner of the viewport (the picture ui.screenshot makes; width and height when that picture was made at another size), the centre of pixel (i, j) at (i + 0.5, j + 0.5). radius is in pixels of that picture, 0 to 64, 0 for faces only; without it, it is what a click uses. hit is false when nothing is there, with outside true when the position is not in the viewport. With select it counts as a click of the mouse: state.picked becomes this pick and what it points at is highlighted, the vertex (state.highlightVertex), or else the edge (state.highlightEdge), or else the face (state.highlight); a pick on nothing clears them. A click in the viewport does the same, so state.picked of ui.get is what the developer pointed at last.",
    },
    UiMethod {
        name: "ui.highlight",
        params: r#"{"face": face name or null, "edge": edge name, "between": [face name, face name], "vertex": vertex name, "corner": [face name, ...], "feature": id or name = the body that has it}"#,
        summary: "Shows a face, an edge or a vertex highlighted in the viewport; one of face, edge, between, vertex and corner. face is a face's persistent name from document.inspect (faceNames, or an alias). edge is an edge's own persistent name (name in document.edges); between names an edge by the two faces it lies between, as document.edges takes it (the same name twice for a seam), and means every edge between them. vertex is a vertex's own persistent name (name under vertices in document.edges); corner names a vertex by faces that meet in it (one to 16 names) and means every vertex in which all of them meet. feature says which body when several have one of that name. face: null removes every highlight. Returns the highlight as it was set (highlight: {feature, face} for a face, highlightEdge: {feature, edge, between} for an edge, highlightVertex: {feature, vertex, corner} for a vertex; the feature by id), the name of the body and how many faces carry the name (faces; a boolean can cut a face into pieces that keep its name) or how many edges or vertices are meant (edges, vertices). A name no body has is an error and changes nothing.",
    },
    UiMethod {
        name: "ui.fit",
        params: r#"{"view": "iso|front|back|right|left|top|bottom"}"#,
        summary: "Frames everything (bodies and sketches), from the named view when one is given, and keeps following the model as it grows.",
    },
    UiMethod {
        name: "ui.screenshot",
        params: r#"{"path": string, "width": integer = the viewport's, "height": integer = the viewport's, "scale": number = the window's pixels per point}"#,
        summary: "A PNG of the viewport as it is framed now: bodies by the CPU reference renderer (the oracle, whichever renderer the window uses), meshed at the display quality the viewport shows, sketches and axes over them. Written under the write root when path is given, otherwise returned as base64.",
    },
    UiMethod {
        name: "ui.notice",
        params: r#"{"text": string}"#,
        summary: "Shows a one-line caption over the viewport; an empty text removes it.",
    },
    UiMethod {
        name: "ui.quit",
        params: "{}",
        summary: "Closes the application.",
    },
];

/// The params of a method: an object, or nothing at all.
#[derive(Clone, Copy)]
struct Fields<'a>(Option<&'a Map<String, Value>>);

impl<'a> Fields<'a> {
    fn get(&self, key: &str) -> Option<&'a Value> {
        self.0?.get(key)
    }
}

/// The params of `method` as an object whose keys are all in `allowed`.
fn fields<'a>(
    method: &str,
    params: Option<&'a Value>,
    allowed: &[&str],
) -> Result<Fields<'a>, String> {
    let map = match params {
        None | Some(Value::Null) => None,
        Some(Value::Object(map)) => Some(map),
        Some(_) => return Err(format!("{method}: parameters must be a JSON object")),
    };
    if let Some(unknown) = map
        .into_iter()
        .flat_map(Map::keys)
        .find(|key| !allowed.contains(&key.as_str()))
    {
        let unknown: String = unknown.chars().take(40).collect();
        let expected = if allowed.is_empty() {
            "it takes no parameters".to_string()
        } else {
            format!("its parameters are: {}", allowed.join(", "))
        };
        return Err(format!(
            "{method}: unknown parameter `{unknown}`; {expected}"
        ));
    }
    Ok(Fields(map))
}

/// A whole number of pixels within the limits of an image side.
fn side(method: &str, map: Fields<'_>, key: &str) -> Result<Option<u32>, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (MIN_SIDE..=MAX_SIDE).contains(n))
            .map(Some)
            .ok_or_else(|| {
                format!("{method}: `{key}` must be a whole number from {MIN_SIDE} to {MAX_SIDE}")
            }),
    }
}

impl Shell {
    /// Runs a `ui.*` method. `None` when `method` is not one of them.
    /// Never panics, whatever the params hold.
    pub fn handle_method(
        &mut self,
        session: &mut Session,
        method: &str,
        params: Option<&Value>,
    ) -> Option<Result<Value, String>> {
        let result = match method {
            "ui.get" => self.ui_get(session, params),
            "ui.set" => self.ui_set(session, params),
            "ui.fit" => self.ui_fit(session, params),
            "ui.screenshot" => self.ui_screenshot(session, params),
            "ui.pick" => self.ui_pick(session, params),
            "ui.highlight" => self.ui_highlight(session, params),
            "ui.notice" => self.ui_notice(params),
            "ui.quit" => fields(method, params, &[]).map(|_| {
                self.request_quit();
                json!({ "quitting": true })
            }),
            _ => return None,
        };
        Some(result)
    }

    fn state_json(&self) -> Result<Value, String> {
        serde_json::to_value(&self.state).map_err(|e| e.to_string())
    }

    fn ui_get(&mut self, session: &mut Session, params: Option<&Value>) -> Result<Value, String> {
        self.ui_get_with(Some(session), params)
    }

    /// `ui.get` while the session is away doing work on the host's engine
    /// thread: the state as the window shows it (the scene and document
    /// of the last sync), with `work` saying what is being done.
    pub fn ui_get_away(&mut self, params: Option<&Value>) -> Result<Value, String> {
        self.ui_get_with(None, params)
    }

    fn ui_get_with(
        &mut self,
        session: Option<&mut Session>,
        params: Option<&Value>,
    ) -> Result<Value, String> {
        fields("ui.get", params, &[])?;
        let (scene, status) = match session {
            Some(session) => (self.sync(session), session.document_status()),
            None => (self.shown_scene_arc(), self.shown_status().cloned()),
        };
        let [width, height] = self.viewport_size();
        let vec3 = |v: craft3d_math::Vec3| json!([v.x, v.y, v.z]);
        let scene_json = scene.as_deref().map_or(Value::Null, |scene| {
            json!({
                "revision": scene.revision,
                "bodies": scene.bodies.len(),
                "sketches": scene.sketches.len(),
                "triangles": scene.triangles(),
                "tolerance": scene.tolerance,
                "coarsened": scene.coarsened,
                "refining": scene.preview,
                "bbox": scene.bbox.corners().map(|(lo, hi)| json!([vec3(lo), vec3(hi)])),
                "problems": scene.problems,
            })
        });
        let features: Vec<Value> = self
            .tree_rows()
            .iter()
            .map(|row| {
                json!({
                    "id": row.id,
                    "name": row.name,
                    "kind": row.kind,
                    "state": row.state.name(),
                    "status": row.status,
                    "error": row.error,
                    "causedBy": row.caused_by.as_ref().map(|(id, name)| json!({"id": id, "name": name})),
                    "selected": row.selected,
                })
            })
            .collect();
        let document = status.map_or(Value::Null, |status| {
            json!({
                "index": status.index,
                "open": status.open,
                "path": status.path,
                "dirty": status.dirty,
                "units": { "length": status.length_unit, "angle": status.angle_unit },
            })
        });
        let report = self.renderer_report();
        let renderer = json!({
            "backend": report.backend.name(),
            "adapter": report.adapter,
            "fallback": report.fallback,
            "direct": report.direct,
            "msaa": report.msaa,
        });
        let render = self.render_stats().map_or(Value::Null, |stats| {
            json!({
                "renderer": stats.renderer.name(),
                "width": stats.width,
                "height": stats.height,
                "ssaa": stats.ssaa,
                "msaa": stats.msaa,
                "triangles": stats.triangles,
                "milliseconds": stats.milliseconds,
                "sharp": stats.sharp,
                "count": self.render_count(),
            })
        });
        Ok(json!({
            "state": self.state_json()?,
            "viewport": {
                "width": width,
                "height": height,
                "pixelsPerPoint": self.pixels_per_point(),
            },
            "document": document,
            "scene": scene_json,
            "features": features,
            "renderer": renderer,
            "render": render,
            "section": self.section_report(),
            "lastCommand": self.last_command.as_ref().map(|report| json!({
                "label": report.label, "ok": report.ok, "text": report.text,
            })),
            "recovery": self.recovery().to_json(),
            "problem": self.problem(),
            "work": self.work(),
            "tabs": self.tabs(),
            "closing": self.closing(),
        }))
    }

    fn ui_set(&mut self, session: &mut Session, params: Option<&Value>) -> Result<Value, String> {
        let patch = match params {
            None | Some(Value::Null) => return Err("ui.set: nothing to set".to_string()),
            Some(patch) => patch,
        };
        let before = self.state.clone();
        let lost_before = self.section_lost().map(str::to_string);
        self.state
            .apply_patch(patch)
            .map_err(|e| format!("ui.set: {e}"))?;
        let sets_section = patch.get("section").is_some();
        if sets_section {
            self.clear_section_lost();
        }
        // A selection that names no feature is dropped here, not later.
        let scene = self.sync(session);
        // A section whose plane means nothing is refused, not switched
        // off behind the caller's back: nothing has changed.
        if sets_section
            && scene.is_some()
            && let Some(why) = self.section_lost().map(str::to_string)
        {
            self.state = before;
            self.sync(session);
            if lost_before.is_none() {
                self.clear_section_lost();
            }
            return Err(format!("ui.set: {why}"));
        }
        Ok(json!({ "state": self.state_json()? }))
    }

    fn ui_fit(&mut self, session: &mut Session, params: Option<&Value>) -> Result<Value, String> {
        let map = fields("ui.fit", params, &["view"])?;
        let view = match map.get("view") {
            None | Some(Value::Null) => None,
            Some(Value::String(name)) => Some(name.as_str()),
            Some(_) => return Err("ui.fit: `view` must be a string".to_string()),
        };
        let mut camera = self.state.camera;
        if let Some(name) = view {
            camera.set_view(name).map_err(|e| format!("ui.fit: {e}"))?;
        }
        let before = self.state.camera;
        self.state.camera = camera;
        if !self.fit(session) {
            self.state.camera = before;
            return Err(
                "ui.fit: there is nothing to frame; add a feature or a sketch first".to_string(),
            );
        }
        Ok(json!({ "camera": self.state_json()?.get("camera") }))
    }

    fn ui_notice(&mut self, params: Option<&Value>) -> Result<Value, String> {
        let map = fields("ui.notice", params, &["text"])?;
        let Some(Value::String(text)) = map.get("text") else {
            return Err("ui.notice: `text` must be a string (empty to remove it)".to_string());
        };
        self.state
            .apply_patch(&json!({ "notice": text }))
            .map_err(|e| format!("ui.notice: {e}"))?;
        Ok(json!({ "notice": self.state.notice }))
    }

    fn ui_pick(&mut self, session: &mut Session, params: Option<&Value>) -> Result<Value, String> {
        let method = "ui.pick";
        let map = fields(
            method,
            params,
            &["x", "y", "width", "height", "radius", "select"],
        )?;
        let number = |key: &str| match map.get(key) {
            None | Some(Value::Null) => Err(format!("{method}: missing parameter `{key}`")),
            Some(value) => value
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("{method}: `{key}` must be a finite number")),
        };
        let (x, y) = (number("x")?, number("y")?);
        let [viewport_width, viewport_height] = self.viewport_size();
        let width = side(method, map, "width")?.unwrap_or(viewport_width);
        let height = side(method, map, "height")?.unwrap_or(viewport_height);
        let radius = match map.get("radius") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_f64()
                    .filter(|r| (0.0..=craft3d_engine::MAX_PICK_RADIUS).contains(r))
                    .ok_or_else(|| {
                        format!(
                            "{method}: `radius` must be a number of pixels from 0 to {}",
                            craft3d_engine::MAX_PICK_RADIUS
                        )
                    })?,
            ),
        };
        let select = match map.get("select") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(select)) => *select,
            Some(_) => return Err(format!("{method}: `select` must be true or false")),
        };
        let scene = self.sync(session);
        let Some(scene) = scene else {
            return Err(match self.problem() {
                Some(problem) => format!("{method}: {problem}"),
                None => format!("{method}: no document is open; run file.new or file.open first"),
            });
        };
        // A click is in the viewport's own pixels.
        let stretch = f64::from(viewport_width) / f64::from(width);
        let (px, py) = if select {
            (
                x * stretch,
                y * f64::from(viewport_height) / f64::from(height),
            )
        } else {
            (x, y)
        };
        let size = if select {
            [viewport_width, viewport_height]
        } else {
            [width, height]
        };
        let pick_radius = if select {
            radius.map(|radius| radius * stretch)
        } else {
            radius
        };
        let pointed = self
            .pick(Some(&scene), px, py, Some(size), pick_radius, select)
            .map_err(|e| format!("{method}: {e}"))?;
        let outside = !(x >= 0.0 && y >= 0.0 && x < f64::from(width) && y < f64::from(height));
        let mut result = json!({
            "x": x,
            "y": y,
            "width": width,
            "height": height,
        });
        if let Some(map) = result.as_object_mut() {
            pointed.describe(map);
            if pointed.kind().is_none() {
                map.insert("outside".into(), outside.into());
            }
            if select {
                let state = self.state_json()?;
                for key in ["picked", "highlight", "highlightEdge", "highlightVertex"] {
                    map.insert(key.into(), state.get(key).cloned().unwrap_or(Value::Null));
                }
            }
        }
        Ok(result)
    }

    fn ui_highlight(
        &mut self,
        session: &mut Session,
        params: Option<&Value>,
    ) -> Result<Value, String> {
        let method = "ui.highlight";
        let map = fields(
            method,
            params,
            &["face", "edge", "between", "vertex", "corner", "feature"],
        )?;
        let feature = match map.get("feature") {
            None | Some(Value::Null) => None,
            Some(Value::String(feature)) => Some(feature.clone()),
            // An id may be given as a number, as the commands take it.
            Some(Value::Number(id)) if id.is_u64() => Some(id.to_string()),
            Some(_) => {
                return Err(format!(
                    "{method}: `feature` must be a feature id (3 or \"F3\") or name"
                ));
            }
        };
        let of_edge = map.get("edge").is_some() || map.get("between").is_some();
        let of_vertex = map.get("vertex").is_some() || map.get("corner").is_some();
        if of_edge || of_vertex {
            if map.get("face").is_some() || (of_edge && of_vertex) {
                return Err(format!(
                    "{method}: give one of `face`, `edge`, `between`, `vertex` and `corner`"
                ));
            }
            return if of_edge {
                self.highlight_edge(session, &map, feature)
            } else {
                self.highlight_vertex(session, &map, feature)
            };
        }
        let face = match map.get("face") {
            None => {
                return Err(format!(
                    "{method}: missing parameter `face`: a face name from document.inspect, or null to remove the highlight (or `edge` or `between` for an edge, `vertex` or `corner` for a vertex)"
                ));
            }
            Some(Value::Null) => {
                if feature.is_some() {
                    return Err(format!(
                        "{method}: `feature` says which body has `face`; without a face there is nothing to highlight"
                    ));
                }
                self.state.highlight = None;
                self.state.highlight_edge = None;
                self.state.highlight_vertex = None;
                return Ok(json!({ "highlight": null, "faces": 0 }));
            }
            Some(Value::String(face)) => face,
            Some(_) => return Err(format!("{method}: `face` must be a face name or null")),
        };
        let Some(scene) = self.sync(session) else {
            return Err(match self.problem() {
                Some(problem) => format!("{method}: {problem}"),
                None => format!("{method}: no document is open; run file.new or file.open first"),
            });
        };
        let found = scene
            .faces_named(feature.as_deref(), face)
            .map_err(|e| format!("{method}: {e}"))?;
        let shown =
            craft3d_render::Highlight::new(self.tokens.face_highlight, found.ids.iter().copied());
        self.state.highlight = Some(crate::state::FaceRef {
            feature: Some(found.feature.clone()),
            face: found.face.clone(),
        });
        // One thing is pointed at.
        self.state.highlight_edge = None;
        self.state.highlight_vertex = None;
        let mut result = json!({
            "highlight": { "feature": found.feature, "face": found.face },
            "name": found.body,
            "faces": found.ids.len(),
        });
        // More pieces than the renderers take runs of: said, not hidden.
        if let (Some(map), (_, left_out @ 1..)) = (result.as_object_mut(), shown) {
            map.insert("facesNotShown".into(), left_out.into());
        }
        Ok(result)
    }

    /// `ui.highlight` for an edge: `edge` or `between`.
    fn highlight_edge(
        &mut self,
        session: &mut Session,
        map: &Fields<'_>,
        feature: Option<String>,
    ) -> Result<Value, String> {
        let method = "ui.highlight";
        let edge = match map.get("edge") {
            None => None,
            Some(Value::String(edge)) => Some(edge.as_str()),
            Some(_) => {
                return Err(format!(
                    "{method}: `edge` must be an edge name from document.edges (`face`: null removes the highlight)"
                ));
            }
        };
        let between: Vec<String> = match map.get("between") {
            None => Vec::new(),
            Some(Value::Array(names)) => match names.as_slice() {
                [Value::String(a), Value::String(b)] => vec![a.clone(), b.clone()],
                _ => return Err(format!("{method}: `between` takes two face names")),
            },
            Some(_) => return Err(format!("{method}: `between` takes two face names")),
        };
        let Some(scene) = self.sync(session) else {
            return Err(match self.problem() {
                Some(problem) => format!("{method}: {problem}"),
                None => format!("{method}: no document is open; run file.new or file.open first"),
            });
        };
        let found = scene
            .edges_named(feature.as_deref(), edge, &between)
            .map_err(|e| format!("{method}: {e}"))?;
        let shown = crate::state::EdgeRef {
            feature: Some(found.feature.clone()),
            edge: found.edge.clone(),
            between: found.between.clone(),
        };
        let result = json!({
            "highlightEdge": shown,
            "name": found.body,
            "edges": found.edges.len(),
        });
        self.state.highlight_edge = Some(shown);
        // One thing is pointed at.
        self.state.highlight = None;
        self.state.highlight_vertex = None;
        Ok(result)
    }

    /// `ui.highlight` for a vertex: `vertex` or `corner`.
    fn highlight_vertex(
        &mut self,
        session: &mut Session,
        map: &Fields<'_>,
        feature: Option<String>,
    ) -> Result<Value, String> {
        let method = "ui.highlight";
        let vertex = match map.get("vertex") {
            None => None,
            Some(Value::String(vertex)) => Some(vertex.as_str()),
            Some(_) => {
                return Err(format!(
                    "{method}: `vertex` must be a vertex name from document.edges (`face`: null removes the highlight)"
                ));
            }
        };
        let wrong = || {
            format!(
                "{method}: `corner` takes one to {} face names",
                craft3d_engine::MAX_CORNER_FACES
            )
        };
        let corner: Vec<String> = match map.get("corner") {
            None => Vec::new(),
            Some(Value::Array(names))
                if !names.is_empty() && names.len() <= craft3d_engine::MAX_CORNER_FACES =>
            {
                names
                    .iter()
                    .map(|name| name.as_str().map(str::to_string))
                    .collect::<Option<_>>()
                    .ok_or_else(wrong)?
            }
            Some(_) => return Err(wrong()),
        };
        let Some(scene) = self.sync(session) else {
            return Err(match self.problem() {
                Some(problem) => format!("{method}: {problem}"),
                None => format!("{method}: no document is open; run file.new or file.open first"),
            });
        };
        let found = scene
            .vertices_named(feature.as_deref(), vertex, &corner)
            .map_err(|e| format!("{method}: {e}"))?;
        let shown = crate::state::VertexRef {
            feature: Some(found.feature.clone()),
            vertex: found.vertex.clone(),
            corner: found.corner.clone(),
        };
        let result = json!({
            "highlightVertex": shown,
            "name": found.body,
            "vertices": found.vertices.len(),
        });
        self.state.highlight_vertex = Some(shown);
        // One thing is pointed at.
        self.state.highlight = None;
        self.state.highlight_edge = None;
        Ok(result)
    }

    fn ui_screenshot(
        &mut self,
        session: &mut Session,
        params: Option<&Value>,
    ) -> Result<Value, String> {
        let method = "ui.screenshot";
        let map = fields(method, params, &["path", "width", "height", "scale"])?;
        let path = match map.get("path") {
            None | Some(Value::Null) => None,
            Some(Value::String(path)) => Some(path.as_str()),
            Some(_) => return Err(format!("{method}: `path` must be a string")),
        };
        let [viewport_width, viewport_height] = self.viewport_size();
        let width = side(method, map, "width")?.unwrap_or(viewport_width);
        let height = side(method, map, "height")?.unwrap_or(viewport_height);
        let scale = match map.get("scale") {
            None | Some(Value::Null) => f64::from(self.pixels_per_point()),
            Some(value) => value
                .as_f64()
                .filter(|s| (0.25..=8.0).contains(s))
                .ok_or_else(|| format!("{method}: `scale` must be a number from 0.25 to 8"))?,
        };
        // Refuse a path the session may not write before rendering.
        if let Some(path) = path {
            session
                .workspace()
                .resolve_write(path)
                .map_err(|e| format!("{method}: {e}"))?;
        }
        let scene = self.sync(session);
        let picture = viewport::picture_marked(
            scene.as_deref(),
            &self.state.camera,
            Quality {
                width,
                height,
                ssaa: crate::shell::REST_SSAA,
            },
            &self.tokens,
            self.state.selected_feature.as_deref(),
            self.face_highlight(scene.as_deref()),
            &self.marks(scene.as_deref()),
            self.opacity(),
            scale,
        )
        .map_err(|e| format!("{method}: {e}"))?;
        let png = craft3d_render::encode_png(picture.width, picture.height, &picture.rgb)
            .map_err(|e| format!("{method}: {e}"))?;
        let mut result = json!({
            "width": picture.width,
            "height": picture.height,
            "triangles": picture.triangles,
            "coverage": picture.coverage,
            "overlayCoverage": picture.overlay_coverage,
            "bytes": png.len(),
        });
        if let Some(map) = result.as_object_mut() {
            match path {
                Some(path) => {
                    session
                        .write_output(path, &png)
                        .map_err(|e| format!("{method}: {e}"))?;
                    map.insert("path".into(), path.into());
                }
                None => {
                    map.insert("mime".into(), "image/png".into());
                    map.insert("base64".into(), craft3d_engine::encode_base64(&png).into());
                }
            }
        }
        Ok(result)
    }
}
