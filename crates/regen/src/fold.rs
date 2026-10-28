//! Folds in regeneration: the sheet the sketch's lines bound, divided
//! along the crease lines, folded rigidly by the fold angles and built as
//! a sheet of planar facets (`craft3d_kernel::origami`,
//! docs/design/origami.md).

use crate::extrude::drawn;
use crate::sketch_pattern::CurveIds;
use crate::{FeatureResult, Names, angle, length};
use craft3d_doc::{Document, FeatureId, Fold, Geometry};
use craft3d_expr::Evaluation;
use craft3d_kernel::origami::{
    self, Angle, Arrangement, BoundarySegment, Crease, CreaseKind, FoldError, Folded, Layers,
};
use craft3d_kernel::profile::{Shape, detect_profiles};
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::Body;
use std::collections::BTreeSet;

/// A fold's crease pattern, laid out flat: what `fold.check` and the
/// crease-pattern export read, without folding it.
#[derive(Clone, Debug)]
pub struct FoldPattern {
    /// The sheet divided into facets, in the sketch's axes.
    pub arrangement: Arrangement,
    /// Per crease of the arrangement (`arrangement.creases`, in order):
    /// what the fold says of it.
    pub creases: Vec<PatternCrease>,
    /// The names of the sketch's curves (pattern instances among them).
    pub names: CurveIds,
    /// The sketch's plane.
    pub plane: Frame,
    /// The sheet's outline: each loop as its corners, outer loops
    /// counter-clockwise and holes clockwise, in the sketch's axes.
    pub outline: Vec<Vec<[f64; 2]>>,
    pub tolerance: Tolerance,
}

/// One crease of a fold's pattern as the fold gives it: a line of the
/// sketch or a copy a pattern made of it.
#[derive(Clone, Debug, PartialEq)]
pub struct PatternCrease {
    /// As the sketch names the curve: `e7`, `e7[2]`.
    pub name: String,
    pub kind: craft3d_doc::CreaseKind,
    pub angle: Option<craft3d_doc::Expr>,
    /// The entry of the fold's creases it comes from.
    pub entry: usize,
}

/// A text of the kernel's, its curves `e` and a number, with each named as
/// the sketch names it (`e7[2]` for an instance of a pattern).
fn renamed(text: &str, names: &CurveIds) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut previous: Option<char> = None;
    while let Some(at) = rest.find('e') {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        let last = before.chars().last().or(previous);
        let digits: String = from
            .chars()
            .skip(1)
            .take_while(char::is_ascii_digit)
            .collect();
        let word = last.is_none_or(|c| !c.is_alphanumeric());
        match digits.parse::<u64>() {
            Ok(id) if word && !digits.is_empty() => {
                out.push_str(&names.name(id));
                rest = from.get(1 + digits.len()..).unwrap_or_default();
                previous = digits.chars().last();
            }
            _ => {
                out.push('e');
                rest = from.get(1..).unwrap_or_default();
                previous = Some('e');
            }
        }
    }
    out.push_str(rest);
    out
}

