//! The DXF and SVG side of `file.export` (the command itself is in
//! `commands.rs`): the outline of a sketch or of a flat face, for laser
//! cutting and for drawings.

use crate::commands::{Run, fail, feature_ref};
use crate::params::Params;
use crate::{CommandError, Session};
use craft3d_doc::{LengthUnit, PersistentName};
use craft3d_drawing::{Color, Drawing, Layer, Path as DrawingPath, Unit};
use craft3d_io::DrawingFormat;
use craft3d_regen::Status;
use serde_json::{Value, json};
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

/// What the paths were taken from, for the result.
struct Source {
    json: Value,
    paths: Vec<DrawingPath>,
    construction: Vec<DrawingPath>,
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
    let source = match (p.has("sketch"), p.has("face")) {
        (true, true) => {
            return fail(format!(
                "{ID}: give either a `sketch` or a `face`, not both"
            ));
        }
        (false, false) => {
            return fail(format!(
                "{ID}: a .{} file is the outline of a sketch or of a flat face: give `sketch` (its id or name) or `face` (a name from document.inspect, faceNames)",
                format.name()
            ));
        }
        (true, false) => {
            if p.has("feature") {
                return fail(format!(
                    "{ID}: `feature` says whose face is meant; it does not go with `sketch`"
                ));
            }
            sketch(session, p)?
        }
        (false, true) => {
            if p.bool("construction")?.is_some() {
                return fail(format!(
                    "{ID}: `construction` is about a sketch; a face has no construction geometry"
                ));
            }
            face(session, p)?
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
    let mut layers = Vec::new();
    for (name, color, paths) in [
        ("outline", Color::Black, source.paths),
        ("construction", Color::Blue, source.construction),
    ] {
        if !paths.is_empty() {
            layers.push(Layer {
                name: name.to_string(),
                color,
                paths,
            });
        }
    }
    let drawing = Drawing {
        unit: match unit {
            LengthUnit::Micrometre => Unit::Micron,
            LengthUnit::Millimetre => Unit::Millimeter,
            LengthUnit::Centimetre => Unit::Centimeter,
            LengthUnit::Metre => Unit::Meter,
            LengthUnit::Inch => Unit::Inch,
            LengthUnit::Foot => Unit::Foot,
        },
        layers,
    };
    let (bytes, report) = craft3d_io::encode_drawing(&drawing, format, tolerance)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    crate::workspace::write_atomic(resolved, &bytes)?;
    let counts = drawing.counts();
    let mut result = json!({
        "path": path,
        "format": format.name(),
        "bytes": bytes.len(),
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
        // [min x, min y, max x, max y] in the axes of the sketch or face.
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
    let (drawn, placed) = with_instances(sketch, solved)
        .map_err(|e| CommandError::new(format!("{ID}: the sketch `{name}`: {e}")))?;
    let outline = craft3d_io::sketch_outline(
        &drawn,
        &|entity| {
            solved
                .point(entity)
                .or_else(|| placed.get(&entity).copied())
        },
        &|entity| solved.radius(entity),
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
        paths: outline.paths,
        construction: outline.construction,
    })
}

/// Where the points [`with_instances`] adds lie.
type Placed = std::collections::BTreeMap<craft3d_doc::EntityId, (f64, f64)>;

/// `sketch` with the curves its patterns make added as entities of their
/// own (on points of their own, at the places the instances have), and
/// where those points are. A sketch without patterns is itself.
fn with_instances(
    sketch: &craft3d_doc::Sketch,
    solved: &craft3d_regen::SketchResult,
) -> Result<(craft3d_doc::Sketch, Placed), String> {
    let mut drawn = sketch.clone();
    let mut placed = Placed::new();
    if sketch.patterns.is_empty() {
        return Ok((drawn, placed));
    }
    let instances = craft3d_regen::pattern_instances(sketch, solved)?;
    add_curves(
        &mut drawn,
        &mut placed,
        instances
            .into_iter()
            .map(|(made, construction)| (made.curve, construction)),
    )?;
    Ok((drawn, placed))
}

/// Adds `curves` to `drawn` as entities on points of their own, and
/// records where those points are.
fn add_curves(
    drawn: &mut craft3d_doc::Sketch,
    placed: &mut Placed,
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
                for point in spline.control {
                    control.push(at(point)?);
                }
                let degree = u32::try_from(spline.degree).map_err(|e| e.to_string())?;
                drawn
                    .add_spline(degree, control, construction)
                    .map_err(text)?;
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
    add_curves(
        &mut drawn,
        &mut placed,
        made.outline.curves.iter().map(|c| (c.curve.clone(), false)),
    )
    .map_err(|e| CommandError::new(format!("{ID}: the offset `{name}`: {e}")))?;
    let outline = craft3d_io::sketch_outline(
        &drawn,
        &|entity| placed.get(&entity).copied(),
        // A circle has its radius in the sketch.
        &|_| None,
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
        paths: outline.paths,
        construction: outline.construction,
    })
}

fn face(session: &mut Session, p: &Params) -> Result<Source, CommandError> {
    let name: PersistentName = p
        .required_str("face")?
        .parse()
        .map_err(|reason| CommandError::new(format!("{ID}: `face`: {reason}")))?;
    if !name.is_face() {
        return fail(format!("{ID}: {name} is not a face"));
    }
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
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let called = |id| {
        doc.feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone())
    };
    // The bodies that have a face of that name: the one asked for, or
    // every body that exists.
    let mut found = Vec::new();
    for outcome in &regen.features {
        if only.is_some_and(|id| id != outcome.id) {
            continue;
        }
        let Some(body) = outcome.own_body() else {
            if only.is_some() {
                let made_by = called(outcome.id);
                return fail(match outcome.consumed_by {
                    Some(taker) => format!(
                        "{ID}: the body of `{made_by}` went into {taker}; ask for a face of that feature"
                    ),
                    None => format!("{ID}: `{made_by}` has no body"),
                });
            }
            continue;
        };
        let Some(names) = &outcome.names else {
            continue;
        };
        let faces = craft3d_regen::faces_named(names, &name);
        if !faces.is_empty() {
            found.push((outcome.id, body, faces));
        }
    }
    let (feature, body, faces) = match found.as_slice() {
        [] => {
            return fail(match only {
                Some(id) => format!(
                    "{ID}: `{}` has no face {name} (document.inspect lists its faceNames)",
                    called(id)
                ),
                None => format!(
                    "{ID}: no body has a face {name} (document.inspect lists the faceNames of every body)"
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
                "{ID}: the bodies of {} each have a face {name}; say which with `feature`",
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
    let outline = craft3d_io::face_outline(body, faces, tolerance)
        .map_err(|e| CommandError::new(format!("{ID}: the face {name}: {e}")))?;
    let frame = outline.frame;
    let vec3 = |v: craft3d_math::Vec3| json!([v.x, v.y, v.z]);
    Ok(Source {
        json: json!({
            "face": name.to_string(),
            "feature": feature.to_string(),
            "name": called(*feature),
            // How many faces answer to the name: the pieces of one face.
            "faces": faces.len(),
            // Where the drawing lies in the model: its origin and axes.
            // The face is seen from outside the body.
            "origin": vec3(frame.origin()),
            "xAxis": vec3(frame.x()),
            "yAxis": vec3(frame.y()),
        }),
        paths: outline.paths,
        construction: Vec::new(),
    })
}
