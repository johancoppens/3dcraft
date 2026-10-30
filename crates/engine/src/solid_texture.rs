//! Layer 3.5: the `pattern` parameter of `body.appearance`,
//! `face.appearance` and `material.preset` (a solid texture, evaluated in
//! the model's space; docs/design/rendering.md, *Layer 3.5: solid
//! textures*), the defaults it takes from the body's box, how it is said
//! back, and how the renderers get it.

use crate::appearance_cmds::color_param;
use crate::params::Params;
use crate::{CommandError, Session};
use craft3d_doc::FeatureId;
use craft3d_doc::{SolidTexture, SolidTextureKind};
use craft3d_math::{Bbox3, Vec3};
use craft3d_render::studio::pattern::{Pattern as RenderPattern, PatternKind as RenderKind};
use craft3d_topo::Body;
use serde_json::{Map, Value, json};

/// The keys of a `pattern` object.
const KEYS: [&str; 14] = [
    "kind",
    "colors",
    "roughness",
    "scale",
    "axis",
    "origin",
    "seed",
    "distortion",
    "detail",
    "grain",
    "rays",
    "spot",
    "wrap",
    "alongAxis",
];

/// The colours and numbers a kind takes when `pattern` does not give them.
fn kind_defaults(kind: SolidTextureKind) -> SolidTexture {
    let base = |colors: &[[u8; 3]]| {
        let mut out = SolidTexture::new(kind, [[0; 3]; 2], 1.0);
        for (slot, c) in out.colors.iter_mut().zip(colors) {
            *slot = *c;
        }
        let last = colors.last().copied().unwrap_or([0; 3]);
        for slot in out.colors.iter_mut().skip(colors.len()) {
            *slot = last;
        }
        // 2 to 4 colours below.
        out.stops = colors.len() as u8;
        out
    };
    match kind {
        SolidTextureKind::Wood => SolidTexture {
            distortion: 0.6,
            grain: 0.5,
            along_axis: true,
            ..base(&[[214, 172, 120], [168, 118, 70], [118, 78, 42]])
        },
        SolidTextureKind::Marble => SolidTexture {
            distortion: 0.8,
            ..base(&[[238, 236, 230], [196, 194, 192], [70, 72, 78]])
        },
        SolidTextureKind::Granite => base(&[
            [200, 196, 192],
            [140, 136, 134],
            [60, 58, 60],
            [170, 120, 110],
        ]),
        SolidTextureKind::Speckle => base(&[[230, 230, 226], [40, 40, 44]]),
        SolidTextureKind::Noise => base(&[[90, 90, 92], [140, 140, 142]]),
        SolidTextureKind::Brushed => SolidTexture {
            grain: 0.5,
            along_axis: true,
            ..base(&[[180, 182, 186], [214, 216, 220]])
        },
        SolidTextureKind::Carbon => base(&[[6, 6, 7], [18, 18, 20], [44, 44, 48]]),
    }
}

fn three(id: &str, key: &str, value: &Value) -> Result<[f64; 3], CommandError> {
    value
        .as_array()
        .filter(|a| a.len() == 3)
        .and_then(|a| {
            let v: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
            <[f64; 3]>::try_from(v).ok()
        })
        .ok_or_else(|| {
            CommandError::new(format!(
                "{id}: `pattern.{key}` must be three numbers [x, y, z], got {value}"
            ))
        })
}

fn number(id: &str, map: &Map<String, Value>, key: &str) -> Result<Option<f64>, CommandError> {
    match map.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|x| x.is_finite())
            .map(Some)
            .ok_or_else(|| {
                CommandError::new(format!("{id}: `pattern.{key}` must be a number, got {v}"))
            }),
    }
}

