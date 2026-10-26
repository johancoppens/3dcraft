//! Plates: a sheet (or faces of any body) given a thickness on a side,
//! as a solid of its own, with joints along its edges to neighbouring
//! plates (`feature.plate`; docs/design/plates.md).

use crate::commands::{Run, add_feature, fail, spec};
use crate::datum_cmds::{face_value, name_value, plane_value};
use crate::face_cmds::faces_value;
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    Document, Expr, FeatureKind, JointKind, MAX_PLATE_JOINTS, PersistentName, Plate, PlateJoint,
    PlateSide,
};
use serde_json::Value;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.plate", "Plate", r#"{"faces": [face name, ...], "thickness": expr, "side": "in|out|mid" = "in", "joints": [{edge: edge name, kind: square|bevel|mitre|touch|over|under|onto, with: face name, thickness: expr, side: in|out|mid, angle: expr, onto: plane or face name}, ...] = [], "name": string = automatic, "when": expr = always on}"#,
        "Adds a plate: faces of a body (a sheet: a fill, a loft of open sections, a developable; or faces of a solid), connected across the edges they share and smooth there, given thickness to a side of them along their outward normal: in puts the material behind the faces (the faces are the plate's outside, as the outside of a hull), out in front of them, mid half on each side. The plate is the faces offset to both levels and, along every edge of the faces' boundary, a rim. A rim is square (along the faces' normal) unless joints says otherwise for that edge: bevel turns it from the normal by angle about the edge (positive leans the front outward); with a neighbouring plate, given by with (a face of the other sheet whose boundary the edge runs along) and that plate's thickness and side (this plate's when left out), mitre ends both plates on the surface through where their outer skins meet and where their inner skins meet (the neighbour's mitre is the same surface: they meet face to face, on the bisector when equally thick), touch ends both square where the skins facing each other's material meet (the inner edges of two stitch-and-glue panels just touch), over makes this plate cover the neighbour's edge, flush with its far skin, and under rests this plate's edge on the neighbour's near skin (one plate over and the other under: the deck rests on the sheer panel); onto ends the plate on a surface (xy, yz, zx, a datum plane, or any face by its name, a stem too), so two plates whose ends lie on one surface meet there in one corner (the bow of a hull). Give the neighbour the matching joint (mitre with mitre, touch with touch, over with under) so the two meet without a gap or overlap; measure.interference and measure.clearance say. The faces' body stays: the plate is a body of its own. On faces of a solid, feature.thicken ends the layer on the solid's neighbouring faces instead. Its faces are named F9:copy(0)/<face> for the front skin of each face (the side the normal points to), F9:copy(1)/<face> for the back skin, and F9:rim(h)/<face> for the rim along an edge of that face (h stands for the edge's name); edges and vertices by their faces. Refused, with the edge: faces that do not hang together or meet at a crease (plate them apart and join them), an edge that does not run along the neighbour's boundary, plates in line with each other (no corner to join at), a joint that runs into the plate (a plate thicker than the faces are narrow), a corner the joints do not reach. Change it with feature.edit: faces, thickness, side, joints.", doc: true, names: ["faces", "joints"], feature_plate),
];

/// A side as `feature.plate` and `feature.edit` take it.
fn side_of(command: &str, key: &str, text: &str) -> Result<PlateSide, CommandError> {
    Ok(match text {
        "in" => PlateSide::In,
        "out" => PlateSide::Out,
        "mid" => PlateSide::Mid,
        other => {
            return fail(format!(
                "{command}: `{key}` is \"{other}\"; give \"in\", \"out\" or \"mid\""
            ));
        }
    })
}

fn kind_of(command: &str, key: &str, text: &str) -> Result<JointKind, CommandError> {
    Ok(match text {
        "square" => JointKind::Square,
        "bevel" => JointKind::Bevel,
        "mitre" | "miter" => JointKind::Mitre,
        "onto" => JointKind::Onto,
        "touch" => JointKind::Touch,
        "over" => JointKind::Over,
        "under" => JointKind::Under,
        other => {
            return fail(format!(
                "{command}: `{key}` is \"{other}\"; give \"square\", \"bevel\", \"mitre\", \"touch\", \"over\", \"under\" or \"onto\""
            ));
        }
    })
}

fn expr_of(command: &str, key: &str, value: &Value) -> Result<Expr, CommandError> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Ok(Expr::new(text.clone())),
        Value::Number(number) => Ok(Expr::new(number.to_string())),
        _ => fail(format!(
            "{command}: `{key}` must be an expression, such as \"4 mm\""
        )),
    }
}

