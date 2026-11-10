//! `cut.list` and `cut.export`: the parts of a design that are cut from
//! sheet stock (docs/design/cutting.md). A body is a part when it is a
//! plate (a prism of constant thickness, recognised from its faces by
//! `craft3d_io::cut::plate_of`) or a sheet-metal part (its flat pattern).
//! The list groups the parts by thickness and material and counts the
//! identical ones; the export draws each part's outline and holes, with
//! an optional kerf, one file per part or side by side on one sheet.
//! Both only read the document.
//!
//! Optimisation (nesting parts on a sheet, a panel saw's order of cuts)
//! is not here: it is a fabrication module's work, on top of these.

use crate::commands::{Run, fail, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Document, FeatureId, FeatureKind, LengthUnit};
use craft3d_drawing::{Color, Drawing, Layer, Path as DrawingPath};
use craft3d_io::DrawingFormat;
use craft3d_io::cut::{self, Mark, MarkKind, Rect, Shape};
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_regen::Regen;
use craft3d_topo::FaceId;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!(
        "cut.list",
        "Cut List",
        r#"{"bodies": [id or name of the feature that made the body] = every body, "format": "json|csv" = "json", "geometry": boolean = false}"#,
        "The cut list of the document (changes nothing): every body that is cut from sheet stock, grouped by thickness and material. A body is a plate when it is a prism of constant thickness: two parallel flat faces facing away from each other (its top and bottom, one face each), every other face square to them (side walls, holes through: a plane square to them, a cylinder along their normal, a freeform wall square to them), every vertex in the top's plane or the bottom's; of the directions in which a body is such a prism the thinnest is its thickness (a box is a plate of its smallest side). The body of a sheet-metal part (feature.sheetMetal, also after a boolean drilled it) is a part by its flat pattern, as file.export with sheetMetal draws it (holes cut later carried back). Each part has name (of the feature that made the body), kind (plate or sheetMetal), thickness, material (body.material's name, or null), length and width (of the rectangle of least area around its outline, length the longer side), area (of the material: outline less holes), cutLength (outline, holes and cut marks), holes, count (how many bodies are this part: identical outline, holes and marks up to a rigid motion of the sheet, turned over included, where a bend up is then a bend down), bodies and features (their names and ids), frame (where the part's own 2D frame lies in the model: origin, xAxis along the length, yAxis, normal; its outline lies in [0, length] x [0, width] there), top (the face it is seen from; null for sheet metal, seen from its sketch's side) and notes. With geometry, each part also has geometry {outline, holes, marks} in its own frame: a path is {start, segments: [{line: [x, y]} | {arc: [x, y], center, radius, ccw} | {ellipse: [x, y], center, major, ratio, ccw} | {spline: {degree, knots, control, weights}}], closed}, {circle: [x, y], radius} or {ellipse: [x, y], major, ratio}; a mark is {kind: cut|bendUp|bendDown|form, path}. groups lists per thickness and material the parts, their count and total area; lengths are given to the decimal place of the document's tolerance and areas to that of its square, what the tolerance tells apart; notPlates lists every other body with the reason, named by face (a pocket's floor, a chamfer, a round, a face at a slant). With format csv the result is {format, csv, parts, distinct, notPlates}: csv has the header row material,thickness (u),name,count,length (u),width (u),area (u^2),cut length (u),holes,kind,bodies,reason in the document's unit u, a row per distinct part, then a row per body that is not a plate (kind not a plate, the reason in the last column). Not for an assembly: list the parts of its part documents.",
        doc: true,
        cut_list
    ),
    spec!(
        "cut.export",
        "Export Cut Parts",
        r#"{"path": string, "format": "dxf|svg" = from the extension, "bodies": [id or name of the feature that made the body] = every body, "layout": "sheet|each" = "sheet", "copies": boolean = false, "kerf": expr = 0, "gap": expr = 5 mm, "width": expr = no limit, "stroke": number = 0.01 (SVG line width in mm)}"#,
        "Writes the outlines of the parts cut.list finds to DXF or SVG under the write root (changes nothing in the document), at true size in the document's unit, each part in its own frame (its length along x), every curve as it is (lines, arcs, circles, ellipses and splines; nothing becomes a polyline). Each part is on a layer named after it (its body's feature; DXF layer, SVG path id layer-...): its outline, its holes and its cut marks; a sheet-metal part's bend lines go on bendUp (blue, dashed) and bendDown (red, dash-dot), a louvre's footprint on form. One part of each kind is drawn, or with copies every body; the result says how many of each to cut. layout sheet puts them on one file side by side, left to right in the order of cut.list, gap apart, in rows of at most width (a new row above the last); a row layout, not nesting. layout each writes one file per part, its name the path's with the layer's after a dash (parts.dxf: parts-Side.dxf). kerf (a length: the distance from the part's edge to the beam's centre, half the width the beam cuts away) offsets every outline out and every hole in by it, its corners rounded with arcs of radius kerf (the path of a round beam); lines stay lines and arcs arcs. Refused, naming the part: a kerf with which the beam cannot cut a line of the part (a slot or notch narrower than it: the largest kerf that fits is given; an inner round of a smaller radius than the kerf is not refused but comes out at the kerf's radius, as a sharp inner corner does, and the part's notes say so), one that closes a hole or splits a loop, and a kerf on a part with an ellipse or a spline (not built). Bodies that are not parts are not drawn and are listed under notPlates with the reason. The result has path (or files, for each), format, unit, layout, kerf, parts (name, layer, count, file, at: where the part's frame origin lies on the sheet, bbox, thickness, material, notes), rows, notPlates, length, bytes.",
        doc: true,
        cut_export
    ),
];

