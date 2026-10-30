//! Sheet metal in regeneration: the base flat (the closed profile of a
//! sketch its flanges sit on; the sketch's other closed profiles are holes
//! and cut-outs of the flats they fall inside), its flanges and reliefs, folded and plated by the kernel
//! (`craft3d_kernel::sheetmetal`, docs/design/sheetmetal.md).
//!
//! **Names.** The folded bottom skin's faces are `F9:face(0)` (the base),
//! `F9:face(2n − 1)` (the bend of the flange of id `n`) and `F9:face(2n)`
//! (its flat), by the flanges' ids, not their places; the part's faces are a plate's of that skin: the top skin
//! of a face `F9:copy(0)/F9:face(k)`, the bottom skin `F9:copy(1)/…`, the
//! rim along an edge `F9:rim(h)/F9:face(k)` (`regen::plate`), its edge
//! named by what it is (`sheetmetal::edge_numbers`), so a flange added
//! later renames only the rim along the edge it is bent from.

use crate::extrude::profile_error;
use crate::{FeatureResult, Names, angle, face_ops, length, number};
use craft3d_doc::{
    BendDirection, CornerKind, Document, FeatureId, FeatureKind, FlangeFrom, FlangeOn, FlangeSide,
    NameRole, PersistentName, Pick, ReliefShape, SheetMetal,
};
use craft3d_expr::Evaluation;
use craft3d_kernel::KernelError;
use craft3d_kernel::profile::detect_profiles;
use craft3d_kernel::profile::{Curve2, SketchCurve};
use craft3d_kernel::sheetmetal::{
    self, Closing, CornerSpec, Cutout, Direction, FlangeSpec, LengthFrom, On, Part, Relief,
    ReliefKind, SheetFace, Spec,
};
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::{Body, FaceId};
use std::sync::Arc;

/// A sheet-metal part as regeneration built it, for the commands that
/// read it (the bend table, the measures, the flat pattern's export).
#[derive(Clone, Debug, PartialEq)]
pub struct SheetMetalPart {
    pub part: Part,
    pub tolerance: Tolerance,
    /// The base's sketch, for messages.
    pub sketch: String,
}

/// The name of a face of the folded bottom skin: the base `F9:face(0)`,
/// the bend of the flange of id `n` `F9:face(2n − 1)`, its flat
/// `F9:face(2n)` (with the ids a part is made with, `n` is the flange's
/// place plus 1: `face(2i + 1)` and `face(2i + 2)` for flange `i`); the
/// further face `j` (1, 2, ...) a hole across a bend leaves of one of
/// those has that face's number in its low 32 bits and `j` above
/// (`F9:face(k + j * 2^32)`), so adding, deleting or moving a flange or a
/// cut elsewhere does not renumber it (ids stay below 2³¹, so `k` below
/// 2³²). `None` for a flange with no id.
pub fn sheet_face_name(feature: FeatureId, part: &Part, face: SheetFace) -> Option<PersistentName> {
    let number = match face {
        SheetFace::Flat(f) => 2 * part.flat_number(f)?,
        SheetFace::Bend(i) => 2 * part.flange_number(i)? + 1,
        SheetFace::FlatPiece(f, j) => 2 * part.flat_number(f)? + ((j as u64) << 32),
        SheetFace::BendPiece(i, j) => 2 * part.flange_number(i)? + 1 + ((j as u64) << 32),
    };
    Some(PersistentName::new(feature, NameRole::Face, vec![number]))
}

fn explain(error: KernelError) -> String {
    match error {
        KernelError::SheetMetal {
            flange: Some(i),
            reason,
        } => format!("flanges.{i}: {reason}"),
        KernelError::SheetMetal {
            flange: None,
            reason,
        } => reason,
        KernelError::InvalidSize { name, value } => format!("{name}: {value} is not a size"),
        other => other.to_string(),
    }
}

