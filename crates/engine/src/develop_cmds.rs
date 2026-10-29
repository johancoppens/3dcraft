//! Developable surfaces and unfolding (docs/design/developable.md): the
//! developable sheet between two curves, a pattern laid flat from faces,
//! the map between a pattern and its part, and how far faces are from
//! developable (also as the assertion measures `curvature` and
//! `developable`).

use crate::commands::{Run, add_feature, exprs, fail, feature_ref, origin, spec};
use crate::params::Params;
use crate::verify_cmds::{Ctx, Dim, Outcome};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    Developable, Document, Expr, FeatureId, FeatureKind, MAX_RAIL_EDGES, PersistentName, Rail,
    Unfold,
};
use craft3d_math::Vec3;
use serde_json::{Map, Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.developable", "Developable", r#"{"a": sketch (id or name) or an object of edges: [edge name, ...], "b": the same, "name": string = automatic, "when": expr = always on}"#,
        "Adds the developable sheet between two curves: a ruled surface whose rulings are chosen so that a flat sheet (plywood, sheet metal, paper) bends onto it without stretching (its Gaussian curvature is zero). Each curve is a sketch whose curves make one open chain or one closed loop (lines, arcs, splines, curves from formulas, end to end), or edges of earlier bodies in order, end to end ({\"edges\": [...]}, as document.inspect names them: the chines of a hull loft). Both are run from their first end to their last. A ruling joins a point of a to a point of b where the tangents of both curves and the ruling lie in one plane; the rulings are followed from the first ends to the last, and where the ruling from an end does not reach the other curve's end a fan (a cone from that end) fills the corner. Where the rulings turn back a fan closes the gap, and a corner of a curve is turned by a fan from it (a crease where both curves turn on one ruling); two closed curves give a tube. The faces, named by place (only where made): F9:face(0) the first run of rulings, F9:face(1) the fan at the start, F9:face(2) the fan at the end, F9:face(10000000000 + n) the n-th fan where the rulings turn back in the first half counted from the start, F9:face(20000000000 + n) in the second half counted from the end, F9:face(30000000000 + c) a fan from a corner of a before its curve c (40000000000 + c for b), F9:face(100000000000 + m) a run of rulings after the fan or corner m, F9:face(200000000000 + m) a run after a fan counted from the end, named by the fan m after it, F9:face(300000000000) the run that reaches the end; their edges along a and b are the curves themselves, so two developables on one curve meet edge to edge. Curves in one plane give the flat region between them. Refused, with where: rulings that would cross (a curve that winds about the other), curves that lie in one plane over a stretch only (the developable is not unique there), curves that meet between their ends, a curve that turns back on itself (a cusp), one closed curve with an open one, two closed curves that are the same or that lie in one plane and touch, cross or lie side by side, and curves with no ruling from an end that satisfies the condition. feature.edit changes a and b.", doc: true, names: ["a", "b"], feature_developable),
    spec!("feature.unfold", "Unfold", r#"{"faces": [face name, ...], "at": [expr; 3] = [0, 0, 0], "angle": expr = 0 deg, "name": string = automatic, "when": expr = always on}"#,
        "Lays faces of one earlier body flat as one piece, a flat sheet of its own (the flat pattern to cut): lengths, angles and areas are kept. The faces must be developable (planes, cylinders, cones, developable sheets, any surface whose Gaussian curvature is zero within the tolerance) and connected across shared edges; a face that is not is refused with its curvature, faces whose shared edge bends differently on its two sides (two panels at a chine, a crease) are refused naming the edge (unfold them one by one). A band round a cylinder or cone is cut open along a straight line of it. The pattern lies in the plane z = at.z, the first face's first vertex at at, that vertex's first edge along +x turned by angle about +z; its face is F12:face(0) and its edges are the images of the faces' boundary edges. The faces' body stays. The result says how far the pattern is from the part: the largest relative change of an edge's length (lengthChange), of the area (areaChange), and the estimated error of the map (mapError). unfold.map maps points between the pattern and the part. feature.edit changes faces, at and angle.", doc: true, names: ["faces"], feature_unfold),
    spec!("unfold.map", "Map Through Unfold", r#"{"unfold": id or name, "points": [[x, y, z], ...], "to": "flat|part"}"#,
        "Maps points between an unfold's flat pattern and the part it was laid from: to flat, points on the part's faces (in the model's frame) go to the pattern where it lies in the document; to part, points on the pattern (z ignored, at most a tolerance off its plane in practice) go to the faces. Each result has the point, the face it is on and how far the given point was from that face (to flat) or that the point is on the pattern (to part). Marks and holes drawn on the pattern are found on the part this way, and the other way round. Read-only.", doc: true, unfold_map),
    spec!("measure.developability", "Developability", r#"{"of": subject = all bodies}"#,
        "How far faces are from developable (a flat sheet bending onto them without stretching): per face the largest |Gaussian curvature| found on a grid of 33 x 33 points of its surface over its region (in 1/length^2; a sampled largest, not a proven bound), where it is, and the stretch a flat sheet would need over the face, |K| r^3 / 6 with r half its diagonal; developable is whether that stretch is within half the document's tolerance (the share an unfold allows). curvature and developable are those of all the faces. Read-only. As assertions: measure curvature (eq, min, max in 1/length^2) and developable (is).", doc: true, names: ["of"], measure_developability),
];

