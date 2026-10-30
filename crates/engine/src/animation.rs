//! The tracks of `view.replay`'s animations: easing, a camera path of
//! keyframes with orbit and dolly, and variable tracks
//! (docs/design/rendering.md › *Animations*).
//!
//! Everything is a function of `s`, the place in the animation from 0
//! (its first frame) to 1 (its last), so a frame does not depend on the
//! frames before it.

use crate::CommandError;
use crate::commands::fail;
use craft3d_math::Vec3;
use craft3d_render::{Camera, Projection};
use serde_json::Value;

/// Most keyframes of one track or of the camera.
pub const MAX_KEYS: usize = 256;
/// Most variable tracks of one animation.
pub const MAX_TRACKS: usize = 64;
/// Largest value a track or a camera key may give.
const MAX_VALUE: f64 = 1e12;

/// How a track moves between two keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    Linear,
    /// Slow start: `u²`.
    In,
    /// Slow end: `1 − (1 − u)²`.
    Out,
    /// Slow start and end: `3u² − 2u³` (smoothstep).
    InOut,
}

impl Easing {
    pub const NAMES: &'static str = "linear, in, out or inOut";

    fn parse(command: &str, what: &str, value: Option<&Value>) -> Result<Easing, CommandError> {
        match value {
            None | Some(Value::Null) => Ok(Easing::Linear),
            Some(Value::String(s)) => match s.as_str() {
                "linear" => Ok(Easing::Linear),
                "in" => Ok(Easing::In),
                "out" => Ok(Easing::Out),
                "inOut" => Ok(Easing::InOut),
                other => {
                    let other: String = other.chars().take(40).collect();
                    fail(format!(
                        "{command}: {what}: unknown easing `{other}`; use {}",
                        Easing::NAMES
                    ))
                }
            },
            Some(_) => fail(format!(
                "{command}: {what}: `easing` must be one of {}",
                Easing::NAMES
            )),
        }
    }

    /// `u` (clamped to 0..1) eased.
    pub fn apply(self, u: f64) -> f64 {
        let u = if u.is_finite() {
            u.clamp(0.0, 1.0)
        } else {
            0.0
        };
        match self {
            Easing::Linear => u,
            Easing::In => u * u,
            Easing::Out => 1.0 - (1.0 - u) * (1.0 - u),
            Easing::InOut => u * u * (3.0 - 2.0 * u),
        }
    }
}

/// A value from a param: a finite number within ±1e12.
fn number(command: &str, what: &str, value: &Value) -> Result<f64, CommandError> {
    match value.as_f64() {
        Some(n) if n.is_finite() && n.abs() <= MAX_VALUE => Ok(n),
        _ => fail(format!(
            "{command}: {what} must be a number within ±1e12, got {}",
            short(value)
        )),
    }
}

fn short(value: &Value) -> String {
    value.to_string().chars().take(60).collect()
}

/// The place of a key in the animation: a number from 0 to 1.
fn key_time(command: &str, what: &str, value: Option<&Value>) -> Result<f64, CommandError> {
    match value.and_then(Value::as_f64) {
        Some(t) if (0.0..=1.0).contains(&t) => Ok(t),
        _ => fail(format!(
            "{command}: {what}: each key needs `t`, its place in the animation from 0 (the first frame) to 1 (the last)"
        )),
    }
}

/// Times that rise strictly, as keys must.
fn rising(command: &str, what: &str, times: &[f64]) -> Result<(), CommandError> {
    if times.windows(2).any(|w| matches!(w, [a, b] if b <= a)) {
        return fail(format!(
            "{command}: {what}: the keys' `t` must rise strictly from one key to the next"
        ));
    }
    Ok(())
}

