//! The N-sided fill (docs/design/surfacing.md › *The N-sided fill*): a
//! closed loop of three to six curves filled by one four-sided network
//! patch per curve around a centre point (Hahn, *Filling polygonal holes
//! with rectangular patches*, 1989), every corner regular. The outer edges
//! take the caller's holds; each inner seam (a spoke from the centre to a
//! curve's midpoint) is held by both patches beside it to one ribbon
//! surface, tangent (G1) or in curvature (G2), so the two patches meet as
//! the ribbon asks. Every ribbon is, at the centre, the jet of one
//! paraboloid (a tangent plane and a second fundamental form), so the
//! twists asked at the centre agree (the vertex enclosure condition;
//! Peters, *Smooth interpolation of a mesh of curves*, 1991). The seams
//! are then proven by [`continuity`].

use super::{Condition, Continuity, H, Net, NetworkError, Order, continuity, refuse};
use crate::curvature::second_derivatives;
use crate::refine::Raw;
use crate::refine::piece;
use crate::{NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Portable, Vec3};

/// Fewest and most curves round a fill.
pub const MIN_SIDES: usize = 3;
pub const MAX_SIDES: usize = 6;

/// How the inner seams are asked to meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inner {
    Tangent,
    Curvature,
}

/// The fill's input: `curves` end to end round the loop (curve `k` from
/// corner `k` to corner `k + 1`), `edges` one hold per curve.
#[derive(Clone, Copy, Debug)]
pub struct Nsided<'a> {
    pub curves: &'a [NurbsCurve],
    pub edges: &'a [Condition<'a>],
    pub inner: Inner,
    /// How far the centre is moved along the loop's normal from where the
    /// curves put it (a length; 0 for none).
    pub rise: f64,
    pub tolerance: f64,
    pub angular: f64,
}

/// One inner seam: what it was asked, what it was built as (with why,
/// where less), and how the two patches meet along it, proven.
#[derive(Clone, Debug)]
pub struct Seam {
    pub asked: Inner,
    pub built: Inner,
    pub why: Option<String>,
    pub continuity: Continuity,
}

/// The fill: patch `k` has corners centre, midpoint `k`, corner `k + 1`,
/// midpoint `k + 1` (its `u` from the centre along spoke `k`, its `v`
/// along spoke `k + 1`).
#[derive(Clone, Debug)]
pub struct Fill {
    pub patches: Vec<NurbsSurface>,
    /// Spoke `k` from the centre to midpoint `k`, over `[0, 1]`.
    pub spokes: Vec<NurbsCurve>,
    /// Curve `k` cut at its midpoint: corner `k` to midpoint `k`, then
    /// midpoint `k` to corner `k + 1`.
    pub halves: Vec<(NurbsCurve, NurbsCurve)>,
    pub centre: Vec3,
    /// Seam `k` along spoke `k`, between patch `k − 1` and patch `k`.
    pub seams: Vec<Seam>,
    /// Per curve held tangent or in curvature: what was asked and built,
    /// and how each of its halves' patches meets its face, proven.
    pub outer: Vec<Option<Held>>,
}

/// The local second-order picture of a face at a point: the unit normal
/// (turned to the fill's side) and its second fundamental form on 3D
/// tangent vectors.
struct Local {
    n: Vec3,
    su: Vec3,
    sv: Vec3,
    suu: Vec3,
    suv: Vec3,
    svv: Vec3,
}

impl Local {
    fn at(surface: &Surface, p: Vec3, towards: Vec3) -> Result<Local, NetworkError> {
        let (u, v) = surface.project(p)?;
        let (_, su, sv) = surface.derivatives(u, v);
        let (suu, suv, svv) = second_derivatives(surface, u, v);
        let n = su.cross(sv).normalized().ok_or_else(|| {
            NetworkError::Refused("a held face has no normal at a midpoint".into())
        })?;
        let n = if n.dot(towards) < 0.0 { -n } else { n };
        Ok(Local {
            n,
            su,
            sv,
            suu,
            suv,
            svv,
        })
    }

    /// The coordinates of a tangent vector in `(S_u, S_v)`.
    fn coords(&self, x: Vec3) -> (f64, f64) {
        let (a, b, c) = (
            self.su.dot(self.su),
            self.su.dot(self.sv),
            self.sv.dot(self.sv),
        );
        let det = a * c - b * b;
        if !det.is_finite() || det.abs() <= f64::MIN_POSITIVE {
            return (0.0, 0.0);
        }
        let (r, s) = (self.su.dot(x), self.sv.dot(x));
        ((c * r - b * s) / det, (a * s - b * r) / det)
    }

    /// `II(x, y)`.
    fn second(&self, x: Vec3, y: Vec3) -> f64 {
        let (a, b) = self.coords(x);
        let (c, d) = self.coords(y);
        self.n
            .dot(self.suu * (a * c) + self.suv * (a * d + b * c) + self.svv * (b * d))
    }
}

fn unit(v: Vec3, what: &str) -> Result<Vec3, NetworkError> {
    v.normalized()
        .filter(|u| u.is_finite())
        .ok_or_else(|| NetworkError::Refused(format!("{what} has no direction")))
}

/// The data of one curve of the loop at its midpoint.
struct Mid {
    m: Vec3,
    /// Unit tangent along the curve.
    e: Vec3,
    /// Unit normal of the fill there (the held face's, else the loop's
    /// turned square to `e`).
    n: Vec3,
    /// Unit leaving direction into the fill.
    x: Vec3,
    /// The normal curvature across (`x`).
    across: f64,
}

