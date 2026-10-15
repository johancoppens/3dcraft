//! What fits. Whether a blend of the size asked for fits is not estimated:
//! the blend is built exactly ([`super::build`]) and the result is held
//! against what a blend is ([`super::finish`]): every blend face there, in
//! one piece, bordering both its neighbours and nothing but the faces at
//! the ends of its edge, tangent where it should be, and the boolean's own
//! checks. When that fails in a way a smaller size can mend (the blend
//! reaches a face it should not, takes a neighbour away, comes out in
//! pieces, or a tool the boolean or the sweep refuses), the largest size
//! that fits is found by building again, smaller, until one builds.
//!
//! **Where to look.** The size at which a blend stops fitting is where
//! its border on a face first reaches something of that face: a vertex,
//! the nearest point of an edge, what another blend takes of the face
//! from the other side, the axis a circular edge turns about, the size at
//! which a fillet's torus reaches its own axis. Those sizes (*events*)
//! are worked out exactly from the faces, without deciding which of them
//! matter: which one does is decided by the builds, by a binary search
//! over them (just past an event the build fails, just before it, it
//! builds). An event that explains the failure is reported exactly; when
//! none does, the size is bisected on the builds alone and the smallest
//! size seen to fail is reported. Every build is bounded ([`MOST_BUILDS`]).

use super::{BlendError, Failure, Path, Plan, Reason, Request, Size, Use, build, face_edges, plan};
use crate::KernelError;
use crate::chamfer::ChamferSize;
use craft3d_geom::Curve3;
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, EdgeId, VertexId};
use std::collections::{BTreeMap, BTreeSet};

/// Most builds one search for the largest size that fits makes.
const MOST_BUILDS: usize = 48;

/// How many tolerances past an event the build that says it matters is
/// made.
const PAST: f64 = 64.0;

/// How many tolerances before an event the builds that say it is where
/// the blend stops fitting are made, nearest first.
const BEFORE: [f64; 3] = [4.0, 64.0, 1024.0];

/// The step down in size while nothing is known to build.
const QUARTER: f64 = 0.25;

/// Points taken along an edge to find where it comes nearest.
const SAMPLES: u32 = 64;

/// A size at which the border of a blend on a face may reach something
/// of that face.
#[derive(Clone, Copy, Debug)]
struct Event {
    /// The size, as a multiple of the size asked for.
    scale: f64,
    /// The tolerance along the face, as a multiple of the size asked for:
    /// the builds either side of the event are made a few of these away.
    unit: f64,
    /// The plan and the side of its edge whose face it is on.
    plan: usize,
    side: usize,
    /// The size it is in the words of the blend (a fillet's radius, a
    /// chamfer's distance along the face).
    size: f64,
}

/// Whether a smaller size could mend what a build failed with.
fn sized(error: &BlendError) -> bool {
    match error {
        BlendError::Boolean { .. } => true,
        BlendError::Kernel(KernelError::InvalidBody { .. } | KernelError::InvalidSize { .. })
        | BlendError::TooLarge { .. } => false,
        BlendError::Kernel(_) => true,
        BlendError::Refused { reason, .. } => matches!(
            reason,
            Reason::Reaches
                | Reason::LostNeighbour
                | Reason::InPieces
                | Reason::EndsBadly
                | Reason::NotTangent
        ),
    }
}

/// Whether a failure carries evidence from the result that the faces
/// leave no room for the blend: [`super::finish`] found it reaching a
/// face it should not, taking a neighbour away or in pieces. A boolean or
/// a sweep that refuses a tool says nothing about room, and may refuse
/// the same at every size (work owed, not a size).
fn no_room(error: &BlendError) -> bool {
    matches!(
        error,
        BlendError::Refused {
            reason: Reason::Reaches | Reason::LostNeighbour | Reason::InPieces,
            ..
        }
    )
}

/// Whether two failures are the same failure: the same refusal, or both
/// the boolean's, or both the same kind of kernel error.
fn same_way(a: &BlendError, b: &BlendError) -> bool {
    match (a, b) {
        (BlendError::Refused { reason: x, .. }, BlendError::Refused { reason: y, .. }) => x == y,
        (BlendError::Boolean { .. }, BlendError::Boolean { .. }) => true,
        (BlendError::Kernel(x), BlendError::Kernel(y)) => {
            std::mem::discriminant(x) == std::mem::discriminant(y)
        }
        _ => false,
    }
}

