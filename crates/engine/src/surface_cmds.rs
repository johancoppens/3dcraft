//! Surfaces through curve networks (docs/design/surfacing.md): a sheet
//! through u and v curves that cross (a Gordon surface), or through four
//! curves round a patch (a Coons patch), each edge free, held on a face or
//! tangent to one; and a sheet through a grid of points
//! (`feature.surface.grid`).

use crate::commands::{Run, add_feature, fail, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    Document, EdgeHold, Expr, FeatureKind, GridPoints, MAX_GRID_POINTS, MAX_GRID_SIDE,
    MAX_NETWORK_CURVES, NetworkEdges, Rail, SplineSpacing, SurfaceGrid, SurfaceNetwork,
};
use serde_json::{Map, Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.surface.network", "Network Surface", r#"{"u": [curve, ...], "v": [curve, ...], "edges": {uFirst|uLast|vFirst|vLast: {kind: free|position|tangent|curvature, face: face name, weight: expr = 1}} = all free, "acrossDegree": 3 or 5 = 3, "name": string = automatic, "when": expr = always on}"#,
        "Adds a sheet through a network of curves that cross (a Gordon surface): the u curves follow each other across the v curves, 2 to 64 of each. A curve is a sketch whose curves make one open chain (lines, arcs, splines end to end) or edges of earlier bodies end to end ({\"edges\": [...]}). Every u curve must cross every v curve within the tolerance; the first and last curve of each family are the sheet's edges, so the others end on them; curves are run the network's way whatever way they were drawn, and a family must cross the other in its order. The sheet passes through every curve within mismatch (a proven bound, held to the tolerance: the gaps where they cross and what the sum makes of them) and is an exact B-spline: two curves each way give the Coons patch, curves along a cylinder or sphere give that surface exactly. edges holds an edge: position checks that its curve lies on the face; tangent makes the sheet meet the face with one tangent plane (G1) all along the edge, the curves crossing the edge bent to leave it along the face with their own speed times weight (a plane, cylinder, cone or sphere, or a spline face, along any curve on it: a spline face is written across the curve exactly, from the edge's own curve in its parameters, or for an edge only within the tolerance of it from its projection); curvature does that and makes the sheet take the face's normal curvature across the edge too (G2): the curves crossing it get the face's curvature in their direction, the sheet's second derivative across is put where the face asks it all along (exactly), and the result is measured by the proven measure.continuity, refused unless G2 (along a spline face the family crossing the edge takes its crossing parameters from the face's line, so where both edges of a family are held in curvature to spline faces the curves crossing them must cross both lines at the same parameters). The face is F9:side(eA,eB,eC,eD), its numbers the feature ids of the sketches (or of the first edges' features) of the first and last u curve and the first and last v curve; its edges F9:startEdge(0, key) and F9:endEdge(0, key) along the first and last u curve, F9:startEdge(1, key) and F9:endEdge(1, key) along the v curves. The result reports network: mismatch, the tangency found on each held edge, and continuity (as measure.continuity reports it) on each edge held in curvature. Refused with the curves named: curves that do not cross, cross out of order or run past an edge, or are crossed far more unevenly than the rest of their family, rational curves whose weights cannot agree, a sheet that would need more than 2^20 control points (curves crossed at their own parameters each bring their own knots), a curve leaving a tangent edge square to its face or along it, curves leaving it to opposite sides, a corner where two held edges ask different tangent planes or twists, tangency to a torus or along a spline face's curve that is neither a parameter line nor a curve the face can be written across (or one crossing its seam). Give it a thickness with feature.plate. feature.edit changes u, v, edges and acrossDegree. acrossDegree (3 by default, or 5) is the degree each family is interpolated across with: 5 keeps a face held in curvature along the sheet free of kinks at its knots where its curves are of degree 5 too (docs/design/surfacing.md, Kinks of a curvature hold); the degree used is the one asked, but never more than the family's curves less one (two curves: degree 1; five curves: 3 by default, 4 when 5 is asked); null in feature.edit is the default again.", doc: true, names: ["u", "v", "edges"], feature_network),
    spec!("feature.surface.coons", "Coons Patch", r#"{"curves": [curve, curve, curve, curve], "edges": {uFirst|uLast|vFirst|vLast: {kind: free|position|tangent|curvature, face: face name, weight: expr = 1}} = all free, "name": string = automatic, "when": expr = always on}"#,
        "Adds the Coons patch of four curves round a patch, end to end in order (each a sketch's open chain or edges of earlier bodies, as feature.surface.network takes them): the network of curves 1 and 3 as its u curves (uFirst, uLast) and curves 4 and 2 as its v curves (vFirst, vLast), so the bilinearly blended surface between them; it is a feature.surface.network and is edited as one.", doc: true, names: ["curves", "edges"], feature_coons),
    spec!("feature.surface.grid", "Grid Surface", r#"{"points": [[[x, y, z], ...], ...], "table": table name, "origin": [x, y, z] = [0, 0, 0], "step": [x, y], "factor": expr = 1, "spacing": "greville|chord" = "greville", "name": string = automatic, "when": expr = always on}"#,
        "Adds a sheet through a grid of points: a cubic B-spline surface (each way; lower with fewer than 4 points that way) that passes through every point, e.g. a measured or computed surface, a scan's patch, a hull, a lens, a terrain. Give the points one of two ways. points: rows of points, every row as many, each point [x, y, z] of expressions (a variable can scale them: [\"10 mm\", \"0 mm\", \"1910 m * vertical_scale\"]). Or table with step: a regular grid whose heights are the numbers of a table of the document (table.set): row r of the table is row r of the grid at y = origin.y + r * step.y, its columns other than the key are the grid's columns, column c at x = origin.x + c * step.x, each point at z = origin.z + cell * factor (the cell in its column's unit times factor must be a length; a plain number is in the document's unit), e.g. {\"table\": \"heights\", \"step\": [\"5 mm\", \"5 mm\"], \"factor\": \"1 m * vertical_scale\"}; a step may be negative. 2 x 2 to 262144 points, at most 16384 a side (a table holds at most 63 heights a row). spacing sets the parameters the sheet takes its points at, one set each way, as a live spline's (sketch.spline): greville (the default) uniform knots with each point at its control point's Greville abscissa; chord the length along each row (column) up to each point as a share of the row's, averaged over the rows, on averaged knots, which follows how far apart the points are (between the points it stays closer to a smooth surface through them: its error falls about 16 times when the grid's step halves, where greville's falls about 4 times). Points on a plane give that plane exactly, with either spacing. The sheet's normal is along a row times across the rows (for the table form it looks along +z, whatever the steps' signs). The face is F9:face(0); its edges F9:edge(0) along the first row, F9:edge(1) along the last row, F9:edge(2) along the first column, F9:edge(3) along the last column; its corners F9:vertex(0) and F9:vertex(1) at the first row's first and last point, F9:vertex(2) and F9:vertex(3) at the last row's: numbered by the borders of the grid, as a primitive's are, so moving the points renames nothing. The result reports grid: rows, columns, deviation (the largest distance found between a point and the sheet, rounding) and bound (what rounding may leave there). Refused with the row and point named: rows of different lengths, fewer than 2 x 2 points, coordinates that are not finite, two neighbouring points within the tolerance, a row or column that doubles back on itself, and a sheet that folds (its normal vanishes or turns over: proven, not sampled, on every span); with the table form a missing table, one with fewer than 2 rows or 2 columns besides its key, a text in its columns, a height times factor that is not a length, a step of zero. Make a solid of it with feature.plate (a thickness), with feature.boolean intersect of a block that it crosses (keeps the part behind the sheet: below one that looks up), or with feature.extrude upTo its face. feature.edit changes points, table, origin, step, factor and spacing.", doc: true, feature_grid),
];

