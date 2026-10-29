//! The DXF and SVG side of `file.export` (the command itself is in
//! `commands.rs`): the outline of a sketch or of a flat face, for laser
//! cutting and for drawings.

use crate::commands::{Run, fail, feature_ref};
use crate::params::Params;
use crate::{CommandError, Session};
use craft3d_doc::{LengthUnit, PersistentName};
use craft3d_drawing::{Color, Drawing, Layer, Path as DrawingPath, Unit};
use craft3d_io::{DrawingFormat, FaceAxes};
use craft3d_regen::Status;
use serde_json::{Map, Value, json};
use std::path::Path;

const ID: &str = "file.export";

/// The drawing format `file.export` should write, if any: by `format`, or
/// else by the extension of `path`.
pub(crate) fn wanted(format: Option<&str>, path: &str) -> Option<DrawingFormat> {
    match format {
        Some(format) => match format {
            "dxf" => Some(DrawingFormat::Dxf),
            "svg" => Some(DrawingFormat::Svg),
            _ => None,
        },
        None => Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(DrawingFormat::from_extension),
    }
}

/// The drawing's unit for a document's.
pub(crate) fn drawing_unit(unit: LengthUnit) -> Unit {
    match unit {
        LengthUnit::Micrometre => Unit::Micron,
        LengthUnit::Millimetre => Unit::Millimeter,
        LengthUnit::Centimetre => Unit::Centimeter,
        LengthUnit::Metre => Unit::Meter,
        LengthUnit::Inch => Unit::Inch,
        LengthUnit::Foot => Unit::Foot,
    }
}

/// What the paths were taken from, for the result.
pub(crate) struct Source {
    pub(crate) json: Value,
    /// Layers of their own (a sheet's faces), before `paths`.
    pub(crate) layers: Vec<Layer>,
    pub(crate) paths: Vec<DrawingPath>,
    pub(crate) construction: Vec<DrawingPath>,
}