/// The creases of `fold` laid out: the kernel's crease (its id the
/// curve's number) and what the fold gives it, in the order of the fold's
/// entries, an `e7[*]` entry as the line and then its instances in the
/// sketch's order. Every copy a pattern made of a crease's line must be
/// a crease too (named one by one or with `[*]`): one left out would be
/// dropped without a word.
fn resolve_creases(
    drawn: &crate::extrude::Drawn<'_>,
    fold: &Fold,
) -> Result<Vec<(Crease, PatternCrease)>, String> {
    use craft3d_doc::{CurveIndex, CurvePath, CurveRef};
    let name = drawn.name;
    let names = &drawn.solved.curve_ids;
    // Every copy the sketch's patterns made, construction ones too.
    let instances = crate::sketch_pattern::instance_curves(drawn.sketch, drawn.solved, true)
        .map_err(|reason| format!("sketch `{name}`: {reason}"))?;
    let copies: Vec<(CurvePath, u64, [f64; 2], [f64; 2])> = instances
        .iter()
        .filter_map(|(curve, _)| {
            let craft3d_kernel::profile::Curve2::Line { start, end } = curve.curve else {
                return None;
            };
            let words = names.words(curve.id);
            let (path, n) = CurvePath::read(&words)?;
            (n == words.len()).then_some((path, curve.id, start, end))
        })
        .collect();
    let parity = |path: &CurvePath| {
        let of = |i: CurveIndex| match i {
            CurveIndex::At(k) => u64::from(k),
            CurveIndex::Grid(i, j) => u64::from(i) + u64::from(j),
            CurveIndex::Seed | CurveIndex::All => 0,
        };
        let sum = path
            .outer
            .iter()
            .fold(of(path.curve.index), |s, &i| s.wrapping_add(of(i)));
        sum % 2 == 1
    };
    let flip = |kind: craft3d_doc::CreaseKind| match kind {
        craft3d_doc::CreaseKind::Mountain => craft3d_doc::CreaseKind::Valley,
        craft3d_doc::CreaseKind::Valley => craft3d_doc::CreaseKind::Mountain,
        craft3d_doc::CreaseKind::Cut => craft3d_doc::CreaseKind::Cut,
    };
    let kernel_kind = |kind: craft3d_doc::CreaseKind| match kind {
        craft3d_doc::CreaseKind::Mountain => CreaseKind::Mountain,
        craft3d_doc::CreaseKind::Valley => CreaseKind::Valley,
        craft3d_doc::CreaseKind::Cut => CreaseKind::Cut,
    };
    let mut entries = BTreeSet::new();
    let mut taken: BTreeSet<u64> = BTreeSet::new();
    // First the entries that name one curve, so they win over `[*]`.
    let mut one: Vec<(usize, u64, [f64; 2], [f64; 2])> = Vec::new();
    for (k, c) in fold.creases.iter().enumerate() {
        let shown = c.name();
        if !entries.insert((c.line, c.index)) {
            return Err(format!("creases: {shown} is listed twice"));
        }
        if c.kind == craft3d_doc::CreaseKind::Cut && c.angle.is_some() {
            return Err(format!(
                "creases: {shown} is a cut, which has no fold angle"
            ));
        }
        let entity = drawn
            .sketch
            .entity(c.line)
            .map_err(|_| format!("creases: sketch `{name}` has no {}", c.line))?;
        let Geometry::Line { start, end } = entity.geometry else {
            return Err(format!(
                "creases: {} of sketch `{name}` is a {}, not a line: creases are straight (a curved crease bends its facets, which rigid folding does not do)",
                c.line,
                entity.geometry.type_name()
            ));
        };
        let point = |p| {
            drawn
                .solved
                .point(p)
                .map(|(x, y)| [x, y])
                .ok_or_else(|| format!("sketch `{name}`: {p} has no solved position"))
        };
        match c.index {
            CurveIndex::Seed => one.push((k, c.line.0, point(start)?, point(end)?)),
            CurveIndex::All => {}
            index => {
                let wanted = CurveRef {
                    entity: c.line.0,
                    index,
                };
                let Some((_, id, a, b)) = copies
                    .iter()
                    .find(|(path, ..)| path.curve == wanted && path.outer.is_empty())
                else {
                    return Err(format!(
                        "creases: sketch `{name}` has no {shown}: no pattern of the sketch makes that copy of {} (sketch.inspect lists its patterns)",
                        c.line
                    ));
                };
                one.push((k, *id, *a, *b));
            }
        }
    }
    let mut out: Vec<(usize, Crease, PatternCrease)> = Vec::new();
    for (k, id, a, b) in one {
        let Some(c) = fold.creases.get(k) else {
            continue;
        };
        taken.insert(id);
        out.push((
            k,
            Crease {
                id,
                start: a,
                end: b,
                kind: kernel_kind(c.kind),
            },
            PatternCrease {
                name: names.name(id),
                kind: c.kind,
                angle: c.angle.clone(),
                entry: k,
            },
        ));
    }
    for (k, c) in fold.creases.iter().enumerate() {
        if c.index != CurveIndex::All {
            continue;
        }
        if c.alternate && c.kind == craft3d_doc::CreaseKind::Cut {
            return Err(format!(
                "creases: {} is a cut, which does not alternate",
                c.name()
            ));
        }
        let mut members: Vec<(u64, [f64; 2], [f64; 2], bool)> = Vec::new();
        if let Ok(entity) = drawn.sketch.entity(c.line)
            && let Geometry::Line { start, end } = entity.geometry
            && let (Some(a), Some(b)) = (drawn.solved.point(start), drawn.solved.point(end))
        {
            members.push((c.line.0, [a.0, a.1], [b.0, b.1], false));
        }
        for (path, id, a, b) in &copies {
            if path.curve.entity == c.line.0 {
                members.push((*id, *a, *b, parity(path)));
            }
        }
        for (id, a, b, odd) in members {
            if !taken.insert(id) {
                continue;
            }
            let kind = if c.alternate && odd {
                flip(c.kind)
            } else {
                c.kind
            };
            out.push((
                k,
                Crease {
                    id,
                    start: a,
                    end: b,
                    kind: kernel_kind(kind),
                },
                PatternCrease {
                    name: names.name(id),
                    kind,
                    angle: c.angle.clone(),
                    entry: k,
                },
            ));
        }
    }
    if let Some(c) = fold
        .creases
        .iter()
        .find(|c| c.alternate && c.index != CurveIndex::All)
    {
        return Err(format!(
            "creases: {} names one curve, which has nothing to alternate with: alternate goes with every copy, {}",
            c.name(),
            CurveRef {
                entity: c.line.0,
                index: CurveIndex::All
            }
        ));
    }
    // A copy of a crease's line that is no crease would be dropped.
    for c in &fold.creases {
        let left: Vec<String> = copies
            .iter()
            .filter(|(path, id, ..)| path.curve.entity == c.line.0 && !taken.contains(id))
            .map(|(_, id, ..)| names.name(*id))
            .take(4)
            .collect();
        if !left.is_empty() {
            return Err(format!(
                "creases: {} is a crease but the sketch's patterns made copies of it that are not ({}): name each copy as a crease ({} as line), or every copy with {}",
                c.name(),
                left.join(", "),
                left.first().map_or("", String::as_str),
                CurveRef {
                    entity: c.line.0,
                    index: CurveIndex::All
                }
            ));
        }
    }
    // In the order of the entries, then of the sketch.
    out.sort_by_key(|(k, ..)| *k);
    Ok(out.into_iter().map(|(_, c, p)| (c, p)).collect())
}

