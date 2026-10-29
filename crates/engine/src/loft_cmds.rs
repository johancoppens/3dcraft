//! The loft: a solid through the closed profiles of a row of sketches.

use crate::commands::{Run, add_sweep, combine_param, fail, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    Document, EntityId, FeatureId, FeatureKind, Loft, LoftSection, MAX_LOFT_SECTIONS,
};
use serde_json::Value;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.loft", "Loft", r#"{"sections": [sketch id or name, or {sketch: id or name, start: curve id}; 2 to 256], "ruled": boolean = false, "closed": boolean = false, "guides": [curve, ...] = none, "cut": id or name, "join": id or name, "intersect": id or name, "name": string = automatic, "when": expr = always on}"#,
        "With guides (2 or more curves, each a sketch's open chain or {\"edges\": [...]}, as feature.surface.network takes them) the sections must be open chains and the loft is the network surface of the sections as u curves and the guides as v curves: every section ends on the first and the last guide, which are the sheet's edges, and crosses the others; such a loft is smooth and open (no ruled, no closed) and its sections take no start. Adds a solid through the closed profiles of two or more sketches, in the order given: its side passes through every profile, its ends are the first and the last. Each sketch holds one closed loop (lines, arcs, splines, or a circle), on any planes that lie one after the other (offset or datum planes; they need not be parallel, but no profile may reach the plane of the next). The curves of the profiles are matched in order: every loop is walked the same way round, the first section from its curve with the smallest id and each later one turned to lie nearest to the section before; start (a curve of that sketch, \"e7\") says where a loop starts instead, which is how a twist is given or taken out. Sections with different numbers of curves (a square to a circle) are cut where the others have their corners, by length. Each column of matched curves gives a face named after them (F9:side(e3,e7)); the ends are F9:startCap and F9:endCap. Smooth through three or more sections unless ruled (straight from each section to the next); closed joins the last section to the first, a ring without ends (3 sections or more). A loft that would pass through itself fails and says so. Sketches whose curves make one open chain instead (lines, arcs and splines end to end, two free ends) give a sheet, not a solid: no ends, the chain walked from the free end of its end curve with the smaller id (or from start, a curve at either end), each later section the way round that lies nearest to the one before; the sheet faces along the chain's direction × the loft's direction, so a subtract of it from a solid keeps the side it faces and an intersect the other (a terrain of sections across x drawn towards +y faces down: subtract keeps what is under it). All sections are closed or all open. cut, join and intersect combine the result with a body at once, as for feature.extrude (a sheet cuts or intersects; it does not join).", doc: true, feature_loft),
];

/// A feature by its id (`7` or `"F7"`) or its name.
fn feature_of(doc: &Document, command: &str, value: &Value) -> Result<FeatureId, CommandError> {
    let by_id = |n: u64| {
        doc.feature(FeatureId(n))
            .map(|f| f.id)
            .ok_or_else(|| CommandError::new(format!("{command}: there is no feature F{n}")))
    };
    match value {
        Value::Number(n) => match n.as_u64() {
            Some(n) => by_id(n),
            None => fail(format!(
                "{command}: a section's sketch must be a feature id or name, got {n}"
            )),
        },
        Value::String(text) => {
            if let Some(feature) = doc.feature_by_name(text) {
                return Ok(feature.id);
            }
            match text.strip_prefix('F').and_then(|d| d.parse::<u64>().ok()) {
                Some(n) => by_id(n),
                None => fail(format!("{command}: there is no feature named `{text}`")),
            }
        }
        _ => fail(format!(
            "{command}: a section's sketch must be a feature id or name"
        )),
    }
}