/// A list of network curves.
pub(crate) fn curves_value(
    doc: &Document,
    p: &Params,
    command: &'static str,
    key: &str,
) -> Result<Vec<Rail>, CommandError> {
    let value = p.get(key).ok_or_else(|| p.missing(key))?;
    let Some(items) = value.as_array() else {
        return fail(format!(
            "{command}: `{key}` must be a list of curves (sketches or {{\"edges\": [...]}})"
        ));
    };
    if items.len() < 2 || items.len() > MAX_NETWORK_CURVES {
        return fail(format!(
            "{command}: `{key}` has 2 to {MAX_NETWORK_CURVES} curves, got {}",
            items.len()
        ));
    }
    let mut out: Vec<Rail> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let slot = format!("{key}[{i}]");
        let wrapped = json!({ "curve": item });
        let one = Params::new(command, &wrapped, &["curve"])?;
        let rail = crate::develop_cmds::rail_value(doc, &one, command, "curve").map_err(|e| {
            CommandError::new(e.to_string().replace("`curve`", &format!("`{slot}`")))
        })?;
        if let Some(again) = out.iter().position(|r| *r == rail) {
            return fail(format!(
                "{command}: `{slot}` is the same curve as `{key}[{again}]`"
            ));
        }
        out.push(rail);
    }
    Ok(out)
}