/// Fills the loop. Refused with the curve named where the loop does not
/// close, a hold is not met, a spoke's ribbon turns along it, or a patch
/// or seam is refused by the network.
pub fn nsided(fill: &Nsided<'_>) -> Result<Fill, NetworkError> {
    let n = fill.curves.len();
    if !(MIN_SIDES..=MAX_SIDES).contains(&n) || fill.edges.len() != n {
        return refuse(format!(
            "an N-sided fill has {MIN_SIDES} to {MAX_SIDES} curves, each with one hold; got {n} curves and {} holds",
            fill.edges.len()
        ));
    }
    let tol = fill.tolerance;
    // Corners, and the loop closing.
    let mut corners = Vec::with_capacity(n);
    for (k, c) in fill.curves.iter().enumerate() {
        let (a, b) = c.domain();
        let next = fill.curves.get((k + 1) % n).ok_or_else(super::internal)?;
        let (na, _) = next.domain();
        let gap = c.eval(b).distance(next.eval(na));
        if gap > tol {
            return refuse(format!(
                "curve {} ends {gap:.3e} from where curve {} starts (the tolerance is {tol:.3e})",
                k + 1,
                (k + 1) % n + 1
            ));
        }
        corners.push(c.eval(a));
    }
    // Halves and midpoints.
    let mut halves = Vec::with_capacity(n);
    let mut raw_mids = Vec::with_capacity(n);
    for (k, c) in fill.curves.iter().enumerate() {
        let (a, b) = c.domain();
        let t = 0.5 * (a + b);
        halves.push((piece(c, a, t)?, piece(c, t, b)?));
        let (m, d) = c.derivatives(t);
        raw_mids.push((m, d, k));
    }
    // The loop's normal (Newell's sum over corners and midpoints).
    let mut ring = Vec::with_capacity(2 * n);
    for k in 0..n {
        ring.push(*corners.get(k).ok_or_else(super::internal)?);
        ring.push(raw_mids.get(k).ok_or_else(super::internal)?.0);
    }
    let mut newell = Vec3::ZERO;
    for (i, p) in ring.iter().enumerate() {
        let q = ring.get((i + 1) % ring.len()).copied().unwrap_or(*p);
        newell += p.cross(q);
    }
    let nc = unit(newell, "the loop (it encloses no area)")?;
    let centroid = raw_mids.iter().fold(Vec3::ZERO, |s, m| s + m.0) * (1.0 / n as f64);
    // The data at the midpoints.
    let mut mids: Vec<Mid> = Vec::with_capacity(n);
    for (m, d, k) in raw_mids {
        let e = unit(d, &format!("curve {} at its midpoint", k + 1))?;
        let cond = fill.edges.get(k).ok_or_else(super::internal)?;
        let mid = match cond.surface() {
            Some(surface) if !matches!(cond, Condition::Position(_)) => {
                let local = Local::at(surface, m, nc)?;
                let x = unit(
                    local.n.cross(e),
                    &format!("curve {}'s leaving direction", k + 1),
                )?;
                Mid {
                    m,
                    e,
                    n: local.n,
                    x,
                    across: local.second(x, x),
                }
            }
            _ => {
                let x = unit(
                    nc.cross(e),
                    &format!("curve {} (it runs along the loop's normal)", k + 1),
                )?;
                let nm = unit(e.cross(x), "a midpoint's normal")?;
                Mid {
                    m,
                    e,
                    n: nm,
                    x,
                    across: 0.0,
                }
            }
        };
        mids.push(mid);
    }
    // The tangent plane at the centre and its second fundamental form,
    // from the normal curvatures across the midpoints (least squares in
    // the directions from the midpoints' centroid).
    let e1 = unit(
        (ring.first().copied().unwrap_or(Vec3::ZERO) - centroid)
            - nc * (ring.first().copied().unwrap_or(Vec3::ZERO) - centroid).dot(nc),
        "the loop's first corner seen from its middle",
    )?;
    let e2 = nc.cross(e1);
    let mut normal = [[0.0_f64; 3]; 3];
    let mut rhs = [0.0_f64; 3];
    for mid in &mids {
        let d = -mid.x;
        let (x, y) = (d.dot(e1), d.dot(e2));
        let row = [x * x, 2.0 * x * y, y * y];
        let len2 = x * x + y * y;
        let row = if len2 > 0.0 {
            row.map(|r| r / len2)
        } else {
            row
        };
        for (i, ri) in row.iter().enumerate() {
            for (j, rj) in row.iter().enumerate() {
                if let Some(slot) = normal.get_mut(i).and_then(|r| r.get_mut(j)) {
                    *slot += ri * rj;
                }
            }
            if let Some(slot) = rhs.get_mut(i) {
                *slot += ri * mid.across;
            }
        }
    }
    let form = solve3(normal, rhs).unwrap_or([0.0; 3]);
    let ii_c = |a: Vec3, b: Vec3| -> f64 {
        let (ax, ay) = (a.dot(e1), a.dot(e2));
        let (bx, by) = (b.dot(e1), b.dot(e2));
        form[0] * ax * bx + form[1] * (ax * by + ay * bx) + form[2] * ay * by
    };
    // The centre: the mean of what each midpoint predicts along its
    // osculating circle across.
    let mut centre = Vec3::ZERO;
    for mid in &mids {
        let l = mid.m.distance(centroid);
        let k = mid.across;
        let a = k * l;
        let (s, c) = a.psin_cos();
        let p = if a.abs() > 1e-8 {
            mid.m + mid.x * (s / k) + mid.n * ((1.0 - c) / k)
        } else {
            mid.m + mid.x * l + mid.n * (0.5 * k * l * l)
        };
        centre += p;
    }
    if !fill.rise.is_finite() {
        return refuse("the fill's rise is not a number".into());
    }
    let centre = centre * (1.0 / n as f64) + nc * fill.rise;
    // Every curve's face as an exact parametrisation about the curve (a
    // ribbon), and the halves as sides.
    let extent = corners
        .iter()
        .fold(0.0_f64, |m, p| m.max(p.distance(centre)))
        .max(tol);
    let mut faces: Vec<Option<FaceRibbon>> = Vec::with_capacity(n);
    let mut leveled: Vec<(NurbsCurve, NurbsCurve)> = Vec::with_capacity(n);
    for (k, (a, b)) in halves.iter().enumerate() {
        let label = format!("curve {}", k + 1);
        let (a, b) = (
            ends_of_weight_one(a, &label)?,
            ends_of_weight_one(b, &label)?,
        );
        let cond = fill.edges.get(k).ok_or_else(super::internal)?;
        let curve = fill.curves.get(k).ok_or_else(super::internal)?;
        faces.push(match cond.held() {
            Some((face, _, _)) => Some(FaceRibbon::about(face, curve, &label)?),
            None => None,
        });
        leveled.push((a, b));
    }
    // The fill, asked as it is; where a seam asked in curvature is not
    // proven G2, again with every curvature hold tangent, said per seam.
    let asked_outer: Vec<bool> = fill
        .edges
        .iter()
        .map(|c| matches!(c, Condition::Curvature { .. }))
        .collect();
    let first = build(
        fill,
        &mids,
        &faces,
        &leveled,
        centre,
        nc,
        &ii_c,
        extent,
        fill.inner,
        &asked_outer,
    );
    let want_g2 = fill.inner == Inner::Curvature || asked_outer.iter().any(|x| *x);
    let (made, fallback) = match first {
        Ok(made) if !want_g2 || made.all_as_built() => (made, None),
        Ok(made) => {
            let why = made.first_shortfall();
            let again = build(
                fill,
                &mids,
                &faces,
                &leveled,
                centre,
                nc,
                &ii_c,
                extent,
                Inner::Tangent,
                &vec![false; n],
            )?;
            (again, Some(why))
        }
        Err(e) if want_g2 => {
            let why = format!("the fill in curvature was refused: {e}");
            let again = build(
                fill,
                &mids,
                &faces,
                &leveled,
                centre,
                nc,
                &ii_c,
                extent,
                Inner::Tangent,
                &vec![false; n],
            )?;
            (again, Some(why))
        }
        Err(e) => return Err(e),
    };
    let Built {
        patches,
        spokes,
        inner,
        outer,
    } = made;
    let mut seams = Vec::with_capacity(n);
    for (k, (how, measured)) in inner.into_iter().enumerate() {
        if measured.order < how.order() {
            return refuse(format!(
                "seam {} (spoke to curve {}) is proven only {} where {} was built: {}",
                k + 1,
                k + 1,
                measured.order.name(),
                how.order().name(),
                super::describe(&measured)
            ));
        }
        let why = match (fill.inner, how) {
            (Inner::Curvature, Inner::Tangent) => Some(format!(
                "held tangent (G1), not in curvature: {}",
                fallback.clone().unwrap_or_default()
            )),
            _ => None,
        };
        seams.push(Seam {
            asked: fill.inner,
            built: how,
            why,
            continuity: measured,
        });
    }
    let mut held = Vec::with_capacity(n);
    for (k, o) in outer.into_iter().enumerate() {
        let Some((how, pair)) = o else {
            held.push(None);
            continue;
        };
        for measured in &pair {
            if measured.order < how.order() {
                return refuse(format!(
                    "curve {} is held to its face but the fill is proven only {} there where {} was built: {}",
                    k + 1,
                    measured.order.name(),
                    how.order().name(),
                    super::describe(measured)
                ));
            }
        }
        let asked = if asked_outer.get(k).copied().unwrap_or(false) {
            Inner::Curvature
        } else {
            Inner::Tangent
        };
        let why = match (asked, how) {
            (Inner::Curvature, Inner::Tangent) => Some(format!(
                "held tangent (G1), not in curvature: {}",
                fallback.clone().unwrap_or_default()
            )),
            _ => None,
        };
        held.push(Some(Held {
            asked,
            built: how,
            why,
            halves: pair,
        }));
    }
    Ok(Fill {
        patches,
        spokes,
        halves: leveled,
        centre,
        seams,
        outer: held,
    })
}

impl Inner {
    fn order(self) -> Order {
        match self {
            Inner::Tangent => Order::G1,
            Inner::Curvature => Order::G2,
        }
    }
}

/// What one build made: the patches, the spokes, and per seam and held
/// curve how it was built and what the measure proved.
struct Built {
    patches: Vec<NurbsSurface>,
    spokes: Vec<NurbsCurve>,
    inner: Vec<(Inner, Continuity)>,
    outer: Vec<Option<(Inner, Vec<Continuity>)>>,
}

impl Built {
    fn all_as_built(&self) -> bool {
        self.inner.iter().all(|(how, c)| c.order >= how.order())
            && self
                .outer
                .iter()
                .flatten()
                .all(|(how, pair)| pair.iter().all(|c| c.order >= how.order()))
    }

    fn first_shortfall(&self) -> String {
        for (k, (how, c)) in self.inner.iter().enumerate() {
            if c.order < how.order() {
                return format!(
                    "seam {} was proven only {} ({})",
                    k + 1,
                    c.order.name(),
                    super::describe(c)
                );
            }
        }
        for (k, o) in self.outer.iter().enumerate() {
            if let Some((how, pair)) = o {
                for c in pair {
                    if c.order < how.order() {
                        return format!(
                            "curve {} was proven only {} against its face ({})",
                            k + 1,
                            c.order.name(),
                            super::describe(c)
                        );
                    }
                }
            }
        }
        String::new()
    }
}

/// One held outer curve: what was asked and built, why less, and the
/// proofs along its two halves.
#[derive(Clone, Debug)]
pub struct Held {
    pub asked: Inner,
    pub built: Inner,
    pub why: Option<String>,
    pub halves: Vec<Continuity>,
}

