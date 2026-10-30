//! Surface analysis for the view (docs/design/surfacing.md, stage 1):
//! the `analysis` object of `view.render` and `ui.set`, the curvature map's
//! values per vertex from the exact surfaces, and curvature combs.
//!
//! View state only: nothing here is saved in a document. The pictures are
//! drawn by `craft3d_render::analysis` (the CPU oracle) and its GPU twin.

use crate::scene::Scene;
use crate::{CommandError, Session};
use craft3d_geom::Surface;
use craft3d_geom::curvature::{curvatures, curve_curvature, second_derivatives};
use craft3d_math::Vec3;
use craft3d_render::analysis::{Analysis, AnalysisColors, AnalysisMode, MAX_STRIPES};
use craft3d_topo::Body;
use serde_json::{Map, Value, json};
use std::sync::Arc;

/// The keys of the `analysis` object.
pub const ANALYSIS_KEYS: [&str; 11] = [
    "mode",
    "stripes",
    "direction",
    "width",
    "offset",
    "pull",
    "minDraft",
    "curvature",
    "range",
    "combs",
    "legend",
];

/// What `analysis` takes, for the params docs.
pub const ANALYSIS_PARAMS: &str = r#"{"mode": "zebra|highlight|curvature|draft|none" = "none", "stripes": number from 1 to 512 = 16, "direction": degrees = 0, "width": dark share 0 to 1 = 0.5, "offset": periods = 0, "pull": [x, y, z] = [0, 0, 1], "minDraft": degrees = 1, "curvature": "gaussian|mean|max" = "gaussian", "range": [low, high] or null = from the values, "combs": {"edges": boolean or [edge name, ...] = false, "iso": [{"face": face name, "feature": id or name, "u": 0 to 1} or {.., "v": 0 to 1}, ...] = [], "scale": number or null = 10 % of the model's size for the largest, "density": spikes per curve from 2 to 400 = 40} or null = null}"#;

/// Most spikes of one comb.
pub const MAX_DENSITY: u32 = 400;
/// Most curves combed.
pub const MAX_COMB_CURVES: usize = 4096;
/// Most mesh vertices a curvature map evaluates (each a projection onto
/// its face's exact surface).
pub const MAX_MAP_VERTICES: usize = 4_000_000;

/// Which curvature the map shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CurvatureKind {
    /// `K = κ₁ κ₂`, length⁻².
    #[default]
    Gaussian,
    /// `H = (κ₁ + κ₂) / 2`, length⁻¹, positive where the body is convex
    /// (outward normal).
    Mean,
    /// `max(|κ₁|, |κ₂|) = |H| + √(H² − K)`, length⁻¹.
    Max,
}

impl CurvatureKind {
    pub fn name(self) -> &'static str {
        match self {
            CurvatureKind::Gaussian => "gaussian",
            CurvatureKind::Mean => "mean",
            CurvatureKind::Max => "max",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "gaussian" => Some(CurvatureKind::Gaussian),
            "mean" => Some(CurvatureKind::Mean),
            "max" => Some(CurvatureKind::Max),
            _ => None,
        }
    }

    /// Its value from `(K, H)` with `H` positive where convex.
    pub fn of(self, k: f64, h: f64) -> f64 {
        match self {
            CurvatureKind::Gaussian => k,
            CurvatureKind::Mean => h,
            CurvatureKind::Max => h.abs() + (h * h - k).max(0.0).sqrt(),
        }
    }
}

/// An iso-curve of a face to comb: at a fraction of the face's parameter
/// box in `u` (a curve along `v`) or in `v`.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoSpec {
    pub face: String,
    pub feature: Option<String>,
    /// `true` for a fixed `u`.
    pub fixed_u: bool,
    pub at: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CombSpec {
    /// `None`: no edges; `Some(empty)`: every edge; else the edges named.
    pub edges: Option<Vec<String>>,
    pub iso: Vec<IsoSpec>,
    /// Model units per unit of curvature; `None` for automatic.
    pub scale: Option<f64>,
    pub density: u32,
}

/// The `analysis` object, parsed.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisSpec {
    /// `None` for no analysis.
    pub mode: Option<AnalysisMode>,
    pub stripes: f64,
    /// Degrees.
    pub direction: f64,
    pub width: f64,
    pub offset: f64,
    pub pull: Vec3,
    /// Degrees.
    pub min_draft: f64,
    pub curvature: CurvatureKind,
    pub range: Option<[f64; 2]>,
    pub combs: Option<CombSpec>,
    /// Whether the window draws the legend.
    pub legend: bool,
}

impl Default for AnalysisSpec {
    fn default() -> Self {
        Self {
            mode: None,
            stripes: 16.0,
            direction: 0.0,
            width: 0.5,
            offset: 0.0,
            pull: Vec3::new(0.0, 0.0, 1.0),
            min_draft: 1.0,
            curvature: CurvatureKind::Gaussian,
            range: None,
            combs: None,
            legend: true,
        }
    }
}

fn number(map: &Map<String, Value>, key: &str) -> Result<Option<f64>, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(v) if v.is_finite() => Ok(Some(v)),
            _ => Err(format!("`{key}` must be a finite number")),
        },
        Some(_) => Err(format!("`{key}` must be a number")),
    }
}

fn vector(value: &Value, key: &str) -> Result<Vec3, String> {
    let Some([x, y, z]) = value.as_array().and_then(|a| {
        let v: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
        match (a.len(), v.as_slice()) {
            (3, [x, y, z]) => Some([*x, *y, *z]),
            _ => None,
        }
    }) else {
        return Err(format!("`{key}` must be [x, y, z]"));
    };
    let v = Vec3::new(x, y, z);
    if !v.is_finite() || v.length() == 0.0 {
        return Err(format!("`{key}` must be a direction: finite and not zero"));
    }
    Ok(v)
}

fn short(text: &str) -> String {
    text.chars().take(60).collect()
}