/// A rail from a parameter: a sketch, or `{"edges": [...]}`.
fn rail_value(doc: &Document, p: &Params, command: &str, key: &str) -> Result<Rail, CommandError> {
    let value = p.get(key).ok_or_else(|| p.missing(key))?;
    if let Value::Object(map) = value {
        let Some(edges) = map.get("edges").and_then(Value::as_array) else {
            return fail(format!(
                "{command}: `{key}` must be a sketch (id or name) or {{\"edges\": [edge name, ...]}}"
            ));
        };
        if map.len() != 1 {
            return fail(format!(
                "{command}: `{key}` takes only `edges` when it is an object"
            ));
        }
        if edges.is_empty() || edges.len() > MAX_RAIL_EDGES {
            return fail(format!(
                "{command}: `{key}`: a rail has 1 to {MAX_RAIL_EDGES} edges, got {}",
                edges.len()
            ));
        }
        let mut names: Vec<PersistentName> = Vec::with_capacity(edges.len());
        for item in edges {
            let name = crate::datum_cmds::name_value(
                command,
                key,
                item,
                "an edge name such as \"F2:between(F2:side(e1),F2:side(e2))\"",
            )?;
            if !name.is_edge() {
                return fail(format!(
                    "{command}: `{key}`: {name} is not an edge (see edgeNames in document.inspect)"
                ));
            }
            if names.contains(&name) {
                return fail(format!("{command}: `{key}`: {name} is given twice"));
            }
            if doc.feature(name.feature).is_none() {
                return fail(format!(
                    "{command}: `{key}`: there is no feature {} for {name}",
                    name.feature
                ));
            }
            names.push(name);
        }
        return Ok(Rail::Edges(names));
    }
    let id = feature_ref(doc, p, command, key)?;
    match doc.feature(id).map(|f| &f.kind) {
        Some(FeatureKind::Sketch(_)) => Ok(Rail::Sketch(id)),
        Some(other) => fail(format!(
            "{command}: `{key}` is a {}, not a sketch; give a sketch or {{\"edges\": [...]}}",
            other.type_name()
        )),
        None => fail(format!("{command}: there is no feature {id}")),
    }
}

fn feature_developable(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.developable";
    let p = Params::new(ID, params, &["a", "b", "name"])?;
    let doc = session.document()?.history.current();
    let a = rail_value(doc, &p, ID, "a")?;
    let b = rail_value(doc, &p, ID, "b")?;
    if a == b {
        return fail(format!("{ID}: `a` and `b` are the same curve"));
    }
    add_feature(
        session,
        ID,
        p.str("name")?,
        FeatureKind::Developable(Developable { a, b }),
    )
}