/// Most piece comparisons one list makes to find identical parts; past
/// them parts are counted apart and the result says which.
const MAX_WORK: usize = 5_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plate,
    SheetMetal,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Plate => "plate",
            Kind::SheetMetal => "sheetMetal",
        }
    }
}

/// A body that is a part, in its own frame.
struct Part {
    feature: FeatureId,
    name: String,
    kind: Kind,
    thickness: f64,
    material: Option<String>,
    shape: Shape,
    rect: Rect,
    /// The part's own frame in the model: origin, x along its length, y,
    /// normal.
    frame: [Vec3; 4],
    top: Option<String>,
    notes: Vec<String>,
}

/// A body that is not a part, and why.
struct Refused {
    feature: FeatureId,
    name: String,
    reason: String,
    faces: Vec<String>,
}

/// Parts that are the same part: indices into the parts.
struct Distinct {
    members: Vec<usize>,
}

struct Group {
    thickness: f64,
    material: Option<String>,
    distinct: Vec<Distinct>,
}

struct Found {
    parts: Vec<Part>,
    refused: Vec<Refused>,
    groups: Vec<Group>,
    /// Parts that could not be compared within the work allowed.
    uncompared: Vec<String>,
    unit: LengthUnit,
    tolerance: Tolerance,
}

impl Found {
    /// A length as the list prints it: to the decimal place of the
    /// document's tolerance, which tells nothing finer apart (a plate
    /// turned 40° is 3 thick, not 3.000000000000014).
    fn length(&self, v: f64) -> f64 {
        to_place(v, self.tolerance.linear)
    }

    /// An area as the list prints it: to the place of the tolerance
    /// squared, never coarser than what an area is told apart to.
    fn area(&self, v: f64) -> f64 {
        to_place(v, self.tolerance.linear * self.tolerance.linear)
    }
}

/// `v` rounded to the decimal place of `step` (the largest power of ten
/// not above it), so never coarser than `step`; `v` itself where that
/// cannot be done (a step that is not a positive number, a value whose
/// digits do not reach that place).
fn to_place(v: f64, step: f64) -> f64 {
    if !(step > 0.0 && step.is_finite() && v.is_finite()) {
        return v;
    }
    let place = step.plog10().floor();
    if !(-30.0..=30.0).contains(&place) {
        return v;
    }
    // `place` is a whole number within ±30.
    let place = place as i32;
    // Powers of ten up to 10^22 are exact, and dividing by one rounds the
    // quotient correctly: 3000000 / 10^6 is 3.
    let scale = 10.0_f64.ppowi(place.abs());
    let scaled = if place < 0 { v * scale } else { v / scale };
    if scaled.abs() >= 4.5e15 {
        return v;
    }
    let rounded = scaled.round();
    let back = if place < 0 {
        rounded / scale
    } else {
        rounded * scale
    };
    // -0 prints as 0.
    if back == 0.0 { 0.0 } else { back }
}

/// The frame in the model of the 2D frame `rect` puts in `frame`'s plane.
fn frame_of(frame: &Frame, rect: &Rect) -> [Vec3; 4] {
    let [c, s] = rect.x;
    let x = frame.x() * c + frame.y() * s;
    let y = frame.x() * -s + frame.y() * c;
    let [u, v] = rect.corner;
    let origin = frame.origin() + x * u + y * v;
    [origin, x, y, frame.z()]
}