impl AnalysisSpec {
    /// `base` with the fields of `value` (an object; `null` or `{"mode":
    /// "none"}` turns the analysis off) changed.
    pub fn patched(base: &AnalysisSpec, value: &Value) -> Result<AnalysisSpec, String> {
        let map = match value {
            Value::Null => {
                return Ok(AnalysisSpec {
                    mode: None,
                    ..base.clone()
                });
            }
            Value::Object(map) => map,
            _ => return Err("`analysis` must be an object or null".into()),
        };
        if let Some(key) = map.keys().find(|k| !ANALYSIS_KEYS.contains(&k.as_str())) {
            return Err(format!(
                "analysis: unknown key `{}`; one of {}",
                short(key),
                ANALYSIS_KEYS.join(", ")
            ));
        }
        let mut out = base.clone();
        match map.get("mode") {
            None => {}
            Some(Value::String(m)) => {
                out.mode = match m.as_str() {
                    "none" => None,
                    "zebra" => Some(AnalysisMode::Zebra),
                    "highlight" => Some(AnalysisMode::Highlight),
                    "curvature" => Some(AnalysisMode::Curvature),
                    "draft" => Some(AnalysisMode::Draft),
                    other => {
                        return Err(format!(
                            "analysis: unknown mode `{}`; use zebra, highlight, curvature, draft or none",
                            short(other)
                        ));
                    }
                }
            }
            Some(_) => return Err("analysis: `mode` must be a string".into()),
        }
        if let Some(v) = number(map, "stripes")? {
            if !(1.0..=MAX_STRIPES).contains(&v) {
                return Err(format!(
                    "analysis: `stripes` must be from 1 to {MAX_STRIPES}"
                ));
            }
            out.stripes = v;
        }
        if let Some(v) = number(map, "direction")? {
            out.direction = v.rem_euclid(360.0);
        }
        if let Some(v) = number(map, "width")? {
            if !(0.0..=1.0).contains(&v) {
                return Err("analysis: `width` must be from 0 to 1".into());
            }
            out.width = v;
        }
        if let Some(v) = number(map, "offset")? {
            out.offset = v.rem_euclid(1.0);
        }
        if let Some(v) = map.get("pull").filter(|v| !v.is_null()) {
            out.pull = vector(v, "pull").map_err(|e| format!("analysis: {e}"))?;
        }
        if let Some(v) = number(map, "minDraft")? {
            if !(0.0..=90.0).contains(&v) {
                return Err("analysis: `minDraft` must be from 0 to 90 degrees".into());
            }
            out.min_draft = v;
        }
        match map.get("curvature") {
            None => {}
            Some(Value::String(k)) => {
                out.curvature = CurvatureKind::parse(k).ok_or_else(|| {
                    format!(
                        "analysis: unknown curvature `{}`; use gaussian, mean or max",
                        short(k)
                    )
                })?;
            }
            Some(_) => return Err("analysis: `curvature` must be a string".into()),
        }
        match map.get("range") {
            None => {}
            Some(Value::Null) => out.range = None,
            Some(Value::Array(a)) => {
                let v: Vec<f64> = a
                    .iter()
                    .filter_map(Value::as_f64)
                    .filter(|v| v.is_finite())
                    .collect();
                match v.as_slice() {
                    // Within single precision, where the GPU places the
                    // values: the two renderers then agree on the ramp.
                    [lo, hi]
                        if a.len() == 2
                            && lo < hi
                            && lo.abs() <= 1e30
                            && hi.abs() <= 1e30
                            && (*lo as f32) < (*hi as f32) =>
                    {
                        out.range = Some([*lo, *hi])
                    }
                    _ => {
                        return Err(
                            "analysis: `range` must be [low, high] with low below high".into()
                        );
                    }
                }
            }
            Some(_) => return Err("analysis: `range` must be [low, high] or null".into()),
        }
        match map.get("legend") {
            None => {}
            Some(Value::Bool(b)) => out.legend = *b,
            Some(_) => return Err("analysis: `legend` must be true or false".into()),
        }
        match map.get("combs") {
            None => {}
            Some(Value::Null) => out.combs = None,
            Some(value) => {
                out.combs = Some(comb_spec(out.combs.as_ref(), value)?);
            }
        }
        Ok(out)
    }

    /// The renderer's analysis with `range` for the map and `colors`;
    /// `None` when off.
    pub fn analysis(&self, range: [f64; 2], colors: AnalysisColors) -> Option<Analysis> {
        let mode = self.mode?;
        Some(Analysis {
            mode,
            stripes: self.stripes,
            direction: self.direction.to_radians(),
            width: self.width,
            offset: self.offset,
            pull: self.pull,
            min_draft: self.min_draft.to_radians(),
            range,
            grazing: craft3d_render::analysis::GRAZING,
            colors,
        })
    }

    /// As `ui.state` and `view.render` report it.
    pub fn json(&self) -> Value {
        json!({
            "mode": self.mode.map_or("none", AnalysisMode::name),
            "stripes": self.stripes,
            "direction": self.direction,
            "width": self.width,
            "offset": self.offset,
            "pull": [self.pull.x, self.pull.y, self.pull.z],
            "minDraft": self.min_draft,
            "curvature": self.curvature.name(),
            "range": self.range,
            "legend": self.legend,
            "combs": self.combs.as_ref().map(|c| json!({
                "edges": match &c.edges {
                    None => json!(false),
                    Some(list) if list.is_empty() => json!(true),
                    Some(list) => json!(list),
                },
                "iso": c.iso.iter().map(|i| {
                    let mut m = Map::new();
                    m.insert("face".into(), json!(i.face));
                    if let Some(f) = &i.feature {
                        m.insert("feature".into(), json!(f));
                    }
                    m.insert((if i.fixed_u { "u" } else { "v" }).into(), json!(i.at));
                    Value::Object(m)
                }).collect::<Vec<_>>(),
                "scale": c.scale,
                "density": c.density,
            })),
        })
    }

    /// Whether anything is drawn: a mode or combs.
    pub fn is_on(&self) -> bool {
        self.mode.is_some() || self.combs.is_some()
    }
}