/// A `pattern` given to `id`, over `base` (the preset's, or none): `None`
/// for none (`null`), else the texture and whether its scale was given.
/// Its axis and origin stay `None` where not given ([`resolve`] fills
/// them from the body).
pub(crate) fn pattern_param(
    id: &str,
    value: &Value,
    base: Option<SolidTexture>,
) -> Result<Option<(SolidTexture, bool)>, CommandError> {
    if value.is_null() {
        return Ok(None);
    }
    let Some(map) = value.as_object() else {
        return Err(CommandError::new(format!(
            "{id}: `pattern` must be an object {{\"kind\": \"wood\", …}} or null, got {value}"
        )));
    };
    if let Some(key) = map.keys().find(|k| !KEYS.contains(&k.as_str())) {
        let shown: String = key.chars().take(40).collect();
        return Err(CommandError::new(format!(
            "{id}: `pattern` has no `{shown}`; it takes {}",
            KEYS.join(", ")
        )));
    }
    let kind = match map.get("kind") {
        Some(Value::String(name)) => Some(SolidTextureKind::from_name(name).ok_or_else(|| {
            let names: Vec<&str> = SolidTextureKind::ALL.iter().map(|k| k.name()).collect();
            let shown: String = name.chars().take(40).collect();
            CommandError::new(format!(
                "{id}: there is no pattern kind `{shown}`; the kinds are {}",
                names.join(", ")
            ))
        })?),
        Some(other) => {
            return Err(CommandError::new(format!(
                "{id}: `pattern.kind` must be a name, got {other}"
            )));
        }
        None => None,
    };
    let (mut t, mut scale_given) = match (base, kind) {
        (Some(base), None) => (base, true),
        (Some(base), Some(kind)) if base.kind == kind => (base, true),
        (_, Some(kind)) => (kind_defaults(kind), false),
        (None, None) => {
            return Err(CommandError::new(format!(
                "{id}: `pattern` needs a `kind`: wood, marble, granite, speckle, noise, brushed or carbon"
            )));
        }
    };
    if let Some(value) = map.get("colors") {
        let items = value
            .as_array()
            .filter(|a| (2..=craft3d_doc::MAX_PATTERN_COLORS).contains(&a.len()))
            .ok_or_else(|| {
                CommandError::new(format!(
                    "{id}: `pattern.colors` must be a list of 2 to {} colours, got {value}",
                    craft3d_doc::MAX_PATTERN_COLORS
                ))
            })?;
        let mut colors = Vec::with_capacity(items.len());
        for item in items {
            colors.push(color_param(id, item)?);
        }
        let last = colors.last().copied().unwrap_or([0; 3]);
        for (k, slot) in t.colors.iter_mut().enumerate() {
            *slot = colors.get(k).copied().unwrap_or(last);
        }
        // 2 to 4, checked above.
        t.stops = colors.len() as u8;
    }
    if let Some(value) = map.get("roughness") {
        t.roughness = if value.is_null() {
            None
        } else {
            let pair = value
                .as_array()
                .filter(|a| a.len() == 2)
                .and_then(|a| {
                    let v: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
                    <[f64; 2]>::try_from(v).ok()
                })
                .ok_or_else(|| {
                    CommandError::new(format!(
                        "{id}: `pattern.roughness` must be two numbers [at the first colour, at the last] or null, got {value}"
                    ))
                })?;
            Some(pair)
        };
    }
    if let Some(scale) = number(id, map, "scale")? {
        t.scale = scale;
        scale_given = true;
    }
    if let Some(value) = map.get("axis") {
        t.axis = if value.is_null() {
            None
        } else {
            Some(three(id, "axis", value)?)
        };
    }
    if let Some(value) = map.get("origin") {
        t.origin = if value.is_null() {
            None
        } else {
            Some(three(id, "origin", value)?)
        };
    }
    if let Some(value) = map.get("seed") {
        t.seed = value
            .as_u64()
            .and_then(|s| u32::try_from(s).ok())
            .ok_or_else(|| {
                CommandError::new(format!(
                    "{id}: `pattern.seed` must be a whole number from 0 to {}, got {value}",
                    u32::MAX
                ))
            })?;
    }
    if let Some(value) = map.get("detail") {
        t.detail = value
            .as_u64()
            .and_then(|s| u32::try_from(s).ok())
            .ok_or_else(|| {
                CommandError::new(format!(
                    "{id}: `pattern.detail` must be a whole number of octaves (1 to 6), got {value}"
                ))
            })?;
    }
    for (key, slot) in [
        ("distortion", &mut t.distortion),
        ("grain", &mut t.grain),
        ("rays", &mut t.rays),
        ("spot", &mut t.spot),
    ] {
        if let Some(v) = number(id, map, key)? {
            *slot = v;
        }
    }
    for (key, slot) in [("wrap", &mut t.wrap), ("alongAxis", &mut t.along_axis)] {
        if let Some(v) = map.get(key) {
            *slot = v.as_bool().ok_or_else(|| {
                CommandError::new(format!(
                    "{id}: `pattern.{key}` must be true or false, got {v}"
                ))
            })?;
        }
    }
    // The scale is checked once it is known (`resolve`).
    let mut checked = t;
    if !scale_given {
        checked.scale = 1.0;
    }
    checked
        .check()
        .map_err(|why| CommandError::new(format!("{id}: {why}")))?;
    Ok(Some((t, scale_given)))
}

