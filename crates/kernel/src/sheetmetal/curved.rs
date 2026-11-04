//! Curved bends (docs/design/sheetmetal.md › *Curved bends*).
//!
//! A **curved flange** is bent along an arc (or a whole circle) of the
//! base's outline or of a hole of it. Every section of the part square to
//! the edge (in the half-plane through the arc's axis, the vertical line
//! through its centre) is the section of a straight bend with the same
//! radius `R`, K-factor `K`, thickness `T`, angle `θ` and flat length:
//! the part is that section turned about the axis over the arc's sweep.
//! So the bend's skins are pieces of tori (major radius the edge's radius
//! `a`, minor `R` and `R + T`) and the flange's skins pieces of cones
//! (a cylinder at 90 degrees, a plane at 180): the volume is Pappus's.
//!
//! **The flat pattern is approximated.** A torus is not developable, so
//! no flat pattern keeps every length. The development used is the radial
//! one, as in practice: along each radial line the section is laid out as
//! a straight bend is (the strip `BA = θ (R + K T)` wide, then the flat
//! `L` long), so the strip and the flat are annular sectors about the
//! arc's centre, and the length across the bend (the neutral fibre) is
//! kept exactly. Along the edge it is not: a material point at the
//! pattern's radius `a + ε s` (`ε` = +1 when the flange grows away from
//! the centre, −1 towards it) lies at the radius `a + ε x(s, r)` in the
//! part (`x` its distance across the edge in the section, `r` its distance
//! from the bend's axis, in `[R, R + T]`), so it is stretched along the
//! edge by the strain `e = ε (x − s) / (a + ε s)`. That strain is the
//! approximation's, and its **bound is proven** (never sampled): `e` is
//! affine in `r`, so its extremes lie at `r = R` or `R + T`; on the flat
//! (`x = r sin θ + d cos θ`, `s = BA + d`) it is a ratio of two affine
//! functions of `d`, monotone, so its extremes lie at the flat's two ends;
//! on the bend (`x = r sin φ`, `s = (R + K T) φ`) it is bounded by interval
//! arithmetic on halved intervals of `φ` (branch and bound) until the
//! bound is within a millionth of the largest value found.

#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::{
    Bend, Cut, Direction, EdgeKey, Flat, KernelError, LengthFrom, P2, SLIVER_TOLERANCES, Segment,
    Shape, SheetFace, outline_key, pattern_area, refuse, sketch_curve, tagged_outline,
};
use craft3d_geom::{Circle, Cone, Curve3, Cylinder, Line, Plane, Surface, Torus};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, Coedge, EdgeId, FaceId, VertexId};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// A curved flange as the caller gives it, in the pattern's axes.
#[derive(Clone, Debug, PartialEq)]
pub struct CurvedSpec {
    /// The sketch curve (an arc or a circle) of the base's outline or of
    /// a hole of it the flange is bent from.
    pub edge: u64,
    pub length: f64,
    /// θ in radians, in (0, π].
    pub angle: f64,
    pub direction: Direction,
    pub radius: f64,
    pub k: f64,
    pub from: LengthFrom,
    /// What its names are numbered by (from the flanges' ids).
    pub id: u64,
}

/// The proven bounds of the strain along the edge a curved flange's
/// development implies: every material point's strain lies in
/// `[least, most]` (`least ≤ 0 ≤ most`; negative is compression).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strain {
    pub least: f64,
    pub most: f64,
}

impl Strain {
    /// The largest stretch or compression, as a size.
    pub fn largest(&self) -> f64 {
        self.most.max(-self.least)
    }
}

/// A curved flange as built.
#[derive(Clone, Debug, PartialEq)]
pub struct Curved {
    pub id: u64,
    /// The sketch curve it is bent from, and where that is: the base's
    /// outer loop (`None`) or hole `h`, and the segment's place in it.
    pub edge: u64,
    pub hole: Option<usize>,
    pub seg: usize,
    pub center: P2,
    /// The edge's radius `a`.
    pub edge_radius: f64,
    /// +1 when the flange grows away from the centre (a shrink flange:
    /// the material is compressed along the edge), −1 towards it (a
    /// stretch flange).
    pub grows: f64,
    /// The polar angle (about `center`, from the pattern's x axis) where
    /// the arc starts, and its counter-clockwise sweep (2π for a circle).
    pub begin: f64,
    pub sweep: f64,
    /// Signed: positive up.
    pub angle: f64,
    pub radius: f64,
    pub k: f64,
    pub thickness: f64,
    /// `θ (R + K T)`, the strip's width along every radial line.
    pub allowance: f64,
    /// The flange's flat beyond the strip.
    pub flat_length: f64,
    pub strain: Strain,
    /// Per end of the arc in its own direction (its start, its end), the
    /// straight flange on the neighbouring edge that it continues, when
    /// the edges meet tangent there and the two flanges are alike (angle,
    /// direction, radius, K-factor, flat length): the bend of that flange.
    /// Joined, the two are one wall: no side between them in the part or
    /// in the flat pattern.
    pub joined: [Option<usize>; 2],
}

impl Curved {
    pub fn theta(&self) -> f64 {
        self.angle.abs()
    }

    /// The radius of the neutral fibre, `R + K T`.
    pub fn neutral(&self) -> f64 {
        self.radius + self.k * self.thickness
    }

    /// Whether it runs round a whole circle.
    pub fn whole(&self) -> bool {
        self.sweep >= TAU
    }

