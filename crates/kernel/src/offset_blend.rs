//! The offsets of fillet faces: rebuilt, not offset.
//!
//! A face operation that moves a rolling-ball fillet by `d` together with
//! the two faces it rolls along gives it the fillet of radius `r ± d`
//! between those faces' offsets ([`crate::blend::rebuild`]): exact,
//! tangent to them by construction, and in the parameters of the old
//! face, so every edge along its borders is that border again
//! ([`crate::tweak::Iso`]). Which faces are fillets is read off the body
//! (see [`fillets`]); the radius a face was made with, when the caller
//! knows it from the face's name, is taken for the measured one when they
//! agree.
//!
//! **Refused, with the faces, before anything is built:** a fillet that
//! moves while a face it rolls along does not move with it by the same
//! distance (its offset would not be tangent to that face, and a fillet
//! of another radius between the faces as they are is not its offset);
//! and a distance at which the radius reaches zero, as too far, with the
//! largest that fits (the radius, in closed form: no body is built in
//! the search for it).

use crate::blend::rebuild::{self, Ball, NoRebuild, Support};
use crate::offset_normal::v_border_neighbours;
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, FaceId};
use std::collections::{BTreeMap, BTreeSet};

/// A fillet face of a body: the faces along its borders `v = 0` and
/// `v = 1`, its ball, and whether an edge lies along its borders
/// `u = start` and `u = end` (a joint with the next fillet of a chain,
/// past which it is not continued).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Fillet {
    pub supports: [FaceId; 2],
    pub ball: Ball,
    pub joints: [bool; 2],
}

/// How far past a joint with the next fillet of a chain a rebuilt fillet
/// reaches, as a share of its old surface's parameter range.
const JOINT_MARGIN: f64 = 1e-3;

pub(crate) const STAYS: &str = "a face this fillet rolls along does not move with it by the same distance: the offset of a fillet is the fillet of the radius changed by the distance between the faces it rolls along moved by that distance, and it would not meet this face";
pub(crate) const VANISHES: &str =
    "the fillet's radius would reach zero: the fillet would vanish into an edge";
pub(crate) const CIRCULAR: &str =
    "the fillets rest on each other in a circle: none can be rebuilt before another";

/// The fillet faces among the freeform faces of `moving`: a face whose
/// borders `v = 0` and `v = 1` each lie along edges to one other face, and
/// which is, to the tolerance, the rolling-ball fillet between those two
/// ([`rebuild::fillet_of`]); `radii` are the radii faces were made with,
/// when known.
pub(crate) fn fillets(
    body: &Body,
    moving: &BTreeSet<FaceId>,
    radii: &BTreeMap<FaceId, f64>,
    tolerance: &Tolerance,
) -> BTreeMap<FaceId, Fillet> {
    let mut found = BTreeMap::new();
    for face in moving {
        let Some(data) = body.face(*face) else {
            continue;
        };
        let Surface::Nurbs(surface) = &data.surface else {
            continue;
        };
        let Some(supports) = v_border_neighbours(body, *face, tolerance.linear) else {
            continue;
        };
        let Some(sides) = supports_of(body, supports) else {
            continue;
        };
        if let Some(ball) = rebuild::fillet_of(
            surface,
            data.reversed,
            sides,
            radii.get(face).copied(),
            tolerance,
        ) {
            let joints = crate::offset_normal::u_borders_taken(body, *face, tolerance.linear);
            found.insert(
                *face,
                Fillet {
                    supports,
                    ball,
                    joints,
                },
            );
        }
    }
    found
}

fn supports_of(body: &Body, faces: [FaceId; 2]) -> Option<[Support<'_>; 2]> {
    let [a, b] = faces.map(|f| body.face(f));
    let (a, b) = (a?, b?);
    Some([
        Support {
            surface: &a.surface,
            reversed: a.reversed,
        },
        Support {
            surface: &b.surface,
            reversed: b.reversed,
        },
    ])
}

/// What refuses moving `fillets` by the distances of `moving` before
/// anything is built: a face a fillet rolls along that does not move with
/// it by the same distance ([`STAYS`], with both faces).
pub(crate) fn stays(
    fillets: &BTreeMap<FaceId, Fillet>,
    moving: &BTreeMap<FaceId, f64>,
) -> Option<Vec<FaceId>> {
    for (face, fillet) in fillets {
        let Some(distance) = moving.get(face) else {
            continue;
        };
        for support in fillet.supports {
            if moving.get(&support) != Some(distance) {
                return Some(vec![*face, support]);
            }
        }
    }
    None
}

/// The largest part of the distances of `moving` at which no fillet's
/// radius reaches zero (`1` or more when none does), with the fillet that
/// limits it.
pub(crate) fn vanishing(
    fillets: &BTreeMap<FaceId, Fillet>,
    moving: &BTreeMap<FaceId, f64>,
    tolerance: &Tolerance,
) -> Option<(f64, FaceId)> {
    let mut least: Option<(f64, FaceId)> = None;
    for (face, fillet) in fillets {
        let Some(distance) = moving.get(face) else {
            continue;
        };
        // How fast the radius shrinks per unit of the distance.
        let shrink = if fillet.ball.convex {
            -distance
        } else {
            *distance
        };
        if shrink.is_nan() || shrink <= 0.0 {
            continue;
        }
        let part = (fillet.ball.radius - tolerance.linear) / shrink;
        if least.is_none_or(|(p, _)| part < p) {
            least = Some((part, *face));
        }
    }
    least
}

