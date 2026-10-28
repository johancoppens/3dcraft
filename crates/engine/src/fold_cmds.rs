//! Origami: a crease pattern on the sheet a sketch bounds, folded rigidly
//! (`feature.fold`, `fold.edit`), what it folds to (`fold.inspect`), and
//! Kawasaki's and Maekawa's conditions at its vertices (`fold.check`).
//! The crease pattern is exported by `file.export` with `fold`
//! (`drawing_cmds.rs`). The design note is docs/design/origami.md.

use crate::commands::{
    Run, add_feature, edit, fail, feature_ref, feature_result, length_unit, spec,
};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    Crease, CreaseKind, CurveIndex, CurveRef, Document, EntityId, Expr, FeatureId, FeatureKind,
    Fold, Geometry, MAX_FOLD_CREASES,
};
use craft3d_kernel::origami::{EdgeKind, vertex_checks};
use serde_json::{Map, Value, json};

/// What every result of `fold.check` says about the two conditions.
pub(crate) const HONEST: &str = "Kawasaki (the sector angles at a vertex, taken alternately, add up to 180 degrees) and Maekawa (the numbers of mountains and valleys at a vertex differ by 2) are necessary for a vertex to fold flat, not sufficient: passing both at every vertex does not prove that the pattern folds flat (whether a crease pattern folds flat is NP-hard: Bern and Hayes 1996). A fold with every crease at 180 degrees is checked further: feature.fold then looks for an order of the layers, and says when there is none or when it could not decide.";

/// What `fold.inspect` says of the order of the layers of a flat fold.
pub(crate) const FLAT_NOTE: &str = "Folded flat, every crease by 180 degrees: facets are numbered as in facets; above lists every pair of facets that overlap as [upper, lower], seen from the sketch's front; facetLayers gives each facet's layer (0 has nothing below it). The order was found under the published rules (adjacent facets by their crease's kind, taco-taco, taco-tortilla, transitivity); a pattern that folds flat need not be reachable by rigid folding, so a fold slid towards 180 degrees may stop short of it.";

