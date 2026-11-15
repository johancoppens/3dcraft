//! Whether each loop of a face is a simple closed curve on its surface
//! (`docs/design/topology.md` › *Loops that are simple*).
//!
//! **What must hold.** A face is the region its loops bound, each loop
//! counted once: every loop goes round what it encloses once, and no two
//! stretches of the face's boundary lie on each other. The region rule
//! (`crate::region`) holds the loops to running the right way round and to
//! not crossing; it cannot see a loop that goes round its circle twice
//! (its area is twice the disc's and positive, its steps meet without
//! crossing) nor one that runs back over itself, and a face so bounded is
//! measured as twice the region, or as a region with a seam of no width
//! in it. Held here, in this order:
//!
//! 1. *An edge that goes round its curve more than once* (a circle or an
//!    ellipse whose range is longer than a turn by more than ten
//!    tolerances along it).
//! 2. *A loop that goes round a surface that closes more than once* (read
//!    from the loop followed in the surface's parameters, each point moved
//!    by whole periods to the one before: a loop that ends `k` periods
//!    from where it starts goes round `k` times). On a cylinder, a cone or
//!    a sphere, where only `u` closes, a simple loop goes round 0 or 1
//!    times; on a torus a simple loop goes round `(p, q)` times with `p`
//!    and `q` having no common factor (or not at all). A band of a
//!    cylinder, whose two loops each go round once, is a region.
//! 3. *A loop that winds more than once about a point it encloses* (each
//!    loop that does not go round a period, followed as the region rule
//!    follows it, cut along [`SCANLINES`] lines of constant `v` across its
//!    box: between two crossings the loop winds about every point the same
//!    number of times, the sum of the crossings' senses to its right; a
//!    point clear of the loop by its margin, twice its polyline's error and
//!    ten tolerances in parameters, about which it winds neither 0 times nor
//!    once its own way round is a fault).
//! 4. *Stretches of the boundary that lie on each other*: two coedges of
//!    different edges of the face's loops one curve along a stretch longer
//!    than ten of the body's working tolerances (the face's boundary passes
//!    there twice). One edge used twice by one loop is not judged: it is a
//!    cut of the face along an edge the face holds on both sides, a seam
//!    where the surface closes, a bridge joining two of its boundary curves
//!    (an annulus written as one loop: sheet metal's collars), or a spur,
//!    each leaving the region and its measure as they are without it, and
//!    a bridge and a spur both lie at one place of a plane, so the two uses
//!    cannot tell a right body from a wrong one. Two stretches are one curve where they lie within the
//!    tighter of their two edges' own tolerances (the document's for an edge
//!    without one) of each other, by a gap that is the same all along them
//!    to the rounding of their coordinates: copies of one curve, or one
//!    offset by less than that tolerance. A face between one and ten
//!    tolerances wide is thin, not degenerate, and passes; so does a face's
//!    sharp tip, whose two edges keep within the tolerance of each other
//!    near it but by a gap that grows from nothing at its vertex, whatever
//!    its angle and whatever the document's angular tolerance (which the
//!    rule does not read). Exact for lines, circles and ellipses: two lines
//!    whose ends lie within the tolerance of each other's line, or two
//!    conics nine points of one of which lie within it of the other (five
//!    points fix a conic; their centres within twice it), are one curve,
//!    and their parameter ranges, read on the first and taken modulo a
//!    turn, overlap by an interval as long as that is. A spline against any
//!    curve is held at [`PLACES`] places along each: three places or more
//!    in a row within the tolerance of the other (the nearest point found
//!    from the nearest of its samples by Newton steps), the first and the
//!    last more than ten tolerances apart, by gaps one to rounding, are a
//!    stretch on it; a run whose gaps differ is scanned on past (on a
//!    sheet too, where a run of the two sides of a cut is passed only once
//!    it is found level). The places are looked for only where one lies in
//!    the box of the other's control points and within an eighth of a step
//!    of its samples' polyline. The boxes of a face's edges are paired pair
//!    by pair for up to `SMALL_FACE` edges, else by a grid over the two axes
//!    they spread furthest along, so that a face costs about its edges.
//!    Two stretches of different loops are a fault wherever they lie. Two of
//!    one loop are one only where they lie at one place in the loop's
//!    parameters, as followed (on every surface but a plane, whose chart is
//!    the plane), told apart along each parameter by ten working
//!    tolerances on the surface at that parameter's speed: a seam's two
//!    uses, or two edges either side of a slit, lie a period (or the width
//!    of a spline surface's domain) apart and are fine, whether or not the
//!    surface is known to close; a face whose loop cannot be followed, or a
//!    place where a parameter does not move the surface, is not held to it.
//!    On a sheet, two stretches that
//!    run opposite ways are the two sides of a cut (a slit into the face, or
//!    a slot: what origami builds, `docs/design/origami.md` › *Slits and
//!    slots*) and are fine; two that run the same way are a fault there too.
//!    A solid's boundary has no cut: on a face of a solid either is a fault.
//!
//! **What is not held.** What the region rule passes without a word is not
//! followed here either: a face on a surface it cannot chart without doubt
//! (a loop through a sphere's pole or a cone's apex, every spline surface
//! that `geom::closure` finds closed or collapsed), a loop whose area its
//! polyline cannot tell from nothing, a face past its caps of points and
//! work, a face with a spline edge whose half steps are not shown to keep
//! to what the walk can follow within the steps and the work a face is
//! allowed (`crate::region`): such a face is held to rule 1 and to rule 4
//! between different loops only (so a loop gone round twice on a face of a
//! revolved spline is caught only where a neighbouring face can be
//! charted). A spline surface that closes within the tolerance but not to
//! `closure`'s rounding (or on knots that are not clamped) is charted as
//! one that does not close: its seam passes, but a loop across the seam is
//! not followed right. A spline edge is followed in steps whose halves are
//! shown to move less than a quarter turn about the axis of a surface that
//! closes about one, and to run one way in the chart of a plane, a
//! cylinder, a cone or a sphere (`crate::region`), so a helix of many turns
//! as one edge and a spiral about a point of the chart are read as they go
//! round (each step is read at its middle only: two halves that each run
//! one way can still go round a point together, unseen). A loop with a
//! spline edge on a torus is not held to 2 or 3, nor to 4 between two of
//! its own stretches: how far such an edge moves about the axis is shown,
//! but nothing shows how far it moves about the tube from one point to the
//! next, and how often a loop goes round the axis says nothing without the
//! other (three times round the axis is a simple loop with 13 turns about
//! the tube and not with 3). Elsewhere (a conic near a pole) a loop whose
//! half step moves more than a quarter period is not held to 2 or 3 (a
//! conic's half step, a sixteenth of a
//! half turn of its own, cannot move the three quarters of a period that
//! would wrap unseen unless the axis passes within the arc's own sagitta
//! of it). A loop narrower than twice its margin has no point clear
//! of it for 3. A spline overlap shorter than about a quarter of both
//! edges, or one the samples or the Newton steps do not show (a spline
//! that bends between its samples by more than an eighth of a step), and
//! two fits of one curve whose gap varies along them by more than
//! rounding, are not seen. Rule 3 stops after [`MOST_WINDING_WORK`] and
//! rule 4's spline test after [`MOST_EVALUATIONS`] (a hostile face): what
//! they have not reached by then is not held.