fn vec3(v: Vec3) -> Value {
    // Adding 0 turns a negative zero into zero.
    json!([v.x + 0.0, v.y + 0.0, v.z + 0.0])
}

/// The flat pattern of the sheet-metal part `part` as a shape, its
/// thickness, the plane it is drawn in and what the pattern does not carry.
fn sheet_part(
    session: &mut Session,
    doc: &Document,
    regen: &Regen,
    part: FeatureId,
    command: &str,
) -> Result<(Shape, f64, Frame, Vec<String>), String> {
    let built = craft3d_regen::sheet_metal_part(doc, regen, part)?;
    let (thickness, plane) = (built.part.thickness, built.part.plane);
    let source =
        crate::sheetmetal_cmds::one_pattern(session, part, command).map_err(|e| e.to_string())?;
    let mut notes = Vec::new();
    if let Some(Value::Array(items)) = source.json.get("notCarried")
        && !items.is_empty()
    {
        notes.push(format!(
            "not on the flat pattern (a later feature's pocket or notch): {}",
            Value::Array(items.clone())
        ));
    }
    let mut loops = Vec::new();
    let mut marks = Vec::new();
    for layer in source.layers {
        let kind = match layer.name.as_str() {
            "outline" => None,
            "bendUp" => Some(MarkKind::BendUp),
            "bendDown" => Some(MarkKind::BendDown),
            "form" => Some(MarkKind::Form),
            other => {
                return Err(format!(
                    "the flat pattern has a layer {other} not known here"
                ));
            }
        };
        for path in layer.paths {
            match kind {
                None if path.is_closed() => loops.push(path),
                None => marks.push(Mark {
                    kind: MarkKind::Cut,
                    path,
                }),
                Some(kind) => marks.push(Mark { kind, path }),
            }
        }
    }
    let shape = Shape::from_loops(loops, marks)?;
    Ok((shape, thickness, plane, notes))
}

/// The parts and the bodies that are none, of the bodies `bodies` names.
fn found(session: &mut Session, p: &Params, command: &str) -> Result<Found, CommandError> {
    let open = session.document_mut()?;
    if open.history.current().is_assembly() {
        return fail(format!(
            "{command}: this is an assembly; open each of its part documents and list its parts there"
        ));
    }
    let regen = open.regen();
    let doc = Arc::clone(open.history.current());
    let only = crate::export_bodies::chosen_for(command, p, &doc, &regen)?;
    let bodies: Vec<_> = regen
        .bodies()
        .filter(|(id, _)| only.as_ref().is_none_or(|o| o.contains(id)))
        .map(|(id, body)| (id, Arc::clone(body)))
        .collect();
    // Which body each sheet-metal part ended in (a boolean that drilled it
    // took its body).
    let mut holds: BTreeMap<FeatureId, Vec<FeatureId>> = BTreeMap::new();
    for f in &doc.features {
        if !matches!(f.kind, FeatureKind::SheetMetal(_)) {
            continue;
        }
        let mut at = f.id;
        for _ in 0..=doc.features.len() {
            match regen.feature(at).and_then(|r| r.consumed_by) {
                Some(next) if next != at => at = next,
                _ => break,
            }
        }
        holds.entry(at).or_default().push(f.id);
    }
    let tolerance = Tolerance {
        linear: doc.tolerance.linear,
        angular: doc.tolerance.angular,
    };
    let mut parts = Vec::new();
    let mut refused = Vec::new();
    for (id, body) in bodies {
        let name = doc
            .feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone());
        let material = crate::appearance::body_material(&doc, &regen, id).map(|(_, m)| m.name);
        let names = regen.feature(id).and_then(|f| f.names.clone());
        let face_name = |face: FaceId| {
            names
                .as_ref()
                .and_then(|n| n.faces.get(face.0 as usize))
                .map_or_else(|| format!("{}", face.0), ToString::to_string)
        };
        let made = match holds.get(&id).map(Vec::as_slice) {
            Some([part]) => sheet_part(session, &doc, &regen, *part, command)
                .map(|(shape, t, plane, notes)| (Kind::SheetMetal, shape, t, plane, None, notes))
                .map_err(|reason| (reason, Vec::new())),
            _ => cut::plate_of(&body, tolerance, &face_name)
                .map(|plate| {
                    // A prism along a thicker axis than one it fails
                    // along: a part all the same, and the list says why
                    // it is not the thinner plate.
                    let notes = plate
                        .thinner
                        .iter()
                        .map(|(t, no)| {
                            format!(
                                "not a plate {t} thick across its thinnest side: {}",
                                no.reason
                            )
                        })
                        .collect();
                    (
                        Kind::Plate,
                        plate.shape,
                        plate.thickness,
                        plate.frame,
                        Some(face_name(plate.top)),
                        notes,
                    )
                })
                .map_err(|no| (no.reason, no.faces.iter().map(|f| face_name(*f)).collect())),
        };
        let own = made.and_then(|(kind, shape, t, frame, top, notes)| {
            let (shape, _, rect) = cut::own_frame(&shape).map_err(|e| (e, Vec::new()))?;
            Ok(Part {
                feature: id,
                name: name.clone(),
                kind,
                thickness: t,
                material: material.clone(),
                frame: frame_of(&frame, &rect),
                shape,
                rect,
                top,
                notes,
            })
        });
        match own {
            Ok(part) => parts.push(part),
            Err((reason, faces)) => refused.push(Refused {
                feature: id,
                name,
                reason,
                faces,
            }),
        }
    }
    let (groups, uncompared) = grouped(&parts, tolerance);
    Ok(Found {
        parts,
        refused,
        groups,
        uncompared,
        unit: doc.units.length,
        tolerance,
    })
}

