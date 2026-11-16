//! The tip: a flat end face of a body replaced by faces that close the
//! body in one point (`feature.tip`; docs/design/sweeps.md › *A tip on a
//! body*).
//!
//! The face is given by its persistent name from `document.inspect`.
//! Whether it can be closed, and how far, is found out by regeneration,
//! like every other parameter: the feature is stored and reports `failed`
//! with the reason.

use crate::commands::{Run, add_feature, fail, spec};
use crate::datum_cmds::face_value;
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Expr, FeatureKind, Tip};
use serde_json::Value;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.tip", "Tip", r#"{"face": face name, "length": expr = automatic, "at": [expr, expr] = the middle, "fullness": expr = 0.4142, "shape": "round" = "round", "name": string = automatic, "when": expr = always on}"#,
        "Adds a feature that closes a body in one rounded point over a flat end face: the face is replaced by faces that run on from the faces round it without a crease along the old edge and all reach the point with one tangent plane, parallel to the end. A loft's end, a swept or extruded spline's: the nose of a hull, the end of a slender blade. Every edge that ran into the end (a deck edge, a trailing edge) runs on to the point and fades out there. face is the end face by its name (faceNames in document.inspect, or view.pick), which may be named as an earlier body of the same chain had it; the body goes into this feature, and nothing behind the end changes. length is how far beyond the end the point lies, along the end's outward normal; left out, it is the radius of the circle with the end's area, never more than half the length at which the faces round the end, carried on, would meet (found again at every regeneration). at is where the point stands over the end, as lengths along the end face's own x and y axes from its plane's origin; left out, over the middle (the centre of area) of what the faces round the end come to in the point's plane. fullness is how full the tip is across, a number from 0.01 to 0.98: 0.4142 (the default, √2 − 1) gives quarter ellipses, so a rod with its end tipped as long as its radius ends in a half ball and any straight prism gains two thirds of its end's area times the length; 0.5 gives parabolas (eleven fifteenths); more is blunter, less more pointed. What is built: fullness 0.05 to 0.95 at lengths from a tenth of the radius of the circle of the end's area; nearer 0 or 1, and tips shorter than that, are often refused (their faces cannot be meshed or measured yet), and below a hundredth of that radius nearly always. shape is round; pointed (a straight cone, creased at the end's edge) is not built yet. The faces that stay keep their names with a prefix, \"F9:copy(0)/...\"; a face of the tip is named after the face it continues, \"F9:tip(0)/F2:side(e7)\"; the edges from the old corners to the point and the point itself are named by their faces. Refused, with the face or the edge and the reason: a face that is not flat, a face with a hole, a face bounded by one closed edge, a face whose neighbours are planes, cylinders, cones, spheres or tori (only spline faces round the end are taken yet: a loft's, a swept or turned spline's), an edge that is not a whole border of the face behind it, faces round the end that do not go on toward the point, an end whose outline no point sees whole (a crescent, a cambered blade section such as a ship propeller's: a tip that closes in one point needs a point that sees all of it), a point that stands where it does not see the whole end or so near its edge that it cannot be proven to, a length at which the faces round the end close in before the point (give a shorter one), a tip so short that it is not tangent to the faces round it within the angular tolerance or cannot be measured (give a longer one, or a fullness nearer the middle), and a tip that would run into another part of the body. A tipped body cannot yet be hollowed or offset: feature.shell, feature.offsetFace, feature.thicken and feature.plate refuse a face that closes in a point. Change it with feature.edit: face, length, at, fullness (length, at and fullness null go back to automatic).", doc: true, names: ["face"], feature_tip),
];

/// `at`: two expressions `[x, y]`.
fn at_value(p: &Params, command: &str) -> Result<Option<[Expr; 2]>, CommandError> {
    let Some(value) = p.get("at") else {
        return Ok(None);
    };
    let wrong = || {
        CommandError::new(format!(
            "{command}: `at` must be an array of two expressions [x, y]: lengths along the end face's own axes"
        ))
    };
    let Value::Array(items) = value else {
        return Err(wrong());
    };
    let [x, y] = items.as_slice() else {
        return Err(wrong());
    };
    Ok(Some([
        Expr::new(p.expr_of("at", x)?),
        Expr::new(p.expr_of("at", y)?),
    ]))
}

/// `shape`: only round is built.
fn shape_value(p: &Params, command: &str) -> Result<(), CommandError> {
    match p.str("shape")? {
        None | Some("round") => Ok(()),
        Some("pointed") => fail(format!(
            "{command}: `shape`: a pointed tip (a straight cone over the end, creased at its edge) is not built yet; leave shape out for a round one"
        )),
        Some(other) => fail(format!(
            "{command}: `shape` must be \"round\", got \"{}\"",
            other.chars().take(40).collect::<String>()
        )),
    }
}

fn feature_tip(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.tip";
    let p = Params::new(
        ID,
        params,
        &["face", "length", "at", "fullness", "shape", "name"],
    )?;
    // The document has to be there before anything is said of the
    // parameters.
    session.document()?;
    let Some(value) = p.get("face") else {
        return Err(p.missing("face"));
    };
    let face = face_value(ID, "face", value)?;
    shape_value(&p, ID)?;
    let kind = FeatureKind::Tip(Tip {
        face,
        length: p.expr("length")?.map(Expr::new),
        at: at_value(&p, ID)?,
        fullness: p.expr("fullness")?.map(Expr::new),
    });
    add_feature(session, ID, p.str("name")?, kind)
}

/// `feature.edit` for a tip: `face`, `at` and `shape` (length and
/// fullness are plain expressions, set where the others are).
pub(crate) fn edit(p: &Params, tipped: &mut Tip) -> Result<(), CommandError> {
    const ID: &str = "feature.edit";
    if let Some(value) = p.get("face") {
        tipped.face = face_value(ID, "face", value)?;
    }
    if p.has("at") {
        tipped.at = at_value(p, ID)?;
    }
    shape_value(p, ID)
}
