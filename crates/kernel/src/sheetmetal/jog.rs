//! A jog across a flat (docs/design/sheetmetal.md › *Jogs across a
//! flat*): a line drawn across a flat cuts its outer loop at two
//! crossings into a near side, which keeps the flat's place, and a far
//! side, which becomes a flat of its own, joined by two bends of opposite
//! sign and the same radius and K-factor with a web between them. The
//! folded far side lies parallel to the near side, offset by `h` (between
//! the two bottom skins, and so between the two top skins).
//!
//! Along the jog's line `o + s d` (`û = (d_y, −d_x)` to its right, the
//! far side; `σ(p) = (p − o)·û`) the near side's loop runs from the
//! crossing `c₂` where the loop comes back from the far side, through the
//! near side, to the crossing `c₁` where it leaves, and is closed by the
//! chord `c₁ → c₂`, which runs along `d` with the near side on its left:
//! the first bend's tangent line, so the bend grows across it as a
//! flange does. The web is the rectangle `w` long beyond that bend's
//! strip, `w = (h − (2R + T)(1 − cos α)) / sin α` (the bottom skin rises
//! `(R + T)(1 − cos α)` round the first bend, whose outside it is,
//! `w sin α` along the web and `R (1 − cos α)` round the second, whose
//! inside it is); the second bend sits on the web's tip; the far side is
//! carried `(BA₁ + w + BA₂) û` on in the pattern (it is drawn as if the
//! jog had no width, as a flange's own outline is) and its loop starts
//! with the chord `c₂ → c₁` so carried: the second bend's far tangent
//! line, as a flange's flat starts with it.
//!
//! The line is drawn as if every bend had no width (the base's own
//! curves, and an outline of a flange, are drawn so too): it crosses the
//! flat whose outline, carried back by that flat's lift (how far it lies
//! from where it would with every bend of no width), holds the line's
//! middle.

use super::{
    Bend, Direction, Flat, KernelError, P2, SLIVER_TOLERANCES, SegId, add, line, refuse, scale,
    shifted, unit,
};
use crate::profile::{Loop2, Segment, Shape, distance, dot, sub};
use craft3d_math::{Portable, Tolerance, Transform};

/// A jog as the caller gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct JogSpec {
    /// Two distinct points of the line, in the pattern's axes as if every
    /// bend had no width.
    pub line: [P2; 2],
    /// The step between the near side's and the far side's bottom skins,
    /// more than 0.
    pub offset: f64,
    /// The angle of each bend, in (0, π).
    pub angle: f64,
    /// Which way the first bend turns the web (the far side goes that way).
    pub direction: Direction,
    pub radius: f64,
    pub k: f64,
    /// The ids its two bends are named by, as a flange's id: the first
    /// bend and the web, then the second bend and the far side.
    pub ids: [u64; 2],
}

/// What a jog makes: the near side (in the flat's place), the web and the
/// far side, the two bends, the near side's chord (the first bend's seat)
/// and the lifts of the web and the far side.
pub(super) struct Made {
    pub near: Flat,
    pub web: Flat,
    pub far: Flat,
    pub bends: [Bend; 2],
    pub chord: usize,
    pub web_lift: P2,
    pub far_lift: P2,
}

/// The web's flat length `w` for a step `h` at angle `α` with inner
/// radius `R` and thickness `T`, or `None` when the bends alone rise
/// past `h` (no web is left).
pub fn web_length(offset: f64, angle: f64, radius: f64, thickness: f64) -> Option<f64> {
    let (s, c) = angle.psin_cos();
    let w = (offset - (2.0 * radius + thickness) * (1.0 - c)) / s;
    (w.is_finite()).then_some(w)
}

