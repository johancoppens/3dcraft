//! How a picture is lit: the plain look or the studio look
//! (`craft3d_render::studio`). View state, never document data: the same
//! object is `view.render`'s and `craft3d-cli render`'s `look` and the
//! window's `ui.set {"look": …}`.

use crate::CommandError;
use craft3d_math::Bbox3;
use craft3d_render::studio::bundled::{ENVIRONMENTS, environment};
use craft3d_render::studio::polish::{Dof, Focus, MAX_ACCUMULATE};
use craft3d_render::studio::{Look, Prepared, Tonemap};
use serde_json::{Value, json};
use std::sync::Arc;

/// The keys a look takes.
pub const LOOK_KEYS: [&str; 17] = [
    "mode",
    "environment",
    "exposure",
    "rotation",
    "ground",
    "ao",
    "shadows",
    "tonemap",
    "light",
    "reflections",
    "contactShadows",
    "specularShadow",
    "groundReflection",
    "bloom",
    "dof",
    "accumulate",
    "background",
];

/// The f-number of a lens given without one.
pub const DEFAULT_FSTOP: f64 = 2.8;
/// The frames the window averages while its camera stands still, when the
/// look does not say.
pub const VIEWPORT_ACCUMULATE: u32 = 16;

/// The params text of a look, for the commands' documentation.
pub const LOOK_PARAMS: &str = r#"{"mode": "plain|studio" = "studio", "environment": "studio|outdoor|workshop" = "studio", "exposure": stops from -10 to 10 = 0, "rotation": degrees = 0, "ground": boolean = true, "ao": boolean = true, "shadows": boolean = true, "tonemap": "agx|aces|linear" = "agx", "light": [azimuth, elevation] in degrees = the environment's brightest direction, "background": "environment|plain" = "environment", "reflections": boolean = true, "contactShadows": boolean = true, "specularShadow": boolean = true, "groundReflection": 0 to 1 = 0, "bloom": 0 to 1 = 0.04, "dof": {"fstop": number = 2.8, "focus": distance | [x, y, z] | "picked" = the centre of the model} | null = null, "accumulate": frames from 1 to 256 = 1 (the window: 16)}"#;

/// A look: plain (the default) or studio with its options.
#[derive(Clone, Debug, PartialEq)]
pub struct LookSpec {
    pub studio: bool,
    pub environment: String,
    /// The look's fields but the lens (`look.dof` stays `None` until
    /// [`LookSpec::resolved`]) and the frames.
    pub look: Look,
    /// Depth of field, if asked for.
    pub dof: Option<DofSpec>,
    /// The frames of a still picture, if given (each consumer has its
    /// default: `view.render` 1, the window [`VIEWPORT_ACCUMULATE`]).
    pub accumulate: Option<u32>,
}

/// A thin lens as a look gives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DofSpec {
    pub fstop: f64,
    pub focus: FocusSpec,
}

/// Where a lens focuses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FocusSpec {
    /// The centre of the model's box.
    Centre,
    /// This distance along the view direction, in model units.
    Distance(f64),
    /// The plane through this point, square to the view.
    Point([f64; 3]),
    /// The point of the window's last pick (the centre without one).
    Picked,
}

impl Default for LookSpec {
    fn default() -> Self {
        LookSpec {
            studio: false,
            environment: "studio".to_string(),
            look: Look::polished(),
            dof: None,
            accumulate: None,
        }
    }
}