/// Reads and builds the part.
pub(crate) fn analyse(
    doc: &Document,
    variables: &Evaluation,
    made: &SheetMetal,
    earlier: &[FeatureResult],
) -> Result<SheetMetalPart, String> {
    let drawn = crate::extrude::drawn_plain(doc, earlier, made.sketch, "a sheet-metal outline")?;
    let name = drawn.name.to_string();
    let tol = drawn.tolerance;
    if drawn.curves.is_empty() {
        return Err(format!(
            "sketch `{name}` has nothing to bend: draw the base as one closed profile"
        ));
    }
    let mut profiles = detect_profiles(&drawn.curves, &tol).map_err(|e| {
        format!(
            "sketch `{name}`: {}",
            profile_error(&e, &drawn.solved.curve_ids)
        )
    })?;
    // The base is the profile the flanges on the base's edges sit on; the
    // sketch's other closed profiles are holes and cut-outs of the flats
    // they fall inside in the flat pattern (the kernel places them).
    let holds = |p: &craft3d_kernel::profile::Profile, id: u64| {
        std::iter::once(&p.outer)
            .chain(&p.holes)
            .any(|l| l.segments.iter().any(|s| s.source == id))
    };
    let mut on: Vec<usize> = made
        .flanges
        .iter()
        .filter_map(|f| match f.on {
            FlangeOn::Base(line) => profiles.iter().position(|p| holds(p, line.0)),
            FlangeOn::Tip(_) | FlangeOn::Side(..) | FlangeOn::Line(..) => None,
        })
        .collect();
    on.sort_unstable();
    on.dedup();
    let at = match (on.as_slice(), profiles.len()) {
        ([one], _) => *one,
        ([], 1) => 0,
        ([], n) => {
            return Err(format!(
                "sketch `{name}` has {n} closed profiles and no flange on an edge of one: put a flange on the base's edge to say which is the base (the others are holes and cut-outs of the flats they fall inside)"
            ));
        }
        (_, _) => {
            return Err(format!(
                "sketch `{name}`: the flanges sit on edges of {} different closed profiles; a sheet-metal part has one base (the others are holes and cut-outs of the flats they fall inside)",
                on.len()
            ));
        }
    };
    if at >= profiles.len() {
        return Err(format!("sketch `{name}` lost its base profile"));
    }
    let base = profiles.remove(at);
    let from = format!("sketch `{name}`");
    let mut cutouts: Vec<Cutout> = profiles
        .into_iter()
        .map(|profile| Cutout {
            profile,
            from: from.clone(),
        })
        .collect();
    for (i, &other) in made.cutouts.iter().enumerate() {
        // A sketch switched off cuts nothing (a vent with `when`).
        let off = earlier.iter().find(|r| r.id == other).is_some_and(|r| {
            matches!(
                r.status,
                crate::Status::Suppressed | crate::Status::Inactive | crate::Status::RolledBack
            )
        });
        if off {
            continue;
        }
        let holes = crate::extrude::drawn_plain(doc, earlier, other, "a cutout")
            .map_err(|e| format!("cutouts.{i}: {e}"))?;
        let hname = holes.name.to_string();
        let curves = carried(&holes.curves, &holes.plane, &drawn.plane, &tol)
            .map_err(|e| format!("cutouts.{i}: sketch `{hname}` {e}"))?;
        let found = detect_profiles(&curves, &tol).map_err(|e| {
            format!(
                "cutouts.{i}: sketch `{hname}`: {}",
                profile_error(&e, &holes.solved.curve_ids)
            )
        })?;
        if found.is_empty() {
            return Err(format!(
                "cutouts.{i}: sketch `{hname}` has no closed profile to cut"
            ));
        }
        let from = format!("sketch `{hname}`");
        cutouts.extend(found.into_iter().map(|profile| Cutout {
            profile,
            from: from.clone(),
        }));
    }
    let thickness = length(doc, variables, "thickness", &made.thickness)?;
    let radius = length(doc, variables, "radius", &made.radius)?;
    let k = number(variables, "kFactor", &made.k_factor)?;
    let mut flanges = Vec::with_capacity(made.flanges.len());
    for (i, f) in made.flanges.iter().enumerate() {
        let param = |what: &str| format!("flanges.{i}.{what}");
        let on = match f.on {
            FlangeOn::Base(line) => {
                if drawn.sketch.entity(line).is_err() {
                    return Err(format!("{}: sketch `{name}` has no {line}", param("on")));
                }
                On::Base(line.0)
            }
            FlangeOn::Tip(j) => On::Tip(usize::try_from(j).unwrap_or(usize::MAX)),
            FlangeOn::Side(j, side) => On::Side(
                usize::try_from(j).unwrap_or(usize::MAX),
                match side {
                    FlangeSide::Start => 0,
                    FlangeSide::End => 1,
                },
            ),
            FlangeOn::Line(j, line) => On::Line(usize::try_from(j).unwrap_or(usize::MAX), line.0),
        };
        let inset = |what: &str, e: &Option<craft3d_doc::Expr>| -> Result<f64, String> {
            match e {
                None => Ok(0.0),
                Some(expr) => length(doc, variables, &param(what), expr),
            }
        };
        let outline = match f.outline {
            None => None,
            Some(sketch) => Some(
                own_outline(doc, earlier, sketch, &drawn.plane, &tol)
                    .map_err(|e| format!("{}: {e}", param("outline")))?,
            ),
        };
        flanges.push(FlangeSpec {
            on,
            inset: [
                inset("insetStart", &f.inset_start)?,
                inset("insetEnd", &f.inset_end)?,
            ],
            // A flange with an outline of its own takes its size from it.
            length: if outline.is_some() {
                0.0
            } else {
                length(doc, variables, &param("length"), &f.length)?
            },
            outline,
            angle: angle(doc, variables, &param("angle"), &f.angle)?,
            direction: match f.direction {
                BendDirection::Up => Direction::Up,
                BendDirection::Down => Direction::Down,
            },
            radius: match &f.radius {
                Some(expr) => length(doc, variables, &param("radius"), expr)?,
                None => radius,
            },
            k: match &f.k_factor {
                Some(expr) => number(variables, &param("kFactor"), expr)?,
                None => k,
            },
            from: match f.from {
                FlangeFrom::Tangent => LengthFrom::Tangent,
                FlangeFrom::Mould => LengthFrom::Mould,
            },
        });
    }
    let relief_of = |what: &str, r: &craft3d_doc::CornerRelief| -> Result<Relief, String> {
        Ok(Relief {
            kind: match pick(
                variables,
                &format!("{what}.shape"),
                &r.shape,
                &ReliefShape::ALL,
                ReliefShape::name,
            )? {
                ReliefShape::Rect => ReliefKind::Rect,
                ReliefShape::Round => ReliefKind::Round,
                ReliefShape::Tear => ReliefKind::Tear,
            },
            size: length(doc, variables, &format!("{what}.size"), &r.size)?,
        })
    };
    let relief = made
        .relief
        .as_ref()
        .map(|r| relief_of("relief", r))
        .transpose()?;
    let bend_relief = made
        .bend_relief
        .as_ref()
        .map(|r| relief_of("bendRelief", r))
        .transpose()?;
    let closing = |what: &str,
                   kind: &Pick<CornerKind>,
                   gap: Option<&craft3d_doc::Expr>,
                   over: Option<u32>|
     -> Result<Closing, String> {
        let kind = pick(
            variables,
            &format!("{what}.kind"),
            kind,
            &CornerKind::ALL,
            CornerKind::name,
        )?;
        let gap = match (kind, gap) {
            (CornerKind::Open, _) => 0.0,
            (_, Some(expr)) => length(doc, variables, &format!("{what}.gap"), expr)?,
            (_, None) => {
                return Err(format!(
                    "{what}: a {} corner needs its `gap` (between the two flats)",
                    kind.name()
                ));
            }
        };
        Ok(Closing {
            kind: match kind {
                CornerKind::Open => sheetmetal::CornerKind::Open,
                CornerKind::Butt => sheetmetal::CornerKind::Butt,
                CornerKind::Overlap => sheetmetal::CornerKind::Overlap,
                CornerKind::Miter => sheetmetal::CornerKind::Miter,
            },
            gap,
            over: over.map(|o| usize::try_from(o).unwrap_or(usize::MAX)),
        })
    };
    let corner = match &made.corner {
        None => Closing::OPEN,
        Some(c) => closing("corner", &c.kind, c.gap.as_ref(), None)?,
    };
    let mut corners = Vec::with_capacity(made.corners.len());
    for (i, c) in made.corners.iter().enumerate() {
        let gap = c
            .gap
            .as_ref()
            .or_else(|| made.corner.as_ref().and_then(|d| d.gap.as_ref()));
        let [a, b] = c.flanges.map(|f| usize::try_from(f).unwrap_or(usize::MAX));
        corners.push(CornerSpec {
            flanges: [a, b],
            closing: closing(&format!("corners.{i}"), &c.kind, gap, c.over)?,
        });
    }
    let spec = Spec {
        base,
        plane: drawn.plane,
        thickness,
        flanges,
        relief,
        bend_relief,
        corner,
        corners,
        cutouts,
        ids: made.flanges.iter().map(|f| u64::from(f.id)).collect(),
    };
    let part = sheetmetal::build(&spec, &tol).map_err(explain)?;
    Ok(SheetMetalPart {
        part,
        tolerance: tol,
        sketch: name,
    })
}