/// A fold's crease pattern and (when it could be) its fold.
#[derive(Clone, Debug)]
pub struct FoldAnalysis {
    pub pattern: FoldPattern,
    /// Where each facet went, or why the fold failed.
    pub folded: Result<Folded, String>,
    /// The facet that stays in place.
    pub fixed: usize,
    /// Folded flat (every crease by 180 degrees): the order of its layers;
    /// a stack folded on: the layers of its flat-folded stacks.
    pub layers: Option<Layers>,
    /// A stack folded on (some creases at 180 degrees, others by other
    /// angles): per facet the direction its stack's layers rise, in the
    /// sheet's axes; `None` otherwise.
    pub stacked: Option<Vec<craft3d_math::Vec3>>,
}

/// A fold folded flat, as its feature's result keeps it: the order of its
/// layers, and per face of its body the display-only offset that lifts
/// each layer off the one below it (the body itself lies exactly in the
/// sketch's plane: docs/design/origami.md › *Folded flat*).
#[derive(Clone, Debug, PartialEq)]
pub struct FlatFold {
    pub layers: Layers,
    /// Per face of the body (one per facet, in facet order): where the
    /// display moves it, along the sketch's normal, by its layer above
    /// the fixed facet's.
    pub offsets: Vec<craft3d_math::Vec3>,
    /// A stack folded on, not the whole sheet folded flat: its layers are
    /// those of its stacks, each lifted along its own plane's normal.
    pub stacked: bool,
}

/// What a curve id is called in messages: `e7`.
fn curve(id: u64) -> String {
    format!("e{id}")
}