/// The edge conditions, from `{"uFirst": {...}, ...}`.
fn edges_value(p: &Params, command: &'static str) -> Result<NetworkEdges, CommandError> {
    let mut edges = NetworkEdges::default();
    let Some(value) = p.get("edges") else {
        return Ok(edges);
    };
    let Some(map) = value.as_object() else {
        return fail(format!(
            "{command}: `edges` must be an object of uFirst, uLast, vFirst and vLast"
        ));
    };
    if let Some(other) = map
        .keys()
        .find(|k| !["uFirst", "uLast", "vFirst", "vLast"].contains(&k.as_str()))
    {
        return fail(format!(
            "{command}: `edges` has no edge `{other}`; the edges are uFirst, uLast, vFirst and vLast"
        ));
    }
    for (key, slot) in edges.all_mut() {
        if let Some(item) = map.get(key) {
            *slot = hold_value(command, key, item)?;
        }
    }
    Ok(edges)
}

pub(crate) fn hold_value(command: &str, key: &str, item: &Value) -> Result<EdgeHold, CommandError> {
    let Some(obj) = item.as_object() else {
        return fail(format!(
            "{command}: `edges.{key}` must be {{\"kind\": \"free|position|tangent|curvature\", \"face\": face name, \"weight\": expr}}"
        ));
    };
    if let Some(other) = obj
        .keys()
        .find(|k| !["kind", "face", "weight"].contains(&k.as_str()))
    {
        return fail(format!(
            "{command}: `edges.{key}` takes kind, face and weight, not `{other}`"
        ));
    }
    let kind = obj.get("kind").and_then(Value::as_str).unwrap_or("free");
    let face = |required: bool| -> Result<Option<craft3d_doc::PersistentName>, CommandError> {
        match obj.get("face") {
            Some(v) => Ok(Some(crate::datum_cmds::face_value(
                command,
                &format!("edges.{key}.face"),
                v,
            )?)),
            None if required => fail(format!(
                "{command}: `edges.{key}` is held to a face: give `face`"
            )),
            None => Ok(None),
        }
    };
    match kind {
        "free" => {
            if obj.contains_key("face") || obj.contains_key("weight") {
                return fail(format!(
                    "{command}: `edges.{key}` is free: it takes no face or weight"
                ));
            }
            Ok(EdgeHold::Free)
        }
        "position" => {
            if obj.contains_key("weight") {
                return fail(format!(
                    "{command}: `edges.{key}`: a weight is for a tangent or curvature edge"
                ));
            }
            let face = face(true)?.ok_or_else(|| {
                CommandError::new(format!("{command}: `edges.{key}` needs a face"))
            })?;
            Ok(EdgeHold::Position { face })
        }
        "tangent" | "curvature" => {
            let face = face(true)?.ok_or_else(|| {
                CommandError::new(format!("{command}: `edges.{key}` needs a face"))
            })?;
            let weight = match obj.get("weight") {
                None => Expr::new("1"),
                Some(Value::Number(n)) => {
                    let w = n.as_f64().unwrap_or(f64::NAN);
                    if !(w.is_finite() && w > 0.0) {
                        return fail(format!(
                            "{command}: `edges.{key}.weight` is a positive number, got {n}"
                        ));
                    }
                    Expr::new(n.to_string())
                }
                Some(Value::String(text)) if !text.trim().is_empty() => Expr::new(text.clone()),
                Some(_) => {
                    return fail(format!(
                        "{command}: `edges.{key}.weight` must be a number or an expression"
                    ));
                }
            };
            Ok(if kind == "curvature" {
                EdgeHold::Curvature { face, weight }
            } else {
                EdgeHold::Tangent { face, weight }
            })
        }
        other => fail(format!(
            "{command}: `edges.{key}.kind` is free, position, tangent or curvature, not `{other}`"
        )),
    }
}