/// The closed profile of sketch `sketch` (holes allowed) in the axes of
/// the base's plane `to`: a flange's own outline.
fn own_outline(
    doc: &Document,
    earlier: &[FeatureResult],
    sketch: FeatureId,
    to: &Frame,
    tol: &Tolerance,
) -> Result<craft3d_kernel::profile::Profile, String> {
    let drawn = crate::extrude::drawn_plain(doc, earlier, sketch, "a sheet-metal outline")?;
    let name = drawn.name.to_string();
    let curves = carried(&drawn.curves, &drawn.plane, to, tol)
        .map_err(|e| format!("sketch `{name}` {e}"))?;
    let mut found = detect_profiles(&curves, tol).map_err(|e| {
        format!(
            "sketch `{name}`: {}",
            profile_error(&e, &drawn.solved.curve_ids)
        )
    })?;
    match found.len() {
        1 => Ok(found.remove(0)),
        n => Err(format!(
            "sketch `{name}` has {n} closed profiles; a flange's outline is one (holes inside it allowed)"
        )),
    }
}

/// The name a [`Pick`] picks: as written, or the expression's value, a
/// whole number from 1 (the first of `all`) to the count of names.
fn pick<T: Copy>(
    variables: &Evaluation,
    what: &str,
    choice: &Pick<T>,
    all: &[T],
    name: fn(T) -> &'static str,
) -> Result<T, String> {
    let expr = match choice {
        Pick::Named(t) => return Ok(*t),
        Pick::Expr(expr) => expr,
    };
    let listed: Vec<String> = all
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{} ({})", i + 1, name(*t)))
        .collect();
    let names: Vec<&str> = all.iter().map(|t| name(*t)).collect();
    let value = number(variables, what, expr).map_err(|e| {
        format!(
            "{e} (it is {}, or an expression that picks {})",
            names.join(", "),
            listed.join(", ")
        )
    })?;
    let rounded = value.round();
    if (value - rounded).abs() <= 1e-9 && (1.0..=all.len() as f64).contains(&rounded) {
        // `rounded` is a whole number from 1 to all.len().
        if let Some(t) = all.get(rounded as usize - 1) {
            return Ok(*t);
        }
    }
    Err(format!(
        "{what}: `{}` is {value}; it picks {}",
        expr.source(),
        listed.join(", ")
    ))
}