use crate::region::{Chart, Followed, Ring, distance, follow, signed_area};
use crate::{Body, EdgeId, Face, LoopId};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use std::f64::consts::{PI, TAU};

/// Lines of constant `v` a loop is cut along to read how often it winds.
pub const SCANLINES: usize = 8;
/// Places along an edge at which a spline overlap is looked for.
pub const PLACES: usize = 9;
/// Most samples a spline edge is followed at for the start of a search
/// for its nearest point.
const MOST_SAMPLES: usize = 64;
/// Newton steps of that search.
const NEWTON_STEPS: usize = 8;
/// Most pairs of coedges a face's boundary is held at; past it the face
/// is not held to rule 4.
const MOST_PAIRS: usize = 1 << 22;
/// Most points looked at (crossings found and distances measured) by rule
/// 3 on one loop; past it the loop is not held to it.
pub const MOST_WINDING_WORK: usize = 1 << 24;
/// Most evaluations of curves rule 4's spline test makes on one face; past
/// it the face is not held to rule 4.
pub const MOST_EVALUATIONS: usize = 1 << 20;
/// Units in the last place of a coordinate by which the gaps of one curve
/// against a copy of it may differ (its evaluation and the nearest point's
/// search, each a few units).
const ROUNDING: f64 = 64.0 * f64::EPSILON;

/// The first loop of `face` that is not a simple closed curve, with why;
/// `None` when every loop is one or this test cannot tell. `followed` is
/// what the region rule followed of the face (`crate::region`), so the face
/// is followed once for both.
pub(crate) fn loop_fault(
    body: &Body,
    face: &Face,
    tol: &Tolerance,
    followed: Option<&Followed>,
    verified: &dyn Fn(usize) -> bool,
) -> Option<(LoopId, String)> {
    // Ten working tolerances: the least length of a stretch and the reach
    // of a margin, as in the region rule. Which curves are one is the
    // edges' own tolerances' (rule 4).
    let working = body.working_tolerance(tol.linear);
    let near = 10.0 * working;
    let uses = uses_of(body, face, tol.linear);
    let loop_at = |ring: usize| face.loops.get(ring).copied();
    // 1. Edges that go round their curve more than once.
    for u in &uses {
        if let Some(turns) = turns_of(u.curve, u.range, near) {
            return Some((
                loop_at(u.ring)?,
                format!(
                    "edge {} goes round its {} {turns:.4} times: an edge goes round its curve once at most",
                    u.edge.0,
                    u.curve.kind()
                ),
            ));
        }
    }
    // A face on a torus is not followed by the region rule: followed here.
    let torus_walk;
    let followed = match followed {
        Some(f) => Some(f),
        None if matches!(face.surface, Surface::Torus(_)) => {
            torus_walk = walk(body, face, near);
            torus_walk.as_ref()
        }
        None => None,
    };
    if let Some(f) = followed {
        // 2. Loops that go round a closed surface more than once.
        for (k, ring) in f.rings.iter().enumerate() {
            if ring.aliased {
                continue;
            }
            if let Some(why) = goes_round(ring, f.periods, &face.surface) {
                return Some((loop_at(k)?, why));
            }
        }
        // 3. Loops that wind more than once about a point inside them.
        for (k, ring) in f.rings.iter().enumerate() {
            if ring.aliased || ring.winds.iter().any(|w| *w != 0.0) {
                continue;
            }
            let margin = 2.0 * ring.error + f.reach;
            if let Some((at, wound)) = wound_wrongly(ring, margin)
                && verified(k)
            {
                return Some((
                    loop_at(k)?,
                    format!(
                        "it winds {wound} times about the point ({:.6}, {:.6}) of the {}'s parameters: a loop goes round what it encloses once (this one goes round it more often, or overlaps itself)",
                        at[0],
                        at[1],
                        face.surface.kind()
                    ),
                ));
            }
        }
    }
    // 4. Stretches of the boundary on each other.
    overlap(
        face,
        &uses,
        Reach {
            near,
            grow: 2.0 * working,
        },
        followed,
        body.kind == crate::BodyKind::Sheet,
        MOST_EVALUATIONS,
    )
}

/// How far rule 4 reaches: ten working tolerances (the least length of a
/// stretch), and how far it grows the boxes of edges (twice the working
/// tolerance: every vertex lies within it of its edge's end there).
#[derive(Clone, Copy)]
struct Reach {
    near: f64,
    grow: f64,
}

/// One coedge of a face's loops.
struct Use<'a> {
    /// The loop's place among the face's loops, and the coedge's in it.
    ring: usize,
    at: usize,
    edge: EdgeId,
    reversed: bool,
    curve: &'a Curve3,
    range: (f64, f64),
    /// The edge's start and end vertices' points (each within its
    /// tolerance of the curve's ends there).
    ends: (Vec3, Vec3),
    /// The edge's own tolerance (the document's where it has none).
    own: f64,
}

fn uses_of<'a>(body: &'a Body, face: &Face, linear: f64) -> Vec<Use<'a>> {
    let mut uses = Vec::new();
    for (ring, id) in face.loops.iter().enumerate() {
        let Some(lp) = body.loop_(*id) else {
            continue;
        };
        for (at, coedge) in lp.coedges.iter().enumerate() {
            let Some(edge) = body.edge(coedge.edge) else {
                continue;
            };
            let point = |v: crate::VertexId, t: f64| {
                body.vertex(v)
                    .map_or_else(|| edge.curve.eval(t), |v| v.point)
            };
            let ends = (
                point(edge.start, edge.range.0),
                point(edge.end, edge.range.1),
            );
            uses.push(Use {
                ring,
                at,
                edge: coedge.edge,
                reversed: coedge.reversed,
                curve: &edge.curve,
                range: edge.range,
                ends,
                own: body.edge_tolerance_at(coedge.edge, linear),
            });
        }
    }
    uses
}

