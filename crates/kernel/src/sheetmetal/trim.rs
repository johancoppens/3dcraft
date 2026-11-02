//! Trimming a bend by a closed corner's end plane (docs/design/sheetmetal.md
//! › *Trimming a bend*).
//!
//! A closed corner runs a flange's flat to a plane. Where that plane comes
//! nearer than the bend's end (walls that lean towards each other, a wide
//! corner without a relief large enough), it cuts the bend too. Each skin
//! of the bend is a cylinder of radius `r` about the axis; a point of it
//! at the angle `φ` from the parent's tangent line, `u` along the axis
//! from the bend's start, is
//!
//! `S + u d + r sin φ û + (z_c − σ r cos φ) ẑ`
//!
//! in the parent's pattern axes (`σ` = 1 for a bend up, −1 down; `z_c`
//! the axis's height), so the plane `n·(P − q) = 0` meets it at
//!
//! `u(φ) = a + b sin φ + c cos φ`, `a = (n·(q − S) − z_c n_z) / (n·d)`,
//! `b = −r (n·û) / (n·d)`, `c = σ r n_z / (n·d)`,
//!
//! refused where `n·d` is within the angular tolerance of 0 (the plane
//! along the bend). The strip's end in the flat pattern is that curve at
//! `s = φ (R + K T)` on the bottom skin. It is drawn as a cubic Hermite
//! spline in `s` whose error is bounded, not sampled: `u` is a sinusoid in
//! `s / ρ`, so `|u⁗(s)| ≤ hypot(b, c) / ρ⁴`, and the Hermite remainder on a
//! step `h` is at most `|u⁗| h⁴ / 384`; the steps are chosen so that this
//! is at most an eighth of the tolerance.
//!
//! A trimmed end is built when the plane cuts both skins over one run of
//! the bend that reaches its far tangent line, `[φ₁, θ]` on the bottom skin
//! and `[φ₁′, θ]` on the top, both strictly inside: the bend's end is then
//! its square end (the circle at `u_end`) up to `φ₁` and the plane's
//! ellipse beyond, and the line where the two ends meet runs from the one
//! skin to the other. Anything else (the plane reaching the parent's
//! tangent line, cutting one skin and not the other, cutting only a middle
//! run) is refused with the reason.

use super::{Bend, KernelError, P2, refuse};
use crate::profile::{Segment, Shape};
use crate::spline2::Spline2;
use craft3d_math::Portable;
use craft3d_math::{Tolerance, Transform, Vec3};

/// The most Hermite steps a trimmed strip's end is drawn with.
/// (A cubic B-spline of `n` steps with double inner knots has `2n + 2`
/// control points, which a profile spline may have at most 4096 of.)
pub const MAX_TRIM_STEPS: usize = 2047;

/// A bend's end cut by a closed corner's end plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trim {
    /// The plane, in the pattern's axes as folded: a point and its unit
    /// normal.
    pub point: Vec3,
    pub normal: Vec3,
    /// Where the bottom skin's end leaves the square end for the plane,
    /// and the same on the top skin (angles from the parent's tangent
    /// line, both strictly inside `(0, θ)`).
    pub phi: f64,
    pub phi_top: f64,
    /// The bottom skin's cut `u(φ) = a + b sin φ + c cos φ` along the
    /// tangent line from the bend's start.
    pub a: f64,
    pub b: f64,
    pub c: f64,
    /// The area of the strip the trim takes away in the pattern (exact).
    pub area: f64,
    /// The Hermite steps the strip's end is drawn with.
    pub steps: usize,
}

impl Trim {
    pub fn u(&self, phi: f64) -> f64 {
        let (s, c) = phi.psin_cos();
        self.a + self.b * s + self.c * c
    }

    fn du(&self, phi: f64) -> f64 {
        let (s, c) = phi.psin_cos();
        self.b * c - self.c * s
    }
}

/// The bend's axis height in its parent's pattern axes, and `σ`.
fn axis(bend: &Bend) -> (f64, f64) {
    if bend.angle > 0.0 {
        (bend.thickness + bend.radius, 1.0)
    } else {
        (-bend.radius, -1.0)
    }
}

/// The bottom skin's radius and the top skin's.
pub fn radii(bend: &Bend) -> (f64, f64) {
    if bend.angle > 0.0 {
        (bend.radius + bend.thickness, bend.radius)
    } else {
        (bend.radius, bend.radius + bend.thickness)
    }
}

fn lift(p: P2) -> Vec3 {
    Vec3::new(p[0], p[1], 0.0)
}