/// The curves of a sketch drawn in the plane `from`, in the axes of the
/// plane `to` (the base sketch's): the same plane, with any origin, axes
/// and side. Every point is carried by the rigid motion between the two
/// frames; a frame whose normal points the other way mirrors the curves
/// in the base's axes, so an arc, counter-clockwise from its start to its
/// end, then runs from its carried end to its carried start. Refused when
/// the plane is not the base's: the normals not parallel within the
/// angular tolerance, or a point off the base's plane by more than the
/// tolerance.
fn carried(
    curves: &[SketchCurve],
    from: &Frame,
    to: &Frame,
    tol: &Tolerance,
) -> Result<Vec<SketchCurve>, String> {
    let off = || {
        "is not in the base sketch's plane: its holes cannot be cut in the flat pattern".to_string()
    };
    let tilt = from.z().cross(to.z()).length();
    if tilt.is_nan() || tilt > tol.angular {
        return Err(off());
    }
    let mirrored = from.z().dot(to.z()) < 0.0;
    let at = |p: [f64; 2]| -> Result<[f64; 2], String> {
        let (x, y, z) = to.local(from.point(p[0], p[1], 0.0));
        if z.abs() <= tol.linear && x.is_finite() && y.is_finite() {
            Ok([x, y])
        } else {
            Err(off())
        }
    };
    curves
        .iter()
        .map(|c| {
            let curve = match &c.curve {
                Curve2::Line { start, end } => Curve2::Line {
                    start: at(*start)?,
                    end: at(*end)?,
                },
                Curve2::Arc { center, start, end } => {
                    let (start, end) = if mirrored {
                        (at(*end)?, at(*start)?)
                    } else {
                        (at(*start)?, at(*end)?)
                    };
                    Curve2::Arc {
                        center: at(*center)?,
                        start,
                        end,
                    }
                }
                Curve2::Circle { center, radius } => Curve2::Circle {
                    center: at(*center)?,
                    radius: *radius,
                },
                Curve2::Spline(spline) => {
                    let mut spline = spline.clone();
                    for p in &mut spline.control {
                        *p = at(*p)?;
                    }
                    Curve2::Spline(spline)
                }
            };
            Ok(SketchCurve { id: c.id, curve })
        })
        .collect()
}