/// `error`, said to fail the same way at smaller sizes too (where it can
/// say so: a boolean's refusal).
fn smaller_too(error: BlendError) -> BlendError {
    match error {
        BlendError::Boolean { edge, error, .. } => BlendError::Boolean {
            edge,
            error,
            smaller_fails: true,
        },
        other => other,
    }
}

/// What one build at a size gave.
#[derive(Clone, Debug)]
enum Outcome {
    Built,
    Failed(BlendError),
    /// Not built: the search had made all the builds it may.
    Spent,
}

/// `size` times `scale`.
fn scaled(size: Size, scale: f64) -> Size {
    match size {
        Size::Radius(radius) => Size::Radius(radius * scale),
        Size::Chamfer(ChamferSize::Distances { first, second }) => {
            Size::Chamfer(ChamferSize::Distances {
                first: first * scale,
                second: second * scale,
            })
        }
        Size::Chamfer(ChamferSize::DistanceAngle { distance, angle }) => {
            Size::Chamfer(ChamferSize::DistanceAngle {
                distance: distance * scale,
                angle,
            })
        }
    }
}

/// The requests that change size with request `index`: all those of
/// its size. A feature has one size for all its edges, and the largest
/// that fits is the size at which they all build together (two fillets
/// that would meet in the middle of a face fit up to half its width each).
fn group(requests: &[Request], index: usize) -> BTreeSet<usize> {
    let Some(own) = requests.get(index) else {
        return BTreeSet::new();
    };
    requests
        .iter()
        .enumerate()
        .filter(|(_, request)| request.size == own.size)
        .map(|(i, _)| i)
        .collect()
}

/// One search: the body, the requests, and the builds made so far.
struct Search<'a> {
    body: &'a Body,
    uses: &'a [Vec<Use>],
    at: &'a [Vec<EdgeId>],
    requests: &'a [Request],
    group: &'a BTreeSet<usize>,
    tolerance: &'a Tolerance,
    builds: usize,
    seen: BTreeMap<u64, Outcome>,
    /// Whether a size was asked for after the bound of builds.
    spent: bool,
}

impl Search<'_> {
    /// Builds the blend with the group's sizes times `scale`. Past its
    /// bound of builds a search builds nothing more.
    fn attempt(&mut self, scale: f64) -> Outcome {
        if let Some(known) = self.seen.get(&scale.to_bits()) {
            return known.clone();
        }
        if self.builds >= MOST_BUILDS {
            self.spent = true;
            return Outcome::Spent;
        }
        self.builds += 1;
        let tol = self.tolerance.linear;
        let mut plans = Vec::with_capacity(self.requests.len());
        let mut failed = None;
        for (i, request) in self.requests.iter().enumerate() {
            let request = if self.group.contains(&i) {
                Request {
                    size: scaled(request.size, scale),
                    ..*request
                }
            } else {
                *request
            };
            match plan(self.body, self.uses, &request, tol) {
                Ok(made) => plans.push(made),
                Err(error) => {
                    failed = Some(error);
                    break;
                }
            }
        }
        let failed = match failed {
            Some(error) => Some(error),
            None => build(self.body, self.uses, self.at, plans, self.tolerance)
                .err()
                .map(|f| f.error),
        };
        let outcome = failed.map_or(Outcome::Built, Outcome::Failed);
        self.seen.insert(scale.to_bits(), outcome.clone());
        outcome
    }

    fn fits(&mut self, scale: f64) -> bool {
        matches!(self.attempt(scale), Outcome::Built)
    }
}

/// What a distance along the face on `side` is in the words of the blend:
/// a fillet's radius over the distance where the ball touches, one for a
/// chamfer.
fn per(plan: &Plan, side: usize) -> f64 {
    match plan.scale.get(side) {
        Some(per) if plan.round.is_some() && per.is_finite() => *per,
        _ => 1.0,
    }
}

