//! Offsets of the profiles of a sketch in regeneration: the solved sketch
//! with its patterns gives its profiles, the kernel offsets them
//! (`craft3d_kernel::offset_profile`), and a feature that sweeps them
//! takes this feature in the place of a sketch. What it sweeps is named
//! after the offset's curves: `F10:side(F9:offset(e7[3]))`,
//! `F10:side(F9:offsetCorner(e7,e8))`, caps `F10:endCap(F9:offset(e1,e7[*]))`
//! (docs/design/names.md).

use crate::extrude::swept;
use crate::sketch_pattern::{collapse, curve_name, full_runs};
use crate::{FeatureResult, length};
use craft3d_doc::{
    Document, FeatureId, FeatureKind, NameRole, OffsetCorners, OffsetHead, OffsetKind,
    OffsetProfile, PersistentName,
};
use craft3d_expr::Evaluation;
use craft3d_kernel::KernelError;
use craft3d_kernel::offset_profile::{
    Corners, OffsetOrigin, OffsetOutline, OffsetSource, loop_curves, offset_profiles,
};

/// What an offset feature made.
#[derive(Clone, Debug, PartialEq)]
pub struct OffsetResult {
    /// The sketch whose profiles were offset.
    pub sketch: FeatureId,
    /// The distance, in document units.
    pub distance: f64,
    pub outline: OffsetOutline,
    /// The name of each curve of the outline, as the names of what a sweep
    /// makes of it have it inside (`F9:offset(e7[3])`), by id.
    pub names: Vec<String>,
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    id: FeatureId,
    offset: &OffsetProfile,
    earlier: &[FeatureResult],
) -> Result<OffsetResult, String> {
    if let Some(source) = doc.feature(offset.sketch)
        && !matches!(source.kind, FeatureKind::Sketch(_))
    {
        return Err(format!(
            "sketch: `{}` is a {}; an offset is made from the profiles of a sketch",
            source.name,
            source.kind.type_name()
        ));
    }
    let made = swept(doc, earlier, offset.sketch, "offset")?;
    let distance = length(doc, variables, "distance", &offset.distance)?;
    let corners = match offset.corners {
        OffsetCorners::Round => Corners::Round,
        OffsetCorners::Sharp => Corners::Sharp,
    };
    let unit = doc.units.length.symbol();
    let outline =
        offset_profiles(&made.profiles, distance, corners, &made.tolerance).map_err(|error| {
            match error {
                KernelError::OffsetProfile {
                    reason,
                    curves,
                    largest,
                } => {
                    let mut text = format!("distance: the offset of sketch `{}` {reason}", made.name);
                    if !curves.is_empty() {
                        let names: Vec<String> = curves.iter().map(|c| curve_name(*c)).collect();
                        text.push_str(&format!(" (at {})", names.join(", ")));
                    }
                    if let Some(largest) = largest {
                        text.push_str(&format!(
                            "; the largest distance at which every loop is still there is {largest} {unit}"
                        ));
                    }
                    text
                }
                KernelError::TooManyOffsetCurves { count, limit } => format!(
                    "distance: the offset of sketch `{}` has {count} curves, more than an offset of profiles takes ({limit}); corners: sharp adds none where lines meet",
                    made.name
                ),
                KernelError::CrowdedOffset {
                    what,
                    curves,
                    limit,
                } => {
                    let names: Vec<String> = curves.iter().map(|c| curve_name(*c)).collect();
                    format!(
                        "distance: the curves of the offset of sketch `{}` lie too close together: it would have more than {limit} {what} (at {}); use a smaller distance, or fewer curves near these",
                        made.name,
                        names.join(", ")
                    )
                }
                other => format!("distance: {other}"),
            }
        })?;
    let runs = full_runs(made.sketch, made.solved);
    let names = outline
        .origins
        .iter()
        .map(|origin| {
            let sources = sources_of(id, origin, &runs)?;
            // Written as the source of a face, then taken out of it.
            let text = PersistentName::new(id, NameRole::Side, sources).to_string();
            Ok(text
                .split_once('(')
                .and_then(|(_, rest)| rest.strip_suffix(')'))
                .unwrap_or(&text)
                .to_string())
        })
        .collect::<Result<Vec<String>, String>>()?;
    Ok(OffsetResult {
        sketch: offset.sketch,
        distance,
        outline,
        names,
    })
}