fn comb_spec(base: Option<&CombSpec>, value: &Value) -> Result<CombSpec, String> {
    let Value::Object(map) = value else {
        return Err("analysis: `combs` must be an object or null".into());
    };
    const KEYS: [&str; 4] = ["edges", "iso", "scale", "density"];
    if let Some(key) = map.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!(
            "analysis: combs: unknown key `{}`; one of {}",
            short(key),
            KEYS.join(", ")
        ));
    }
    let mut out = base.cloned().unwrap_or(CombSpec {
        edges: None,
        iso: Vec::new(),
        scale: None,
        density: 40,
    });
    match map.get("edges") {
        None => {}
        Some(Value::Bool(true)) => out.edges = Some(Vec::new()),
        Some(Value::Bool(false)) | Some(Value::Null) => out.edges = None,
        Some(Value::Array(list)) => {
            let names: Vec<String> = list
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            if names.len() != list.len() || names.is_empty() {
                return Err("analysis: combs: `edges` must be true, false or edge names".into());
            }
            if names.len() > MAX_COMB_CURVES {
                return Err(format!("analysis: combs: at most {MAX_COMB_CURVES} edges"));
            }
            out.edges = Some(names);
        }
        Some(_) => {
            return Err("analysis: combs: `edges` must be true, false or edge names".into());
        }
    }
    match map.get("iso") {
        None => {}
        Some(Value::Array(list)) => {
            if list.len() > MAX_COMB_CURVES {
                return Err(format!(
                    "analysis: combs: at most {MAX_COMB_CURVES} iso-curves"
                ));
            }
            let mut iso = Vec::new();
            for item in list {
                let Value::Object(m) = item else {
                    return Err("analysis: combs: an iso-curve is {face, u} or {face, v}".into());
                };
                if let Some(key) = m
                    .keys()
                    .find(|k| !["face", "feature", "u", "v"].contains(&k.as_str()))
                {
                    return Err(format!(
                        "analysis: combs: iso: unknown key `{}`",
                        short(key)
                    ));
                }
                let face = m
                    .get("face")
                    .and_then(Value::as_str)
                    .ok_or("analysis: combs: an iso-curve needs `face`, a face name")?
                    .to_string();
                let feature = match m.get("feature") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(f)) => Some(f.clone()),
                    Some(_) => {
                        return Err("analysis: combs: iso: `feature` must be a string".into());
                    }
                };
                let (fixed_u, at) = match (number(m, "u")?, number(m, "v")?) {
                    (Some(u), None) => (true, u),
                    (None, Some(v)) => (false, v),
                    _ => {
                        return Err(
                            "analysis: combs: an iso-curve gives one of `u` or `v`, from 0 to 1"
                                .into(),
                        );
                    }
                };
                if !(0.0..=1.0).contains(&at) {
                    return Err("analysis: combs: iso: `u` or `v` must be from 0 to 1".into());
                }
                iso.push(IsoSpec {
                    face,
                    feature,
                    fixed_u,
                    at,
                });
            }
            out.iso = iso;
        }
        Some(_) => return Err("analysis: combs: `iso` must be a list".into()),
    }
    match map.get("scale") {
        None => {}
        Some(Value::Null) => out.scale = None,
        Some(_) => {
            let v = number(map, "scale")?.unwrap_or(0.0);
            if v <= 0.0 {
                return Err("analysis: combs: `scale` must be positive or null".into());
            }
            out.scale = Some(v);
        }
    }
    if let Some(v) = number(map, "density")? {
        if !(2.0..=f64::from(MAX_DENSITY)).contains(&v) || v.fract() != 0.0 {
            return Err(format!(
                "analysis: combs: `density` must be a whole number from 2 to {MAX_DENSITY}"
            ));
        }
        out.density = v as u32;
    }
    Ok(out)
}

/// Parses `view.render`'s `analysis` for `cmd`.
pub(crate) fn analysis_param(
    cmd: &str,
    value: Option<&Value>,
) -> Result<AnalysisSpec, CommandError> {
    match value {
        None | Some(Value::Null) => Ok(AnalysisSpec::default()),
        Some(v) => AnalysisSpec::patched(&AnalysisSpec::default(), v)
            .map_err(|e| CommandError::new(format!("{cmd}: {e}"))),
    }
}

/// The symbol of the active document's length unit (`mm`), for a
/// legend; empty without a document.
pub fn length_unit(session: &Session) -> String {
    session
        .document()
        .map(|d| d.history.current().units.length.symbol().to_string())
        .unwrap_or_default()
}

/// The bodies of the scene's regeneration, by the scene's body order:
/// `None` for one that is not there.
fn scene_bodies(
    session: &mut Session,
    scene: &Scene,
) -> Result<Vec<Option<Arc<Body>>>, CommandError> {
    let open = session.document_mut()?;
    let regen = open.shown_regen();
    let by_id: std::collections::BTreeMap<String, Arc<Body>> = regen
        .bodies()
        .map(|(id, body)| (id.to_string(), Arc::clone(body)))
        .collect();
    Ok(scene
        .bodies
        .iter()
        .map(|b| by_id.get(&b.feature).cloned())
        .collect())
}

/// The curvature of `kind` at every vertex of the scene's mesh, from the
/// exact surface of the vertex's face: the vertex is projected onto the
/// surface (from the last vertex's parameters, else from scratch), and the
/// curvatures there are those of `craft3d_geom::curvature` (exact second
/// derivatives), the mean taken positive where the body is convex. NaN
/// where the surface has no curvature there (a cone's apex), where the
/// projection lands farther than `tolerance` from the vertex (a mesh
/// drawn apart from its body, a fold laid flat), or for a body the
/// regeneration does not have.
pub fn vertex_values(
    session: &mut Session,
    scene: &Scene,
    kind: CurvatureKind,
) -> Result<Vec<f32>, CommandError> {
    let mesh = &scene.mesh;
    if mesh.positions.len() > MAX_MAP_VERTICES {
        return Err(CommandError::new(format!(
            "the curvature map takes at most {MAX_MAP_VERTICES} vertices; the scene has {}",
            mesh.positions.len()
        )));
    }
    let bodies = scene_bodies(session, scene)?;
    // The face of every vertex (a vertex belongs to one face's patch).
    let mut face_of = vec![u32::MAX; mesh.positions.len()];
    for (tri, &face) in mesh.triangles.iter().zip(&mesh.face_ids) {
        for &k in tri {
            if let Some(slot) = face_of.get_mut(k as usize) {
                *slot = face;
            }
        }
    }
    let reach = (scene.tolerance * 4.0).max(scene.bodies_box.diagonal() * 1e-9);
    let mut out = vec![f32::NAN; mesh.positions.len()];
    for (index, sb) in scene.bodies.iter().enumerate() {
        let Some(Some(body)) = bodies.get(index) else {
            continue;
        };
        let first = sb.first_face_id;
        let mut last: Option<(u32, (f64, f64))> = None;
        for (k, (&p, &face)) in mesh.positions.iter().zip(&face_of).enumerate() {
            let Some(local) = face.checked_sub(first) else {
                continue;
            };
            let Some(topo) = body.faces.get(local as usize) else {
                continue;
            };
            let surface = &topo.surface;
            let start = last.filter(|(f, _)| *f == face).map(|(_, uv)| uv);
            let uv = match start {
                Some(start) => surface
                    .project_from(p, start)
                    .ok()
                    .filter(|&(u, v)| (surface.eval(u, v) - p).length() <= reach)
                    .or_else(|| surface.project(p).ok()),
                None => surface.project(p).ok(),
            };
            let Some((u, v)) = uv.filter(|&(u, v)| (surface.eval(u, v) - p).length() <= reach)
            else {
                continue;
            };
            last = Some((face, (u, v)));
            let Some((kg, h)) = curvatures(surface, u, v) else {
                continue;
            };
            // `h` has the sign of S_u × S_v, positive where the surface
            // bends towards it; outward is that normal unless the face is
            // reversed, and convex bends away from the outward normal.
            let h = if topo.reversed { h } else { -h };
            if let Some(slot) = out.get_mut(k) {
                *slot = kind.of(kg, h) as f32;
            }
        }
    }
    Ok(out)
}