#[allow(clippy::too_many_arguments)]
fn build(
    fill: &Nsided<'_>,
    mids: &[Mid],
    faces: &[Option<FaceRibbon>],
    leveled: &[(NurbsCurve, NurbsCurve)],
    centre: Vec3,
    nc: Vec3,
    ii_c: &dyn Fn(Vec3, Vec3) -> f64,
    extent: f64,
    inner: Inner,
    outer_g2: &[bool],
) -> Result<Built, NetworkError> {
    let n = mids.len();
    let tol = fill.tolerance;
    // Sides of the halves.
    let mut sides: Vec<(Side, Side)> = Vec::with_capacity(n);
    for (k, (a, b)) in leveled.iter().enumerate() {
        let label = format!("curve {}", k + 1);
        let mode = match (
            faces.get(k).and_then(Option::as_ref),
            outer_g2.get(k).copied().unwrap_or(false),
        ) {
            (Some(_), true) => Mode::Curvature,
            (Some(_), false) => Mode::Tangent,
            (None, _) => Mode::Free,
        };
        let face = faces.get(k).and_then(Option::as_ref);
        sides.push((
            Side::outer(a, face, mode, tol, &label)?,
            Side::outer(b, face, mode, tol, &label)?,
        ));
    }
    // Spokes: their ribbons from the paraboloid at the centre and from
    // each curve's face (or a plane across a free curve) at its midpoint.
    let mut spokes = Vec::with_capacity(n);
    let mut spoke_sides = Vec::with_capacity(n);
    for (k, mid) in mids.iter().enumerate() {
        let towards = mid.m - centre;
        let lambda = towards.length();
        let d = unit(towards - nc * towards.dot(nc), &format!("spoke {}", k + 1))?;
        let t0 = nc.cross(d);
        let (_, after) = sides.get(k).ok_or_else(super::internal)?;
        let jets = match faces.get(k).and_then(Option::as_ref) {
            Some(face) => face.jets_at_start(after, mid, lambda)?,
            None => FaceRibbon::Translation { d: mid.x }.jets_at_start(after, mid, lambda)?,
        };
        let zero = [0.0; 4];
        let q_s = [
            [centre.x, centre.y, centre.z, 1.0],
            h_of(d * lambda),
            h_of(nc * (lambda * lambda * ii_c(d, d))),
            zero,
            zero,
        ];
        let q_t = [h_of(t0), h_of(nc * (lambda * ii_c(d, t0))), zero, zero];
        let q_k = [h_of(nc * ii_c(t0, t0)), zero, zero];
        let s = two_piece(&q_s, &jets.s, 6)?;
        let t = two_piece(&q_t, &jets.t, 5)?;
        let kk = two_piece(&q_k, &jets.k, 4)?;
        let spoke = s.clone().curve()?;
        // The ribbon must stay across the spoke.
        for i in 0..=64 {
            let at = i as f64 / 64.0;
            let (_, dc) = spoke.derivatives(at);
            let tv = super::xyz(super::eval(&t, at)?);
            let sine = dc.cross(tv).length() / (dc.length() * tv.length()).max(f64::MIN_POSITIVE);
            if sine.is_nan() || sine < 10.0_f64.to_radians().psin() {
                return refuse(format!(
                    "spoke {} to the midpoint of curve {}: the plane the patches share along it turns within 10° of the spoke at {at:.3} (a loop too twisted for a centre point)",
                    k + 1,
                    k + 1
                ));
            }
        }
        spoke_sides.push(Side {
            c: s,
            f: Some(t),
            g: Some(kk),
            mode: match inner {
                Inner::Tangent => Mode::Tangent,
                Inner::Curvature => Mode::Curvature,
            },
        });
        spokes.push(spoke);
    }
    let mut patches = Vec::with_capacity(n);
    for k in 0..n {
        let next = (k + 1) % n;
        let (_, second) = sides.get(k).ok_or_else(super::internal)?;
        let (first_next, _) = sides.get(next).ok_or_else(super::internal)?;
        let four = [
            spoke_sides.get(k).ok_or_else(super::internal)?.clone(),
            first_next.reversed(),
            spoke_sides.get(next).ok_or_else(super::internal)?.clone(),
            second.clone(),
        ];
        let surface = quintic_coons(&four, extent, fill.angular).map_err(|e| {
            NetworkError::Refused(format!(
                "the patch at the corner of curves {} and {}: {e}",
                k + 1,
                next + 1
            ))
        })?;
        patches.push(surface);
    }
    // Every inner seam measured; every held outer curve measured.
    let mut inner_made = Vec::with_capacity(n);
    for k in 0..n {
        let a = patches.get((k + n - 1) % n).ok_or_else(super::internal)?;
        let b = patches.get(k).ok_or_else(super::internal)?;
        let spoke = spokes.get(k).ok_or_else(super::internal)?;
        let measured = continuity(
            &Surface::Nurbs(a.clone()),
            &Surface::Nurbs(b.clone()),
            spoke,
            fill.tolerance,
            fill.angular,
        )
        .map_err(|e| {
            NetworkError::Refused(format!("seam {} (spoke to curve {}): {e}", k + 1, k + 1))
        })?;
        inner_made.push((inner, measured));
    }
    let mut outer = Vec::with_capacity(n);
    for k in 0..n {
        let cond = fill.edges.get(k).ok_or_else(super::internal)?;
        let Some((face, _, _)) = cond.held() else {
            outer.push(None);
            continue;
        };
        let how = if outer_g2.get(k).copied().unwrap_or(false) {
            Inner::Curvature
        } else {
            Inner::Tangent
        };
        let (a, b) = leveled.get(k).ok_or_else(super::internal)?;
        let before = patches.get((k + n - 1) % n).ok_or_else(super::internal)?;
        let after = patches.get(k).ok_or_else(super::internal)?;
        let mut pair = Vec::with_capacity(2);
        for (patch, half) in [(before, a), (after, b)] {
            let measured = continuity(
                &Surface::Nurbs(patch.clone()),
                face,
                half,
                fill.tolerance,
                fill.angular,
            )
            .map_err(|e| NetworkError::Refused(format!("curve {} against its face: {e}", k + 1)))?;
            pair.push(measured);
        }
        outer.push(Some((how, pair)));
    }
    Ok(Built {
        patches,
        spokes,
        inner: inner_made,
        outer,
    })
}

/// How a side of a patch is held: free, tangent to its ribbon's face, or
/// in curvature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Free,
    Tangent,
    Curvature,
}

/// One side of a patch over `[0, 1]`, homogeneous: its curve `c`, and
/// where held the face about it as a ribbon: `f` its derivative across
/// (`R_ρ` along the curve) and `g` its second (`R_ρρ`).
#[derive(Clone, Debug)]
struct Side {
    c: Raw,
    f: Option<Raw>,
    g: Option<Raw>,
    mode: Mode,
}

impl Side {
    /// A half of an outer curve, with its face's ribbon where held.
    fn outer(
        curve: &NurbsCurve,
        face: Option<&FaceRibbon>,
        mode: Mode,
        tol: f64,
        label: &str,
    ) -> Result<Side, NetworkError> {
        let mut c = Raw::of(curve);
        c.normalize()?;
        let Some(face) = face else {
            return Ok(Side {
                c,
                f: None,
                g: None,
                mode: Mode::Free,
            });
        };
        let (c, f, g) = face.ribbon(curve, c, tol, label)?;
        Ok(Side {
            c,
            f: Some(f),
            g: Some(g),
            mode,
        })
    }

    fn reversed(&self) -> Side {
        Side {
            c: super::reverse(&self.c),
            f: self.f.as_ref().map(super::reverse),
            g: self.g.as_ref().map(super::reverse),
            mode: self.mode,
        }
    }
}

/// A face written exactly about a curve on it: the curve is its line
/// `ρ = 0`. Rotations and translations of the curve for the analytic
/// kinds (homogeneous: a rotation by `2 atan t` is rational of degree two
/// in `t`), the face itself along one of its parameter lines for a
/// spline.
#[derive(Clone, Debug)]
enum FaceRibbon {
    Rotation { o: Vec3, a: Vec3 },
    Translation { d: Vec3 },
    Scale { apex: Vec3 },
    Spline { face: NurbsSurface },
}

/// The jets a spoke takes at its midpoint end (`s = 1`), from the face
/// about its curve: the spoke's point and derivatives (to the fourth),
/// its ribbon's field across (to the third) and second field (to the
/// second), homogeneous.
struct Jets {
    s: [H; 5],
    t: [H; 4],
    k: [H; 3],
}

impl FaceRibbon {
    /// The ribbon that writes `face` about `curve` best: of the
    /// candidates, the one whose field across stays furthest from the
    /// curve's own direction (at least 5°).
    fn about(face: &Surface, curve: &NurbsCurve, label: &str) -> Result<FaceRibbon, NetworkError> {
        let (a, b) = curve.domain();
        let samples: Vec<(Vec3, Vec3)> = (0..=16)
            .map(|i| curve.derivatives(a + (b - a) * f64::from(i) / 16.0))
            .collect();
        let spread = |field: &dyn Fn(Vec3) -> Vec3| -> f64 {
            samples
                .iter()
                .map(|(p, d)| {
                    let f = field(*p);
                    d.cross(f).length() / (d.length() * f.length()).max(f64::MIN_POSITIVE)
                })
                .fold(f64::INFINITY, f64::min)
        };
        let candidates: Vec<FaceRibbon> = match face {
            Surface::Plane(p) => {
                let n = p.frame().z();
                let chord = curve.eval(b) - curve.eval(a);
                let along = if chord.length() > 0.0 {
                    chord
                } else {
                    curve.derivatives(0.5 * (a + b)).1
                };
                vec![FaceRibbon::Translation {
                    d: unit(n.cross(along), label)?,
                }]
            }
            Surface::Cylinder(cy) => vec![
                FaceRibbon::Translation { d: cy.frame().z() },
                FaceRibbon::Rotation {
                    o: cy.frame().origin(),
                    a: cy.frame().z(),
                },
            ],
            Surface::Cone(co) => vec![
                FaceRibbon::Scale {
                    apex: co.frame().point(0.0, 0.0, co.apex_v()),
                },
                FaceRibbon::Rotation {
                    o: co.frame().origin(),
                    a: co.frame().z(),
                },
            ],
            Surface::Sphere(s) => {
                let c = s.frame().origin();
                let mut m = Vec3::ZERO;
                for (p, d) in &samples {
                    m += (*p - c).cross(*d);
                }
                let q = curve.eval(0.5 * (a + b)) - c;
                let axis = m
                    .normalized()
                    .and_then(|m| m.cross(q).normalized())
                    .or_else(|| q.cross(Vec3::new(1.0, 0.0, 0.0)).normalized())
                    .ok_or_else(super::internal)?;
                vec![FaceRibbon::Rotation { o: c, a: axis }]
            }
            Surface::Nurbs(nu) => vec![FaceRibbon::Spline { face: nu.clone() }],
            Surface::Bezier(bz) => vec![FaceRibbon::Spline {
                face: NurbsSurface::from_bezier(bz)?,
            }],
            Surface::Torus(_) => {
                return refuse(format!(
                    "{label} is held to a torus; a fill held to a torus is not built yet (the continuity measure does not compare with a torus)"
                ));
            }
        };
        let mut best: Option<(f64, FaceRibbon)> = None;
        for c in candidates {
            let s = match &c {
                FaceRibbon::Spline { .. } => 1.0,
                other => spread(&|p| other.across(p)),
            };
            if best.as_ref().is_none_or(|(b, _)| s > *b) {
                best = Some((s, c));
            }
        }
        let (s, c) = best.ok_or_else(super::internal)?;
        if s < 5.0_f64.to_radians().psin() {
            return refuse(format!(
                "{label}: its face cannot be written about it (every way across runs within 5° along it)"
            ));
        }
        Ok(c)
    }