impl LookSpec {
    /// `base` with the fields of `value` (an object of [`LOOK_KEYS`]) set;
    /// without `mode` it stays as it was. An unknown key, a wrong type, an
    /// unknown environment or tone curve is an error.
    pub fn patched(base: &LookSpec, value: &Value) -> Result<LookSpec, String> {
        let Value::Object(map) = value else {
            return Err("`look` must be an object".to_string());
        };
        if let Some(key) = map.keys().find(|k| !LOOK_KEYS.contains(&k.as_str())) {
            let key: String = key.chars().take(40).collect();
            return Err(format!(
                "`look` has no field `{key}`; it takes {}",
                LOOK_KEYS.join(", ")
            ));
        }
        let mut out = base.clone();
        let text = |key: &str| -> Result<Option<String>, String> {
            match map.get(key) {
                None => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.chars().take(40).collect())),
                Some(_) => Err(format!("`look.{key}` must be a string")),
            }
        };
        let number = |key: &str| -> Result<Option<f64>, String> {
            match map.get(key) {
                None => Ok(None),
                Some(v) => v
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .map(Some)
                    .ok_or_else(|| format!("`look.{key}` must be a finite number")),
            }
        };
        let flag = |key: &str| -> Result<Option<bool>, String> {
            match map.get(key) {
                None => Ok(None),
                Some(Value::Bool(b)) => Ok(Some(*b)),
                Some(_) => Err(format!("`look.{key}` must be true or false")),
            }
        };
        out.studio = match text("mode")?.as_deref() {
            None => base.studio,
            Some("studio") => true,
            Some("plain") => false,
            Some(other) => {
                return Err(format!("`look.mode` `{other}` is not plain or studio"));
            }
        };
        if let Some(name) = text("environment")? {
            if !ENVIRONMENTS.contains(&name.as_str()) {
                return Err(format!(
                    "`look.environment` `{name}` is not one of {}",
                    ENVIRONMENTS.join(", ")
                ));
            }
            out.environment = name;
        }
        if let Some(e) = number("exposure")? {
            out.look.exposure = e.clamp(-10.0, 10.0);
        }
        if let Some(r) = number("rotation")? {
            out.look.rotation = r.rem_euclid(360.0);
        }
        if let Some(b) = flag("ground")? {
            out.look.ground = b;
        }
        if let Some(b) = flag("ao")? {
            out.look.ao = b;
        }
        if let Some(b) = flag("shadows")? {
            out.look.shadows = b;
        }
        if let Some(name) = text("tonemap")? {
            out.look.tonemap = Tonemap::from_name(&name)
                .ok_or_else(|| format!("`look.tonemap` `{name}` is not agx, aces or linear"))?;
        }
        match text("background")?.as_deref() {
            None => {}
            Some("environment") => out.look.plain_background = false,
            Some("plain") => out.look.plain_background = true,
            Some(other) => {
                return Err(format!(
                    "`look.background` `{other}` is not environment or plain"
                ));
            }
        }
        match map.get("light") {
            None => {}
            Some(Value::Null) => out.look.light = None,
            Some(Value::Array(pair)) => {
                let [Some(az), Some(el)] = [pair.first(), pair.get(1)]
                    .map(|v| v.and_then(Value::as_f64).filter(|n| n.is_finite()))
                else {
                    return Err("`look.light` must be [azimuth, elevation] in degrees".to_string());
                };
                if pair.len() != 2 {
                    return Err("`look.light` must be [azimuth, elevation] in degrees".to_string());
                }
                out.look.light = Some([az.rem_euclid(360.0), el.clamp(-90.0, 90.0)]);
            }
            Some(_) => {
                return Err(
                    "`look.light` must be [azimuth, elevation] in degrees or null".to_string(),
                );
            }
        }
        if let Some(b) = flag("reflections")? {
            out.look.ssr = b;
        }
        if let Some(b) = flag("contactShadows")? {
            out.look.contact_shadows = b;
        }
        if let Some(b) = flag("specularShadow")? {
            out.look.specular_shadow = b;
        }
        if let Some(g) = number("groundReflection")? {
            out.look.ground_reflection = g.clamp(0.0, 1.0);
        }
        if let Some(b) = number("bloom")? {
            out.look.bloom = b.clamp(0.0, 1.0);
        }
        match map.get("accumulate") {
            None => {}
            Some(Value::Null) => out.accumulate = None,
            Some(v) => {
                let n = v
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .ok_or("`look.accumulate` must be a number of frames")?;
                out.accumulate = Some(n.round().clamp(1.0, f64::from(MAX_ACCUMULATE)) as u32);
            }
        }
        match map.get("dof") {
            None => {}
            Some(Value::Null) => out.dof = None,
            Some(Value::Object(lens)) => {
                if let Some(key) = lens
                    .keys()
                    .find(|k| !["fstop", "focus"].contains(&k.as_str()))
                {
                    let key: String = key.chars().take(40).collect();
                    return Err(format!(
                        "`look.dof` has no field `{key}`; it takes fstop, focus"
                    ));
                }
                let mut dof = out.dof.unwrap_or(DofSpec {
                    fstop: DEFAULT_FSTOP,
                    focus: FocusSpec::Centre,
                });
                if let Some(f) = lens.get("fstop") {
                    let f = f
                        .as_f64()
                        .filter(|f| f.is_finite())
                        .ok_or("`look.dof.fstop` must be a finite number")?;
                    dof.fstop = f.clamp(0.5, 64.0);
                }
                match lens.get("focus") {
                    None => {}
                    Some(Value::Null) => dof.focus = FocusSpec::Centre,
                    Some(Value::String(s)) if s == "picked" => dof.focus = FocusSpec::Picked,
                    Some(Value::String(s)) if s == "centre" || s == "center" => {
                        dof.focus = FocusSpec::Centre;
                    }
                    Some(Value::Number(n)) => {
                        let d = n
                            .as_f64()
                            .filter(|d| d.is_finite() && *d > 0.0)
                            .ok_or("`look.dof.focus` must be a positive distance")?;
                        dof.focus = FocusSpec::Distance(d);
                    }
                    Some(Value::Array(p)) => {
                        let xyz: Vec<f64> = p
                            .iter()
                            .filter_map(|v| v.as_f64().filter(|x| x.is_finite()))
                            .collect();
                        let [x, y, z] = xyz.as_slice() else {
                            return Err("`look.dof.focus` must be [x, y, z]".to_string());
                        };
                        if p.len() != 3 {
                            return Err("`look.dof.focus` must be [x, y, z]".to_string());
                        }
                        dof.focus = FocusSpec::Point([*x, *y, *z]);
                    }
                    Some(_) => {
                        return Err("`look.dof.focus` must be a distance, [x, y, z], \"picked\" or \"centre\"".to_string());
                    }
                }
                out.dof = Some(dof);
            }
            Some(_) => return Err("`look.dof` must be an object or null".to_string()),
        }
        Ok(out)
    }

    /// The look as JSON, every field.
    pub fn json(&self) -> Value {
        json!({
            "mode": if self.studio { "studio" } else { "plain" },
            "environment": self.environment,
            "exposure": self.look.exposure,
            "rotation": self.look.rotation,
            "ground": self.look.ground,
            "ao": self.look.ao,
            "shadows": self.look.shadows,
            "tonemap": self.look.tonemap.name(),
            "light": self.look.light,
            "background": if self.look.plain_background { "plain" } else { "environment" },
            "reflections": self.look.ssr,
            "contactShadows": self.look.contact_shadows,
            "specularShadow": self.look.specular_shadow,
            "groundReflection": self.look.ground_reflection,
            "bloom": self.look.bloom,
            "dof": self.dof.map(|d| json!({
                "fstop": d.fstop,
                "focus": match d.focus {
                    FocusSpec::Centre => json!("centre"),
                    FocusSpec::Distance(z) => json!(z),
                    FocusSpec::Point(p) => json!(p),
                    FocusSpec::Picked => json!("picked"),
                },
            })),
            "accumulate": self.accumulate,
        })
    }

    /// The look to draw a model within `model` with: the lens focused
    /// (on the model's centre, or the last pick `picked`, as asked) for a
    /// document whose length unit is `mm_per_unit` millimetres, and the
    /// frames (`default_frames` when the look does not say).
    pub fn resolved(
        &self,
        model: &Bbox3,
        mm_per_unit: f64,
        picked: Option<[f64; 3]>,
        default_frames: u32,
    ) -> Look {
        let centre = model
            .corners()
            .map(|(lo, hi)| {
                let c = (lo + hi) * 0.5;
                [c.x, c.y, c.z]
            })
            .unwrap_or([0.0; 3]);
        Look {
            dof: self.dof.map(|d| Dof {
                fstop: d.fstop,
                focus: match d.focus {
                    FocusSpec::Centre => Focus::Point(centre),
                    FocusSpec::Distance(z) => Focus::Distance(z),
                    FocusSpec::Point(p) => Focus::Point(p),
                    FocusSpec::Picked => Focus::Point(picked.unwrap_or(centre)),
                },
                mm_per_unit,
            }),
            accumulate: self
                .accumulate
                .unwrap_or(default_frames)
                .clamp(1, MAX_ACCUMULATE),
            ..self.look
        }
    }

    /// The prepared environment of a studio look, without waiting: when it
    /// is not prepared yet, `None`, and on the desktop its preparation is
    /// started on a thread of its own (once per environment; in the
    /// browser the page fetches and prepares it). What the window draws
    /// with, so that preparing never holds up a frame.
    pub fn prepared_if_ready(&self) -> Option<Arc<Prepared>> {
        if !self.studio {
            return None;
        }
        if let Some(env) = craft3d_render::studio::bundled::ready(&self.environment) {
            return Some(env);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            static STARTED: std::sync::Mutex<std::collections::BTreeSet<String>> =
                std::sync::Mutex::new(std::collections::BTreeSet::new());
            let first = STARTED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(self.environment.clone());
            if first {
                let name = self.environment.clone();
                // A failure is the same failure when it is asked for
                // again where it is waited for (`prepared`).
                let _ = std::thread::Builder::new()
                    .name("craft3d environment".to_string())
                    .spawn(move || {
                        let _ = environment(&name);
                    });
            }
        }
        None
    }

    /// The prepared environment of a studio look (`None` for the plain
    /// look). Preparing takes a moment the first time per environment.
    pub fn prepared(&self) -> Result<Option<Arc<Prepared>>, String> {
        if !self.studio {
            return Ok(None);
        }
        environment(&self.environment)
            .map(Some)
            .map_err(|e| e.to_string())
    }
}