/// How many times an edge goes round its closed curve, when that is more
/// than once by more than `near` along it.
fn turns_of(curve: &Curve3, (t0, t1): (f64, f64), near: f64) -> Option<f64> {
    let size = match curve {
        Curve3::Circle(c) => c.radius(),
        Curve3::Ellipse(e) => {
            let (a, b) = e.semi_axes();
            a.min(b)
        }
        Curve3::Line(_) | Curve3::Nurbs(_) => return None,
    };
    let sweep = t1 - t0;
    (sweep.is_finite() && size > 0.0 && (sweep - TAU) * size > near).then(|| sweep / TAU)
}

/// The loops of a face on a torus followed in its parameters, bands kept.
fn walk(body: &Body, face: &Face, near: f64) -> Option<Followed> {
    let surface = &face.surface;
    let periods = [surface.period_u(), surface.period_v()];
    let mut chart = Chart::new(surface, periods, false, near);
    let mut total = 0_usize;
    let mut rings = Vec::with_capacity(face.loops.len());
    for id in &face.loops {
        let lp = body.loop_(*id)?;
        rings.push(follow(body, &lp.coedges, &mut chart, &mut total, true)?);
    }
    let reach = near / chart.scale.max(f64::MIN_POSITIVE);
    Some(Followed {
        rings,
        periods,
        reach,
    })
}

/// Why `ring` goes round its surface more often than a simple loop can.
fn goes_round(ring: &Ring, periods: [Option<f64>; 2], surface: &Surface) -> Option<String> {
    let turns = |d: usize| -> i64 {
        match (periods.get(d).copied().flatten(), ring.winds.get(d)) {
            (Some(p), Some(w)) if p > 0.0 && w.is_finite() => {
                let k = (w / p).round();
                if k.abs() < 1e6 { k as i64 } else { i64::MAX }
            }
            _ => 0,
        }
    };
    let (ku, kv) = (turns(0), turns(1));
    let simple = match (periods[0].is_some(), periods[1].is_some()) {
        (true, true) => (ku == 0 && kv == 0) || common_factor(ku, kv) == 1,
        _ => ku.abs() <= 1 && kv.abs() <= 1,
    };
    if simple {
        return None;
    }
    Some(match surface {
        Surface::Torus(_) => format!(
            "it goes round the torus {} times about its axis and {} times about its tube: a loop on a torus goes round it p and q times with p and q of no common factor",
            ku.abs(),
            kv.abs()
        ),
        _ => format!(
            "it goes round the {} {} times: a loop on a surface that closes goes round it once at most (this one runs over itself)",
            surface.kind(),
            ku.abs().max(kv.abs())
        ),
    })
}

fn common_factor(a: i64, b: i64) -> i64 {
    let (mut a, mut b) = (a.unsigned_abs(), b.unsigned_abs());
    // Bounded: each step at least halves the larger within two steps.
    for _ in 0..200 {
        if b == 0 {
            break;
        }
        (a, b) = (b, a % b);
    }
    i64::try_from(a).unwrap_or(i64::MAX)
}

/// A point clear of `ring` by more than `margin` about which it winds
/// neither not at all nor once its own way round, with how often it winds
/// there; `None` also past [`MOST_WINDING_WORK`].
fn wound_wrongly(ring: &Ring, margin: f64) -> Option<([f64; 2], i64)> {
    wound_within(ring, margin, MOST_WINDING_WORK)
}

/// [`wound_wrongly`] within `most` points looked at.
pub(crate) fn wound_within(ring: &Ring, margin: f64, most: usize) -> Option<([f64; 2], i64)> {
    let points = &ring.points;
    let n = points.len();
    let area = signed_area(points);
    if n < 3 || !area.is_finite() || area == 0.0 {
        return None;
    }
    let sense: i64 = if area > 0.0 { 1 } else { -1 };
    let (low, high) = (ring.low[1], ring.high[1]);
    let wide = high - low > 2.0 * margin;
    if !wide {
        return None;
    }
    let mut work = 0_usize;
    let mut crossings: Vec<(f64, i64)> = Vec::new();
    for line in 0..SCANLINES {
        let y = low + (high - low) * (line as f64 + 0.5) / SCANLINES as f64;
        crossings.clear();
        work = work.saturating_add(n);
        if work > most {
            return None;
        }
        for i in 0..n {
            let (Some(a), Some(b)) = (points.get(i), points.get((i + 1) % n)) else {
                continue;
            };
            let up = a[1] <= y && b[1] > y;
            let down = b[1] <= y && a[1] > y;
            if up || down {
                let x = a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x.is_finite() {
                    crossings.push((x, if up { 1 } else { -1 }));
                }
            }
        }
        crossings.sort_by(|p, q| p.0.total_cmp(&q.0).then(p.1.cmp(&q.1)));
        // A point between crossings `i` and `i + 1` is wound by the senses
        // of the crossings to its right.
        let mut right: i64 = crossings.iter().map(|c| c.1).sum();
        for pair in crossings.windows(2) {
            let [(x0, s0), (x1, _)] = pair else {
                continue;
            };
            right -= s0;
            let room = x1 - x0 > 2.0 * margin;
            if right == 0 || right == sense || !room {
                continue;
            }
            work = work.saturating_add(n);
            if work > most {
                return None;
            }
            let at = [0.5 * (x0 + x1), y];
            if distance(at, points) > margin {
                return Some((at, right));
            }
        }
    }
    None
}