    /// The direction across at a point (analytic kinds).
    fn across(&self, p: Vec3) -> Vec3 {
        match self {
            FaceRibbon::Rotation { o, a } => a.cross(p - *o),
            FaceRibbon::Translation { d } => *d,
            FaceRibbon::Scale { apex } => p - *apex,
            FaceRibbon::Spline { .. } => Vec3::ZERO,
        }
    }

    /// The side's curve and its fields `R_ρ`, `R_ρρ` along it.
    fn ribbon(
        &self,
        curve: &NurbsCurve,
        c: Raw,
        tol: f64,
        label: &str,
    ) -> Result<(Raw, Raw, Raw), NetworkError> {
        let zero = |c: &Raw| super::mapped(c, |_| [0.0; 4]);
        Ok(match self {
            FaceRibbon::Rotation { o, a } => {
                let (o, a) = (*o, *a);
                let f = super::mapped(&c, |h| {
                    let x = super::xyz(h) - o * h[3];
                    h_of(a.cross(x) * 2.0)
                });
                let g = super::mapped(&c, |h| {
                    let x = super::xyz(h) - o * h[3];
                    let v = (a * (2.0 * a.dot(x)) - x) * 2.0 + o * (2.0 * h[3]);
                    [v.x, v.y, v.z, 2.0 * h[3]]
                });
                (c, f, g)
            }
            FaceRibbon::Translation { d } => {
                let d = *d;
                let f = super::mapped(&c, |h| [d.x * h[3], d.y * h[3], d.z * h[3], 0.0]);
                let g = zero(&c);
                (c, f, g)
            }
            FaceRibbon::Scale { apex } => {
                let p = *apex;
                let f = super::mapped(&c, |h| {
                    [h[0] - p.x * h[3], h[1] - p.y * h[3], h[2] - p.z * h[3], 0.0]
                });
                let g = zero(&c);
                (c, f, g)
            }
            FaceRibbon::Spline { face } => {
                let track = super::Track::new(curve, tol)?;
                let Some([iso, field, second]) = super::iso_track(face, &track, tol)? else {
                    return refuse(format!(
                        "{label}: it does not run along one of its face's parameter lines; a fill held to a spline face along any other curve is not built yet"
                    ));
                };
                let (w0, w1) = (
                    iso.points.first().map_or(0.0, |h| h[3]),
                    iso.points.last().map_or(0.0, |h| h[3]),
                );
                if (w0 - 1.0).abs() > 1e-12 || (w1 - 1.0).abs() > 1e-12 {
                    return refuse(format!(
                        "{label} runs along a rational spline face's line whose end weights are not 1; not supported yet"
                    ));
                }
                (iso, field, second)
            }
        })
    }

    /// The jets of the spoke arriving at the midpoint `mid`, where the
    /// side `after` (the curve's second half, from the midpoint) starts:
    /// the spoke runs along the face's line across (`ρ`) through the
    /// midpoint, into the fill, at the speed `lambda`; its ribbon along
    /// the face's other direction (`σ`, the curve's).
    fn jets_at_start(&self, after: &Side, mid: &Mid, lambda: f64) -> Result<Jets, NetworkError> {
        // r[j][i] = ∂σ^j ∂ρ^i R at the midpoint, homogeneous.
        let mut r = [[[0.0; 4]; 5]; 3];
        match self {
            FaceRibbon::Spline { face } => {
                let p = super::point(super::eval(&after.c, 0.0)?).ok_or_else(super::internal)?;
                let (u, v) = Surface::Nurbs(face.clone()).project(p)?;
                let (_, su, sv) = face.derivatives(u, v);
                let along_u = su.normalized().map_or(0.0, |x| x.dot(mid.e).abs())
                    >= sv.normalized().map_or(0.0, |x| x.dot(mid.e).abs());
                for (j, row) in r.iter_mut().enumerate() {
                    for (i, slot) in row.iter_mut().enumerate() {
                        if j + i > 4 {
                            continue;
                        }
                        let (du, dv) = if along_u { (j, i) } else { (i, j) };
                        *slot = spline_partial(face, u, v, du, dv)?;
                    }
                }
            }
            other => {
                // σ is the side's own parameter, at its start.
                let mut xs = [[0.0; 4]; 3];
                let mut d = after.c.clone();
                for slot in &mut xs {
                    *slot = super::eval(&d, 0.0)?;
                    d = super::derivative(&d)?;
                }
                for (j, xw) in xs.iter().enumerate() {
                    let (x, w) = (super::xyz(*xw), xw[3]);
                    for i in 0..5 {
                        if j + i > 4 {
                            continue;
                        }
                        let h = match other {
                            FaceRibbon::Rotation { o, a } => {
                                let rel = x - *o * w;
                                let e = [1.0, 0.0, 2.0, 0.0, 0.0].get(i).copied().unwrap_or(0.0);
                                let turned = match i {
                                    0 => rel,
                                    1 => a.cross(rel) * 2.0,
                                    2 => (*a * (2.0 * a.dot(rel)) - rel) * 2.0,
                                    _ => Vec3::ZERO,
                                };
                                let v = turned + *o * (w * e);
                                [v.x, v.y, v.z, w * e]
                            }
                            FaceRibbon::Translation { d } => match i {
                                0 => *xw,
                                1 => [d.x * w, d.y * w, d.z * w, 0.0],
                                _ => [0.0; 4],
                            },
                            FaceRibbon::Scale { apex } => match i {
                                0 => *xw,
                                1 => {
                                    let v = x - *apex * w;
                                    [v.x, v.y, v.z, 0.0]
                                }
                                _ => [0.0; 4],
                            },
                            FaceRibbon::Spline { .. } => [0.0; 4],
                        };
                        if let Some(slot) = r.get_mut(j).and_then(|row| row.get_mut(i)) {
                            *slot = h;
                        }
                    }
                }
            }
        }
        let projected = |h: H, at: H| -> Vec3 { super::xyz(h) - super::xyz(at) * (h[3] / at[3]) };
        let [r0, r1, r2] = r;
        let base = r0[0];
        if !(base[3].is_finite() && base[3] > 0.0) {
            return Err(super::internal());
        }
        // ρ into the fill, σ along the curve.
        let across = projected(r0[1], base);
        let rho = if across.dot(mid.x) < 0.0 { -1.0 } else { 1.0 };
        let speed = across.length() / base[3];
        if !(speed > 0.0 && speed.is_finite()) {
            return refuse("a held face has no direction across a curve at its midpoint".into());
        }
        let along = projected(r1[0], base);
        let sigma = if along.dot(mid.e) < 0.0 { -1.0 } else { 1.0 };
        let kappa = sigma * base[3] / along.length().max(f64::MIN_POSITIVE);
        let ell = lambda / speed;
        // d/ds = −ℓ ρ ∂ρ: the spoke arrives at s = 1 moving out of the fill.
        let f = |i: usize| -> f64 { (-ell * rho).ppowi(i as i32) };
        let scale = |h: H, x: f64| [h[0] * x, h[1] * x, h[2] * x, h[3] * x];
        Ok(Jets {
            s: [
                r0[0],
                scale(r0[1], f(1)),
                scale(r0[2], f(2)),
                scale(r0[3], f(3)),
                scale(r0[4], f(4)),
            ],
            t: [
                scale(r1[0], kappa),
                scale(r1[1], kappa * f(1)),
                scale(r1[2], kappa * f(2)),
                scale(r1[3], kappa * f(3)),
            ],
            k: [
                scale(r2[0], kappa * kappa),
                scale(r2[1], kappa * kappa * f(1)),
                scale(r2[2], kappa * kappa * f(2)),
            ],
        })
    }
}