    /// The radius in the flat pattern at `s` across the edge.
    pub fn pattern_radius(&self, s: f64) -> f64 {
        self.edge_radius + self.grows * s
    }

    /// The tip's radius in the flat pattern.
    pub fn tip_radius(&self) -> f64 {
        self.pattern_radius(self.allowance + self.flat_length)
    }

    /// Its area in the flat pattern: the strip's and the flat's annular
    /// sectors.
    pub fn area(&self) -> f64 {
        let (a, b) = (self.edge_radius, self.tip_radius());
        0.5 * self.sweep * (b * b - a * a).abs()
    }

    /// Its volume in the part, by Pappus: the section (the bend's annular
    /// sector between `R` and `R + T` over θ, then the flat `T` thick and
    /// `L` long) turned about the arc's axis over the sweep, `Δ ∫ ρ dA`.
    pub fn volume(&self) -> f64 {
        section_moment(
            self.edge_radius,
            self.grows,
            self.theta(),
            self.radius,
            self.thickness,
            self.flat_length,
        ) * self.sweep
    }

    /// The bend line in the flat pattern: the arc across the middle of the
    /// strip, `(centre, radius, begin, sweep)`.
    pub fn bend_line(&self) -> (P2, f64, f64, f64) {
        (
            self.center,
            self.pattern_radius(0.5 * self.allowance),
            self.begin,
            self.sweep,
        )
    }
}

/// `∫ ρ dA` over a curved flange's section (`ρ` the distance from the
/// arc's axis): `a A + ε ∫ x dA` with `A = θ T (R + T/2) + L T` and
/// `∫ x dA = (1 − cos θ)((R + T)³ − R³)/3 + sin θ L ((R + T)² − R²)/2 +
/// cos θ T L²/2` (the same for a bend up and down: the section is the
/// straight bend's mirrored).
pub fn section_moment(a: f64, grows: f64, theta: f64, r: f64, t: f64, l: f64) -> f64 {
    let (s, c) = theta.psin_cos();
    let outer = r + t;
    let area = theta * t * (r + 0.5 * t) + l * t;
    let mx = (1.0 - c) * (outer * outer * outer - r * r * r) / 3.0
        + s * l * (outer * outer - r * r) / 2.0
        + c * t * l * l / 2.0;
    a * area + grows * mx
}

/// The strain along the edge at the pattern's distance `s` across it
/// (`s = (R + K T) φ` on the bend, `BA + d` on the flat) and the distance
/// `r` from the bend's axis, as a closed form: what the bound bounds.
pub fn strain_at(
    a: f64,
    grows: f64,
    theta: f64,
    neutral: f64,
    flat_length: f64,
    s: f64,
    r: f64,
) -> f64 {
    let ba = theta * neutral;
    let x = if s <= ba {
        r * (s / neutral).psin()
    } else {
        let d = (s - ba).min(flat_length);
        let (sn, cs) = theta.psin_cos();
        r * sn + d * cs
    };
    grows * (x - s) / (a + grows * s)
}

/// The most branch-and-bound steps one bound may take; past them the
/// bound is the largest of the intervals left (still proven, only looser).
const MAX_STEPS: usize = 2048;

/// The proven bounds of the strain along the edge over the whole flange
/// (bend and flat, every depth): `None` where the pattern's radius is not
/// positive over it (refused before).
pub fn strain_bound(
    a: f64,
    grows: f64,
    theta: f64,
    radius: f64,
    k: f64,
    t: f64,
    flat_length: f64,
) -> Option<Strain> {
    let rn = radius + k * t;
    let ba = theta * rn;
    let ends = [a, a + grows * (ba + flat_length)];
    if !(ends.iter().all(|x| x.is_finite() && *x > 0.0) && rn > 0.0 && theta > 0.0) {
        return None;
    }
    let mut most = 0.0f64;
    let mut least = 0.0f64;
    let (sn, cs) = theta.psin_cos();
    for r in [radius, radius + t] {
        most = most.max(bend_extreme(a, grows, r, rn, theta, 1.0));
        least = least.min(-bend_extreme(a, grows, r, rn, theta, -1.0));
        // The flat: (r sin θ − BA − d (1 − cos θ)) / (a + ε (BA + d)) is a
        // ratio of affine functions of d, monotone: its ends.
        for d in [0.0, flat_length] {
            let e = grows * (r * sn - ba - d * (1.0 - cs)) / (a + grows * (ba + d));
            most = most.max(e);
            least = least.min(e);
        }
    }
    // The rounding of the evaluations (relative 1e-16 of the numbers'
    // size, which the strains are ratios of) is held by a margin.
    let margin = 1e-12;
    Some(Strain {
        least: least - margin * (1.0 + least.abs()),
        most: most + margin * (1.0 + most.abs()),
    })
}