/// The first two stretches of the face's boundary that lie on each other;
/// `None` also past [`MOST_PAIRS`] pairs or `most` evaluations of curves.
fn overlap(
    face: &Face,
    uses: &[Use<'_>],
    reach: Reach,
    followed: Option<&Followed>,
    sheet: bool,
    most: usize,
) -> Option<(LoopId, String)> {
    if uses.len() < 2 {
        return None;
    }
    // Boxes grown so that every vertex is inside its edge's; the pairs of
    // them that meet.
    let boxes: Vec<Option<(Vec3, Vec3)>> = uses
        .iter()
        .map(|u| curve_box(u).map(|(l, h)| (l - grow(reach.grow), h + grow(reach.grow))))
        .collect();
    let surface = &face.surface;
    let plane = matches!(surface, Surface::Plane(_));
    // Filled as spline pairs ask for them.
    let mut samples: Vec<Option<Samples>> = Vec::new();
    let mut evaluations = 0_usize;
    for (i, j) in meeting(&boxes)? {
        let (Some(a), Some(b)) = (uses.get(i), uses.get(j)) else {
            continue;
        };
        let found = on_each_other(a, b, reach, &mut samples, (i, j), &mut evaluations);
        if evaluations > most {
            return None;
        }
        let Some(found) = found else {
            continue;
        };
        // On a sheet, stretches that run opposite ways are the two sides
        // of a cut (a slit or a slot), which a sheet may have.
        if sheet && !found.same_way {
            continue;
        }
        // Two stretches of one loop: only where both are at one place in
        // the loop's parameters as followed (not a period, or a spline
        // surface's domain, apart, as a seam's two uses are).
        let same_place = a.ring != b.ring
            || plane
            || followed.is_some_and(|f| at_one_place(f, surface, a, b, &found, reach.near));
        if !same_place {
            continue;
        }
        let ring = a.ring.min(b.ring);
        let loop_ = face.loops.get(ring).copied()?;
        let p = found.at;
        let why = if a.ring != b.ring {
            let other = a.ring.max(b.ring);
            format!(
                "it runs over loop {} of the face along edges {} and {} (a stretch {:.3e} long about ({:.6e}, {:.6e}, {:.6e})): the face has no width there",
                face.loops.get(other).map_or(u32::MAX, |l| l.0),
                a.edge.0.min(b.edge.0),
                a.edge.0.max(b.edge.0),
                found.length,
                p.x,
                p.y,
                p.z
            )
        } else {
            format!(
                "it runs over itself along edges {} and {} (a stretch {:.3e} long about ({:.6e}, {:.6e}, {:.6e})): a loop must not run over itself (the face would have no width there, or be counted twice)",
                a.edge.0.min(b.edge.0),
                a.edge.0.max(b.edge.0),
                found.length,
                p.x,
                p.y,
                p.z
            )
        };
        return Some((loop_, why));
    }
    None
}

/// Most cells a box may reach in the grid of [`meeting`]; a larger one is
/// held to every other box.
const MOST_CELLS: usize = 64;
/// Most edges of a face whose boxes [`meeting`] holds pair by pair.
const SMALL_FACE: usize = 32;

/// Every pair of the boxes that meet, once each and in a fixed order: by a
/// grid over the two axes the boxes spread furthest along, about as many
/// cells as boxes, a pair taken in the first cell both reach (a box that
/// reaches more than [`MOST_CELLS`] is held to every other). `None` past
/// [`MOST_PAIRS`] pairs looked at.
fn meeting(boxes: &[Option<(Vec3, Vec3)>]) -> Option<Vec<(usize, usize)>> {
    let live: Vec<(usize, Vec3, Vec3)> = boxes
        .iter()
        .enumerate()
        .filter_map(|(i, b)| b.map(|(l, h)| (i, l, h)))
        .collect();
    let mut pairs = Vec::new();
    if live.len() < 2 {
        return Some(pairs);
    }
    let meet = |(l, h): (Vec3, Vec3), (m, n): (Vec3, Vec3)| {
        l.x <= n.x && m.x <= h.x && l.y <= n.y && m.y <= h.y && l.z <= n.z && m.z <= h.z
    };
    // A face of few edges: every pair.
    if live.len() <= SMALL_FACE {
        for (x, &(i, l, h)) in live.iter().enumerate() {
            for &(j, m, n) in live.iter().skip(x + 1) {
                if meet((l, h), (m, n)) {
                    pairs.push((i, j));
                }
            }
        }
        return Some(pairs);
    }
    let far = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let (low, high) = live.iter().fold((far, far * -1.0), |(l, h), (_, bl, bh)| {
        (l.min(*bl), h.max(*bh))
    });
    let spread = high - low;
    let mut order = [0_usize, 1, 2];
    order.sort_by(|p, q| axis(spread, *q).total_cmp(&axis(spread, *p)));
    let [a, b, _] = order;
    let side = ((live.len() as f64).sqrt().ceil() as usize).clamp(1, 1024);
    let cell = |v: Vec3, k: usize| -> usize {
        let f = (axis(v, k) - axis(low, k)) / axis(spread, k) * side as f64;
        if f.is_finite() && f > 0.0 {
            (f as usize).min(side - 1)
        } else {
            0
        }
    };
    // Each box's cells, and the boxes in each cell (in their order), in
    // two flat arrays: where each cell's boxes start, and the boxes.
    let mut reach: Vec<[usize; 4]> = Vec::with_capacity(live.len());
    let mut big = Vec::new();
    let mut starts = vec![0_usize; side * side + 1];
    for (k, (_, l, h)) in live.iter().enumerate() {
        let r = [cell(*l, a), cell(*h, a), cell(*l, b), cell(*h, b)];
        reach.push(r);
        let count = (r[1].saturating_sub(r[0]) + 1).saturating_mul(r[3].saturating_sub(r[2]) + 1);
        if count > MOST_CELLS {
            big.push(k);
            continue;
        }
        for ca in r[0]..=r[1] {
            for cb in r[2]..=r[3] {
                if let Some(n) = starts.get_mut(ca * side + cb + 1) {
                    *n += 1;
                }
            }
        }
    }
    for c in 1..starts.len() {
        let before = starts.get(c - 1).copied().unwrap_or(0);
        if let Some(n) = starts.get_mut(c) {
            *n += before;
        }
    }
    let mut filled = starts.clone();
    let mut members = vec![0_usize; starts.last().copied().unwrap_or(0)];
    for (k, r) in reach.iter().enumerate() {
        if big.binary_search(&k).is_ok() {
            continue;
        }
        for ca in r[0]..=r[1] {
            for cb in r[2]..=r[3] {
                if let Some(at) = filled.get_mut(ca * side + cb)
                    && let Some(slot) = members.get_mut(*at)
                {
                    *slot = k;
                    *at += 1;
                }
            }
        }
    }
    let mut work = 0_usize;
    for c in 0..side * side {
        let (Some(&from), Some(&to)) = (starts.get(c), starts.get(c + 1)) else {
            continue;
        };
        let Some(list) = members.get(from..to) else {
            continue;
        };
        for (x, &p) in list.iter().enumerate() {
            for &q in list.iter().skip(x + 1) {
                work = work.saturating_add(1);
                if work > MOST_PAIRS {
                    return None;
                }
                let (Some(bp), Some(bq), Some(rp), Some(rq)) =
                    (live.get(p), live.get(q), reach.get(p), reach.get(q))
                else {
                    continue;
                };
                // Taken in the first cell both reach.
                let first = rp[0].max(rq[0]) * side + rp[2].max(rq[2]);
                if first == c && meet((bp.1, bp.2), (bq.1, bq.2)) {
                    pairs.push((bp.0, bq.0));
                }
            }
        }
    }
    for &g in &big {
        let Some(&(gi, gl, gh)) = live.get(g) else {
            continue;
        };
        for (k, &(ki, kl, kh)) in live.iter().enumerate() {
            // Two big boxes once.
            if k == g || (big.binary_search(&k).is_ok() && k < g) {
                continue;
            }
            work = work.saturating_add(1);
            if work > MOST_PAIRS {
                return None;
            }
            if meet((gl, gh), (kl, kh)) {
                pairs.push((gi.min(ki), gi.max(ki)));
            }
        }
    }
    Some(pairs)
}

fn grow(by: f64) -> Vec3 {
    Vec3::new(by, by, by)
}

fn axis(v: Vec3, k: usize) -> f64 {
    match k {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

/// Points of a curve evenly in its parameter with their parameters, and
/// the longest step between two of them.
#[derive(Clone)]
struct Samples {
    points: Vec<(f64, Vec3)>,
    step: f64,
}

/// A box holding the coedge's curve over its range, but for the
/// tolerance of its end vertices, which stand for its ends; `None` for one
/// that is not finite.
fn curve_box(u: &Use<'_>) -> Option<(Vec3, Vec3)> {
    let (curve, (t0, t1), (start, end)) = (u.curve, u.range, u.ends);
    let (low, high) = match curve {
        Curve3::Line(_) => (start.min(end), start.max(end)),
        Curve3::Circle(c) => {
            // An arc of less than half a turn lies within its sagitta of
            // its chord, `r (1 − cos(s / 2)) ≤ r s² / 8`; a longer one is
            // boxed by its extremes.
            let sweep = t1 - t0;
            let short = sweep < PI;
            if short {
                let sagitta = c.radius() * sweep * sweep / 8.0;
                (
                    start.min(end) - grow(sagitta),
                    start.max(end) + grow(sagitta),
                )
            } else {
                arc_box(curve, c.frame(), c.radius(), c.radius(), (t0, t1))
            }
        }
        Curve3::Ellipse(e) => {
            let (a, b) = e.semi_axes();
            arc_box(curve, e.frame(), a, b, (t0, t1))
        }
        Curve3::Nurbs(n) => {
            let mut low = start.min(end);
            let mut high = start.max(end);
            for p in n.control_points() {
                low = low.min(*p);
                high = high.max(*p);
            }
            (low, high)
        }
    };
    (low.is_finite() && high.is_finite()).then_some((low, high))
}

/// The box of an arc of a circle or an ellipse over `(t0, t1)`: its ends,
/// and along each axis the points where it is furthest that way, where
/// they lie on the arc (a whole turn: the box of the conic).
fn arc_box(curve: &Curve3, frame: &Frame, a: f64, b: f64, (t0, t1): (f64, f64)) -> (Vec3, Vec3) {
    let (x, y) = (frame.x(), frame.y());
    let less_than_a_turn = t1 - t0 < TAU;
    if !less_than_a_turn {
        let half = Vec3::new(
            (a * x.x).phypot(b * y.x),
            (a * x.y).phypot(b * y.y),
            (a * x.z).phypot(b * y.z),
        );
        return (frame.origin() - half, frame.origin() + half);
    }
    let (start, end) = (curve.eval(t0), curve.eval(t1));
    let (mut low, mut high) = (start.min(end), start.max(end));
    for (xi, yi) in [(x.x, y.x), (x.y, y.y), (x.z, y.z)] {
        // `a cos t xi + b sin t yi` is largest at `atan2(b yi, a xi)` and
        // least half a turn on.
        let top = (b * yi).patan2(a * xi);
        for extreme in [top, top + PI] {
            let first = extreme + ((t0 - extreme) / TAU).ceil() * TAU;
            if first.is_finite() && first <= t1 {
                let p = curve.eval(first);
                low = low.min(p);
                high = high.max(p);
            }
        }
    }
    (low, high)
}

/// Two coedges found on each other: a point of the stretch, its
/// parameter on each of the two edges, how long the stretch is at least,
/// and whether the two run the same way along it.
struct Found {
    at: Vec3,
    ta: f64,
    tb: f64,
    length: f64,
    same_way: bool,
}

/// `1` for a coedge that runs with its edge, `-1` against it.
fn sense(u: &Use<'_>) -> f64 {
    if u.reversed { -1.0 } else { 1.0 }
}

/// How far two curves' gaps may differ and still be one gap: the
/// rounding of the largest coordinate of their boxes.
fn rounding(a: &Use<'_>, b: &Use<'_>) -> f64 {
    let size = |u: &Use<'_>| {
        curve_box(u).map_or(f64::INFINITY, |(l, h)| {
            [l.x, l.y, l.z, h.x, h.y, h.z]
                .iter()
                .fold(0.0_f64, |m, c| m.max(c.abs()))
        })
    };
    ROUNDING * size(a).max(size(b))
}

/// Whether gaps measured along two stretches are one gap to `floor`: those
/// of copies of one curve (or of one offset by less than the tolerance)
/// are; those of two curves that part at a tip, growing from nothing at its
/// vertex, are not.
fn level(gaps: impl Iterator<Item = f64>, floor: f64) -> bool {
    let (low, high) = gaps.fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), g| {
        (l.min(g), h.max(g))
    });
    low.is_finite() && high.is_finite() && high - low <= floor
}