/// Builds a sheet-metal part: the solid and its names.
pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    id: FeatureId,
    made: &SheetMetal,
    earlier: &[FeatureResult],
) -> Result<(Body, Names, Arc<SheetMetalPart>), String> {
    let built = analyse(doc, variables, made, earlier)?;
    let (part, tolerance) = (&built.part, built.tolerance);
    let mut sheet_names = Names::default();
    for face in &part.sheet_faces {
        sheet_names.faces.push(
            sheet_face_name(id, part, *face)
                .ok_or_else(|| "a flange of the part has no id".to_string())?,
        );
    }
    let none = vec![None; part.sheet.edges.len()];
    let no_vertices = vec![None; part.sheet.vertices.len()];
    sheet_names.name_entities(
        id,
        &part.sheet,
        tolerance.linear,
        &none,
        &no_vertices,
        |_| None,
    )?;
    let picked: Vec<FaceId> = (0..part.sheet.faces.len())
        .filter_map(|f| u32::try_from(f).ok().map(FaceId))
        .collect();
    let made_by = doc
        .feature(id)
        .map_or_else(|| id.to_string(), |f| f.name.clone());
    // Every edge is named by what it is (`sheetmetal::edge_numbers`), not
    // by the faces round it: a flange added elsewhere, a corner closed or
    // opened, leaves every other edge's name, and so every rim's, as it
    // was (rule 4; docs/design/sheetmetal.md › *Names*). With a corner
    // closed an extension's end is named as the side it replaces, a piece
    // an extension adds `F9:edge(6, flat, segment)`.
    let edge_name = |edge: usize| -> Option<PersistentName> {
        let key = *part.edge_keys.get(edge)?;
        Some(PersistentName::new(
            id,
            NameRole::Edge,
            sheetmetal::edge_numbers(part, key)?,
        ))
    };
    let names = crate::plate::plate_names_with(
        id,
        &part.solid,
        &part.sheet,
        &sheet_names,
        &edge_name,
        &picked,
        &made_by,
        tolerance.linear,
    )?;
    face_ops::not_through_itself(
        &part.solid.body,
        &names.faces,
        "flanges",
        "sheet-metal part",
        "flanges that run into each other or into the base when bent: ",
        &tolerance,
    )?;
    Ok((part.solid.body.clone(), names, Arc::new(built)))
}