/// The parts by thickness and material, the same parts together.
fn grouped(parts: &[Part], tolerance: Tolerance) -> (Vec<Group>, Vec<String>) {
    let prepared: Vec<cut::Prepared> = parts.iter().map(|p| cut::prepare(&p.shape)).collect();
    let mut groups: Vec<Group> = Vec::new();
    let mut uncompared = Vec::new();
    let mut work = MAX_WORK;
    for (i, part) in parts.iter().enumerate() {
        let near = 10.0 * tolerance.linear + 1e-9 * part.thickness.abs();
        let at = groups.iter().position(|g| {
            g.material == part.material && (g.thickness - part.thickness).abs() <= near
        });
        let group = match at.and_then(|at| groups.get_mut(at)) {
            Some(group) => group,
            None => {
                groups.push(Group {
                    thickness: part.thickness,
                    material: part.material.clone(),
                    distinct: Vec::new(),
                });
                match groups.last_mut() {
                    Some(group) => group,
                    None => continue,
                }
            }
        };
        let size = part.rect.length.phypot(part.rect.width);
        let tol = (16.0 * tolerance.linear).max(1e-9 * size);
        let mut joined = false;
        for d in &mut group.distinct {
            let Some((first, ready)) = d
                .members
                .first()
                .and_then(|&f| Some((parts.get(f)?, prepared.get(f)?)))
            else {
                continue;
            };
            let Some(mine) = prepared.get(i) else {
                continue;
            };
            if first.kind != part.kind {
                continue;
            }
            match cut::same_part(ready, mine, tol, &mut work) {
                Some(true) => {
                    d.members.push(i);
                    joined = true;
                    break;
                }
                Some(false) => {}
                None => {
                    uncompared.push(part.name.clone());
                    break;
                }
            }
        }
        if !joined {
            group.distinct.push(Distinct { members: vec![i] });
        }
    }
    groups.sort_by(|a, b| {
        a.thickness
            .total_cmp(&b.thickness)
            .then_with(|| a.material.cmp(&b.material))
    });
    uncompared.dedup();
    (groups, uncompared)
}

fn point(p: [f64; 2]) -> Value {
    json!([p[0] + 0.0, p[1] + 0.0])
}

fn segment_json(s: &craft3d_drawing::Segment) -> Value {
    use craft3d_drawing::Segment as S;
    match s {
        S::Line { to } => json!({"line": point(*to)}),
        S::Arc {
            to,
            center,
            radius,
            ccw,
        } => json!({"arc": point(*to), "center": point(*center), "radius": radius, "ccw": ccw}),
        S::EllipticArc {
            to,
            center,
            major,
            ratio,
            ccw,
        } => {
            json!({"ellipse": point(*to), "center": point(*center), "major": point(*major), "ratio": ratio, "ccw": ccw})
        }
        S::Spline(spline) => json!({"spline": {
            "degree": spline.degree(),
            "knots": spline.knots(),
            "control": spline.control_points().iter().map(|p| point(*p)).collect::<Vec<_>>(),
            "weights": spline.weights(),
        }}),
    }
}