/// `feature.edit` for a developable: `a`, `b`.
pub(crate) fn edit_developable(
    doc: &Document,
    p: &Params,
    sheet: &mut Developable,
) -> Result<(), CommandError> {
    if p.has("a") {
        sheet.a = rail_value(doc, p, "feature.edit", "a")?;
    }
    if p.has("b") {
        sheet.b = rail_value(doc, p, "feature.edit", "b")?;
    }
    Ok(())
}

fn feature_unfold(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.unfold";
    let p = Params::new(ID, params, &["faces", "at", "angle", "name"])?;
    session.document()?;
    let Some(value) = p.get("faces") else {
        return Err(p.missing("faces"));
    };
    let faces = crate::face_cmds::faces_value(ID, "faces", value, "unfold", false)?;
    if let Some(first) = faces.first()
        && let Some(other) = faces.iter().find(|f| f.feature != first.feature)
    {
        return fail(format!(
            "{ID}: `faces`: {other} and {first} are named after different features; the faces of an unfold are faces of one body"
        ));
    }
    let at = exprs(p.expr3("at")?.unwrap_or_else(origin));
    let angle = Expr::new(p.expr("angle")?.unwrap_or_else(|| "0 deg".to_string()));
    add_feature(
        session,
        ID,
        p.str("name")?,
        FeatureKind::Unfold(Unfold { faces, at, angle }),
    )
}

/// What a developable or an unfold reports in its feature's result.
pub(crate) fn report_json(map: &mut Map<String, Value>, made: &craft3d_regen::Developed) {
    match made {
        craft3d_regen::Developed::Sheet {
            reversed_b,
            rulings,
            deviation,
            unresolved,
            curvature,
        } => {
            map.insert(
                "developable".into(),
                json!({"reversedB": reversed_b, "rulings": rulings, "deviation": deviation, "unresolved": unresolved, "curvature": curvature}),
            );
        }
        craft3d_regen::Developed::SheetMetal(_) | craft3d_regen::Developed::FlatFold(_) => {}
        craft3d_regen::Developed::Pattern(dev) => {
            let e = dev.error();
            map.insert(
                "unfold".into(),
                json!({"lengthChange": e.length, "areaChange": e.area, "mapError": e.map}),
            );
        }
    }
}

fn point_of(value: &Value) -> Option<Vec3> {
    let items = value.as_array()?;
    let get = |k: usize| items.get(k).and_then(Value::as_f64);
    let p = Vec3::new(get(0)?, get(1)?, get(2).unwrap_or(0.0));
    (p.is_finite() && (items.len() == 2 || items.len() == 3)).then_some(p)
}

/// Most points one `unfold.map` maps.
const MAX_MAPPED: usize = 10_000;