/// The layers of a flat fold drawn apart: each face of `mesh` (a body's,
/// face ids its faces) moved by its offset, and each edge by the mean of
/// its faces' offsets; a position that faces with different offsets share
/// is split. Display only: the body is not changed.
pub(crate) fn layered_mesh(
    mesh: &craft3d_tess::Mesh,
    edges: &crate::meshes::EdgePolylines,
    body: &craft3d_topo::Body,
    offsets: &[craft3d_math::Vec3],
) -> (craft3d_tess::Mesh, crate::meshes::EdgePolylines) {
    use craft3d_math::Vec3;
    use std::collections::BTreeMap;
    let offset = |face: u32| {
        usize::try_from(face)
            .ok()
            .and_then(|f| offsets.get(f))
            .copied()
            .unwrap_or(Vec3::ZERO)
    };
    let mut out = craft3d_tess::Mesh::default();
    let mut moved: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for (t, corners) in mesh.triangles.iter().enumerate() {
        let face = mesh.face_ids.get(t).copied().unwrap_or(0);
        let mut new = [0u32; 3];
        for (slot, &k) in new.iter_mut().zip(corners) {
            let index = match moved.get(&(k, face)) {
                Some(&i) => i,
                None => {
                    let Ok(i) = u32::try_from(out.positions.len()) else {
                        return (mesh.clone(), edges.clone());
                    };
                    let at = usize::try_from(k).ok();
                    let p = at
                        .and_then(|k| mesh.positions.get(k))
                        .copied()
                        .unwrap_or(Vec3::ZERO);
                    let n = at
                        .and_then(|k| mesh.normals.get(k))
                        .copied()
                        .unwrap_or(Vec3::ZERO);
                    out.positions.push(p + offset(face));
                    out.normals.push(n);
                    moved.insert((k, face), i);
                    i
                }
            };
            *slot = index;
        }
        out.triangles.push(new);
        out.face_ids.push(face);
    }
    // The faces of each edge.
    let mut faces_of: Vec<Vec<u32>> = vec![Vec::new(); body.edges.len()];
    for (f, face) in body.faces.iter().enumerate() {
        let Ok(f) = u32::try_from(f) else { continue };
        for l in face.loops.iter().filter_map(|l| body.loop_(*l)) {
            for c in &l.coedges {
                if let Some(list) = usize::try_from(c.edge.0)
                    .ok()
                    .and_then(|e| faces_of.get_mut(e))
                {
                    list.push(f);
                }
            }
        }
    }
    let lines = edges
        .iter()
        .enumerate()
        .map(|(e, line)| {
            let faces = faces_of.get(e).map(Vec::as_slice).unwrap_or(&[]);
            let mut shift = faces.iter().fold(Vec3::ZERO, |sum, &f| sum + offset(f));
            if !faces.is_empty() {
                shift = shift * (1.0 / f64::from(u32::try_from(faces.len()).unwrap_or(1)));
            }
            line.iter().map(|&p| p + shift).collect()
        })
        .collect();
    (out, lines)
}

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.fold", "Fold", r#"{"sketch": id or name, "creases": [{line: entity id or e7[2] or e7[*], kind: mountain|valley|cut, angle: expr = solved, alternate: boolean = false}], "fixed": [x, y] = the largest facet, "name": string = automatic, "when": expr = always on}"#,
        "Adds a fold: a sheet folded rigidly along a crease pattern. The closed profiles of the sketch's lines that are not creases bound the sheet (straight lines only; holes allowed); the creases are straight lines of the sketch (drawn or construction), each a mountain or a valley (seen from the side the sketch's normal points to, a valley folds towards the viewer) or a cut (the sheet is cut through there). A crease runs between the boundary, cuts and other creases; where creases cross or end on each other is a vertex, and the creases divide the sheet into facets. Each facet turns as a rigid body about the creases it shares with its neighbours: angle is a crease's fold angle, an expression from 0 (flat) to 180 deg (folded flat), every piece of the line folding by it (so a variable folds the sheet as it changes); a crease without an angle has each of its pieces solved from the others, by the condition that the facets round every closed loop of creases meet again (a degree-4 vertex has one degree of freedom: give one of its creases an angle). fixed is a point (sketch axes) inside the facet that stays in place; each other piece of a cut sheet keeps its first facet in place. Fails, with the reason, when the angles cannot all hold (not rigidly foldable at these angles), a solved crease would turn against its kind or past 180 degrees, the given angles leave some free (named; also with no angle given or every given angle 0, since the flat sheet does not decide which way solved creases go), the fold cannot be followed continuously from the flat sheet along its tangent, reaches a bifurcation partway (undetermined, the creases named), the solve does not converge or needs more work than its budget, or two facets pass through or lie on each other. With every given angle 180 deg the sheet is folded flat: every crease, the solved ones too, folds by 180 deg its kind's way, each facet placed by the reflections across the creases from the fixed one, and the layers are ordered (adjacent facets by their crease's kind, taco-taco, taco-tortilla, transitivity); it fails with the reason when the facets round a vertex do not close (Kawasaki), when no order exists (cannot fold flat with this mountain-valley assignment, the facets named), or when the search ran out of work (not decided: flat-foldability is NP-hard); a given 180 beside another angle is refused. The body lies in the sketch's plane; only the picture lifts the layers apart. A crease line copied by a sketch pattern gives creases of its copies: line e7[2] is one copy, e7[*] the line and every copy (alternate: kinds alternate along the copies, the given one on the line; a crease naming one copy wins over [*]); a copy left out is refused, never dropped. The result is a sheet of planar faces named as a fill's: a facet F7:startCap(e1,e5,e8) after the curves of its outer boundary, an edge F7:startEdge(e5) (piece numbers along the curve; a cut's two sides pieces 2i and 2i+1), a vertex F7:startVertex(e1,e5) after the curves that meet there. fold.inspect gives the solved angles, fold.check Kawasaki and Maekawa per vertex, file.export with fold the crease pattern as SVG or DXF; fold.edit changes the creases.", doc: true, feature_fold),
    spec!("fold.edit", "Edit Fold", r#"{"fold": id or name, "creases": [{line, kind, angle} as feature.fold], "set": [{line: entity id or e7[2] or e7[*], kind: mountain|valley|cut, angle: expr or null, alternate: boolean}], "remove": [entity id or e7[2] or e7[*]], "fixed": [x, y] or null}"#,
        "Changes a fold (one undo step): creases replaces all its creases; set changes the creases it names (kind and angle as given, angle null to solve it) and adds those it does not have (kind then needed); remove takes creases out (the lines stay in the sketch, as boundary if they close a profile); fixed moves the point of the facet that stays (null: the largest facet). Returns the fold as feature.fold does.", doc: true, fold_edit),
    spec!("fold.inspect", "Inspect Fold", r#"{"fold": id or name}"#,
        "What a fold folds to, without changing the document: facets (each with its area and the curves of its outer boundary), creases (each line with its kind, given angle and the solved angle of each of its pieces, in degrees, valleys positive), fixed (the facet that stays), gap (how far the loops of facets are from closing, a length), flat (folded flat, every crease at 180 deg) with layers when it is (facetLayers per facet, above the pairs of facets that overlap as [upper, lower] seen from the sketch's front, count, cyclic, and per facet layer and faceDown) and, when the fold fails, error with the reason. The crease pattern itself must divide the sheet; a pattern that cannot is an error.", doc: true, fold_inspect),
    spec!("fold.check", "Check Fold", r#"{"fold": id or name}"#,
        "Two small checks on a fold's crease pattern, at every interior vertex (not on the boundary; vertices on a cut are listed apart and not checked): Kawasaki, the sector angles taken alternately add up to 180 degrees each (sums, and off: how far the first sum is from 180, in degrees; an odd number of creases never holds), and Maekawa, the numbers of mountains and valleys differ by 2 (mountains, valleys, difference). Each vertex says where it is (sketch axes), its creases counter-clockwise and its sector angles; kawasakiFails and maekawaFails count the vertices that fail. These conditions are necessary for a vertex to fold flat, not sufficient: passing both at every vertex does NOT prove that the pattern folds flat (global flat-foldability is NP-hard, Bern and Hayes 1996), and the result says so (note). Read-only.", doc: true, fold_check),
];

