//! `sketch.curve`: a curve given by formulas, `x(t)` and `y(t)` for `t` over
//! a range (`craft3d_doc::Geometry::Formula`). Regeneration fits a B-spline
//! to it within the document's tolerance every time, from the variables as
//! they are then (`craft3d_regen::curve_fit`); this command checks that it
//! can be fitted now, makes its end points and adds it.

use crate::commands::{Run, fail, spec};
use crate::params::Params;
use crate::sketch_cmds::{change, entity_id, sketch_ref, sketch_result};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{EntityId, Expr, FORMULA_PARAMETER, Geometry, SketchError};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("sketch.curve", "Sketch Curve from Formulas", r#"{"sketch": id or name, "x": expr in t, "y": expr in t, "t": [from, to], "start": point id = a new point, "end": point id = a new point, "closed": boolean = when the ends meet, "construction": boolean = false}"#,
    "Adds a curve given by formulas: the point (x, y) in the sketch's axes for t from t[0] to t[1]. x and y are expressions in t and the document's variables (inside them t is the curve's parameter, also when a variable is called t); t[0] and t[1] are expressions in the variables, both plain numbers, both angles or both lengths. A plain t inside sin, cos and tan is in radians: an involute is x = \"rb * (cos(t) + t * sin(t))\", y = \"rb * (sin(t) - t * cos(t))\" for t from 0 to \"tan(acos(rb / ra))\". The curve is fitted again at every regeneration, so it follows the variables: a cubic B-spline within half the document's tolerance of the formulas at every point checked (three per span between samples, refined until they hold), with its ends exactly where the formulas put them. It ends on two points (start and end: existing points, or new ones made there), which the curve holds in place: lines, arcs and splines built on them, or made coincident with them, close a profile with it; a constraint that would move them fails the sketch. When the ends meet (or closed is true) the curve starts and ends on one point and is a profile on its own. It has no unknowns of its own; it takes pointOn (a point held on the fitted curve) and tangent: to a line or a circle or arc anywhere (a line touching it where the curve runs parallel to the line, the one such place nearest to it; a circle at the foot of its centre, on the side of the curve the centre is on), or to a line, an arc, a spline or a live spline at an end point the two share (the other then leaves that point along the curve); two formula curves take no tangent (nothing moves them). Refused, with the reason, when the formulas fail or are not finite somewhere in the range, give no length, or cannot be fitted (a kink or a jump: split the curve there). The result gives the entity, its points and the fit: controlPoints and deviation (the largest distance found from the formulas).", doc: true, sketch_curve),
];

fn sketch_curve(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.curve";
    let p = Params::new(
        COMMAND,
        params,
        &[
            "sketch",
            "x",
            "y",
            "t",
            "start",
            "end",
            "closed",
            "construction",
        ],
    )?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let x = p.expr("x")?.ok_or_else(|| p.missing("x"))?;
    let y = p.expr("y")?.ok_or_else(|| p.missing("y"))?;
    let [from, to] = match p.get("t") {
        Some(Value::Array(items)) if items.len() == 2 => {
            let mut range = items.iter().map(|item| p.expr_of("t", item));
            match (range.next(), range.next()) {
                (Some(from), Some(to)) => [from?, to?],
                _ => return Err(p.missing("t")),
            }
        }
        Some(_) => {
            return fail(format!(
                "{COMMAND}: `t` must be the range of the parameter, [from, to] (two expressions)"
            ));
        }
        None => return Err(p.missing("t")),
    };
    let point = |key: &str| -> Result<Option<EntityId>, CommandError> {
        p.get(key)
            .map(|value| entity_id(COMMAND, key, value))
            .transpose()
    };
    let (start, end) = (point("start")?, point("end")?);
    let closed = p.bool("closed")?;
    let construction = p.bool("construction")?.unwrap_or(false);
    if closed == Some(true) && end.is_some() && end != start {
        return fail(format!(
            "{COMMAND}: a closed curve starts and ends on one point: give `start`, not another `end`"
        ));
    }
    let formulas = [Expr::new(x), Expr::new(y), Expr::new(from), Expr::new(to)];
    // Fitted now with the variables as they are: an error here is the
    // one regeneration would give, and the ends are where to put points.
    let doc = session.document()?.history.current();
    let tolerance = doc.tolerance.linear;
    let variables = doc
        .variable_table()
        .map_err(|e| CommandError::new(format!("{COMMAND}: {e}")))?
        .evaluate();
    let [fx, fy, ff, ft] = &formulas;
    let fit = craft3d_regen::fit_formula_curve(doc, &variables, "the curve", [fx, fy, ff, ft])
        .map_err(|e| CommandError::new(format!("{COMMAND}: {e}")))?;
    let (Some(&first), Some(&last)) = (fit.spline.control.first(), fit.spline.control.last())
    else {
        return fail(format!("{COMMAND}: the curve has no fitted points"));
    };
    let meet = (first[0] - last[0]).hypot(first[1] - last[1]) <= tolerance;
    let closed = closed.unwrap_or(meet && end.is_none_or(|e| Some(e) == start));
    if closed && !meet {
        return fail(format!(
            "{COMMAND}: the curve does not close: its formulas put its start at ({}, {}) and its end at ({}, {}); leave `closed` out, or change the formulas",
            first[0], first[1], last[0], last[1]
        ));
    }
    let deviation = fit.deviation;
    let control = fit.spline.control.len();
    let before = crate::commands::failing(session)?;
    let (curve, start, end) = change(session, COMMAND, "Add curve", id, |s| {
        // A point given is where the curve puts it, or the sketch fails:
        // said now rather than after.
        let ensure = |s: &craft3d_doc::Sketch, id: EntityId, at: [f64; 2]| {
            let (px, py) = s.point(id)?;
            let off = (px - at[0]).hypot(py - at[1]);
            // Another formula curve that ends there holds the point where
            // its formulas put it: both have to meet.
            let other = s.entities.iter().find(|e| {
                matches!(e.geometry, Geometry::Formula { start, end, .. } if start == id || end == id)
            });
            if off > tolerance
                && let Some(other) = other
            {
                return Err(SketchError::Invalid(format!(
                    "the formula curve {} ends at {id}, at ({px}, {py}), but this curve starts or ends at ({}, {}): they do not meet; change the formulas, or give another point",
                    other.id, at[0], at[1]
                )));
            }
            if off > tolerance
                && s.constraints
                    .iter()
                    .any(|c| c.kind.entities().contains(&id))
            {
                return Err(SketchError::Invalid(format!(
                    "{id} is at ({px}, {py}) and held there by constraints, but the curve starts or ends at ({}, {}); give another point, or none",
                    at[0], at[1]
                )));
            }
            Ok(id)
        };
        let start = match start {
            Some(id) => ensure(s, id, first)?,
            None => s.add_point(first[0], first[1])?,
        };
        let end = if closed {
            start
        } else {
            match end {
                Some(id) => ensure(s, id, last)?,
                None => s.add_point(last[0], last[1])?,
            }
        };
        let curve = s.add_formula(formulas, start, end, construction)?;
        Ok((curve, start, end))
    })?;
    sketch_result(
        session,
        id,
        Some(&before),
        json!({
            "entity": curve.to_string(),
            "start": start.to_string(),
            "end": end.to_string(),
            "closed": start == end,
            "parameter": FORMULA_PARAMETER,
            "fit": {"degree": craft3d_regen::curve_fit::FIT_DEGREE, "controlPoints": control, "deviation": deviation},
        }),
    )
}
