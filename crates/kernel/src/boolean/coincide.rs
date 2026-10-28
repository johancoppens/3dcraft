//! Faces that lie on one another where one is over the other, though their
//! surfaces part beyond them (docs/design/booleans.md › *Faces that lie on
//! one another over a region*).
//!
//! `meet_surfaces` takes two surfaces for one when a grid of one lies on
//! the other everywhere in the box it is asked in. Two freeform faces can
//! lie on one another over the whole of where they overlap while their
//! surfaces, continued past the faces, part: a plate's rim cut from the
//! skin of the plate over it and traced past its ends, the same skin
//! fitted twice over different ranges, a patch and a patch that agrees
//! with it only over the first spans. Inside the box the surfaces are then
//! neither one (the grid finds them apart past the faces) nor two that
//! cross (the distance between them is zero over an area, which the
//! marcher cannot trace), and the pair was refused.
//!
//! What the boolean needs is not that the surfaces are one, but that the
//! *faces* are, wherever one is over the other: then the region they share
//! is bounded by edges of the faces (which the faces beside them imprint,
//! as for coincident planes), and its class is the same or the opposite
//! orientation, as for any coincident pair. That is what [`coincide`]
//! tells, by sampling each freeform face of the pair over its own trimmed
//! parameters, span by span of its surface:
//!
//! - a sample inside its face whose nearest point of the other surface is
//!   inside the other face is *over* the other face;
//! - every sample over the other face must be within a quarter of the
//!   tolerance of it, and the samples between them must not bend away by
//!   more than a quarter more (half the tolerance in all, read from the
//!   second differences of the distance along each row of samples, as the
//!   gap rule reads faces over each other: within half the tolerance they
//!   are one surface);
//! - at least [`EVIDENCE`] samples must be over the other face (an area,
//!   not an edge both share), and their normals must agree throughout or
//!   be opposite throughout.
//!
//! When samples over the other face are within the tolerance and others,
//! also over it, are apart, the faces share a region whose boundary is no
//! edge of either (the surfaces part inside both faces): that is refused,
//! never guessed ([`PARTLY`]). Anything else (too few samples over the
//! other face, a face too large to sample, a side that cannot be told) is
//! no evidence, and the caller keeps the refusal it had.

use super::freeform::is_freeform;
use super::outline::{Outline, Side};
use super::section::Solid;
use super::{Reason, Stop};
use craft3d_geom::Surface;

/// What [`coincide`] found of a pair of faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Coincidence {
    /// The faces lie on one another wherever one is over the other; the
    /// value says whether their surfaces' normals agree there.
    Faces(bool),
    /// They lie on one another over part of where one is over the other
    /// and apart over the rest ([`PARTLY`]).
    Partly,
    /// No evidence either way.
    Unknown,
}

pub(super) const PARTLY: Reason = "two faces lie on one another over part of where one is over the other and apart over the rest, so the region they share is bounded by no edge of either (surfaces that part inside both faces)";

/// Samples over the other face, within the tolerance, that make an area.
const EVIDENCE: usize = 16;
/// Samples per span of the surface, each way.
const PER_SPAN: usize = 8;
/// The fewest pieces each way across a face's parameters.
const LEAST: usize = 32;
/// Places along each piece of a face's boundary projected to find the
/// parameters the face spans.
const BORDER_PLACES: usize = 64;
/// The most samples of one face; past this the face is not sampled.
const MOST: usize = 1 << 18;

/// The distances of one row of samples, `NaN` where a sample is not over
/// the other face.
type Row = Vec<f64>;