/// A crease kind from its name.
fn kind_value(id: &str, value: Option<&Value>) -> Result<Option<CreaseKind>, CommandError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(name)) => CreaseKind::from_name(name).map(Some).ok_or_else(|| {
            CommandError::new(format!(
                "{id}: unknown crease kind `{}`; use mountain, valley or cut",
                name.chars().take(40).collect::<String>()
            ))
        }),
        Some(_) => fail(format!(
            "{id}: a crease's `kind` is mountain, valley or cut"
        )),
    }
}

/// One crease as given: the line and which copy of it, its kind (if
/// given), its angle (`None` not given, `Some(None)` given as null:
/// solved) and whether its kind alternates (if given).
type Given = (
    EntityId,
    CurveIndex,
    Option<CreaseKind>,
    Option<Option<Expr>>,
    Option<bool>,
);

/// A crease's line: an entity id (`e7`, or its number), or a copy a
/// sketch pattern made of it (`e7[2]`, `e7[1,3]`), or the line and every
/// copy (`e7[*]`).
fn line_ref(id: &str, key: &str, value: &Value) -> Result<(EntityId, CurveIndex), CommandError> {
    if let Value::String(text) = value
        && text.contains('[')
    {
        let found = CurveRef::parse(text.trim()).ok_or_else(|| {
            CommandError::new(format!(
                "{id}: `{key}` `{}` is no line: write e7, e7[2] (a copy a sketch pattern made), e7[1,3] (in a grid) or e7[*] (the line and every copy)",
                text.chars().take(40).collect::<String>()
            ))
        })?;
        return Ok((EntityId(found.entity), found.index));
    }
    Ok((
        crate::sketch_cmds::entity_id(id, key, value)?,
        CurveIndex::Seed,
    ))
}

fn crease_value(p: &Params, key: &str, value: &Value) -> Result<Given, CommandError> {
    let id = p.command();
    let Value::Object(map) = value else {
        return fail(format!(
            "{id}: each entry of `{key}` is an object {{\"line\": entity id, \"kind\": \"mountain|valley|cut\", \"angle\": expr}}"
        ));
    };
    if let Some(unknown) = map
        .keys()
        .find(|k| !["line", "kind", "angle", "alternate"].contains(&k.as_str()))
    {
        return fail(format!(
            "{id}: unknown key `{}` in a crease; a crease has line, kind, angle and alternate",
            unknown.chars().take(40).collect::<String>()
        ));
    }
    let line = map
        .get("line")
        .ok_or_else(|| CommandError::new(format!("{id}: a crease needs its `line`")))?;
    let (line, index) = line_ref(id, "line", line)?;
    let kind = kind_value(id, map.get("kind"))?;
    let angle = match map.get("angle") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => Some(Some(Expr::new(p.expr_of("angle", v)?))),
    };
    let alternate = match map.get("alternate") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return fail(format!("{id}: a crease's `alternate` is true or false")),
    };
    Ok((line, index, kind, angle, alternate))
}