/// Writes the outline of the sketch or the face the params name to
/// `resolved`. `path` is the path as the request gave it, for the result.
pub(crate) fn export(
    session: &mut Session,
    p: &Params,
    path: &str,
    resolved: &Path,
    format: DrawingFormat,
) -> Run {
    let given: Vec<&str> = [
        "sketch",
        "face",
        "faces",
        "set",
        "fold",
        "sheetMetal",
        "trace",
    ]
    .into_iter()
    .filter(|key| p.has(key))
    .collect();
    let refuse = |keys: &[&str], why: &str| -> Result<(), CommandError> {
        match keys.iter().find(|key| p.has(key)) {
            Some(key) => fail(format!("{ID}: `{key}` {why}")),
            None => Ok(()),
        }
    };
    // Neither: the model as seen (`view_export.rs`).
    if given.is_empty() {
        return crate::view_export::export(session, p, path, resolved, format);
    }
    refuse(
        &crate::view_export::VIEW_KEYS,
        "is about a drawing of the model as seen, which is what a .dxf or .svg file holds without `sketch`, `face`, `faces` or `set`",
    )?;
    let source = match given.as_slice() {
        [] => {
            return fail(format!(
                "{ID}: a .{} file is the outline of a sketch, of a flat face or of a sheet of flat faces: give `sketch` (its id or name), `face` (a name from document.inspect, faceNames), `faces` (a list of them) or `set` (a named set of faces, set.define)",
                format.name()
            ));
        }
        ["fold"] => {
            refuse(
                &["feature", "construction", "xAxis", "origin", "gap", "width"],
                "is about the outline of a sketch or of faces; a crease pattern is drawn in its sketch's axes",
            )?;
            crate::fold_cmds::crease_pattern(session, p, ID)?
        }
        ["sheetMetal"] => {
            // A list of parts is a sheet of their flat patterns, `gap`
            // apart in rows of at most `width`.
            let sheet = matches!(p.get("sheetMetal"), Some(Value::Array(_)));
            let own: &[&str] = if sheet {
                &["feature", "construction", "xAxis", "origin"]
            } else {
                &["feature", "construction", "xAxis", "origin", "gap", "width"]
            };
            refuse(
                own,
                "is about the outline of a sketch or of faces; a flat pattern is drawn in its sketch's axes",
            )?;
            crate::sheetmetal_cmds::flat_pattern(session, p, ID)?
        }
        ["trace"] => {
            refuse(
                &["feature", "construction", "xAxis", "origin", "gap", "width"],
                "is about the outline of a sketch or of faces; a trace is drawn in its own plane",
            )?;
            crate::motion::trace_drawing(session, p, ID)?
        }
        ["sketch"] => {
            refuse(
                &["feature"],
                "says whose face is meant; it does not go with `sketch`",
            )?;
            refuse(
                &["xAxis", "origin", "gap", "width"],
                "is about the drawing of a face; a sketch is drawn in its own axes",
            )?;
            sketch(session, p)?
        }
        [faces] => {
            if p.bool("construction")?.is_some() {
                return fail(format!(
                    "{ID}: `construction` is about a sketch; a face has no construction geometry"
                ));
            }
            if *faces == "face" {
                refuse(
                    &["gap", "width"],
                    "is about a sheet of several faces (`faces`)",
                )?;
                one_face(session, p)?
            } else {
                refuse(
                    &["feature", "origin"],
                    "is for one `face`; on a sheet each entry of `faces` may say its own `feature` (a set's faces are each of their own body), and every face is placed by the corner of its outline",
                )?;
                sheet(session, p)?
            }
        }
        several => {
            let keys: Vec<String> = several.iter().map(|k| format!("`{k}`")).collect();
            return fail(format!(
                "{ID}: give either a `sketch` or a `face` (or `faces` or a `set`, for a sheet), not {} at once",
                keys.join(" and ")
            ));
        }
    };
    let doc = session.document()?.history.current();
    let tolerance = match (format, p.number("tolerance")?) {
        (DrawingFormat::Dxf, Some(_)) => {
            return fail(format!(
                "{ID}: `tolerance` is for SVG, where a spline of a degree above 3 becomes cubic Béziers within it; DXF holds every curve exactly"
            ));
        }
        (_, Some(tolerance)) if !(tolerance.is_finite() && tolerance > 0.0) => {
            return fail(format!("{ID}: `tolerance` must be positive"));
        }
        (_, Some(tolerance)) => tolerance,
        (_, None) => doc.tolerance.linear,
    };
    let unit = doc.units.length;
    let mut layers = source.layers;
    for (name, color, paths) in [
        ("outline", Color::Black, source.paths),
        ("construction", Color::Blue, source.construction),
    ] {
        if !paths.is_empty() {
            layers.push(Layer {
                name: name.to_string(),
                color,
                dash: None,
                dot: false,
                paths,
            });
        }
    }
    let drawing = Drawing {
        unit: drawing_unit(unit),
        layers,
    };
    let (bytes, report) =
        craft3d_io::encode_drawing_stroked(&drawing, format, tolerance, stroke(p, format)?)
            .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let size = bytes.len();
    session.write_resolved(resolved, bytes)?;
    let counts = drawing.counts();
    let mut result = json!({
        "path": path,
        "format": format.name(),
        "bytes": size,
        "unit": unit.symbol(),
        "paths": counts.paths,
        // Paths that come back to their start: what a laser cuts out.
        "closed": counts.closed,
        "lines": counts.lines,
        "arcs": counts.arcs,
        "circles": counts.circles,
        "ellipses": counts.ellipses,
        "splines": counts.splines,
        "length": drawing.length(),
        // [min x, min y, max x, max y] in the axes of the sketch or face,
        // or of the sheet.
        "bbox": drawing.bbox(),
    });
    if let (Some(map), Value::Object(source)) = (result.as_object_mut(), source.json) {
        map.extend(source);
        match report {
            // Splines SVG has no exact form for, and how far the Béziers
            // written for them lie from them at most.
            Some(report) => {
                map.insert("approximated".into(), report.approximated.into());
                map.insert("deviation".into(), report.deviation.into());
                map.insert("tolerance".into(), tolerance.into());
            }
            None => {
                map.insert("version".into(), craft3d_drawing::DXF_VERSION.into());
            }
        }
    }
    Ok(result)
}