/// Checks a jog's numbers.
pub(super) fn check(g: usize, jog: &JogSpec, tol: f64, angular: f64) -> Result<(), String> {
    let [a, b] = jog.line;
    if !(a.iter().chain(&b).all(|x| x.is_finite())) || !(distance(a, b) > tol) {
        return Err(format!(
            "jog {g}: its line needs two points more than the tolerance apart, got {a:?} and {b:?}"
        ));
    }
    if !(jog.offset.is_finite() && jog.offset > tol) {
        return Err(format!(
            "jog {g}: its offset has to be more than the tolerance, got {}",
            jog.offset
        ));
    }
    if !(jog.angle > angular && jog.angle < std::f64::consts::PI - angular) {
        return Err(format!(
            "jog {g}: its angle has to lie between 0 and 180 degrees (both bends turn by it, the second back), got {} degrees",
            jog.angle.to_degrees()
        ));
    }
    if !(jog.radius.is_finite() && jog.radius > tol) {
        return Err(format!(
            "jog {g}: its radius has to be more than the tolerance (a sharp bend is not built), got {}",
            jog.radius
        ));
    }
    if !(jog.k.is_finite() && (0.0..=1.0).contains(&jog.k)) {
        return Err(format!(
            "jog {g}: its K-factor has to lie in [0, 1], got {}",
            jog.k
        ));
    }
    Ok(())
}

/// Whether the flat's outline, carried back by `lift`, holds `p` (inside
/// its outer loop, outside its holes, more than the tolerance from both).
pub(super) fn holds(flat: &Flat, lift: P2, p: P2, tol: f64) -> bool {
    let q = add(p, lift);
    let near = |segs: &[Segment]| segs.iter().any(|s| segment_distance(s, q) <= tol);
    if near(&flat.outer) || flat.holes.iter().any(|h| near(h)) {
        return false;
    }
    let inside = |segs: &[Segment]| {
        Loop2 {
            segments: segs.to_vec(),
        }
        .winding_about(q)
        .abs()
            > 0.5
    };
    inside(&flat.outer) && !flat.holes.iter().any(|h| inside(h))
}