/// Whether the coedges `a` and `b` lie on each other over more than
/// `near`: within the tighter of their own tolerances, by one gap.
fn on_each_other(
    a: &Use<'_>,
    b: &Use<'_>,
    reach: Reach,
    samples: &mut Vec<Option<Samples>>,
    (ia, ib): (usize, usize),
    evaluations: &mut usize,
) -> Option<Found> {
    if a.edge == b.edge {
        // One edge used twice by one loop is a cut of the face along an
        // edge it holds on both sides: a seam where the surface closes, a
        // bridge joining two of its boundary curves (an annulus written as
        // one loop, as sheet metal's collars are), or a spur. Each leaves
        // the region and its measure what they are without the cut, and
        // the two uses alone cannot tell a right body from a wrong one
        // (the bridge and the spur both lie at one place of a plane): not
        // judged. (Two loops that share an edge are
        // `Defect::EdgeInTwoLoopsOfFace`.)
        return None;
    }
    // Two curves are one within the tighter of their edges' tolerances:
    // each then lies within its own of the other.
    let on = a.own.min(b.own);
    match (a.curve, b.curve) {
        (Curve3::Line(_), Curve3::Line(_)) => lines_on_each_other(a, b, on, reach.near),
        (Curve3::Circle(_) | Curve3::Ellipse(_), Curve3::Circle(_) | Curve3::Ellipse(_)) => {
            conics_on_each_other(a, b, on, reach.near)
        }
        (Curve3::Nurbs(_), _) | (_, Curve3::Nurbs(_)) => {
            sampled_on_each_other(a, b, (on, reach), samples, (ia, ib), evaluations)
        }
        // A line and a conic share no stretch.
        _ => None,
    }
}

