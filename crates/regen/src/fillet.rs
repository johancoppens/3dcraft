//! Fillets in regeneration: edges of the body of an earlier feature are
//! replaced by faces tangent to both their neighbours, with one radius.
//! How an edge is referred to and how the result is named is in
//! [`crate::blend`], which chamfers share.

use crate::blend::{self, Picked, Roles};
use crate::datum::named_body_of;
use crate::derive::source_body;
use crate::{FadeReport, FeatureResult, Names, length};
use craft3d_doc::{Document, FeatureId, Fillet, NameRole, identity};
use craft3d_expr::Evaluation;
use craft3d_kernel::fillet::{
    self as kernel, Fade, FadeFault, FilletEdge, FilletFace, FilletLawEdge, RadiusLaw,
};
use craft3d_kernel::{KernelError, Shown};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, EdgeId, FaceId};

const ROLES: Roles = Roles {
    face: NameRole::Fillet,
    corner: NameRole::FilletCorner,
    noun: "fillet",
};

/// A kernel error in the words of the document: the reference, the names
/// of the faces, the parameter.
fn explain(
    error: KernelError,
    fillet: &Fillet,
    picked: &[Picked],
    names: &Names,
    unit: &str,
) -> String {
    let reference = |edge: EdgeId| picked.iter().find(|p| p.edge == edge);
    let role = |edge: EdgeId| match reference(edge) {
        Some(pick) => blend::role_of(&fillet.edges, pick.reference),
        None => "edges".to_string(),
    };
    let face_name = |face: FaceId| {
        names
            .faces
            .get(face.0 as usize)
            .map_or_else(|| format!("face {}", face.0), ToString::to_string)
    };
    match error {
        KernelError::FilletTooLarge {
            edge,
            face,
            radius,
            largest,
            shown: Shown::Faces,
        } => format!(
            "radius: too large for {}: with a radius of {} {unit} the line along which the fillet touches the face {} leaves that face; the largest radius that fits there is below {} {unit}",
            role(edge),
            craft3d_kernel::short_number(radius),
            face_name(face),
            craft3d_kernel::fits_number(largest)
        ),
        // Shown by builds: the radius asked and seven tried between it and
        // the largest found to fit do not fit. Not every radius between
        // (where the faces leave room between two of those tried, one
        // builds), nor any larger (one may fit that takes away whatever
        // was in the way).
        KernelError::FilletTooLarge {
            edge,
            face,
            radius,
            largest,
            shown: Shown::Builds,
        } => {
            let below = craft3d_kernel::fits_number(largest);
            let tried = if craft3d_kernel::fits_shown(largest) < radius {
                format!(", nor the radii tried from {below} {unit} up to it")
            } else {
                String::new()
            };
            format!(
                "radius: too large for {}: the {} {unit} asked does not fit beside the face {}{tried}; the largest found to fit is just below {below} {unit}",
                role(edge),
                craft3d_kernel::short_number(radius),
                face_name(face),
            )
        }
        // It builds, and every radius between it and the one asked is past
        // the rim's axis (worked out).
        KernelError::FilletTooLarge {
            edge,
            radius,
            largest,
            shown: Shown::Itself,
            ..
        } => format!(
            "radius: too large for {}: with a radius of {} {unit} the ball's centre would pass the edge's axis; the largest radius that fits there is {} {unit}, which puts it on that axis",
            role(edge),
            craft3d_kernel::short_number(radius),
            craft3d_kernel::short_number(largest)
        ),
        KernelError::FilletSpindle {
            edge,
            radius,
            below,
            own,
        } => format!(
            "radius: not built for {}: {}",
            role(edge),
            craft3d_kernel::spindle_text(radius, below, own, &format!(" {unit}"))
        ),
        KernelError::Fillet { edge, face, reason } => {
            let at = edge.map_or_else(|| "edges".to_string(), role);
            match face {
                Some(face) => format!("{at}: {reason} (the face {})", face_name(face)),
                None => format!("{at}: {reason}"),
            }
        }
        KernelError::FilletPinched {
            edge,
            at,
            why,
            places,
        } => format!(
            "{}: {}",
            role(edge),
            craft3d_kernel::fillet_pinched_text(at, why, places, &format!(" {unit}"))
        ),
        KernelError::FilletBoolean {
            edge,
            error,
            smaller_fails,
        } => format!(
            "{}: the boolean that cuts this fillet refused it: {error}{}",
            role(edge),
            if smaller_fails {
                "; smaller radii tried fail the same way"
            } else {
                ""
            }
        ),
        other => other.to_string(),
    }
}

