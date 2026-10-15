//! Two freeform fillets that end in one vertex at an angle, the vertex's
//! third edge left sharp: the rims of a blade's tip face where they meet
//! at its leading edge, two edges of a block-like corner whose faces are
//! not planes.
//!
//! **What is kept.** Near the vertex the body is the corner of three faces:
//! `E`, which both edges lie on, and `F_a`, `F_b`, the other faces of the
//! two, which meet in the third edge. Rounding an edge keeps of the corner
//! of its two faces what balls of the radius inside it reach (for a convex
//! edge; for a concave one the same is true of the empty side). With both
//! edges rounded what is kept is what both keep: the faces of the result
//! near the vertex are each fillet's face where it lies inside the other's
//! corner and inside the far face (`F_b` for the fillet of `a`), and the
//! far faces where they lie inside the other fillet's corner.
//!
//! **How it is built.** Each fillet face is followed on past the vertex
//! until it is wholly out of its far face, as at a corner of three faces.
//! Then it is trimmed: per point across it (each `v`), where along it it
//! leaves what is kept, found by bisection on the larger of two distances
//! (outside the far face; inside the other fillet's corner, measured to
//! the other fillet's surface, or past its borders to the face beside it).
//! The two fillets run out into each other in the curve they share, which
//! starts on `E` where their borders there cross. Of the two, the one whose
//! border on its own side face reaches the far face first (lower down the
//! third edge) is also trimmed by the far face, from there up to where the
//! other fillet's border on the far face meets it: that fillet's end is two
//! curves, the other's one, and the third edge is shortened to where the
//! first one's border crosses it. When both reach the third edge in one
//! point, both ends are one curve that runs to that point, a vertex of four
//! edges.
//!
//! **Never a wrong solid.** Every point of the trims is found exactly and
//! the curves through them are refined to a quarter of the tolerance; the
//! two ends of the shared curve are held against both fillets' borders;
//! anything that does not fit this picture is refused with the reason.

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::super::super::{BlendError, Reason, lost, refused};
use super::super::{Item, MAX_SAMPLES, Side, curve_through, refuse};
use super::{Chain, Computed, EndKind, Meet, MeetEnd, PieceMade, measure, prepare, splice_chain};
use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, Coedge, Edge, EdgeId, FaceId, Vertex, VertexId};

/// One of the two fillets at a meet.
#[derive(Clone, Copy)]
struct Arm<'a> {
    /// Which chain, which of its ends (0 its start, 1 its end) and which
    /// of its pieces.
    chain: usize,
    end: usize,
    piece: usize,
    surface: &'a NurbsSurface,
    /// Its surface as a face (oriented out of the body).
    side: Side<'a>,
    /// The faces of its edge, and which of the two is the shared one.
    faces: [FaceId; 2],
    shared: usize,
    /// The far face: the other fillet's own face.
    far: FaceId,
    /// Where along it the trim is looked for.
    range: (f64, f64),
}

impl Arm<'_> {
    /// The `v` of its border on the shared face, and on its own face.
    fn v_shared(&self) -> f64 {
        if self.shared == 0 { 0.0 } else { 1.0 }
    }
    fn v_own(&self) -> f64 {
        1.0 - self.v_shared()
    }
    fn own(&self) -> Option<FaceId> {
        self.faces.get(1 - self.shared).copied()
    }
}

/// What a meet is worked out to be.
struct Made {
    /// Where the borders on the shared face cross, where the shared curve
    /// meets the far face of the long arm (or the third edge), and where
    /// the third edge now ends.
    q: Vec3,
    t: Vec3,
    r: Vec3,
    /// The arm also trimmed by its far face (`None`: both run to the
    /// third edge in one point).
    long: Option<usize>,
    /// The curve the two share (from `q` to `t`), and the long arm's cut
    /// by its far face (from `t` to `r`).
    shared: NurbsCurve,
    cut: Option<NurbsCurve>,
    /// The third edge's parameter at `r`.
    third_at: f64,
    /// Per arm, the parameters along it where its borders on the shared
    /// face and on its own face end.
    at: [[f64; 2]; 2],
}

/// A face of the body as a side.
fn side_of(body: &Body, face: FaceId) -> Result<Side<'_>, BlendError> {
    let f = body.face(face).ok_or_else(lost)?;
    Ok(Side {
        surface: &f.surface,
        reversed: f.reversed,
    })
}