/// Two lines: one curve when both ends of `b` lie within `on` of `a`'s
/// line, at one gap; their ranges read on `a` overlap by more than `near`.
fn lines_on_each_other(a: &Use<'_>, b: &Use<'_>, on: f64, near: f64) -> Option<Found> {
    let (Curve3::Line(la), Curve3::Line(lb)) = (a.curve, b.curve) else {
        return None;
    };
    let (o, d) = (la.origin(), la.direction());
    let along = |p: Vec3| (p - o).dot(d);
    let off = |p: Vec3| ((p - o) - d * along(p)).length();
    let (p0, p1) = (b.curve.eval(b.range.0), b.curve.eval(b.range.1));
    let (g0, g1) = (off(p0), off(p1));
    // Within the tolerance at both ends, and as far off at both: a line
    // that leaves the other at a tip is off by nothing at one end.
    if !(g0 <= on && g1 <= on && level([g0, g1].into_iter(), rounding(a, b))) {
        return None;
    }
    let (s0, s1) = (along(p0), along(p1));
    let low = a.range.0.max(s0.min(s1));
    let high = a.range.1.min(s0.max(s1));
    let ta = 0.5 * (low + high);
    let at = a.curve.eval(ta);
    let tb = (at - lb.origin()).dot(lb.direction());
    (high - low > near).then(|| Found {
        at,
        ta,
        tb,
        length: high - low,
        same_way: d.dot(lb.direction()) * sense(a) * sense(b) > 0.0,
    })
}

/// The frame and semi-axes of a circle or an ellipse.
fn conic(curve: &Curve3) -> Option<(Frame, f64, f64)> {
    match curve {
        Curve3::Circle(c) => Some((*c.frame(), c.radius(), c.radius())),
        Curve3::Ellipse(e) => {
            let (a, b) = e.semi_axes();
            Some((*e.frame(), a, b))
        }
        _ => None,
    }
}

/// Two conics: one curve when nine points of `b` lie within `on` of
/// `a`'s, at one gap (and their centres within twice that); their ranges,
/// read on `a` and taken modulo a turn, overlap over more than `near`.
fn conics_on_each_other(a: &Use<'_>, b: &Use<'_>, on: f64, near: f64) -> Option<Found> {
    let (fa, aa, ba) = conic(a.curve)?;
    let (fb, ..) = conic(b.curve)?;
    if fa.origin().distance(fb.origin()) > 2.0 * on {
        return None;
    }
    // `a`'s angle of a point, and how far the point is from `a`'s point
    // there.
    let on_a = |p: Vec3| -> (f64, f64) {
        let (x, y, _) = fa.local(p);
        let t = (y / ba).patan2(x / aa);
        let (s, c) = t.psin_cos();
        (t, fa.point(aa * c, ba * s, 0.0).distance(p))
    };
    let (t0, t1) = b.range;
    let mut angles = [0.0_f64; 9];
    let mut gaps = [0.0_f64; 9];
    let mut before: Option<f64> = None;
    for (k, (slot, gap)) in angles.iter_mut().zip(gaps.iter_mut()).enumerate() {
        let (t, off) = on_a(b.curve.eval(t0 + (t1 - t0) * k as f64 / 8.0));
        let lies_on = off <= on;
        if !lies_on {
            return None;
        }
        *gap = off;
        // Unwrapped by continuity: the steps are an eighth of a turn at
        // most (an edge longer than a turn is a fault before this).
        let t = match before {
            Some(prev) => prev + (t - prev + PI).rem_euclid(TAU) - PI,
            None => t,
        };
        *slot = t;
        before = Some(t);
    }
    if !level(gaps.iter().copied(), rounding(a, b)) {
        return None;
    }
    let (first, last) = (angles[0], angles[8]);
    let (low, high) = (first.min(last), first.max(last));
    let (r0, r1) = a.range;
    let mut total = 0.0;
    let mut middle = None;
    for k in -2..=2 {
        let shift = f64::from(k) * TAU;
        let (l, h) = (r0.max(low + shift), r1.min(high + shift));
        if h > l {
            total += h - l;
            middle.get_or_insert(0.5 * (l + h));
        }
    }
    // A lower bound on the length along the conic.
    let length = total * aa.min(ba);
    // `b` runs the way `a`'s angle grows, or against it.
    let along = if last > first { 1.0 } else { -1.0 };
    let ta = middle.filter(|_| length > near)?;
    let at = a.curve.eval(ta);
    let tb = b.curve.project(at).ok()?;
    Some(Found {
        at,
        ta,
        tb,
        length,
        same_way: along * sense(a) * sense(b) > 0.0,
    })
}