/// One fade of a fillet as it is built: the place of the fade in the
/// feature's list, the picked edge it is of and which end of it (0: the
/// edge's start, 1: its end), and its lengths in the document's unit.
struct Faded {
    fade: usize,
    pick: usize,
    end: usize,
    /// The vertex of the body at that end.
    vertex: craft3d_topo::VertexId,
    over: f64,
    back: f64,
}

/// A length as a message says it: as the kernel's messages do, and past
/// what nine digits say in the form `1e300` (not three hundred digits).
fn length_text(value: f64) -> String {
    if value.is_finite() && value.abs() >= 1e9 {
        format!("{value:e}")
    } else {
        craft3d_kernel::short_number(value)
    }
}

/// The fades of `fillet` on the edges `picked` of `body`: each `at` a
/// vertex of the body that ends exactly one of them, each length a
/// length, at most one fade per end, a faded edge on its own (no other
/// picked edge ends in one of its vertices) and long enough for its
/// fades. Refused with the parameter and what to change.
#[allow(clippy::too_many_arguments)]
fn fades(
    doc: &Document,
    variables: &Evaluation,
    fillet: &Fillet,
    picked: &[Picked],
    body: &Body,
    names: &Names,
    source: FeatureId,
    made_by: &str,
    linear: f64,
    earlier: &[FeatureResult],
) -> Result<Vec<Faded>, String> {
    let unit = doc.units.length.symbol();
    let vertex_called = |vertex: craft3d_topo::VertexId| {
        names
            .vertex_name(vertex.0 as usize)
            .map_or_else(|| format!("vertex {}", vertex.0), |n| n.to_string())
    };
    // The ends of the picked edges, for what is refused: those a fade
    // takes (named in a way proven to stay with their vertex), and those
    // it does not, said as such, so that no refusal offers a name another
    // refusal then turns down.
    let ends_listed = || {
        let (mut taken, mut not): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
        for pick in picked {
            if let Some(edge) = body.edge(pick.edge) {
                for v in [edge.start, edge.end] {
                    let name = names.vertex_name(v.0 as usize);
                    let proven = name
                        .as_ref()
                        .is_some_and(|n| identity::unproven(doc, n).is_none());
                    let called = vertex_called(v);
                    let list = if proven { &mut taken } else { &mut not };
                    if !list.contains(&called) {
                        list.push(called);
                    }
                }
            }
        }
        let mut text = if taken.is_empty() {
            "no end of the edges this fillet rounds has a name a fade takes".to_string()
        } else {
            format!(
                "the ends of the edges this fillet rounds that a fade takes: {}",
                taken.join(", ")
            )
        };
        if !not.is_empty() {
            text.push_str(&format!(
                "; not {}, named after the face of a blend or by where it lies, which is not proven to stay with its vertex: a fade at such an end (one an earlier fillet or chamfer has rounded) is not built yet",
                not.join(", ")
            ));
        }
        text
    };
    let called = |id: FeatureId| {
        doc.feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone())
    };
    let mut out: Vec<Faded> = Vec::with_capacity(fillet.fade.len());
    for (index, fade) in fillet.fade.iter().enumerate() {
        let role = format!("fade.{index}");
        let over = length(doc, variables, &format!("{role}.over"), &fade.over)?;
        if !(over.is_finite() && over > 0.0) {
            return Err(format!(
                "{role}.over: the length over which the radius falls to nothing has to be more than zero, got {}",
                length_text(over)
            ));
        }
        let back = match &fade.back {
            Some(expr) => length(doc, variables, &format!("{role}.back"), expr)?,
            None => 0.0,
        };
        if !(back.is_finite() && back >= 0.0) {
            return Err(format!(
                "{role}.back: how far short of the vertex the fillet ends has to be zero or more, got {}",
                length_text(back)
            ));
        }
        let at = &fade.at;
        if !at.is_vertex() {
            return Err(format!(
                "{role}.at: {at} is not a vertex; a fade is at a vertex that ends an edge the fillet rounds (document.edges lists the vertices of a body)"
            ));
        }
        if at.feature != source {
            // Of a body that went into this one on the way (its name could
            // not be followed: `follow::vertex`), or of another body.
            let went_in = crate::follow::chain(earlier, at.feature).contains(&source);
            return Err(if went_in {
                format!(
                    "{role}.at: {at} is a vertex of `{}`, whose body went into `{made_by}` before this fillet, and `{made_by}` has no vertex that is it any more (a feature on the way, a blend or a cut, took it or moved it); {}",
                    called(at.feature),
                    ends_listed()
                )
            } else {
                format!(
                    "{role}.at: {at} is a vertex of `{}`, and the edges this fillet rounds are of `{made_by}`; {}",
                    called(at.feature),
                    ends_listed()
                )
            });
        }
        let Some(vertex) = names
            .vertex_named(at)
            .and_then(|i| u32::try_from(i).ok())
            .map(craft3d_topo::VertexId)
        else {
            return Err(format!(
                "{role}.at: `{made_by}` has no vertex {at} (any more); {}",
                ends_listed()
            ));
        };
        let mut ends: Vec<(usize, usize)> = Vec::new();
        for (pick_index, pick) in picked.iter().enumerate() {
            let Some(edge) = body.edge(pick.edge) else {
                continue;
            };
            if edge.start == vertex && edge.end == vertex {
                return Err(format!(
                    "{role}.at: {at} is where {} closes on itself: an edge that closes into a ring has no end for the fillet to fade at",
                    blend::role_of(&fillet.edges, pick.reference)
                ));
            }
            if edge.start == vertex {
                ends.push((pick_index, 0));
            } else if edge.end == vertex {
                ends.push((pick_index, 1));
            }
        }
        let (pick, end) = match ends.as_slice() {
            [one] => *one,
            [] => {
                return Err(format!(
                    "{role}.at: {at} is not an end of an edge this fillet rounds; {}",
                    ends_listed()
                ));
            }
            [first, second, ..] => {
                let role_of = |p: usize| {
                    picked.get(p).map_or_else(String::new, |pick| {
                        blend::role_of(&fillet.edges, pick.reference)
                    })
                };
                return Err(format!(
                    "{role}.at: {at} is an end of {} and of {}: a fillet fades along an edge on its own, so it cannot round both edges that meet where it fades; round the other edge in a feature of its own, or leave it out",
                    role_of(first.0),
                    role_of(second.0)
                ));
            }
        };
        if let Some(earlier) = out.iter().find(|f| f.pick == pick && f.end == end) {
            return Err(format!(
                "{role}.at: the fillet already fades at {at} (fade.{}); give one fade per end of an edge",
                earlier.fade
            ));
        }
        out.push(Faded {
            fade: index,
            pick,
            end,
            vertex,
            over,
            back,
        });
    }
    // A faded edge on its own: no other edge the fillet rounds ends in
    // one of its vertices (a chain, two edges that meet).
    for faded in &out {
        let (Some(pick), Some(edge)) = (
            picked.get(faded.pick),
            picked.get(faded.pick).and_then(|p| body.edge(p.edge)),
        ) else {
            continue;
        };
        let mine = [edge.start, edge.end];
        let other = picked.iter().enumerate().find_map(|(i, p)| {
            if i == faded.pick {
                return None;
            }
            let theirs = body.edge(p.edge)?;
            let shared = [theirs.start, theirs.end]
                .into_iter()
                .find(|v| mine.contains(v))?;
            Some((p, shared))
        });
        if let Some((p, shared)) = other {
            let how = if fillet.chain {
                " (chain takes the edges that continue it)"
            } else {
                ""
            };
            return Err(format!(
                "fade.{}: the fillet fades along {}, and {}{how} ends in its vertex {} too: a fillet that fades is built along one edge on its own, not yet in a chain or where another rounded edge meets it; round the other edge in a feature of its own{}",
                faded.fade,
                blend::role_of(&fillet.edges, pick.reference),
                if p.reference == pick.reference {
                    "another edge it gives".to_string()
                } else {
                    blend::role_of(&fillet.edges, p.reference)
                },
                vertex_called(shared),
                if fillet.chain {
                    ", or leave chain out"
                } else {
                    ""
                }
            ));
        }
    }
    // Long enough for its fades: what each takes along the edge, `over`
    // and `back`, together no longer than the edge.
    for (pick_index, pick) in picked.iter().enumerate() {
        let mine: Vec<&Faded> = out.iter().filter(|f| f.pick == pick_index).collect();
        if mine.is_empty() {
            continue;
        }
        let taken: f64 = mine.iter().map(|f| f.over + f.back).sum();
        let Some(long) = kernel::law_length(body, pick.edge, linear) else {
            continue;
        };
        if taken > long * (1.0 + 1e-12) {
            let which: Vec<String> = mine.iter().map(|f| format!("fade.{}", f.fade)).collect();
            return Err(format!(
                "{}: over and back take {} {unit} along {}, which is {} {unit} long: give a shorter over or back",
                which.join(" and "),
                length_text(taken),
                blend::role_of(&fillet.edges, pick.reference),
                length_text(long)
            ));
        }
    }
    Ok(out)
}

