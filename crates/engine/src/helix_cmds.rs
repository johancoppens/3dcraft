//! Helices: a datum curve about an axis that a sweep can follow (a coil
//! spring, the groove of a thread).

use crate::commands::{Run, fail, spec};
use crate::datum_cmds::{axis_value, point_value, store};
use crate::params::Params;
use crate::{CommandSpec, Session};
use craft3d_doc::{DatumHelix, Expr, FeatureKind, HelixExtent, HelixStart};
use serde_json::Value;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("datum.helix", "Helix", r#"{"axis": "x|y|z" or [face name, face name] or datum axis, "radius": expr, "start": point, "pitch": expr, "turns": expr, "height": expr, "leftHanded": boolean = false, "replace": id or name, "name": string = automatic}"#,
        "Adds a helix: a curve that winds about an axis at a constant radius and advances one pitch along it per turn, for a sweep to follow (feature.sweep, path): a coil spring, the groove of a thread. It makes no body; it is drawn as a curve. axis is a world axis (through the origin), the straight edge between two faces of one body ([face name, face name]) or a datum axis (id or name); the helix advances the way it points. Where it starts, one of two: radius, a length, starts it level with the axis's own point (the origin for a world axis), towards the first of the world's x, y and z axes that is least along the axis (x for z, y for x, x for y); start, a point ([x, y, z], a vertex name, {\"center\": [face name, face name]} or a datum point), starts it there, at that point's distance from the axis. pitch is how far it advances per turn; how far it goes is turns (a number, need not be whole) or height (a length along the axis). It turns counter-clockwise seen from the tip of the axis, or clockwise with leftHanded. Lengths and turns are expressions and follow variables; feature.edit changes radius, pitch, turns, height and leftHanded. replace gives an existing helix this definition instead of adding one. The result has the helix under datum: origin (on the axis, level with the start), axis, start and end points, radius, pitch, turns, height, length (turns · √((2πr)² + p²)) and leftHanded. A helix is not a rational curve: where a curve is needed it is a polynomial B-spline within a quarter of the document's tolerance.", doc: true, datum_helix),
];

fn datum_helix(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "datum.helix";
    let p = Params::new(
        ID,
        params,
        &[
            "axis",
            "radius",
            "start",
            "pitch",
            "turns",
            "height",
            "leftHanded",
            "replace",
            "name",
        ],
    )?;
    let doc = session.document()?.history.current();
    let axis = match p.get("axis") {
        Some(value) => axis_value(doc, ID, "axis", value)?,
        None => return Err(p.missing("axis")),
    };
    let start = match (p.expr("radius")?, p.get("start")) {
        (Some(_), Some(_)) => {
            return fail(format!(
                "{ID}: give either `radius` or `start`: both say where the helix starts"
            ));
        }
        (Some(radius), None) => HelixStart::Radius {
            radius: Expr::new(radius),
        },
        (None, Some(value)) => HelixStart::Point {
            point: point_value(doc, &p, "start", value)?,
        },
        (None, None) => {
            return fail(format!(
                "{ID}: missing parameter `radius` (a length) or `start` (a point): where the helix starts"
            ));
        }
    };
    let Some(pitch) = p.expr("pitch")? else {
        return Err(p.missing("pitch"));
    };
    let extent = match (p.expr("turns")?, p.expr("height")?) {
        (Some(_), Some(_)) => {
            return fail(format!(
                "{ID}: give either `turns` or `height`: both say how far the helix goes"
            ));
        }
        (Some(turns), None) => HelixExtent::Turns {
            turns: Expr::new(turns),
        },
        (None, Some(height)) => HelixExtent::Height {
            height: Expr::new(height),
        },
        (None, None) => {
            return fail(format!(
                "{ID}: missing parameter `turns` (a number) or `height` (a length): how far the helix goes"
            ));
        }
    };
    let kind = FeatureKind::DatumHelix(DatumHelix {
        axis,
        start,
        pitch: Expr::new(pitch),
        extent,
        left_handed: p.bool("leftHanded")?.unwrap_or(false),
    });
    store(session, ID, &p, kind)
}