/// `(a, b, c)` of the plane's cut of the skin of radius `r`, in the
/// parent's pattern axes.
fn sinusoid(
    i: usize,
    bend: &Bend,
    parent: &Transform,
    plane: (Vec3, Vec3),
    r: f64,
    tolerance: &Tolerance,
) -> Result<(f64, f64, f64), KernelError> {
    let back = parent
        .inverse()
        .ok_or_else(|| refuse(Some(i), "a flat's motion cannot be undone"))?;
    let (q, n) = (back.point(plane.0), back.vector(plane.1));
    let d = super::unit(super::sub(bend.end, bend.start))
        .ok_or_else(|| refuse(Some(i), "a bend has no length"))?;
    let den = n.dot(lift(d));
    if !(den.abs() > n.length() * tolerance.angular) {
        return Err(refuse(
            Some(i),
            "a closed corner's end plane runs along the bend, so it cannot trim it",
        ));
    }
    let (zc, sigma) = axis(bend);
    let s = lift(bend.start);
    let a = (n.dot(q - s) - zc * n.z) / den;
    let b = -r * n.dot(lift(bend.across)) / den;
    let c = sigma * r * n.z / den;
    if !(a.is_finite() && b.is_finite() && c.is_finite()) {
        return Err(refuse(Some(i), "a closed corner's end plane is not finite"));
    }
    Ok((a, b, c))
}

/// The angles in `(0, θ)` where `a + b sin φ + c cos φ = level`, sorted:
/// `b sin φ + c cos φ = m sin(φ + ψ)` with `m = hypot(b, c)` and
/// `ψ = atan2(c, b)`.
fn roots(abc: (f64, f64, f64), level: f64, theta: f64) -> Vec<f64> {
    let (a, b, c) = abc;
    let m = b.phypot(c);
    if !(m > 0.0) {
        return Vec::new();
    }
    let x = (level - a) / m;
    if !(x.abs() <= 1.0) {
        return Vec::new();
    }
    let psi = c.patan2(b);
    let base = x.pasin();
    let mut out: Vec<f64> = Vec::new();
    for k in -2..=2 {
        let turn = f64::from(k) * std::f64::consts::TAU;
        for phi in [base - psi + turn, std::f64::consts::PI - base - psi + turn] {
            if phi > 0.0 && phi < theta && !out.iter().any(|o| (o - phi).abs() <= 1e-15) {
                out.push(phi);
            }
        }
    }
    out.sort_by(f64::total_cmp);
    out
}