/// The offset feature `id` made, when it is one and was built.
pub(crate) fn made<'a>(
    doc: &Document,
    earlier: &'a [FeatureResult],
    id: FeatureId,
) -> Option<&'a OffsetResult> {
    if !matches!(
        doc.feature(id).map(|f| &f.kind),
        Some(FeatureKind::OffsetProfile(_))
    ) {
        return None;
    }
    earlier
        .iter()
        .find(|r| r.id == id)
        .and_then(|r| r.offset.as_deref())
}

/// The heads and curves of one curve of an offset: its loop first, when
/// that is what tells it from other pieces of the same curve, with whole
/// runs of a pattern written once (`e7[*]`), so the loop is named the same
/// whatever the count.
fn sources_of(
    feature: FeatureId,
    origin: &OffsetOrigin,
    runs: &std::collections::BTreeMap<u64, usize>,
) -> Result<Vec<u64>, String> {
    let head = |kind| {
        OffsetHead { feature, kind }
            .encode()
            .ok_or_else(|| crate::too_long("an offset's feature id is beyond what a name holds"))
    };
    let mut out = Vec::new();
    if let Some(within) = &origin.within {
        out.push(head(OffsetKind::Loop)?);
        out.extend(collapse(within, runs).unwrap_or_else(|| within.clone()));
    }
    out.push(head(match origin.source {
        OffsetSource::Curve(_) => OffsetKind::Offset,
        OffsetSource::Corner(..) => OffsetKind::Corner,
        OffsetSource::Extension(..) => OffsetKind::Extension,
    })?);
    out.extend(origin.source.curves());
    Ok(out)
}

/// Renames what a sweep of the offset feature `input` made, from the
/// numbers the kernel had for its curves to the names of the offset's
/// curves. Caps (`caps`) are named after the curves of the sketch their
/// loop is the offset of, runs of a pattern once (`e7[*]`), so a cap
/// keeps its name when a corner comes or goes or a pattern's count
/// changes. Nothing changes when `input` is not an offset.
pub(crate) fn rename(
    doc: &Document,
    earlier: &[FeatureResult],
    input: FeatureId,
    names: &mut [PersistentName],
    caps: &[NameRole],
) -> Result<(), String> {
    let Some(made) = made(doc, earlier, input) else {
        return Ok(());
    };
    let origins = &made.outline.origins;
    let runs = match (
        doc.feature(made.sketch).map(|f| &f.kind),
        earlier
            .iter()
            .find(|r| r.id == made.sketch)
            .and_then(|r| r.sketch.as_ref()),
    ) {
        (Some(FeatureKind::Sketch(sketch)), Some(solved)) => full_runs(sketch, solved),
        _ => Default::default(),
    };
    let origin = |id: u64| {
        usize::try_from(id)
            .ok()
            .and_then(|i| origins.get(i))
            .ok_or_else(|| format!("the offset has no curve {id}; this is a bug"))
    };
    for name in names.iter_mut() {
        // The link the kernel's numbers are in: under a piece, if any.
        let mut link = name;
        while link.role == NameRole::Piece {
            let Some(next) = link.of.as_deref_mut() else {
                break;
            };
            link = next;
        }
        let mut sources = Vec::new();
        if caps.contains(&link.role) {
            let found = link
                .sources
                .iter()
                .map(|&id| origin(id))
                .collect::<Result<Vec<_>, _>>()?;
            let curves = loop_curves(found.into_iter());
            let curves = collapse(&curves, &runs).unwrap_or(curves);
            let head = OffsetHead {
                feature: input,
                kind: OffsetKind::Offset,
            }
            .encode()
            .ok_or_else(|| crate::too_long("an offset's feature id is beyond what a name holds"))?;
            sources.push(head);
            sources.extend(curves);
        } else {
            for &id in &link.sources {
                sources.extend(sources_of(input, origin(id)?, &runs)?);
            }
        }
        link.sources = sources;
    }
    Ok(())
}