/// A proven upper bound of `sign · h(φ)` over `[0, θ]`, `h(φ) = ε (r sin φ
/// − ρ φ) / (a + ε ρ φ)`: branch and bound. On each interval `[p, q]` the
/// bound is the lesser of two enclosures: the interval one (the numerator's
/// and the denominator's ranges), and the second-order one about the middle
/// `m`, `h(m) + |h'(m)| w/2 + M₂ w²/8` (`w = q − p`), with `M₂` a bound of
/// `|h''|` over the interval from `h'' = N''/D − 2 N' D'/D² + 2 N D'²/D³`
/// (`N = r sin φ − ρ φ`, `D = a + ε ρ φ`, `D' = ε ρ`, each by its range).
fn bend_extreme(a: f64, grows: f64, r: f64, rn: f64, theta: f64, sign: f64) -> f64 {
    let h = |phi: f64| sign * grows * (r * phi.psin() - rn * phi) / (a + grows * rn * phi);
    // An upper bound of sign · h over [p, q] (0 ≤ p < q ≤ π).
    let upper = |p: f64, q: f64| -> f64 {
        let (sp, sq) = (p.psin(), q.psin());
        let slack = 1e-15;
        let s_lo = sp.min(sq) - slack;
        let s_hi = if p <= FRAC_PI_2 && FRAC_PI_2 <= q {
            1.0
        } else {
            sp.max(sq) + slack
        };
        // The numerator's range, times sign · ε: its top end.
        let n_hi = if grows * sign < 0.0 {
            -(r * s_lo - rn * q)
        } else {
            r * s_hi - rn * p
        };
        let (d1, d2) = (a + grows * rn * p, a + grows * rn * q);
        let interval = (n_hi / d1).max(n_hi / d2);
        // The second-order enclosure about the middle.
        let d_lo = d1.min(d2);
        let n_abs = (r * s_lo - rn * q).abs().max((r * s_hi - rn * p).abs());
        let (cp, cq) = (p.pcos(), q.pcos());
        let n1_abs = (r * cp - rn).abs().max((r * cq - rn).abs()) + slack * r;
        let n2_abs = r * s_hi.max(0.0);
        let m2 = n2_abs / d_lo
            + 2.0 * n1_abs * rn / (d_lo * d_lo)
            + 2.0 * n_abs * rn * rn / (d_lo * d_lo * d_lo);
        let m = 0.5 * (p + q);
        let (sm, cm) = m.psin_cos();
        let (n, n1, d) = (r * sm - rn * m, r * cm - rn, a + grows * rn * m);
        let slope = (n1 * d - n * grows * rn) / (d * d);
        let w = q - p;
        let taylor = h(m) + slope.abs() * 0.5 * w + m2 * w * w / 8.0;
        interval.min(taylor)
    };
    let mut best = h(0.0).max(h(theta)).max(h(0.5 * theta));
    let mut open: Vec<(f64, f64, f64)> = vec![(0.0, theta, upper(0.0, theta))];
    for _ in 0..MAX_STEPS {
        let Some((at, &(p, q, ub))) = open
            .iter()
            .enumerate()
            .max_by(|x, y| x.1.2.total_cmp(&y.1.2))
        else {
            break;
        };
        if ub <= best + 1e-9 * best.abs() + 1e-14 {
            break;
        }
        open.swap_remove(at);
        let m = 0.5 * (p + q);
        best = best.max(h(m));
        open.push((p, m, upper(p, m)));
        open.push((m, q, upper(m, q)));
    }
    open.iter().map(|x| x.2).fold(best, f64::max)
}