fn list(ids: &[u64]) -> String {
    ids.iter()
        .map(|&id| curve(id))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A facet as messages name it: by the curves of its outer boundary.
fn facet_name(arr: &Arrangement, facet: usize) -> String {
    let mut ids: Vec<u64> = arr
        .facets
        .get(facet)
        .into_iter()
        .flat_map(|f| f.outer.iter())
        .filter_map(|&h| arr.edges.get(h / 2).map(|e| e.source))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    format!("the facet bounded by {}", list(&ids))
}

/// A kernel error in the sketch's words.
fn described(error: &FoldError, arr: Option<&Arrangement>) -> String {
    match error {
        FoldError::TooShort { id } => format!("the crease {} is no longer than the tolerance", curve(*id)),
        FoldError::NotFinite { id } => format!("{} has a coordinate that is not finite", curve(*id)),
        FoldError::Overlap { a, b } => format!(
            "{} and {} lie along each other: a crease may cross or end on another crease and on the boundary, not run along them",
            curve(*a),
            curve(*b)
        ),
        FoldError::Crowded { id } => format!(
            "the points near {} lie closer together than the tolerance can tell apart",
            curve(*id)
        ),
        FoldError::Outside { id } => format!("the crease {} reaches outside the sheet (or over a hole in it)", curve(*id)),
        FoldError::Dangling { id } => format!(
            "the crease {} ends inside a facet without dividing it: a crease runs from the boundary, a cut or another crease to the boundary, a cut or another crease",
            curve(*id)
        ),
        FoldError::Sliver { facet } => match arr {
            Some(arr) => format!("{} is thinner than the tolerance", facet_name(arr, *facet)),
            None => "a facet is thinner than the tolerance".to_string(),
        },
        FoldError::Fixed => "`fixed` lies in no facet: give a point inside the facet that stays in place, not on a crease".to_string(),
        FoldError::AngleRange { id } => format!(
            "the fold angle of {} is not between 0 and 180 degrees",
            curve(*id)
        ),
        FoldError::Undetermined { ids } => format!(
            "nothing determines the fold angles of {}: give {} an angle (a crease on no closed loop of creases, one the given angles leave free, or a bifurcation partway where the fold could go on more than one way)",
            list(ids),
            if ids.len() == 1 { "it" } else { "one or more of them" }
        ),
        FoldError::TooManyFree { count } => format!(
            "{count} fold angles to solve for, more than {} at once: give more creases an angle",
            origami::MAX_FREE
        ),
        FoldError::NotClosing { id, gap } => format!(
            "not rigidly foldable at these angles: the facets round the loop through {} do not meet again (they stay {gap:e} apart); change the angles, or leave more of them to be solved",
            curve(*id)
        ),
        FoldError::NotFollowed { at } => format!(
            "the fold could not be followed from the flat sheet past {:.6} degrees of its largest angle: no step, however small, found a fold that follows on from the last along its tangent (or the first step found no branch through the flat sheet that keeps the kinds)",
            at.to_degrees()
        ),
        FoldError::Ambiguous { ids } => format!(
            "the fold is ambiguous: from the flat sheet it can go two ways that both keep every crease's kind, and {} fold differently on them; give one of them an angle",
            list(ids)
        ),
        FoldError::TooMuchWork => format!(
            "the fold needs more work than its budget ({} steps): give more creases an angle",
            origami::MAX_WORK
        ),
        FoldError::NotConverged { gap } => format!(
            "the fold did not converge (the facets stay {gap:e} apart): leave more angles to be solved, or fold less far"
        ),
        FoldError::WrongWay { id, angle } => format!(
            "at these angles the crease {} would fold the other way from its kind (by {:.6} degrees, valleys counted positive), or past 180 degrees: the pattern does not fold rigidly so with this mountain-valley assignment",
            curve(*id),
            angle.to_degrees()
        ),
        FoldError::Meets(c) => match arr {
            Some(arr) => format!(
                "{} and {} {} at these angles (at {:?} in the sketch's axes, folded): a sheet may not pass through or lie on itself",
                facet_name(arr, c.facets[0]),
                facet_name(arr, c.facets[1]),
                c.kind.verb(),
                [c.at.x, c.at.y, c.at.z]
            ),
            None => error.to_string(),
        },
        FoldError::NotFlat { id, gap } => format!(
            "does not fold flat: with every crease folded by 180 degrees the facets round the loop through {} do not meet again (they stay {gap:e} apart); Kawasaki's condition fails at a vertex of that loop (fold.check names it)",
            curve(*id)
        ),
        FoldError::NoLayers { facets } => {
            let named: Vec<String> = match arr {
                Some(arr) => facets.iter().take(8).map(|&f| facet_name(arr, f)).collect(),
                None => facets.iter().take(8).map(|f| format!("facet {f}")).collect(),
            };
            format!(
                "cannot fold flat with this mountain-valley assignment: no order of the layers keeps {}{} from passing through each other (a facet between the two of a fold, or two folds interleaved); change the kinds of the creases there",
                named.join(", "),
                if facets.len() > 8 { " and more" } else { "" }
            )
        }
        FoldError::LayersUndecided { what, count } => format!(
            "whether it folds flat was not decided within the work allowed (at most {} steps and {} rules; here {count} {what}): the general question is NP-hard (Bern and Hayes 1996); 3DCraft does not guess",
            origami::MAX_LAYER_WORK,
            origami::MAX_LAYER_CONSTRAINTS
        ),
        FoldError::Overlay(reason) => format!(
            "the layers of the flat fold could not be told apart: {reason}"
        ),
        FoldError::MixedFlat { id } => format!(
            "{} folds by 180 degrees while other creases fold by less: facets folded onto each other are layered only when the sheet is folded flat, every crease by 180 degrees (a crease to stay flat is left out of the fold); fold it to just under 180 degrees, or every crease to 180 (layers that fold on by other angles are owed)",
            curve(*id)
        ),
        FoldError::Clearance { facet, reason } => match arr {
            Some(arr) => format!(
                "the thick panel of {} does not fit: {reason} (make the clearance smaller)",
                facet_name(arr, *facet)
            ),
            None => error.to_string(),
        },
        FoldError::PanelsMeet(c) => match arr {
            Some(arr) => format!(
                "the thick panels of {} and {} {} at these angles (at {:?} in the sketch's axes, folded): fold less far, or make the clearance larger or the panels thinner (beside a crease folded towards the panels they meet past 2 atan(clearance / thickness))",
                facet_name(arr, c.facets[0]),
                facet_name(arr, c.facets[1]),
                c.kind.verb(),
                [c.at.x, c.at.y, c.at.z]
            ),
            None => error.to_string(),
        },
        other => other.to_string(),
    }
}

/// Lays out the crease pattern of `fold`.
fn pattern(doc: &Document, fold: &Fold, earlier: &[FeatureResult]) -> Result<FoldPattern, String> {
    let source = doc.feature(fold.sketch).ok_or_else(|| {
        format!(
            "sketch: there is no feature {} (was the sketch deleted?)",
            fold.sketch
        )
    })?;
    if !matches!(source.kind, craft3d_doc::FeatureKind::Sketch(_)) {
        return Err(format!(
            "sketch: `{}` is a {}, not a sketch: a fold's creases are lines of a sketch",
            source.name,
            source.kind.type_name()
        ));
    }
    let drawn = drawn(doc, earlier, fold.sketch)?;
    let name = drawn.name;
    let tol = drawn.tolerance;

    // The creases: lines of the sketch or copies of them its patterns
    // made, each once.
    let resolved = resolve_creases(&drawn, fold)?;
    let ids: BTreeSet<u64> = resolved.iter().map(|r| r.0.id).collect();
    let (creases, sources): (Vec<Crease>, Vec<PatternCrease>) = resolved.into_iter().unzip();

    // The sheet: the closed profiles of the sketch's other curves, of
    // straight lines.
    let others: Vec<_> = drawn
        .curves
        .iter()
        .filter(|c| !ids.contains(&c.id))
        .cloned()
        .collect();
    if others.is_empty() {
        return Err(format!(
            "sketch `{name}` has no outline for the sheet: draw a closed profile of lines besides the creases"
        ));
    }
    let profiles = detect_profiles(&others, &tol).map_err(|e| {
        format!(
            "sketch `{name}`: {}",
            crate::extrude::profile_error(&e, &drawn.solved.curve_ids)
        )
    })?;
    let mut boundary = Vec::new();
    let mut outline = Vec::new();
    for profile in &profiles {
        for path in std::iter::once(&profile.outer).chain(&profile.holes) {
            outline.push(path.segments.iter().map(|s| s.start).collect::<Vec<_>>());
            for segment in &path.segments {
                if segment.shape != Shape::Line {
                    return Err(format!(
                        "sketch `{name}`: {} bounds the sheet and is not a straight line; a fold's sheet is bounded by lines (owed: dividing a curved outline by the creases)",
                        drawn.solved.curve_name(segment.source)
                    ));
                }
                boundary.push(BoundarySegment {
                    id: segment.source,
                    start: segment.start,
                    end: segment.end,
                    reversed: segment.reversed,
                });
            }
        }
    }
    let names = &drawn.solved.curve_ids;
    let arrangement = origami::arrange(&boundary, &creases, tol.linear)
        .map_err(|e| renamed(&described(&e, None), names))?;
    Ok(FoldPattern {
        creases: sources,
        names: names.clone(),
        arrangement,
        plane: drawn.plane,
        outline,
        tolerance: tol,
    })
}

/// Lays out the crease pattern of `fold` and folds it.
pub(crate) fn analyse(
    doc: &Document,
    variables: &Evaluation,
    fold: &Fold,
    earlier: &[FeatureResult],
) -> Result<FoldAnalysis, String> {
    let pattern = pattern(doc, fold, earlier)?;
    let arrangement = &pattern.arrangement;
    let tol = pattern.tolerance;

    // Which facet stays.
    let fixed = match &fold.fixed {
        Some([x, y]) => {
            let at = [
                length(doc, variables, "fixed.x", x)?,
                length(doc, variables, "fixed.y", y)?,
            ];
            arrangement.facet_at(at).ok_or_else(|| {
                renamed(
                    &described(&FoldError::Fixed, Some(arrangement)),
                    &pattern.names,
                )
            })?
        }
        None => {
            arrangement
                .facets
                .iter()
                .enumerate()
                .fold((0usize, f64::NEG_INFINITY), |best, (i, f)| {
                    if f.area > best.1 { (i, f.area) } else { best }
                })
                .0
        }
    };

    // The angles, per hinge.
    let mut angles = Vec::with_capacity(arrangement.hinges.len());
    for hinge in &arrangement.hinges {
        let Some((c, kernel)) = pattern
            .creases
            .get(hinge.crease)
            .zip(arrangement.creases.get(hinge.crease))
        else {
            return Err("a crease lost its place in the pattern; this is a bug".to_string());
        };
        let sign = kernel.kind.sign();
        angles.push(match &c.angle {
            Some(expr) => {
                let value = angle(doc, variables, &format!("crease.{}", c.name), expr)?;
                if !(value.is_finite() && (0.0..=std::f64::consts::PI).contains(&value)) {
                    return Err(format!(
                        "crease.{}: `{}` is {} degrees; a fold angle is from 0 (flat) to 180 degrees (folded flat), and the crease's kind says which way",
                        c.name,
                        expr.source(),
                        value.to_degrees()
                    ));
                }
                Angle::Given(sign * value)
            }
            None => Angle::Free { sign },
        });
    }
    let mut layers = None;
    let mut stacked = None;
    let folded = if origami::folds_stacked(&angles, tol.angular) {
        // A stack folded on: the 180-degree creases folded flat into
        // stacks (layered), the stacks folded on by the other creases.
        origami::fold_stacked(arrangement, &angles, fixed, tol.linear, tol.angular)
            .and_then(|s| {
                match origami::first_meeting_stacked(
                    arrangement,
                    &s.folded,
                    tol.linear,
                    tol.angular,
                    &s.stack,
                    &s.bends,
                )? {
                    Some(meeting) => Err(FoldError::Meets(meeting)),
                    None => Ok(s),
                }
            })
            .map(|s| {
                layers = Some(s.layers);
                stacked = Some(s.up);
                s.folded
            })
            .map_err(|e| described(&e, Some(arrangement)))
    } else {
        match origami::folds_flat(arrangement, &angles, tol.angular) {
            // Folded flat: the facets placed by reflections, and the order of
            // the layers is the check that the sheet does not pass through
            // itself (all of it lies in one plane).
            Ok(true) => origami::fold_flat(arrangement, &angles, fixed, tol.linear)
                .and_then(|folded| {
                    let order = origami::layer_order(arrangement, &folded, tol.linear)?;
                    layers = Some(order);
                    Ok(folded)
                })
                .map_err(|e| described(&e, Some(arrangement))),
            Ok(false) => origami::fold(arrangement, &angles, fixed, tol.linear, tol.angular)
                .map_err(|e| described(&e, Some(arrangement)))
                .and_then(|folded| {
                    match origami::first_meeting(arrangement, &folded, tol.linear, tol.angular) {
                        Ok(Some(meeting)) => {
                            Err(described(&FoldError::Meets(meeting), Some(arrangement)))
                        }
                        Ok(None) => Ok(folded),
                        Err(e) => Err(described(&e, Some(arrangement))),
                    }
                }),
            Err(e) => Err(described(&e, Some(arrangement))),
        }
    };
    let folded = folded.map_err(|e| renamed(&e, &pattern.names));
    Ok(FoldAnalysis {
        pattern,
        folded,
        fixed,
        layers,
        stacked,
    })
}

/// Builds a fold: the folded sheet and the names of its faces, edges and
/// vertices (as a fill's: docs/design/origami.md › Names).
pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    id: FeatureId,
    fold: &Fold,
    earlier: &[FeatureResult],
) -> Result<(Body, Names, Option<FlatFold>), String> {
    let analysis = analyse(doc, variables, fold, earlier)?;
    let folded = analysis.folded?;
    let p = &analysis.pattern;
    let made = match &fold.panels {
        Some(panels) => {
            let thick = origami::Panels {
                thickness: length(doc, variables, "panels.thickness", &panels.thickness)?,
                clearance: length(doc, variables, "panels.clearance", &panels.clearance)?,
                side: match panels.side {
                    craft3d_doc::PanelSide::Back => origami::PanelSide::Back,
                    craft3d_doc::PanelSide::Front => origami::PanelSide::Front,
                },
            };
            origami::thick_panels(&p.arrangement, &folded, &p.plane, &thick, &p.tolerance)
        }
        None => origami::folded_sheet(&p.arrangement, &folded, &p.plane, &p.tolerance),
    }
    .map_err(|e| renamed(&described(&e, Some(&p.arrangement)), &p.names))?;
    let names = Names::of_origins(id, &made.origins);
    // Thick panels folded flat pass through each other and were refused;
    // the layers' picture is the sheet's.
    let layers = if fold.panels.is_some() {
        None
    } else {
        analysis.layers
    };
    let up = analysis.stacked;
    let flat = layers.map(|layers| {
        let offsets = display_offsets(&layers, analysis.fixed, &made.body, &p.plane, up.as_deref());
        FlatFold {
            layers,
            offsets,
            stacked: up.is_some(),
        }
    });
    Ok((made.body, names, flat))
}

