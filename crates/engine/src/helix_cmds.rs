//! Helices and threads: a datum curve about an axis that a sweep can
//! follow (a coil spring), and an ISO metric thread cut into a
//! cylindrical face by the groove of its profile swept along a helix.

use crate::commands::{Run, add_feature, fail, spec};
use crate::datum_cmds::{axis_value, face_value, point_value, store};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{DatumHelix, Expr, FeatureKind, HelixExtent, HelixStart, Thread, ThreadSize};
use craft3d_regen::{ThreadReport, coarse_pitch, no_coarse_pitch, nominal_diameter};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("datum.helix", "Helix", r#"{"axis": "x|y|z" or [face name, face name] or datum axis, "radius": expr, "start": point, "pitch": expr, "turns": expr, "height": expr, "leftHanded": boolean = false, "replace": id or name, "name": string = automatic, "when": expr = always on}"#,
        "Adds a helix: a curve that winds about an axis at a constant radius and advances one pitch along it per turn, for a sweep to follow (feature.sweep, path): a coil spring, the groove of a thread. It makes no body; it is drawn as a curve. axis is a world axis (through the origin), the straight edge between two faces of one body ([face name, face name]) or a datum axis (id or name); the helix advances the way it points. Where it starts, one of two: radius, a length, starts it level with the axis's own point (the origin for a world axis), towards the first of the world's x, y and z axes that is least along the axis (x for z, y for x, x for y); start, a point ([x, y, z], a vertex name, {\"center\": [face name, face name]} or a datum point), starts it there, at that point's distance from the axis. pitch is how far it advances per turn; how far it goes is turns (a number, need not be whole) or height (a length along the axis). It turns counter-clockwise seen from the tip of the axis, or clockwise with leftHanded. Lengths and turns are expressions and follow variables; feature.edit changes radius, pitch, turns, height and leftHanded. replace gives an existing helix this definition instead of adding one. The result has the helix under datum: origin (on the axis, level with the start), axis, start and end points, radius, pitch, turns, height, length (turns · √((2πr)² + p²)) and leftHanded. A helix is not a rational curve: where a curve is needed it is a polynomial B-spline within a quarter of the document's tolerance.", doc: true, names: ["axis"], datum_helix),
    spec!("feature.thread", "Thread", r#"{"face": face name, "start": face name, "size": "M10" or expr = the size of the face, "pitch": expr = the coarse pitch of size, "length": expr, "leftHanded": boolean = false, "modeled": boolean = false, "cosmetic": boolean = true, "name": string = automatic, "when": expr = always on}"#,
        "Puts an ISO metric thread on a cylindrical face of a body: external on a shank (a bolt, a stud), internal in a hole (a tapped hole, a nut), which the face says: a face whose material is outside it is a hole. The thread is cosmetic unless modeled is true, as in other modellers: the body is left as it is and the face is marked with the thread (size, diameters, pitch, length), which is fast and what most fasteners want; with modeled the groove is cut into the body (slow: seconds to regenerate). cosmetic is the opposite of modeled, accepted as well (cosmetic: false is modeled: true); giving both, disagreeing, is refused. face is the cylinder by its name (document.inspect, faceNames); start is the flat face square to its axis at the end of the cylinder where the thread starts (the end of the shank: nothing of the body lies beyond it), by its name, a face of the same body; both may be named as an earlier body had them (F2:face(0) after a chamfer took the body), and are followed to its newest form. size is the designation of ISO 261, M and the nominal diameter in millimetres (M10, M2.5), or the nominal diameter as an expression of a length (d, \"10 mm\"), or left out: then the thread takes the ISO 261 size whose nominal diameter the face has, found again at every regeneration, so a thread on a shank made from a variable follows it (a face of a diameter no size of the table has fails, naming the diameter and the nearest sizes). A text that reads as a designation (M and a number) is one; any other text is an expression. The face has to be the size's diameter (a shank turned a little under size is threaded all the same, to the basic profile of the nominal size); a hole is the tap drill's, D − P (M10: 8.5 mm), or anything from the minor diameter up to it, and a hole left without a size takes the size whose tap drill it is nearest of those it fits. ISO 261 is known from M1 to M68: the coarse pitches of every size that has one (M11: 1.5 mm), and for the sizes with fine pitches only (M15, M25, …) a pitch has to be given (the refusal names the fine pitches). pitch is the coarse pitch of ISO 261 for the size unless given (M10: 1.5 mm; a fine pitch such as 1.25 mm is given as pitch), looked up again at every regeneration, so it follows the size; a pitch given stays. length is how far the thread goes at full depth from start; after it the groove lifts out of the material over a quarter turn (the run-out), and the cylinder has to be long enough for both. The profile is the basic one of ISO 68-1: 60° flanks, the crest flat P/8 wide at the major diameter, the root flat P/4 wide at the minor diameter, D − 1.0825 P; an internal thread's groove is the nut's: its root a flat P/8 wide at the major diameter, its flanks 3P/4 apart at the minor diameter, cut outwards into the wall and running out into the hole; when modeled, its groove is swept along a helix (right-handed unless leftHanded) and subtracted from the body; the body goes into this feature either way. The groove's faces are B-spline surfaces within half the document's tolerance of the true thread, one per half turn, and the root is a cylinder: F9:thread(0) the root, F9:thread(1) the flank that faces the start, F9:thread(2) the flank that faces away, the second and later of each numbered as pieces (F9:piece(3)/F9:thread(1)); the other faces are copies of the body's (F9:copy(0)/F2:side(e7)). A cosmetic thread leaves every face a copy of the body's. The result, and the feature in document.inspect, has the thread under thread: size (the designation, also when it was found from a diameter), major and minor diameter, pitch, pitchGiven (false when it is the coarse pitch of the size), length, leftHanded, modeled, cosmetic, internal, tapDrill (D − P, for an internal thread; null otherwise), and how far the groove's faces were found from the true thread (deviation). feature.edit changes size, pitch, length, leftHanded and modeled (or cosmetic); size null goes back to the size of the face, pitch null to the coarse pitch of the size. Refused, with the reason: a face that is not a cylinder or not of the size's diameter, a start face that is not square to the axis at an end of the cylinder, a thread longer than the cylinder.", doc: true, names: ["face", "start"], feature_thread),
];