/// Most rounds of refinement of an analysis picture's mesh.
const REFINE_ROUNDS: usize = 8;

/// An edge inside a face whose ends' values differ by more than this
/// share of the map's range is split.
const REFINE_STEP: f64 = 0.02;

/// The scene's mesh refined for an analysis picture where the curvature
/// changes fast, with the curvature of `kind` at every vertex (as
/// [`vertex_values`]): the map interpolates a value per vertex, so where
/// the values of an edge's ends differ by more than [`REFINE_STEP`] of the
/// range, the edge is split at its midpoint, the new vertex put on the
/// face's exact surface with the surface's normal and its exact value;
/// a triangle with one, two or three split edges becomes two, three or
/// four, so the mesh stays conforming (both triangles of an edge split
/// it). Edges on a face's border (one triangle) are not split, so faces
/// do not crack apart; nor are edges shorter than the scene's tolerance.
/// At most [`REFINE_ROUNDS`] rounds and [`MAX_MAP_VERTICES`] vertices.
/// For the analysis views only: the scene's own mesh is not changed.
pub fn refined_map(
    session: &mut Session,
    scene: &Scene,
    kind: CurvatureKind,
) -> Result<(craft3d_tess::Mesh, Vec<f32>), CommandError> {
    use std::collections::BTreeMap;
    let mut mesh: craft3d_tess::Mesh = (*scene.mesh).clone();
    let mut values = vertex_values(session, scene, kind)?;
    let bodies = scene_bodies(session, scene)?;
    let [low, high] = auto_range(&values, kind);
    let step = REFINE_STEP * (high - low).abs();
    if !(step.is_finite() && step > 0.0) {
        return Ok((mesh, values));
    }
    let floor = scene.tolerance;
    let reach = (scene.tolerance * 4.0).max(scene.bodies_box.diagonal() * 1e-9);
    // The surface of a face id, and whether the face is reversed.
    let surface_of = |face: u32| -> Option<(&Surface, bool)> {
        scene
            .bodies
            .iter()
            .zip(&bodies)
            .filter(|(sb, _)| sb.first_face_id <= face)
            .max_by_key(|(sb, _)| sb.first_face_id)
            .and_then(|(sb, body)| {
                let body = body.as_ref()?;
                let topo = body
                    .faces
                    .get(face.checked_sub(sb.first_face_id)? as usize)?;
                Some((&topo.surface, topo.reversed))
            })
    };
    for _ in 0..REFINE_ROUNDS {
        // Each edge with its triangles' face and how many use it.
        let mut uses: BTreeMap<(u32, u32), (u32, u32)> = BTreeMap::new();
        for (tri, &face) in mesh.triangles.iter().zip(&mesh.face_ids) {
            for k in 0..3 {
                let corner = |i: usize| tri.get(i % 3).copied().unwrap_or(0);
                let (a, b) = (corner(k), corner(k + 1));
                let entry = uses.entry((a.min(b), a.max(b))).or_insert((0, face));
                entry.0 += 1;
            }
        }
        let value = |i: u32| values.get(i as usize).map_or(f32::NAN, |v| *v);
        let position = |i: u32| mesh.positions.get(i as usize).copied();
        let marked: Vec<((u32, u32), u32)> = uses
            .iter()
            .filter(|(key, use_)| {
                let ((a, b), n) = (**key, use_.0);
                let (va, vb) = (f64::from(value(a)), f64::from(value(b)));
                let long = match (position(a), position(b)) {
                    (Some(p), Some(q)) => p.distance(q) > floor,
                    _ => false,
                };
                n == 2 && long && va.is_finite() && vb.is_finite() && (va - vb).abs() > step
            })
            .map(|(k, &(_, face))| (*k, face))
            .collect();
        if marked.is_empty() || mesh.positions.len().saturating_add(marked.len()) > MAX_MAP_VERTICES
        {
            break;
        }
        // The midpoints, on the exact surface where it can be found.
        let mut fresh: Vec<((u32, u32), Vec3, Vec3, f32)> = Vec::with_capacity(marked.len());
        for ((a, b), face) in marked {
            let (Some(p), Some(q)) = (position(a), position(b)) else {
                continue;
            };
            let na = mesh.normals.get(a as usize).copied().unwrap_or(Vec3::ZERO);
            let nb = mesh.normals.get(b as usize).copied().unwrap_or(Vec3::ZERO);
            let guess = (p + q) * 0.5;
            let mut point = guess;
            let mut normal = (na + nb).normalized().unwrap_or(Vec3::ZERO);
            let mut v = 0.5 * (value(a) + value(b));
            if let Some((surface, reversed)) = surface_of(face)
                && let Ok((u, w)) = surface.project(guess)
                && (surface.eval(u, w) - guess).length() <= reach
            {
                point = surface.eval(u, w);
                if let Some(n) = surface.normal(u, w) {
                    normal = if n.dot(na + nb) < 0.0 { -n } else { n };
                }
                if let Some((kg, h)) = curvatures(surface, u, w) {
                    let h = if reversed { h } else { -h };
                    v = kind.of(kg, h) as f32;
                }
            }
            fresh.push(((a, b), point, normal, v));
        }
        let mut middle: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for (key, point, normal, v) in fresh {
            let Ok(index) = u32::try_from(mesh.positions.len()) else {
                break;
            };
            mesh.positions.push(point);
            mesh.normals.push(normal);
            values.push(v);
            middle.insert(key, index);
        }
        // Each triangle split by its split edges (in its own turn order).
        let mut triangles = Vec::with_capacity(mesh.triangles.len() * 2);
        let mut faces = Vec::with_capacity(mesh.triangles.len() * 2);
        for (tri, &face) in mesh.triangles.iter().zip(&mesh.face_ids) {
            let corner = |i: usize| tri.get(i % 3).copied().unwrap_or(0);
            let mid = |k: usize| {
                let (a, b) = (corner(k), corner(k + 1));
                middle.get(&(a.min(b), a.max(b))).copied()
            };
            let ms = [mid(0), mid(1), mid(2)];
            let count = ms.iter().filter(|m| m.is_some()).count();
            // Turned so that the split edges come first: edge k runs from
            // corner k to corner k + 1.
            let turn = (0..3)
                .find(|&r| match count {
                    1 => ms.get(r).copied().flatten().is_some(),
                    2 => {
                        ms.get(r).copied().flatten().is_some()
                            && ms.get((r + 1) % 3).copied().flatten().is_some()
                    }
                    _ => true,
                })
                .unwrap_or(0);
            let c = |k: usize| corner(k + turn);
            let m = |k: usize| ms.get((k + turn) % 3).copied().flatten();
            let new: Vec<[u32; 3]> = match (count, m(0), m(1), m(2)) {
                (1, Some(m0), _, _) => vec![[c(0), m0, c(2)], [m0, c(1), c(2)]],
                (2, Some(m0), Some(m1), _) => {
                    vec![[m0, c(1), m1], [c(0), m0, m1], [c(0), m1, c(2)]]
                }
                (3, Some(m0), Some(m1), Some(m2)) => {
                    vec![[c(0), m0, m2], [m0, c(1), m1], [m2, m1, c(2)], [m0, m1, m2]]
                }
                _ => vec![*tri],
            };
            for t in new {
                triangles.push(t);
                faces.push(face);
            }
        }
        mesh.triangles = triangles;
        mesh.face_ids = faces;
    }
    Ok((mesh, values))
}