/// Finds and checks the curved flanges on the base, before anything of
/// them is built.
pub(super) fn lay_out(
    flats: &[Flat],
    bends: &[Bend],
    specs: &[CurvedSpec],
    max_strain: Option<f64>,
    t: f64,
    tolerance: &Tolerance,
) -> Result<Vec<Curved>, KernelError> {
    let tol = tolerance.linear;
    let base = flats.first().ok_or_else(|| refuse(None, "no base"))?;
    let mut out: Vec<Curved> = Vec::with_capacity(specs.len());
    if let Some(m) = max_strain
        && !(m.is_finite() && m > 0.0)
    {
        return Err(refuse(
            None,
            format!("maxStrain is a strain above 0 (0.05 is 5 per cent), got {m}"),
        ));
    }
    for (c, f) in specs.iter().enumerate() {
        let no = |reason: String| refuse(None, format!("curvedFlanges.{c}: {reason}"));
        if !(f.length.is_finite() && f.length > tol) {
            return Err(no(format!(
                "a flange's length has to be more than the tolerance, got {}",
                f.length
            )));
        }
        if !(f.angle.is_finite() && f.angle > 0.0 && f.angle <= PI) {
            return Err(no(format!(
                "a bend's angle is more than 0 and at most 180 degrees, got {}",
                f.angle.to_degrees()
            )));
        }
        if !(f.k.is_finite() && (0.0..=1.0).contains(&f.k)) {
            return Err(no(format!(
                "a K-factor is the neutral fibre's place across the thickness, from 0 to 1, got {}",
                f.k
            )));
        }
        if !(f.radius.is_finite() && f.radius > tol) {
            return Err(no(format!(
                "a curved bend needs an inner radius above the tolerance, got {} (a sharp curved bend's inner skin would be the torus's tube shrunk to a circle: not built)",
                f.radius
            )));
        }
        if specs.iter().take(c).any(|x| x.edge == f.edge) {
            return Err(no(format!(
                "the curve {} carries another curved flange; an edge carries one",
                f.edge
            )));
        }
        // The edge: one arc or circle of the base's outer loop or a hole.
        let mut found: Vec<(Option<usize>, usize, &Segment)> = Vec::new();
        for (k, s) in base.outer.iter().enumerate() {
            if s.source == f.edge {
                found.push((None, k, s));
            }
        }
        for (h, hole) in base.holes.iter().enumerate() {
            for (k, s) in hole.iter().enumerate() {
                if s.source == f.edge {
                    found.push((Some(h), k, s));
                }
            }
        }
        let (hole, seg_at, seg) = match found.as_slice() {
            [] => {
                return Err(no(format!(
                    "the curve {} is no edge of the base's outline or of a hole of it (a curved flange is bent from the base)",
                    f.edge
                )));
            }
            [one] => *one,
            _ => {
                return Err(no(format!(
                    "the curve {} is cut into several edges of the base (a flange or a relief splits it); a curved flange needs one whole arc",
                    f.edge
                )));
            }
        };
        let (center, a) = match seg.shape {
            Shape::Arc { center, radius } | Shape::Circle { center, radius } => (center, radius),
            Shape::Line => {
                return Err(no(format!(
                    "the curve {} is a straight line: a flange on it is one of `flanges`",
                    f.edge
                )));
            }
            Shape::Spline(_) => {
                return Err(no(format!(
                    "the curve {} is a spline; a curved flange is bent from an arc or a circle (along a spline the bend would be a canal surface, which has no exact surface here: not built)",
                    f.edge
                )));
            }
        };
        let (begin, sweep) = seg
            .angles()
            .ok_or_else(|| no("the arc has no angles".to_string()))?;
        if !(sweep > 0.0 && sweep <= TAU) {
            return Err(no("the arc's ends coincide".to_string()));
        }
        let grows = if seg.reversed { -1.0 } else { 1.0 };
        let theta = f.angle;
        let rn = f.radius + f.k * t;
        let ba = theta * rn;
        let flat_length = match f.from {
            LengthFrom::Tangent => f.length,
            LengthFrom::Mould => {
                let half = 0.5 * theta;
                if !(half < FRAC_PI_2 - tolerance.angular) {
                    return Err(no(
                        "at 180 degrees the outside surfaces never meet: measure the length from the tangent line"
                            .to_string(),
                    ));
                }
                f.length - (f.radius + t) * half.ptan()
            }
        };
        if !(flat_length > tol) {
            return Err(no(format!(
                "the flange is not longer than its setback (its flat would be {flat_length} long)"
            )));
        }
        let floor = SLIVER_TOLERANCES * tol;
        if !(a - (f.radius + t) > floor) {
            return Err(no(format!(
                "the edge's radius {a} is not more than the bend's outer radius {} (radius + thickness) by {SLIVER_TOLERANCES} tolerances: the bend would reach the axis of its own arc",
                f.radius + t
            )));
        }
        let tip = a + grows * (ba + flat_length);
        if !(tip > floor) {
            return Err(no(format!(
                "the flange's flat reaches the centre of its edge's arc in the flat pattern (its inner edge would be at radius {tip}): the edge's radius {a} is less than the bend allowance {ba} and the flat {flat_length}; shorten the flange"
            )));
        }
        let strain = strain_bound(a, grows, theta, f.radius, f.k, t, flat_length)
            .ok_or_else(|| no("its strain could not be bounded".to_string()))?;
        // In the part every point lies at (1 + e) times its pattern radius
        // from the axis: it must stay clear of the axis.
        let near = (1.0 + strain.least) * a.min(tip);
        if !(near > floor) {
            return Err(no(format!(
                "bent, the flange reaches the axis of its edge's arc (the strain along the edge is down to {:.6}): shorten the flange or lower its angle",
                strain.least
            )));
        }
        if let Some(m) = max_strain
            && strain.largest() > m
        {
            return Err(no(format!(
                "its flat pattern is approximated (a curved bend is not developable) and stretches or compresses the material along the edge by up to {:.6} (proven: from {:.6} to {:.6}), more than maxStrain {m}: a larger edge radius, a shorter flange or a smaller angle lowers it",
                strain.largest(),
                strain.least,
                strain.most
            )));
        }
        let joined = [
            if seg.reversed { seg.end } else { seg.start },
            if seg.reversed { seg.start } else { seg.end },
        ]
        .map(|p| {
            continues(
                bends,
                flats,
                p,
                center,
                grows,
                theta,
                f,
                t,
                flat_length,
                tolerance,
            )
        });
        if sweep >= TAU && joined.iter().any(Option::is_some) {
            return Err(no("a whole circle meets no other flange".to_string()));
        }
        out.push(Curved {
            joined,
            id: f.id,
            edge: f.edge,
            hole,
            seg: seg_at,
            center,
            edge_radius: a,
            grows,
            begin,
            sweep,
            angle: match f.direction {
                Direction::Up => theta,
                Direction::Down => -theta,
            },
            radius: f.radius,
            k: f.k,
            thickness: t,
            allowance: ba,
            flat_length,
            strain,
        });
    }
    Ok(out)
}