/// A thread's size as the commands take it: a text that reads as a
/// designation of ISO 261 (`M10`: M and a number) is one, any other text
/// is the nominal diameter as an expression of a length (`d`). A text
/// that starts like a designation but is none (`M10x1.25`) is refused, as
/// is anything that is not a text.
pub(crate) fn size_value(command: &str, size: &Value) -> Result<ThreadSize, CommandError> {
    let Value::String(text) = size else {
        return fail(format!(
            "{command}: `size` must be a designation of ISO 261 such as \"M10\", or a length as text such as \"d\""
        ));
    };
    if nominal_diameter(text).is_some() {
        return Ok(ThreadSize::Designation(text.trim().to_string()));
    }
    let trimmed = text.trim();
    let mut chars = trimmed.chars();
    let designation_like =
        matches!(chars.next(), Some('M' | 'm')) && chars.next().is_some_and(|c| c.is_ascii_digit());
    if designation_like || trimmed.is_empty() {
        return fail(format!(
            "{command}: `size`: `{}` is not an ISO metric size; give M and the nominal diameter in millimetres, such as M10, or a length such as d",
            text.chars().take(40).collect::<String>()
        ));
    }
    Ok(ThreadSize::Diameter {
        diameter: Expr::new(trimmed),
    })
}

/// Whether a thread is modelled, from `modeled` or its opposite
/// `cosmetic` (the name of version 13, still accepted); `None` when
/// neither is given. Both given and disagreeing is refused.
pub(crate) fn modeled_value(command: &str, p: &Params) -> Result<Option<bool>, CommandError> {
    match (p.bool("modeled")?, p.bool("cosmetic")?) {
        (Some(modeled), Some(cosmetic)) if modeled == cosmetic => fail(format!(
            "{command}: `modeled` and `cosmetic` say the opposite of each other; give one: a thread is cosmetic unless modeled"
        )),
        (Some(modeled), _) => Ok(Some(modeled)),
        (None, Some(cosmetic)) => Ok(Some(!cosmetic)),
        (None, None) => Ok(None),
    }
}

/// What a thread says of itself, for a command's result.
pub(crate) fn thread_json(report: &ThreadReport) -> Value {
    json!({
        "size": report.size,
        "major": report.major,
        "minor": report.minor,
        "pitch": report.pitch,
        "pitchGiven": report.pitch_given,
        "length": report.length,
        "leftHanded": report.left_handed,
        "modeled": !report.cosmetic,
        "cosmetic": report.cosmetic,
        "deviation": report.deviation,
        "internal": report.internal,
        "tapDrill": report.internal.then_some(report.major - report.pitch),
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
            "modeled",
            "cosmetic",
            "name",
        ],
    )?;
    let modeled = modeled_value(ID, &p)?.unwrap_or(false);
    let face = face_value(ID, "face", p.get("face").ok_or_else(|| p.missing("face"))?)?;
    let start = face_value(
        ID,
        "start",
        p.get("start").ok_or_else(|| p.missing("start"))?,
    )?;
    let size = match p.get("size") {
        Some(value) => Some(size_value(ID, value)?),
        None => None,
    };
    let pitch = p.expr("pitch")?.map(Expr::new);
    // A designation the table has no coarse pitch for needs one given;
    // for a diameter or the face's size, regeneration says so.
    if pitch.is_none()
        && let Some(ThreadSize::Designation(text)) = &size
        && let nominal = nominal_diameter(text).unwrap_or(0.0)
        && coarse_pitch(nominal).is_none()
    {
        return fail(format!("{ID}: {}", no_coarse_pitch(text.trim(), nominal)));
    }
    let Some(length) = p.expr("length")? else {
        return Err(p.missing("length"));
    };
    let kind = FeatureKind::Thread(Thread {
        face,
        start,
        size,
        pitch,
        length: Expr::new(length),
        left_handed: p.bool("leftHanded")?.unwrap_or(false),
        modeled,
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