/// The segment of `times` that `s` lies in and the place within it:
/// `(i, u)` between key `i` and key `i + 1`; before the first key and
/// after the last the ends hold (`u` 0 or 1).
fn segment(times: &[f64], s: f64) -> (usize, f64) {
    let last = times.len().saturating_sub(1);
    match (times.first(), times.last()) {
        (Some(&first), _) if s <= first || last == 0 => (0, 0.0),
        (_, Some(&end)) if s >= end => (last.saturating_sub(1), 1.0),
        _ => {
            let i = times
                .windows(2)
                .position(|w| matches!(w, [a, b] if s >= *a && s < *b))
                .unwrap_or(0);
            let (a, b) = (
                times.get(i).copied().unwrap_or(0.0),
                times.get(i + 1).copied().unwrap_or(1.0),
            );
            let span = b - a;
            (i, if span > 0.0 { (s - a) / span } else { 1.0 })
        }
    }
}

/// A variable track: `{variable, from, to}` or `{variable, keys: [{t,
/// value}]}`, with an optional `unit` and `easing`.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub variable: String,
    pub times: Vec<f64>,
    pub values: Vec<f64>,
    /// Written after each value (`"mm"`, `"deg"`), or none.
    pub unit: Option<String>,
    pub easing: Easing,
}

impl Track {
    /// The tracks of `animate`.
    pub fn parse_all(command: &str, value: &Value) -> Result<Vec<Track>, CommandError> {
        let Some(items) = value.as_array() else {
            return fail(format!(
                "{command}: `animate` is a list of tracks, [{{\"variable\": name, \"from\": number, \"to\": number}}] or [{{\"variable\": name, \"keys\": [{{\"t\": 0 to 1, \"value\": number}}]}}]"
            ));
        };
        if items.len() > MAX_TRACKS {
            return fail(format!(
                "{command}: {} tracks in `animate`, more than the {MAX_TRACKS} one animation takes",
                items.len()
            ));
        }
        let mut tracks: Vec<Track> = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let track = Track::parse(command, i, item)?;
            if tracks.iter().any(|t| t.variable == track.variable) {
                return fail(format!(
                    "{command}: `animate` has two tracks for `{}`; give one with keys",
                    track.variable
                ));
            }
            tracks.push(track);
        }
        Ok(tracks)
    }

    fn parse(command: &str, index: usize, value: &Value) -> Result<Track, CommandError> {
        let what = format!("animate[{index}]");
        let Some(map) = value.as_object() else {
            return fail(format!(
                "{command}: {what} must be an object {{\"variable\", \"from\", \"to\"}} or {{\"variable\", \"keys\"}}"
            ));
        };
        if let Some(key) = map
            .keys()
            .find(|k| !["variable", "from", "to", "keys", "unit", "easing"].contains(&k.as_str()))
        {
            let key: String = key.chars().take(40).collect();
            return fail(format!(
                "{command}: {what}: unknown field `{key}`; a track has variable, from, to, keys, unit and easing"
            ));
        }
        let Some(variable) = map.get("variable").and_then(Value::as_str) else {
            return fail(format!(
                "{command}: {what}: `variable` must name a variable of the document"
            ));
        };
        if variable.is_empty() || variable.len() > 128 {
            return fail(format!(
                "{command}: {what}: `variable` must name a variable of the document"
            ));
        }
        let unit = match map.get("unit") {
            None | Some(Value::Null) => None,
            Some(Value::String(u))
                if !u.is_empty()
                    && u.len() <= 32
                    && u.chars().all(|c| {
                        c.is_ascii_alphabetic() || c == '/' || c == '^' || c.is_ascii_digit()
                    }) =>
            {
                Some(u.clone())
            }
            Some(_) => {
                return fail(format!(
                    "{command}: {what}: `unit` must be a unit such as \"mm\" or \"deg\""
                ));
            }
        };
        let easing = Easing::parse(command, &what, map.get("easing"))?;
        let ends = (map.get("from"), map.get("to"));
        let (times, values) = match (map.get("keys"), ends) {
            (Some(_), (Some(_), _) | (_, Some(_))) => {
                return fail(format!(
                    "{command}: {what}: give from and to, or keys, not both"
                ));
            }
            (Some(Value::Array(keys)), _) => {
                if keys.is_empty() || keys.len() > MAX_KEYS {
                    return fail(format!(
                        "{command}: {what}: `keys` must have 1 to {MAX_KEYS} keys"
                    ));
                }
                let mut times = Vec::with_capacity(keys.len());
                let mut values = Vec::with_capacity(keys.len());
                for key in keys {
                    times.push(key_time(command, &what, key.get("t"))?);
                    let Some(v) = key.get("value") else {
                        return fail(format!("{command}: {what}: each key needs a `value`"));
                    };
                    values.push(number(command, &format!("{what}: a key's value"), v)?);
                }
                rising(command, &what, &times)?;
                (times, values)
            }
            (Some(_), _) => {
                return fail(format!(
                    "{command}: {what}: `keys` must be a list [{{\"t\": 0 to 1, \"value\": number}}]"
                ));
            }
            (None, (Some(from), Some(to))) => (
                vec![0.0, 1.0],
                vec![
                    number(command, &format!("{what}: `from`"), from)?,
                    number(command, &format!("{what}: `to`"), to)?,
                ],
            ),
            (None, _) => {
                return fail(format!("{command}: {what}: give from and to, or keys"));
            }
        };
        Ok(Track {
            variable: variable.to_string(),
            times,
            values,
            unit,
            easing,
        })
    }

    /// The value at `s`: between two keys the easing moves it from one to
    /// the next; before the first key and after the last, the ends hold.
    pub fn at(&self, s: f64) -> f64 {
        let (i, u) = segment(&self.times, s);
        let a = self.values.get(i).copied().unwrap_or(0.0);
        let b = self.values.get(i + 1).copied().unwrap_or(a);
        a + (b - a) * self.easing.apply(u)
    }

    /// The expression the variable is set to at `s`.
    pub fn expr_at(&self, s: f64) -> String {
        let v = self.at(s);
        match &self.unit {
            Some(unit) => format!("{v} {unit}"),
            None => format!("{v}"),
        }
    }
}

