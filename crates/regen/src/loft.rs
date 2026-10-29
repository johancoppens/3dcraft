//! Lofts in regeneration: the one closed profile of each section's solved
//! sketch, in the order of the feature, goes to the kernel (or, where a
//! sketch's curves make one open chain instead, that chain: a loft of open
//! sections is a sheet); the body it makes is searched for places where it
//! passes through itself before it is handed on.

use crate::extrude::{drawn, swept};
use crate::selfcheck::{Crossing, crossing};
use crate::{FeatureResult, Names};
use craft3d_doc::{Document, FeatureId, Loft, MAX_LOFT_SECTIONS, NameRole, PersistentName};
use craft3d_kernel::extrude::{Origin, Role};
use craft3d_kernel::loft::{Options, Section, loft};
use craft3d_kernel::profile::open_chain;
use craft3d_topo::Body;

/// A loft with guides: the network surface of its sections (open chains)
/// as u curves and its guides as v curves.
fn guided(
    doc: &Document,
    variables: &craft3d_expr::Evaluation,
    id: FeatureId,
    feature: &Loft,
    earlier: &[FeatureResult],
) -> Result<(Body, Names), String> {
    if feature.ruled || feature.closed {
        return Err(
            "guides: a loft with guides is smooth and open (no ruled, no closed)".to_string(),
        );
    }
    let most = craft3d_doc::MAX_NETWORK_CURVES;
    let (m, n) = (feature.sections.len(), feature.guides.len());
    if !(2..=most).contains(&m) || !(2..=most).contains(&n) {
        return Err(format!(
            "sections, guides: a loft with guides has 2 to {most} sections and 2 to {most} guides; this one has {m} sections and {n} guides"
        ));
    }
    // An open section is run the way the guides cross it, so a curve to
    // start from means nothing here: refused rather than ignored.
    if let Some(k) = feature.sections.iter().position(|s| s.start.is_some()) {
        return Err(format!(
            "sections: section {} gives `start`, which a loft with guides does not use (its open sections are run the way the guides cross them); leave it out",
            k + 1
        ));
    }
    let network = craft3d_doc::SurfaceNetwork {
        u: feature
            .sections
            .iter()
            .map(|s| craft3d_doc::Rail::Sketch(s.sketch))
            .collect(),
        v: feature.guides.clone(),
        edges: craft3d_doc::NetworkEdges::default(),
    };
    let (body, names, _) = crate::network::build(doc, variables, id, &network, earlier)
        .map_err(|e| guided_words(&e))?;
    Ok((body, names))
}

/// A network's message in a loft's words: its u curves are the sections,
/// its v curves the guides.
fn guided_words(text: &str) -> String {
    text.replace("u, v: ", "sections, guides: ")
        .replace("u: curve", "sections: section")
        .replace("v: curve", "guides: guide")
        .replace("u curves", "sections")
        .replace("v curves", "guides")
        .replace("u curve", "section")
        .replace("v curve", "guide")
}

/// The persistent name of a face a loft made: `F9:side(e3,e7)` after the
/// matched curves of the sections it passes through, `F9:startCap(…)` and
/// `F9:endCap(…)` after the curves of the first and the last section.
fn name_of(feature: FeatureId, origin: &Origin) -> PersistentName {
    let role = match origin.role {
        Role::StartCap => NameRole::StartCap,
        Role::EndCap => NameRole::EndCap,
        // A loft makes faces only; anything else would be a side.
        _ => NameRole::Side,
    };
    PersistentName::new(feature, role, origin.sources.clone())
}