/// `∂u^i ∂v^j` of the homogeneous spline surface at `(u, v)` (its Bézier
/// span there, continued past a border: the derivatives a face continued
/// across its edge has).
fn spline_partial(
    face: &NurbsSurface,
    u: f64,
    v: f64,
    i: usize,
    j: usize,
) -> Result<H, NetworkError> {
    let spans = face.bezier_spans()?;
    let span = spans
        .iter()
        .min_by(|a, b| {
            let d = |s: &crate::BezierSpan| {
                let du = if u < s.u.0 {
                    s.u.0 - u
                } else if u > s.u.1 {
                    u - s.u.1
                } else {
                    0.0
                };
                let dv = if v < s.v.0 {
                    s.v.0 - v
                } else if v > s.v.1 {
                    v - s.v.1
                } else {
                    0.0
                };
                du + dv
            };
            d(a).total_cmp(&d(b))
        })
        .ok_or_else(super::internal)?;
    let (p, q) = span.patch.degrees();
    let pts: Vec<H> = span
        .patch
        .control_points()
        .iter()
        .zip(span.patch.weights())
        .map(|(x, w)| [x.x * w, x.y * w, x.z * w, w])
        .collect();
    let (wu, wv) = (span.u.1 - span.u.0, span.v.1 - span.v.0);
    let (s, t) = ((u - span.u.0) / wu, (v - span.v.0) / wv);
    // Differences i times along u, j times along v.
    let mut net: Vec<Vec<H>> = (0..=p)
        .map(|a| {
            (0..=q)
                .map(|b| pts.get(a * (q + 1) + b).copied().unwrap_or([0.0; 4]))
                .collect()
        })
        .collect();
    let mut factor = 1.0;
    for k in 0..i {
        let m = p - k;
        if m == 0 {
            return Ok([0.0; 4]);
        }
        factor *= m as f64 / wu;
        net = net
            .windows(2)
            .map(|w| match w {
                [a, b] => a
                    .iter()
                    .zip(b)
                    .map(|(x, y)| super::h_add(*y, *x, 1.0, -1.0))
                    .collect(),
                _ => Vec::new(),
            })
            .collect();
    }
    for k in 0..j {
        let m = q - k;
        if m == 0 {
            return Ok([0.0; 4]);
        }
        factor *= m as f64 / wv;
        net = net
            .iter()
            .map(|row| {
                row.windows(2)
                    .map(|w| match w {
                        [a, b] => super::h_add(*b, *a, 1.0, -1.0),
                        _ => [0.0; 4],
                    })
                    .collect()
            })
            .collect();
    }
    // Evaluate the net at (s, t) by de Casteljau.
    let casteljau = |row: &[H], x: f64| -> H {
        let mut r = row.to_vec();
        while r.len() > 1 {
            r = r
                .windows(2)
                .map(|w| match w {
                    [a, b] => super::h_add(*a, *b, 1.0 - x, x),
                    _ => [0.0; 4],
                })
                .collect();
        }
        r.first().copied().unwrap_or([0.0; 4])
    };
    let column: Vec<H> = net.iter().map(|row| casteljau(row, t)).collect();
    let h = casteljau(&column, s);
    Ok([h[0] * factor, h[1] * factor, h[2] * factor, h[3] * factor])
}

/// The spline of two pieces of degree `m` over `[0, ½]` and `[½, 1]`
/// with the given jets (value and derivatives, homogeneous) at 0 and at
/// 1, `m − 1` of each, joined with three continuous derivatives in the
/// middle (closed form: the two control points nearest the join of each
/// piece are free, four unknowns for four conditions).
fn two_piece(start: &[H], end: &[H], m: usize) -> Result<Raw, NetworkError> {
    if start.len() + 1 != m || end.len() + 1 != m || m < 4 {
        return Err(super::internal());
    }
    let falling = |k: usize| -> f64 { (0..k).fold(1.0, |acc, i| acc * (m - i) as f64) };
    let binom = |n: usize, k: usize| -> f64 {
        (0..k).fold(1.0, |acc, i| acc * (n - i) as f64 / (i + 1) as f64)
    };
    let mut p1 = vec![[0.0; 4]; m + 1];
    let mut p2 = vec![[0.0; 4]; m + 1];
    for i in 0..start.len() {
        let mut b0 = [0.0; 4];
        let mut b1 = [0.0; 4];
        for k in 0..=i {
            let half = 0.5_f64.ppowi(k as i32);
            let c = binom(i, k) / falling(k) * half;
            if let (Some(js), Some(je)) = (start.get(k), end.get(k)) {
                b0 = super::h_add(b0, *js, 1.0, c);
                b1 = super::h_add(b1, *je, 1.0, if k % 2 == 1 { -c } else { c });
            }
        }
        if let Some(slot) = p1.get_mut(i) {
            *slot = b0;
        }
        if let Some(slot) = p2.get_mut(m - i) {
            *slot = b1;
        }
    }
    let at = |v: &[H], i: usize| v.get(i).copied().unwrap_or([0.0; 4]);
    let (pp, pp3, q2, q3) = (at(&p1, m - 2), at(&p1, m - 3), at(&p2, 2), at(&p2, 3));
    let sum = |terms: &[(H, f64)]| {
        terms
            .iter()
            .fold([0.0; 4], |acc, (h, f)| super::h_add(acc, *h, 1.0, *f))
    };
    let x = sum(&[(pp, 0.75), (q2, 0.75), (pp3, -0.25), (q3, -0.25)]);
    let y = sum(&[(x, 1.0), (q2, -0.25), (pp, 0.25)]);
    let z = sum(&[(x, 2.0), (y, -1.0)]);
    if let Some(slot) = p1.get_mut(m - 1) {
        *slot = y;
    }
    if let Some(slot) = p1.get_mut(m) {
        *slot = x;
    }
    if let Some(slot) = p2.get_mut(0) {
        *slot = x;
    }
    if let Some(slot) = p2.get_mut(1) {
        *slot = z;
    }
    Ok(crate::refine::from_pieces_raw(
        m,
        &[0.0, 0.5, 1.0],
        &[p1, p2],
    )?)
}

/// The curve with end weights 1: the same curve at the same parameters,
/// its homogeneous points multiplied by the linear `λ(t)` that is
/// `1 / w` at each end (positive between: a scalar multiple of a
/// homogeneous curve is the same curve). One degree higher where it was
/// rational with other end weights.
fn ends_of_weight_one(curve: &NurbsCurve, label: &str) -> Result<NurbsCurve, NetworkError> {
    let w = curve.weights();
    let (a, b) = (
        w.first().copied().unwrap_or(1.0),
        w.last().copied().unwrap_or(1.0),
    );
    if a == 1.0 && b == 1.0 {
        return Ok(curve.clone());
    }
    if !(a > 0.0 && b > 0.0 && a.is_finite() && b.is_finite()) {
        return refuse(format!("{label}: its end weights are not positive"));
    }
    let (t0, t1) = curve.domain();
    let lambda = Raw {
        degree: 1,
        knots: vec![t0, t0, t1, t1],
        points: vec![[1.0 / a, 0.0, 0.0, 0.0], [1.0 / b, 0.0, 0.0, 0.0]],
    };
    let mut made = super::times(&lambda, &Raw::of(curve))?;
    // Exactly 1 at the ends (the products' rounding taken off).
    for i in [0, made.points.len().saturating_sub(1)] {
        if let Some(h) = made.points.get_mut(i) {
            let w = h[3];
            if w > 0.0 {
                *h = [h[0] / w, h[1] / w, h[2] / w, 1.0];
            }
        }
    }
    Ok(made.curve()?)
}

fn h_of(v: Vec3) -> H {
    [v.x, v.y, v.z, 0.0]
}

fn h_sub(a: H, b: H) -> H {
    super::h_add(a, b, 1.0, -1.0)
}

/// A side's curve and fields at one end: `c` to the fourth derivative,
/// `f` to the third, `g` to the second (zero where it has none).
struct End {
    c: [H; 5],
    f: [H; 4],
    g: [H; 3],
}

impl End {
    fn of(side: &Side, t: f64) -> Result<End, NetworkError> {
        let mut c = [[0.0; 4]; 5];
        let mut d = side.c.clone();
        for slot in &mut c {
            *slot = super::eval(&d, t)?;
            d = super::derivative(&d)?;
        }
        let mut f = [[0.0; 4]; 4];
        if let Some(raw) = &side.f {
            let mut d = raw.clone();
            for slot in &mut f {
                *slot = super::eval(&d, t)?;
                d = super::derivative(&d)?;
            }
        }
        let mut g = [[0.0; 4]; 3];
        if let Some(raw) = &side.g {
            let mut d = raw.clone();
            for slot in &mut g {
                *slot = super::eval(&d, t)?;
                d = super::derivative(&d)?;
            }
        }
        Ok(End { c, f, g })
    }

    /// The columns `[f, c′, c]` and their derivatives of order `o`.
    fn cols(&self, o: usize) -> [H; 3] {
        let at = |v: &[H], i: usize| v.get(i).copied().unwrap_or([0.0; 4]);
        [at(&self.f, o), at(&self.c, o + 1), at(&self.c, o)]
    }
}

fn add_scaled(acc: H, cols: &[H; 3], s: &[f64; 3], by: f64) -> H {
    cols.iter()
        .zip(s)
        .fold(acc, |acc, (c, x)| super::h_add(acc, *c, 1.0, x * by))
}

fn coefficients3(a: &[H; 3], y: H, centre: Vec3, length: f64, cutoff: f64) -> ([f64; 3], H) {
    let (s, left) = min_norm(a, y, centre, length, cutoff);
    let mut out = [0.0; 3];
    for (o, x) in out.iter_mut().zip(s) {
        *o = x;
    }
    (out, left)
}

