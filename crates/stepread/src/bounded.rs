//! Bounded curves as one exact B-spline: a curve run the other way, the
//! part of a curve between two of its parameters (`TRIMMED_CURVE`), and
//! curves joined end to end (`COMPOSITE_CURVE`).
//!
//! Every construction is exact (up to rounding): a line segment is a
//! B-spline of degree one; an arc of a circle is the rational quadratic of
//! pieces of at most a quarter turn (`geom::network::arc`), and of an
//! ellipse the same arc with its control points scaled along the minor
//! axis (an affine map takes a rational Bézier curve to the one of its
//! mapped control points, with the same weights); part of a B-spline is
//! its Bézier pieces cut by de Casteljau's algorithm in homogeneous
//! coordinates. Curves are joined as Bézier pieces raised to one degree
//! (degree elevation of each piece in homogeneous coordinates, *The NURBS
//! Book* §5.5, which changes no point) and laid end to end with inner
//! knots of full multiplicity; the weights of each piece are scaled so
//! its first matches the end of the one before (a rational Bézier curve is
//! the same curve with all its weights times one number).

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]
use craft3d_geom::{Curve3, Line, NurbsCurve};
use craft3d_math::Vec3;
use std::f64::consts::TAU;

/// Most control points a curve joined here may have.
pub const MAX_JOINED: usize = 1 << 20;

/// A rational Bézier piece: its parameter range and its control points
/// with their weights.
#[derive(Clone, Debug)]
struct Piece {
    range: (f64, f64),
    points: Vec<(Vec3, f64)>,
}

/// `curve` run the other way, exactly: a line along its negated direction,
/// a circle or an ellipse on its frame seen from the other side (the
/// parameter `t` becomes `−t`), a B-spline with its knots mirrored and its
/// points and weights in the opposite order.
pub fn reversed(curve: &Curve3) -> Result<Curve3, String> {
    let degenerate = || "a curve that cannot be turned round".to_string();
    Ok(match curve {
        Curve3::Line(line) => {
            Curve3::Line(Line::new(line.origin(), -line.direction()).map_err(|e| e.to_string())?)
        }
        Curve3::Circle(c) => {
            let frame = c
                .frame()
                .reversed_at(c.frame().origin())
                .ok_or_else(degenerate)?;
            Curve3::Circle(craft3d_geom::Circle::new(frame, c.radius()).map_err(|e| e.to_string())?)
        }
        Curve3::Ellipse(e) => {
            let frame = e
                .frame()
                .reversed_at(e.frame().origin())
                .ok_or_else(degenerate)?;
            let (a, b) = e.semi_axes();
            Curve3::Ellipse(craft3d_geom::Ellipse::new(frame, a, b).map_err(|e| e.to_string())?)
        }
        Curve3::Nurbs(n) => Curve3::Nurbs(reversed_nurbs(n)?),
    })
}

/// A B-spline run the other way: `t ↦ a + b − t` over its domain `[a, b]`.
pub fn reversed_nurbs(curve: &NurbsCurve) -> Result<NurbsCurve, String> {
    let (a, b) = curve.domain();
    let knots: Vec<f64> = curve.knots().iter().rev().map(|&k| a + b - k).collect();
    let points: Vec<Vec3> = curve.control_points().iter().rev().copied().collect();
    let weights: Vec<f64> = curve.weights().iter().rev().copied().collect();
    NurbsCurve::new(curve.degree(), knots, points, weights).map_err(|e| e.to_string())
}