/// A keyframe of the camera: what it gives, the rest from the framed
/// camera.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CameraKey {
    t: f64,
    eye: Option<Vec3>,
    target: Option<Vec3>,
    up: Option<Vec3>,
    fov: Option<f64>,
}

/// `view.replay`'s `camera` (and its `orbit`): keyframes through which
/// the camera moves smoothly, turned about the vertical by `orbit` and
/// moved towards its target by `dolly` over the animation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraPath {
    keys: Vec<CameraKey>,
    /// Degrees about the vertical through the target, over the whole
    /// animation.
    pub orbit: f64,
    /// The distance to the target at the end over the one at the start.
    pub dolly: f64,
    easing: Option<Easing>,
}

impl CameraPath {
    /// The camera path of `camera` with the top-level `orbit` (one or the
    /// other).
    pub fn parse(
        command: &str,
        camera: Option<&Value>,
        orbit: Option<f64>,
    ) -> Result<CameraPath, CommandError> {
        let check_orbit = |o: f64| -> Result<f64, CommandError> {
            if o.is_finite() && o.abs() <= 3600.0 {
                Ok(o)
            } else {
                fail(format!(
                    "{command}: `orbit` must be a number of degrees within ±3600"
                ))
            }
        };
        let mut path = CameraPath {
            orbit: orbit.map(check_orbit).transpose()?.unwrap_or(0.0),
            dolly: 1.0,
            ..CameraPath::default()
        };
        let map = match camera {
            None | Some(Value::Null) => return Ok(path),
            Some(Value::Object(map)) => map,
            Some(_) => {
                return fail(format!(
                    "{command}: `camera` is {{\"keys\": [{{\"t\", \"eye\", \"target\", \"up\", \"fov\"}}], \"orbit\": degrees, \"dolly\": factor, \"easing\": name}}"
                ));
            }
        };
        if let Some(key) = map
            .keys()
            .find(|k| !["keys", "orbit", "dolly", "easing"].contains(&k.as_str()))
        {
            let key: String = key.chars().take(40).collect();
            return fail(format!(
                "{command}: camera: unknown field `{key}`; the camera has keys, orbit, dolly and easing"
            ));
        }
        if let Some(o) = map.get("orbit").filter(|v| !v.is_null()) {
            if orbit.is_some() {
                return fail(format!(
                    "{command}: give `orbit` or `camera.orbit`, not both"
                ));
            }
            path.orbit = check_orbit(number(command, "`camera.orbit`", o)?)?;
        }
        if let Some(d) = map.get("dolly").filter(|v| !v.is_null()) {
            let d = number(command, "`camera.dolly`", d)?;
            if !(0.01..=100.0).contains(&d) {
                return fail(format!(
                    "{command}: `camera.dolly` must be a factor from 0.01 to 100 (the distance to the target at the end over the one at the start)"
                ));
            }
            path.dolly = d;
        }
        path.easing = match map.get("easing") {
            None | Some(Value::Null) => None,
            value => Some(Easing::parse(command, "camera", value)?),
        };
        match map.get("keys") {
            None | Some(Value::Null) => {}
            Some(Value::Array(keys)) if !keys.is_empty() && keys.len() <= MAX_KEYS => {
                for key in keys {
                    path.keys.push(Self::key(command, key)?);
                }
                let times: Vec<f64> = path.keys.iter().map(|k| k.t).collect();
                rising(command, "camera", &times)?;
            }
            Some(_) => {
                return fail(format!(
                    "{command}: `camera.keys` must be a list of 1 to {MAX_KEYS} keys {{\"t\": 0 to 1, \"eye\": [x, y, z], \"target\": [x, y, z], \"up\": [x, y, z], \"fov\": degrees}}"
                ));
            }
        }
        Ok(path)
    }