/// The refusal for the face on `side` of `plan`, with `largest` in the
/// words of the blend.
fn too_large(plan: &Plan, side: usize, largest: f64) -> BlendError {
    let distance = plan.distances.get(side).copied().unwrap_or(0.0);
    let [first, _] = plan.faces;
    BlendError::TooLarge {
        edge: plan.edge,
        face: plan.faces.get(side).copied().unwrap_or(first),
        size: distance * per(plan, side),
        largest: largest.max(0.0),
    }
}

/// How far across the edge of `plan` a point is, along the face on
/// `side`: for a straight edge measured along the face square to it, for
/// a circular one in the plane through the axis.
fn across(body: &Body, plan: &Plan, side: usize, point: Vec3) -> f64 {
    match plan.path {
        Path::Line { start, .. } => plan.coordinate(side, point - start),
        Path::Circle { centre, axis, .. } => {
            let level = |p: Vec3| {
                let offset = p - centre;
                let height = offset.dot(axis);
                ((offset - axis * height).length(), height)
            };
            let start = body
                .edge(plan.edge)
                .map(|e| e.curve.eval(e.range.0))
                .unwrap_or(centre);
            let (r0, h0) = level(start);
            let (r, h) = level(point);
            (r - r0).hypot(h - h0)
        }
    }
}

/// Where the edge `other` comes nearest across the edge of `plan` on
/// `side`: the measure at each of its local least points, found among
/// samples and refined by golden sections (the ends of the edge are its
/// vertices, measured apart).
fn nearest(body: &Body, plan: &Plan, side: usize, other: EdgeId) -> Vec<f64> {
    let Some(data) = body.edge(other) else {
        return Vec::new();
    };
    // Along a straight edge between planes the measure along a line is
    // linear: least at an end.
    if plan.flat()
        && matches!(plan.path, Path::Line { .. })
        && matches!(data.curve, Curve3::Line(_))
    {
        return Vec::new();
    }
    let (t0, t1) = data.range;
    let at = |s: f64| t0 + s * (t1 - t0);
    let measure = |s: f64| across(body, plan, side, data.curve.eval(at(s)));
    let values: Vec<f64> = (0..=SAMPLES)
        .map(|i| measure(f64::from(i) / f64::from(SAMPLES)))
        .collect();
    let mut found = Vec::new();
    for (i, window) in values.windows(3).enumerate() {
        let [a, b, c] = window else { continue };
        if !(b <= a && b <= c) {
            continue;
        }
        let step = 1.0 / f64::from(SAMPLES);
        let middle = f64::from(u32::try_from(i + 1).unwrap_or(SAMPLES)) * step;
        let (mut lo, mut hi) = (middle - step, middle + step);
        let ratio = 0.5 * (5.0_f64.sqrt() - 1.0);
        for _ in 0..60 {
            let x1 = hi - ratio * (hi - lo);
            let x2 = lo + ratio * (hi - lo);
            if measure(x1) <= measure(x2) {
                hi = x2;
            } else {
                lo = x1;
            }
        }
        let least = measure(0.5 * (lo + hi)).min(*b);
        if least.is_finite() {
            found.push(least);
        }
    }
    found
}