/// How far `p` lies outside a face (positive: out of what is kept; for a
/// concave meet, what is kept is the empty side).
fn beyond(side: Side<'_>, p: Vec3, sign: f64) -> Option<f64> {
    let uv = side.surface.project(p).ok()?;
    let (d, _) = side.signed(p, uv)?;
    Some(sign * d)
}

/// How far `p` lies inside the corner arm `arm` takes away (positive:
/// out of what is kept): beyond its surface where the nearest point of it
/// is within its borders, and beyond the face past a border otherwise;
/// `None` past the border on its own face, where the corner is that
/// face's and the far face of the other arm holds instead.
fn beyond_fillet(body: &Body, arm: &Arm<'_>, p: Vec3, sign: f64, tol: f64) -> Option<Option<f64>> {
    let (u, v) = foot_on(arm, p)?;
    let past = |side: usize| -> Option<Option<f64>> {
        if side == arm.shared {
            // Beyond its border on the shared face the corner is that
            // face's, which the points of the other arm's border there lie
            // in: they are kept (they leave what is kept where they meet
            // this arm's surface, which touches the face along its border).
            let face = *arm.faces.get(side)?;
            Some(Some(beyond(side_of(body, face).ok()?, p, sign)? - tol))
        } else {
            Some(None)
        }
    };
    if v < 0.0 {
        return past(0);
    }
    if v > 1.0 {
        return past(1);
    }
    let n = arm.side.normal((u, v))?;
    Some(Some(sign * (p - arm.surface.eval(u, v)).dot(n)))
}

/// The nearest point of `arm`'s surface to `p`, as its parameters: by
/// damped Gauss-Newton steps from the nearest point of a grid, kept to a
/// little past its domain (a `v` past 0 or 1 says the point is beyond a
/// border).
fn foot_on(arm: &Arm<'_>, p: Vec3) -> Option<(f64, f64)> {
    let ((u0, u1), _) = arm.surface.domain();
    let (mut u, mut v) = nearest_start(arm, p)?;
    let span = (u1 - u0).abs().max(1e-12);
    for _ in 0..80 {
        let (s, su, sv) = arm.surface.derivatives(u, v);
        let r = p - s;
        let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
        let det = a * c - b * b;
        if !(det.abs() > 0.0) || !det.is_finite() {
            return None;
        }
        let (ru, rv) = (su.dot(r), sv.dot(r));
        let mut du = (c * ru - b * rv) / det;
        let mut dv = (a * rv - b * ru) / det;
        let longest = (du / span).abs().max(dv.abs());
        if longest > 0.25 {
            du *= 0.25 / longest;
            dv *= 0.25 / longest;
        }
        let (nu, nv) = (
            (u + du).clamp(u0 - 0.1 * span, u1 + 0.1 * span),
            (v + dv).clamp(-0.5, 1.5),
        );
        let moved = (su * (nu - u) + sv * (nv - v)).length();
        (u, v) = (nu, nv);
        if !(u.is_finite() && v.is_finite()) {
            return None;
        }
        if moved <= 1e-15 * (1.0 + s.length()) {
            break;
        }
    }
    Some((u, v))
}

/// A start for the nearest point of `arm`'s surface to `p`: the nearest of
/// a grid over the part near the meet.
fn nearest_start(arm: &Arm<'_>, p: Vec3) -> Option<(f64, f64)> {
    let ((u0, u1), _) = arm.surface.domain();
    let (a, b) = arm.range;
    let (lo, hi) = (a.min(b).max(u0), a.max(b).min(u1));
    let mut best = (f64::INFINITY, (lo, 0.5));
    for i in 0..=24 {
        let u = lo + (hi - lo) * f64::from(i) / 24.0;
        for j in 0..=8 {
            let v = f64::from(j) / 8.0;
            let d = arm.surface.eval(u, v).distance(p);
            if d < best.0 {
                best = (d, (u, v));
            }
        }
    }
    best.0.is_finite().then_some(best.1)
}