/// The trim of bend `i`'s end `end` (0 its start, 1 its end) by `plane`,
/// or `None` when the plane does not cut the bottom skin at its far
/// tangent line (it comes no nearer than the bend's end there: the flat is
/// extended or trimmed alone). Refused, naming the flange, where the cut
/// is one this method does not build (module documentation).
pub fn trim(
    i: usize,
    bend: &Bend,
    parent: &Transform,
    end: usize,
    plane: (Vec3, Vec3),
    tolerance: &Tolerance,
) -> Result<Option<Trim>, KernelError> {
    let tol = tolerance.linear;
    let theta = bend.theta();
    let len = super::distance(bend.start, bend.end);
    let u_end = if end == 0 { 0.0 } else { len };
    // Positive where the plane takes material away from the end.
    let sign = if end == 0 { 1.0 } else { -1.0 };
    let (rb, rt) = radii(bend);
    let bottom = sinusoid(i, bend, parent, plane, rb, tolerance)?;
    let top = sinusoid(i, bend, parent, plane, rt, tolerance)?;
    let cut = |abc: (f64, f64, f64), phi: f64| {
        let (s, c) = phi.psin_cos();
        sign * (abc.0 + abc.1 * s + abc.2 * c - u_end)
    };
    if !(cut(bottom, theta) > tol) {
        return Ok(None);
    }
    let which = if end == 0 { "start" } else { "end" };
    let advice = "give the corner a relief (or a larger one), mitre it, or open it";
    let floor = super::SLIVER_TOLERANCES * tol;
    let mut found = [0.0f64; 2];
    for (k, (abc, r, skin)) in [(bottom, rb, "bottom"), (top, rt, "top")]
        .into_iter()
        .enumerate()
    {
        if !(cut(abc, 0.0) < -tol) {
            return Err(refuse(
                Some(i),
                format!(
                    "the closed corner at its bend's {which} has its end plane reach the parent's tangent line on the {skin} skin, which would cut the parent flat too (not supported, owed): {advice}"
                ),
            ));
        }
        if !(cut(abc, theta) > tol) {
            return Err(refuse(
                Some(i),
                format!(
                    "the closed corner at its bend's {which} cuts the bend's bottom skin but not its top skin at the flange's tangent line, a step the sheet cannot make (not supported): {advice}"
                ),
            ));
        }
        let rs = roots(abc, u_end, theta);
        let [phi] = rs.as_slice() else {
            return Err(refuse(
                Some(i),
                format!(
                    "the closed corner at its bend's {which} cuts the bend's {skin} skin in more than one run (not supported): {advice}"
                ),
            ));
        };
        if !(r * phi >= floor && r * (theta - phi) >= floor) {
            return Err(refuse(
                Some(i),
                format!(
                    "the closed corner at its bend's {which} cuts the bend's {skin} skin within {floor} ({} tolerances) of its tangent lines, a sliver: {advice}",
                    super::SLIVER_TOLERANCES
                ),
            ));
        }
        if let Some(slot) = found.get_mut(k) {
            *slot = *phi;
        }
    }
    let [phi, phi_top] = found;
    // Both skins are cut over one run reaching the far tangent line, so
    // the line where the square end meets the plane runs from the one
    // skin to the other: the plate's corner there.
    let rho = bend.neutral();
    if !(rho > 0.0) {
        return Err(refuse(Some(i), "a bend's neutral radius is not positive"));
    }
    let (a, b, c) = bottom;
    // The area taken away: ∫ sign (u − u_end) ds over s = ρ φ in
    // [ρ φ₁, ρ θ].
    let (s1, c1) = phi.psin_cos();
    let (st, ct) = theta.psin_cos();
    let area =
        sign * ((a - u_end) * rho * (theta - phi) + b * rho * (c1 - ct) + c * rho * (st - s1));
    // The Hermite steps: |u⁗| h⁴ / 384 ≤ tol / 8 with |u⁗| ≤ hypot(b, c) / ρ⁴.
    let m4 = b.phypot(c) / (rho * rho * rho * rho);
    let width = rho * (theta - phi);
    // Nudged down by a relative 1e-12, more than the few roundings of the
    // two square roots and the quotient, so the step taken is never above
    // the one the bound allows.
    let h_max = if m4 > 0.0 {
        (384.0 * 0.125 * tol / m4).sqrt().sqrt() * (1.0 - 1e-12)
    } else {
        width
    };
    let steps = (width / h_max).ceil().max(1.0);
    if !(steps.is_finite() && steps <= MAX_TRIM_STEPS as f64) {
        return Err(refuse(
            Some(i),
            format!(
                "the closed corner's trim of its bend's {which} needs more than {MAX_TRIM_STEPS} spline steps to hold the tolerance"
            ),
        ));
    }
    // Rounding in evaluating the sinusoid and its spline must stay well
    // inside the other eighth of the tolerance: each value is a few
    // products and sums of numbers no larger than `scale` (the sinusoid's
    // coefficients, the strip's width, the bend's length), with Portable's
    // sine and cosine within a few ulps, so its error is below some tens
    // of ulps of `scale`, about 1e-14 of it; 1e-13 of it holds that with a
    // margin, and must be at most an eighth of the tolerance (else the
    // part is too large for its tolerance and the trim is refused).
    let scale = a.abs() + b.abs() + c.abs() + width + len;
    if !(scale * 1e-13 <= 0.125 * tol) {
        return Err(refuse(
            Some(i),
            "the closed corner's trim of its bend is too large for the tolerance to hold it",
        ));
    }
    Ok(Some(Trim {
        point: plane.0,
        normal: plane.1,
        phi,
        phi_top,
        a,
        b,
        c,
        area,
        steps: steps as usize,
    }))
}