/// Every event of the plans of the group, on both faces of each.
fn events(body: &Body, plans: &[Plan], group: &BTreeSet<usize>, tol: f64) -> Vec<Event> {
    let mut found = Vec::new();
    for &index in group {
        let Some(plan) = plans.get(index) else {
            continue;
        };
        let ends: Vec<VertexId> = plan.ends.into_iter().flatten().collect();
        for (side, face) in plan.faces.into_iter().enumerate() {
            let Some(&distance) = plan.distances.get(side) else {
                continue;
            };
            if !(distance.is_finite() && distance > 0.0) {
                continue;
            }
            let per = per(plan, side);
            let unit = tol / distance;
            let mut push = |reach: f64| {
                if reach.is_finite() && reach > tol {
                    found.push(Event {
                        scale: reach / distance,
                        unit,
                        plan: index,
                        side,
                        size: reach * per,
                    });
                }
            };
            // What of the face the border can reach.
            let mut reaches = Vec::new();
            let mut corners = BTreeSet::new();
            for other in face_edges(body, face) {
                if other == plan.edge {
                    continue;
                }
                if let Some(data) = body.edge(other) {
                    corners.insert(data.start);
                    corners.insert(data.end);
                }
                reaches.extend(nearest(body, plan, side, other));
            }
            for vertex in corners {
                if ends.contains(&vertex) {
                    continue;
                }
                if let Some(v) = body.vertex(vertex) {
                    reaches.push(across(body, plan, side, v.point));
                }
            }
            for &reach in &reaches {
                push(reach);
                // Less what another blend takes of the face.
                for (other, there) in plans.iter().enumerate() {
                    if other == index {
                        continue;
                    }
                    let Some(taken) = there.distance_on(face) else {
                        continue;
                    };
                    if group.contains(&other) {
                        push(reach * (distance / (distance + taken)));
                    } else {
                        push(reach - taken);
                    }
                }
            }
            if let Path::Circle {
                axis,
                radial,
                radius,
                ..
            } = plan.path
            {
                // The border reaches the axis.
                let Some(&into) = plan.into.get(side) else {
                    continue;
                };
                let inward = -into.dot(radial);
                if inward > 1e-9 && into.dot(axis).abs() < 1.0 {
                    push(radius / inward);
                }
                // A fillet's torus reaches its own axis: the ball's centre
                // comes as near to the axis as its radius.
                if let Some(round) = plan.round {
                    let [a, b] = plan.into;
                    let cosine = a.dot(b).clamp(-1.0, 1.0);
                    let half_tan = (1.0 - cosine * cosine).max(0.0).sqrt() / (1.0 + cosine);
                    let towards = (a + b).dot(radial) / (half_tan * (1.0 + cosine));
                    let at = radius / (1.0 - towards);
                    if side == 0 && at.is_finite() && at > tol && round > 0.0 {
                        found.push(Event {
                            scale: at / round,
                            unit: tol / round,
                            plan: index,
                            side,
                            size: at,
                        });
                    }
                }
            }
        }
    }
    found.sort_by(|a, b| a.scale.total_cmp(&b.scale));
    found
}

/// What a blend that did not build is refused with: the error, or, when
/// a smaller size builds, [`BlendError::TooLarge`] with the largest that
/// fits (see the module documentation).
pub(super) fn explain(
    body: &Body,
    uses: &[Vec<Use>],
    at: &[Vec<EdgeId>],
    requests: &[Request],
    plans: &[Plan],
    failure: Failure,
    tolerance: &Tolerance,
) -> BlendError {
    let Failure { edge, error } = failure;
    if !sized(&error) {
        return error;
    }
    let Some(index) = edge.and_then(|edge| plans.iter().position(|p| p.edge == edge)) else {
        return error;
    };
    let Some(own) = plans.get(index) else {
        return error;
    };
    // The side the error names, or the first.
    let named = match &error {
        BlendError::Refused {
            face: Some(face), ..
        } => own.faces.iter().position(|f| f == face),
        _ => None,
    }
    .unwrap_or(0);
    let tol = tolerance.linear;
    let group = group(requests, index);
    // The smallest size the booleans still tell apart, as a multiple.
    let smallest = group
        .iter()
        .filter_map(|i| plans.get(*i))
        .flat_map(|p| p.distances.into_iter().chain(p.round))
        .fold(f64::INFINITY, f64::min);
    // The smallest size tried: a thousandth of the size asked for, and
    // well clear of what the booleans tell apart.
    let floor = (64.0 * tol / smallest).max(1.0 / 1024.0);
    if !(floor.is_finite() && floor > 0.0 && floor < 1.0) {
        return error;
    }
    let mut search = Search {
        body,
        uses,
        at,
        requests,
        group: &group,
        tolerance,
        builds: 0,
        seen: BTreeMap::new(),
        spent: false,
    };
    // A quarter of the size at a time until something builds: what is
    // known to build and to fail, as multiples.
    let (mut fits, mut fails) = (QUARTER, 1.0_f64);
    let mut all_room = no_room(&error);
    let mut all_same = true;
    loop {
        if fits < floor {
            // Not even the smallest builds. Nothing fits where every
            // failure says the faces leave no room; any other failure is
            // reported as it is, saying that smaller sizes fail the same
            // way when they do.
            return if all_room {
                too_large(own, named, 0.0)
            } else if all_same {
                smaller_too(error)
            } else {
                error
            };
        }
        match search.attempt(fits) {
            Outcome::Built => break,
            Outcome::Failed(failure) => {
                all_room &= no_room(&failure);
                all_same &= same_way(&error, &failure);
            }
            Outcome::Spent => return error,
        }
        fails = fits;
        fits *= QUARTER;
    }
    let events: Vec<Event> = events(body, plans, &group, tol)
        .into_iter()
        .filter(|e| e.scale > fits + PAST * e.unit && e.scale < fails + PAST * e.unit)
        .collect();
    // The first event just past which nothing builds.
    let (mut lo, mut hi) = (0, events.len());
    while lo < hi {
        let middle = lo + (hi - lo) / 2;
        let Some(event) = events.get(middle) else {
            break;
        };
        let past = event.scale + PAST * event.unit;
        if past < fails && search.fits(past) {
            lo = middle + 1;
        } else {
            hi = middle;
        }
    }
    if search.spent {
        return error;
    }
    if let Some(event) = lo.checked_sub(1).and_then(|i| events.get(i)) {
        fits = fits.max(event.scale + PAST * event.unit);
    }
    if let Some(event) = events.get(lo) {
        // Just before it something builds: the event is where the blend
        // stops fitting, to within the probes (it builds a few tolerances
        // before it and fails a few past it). How near before depends on
        // how well the booleans tell the border from what it reaches.
        for before in BEFORE {
            let at = event.scale - before * event.unit;
            if at <= fits {
                break;
            }
            match search.attempt(at) {
                Outcome::Built => {
                    let plan = plans.get(event.plan).unwrap_or(own);
                    return too_large(plan, event.side, event.size);
                }
                Outcome::Failed(_) => fails = fails.min(at),
                Outcome::Spent => return error,
            }
        }
    }
    // No event explains it: bisect on the builds alone.
    let fails = bisect(fits, fails, 4.0 * tol / smallest, |at| search.attempt(at));
    let size = own.distances.get(named).copied().unwrap_or(0.0) * per(own, named);
    too_large(own, named, fails * size)
}