fn crease_list(p: &Params, key: &str) -> Result<Option<Vec<Crease>>, CommandError> {
    let id = p.command();
    let Some(value) = p.get(key) else {
        return Ok(None);
    };
    let Value::Array(items) = value else {
        return fail(format!("{id}: `{key}` must be a list of creases"));
    };
    if items.len() > MAX_FOLD_CREASES {
        return fail(format!("{id}: more than {MAX_FOLD_CREASES} creases"));
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let (line, index, kind, angle, alternate) = crease_value(p, key, item)?;
        let shown = CurveRef {
            entity: line.0,
            index,
        };
        let kind = kind.ok_or_else(|| {
            CommandError::new(format!(
                "{id}: the crease {shown} needs its `kind` (mountain, valley or cut)"
            ))
        })?;
        out.push(Crease {
            line,
            kind,
            angle: angle.flatten(),
            index,
            alternate: alternate.unwrap_or(false),
        });
    }
    Ok(Some(out))
}

/// `fixed` as given: `None` not given, `Some(None)` null.
fn fixed_value(p: &Params) -> Result<Option<Option<[Expr; 2]>>, CommandError> {
    let id = p.command();
    if !p.has("fixed") {
        return Ok(None);
    }
    match p.get("fixed") {
        None => Ok(Some(None)),
        Some(Value::Array(items)) => match items.as_slice() {
            [x, y] => Ok(Some(Some([
                Expr::new(p.expr_of("fixed", x)?),
                Expr::new(p.expr_of("fixed", y)?),
            ]))),
            _ => fail(format!("{id}: `fixed` is a point [x, y] of the sketch")),
        },
        Some(_) => fail(format!("{id}: `fixed` is a point [x, y] of the sketch")),
    }
}

/// Refuses creases that are no lines of the sketch, are listed twice or
/// give a cut an angle.
fn check_creases(
    doc: &Document,
    id: &str,
    sketch: FeatureId,
    creases: &[Crease],
) -> Result<(), CommandError> {
    let feature = doc
        .feature(sketch)
        .ok_or_else(|| CommandError::new(format!("{id}: there is no feature {sketch}")))?;
    let FeatureKind::Sketch(drawn) = &feature.kind else {
        return fail(format!(
            "{id}: `{}` is a {}, not a sketch: a fold's creases are lines of a sketch",
            feature.name,
            feature.kind.type_name()
        ));
    };
    let mut seen = std::collections::BTreeSet::new();
    for c in creases {
        if !seen.insert((c.line, c.index)) {
            return fail(format!("{id}: the crease {} is listed twice", c.name()));
        }
        if c.alternate && c.index != CurveIndex::All {
            return fail(format!(
                "{id}: {} names one curve; alternate goes with the line and every copy of it, {}",
                c.name(),
                CurveRef {
                    entity: c.line.0,
                    index: CurveIndex::All
                }
            ));
        }
        if c.alternate && c.kind == CreaseKind::Cut {
            return fail(format!(
                "{id}: {} is a cut, which does not alternate",
                c.name()
            ));
        }
        let entity = drawn.entity(c.line).map_err(|_| {
            CommandError::new(format!(
                "{id}: the sketch `{}` has no {}",
                feature.name, c.line
            ))
        })?;
        if !matches!(entity.geometry, Geometry::Line { .. }) {
            return fail(format!(
                "{id}: {} is a {}, not a line: creases are straight lines",
                c.line,
                entity.geometry.type_name()
            ));
        }
        if c.kind == CreaseKind::Cut && c.angle.is_some() {
            return fail(format!(
                "{id}: {} is a cut, which has no fold angle",
                c.name()
            ));
        }
    }
    Ok(())
}

fn feature_fold(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.fold";
    let p = Params::new(ID, params, &["sketch", "creases", "fixed", "name"])?;
    let doc = session.document()?.history.current();
    let sketch = feature_ref(doc, &p, ID, "sketch")?;
    let creases = crease_list(&p, "creases")?.ok_or_else(|| p.missing("creases"))?;
    if creases.is_empty() {
        return fail(format!("{ID}: give at least one crease"));
    }
    check_creases(doc, ID, sketch, &creases)?;
    let fixed = fixed_value(&p)?.flatten();
    add_feature(
        session,
        ID,
        p.str("name")?,
        FeatureKind::Fold(Fold {
            sketch,
            creases,
            fixed,
        }),
    )
}

