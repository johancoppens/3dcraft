//! The outline a cutter's beam centre follows: the part offset by the
//! kerf, the outline outwards and the holes inwards, by the kernel's one
//! offset of closed profiles (`craft3d_kernel::offset_profile`, the same
//! that `feature.offsetProfile` uses), with round corners: every point of
//! the path at the kerf's distance from the part, as a round beam cuts.
//! Lines stay lines and arcs arcs about the same centre.

use super::Shape;
use craft3d_drawing::{Path, Segment};
use craft3d_kernel::offset_profile::{Corners, OffsetSource, offset_profiles};
use craft3d_kernel::profile::{Curve2, Loop2, Shape as Along, SketchCurve, detect_profiles};
use craft3d_math::{Portable, Tolerance};

/// The curves of a loop, numbered on from `next`.
fn curves_of(path: &Path, next: &mut u64, out: &mut Vec<SketchCurve>) -> Result<(), String> {
    let mut push = |curve: Curve2| {
        out.push(SketchCurve { id: *next, curve });
        *next = next.saturating_add(1);
    };
    match path {
        Path::Circle { center, radius } => push(Curve2::Circle {
            center: *center,
            radius: *radius,
        }),
        Path::Ellipse { .. } => {
            return Err(
                "a kerf offset of an ellipse is not built (the offset of an ellipse is no ellipse); export it without a kerf".into(),
            );
        }
        Path::Chain {
            start, segments, ..
        } => {
            let mut at = *start;
            for segment in segments {
                let to = segment.end();
                match segment {
                    Segment::Line { .. } => push(Curve2::Line { start: at, end: to }),
                    Segment::Arc { center, ccw, .. } => {
                        // A curve of the kernel's runs counter-clockwise.
                        let (start, end) = if *ccw { (at, to) } else { (to, at) };
                        push(Curve2::Arc {
                            center: *center,
                            start,
                            end,
                        });
                    }
                    Segment::EllipticArc { .. } => {
                        return Err("a kerf offset of an elliptic arc is not built (its offset is no ellipse); export it without a kerf".into());
                    }
                    Segment::Spline(_) => {
                        return Err("a kerf offset of a spline is not built (its offset is no spline of the same kind); export it without a kerf".into());
                    }
                }
                at = to;
            }
        }
    }
    Ok(())
}

/// A loop of the kernel's back as a path.
fn path_of(lp: &Loop2) -> Result<Path, String> {
    if let [only] = lp.segments.as_slice()
        && let Along::Circle { center, radius } = only.shape
    {
        return Ok(Path::Circle { center, radius });
    }
    let first = lp
        .segments
        .first()
        .ok_or_else(|| "the kerf left an empty loop".to_string())?;
    let mut segments = Vec::with_capacity(lp.segments.len());
    for s in &lp.segments {
        segments.push(match &s.shape {
            Along::Line => Segment::Line { to: s.end },
            Along::Arc { center, radius } => Segment::Arc {
                to: s.end,
                center: *center,
                radius: *radius,
                ccw: !s.reversed,
            },
            Along::Circle { .. } | Along::Spline(_) => {
                return Err("the kerf left a curve an outline cannot hold".into());
            }
        });
    }
    Ok(Path::Chain {
        start: first.start,
        segments,
        closed: true,
    })
}