/// The surfaces of `fillets` moved by their distances in `moving`,
/// rebuilt between the new surfaces of their supports in `surfaces`
/// (which they are added to); `past` says, per fillet, how far (in the
/// model's units) to go on past the old surface's start and end.
pub(crate) fn rebuild_all(
    body: &Body,
    fillets: &BTreeMap<FaceId, Fillet>,
    moving: &BTreeMap<FaceId, f64>,
    surfaces: &mut BTreeMap<FaceId, Surface>,
    past: &dyn Fn(FaceId) -> (f64, f64),
    tolerance: &Tolerance,
) -> Result<(), (FaceId, NoRebuild)> {
    let mut left: Vec<FaceId> = fillets
        .keys()
        .copied()
        .filter(|f| moving.contains_key(f))
        .collect();
    // A fillet whose support is a fillet waits for it.
    while !left.is_empty() {
        let mut waiting = Vec::new();
        let mut done_one = false;
        for face in left {
            let (Some(fillet), Some(distance)) = (fillets.get(&face), moving.get(&face)) else {
                continue;
            };
            if fillet.supports.iter().any(|s| !surfaces.contains_key(s)) {
                waiting.push(face);
                continue;
            }
            let fails = |why: NoRebuild| (face, why);
            let (Some(data), Some(old)) = (body.face(face), supports_of(body, fillet.supports))
            else {
                return Err(fails(NoRebuild::Fails(rebuild::UNSOLVED)));
            };
            let Surface::Nurbs(surface) = &data.surface else {
                return Err(fails(NoRebuild::Fails(rebuild::UNSOLVED)));
            };
            let made = {
                let new = |k: usize| -> Option<Support<'_>> {
                    let support = *fillet.supports.get(k)?;
                    Some(Support {
                        surface: surfaces.get(&support)?,
                        reversed: body.face(support)?.reversed,
                    })
                };
                let (Some(a), Some(b)) = (new(0), new(1)) else {
                    return Err(fails(NoRebuild::Fails(rebuild::UNSOLVED)));
                };
                let (range, core) = extent(body, face, surface, fillet, past(face));
                rebuild::rebuilt(
                    surface,
                    fillet.ball,
                    [a, b],
                    old,
                    *distance,
                    range,
                    core,
                    fillet.joints,
                    tolerance,
                )
                .map_err(fails)?
            };
            surfaces.insert(face, Surface::Nurbs(made));
            done_one = true;
        }
        if !done_one && !waiting.is_empty() {
            return Err((
                waiting.first().copied().unwrap_or(FaceId(0)),
                NoRebuild::Fails(CIRCULAR),
            ));
        }
        left = waiting;
    }
    Ok(())
}

/// The part of `surface`'s `u` the fillet `face` needs rebuilt: where its
/// edges lie on it, a step of a fiftieth of that more each way (the face
/// moves along it as its ends are found again), and `past` (lengths, in
/// the model's units) more before and after where the closing of the body
/// asks for the face to grow; never past a joint with the next fillet of
/// a chain (an edge along the border there), and inside the old surface
/// but for `past`.
fn extent(
    body: &Body,
    face: FaceId,
    surface: &NurbsSurface,
    fillet: &Fillet,
    past: (f64, f64),
) -> ((f64, f64), (f64, f64)) {
    let ((u0, u1), _) = surface.domain();
    let whole = Surface::Nurbs(surface.clone());
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    if let Some(data) = body.face(face) {
        for coedge in data
            .loops
            .iter()
            .filter_map(|l| body.loop_(*l))
            .flat_map(|l| &l.coedges)
        {
            let Some(edge) = body.edge(coedge.edge) else {
                continue;
            };
            for k in 0..=8 {
                let t = edge.range.0 + (edge.range.1 - edge.range.0) * f64::from(k) / 8.0;
                if let Ok((u, _)) = whole.project(edge.curve.eval(t)) {
                    lo = lo.min(u);
                    hi = hi.max(u);
                }
            }
        }
    }
    if !(lo.is_finite() && hi.is_finite() && hi > lo) {
        return ((u0, u1), (u0, u1));
    }
    let step = 0.02 * (hi - lo);
    let (by_start, by_end) = in_u(surface, past.0, past.1);
    let [joint_start, joint_end] = fillet.joints;
    // At a joint, a little past it: the next fillet's section there was
    // found in its own plane, so the two meet only within the tolerance
    // and each has to reach a little past the other's end (the balls past
    // it are followed as far as they are found on the faces continued).
    let joint = JOINT_MARGIN * (u1 - u0);
    let start = if joint_start {
        u0 - joint
    } else {
        (lo - step).max(u0) - by_start
    };
    let end = if joint_end {
        u1 + joint
    } else {
        (hi + step).min(u1) + by_end
    };
    ((start, end), (lo, hi))
}

/// Lengths `a` and `b` past the start and the end of `surface` along `u`,
/// in its parameter: by the fastest speed across the border there.
fn in_u(surface: &NurbsSurface, a: f64, b: f64) -> (f64, f64) {
    let ((u0, u1), _) = surface.domain();
    let at = |u: f64, by: f64| {
        if !(by > 0.0 && by.is_finite()) {
            return 0.0;
        }
        let speed = (0..=4)
            .map(|k| surface.derivatives(u, f64::from(k) / 4.0).1.length())
            .fold(0.0_f64, f64::max);
        if speed > 0.0 && speed.is_finite() {
            by / speed
        } else {
            0.0
        }
    };
    (at(u0, a), at(u1, b))
}