/// Where along `surface` (between `inner`, kept, and `outer`, not) the
/// point at `v` leaves what is kept by `g`.
fn crossing_by(
    surface: &NurbsSurface,
    v: f64,
    (inner, outer): (f64, f64),
    g: &dyn Fn(Vec3) -> Option<f64>,
    scale: f64,
) -> Option<f64> {
    let at = |s: f64| g(surface.eval(s, v));
    // Further in means towards the middle of the face's run.
    let middle = 0.5;
    let mut from = inner;
    let mut tries = 0;
    while !(at(from)? < 0.0) {
        // Further in, halfway to the middle of the face each time.
        tries += 1;
        if tries > 40 {
            return None;
        }
        from += (middle - from) * 0.5;
    }
    let (mut lo, mut hi) = (from, outer);
    let (mut glo, ghi) = (at(lo)?, at(hi)?);
    if !(glo < 0.0 && ghi > 0.0) {
        return None;
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        let gm = at(mid)?;
        if gm < 0.0 {
            lo = mid;
            glo = gm;
        } else {
            hi = mid;
        }
        if glo.abs() <= 1e-15 * scale {
            return Some(lo);
        }
        if (hi - lo).abs() <= 1e-15 * (1.0 + mid.abs()) {
            break;
        }
    }
    Some(0.5 * (lo + hi))
}

/// The curve on `surface` where, for `v` from `v0` to `v1`, it leaves
/// what `g` keeps; fitted within `limit` to points found exactly. Its
/// parameter runs from 0 at `v0` to 1 at `v1`; with the parameters along
/// the surface at its two ends. `start`, when given, is the parameter
/// along the surface where it starts (found otherwise than by `g`: where
/// two borders on a face both fillets are tangent to cross, which `g`
/// cannot tell to the tolerance).
fn trim(
    surface: &NurbsSurface,
    (v0, v1): (f64, f64),
    range: (f64, f64),
    g: &dyn Fn(Vec3) -> Option<f64>,
    (limit, scale): (f64, f64),
    start: Option<f64>,
) -> Option<(NurbsCurve, [f64; 2])> {
    let mut ts: Vec<f64> = (0..=8).map(|k| f64::from(k) / 8.0).collect();
    let v_at = |t: f64| v0 + (v1 - v0) * t;
    let crossing = |t: f64| match start {
        Some(s) if t <= 0.0 => Some(s),
        _ => crossing_by(surface, v_at(t), range, g, scale),
    };
    for _ in 0..12 {
        let ss: Vec<f64> = ts
            .iter()
            .map(|t| crossing(*t))
            .collect::<Option<Vec<_>>>()?;
        let points: Vec<Vec3> = ts
            .iter()
            .zip(&ss)
            .map(|(t, s)| surface.eval(*s, v_at(*t)))
            .collect();
        let curve = curve_through(&ts, &points)?;
        let mut added = Vec::new();
        for pair in ts.windows(2) {
            let [a, b] = pair else { continue };
            let t = 0.5 * (a + b);
            let s = crossing_by(surface, v_at(t), range, g, scale)?;
            let p = curve.eval(t);
            let d = p.distance(surface.eval(s, v_at(t)));
            // Where the surface and what `g` measures are nearly tangent
            // (near the start of the curve both fillets shared, where they
            // touch the face they share), the crossing is found only to
            // more than the tolerance along the surface, but every point
            // of the curve there lies on both within it.
            let on_both = || -> Option<bool> {
                Some(off_surface(surface, p, (s, v_at(t)))? <= limit && g(p)?.abs() <= limit)
            };
            if !(d <= limit || on_both() == Some(true)) {
                added.push(t);
            }
        }
        if added.is_empty() {
            let (Some(&first), Some(&last)) = (ss.first(), ss.last()) else {
                return None;
            };
            return Some((curve, [first, last]));
        }
        if ts.len() + added.len() > MAX_SAMPLES {
            return None;
        }
        ts.extend(added);
        ts.sort_by(f64::total_cmp);
    }
    None
}

/// How far `p` lies from `surface`, by Gauss-Newton steps from `start`
/// (near its foot).
fn off_surface(surface: &NurbsSurface, p: Vec3, (mut u, mut v): (f64, f64)) -> Option<f64> {
    for _ in 0..30 {
        let (s, su, sv) = surface.derivatives(u, v);
        let r = p - s;
        let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
        let det = a * c - b * b;
        if !(det.abs() > 0.0) || !det.is_finite() {
            return None;
        }
        let (ru, rv) = (su.dot(r), sv.dot(r));
        let (du, dv) = ((c * ru - b * rv) / det, (a * rv - b * ru) / det);
        u += du;
        v += dv;
        if !(u.is_finite() && v.is_finite()) {
            return None;
        }
        if (su * du + sv * dv).length() <= 1e-15 * (1.0 + s.length()) {
            break;
        }
    }
    Some(surface.eval(u, v).distance(p))
}

