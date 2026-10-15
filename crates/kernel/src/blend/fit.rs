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

/// Whether a failure at the smallest size there is says that the faces
/// leave no room at all (rather than that the blend is of a kind that is
/// not built there).
fn no_room(error: &BlendError) -> bool {
    match error {
        BlendError::Refused { reason, .. } => matches!(
            reason,
            Reason::Reaches | Reason::LostNeighbour | Reason::InPieces
        ),
        other => sized(other),
    }
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

/// The requests that change size with request `index`: those of the same
/// size that share a vertex with it, through any number of others (a
/// chain, the edges round a corner, which have to stay one size).
fn group(body: &Body, requests: &[Request], index: usize) -> BTreeSet<usize> {
    let Some(own) = requests.get(index) else {
        return BTreeSet::new();
    };
    let ends = |request: &Request| -> Vec<VertexId> {
        body.edge(request.edge)
            .map(|e| vec![e.start, e.end])
            .unwrap_or_default()
    };
    let mut members = BTreeSet::from([index]);
    let mut touched: BTreeSet<VertexId> = ends(own).into_iter().collect();
    for _ in 0..requests.len() {
        let mut grew = false;
        for (i, request) in requests.iter().enumerate() {
            if members.contains(&i) || request.size != own.size {
                continue;
            }
            let at = ends(request);
            if at.iter().any(|v| touched.contains(v)) {
                members.insert(i);
                touched.extend(at);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    members
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
    seen: BTreeMap<u64, Option<BlendError>>,
}

impl Search<'_> {
    /// Builds the blend with the group's sizes times `scale`: `None` when
    /// it builds, the error otherwise. A search past its bound of builds
    /// fails every further size.
    fn attempt(&mut self, scale: f64) -> Option<BlendError> {
        if let Some(known) = self.seen.get(&scale.to_bits()) {
            return known.clone();
        }
        if self.builds >= MOST_BUILDS {
            return Some(BlendError::Refused {
                edge: None,
                face: None,
                reason: Reason::InPieces,
            });
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
        let outcome = match failed {
            Some(error) => Some(error),
            None => build(self.body, self.uses, self.at, plans, self.tolerance)
                .err()
                .map(|f| f.error),
        };
        self.seen.insert(scale.to_bits(), outcome.clone());
        outcome
    }

    fn fits(&mut self, scale: f64) -> bool {
        self.attempt(scale).is_none()
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
                        push(reach * distance / (distance + taken));
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
    let group = group(body, requests, index);
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
    };
    // A refusal of the kind of blend (one that does not end cleanly) that
    // a quarter of the size does not mend is not about the size.
    if !no_room(&error)
        && search
            .attempt(QUARTER)
            .is_some_and(|smaller| !no_room(&smaller))
    {
        return error;
    }
    let events: Vec<Event> = events(body, plans, &group, tol)
        .into_iter()
        .filter(|e| e.scale > floor + PAST * e.unit && e.scale < 1.0 + PAST * e.unit)
        .collect();
    // The first event just past which nothing builds.
    let (mut lo, mut hi) = (0, events.len());
    while lo < hi {
        let middle = lo + (hi - lo) / 2;
        let Some(event) = events.get(middle) else {
            break;
        };
        let past = event.scale + PAST * event.unit;
        if past < 1.0 && search.fits(past) {
            lo = middle + 1;
        } else {
            hi = middle;
        }
    }
    // What is known to build and to fail, as multiples; nothing is known
    // to build yet when no event before it did.
    let mut known = (None, 1.0_f64);
    if let Some(event) = lo.checked_sub(1).and_then(|i| events.get(i)) {
        known.0 = Some(event.scale + PAST * event.unit);
    }
    if let Some(event) = events.get(lo) {
        // Just before it something builds: the event is where the blend
        // stops fitting. How near before depends on how well the
        // booleans tell the border from what it reaches.
        for before in BEFORE {
            let at = event.scale - before * event.unit;
            if at <= known.0.unwrap_or(floor) {
                break;
            }
            if search.fits(at) {
                let plan = plans.get(event.plan).unwrap_or(own);
                return too_large(plan, event.side, event.size);
            }
            known.1 = known.1.min(at);
        }
    }
    let fits = match known.0 {
        Some(fits) => fits,
        None => {
            // A quarter at a time until something builds.
            let mut at = QUARTER * known.1;
            loop {
                if at < floor {
                    // Not even the smallest builds: the faces leave no
                    // room, or the blend is not built there at any size.
                    let least = search.attempt(known.1);
                    return if no_room(&error) && least.as_ref().is_none_or(no_room) {
                        too_large(own, named, 0.0)
                    } else {
                        error
                    };
                }
                if search.fits(at) {
                    break at;
                }
                known.1 = at;
                at *= QUARTER;
            }
        }
    };
    let known = (fits, known.1);
    // No event explains it: bisect on the builds alone.
    let resolution = 4.0 * tol / smallest;
    let (mut fits, mut fails) = known;
    while fails - fits > resolution && search.builds < MOST_BUILDS {
        let middle = 0.5 * (fits + fails);
        if search.fits(middle) {
            fits = middle;
        } else {
            fails = middle;
        }
    }
    let size = own.distances.get(named).copied().unwrap_or(0.0) * per(own, named);
    too_large(own, named, fails * size)
}