/// The deepest cut a closed corner's end plane makes into bend `i`'s end
/// `end` that the bend is not trimmed for. On each skin the plane reaches
/// past the bend's square end by `cut(φ) = sign (u(φ) − u_end)`, a
/// sinusoid in `φ`, so its largest value over `(0, θ]` is at `θ` or at a
/// stationary angle `φ* = atan2(b, c) + kπ`, in closed form. At the
/// parent's tangent line (`φ = 0`) the bend's end is the corner's own:
/// two bends that reach a corner meet there at its point, as at an open
/// corner, and when `root_allowed` (the caller's: a mitre whose halving
/// plane passes through the corner's point on both skins, within the
/// tolerance) the root's cut is the half gap the plane is shifted by, so a
/// cut no deeper than it keeps the bend on its own side of the halving
/// plane, which the other wall does not cross: a cut counts where it goes
/// deeper than at the root by more than the tolerance, `cut(φ) >
/// max(cut(0), 0) + tol` for some `φ` in `(0, θ]`. Otherwise (overlap,
/// butt, a mitre of walls unlike each other, whose halving plane misses
/// the corner's point) nothing proves the other bend leaves room, and
/// every cut counts: `cut(φ) > tol` on `[0, θ]`. `None` when neither skin is cut so;
/// else the skin, the angle and the depth. A plane that cuts the top skin
/// only (a bend down whose outside is cut), or the middle of either skin
/// while its ends stay clear, makes no trim this method builds (module
/// documentation), and leaving the bend square there would make a part
/// that runs through itself, so the caller refuses it.
pub fn untrimmed_cut(
    i: usize,
    bend: &Bend,
    parent: &Transform,
    end: usize,
    plane: (Vec3, Vec3),
    root_allowed: bool,
    tolerance: &Tolerance,
) -> Result<Option<(&'static str, f64, f64)>, KernelError> {
    let theta = bend.theta();
    let len = super::distance(bend.start, bend.end);
    let u_end = if end == 0 { 0.0 } else { len };
    let sign = if end == 0 { 1.0 } else { -1.0 };
    let (rb, rt) = radii(bend);
    let mut deepest: Option<(&'static str, f64, f64)> = None;
    for (r, skin) in [(rb, "bottom"), (rt, "top")] {
        let (a, b, c) = sinusoid(i, bend, parent, plane, r, tolerance)?;
        let cut = |phi: f64| {
            let (s, co) = phi.psin_cos();
            sign * (a + b * s + c * co - u_end)
        };
        let star = b.patan2(c);
        let floor = if root_allowed {
            cut(0.0).max(0.0) + tolerance.linear
        } else {
            tolerance.linear
        };
        let mut angles = if root_allowed {
            vec![theta]
        } else {
            vec![0.0, theta]
        };
        for k in -2..=2 {
            let phi = star + f64::from(k) * std::f64::consts::PI;
            if phi > 0.0 && phi < theta {
                angles.push(phi);
            }
        }
        for phi in angles {
            let depth = cut(phi);
            if !depth.is_finite() {
                return Err(refuse(Some(i), "a closed corner's end plane is not finite"));
            }
            if depth > floor && deepest.is_none_or(|(_, _, d)| depth > d) {
                deepest = Some((skin, phi, depth));
            }
        }
    }
    Ok(deepest)
}

/// The strip's end at `end` in the pattern, in the outline's order (from
/// the parent's tangent line to the flange's at the start, back at the
/// end): the square piece up to `φ₁`, then the plane's cut as a cubic
/// spline through Hermite steps (its error bounded by the module's
/// rule). Without a trim, the line across the strip.
pub fn strip_end(bend: &Bend, end: usize) -> Vec<Segment> {
    let d = super::unit(super::sub(bend.end, bend.start)).unwrap_or([1.0, 0.0]);
    let len = super::distance(bend.start, bend.end);
    let u_end = if end == 0 { 0.0 } else { len };
    let rho = bend.neutral();
    let ba = bend.allowance;
    let at = |u: f64, s: f64| -> P2 {
        [
            bend.start[0] + u * d[0] + s * bend.across[0],
            bend.start[1] + u * d[1] + s * bend.across[1],
        ]
    };
    let far = |p: P2| [p[0] + ba * bend.across[0], p[1] + ba * bend.across[1]];
    let corner = if end == 0 { bend.start } else { bend.end };
    let Some(t) = bend.trim.get(end).copied().flatten() else {
        let mut seg = super::line(corner, far(corner));
        if end == 1 {
            seg = super::line(far(corner), corner);
        }
        return vec![seg];
    };
    let s1 = rho * t.phi;
    let knee = at(u_end, s1);
    let n = t.steps.max(1);
    let h = (ba - s1) / n as f64;
    // Per station: the point and dP/ds.
    let station = |k: usize| -> (P2, P2) {
        let s = if k == n { ba } else { s1 + h * k as f64 };
        let phi = if k == n { bend.theta() } else { s / rho };
        let u = if k == 0 { u_end } else { t.u(phi) };
        let du = t.du(phi) / rho;
        (
            at(u, s),
            [du * d[0] + bend.across[0], du * d[1] + bend.across[1]],
        )
    };
    // Hermite pieces as Bézier pieces `[P, P + h v / 3, Q − h w / 3, Q]`;
    // with equal steps each inner joint is the middle of its two inner
    // control points, so the C¹ spline has double inner knots and leaves
    // the joints out.
    let mut control: Vec<P2> = Vec::with_capacity(2 * n + 2);
    let mut knots: Vec<f64> = vec![s1; 4];
    let (p0, mut v0) = station(0);
    control.push(p0);
    let mut last = p0;
    for k in 1..=n {
        let (p1, v1) = station(k);
        let third = h / 3.0;
        control.push([last[0] + third * v0[0], last[1] + third * v0[1]]);
        control.push([p1[0] - third * v1[0], p1[1] - third * v1[1]]);
        if k < n {
            knots.extend([s1 + h * k as f64; 2]);
        }
        (last, v0) = (p1, v1);
    }
    control.push(last);
    knots.extend([ba; 4]);
    let tip = control.last().copied().unwrap_or(knee);
    let spline = Segment {
        source: 0,
        piece: 0,
        reversed: end == 1,
        start: if end == 0 { knee } else { tip },
        end: if end == 0 { tip } else { knee },
        shape: Shape::Spline(Spline2 {
            degree: 3,
            knots,
            control,
        }),
    };
    if end == 0 {
        vec![super::line(corner, knee), spline]
    } else {
        vec![spline, super::line(knee, corner)]
    }
}

/// The point where the trimmed end meets the flange's tangent line, in
/// the pattern (`u(θ)` on the far line).
pub fn far_point(bend: &Bend, t: &Trim) -> P2 {
    let d = super::unit(super::sub(bend.end, bend.start)).unwrap_or([1.0, 0.0]);
    let u = t.u(bend.theta());
    [
        bend.start[0] + u * d[0] + bend.allowance * bend.across[0],
        bend.start[1] + u * d[1] + bend.allowance * bend.across[1],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hem (180 degrees, up) from `(0, 0)` to `(10, 0)`, `R = T = 1`,
    /// its flange towards `−y`, cut at its end by the plane through
    /// `(10 + δ, 0, 0)` with the normal `(1, −k, 0)`: `u(φ) = 10 + δ − k
    /// r sin φ` on the skin of radius `r`, so both ends of the bend stay
    /// clear by `δ` and the middle is cut by `k r − δ` at 90 degrees, the
    /// bottom skin (`r = 2`) deepest. Square to the bend (`k = 0`) nothing
    /// is cut.
    #[test]
    fn a_cut_through_the_middle_of_a_bend_only_is_found() {
        for scale in [1e-3, 1.0, 1e6] {
            let tol = Tolerance::new(1e-6 * scale, 1e-9).unwrap();
            let bend = Bend {
                parent: 0,
                child: 1,
                angle: std::f64::consts::PI,
                radius: scale,
                k: 0.4,
                thickness: scale,
                allowance: std::f64::consts::PI * 1.4 * scale,
                start: [0.0, 0.0],
                end: [10.0 * scale, 0.0],
                across: [0.0, -1.0],
                flat_length: 5.0 * scale,
                trim: [None, None],
            };
            let delta = 0.2 * scale;
            let plane = |k: f64| {
                (
                    Vec3::new(10.0 * scale + delta, 0.0, 0.0),
                    Vec3::new(1.0, -k, 0.0).normalized().unwrap(),
                )
            };
            let id = Transform::IDENTITY;
            let (skin, phi, depth) = untrimmed_cut(0, &bend, &id, 1, plane(0.5), true, &tol)
                .unwrap()
                .expect("the middle of the bend is cut");
            assert_eq!(skin, "bottom");
            assert!((phi - std::f64::consts::FRAC_PI_2).abs() < 1e-12, "{phi}");
            // u_end − u = k r sin φ − δ, the plane meeting the skin at
            // u = a + b sin φ with b = −r k (the normal's slant), so
            // the depth at 90 degrees is k r − δ.
            let expected = 0.5 * 2.0 * scale - delta;
            assert!(
                (depth - expected).abs() <= 1e-12 * scale,
                "{depth} {expected}"
            );
            // Neither end of the bend is cut: trim() makes no trim here.
            assert_eq!(trim(0, &bend, &id, 1, plane(0.5), &tol).unwrap(), None);
            assert_eq!(
                untrimmed_cut(0, &bend, &id, 1, plane(0.0), true, &tol).unwrap(),
                None
            );
        }
    }
}