fn sketch(session: &mut Session, p: &Params) -> Result<Source, CommandError> {
    {
        let doc = session.document()?.history.current();
        let id = feature_ref(doc, p, ID, "sketch")?;
        if matches!(
            doc.feature(id).map(|f| &f.kind),
            Some(craft3d_doc::FeatureKind::OffsetProfile(_))
        ) {
            return offset(session, id);
        }
    }
    let id = crate::sketch_cmds::sketch_ref(session, p, ID)?;
    let construction = p.bool("construction")?.unwrap_or(false);
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let feature = doc
        .feature(id)
        .ok_or_else(|| CommandError::new(format!("{ID}: there is no feature {id}")))?;
    let name = &feature.name;
    let sketch = doc
        .sketch(id)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let outcome = regen.feature(id);
    // A sketch whose constraints conflict has no shape to cut.
    if let Some(Status::Failed(reason)) = outcome.map(|o| &o.status) {
        return fail(format!(
            "{ID}: the sketch `{name}` failed: {reason}; fix it first (sketch.solve says what is wrong)"
        ));
    }
    let Some(solved) = outcome.and_then(|o| o.sketch.as_ref()) else {
        return fail(format!(
            "{ID}: the sketch `{name}` is not regenerated (suppressed or rolled back)"
        ));
    };
    // The instances of the sketch's patterns are curves of the outline
    // too: drawn as entities of a copy, at the places regeneration made.
    let (drawn, placed, splines) = with_instances(sketch, solved)
        .map_err(|e| CommandError::new(format!("{ID}: the sketch `{name}`: {e}")))?;
    let outline = craft3d_io::sketch_outline(
        &drawn,
        &|entity| {
            solved
                .point(entity)
                .or_else(|| placed.get(&entity).copied())
        },
        &|entity| solved.radius(entity),
        &|entity| {
            splines.get(&entity).cloned().or_else(|| {
                solved.curve(entity).map(|c| {
                    (
                        c.spline.degree,
                        c.spline.knots.clone(),
                        c.spline.control.clone(),
                    )
                })
            })
        },
        construction,
        doc.tolerance.linear,
    )
    .map_err(|e| CommandError::new(format!("{ID}: the sketch `{name}`: {e}")))?;
    if outline.paths.is_empty() && outline.construction.is_empty() {
        return fail(format!(
            "{ID}: the sketch `{name}` has no lines, arcs or circles to export{}",
            if construction {
                ""
            } else {
                " (construction geometry is left out; pass \"construction\": true to include it)"
            }
        ));
    }
    let skipped: Vec<String> = outline.degenerate.iter().map(ToString::to_string).collect();
    Ok(Source {
        json: json!({
            "sketch": id.to_string(),
            "name": name,
            // Lines and arcs without length, left out.
            "skipped": skipped,
        }),
        layers: Vec::new(),
        paths: outline.paths,
        construction: outline.construction,
    })
}

/// Where the points [`with_instances`] adds lie.
type Placed = std::collections::BTreeMap<craft3d_doc::EntityId, (f64, f64)>;
/// The splines [`with_instances`] adds as they are: on their own knots,
/// which a sketch spline (on uniform knots) cannot hold.
type Splines = std::collections::BTreeMap<craft3d_doc::EntityId, craft3d_io::FittedSpline>;

/// `sketch` with the curves its patterns make added as entities of their
/// own (on points of their own, at the places the instances have), and
/// where those points are. A sketch without patterns is itself.
fn with_instances(
    sketch: &craft3d_doc::Sketch,
    solved: &craft3d_regen::SketchResult,
) -> Result<(craft3d_doc::Sketch, Placed, Splines), String> {
    let mut drawn = sketch.clone();
    let mut placed = Placed::new();
    let mut splines = Splines::new();
    if sketch.patterns.is_empty() {
        return Ok((drawn, placed, splines));
    }
    let instances = craft3d_regen::pattern_instances(sketch, solved)?;
    add_curves(
        &mut drawn,
        &mut placed,
        &mut splines,
        instances
            .into_iter()
            .map(|(made, construction)| (made.curve, construction)),
    )?;
    Ok((drawn, placed, splines))
}

/// Adds `curves` to `drawn` as entities on points of their own, and
/// records where those points are.
fn add_curves(
    drawn: &mut craft3d_doc::Sketch,
    placed: &mut Placed,
    splines: &mut Splines,
    curves: impl Iterator<Item = (craft3d_regen::Curve2, bool)>,
) -> Result<(), String> {
    use craft3d_regen::Curve2;
    let text = |e: craft3d_doc::SketchError| e.to_string();
    for (curve, construction) in curves {
        let mut at = |[x, y]: [f64; 2]| -> Result<craft3d_doc::EntityId, String> {
            let id = drawn.add_point(x, y).map_err(text)?;
            placed.insert(id, (x, y));
            Ok(id)
        };
        match curve {
            Curve2::Line { start, end } => {
                let (a, b) = (at(start)?, at(end)?);
                drawn.add_line(a, b, construction).map_err(text)?;
            }
            Curve2::Arc { center, start, end } => {
                let (c, a, b) = (at(center)?, at(start)?, at(end)?);
                drawn.add_arc(c, a, b, construction).map_err(text)?;
            }
            Curve2::Circle { center, radius } => {
                let c = at(center)?;
                drawn.add_circle(c, radius, construction).map_err(text)?;
            }
            Curve2::Spline(spline) => {
                let mut control = Vec::with_capacity(spline.control.len());
                for point in &spline.control {
                    control.push(at(*point)?);
                }
                let degree = u32::try_from(spline.degree).map_err(|e| e.to_string())?;
                let id = drawn
                    .add_spline(degree, control, construction)
                    .map_err(text)?;
                splines.insert(id, (spline.degree, spline.knots, spline.control));
            }
        }
    }
    Ok(())
}

