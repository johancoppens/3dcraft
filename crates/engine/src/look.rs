//! How a picture is lit: the plain look or the studio look
//! (`craft3d_render::studio`). View state, never document data: the same
//! object is `view.render`'s and `craft3d-cli render`'s `look` and the
//! window's `ui.set {"look": …}`.

use crate::CommandError;
use craft3d_render::studio::bundled::{ENVIRONMENTS, environment};
use craft3d_render::studio::{Look, Prepared, Tonemap};
use serde_json::{Value, json};
use std::sync::Arc;

/// The keys a look takes.
pub const LOOK_KEYS: [&str; 10] = [
    "mode",
    "environment",
    "exposure",
    "rotation",
    "ground",
    "ao",
    "shadows",
    "tonemap",
    "light",
    "background",
];

/// The params text of a look, for the commands' documentation.
pub const LOOK_PARAMS: &str = r#"{"mode": "plain|studio" = "studio", "environment": "studio|outdoor|workshop" = "studio", "exposure": stops from -10 to 10 = 0, "rotation": degrees = 0, "ground": boolean = true, "ao": boolean = true, "shadows": boolean = true, "tonemap": "agx|aces|linear" = "agx", "light": [azimuth, elevation] in degrees = the environment's brightest direction, "background": "environment|plain" = "environment"}"#;

/// A look: plain (the default) or studio with its options.
#[derive(Clone, Debug, PartialEq)]
pub struct LookSpec {
    pub studio: bool,
    pub environment: String,
    pub look: Look,
}

impl Default for LookSpec {
    fn default() -> Self {
        LookSpec {
            studio: false,
            environment: "studio".to_string(),
            look: Look::default(),
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
        })
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
            json!({"background": "black"}),
            json!({"background": true}),
        ] {
            assert!(LookSpec::patched(&plain, &bad).is_err(), "{bad}");
        }
        let round = LookSpec::patched(&plain, &studio.json()).unwrap();
        assert_eq!(round, studio);
        let flat = LookSpec::patched(&studio, &json!({"background": "plain"})).unwrap();
        assert!(flat.look.plain_background && !studio.look.plain_background);
        assert_eq!(LookSpec::patched(&plain, &flat.json()).unwrap(), flat);
    }
}