/// The fold `key` names.
fn fold_ref(session: &Session, p: &Params, key: &str) -> Result<(FeatureId, Fold), CommandError> {
    let id = p.command();
    let doc = session.document()?.history.current();
    let feature = feature_ref(doc, p, id, key)?;
    match doc.feature(feature).map(|f| (&f.kind, &f.name)) {
        Some((FeatureKind::Fold(fold), _)) => Ok((feature, fold.clone())),
        Some((other, name)) => fail(format!(
            "{id}: `{name}` is a {}, not a fold",
            other.type_name()
        )),
        None => fail(format!("{id}: there is no feature {feature}")),
    }
}

fn fold_edit(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "fold.edit";
    let p = Params::new(ID, params, &["fold", "creases", "set", "remove", "fixed"])?;
    let (id, mut fold) = fold_ref(session, &p, "fold")?;
    if !["creases", "set", "remove", "fixed"]
        .iter()
        .any(|k| p.has(k))
    {
        return fail(format!("{ID}: give creases, set, remove or fixed"));
    }
    if let Some(creases) = crease_list(&p, "creases")? {
        fold.creases = creases;
    }
    if let Some(value) = p.get("set") {
        let Value::Array(items) = value else {
            return fail(format!("{ID}: `set` must be a list of creases"));
        };
        for item in items {
            let (line, index, kind, angle, alternate) = crease_value(&p, "set", item)?;
            let shown = CurveRef {
                entity: line.0,
                index,
            };
            match fold
                .creases
                .iter_mut()
                .find(|c| c.line == line && c.index == index)
            {
                Some(c) => {
                    if let Some(kind) = kind {
                        c.kind = kind;
                    }
                    if let Some(alternate) = alternate {
                        c.alternate = alternate;
                    }
                    if let Some(angle) = angle {
                        c.angle = angle;
                    }
                    if c.kind == CreaseKind::Cut {
                        c.angle = None;
                    }
                }
                None => {
                    let kind = kind.ok_or_else(|| {
                        CommandError::new(format!(
                            "{ID}: {shown} is not a crease of the fold yet: give its `kind`"
                        ))
                    })?;
                    fold.creases.push(Crease {
                        line,
                        kind,
                        angle: angle.flatten(),
                        index,
                        alternate: alternate.unwrap_or(false),
                    });
                }
            }
        }
    }
    if let Some(value) = p.get("remove") {
        let Value::Array(items) = value else {
            return fail(format!("{ID}: `remove` must be a list of entity ids"));
        };
        for item in items {
            let (line, index) = line_ref(ID, "remove", item)?;
            let before = fold.creases.len();
            fold.creases
                .retain(|c| !(c.line == line && c.index == index));
            if fold.creases.len() == before {
                let shown = CurveRef {
                    entity: line.0,
                    index,
                };
                return fail(format!("{ID}: {shown} is not a crease of the fold"));
            }
        }
    }
    if fold.creases.len() > MAX_FOLD_CREASES {
        return fail(format!("{ID}: more than {MAX_FOLD_CREASES} creases"));
    }
    if fold.creases.is_empty() {
        return fail(format!("{ID}: a fold keeps at least one crease"));
    }
    if let Some(fixed) = fixed_value(&p)? {
        fold.fixed = fixed;
    }
    check_creases(
        session.document()?.history.current(),
        ID,
        fold.sketch,
        &fold.creases,
    )?;
    edit(session, ID, "Edit fold", |doc| {
        doc.feature_mut(id)?.kind = FeatureKind::Fold(fold);
        Ok(())
    })?;
    feature_result(session, id)
}