fn check_families(command: &str, u: &[Rail], v: &[Rail]) -> Result<(), CommandError> {
    if let Some(shared) = u.iter().find(|r| v.contains(r)) {
        let what = match shared {
            Rail::Sketch(id) => format!("{id}"),
            Rail::Edges(edges) => edges.first().map(ToString::to_string).unwrap_or_default(),
        };
        return fail(format!("{command}: {what} is both a u curve and a v curve"));
    }
    Ok(())
}

fn feature_network(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.surface.network";
    let p = Params::new(ID, params, &["u", "v", "edges", "acrossDegree", "name"])?;
    let doc = session.document()?.history.current();
    let u = curves_value(doc, &p, ID, "u")?;
    let v = curves_value(doc, &p, ID, "v")?;
    check_families(ID, &u, &v)?;
    let edges = edges_value(&p, ID)?;
    let across_degree = across_value(&p, ID)?;
    add_feature(
        session,
        ID,
        p.str("name")?,
        FeatureKind::SurfaceNetwork(SurfaceNetwork {
            u,
            v,
            edges,
            across_degree,
        }),
    )
}

fn feature_coons(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.surface.coons";
    let p = Params::new(ID, params, &["curves", "edges", "name"])?;
    let doc = session.document()?.history.current();
    let curves = curves_value(doc, &p, ID, "curves")?;
    let [a, b, c, d] = curves.as_slice() else {
        return fail(format!(
            "{ID}: `curves` are the four curves round the patch, got {}",
            curves.len()
        ));
    };
    let edges = edges_value(&p, ID)?;
    add_feature(
        session,
        ID,
        p.str("name")?,
        FeatureKind::SurfaceNetwork(SurfaceNetwork {
            u: vec![a.clone(), c.clone()],
            v: vec![d.clone(), b.clone()],
            edges,
            across_degree: None,
        }),
    )
}

/// The points of a grid surface from `points`, or from `table` with
/// `step`, `origin` and `factor`.
fn grid_points(doc: &Document, p: &Params, command: &str) -> Result<GridPoints, CommandError> {
    match (p.get("points"), p.get("table")) {
        (Some(_), Some(_)) => fail(format!("{command}: give `points` or `table`, not both")),
        (None, None) => fail(format!(
            "{command}: give `points` (rows of [x, y, z]) or `table` with `step` (the heights of a regular grid)"
        )),
        (Some(value), None) => {
            if let Some(key) = ["origin", "step", "factor"].into_iter().find(|k| p.has(k)) {
                return fail(format!(
                    "{command}: `{key}` places a grid read from a table; with `points` each point is given whole"
                ));
            }
            Ok(GridPoints::Given {
                rows: rows_value(p, command, value)?,
            })
        }
        (None, Some(value)) => {
            let table = table_value(doc, command, value)?;
            let origin = match p.expr3("origin")? {
                Some(o) => o.map(Expr::new),
                None => [Expr::new("0"), Expr::new("0"), Expr::new("0")],
            };
            Ok(GridPoints::Table {
                table,
                origin,
                step: step_value(p, command)?.ok_or_else(|| {
                    CommandError::new(format!(
                        "{command}: a grid read from a table needs `step`: [along x, along y], the distance between its columns and between its rows"
                    ))
                })?,
                factor: Expr::new(p.expr("factor")?.unwrap_or_else(|| "1".to_string())),
            })
        }
    }
}

/// `table`: the name of a table the document has.
fn table_value(doc: &Document, command: &str, value: &Value) -> Result<String, CommandError> {
    let Some(table) = value.as_str() else {
        return fail(format!(
            "{command}: `table` is the name of a table of the document (table.set)"
        ));
    };
    if doc.table(table).is_none() {
        let known: Vec<&str> = doc.tables.iter().map(|t| t.name.as_str()).collect();
        return fail(if known.is_empty() {
            format!(
                "{command}: there is no table named `{table}` (the document has no tables; make one with table.set)"
            )
        } else {
            format!(
                "{command}: there is no table named `{table}` (the tables are {})",
                known.join(", ")
            )
        });
    }
    Ok(table.to_string())
}