    fn key(command: &str, value: &Value) -> Result<CameraKey, CommandError> {
        let Some(map) = value.as_object() else {
            return fail(format!(
                "{command}: camera: a key is an object {{\"t\", \"eye\", \"target\", \"up\", \"fov\"}}"
            ));
        };
        if let Some(key) = map
            .keys()
            .find(|k| !["t", "eye", "target", "up", "fov"].contains(&k.as_str()))
        {
            let key: String = key.chars().take(40).collect();
            return fail(format!(
                "{command}: camera: unknown field `{key}` in a key; a key has t, eye, target, up and fov"
            ));
        }
        let point = |name: &str| -> Result<Option<Vec3>, CommandError> {
            match map.get(name) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Array(xyz)) if xyz.len() == 3 => {
                    let what = format!("camera: a key's `{name}`");
                    let c = |i: usize| -> Result<f64, CommandError> {
                        number(command, &what, xyz.get(i).unwrap_or(&Value::Null))
                    };
                    Ok(Some(Vec3::new(c(0)?, c(1)?, c(2)?)))
                }
                Some(_) => fail(format!(
                    "{command}: camera: a key's `{name}` must be [x, y, z]"
                )),
            }
        };
        let up = point("up")?;
        if up.is_some_and(|u| u.normalized().is_none()) {
            return fail(format!(
                "{command}: camera: a key's `up` must have a direction"
            ));
        }
        let fov = match map.get("fov") {
            None | Some(Value::Null) => None,
            Some(v) => match v.as_f64() {
                Some(f) if (1.0..=170.0).contains(&f) => Some(f),
                _ => {
                    return fail(format!(
                        "{command}: camera: a key's `fov` must be degrees from 1 to 170"
                    ));
                }
            },
        };
        Ok(CameraKey {
            t: key_time(command, "camera", map.get("t"))?,
            eye: point("eye")?,
            target: point("target")?,
            up,
            fov,
        })
    }

    /// True when the camera does not move over the animation.
    pub fn is_still(&self) -> bool {
        self.orbit == 0.0 && self.dolly == 1.0 && self.keys.len() <= 1
    }

    /// True when a key gives a field of view (a perspective camera's).
    pub fn has_fov(&self) -> bool {
        self.keys.iter().any(|k| k.fov.is_some())
    }

    /// The camera at `s`, from `base` (the framed camera): through the
    /// keys (a Catmull-Rom spline through their eyes and targets, the up
    /// and the field of view between them), turned by the orbit about the
    /// vertical through its target, moved towards the target by the
    /// dolly. `None` when the camera there has no direction (its eye on
    /// its target, or looking along its up).
    pub fn at(&self, base: &Camera, s: f64) -> Option<Camera> {
        let easing = self.easing.unwrap_or(Easing::Linear);
        let mut camera = *base;
        if !self.keys.is_empty() {
            let times: Vec<f64> = self.keys.iter().map(|k| k.t).collect();
            let (i, u) = segment(&times, s);
            let u = easing.apply(u);
            let field = |k: usize, f: &dyn Fn(&CameraKey) -> Option<Vec3>, fallback: Vec3| {
                self.keys.get(k).and_then(f).unwrap_or(fallback)
            };
            let spline = |f: &dyn Fn(&CameraKey) -> Option<Vec3>, fallback: Vec3| {
                let points: Vec<Vec3> = (0..self.keys.len())
                    .map(|k| field(k, f, fallback))
                    .collect();
                hermite_through(&times, &points, i, u)
            };
            camera.eye = spline(&|k| k.eye, base.eye);
            camera.target = spline(&|k| k.target, base.target);
            let last = self.keys.len().saturating_sub(1);
            let up_a = field(i, &|k| k.up, base.up);
            let up_b = field((i + 1).min(last), &|k| k.up, base.up);
            camera.up = (up_a + (up_b - up_a) * u).normalized().unwrap_or(base.up);
            if let Projection::Perspective { fov_y_deg } = base.projection {
                let fov = |k: usize| {
                    self.keys
                        .get(k)
                        .and_then(|key| key.fov)
                        .unwrap_or(fov_y_deg)
                };
                let (a, b) = (fov(i), fov((i + 1).min(last)));
                camera.projection = Projection::Perspective {
                    fov_y_deg: a + (b - a) * u,
                };
            }
        }
        let progress = easing.apply(s);
        if self.dolly != 1.0 {
            let factor = 1.0 + (self.dolly - 1.0) * progress;
            camera.eye = camera.target + (camera.eye - camera.target) * factor;
            if let Projection::Orthographic { half_height } = camera.projection {
                camera.projection = Projection::Orthographic {
                    half_height: half_height * factor,
                };
            }
        }
        if self.orbit != 0.0 {
            let (sn, c) = (self.orbit * progress).to_radians().sin_cos();
            let turn = |v: Vec3| Vec3::new(c * v.x - sn * v.y, sn * v.x + c * v.y, v.z);
            camera.eye = camera.target + turn(camera.eye - camera.target);
            camera.up = turn(camera.up);
        }
        let dir = (camera.target - camera.eye).normalized()?;
        dir.cross(camera.up).normalized()?;
        let finite = |v: Vec3| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
        (finite(camera.eye) && finite(camera.target)).then_some(camera)
    }
}