/// Where the borders of the two arms on the shared face cross, from
/// parameters along them near it: the nearest points of the two curves
/// (Gauss-Newton on the distance; both lie on the shared face to a quarter
/// of the tolerance, so they pass within half of it), with the point
/// halfway between them. `None` where they run along each other or do not
/// come within the tolerance.
fn cross_borders(
    la: &Arm<'_>,
    oa: &Arm<'_>,
    [mut a, mut b]: [f64; 2],
    tol: f64,
) -> Option<([f64; 2], Vec3)> {
    let (vl, vo) = (la.v_shared(), oa.v_shared());
    let ((la0, la1), _) = la.surface.domain();
    let ((oa0, oa1), _) = oa.surface.domain();
    let (sl, so) = ((la1 - la0).abs(), (oa1 - oa0).abs());
    // How long the two borders are near the meet: the crossing of two
    // curves that lie on the face to the tolerance and meet at the angle
    // θ is known along them only to tol / sin θ, which has to be less
    // than that.
    let reach = |arm: &Arm<'_>, v: f64| {
        let (r0, r1) = arm.range;
        arm.surface.eval(r0, v).distance(arm.surface.eval(r1, v))
    };
    let length = reach(la, vl).min(reach(oa, vo));
    for _ in 0..60 {
        let (p, pu, _) = la.surface.derivatives(a, vl);
        let (q, qu, _) = oa.surface.derivatives(b, vo);
        let r = p - q;
        // The normal equations of the least distance.
        let (m11, m12, m22) = (pu.dot(pu), -pu.dot(qu), qu.dot(qu));
        let (g1, g2) = (pu.dot(r), -qu.dot(r));
        let det = m11 * m22 - m12 * m12;
        // sin² θ = det / (m11 m22): the two run so nearly along each other
        // that where they cross is not known within their length.
        if !(det > 0.0 && det.sqrt() * length > tol * (m11 * m22).sqrt()) {
            return None;
        }
        let mut da = -(m22 * g1 - m12 * g2) / det;
        let mut db = -(m11 * g2 - m12 * g1) / det;
        let longest = (da / sl.max(1e-300)).abs().max((db / so.max(1e-300)).abs());
        if longest > 0.05 {
            da *= 0.05 / longest;
            db *= 0.05 / longest;
        }
        a += da;
        b += db;
        if !(a.is_finite() && b.is_finite()) {
            return None;
        }
        if (pu * da).length() + (qu * db).length() <= 1e-15 * (1.0 + p.length()) {
            break;
        }
    }
    let inside = |s: f64, (lo, hi): (f64, f64)| s >= lo.min(hi) && s <= lo.max(hi);
    if !(inside(a, (la0, la1)) && inside(b, (oa0, oa1))) {
        return None;
    }
    let (p, q) = (la.surface.eval(a, vl), oa.surface.eval(b, vo));
    (p.distance(q) <= tol).then(|| ([a, b], (p + q) * 0.5))
}