/// The straight flange whose bend ends at `p` (an end of a curved
/// flange's arc) and that the curved flange continues: bent from the base,
/// its tangent line ending at `p` and running on along the arc's tangent
/// there (`across` the arc's radial direction the flange grows to), alike
/// in angle and direction, radius, K-factor and flat length, its flat its
/// rectangle (no outline of its own, no closed corner, no trim). Its bend.
#[allow(clippy::too_many_arguments)]
fn continues(
    bends: &[Bend],
    flats: &[Flat],
    p: P2,
    center: P2,
    grows: f64,
    theta: f64,
    f: &CurvedSpec,
    t: f64,
    flat_length: f64,
    tolerance: &Tolerance,
) -> Option<usize> {
    let tol = tolerance.linear;
    let radial = {
        let d = [p[0] - center[0], p[1] - center[1]];
        let n = d[0].phypot(d[1]);
        (n > 0.0).then(|| [grows * d[0] / n, grows * d[1] / n])?
    };
    let signed = match f.direction {
        Direction::Up => theta,
        Direction::Down => -theta,
    };
    bends.iter().position(|b| {
        let at = |q: P2| (q[0] - p[0]).phypot(q[1] - p[1]) <= tol;
        b.parent == 0
            && (at(b.start) || at(b.end))
            && (b.across[0] * radial[1] - b.across[1] * radial[0]).abs() <= tolerance.angular
            && b.across[0] * radial[0] + b.across[1] * radial[1] > 0.0
            && (b.angle - signed).abs() <= tolerance.angular
            && (b.radius - f.radius).abs() <= tol
            && (b.k - f.k).abs() * t <= tol
            && (b.flat_length - flat_length).abs() <= tol
            && b.trim.iter().all(Option::is_none)
            && flats
                .get(b.child)
                .is_some_and(|c| !c.outlined && c.ends.is_empty())
    })
}

/// Whether `s` is curved flange `c`'s edge in the pattern.
fn is_edge(s: &Segment, c: &Curved, tol: f64) -> bool {
    let same = |p: P2, q: P2| (p[0] - q[0]).phypot(p[1] - q[1]) <= tol;
    s.source == c.edge
        && match s.shape {
            Shape::Arc { center, radius } | Shape::Circle { center, radius } => {
                same(center, c.center) && (radius - c.edge_radius).abs() <= tol
            }
            _ => false,
        }
}

/// The pieces that replace curved flange `c`'s edge `s` in the outline:
/// the strip's and the flat's side along each radial line and the tip, or
/// the tip circle for a whole circle.
fn replaced(s: &Segment, c: &Curved) -> Vec<Segment> {
    let rt = c.tip_radius();
    let out = |p: P2| -> P2 {
        let f = rt / c.edge_radius;
        [
            c.center[0] + f * (p[0] - c.center[0]),
            c.center[1] + f * (p[1] - c.center[1]),
        ]
    };
    let (s1, e1) = (out(s.start), out(s.end));
    let tip = Segment {
        source: s.source,
        piece: s.piece,
        reversed: s.reversed,
        start: s1,
        end: e1,
        shape: match s.shape {
            Shape::Circle { center, .. } => Shape::Circle { center, radius: rt },
            _ => Shape::Arc {
                center: c.center,
                radius: rt,
            },
        },
    };
    if c.whole() {
        return vec![tip];
    }
    let line = |a: P2, b: P2| Segment {
        source: s.source,
        piece: s.piece,
        reversed: false,
        start: a,
        end: b,
        shape: Shape::Line,
    };
    // Where the loop enters the arc (its own start unless it runs against
    // it) and leaves it: a joined end has no side.
    let (enter, leave) = if s.reversed {
        (c.joined[1], c.joined[0])
    } else {
        (c.joined[0], c.joined[1])
    };
    let mut out = Vec::with_capacity(3);
    if enter.is_none() {
        out.push(line(s.start, s1));
    }
    out.push(tip);
    if leave.is_none() {
        out.push(line(e1, s.end));
    }
    out
}

/// Whether `s` is a piece of a joined straight flange's side at an end of
/// curved flange `c`: a line on the radial line through that end, between
/// the edge's radius and the tip's.
fn joined_side(s: &Segment, c: &Curved, tol: f64) -> bool {
    if !matches!(s.shape, Shape::Line) {
        return false;
    }
    let (lo, hi) = {
        let (a, b) = (c.edge_radius, c.tip_radius());
        (a.min(b) - tol, a.max(b) + tol)
    };
    (0..2)
        .filter(|&j| c.joined.get(j).copied().flatten().is_some())
        .any(|j| {
            let psi = c.begin + if j == 0 { 0.0 } else { c.sweep };
            let (sn, cs) = psi.psin_cos();
            let on = |q: P2| {
                let d = [q[0] - c.center[0], q[1] - c.center[1]];
                let along = d[0] * cs + d[1] * sn;
                let off = d[1] * cs - d[0] * sn;
                off.abs() <= tol && along >= lo && along <= hi
            };
            on(s.start) && on(s.end)
        })
}

/// The flat pattern's outline with the curved flanges: each one's edge
/// replaced by its sides and tip.
pub(super) fn with_curved(
    chains: Vec<Vec<Segment>>,
    curved: &[Curved],
    tol: f64,
) -> Vec<Vec<Segment>> {
    if curved.is_empty() {
        return chains;
    }
    chains
        .into_iter()
        .map(|chain| {
            chain
                .into_iter()
                .filter(|s| !curved.iter().any(|c| joined_side(s, c, tol)))
                .flat_map(|s| match curved.iter().find(|c| is_edge(&s, c, tol)) {
                    Some(c) => replaced(&s, c),
                    None => vec![s],
                })
                .collect()
        })
        .collect()
}