/// The part of `curve` from its parameter `from` to `to`, as one exact
/// B-spline running from `from` to `to`: forward when `forward`, backward
/// otherwise (round a circle or an ellipse the way `forward` says, once at
/// most, the whole of it when the two are the same). On an open curve the
/// order of the two must agree with `forward`.
pub fn part(curve: &Curve3, from: f64, to: f64, forward: bool) -> Result<NurbsCurve, String> {
    if !(from.is_finite() && to.is_finite()) {
        return Err("the ends of a trimmed curve must be finite".to_string());
    }
    let geom = |e: craft3d_geom::GeomError| e.to_string();
    match curve {
        Curve3::Circle(_) | Curve3::Ellipse(_) => {
            let sweep = if forward {
                let s = (to - from).rem_euclid(TAU);
                if s > 0.0 { s } else { TAU }
            } else {
                let s = (from - to).rem_euclid(TAU);
                -(if s > 0.0 { s } else { TAU })
            };
            let (frame, a, b) = match curve {
                Curve3::Circle(c) => (*c.frame(), c.radius(), c.radius()),
                Curve3::Ellipse(e) => {
                    let (a, b) = e.semi_axes();
                    (*e.frame(), a, b)
                }
                _ => return Err("not a circle or an ellipse".to_string()),
            };
            let arc = craft3d_geom::network::arc(&frame, a, from, sweep).map_err(geom)?;
            if a == b {
                return Ok(arc);
            }
            // The ellipse is the circle of radius `a` scaled by `b / a`
            // along the frame's y axis.
            let squeeze = b / a;
            let points = arc
                .control_points()
                .iter()
                .map(|&p| {
                    let (x, y, z) = frame.local(p);
                    frame.point(x, y * squeeze, z)
                })
                .collect();
            NurbsCurve::new(2, arc.knots().to_vec(), points, arc.weights().to_vec()).map_err(geom)
        }
        _ => {
            order(from, to, forward)?;
            let (lo, hi) = if forward { (from, to) } else { (to, from) };
            let ahead = match curve {
                Curve3::Line(_) => NurbsCurve::polynomial(
                    1,
                    vec![lo, lo, hi, hi],
                    vec![curve.eval(lo), curve.eval(hi)],
                )
                .map_err(geom)?,
                Curve3::Nurbs(n) => restricted(n, lo, hi)?,
                _ => return Err("not an open curve".to_string()),
            };
            if forward {
                Ok(ahead)
            } else {
                reversed_nurbs(&ahead)
            }
        }
    }
}

/// Two parameters of an open curve in the order `forward` says, apart.
pub fn order(from: f64, to: f64, forward: bool) -> Result<(), String> {
    if from == to {
        return Err(format!(
            "its two trims are the same ({from}): no part of the curve lies between them"
        ));
    }
    if (from < to) != forward {
        return Err(format!(
            "its trims run from {from} to {to}, against its sense_agreement (.{}.) on a curve that does not close",
            if forward { "T" } else { "F" }
        ));
    }
    Ok(())
}

/// `curve` over `[lo, hi]` (within its domain), as a B-spline of its
/// Bézier pieces there.
fn restricted(curve: &NurbsCurve, lo: f64, hi: f64) -> Result<NurbsCurve, String> {
    let (a, b) = curve.domain();
    let slack = 1e-12 * (b - a).abs().max(a.abs()).max(b.abs());
    if !(lo >= a - slack && hi <= b + slack && lo < hi) {
        return Err(format!(
            "its trims {lo} to {hi} leave the B-spline's domain {a} to {b}"
        ));
    }
    let (lo, hi) = (lo.max(a), hi.min(b));
    let mut pieces = Vec::new();
    for ((p0, p1), points) in curve.bezier_pieces() {
        if !(p1 > lo && p0 < hi) {
            continue;
        }
        let mut piece = Piece {
            range: (p0, p1),
            points,
        };
        if p0 < lo {
            piece = cut(&piece, lo).1;
        }
        if piece.range.1 > hi {
            piece = cut(&piece, hi).0;
        }
        pieces.push(piece);
    }
    joined(pieces, 0.0)
}