fn path_json(path: &DrawingPath) -> Value {
    match path {
        DrawingPath::Chain {
            start,
            segments,
            closed,
        } => json!({
            "start": point(*start),
            "segments": segments.iter().map(segment_json).collect::<Vec<_>>(),
            "closed": closed,
        }),
        DrawingPath::Circle { center, radius } => {
            json!({"circle": point(*center), "radius": radius})
        }
        DrawingPath::Ellipse {
            center,
            major,
            ratio,
        } => json!({"ellipse": point(*center), "major": point(*major), "ratio": ratio}),
    }
}

fn part_json(found: &Found, d: &Distinct, geometry: bool) -> Option<Value> {
    let members: Vec<&Part> = d
        .members
        .iter()
        .filter_map(|&i| found.parts.get(i))
        .collect();
    let part = members.first()?;
    let [origin, x, y, normal] = part.frame;
    let mut json = json!({
        "name": part.name,
        "kind": part.kind.name(),
        "count": members.len(),
        "bodies": members.iter().map(|m| m.name.clone()).collect::<Vec<_>>(),
        "features": members.iter().map(|m| m.feature.to_string()).collect::<Vec<_>>(),
        "thickness": found.length(part.thickness),
        "material": part.material,
        "length": found.length(part.rect.length),
        "width": found.length(part.rect.width),
        "area": found.area(part.shape.area()),
        "cutLength": found.length(part.shape.cut_length()),
        "holes": part.shape.holes.len(),
        "frame": {"origin": vec3(origin), "xAxis": vec3(x), "yAxis": vec3(y), "normal": vec3(normal)},
        "top": part.top,
        "notes": part.notes,
    });
    if geometry && let Some(map) = json.as_object_mut() {
        let marks: Vec<Value> = part
            .shape
            .marks
            .iter()
            .map(|m| {
                let kind = match m.kind {
                    MarkKind::Cut => "cut",
                    MarkKind::BendUp => "bendUp",
                    MarkKind::BendDown => "bendDown",
                    MarkKind::Form => "form",
                };
                json!({"kind": kind, "path": path_json(&m.path)})
            })
            .collect();
        map.insert(
            "geometry".into(),
            json!({
                "outline": path_json(&part.shape.outline),
                "holes": part.shape.holes.iter().map(path_json).collect::<Vec<_>>(),
                "marks": marks,
            }),
        );
    }
    Some(json)
}

fn refused_json(r: &Refused) -> Value {
    json!({
        "name": r.name,
        "feature": r.feature.to_string(),
        "reason": r.reason,
        "faces": r.faces,
    })
}