/// The display-only lift of each layer of a flat fold: per facet its
/// layer less the fixed facet's, times a step along the sketch's normal
/// of a thousandth of the body's diagonal (less when there are many
/// layers: all of them within a fiftieth). Only the picture moves; the
/// body stays in the plane.
fn display_offsets(
    layers: &Layers,
    fixed: usize,
    body: &Body,
    plane: &Frame,
    up: Option<&[craft3d_math::Vec3]>,
) -> Vec<craft3d_math::Vec3> {
    let size = craft3d_math::Bbox3::from_points(body.vertices.iter().map(|v| v.point)).diagonal();
    let highest = layers.layer.iter().copied().max().unwrap_or(0);
    let count = f64::from(u32::try_from(highest).unwrap_or(u32::MAX)) + 1.0;
    let step = size * (1e-3f64).min(0.02 / count);
    let base = layers.layer.get(fixed).copied().unwrap_or(0);
    layers
        .layer
        .iter()
        .enumerate()
        .map(|(f, &l)| {
            let rise = f64::from(u32::try_from(l).unwrap_or(u32::MAX))
                - f64::from(u32::try_from(base).unwrap_or(u32::MAX));
            // The stack's own up for a stack folded on, the sketch's
            // normal for the sheet folded flat.
            let normal = match up.and_then(|u| u.get(f)) {
                Some(v) => plane.vector(v.x, v.y, v.z),
                None => plane.z(),
            };
            normal * (rise * step)
        })
        .collect()
}