/// The outline an offset of the profiles of a sketch made: the beam path
/// of a laser, with its kerf.
fn offset(session: &mut Session, id: craft3d_doc::FeatureId) -> Result<Source, CommandError> {
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let Some(feature) = doc.feature(id) else {
        return fail(format!("{ID}: there is no feature {id}"));
    };
    let name = &feature.name;
    let outcome = regen.feature(id);
    if let Some(Status::Failed(reason)) = outcome.map(|o| &o.status) {
        return fail(format!(
            "{ID}: the offset `{name}` failed: {reason}; fix it first"
        ));
    }
    let Some(made) = outcome.and_then(|o| o.offset.as_ref()) else {
        return fail(format!(
            "{ID}: the offset `{name}` is not regenerated (suppressed or rolled back)"
        ));
    };
    let plane = doc
        .sketch(made.sketch)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?
        .plane
        .clone();
    let mut drawn = craft3d_doc::Sketch::new(plane);
    let mut placed = Placed::new();
    let mut splines = Splines::new();
    add_curves(
        &mut drawn,
        &mut placed,
        &mut splines,
        made.outline.curves.iter().map(|c| (c.curve.clone(), false)),
    )
    .map_err(|e| CommandError::new(format!("{ID}: the offset `{name}`: {e}")))?;
    let outline = craft3d_io::sketch_outline(
        &drawn,
        &|entity| placed.get(&entity).copied(),
        // A circle has its radius in the sketch.
        &|_| None,
        &|entity| splines.get(&entity).cloned(),
        false,
        doc.tolerance.linear,
    )
    .map_err(|e| CommandError::new(format!("{ID}: the offset `{name}`: {e}")))?;
    let skipped: Vec<String> = outline.degenerate.iter().map(ToString::to_string).collect();
    Ok(Source {
        json: json!({
            "offset": id.to_string(),
            "name": name,
            "skipped": skipped,
        }),
        layers: Vec::new(),
        paths: outline.paths,
        construction: outline.construction,
    })
}

/// One flat face to draw: its name as given, whose body when said, and
/// how its axes run.
struct Wanted<'a> {
    given: &'a str,
    only: Option<craft3d_doc::FeatureId>,
    axes: FaceAxes,
    /// The parameter, for errors: `` `face` `` or `` `faces.3` ``.
    role: String,
}

/// A face drawn in its frame.
struct Drawn {
    json: Map<String, Value>,
    paths: Vec<DrawingPath>,
    frame: craft3d_math::Frame,
}

fn vec3(v: craft3d_math::Vec3) -> Value {
    // Adding 0 turns a negative zero into zero.
    json!([v.x + 0.0, v.y + 0.0, v.z + 0.0])
}

/// `xAxis`: three finite numbers, a direction in the model.
fn vector(value: Option<&Value>, key: &str) -> Result<Option<craft3d_math::Vec3>, CommandError> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let numbers: Option<Vec<f64>> = value.as_array().and_then(|items| {
        items
            .iter()
            .map(|n| n.as_f64().filter(|n| n.is_finite()))
            .collect()
    });
    match numbers.as_deref() {
        Some(&[x, y, z]) => Ok(Some(craft3d_math::Vec3::new(x, y, z))),
        _ => fail(format!(
            "{ID}: `{key}` must be a direction [x, y, z] of three finite numbers, got {value}"
        )),
    }
}

fn axes(x: Option<craft3d_math::Vec3>) -> FaceAxes {
    x.map_or(FaceAxes::Upright, FaceAxes::Along)
}