fn part_of(doc: &Document, id: FeatureId) -> Result<&SheetMetal, String> {
    let feature = doc
        .feature(id)
        .ok_or_else(|| format!("there is no feature {id}"))?;
    match &feature.kind {
        FeatureKind::SheetMetal(made) => Ok(made),
        other => Err(format!(
            "`{}` is a {}, not a sheet-metal part",
            feature.name,
            other.type_name()
        )),
    }
}

/// The sheet-metal part `id` of `doc` as regeneration (`regen`) built it:
/// what the bend table, the manufacturability measures and the flat
/// pattern's export read. Refused, with the reason, when the feature did
/// not build (failed, switched off, rolled back) or is no sheet-metal part.
pub fn sheet_metal_part(
    doc: &Document,
    regen: &crate::Regen,
    id: FeatureId,
) -> Result<Arc<SheetMetalPart>, String> {
    let feature = doc
        .feature(id)
        .ok_or_else(|| format!("there is no feature {id}"))?;
    part_of(doc, id)?;
    let result = regen
        .features
        .iter()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("`{}` has not been regenerated", feature.name))?;
    match &result.status {
        crate::Status::Ok => {}
        crate::Status::Failed(reason) => {
            return Err(format!("`{}` failed: {reason}", feature.name));
        }
        crate::Status::Suppressed | crate::Status::Inactive => {
            return Err(format!("`{}` is switched off", feature.name));
        }
        crate::Status::RolledBack => {
            return Err(format!("`{}` is after the rollback marker", feature.name));
        }
    }
    match &result.developed {
        Some(crate::Developed::SheetMetal(part)) => Ok(part.clone()),
        _ => Err(format!("`{}` keeps no part; this is a bug", feature.name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Vec3;

    /// A sketch whose frame is the base's mirrored (normal -z, x along
    /// -x, origin at (10, 5)): every point lands where the sketch has it in
    /// the world, and an arc, counter-clockwise from start to end in its
    /// own axes, runs from its carried end to its carried start, so it is
    /// counter-clockwise in the base's and covers the same points.
    #[test]
    fn a_mirrored_sketch_is_carried_into_the_base_axes() {
        let tol = Tolerance::new(1e-9, 1e-9).unwrap();
        let down = Vec3::new(0.0, 0.0, -1.0);
        let back = Vec3::new(-1.0, 0.0, 0.0);
        let from = Frame::new(Vec3::new(10.0, 5.0, 0.0), down, back).unwrap();
        let curves = vec![
            SketchCurve {
                id: 1,
                curve: Curve2::Arc {
                    center: [0.0, 0.0],
                    start: [1.0, 0.0],
                    end: [0.0, 1.0],
                },
            },
            SketchCurve {
                id: 2,
                curve: Curve2::Line {
                    start: [0.0, 0.0],
                    end: [2.0, 3.0],
                },
            },
        ];
        let out = carried(&curves, &from, &Frame::WORLD, &tol).unwrap();
        assert_eq!(
            out[0].curve,
            Curve2::Arc {
                center: [10.0, 5.0],
                start: [10.0, 6.0],
                end: [9.0, 5.0],
            }
        );
        assert_eq!(
            out[1].curve,
            Curve2::Line {
                start: [10.0, 5.0],
                end: [8.0, 8.0],
            }
        );
        // Another plane: refused.
        let side = Frame::new(Vec3::ZERO, Vec3::X, Vec3::Y).unwrap();
        assert!(carried(&curves, &side, &Frame::WORLD, &tol).is_err());
        let above = Frame::new(Vec3::new(0.0, 0.0, 1.0), Vec3::Z, Vec3::X).unwrap();
        assert!(carried(&curves, &above, &Frame::WORLD, &tol).is_err());
    }
}