/// The scalars of a held side's field across at one end, from the field
/// and its first two derivatives there (`D = a f + b c′ + c c`), each
/// derivative moved by the least that makes it one of such a field
/// (returned with the scalars: value, slope, second derivative).
struct Scalars {
    s: [[f64; 3]; 3],
    d: [H; 3],
}

struct Frame3 {
    centre: Vec3,
    length: f64,
    cutoff: f64,
}

fn d_level(end: &End, d: [H; 3], fr: &Frame3) -> Scalars {
    let a0 = end.cols(0);
    let a1 = end.cols(1);
    let a2 = end.cols(2);
    let (s0, _) = coefficients3(&a0, d[0], fr.centre, fr.length, fr.cutoff);
    let y1 = add_scaled(d[1], &a1, &s0, -1.0);
    let (s1, l1) = coefficients3(&a0, y1, fr.centre, fr.length, fr.cutoff);
    let d1 = h_sub(d[1], l1);
    let y2 = add_scaled(add_scaled(d[2], &a1, &s1, -2.0), &a2, &s0, -1.0);
    let (s2, l2) = coefficients3(&a0, y2, fr.centre, fr.length, fr.cutoff);
    let d2 = h_sub(d[2], l2);
    Scalars {
        s: [s0, s1, s2],
        d: [d[0], d1, d2],
    }
}

/// `T = b² c″ + 2ab f′ + a² g` and its first two derivatives at an end,
/// from the scalars `a` (of `f`) and `b` (of `c′`) there.
fn t_jets(end: &End, s: &[[f64; 3]; 3]) -> [H; 3] {
    let [s0, s1, s2] = s;
    let (a0, a1, a2) = (s0[0], s1[0], s2[0]);
    let (b0, b1, b2) = (s0[1], s1[1], s2[1]);
    let c = &end.c;
    let f = &end.f;
    let g = &end.g;
    let at = |v: &[H], i: usize| v.get(i).copied().unwrap_or([0.0; 4]);
    let sum = |terms: &[(H, f64)]| {
        terms
            .iter()
            .fold([0.0; 4], |acc, (h, x)| super::h_add(acc, *h, 1.0, *x))
    };
    let t0 = sum(&[
        (at(c, 2), b0 * b0),
        (at(f, 1), 2.0 * a0 * b0),
        (at(g, 0), a0 * a0),
    ]);
    let t1 = sum(&[
        (at(c, 2), 2.0 * b0 * b1),
        (at(c, 3), b0 * b0),
        (at(f, 1), 2.0 * (a1 * b0 + a0 * b1)),
        (at(f, 2), 2.0 * a0 * b0),
        (at(g, 0), 2.0 * a0 * a1),
        (at(g, 1), a0 * a0),
    ]);
    let t2 = sum(&[
        (at(c, 2), 2.0 * (b1 * b1 + b0 * b2)),
        (at(c, 3), 4.0 * b0 * b1),
        (at(c, 4), b0 * b0),
        (at(f, 1), 2.0 * (a2 * b0 + 2.0 * a1 * b1 + a0 * b2)),
        (at(f, 2), 4.0 * (a1 * b0 + a0 * b1)),
        (at(f, 3), 2.0 * a0 * b0),
        (at(g, 0), 2.0 * (a1 * a1 + a0 * a2)),
        (at(g, 1), 4.0 * a0 * a1),
        (at(g, 2), a0 * a0),
    ]);
    [t0, t1, t2]
}

/// The scalars of a curvature side's second field across at one end:
/// `E = T + α c + β c′ + γ f`, each derivative of `E` moved by the least
/// that makes it one of such a field.
fn e_level(end: &End, e: [H; 3], t: &[H; 3], fr: &Frame3) -> Scalars {
    let a0 = end.cols(0);
    let a1 = end.cols(1);
    let a2 = end.cols(2);
    let (r0, _) = coefficients3(&a0, h_sub(e[0], t[0]), fr.centre, fr.length, fr.cutoff);
    let y1 = add_scaled(h_sub(e[1], t[1]), &a1, &r0, -1.0);
    let (r1, l1) = coefficients3(&a0, y1, fr.centre, fr.length, fr.cutoff);
    let e1 = h_sub(e[1], l1);
    let y2 = add_scaled(
        add_scaled(h_sub(e[2], t[2]), &a1, &r1, -2.0),
        &a2,
        &r0,
        -1.0,
    );
    let (r2, l2) = coefficients3(&a0, y2, fr.centre, fr.length, fr.cutoff);
    let e2 = h_sub(e[2], l2);
    Scalars {
        s: [r0, r1, r2],
        d: [e[0], e1, e2],
    }
}

/// The jets of a patch at a corner: `j[k][l] = ∂u^k ∂v^l S`, `k, l ≤ 2`.
type Jet = [[H; 3]; 3];

/// The quintic Hermite of homogeneous values, slopes and second
/// derivatives at 0 and 1.
fn quintic_h(a: [H; 3], b: [H; 3]) -> Raw {
    let p = |h: &[(H, f64)]| {
        h.iter()
            .fold([0.0; 4], |acc, (x, f)| super::h_add(acc, *x, 1.0, *f))
    };
    Raw {
        degree: 5,
        knots: [vec![0.0; 6], vec![1.0; 6]].concat(),
        points: vec![
            a[0],
            p(&[(a[0], 1.0), (a[1], 0.2)]),
            p(&[(a[0], 1.0), (a[1], 0.4), (a[2], 0.05)]),
            p(&[(b[0], 1.0), (b[1], -0.4), (b[2], 0.05)]),
            p(&[(b[0], 1.0), (b[1], -0.2)]),
            b[0],
        ],
    }
}

/// Column `i` of the scalars' values, slopes and second derivatives.
fn column(at: &[[f64; 3]; 3], i: usize) -> [f64; 3] {
    let pick = |o: usize| at.get(o).and_then(|r| r.get(i)).copied().unwrap_or(0.0);
    [pick(0), pick(1), pick(2)]
}

fn quintic_scalar(a: [f64; 3], b: [f64; 3]) -> Raw {
    quintic_h(a.map(|x| [x, 0.0, 0.0, 0.0]), b.map(|x| [x, 0.0, 0.0, 0.0]))
}

/// `Σ sᵢ(t) Aᵢ(t)` over the side's columns `[f, c′, c]`, the scalars
/// quintic from their values at the two ends.
fn field_of(side: &Side, at0: &[[f64; 3]; 3], at1: &[[f64; 3]; 3]) -> Result<Raw, NetworkError> {
    let f = side.f.clone().ok_or_else(super::internal)?;
    let cols = [f, super::derivative(&side.c)?, side.c.clone()];
    let mut total: Option<Raw> = None;
    for (i, col) in cols.iter().enumerate() {
        let (v0, v1) = (column(at0, i), column(at1, i));
        if v0.iter().chain(&v1).all(|x| *x == 0.0) {
            continue;
        }
        let term = super::times(&quintic_scalar(v0, v1), col)?;
        total = Some(match total {
            None => term,
            Some(t) => super::combined(&t, &term, 1.0, 1.0)?,
        });
    }
    Ok(total.unwrap_or_else(|| super::mapped(&side.c, |_| [0.0; 4])))
}

/// `T = b² c″ + 2ab f′ + a² g` along a curvature side, exactly.
fn t_field(side: &Side, at0: &[[f64; 3]; 3], at1: &[[f64; 3]; 3]) -> Result<Raw, NetworkError> {
    let f = side.f.clone().ok_or_else(super::internal)?;
    let g = side.g.clone().ok_or_else(super::internal)?;
    let pick = |i: usize| quintic_scalar(column(at0, i), column(at1, i));
    let (a, b) = (pick(0), pick(1));
    let c2 = super::derivative(&super::derivative(&side.c)?)?;
    let f1 = super::derivative(&f)?;
    let bb = super::times(&b, &b)?;
    let ab = super::times(&a, &b)?;
    let aa = super::times(&a, &a)?;
    let t1 = super::times(&bb, &c2)?;
    let t2 = super::times(&ab, &f1)?;
    let t3 = super::times(&aa, &g)?;
    super::combined(&super::combined(&t1, &t2, 1.0, 2.0)?, &t3, 1.0, 1.0)
}