/// The face `wanted` names, drawn. The name is resolved as a feature's
/// reference to a face is (docs/design/names.md): a face of an earlier
/// body is followed through the features that took that body to its
/// newest form, and `feature` likewise means the body that feature's body
/// is now.
fn draw(session: &mut Session, wanted: &Wanted) -> Result<Drawn, CommandError> {
    let role = &wanted.role;
    let name: PersistentName = wanted
        .given
        .parse()
        .map_err(|reason| CommandError::new(format!("{ID}: {role}: {reason}")))?;
    if !name.is_face() {
        return fail(format!("{ID}: {role}: {name} is not a face"));
    }
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let called = |id| {
        doc.feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone())
    };
    let followed = craft3d_regen::follow_face(doc, &regen.features, &name, role)
        .map_err(|reason| CommandError::new(format!("{ID}: {reason}")))?;
    let full = followed.clone().unwrap_or_else(|| name.clone());
    let only = wanted
        .only
        .map(|id| craft3d_regen::newest_body(&regen.features, id));
    // The bodies that have a face of that name: the one asked for, or
    // every body that exists.
    let mut found = Vec::new();
    for outcome in &regen.features {
        if only.is_some_and(|id| id != outcome.id) {
            continue;
        }
        let Some(body) = outcome.own_body() else {
            if only.is_some() {
                return fail(format!("{ID}: `{}` has no body", called(outcome.id)));
            }
            continue;
        };
        let Some(names) = &outcome.names else {
            continue;
        };
        if let Some(why) = craft3d_regen::ambiguous_cap(names, &full) {
            return fail(format!("{ID}: {role}: {why}"));
        }
        let faces = craft3d_regen::faces_named(names, &full);
        if !faces.is_empty() {
            found.push((outcome.id, body, faces));
        }
    }
    let (feature, body, faces) = match found.as_slice() {
        [] => {
            return fail(match only {
                Some(id) if followed.is_some() => format!(
                    "{ID}: {role}: `{}` has no face {name}: the body that had it went into `{}`, where it is {full} (a copy that keeps its source, such as a mirror with keep, is a body of its own: give its name from document.inspect, faceNames)",
                    called(id),
                    called(full.feature)
                ),
                Some(id) => format!(
                    "{ID}: {role}: `{}` has no face {full} (document.inspect lists its faceNames)",
                    called(id)
                ),
                None => format!(
                    "{ID}: {role}: no body has a face {full} (document.inspect lists the faceNames of every body)"
                ),
            });
        }
        [one] => one,
        several => {
            let bodies: Vec<String> = several
                .iter()
                .take(8)
                .map(|(id, ..)| format!("`{}`", called(*id)))
                .collect();
            return fail(format!(
                "{ID}: {role}: the bodies of {} each have a face {full}; say which with `feature`",
                bodies.join(", ")
            ));
        }
    };
    let tolerance = craft3d_math::Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or_else(|| {
            CommandError::new(format!(
                "{ID}: the document's tolerance is not positive and finite"
            ))
        })?;
    let outline = craft3d_io::face_outline_in(body, faces, wanted.axes, tolerance)
        .map_err(|e| CommandError::new(format!("{ID}: {role}: the face {full}: {e}")))?;
    let frame = outline.frame;
    let mut json = Map::new();
    json.insert("face".into(), name.to_string().into());
    if followed.is_some() {
        // The name as the newest body has it: what it was followed to.
        json.insert("followed".into(), full.to_string().into());
    }
    json.insert("feature".into(), feature.to_string().into());
    json.insert("name".into(), called(*feature).into());
    // How many faces answer to the name: the pieces of one face.
    json.insert("faces".into(), faces.len().into());
    // Where the drawing lies in the model: its origin and axes. The
    // face is seen from outside the body.
    json.insert("origin".into(), vec3(frame.origin()));
    json.insert("xAxis".into(), vec3(frame.x()));
    json.insert("yAxis".into(), vec3(frame.y()));
    Ok(Drawn {
        json,
        paths: outline.paths,
        frame,
    })
}

/// `[min x, min y, max x, max y]` of paths.
fn bbox(paths: &[DrawingPath]) -> Result<[f64; 4], CommandError> {
    let drawing = Drawing {
        unit: Unit::Millimeter,
        layers: vec![Layer {
            name: "outline".into(),
            color: Color::Black,
            dash: None,
            dot: false,
            paths: paths.to_vec(),
        }],
    };
    drawing
        .bbox()
        .ok_or_else(|| CommandError::new(format!("{ID}: the face has nothing to draw")))
}

fn moved(paths: Vec<DrawingPath>, d: [f64; 2]) -> Vec<DrawingPath> {
    paths.iter().map(|path| path.translated(d)).collect()
}

/// The outline of one flat face, in its drawing frame.
fn one_face(session: &mut Session, p: &Params) -> Result<Source, CommandError> {
    let only = if p.has("feature") {
        Some(feature_ref(
            session.document()?.history.current(),
            p,
            ID,
            "feature",
        )?)
    } else {
        None
    };
    let corner = match p.str("origin")? {
        None | Some("model") => false,
        Some("corner") => true,
        Some(other) => {
            return fail(format!(
                "{ID}: `origin` is `model` (the model's origin dropped onto the face's plane) or `corner` (the lower left corner of the outline), not `{other}`"
            ));
        }
    };
    let wanted = Wanted {
        given: p.required_str("face")?,
        only,
        axes: axes(vector(p.get("xAxis"), "xAxis")?),
        role: "`face`".into(),
    };
    let mut drawn = draw(session, &wanted)?;
    if corner {
        let [x, y, ..] = bbox(&drawn.paths)?;
        drawn.paths = moved(drawn.paths, [-x, -y]);
        let origin = drawn.frame.point(x, y, 0.0);
        drawn.json.insert("origin".into(), vec3(origin));
    }
    Ok(Source {
        json: Value::Object(drawn.json),
        layers: Vec::new(),
        paths: drawn.paths,
        construction: Vec::new(),
    })
}