/// A range for the map from its values: symmetric about 0 for the
/// Gaussian and the mean curvature, from 0 for the largest; to the 95th
/// percentile of the magnitudes, so that a few singular places (a pole,
/// a sharp blend) do not wash the rest out. `[-1, 1]` (or `[0, 1]`) when
/// there are no values or every magnitude is 0 (a plane: then every value
/// is in the middle of the ramp).
pub fn auto_range(values: &[f32], kind: CurvatureKind) -> [f64; 2] {
    let mut mags: Vec<f64> = values
        .iter()
        .filter(|v| v.is_finite())
        .map(|&v| f64::from(v).abs())
        .collect();
    mags.sort_by(f64::total_cmp);
    let top = if mags.is_empty() {
        1.0
    } else {
        let at = ((mags.len() - 1) as f64 * 0.95).round() as usize;
        mags.get(at).copied().unwrap_or(1.0)
    };
    let top = if top > 0.0 && top.is_finite() {
        top
    } else {
        1.0
    };
    match kind {
        CurvatureKind::Max => [0.0, top],
        _ => [-top, top],
    }
}

/// The combs of `spec` over the scene's bodies, as polylines: per curve
/// every spike (from the curve to its tip) and the line through the tips.
/// A spike is `−scale · κ N` at the point, on the side away from the
/// centre of curvature; `κ N` is the curvature vector of the curve
/// (`craft3d_geom::curvature::curve_curvature`, exact second
/// derivatives), for an iso-curve `S(u₀, ·)` from `S_v` and `S_vv`.
/// `scale` automatic makes the largest spike a tenth of the model's
/// diagonal.
pub fn combs(
    session: &mut Session,
    scene: &Scene,
    spec: &CombSpec,
) -> Result<Vec<Vec<Vec3>>, CommandError> {
    let bodies = scene_bodies(session, scene)?;
    let density = spec.density.clamp(2, MAX_DENSITY) as usize;
    // Per curve: points and curvature vectors.
    let mut curves: Vec<Vec<(Vec3, Vec3)>> = Vec::new();
    if let Some(names) = &spec.edges {
        for (index, sb) in scene.bodies.iter().enumerate() {
            let Some(Some(body)) = bodies.get(index) else {
                continue;
            };
            for (e, edge) in body.edges.iter().enumerate() {
                if !names.is_empty()
                    && !sb
                        .facts
                        .edge_names
                        .get(e)
                        .is_some_and(|n| names.iter().any(|m| m == n))
                {
                    continue;
                }
                if curves.len() >= MAX_COMB_CURVES {
                    break;
                }
                if let craft3d_geom::Curve3::Nurbs(c) = &edge.curve
                    && c.degree() > craft3d_geom::curvature::MAX_CURVATURE_DEGREE
                {
                    return Err(CommandError::new(format!(
                        "analysis: combs: an edge of degree {} has no curvature here (at most {})",
                        c.degree(),
                        craft3d_geom::curvature::MAX_CURVATURE_DEGREE
                    )));
                }
                let (t0, t1) = edge.range;
                let samples: Vec<(Vec3, Vec3)> = (0..density)
                    .filter_map(|i| {
                        let t = t0 + (t1 - t0) * i as f64 / (density - 1) as f64;
                        let (k, _) = curve_curvature(&edge.curve, t)?;
                        Some((edge.curve.eval(t), k))
                    })
                    .collect();
                curves.push(samples);
            }
        }
        if let Some(missing) = names.iter().find(|n| {
            !scene
                .bodies
                .iter()
                .any(|b| b.facts.edge_names.iter().any(|m| m == *n))
        }) {
            return Err(CommandError::new(format!(
                "analysis: combs: there is no edge `{}` in the view",
                short(missing)
            )));
        }
    }
    for iso in &spec.iso {
        let found = scene.bodies.iter().enumerate().find_map(|(index, sb)| {
            if iso
                .feature
                .as_ref()
                .is_some_and(|f| *f != sb.feature && *f != sb.name)
            {
                return None;
            }
            let face = sb.facts.face_names.iter().position(|n| *n == iso.face)?;
            Some((index, sb, face))
        });
        let Some((index, sb, face)) = found else {
            return Err(CommandError::new(format!(
                "analysis: combs: there is no face `{}` in the view",
                short(&iso.face)
            )));
        };
        let Some(Some(body)) = bodies.get(index) else {
            continue;
        };
        let Some(topo) = body.faces.get(face) else {
            continue;
        };
        let Some(((u0, u1), (v0, v1))) =
            parameter_box(scene, sb.first_face_id, face, &topo.surface)
        else {
            continue;
        };
        let surface = &topo.surface;
        if !craft3d_geom::curvature::surface_curvature_supported(surface) {
            return Err(CommandError::new(format!(
                "analysis: combs: the face `{}` is a spline of a degree beyond {}, whose curvature is not worked out",
                short(&iso.face),
                craft3d_geom::curvature::MAX_CURVATURE_DEGREE
            )));
        }
        let samples: Vec<(Vec3, Vec3)> = (0..density)
            .filter_map(|i| {
                let s = i as f64 / (density - 1) as f64;
                let (u, v) = if iso.fixed_u {
                    (u0 + (u1 - u0) * iso.at, v0 + (v1 - v0) * s)
                } else {
                    (u0 + (u1 - u0) * s, v0 + (v1 - v0) * iso.at)
                };
                let (p, su, sv) = surface.derivatives(u, v);
                let (suu, _, svv) = second_derivatives(surface, u, v);
                let (d1, d2) = if iso.fixed_u { (sv, svv) } else { (su, suu) };
                let speed2 = d1.length_squared();
                if !(speed2.is_finite() && speed2 > 0.0) {
                    return None;
                }
                let k = d1.cross(d2).cross(d1) * (1.0 / (speed2 * speed2));
                k.is_finite().then_some((p, k))
            })
            .collect();
        curves.push(samples);
    }
    let largest = curves
        .iter()
        .flatten()
        .map(|(_, k)| k.length())
        .fold(0.0_f64, f64::max);
    let diagonal = scene.bodies_box.diagonal();
    let scale = match spec.scale {
        Some(s) => s,
        None if largest > 0.0 && diagonal.is_finite() && diagonal > 0.0 => 0.1 * diagonal / largest,
        None => 0.0,
    };
    let mut out = Vec::new();
    for curve in curves {
        if curve.len() < 2 {
            continue;
        }
        let tips: Vec<Vec3> = curve.iter().map(|&(p, k)| p - k * scale).collect();
        for (&(p, _), &tip) in curve.iter().zip(&tips) {
            if (tip - p).length() > 0.0 {
                out.push(vec![p, tip]);
            }
        }
        out.push(tips);
    }
    Ok(out)
}