fn fold_of(doc: &Document, id: FeatureId) -> Result<&Fold, String> {
    let feature = doc
        .feature(id)
        .ok_or_else(|| format!("there is no feature {id}"))?;
    match &feature.kind {
        craft3d_doc::FeatureKind::Fold(fold) => Ok(fold),
        other => Err(format!(
            "`{}` is a {}, not a fold",
            feature.name,
            other.type_name()
        )),
    }
}

/// The crease pattern of fold `id` of `doc`, laid out flat (not folded),
/// with the results of regeneration (`regen`) for the sketch it reads:
/// what `fold.check` and the crease-pattern export read. Errors when the
/// feature is no fold or its pattern cannot be laid out.
pub fn fold_pattern(
    doc: &Document,
    regen: &crate::Regen,
    id: FeatureId,
) -> Result<FoldPattern, String> {
    pattern(doc, fold_of(doc, id)?, &regen.features)
}

/// The crease pattern of fold `id`, laid out and folded (`fold.inspect`).
pub fn fold_analysis(
    doc: &Document,
    regen: &crate::Regen,
    id: FeatureId,
) -> Result<FoldAnalysis, String> {
    let variables = crate::feature_variables(doc);
    analyse(doc, &variables, fold_of(doc, id)?, &regen.features)
}