/// The most faces one sheet takes.
const MAX_SHEET_FACES: usize = 10_000;

/// `name` as a layer name: every run of characters other than letters,
/// digits and `-` becomes one `_`, at most 31 characters, and not one of
/// `taken` (any case): then `-2`, `-3`, … at its end.
fn layer_name(name: &str, taken: &[String]) -> String {
    let mut base = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '-' {
            base.push(c);
        } else if !base.ends_with('_') {
            base.push('_');
        }
    }
    let base = base.trim_matches('_');
    let base = if base.is_empty() { "face" } else { base };
    let free = |candidate: &str| !taken.iter().any(|t| t.eq_ignore_ascii_case(candidate));
    let cut = |text: &str, room: usize| text.chars().take(room).collect::<String>();
    let whole = cut(base, 31);
    if free(&whole) {
        return whole;
    }
    // One more than there are names taken always finds a free one.
    for k in 2..=taken.len().saturating_add(2) {
        let suffix = format!("-{k}");
        let candidate = format!("{}{suffix}", cut(base, 31 - suffix.len()));
        if free(&candidate) {
            return candidate;
        }
    }
    whole
}

/// The flat faces a named set holds now (`file.export {set}`), in
/// document order (bodies in history order, faces in body order): each
/// its persistent name, its body and the name of that body. The faces it
/// holds that are not flat, and how many items of each other kind it
/// holds, are said in the result and left off the sheet. A body hidden in
/// the view (`feature.visible`) is drawn all the same: what is shown is
/// view data, what is cut is the set.
struct SetFaces {
    flat: Vec<(String, craft3d_doc::FeatureId, String)>,
    not_flat: Vec<Value>,
    /// Items of other kinds by kind, those counted (a kind too many to
    /// look through on a large model is left out, not refused).
    others: Map<String, Value>,
}

fn set_faces(session: &mut Session, name: &str) -> Result<SetFaces, CommandError> {
    use crate::select_cmds::{Parser, run_query};
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    if doc.set_named(name).is_none() {
        let sets: Vec<String> = doc
            .sets
            .iter()
            .take(20)
            .map(|s| format!("`{}`", s.name))
            .collect();
        return fail(format!(
            "{ID}: there is no set `{}`; {}",
            name.chars().take(80).collect::<String>(),
            if sets.is_empty() {
                "the document has no named sets: define one with set.define, such as {\"name\": \"Cut sheet\", \"kind\": \"face\", \"where\": ...}".to_string()
            } else {
                format!("the sets are {}", sets.join(", "))
            }
        ));
    }
    let run = |query: Value, limit: usize| {
        Parser::new(doc)
            .query(&query, 0)
            .and_then(|q| run_query(doc, &regen, q, limit, false))
            .map_err(|e| CommandError::new(format!("{ID}: the set `{name}`: {e}")))
    };
    // Counted kind by kind, each as far as it can be: only to say so.
    let mut others = Map::new();
    for kind in ["body", "edge", "vertex", "feature", "sketch", "datum"] {
        if let Ok((_, count)) = run(json!({ "kind": kind, "where": { "set": name } }), 0)
            && count > 0
        {
            others.insert(kind.into(), count.into());
        }
    }
    let other_count: u64 = others.values().filter_map(Value::as_u64).sum();
    let (faces, face_count) = run(
        json!({ "kind": "face", "where": { "set": name } }),
        MAX_SHEET_FACES.saturating_add(1),
    )?;
    if face_count > MAX_SHEET_FACES {
        return fail(format!(
            "{ID}: the set `{name}` holds {face_count} faces; a sheet takes at most {MAX_SHEET_FACES}"
        ));
    }
    let (planes, _) = run(
        json!({ "kind": "face", "where": [{ "set": name }, { "surface": "plane" }] }),
        MAX_SHEET_FACES,
    )?;
    let mut flat = Vec::with_capacity(planes.len());
    let mut not_flat = Vec::new();
    for hit in &faces {
        let item = &hit.item;
        let is_flat = planes
            .iter()
            .any(|h| h.item.name == item.name && h.item.feature == item.feature);
        let Some(id) = crate::selection::feature_named(doc, &item.feature) else {
            continue;
        };
        if is_flat {
            let body = doc
                .feature(id)
                .map_or_else(|| item.feature.clone(), |f| f.name.clone());
            flat.push((item.name.clone(), id, body));
        } else {
            not_flat.push(json!({ "face": item.name, "feature": item.feature }));
        }
    }
    if flat.is_empty() {
        return fail(format!(
            "{ID}: the set `{name}` holds no flat face to draw ({face_count} faces, {} of them not flat, and {} other items); a sheet is of flat faces",
            not_flat.len(),
            other_count
        ));
    }
    Ok(SetFaces {
        flat,
        not_flat,
        others,
    })
}