/// The box in a face's parameters that its mesh's vertices span (the face
/// trimmed; the parameters of a vertex by projection).
fn parameter_box(
    scene: &Scene,
    first_face_id: u32,
    face: usize,
    surface: &Surface,
) -> Option<((f64, f64), (f64, f64))> {
    let id = first_face_id.checked_add(u32::try_from(face).ok()?)?;
    let mesh = &scene.mesh;
    let mut seen = std::collections::BTreeSet::new();
    for (tri, &f) in mesh.triangles.iter().zip(&mesh.face_ids) {
        if f == id {
            seen.extend(tri.iter().copied());
        }
    }
    let (mut lo, mut hi) = (
        (f64::INFINITY, f64::INFINITY),
        (f64::NEG_INFINITY, f64::NEG_INFINITY),
    );
    // At most 512 vertices are projected: enough for the box.
    let step = (seen.len() / 512).max(1);
    for &k in seen.iter().step_by(step) {
        let Some(&p) = mesh.positions.get(k as usize) else {
            continue;
        };
        if let Ok((u, v)) = surface.project(p) {
            lo = (lo.0.min(u), lo.1.min(v));
            hi = (hi.0.max(u), hi.1.max(v));
        }
    }
    (lo.0 <= hi.0 && lo.1 <= hi.1 && lo.0.is_finite() && hi.1.is_finite())
        .then_some(((lo.0, hi.0), (lo.1, hi.1)))
}

// ------------------------------------------------------------ measures

use crate::verify_cmds::{Ctx, Dim, Outcome};

/// How the least draft over one triangle of a curved face is bounded from
/// below, given its corners and their exact normals (unit, outward) and
/// the unit pull `d`: `None` where no bound is known (a cone, a spline).
///
/// - *Cylinder*: the normal depends on the angle about the axis only,
///   `n·d = s R cos(θ − φ)` (`R` the length of `d` across the axis, `s` the
///   side the normals face), so its least over the triangle's angular
///   range (the corners' angles, less than half a turn apart) is exact:
///   the ends, or `−R` where `φ` (or `φ + π`) lies within. Exact.
/// - *Sphere*: two points of a triangle are at most its longest side `c`
///   apart, so their normals at most `2 asin(c / 2r)`: the least corner's
///   draft less that.
/// - *Torus*: at most `κ_max · π/2 · c`, `κ_max` the larger of `1/minor`
///   and `1/|major − minor|` (the arc between two points at most `π/2`
///   times their chord), capped at π.
fn triangle_lower(surface: &Surface, corners: &[(Vec3, Vec3); 3], d: Vec3) -> Option<f64> {
    let draft = |n: Vec3| n.dot(d).clamp(-1.0, 1.0).asin();
    let least = corners
        .iter()
        .map(|(_, n)| draft(*n))
        .fold(f64::INFINITY, f64::min);
    let [(pa, _), (pb, _), (pc, _)] = *corners;
    let longest = (pb - pa)
        .length()
        .max((pc - pb).length())
        .max((pa - pc).length());
    let turn = |t: f64| {
        let t = t.min(std::f64::consts::PI);
        t.is_finite().then_some(least - t)
    };
    match surface {
        Surface::Cylinder(c) => {
            let f = c.frame();
            let (x, y) = (f.x(), f.y());
            let angle = |p: Vec3| {
                let q = p - f.origin();
                q.dot(y).atan2(q.dot(x))
            };
            // The side: the normals point away from the axis or towards it.
            let radial = |p: Vec3| {
                let q = p - f.origin();
                q - f.z() * q.dot(f.z())
            };
            let s = if corners.iter().map(|(p, n)| radial(*p).dot(*n)).sum::<f64>() >= 0.0 {
                1.0
            } else {
                -1.0
            };
            let (a, b) = (x.dot(d), y.dot(d));
            let r = a.hypot(b);
            let phi = b.atan2(a);
            // Unwrap the corners' angles about the first.
            let first = angle(pa);
            let tau = std::f64::consts::TAU;
            let mut lo = first;
            let mut hi = first;
            for (p, _) in corners.iter().skip(1) {
                let t = angle(*p);
                let t = t - tau * ((t - first) / tau).round();
                lo = lo.min(t);
                hi = hi.max(t);
            }
            if hi - lo >= std::f64::consts::PI {
                return turn(std::f64::consts::PI);
            }
            let value = |t: f64| s * r * (t - phi).cos();
            let mut m = value(lo).min(value(hi));
            // Where s·cos(θ − φ) is −1: θ = φ + π for s > 0, φ for s < 0.
            let at = if s > 0.0 {
                phi + std::f64::consts::PI
            } else {
                phi
            };
            let k = ((lo - at) / tau).ceil();
            if at + k * tau <= hi {
                m = -r;
            }
            Some(m.clamp(-1.0, 1.0).asin())
        }
        Surface::Sphere(sp) => turn(2.0 * (longest / (2.0 * sp.radius())).min(1.0).asin()),
        Surface::Torus(t) => {
            let (major, minor) = t.radii();
            let k = (1.0 / minor).max(1.0 / (major - minor).abs());
            turn(k * std::f64::consts::FRAC_PI_2 * longest)
        }
        _ => None,
    }
}