/// The `sections` of a loft: a list of sketches, each given as an id or
/// name, or as `{"sketch": .., "start": "e7"}`.
pub(crate) fn loft_sections(
    doc: &Document,
    command: &str,
    value: &Value,
) -> Result<Vec<LoftSection>, CommandError> {
    let Value::Array(items) = value else {
        return fail(format!(
            "{command}: `sections` must be a list of sketches, in the order the loft passes through them"
        ));
    };
    if items.len() < 2 || items.len() > MAX_LOFT_SECTIONS {
        return fail(format!(
            "{command}: `sections` lists at least 2 sketches and at most {MAX_LOFT_SECTIONS}, got {}",
            items.len()
        ));
    }
    let mut sections = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let number = index + 1;
        let (sketch, start) = match item {
            Value::Object(map) => {
                if let Some(unknown) = map
                    .keys()
                    .find(|k| !["sketch", "start"].contains(&k.as_str()))
                {
                    return fail(format!(
                        "{command}: section {number} has no parameter `{unknown}`; a section is a sketch, or {{\"sketch\": .., \"start\": ..}}"
                    ));
                }
                let Some(sketch) = map.get("sketch") else {
                    return fail(format!("{command}: section {number} is missing `sketch`"));
                };
                let start = match map.get("start") {
                    None | Some(Value::Null) => None,
                    Some(Value::Number(n)) => match n.as_u64() {
                        Some(id) => Some(EntityId(id)),
                        None => {
                            return fail(format!(
                                "{command}: section {number}: `start` must be a curve of the sketch, such as \"e7\""
                            ));
                        }
                    },
                    Some(Value::String(text)) => {
                        match text.strip_prefix('e').unwrap_or(text).parse::<u64>() {
                            Ok(id) => Some(EntityId(id)),
                            Err(_) => {
                                return fail(format!(
                                    "{command}: section {number}: `start` must be a curve of the sketch, such as \"e7\", got `{text}`"
                                ));
                            }
                        }
                    }
                    Some(_) => {
                        return fail(format!(
                            "{command}: section {number}: `start` must be a curve of the sketch, such as \"e7\""
                        ));
                    }
                };
                (sketch, start)
            }
            other => (other, None),
        };
        let sketch = feature_of(doc, command, sketch)?;
        let feature = doc
            .feature(sketch)
            .ok_or_else(|| CommandError::new(format!("{command}: there is no feature {sketch}")))?;
        if !matches!(feature.kind, FeatureKind::Sketch(_)) {
            return fail(format!(
                "{command}: section {number}: `{}` is a {}, not a sketch",
                feature.name,
                feature.kind.type_name()
            ));
        }
        if let Some(again) = sections
            .iter()
            .position(|s: &LoftSection| s.sketch == sketch)
        {
            return fail(format!(
                "{command}: `{}` is section {} and section {number}; a loft passes through each sketch once",
                feature.name,
                again + 1
            ));
        }
        sections.push(LoftSection { sketch, start });
    }
    Ok(sections)
}

fn feature_loft(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.loft";
    let p = Params::new(
        ID,
        params,
        &[
            "sections",
            "ruled",
            "closed",
            "guides",
            "cut",
            "join",
            "intersect",
            "name",
        ],
    )?;
    let doc = session.document()?.history.current();
    let sections = loft_sections(
        doc,
        ID,
        p.get("sections").ok_or_else(|| p.missing("sections"))?,
    )?;
    let closed = p.bool("closed")?.unwrap_or(false);
    if closed && sections.len() < 3 {
        return fail(format!(
            "{ID}: a closed loft passes through at least 3 sketches, got {}",
            sections.len()
        ));
    }
    let combine = combine_param(doc, &p, ID)?;
    let guides = if p.has("guides") {
        crate::surface_cmds::curves_value(doc, &p, ID, "guides")?
    } else {
        Vec::new()
    };
    let ruled = p.bool("ruled")?.unwrap_or(false);
    if !guides.is_empty() && (ruled || closed) {
        return fail(format!(
            "{ID}: a loft with guides is smooth and open: it takes no `ruled` or `closed`"
        ));
    }
    if !guides.is_empty()
        && let Some(k) = sections.iter().position(|s| s.start.is_some())
    {
        return fail(format!(
            "{ID}: section {}: a loft with guides takes no `start` (its open sections are run the way the guides cross them)",
            k + 1
        ));
    }
    let kind = FeatureKind::Loft(Loft {
        sections,
        ruled,
        closed,
        guides,
    });
    add_sweep(session, ID, p.str("name")?, kind, combine)
}