/// A piece cut at `t` inside its range: the parts before and after
/// (de Casteljau in homogeneous coordinates).
fn cut(piece: &Piece, t: f64) -> (Piece, Piece) {
    let (a, b) = piece.range;
    let s = (t - a) / (b - a);
    let mut row: Vec<(Vec3, f64)> = piece.points.iter().map(|&(p, w)| (p * w, w)).collect();
    let n = row.len();
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for level in 0..n {
        if let Some(&first) = row.first() {
            left.push(first);
        }
        if let Some(&last) = row.last() {
            right.push(last);
        }
        if level + 1 < n {
            row = row
                .windows(2)
                .filter_map(|w| match w {
                    [(p, u), (q, v)] => Some((*p * (1.0 - s) + *q * s, u * (1.0 - s) + v * s)),
                    _ => None,
                })
                .collect();
        }
    }
    right.reverse();
    let back = |h: Vec<(Vec3, f64)>| -> Vec<(Vec3, f64)> {
        h.into_iter().map(|(p, w)| (p * (1.0 / w), w)).collect()
    };
    let (mut left, mut right) = (back(left), back(right));
    // The piece's own ends, bit for bit (not divided back out of their
    // homogeneous form), so the pieces of one curve meet exactly.
    if let (Some(slot), Some(&first)) = (left.first_mut(), piece.points.first()) {
        *slot = first;
    }
    if let (Some(slot), Some(&last)) = (right.last_mut(), piece.points.last()) {
        *slot = last;
    }
    (
        Piece {
            range: (a, t),
            points: left,
        },
        Piece {
            range: (t, b),
            points: right,
        },
    )
}

/// One degree more, the same curve (homogeneous coordinates).
fn elevated(points: &[(Vec3, f64)]) -> Vec<(Vec3, f64)> {
    let p = points.len().saturating_sub(1);
    let h: Vec<(Vec3, f64)> = points.iter().map(|&(q, w)| (q * w, w)).collect();
    let mut out = Vec::with_capacity(p + 2);
    for i in 0..=p + 1 {
        let alpha = i as f64 / (p + 1) as f64;
        let before = if i == 0 { None } else { h.get(i - 1) };
        let here = h.get(i);
        let (q, w) = match (before, here) {
            (Some(&(a, u)), Some(&(b, v))) => {
                (a * alpha + b * (1.0 - alpha), u * alpha + v * (1.0 - alpha))
            }
            (Some(&(a, u)), None) => (a, u),
            (None, Some(&(b, v))) => (b, v),
            (None, None) => continue,
        };
        out.push((q * (1.0 / w), w));
    }
    // The ends, bit for bit.
    if let (Some(slot), Some(&first)) = (out.first_mut(), points.first()) {
        *slot = first;
    }
    if let (Some(slot), Some(&last)) = (out.last_mut(), points.last()) {
        *slot = last;
    }
    out
}

/// Curves joined end to end into one B-spline, the first's start to the
/// last's end, each over one unit of the joined parameter (`[k, k + 1]`
/// for the `k`-th), split among its pieces in proportion to their
/// parameters. A joint whose two ends lie more than `near` apart is
/// refused, naming it; ends within `near` become one control point at the
/// end of the first, as two vertices of a file within its uncertainty
/// become one (the edges on the joined curve are then proven on their
/// faces like any other).
pub fn join(curves: &[NurbsCurve], near: f64) -> Result<NurbsCurve, String> {
    let mut pieces = Vec::new();
    for (k, curve) in curves.iter().enumerate() {
        let (a, b) = curve.domain();
        let width = b - a;
        if !(width > 0.0 && width.is_finite()) {
            return Err(format!("segment {k} has an empty parameter range"));
        }
        let base = k as f64;
        for ((p0, p1), points) in curve.bezier_pieces() {
            pieces.push(Piece {
                range: (base + (p0 - a) / width, base + (p1 - a) / width),
                points,
            });
        }
        if let Some(last) = pieces.last_mut() {
            last.range.1 = base + 1.0;
        }
    }
    joined(pieces, near)
}