pub(crate) fn build(
    doc: &Document,
    variables: &craft3d_expr::Evaluation,
    id: FeatureId,
    feature: &Loft,
    earlier: &[FeatureResult],
) -> Result<(Body, Names), String> {
    if !feature.guides.is_empty() {
        return guided(doc, variables, id, feature, earlier);
    }
    let count = feature.sections.len();
    let least = if feature.closed { 3 } else { 2 };
    if count < least || count > MAX_LOFT_SECTIONS {
        return Err(format!(
            "sections: a {}loft passes through at least {least} sketches and at most {MAX_LOFT_SECTIONS}, this one has {count}",
            if feature.closed { "closed " } else { "" }
        ));
    }
    let mut sections = Vec::with_capacity(count);
    // Per section, why its sketch is no closed profile when it is an open
    // chain instead.
    let mut open: Vec<Option<String>> = Vec::with_capacity(count);
    let mut tolerance = None;
    for (index, section) in feature.sections.iter().enumerate() {
        let number = index + 1;
        if let Some(again) = feature
            .sections
            .iter()
            .take(index)
            .position(|s| s.sketch == section.sketch)
        {
            let name = doc
                .feature(section.sketch)
                .map_or_else(|| section.sketch.to_string(), |f| f.name.clone());
            return Err(format!(
                "sections: `{name}` is section {} and section {number}; a loft passes through each sketch once",
                again + 1
            ));
        }
        if doc
            .feature(section.sketch)
            .is_some_and(|f| matches!(f.kind, craft3d_doc::FeatureKind::OffsetProfile(_)))
        {
            return Err(format!(
                "sections: section {number}: an offset as a section of a loft is not supported yet; loft the sketch"
            ));
        }
        // What `swept` says of a sketch, as what it says of this section;
        // a sketch whose curves make one open chain instead is an open
        // section (a loft of open sections is a sheet).
        let made = match swept(doc, earlier, section.sketch, "loft") {
            Ok(made) => made,
            Err(reason) => {
                let reason = reason.strip_prefix("sketch: ").unwrap_or(&reason);
                let chain = drawn(doc, earlier, section.sketch)
                    .map(|drawn| (open_chain(&drawn.curves, &drawn.tolerance), drawn));
                let Ok((Ok(outline), drawn)) = chain else {
                    return Err(format!("sections: section {number}: {reason}"));
                };
                let name = drawn.name;
                if !drawn.solved.curve_ids.is_empty() {
                    return Err(format!(
                        "sections: section {number}: sketch `{name}` has nested patterns; a section of a loft made by a pattern of a pattern is not supported yet"
                    ));
                }
                if let Some(start) = section.start
                    && !outline.segments.iter().any(|s| s.source == start.0)
                {
                    return Err(format!(
                        "sections: section {number}: {start} is not a curve of the open chain of sketch `{name}`; `start` names the curve at the end the chain starts from"
                    ));
                }
                tolerance = Some(drawn.tolerance);
                sections.push(Section {
                    plane: drawn.plane,
                    outline,
                    start: section.start.map(|e| e.0),
                });
                open.push(Some(reason.to_string()));
                continue;
            }
        };
        let name = made.name;
        if !made.solved.curve_ids.is_empty() {
            return Err(format!(
                "sections: section {number}: sketch `{name}` has nested patterns; a section of a loft made by a pattern of a pattern is not supported yet"
            ));
        }
        let [profile] = made.profiles.as_slice() else {
            return Err(format!(
                "sections: section {number}: sketch `{name}` has {} closed profiles; a section of a loft is one closed loop",
                made.profiles.len()
            ));
        };
        if !profile.holes.is_empty() {
            return Err(format!(
                "sections: section {number}: the profile of sketch `{name}` has a hole; a section of a loft is one closed loop (loft the hole on its own and cut it out)"
            ));
        }
        if let Some(start) = section.start
            && !profile.outer.segments.iter().any(|s| s.source == start.0)
        {
            return Err(format!(
                "sections: section {number}: {start} is not a curve of the profile of sketch `{name}`; `start` names the curve the loop starts with"
            ));
        }
        tolerance = Some(made.tolerance);
        sections.push(Section {
            plane: made.plane,
            outline: profile.outer.clone(),
            start: section.start.map(|e| e.0),
        });
        open.push(None);
    }
    // Closed and open sections mixed: most likely a profile left open by
    // mistake, so the open one is named with why it is no closed profile.
    if let (Some(at), Some(closed)) = (
        open.iter().position(Option::is_some),
        open.iter().position(Option::is_none),
    ) && let Some(Some(reason)) = open.get(at)
    {
        return Err(format!(
            "sections: section {}: {reason}; its curves make an open chain, but section {} is a closed loop: the sections of a loft are all closed loops (a solid) or all open chains (a sheet)",
            at + 1,
            closed + 1
        ));
    }
    let tolerance =
        tolerance.ok_or_else(|| "sections: a loft needs sketches to pass through".to_string())?;
    let options = Options {
        ruled: feature.ruled,
        closed: feature.closed,
    };
    let made = loft(&sections, &options, &tolerance).map_err(|e| format!("sections: {e}"))?;
    let mut faces: Vec<PersistentName> = made.faces.iter().map(|o| name_of(id, o)).collect();
    // A cap whose loop holds a whole run of a sketch pattern names it once
    // (`e7[*]`): the start cap after the first section, the end cap after
    // the last.
    if let (Some(first), Some(last)) = (feature.sections.first(), feature.sections.last()) {
        crate::collapse_runs(
            doc,
            earlier,
            first.sketch,
            &mut faces,
            &[NameRole::StartCap],
        );
        crate::collapse_runs(doc, earlier, last.sketch, &mut faces, &[NameRole::EndCap]);
    }

    // A loft whose sections are matched with too much twist, or that
    // bends back into itself, is a valid body in how it hangs together
    // and passes through itself all the same. It is never handed on.
    let advice = "give a section another `start` (the curve its loop starts with), or move the sections apart";
    match crossing(&made.body, &faces, tolerance.linear) {
        Crossing::None => {}
        Crossing::Spent => return Err(Crossing::spent("sections", "loft")),
        Crossing::Unknown(error) => {
            return Err(format!(
                "sections: the loft cannot be meshed to see whether it passes through itself ({error}); its sections are probably matched with a twist: {advice}"
            ));
        }
        touching @ Crossing::Touching(..) => {
            touching.refused("sections", "loft", advice)?;
        }
        Crossing::Found(pair) => {
            let between = pair
                .map(|[a, b]| format!(": {a} and {b} cross"))
                .unwrap_or_default();
            return Err(format!(
                "sections: the loft passes through itself{between}; {advice}"
            ));
        }
    }

    let names = Names::by_faces(id, &made.body, faces, tolerance.linear)?;
    Ok((made.body, names))
}