/// The Catmull-Rom spline through `points` at the times `times` (strictly
/// rising), in the segment from key `i` to key `i + 1` at `u` from 0 to 1:
/// a cubic Hermite segment whose tangent at a key is the chord between
/// its neighbours over the time between them (one-sided at the ends), in
/// units of the segment's own time. With keys evenly spaced it is the
/// uniform spline; with uneven times a short segment is not overshot by
/// the long one beside it, as the uniform spline in the key's index is.
fn hermite_through(times: &[f64], points: &[Vec3], i: usize, u: f64) -> Vec3 {
    let last = points.len().saturating_sub(1);
    let at = |k: usize| points.get(k.min(last)).copied().unwrap_or(Vec3::ZERO);
    let time = |k: usize| times.get(k.min(last)).copied().unwrap_or(0.0);
    if last == 0 {
        return at(0);
    }
    let tangent = |k: usize| {
        let (a, b) = (k.saturating_sub(1), (k + 1).min(last));
        let span = time(b) - time(a);
        if span > 0.0 {
            (at(b) - at(a)) * (1.0 / span)
        } else {
            Vec3::ZERO
        }
    };
    let h = time(i + 1) - time(i);
    let (u2, u3) = (u * u, u * u * u);
    at(i) * (2.0 * u3 - 3.0 * u2 + 1.0)
        + tangent(i) * (h * (u3 - 2.0 * u2 + u))
        + at(i + 1) * (3.0 * u2 - 2.0 * u3)
        + tangent(i + 1) * (h * (u3 - u2))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn easings_run_from_zero_to_one() {
        for e in [Easing::Linear, Easing::In, Easing::Out, Easing::InOut] {
            assert_eq!(e.apply(0.0), 0.0);
            assert_eq!(e.apply(1.0), 1.0);
            assert_eq!(e.apply(-3.0), 0.0);
            assert_eq!(e.apply(f64::NAN), 0.0);
            assert!((0.0..=1.0).contains(&e.apply(0.3)));
        }
        assert_eq!(Easing::InOut.apply(0.5), 0.5);
        assert!(Easing::In.apply(0.5) < 0.5 && Easing::Out.apply(0.5) > 0.5);
    }

    #[test]
    fn a_track_from_to_and_with_keys() {
        let tracks = Track::parse_all(
            "x",
            &json!([
                {"variable": "w", "from": 10, "to": 30, "unit": "mm"},
                {"variable": "n", "keys": [{"t": 0, "value": 2}, {"t": 0.5, "value": 6}, {"t": 1, "value": 4}], "easing": "inOut"}
            ]),
        )
        .unwrap();
        assert_eq!(tracks[0].at(0.0), 10.0);
        assert_eq!(tracks[0].at(0.5), 20.0);
        assert_eq!(tracks[0].expr_at(1.0), "30 mm");
        assert_eq!(tracks[1].at(0.5), 6.0);
        assert_eq!(tracks[1].at(0.75), 5.0);
        assert_eq!(tracks[1].at(1.0), 4.0);
        assert_eq!(tracks[1].expr_at(0.0), "2");
        // One key holds.
        let one = Track::parse_all(
            "x",
            &json!([{"variable": "w", "keys": [{"t": 0.3, "value": 7}]}]),
        )
        .unwrap();
        assert_eq!(one[0].at(0.0), 7.0);
        assert_eq!(one[0].at(1.0), 7.0);
    }

    #[test]
    fn bad_tracks_are_refused() {
        for bad in [
            json!({}),
            json!("w"),
            json!([1]),
            json!([{"from": 1, "to": 2}]),
            json!([{"variable": "w"}]),
            json!([{"variable": "w", "from": 1}]),
            json!([{"variable": "w", "from": 1, "to": 2, "keys": []}]),
            json!([{"variable": "w", "keys": []}]),
            json!([{"variable": "w", "keys": [{"t": 0.5, "value": 1}, {"t": 0.5, "value": 2}]}]),
            json!([{"variable": "w", "keys": [{"t": 2, "value": 1}]}]),
            json!([{"variable": "w", "keys": [{"t": 0}]}]),
            json!([{"variable": "w", "from": 1e300, "to": 2}]),
            json!([{"variable": "w", "from": "a", "to": 2}]),
            json!([{"variable": "w", "from": 1, "to": 2, "unit": "mm; rm"}]),
            json!([{"variable": "w", "from": 1, "to": 2, "easing": "bounce"}]),
            json!([{"variable": "w", "from": 1, "to": 2, "speed": 3}]),
            json!([{"variable": "w", "from": 1, "to": 2}, {"variable": "w", "from": 1, "to": 2}]),
        ] {
            assert!(Track::parse_all("x", &bad).is_err(), "{bad}");
        }
    }

    fn base() -> Camera {
        Camera {
            eye: Vec3::new(10.0, 0.0, 0.0),
            target: Vec3::new(0.0, 0.0, 0.0),
            up: Vec3::Z,
            projection: Projection::Perspective { fov_y_deg: 30.0 },
        }
    }

    #[test]
    fn a_camera_path_through_its_keys_with_orbit_and_dolly() {
        let still = CameraPath::parse("x", None, None).unwrap();
        assert!(still.is_still());
        assert_eq!(still.at(&base(), 0.7).unwrap(), base());
        // The keys are passed through exactly.
        let path = CameraPath::parse(
            "x",
            Some(&json!({"keys": [
                {"t": 0, "eye": [10, 0, 0]},
                {"t": 0.5, "eye": [0, 10, 0], "fov": 50},
                {"t": 1, "eye": [-10, 0, 0]}
            ]})),
            None,
        )
        .unwrap();
        assert!(!path.is_still());
        let mid = path.at(&base(), 0.5).unwrap();
        assert!((mid.eye - Vec3::new(0.0, 10.0, 0.0)).length() < 1e-12);
        assert_eq!(mid.projection, Projection::Perspective { fov_y_deg: 50.0 });
        assert!((path.at(&base(), 1.0).unwrap().eye - Vec3::new(-10.0, 0.0, 0.0)).length() < 1e-12);
        // An orbit of 90 degrees ends a quarter turn round; a dolly of 0.5
        // halves the distance.
        let turn = CameraPath::parse("x", Some(&json!({"orbit": 90, "dolly": 0.5})), None).unwrap();
        let end = turn.at(&base(), 1.0).unwrap();
        assert!(
            (end.eye - Vec3::new(0.0, 5.0, 0.0)).length() < 1e-12,
            "{:?}",
            end.eye
        );
        // The top-level orbit is the same thing.
        let top = CameraPath::parse("x", None, Some(90.0)).unwrap();
        assert!((top.at(&base(), 1.0).unwrap().eye - Vec3::new(0.0, 10.0, 0.0)).length() < 1e-12);
        // A key that puts the eye on the target has no direction.
        let bad = CameraPath::parse(
            "x",
            Some(&json!({"keys": [{"t": 0, "eye": [0, 0, 0]}]})),
            None,
        )
        .unwrap();
        assert!(bad.at(&base(), 0.0).is_none());
    }

    #[test]
    fn uneven_keys_are_not_overshot() {
        // A short first segment and a long second one, all along x: the
        // camera moves on without turning back.
        let path = CameraPath::parse(
            "x",
            Some(&json!({"keys": [
                {"t": 0, "eye": [10, 0, 0]},
                {"t": 0.1, "eye": [11, 0, 0]},
                {"t": 1, "eye": [20, 0, 0]}
            ]})),
            None,
        )
        .unwrap();
        let mut last = f64::NEG_INFINITY;
        for k in 0..=200 {
            let x = path.at(&base(), f64::from(k) / 200.0).unwrap().eye.x;
            assert!(x >= last - 1e-12, "turned back at {k}: {x} after {last}");
            last = x;
        }
        assert!((path.at(&base(), 0.1).unwrap().eye.x - 11.0).abs() < 1e-12);
        assert!((last - 20.0).abs() < 1e-12);
    }

    #[test]
    fn bad_cameras_are_refused() {
        for bad in [
            json!(3),
            json!({"spin": 1}),
            json!({"orbit": 1e9}),
            json!({"dolly": 0}),
            json!({"dolly": "near"}),
            json!({"easing": 1}),
            json!({"keys": []}),
            json!({"keys": {}}),
            json!({"keys": [1]}),
            json!({"keys": [{"eye": [1, 2, 3]}]}),
            json!({"keys": [{"t": 0, "eye": [1, 2]}]}),
            json!({"keys": [{"t": 0, "eye": [1, 2, "z"]}]}),
            json!({"keys": [{"t": 0, "up": [0, 0, 0]}]}),
            json!({"keys": [{"t": 0, "fov": 0}]}),
            json!({"keys": [{"t": 0, "roll": 3}]}),
            json!({"keys": [{"t": 1}, {"t": 0}]}),
        ] {
            assert!(CameraPath::parse("x", Some(&bad), None).is_err(), "{bad}");
        }
        assert!(CameraPath::parse("x", Some(&json!({"orbit": 10})), Some(5.0)).is_err());
        assert!(CameraPath::parse("x", None, Some(f64::NAN)).is_err());
    }
}