/// The biquintically blended Coons patch (the Boolean sum of the quintic
/// Hermite blends of the four sides with their first and second
/// cross-boundary fields, less the tensor quintic of the corner jets) of
/// four sides `v = 0`, `v = 1`, `u = 0`, `u = 1`, each over `[0, 1]`
/// running with its parameter. A held side's first field lies in its
/// face's tangent planes all along and a curvature side's second field
/// gives the face's normal curvature across, exactly (combinations of
/// the face's ribbon); the corner jets are made to agree with every held
/// side meeting there, order by order (twist, then the third mixed
/// derivatives, then the fourth), each moved by the least.
fn quintic_coons(
    sides: &[Side; 4],
    length: f64,
    cutoff: f64,
) -> Result<NurbsSurface, NetworkError> {
    let ends: Vec<[End; 2]> = sides
        .iter()
        .map(|s| Ok([End::of(s, 0.0)?, End::of(s, 1.0)?]))
        .collect::<Result<_, NetworkError>>()?;
    let end = |side: usize, t: usize| -> Result<&End, NetworkError> {
        ends.get(side)
            .and_then(|e| e.get(t))
            .ok_or_else(super::internal)
    };
    let mode = |side: usize| sides.get(side).map_or(Mode::Free, |s| s.mode);
    let centre = super::point(end(0, 0)?.c[0]).ok_or_else(super::internal)?;
    let fr = Frame3 {
        centre,
        length,
        cutoff,
    };
    // The corner jets: corner (i, j) is where side j (along u, at v = j)
    // meets side 2 + i (along v, at u = i).
    let mut jets: [[Jet; 2]; 2] = [[[[[0.0; 4]; 3]; 3]; 2]; 2];
    for i in 0..2 {
        for j in 0..2 {
            let (sa, sb) = (j, 2 + i);
            let (ea, eb) = (end(sa, i)?, end(sb, j)?);
            let mut jet: Jet = [[[0.0; 4]; 3]; 3];
            jet[0][0] = ea.c[0];
            jet[1][0] = ea.c[1];
            jet[2][0] = ea.c[2];
            jet[0][1] = eb.c[1];
            jet[0][2] = eb.c[2];
            // The bilinear Coons patch's twist to start from.
            let (o0, o1) = (end(0, i)?, end(1, i)?);
            let (p0, p1) = (end(2, j)?, end(3, j)?);
            let mut twist = h_sub(o1.c[1], o0.c[1]);
            twist = super::h_add(twist, h_sub(p1.c[1], p0.c[1]), 1.0, 1.0);
            let (c00, c10, c01, c11) = (
                end(0, 0)?.c[0],
                end(0, 1)?.c[0],
                end(1, 0)?.c[0],
                end(1, 1)?.c[0],
            );
            twist = super::h_add(twist, h_sub(c00, c10), 1.0, -1.0);
            twist = super::h_add(twist, h_sub(c11, c01), 1.0, -1.0);
            jet[1][1] = twist;
            let held = |m: Mode| m != Mode::Free;
            let curv = |m: Mode| m == Mode::Curvature;
            for _ in 0..6 {
                if held(mode(sa)) {
                    jet[1][1] = d_level(ea, [jet[0][1], jet[1][1], jet[2][1]], &fr).d[1];
                }
                if held(mode(sb)) {
                    jet[1][1] = d_level(eb, [jet[1][0], jet[1][1], jet[1][2]], &fr).d[1];
                }
            }
            for _ in 0..6 {
                if held(mode(sa)) {
                    jet[2][1] = d_level(ea, [jet[0][1], jet[1][1], jet[2][1]], &fr).d[2];
                }
                if curv(mode(sb)) {
                    let sc = d_level(eb, [jet[1][0], jet[1][1], jet[1][2]], &fr);
                    let t = t_jets(eb, &sc.s);
                    jet[2][1] = e_level(eb, [jet[2][0], jet[2][1], jet[2][2]], &t, &fr).d[1];
                }
                if curv(mode(sa)) {
                    let sc = d_level(ea, [jet[0][1], jet[1][1], jet[2][1]], &fr);
                    let t = t_jets(ea, &sc.s);
                    jet[1][2] = e_level(ea, [jet[0][2], jet[1][2], jet[2][2]], &t, &fr).d[1];
                }
                if held(mode(sb)) {
                    jet[1][2] = d_level(eb, [jet[1][0], jet[1][1], jet[1][2]], &fr).d[2];
                }
            }
            for _ in 0..6 {
                if curv(mode(sa)) {
                    let sc = d_level(ea, [jet[0][1], jet[1][1], jet[2][1]], &fr);
                    let t = t_jets(ea, &sc.s);
                    jet[2][2] = e_level(ea, [jet[0][2], jet[1][2], jet[2][2]], &t, &fr).d[2];
                }
                if curv(mode(sb)) {
                    let sc = d_level(eb, [jet[1][0], jet[1][1], jet[1][2]], &fr);
                    let t = t_jets(eb, &sc.s);
                    jet[2][2] = e_level(eb, [jet[2][0], jet[2][1], jet[2][2]], &t, &fr).d[2];
                }
            }
            if let Some(slot) = jets.get_mut(i).and_then(|r| r.get_mut(j)) {
                *slot = jet;
            }
        }
    }
    // The fields across each side, from its two corners.
    let data = |side: usize, t: usize| -> ([H; 3], [H; 3]) {
        let (i, j) = if side < 2 { (t, side) } else { (side - 2, t) };
        let empty: Jet = [[[0.0; 4]; 3]; 3];
        let jet = jets.get(i).and_then(|r| r.get(j)).unwrap_or(&empty);
        if side < 2 {
            (
                [jet[0][1], jet[1][1], jet[2][1]],
                [jet[0][2], jet[1][2], jet[2][2]],
            )
        } else {
            (
                [jet[1][0], jet[1][1], jet[1][2]],
                [jet[2][0], jet[2][1], jet[2][2]],
            )
        }
    };
    let mut fields: Vec<(Raw, Raw)> = Vec::with_capacity(4);
    for (k, side) in sides.iter().enumerate() {
        let (d0, e0) = data(k, 0);
        let (d1, e1) = data(k, 1);
        let (en0, en1) = (end(k, 0)?, end(k, 1)?);
        let (d_raw, e_raw) = match side.mode {
            Mode::Free => (quintic_h(d0, d1), quintic_h(e0, e1)),
            Mode::Tangent => {
                let (a, b) = (d_level(en0, d0, &fr), d_level(en1, d1, &fr));
                (field_of(side, &a.s, &b.s)?, quintic_h(e0, e1))
            }
            Mode::Curvature => {
                let (a, b) = (d_level(en0, d0, &fr), d_level(en1, d1, &fr));
                let d_raw = field_of(side, &a.s, &b.s)?;
                let (ta, tb) = (t_jets(en0, &a.s), t_jets(en1, &b.s));
                let (ra, rb) = (e_level(en0, e0, &ta, &fr), e_level(en1, e1, &tb, &fr));
                let t_raw = t_field(side, &a.s, &b.s)?;
                let rest = field_of(side, &ra.s, &rb.s)?;
                (d_raw, super::combined(&t_raw, &rest, 1.0, 1.0)?)
            }
        };
        fields.push((d_raw, e_raw));
    }
    let field = |k: usize| fields.get(k).ok_or_else(super::internal);
    // P1: along u, quintic Hermite in v; P2 likewise transposed.
    let rows_of =
        |a: &Raw, da: &Raw, ea: &Raw, b: &Raw, db: &Raw, eb: &Raw| -> Result<Net, NetworkError> {
            let r1 = super::combined(a, da, 1.0, 0.2)?;
            let r2 = super::combined(&super::combined(a, da, 1.0, 0.4)?, ea, 1.0, 0.05)?;
            let r3 = super::combined(&super::combined(b, db, 1.0, -0.4)?, eb, 1.0, 0.05)?;
            let r4 = super::combined(b, db, 1.0, -0.2)?;
            let mut six = vec![a.clone(), r1, r2, r3, r4, b.clone()];
            let refs: Vec<&Raw> = six.iter().collect();
            let (degree, target) = super::union(&refs)?;
            for r in &mut six {
                super::refine_to(r, degree, &target)?;
            }
            let first = six.first().ok_or_else(super::internal)?;
            let count = first.points.len();
            let mut rows = Vec::with_capacity(count);
            for i in 0..count {
                let mut row = Vec::with_capacity(6);
                for r in &six {
                    row.push(*r.points.get(i).ok_or_else(super::internal)?);
                }
                rows.push(row);
            }
            Ok(Net {
                pu: first.degree,
                pv: 5,
                ku: first.knots.clone(),
                kv: [vec![0.0; 6], vec![1.0; 6]].concat(),
                rows,
            })
        };
    let side = |k: usize| sides.get(k).ok_or_else(super::internal);
    let (d0, e0) = field(0)?;
    let (d1, e1) = field(1)?;
    let (d2, e2) = field(2)?;
    let (d3, e3) = field(3)?;
    let p1 = rows_of(&side(0)?.c, d0, e0, &side(1)?.c, d1, e1)?;
    let p2 = rows_of(&side(2)?.c, d2, e2, &side(3)?.c, d3, e3)?.transposed();
    // P12: the biquintic of the corner jets.
    let m0 = [[1.0, 0.0, 0.0], [1.0, 0.2, 0.0], [1.0, 0.4, 0.05]];
    let m1 = [[1.0, 0.0, 0.0], [1.0, -0.2, 0.0], [1.0, -0.4, 0.05]];
    let mut b = vec![vec![[0.0; 4]; 6]; 6];
    for (i, jets_i) in jets.iter().enumerate() {
        for (j, jet) in jets_i.iter().enumerate() {
            let mu = if i == 0 { &m0 } else { &m1 };
            let mv = if j == 0 { &m0 } else { &m1 };
            for a in 0..3 {
                for c in 0..3 {
                    let mut h = [0.0; 4];
                    for k in 0..3 {
                        for l in 0..3 {
                            let at = |m: &[[f64; 3]; 3], r: usize, q: usize| {
                                m.get(r).and_then(|row| row.get(q)).copied().unwrap_or(0.0)
                            };
                            let jkl = jet
                                .get(k)
                                .and_then(|row| row.get(l))
                                .copied()
                                .unwrap_or([0.0; 4]);
                            h = super::h_add(h, jkl, 1.0, at(mu, a, k) * at(mv, c, l));
                        }
                    }
                    let (ra, rc) = (
                        if i == 0 { a } else { 5 - a },
                        if j == 0 { c } else { 5 - c },
                    );
                    if let Some(slot) = b.get_mut(ra).and_then(|r| r.get_mut(rc)) {
                        *slot = h;
                    }
                }
            }
        }
    }
    let p12 = Net {
        pu: 5,
        pv: 5,
        ku: [vec![0.0; 6], vec![1.0; 6]].concat(),
        kv: [vec![0.0; 6], vec![1.0; 6]].concat(),
        rows: b,
    };
    let mut nets = [p1, p2, p12];
    super::compatible_nets(&mut nets)?;
    let [p1, p2, p12] = nets;
    let rows: Vec<Vec<H>> = p1
        .rows
        .iter()
        .zip(&p2.rows)
        .zip(&p12.rows)
        .map(|((a, b), c)| {
            a.iter()
                .zip(b)
                .zip(c)
                .map(|((x, y), z)| super::h_add(super::h_add(*x, *y, 1.0, 1.0), *z, 1.0, -1.0))
                .collect()
        })
        .collect();
    Net { rows, ..p1 }.surface()
}