/// The least distance from `p` to a segment: exact for lines, arcs and
/// circles; for a spline the least over its control polygon's legs less
/// nothing (only used to keep a line's middle off the outline, where a
/// spline outline is refused by [`split`] if the line comes near it).
fn segment_distance(seg: &Segment, p: P2) -> f64 {
    let leg = |a: P2, b: P2| {
        let ab = sub(b, a);
        let l2 = dot(ab, ab);
        let t = if l2 > 0.0 {
            (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        distance(p, add(a, scale(ab, t)))
    };
    match &seg.shape {
        Shape::Line => leg(seg.start, seg.end),
        Shape::Circle { center, radius } => (distance(p, *center) - radius).abs(),
        Shape::Arc { center, radius } => {
            let ends = distance(p, seg.start).min(distance(p, seg.end));
            let Some((from, sweep)) = seg.angles() else {
                return ends;
            };
            let v = sub(p, *center);
            let phi = v[1].patan2(v[0]);
            if (phi - from).rem_euclid(std::f64::consts::TAU) <= sweep {
                ends.min((distance(p, *center) - radius).abs())
            } else {
                ends
            }
        }
        Shape::Spline(spline) => spline
            .control
            .windows(2)
            .filter_map(|w| match w {
                [a, b] => Some(leg(*a, *b)),
                _ => None,
            })
            .fold(f64::INFINITY, f64::min),
    }
}

/// Where the line `σ = 0` crosses a segment, strictly inside it, in the
/// order the segment runs: lines by the change of sign of `σ` at their
/// ends (an end within the tolerance of the line is refused by the
/// caller), arcs at the circle's roots inside the arc. Refused: a circle
/// (a loop of one curve), a spline that comes within the tolerance of the
/// line, an arc that touches the line.
fn crossings(g: usize, seg: &Segment, o: P2, d: P2, u: P2, tol: f64) -> Result<Vec<P2>, String> {
    let sigma = |p: P2| dot(sub(p, o), u);
    match &seg.shape {
        Shape::Line => {
            let (a, b) = (sigma(seg.start), sigma(seg.end));
            if a * b < 0.0 {
                let t = a / (a - b);
                Ok(vec![add(seg.start, scale(sub(seg.end, seg.start), t))])
            } else {
                Ok(Vec::new())
            }
        }
        Shape::Circle { .. } => Err(format!(
            "jog {g}: its line crosses a whole circle of the outline (curve {}); a jog across a loop of one circle is not built (owed): draw the outline of more than one curve",
            seg.source
        )),
        Shape::Spline(spline) => {
            let all = |f: &dyn Fn(f64) -> bool| spline.control.iter().all(|&p| f(sigma(p)));
            if all(&|s| s > tol) || all(&|s| s < -tol) {
                Ok(Vec::new())
            } else {
                Err(format!(
                    "jog {g}: its line crosses or comes within the tolerance of the spline curve {} of the outline (by its control points); a jog across a spline is not built (owed): let the line cross lines or arcs",
                    seg.source
                ))
            }
        }
        Shape::Arc { center, radius } => {
            let delta = sigma(*center);
            let Some((from, sweep)) = seg.angles() else {
                return Err(format!("jog {g}: an arc of the outline has no angles"));
            };
            let inside = |p: P2| {
                let v = sub(p, *center);
                let phi = v[1].patan2(v[0]);
                let at = (phi - from).rem_euclid(std::f64::consts::TAU);
                (at > 0.0 && at < sweep).then_some(at)
            };
            if (delta.abs() - radius).abs() <= tol {
                // The foot of the centre on the line: the arc touches it
                // there when that point lies on the arc.
                let foot = sub(*center, scale(u, delta));
                if inside(foot).is_some() {
                    return Err(format!(
                        "jog {g}: its line touches the arc curve {} of the outline (within the tolerance); move it so that it crosses the outline cleanly",
                        seg.source
                    ));
                }
                return Ok(Vec::new());
            }
            if delta.abs() > *radius {
                return Ok(Vec::new());
            }
            let half = (radius * radius - delta * delta).max(0.0).sqrt();
            let foot = sub(*center, scale(u, delta));
            let mut found: Vec<(f64, P2)> = [add(foot, scale(d, half)), sub(foot, scale(d, half))]
                .into_iter()
                .filter_map(|p| inside(p).map(|at| (at, p)))
                .collect();
            // In the order the loop runs the arc.
            found.sort_by(|x, y| x.0.total_cmp(&y.0));
            if seg.reversed {
                found.reverse();
            }
            Ok(found.into_iter().map(|(_, p)| p).collect())
        }
    }
}

/// A 48-bit FNV-1a hash of a segment's id: what the pieces of an edge a
/// jog cuts are named by, with the jog's id, so a jog moved to cut
/// another edge renames its pieces instead of handing them on.
fn hashed(id: &SegId) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for word in id {
        for byte in word.to_le_bytes() {
            h = (h ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    h & 0xffff_ffff_ffff
}

/// Splits flat `q` (laid out, its outline in the pattern's axes, `lift`
/// how far it lies from where it is drawn) along jog `g`'s line into the
/// near side, the web and the far side, with the two bends (`first` the
/// first bend's index; the web is flat `first + 1`, the far side
/// `first + 2`), numbered `numbers` (the ids less 1).
#[allow(clippy::too_many_arguments)]
pub(super) fn split(
    g: usize,
    jog: &JogSpec,
    q: usize,
    flat: &Flat,
    lift: P2,
    first: usize,
    numbers: [u64; 2],
    thickness: f64,
    tolerance: &Tolerance,
) -> Result<Made, KernelError> {
    let tol = tolerance.linear;
    let no = |reason: String| refuse(Some(first), reason);
    let [a, b] = jog.line;
    let (a, b) = (add(a, lift), add(b, lift));
    let mut d = unit(sub(b, a)).ok_or_else(|| no(format!("jog {g}: its line has no length")))?;
    let side = |d: P2| [d[1], -d[0]];
    // The near side: the base's is on the line's left; a flange's (or an
    // earlier far side's) is the side its own bend's tangent line, its
    // segment 0, lies on, which it must lie on wholly.
    if q > 0 {
        let seat = flat
            .outer
            .first()
            .ok_or_else(|| no(format!("jog {g}: the flat it crosses has no outline")))?;
        let u = side(d);
        let (sa, sb) = (dot(sub(seat.start, a), u), dot(sub(seat.end, a), u));
        if sa > tol && sb > tol {
            d = scale(d, -1.0);
        } else if !(sa < -tol && sb < -tol) {
            return Err(no(format!(
                "jog {g}: its line crosses or touches the tangent line of the bend the flat it crosses hangs from; a jog crosses a flat clear of that bend"
            )));
        }
    }
    let u = side(d);
    let sigma = |p: P2| dot(sub(p, a), u);

    // Every vertex off the line; the crossings in the loop's order.
    for s in &flat.outer {
        if sigma(s.start).abs() <= tol {
            return Err(no(format!(
                "jog {g}: its line passes within the tolerance of a corner of the flat's outline (at {:?}); move it so that it crosses edges, not corners",
                s.start
            )));
        }
    }
    let mut cut: Vec<(usize, Vec<P2>)> = Vec::new();
    let mut count = 0usize;
    for (k, s) in flat.outer.iter().enumerate() {
        let found = crossings(g, s, a, d, u, tol).map_err(no)?;
        count += found.len();
        if !found.is_empty() {
            cut.push((k, found));
        }
    }
    if count != 2 {
        return Err(no(format!(
            "jog {g}: its line crosses the outline of the flat it lies across {count} times; a jog cuts a flat into two pieces, so its line crosses the outline exactly twice"
        )));
    }
    // The pieces of the loop in order from the start of segment 0, each
    // with its id and whether it lies on the far side; a crossing between
    // pieces marks where the loop changes side.
    let first_far = flat
        .outer
        .first()
        .map(|s| sigma(s.start) > 0.0)
        .ok_or_else(|| no(format!("jog {g}: the flat has no outline")))?;
    let mut pieces: Vec<(Segment, SegId, bool, Option<usize>)> = Vec::new();
    let mut points: Vec<P2> = Vec::new();
    let mut breaks: Vec<usize> = Vec::new();
    let mut far = first_far;
    for (k, s) in flat.outer.iter().enumerate() {
        let id = flat.ids.get(k).copied().unwrap_or([12, 0, k as u64, 0]);
        let at = cut.iter().find(|(j, _)| *j == k).map(|(_, p)| p.as_slice());
        match at {
            None => pieces.push((s.clone(), id, far, Some(k))),
            Some(cross) => {
                let h = hashed(&id);
                let mut from = s.start;
                for (j, &p) in cross.iter().enumerate() {
                    let piece_id = [15, numbers[0], h, u64::from(far) * 4 + j as u64];
                    pieces.push((super::piece(s, from, p), piece_id, far, None));
                    breaks.push(pieces.len());
                    points.push(p);
                    far = !far;
                    from = p;
                }
                let j = cross.len() as u64;
                let piece_id = [15, numbers[0], h, u64::from(far) * 4 + j];
                pieces.push((super::piece(s, from, s.end), piece_id, far, None));
            }
        }
    }
    let floor = SLIVER_TOLERANCES * tol;
    if let Some((short, _, _, _)) = pieces
        .iter()
        .find(|(s, _, _, _)| !(distance(s.start, s.end) > floor) && matches!(s.shape, Shape::Line))
    {
        return Err(no(format!(
            "jog {g}: its line crosses the edge of curve {} within {floor} ({SLIVER_TOLERANCES} tolerances) of its end; move it so that it crosses edges clear of their ends",
            short.source
        )));
    }
    // c₁ where the loop goes from near to far, c₂ back.
    let (c1, c2) = match (points.as_slice(), first_far) {
        ([p, q], false) => (*p, *q),
        ([p, q], true) => (*q, *p),
        _ => {
            return Err(no(format!(
                "jog {g}: its line crosses the outline other than twice"
            )));
        }
    };
    if !(dot(sub(c2, c1), d) > floor) {
        return Err(no(format!(
            "jog {g}: the outline does not run across its line as one flat cut in two (the near side's chord would run against the line)"
        )));
    }
    // The loop's pieces from after c₂ round to c₁ (near) and from after
    // c₁ round to c₂ (far): rotate the list to start after a break.
    let n = pieces.len();
    let rotated = |start: usize| -> Vec<(Segment, SegId, bool, Option<usize>)> {
        (0..n)
            .filter_map(|i| pieces.get((start + i) % n).cloned())
            .collect()
    };
    let (Some(&b1), Some(&b2)) = (breaks.first(), breaks.get(1)) else {
        return Err(no(format!("jog {g}: the outline lost a crossing")));
    };
    // Which break is c₁'s (the start of the far run).
    let start_far = pieces.get(b1 % n).is_some_and(|p| p.2);
    let (after_c1, after_c2) = if start_far { (b1, b2) } else { (b2, b1) };
    let near_run: Vec<(Segment, SegId, Option<usize>)> = rotated(after_c2)
        .into_iter()
        .take_while(|p| !p.2)
        .map(|(s, id, _, k)| (s, id, k))
        .collect();
    let far_run: Vec<(Segment, SegId)> = rotated(after_c1)
        .into_iter()
        .take_while(|p| p.2)
        .map(|(s, id, _, _)| (s, id))
        .collect();
    if near_run.len() + far_run.len() != n {
        return Err(no(format!(
            "jog {g}: the outline does not fall into one near and one far run"
        )));
    }

    // The bends: the same radius and K-factor, opposite signs.
    let t = thickness;
    let theta = jog.angle;
    let neutral = jog.radius + jog.k * t;
    let allowance = theta * neutral;
    let w = web_length(jog.offset, theta, jog.radius, t).unwrap_or(f64::NAN);
    if !(w > tol) {
        return Err(no(format!(
            "jog {g}: its bends alone rise (2 radius + thickness)(1 − cos angle) = {}, which leaves a web of {w} for its offset {} (more than the tolerance is needed): give it a larger offset, a smaller radius or a steeper angle",
            (2.0 * jog.radius + t) * (1.0 - theta.pcos()),
            jog.offset
        )));
    }
    let signed = match jog.direction {
        Direction::Up => theta,
        Direction::Down => -theta,
    };
    let a0 = add(c1, scale(u, allowance));
    let a1 = add(c2, scale(u, allowance));
    let tip0 = add(a0, scale(u, w));
    let tip1 = add(a1, scale(u, w));
    let shift = scale(u, 2.0 * allowance + w);
    let bend =
        |parent: usize, child: usize, angle: f64, start: P2, end: P2, flat_length: f64| Bend {
            parent,
            child,
            angle,
            radius: jog.radius,
            k: jog.k,
            thickness: t,
            allowance,
            start,
            end,
            across: u,
            flat_length,
            trim: [None; 2],
        };

    // The near side: its run, then the chord c₁ → c₂ (the first bend's
    // seat).
    let mut near_outer: Vec<Segment> = near_run.iter().map(|(s, _, _)| s.clone()).collect();
    let mut near_ids: Vec<SegId> = near_run.iter().map(|(_, id, _)| *id).collect();
    let mut chord = near_outer.len();
    near_outer.push(line(c1, c2));
    near_ids.push([15, numbers[0], 0, 8]);
    // A flange's near side starts with its bend's tangent line, as every
    // flange's flat does (the fold and the sheet read it there).
    if q > 0 {
        let at = near_run
            .iter()
            .position(|(_, _, k)| *k == Some(0))
            .ok_or_else(|| no(format!("jog {g}: the flat lost its tangent line")))?;
        near_outer.rotate_left(at);
        near_ids.rotate_left(at);
        chord = (chord + near_outer.len() - at) % near_outer.len();
    }
    // The far side: the chord c₂ → c₁ carried, then its run carried.
    let mut far_outer = vec![shifted(&line(c2, c1), shift)];
    let mut far_ids: Vec<SegId> = vec![[15, numbers[0], 0, 9]];
    for (s, id) in &far_run {
        far_outer.push(shifted(s, shift));
        far_ids.push(*id);
    }
    let reach = far_outer
        .iter()
        .map(|s| super::reach_along(s, add(c1, shift), u).1)
        .fold(0.0, f64::max);

    // The holes go with the side they lie on; a hole the line crosses or
    // comes within the tolerance of is refused.
    let mut near_holes = Vec::new();
    let mut near_hole_ids = Vec::new();
    let mut far_holes = Vec::new();
    let mut far_hole_ids = Vec::new();
    for (h, hole) in flat.holes.iter().enumerate() {
        let mut sides = Vec::new();
        for s in hole {
            // The least and the most of σ over the curve (exact for lines,
            // arcs and circles, by the control points for a spline).
            let lo_hi = super::reach_along(s, a, u);
            if lo_hi.0 <= tol && lo_hi.1 >= -tol {
                return Err(no(format!(
                    "jog {g}: its line crosses or comes within the tolerance of hole {h} of the flat it lies across (curve {}); a hole across a jog's line is not built (owed): move the hole or the line",
                    s.source
                )));
            }
            sides.push(lo_hi.0 > 0.0);
        }
        let ids = flat.hole_ids.get(h).cloned().unwrap_or_default();
        if sides.iter().all(|&f| f) {
            far_holes.push(hole.iter().map(|s| shifted(s, shift)).collect());
            far_hole_ids.push(ids);
        } else {
            near_holes.push(hole.clone());
            near_hole_ids.push(ids);
        }
    }

    let web_number = numbers[0];
    let n_near = near_outer.len();
    let n_far = far_outer.len();
    let fresh = |outer: Vec<Segment>,
                 holes: Vec<Vec<Segment>>,
                 ids: Vec<SegId>,
                 hole_ids: Vec<Vec<SegId>>,
                 m: usize,
                 tip: Option<usize>,
                 outlined: bool| Flat {
        outer,
        holes,
        transform: Transform::IDENTITY,
        tip,
        ends: Vec::new(),
        was: (0..m).map(Some).collect(),
        seats: Vec::new(),
        ids,
        hole_ids,
        outlined,
        pieces: (0..=m).collect(),
        added: Vec::new(),
        notches: Vec::new(),
    };
    let near = fresh(
        near_outer,
        near_holes,
        near_ids,
        near_hole_ids,
        n_near,
        None,
        q > 0,
    );
    let web = fresh(
        vec![
            line(a1, a0),
            line(a0, tip0),
            line(tip0, tip1),
            line(tip1, a1),
        ],
        Vec::new(),
        (0..4).map(|side| [3, web_number, side, 0]).collect(),
        Vec::new(),
        4,
        Some(2),
        false,
    );
    let far = fresh(
        far_outer,
        far_holes,
        far_ids,
        far_hole_ids,
        n_far,
        None,
        true,
    );
    Ok(Made {
        near,
        web,
        far,
        bends: [
            bend(q, first + 1, signed, c1, c2, w),
            bend(first + 1, first + 2, -signed, tip0, tip1, reach),
        ],
        chord,
        web_lift: add(lift, scale(u, allowance)),
        far_lift: add(lift, shift),
    })
}