/// The box's extents by axis, longest first (ties in the order x, y, z).
fn axes_by_extent(bbox: &Bbox3) -> [(Vec3, f64); 3] {
    let (lo, hi) = bbox.corners().unwrap_or((Vec3::ZERO, Vec3::ZERO));
    let size = hi - lo;
    let mut axes = [(Vec3::X, size.x), (Vec3::Y, size.y), (Vec3::Z, size.z)];
    axes.sort_by(|a, b| b.1.total_cmp(&a.1));
    axes
}

/// `t` with its scale (when not `given`), axis and origin filled from the
/// body's box (docs/design/rendering.md, *Layer 3.5*, *Defaults*): the
/// axis along the box's longest side; for wood tilted towards its
/// thinnest side by 0.06 (a board is never sawn exactly along the pith,
/// which is what makes its rings show as arches), with the pith 0.6 of
/// the middle side away from the centre towards the thinnest side, out of
/// the body; every other kind about the box's centre. Why not, when the
/// body has no box and something is missing.
pub(crate) fn resolve(
    id: &str,
    t: SolidTexture,
    given: bool,
    body: Option<&Body>,
) -> Result<SolidTexture, CommandError> {
    let mut t = t;
    let needs = !given || t.axis.is_none() || t.origin.is_none();
    if !needs {
        return Ok(t);
    }
    let bbox = body
        .and_then(|b| b.held_bbox().ok())
        .filter(|b| {
            b.corners()
                .is_some_and(|(lo, hi)| lo.is_finite() && hi.is_finite() && (hi - lo).length() > 0.0)
        })
        .ok_or_else(|| {
            CommandError::new(format!(
                "{id}: the body has no geometry yet to fit the pattern to; give `pattern.scale`, `pattern.axis` and `pattern.origin`"
            ))
        })?;
    let (scale, axis, origin) = fitted(t.kind, &bbox);
    if !given {
        t.scale = scale;
    }
    if t.axis.is_none() {
        t.axis = Some(axis);
    }
    if t.origin.is_none() {
        t.origin = Some(origin);
    }
    t.check()
        .map_err(|why| CommandError::new(format!("{id}: {why}")))?;
    Ok(t)
}

/// The scale, axis and origin a pattern of `kind` takes from a box (see
/// [`resolve`]).
pub(crate) fn fitted(kind: SolidTextureKind, bbox: &Bbox3) -> (f64, [f64; 3], [f64; 3]) {
    let [(long, l), (_, m), (thin, _)] = axes_by_extent(bbox);
    let centre = bbox.center().unwrap_or(Vec3::ZERO);
    let floor = l.max(f64::MIN_POSITIVE);
    let mid_len = if m > 0.0 { m } else { floor };
    let arr = |v: Vec3| [v.x, v.y, v.z];
    match kind {
        SolidTextureKind::Wood => {
            let axis = long + thin * 0.06;
            let origin = centre - thin * (0.6 * mid_len);
            (mid_len / 20.0, arr(axis), arr(origin))
        }
        SolidTextureKind::Marble => (floor / 4.0, arr(long), arr(centre)),
        SolidTextureKind::Granite => (floor / 80.0, arr(long), arr(centre)),
        SolidTextureKind::Speckle => (floor / 50.0, arr(long), arr(centre)),
        SolidTextureKind::Noise => (floor / 6.0, arr(long), arr(centre)),
        SolidTextureKind::Brushed => (floor / 100.0, arr(long), arr(centre)),
        SolidTextureKind::Carbon => (mid_len / 12.0, arr(long), arr(centre)),
    }
}

