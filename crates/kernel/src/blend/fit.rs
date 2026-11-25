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
//! **Where to look.** The size of every edge of the same size changes
//! together (a feature has one size). First a quarter of the size at a
//! time until one builds. Then the sizes at which a border can first
//! reach something of its face (*events*: a vertex, the nearest point of
//! an edge, what another blend takes of the face from the other side, the
//! axis a circular edge turns about, the size at which a fillet's torus
//! reaches its own axis) are worked out from the faces, without deciding
//! which of them matter: a binary search over them with builds just past
//! each decides, and the event found is where the blend stops fitting
//! when a build just before it succeeds: to within those builds, a few
//! tolerances either side.
//!
//! **Said only where shown.** A largest size is said only where it is
//! shown. Where an event gives it, with a build just before it, and the
//! sizes evenly between it and the size asked all fail too ([`proven`]).
//! Where no event explains the failure, only where the result itself shows
//! that the faces leave no room ([`no_room`]: the blend reaches a face it
//! should not, takes a neighbour away, comes out in pieces) at the size
//! asked, at the size a bisection between a build and such a failure ends
//! on (or 4 or 64 tolerances past it, the build at 4 failing too: the size
//! said may then be where the boolean's refusals of a sliver begin, up to
//! 64 tolerances below the first size tried that shows no room), and at
//! the sizes evenly between those two. A failure of the boolean
//! (a refusal at the edge of tolerance, an invalid body,
//! [`BlendError::Invalid`]) is no such evidence: it may come at a size that
//! fits, with sizes on either side of it that build, so where builds that
//! fail so stop is not where the room ends. Nor is nothing said to fit
//! where nothing tried builds: no build shows where the room ends.
//! Otherwise the refusal at the size asked is reported as it is (the
//! boolean's in the blend's words), saying that smaller sizes fail the
//! same way where every one tried does. Every search is bounded
//! ([`MOST_BUILDS`]), and a spent budget proves nothing.
//!
//! **Up to the size asked.** No size at or above the one asked is built,
//! so what is shown is that the sizes tried from the one said up to the
//! one asked fail (seven evenly between, and the one asked), and one just
//! below it builds: so the refusal says ([`crate::Shown::Builds`]), not
//! that nothing larger fits. A size between two of those tried may build
//! where the faces leave a window of room there, and the refusal's words
//! then say more than is shown (two shallow pockets near the edge:
//! `crates/engine/tests/blend_too_large_shown_review2.rs`, failing). Where
//! the faces leave room again above the size asked (a shallow pocket near
//! the edge that a larger blend takes whole), larger sizes build
//! (`crates/engine/tests/blend_too_large_shown_review.rs`). An event is
//! taken at the size asked only to a tolerance, the rounding of its
//! measure (`Proof::slack`).

use super::{
    BlendError, Failure, Path, Plan, Reason, Request, Size, Use, construct, face_edges, half_tan,
    outward, plan, section,
};
use crate::KernelError;
use crate::chamfer::ChamferSize;
use craft3d_geom::Curve3;
use craft3d_math::Portable;
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
        BlendError::Boolean { .. } | BlendError::Invalid { .. } => true,
        BlendError::Kernel(KernelError::InvalidBody { .. } | KernelError::InvalidSize { .. })
        | BlendError::TooLarge { .. }
        | BlendError::Spindle { .. }
        // Tangent faces leave no width at any size.
        | BlendError::Pinched { .. }
        | BlendError::Fade { .. } => false,
        BlendError::Kernel(_) => true,
        BlendError::Refused { reason, .. } => matches!(
            reason,
            Reason::Reaches
                | Reason::LostNeighbour
                | Reason::InPieces
                | Reason::EndsBadly
                | Reason::NotTangent
                | Reason::NoNormal
                | Reason::JointNoNormal
                | Reason::MeetsWithinAngle
                | Reason::FreeTooLarge
                | Reason::FreeRunsOver
                | Reason::FreeCarry
                | Reason::FreeFolds
                | Reason::Through
        ),
    }
}