/// The mesh a measure of the view's kind looks at: within 2e-4 of the
/// body's size and 8° between neighbouring normals, at most 2 million
/// triangles.
fn measure_mesh(body: &Arc<Body>, tolerance: f64) -> Result<craft3d_tess::Mesh, String> {
    let size = crate::mass::mass_of(body)
        .ok()
        .map(|m| m.area.sqrt())
        .filter(|a| a.is_finite() && *a > 0.0)
        .unwrap_or(1.0);
    let params = craft3d_tess::TessParams {
        chordal_tolerance: (size * 2e-4).max(10.0 * tolerance),
        max_angle: Some(8f64.to_radians()),
        max_triangles: Some(2_000_000),
        ..craft3d_tess::TessParams::default()
    };
    craft3d_tess::tessellate_body(body, &params).map_err(|e| e.to_string())
}

/// The measure `draft`: the least draft angle of the faces named against
/// the pull `direction`, in radians (the angle between the face and the
/// pull, `asin(n · d)` with `n` the outward normal: positive where the face
/// releases along the pull), and the share of their area below
/// `minDraft` (degrees, default 1; not `min`, which an assertion's bound
/// is). A plane is exact. Elsewhere the normals are the exact surface's at
/// the vertices of a mesh within 2e-4 of the body's size and 8° between
/// neighbours, and the least is at a real place (the measure's upper
/// bound); on a cylinder, a sphere and a torus every triangle bounds the
/// least below ([`triangle_lower`]: exact on a cylinder, the normal's turn
/// over the triangle on the others), proven; on a cone and a spline the
/// least is sampled and an assertion on it is not proven.
pub(crate) fn draft_measure(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    crate::verify_cmds::only(p, &["faces", "direction", "minDraft"])?;
    let d = match p.get("direction") {
        None | Some(Value::Null) => Vec3::new(0.0, 0.0, 1.0),
        v => crate::verify_cmds::direction(v, "direction")?,
    };
    let min = match p.get("minDraft") {
        None | Some(Value::Null) => 1.0,
        Some(v) => v
            .as_f64()
            .filter(|m| m.is_finite() && (0.0..=90.0).contains(m))
            .ok_or("`minDraft` must be a number of degrees from 0 to 90")?,
    }
    .to_radians();
    let found = crate::verify_cmds::faces_of_subject(ctx, p.get("faces"))?;
    let draft_of = |n: Vec3| n.dot(d).clamp(-1.0, 1.0).asin();
    let (mut least, mut lower, mut proven) = (f64::INFINITY, f64::INFINITY, true);
    let (mut area, mut below) = (0.0_f64, 0.0_f64);
    let mut at = Vec3::ZERO;
    let mut least_face = Value::Null;
    let mut listed = Vec::new();
    for (feature, body, faces, name) in &found {
        let mesh = measure_mesh(body, ctx.tolerance.linear).map_err(|e| format!("{name}: {e}"))?;
        // The triangles of each face, gathered once.
        let mut by_face: Vec<Vec<usize>> = vec![Vec::new(); body.faces.len()];
        for (t, &f) in mesh.face_ids.iter().enumerate() {
            if let Some(list) = by_face.get_mut(f as usize) {
                list.push(t);
            }
        }
        for &face in faces {
            let (Some(topo), Some(triangles)) = (body.faces.get(face), by_face.get(face)) else {
                continue;
            };
            let (mut f_least, mut f_at, mut f_lower) = (f64::INFINITY, Vec3::ZERO, f64::INFINITY);
            let (mut f_area, mut f_below, mut bounded) = (0.0_f64, 0.0_f64, true);
            for &t in triangles {
                let Some(tri) = mesh.triangles.get(t) else {
                    continue;
                };
                let corner = |k: u32| -> Option<(Vec3, Vec3)> {
                    let n = mesh.normals.get(k as usize)?.normalized()?;
                    Some((*mesh.positions.get(k as usize)?, n))
                };
                let (Some(a), Some(b), Some(c)) = (corner(tri[0]), corner(tri[1]), corner(tri[2]))
                else {
                    continue;
                };
                let corners = [a, b, c];
                for (q, n) in corners {
                    let angle = draft_of(n);
                    if angle < f_least {
                        (f_least, f_at) = (angle, q);
                    }
                }
                match triangle_lower(&topo.surface, &corners, d) {
                    Some(l) => f_lower = f_lower.min(l),
                    None => bounded = false,
                }
                let tri_area = (b.0 - a.0).cross(c.0 - a.0).length() * 0.5;
                if let Some(mid) = (a.1 + b.1 + c.1).normalized() {
                    f_area += tri_area;
                    if draft_of(mid) < min {
                        f_below += tri_area;
                    }
                }
            }
            let face_name = ctx.face_name(*feature, face);
            let (method, f_lower) = match &topo.surface {
                Surface::Plane(plane) => {
                    let n = plane.frame().z() * if topo.reversed { -1.0 } else { 1.0 };
                    let exact = draft_of(n);
                    f_least = exact;
                    f_below = if exact < min { f_area } else { 0.0 };
                    ("exact", Some(exact))
                }
                Surface::Cylinder(_) | Surface::Sphere(_) | Surface::Torus(_)
                    if bounded && f_lower.is_finite() =>
                {
                    ("bound", Some(f_lower.min(f_least)))
                }
                _ => ("sampled", None),
            };
            if !f_least.is_finite() {
                continue;
            }
            area += f_area;
            below += f_below;
            match f_lower {
                Some(l) => lower = lower.min(l),
                None => proven = false,
            }
            if f_least < least {
                (least, at, least_face) = (f_least, f_at, face_name.clone());
            }
            if listed.len() < 200 {
                listed.push(json!({
                    "face": face_name,
                    "body": ctx.feature_name(*feature),
                    "least": f_least.to_degrees(),
                    "atLeast": f_lower.map(f64::to_degrees),
                    "method": method,
                    "shareBelow": if f_area > 0.0 { f_below / f_area } else { 0.0 },
                }));
            }
        }
    }
    if !least.is_finite() {
        return Err("no face to measure the draft of".into());
    }
    let dim = Dim::Of(craft3d_expr::Dimension::ANGLE);
    let mut out = Outcome::number(dim, least);
    out.upper = Some(least);
    out.lower = proven.then_some(lower.min(least));
    out.detail.insert("least".into(), json!(least.to_degrees()));
    out.detail.insert(
        "atLeast".into(),
        json!(proven.then_some(lower.min(least).to_degrees())),
    );
    out.detail.insert("at".into(), json!([at.x, at.y, at.z]));
    out.detail.insert("face".into(), least_face);
    out.detail.insert(
        "shareBelow".into(),
        json!(if area > 0.0 { below / area } else { 0.0 }),
    );
    out.detail
        .insert("minDraft".into(), json!(min.to_degrees()));
    out.detail
        .insert("direction".into(), json!([d.x, d.y, d.z]));
    out.detail.insert("proven".into(), json!(proven));
    out.detail.insert("faces".into(), json!(listed));
    Ok(out)
}