/// Bézier pieces, in order, as one B-spline: raised to the highest
/// degree among them, end to end.
fn joined(mut pieces: Vec<Piece>, near: f64) -> Result<NurbsCurve, String> {
    let degree = pieces
        .iter()
        .map(|p| p.points.len().saturating_sub(1))
        .max()
        .unwrap_or(0);
    if degree == 0 || pieces.len().saturating_mul(degree).saturating_add(1) > MAX_JOINED {
        return Err(format!(
            "{} pieces of degree up to {degree}: no curve to join, or more than {MAX_JOINED} control points",
            pieces.len()
        ));
    }
    for piece in &mut pieces {
        while piece.points.len() < degree + 1 {
            piece.points = elevated(&piece.points);
        }
    }
    let first = pieces
        .first()
        .and_then(|p| p.points.first())
        .copied()
        .ok_or_else(|| "nothing to join".to_string())?;
    let start = pieces.first().map_or(0.0, |p| p.range.0);
    let mut knots = vec![start; degree + 1];
    let mut points = vec![first.0];
    let mut weights = vec![first.1];
    for (k, piece) in pieces.iter().enumerate() {
        let (Some(&(p, w)), Some(&last)) = (piece.points.first(), weights.last()) else {
            continue;
        };
        if k > 0 {
            let gap = points.last().map_or(f64::INFINITY, |q| q.distance(p));
            if !(gap <= near) {
                return Err(format!(
                    "piece {k} starts {gap:e} from where the one before ends, more than the file's uncertainty {near:e}"
                ));
            }
        }
        // The same curve, its weights scaled to meet the one before.
        let scale = last / w;
        if !(scale.is_finite() && scale > 0.0) {
            return Err("a weight is not positive and finite".to_string());
        }
        for &(q, v) in piece.points.iter().skip(1) {
            points.push(q);
            weights.push(v * scale);
        }
        let end = piece.range.1;
        let count = if k + 1 == pieces.len() {
            degree + 1
        } else {
            degree
        };
        knots.extend(std::iter::repeat_n(end, count));
    }
    NurbsCurve::new(degree, knots, points, weights).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_geom::{Circle, Ellipse};
    use craft3d_math::Frame;

    fn tilted() -> Frame {
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5),
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap()
    }

    #[test]
    fn parts_and_reversals_are_the_same_points() {
        let circle = Curve3::Circle(Circle::new(tilted(), 2.0).unwrap());
        let ellipse = Curve3::Ellipse(Ellipse::new(tilted(), 3.0, 1.2).unwrap());
        for curve in [&circle, &ellipse] {
            for (from, to, forward) in [
                (0.3, 2.0, true),
                (2.0, 0.3, true),
                (0.3, 2.0, false),
                (1.0, 1.0, true),
            ] {
                let part = part(curve, from, to, forward).unwrap();
                let (a, b) = part.domain();
                assert!(part.eval(a).distance(curve.eval(from)) < 1e-14);
                assert!(part.eval(b).distance(curve.eval(to)) < 1e-14);
                // Every point on the curve, and inside the sweep.
                for k in 0..=64 {
                    let p = part.eval(a + (b - a) * f64::from(k) / 64.0);
                    let back = curve.eval(curve.project(p).unwrap());
                    assert!(p.distance(back) < 1e-13, "{curve:?} {from} {to}");
                }
                let middle = part.eval(0.5 * (a + b));
                let t = curve.project(middle).unwrap();
                let sweep = if forward {
                    (to - from).rem_euclid(TAU)
                } else {
                    (from - to).rem_euclid(TAU)
                };
                let off = if forward {
                    (t - from).rem_euclid(TAU)
                } else {
                    (from - t).rem_euclid(TAU)
                };
                assert!(sweep == 0.0 || off < sweep, "{from} {to} {forward}: {t}");
            }
        }
        // A B-spline restricted and turned round.
        let nurbs = NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 0.4, 1.0, 1.0, 1.0, 1.0],
            vec![
                Vec3::ZERO,
                Vec3::new(1.0, 2.0, 0.0),
                Vec3::new(2.0, -1.0, 1.0),
                Vec3::new(3.0, 0.0, 2.0),
                Vec3::new(4.0, 1.0, 0.0),
            ],
            vec![1.0, 0.7, 1.3, 0.9, 1.0],
        )
        .unwrap();
        let curve = Curve3::Nurbs(nurbs.clone());
        let p = part(&curve, 0.1, 0.8, true).unwrap();
        for k in 0..=40 {
            let t = 0.1 + 0.7 * f64::from(k) / 40.0;
            assert!(p.eval(t).distance(nurbs.eval(t)) < 1e-13);
        }
        let back = part(&curve, 0.8, 0.1, false).unwrap();
        let (a, b) = back.domain();
        for k in 0..=40 {
            let s = f64::from(k) / 40.0;
            assert!(
                back.eval(a + (b - a) * s)
                    .distance(nurbs.eval(0.8 - 0.7 * s))
                    < 1e-13
            );
        }
        assert!(part(&curve, 0.1, 0.8, false).is_err());
        assert!(part(&curve, 0.5, 0.5, true).is_err());
        assert!(part(&curve, -0.5, 0.5, true).is_err());
        assert!(part(&curve, f64::NAN, 0.5, true).is_err());
        let line = Curve3::Line(Line::new(Vec3::ZERO, Vec3::X).unwrap());
        let seg = part(&line, 2.0, -1.0, false).unwrap();
        assert_eq!(seg.eval(seg.domain().0), Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(seg.eval(seg.domain().1), Vec3::new(-1.0, 0.0, 0.0));
        for curve in [&circle, &ellipse, &line, &curve] {
            let back = reversed(curve).unwrap();
            let (a, b) = match curve.domain() {
                (a, b) if a.is_finite() => (a, b),
                _ => (-3.0, 3.0),
            };
            for k in 0..=20 {
                let t = a + (b - a) * f64::from(k) / 20.0;
                let mirrored = match curve {
                    Curve3::Nurbs(_) => a + b - t,
                    _ => -t,
                };
                assert!(back.eval(mirrored).distance(curve.eval(t)) < 1e-13);
            }
        }
    }

    /// A line, a quarter circle and a rational cubic joined: the same
    /// points, one curve of degree three, a joint too far apart refused.
    #[test]
    fn curves_are_joined_exactly() {
        let line = NurbsCurve::polynomial(
            1,
            vec![0.0, 0.0, 2.0, 2.0],
            vec![Vec3::new(-2.0, 0.0, 0.0), Vec3::ZERO],
        )
        .unwrap();
        let arc = craft3d_geom::network::arc(
            &Frame::WORLD.at(Vec3::new(0.0, 1.0, 0.0)).unwrap(),
            1.0,
            -std::f64::consts::FRAC_PI_2,
            std::f64::consts::FRAC_PI_2,
        )
        .unwrap();
        let end = arc.eval(arc.domain().1);
        let cubic = NurbsCurve::new(
            3,
            vec![5.0, 5.0, 5.0, 5.0, 9.0, 9.0, 9.0, 9.0],
            vec![
                end,
                end + Vec3::new(0.0, 1.0, 0.0),
                end + Vec3::new(1.0, 2.0, 1.0),
                end + Vec3::new(2.0, 2.0, 0.0),
            ],
            vec![2.0, 1.0, 3.0, 0.5],
        )
        .unwrap();
        // The arc starts at cos(−π/2) = 6e-17, not at 0: rounding apart.
        assert!(join(&[line.clone(), arc.clone()], 0.0).is_err());
        let joined = join(&[line.clone(), arc.clone(), cubic.clone()], 1e-15).unwrap();
        assert_eq!(joined.degree(), 3);
        assert_eq!(joined.domain(), (0.0, 3.0));
        for (k, (curve, (a, b))) in [
            (&line, line.domain()),
            (&arc, arc.domain()),
            (&cubic, cubic.domain()),
        ]
        .into_iter()
        .enumerate()
        {
            for i in 0..=30 {
                let s = f64::from(i) / 30.0;
                let p = joined.eval(k as f64 + s);
                let q = curve.eval(a + (b - a) * s);
                assert!(p.distance(q) < 1e-13, "segment {k} at {s}: {p:?} {q:?}");
            }
        }
        let apart = NurbsCurve::polynomial(
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![Vec3::new(1e-3, 0.0, 0.0), Vec3::X],
        )
        .unwrap();
        let e = join(&[line.clone(), apart.clone()], 1e-6).unwrap_err();
        assert!(e.contains("piece 1 starts"), "{e}");
        assert!(join(&[line, apart], 1e-2).is_ok());
        assert!(join(&[], 1.0).is_err());
        // The radius of the joined arc is the circle's.
        let p = joined.eval(1.5);
        assert!((p.distance(Vec3::new(0.0, 1.0, 0.0)) - 1.0).abs() < 1e-14);
    }
}
