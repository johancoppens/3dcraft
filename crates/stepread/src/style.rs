//! Colours of solids and faces (ISO 10303-46, the presentation resources
//! AP214 and AP242 use): `STYLED_ITEM` → `PRESENTATION_STYLE_ASSIGNMENT`
//! → `SURFACE_STYLE_USAGE` → `SURFACE_SIDE_STYLE` → `SURFACE_STYLE_FILL_
//! AREA` → `FILL_AREA_STYLE` → `FILL_AREA_STYLE_COLOUR` → `COLOUR_RGB`
//! (as `craft3d-io`'s export writes them), or `SURFACE_STYLE_RENDERING`
//! (`_WITH_PROPERTIES`) with its surface colour, or a
//! `DRAUGHTING_PRE_DEFINED_COLOUR` by name.
//!
//! A colour is information, not geometry: one that cannot be read (a
//! wrong reference, a number outside 0 to 1 or not finite) is left out,
//! never an error that would refuse the body. Every chain is followed
//! to a fixed depth (the entities above, no recursion), so a hostile
//! file cannot make it loop.
//!
//! A body has one colour in the document (`craft3d-doc`'s appearance per
//! body), so a solid takes its own style, else that of one of its
//! shells, else the colour most of its styled faces have (the first of
//! equals in face order); faces' own colours beyond that are not kept.

use crate::{Parsed, real};
use craft3d_step::Param;
use std::collections::BTreeMap;

/// sRGB from a factor 0 to 1, rounded; `None` outside or not finite.
fn channel(x: f64) -> Option<u8> {
    (x.is_finite() && (0.0..=1.0).contains(&x)).then(|| (x * 255.0).round() as u8)
}

/// The colour of a `COLOUR_RGB` or a `DRAUGHTING_PRE_DEFINED_COLOUR`.
fn colour(parsed: &Parsed, id: u64) -> Option<[u8; 3]> {
    let entity = &parsed.instance(id).ok()?.entity;
    if let Some(rgb) = entity.record("COLOUR_RGB") {
        // (name, red, green, blue): the last three.
        let n = rgb.params.len();
        let at = |k: usize| rgb.params.get(n.checked_sub(k)?).and_then(real);
        return Some([channel(at(3)?)?, channel(at(2)?)?, channel(at(1)?)?]);
    }
    if let Some(named) = entity.record("DRAUGHTING_PRE_DEFINED_COLOUR") {
        return Some(match named.params.first()?.as_str()? {
            "red" => [255, 0, 0],
            "green" => [0, 255, 0],
            "blue" => [0, 0, 255],
            "yellow" => [255, 255, 0],
            "magenta" => [255, 0, 255],
            "cyan" => [0, 255, 255],
            "black" => [0, 0, 0],
            "white" => [255, 255, 255],
            _ => return None,
        });
    }
    None
}

fn refs(param: Option<&Param>) -> impl Iterator<Item = u64> + '_ {
    param
        .and_then(Param::as_list)
        .unwrap_or(&[])
        .iter()
        .filter_map(Param::as_ref)
}

/// The colour of a surface side style: its fill area's, else its
/// rendering's.
fn side_colour(parsed: &Parsed, side: u64) -> Option<[u8; 3]> {
    let record = parsed.record(side, "SURFACE_SIDE_STYLE").ok()?;
    let mut rendered = None;
    for element in refs(record.params.get(1)) {
        let Ok(instance) = parsed.instance(element) else {
            continue;
        };
        let entity = &instance.entity;
        if let Some(fill) = entity.record("SURFACE_STYLE_FILL_AREA") {
            let Some(area) = fill.params.first().and_then(Param::as_ref) else {
                continue;
            };
            let Ok(area) = parsed.record(area, "FILL_AREA_STYLE") else {
                continue;
            };
            for style in refs(area.params.get(1)) {
                if let Ok(fill_colour) = parsed.record(style, "FILL_AREA_STYLE_COLOUR")
                    && let Some(c) = fill_colour
                        .params
                        .get(1)
                        .and_then(Param::as_ref)
                        .and_then(|c| colour(parsed, c))
                {
                    return Some(c);
                }
            }
        } else if let Some(rendering) = entity
            .record("SURFACE_STYLE_RENDERING")
            .or_else(|| entity.record("SURFACE_STYLE_RENDERING_WITH_PROPERTIES"))
        {
            rendered = rendered.or_else(|| {
                rendering
                    .params
                    .get(1)
                    .and_then(Param::as_ref)
                    .and_then(|c| colour(parsed, c))
            });
        }
    }
    rendered
}