/// The pattern with the curved flanges is one sheet: its outline, run
/// through the sketch's profile detection, is one region with every hole
/// and the area the pieces add up to (as `pattern_clear`).
pub(super) fn pattern_clear(
    flats: &[Flat],
    bends: &[Bend],
    cuts: &[Cut],
    curved: &[Curved],
    tolerance: &Tolerance,
) -> Result<(), KernelError> {
    use crate::profile::{SketchCurve, detect_profiles};
    let tol = tolerance.linear;
    let chains: Vec<Vec<Segment>> = tagged_outline(flats, bends, cuts, tol)
        .into_iter()
        .map(|c| c.into_iter().map(|(s, _)| s).collect())
        .collect();
    let chains = with_curved(chains, curved, tol);
    let curves: Vec<SketchCurve> = chains
        .iter()
        .flatten()
        .enumerate()
        .map(|(i, s)| SketchCurve {
            id: i as u64 + 1,
            curve: sketch_curve(s),
        })
        .collect();
    let overlaps = || {
        refuse(
            None,
            "curvedFlanges: the flat pattern overlaps itself: a curved flange's strip or flat crosses, touches or lies along another piece of the pattern (a flange on a neighbouring edge, a hole, the base), so it cannot be cut from one sheet; shorten or move the flanges",
        )
    };
    let profiles = match detect_profiles(&curves, tolerance) {
        Ok(p) => p,
        Err(KernelError::CrossingCurves { .. } | KernelError::BranchingProfile { .. }) => {
            return Err(overlaps());
        }
        Err(other) => {
            return Err(refuse(
                None,
                format!(
                    "curvedFlanges: the flat pattern's outline could not be checked for overlapping itself: {other}"
                ),
            ));
        }
    };
    let holes: usize = flats.iter().map(|f| f.holes.len()).sum::<usize>() + cuts.len();
    let area = pattern_area(flats, bends, cuts) + curved.iter().map(Curved::area).sum::<f64>();
    match profiles.as_slice() {
        [one]
            if one.holes.len() == holes
                && (one.area() - area).abs() <= tol * area.abs().sqrt() * 8.0 =>
        {
            Ok(())
        }
        _ => Err(overlaps()),
    }
}

/// The section's point at `x` across the edge and height `z`, at the
/// polar angle given by its unit `e`, in the pattern's axes.
fn at(c: &Curved, e: Vec3, x: f64, z: f64) -> Vec3 {
    let rho = c.edge_radius + c.grows * x;
    Vec3::new(c.center[0], c.center[1], z) + e * rho
}

/// An end of a curved flange's arc: the vertex on the base, at the far
/// tangent line and at the tip, and a joined straight flange's bend end
/// arc and side there.
type End = (VertexId, VertexId, VertexId, Option<(EdgeId, EdgeId)>);