/// `step`: two expressions, `[x, y]`.
fn step_value(p: &Params, command: &str) -> Result<Option<[Expr; 2]>, CommandError> {
    let Some(value) = p.get("step") else {
        return Ok(None);
    };
    let Some([x, y]) = value.as_array().map(Vec::as_slice) else {
        return fail(format!(
            "{command}: `step` is [x, y]: the distance between the grid's columns (along x) and between its rows (along y)"
        ));
    };
    Ok(Some([
        Expr::new(p.expr_of("step", x)?),
        Expr::new(p.expr_of("step", y)?),
    ]))
}

/// `points`: rows of points, each `[x, y, z]` of expressions, every row
/// as long.
fn rows_value(
    p: &Params,
    command: &str,
    value: &Value,
) -> Result<Vec<Vec<[Expr; 3]>>, CommandError> {
    let Some(rows) = value.as_array() else {
        return fail(format!(
            "{command}: `points` is a list of rows, each a list of points [x, y, z]"
        ));
    };
    if rows.len() < 2 || rows.len() > MAX_GRID_SIDE {
        return fail(format!(
            "{command}: `points` has 2 to {MAX_GRID_SIDE} rows, got {}",
            rows.len()
        ));
    }
    let width = rows.first().and_then(Value::as_array).map_or(0, Vec::len);
    if !(2..=MAX_GRID_SIDE).contains(&width) {
        return fail(format!(
            "{command}: a row of `points` has 2 to {MAX_GRID_SIDE} points, row 1 has {width}"
        ));
    }
    let total = rows.len().saturating_mul(width);
    if total > MAX_GRID_POINTS {
        return fail(format!(
            "{command}: a grid holds at most {MAX_GRID_POINTS} points; {} rows of {width} are {total}",
            rows.len()
        ));
    }
    let mut out = Vec::with_capacity(rows.len());
    for (r, row) in rows.iter().enumerate() {
        let Some(row) = row.as_array() else {
            return fail(format!(
                "{command}: row {} of `points` is not a list of points [x, y, z]",
                r + 1
            ));
        };
        if row.len() != width {
            return fail(format!(
                "{command}: row {} of `points` has {} points and row 1 has {width}: every row of a grid has as many points",
                r + 1,
                row.len()
            ));
        }
        let mut points = Vec::with_capacity(width);
        for (c, point) in row.iter().enumerate() {
            let Some([x, y, z]) = point.as_array().map(Vec::as_slice) else {
                return fail(format!(
                    "{command}: row {}, point {} of `points` is not [x, y, z]",
                    r + 1,
                    c + 1
                ));
            };
            let slot = format!("points[{r}][{c}]");
            let coordinate = |e: &Value| -> Result<Expr, CommandError> {
                let text = p.expr_of(&slot, e)?;
                if text.trim().is_empty() {
                    return fail(format!(
                        "{command}: row {}, point {} of `points` has an empty expression",
                        r + 1,
                        c + 1
                    ));
                }
                Ok(Expr::new(text))
            };
            points.push([coordinate(x)?, coordinate(y)?, coordinate(z)?]);
        }
        out.push(points);
    }
    Ok(out)
}

/// `spacing`: greville (the default) or chord.
fn spacing_value(p: &Params, command: &str) -> Result<Option<SplineSpacing>, CommandError> {
    match p.str("spacing")? {
        None => Ok(None),
        Some(name) => SplineSpacing::from_name(name).map(Some).ok_or_else(|| {
            CommandError::new(format!(
                "{command}: `spacing` is {}, got \"{name}\"",
                SplineSpacing::NAMES
            ))
        }),
    }
}

fn feature_grid(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.surface.grid";
    let p = Params::new(
        ID,
        params,
        &[
            "points", "table", "origin", "step", "factor", "spacing", "name",
        ],
    )?;
    let doc = session.document()?.history.current();
    let points = grid_points(doc, &p, ID)?;
    let spacing = spacing_value(&p, ID)?.unwrap_or_default();
    add_feature(
        session,
        ID,
        p.str("name")?,
        FeatureKind::SurfaceGrid(SurfaceGrid { points, spacing }),
    )
}