/// What a fade said by the kernel is refused for, at the fade it is of:
/// `fade.i (at …): …`, with the unit.
fn fade_fault(
    fault: FadeFault,
    edge: EdgeId,
    fillet: &Fillet,
    picked: &[Picked],
    faded: &[Faded],
    unit: &str,
) -> String {
    let text = craft3d_kernel::fillet_fade_text(fault, &format!(" {unit}"));
    let on_edge: Vec<&Faded> = faded
        .iter()
        .filter(|f| picked.get(f.pick).is_some_and(|p| p.edge == edge))
        .collect();
    // The fade the numbers of the fault are of, when they say.
    let which = on_edge
        .iter()
        .find(|f| match fault {
            FadeFault::TooShort { over, .. } => f.over == over,
            FadeFault::BackTooSmall { back, .. } => f.back == back,
            FadeFault::EndTooSmall { .. }
            | FadeFault::CurvatureJump { .. }
            | FadeFault::NoBall { .. }
            | FadeFault::TurnsBack { .. }
            | FadeFault::BackNotATip { .. } => false,
        })
        .or(match on_edge.as_slice() {
            [only] => Some(only),
            _ => None,
        });
    match which.and_then(|f| fillet.fade.get(f.fade).map(|given| (f.fade, given))) {
        Some((index, given)) => format!("fade.{index} (at {}): {text}", given.at),
        None => {
            let role = picked.iter().find(|p| p.edge == edge).map_or_else(
                || "edges".to_string(),
                |p| blend::role_of(&fillet.edges, p.reference),
            );
            format!("{role}: {text}")
        }
    }
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: FeatureId,
    fillet: &Fillet,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>, Vec<FadeReport>), String> {
    let Some(source_id) = fillet.source() else {
        return Err("edges: a fillet needs at least one edge".to_string());
    };
    if source_id == feature {
        return Err("edges: a fillet cannot be of its own edges".to_string());
    }
    // The feature the faces are of has to be there, earlier, and built:
    // said with the names that are lost with it, as every reference by
    // name says it. Then its body has to be its own still.
    if let Some(edge) = fillet.edges.first() {
        let names = edge.names();
        named_body_of(doc, earlier, edge.lead(), names.get(1).copied(), "edges.0")?;
    }
    let source = source_body(doc, earlier, source_id, "edges")?;
    let made_by = doc
        .feature(source_id)
        .map_or_else(|| source_id.to_string(), |f| f.name.clone());
    let (Some(body), Some(names)) = (&source.body, &source.names) else {
        return Err(format!(
            "edges: `{made_by}` does not name its faces, so its edges cannot be referred to"
        ));
    };
    let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    let radius = length(doc, variables, "radius", &fillet.radius)?;
    if !(radius.is_finite() && radius > 0.0) {
        return Err(format!("radius: has to be more than zero, got {radius}"));
    }
    let picked = blend::pick(
        &fillet.edges,
        fillet.chain,
        ROLES.noun,
        blend::First::Either,
        source_id,
        body,
        names,
        &made_by,
    )?;
    let unit = doc.units.length.symbol();
    let (made, faded) = if fillet.fade.is_empty() {
        // One radius all along: as every fillet was built before fades.
        let requests: Vec<FilletEdge> = picked
            .iter()
            .map(|pick| FilletEdge {
                edge: pick.edge,
                radius,
            })
            .collect();
        let made = kernel::fillet_capped(
            body,
            &requests,
            &tolerance,
            Some(doc.tolerance.loosest_linear()),
        ).map_err(|error| match error {
            KernelError::InvalidSize { value, .. } => format!(
                "radius: a radius of {value} {unit} is too small to tell from the edge at this document's tolerance"
            ),
            other => explain(other, fillet, &picked, names, unit),
        })?;
        (made, Vec::new())
    } else {
        let faded = fades(
            doc,
            variables,
            fillet,
            &picked,
            body,
            names,
            source_id,
            &made_by,
            tolerance.linear,
            earlier,
        )?;
        let requests: Vec<FilletLawEdge> = (0..picked.len())
            .filter_map(|i| {
                let pick = picked.get(i)?;
                Some(FilletLawEdge {
                    edge: pick.edge,
                    law: law_of(radius, &faded, i),
                })
            })
            .collect();
        let made = kernel::fillet_by_law_capped(
            body,
            &requests,
            &tolerance,
            Some(doc.tolerance.loosest_linear()),
        )
        .map_err(|error| match error {
            KernelError::InvalidSize { name: "radius", value } => format!(
                "radius: a radius of {value} {unit} is too small to tell from the edge at this document's tolerance"
            ),
            KernelError::FilletFade { edge, fault } => {
                fade_fault(fault, edge, fillet, &picked, &faded, unit)
            }
            other => explain(other, fillet, &picked, names, unit),
        })?;
        (made, faded)
    };
    let faces: Vec<blend::Made<'_>> = made
        .faces
        .iter()
        .map(|face| match face {
            FilletFace::Kept(faces) => blend::Made::Kept(faces),
            FilletFace::Fillet(edges) => blend::Made::Blend(edges),
            FilletFace::Corner(vertex) => blend::Made::Corner(*vertex),
        })
        .collect();
    let mut named = blend::name(
        feature,
        ROLES,
        body,
        names,
        &picked,
        &made.body,
        &faces,
        &made.edges,
        &made.vertices,
        tolerance.linear,
    )?;
    if let Some(named) = named.as_mut() {
        by_their_end(
            named,
            &made,
            body,
            &picked,
            &faded,
            radius,
            tolerance.linear,
        )?;
    }
    blend::not_through_itself(
        &made.body,
        &faces,
        named.as_ref(),
        tolerance.linear,
        ROLES,
        "radius",
    )?;
    let reports = faded
        .iter()
        .filter_map(|f| {
            let pick = picked.get(f.pick)?;
            let given = fillet.fade.get(f.fade)?;
            let law = law_of(radius, &faded, f.pick);
            let tip = kernel::tip_point(body, pick.edge, law, f.end, tolerance.linear)?;
            // The vertex the face closes in, by its name in the
            // filleted body.
            let vertex = made
                .body
                .vertices
                .iter()
                .enumerate()
                .map(|(i, v)| (i, v.point.distance(tip)))
                .filter(|(_, d)| *d <= 10.0 * tolerance.linear)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .and_then(|(i, _)| named.as_ref()?.vertex_name(i));
            Some(FadeReport {
                at: given.at.clone(),
                edge: pick.reference,
                tip,
                vertex,
                over: f.over,
                back: f.back,
                length: kernel::law_length(body, pick.edge, tolerance.linear),
            })
        })
        .collect();
    Ok((made.body, named, reports))
}