/// Whether the faces `fa` of `a` and `fb` of `b` lie on one another
/// wherever one is over the other (the module's documentation).
pub(super) fn coincide(
    a: &Solid,
    fa: usize,
    b: &Solid,
    fb: usize,
    tol: f64,
) -> Result<Coincidence, Stop> {
    let (oa, ob) = (a.outline(fa)?, b.outline(fb)?);
    if !(is_freeform(&oa.surface) || is_freeform(&ob.surface)) {
        return Ok(Coincidence::Unknown);
    }
    let mut same = None;
    let mut evidence = 0_usize;
    let mut partly = false;
    for ((own, of_own), (other, of_other)) in [((a, fa), (b, fb)), ((b, fb), (a, fa))] {
        if !is_freeform(&own.outline(of_own)?.surface) {
            continue;
        }
        match sample((own, of_own), (other, of_other), tol) {
            Found::Over { agree, count } => {
                if same.is_some_and(|s| s != agree) {
                    return Ok(Coincidence::Unknown);
                }
                same = Some(agree);
                evidence += count;
            }
            Found::Partly => partly = true,
            Found::Nothing => {}
            Found::Unknown => return Ok(Coincidence::Unknown),
        }
    }
    Ok(match (partly, same) {
        (true, _) => Coincidence::Partly,
        (false, Some(agree)) if evidence >= EVIDENCE => Coincidence::Faces(agree),
        _ => Coincidence::Unknown,
    })
}

enum Found {
    /// Every sample over the other face is on it: how many there are,
    /// and whether the normals agree.
    Over { agree: bool, count: usize },
    /// At least [`EVIDENCE`] on it and some apart.
    Partly,
    /// None over the other face.
    Nothing,
    /// Cannot be told.
    Unknown,
}

/// The parameters at which `own` is sampled: each span of its surface
/// within `lo..hi` in [`PER_SPAN`] pieces, and at least [`LEAST`] pieces
/// in all.
fn stations(breaks: &[f64], lo: f64, hi: f64) -> Vec<f64> {
    let mut cuts: Vec<f64> = vec![lo];
    cuts.extend(breaks.iter().copied().filter(|&k| k > lo && k < hi));
    cuts.push(hi);
    cuts.dedup();
    let spans = cuts.len().saturating_sub(1).max(1);
    let per = PER_SPAN.max(LEAST.div_ceil(spans));
    let mut out = Vec::new();
    for pair in cuts.windows(2) {
        let [a, b] = pair else { continue };
        for k in 0..per {
            out.push(a + (b - a) * k as f64 / per as f64);
        }
    }
    out.push(hi);
    out
}

/// The knots of a freeform surface along u (`along_u`) or v.
fn knots(surface: &Surface, along_u: bool) -> Vec<f64> {
    match surface {
        Surface::Nurbs(n) => {
            let mut k = if along_u {
                n.knots_u().to_vec()
            } else {
                n.knots_v().to_vec()
            };
            k.dedup();
            k
        }
        _ => Vec::new(),
    }
}

/// The parameters of `own`'s surface that its loops span (the whole
/// domain for a face without loops), within the domain.
fn trimmed(own: &Outline) -> Option<((f64, f64), (f64, f64))> {
    let ((u0, u1), (v0, v1)) = own.surface.domain();
    if own.loops.is_empty() {
        return Some(((u0, u1), (v0, v1)));
    }
    let (mut ulo, mut uhi, mut vlo, mut vhi) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for seg in own.loops.iter().flatten() {
        for i in 0..=BORDER_PLACES {
            let p = seg.at(i as f64 / BORDER_PLACES as f64);
            let (u, v) = own.sheet().project(p)?;
            ulo = ulo.min(u);
            uhi = uhi.max(u);
            vlo = vlo.min(v);
            vhi = vhi.max(v);
        }
    }
    // (Grown by a fiftieth, for a boundary that bulges between the
    // places it was projected from.)
    let (pu, pv) = (0.02 * (uhi - ulo), 0.02 * (vhi - vlo));
    let (ulo, uhi) = ((ulo - pu).max(u0), (uhi + pu).min(u1));
    let (vlo, vhi) = ((vlo - pv).max(v0), (vhi + pv).min(v1));
    (ulo < uhi && vlo < vhi).then_some(((ulo, uhi), (vlo, vhi)))
}