/// Whether a failure carries evidence from the result that the faces
/// leave no room for the blend: [`super::finish`] found it reaching a
/// face it should not, taking a neighbour away or in pieces, or the
/// construction in place found it running past an edge, off its face,
/// folding or through a face beside it. A boolean or a sweep that refuses
/// a tool says nothing about room, and may refuse at sizes that fit.
fn no_room(error: &BlendError) -> bool {
    matches!(
        error,
        BlendError::Refused {
            reason: Reason::Reaches
                | Reason::LostNeighbour
                | Reason::InPieces
                | Reason::FreeTooLarge
                | Reason::FreeRunsOver
                | Reason::FreeFolds
                | Reason::Through,
            ..
        }
    )
}

/// Whether two failures are the same failure: the same refusal, or both
/// the boolean's, or both the same kind of kernel error.
fn same_way(a: &BlendError, b: &BlendError) -> bool {
    match (a, b) {
        (BlendError::Refused { reason: x, .. }, BlendError::Refused { reason: y, .. }) => x == y,
        (BlendError::Spindle { .. }, BlendError::Spindle { .. }) => true,
        (
            BlendError::Boolean {
                edge: x, error: e, ..
            },
            BlendError::Boolean {
                edge: y, error: f, ..
            },
        ) => x == y && std::mem::discriminant(e.as_ref()) == std::mem::discriminant(f.as_ref()),
        (BlendError::Invalid { edge: x, .. }, BlendError::Invalid { edge: y, .. }) => x == y,
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
        BlendError::Boolean { edge, error, .. } | BlendError::Invalid { edge, error } => {
            BlendError::Boolean {
                edge,
                error,
                smaller_fails: true,
            }
        }
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
        Size::Law(law) => Size::Law(law.scaled(scale)),
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
    /// The requests built in place (`super::in_place`).
    routed: &'a [usize],
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
        let mut requests = Vec::with_capacity(self.requests.len());
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
            requests.push(request);
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
            None => construct(
                self.body,
                self.uses,
                self.at,
                &requests,
                plans,
                self.routed,
                self.tolerance,
            )
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
        shown: crate::Shown::Builds,
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
            (r - r0).phypot(h - h0)
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
    if plan.between_planes() && matches!(data.curve, Curve3::Line(_)) {
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
fn events(
    body: &Body,
    plans: &[Plan],
    requests: &[Request],
    group: &BTreeSet<usize>,
    tol: f64,
) -> Vec<Event> {
    let mut found = Vec::new();
    for &index in group {
        let Some(plan) = plans.get(index) else {
            continue;
        };
        // A chamfer's line across the edge that enters a face beside it at
        // the size asked for: the size at which it comes to lie along that
        // face (`section::touching`). For a distance and an angle it is
        // said as the distance, the one length given.
        if let (None, Some(section), Path::Line { start, along, .. }, Some(request)) = (
            plan.round,
            plan.section.as_ref(),
            plan.path,
            requests.get(index),
        ) && let Size::Chamfer(size) = request.size
            && let [Ok(a), Ok(b)] = plan.faces.map(|face| outward(body, face, start))
            && let Some((at, entered)) = section::touching(
                plan.edge,
                plan.faces,
                section.traces,
                [a, b],
                along,
                plan.convex,
                size,
                tol,
            )
        {
            let side = match size {
                ChamferSize::Distances { .. } => entered,
                ChamferSize::DistanceAngle { .. } => 0,
            };
            if let Some(&distance) = plan.distances.get(side)
                && distance.is_finite()
                && distance > 0.0
            {
                found.push(Event {
                    scale: at,
                    unit: tol / distance,
                    plan: index,
                    side,
                    size: at * distance,
                });
            }
        }
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
                    let towards = (a + b).dot(radial) / (half_tan(cosine) * (1.0 + cosine));
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

/// What a blend that did not build is refused with: the error, or
/// [`BlendError::TooLarge`] with the largest size that fits where that is
/// shown (see the module documentation).
#[allow(clippy::too_many_arguments)]
pub(super) fn explain(
    body: &Body,
    uses: &[Vec<Use>],
    at: &[Vec<EdgeId>],
    requests: &[Request],
    plans: &[Plan],
    routed: &[usize],
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
        routed,
        group: &group,
        tolerance,
        builds: 0,
        seen: BTreeMap::new(),
        spent: false,
    };
    // A quarter of the size at a time until something builds: what is
    // known to build and to fail, as multiples.
    let (mut fits, mut fails) = (QUARTER, 1.0_f64);
    let mut all_same = true;
    loop {
        if fits < floor {
            // Not even the smallest builds: nothing shows a size that
            // fits, nor that none does (a size between those tried may).
            // The failure is reported as it is, saying that smaller sizes
            // fail the same way when they do.
            return if all_same { smaller_too(error) } else { error };
        }
        match search.attempt(fits) {
            Outcome::Built => break,
            Outcome::Failed(failure) => all_same &= same_way(&error, &failure),
            Outcome::Spent => return error,
        }
        fails = fits;
        fits *= QUARTER;
    }
    let events: Vec<Event> = events(body, plans, requests, &group, tol)
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
        // before it and fails a few past it), when the sizes past it fail
        // too. How near before depends on how well the booleans tell the
        // border from what it reaches.
        for before in BEFORE {
            let at = event.scale - before * event.unit;
            if at <= fits {
                break;
            }
            // A size at or above the one asked that builds would show the
            // size asked is not too large: not tried.
            if at >= 1.0 {
                continue;
            }
            match search.attempt(at) {
                Outcome::Built => {
                    let plan = plans.get(event.plan).unwrap_or(own);
                    let found = too_large(plan, event.side, event.size);
                    let proof = Proof {
                        bound: event.scale,
                        slack: event.unit,
                        room: false,
                    };
                    return proven(&mut search, proof, found, error);
                }
                Outcome::Failed(_) => fails = fails.min(at),
                Outcome::Spent => return error,
            }
        }
    }
    // No event explains it: bisect on the builds, only where the result
    // at the size asked shows no room, and say what it finds only where the
    // result shows no room at it or 4 or 64 tolerances past it (the build
    // at 4 failing too, the sizes between it and 64 not built: where a face
    // is about to be taken whole, the boolean may refuse the sliver left of
    // it) and at the sizes between it
    // and the size asked. A size found where builds fail at the boolean is
    // wherever its faults happen to lie ([`proven`]).
    if !no_room(&error) {
        return error;
    }
    let resolution = 4.0 * tol / smallest;
    let Some(found) = bisect(fits, fails, resolution, |at| search.attempt(at)) else {
        return error;
    };
    if !shows_no_room(&mut search, found, tol / smallest) {
        return error;
    }
    // The side the error names, or the first.
    let named = match &error {
        BlendError::Refused {
            face: Some(face), ..
        } => own.faces.iter().position(|f| f == face),
        _ => None,
    }
    .unwrap_or(0);
    let size = own.distances.get(named).copied().unwrap_or(0.0) * per(own, named);
    let proof = Proof {
        bound: found,
        slack: 0.0,
        room: true,
    };
    proven(
        &mut search,
        proof,
        too_large(own, named, found * size),
        error,
    )
}

/// How many tolerances past a size a bisection ended on the builds that
/// look for evidence of no room there are made, nearest first.
const ABOVE: [f64; 2] = [4.0, 64.0];

/// Whether the result shows no room ([`no_room`]) at `found` (a multiple
/// of the size asked, where a bisection ended), or 4 or 64 tolerances
/// past it (`unit`, a tolerance as a multiple) with the build at 4 failing
/// too (the sizes between are not built); the size asked, which failed so,
/// where it lies no further past `found`.
fn shows_no_room(search: &mut Search<'_>, found: f64, unit: f64) -> bool {
    for past in std::iter::once(0.0).chain(ABOVE) {
        let at = found + past * unit;
        if at >= 1.0 {
            return true;
        }
        match search.attempt(at) {
            Outcome::Failed(failure) if no_room(&failure) => return true,
            Outcome::Failed(_) => {}
            Outcome::Built | Outcome::Spent => return false,
        }
    }
    false
}

/// Into how many parts the sizes between a largest size found and the
/// size asked are divided to prove it ([`proven`]): a build at each of
/// the seven sizes between the parts.
const PROOFS: u32 = 8;

/// What [`proven`] holds a largest size found to.
struct Proof {
    /// The size found, as a multiple of the size asked.
    bound: f64,
    /// How far past the size asked it may lie, as a multiple of it: an
    /// event one tolerance from the size asked (which failed) is at it, to
    /// the rounding of the measure (a chamfer exactly as large as its wall
    /// is high).
    slack: f64,
    /// Whether every failure between has to show no room ([`no_room`]),
    /// or any failure will do.
    room: bool,
}

/// `found`, the refusal as too large with `proof.bound` (a multiple of the
/// size asked) as the largest size that fits, when the sizes evenly
/// between it and the size asked ([`PROOFS`]) all fail as well (each with
/// evidence of no room, where `proof.room`); otherwise `error`, the
/// refusal at the size asked.
///
/// The failures the search meets may be the boolean's at sizes that fit,
/// not a limit the faces set: in random design 124545 (before the
/// boolean's repair of 2026-11-24) a fillet of a rim built at radii of
/// 0.9, 1.05 to 1.199 and 1.3 and failed at 0.7, 0.8, 1, 1.1, 1.2 and 1.5,
/// though the faces leave room up to 2.3965, and the search on builds
/// alone said "below 0.6528" at 1; a hole's rim in a cone 6 high, at 0.999
/// of the 1.999936 that fits, is said "below 1.99793" by `main` of
/// 2026-11-24, where 1.99894 builds (`docs/design/blends.md` › *Too large
/// only where it is shown*). So a largest size is said only where the
/// faces give it, an event (the border reaches something of its face)
/// with a build just before it, or the result shows no room ([`explain`]),
/// and only where no size between it and the size asked builds, which
/// would show it is not the largest that fits. A spent budget proves
/// nothing.
fn proven(
    search: &mut Search<'_>,
    proof: Proof,
    found: BlendError,
    error: BlendError,
) -> BlendError {
    let Proof { bound, slack, room } = proof;
    // At the size asked, to a tolerance, the size asked, which failed, is
    // all there is between.
    if !(bound.is_finite() && bound <= 1.0 + slack) {
        return error;
    }
    for step in 1..PROOFS {
        let at = bound + (1.0 - bound) * f64::from(step) / f64::from(PROOFS);
        if at <= bound || at >= 1.0 {
            continue;
        }
        match search.attempt(at) {
            Outcome::Failed(failure) if !room || no_room(&failure) => {}
            Outcome::Failed(_) | Outcome::Built | Outcome::Spent => return error,
        }
    }
    found
}

/// Bisects between `fits` (built) and `fails` (failed) with `attempt`
/// until the two are `resolution` apart, no number lies between them, or
/// [`MOST_BUILDS`] steps are made; gives the least size seen to fail,
/// or `None` when the search ran out of builds on the way.
fn bisect(
    mut fits: f64,
    mut fails: f64,
    resolution: f64,
    mut attempt: impl FnMut(f64) -> Outcome,
) -> Option<f64> {
    for _ in 0..MOST_BUILDS {
        let middle = 0.5 * (fits + fails);
        if fails - fits <= resolution || middle <= fits || middle >= fails {
            break;
        }
        match attempt(middle) {
            Outcome::Built => fits = middle,
            Outcome::Failed(_) => fails = middle,
            Outcome::Spent => return None,
        }
    }
    Some(fails)
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
        let fails = fails.unwrap();
        assert!(calls <= MOST_BUILDS, "{calls}");
        assert!(fails > limit && fails - limit < 1e-9, "{fails}");
        // An attempt that answers from a cache without building does not
        // stop it either: it is bounded by steps.
        let next = f64::from_bits(0.25_f64.to_bits() + 1);
        let fails = bisect(0.25, next, 0.0, |_| Outcome::Built);
        assert_eq!(fails, Some(next));
        let mut steps = 0;
        let fails = bisect(0.0, 1.0, 0.0, |_| {
            steps += 1;
            Outcome::Built
        });
        assert!(
            steps <= MOST_BUILDS && fails == Some(1.0),
            "{steps} {fails:?}"
        );
        // Out of builds on the way: no size is claimed.
        assert_eq!(bisect(0.0, 1.0, 0.0, |_| Outcome::Spent), None);
    }
}