/// The texture as the renderers take it; an axis or origin the document
/// leaves to the body (a file written by hand) from `body`'s box now.
pub(crate) fn render_pattern(t: &SolidTexture, body: Option<&Body>) -> RenderPattern {
    let (axis, origin) = match (t.axis, t.origin) {
        (Some(axis), Some(origin)) => (axis, origin),
        (axis, origin) => {
            let (_, a, o) = body
                .and_then(|b| b.held_bbox().ok())
                .filter(|b| {
                    b.corners()
                        .is_some_and(|(lo, hi)| lo.is_finite() && hi.is_finite())
                })
                .map_or((1.0, [0.0, 0.0, 1.0], [0.0; 3]), |b| fitted(t.kind, &b));
            (axis.unwrap_or(a), origin.unwrap_or(o))
        }
    };
    // Deliberate narrowing: the renderers read f32 numbers.
    let n = |v: f64| v as f32;
    RenderPattern {
        kind: match t.kind {
            SolidTextureKind::Wood => RenderKind::Wood,
            SolidTextureKind::Marble => RenderKind::Marble,
            SolidTextureKind::Granite => RenderKind::Granite,
            SolidTextureKind::Speckle => RenderKind::Speckle,
            SolidTextureKind::Noise => RenderKind::Noise,
            SolidTextureKind::Brushed => RenderKind::Brushed,
            SolidTextureKind::Carbon => RenderKind::Carbon,
        },
        ramp: t.colors,
        stops: t.stops,
        roughness: t.roughness.map(|r| r.map(n)),
        scale: n(t.scale),
        origin,
        axis: axis.map(n),
        seed: t.seed,
        distortion: n(t.distortion),
        detail: t.detail,
        grain: n(t.grain),
        rays: n(t.rays),
        spot: n(t.spot),
        wrap: t.wrap,
    }
    .sanitized()
}

/// The texture's mean colour (sRGB): what a format that cannot hold it
/// exports, and the colour the commands give the appearance.
pub(crate) fn mean_color(t: &SolidTexture) -> [u8; 3] {
    render_pattern(t, None).mean_color()
}

/// The texture as it is said back (its mean colour is not: it follows
/// the formulas, and the appearance's colour holds it).
pub(crate) fn pattern_json(t: &SolidTexture) -> Value {
    let hex = |[r, g, b]: [u8; 3]| format!("#{r:02x}{g:02x}{b:02x}");
    let colors: Vec<String> = t.ramp().iter().map(|c| hex(*c)).collect();
    let mut out = json!({
        "kind": t.kind.name(),
        "colors": colors,
        "scale": t.scale,
        "seed": t.seed,
        "distortion": t.distortion,
        "detail": t.detail,
        "grain": t.grain,
        "rays": t.rays,
        "spot": t.spot,
        "wrap": t.wrap,
        "alongAxis": t.along_axis,
    });
    if let Some(map) = out.as_object_mut() {
        if let Some(r) = t.roughness {
            map.insert("roughness".into(), json!(r));
        }
        if let Some(a) = t.axis {
            map.insert("axis".into(), json!(a));
        }
        if let Some(o) = t.origin {
            map.insert("origin".into(), json!(o));
        }
    }
    out
}

/// The `pattern` of `p`, for the body of `feature`, over `base`: `None`
/// when `p` has none, `Some(None)` for `null` (none), else the texture
/// with its defaults filled from the body's box ([`resolve`]).
pub(crate) fn given_pattern(
    session: &mut Session,
    p: &Params,
    feature: FeatureId,
    base: Option<SolidTexture>,
) -> Result<Option<Option<SolidTexture>>, CommandError> {
    let id = p.command();
    let Some(value) = p.get("pattern") else {
        return Ok(None);
    };
    let Some((t, given)) = pattern_param(id, value, base)? else {
        return Ok(Some(None));
    };
    let open = session.document_mut()?;
    let regen = open.regen();
    let body = regen.feature(feature).and_then(|r| r.body.as_deref());
    resolve(id, t, given, body).map(|t| Some(Some(t)))
}