/// The crease pattern of the fold `fold` names, flat, for `file.export`:
/// the outline and each kind of crease on a layer of its own, styled as
/// crease patterns are drawn (Lang's and the usual convention: mountains
/// dash-dot, valleys dashed).
pub(crate) fn crease_pattern(
    session: &mut Session,
    p: &Params,
    command: &str,
) -> Result<crate::drawing_cmds::Source, CommandError> {
    use craft3d_drawing::{Color, Layer, Path, Segment};
    let (id, _) = fold_ref(session, p, "fold")?;
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let name = doc
        .feature(id)
        .map_or_else(|| id.to_string(), |f| f.name.clone());
    let analysis = craft3d_regen::fold_pattern(doc, &regen, id)
        .map_err(|e| CommandError::new(format!("{command}: the fold `{name}`: {e}")))?;
    let outline: Vec<Path> = analysis
        .outline
        .iter()
        .filter_map(|ring| {
            let (&start, rest) = ring.split_first()?;
            let mut segments: Vec<Segment> = rest.iter().map(|&to| Segment::Line { to }).collect();
            segments.push(Segment::Line { to: start });
            Some(Path::Chain {
                start,
                segments,
                closed: true,
            })
        })
        .collect();
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in analysis.outline.iter().flatten() {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let size = (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    let dash = if size.is_finite() && size > 0.0 {
        size / 60.0
    } else {
        1.0
    };
    let mut by_kind: [Vec<Path>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    // Every crease as the pattern resolved it: a line and each copy a
    // sketch pattern made of it, each with its own kind.
    for (c, k) in analysis.creases.iter().zip(&analysis.arrangement.creases) {
        let slot = match c.kind {
            CreaseKind::Mountain => 0,
            CreaseKind::Valley => 1,
            CreaseKind::Cut => 2,
        };
        if let Some(list) = by_kind.get_mut(slot) {
            list.push(Path::Chain {
                start: k.start,
                segments: vec![Segment::Line { to: k.end }],
                closed: false,
            });
        }
    }
    let [mountains, valleys, cuts] = by_kind;
    let counts = (mountains.len(), valleys.len(), cuts.len());
    let mut layers = vec![Layer {
        name: "boundary".into(),
        color: Color::Black,
        dash: None,
        dot: false,
        paths: outline,
    }];
    for (name, color, dash, dot, paths) in [
        ("mountain", Color::Red, Some(dash), true, mountains),
        ("valley", Color::Blue, Some(dash), false, valleys),
        ("cut", Color::Black, None, false, cuts),
    ] {
        if !paths.is_empty() {
            layers.push(Layer {
                name: name.into(),
                color,
                dash,
                dot,
                paths,
            });
        }
    }
    Ok(crate::drawing_cmds::Source {
        json: json!({
            "fold": id.to_string(),
            "name": name,
            "mountains": counts.0,
            "valleys": counts.1,
            "cuts": counts.2,
        }),
        layers,
        paths: Vec::new(),
        construction: Vec::new(),
    })
}

fn degrees(radians: f64) -> f64 {
    radians.to_degrees()
}

fn fold_inspect(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "fold.inspect";
    let p = Params::new(ID, params, &["fold"])?;
    let (id, fold) = fold_ref(session, &p, "fold")?;
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let analysis = craft3d_regen::fold_analysis(doc, &regen, id)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let arr = &analysis.pattern.arrangement;
    let unit = length_unit(doc);
    let layers = analysis.layers.as_ref();
    let names = &analysis.pattern.names;
    let facets: Vec<Value> = arr
        .facets
        .iter()
        .enumerate()
        .map(|(index, f)| {
            let mut curves: Vec<u64> = f
                .outer
                .iter()
                .filter_map(|&h| arr.edges.get(h / 2).map(|e| e.source))
                .collect();
            curves.sort_unstable();
            curves.dedup();
            let mut facet = json!({
                "area": f.area,
                "curves": curves.iter().map(|&c| names.name(c)).collect::<Vec<_>>(),
                "holes": f.holes.len(),
            });
            if let (Some(layers), Value::Object(map)) = (layers, &mut facet) {
                map.insert("layer".into(), json!(layers.layer.get(index)));
                map.insert("faceDown".into(), json!(layers.flipped.get(index)));
            }
            facet
        })
        .collect();
    let folded = analysis.folded.as_ref().ok();
    let creases: Vec<Value> = analysis
        .pattern
        .creases
        .iter()
        .zip(&arr.creases)
        .enumerate()
        .map(|(i, (c, k))| {
            let pieces: Vec<Value> = arr
                .hinges
                .iter()
                .enumerate()
                .filter(|(_, h)| h.crease == i)
                .map(|(n, h)| {
                    let piece = arr.edges.get(h.edge).map_or(0, |e| e.piece);
                    let angle = folded.and_then(|f| f.angles.get(n)).map(|a| degrees(*a));
                    json!({"piece": piece, "angle": angle})
                })
                .collect();
            let cut_pieces = arr
                .edges
                .iter()
                .filter(|e| e.source == k.id && matches!(e.kind, EdgeKind::Crease(_)))
                .count();
            let entry = fold.creases.get(c.entry).map(Crease::name);
            json!({
                "line": c.name,
                "kind": c.kind.name(),
                "angle": c.angle.as_ref().map(|a| a.source().to_string()),
                "entry": entry,
                "pieces": if c.kind == CreaseKind::Cut { json!(cut_pieces) } else { Value::Array(pieces) },
            })
        })
        .collect();
    let mut out = Map::new();
    out.insert("fold".into(), id.to_string().into());
    out.insert("facets".into(), facets.into());
    out.insert("creases".into(), creases.into());
    out.insert("fixed".into(), analysis.fixed.into());
    out.insert("unit".into(), unit.into());
    out.insert("anglesIn".into(), "deg".into());
    out.insert("flat".into(), layers.is_some().into());
    if let Some(layers) = layers {
        out.insert(
            "layers".into(),
            json!({
                "facetLayers": layers.layer,
                "above": layers.above,
                "count": layers.layer.iter().max().map_or(0, |l| l + 1),
                "cyclic": layers.cyclic,
                "cells": layers.cells,
                "stackedCells": layers.stacked,
                "note": FLAT_NOTE,
            }),
        );
    }
    match &analysis.folded {
        Ok(folded) => {
            out.insert("gap".into(), folded.gap.into());
            out.insert("ok".into(), true.into());
        }
        Err(reason) => {
            out.insert("ok".into(), false.into());
            out.insert("error".into(), reason.clone().into());
        }
    }
    Ok(Value::Object(out))
}

fn fold_check(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "fold.check";
    let p = Params::new(ID, params, &["fold"])?;
    let (id, _) = fold_ref(session, &p, "fold")?;
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let analysis = craft3d_regen::fold_pattern(doc, &regen, id)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let (checks, cuts) = vertex_checks(&analysis.arrangement, doc.tolerance.angular);
    let kawasaki_fails = checks.iter().filter(|c| !c.kawasaki_holds).count();
    let maekawa_fails = checks.iter().filter(|c| !c.maekawa_holds).count();
    let vertices: Vec<Value> = checks
        .iter()
        .map(|c| {
            json!({
                "at": c.at,
                "degree": c.creases.len(),
                "creases": c.creases.iter().map(|&e| analysis.names.name(e)).collect::<Vec<_>>(),
                "sectors": c.sectors.iter().map(|s| degrees(*s)).collect::<Vec<_>>(),
                "kawasaki": {
                    "holds": c.kawasaki_holds,
                    "sums": [degrees(c.sums[0]), degrees(c.sums[1])],
                    "off": c.kawasaki.map(degrees),
                },
                "maekawa": {
                    "holds": c.maekawa_holds,
                    "mountains": c.mountains,
                    "valleys": c.valleys,
                    "difference": c.maekawa,
                },
            })
        })
        .collect();
    let message = if checks.is_empty() {
        "no interior vertex to check".to_string()
    } else if kawasaki_fails == 0 && maekawa_fails == 0 {
        format!(
            "Kawasaki and Maekawa hold at all {} interior vertices; this is necessary for folding flat, not a proof that the pattern folds flat",
            checks.len()
        )
    } else {
        format!(
            "Kawasaki fails at {kawasaki_fails} and Maekawa at {maekawa_fails} of {} interior vertices: the pattern does not fold flat",
            checks.len()
        )
    };
    Ok(json!({
        "fold": id.to_string(),
        "vertices": vertices,
        "interior": checks.len(),
        "kawasakiFails": kawasaki_fails,
        "maekawaFails": maekawa_fails,
        "onCuts": cuts.iter().map(|(_, at)| json!(at)).collect::<Vec<_>>(),
        "anglesIn": "deg",
        "message": message,
        "note": HONEST,
    }))
}

/// The fold an assertion's `of` names, by name or id.
fn fold_named(
    ctx: &crate::verify_cmds::Ctx<'_>,
    p: &Map<String, Value>,
) -> Result<FeatureId, String> {
    let Some(Value::String(text)) = p.get("of") else {
        return Err("`of` is required: a fold (feature.fold) by name or id".to_string());
    };
    let shown: String = text.chars().take(80).collect();
    let id = ctx
        .doc
        .feature_by_name(text)
        .map(|f| f.id)
        .or_else(|| {
            text.strip_prefix('F')
                .and_then(|d| d.parse::<u64>().ok())
                .and_then(|n| ctx.doc.feature(FeatureId(n)))
                .map(|f| f.id)
        })
        .ok_or_else(|| format!("there is no feature named `{shown}`"))?;
    match ctx.doc.feature(id).map(|f| &f.kind) {
        Some(FeatureKind::Fold(_)) => Ok(id),
        Some(other) => Err(format!("`{shown}` is a {}, not a fold", other.type_name())),
        None => Err(format!("there is no feature named `{shown}`")),
    }
}

/// The assertion measures of a fold: `kawasakiFails` and `maekawaFails`
/// (how many interior vertices fail each condition, as `fold.check`
/// counts them) and `flatFoldable` (whether the pattern folds flat with
/// its kinds: every crease by 180 degrees, and an order of the layers).
pub(crate) fn take(
    ctx: &crate::verify_cmds::Ctx<'_>,
    name: &str,
    p: &Map<String, Value>,
) -> Result<crate::verify_cmds::Outcome, String> {
    use crate::verify_cmds::{Dim, Outcome};
    use craft3d_kernel::origami::{Angle, FoldError, fold_flat, layer_order};
    crate::verify_cmds::only(p, &["of"])?;
    let id = fold_named(ctx, p)?;
    let fname = ctx.feature_name(id);
    let pattern = craft3d_regen::fold_pattern(ctx.doc, ctx.regen, id)
        .map_err(|e| format!("`{fname}`: {e}"))?;
    let arr = &pattern.arrangement;
    if name == "flatFoldable" {
        let angles: Vec<Angle> = arr
            .hinges
            .iter()
            .map(|h| {
                let sign = arr.creases.get(h.crease).map_or(1.0, |c| c.kind.sign());
                Angle::Given(sign * std::f64::consts::PI)
            })
            .collect();
        let fixed = arr
            .facets
            .iter()
            .enumerate()
            .fold((0usize, f64::NEG_INFINITY), |best, (i, f)| {
                if f.area > best.1 { (i, f.area) } else { best }
            })
            .0;
        let linear = pattern.tolerance.linear;
        // Built flat already: its layers as regeneration found them.
        let built = ctx.regen.feature(id).and_then(|r| match &r.developed {
            Some(craft3d_regen::Developed::FlatFold(flat)) => Some(flat.layers.clone()),
            _ => None,
        });
        let found = match built {
            Some(layers) => Ok(layers),
            None => fold_flat(arr, &angles, fixed, linear)
                .and_then(|folded| layer_order(arr, &folded, linear)),
        };
        let mut out = match found {
            Ok(layers) => {
                let mut out = Outcome::flag(true);
                out.detail.insert(
                    "layers".into(),
                    json!(layers.layer.iter().max().map_or(0, |l| l + 1)),
                );
                out
            }
            Err(e @ (FoldError::NotFlat { .. } | FoldError::NoLayers { .. })) => {
                let mut out = Outcome::flag(false);
                out.note = Some(e.to_string());
                out
            }
            Err(e) => {
                return Err(format!(
                    "`{fname}`: whether it folds flat could not be decided: {e}"
                ));
            }
        };
        out.detail.insert("note".into(), FLAT_NOTE.into());
        return Ok(out);
    }
    let (checks, _) = vertex_checks(arr, ctx.doc.tolerance.angular);
    let failing: Vec<Value> = checks
        .iter()
        .filter(|c| {
            if name == "kawasakiFails" {
                !c.kawasaki_holds
            } else {
                !c.maekawa_holds
            }
        })
        .take(200)
        .map(|c| json!({"at": c.at, "creases": c.creases.iter().map(|&e| pattern.names.name(e)).collect::<Vec<_>>()}))
        .collect();
    let count = checks
        .iter()
        .filter(|c| {
            if name == "kawasakiFails" {
                !c.kawasaki_holds
            } else {
                !c.maekawa_holds
            }
        })
        .count();
    let mut out = Outcome::number(
        Dim::Count,
        f64::from(u32::try_from(count).unwrap_or(u32::MAX)),
    );
    out.detail.insert("interior".into(), checks.len().into());
    out.detail.insert("failing".into(), failing.into());
    out.detail.insert("note".into(), HONEST.into());
    Ok(out)
}