/// Bisects between `fits` (built) and `fails` (failed) with `attempt`
/// until the two are `resolution` apart, no number lies between them, or
/// [`MOST_BUILDS`] steps are made; gives the least size seen to fail.
fn bisect(
    mut fits: f64,
    mut fails: f64,
    resolution: f64,
    mut attempt: impl FnMut(f64) -> Outcome,
) -> f64 {
    for _ in 0..MOST_BUILDS {
        let middle = 0.5 * (fits + fails);
        if fails - fits <= resolution || middle <= fits || middle >= fails {
            break;
        }
        match attempt(middle) {
            Outcome::Built => fits = middle,
            Outcome::Failed(_) => fails = middle,
            Outcome::Spent => break,
        }
    }
    fails
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A resolution below what a float can halve (a document tolerance
    /// of 1e-300) ends the bisection when the middle rounds to an end,
    /// and a size it has seen does not keep it going: it once looped for
    /// ever there.
    #[test]
    fn a_bisection_finer_than_floats_ends() {
        let limit = 0.3;
        let mut calls = 0;
        let fails = bisect(0.25, 0.5, 1e-300, |at| {
            calls += 1;
            if at <= limit {
                Outcome::Built
            } else {
                Outcome::Failed(BlendError::Refused {
                    edge: None,
                    face: None,
                    reason: Reason::Reaches,
                })
            }
        });
        assert!(calls <= MOST_BUILDS, "{calls}");
        assert!(fails > limit && fails - limit < 1e-9, "{fails}");
        // An attempt that answers from a cache without building does not
        // stop it either: it is bounded by steps.
        let next = f64::from_bits(0.25_f64.to_bits() + 1);
        let fails = bisect(0.25, next, 0.0, |_| Outcome::Built);
        assert_eq!(fails, next);
        let mut steps = 0;
        let fails = bisect(0.0, 1.0, 0.0, |_| {
            steps += 1;
            Outcome::Built
        });
        assert!(steps <= MOST_BUILDS && fails == 1.0, "{steps} {fails}");
    }
}