/// A spline against any curve: three places or more in a row of one
/// within `on` of the other, more than `near` apart from the first to the
/// last, at one gap.
#[allow(clippy::too_many_arguments)]
fn sampled_on_each_other(
    a: &Use<'_>,
    b: &Use<'_>,
    (on, reach): (f64, Reach),
    samples: &mut Vec<Option<Samples>>,
    (ia, ib): (usize, usize),
    evaluations: &mut usize,
) -> Option<Found> {
    let most = ia.max(ib);
    if samples.len() <= most {
        samples.resize(most + 1, None);
    }
    for (x, y, iy, x_is_a) in [(a, b, ib, true), (b, a, ia, false)] {
        let (t0, t1) = x.range;
        let parameter = |k: usize| t0 + (t1 - t0) * k as f64 / (PLACES + 1) as f64;
        let place = |k: usize| x.curve.eval(parameter(k));
        // A run of three in a row holds a third, sixth or ninth place. One
        // further than `on` and a step from every sample of `y` is not
        // looked for further (the search's Newton steps are what costs).
        *evaluations = evaluations.saturating_add(3);
        let thirds: Vec<Vec3> = [3, 6, 9].into_iter().map(place).collect();
        // A spline lies in the box of its control points.
        let inside = curve_box(y).is_some_and(|(l, h)| {
            let (l, h) = (l - grow(reach.grow), h + grow(reach.grow));
            thirds.iter().any(|p| {
                l.x <= p.x && p.x <= h.x && l.y <= p.y && p.y <= h.y && l.z <= p.z && p.z <= h.z
            })
        });
        if !inside {
            continue;
        }
        // Near the polyline of `y`'s samples (whose chords keep within an
        // eighth of a step of the curve where the curve is smooth on the
        // scale of its samples).
        let close = {
            let Some(slot) = samples.get_mut(iy) else {
                continue;
            };
            let list = slot.get_or_insert_with(|| {
                let s = sample(y.curve, y.range);
                *evaluations = evaluations.saturating_add(s.points.len());
                s
            });
            let reach = on + 0.125 * list.step;
            thirds.iter().any(|p| {
                list.points.windows(2).any(|w| match w {
                    [(_, a), (_, b)] => to_chord(*p, *a, *b) <= reach,
                    _ => false,
                })
            })
        };
        if !close {
            continue;
        }
        // At every place: `y`'s nearest point (gap and parameter) when
        // within `on`.
        let mut gaps: Vec<Option<(f64, f64)>> = Vec::with_capacity(PLACES);
        for k in 1..=PLACES {
            let p = place(k);
            let list = samples.get(iy).and_then(Option::as_ref)?;
            *evaluations = evaluations.saturating_add(2 + 2 * NEWTON_STEPS);
            gaps.push(nearest(y.curve, y.range, &list.points, p).filter(|(d, _)| *d <= on));
        }
        // The runs of places in a row within `on`, each held whole.
        let mut k = 0;
        while k < PLACES {
            let start = k;
            while gaps.get(k).is_some_and(Option::is_some) {
                k += 1;
            }
            let run = start..k;
            k += 1;
            if run.len() < 3 {
                continue;
            }
            let (first, last) = (run.start + 1, run.end);
            let length = place(first).distance(place(last));
            let run_gaps = gaps
                .get(run.clone())
                .into_iter()
                .flatten()
                .filter_map(|g| g.map(|(d, _)| d));
            if !(length > reach.near && level(run_gaps, rounding(a, b))) {
                continue;
            }
            // The middle of the run, and the two curves' ways there.
            let middle = run.start + run.len() / 2;
            let Some(&Some((_, ty))) = gaps.get(middle) else {
                continue;
            };
            let tx = parameter(middle + 1);
            *evaluations = evaluations.saturating_add(3);
            let dx = x.curve.derivatives(tx).1 * sense(x);
            let dy = y.curve.derivatives(ty).1 * sense(y);
            let (ta, tb) = if x_is_a { (tx, ty) } else { (ty, tx) };
            return Some(Found {
                at: x.curve.eval(tx),
                ta,
                tb,
                length,
                same_way: dx.dot(dy) > 0.0,
            });
        }
    }
    None
}