fn unfold_map(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "unfold.map";
    let p = Params::new(ID, params, &["unfold", "points", "to"])?;
    let to = p.required_str("to")?;
    let to_flat = match to {
        "flat" => true,
        "part" => false,
        other => {
            return fail(format!(
                "{ID}: `to` is \"flat\" or \"part\", not \"{}\"",
                other.chars().take(40).collect::<String>()
            ));
        }
    };
    let Some(Value::Array(items)) = p.get("points") else {
        return fail(format!("{ID}: `points` must be a list of [x, y, z]"));
    };
    if items.is_empty() || items.len() > MAX_MAPPED {
        return fail(format!(
            "{ID}: `points`: 1 to {MAX_MAPPED} points, got {}",
            items.len()
        ));
    }
    let points: Vec<Vec3> = items
        .iter()
        .map(point_of)
        .collect::<Option<_>>()
        .ok_or_else(|| {
            CommandError::new(format!(
                "{ID}: `points`: each point is [x, y, z] of finite numbers"
            ))
        })?;
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let id: FeatureId = feature_ref(doc, &p, ID, "unfold")?;
    let Some(FeatureKind::Unfold(pattern)) = doc.feature(id).map(|f| &f.kind) else {
        return fail(format!("{ID}: `unfold` is not an unfold feature"));
    };
    let frame = craft3d_regen::placement(doc, pattern)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let Some(craft3d_regen::Developed::Pattern(dev)) =
        regen.feature(id).and_then(|r| r.developed.clone())
    else {
        return fail(format!(
            "{ID}: the unfold is not built (see its feature's error)"
        ));
    };
    let faces = dev.faces();
    let source = pattern
        .source()
        .and_then(|s| regen.feature(s))
        .and_then(|r| r.body.clone())
        .ok_or_else(|| CommandError::new(format!("{ID}: the faces' body is not there")))?;
    let mut out = Vec::with_capacity(points.len());
    for q in points {
        if to_flat {
            // The face of the set nearest the point.
            let mut best: Option<(f64, craft3d_topo::FaceId)> = None;
            for &face in &faces {
                let Some(f) = source.face(face) else { continue };
                let Ok((u, v)) = f.surface.project(q) else {
                    continue;
                };
                let d = f.surface.eval(u, v).distance(q);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, face));
                }
            }
            let Some((distance, face)) = best else {
                return fail(format!("{ID}: no face of the unfold to map onto"));
            };
            let flat = dev
                .to_flat(face, q)
                .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
            let at = frame.point(flat[0], flat[1], 0.0);
            out.push(json!({"point": [at.x, at.y, at.z], "face": face.0, "distance": distance}));
        } else {
            let (x, y, _) = frame.local(q);
            match dev.to_3d([x, y]) {
                Ok((face, at)) => out
                    .push(json!({"point": [at.x, at.y, at.z], "face": face.0, "onPattern": true})),
                Err(_) => out.push(json!({"point": Value::Null, "onPattern": false})),
            }
        }
    }
    let error = dev.error();
    Ok(json!({"points": out, "mapError": error.map, "lengthChange": error.length}))
}

fn measure_developability(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "measure.developability";
    Params::new(ID, params, &["of"])?;
    let map = match params {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let ctx = Ctx::new(doc, &regen).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let out = take(&ctx, "curvature", &map).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let mut result = out.detail;
    result.insert("curvature".into(), out.number.into());
    result.insert(
        "lengthUnit".into(),
        doc.units.length.symbol().to_string().into(),
    );
    Ok(Value::Object(result))
}

/// The measures `curvature` (the largest |K| of faces) and `developable`
/// (whether every face's stretch is within the tolerance).
pub(crate) fn take(ctx: &Ctx<'_>, name: &str, p: &Map<String, Value>) -> Result<Outcome, String> {
    crate::verify_cmds::only(p, &["of"])?;
    let found = crate::verify_cmds::faces_of_subject(ctx, p.get("of"))?;
    let tol = ctx.tolerance.linear;
    let mut largest: f64 = 0.0;
    let mut all = true;
    let mut listed = Vec::new();
    let mut proven = true;
    for (feature, body, faces, face_name) in &found {
        for &face in faces {
            let c = craft3d_kernel::unfold::face_curvature(body, craft3d_topo::FaceId(face as u32))
                .map_err(|e| format!("{face_name}: {e}"))?;
            largest = largest.max(c.largest);
            let ok = c.stretch <= craft3d_kernel::unfold::DEVELOPABLE_SHARE * tol;
            all &= ok;
            proven &= c.everywhere;
            if listed.len() < 200 {
                listed.push(json!({
                    "face": face_name,
                    "body": ctx.feature_name(*feature),
                    "curvature": c.largest,
                    "at": [c.at.x, c.at.y, c.at.z],
                    "stretch": c.stretch,
                    "developable": ok,
                }));
            }
        }
    }
    let mut out = if name == "developable" {
        Outcome::flag(all)
    } else {
        Outcome::number(Dim::Length(-2), largest)
    };
    out.detail.insert("faces".into(), listed.into());
    out.detail.insert("developable".into(), all.into());
    out.detail.insert("proven".into(), false.into());
    out.detail.insert("sampledEverywhere".into(), proven.into());
    Ok(out)
}
