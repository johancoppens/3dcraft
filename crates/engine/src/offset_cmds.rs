//! The offset of the closed profiles of a sketch: kerf compensation for a
//! laser, a wall drawn as one outline, clearance round a part.

use crate::commands::{Run, add_feature, fail, feature_ref, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Expr, FeatureKind, OffsetCorners, OffsetHead, OffsetKind, OffsetProfile};
use craft3d_regen::{OffsetResult, OffsetSource};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.offsetProfile", "Offset Profile", r#"{"sketch": id or name, "distance": expr, "corners": "round|sharp" = "round", "name": string = automatic}"#,
        "Offsets every closed profile of a sketch by distance, square to itself in the sketch's plane: a positive distance grows the material (outer loops out, holes in), a negative one shrinks it. It makes no body: an extrusion, a revolution or a sweep (as its profile) takes the offset in the place of a sketch, and file.export of it (as sketch) writes the offset outline, a laser's beam path with its kerf: offset by kerf/2. It is made at every regeneration from the solved sketch with its patterns, so it follows edits and a pattern's count. Lines stay lines and arcs arcs about the same centre (radius r ± d; an arc whose radius would no longer be positive is left out, and its neighbours meet by the corner rule). Where the offset curves of two neighbours part, corners round puts an arc of radius |distance| about the corner (the true offset: every point at the distance from the outline, the path of a round beam), sharp carries both on along their tangents to a mitre. Where they overlap they are trimmed where they meet; where the offset would cross itself only the part at the full distance from the outline is kept. A region that splits in two, or regions that join (a channel closing into a hole), are kept and reported under offset (loopsBefore, loopsAfter, regionsBefore, regionsAfter, wholeUpTo: the largest distance that keeps every loop and region as it is); a loop that would vanish (a hole that closes, a region shrunk to nothing) is refused with that largest distance. Splines are not supported yet. Names: what a sweep makes of it is named after the offset's curves, F10:side(F9:offset(e7)) (e7[3] for an instance of a pattern), the joint at a corner F10:side(F9:offsetCorner(e7,e8)) (the curves in the direction the loop runs), a further piece of one curve F10:side(F9:piece(1)/F9:offset(e7)), a cap after the curves of the sketch whose offset its loop is, F10:endCap(F9:offset(e1,e2,e7[*])). The result has the offset under offset: distance, curves, corners (the joints it added at corners), loopsBefore, loopsAfter, regionsBefore, regionsAfter, wholeUpTo and the names of its curves (the first 64). feature.edit changes sketch, distance and corners.", doc: true, feature_offset_profile),
];

/// The `corners` parameter; `None` when it is not given.
pub(crate) fn corners_param(
    p: &Params,
    command: &str,
) -> Result<Option<OffsetCorners>, CommandError> {
    match p.str("corners")? {
        None => Ok(None),
        Some("round") => Ok(Some(OffsetCorners::Round)),
        Some("sharp") => Ok(Some(OffsetCorners::Sharp)),
        Some(other) => fail(format!(
            "{command}: `corners` must be round or sharp, not `{}`",
            other.chars().take(40).collect::<String>()
        )),
    }
}

/// What an offset made, for results and document.inspect.
pub(crate) fn offset_json(made: &OffsetResult, feature: craft3d_doc::FeatureId) -> Value {
    let outline = &made.outline;
    let names: Vec<String> = outline
        .origins
        .iter()
        .take(64)
        .map(|origin| {
            let (kind, curves) = match origin.source {
                OffsetSource::Curve(c) => (OffsetKind::Offset, vec![c]),
                OffsetSource::Corner(a, b) => (OffsetKind::Corner, vec![a, b]),
            };
            let head = OffsetHead {
                feature,
                kind,
                piece: origin.piece,
            };
            let curves: Vec<String> = curves
                .iter()
                .map(|c| {
                    craft3d_doc::CurveRef::decode(*c)
                        .map_or_else(|| format!("e{c}"), |c| c.to_string())
                })
                .collect();
            let piece = if head.piece > 0 {
                format!("{feature}:piece({})/", head.piece)
            } else {
                String::new()
            };
            format!("{piece}{feature}:{}({})", kind.name(), curves.join(","))
        })
        .collect();
    json!({
        "distance": made.distance,
        "curves": outline.curves.len(),
        // The joints at corners where the offset curves part.
        "corners": outline
            .origins
            .iter()
            .filter(|o| matches!(o.source, OffsetSource::Corner(..)))
            .count(),
        "loopsBefore": outline.loops_before,
        "loopsAfter": outline.loops_after,
        "regionsBefore": outline.regions_before,
        "regionsAfter": outline.regions_after,
        "wholeUpTo": outline.whole_up_to,
        "names": names,
    })
}

fn feature_offset_profile(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.offsetProfile";
    let p = Params::new(ID, params, &["sketch", "distance", "corners", "name"])?;
    let doc = session.document()?.history.current();
    let sketch = feature_ref(doc, &p, ID, "sketch")?;
    if let Some(feature) = doc.feature(sketch)
        && !matches!(feature.kind, FeatureKind::Sketch(_))
    {
        return fail(format!(
            "{ID}: `{}` is a {}; an offset is made from the profiles of a sketch",
            feature.name,
            feature.kind.type_name()
        ));
    }
    doc.sketch(sketch)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let Some(distance) = p.expr("distance")? else {
        return Err(p.missing("distance"));
    };
    let corners = corners_param(&p, ID)?.unwrap_or_default();
    let kind = FeatureKind::OffsetProfile(OffsetProfile {
        sketch,
        distance: Expr::new(distance),
        corners,
    });
    add_feature(session, ID, p.str("name")?, kind)
}