/// The measure `curvature` with `at`: the curvature of the faces named
/// (default every body) at the point of them nearest to `at` (the face by
/// its mesh, then the exact surface's nearest point): `K` as the number
/// (length⁻²), with `H` (positive where convex), the principal curvatures
/// `k1 ≥ k2`, the face and the point. Exact: the surface's second
/// derivatives (`craft3d_geom::curvature`).
pub(crate) fn curvature_at(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    crate::verify_cmds::only(p, &["of", "at"])?;
    let target = match p.get("at") {
        Some(Value::Array(a)) if a.len() == 3 => {
            let v: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
            match v.as_slice() {
                [x, y, z] if v.iter().all(|c| c.is_finite()) => Vec3::new(*x, *y, *z),
                _ => return Err("`at` must be [x, y, z]".into()),
            }
        }
        _ => return Err("`at` must be [x, y, z]".into()),
    };
    let found = crate::verify_cmds::faces_of_subject(ctx, p.get("of"))?;
    let mut best: Option<(f64, FeatureIdAndFace)> = None;
    for (feature, body, faces, _) in &found {
        let mesh = measure_mesh(body, ctx.tolerance.linear)?;
        let wanted: std::collections::BTreeSet<usize> = faces.iter().copied().collect();
        for (tri, &f) in mesh.triangles.iter().zip(&mesh.face_ids) {
            if !wanted.contains(&(f as usize)) {
                continue;
            }
            let Some(corners) = tri
                .iter()
                .map(|&k| mesh.positions.get(k as usize).copied())
                .collect::<Option<Vec<Vec3>>>()
            else {
                continue;
            };
            let [a, b, c] = corners.as_slice() else {
                continue;
            };
            let dist = (closest_on_triangle(target, *a, *b, *c) - target).length();
            if best.as_ref().is_none_or(|(d, _)| dist < *d) {
                best = Some((dist, (*feature, Arc::clone(body), f as usize)));
            }
        }
    }
    let Some((_, (feature, body, face))) = best else {
        return Err("no face to measure the curvature of".into());
    };
    let topo = body.faces.get(face).ok_or("the face is gone")?;
    let (u, v) = topo.surface.project(target).map_err(|e| e.to_string())?;
    let point = topo.surface.eval(u, v);
    let (k, h) = curvatures(&topo.surface, u, v)
        .ok_or("the surface has no curvature there (no normal: an apex or a pole)")?;
    let h = if topo.reversed { h } else { -h };
    let root = (h * h - k).max(0.0).sqrt();
    let mut out = Outcome::number(Dim::Length(-2), k);
    out.detail.insert("gaussian".into(), json!(k));
    out.detail.insert("mean".into(), json!(h));
    out.detail.insert("k1".into(), json!(h + root));
    out.detail.insert("k2".into(), json!(h - root));
    out.detail
        .insert("face".into(), ctx.face_name(feature, face));
    out.detail
        .insert("body".into(), json!(ctx.feature_name(feature)));
    out.detail
        .insert("point".into(), json!([point.x, point.y, point.z]));
    out.detail
        .insert("distance".into(), json!((point - target).length()));
    Ok(out)
}

type FeatureIdAndFace = (craft3d_doc::FeatureId, Arc<Body>, usize);

/// The point of the triangle `abc` nearest to `p` (Ericson, *Real-Time
/// Collision Detection*, 5.1.5).
fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let t = d1 / (d1 - d3);
        return if t.is_finite() { a + ab * t } else { a };
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let t = d2 / (d2 - d6);
        return if t.is_finite() { a + ac * t } else { a };
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let t = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return if t.is_finite() { b + (c - b) * t } else { b };
    }
    let denom = va + vb + vc;
    if !(denom.is_finite() && denom != 0.0) {
        return a;
    }
    let (v, w) = (vb / denom, vc / denom);
    a + ab * v + ac * w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_analysis_is_parsed_and_bad_fields_are_errors() {
        let base = AnalysisSpec::default();
        let z = AnalysisSpec::patched(
            &base,
            &json!({"mode": "zebra", "stripes": 24, "direction": 400, "width": 0.3}),
        )
        .unwrap();
        assert_eq!(z.mode, Some(AnalysisMode::Zebra));
        assert_eq!(z.direction, 40.0);
        assert!(AnalysisSpec::patched(&base, &json!({"mode": "rainbow"})).is_err());
        assert!(AnalysisSpec::patched(&base, &json!({"stripes": 0})).is_err());
        assert!(AnalysisSpec::patched(&base, &json!({"width": 2})).is_err());
        assert!(AnalysisSpec::patched(&base, &json!({"pull": [0, 0, 0]})).is_err());
        assert!(AnalysisSpec::patched(&base, &json!({"range": [1, 0]})).is_err());
        assert!(AnalysisSpec::patched(&base, &json!({"wat": 1})).is_err());
        assert!(AnalysisSpec::patched(&base, &json!({"combs": {"density": 1}})).is_err());
        assert!(
            AnalysisSpec::patched(&base, &json!({"combs": {"iso": [{"face": "F1:a"}]}})).is_err()
        );
        assert!(AnalysisSpec::patched(&base, &json!(3)).is_err());
        let off = AnalysisSpec::patched(&z, &Value::Null).unwrap();
        assert_eq!(off.mode, None);
        let round = AnalysisSpec::patched(&base, &z.json()).unwrap();
        assert_eq!(round, z);
        let c = AnalysisSpec::patched(
            &base,
            &json!({"combs": {"edges": true, "iso": [{"face": "F1:x", "u": 0.5}], "density": 20}}),
        )
        .unwrap();
        assert_eq!(AnalysisSpec::patched(&base, &c.json()).unwrap(), c);
    }

    #[test]
    fn auto_range_is_symmetric_and_skips_outliers() {
        let mut v = vec![1.0_f32; 100];
        v[3] = 1000.0;
        v[4] = f32::NAN;
        assert_eq!(auto_range(&v, CurvatureKind::Gaussian), [-1.0, 1.0]);
        assert_eq!(auto_range(&v, CurvatureKind::Max), [0.0, 1.0]);
        assert_eq!(auto_range(&[], CurvatureKind::Mean), [-1.0, 1.0]);
    }

    #[test]
    fn the_largest_curvature_of_a_saddle() {
        assert_eq!(CurvatureKind::Max.of(-4.0, 0.0), 2.0);
        assert_eq!(CurvatureKind::Max.of(1.0, -1.0), 1.0);
    }
}