/// Whether the environment `name` is prepared already (always, on the
/// desktop, after its first use; in the browser once its page fetched it).
pub fn environment_ready(name: &str) -> bool {
    craft3d_render::studio::bundled::ready(name).is_some()
}

/// Prepares the environment `name` from the bytes of its `.hdr` file (the
/// browser's page, which fetches the bundled ones on demand).
pub fn register_environment(name: &str, bytes: &[u8]) -> Result<(), String> {
    if !ENVIRONMENTS.contains(&name) {
        return Err(format!("no environment {name:?}"));
    }
    craft3d_render::studio::bundled::register(name, bytes)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `view.render`'s `look`, or the plain look when it is not given.
pub(crate) fn look_param(cmd: &str, value: Option<&Value>) -> Result<LookSpec, CommandError> {
    match value {
        None | Some(Value::Null) => Ok(LookSpec::default()),
        // A look given is the studio look unless it says plain.
        Some(value) => LookSpec::patched(
            &LookSpec {
                studio: true,
                ..LookSpec::default()
            },
            value,
        )
        .map_err(|e| CommandError::new(format!("{cmd}: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_look_is_parsed_and_bad_fields_are_errors() {
        let plain = LookSpec::default();
        let studio = LookSpec::patched(
            &plain,
            &json!({"mode": "studio", "environment": "outdoor", "exposure": 99}),
        )
        .unwrap();
        assert!(studio.studio);
        assert!(
            !LookSpec::patched(&plain, &json!({"ao": false}))
                .unwrap()
                .studio
        );
        assert!(look_param("view.render", Some(&json!({}))).unwrap().studio);
        assert!(!look_param("view.render", None).unwrap().studio);
        assert_eq!(studio.environment, "outdoor");
        assert_eq!(studio.look.exposure, 10.0);
        let back = LookSpec::patched(&studio, &json!({"mode": "plain"})).unwrap();
        assert!(!back.studio && back.environment == "outdoor");
        assert!(!LookSpec::patched(&back, &json!({})).unwrap().studio);
        for bad in [
            json!(1),
            json!({"mode": "photo"}),
            json!({"environment": "moon"}),
            json!({"exposure": "high"}),
            json!({"tonemap": "reinhard"}),
            json!({"light": [1]}),
            json!({"light": [1, 2, 3]}),
            json!({"ground": 1}),
            json!({"colour": 1}),
            json!({"dof": 1}),
            json!({"dof": {"fstop": "wide"}}),
            json!({"dof": {"focus": -1}}),
            json!({"dof": {"focus": [1, 2]}}),
            json!({"dof": {"focus": "there"}}),
            json!({"dof": {"lens": 50}}),
            json!({"accumulate": "many"}),
            json!({"bloom": true}),
            json!({"reflections": 1}),
            json!({"background": "black"}),
            json!({"background": true}),
        ] {
            assert!(LookSpec::patched(&plain, &bad).is_err(), "{bad}");
        }
        let round = LookSpec::patched(&plain, &studio.json()).unwrap();
        assert_eq!(round, studio);
        let lens = LookSpec::patched(
            &studio,
            &json!({"dof": {"fstop": 4, "focus": [1, 2, 3]}, "accumulate": 1000, "bloom": 7, "reflections": false}),
        )
        .unwrap();
        assert_eq!(lens.accumulate, Some(MAX_ACCUMULATE));
        assert_eq!(lens.look.bloom, 1.0);
        assert!(!lens.look.ssr);
        assert_eq!(LookSpec::patched(&plain, &lens.json()).unwrap(), lens);
        let model = Bbox3::from_points([
            craft3d_math::Vec3::ZERO,
            craft3d_math::Vec3::new(2.0, 2.0, 2.0),
        ]);
        let look = lens.resolved(&model, 25.4, None, 16);
        assert_eq!(look.accumulate, MAX_ACCUMULATE);
        let dof = look.dof.unwrap();
        assert_eq!(
            (dof.fstop, dof.focus, dof.mm_per_unit),
            (4.0, Focus::Point([1.0, 2.0, 3.0]), 25.4)
        );
        let picked = LookSpec::patched(&plain, &json!({"dof": {"focus": "picked"}})).unwrap();
        let at = |p| picked.resolved(&model, 1.0, p, 16).dof.unwrap().focus;
        assert_eq!(at(None), Focus::Point([1.0; 3]));
        assert_eq!(at(Some([5.0, 0.0, 0.0])), Focus::Point([5.0, 0.0, 0.0]));
        assert_eq!(picked.resolved(&model, 1.0, None, 16).accumulate, 16);
        let flat = LookSpec::patched(&studio, &json!({"background": "plain"})).unwrap();
        assert!(flat.look.plain_background && !studio.look.plain_background);
        assert_eq!(LookSpec::patched(&plain, &flat.json()).unwrap(), flat);
    }
}
