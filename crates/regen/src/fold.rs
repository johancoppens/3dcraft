//! Folds in regeneration: the sheet the sketch's lines bound, divided
//! along the crease lines, folded rigidly by the fold angles and built as
//! a sheet of planar facets (`craft3d_kernel::origami`,
//! docs/design/origami.md).

use crate::extrude::drawn;
use crate::{FeatureResult, Names, angle, length};
use craft3d_doc::{Document, FeatureId, Fold, Geometry};
use craft3d_expr::Evaluation;
use craft3d_kernel::origami::{
    self, Angle, Arrangement, BoundarySegment, Crease, CreaseKind, FoldError, Folded,
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
    /// The sketch's plane.
    pub plane: Frame,
    /// The sheet's outline: each loop as its corners, outer loops
    /// counter-clockwise and holes clockwise, in the sketch's axes.
    pub outline: Vec<Vec<[f64; 2]>>,
    pub tolerance: Tolerance,
}

/// A fold's crease pattern and (when it could be) its fold.
#[derive(Clone, Debug)]
pub struct FoldAnalysis {
    pub pattern: FoldPattern,
    /// Where each facet went, or why the fold failed.
    pub folded: Result<Folded, String>,
    /// The facet that stays in place.
    pub fixed: usize,
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
        FoldError::Slit { id } => format!(
            "the cut {} ends inside a facet: a cut must divide the sheet (a slit that ends inside a facet is not built yet)",
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
            "{count} fold angles to solve for, more than {} at once: give more creases an angle (a sparse solve is owed)",
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

    // The creases: lines of the sketch, each once.
    let mut ids = BTreeSet::new();
    let mut creases = Vec::with_capacity(fold.creases.len());
    for c in &fold.creases {
        if !ids.insert(c.line.0) {
            return Err(format!("creases: {} is listed twice", c.line));
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
        if c.kind == craft3d_doc::CreaseKind::Cut && c.angle.is_some() {
            return Err(format!(
                "creases: {} is a cut, which has no fold angle",
                c.line
            ));
        }
        creases.push(Crease {
            id: c.line.0,
            start: point(start)?,
            end: point(end)?,
            kind: match c.kind {
                craft3d_doc::CreaseKind::Mountain => CreaseKind::Mountain,
                craft3d_doc::CreaseKind::Valley => CreaseKind::Valley,
                craft3d_doc::CreaseKind::Cut => CreaseKind::Cut,
            },
        });
    }

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
    let arrangement =
        origami::arrange(&boundary, &creases, tol.linear).map_err(|e| described(&e, None))?;
    Ok(FoldPattern {
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
            arrangement
                .facet_at(at)
                .ok_or_else(|| described(&FoldError::Fixed, Some(arrangement)))?
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
        let Some((c, kernel)) = fold
            .creases
            .get(hinge.crease)
            .zip(arrangement.creases.get(hinge.crease))
        else {
            return Err("a crease lost its place in the pattern; this is a bug".to_string());
        };
        let sign = kernel.kind.sign();
        angles.push(match &c.angle {
            Some(expr) => {
                let value = angle(doc, variables, &format!("crease.{}", c.line), expr)?;
                if !(value.is_finite() && (0.0..=std::f64::consts::PI).contains(&value)) {
                    return Err(format!(
                        "crease.{}: `{}` is {} degrees; a fold angle is from 0 (flat) to 180 degrees (folded flat), and the crease's kind says which way",
                        c.line,
                        expr.source(),
                        value.to_degrees()
                    ));
                }
                Angle::Given(sign * value)
            }
            None => Angle::Free { sign },
        });
    }
    let folded = origami::fold(arrangement, &angles, fixed, tol.linear, tol.angular)
        .map_err(|e| described(&e, Some(arrangement)))
        .and_then(|folded| {
            match origami::first_meeting(arrangement, &folded, tol.linear, tol.angular) {
                Ok(Some(meeting)) => Err(described(&FoldError::Meets(meeting), Some(arrangement))),
                Ok(None) => Ok(folded),
                Err(e) => Err(described(&e, Some(arrangement))),
            }
        });
    Ok(FoldAnalysis {
        pattern,
        folded,
        fixed,
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
) -> Result<(Body, Names), String> {
    let analysis = analyse(doc, variables, fold, earlier)?;
    let folded = analysis.folded?;
    let p = &analysis.pattern;
    let made = origami::folded_sheet(&p.arrangement, &folded, &p.plane, &p.tolerance)
        .map_err(|e| described(&e, Some(&p.arrangement)))?;
    let names = Names::of_origins(id, &made.origins);
    Ok((made.body, names))
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