fn sample((solid, face): (&Solid, usize), (twin, of_twin): (&Solid, usize), tol: f64) -> Found {
    let (Ok(own), Ok(other), Some(other_box)) = (
        solid.outline(face),
        twin.outline(of_twin),
        twin.boxes.get(of_twin),
    ) else {
        return Found::Unknown;
    };
    let Some(((ulo, uhi), (vlo, vhi))) = trimmed(own) else {
        return Found::Unknown;
    };
    let us = stations(&knots(&own.surface, true), ulo, uhi);
    let vs = stations(&knots(&own.surface, false), vlo, vhi);
    if us.len().saturating_mul(vs.len()) > MOST {
        return Found::Unknown;
    }
    let mut rows: Vec<Row> = Vec::with_capacity(us.len());
    let mut agree: Option<bool> = None;
    let (mut count, mut apart) = (0_usize, false);
    for &u in &us {
        let mut row = Vec::with_capacity(vs.len());
        for &v in &vs {
            row.push(f64::NAN);
            let p = own.surface.eval(u, v);
            if !(p.is_finite() && other_box.contains(p, 2.0 * tol)) {
                continue;
            }
            let Some((pu, pv)) = other.sheet().project(p) else {
                return Found::Unknown;
            };
            let q = other.surface.eval(pu, pv);
            let d = p.distance(q);
            if !d.is_finite() {
                return Found::Unknown;
            }
            // Only a sample of this face whose foot is inside the other
            // face is over it.
            let near = d <= 0.25 * tol;
            // Every sample near the other surface is asked whether it is
            // over the other face and how its normal lies there, not only
            // the first few: a fold or a turned patch further on must not
            // be taken for the same orientation.
            // (The other face's cover proves most feet outside it at no
            // cost: a foot further than four tolerances from the face.)
            if !twin.may_hold(of_twin, q, tol) {
                continue;
            }
            let over = match other.side(q, tol) {
                Ok(Side::In) => match own.side(p, tol) {
                    Ok(Side::In) => true,
                    Ok(_) => false,
                    Err(_) => return Found::Unknown,
                },
                Ok(_) => false,
                Err(_) => return Found::Unknown,
            };
            if !over {
                continue;
            }
            if !near {
                apart = true;
                continue;
            }
            if let Some(last) = row.last_mut() {
                *last = d;
            }
            count += 1;
            let (Some(n), Some(m)) = (own.surface.normal(u, v), other.surface.normal(pu, pv))
            else {
                return Found::Unknown;
            };
            let here = n.dot(m) > 0.0;
            if agree.is_some_and(|a| a != here) {
                return Found::Unknown;
            }
            agree = Some(here);
        }
        rows.push(row);
    }
    if apart {
        return if count >= EVIDENCE {
            Found::Partly
        } else {
            Found::Unknown
        };
    }
    let Some(agree) = agree else {
        return Found::Nothing;
    };
    if !between(&rows, tol) {
        return Found::Unknown;
    }
    Found::Over { agree, count }
}

/// True when the distance between the samples stays within half the
/// tolerance: the largest sample plus an eighth of the largest second
/// difference along a row or a column of three samples over the other
/// face (the most a function with that second difference rises between
/// two samples above the chord through them).
fn between(rows: &[Row], tol: f64) -> bool {
    let mut most = 0.0_f64;
    let mut bend = 0.0_f64;
    let second = |a: f64, b: f64, c: f64| (a - 2.0 * b + c).abs();
    for (i, row) in rows.iter().enumerate() {
        for (j, &d) in row.iter().enumerate() {
            if d.is_nan() {
                continue;
            }
            most = most.max(d);
            if let (Some(&l), Some(&r)) = (row.get(j.wrapping_sub(1)), row.get(j + 1))
                && !(l.is_nan() || r.is_nan())
            {
                bend = bend.max(second(l, d, r));
            }
            let up = i.checked_sub(1).and_then(|k| rows.get(k)?.get(j)).copied();
            let down = rows.get(i + 1).and_then(|r| r.get(j)).copied();
            if let (Some(l), Some(r)) = (up, down)
                && !(l.is_nan() || r.is_nan())
            {
                bend = bend.max(second(l, d, r));
            }
        }
    }
    most + 0.125 * bend <= 0.5 * tol
}