/// A CSV field: quoted when it holds a comma, a quote or a line break
/// (RFC 4180).
/// A text that starts like a formula (`=`, `+`, `-`, `@`) gets a `'` in
/// front, so a spreadsheet shows it rather than runs it.
fn csv_field(text: &str) -> String {
    let text = if text.starts_with(['=', '+', '-', '@']) {
        format!("'{text}")
    } else {
        text.to_string()
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

fn csv(found: &Found) -> String {
    let u = found.unit.symbol();
    let mut out = format!(
        "material,thickness ({u}),name,count,length ({u}),width ({u}),area ({u}^2),cut length ({u}),holes,kind,bodies,reason\n"
    );
    for g in &found.groups {
        for d in &g.distinct {
            let members: Vec<&Part> = d
                .members
                .iter()
                .filter_map(|&i| found.parts.get(i))
                .collect();
            let Some(part) = members.first() else {
                continue;
            };
            let bodies: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
            let row = [
                csv_field(g.material.as_deref().unwrap_or("")),
                found.length(g.thickness).to_string(),
                csv_field(&part.name),
                members.len().to_string(),
                found.length(part.rect.length).to_string(),
                found.length(part.rect.width).to_string(),
                found.area(part.shape.area()).to_string(),
                found.length(part.shape.cut_length()).to_string(),
                part.shape.holes.len().to_string(),
                part.kind.name().to_string(),
                csv_field(&bodies.join("; ")),
                String::new(),
            ];
            out.push_str(&row.join(","));
            out.push('\n');
        }
    }
    for r in &found.refused {
        let row = [
            String::new(),
            String::new(),
            csv_field(&r.name),
            "1".into(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            "not a plate".into(),
            csv_field(&r.name),
            csv_field(&r.reason),
        ];
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

fn cut_list(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "cut.list";
    let p = Params::new(ID, params, &["bodies", "format", "geometry"])?;
    let csv_wanted = match p.str("format")? {
        None | Some("json") => false,
        Some("csv") => true,
        Some(other) => return fail(format!("{ID}: `format` must be json or csv, not `{other}`")),
    };
    let geometry = p.bool("geometry")?.unwrap_or(false);
    if csv_wanted && geometry {
        return fail(format!(
            "{ID}: `geometry` is for the JSON list; CSV has one row per part"
        ));
    }
    let found = found(session, &p, ID)?;
    let distinct: usize = found.groups.iter().map(|g| g.distinct.len()).sum();
    let not_plates: Vec<Value> = found.refused.iter().map(refused_json).collect();
    if csv_wanted {
        return Ok(json!({
            "format": "csv",
            "csv": csv(&found),
            "parts": found.parts.len(),
            "distinct": distinct,
            "notPlates": not_plates.len(),
        }));
    }
    let groups: Vec<Value> = found
        .groups
        .iter()
        .map(|g| {
            let parts: Vec<Value> = g
                .distinct
                .iter()
                .filter_map(|d| part_json(&found, d, geometry))
                .collect();
            let count: usize = g.distinct.iter().map(|d| d.members.len()).sum();
            let area: f64 = g
                .distinct
                .iter()
                .flat_map(|d| d.members.iter())
                .filter_map(|&i| found.parts.get(i))
                .map(|p| p.shape.area())
                .sum();
            json!({
                "thickness": found.length(g.thickness),
                "material": g.material,
                "count": count,
                "area": found.area(area),
                "parts": parts,
            })
        })
        .collect();
    Ok(json!({
        "unit": found.unit.symbol(),
        "parts": found.parts.len(),
        "distinct": distinct,
        "groups": groups,
        "notPlates": not_plates,
        "uncompared": found.uncompared,
    }))
}

fn cut_export(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "cut.export";
    let p = Params::new(
        ID,
        params,
        &[
            "path", "format", "bodies", "layout", "copies", "kerf", "gap", "width", "stroke",
        ],
    )?;
    let path = p.required_str("path")?;
    let format = match p.str("format")? {
        Some("dxf") => DrawingFormat::Dxf,
        Some("svg") => DrawingFormat::Svg,
        Some(other) => return fail(format!("{ID}: `format` must be dxf or svg, not `{other}`")),
        None => std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(DrawingFormat::from_extension)
            .ok_or_else(|| {
                CommandError::new(format!(
                    "{ID}: cannot tell the format from `{path}`; use a .dxf or .svg name, or pass `format`"
                ))
            })?,
    };
    let each = match p.str("layout")? {
        None | Some("sheet") => false,
        Some("each") => true,
        Some(other) => {
            return fail(format!(
                "{ID}: `layout` must be sheet or each, not `{other}`"
            ));
        }
    };
    let copies = p.bool("copies")?.unwrap_or(false);
    let stroke = match p.number("stroke")? {
        None => None,
        Some(_) if format == DrawingFormat::Dxf => {
            return fail(format!(
                "{ID}: `stroke` is the width of an SVG's lines and a DXF has none; leave it out or write .svg"
            ));
        }
        Some(w) if w.is_finite() && w > 0.0 && w <= 100.0 => Some(w),
        Some(w) => {
            return fail(format!(
                "{ID}: `stroke` must be a line width in millimetres above 0 and at most 100, not {w}"
            ));
        }
    };
    if session.document()?.history.current().is_assembly() {
        return fail(format!(
            "{ID}: this is an assembly; open each of its part documents and export its parts there"
        ));
    }
    let unit = session.document()?.history.current().units.length;
    let u = unit.symbol();
    let length = |key: &str| -> Result<Option<f64>, CommandError> {
        match p.expr(key)? {
            None => Ok(None),
            Some(source) => session
                .evaluate_typed(&source, crate::typed::TypedKind::Length)
                .map(Some)
                .map_err(|e| CommandError::new(format!("{ID}: `{key}`: {e}"))),
        }
    };
    let kerf = match length("kerf")? {
        None => 0.0,
        Some(k) if k.is_finite() && k >= 0.0 => k,
        Some(k) => {
            return fail(format!(
                "{ID}: `kerf` must be a length of 0 or more, not {}",
                crate::drawing_cmds::short_length(k, u)
            ));
        }
    };
    let (gap, width) = crate::drawing_cmds::sheet_gap_width(session, &p, ID)?;
    if each && (p.get("gap").is_some() || p.get("width").is_some()) {
        return fail(format!(
            "{ID}: `gap` and `width` lay parts out on one sheet; with layout each every part has a file of its own"
        ));
    }
    let found = found(session, &p, ID)?;
    if found.parts.is_empty() {
        let why: Vec<String> = found
            .refused
            .iter()
            .take(5)
            .map(|r| format!("`{}`: {}", r.name, r.reason))
            .collect();
        return fail(format!(
            "{ID}: no body is a part to cut{}",
            if why.is_empty() {
                " (the document has no bodies)".to_string()
            } else {
                format!(": {}", why.join("; "))
            }
        ));
    }
    // The parts to draw, each with how many to cut.
    let mut drawn: Vec<(&Part, usize)> = Vec::new();
    for g in &found.groups {
        for d in &g.distinct {
            let members: Vec<&Part> = d
                .members
                .iter()
                .filter_map(|&i| found.parts.get(i))
                .collect();
            if copies {
                drawn.extend(members.iter().map(|m| (*m, 1)));
            } else if let Some(first) = members.first() {
                drawn.push((first, members.len()));
            }
        }
    }
    let tolerance = found.tolerance;
    let mut taken: Vec<String> = ["outline", "bendUp", "bendDown", "form", "0"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    // Each part: its layer and its shape with the kerf.
    let mut shapes = Vec::with_capacity(drawn.len());
    for (part, count) in &drawn {
        let (shape, notes) = cut::kerf(&part.shape, kerf, &tolerance)
            .map_err(|e| CommandError::new(format!("{ID}: part `{}`: {e}", part.name)))?;
        let bbox = shape.bbox().ok_or_else(|| {
            CommandError::new(format!("{ID}: part `{}` has nothing to draw", part.name))
        })?;
        let layer = crate::drawing_cmds::layer_name(&part.name, &taken);
        taken.push(layer.clone());
        shapes.push((*part, *count, shape, bbox, layer, notes));
    }
    let slack = tolerance.linear;
    let mut placed: Vec<(usize, [f64; 2])> = Vec::with_capacity(shapes.len());
    let mut rows = 0_usize;
    if each {
        for (i, (part, _, _, b, _, _)) in shapes.iter().enumerate() {
            let reach = (b[2] - b[0]).max(b[3] - b[1]);
            if !(reach.is_finite() && reach * f64::EPSILON <= slack) {
                return fail(format!(
                    "{ID}: part `{}` reaches {reach} {u}, where a coordinate cannot be held to the document's tolerance {slack} {u}",
                    part.name
                ));
            }
            placed.push((i, [-b[0], -b[1]]));
        }
    } else {
        let (mut x, mut y, mut row_height, mut in_row) = (0.0_f64, 0.0_f64, 0.0_f64, 0_usize);
        for (i, (part, _, _, b, _, _)) in shapes.iter().enumerate() {
            let (w, h) = (b[2] - b[0], b[3] - b[1]);
            if let Some(width) = width {
                if w > width + slack {
                    return fail(format!(
                        "{ID}: part `{}` is {w} {u} long, more than the sheet's width {width} {u}; give a larger `width` or leave it out",
                        part.name
                    ));
                }
                if in_row > 0 && x + w > width + slack {
                    y += row_height + gap;
                    (x, row_height, in_row) = (0.0, 0.0, 0);
                }
            }
            if in_row == 0 {
                rows += 1;
            }
            let reach = (x + w).max(y + h);
            if !(reach.is_finite() && reach * f64::EPSILON <= slack) {
                return fail(format!(
                    "{ID}: the sheet would reach {reach} {u}, where a coordinate cannot be held to the document's tolerance {slack} {u}; use a smaller `gap`"
                ));
            }
            placed.push((i, [x - b[0], y - b[1]]));
            x += w + gap;
            row_height = row_height.max(h);
            in_row += 1;
        }
    }
    // The drawing of the parts `which`, each moved by its offset.
    let draw = |which: &[(usize, [f64; 2])]| -> Drawing {
        let mut layers = Vec::new();
        let mut marks: BTreeMap<MarkKind, Vec<DrawingPath>> = BTreeMap::new();
        let mut size = 0.0_f64;
        for (i, at) in which {
            let Some((_, _, shape, b, layer, _)) = shapes.get(*i) else {
                continue;
            };
            size = size.max((b[2] - b[0]).phypot(b[3] - b[1]));
            let mut paths: Vec<DrawingPath> = std::iter::once(&shape.outline)
                .chain(&shape.holes)
                .map(|p| p.translated(*at))
                .collect();
            for m in &shape.marks {
                if m.kind == MarkKind::Cut {
                    paths.push(m.path.translated(*at));
                } else {
                    marks
                        .entry(m.kind)
                        .or_default()
                        .push(m.path.translated(*at));
                }
            }
            layers.push(Layer {
                name: layer.clone(),
                color: Color::Black,
                dash: None,
                dot: false,
                paths,
            });
        }
        let dash = if size.is_finite() && size > 0.0 {
            size / 60.0
        } else {
            1.0
        };
        for (kind, paths) in marks {
            let (name, color, dot) = match kind {
                MarkKind::BendUp => ("bendUp", Color::Blue, false),
                MarkKind::BendDown => ("bendDown", Color::Red, true),
                MarkKind::Form | MarkKind::Cut => ("form", Color::Green, true),
            };
            layers.push(Layer {
                name: name.into(),
                color,
                dash: Some(dash),
                dot,
                paths,
            });
        }
        Drawing {
            unit: crate::drawing_cmds::drawing_unit(unit),
            layers,
        }
    };
    let mut files: Vec<(String, Drawing)> = Vec::new();
    if each {
        let base = std::path::Path::new(path);
        let stem = base
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("part")
            .to_string();
        let extension = format.name();
        for place in &placed {
            let Some((_, _, _, _, layer, _)) = shapes.get(place.0) else {
                continue;
            };
            let name = base.with_file_name(format!("{stem}-{layer}.{extension}"));
            let name = name.to_str().ok_or_else(|| {
                CommandError::new(format!("{ID}: `{path}` gives a file name that is not text"))
            })?;
            files.push((name.to_string(), draw(std::slice::from_ref(place))));
        }
    } else {
        files.push((path.to_string(), draw(&placed)));
    }
    let mut bytes_total = 0_usize;
    let mut length_total = 0.0;
    // Every file is made and every path resolved before any is written,
    // so a part that cannot be drawn or a path outside the root writes
    // nothing; a disk that fails half way can still leave the files
    // written before it (each one whole: written atomically).
    let mut encoded = Vec::with_capacity(files.len());
    for (name, drawing) in &files {
        let (bytes, _) =
            craft3d_io::encode_drawing_stroked(drawing, format, tolerance.linear, stroke)
                .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
        let resolved = session.workspace.resolve_write(name)?;
        bytes_total = bytes_total.saturating_add(bytes.len());
        length_total += drawing.length();
        encoded.push((name.clone(), resolved, bytes));
    }
    let mut written = Vec::with_capacity(encoded.len());
    for (name, resolved, bytes) in encoded {
        session.write_resolved(&resolved, bytes)?;
        written.push(name);
    }
    let parts: Vec<Value> = placed
        .iter()
        .filter_map(|(i, at)| {
            let (part, count, _, b, layer, notes) = shapes.get(*i)?;
            let file = if each {
                written.get(*i).cloned()
            } else {
                Some(path.to_string())
            };
            Some(json!({
                "name": part.name,
                "layer": layer,
                "count": count,
                "file": file,
                "at": point(*at),
                "bbox": [b[0] + at[0], b[1] + at[1], b[2] + at[0], b[3] + at[1]],
                "thickness": found.length(part.thickness),
                "material": part.material,
                "notes": notes,
            }))
        })
        .collect();
    let mut result: Map<String, Value> = Map::new();
    if each {
        result.insert("files".into(), json!(written));
    } else {
        result.insert("path".into(), path.into());
        result.insert("rows".into(), rows.into());
        result.insert("gap".into(), gap.into());
    }
    result.insert("format".into(), format.name().into());
    result.insert("unit".into(), unit.symbol().into());
    result.insert(
        "layout".into(),
        (if each { "each" } else { "sheet" }).into(),
    );
    result.insert("kerf".into(), kerf.into());
    result.insert("parts".into(), Value::Array(parts));
    result.insert(
        "notPlates".into(),
        Value::Array(found.refused.iter().map(refused_json).collect()),
    );
    result.insert("length".into(), length_total.into());
    result.insert("bytes".into(), bytes_total.into());
    Ok(Value::Object(result))
}
