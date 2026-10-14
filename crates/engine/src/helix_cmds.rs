//! Helices and threads: a datum curve about an axis that a sweep can
//! follow (a coil spring), and an ISO metric thread cut into a
//! cylindrical face by the groove of its profile swept along a helix.

use crate::commands::{Run, add_feature, fail, spec};
use crate::datum_cmds::{axis_value, face_value, point_value, store};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{DatumHelix, Expr, FeatureKind, HelixExtent, HelixStart, Thread};
use craft3d_regen::{ThreadReport, coarse_pitch, nominal_diameter};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("datum.helix", "Helix", r#"{"axis": "x|y|z" or [face name, face name] or datum axis, "radius": expr, "start": point, "pitch": expr, "turns": expr, "height": expr, "leftHanded": boolean = false, "replace": id or name, "name": string = automatic}"#,
        "Adds a helix: a curve that winds about an axis at a constant radius and advances one pitch along it per turn, for a sweep to follow (feature.sweep, path): a coil spring, the groove of a thread. It makes no body; it is drawn as a curve. axis is a world axis (through the origin), the straight edge between two faces of one body ([face name, face name]) or a datum axis (id or name); the helix advances the way it points. Where it starts, one of two: radius, a length, starts it level with the axis's own point (the origin for a world axis), towards the first of the world's x, y and z axes that is least along the axis (x for z, y for x, x for y); start, a point ([x, y, z], a vertex name, {\"center\": [face name, face name]} or a datum point), starts it there, at that point's distance from the axis. pitch is how far it advances per turn; how far it goes is turns (a number, need not be whole) or height (a length along the axis). It turns counter-clockwise seen from the tip of the axis, or clockwise with leftHanded. Lengths and turns are expressions and follow variables; feature.edit changes radius, pitch, turns, height and leftHanded. replace gives an existing helix this definition instead of adding one. The result has the helix under datum: origin (on the axis, level with the start), axis, start and end points, radius, pitch, turns, height, length (turns · √((2πr)² + p²)) and leftHanded. A helix is not a rational curve: where a curve is needed it is a polynomial B-spline within a quarter of the document's tolerance.", doc: true, datum_helix),
    spec!("feature.thread", "Thread", r#"{"face": face name, "start": face name, "size": "M10", "pitch": expr = the coarse pitch of size, "length": expr, "leftHanded": boolean = false, "cosmetic": boolean = false, "name": string = automatic}"#,
        "Cuts an external ISO metric thread into a cylindrical face of a body: a bolt's shank, a stud. face is the cylinder by its name (document.inspect, faceNames); start is the flat face square to its axis at the end of the cylinder where the thread starts (the end of the shank: nothing of the body lies beyond it), by its name, a face of the same body. size is the designation of ISO 261, M and the nominal diameter in millimetres (M10, M2.5); the face has to be that diameter (a shank turned a little under size is threaded all the same, to the basic profile of the nominal size). pitch is the coarse pitch of ISO 261 for the size unless given (M10: 1.5 mm; a fine pitch such as 1.25 mm is given as pitch). length is how far the thread goes at full depth from start; after it the groove lifts out of the material over a quarter turn (the run-out), and the cylinder has to be long enough for both. The profile is the basic one of ISO 68-1: 60° flanks, the crest flat P/8 wide at the major diameter, the root flat P/4 wide at the minor diameter, D − 1.0825 P; its groove is swept along a helix (right-handed unless leftHanded) and subtracted from the body, which goes into this feature. The groove's faces are B-spline surfaces within half the document's tolerance of the true thread, one per half turn, and the root is a cylinder: F9:thread(0) the root, F9:thread(1) the flank that faces the start, F9:thread(2) the flank that faces away, the second and later of each numbered as pieces (F9:piece(3)/F9:thread(1)); the other faces are copies of the body's (F9:copy(0)/F2:side(e7)). cosmetic leaves the body as it is and only marks the thread. The result, and the feature in document.inspect, has the thread under thread: size, major and minor diameter, pitch, length, leftHanded, cosmetic, and how far the groove's faces were found from the true thread (deviation). feature.edit changes size, pitch, length, leftHanded and cosmetic. Refused, with the reason: a face that is not a cylinder or not of the size's diameter, a start face that is not square to the axis at an end of the cylinder, a thread longer than the cylinder.", doc: true, feature_thread),
];

/// A thread's designation, checked: `M10`.
pub(crate) fn size_value(command: &str, size: &str) -> Result<String, CommandError> {
    match nominal_diameter(size) {
        Some(_) => Ok(size.trim().to_string()),
        None => fail(format!(
            "{command}: `size`: `{}` is not an ISO metric size; give M and the nominal diameter in millimetres, such as M10",
            size.chars().take(40).collect::<String>()
        )),
    }
}

/// What a thread says of itself, for a command's result.
pub(crate) fn thread_json(report: &ThreadReport) -> Value {
    json!({
        "size": report.size,
        "major": report.major,
        "minor": report.minor,
        "pitch": report.pitch,
        "length": report.length,
        "leftHanded": report.left_handed,
        "cosmetic": report.cosmetic,
        "deviation": report.deviation,
    })
}

fn feature_thread(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.thread";
    let p = Params::new(
        ID,
        params,
        &[
            "face",
            "start",
            "size",
            "pitch",
            "length",
            "leftHanded",
            "cosmetic",
            "name",
        ],
    )?;
    let face = face_value(ID, "face", p.get("face").ok_or_else(|| p.missing("face"))?)?;
    let start = face_value(
        ID,
        "start",
        p.get("start").ok_or_else(|| p.missing("start"))?,
    )?;
    let size = size_value(ID, p.required_str("size")?)?;
    let pitch = match p.expr("pitch")? {
        Some(pitch) => pitch,
        None => {
            let nominal = nominal_diameter(&size).unwrap_or(0.0);
            match coarse_pitch(nominal) {
                Some(pitch) => format!("{pitch} mm"),
                None => {
                    return fail(format!(
                        "{ID}: {size} has no coarse pitch in ISO 261; give `pitch`"
                    ));
                }
            }
        }
    };
    let Some(length) = p.expr("length")? else {
        return Err(p.missing("length"));
    };
    let kind = FeatureKind::Thread(Thread {
        face,
        start,
        size,
        pitch: Expr::new(pitch),
        length: Expr::new(length),
        left_handed: p.bool("leftHanded")?.unwrap_or(false),
        cosmetic: p.bool("cosmetic")?.unwrap_or(false),
    });
    add_feature(session, ID, p.str("name")?, kind)
}

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