/// The least-norm coefficients `s` with `Σ sᵢ aᵢ` nearest to `y`, and
/// what is left of `y` outside the columns' span: the columns written
/// about `centre` with the weight measured in `length` (as the network's
/// coefficients are, so every coordinate is a length) and scaled to unit
/// length, an orthonormal basis of their span by Gram–Schmidt (a column
/// within `cutoff` of the others' span is dependent), then `s = Aᵀ z` with
/// `A Aᵀ z` the projection of `y`. Least norm, unlike a basic solution,
/// does not depend on which of several dependent columns comes first, so
/// a face's fields give the same field across whatever their order.
fn min_norm(a: &[H], y: H, centre: Vec3, length: f64, cutoff: f64) -> (Vec<f64>, H) {
    let to = |h: &H| -> [f64; 4] {
        [
            h[0] - centre.x * h[3],
            h[1] - centre.y * h[3],
            h[2] - centre.z * h[3],
            h[3] * length,
        ]
    };
    let dot = |p: &[f64; 4], q: &[f64; 4]| p.iter().zip(q).map(|(x, z)| x * z).sum::<f64>();
    let cols: Vec<[f64; 4]> = a.iter().map(to).collect();
    let norms: Vec<f64> = cols.iter().map(|c| dot(c, c).sqrt()).collect();
    // A column no larger than the rounding of the coordinates it came
    // from is noise, not a direction (the network's rule).
    let weight = a
        .iter()
        .chain(std::iter::once(&y))
        .fold(0.0_f64, |m, h| m.max(h[3].abs()));
    let noise = 64.0 * f64::EPSILON * (centre.length() * weight + length);
    let unit: Vec<[f64; 4]> = cols
        .iter()
        .zip(&norms)
        .map(|(c, n)| {
            if *n > noise {
                c.map(|x| x / n)
            } else {
                [0.0; 4]
            }
        })
        .collect();
    // An orthonormal basis of the span.
    let mut basis: Vec<[f64; 4]> = Vec::with_capacity(4);
    for (c, n) in unit.iter().zip(&norms) {
        if *n <= noise {
            continue;
        }
        let mut v = *c;
        for _ in 0..2 {
            for b in &basis {
                let f = dot(&v, b);
                for (x, bx) in v.iter_mut().zip(b) {
                    *x -= f * bx;
                }
            }
        }
        let len = dot(&v, &v).sqrt();
        if len > cutoff && basis.len() < 4 {
            basis.push(v.map(|x| x / len));
        }
    }
    // The columns as the basis has them (a column within the cutoff of
    // the span counts by its part in it).
    let unit: Vec<[f64; 4]> = unit
        .iter()
        .map(|u| {
            let mut v = [0.0; 4];
            for b in &basis {
                let f = dot(u, b);
                for (x, bx) in v.iter_mut().zip(b) {
                    *x += f * bx;
                }
            }
            v
        })
        .collect();
    let ys = to(&y);
    let mut proj = [0.0; 4];
    for b in &basis {
        let f = dot(&ys, b);
        for (x, bx) in proj.iter_mut().zip(b) {
            *x += f * bx;
        }
    }
    let left_s: [f64; 4] = [
        ys[0] - proj[0],
        ys[1] - proj[1],
        ys[2] - proj[2],
        ys[3] - proj[3],
    ];
    // Back from the local coordinates: w = x₃ / length, xyz + centre w.
    let back = |v: [f64; 4]| -> H {
        let w = v[3] / length;
        [
            v[0] + centre.x * w,
            v[1] + centre.y * w,
            v[2] + centre.z * w,
            w,
        ]
    };
    let left = back(left_s);
    // M = Bᵀ U Uᵀ B (r × r) in the basis; z = B M⁻¹ Bᵀ proj.
    let r = basis.len();
    let mut m = vec![vec![0.0_f64; r]; r];
    for (i, bi) in basis.iter().enumerate() {
        for (j, bj) in basis.iter().enumerate() {
            let v: f64 = unit.iter().map(|u| dot(bi, u) * dot(bj, u)).sum();
            if let Some(slot) = m.get_mut(i).and_then(|row| row.get_mut(j)) {
                *slot = v;
            }
        }
    }
    let rhs: Vec<f64> = basis.iter().map(|b| dot(b, &proj)).collect();
    let Some(w) = solve_small(m, rhs) else {
        return (vec![0.0; a.len()], y);
    };
    let mut z = [0.0; 4];
    for (b, wi) in basis.iter().zip(&w) {
        for (x, bx) in z.iter_mut().zip(b) {
            *x += wi * bx;
        }
    }
    let s = unit
        .iter()
        .zip(&norms)
        .map(|(u, n)| if *n > noise { dot(u, &z) / n } else { 0.0 })
        .collect();
    (s, left)
}

/// `M x = b` for a small symmetric positive definite `M` (Gaussian
/// elimination with partial pivoting); `None` where singular.
fn solve_small(mut m: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let pivot = (col..n).max_by(|&i, &j| {
            let a = m
                .get(i)
                .and_then(|r| r.get(col))
                .copied()
                .unwrap_or(0.0)
                .abs();
            let c = m
                .get(j)
                .and_then(|r| r.get(col))
                .copied()
                .unwrap_or(0.0)
                .abs();
            a.total_cmp(&c)
        })?;
        m.swap(col, pivot);
        b.swap(col, pivot);
        let p = *m.get(col)?.get(col)?;
        if !p.is_finite() || p.abs() <= 1e-300 {
            return None;
        }
        for row in col + 1..n {
            let f = *m.get(row)?.get(col)? / p;
            let top = m.get(col)?.clone();
            let r = m.get_mut(row)?;
            for (x, t) in r.iter_mut().zip(&top) {
                *x -= f * t;
            }
            let bc = *b.get(col)?;
            *b.get_mut(row)? -= f * bc;
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let r = m.get(row)?;
        let sum: f64 = (row + 1..n)
            .map(|j| r.get(j).copied().unwrap_or(0.0) * x.get(j).copied().unwrap_or(0.0))
            .sum();
        *x.get_mut(row)? = (b.get(row)? - sum) / r.get(row)?;
    }
    Some(x)
}

/// `A x = b` for a symmetric 3×3 `A` (Cramer's rule); `None` where
/// singular.
fn solve3(a: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: &[[f64; 3]; 3]| -> f64 {
        let [[a, b, c], [d, e, f], [g, h, i]] = *m;
        a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
    };
    let d = det(&a);
    let scale = a.iter().flatten().fold(0.0_f64, |m, x| m.max(x.abs()));
    if !d.is_finite() || d.abs() <= scale * scale * scale * 1e-12 {
        return None;
    }
    let mut out = [0.0; 3];
    for (i, slot) in out.iter_mut().enumerate() {
        let mut m = a;
        for (row, bi) in m.iter_mut().zip(b) {
            if let Some(x) = row.get_mut(i) {
                *x = bi;
            }
        }
        *slot = det(&m) / d;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn least_norm_reproduces_what_lies_in_the_span() {
        let a: Vec<H> = vec![
            [1.0, 2.0, 0.5, 0.0],
            [0.0, 1.0, -1.0, 0.0],
            [1.0, 3.0, -0.5, 0.0],
            [2.0, 0.0, 1.0, 1.0],
            [0.3, 0.1, 0.2, 0.5],
        ];
        let y: H = [0.7, -1.2, 2.0, 0.9];
        let (s, left) = min_norm(&a, y, Vec3::new(1.0, 2.0, 3.0), 4.0, 1e-9);
        let mut back = [0.0; 4];
        for (si, h) in s.iter().zip(&a) {
            for (b, x) in back.iter_mut().zip(h) {
                *b += si * x;
            }
        }
        for i in 0..4 {
            assert!(
                (back[i] + left[i] - y[i]).abs() < 1e-12,
                "{back:?} + {left:?} vs {y:?}"
            );
        }
        assert!(
            left.iter().map(|x| x * x).sum::<f64>().sqrt() < 1e-12,
            "{left:?}"
        );
    }
}