/// `shape` offset by `k` (the distance from the part's edge to the beam's
/// centre: half the width the beam cuts away): the outline `k` out, each
/// hole `k` in, the corners the beam goes round arcs of radius `k`. The
/// marks stay where they are. `k = 0` is the shape itself. An inner round
/// of radius at most `k` comes out at radius `k`, as a sharp inner corner
/// does: it is said in the notes returned. Refused, with
/// the reason: a negative or infinite `k`, an ellipse or a spline in a
/// loop (not built), and a kerf that changes the loops (a hole that
/// closes, a slot or a notch narrower than `2 k` that closes, a loop that
/// splits), with the largest kerf that keeps them.
pub fn kerf(shape: &Shape, k: f64, tolerance: &Tolerance) -> Result<(Shape, Vec<String>), String> {
    if !(k.is_finite() && k >= 0.0) {
        return Err(format!("the kerf {k} must be a length of 0 or more"));
    }
    if k == 0.0 {
        return Ok((shape.clone(), Vec::new()));
    }
    let mut curves = Vec::new();
    let mut next = 0_u64;
    // What each curve is, for a message: the loop and its place in it.
    let mut labels: Vec<String> = Vec::new();
    for (i, path) in std::iter::once(&shape.outline)
        .chain(&shape.holes)
        .enumerate()
    {
        let before = curves.len();
        curves_of(path, &mut next, &mut curves)?;
        let whose = if i == 0 {
            "the outline".to_string()
        } else {
            format!("hole {i}")
        };
        for n in 0..curves.len() - before {
            labels.push(format!("segment {n} of {whose}"));
        }
    }
    let profiles = detect_profiles(&curves, tolerance)
        .map_err(|e| format!("the outline does not read as one region with holes: {e}"))?;
    match profiles.as_slice() {
        [one] if one.holes.len() == shape.holes.len() => {}
        _ => {
            return Err(format!(
                "the outline does not read as one region with {} holes",
                shape.holes.len()
            ));
        }
    }
    let out = offset_profiles(&profiles, k, Corners::Round, tolerance)
        .map_err(|e| format!("the kerf {k}: {e}"))?;
    if out.loops_after != out.loops_before || out.regions_after != out.regions_before {
        let fits = out
            .whole_up_to
            .map(|d| format!("; the largest kerf that keeps every loop is {d}"))
            .unwrap_or_default();
        return Err(format!(
            "the kerf {k} turns {} loops into {} (a slot, notch or gap narrower than twice the kerf closes, or a hole splits){fits}",
            out.loops_before, out.loops_after
        ));
    }
    // Every curve of the part has its one offset on the path, or the beam
    // does not cut it: a notch or slot narrower than the beam, filled; an
    // inner round of a smaller radius than the kerf.
    // An inner round of a smaller radius than the kerf is the exception:
    // the beam cannot follow it, as it cannot follow a sharp inner corner,
    // and both come out rounded at the kerf's radius; it is said, not
    // refused.
    let mut notes = Vec::new();
    for (id, label) in labels.iter().enumerate() {
        let pieces = out
            .origins
            .iter()
            .filter(|o| o.source == OffsetSource::Curve(id as u64))
            .count();
        if pieces == 0
            && let Some(r) = curves.get(id).and_then(|c| inner_round(c, k, tolerance))
        {
            notes.push(format!(
                "{label}, an inner round of radius {r}, comes out at the kerf's radius {k}"
            ));
            continue;
        }
        if pieces != 1 {
            let fits = largest_keeping(shape, k, tolerance, &labels)
                .map(|d| format!("; the largest kerf that cuts every curve is {d}"))
                .unwrap_or_default();
            return Err(format!(
                "the kerf {k} does not follow {label} ({}): a notch, a slot or an inner corner narrower than the beam, which the beam cannot cut{fits}",
                if pieces == 0 {
                    "it is left out"
                } else {
                    "it falls apart"
                }
            ));
        }
    }
    let [region] = out.profiles.as_slice() else {
        return Err(format!(
            "the kerf {k} leaves {} regions, not one",
            out.profiles.len()
        ));
    };
    Ok((
        Shape {
            outline: path_of(&region.outer)?,
            holes: region.holes.iter().map(path_of).collect::<Result<_, _>>()?,
            marks: shape.marks.clone(),
        },
        notes,
    ))
}

/// The radius of `curve` when it is an arc the kerf `k` shrinks to
/// nothing (an inner round of radius at most `k`): such an arc has no
/// offset, and the path turns about it at radius `k`.
fn inner_round(curve: &SketchCurve, k: f64, tolerance: &Tolerance) -> Option<f64> {
    let Curve2::Arc { center, start, .. } = curve.curve else {
        return None;
    };
    let r = (start[0] - center[0]).phypot(start[1] - center[1]);
    (r <= k + tolerance.linear).then_some(r)
}

/// Whether the kerf `k` keeps one offset piece of every curve and every
/// loop.
fn keeps_all(curves: &[SketchCurve], k: f64, tolerance: &Tolerance) -> bool {
    let Ok(profiles) = detect_profiles(curves, tolerance) else {
        return false;
    };
    let Ok(out) = offset_profiles(&profiles, k, Corners::Round, tolerance) else {
        return false;
    };
    out.loops_after == out.loops_before
        && out.regions_after == out.regions_before
        && curves.iter().enumerate().all(|(id, curve)| {
            let pieces = out
                .origins
                .iter()
                .filter(|o| o.source == OffsetSource::Curve(id as u64))
                .count();
            pieces == 1 || (pieces == 0 && inner_round(curve, k, tolerance).is_some())
        })
}

/// The largest kerf below `k` that keeps every curve, to a thousandth of
/// `k`, by bisection (at most 12 offsets).
fn largest_keeping(shape: &Shape, k: f64, tolerance: &Tolerance, labels: &[String]) -> Option<f64> {
    let mut curves = Vec::new();
    let mut next = 0_u64;
    for path in std::iter::once(&shape.outline).chain(&shape.holes) {
        curves_of(path, &mut next, &mut curves).ok()?;
    }
    if curves.len() != labels.len() {
        return None;
    }
    let (mut lo, mut hi) = (0.0, k);
    for _ in 0..12 {
        let mid = 0.5 * (lo + hi);
        if keeps_all(&curves, mid, tolerance) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo > 0.0).then_some(lo)
}