/// Adds every curved flange's two faces of the folded bottom skin to the
/// sheet (the bend's torus and the flat's cone, cylinder or plane), on
/// the base's edge; their edges' keys and the faces' roles.
pub(super) fn add_to_sheet(
    plane: &Frame,
    curved: &[Curved],
    flats: &[Flat],
    sheet: &mut Body,
    roles: &mut Vec<SheetFace>,
    keys: &mut Vec<EdgeKey>,
    tolerance: &Tolerance,
) -> Result<(), KernelError> {
    let to_world = Transform::from_frame(plane);
    let base_face = roles
        .iter()
        .position(|r| *r == SheetFace::Flat(0))
        .and_then(|f| u32::try_from(f).ok())
        .map(FaceId)
        .ok_or_else(|| refuse(None, "curvedFlanges: the base has no face"))?;
    let shell = sheet
        .shells
        .iter()
        .position(|s| s.faces.contains(&base_face))
        .ok_or_else(|| refuse(None, "curvedFlanges: the base is in no shell"))?;
    for (n, c) in curved.iter().enumerate() {
        let no = |reason: &str| refuse(None, format!("curvedFlanges.{n}: {reason}"));
        let key = match c.hole {
            None => outline_key(flats, 0, c.seg),
            Some(hole) => EdgeKey::Hole {
                flat: 0,
                hole,
                seg: c.seg,
            },
        };
        let base_edge = keys
            .iter()
            .position(|k| *k == key)
            .and_then(|e| u32::try_from(e).ok())
            .map(EdgeId)
            .ok_or_else(|| no("its edge is not in the folded sheet"))?;
        // How the base's face runs along the edge.
        let base_reversed = sheet
            .faces
            .get(base_face.0 as usize)
            .into_iter()
            .flat_map(|f| f.loops.iter())
            .filter_map(|l| sheet.loops.get(l.0 as usize))
            .flat_map(|l| l.coedges.iter())
            .find(|co| co.edge == base_edge)
            .map(|co| co.reversed)
            .ok_or_else(|| no("the base's face does not run along its edge"))?;
        let edge = sheet
            .edges
            .get(base_edge.0 as usize)
            .cloned()
            .ok_or_else(|| no("its edge is missing"))?;
        let Curve3::Circle(base_circle) = &edge.curve else {
            return Err(no("its edge is not a circle in the folded sheet"));
        };
        let frame0 = *base_circle.frame();
        let point_of =
            |sheet: &Body, v: VertexId| sheet.vertices.get(v.0 as usize).map(|x| x.point);
        let (pb0, pb1) = (
            point_of(sheet, edge.start).ok_or_else(|| no("a vertex is missing"))?,
            point_of(sheet, edge.end).ok_or_else(|| no("a vertex is missing"))?,
        );
        let t = c.thickness;
        let up = c.angle > 0.0;
        let sigma = if up { 1.0 } else { -1.0 };
        let theta = c.theta();
        let rb = if up { c.radius + t } else { c.radius };
        let zc = if up { t + c.radius } else { -c.radius };
        let (sn, cs) = theta.psin_cos();
        let (xe, ze) = (rb * sn, zc - sigma * rb * cs);
        let lf = c.flat_length;
        let (xt, zt) = (xe + lf * cs, ze + sigma * lf * sn);
        let rho_e = c.edge_radius + c.grows * xe;
        let rho_t = c.edge_radius + c.grows * xt;
        let unit_at = |p: Vec3| -> Option<Vec3> {
            let (x, y, _) = plane.local(p);
            Vec3::new(x - c.center[0], y - c.center[1], 0.0).normalized()
        };
        let e0 = unit_at(pb0).ok_or_else(|| no("its edge's start is on its centre"))?;
        let e1 = unit_at(pb1).ok_or_else(|| no("its edge's end is on its centre"))?;
        let world = |p: Vec3| to_world.point(p);
        let axis_point = |z: f64| world(Vec3::new(c.center[0], c.center[1], z));
        let z_world = to_world.vector(Vec3::Z);
        let level = |z: f64, r: f64| -> Result<Curve3, KernelError> {
            let f = frame0
                .at(axis_point(z))
                .ok_or_else(|| no("a circle's centre is not finite"))?;
            Ok(Curve3::Circle(Circle::new(f, r)?))
        };
        let range = edge.range;
        let whole = c.whole();
        // Per end of the arc (its own start, its end): the vertex on the
        // base, at the far tangent line and at the tip, and the meridian of
        // the bend and the flat's line there; a joined end takes the
        // straight flange's own (its bend's arc and its flat's side).
        let tol = tolerance.linear;
        let mut ends: Vec<End> = Vec::new();
        for (j, (e, vb)) in [(e0, edge.start), (e1, edge.end)].into_iter().enumerate() {
            let (p1, p2) = (world(at(c, e, xe, ze)), world(at(c, e, xt, zt)));
            let Some(bend) = c.joined.get(j).copied().flatten() else {
                if whole && j == 1 {
                    let first = ends
                        .first()
                        .copied()
                        .ok_or_else(|| no("a circle lost its start"))?;
                    ends.push(first);
                } else {
                    let v1 = sheet.add_vertex(p1);
                    let v2 = sheet.add_vertex(p2);
                    ends.push((vb, v1, v2, None));
                }
                continue;
            };
            // The straight bend's arc at this end starts at the base's
            // vertex; its flat's side runs from that arc's far end to the
            // tip.
            let arc = (0..keys.len())
                .find(|&x| {
                    matches!(keys.get(x), Some(EdgeKey::Arc { bend: b, .. }) if *b == bend)
                        && sheet.edges.get(x).is_some_and(|ed| ed.start == vb)
                })
                .and_then(|x| u32::try_from(x).ok())
                .map(EdgeId)
                .ok_or_else(|| no("the flange it continues has no bend's end at its arc's end"))?;
            let v1 = sheet
                .edges
                .get(arc.0 as usize)
                .map(|ed| ed.end)
                .ok_or_else(|| no("an edge is missing"))?;
            let near =
                |v: VertexId, q: Vec3| point_of(sheet, v).is_some_and(|x| x.distance(q) <= tol);
            if !near(v1, p1) {
                return Err(no(
                    "the flange it continues does not meet its bend's end (folded elsewhere)",
                ));
            }
            let side = sheet.edges.iter().enumerate().find_map(|(x, ed)| {
                if !matches!(ed.curve, Curve3::Line(_)) {
                    return None;
                }
                let other = if ed.start == v1 {
                    ed.end
                } else if ed.end == v1 {
                    ed.start
                } else {
                    return None;
                };
                near(other, p2).then_some((x, other))
            });
            let (x, v2) = side.ok_or_else(|| {
                no("the flange it continues has no side from its bend to its tip at its arc's end")
            })?;
            let line = EdgeId(u32::try_from(x).map_err(|_| no("too many edges"))?);
            ends.push((vb, v1, v2, Some((arc, line))));
        }
        let [(_, v10, v20, _), (_, v11, v21, _)] = [
            *ends.first().ok_or_else(|| no("no start"))?,
            *ends.get(1).ok_or_else(|| no("no end"))?,
        ];
        let far = sheet.add_edge(level(ze, rho_e)?, range, v10, v11);
        let tip = sheet.add_edge(level(zt, rho_t)?, range, v20, v21);
        let mut made: Vec<(EdgeId, usize)> = vec![(far, 0), (tip, 1)];
        // The sides: the meridian arcs of the bend and the flat's lines.
        let mut sides: Vec<(EdgeId, EdgeId)> = Vec::new();
        if !whole {
            for (j, (e, (vb, v1, v2, joined))) in [e0, e1].into_iter().zip(ends.clone()).enumerate()
            {
                if let Some(pair) = joined {
                    sides.push(pair);
                    continue;
                }
                let q = world(at(c, e, 0.0, zc));
                let xd = z_world * (-sigma);
                let yd = to_world.vector(e * c.grows);
                let f = Frame::new(q, xd.cross(yd), xd)
                    .ok_or_else(|| no("a side's plane is not finite"))?;
                let meridian =
                    sheet.add_edge(Curve3::Circle(Circle::new(f, rb)?), (0.0, theta), vb, v1);
                let (p1, p2) = (world(at(c, e, xe, ze)), world(at(c, e, xt, zt)));
                let line = sheet.add_edge(
                    Curve3::Line(Line::through(p1, p2)?),
                    (0.0, p1.distance(p2)),
                    v1,
                    v2,
                );
                made.push((meridian, 2 + j));
                made.push((line, 4 + j));
                sides.push((meridian, line));
            }
        }
        // A coedge from vertex `from` along `edge`.
        let along = |sheet: &Body, edge: EdgeId, from: VertexId| -> Coedge {
            let reversed = sheet
                .edges
                .get(edge.0 as usize)
                .is_some_and(|ed| ed.start != from);
            Coedge { edge, reversed }
        };
        // The bend's loop runs against the base's along the edge, the
        // flat's against the bend's along the far tangent line: with the
        // base running from the arc's start to its end (`f` false), the
        // bend runs end → start on the base, then out at the start, along
        // the far line to the end and back; the flat the other way.
        let f = base_reversed;
        let (vb0, vb1) = (edge.start, edge.end);
        let (bend_loops, flat_loops) = match sides.as_slice() {
            [(m0, l0), (m1, l1)] => {
                let bend = if f {
                    vec![
                        along(sheet, base_edge, vb0),
                        along(sheet, *m1, vb1),
                        along(sheet, far, v11),
                        along(sheet, *m0, v10),
                    ]
                } else {
                    vec![
                        along(sheet, base_edge, vb1),
                        along(sheet, *m0, vb0),
                        along(sheet, far, v10),
                        along(sheet, *m1, v11),
                    ]
                };
                let flat = if f {
                    vec![
                        along(sheet, far, v10),
                        along(sheet, *l1, v11),
                        along(sheet, tip, v21),
                        along(sheet, *l0, v20),
                    ]
                } else {
                    vec![
                        along(sheet, far, v11),
                        along(sheet, *l0, v10),
                        along(sheet, tip, v20),
                        along(sheet, *l1, v21),
                    ]
                };
                (vec![bend], vec![flat])
            }
            _ => {
                let co = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
                (
                    vec![vec![co(base_edge, !f)], vec![co(far, f)]],
                    vec![vec![co(far, !f)], vec![co(tip, f)]],
                )
            }
        };
        // The surfaces, about the arc's axis.
        let torus = Surface::Torus(Torus::new(
            frame0
                .at(axis_point(zc))
                .ok_or_else(|| no("the bend's axis is not finite"))?,
            c.edge_radius,
            rb,
        )?);
        let flat_frame = frame0
            .at(axis_point(ze))
            .ok_or_else(|| no("the flat's axis is not finite"))?;
        let flat_surface = if (lf * cs).abs() <= 1e-3 * tol {
            Surface::Cylinder(Cylinder::new(flat_frame, rho_e)?)
        } else if (lf * sn).abs() <= 1e-3 * tol || (cs / sn).abs() > 1e12 {
            Surface::Plane(Plane::new(flat_frame))
        } else {
            let slope = c.grows * cs / (sigma * sn);
            Surface::Cone(Cone::new(flat_frame, rho_e, slope.patan())?)
        };
        // Each face's normal points into the material, as every face of
        // the folded bottom skin does: the section's normal there is
        // (−σ sin φ, cos φ) across and up.
        let (ms, mc) = (c.begin + 0.5 * c.sweep).psin_cos();
        let mid = Vec3::new(mc, ms, 0.0);
        let facing = |surface: &Surface, x: f64, z: f64, phi: f64| -> Result<bool, KernelError> {
            let p = world(at(c, mid, x, z));
            let (u, v) = surface.project(p)?;
            let normal = surface
                .normal(u, v)
                .ok_or_else(|| no("a face has no normal"))?;
            let (s, k) = phi.psin_cos();
            let material = to_world.vector(mid * (c.grows * -sigma * s) + Vec3::Z * k);
            Ok(normal.dot(material) < 0.0)
        };
        let half = 0.5 * theta;
        let bend_reversed = facing(
            &torus,
            rb * half.psin(),
            zc - sigma * rb * half.pcos(),
            half,
        )?;
        let flat_reversed = facing(
            &flat_surface,
            xe + 0.5 * lf * cs,
            ze + 0.5 * sigma * lf * sn,
            theta,
        )?;
        let bend_loops = bend_loops.into_iter().map(|l| sheet.add_loop(l)).collect();
        let flat_loops = flat_loops.into_iter().map(|l| sheet.add_loop(l)).collect();
        let bend_face = sheet.add_face(torus, bend_reversed, bend_loops);
        let flat_face = sheet.add_face(flat_surface, flat_reversed, flat_loops);
        if let Some(s) = sheet.shells.get_mut(shell) {
            s.faces.push(bend_face);
            s.faces.push(flat_face);
        }
        roles.push(SheetFace::CurvedBend(n));
        roles.push(SheetFace::CurvedFlat(n));
        made.sort_by_key(|(e, _)| e.0);
        for (e, what) in made {
            if keys.len() != e.0 as usize {
                return Err(no("its edges are out of order"));
            }
            keys.push(EdgeKey::Curved {
                flange: n,
                edge: what,
            });
        }
    }
    let defects = craft3d_topo::check_remembered(sheet, tolerance);
    if !defects.is_empty() {
        return Err(KernelError::InvalidBody {
            operation: "sheet metal (curved flanges)",
            defects: defects.into_iter().take(crate::MAX_REPORTED).collect(),
        });
    }
    Ok(())
}
