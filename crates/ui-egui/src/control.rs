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
        summary: "Returns the UI state (camera, panels, selectedFeature, notice), the viewport size, what the scene holds, the feature tree rows, which renderer the viewport uses (gpu or cpu, the adapter, whether the GPU draws directly on the window's device and with how many samples, why it fell back) and how the viewport was last rendered.",
    },
    UiMethod {
        name: "ui.set",
        params: r#"{"view": "iso|front|back|right|left|top|bottom", "camera": {"target": [x, y, z], "distance": number, "yaw": degrees, "pitch": degrees, "projection": "perspective|orthographic", "fovY": degrees, "autoFit": boolean}, "panels": {"tree": boolean, "status": boolean}, "selectedFeature": id or null, "notice": string}"#,
        summary: "Changes the fields given and leaves the rest. An unknown field or a wrong type is an error, and then nothing has changed. Numbers out of range are clamped.",
    },
    UiMethod {
        name: "ui.fit",
        params: r#"{"view": "iso|front|back|right|left|top|bottom"}"#,
        summary: "Frames everything (bodies and sketches), from the named view when one is given, and keeps following the model as it grows.",
    },
    UiMethod {
        name: "ui.screenshot",
        params: r#"{"path": string, "width": integer = the viewport's, "height": integer = the viewport's, "scale": number = the window's pixels per point}"#,
        summary: "A PNG of the viewport as it is framed now: bodies by the CPU reference renderer (the oracle, whichever renderer the window uses), sketches and axes over them. Written under the write root when path is given, otherwise returned as base64.",
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
        fields("ui.get", params, &[])?;
        let scene = self.sync(session);
        let [width, height] = self.viewport_size();
        let vec3 = |v: craft3d_math::Vec3| json!([v.x, v.y, v.z]);
        let scene_json = scene.as_deref().map_or(Value::Null, |scene| {
            json!({
                "revision": scene.revision,
                "bodies": scene.bodies.len(),
                "sketches": scene.sketches.len(),
                "triangles": scene.triangles(),
                "tolerance": scene.tolerance,
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
                    "selected": row.selected,
                })
            })
            .collect();
        let document = session.document_status().map_or(Value::Null, |status| {
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
            "lastCommand": self.last_command.as_ref().map(|report| json!({
                "label": report.label, "ok": report.ok, "text": report.text,
            })),
            "problem": self.problem(),
        }))
    }

    fn ui_set(&mut self, session: &mut Session, params: Option<&Value>) -> Result<Value, String> {
        let patch = match params {
            None | Some(Value::Null) => return Err("ui.set: nothing to set".to_string()),
            Some(patch) => patch,
        };
        self.state
            .apply_patch(patch)
            .map_err(|e| format!("ui.set: {e}"))?;
        // A selection that names no feature is dropped here, not later.
        self.sync(session);
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
        let picture = viewport::picture(
            scene.as_deref(),
            &self.state.camera,
            Quality {
                width,
                height,
                ssaa: crate::shell::REST_SSAA,
            },
            &self.tokens,
            self.state.selected_feature.as_deref(),
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