/// Several flat faces on one sheet: each drawn in its own frame, as a
/// layer of its own, and set side by side in rows by their bounding
/// boxes, `gap` apart, in the order given; or every flat face a named
/// set holds now (`set`), the faces of one body on one layer named after
/// it. Faces given the same layer share it.
fn sheet(session: &mut Session, p: &Params) -> Result<Source, CommandError> {
    let set_name = match p.get("set") {
        None => None,
        Some(Value::String(name)) => Some(name.clone()),
        Some(other) => {
            return fail(format!(
                "{ID}: `set` must be the name of a named set of faces (set.list), got {other}"
            ));
        }
    };
    let from_set = match &set_name {
        Some(name) => Some(set_faces(session, name)?),
        None => None,
    };
    let no_entries: Vec<Value> = Vec::new();
    let entries = match (&from_set, p.get("faces")) {
        (Some(_), _) => &no_entries,
        (None, Some(Value::Array(entries))) => entries,
        (None, _) => {
            return fail(format!(
                "{ID}: `faces` must be a list of face names (or of {{\"face\", \"feature\", \"xAxis\", \"layer\"}} objects); a named set is given as `set`"
            ));
        }
    };
    if from_set.is_none() && entries.is_empty() {
        return fail(format!("{ID}: `faces` is empty: name at least one face"));
    }
    if entries.len() > MAX_SHEET_FACES {
        return fail(format!(
            "{ID}: `faces` has {} entries; a sheet takes at most {MAX_SHEET_FACES}",
            entries.len()
        ));
    }
    let unit = session.document()?.history.current().units.length;
    // Lengths as every other: an expression ("3 mm", "wall * 2") or a
    // number in the document's unit.
    let length = |key: &str| -> Result<Option<f64>, CommandError> {
        match p.expr(key)? {
            None => Ok(None),
            Some(source) => session
                .evaluate_typed(&source, crate::typed::TypedKind::Length)
                .map(Some)
                .map_err(|e| CommandError::new(format!("{ID}: `{key}`: {e}"))),
        }
    };
    let gap = match length("gap")? {
        Some(gap) if gap >= 0.0 => gap,
        Some(_) => return fail(format!("{ID}: `gap` must not be negative")),
        // 5 mm in the document's unit.
        None => 5.0 / drawing_unit(unit).millimetres(),
    };
    let width = match length("width")? {
        Some(width) if width > 0.0 => Some(width),
        Some(_) => return fail(format!("{ID}: `width` must be positive")),
        None => None,
    };
    let x_axis = vector(p.get("xAxis"), "xAxis")?;
    // Each entry: the face, whose body, its axes, its layer, and what a
    // layer is named after when it is not given.
    let mut wanted = Vec::with_capacity(entries.len());
    let mut layers: Vec<Option<String>> = Vec::with_capacity(entries.len());
    let mut bases: Vec<&str> = Vec::with_capacity(entries.len());
    // A set's faces: a layer per body, named after it.
    let mut body_layers: Vec<(craft3d_doc::FeatureId, String)> = Vec::new();
    for (index, (face, id, body)) in from_set.iter().flat_map(|s| s.flat.iter()).enumerate() {
        wanted.push(Wanted {
            given: face,
            only: Some(*id),
            axes: axes(x_axis),
            role: format!("`set` face {index}"),
        });
        let layer = match body_layers.iter().find(|(b, _)| b == id) {
            Some((_, layer)) => layer.clone(),
            None => {
                let taken: Vec<String> = body_layers.iter().map(|(_, l)| l.clone()).collect();
                let layer = layer_name(body, &taken);
                body_layers.push((*id, layer.clone()));
                layer
            }
        };
        layers.push(Some(layer));
        bases.push(body);
    }
    for (index, entry) in entries.iter().enumerate() {
        let role = format!("`faces.{index}`");
        match entry {
            Value::String(given) => {
                wanted.push(Wanted {
                    given,
                    only: None,
                    axes: axes(x_axis),
                    role,
                });
                layers.push(None);
                bases.push(given);
            }
            Value::Object(_) => {
                let e = Params::new(ID, entry, &["face", "feature", "xAxis", "layer"])?;
                let only = if e.has("feature") {
                    Some(feature_ref(
                        session.document()?.history.current(),
                        &e,
                        ID,
                        "feature",
                    )?)
                } else {
                    None
                };
                let own = vector(e.get("xAxis"), &format!("faces.{index}.xAxis"))?;
                let given = e
                    .str("face")?
                    .ok_or_else(|| CommandError::new(format!("{ID}: {role} has no `face`")))?;
                wanted.push(Wanted {
                    given,
                    only,
                    axes: axes(own.or(x_axis)),
                    role,
                });
                bases.push(given);
                layers.push(e.str("layer")?.map(str::to_string));
            }
            other => {
                return fail(format!(
                    "{ID}: {role} must be a face name or an object with `face`, got {other}"
                ));
            }
        }
    }
    // Names given first, so that a derived one keeps clear of them.
    let mut taken: Vec<String> = layers.iter().flatten().cloned().collect();
    let mut placed = Vec::with_capacity(wanted.len());
    let mut sheet_json = Vec::with_capacity(wanted.len());
    let (mut x, mut y, mut row_height, mut rows) = (0.0_f64, 0.0_f64, 0.0_f64, 0_usize);
    let mut in_row = 0_usize;
    let slack = session.document()?.history.current().tolerance.linear;
    for ((want, layer), base) in wanted.iter().zip(layers).zip(bases) {
        let drawn = draw(session, want)?;
        let [x0, y0, x1, y1] = bbox(&drawn.paths)?;
        let (w, h) = (x1 - x0, y1 - y0);
        if let Some(width) = width {
            if w > width + slack {
                return fail(format!(
                    "{ID}: `{}`: the face {} is {w} wide, more than the sheet's width {width}",
                    want.role, want.given
                ));
            }
            // The next row when this face would run past the sheet.
            if in_row > 0 && x + w > width + slack {
                y += row_height + gap;
                (x, row_height, in_row) = (0.0, 0.0, 0);
            }
        }
        if in_row == 0 {
            rows += 1;
        }
        // A sheet so large that a coordinate on it cannot be held to the
        // document's tolerance would not cut what was drawn.
        let reach = (x + w).max(y + h);
        if !(reach.is_finite() && reach * f64::EPSILON <= slack) {
            return fail(format!(
                "{ID}: the sheet would reach {reach}, where a coordinate cannot be held to the document's tolerance {slack}; use a smaller `gap`"
            ));
        }
        let at = [x - x0, y - y0];
        let paths = moved(drawn.paths, at);
        let layer = match layer {
            Some(layer) => layer,
            None => {
                let name = layer_name(base, &taken);
                taken.push(name.clone());
                name
            }
        };
        let mut json = drawn.json;
        json.insert("layer".into(), layer.clone().into());
        // Where the face's frame origin lies on the sheet, and the box
        // its outline takes there.
        json.insert("at".into(), json!(at));
        json.insert("bbox".into(), json!([x, y, x + w, y + h]));
        sheet_json.push(Value::Object(json));
        // Faces on one layer (a body's, in a set) share it.
        match placed.iter_mut().find(|l: &&mut Layer| l.name == layer) {
            Some(shared) => shared.paths.extend(paths),
            None => placed.push(Layer {
                name: layer,
                color: Color::Black,
                dash: None,
                dot: false,
                paths,
            }),
        }
        x += w + gap;
        row_height = row_height.max(h);
        in_row += 1;
    }
    Ok(Source {
        json: {
            let mut json = json!({
                "sheet": sheet_json,
                "rows": rows,
                "gap": gap,
            });
            if let (Some(map), Some(name), Some(set)) =
                (json.as_object_mut(), set_name, from_set.as_ref())
            {
                map.insert("set".into(), name.into());
                map.insert("notFlat".into(), set.not_flat.clone().into());
                map.insert("others".into(), Value::Object(set.others.clone()));
            }
            json
        },
        layers: placed,
        paths: Vec::new(),
        construction: Vec::new(),
    })
}

/// The SVG's stroke width (`stroke`, in millimetres, whatever the
/// document's unit): `None` for the laser's hairline. Refused for DXF,
/// which has no stroke width, and when not positive or over 100 mm.
pub(crate) fn stroke(p: &Params, format: DrawingFormat) -> Result<Option<f64>, CommandError> {
    let Some(width) = p.number("stroke")? else {
        return Ok(None);
    };
    if format == DrawingFormat::Dxf {
        return fail(format!(
            "{ID}: `stroke` is the width of an SVG's lines; a DXF has none"
        ));
    }
    if !(width.is_finite() && width > 0.0 && width <= 100.0) {
        return fail(format!(
            "{ID}: `stroke` is a line width in millimetres, above 0 and at most 100 (0.01 is a laser's hairline, 0.3 prints well)"
        ));
    }
    Ok(Some(width))
}