/// The surface colour of a list of presentation style assignments.
fn styles_colour(parsed: &Parsed, styles: Option<&Param>) -> Option<[u8; 3]> {
    for assignment in refs(styles) {
        let Ok(instance) = parsed.instance(assignment) else {
            continue;
        };
        let Some(record) = instance
            .entity
            .record("PRESENTATION_STYLE_ASSIGNMENT")
            .or_else(|| instance.entity.record("PRESENTATION_STYLE_BY_CONTEXT"))
        else {
            continue;
        };
        for usage in refs(record.params.first()) {
            if let Ok(usage) = parsed.record(usage, "SURFACE_STYLE_USAGE")
                && let Some(c) = usage
                    .params
                    .get(1)
                    .and_then(Param::as_ref)
                    .and_then(|side| side_colour(parsed, side))
            {
                return Some(c);
            }
        }
    }
    None
}

/// The colour of every styled item of the file, by the instance id of the
/// item: the first plain `STYLED_ITEM` of an item, overridden by an
/// `OVER_RIDING_STYLED_ITEM` of it.
pub(crate) fn colors(parsed: &Parsed) -> BTreeMap<u64, [u8; 3]> {
    let mut out = BTreeMap::new();
    for instance in &parsed.file.data {
        let entity = &instance.entity;
        let overriding = entity.is("OVER_RIDING_STYLED_ITEM")
            || entity.is("CONTEXT_DEPENDENT_OVER_RIDING_STYLED_ITEM");
        let Some(record) = entity
            .record("STYLED_ITEM")
            .or_else(|| entity.record("OVER_RIDING_STYLED_ITEM"))
            .or_else(|| entity.record("CONTEXT_DEPENDENT_OVER_RIDING_STYLED_ITEM"))
        else {
            continue;
        };
        let Some(item) = record.params.get(2).and_then(Param::as_ref) else {
            continue;
        };
        let Some(c) = styles_colour(parsed, record.params.get(1)) else {
            continue;
        };
        if overriding {
            out.insert(item, c);
        } else {
            out.entry(item).or_insert(c);
        }
    }
    out
}

/// The colour of the solid `item`, whose faces are `faces` (instance ids
/// of the file's faces): see the module documentation.
pub(crate) fn color_of(parsed: &Parsed, item: u64, faces: &[u64]) -> Option<[u8; 3]> {
    let colors = &parsed.colors;
    if colors.is_empty() {
        return None;
    }
    if let Some(&c) = colors.get(&item) {
        return Some(c);
    }
    // A style on a shell of the solid (its outer shell, its voids, the
    // shells of a surface model).
    if let Ok(instance) = parsed.instance(item) {
        let mut found = None;
        instance.entity.for_each_ref(&mut |r| {
            if found.is_none() {
                found = colors.get(&r).copied();
            }
        });
        if found.is_some() {
            return found;
        }
    }
    let mut counts: Vec<([u8; 3], usize)> = Vec::new();
    for face in faces {
        if let Some(&c) = colors.get(face) {
            match counts.iter_mut().find(|(seen, _)| *seen == c) {
                Some((_, n)) => *n += 1,
                None => counts.push((c, 1)),
            }
        }
    }
    let mut best: Option<([u8; 3], usize)> = None;
    for (c, n) in counts {
        if best.is_none_or(|(_, m)| n > m) {
            best = Some((c, n));
        }
    }
    best.map(|(c, _)| c)
}