/// The joints of a plate, as a list of objects.
fn joints_value(
    doc: &Document,
    command: &str,
    value: &Value,
) -> Result<Vec<PlateJoint>, CommandError> {
    let Some(items) = value.as_array() else {
        return fail(format!(
            "{command}: `joints` must be a list of {{\"edge\", \"kind\", ...}}"
        ));
    };
    if items.len() > MAX_PLATE_JOINTS {
        return fail(format!(
            "{command}: `joints` has {} joints; a plate takes {MAX_PLATE_JOINTS} at most",
            items.len()
        ));
    }
    let mut out: Vec<PlateJoint> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let key = |k: &str| format!("joints.{i}.{k}");
        let Some(fields) = item.as_object() else {
            return fail(format!(
                "{command}: `joints.{i}` must be an object {{\"edge\", \"kind\", ...}}"
            ));
        };
        for name in fields.keys() {
            if !["edge", "kind", "with", "thickness", "side", "angle", "onto"]
                .contains(&name.as_str())
            {
                return fail(format!(
                    "{command}: `joints.{i}` has `{name}`; a joint has edge, kind, with, thickness, side, angle and onto"
                ));
            }
        }
        let Some(edge) = fields.get("edge") else {
            return fail(format!("{command}: `{}` is missing", key("edge")));
        };
        let edge: PersistentName = name_value(
            command,
            &key("edge"),
            edge,
            "an edge name such as \"F2:startEdge(e1)\" (document.edges lists them)",
        )?;
        if !edge.is_edge() {
            return fail(format!(
                "{command}: `{}`: {edge} is not an edge (document.edges lists the edges)",
                key("edge")
            ));
        }
        if out.iter().any(|j| j.edge == edge) {
            return fail(format!(
                "{command}: `{}`: {edge} has a joint already",
                key("edge")
            ));
        }
        let kind = match fields.get("kind") {
            Some(Value::String(text)) => kind_of(command, &key("kind"), text)?,
            Some(_) => return fail(format!("{command}: `{}` must be a string", key("kind"))),
            None => return fail(format!("{command}: `{}` is missing", key("kind"))),
        };
        let with = match fields.get("with") {
            Some(v) => Some(face_value(command, &key("with"), v)?),
            None => None,
        };
        let thickness = match fields.get("thickness") {
            Some(v) => Some(expr_of(command, &key("thickness"), v)?),
            None => None,
        };
        let side = match fields.get("side") {
            Some(Value::String(text)) => Some(side_of(command, &key("side"), text)?),
            Some(_) => return fail(format!("{command}: `{}` must be a string", key("side"))),
            None => None,
        };
        let angle = match fields.get("angle") {
            Some(v) => Some(expr_of(command, &key("angle"), v)?),
            None => None,
        };
        let onto = match fields.get("onto") {
            Some(v) => Some(plane_value(doc, command, &key("onto"), v)?),
            None => None,
        };
        match (kind, &onto) {
            (JointKind::Onto, None) => {
                return fail(format!(
                    "{command}: `{}`: an onto end needs the surface it lies on: xy, yz, zx, a datum plane or a face by its name",
                    key("onto")
                ));
            }
            (JointKind::Onto, Some(_)) | (_, None) => {}
            (_, Some(_)) => {
                return fail(format!(
                    "{command}: `{}`: only an onto end lies on a surface",
                    key("onto")
                ));
            }
        }
        match (kind.has_neighbour(), &with) {
            (true, None) => {
                return fail(format!(
                    "{command}: `{}`: this joint needs the face of the neighbouring sheet the edge runs along",
                    key("with")
                ));
            }
            (false, Some(_)) => {
                return fail(format!(
                    "{command}: `{}`: a square, bevelled or onto end has no neighbour; leave with out",
                    key("with")
                ));
            }
            _ => {}
        }
        if !kind.has_neighbour() && (thickness.is_some() || side.is_some()) {
            return fail(format!(
                "{command}: `joints.{i}`: thickness and side are the neighbour's, and a square, bevelled or onto end has none"
            ));
        }
        match (kind, &angle) {
            (JointKind::Bevel, None) => {
                return fail(format!(
                    "{command}: `{}`: a bevel needs its angle from the normal, such as \"30 deg\"",
                    key("angle")
                ));
            }
            (JointKind::Bevel, Some(_)) | (_, None) => {}
            (_, Some(_)) => {
                return fail(format!(
                    "{command}: `{}`: only a bevel has an angle",
                    key("angle")
                ));
            }
        }
        out.push(PlateJoint {
            edge,
            kind,
            with,
            thickness,
            side,
            angle,
            onto,
        });
    }
    Ok(out)
}

/// `feature.edit` for a plate: `faces`, `side` and `joints` (`thickness`
/// is a plain expression, set where the others are).
pub(crate) fn edit_plate(doc: &Document, p: &Params, made: &mut Plate) -> Result<(), CommandError> {
    const ID: &str = "feature.edit";
    if let Some(value) = p.get("faces") {
        made.faces = faces_value(ID, "faces", value, "plate", false)?;
    }
    if let Some(side) = p.str("side")? {
        made.side = side_of(ID, "side", side)?;
    }
    if let Some(value) = p.get("joints") {
        made.joints = joints_value(doc, ID, value)?;
    }
    Ok(())
}

fn feature_plate(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.plate";
    let p = Params::new(
        ID,
        params,
        &["faces", "thickness", "side", "joints", "name"],
    )?;
    // The document has to be there before anything is said of the
    // parameters.
    session.document()?;
    let Some(value) = p.get("faces") else {
        return Err(p.missing("faces"));
    };
    let faces = faces_value(ID, "faces", value, "plate", false)?;
    let Some(thickness) = p.expr("thickness")? else {
        return Err(p.missing("thickness"));
    };
    let side = match p.str("side")? {
        Some(text) => side_of(ID, "side", text)?,
        None => PlateSide::default(),
    };
    let joints = match p.get("joints") {
        Some(value) => joints_value(session.document()?.history.current(), ID, value)?,
        None => Vec::new(),
    };
    let kind = FeatureKind::Plate(Plate {
        faces,
        thickness: Expr::new(thickness),
        side,
        joints,
    });
    crate::blend_cmds::of_one_body(ID, session, &kind)?;
    add_feature(session, ID, p.str("name")?, kind)
}