/// The tip of each fade short of its vertex, and what is left of the edge
/// beyond it, named by the end they belong to (AGENTS.md rule 4): the
/// faces at the vertex the fade is at, other than those the tip lies on,
/// are written into both names as faces near them. Named by their faces
/// alone they would be the same at either end of the edge, and a fade
/// moved to the other end would carry a datum point at its tip, or a note
/// on the edge beyond, to the new place without a word; so the old names
/// are gone, and what referred to them fails with the name. (Where the
/// vertex has no face but the edge's two, as a hull's forefoot, the names
/// are those of the faces alone, and differ from those at an end that
/// has one.)
fn by_their_end(
    named: &mut Names,
    made: &kernel::FilletResult,
    source: &Body,
    picked: &[Picked],
    faded: &[Faded],
    radius: f64,
    linear: f64,
) -> Result<(), String> {
    for f in faded.iter().filter(|f| f.back > 0.0) {
        let Some(pick) = picked.get(f.pick) else {
            continue;
        };
        let law = law_of(radius, faded, f.pick);
        let Some(tip) = kernel::tip_point(source, pick.edge, law, f.end, linear) else {
            continue;
        };
        let Some(tip_index) = made
            .body
            .vertices
            .iter()
            .enumerate()
            .map(|(i, v)| (i, v.point.distance(tip)))
            .filter(|(_, d)| *d <= 10.0 * linear)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
        else {
            continue;
        };
        let Some(at_index) = made.vertices.iter().position(|v| *v == Some(f.vertex)) else {
            continue;
        };
        let at_faces = faces_at(&made.body, at_index);
        let tip_faces = faces_at(&made.body, tip_index);
        let near: Vec<usize> = at_faces.difference(&tip_faces).copied().collect();
        if near.is_empty() {
            continue;
        }
        named.add_near(true, tip_index, &near)?;
        let beyond = made.body.edges.iter().position(|e| {
            let ends = [e.start.0 as usize, e.end.0 as usize];
            ends.contains(&tip_index) && ends.contains(&at_index)
        });
        if let Some(edge) = beyond {
            named.add_near(false, edge, &near)?;
        }
    }
    Ok(())
}

/// The faces of `body` an edge of which ends in vertex `vertex`, by
/// their index.
fn faces_at(body: &Body, vertex: usize) -> std::collections::BTreeSet<usize> {
    let mut out = std::collections::BTreeSet::new();
    for (index, face) in body.faces.iter().enumerate() {
        let touches = face
            .loops
            .iter()
            .filter_map(|l| body.loop_(*l))
            .flat_map(|l| &l.coedges)
            .any(|c| {
                body.edge(c.edge)
                    .is_some_and(|e| e.start.0 as usize == vertex || e.end.0 as usize == vertex)
            });
        if touches {
            out.insert(index);
        }
    }
    out
}

/// The radius law of the picked edge `pick`: `radius`, and a fade to
/// nothing at each end the fillet fades at.
fn law_of(radius: f64, faded: &[Faded], pick: usize) -> RadiusLaw {
    let mut fades = [None, None];
    for f in faded.iter().filter(|f| f.pick == pick) {
        if let Some(slot) = fades.get_mut(f.end) {
            *slot = Some(Fade::tip(f.over, f.back));
        }
    }
    RadiusLaw { radius, fades }
}
