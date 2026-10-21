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

/// Most references the colours of a file may follow in all: past this
/// the file's remaining colours are left out (the bodies still read).
/// With every style, side and fill area read once (memoised), a file of
/// 128 MiB holds fewer references than this; it is a bound for the
/// unforeseen, not a limit a file meets.
pub(crate) const MAX_STYLE_STEPS: usize = 1 << 26;

/// Reads colours, each style entity once: a style assignment, a side
/// style or a fill area shared by many styled items (or listed many times
/// in one) is followed once and its colour remembered, so the work is the
/// size of the file, not the product of its lists (review of
/// 2026-10-21: one assignment of n entries shared by n styled items took
/// n² steps inside every parse).
struct Styler<'a> {
    parsed: &'a Parsed,
    assignments: BTreeMap<u64, Option<[u8; 3]>>,
    sides: BTreeMap<u64, Option<[u8; 3]>>,
    areas: BTreeMap<u64, Option<[u8; 3]>>,
    steps: usize,
}

impl Styler<'_> {
    /// Counts one step; false once the budget is spent.
    fn step(&mut self) -> bool {
        self.steps = self.steps.saturating_add(1);
        self.steps <= MAX_STYLE_STEPS
    }

    /// The colour of a fill area style: its first `FILL_AREA_STYLE_COLOUR`.
    fn area(&mut self, area: u64) -> Option<[u8; 3]> {
        if let Some(&known) = self.areas.get(&area) {
            return known;
        }
        let parsed = self.parsed;
        let mut found = None;
        if let Ok(record) = parsed.record(area, "FILL_AREA_STYLE") {
            for style in refs(record.params.get(1)) {
                if !self.step() {
                    break;
                }
                if let Ok(fill) = parsed.record(style, "FILL_AREA_STYLE_COLOUR")
                    && let Some(c) = fill
                        .params
                        .get(1)
                        .and_then(Param::as_ref)
                        .and_then(|c| colour(parsed, c))
                {
                    found = Some(c);
                    break;
                }
            }
        }
        self.areas.insert(area, found);
        found
    }

    /// The colour of a surface side style: its fill area's, else its
    /// rendering's.
    fn side(&mut self, side: u64) -> Option<[u8; 3]> {
        if let Some(&known) = self.sides.get(&side) {
            return known;
        }
        let parsed = self.parsed;
        let mut filled = None;
        let mut rendered = None;
        if let Ok(record) = parsed.record(side, "SURFACE_SIDE_STYLE") {
            for element in refs(record.params.get(1)) {
                if !self.step() {
                    break;
                }
                let Ok(instance) = parsed.instance(element) else {
                    continue;
                };
                let entity = &instance.entity;
                if let Some(fill) = entity.record("SURFACE_STYLE_FILL_AREA") {
                    if let Some(area) = fill.params.first().and_then(Param::as_ref)
                        && let Some(c) = self.area(area)
                    {
                        filled = Some(c);
                        break;
                    }
                } else if rendered.is_none()
                    && let Some(rendering) = entity
                        .record("SURFACE_STYLE_RENDERING")
                        .or_else(|| entity.record("SURFACE_STYLE_RENDERING_WITH_PROPERTIES"))
                {
                    rendered = rendering
                        .params
                        .get(1)
                        .and_then(Param::as_ref)
                        .and_then(|c| colour(parsed, c));
                }
            }
        }
        let found = filled.or(rendered);
        self.sides.insert(side, found);
        found
    }

    /// The surface colour of one presentation style assignment.
    fn assignment(&mut self, assignment: u64) -> Option<[u8; 3]> {
        if let Some(&known) = self.assignments.get(&assignment) {
            return known;
        }
        let parsed = self.parsed;
        let mut found = None;
        if let Ok(instance) = parsed.instance(assignment)
            && let Some(record) = instance
                .entity
                .record("PRESENTATION_STYLE_ASSIGNMENT")
                .or_else(|| instance.entity.record("PRESENTATION_STYLE_BY_CONTEXT"))
        {
            for usage in refs(record.params.first()) {
                if !self.step() {
                    break;
                }
                if let Ok(usage) = parsed.record(usage, "SURFACE_STYLE_USAGE")
                    && let Some(side) = usage.params.get(1).and_then(Param::as_ref)
                    && let Some(c) = self.side(side)
                {
                    found = Some(c);
                    break;
                }
            }
        }
        self.assignments.insert(assignment, found);
        found
    }
}

/// The colour of every styled item of the file, by the instance id of the
/// item: the first plain `STYLED_ITEM` of an item, overridden by an
/// `OVER_RIDING_STYLED_ITEM` of it. Bounded by [`MAX_STYLE_STEPS`].
pub(crate) fn colors(parsed: &Parsed) -> BTreeMap<u64, [u8; 3]> {
    let mut styler = Styler {
        parsed,
        assignments: BTreeMap::new(),
        sides: BTreeMap::new(),
        areas: BTreeMap::new(),
        steps: 0,
    };
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
        if !overriding && out.contains_key(&item) {
            continue;
        }
        let mut found = None;
        for assignment in refs(record.params.get(1)) {
            if !styler.step() {
                return out;
            }
            if let Some(c) = styler.assignment(assignment) {
                found = Some(c);
                break;
            }
        }
        if let Some(c) = found {
            out.insert(item, c);
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
    // The colour most faces have; of equals, the one met first.
    let mut counts: BTreeMap<[u8; 3], (usize, usize)> = BTreeMap::new();
    for (at, face) in faces.iter().enumerate() {
        if let Some(&c) = colors.get(face) {
            let entry = counts.entry(c).or_insert((0, at));
            entry.0 += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|(_, (n, first)), (_, (m, second))| n.cmp(m).then(second.cmp(first)))
        .map(|(c, _)| c)
}