/// Works out the meet of `arms` at `vertex` (see the module
/// documentation).
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn work_out(
    body: &Body,
    arms: &[Arm<'_>; 2],
    third: EdgeId,
    sign: f64,
    edge: EdgeId,
    tolerance: &Tolerance,
    scale: f64,
) -> Result<Made, BlendError> {
    let tol = tolerance.linear;
    let limit = 0.25 * tol;
    let not_built = || refused(edge, Reason::FreeMeet);
    let [a, b] = arms;
    // What arm `x` keeps: inside its far face and outside the other's
    // corner; and the two parts of that.
    let far = |x: &Arm<'_>, p: Vec3| -> Option<f64> { beyond(side_of(body, x.far).ok()?, p, sign) };
    let other = |x: usize| if x == 0 { b } else { a };
    let corner = |x: usize, p: Vec3| -> Option<f64> {
        Some(beyond_fillet(body, other(x), p, sign, tol)?.unwrap_or(f64::NEG_INFINITY))
    };
    let both = |x: usize, p: Vec3| -> Option<f64> {
        let arm = if x == 0 { a } else { b };
        Some(far(arm, p)?.max(corner(x, p)?))
    };
    let third_data = body.edge(third).ok_or_else(lost)?;
    // Which constraint holds each arm at its own face's border, where it
    // leaves what is kept there, and how well that is known: the surfaces
    // are within a quarter of the tolerance of the true ones, so a
    // crossing of the border with the third edge, which it meets at the
    // angle θ, is known along it to half the tolerance over sin θ.
    let mut gaps = [0.0_f64; 2];
    let mut ends = [Vec3::ZERO; 2];
    let mut known = [0.0_f64; 2];
    for (x, arm) in arms.iter().enumerate() {
        let g = |p: Vec3| both(x, p);
        let s =
            crossing_by(arm.surface, arm.v_own(), arm.range, &g, scale).ok_or_else(not_built)?;
        let p = arm.surface.eval(s, arm.v_own());
        let (f, o) = (
            far(arm, p).ok_or_else(not_built)?,
            corner(x, p).ok_or_else(not_built)?,
        );
        // Positive: held by the far face (the other fillet's corner is
        // further on).
        if let Some(slot) = gaps.get_mut(x) {
            *slot = f - o;
        }
        if let Some(slot) = ends.get_mut(x) {
            *slot = p;
        }
        let (_, along, _) = arm.surface.derivatives(s, arm.v_own());
        let at = third_data.curve.project(p).map_err(|_| not_built())?;
        let (_, edge_along) = third_data.curve.derivatives(at);
        let sine = match (along.normalized(), edge_along.normalized()) {
            (Some(x), Some(y)) => x.cross(y).length(),
            _ => 0.0,
        };
        // A border that runs along the third edge does not say where it
        // reaches it.
        if !(sine > 0.0) {
            return Err(not_built());
        }
        if let Some(slot) = known.get_mut(x) {
            *slot = 0.5 * tol / sine;
        }
    }
    // Two places closer than both are known are one; and never closer
    // than the tolerance tells apart.
    let close = (known[0] + known[1]).max(tol);
    // Both run to the third edge in one point when their own borders leave
    // what is kept in one point. The gaps alone cannot tell: near the other
    // fillet's border on its own face that fillet is tangent to the face,
    // so a point a twentieth of the radius away along the face is within a
    // thousandth of the radius of it (and as close to the third edge).
    let [ea, eb] = ends;
    let long = match gaps {
        _ if ea.distance(eb) <= close => None,
        [ga, gb] if ga > 0.0 && gb < 0.0 => Some(0),
        [ga, gb] if gb > 0.0 && ga < 0.0 => Some(1),
        _ => return Err(not_built()),
    };
    let (l, o) = match long {
        Some(1) => (1, 0),
        _ => (0, 1),
    };
    let (la, oa) = (arms.get(l).ok_or_else(lost)?, arms.get(o).ok_or_else(lost)?);
    // The long arm's trim: by the other fillet from the shared face to
    // `t`, by its far face from `t` to its own face.
    let by_both = |p: Vec3| both(l, p);
    let (vq, vr) = (la.v_shared(), la.v_own());
    // The short arm's borders end where the long arm's trim reaches them:
    // on the shared face at `q`, on its own face at `t` (where the long
    // arm's trim goes over from the short arm to the short arm's own
    // face, tangent to it there: found along the short arm's border,
    // which crosses the long arm, rather than along the long arm's trim,
    // whose two pieces touch there).
    let by_long = |p: Vec3| both(o, p);
    // Where the two borders on the shared face cross: both fillets are
    // tangent to that face there, so the distance to the other fillet's
    // surface along a border is nearly flat, and where it changes sign is
    // found only to about the square root of the tolerance times the
    // radius. The crossing of the two borders is found directly instead
    // (two curves on one face that cross at the angle the edges meet at),
    // starting from where the distances put it.
    let lq0 =
        crossing_by(la.surface, la.v_shared(), la.range, &by_both, scale).ok_or_else(not_built)?;
    let oq0 =
        crossing_by(oa.surface, oa.v_shared(), oa.range, &by_long, scale).ok_or_else(not_built)?;
    let ([lq, oq], q) = cross_borders(la, oa, [lq0, oq0], tol).ok_or_else(not_built)?;
    let ot =
        crossing_by(oa.surface, oa.v_own(), oa.range, &by_long, scale).ok_or_else(not_built)?;
    let t_short = oa.surface.eval(ot, oa.v_own());
    let vt = match long {
        None => vr,
        Some(_) => {
            let (_, v) = foot_on(la, t_short).ok_or_else(not_built)?;
            if !((v - vq) * (vr - v) > 0.0) {
                return Err(not_built());
            }
            v
        }
    };
    // Both by the larger of the two, which is the one that holds there.
    let (shared, [sq, st]) = trim(
        la.surface,
        (vq, vt),
        la.range,
        &by_both,
        (limit, scale),
        Some(lq),
    )
    .ok_or_else(not_built)?;
    let (cut, sr) = match long {
        None => (None, st),
        Some(_) => {
            let (cut, [_, sr]) = trim(
                la.surface,
                (vt, vr),
                la.range,
                &by_both,
                (limit, scale),
                None,
            )
            .ok_or_else(not_built)?;
            (Some(cut), sr)
        }
    };
    let t = la.surface.eval(st, vt);
    let r = la.surface.eval(sr, vr);
    if !(t_short.distance(t) <= close) {
        return Err(not_built());
    }
    // The third edge, from the vertex back to `r`.
    let data = third_data;
    let mut third_at = data.curve.project(r).map_err(|_| not_built())?;
    if let Some(period) = data.curve.period().filter(|p| *p > 0.0) {
        let (lo, hi) = data.range;
        while third_at < lo - tol {
            third_at += period;
        }
        while third_at > hi + tol {
            third_at -= period;
        }
    }
    let (lo, hi) = data.range;
    // Strictly inside the third edge's range: it keeps a piece.
    let inside = third_at > lo && third_at < hi;
    if !(inside && data.curve.eval(third_at).distance(r) <= close) {
        return Err(refuse(edge, la.own(), Reason::FreeTooLarge));
    }
    let mut at = [[0.0; 2]; 2];
    *at.get_mut(l).ok_or_else(lost)? = [sq, sr];
    *at.get_mut(o).ok_or_else(lost)? = [oq, ot];
    Ok(Made {
        q,
        t,
        r,
        long,
        shared,
        cut,
        third_at,
        at,
    })
}

/// Blends the chains that end in meets (`which` of `chains`), with the
/// meets between them, in place: works all of them out on the body as it
/// is, trims them at the meets, puts them in and shortens the third
/// edges. Returns each blend face with the index of its request.
#[allow(clippy::too_many_lines)]
pub(super) fn build_with(
    body: &mut Body,
    chains: &[Chain],
    which: &[usize],
    meets: &[Meet],
    items: &[Item],
    tolerance: &Tolerance,
) -> Result<Vec<(FaceId, usize)>, BlendError> {
    let tol = tolerance.linear;
    let mut computed: Vec<(usize, Computed)> = Vec::with_capacity(which.len());
    for c in which {
        let chain = chains.get(*c).ok_or_else(lost)?;
        computed.push((*c, prepare(body, chain, items, meets, tolerance)?));
    }
    // The blend surfaces as faces, oriented, to measure against.
    let wrapped: Vec<Vec<Surface>> = computed
        .iter()
        .map(|(_, c)| {
            c.pieces
                .iter()
                .map(|p| Surface::Nurbs(p.surface.clone()))
                .collect()
        })
        .collect();
    let oriented = |m: &PieceMade| -> bool {
        let [(u0, u1), _] = m.spans;
        let (_, du, dv) = m.surface.derivatives(0.5 * (u0 + u1), 0.5);
        du.cross(dv).dot(m.outward) < 0.0
    };
    let mut mades = Vec::with_capacity(meets.len());
    for (index, meet) in meets.iter().enumerate() {
        let mut arms = Vec::with_capacity(2);
        for item in meet.items {
            // The chain end at this meet whose end piece is `item`'s.
            let mut found = None;
            for (k, (c, comp)) in computed.iter().enumerate() {
                let chain = chains.get(*c).ok_or_else(lost)?;
                for end in 0..2 {
                    if chain.meets.get(end).copied().flatten() != Some(index) {
                        continue;
                    }
                    let piece = if end == 0 { 0 } else { chain.pieces.len() - 1 };
                    let p = chain.pieces.get(piece).ok_or_else(lost)?;
                    if p.item != item {
                        continue;
                    }
                    let Some(Some(EndKind::Meet { range, .. })) =
                        comp.ends.as_ref().map(|e| e.get(end))
                    else {
                        return Err(lost());
                    };
                    let m = comp.pieces.get(piece).ok_or_else(lost)?;
                    let surface = wrapped.get(k).and_then(|w| w.get(piece)).ok_or_else(lost)?;
                    let shared = p
                        .faces
                        .iter()
                        .position(|f| *f == meet.shared)
                        .ok_or_else(lost)?;
                    let far_faces =
                        super::faces_of(&super::super::super::edge_uses(body), meet.third)
                            .ok_or_else(lost)?;
                    let far = far_faces
                        .into_iter()
                        .find(|f| !p.faces.contains(f))
                        .ok_or_else(lost)?;
                    found = Some(Arm {
                        chain: k,
                        end,
                        piece,
                        surface: &m.surface,
                        side: Side {
                            surface,
                            reversed: oriented(m),
                        },
                        faces: p.faces,
                        shared,
                        far,
                        range: *range,
                    });
                }
            }
            arms.push(found.ok_or_else(lost)?);
        }
        let [a, b] = <[Arm<'_>; 2]>::try_from(arms).map_err(|_| lost())?;
        let first = items.get(meet.items[0]).ok_or_else(lost)?;
        let sign = if first.convex { 1.0 } else { -1.0 };
        let size = match first.request.size {
            super::super::super::Size::Radius(r) => r,
            super::super::super::Size::Chamfer(_) => return Err(lost()),
        };
        let scale = body
            .vertex(meet.vertex)
            .map_or(tol, |v| v.point.length().max(tol))
            .max(size);
        let edge = items.get(meet.items[1]).ok_or_else(lost)?.request.edge;
        let ends = [
            (a.chain, a.end, a.piece, a.shared),
            (b.chain, b.end, b.piece, b.shared),
        ];
        let made = work_out(body, &[a, b], meet.third, sign, edge, tolerance, scale)?;
        mades.push((made, ends));
    }
    drop(wrapped);
    // The trims shorten the borders at the meets.
    for (made, ends) in &mades {
        for (x, (k, end, piece, shared)) in ends.iter().enumerate() {
            let [at_shared, at_own] = *made.at.get(x).ok_or_else(lost)?;
            let comp = &mut computed.get_mut(*k).ok_or_else(lost)?.1;
            let spans = &mut comp.pieces.get_mut(*piece).ok_or_else(lost)?.spans;
            for (side, s) in [(*shared, at_shared), (1 - *shared, at_own)] {
                let span = spans.get_mut(side).ok_or_else(lost)?;
                if *end == 0 {
                    span.0 = s;
                } else {
                    span.1 = s;
                }
                if !(span.1 > span.0) {
                    let c = computed.get(*k).map_or(usize::MAX, |c| c.0);
                    let edge = chains
                        .get(c)
                        .and_then(|c| c.pieces.get(*piece))
                        .map_or(EdgeId(u32::MAX), |p| p.edge);
                    return Err(refused(edge, Reason::FreeTooLarge));
                }
            }
        }
    }
    // The vertices and edges of the meets.
    let new_vertex = |body: &mut Body, point: Vec3| -> Result<VertexId, BlendError> {
        let id = VertexId(u32::try_from(body.vertices.len()).map_err(|_| lost())?);
        body.vertices.push(Vertex { point });
        Ok(id)
    };
    let new_edge = |body: &mut Body, edge: Edge| -> Result<EdgeId, BlendError> {
        let id = EdgeId(u32::try_from(body.edges.len()).map_err(|_| lost())?);
        body.edges.push(edge);
        Ok(id)
    };
    // Per chain (in `computed`), what its two ends get from meets.
    let mut given: Vec<[Option<MeetEnd>; 2]> = vec![[None, None]; computed.len()];
    let mut finishing = Vec::with_capacity(mades.len());
    for ((made, ends), meet) in mades.iter().zip(meets) {
        let q = meet.vertex;
        if let Some(v) = body.vertices.get_mut(q.0 as usize) {
            v.point = made.q;
        }
        let r = new_vertex(body, made.r)?;
        let t = match made.long {
            Some(_) => new_vertex(body, made.t)?,
            None => r,
        };
        let shared = new_edge(
            body,
            Edge {
                curve: Curve3::Nurbs(made.shared.clone()),
                range: (0.0, 1.0),
                start: q,
                end: t,
            },
        )?;
        let cut = match &made.cut {
            Some(curve) => Some(new_edge(
                body,
                Edge {
                    curve: Curve3::Nurbs(curve.clone()),
                    range: (0.0, 1.0),
                    start: t,
                    end: r,
                },
            )?),
            None => None,
        };
        for (x, (k, end, _, side_shared)) in ends.iter().enumerate() {
            let is_long = made.long == Some(x);
            let own_corner = if is_long || made.long.is_none() { r } else { t };
            // From the shared face's corner to the own face's.
            let mut edges = vec![(shared, false)];
            if is_long {
                edges.push((cut.ok_or_else(lost)?, false));
            }
            let mut corners = [q, own_corner];
            if *side_shared == 1 {
                corners = [own_corner, q];
                edges = edges.into_iter().rev().map(|(e, r)| (e, !r)).collect();
            }
            let slot = given
                .get_mut(*k)
                .and_then(|g| g.get_mut(*end))
                .ok_or_else(lost)?;
            *slot = Some(MeetEnd { corners, edges });
        }
        finishing.push((r, t, cut));
    }
    // The chains, with their ends.
    let mut spliced = Vec::with_capacity(computed.len());
    for ((c, comp), ends) in computed.into_iter().zip(given) {
        let chain = chains.get(c).ok_or_else(lost)?;
        spliced.push((c, splice_chain(body, chain, comp, ends, tolerance)?));
    }
    // The third edges, and the long arm's cut in its far face.
    for (((made, ends), meet), (r, t, cut)) in mades.iter().zip(meets).zip(&finishing) {
        let slot = body.edges.get_mut(meet.third.0 as usize).ok_or_else(lost)?;
        if slot.start == meet.vertex {
            slot.range.0 = made.third_at;
            slot.start = *r;
        } else if slot.end == meet.vertex {
            slot.range.1 = made.third_at;
            slot.end = *r;
        } else {
            return Err(lost());
        }
        // Meets at both ends of one third edge (a thin face's two rims
        // rounded where they meet a third face) take its two ends; what
        // they take may not overlap.
        if !(slot.range.1 - slot.range.0 > 0.0) {
            let edge = items.get(meet.items[1]).ok_or_else(lost)?.request.edge;
            let face = items
                .get(meet.items[0])
                .and_then(|i| i.faces.iter().copied().find(|f| *f != meet.shared));
            return Err(refuse(edge, face, Reason::FreeTooLarge));
        }
        if let (Some(long), Some(cut)) = (made.long, cut) {
            // The far face of the long arm is the other arm's own face.
            let _ = ends;
            let far_faces = super::faces_of(&super::super::super::edge_uses(body), meet.third)
                .ok_or_else(lost)?;
            let long_item = meet.items.get(long).ok_or_else(lost)?;
            let long_faces = items.get(*long_item).ok_or_else(lost)?.faces;
            let far = far_faces
                .into_iter()
                .find(|f| !long_faces.contains(f))
                .ok_or_else(lost)?;
            insert_between(body, far, *cut, *t, *r)?;
        }
    }
    // Measured once everything is in.
    let mut faces = Vec::new();
    for (c, made) in &spliced {
        let chain = chains.get(*c).ok_or_else(lost)?;
        faces.extend(measure(body, chain, made, tolerance)?);
    }
    Ok(faces)
}

/// Puts `cut` (from `from` to `to`) into the loop of `face` between the
/// coedge that arrives at one of the two and the one that leaves the
/// other.
fn insert_between(
    body: &mut Body,
    face: FaceId,
    cut: EdgeId,
    from: VertexId,
    to: VertexId,
) -> Result<(), BlendError> {
    let loops = body.face(face).ok_or_else(lost)?.loops.clone();
    for l in loops {
        let ring = body.loops.get(l.0 as usize).ok_or_else(lost)?.clone();
        let ends = |c: &Coedge| -> Option<(VertexId, VertexId)> {
            let e = body.edge(c.edge)?;
            Some(if c.reversed {
                (e.end, e.start)
            } else {
                (e.start, e.end)
            })
        };
        let n = ring.coedges.len();
        for i in 0..n {
            let (Some(here), Some(next)) = (ring.coedges.get(i), ring.coedges.get((i + 1) % n))
            else {
                continue;
            };
            let (Some((_, arrives)), Some((leaves, _))) = (ends(here), ends(next)) else {
                continue;
            };
            let insert = if arrives == from && leaves == to {
                Some(false)
            } else if arrives == to && leaves == from {
                Some(true)
            } else {
                None
            };
            if let Some(reversed) = insert {
                let mut coedges = ring.coedges.clone();
                coedges.insert(
                    i + 1,
                    Coedge {
                        edge: cut,
                        reversed,
                    },
                );
                if let Some(slot) = body.loops.get_mut(l.0 as usize) {
                    slot.coedges = coedges;
                }
                return Ok(());
            }
        }
    }
    Err(lost())
}