/// `feature.edit` for a grid surface: `points` (the points given whole),
/// `table`, `origin`, `step` and `factor` (a grid read from a table; from
/// points to a table, `table` and `step` are needed), `spacing`.
pub(crate) fn edit_grid(
    doc: &Document,
    p: &Params,
    grid: &mut SurfaceGrid,
) -> Result<(), CommandError> {
    const ID: &str = "feature.edit";
    let table_keys = ["table", "origin", "step", "factor"];
    if let Some(value) = p.get("points") {
        if let Some(key) = table_keys.into_iter().find(|k| p.has(k)) {
            return fail(format!(
                "{ID}: `{key}` places a grid read from a table; with `points` each point is given whole"
            ));
        }
        grid.points = GridPoints::Given {
            rows: rows_value(p, ID, value)?,
        };
    } else if table_keys.into_iter().any(|k| p.has(k)) {
        match &mut grid.points {
            GridPoints::Table {
                table,
                origin,
                step,
                factor,
            } => {
                if let Some(value) = p.get("table") {
                    *table = table_value(doc, ID, value)?;
                }
                if let Some(o) = p.expr3("origin")? {
                    *origin = o.map(Expr::new);
                }
                if let Some(s) = step_value(p, ID)? {
                    *step = s;
                }
                if let Some(f) = p.expr("factor")? {
                    *factor = Expr::new(f);
                }
            }
            GridPoints::Given { .. } => {
                grid.points = grid_points(doc, p, ID)?;
            }
        }
    }
    if let Some(spacing) = spacing_value(p, ID)? {
        grid.spacing = spacing;
    }
    Ok(())
}

/// What a grid surface reports in its feature's result.
pub(crate) fn grid_json(map: &mut Map<String, Value>, made: &craft3d_regen::Gridded) {
    map.insert(
        "grid".into(),
        json!({"rows": made.rows, "columns": made.columns, "deviation": made.deviation, "bound": made.bound}),
    );
}

/// `feature.edit` for a network surface: `u`, `v`, `edges`.
pub(crate) fn edit(
    doc: &Document,
    p: &Params,
    sheet: &mut SurfaceNetwork,
) -> Result<(), CommandError> {
    if p.has("u") {
        sheet.u = curves_value(doc, p, "feature.edit", "u")?;
    }
    if p.has("v") {
        sheet.v = curves_value(doc, p, "feature.edit", "v")?;
    }
    check_families("feature.edit", &sheet.u, &sheet.v)?;
    if p.has("edges") {
        sheet.edges = edges_value(p, "feature.edit")?;
    }
    if p.has("acrossDegree") {
        sheet.across_degree = across_value(p, "feature.edit")?;
    }
    Ok(())
}

/// `acrossDegree`: 3 (the default, kept as none) or 5; anything else is
/// refused, saying which degrees there are.
fn across_value(p: &Params, command: &str) -> Result<Option<u8>, CommandError> {
    let Some(value) = p.get("acrossDegree") else {
        return Ok(None);
    };
    match value.as_u64() {
        Some(3) => Ok(None),
        Some(5) => Ok(Some(5)),
        _ => fail(format!(
            "{command}: `acrossDegree` is the degree a network is interpolated across with, 3 (the default) or 5, not {value}"
        )),
    }
}

/// What a network surface reports in its feature's result.
pub(crate) fn report_json(map: &mut Map<String, Value>, made: &craft3d_regen::Networked) {
    let mut tangency = Map::new();
    for (key, value) in ["uFirst", "uLast", "vFirst", "vLast"]
        .iter()
        .zip(made.tangency)
    {
        if let Some(angle) = value {
            tangency.insert((*key).to_string(), json!(angle));
        }
    }
    let mut seams = Map::new();
    for (key, value) in ["uFirst", "uLast", "vFirst", "vLast"]
        .iter()
        .zip(made.seams.iter())
    {
        if let Some(c) = value {
            seams.insert(
                (*key).to_string(),
                crate::continuity_cmds::continuity_json(c),
            );
        }
    }
    map.insert(
        "network".into(),
        json!({"mismatch": made.mismatch, "tangency": tangency, "continuity": seams}),
    );
}