/// The distance from `p` to the chord from `a` to `b`.
fn to_chord(p: Vec3, a: Vec3, b: Vec3) -> f64 {
    let d = b - a;
    let length2 = d.length_squared();
    let t = if length2 > 0.0 {
        ((p - a).dot(d) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + d * t).distance(p)
}

/// Points of `curve` over `range`, evenly in its parameter, with their
/// parameters.
fn sample(curve: &Curve3, (t0, t1): (f64, f64)) -> Samples {
    let count = match curve {
        Curve3::Nurbs(n) => (2 * n.control_points().len()).clamp(16, MOST_SAMPLES),
        _ => 16,
    };
    let points: Vec<(f64, Vec3)> = (0..=count)
        .map(|k| {
            let t = t0 + (t1 - t0) * k as f64 / count as f64;
            (t, curve.eval(t))
        })
        .collect();
    let step = points
        .windows(2)
        .filter_map(|w| match w {
            [(_, a), (_, b)] => Some(a.distance(*b)),
            _ => None,
        })
        .fold(0.0, f64::max);
    Samples { points, step }
}

/// How far `p` is from `curve` over `range` at the point Newton steps
/// find from the nearest sample, and that point's parameter; `None` when
/// the distance is not a number.
fn nearest(
    curve: &Curve3,
    (t0, t1): (f64, f64),
    samples: &[(f64, Vec3)],
    p: Vec3,
) -> Option<(f64, f64)> {
    let (mut at, mut best) = samples
        .iter()
        .map(|(t, q)| (*t, q.distance(p)))
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    let mut best_at = at;
    for _ in 0..NEWTON_STEPS {
        let (q, d) = curve.derivatives(at);
        let speed = d.length_squared();
        if !(speed > 0.0 && speed.is_finite()) {
            break;
        }
        let next = (at - (q - p).dot(d) / speed).clamp(t0.min(t1), t0.max(t1));
        if !next.is_finite() {
            break;
        }
        let moved = (next - at).abs();
        at = next;
        let found = curve.eval(at).distance(p);
        if found < best {
            best = found;
            best_at = at;
        }
        // Settled to the rounding of the parameter.
        if moved <= 1e-14 * (t1 - t0).abs() {
            break;
        }
    }
    best.is_finite().then_some((best, best_at))
}

/// Whether the point found on both coedges lies at one place in their
/// loop's parameters as reached along each: found from the loop's own
/// points at its parameter on each edge, by a walk on the surface from
/// there (so a seam's two uses, at either border of a spline surface's
/// domain or a period apart, are two places whether or not the surface is
/// known to close). `false` where that cannot be told.
fn at_one_place(
    f: &Followed,
    surface: &Surface,
    a: &Use<'_>,
    b: &Use<'_>,
    found: &Found,
    near: f64,
) -> bool {
    let Some(ring) = f.rings.get(a.ring) else {
        return false;
    };
    if ring.aliased {
        return false;
    }
    // The place in parameters, and how fast the surface moves along each
    // parameter there.
    let place = |c: &Use<'_>, t: f64| -> Option<([f64; 2], [f64; 2])> {
        let from = *ring.starts.get(c.at)?;
        let to = ring
            .starts
            .get(c.at + 1)
            .copied()
            .unwrap_or(ring.points.len());
        let steps = to.checked_sub(from).filter(|s| *s > 0)?;
        let (t0, t1) = c.range;
        let fraction = if c.reversed {
            (t1 - t) / (t1 - t0)
        } else {
            (t - t0) / (t1 - t0)
        };
        if !fraction.is_finite() {
            return None;
        }
        // The loop's point at the start of the step that holds `t`.
        let k = ((fraction * steps as f64).floor().max(0.0) as usize).min(steps - 1);
        let reference = *ring.points.get(from + k)?;
        let ((u, v), (at, du, dv)) = surface
            .project_from_with_derivatives(found.at, (reference[0], reference[1]))
            .ok()?;
        let lands = at.distance(found.at) <= near;
        if !lands {
            return None;
        }
        // Moved by whole periods to the reference, as the walk moved it.
        let mut q = [u, v];
        for (d, period) in f.periods.iter().enumerate() {
            if let (Some(p), Some(x), Some(r)) = (period, q.get_mut(d), reference.get(d)) {
                *x += ((r - *x) / p).round() * p;
            }
        }
        (q[0].is_finite() && q[1].is_finite()).then_some((q, [du.length(), dv.length()]))
    };
    let (Some((pa, speed)), Some((pb, _))) = (place(a, found.ta), place(b, found.tb)) else {
        return false;
    };
    // One place: apart along each parameter by less than `near` on the
    // surface, read at the speed of that parameter there (a period of a
    // large cylinder's angle is a short way in parameters and a long way
    // on it). Where a parameter does not move the surface (a pole) the
    // place cannot be told.
    (0..2).all(|d| match (pa.get(d), pb.get(d), speed.get(d)) {
        (Some(x), Some(y), Some(s)) if *s > 0.0 && s.is_finite() => (x - y).abs() * s <= near,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    /// A loop gone round its circle twice, in `n` points.
    fn doubled(n: usize) -> Ring {
        let points: Vec<[f64; 2]> = (0..n)
            .map(|k| {
                let (s, c) = (2.0 * TAU * k as f64 / n as f64).psin_cos();
                [c, s]
            })
            .collect();
        Ring {
            points,
            winds: [0.0, 0.0],
            error: 0.0,
            low: [-1.0, -1.0],
            high: [1.0, 1.0],
            starts: vec![0],
            aliased: false,
        }
    }

    #[test]
    fn a_loop_past_the_work_cap_is_not_held_to_how_often_it_winds() {
        let ring = doubled(1000);
        // Found where the work stays under the cap: on the first line, at
        // the cost of a line's crossings and one distance.
        assert_eq!(wound_wrongly(&ring, 1e-6).map(|w| w.1), Some(2));
        assert_eq!(wound_within(&ring, 1e-6, 2000).map(|w| w.1), Some(2));
        // Past it: "cannot tell" (a pass), however wrong the loop.
        assert_eq!(wound_within(&ring, 1e-6, 1999), None);
        assert_eq!(wound_within(&ring, 1e-6, 999), None);
    }

    /// A plane face: a cubic spline out from the origin and its second
    /// half, split from it by de Casteljau, back (a spike), then lines
    /// home.
    fn spike() -> (crate::Body, crate::Face) {
        use crate::{BodyKind, Coedge};
        use craft3d_geom::{Line, NurbsCurve, Plane};
        let v = |x: f64, y: f64| Vec3::new(x, y, 0.0);
        let bezier = |p: [Vec3; 4]| {
            Curve3::Nurbs(
                NurbsCurve::polynomial(3, vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0], p.to_vec())
                    .unwrap(),
            )
        };
        let (p0, c1, c2, p3) = (v(0.0, 0.0), v(1.0, 1.0), v(3.0, -1.0), v(4.0, 0.0));
        let half = |a: Vec3, b: Vec3| (a + b) * 0.5;
        let (q0, q1, q2) = (half(p0, c1), half(c1, c2), half(c2, p3));
        let (r0, r1) = (half(q0, q1), half(q1, q2));
        let m = half(r0, r1);
        let corner = v(2.0, 2.0);
        let mut body = crate::Body::new(BodyKind::Solid);
        let (a, b, c, d) = (
            body.add_vertex(p0),
            body.add_vertex(p3),
            body.add_vertex(m),
            body.add_vertex(corner),
        );
        let out = body.add_edge(bezier([p0, c1, c2, p3]), (0.0, 1.0), a, b);
        let back = body.add_edge(bezier([m, r1, q2, p3]), (0.0, 1.0), c, b);
        let up = body.add_edge(
            Curve3::Line(Line::through(m, corner).unwrap()),
            (0.0, m.distance(corner)),
            c,
            d,
        );
        let home = body.add_edge(
            Curve3::Line(Line::through(corner, p0).unwrap()),
            (0.0, corner.distance(p0)),
            d,
            a,
        );
        let co = |edge, reversed| Coedge { edge, reversed };
        let lp = body.add_loop(vec![
            co(out, false),
            co(back, true),
            co(up, false),
            co(home, false),
        ]);
        let face = crate::Face {
            surface: Surface::Plane(Plane::new(Frame::WORLD)),
            reversed: false,
            loops: vec![lp],
        };
        (body, face)
    }

    #[test]
    fn a_face_past_the_evaluation_budget_is_not_held_to_rule_four() {
        let (body, face) = spike();
        let uses = uses_of(&body, &face, 1e-6);
        let reach = Reach {
            near: 1e-5,
            grow: 2e-6,
        };
        let found = overlap(&face, &uses, reach, None, false, MOST_EVALUATIONS);
        assert!(found.is_some_and(|(_, why)| why.contains("runs over itself")));
        // Past the budget: "cannot tell" (a pass).
        assert_eq!(overlap(&face, &uses, reach, None, false, 10), None);
    }
}
