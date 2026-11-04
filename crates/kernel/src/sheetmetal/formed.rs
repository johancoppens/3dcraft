//! Formed louvres (docs/design/sheetmetal.md › *Formed louvres*).
//!
//! A formed louvre is lanced along a line of a flat and stretched by a
//! tool into a hood behind the line: the material behind the lance rises
//! to an opening along it. It is not developable, so the flat pattern
//! carries only the lance (a cut) and the tool's footprint (a mark); the
//! part is the folded part with the footprint taken out and the hood put
//! in, by booleans after the fold (the caller's: regeneration).
//!
//! **The hood's section** (in the plane square to the lance, `x` back from
//! the lance into the flat, `y` along the flat's normal on the side the
//! hood rises to) is an S of two circular arcs of one radius
//! `ρ = (w² + h²)/(4h)` (`w` the width, `h` the height of the opening's
//! top): from the lip `(0, h)`, level, down about `(0, h − ρ)` to the
//! middle `(w/2, h/2)`, then up about `(w, ρ)` to `(w, 0)`, level with the
//! flat, each arc turning by `α = 2 atan(h/w)`. So the hood leaves the
//! flat tangent to it and its lip is level. Its top skin is the S offset
//! by the thickness `T` along its normal (`ρ + T` about the first centre,
//! `ρ − T` about the second); its rims at the lip and at the back are
//! square to it (vertical). The section's area is `T · 2αρ` exactly (the
//! two bands' `α ((ρ + T)² − ρ²)/2 + α (ρ² − (ρ − T)²)/2`).
//!
//! **Ends.** *Open*: the section swept along the whole lance (`L = ℓ`);
//! the footprint is the rectangle `ℓ × w` behind the lance, and the lance
//! in the pattern runs along the front and both sides (a U). *Closed*
//! (rounded): the section swept along the lance's middle `L = ℓ − 2w` and
//! turned a quarter turn about the vertical line through each end of that
//! middle, so each end is a surface of revolution (a piece of a sphere
//! about the lip's end and of a torus beyond the inflection), smooth since
//! the section's lip is level; the footprint is the rectangle `L × w` and
//! a quarter disc of radius `w` at each end, its front the lance `ℓ`.
//!
//! **The stretch.** The section's fibre from the back to the lip was `w`
//! long in the flat and is the S, `2αρ`, after forming: the stretch
//! `2αρ/w − 1` is reported as a measure for the caller's limit (the
//! material's elongation at break and the tool's own rules are the
//! caller's, as for every sheet-metal measure).

use super::{KernelError, P2, Part, SLIVER_TOLERANCES, Segment, refuse};
use crate::extrude::{OpResult, extrude};
use crate::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use crate::revolve::revolve;
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::Body;

/// How a formed louvre's hood ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoodEnds {
    /// Turned about the vertical at each end of its middle: closed.
    Closed,
    /// Cut along both sides: the section swept along the whole lance.
    Open,
}

/// A formed louvre as asked for, in the pattern's axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FormedSpec {
    /// The lance, from its start to its end; the hood rises behind it, on
    /// its left.
    pub lance: [P2; 2],
    /// How far back from the lance the hood reaches.
    pub width: f64,
    /// The opening's height: the hood's bottom skin at the lip above the
    /// flat's bottom skin (for `up`; below its top skin for down).
    pub height: f64,
    pub ends: HoodEnds,
    /// The side the hood rises to: up, along the flat's normal.
    pub up: bool,
}

/// A formed louvre placed in a flat of a part.
#[derive(Clone, Debug, PartialEq)]
pub struct Formed {
    pub spec: FormedSpec,
    /// The flat it lies in.
    pub flat: usize,
    /// The section's radius `ρ` and each arc's turn `α`.
    pub radius: f64,
    pub turn: f64,
    /// The footprint (pattern): the region the tool takes the flat's
    /// material from, a closed profile.
    pub footprint: Profile,
    /// The lance's pieces in the pattern (cuts).
    pub lance: Vec<Segment>,
}

impl Formed {
    /// The stretch of the section's fibre, `2αρ/w − 1`.
    pub fn stretch(&self) -> f64 {
        2.0 * self.turn * self.radius / self.spec.width - 1.0
    }

    /// The length of the hood's straight middle.
    pub fn middle(&self) -> f64 {
        let length = super::distance(self.spec.lance[0], self.spec.lance[1]);
        match self.spec.ends {
            HoodEnds::Open => length,
            HoodEnds::Closed => length - 2.0 * self.spec.width,
        }
    }

    /// The footprint's area (exact): `L w`, plus `π w²/2` closed.
    pub fn footprint_area(&self) -> f64 {
        let w = self.spec.width;
        let l = self.middle();
        match self.spec.ends {
            HoodEnds::Open => l * w,
            HoodEnds::Closed => l * w + 0.5 * std::f64::consts::PI * w * w,
        }
    }

    /// The hood's volume (exact): the section's area `2αρT` along the
    /// middle, and closed, the two quarter turns, `π ∫ x dA` over the
    /// section (Pappus).
    pub fn hood_volume(&self, t: f64) -> f64 {
        let (r, a, w) = (self.radius, self.turn, self.spec.width);
        let swept = 2.0 * a * r * t * self.middle();
        match self.spec.ends {
            HoodEnds::Open => swept,
            HoodEnds::Closed => {
                let c = a.pcos();
                let cube = |x: f64| x * x * x;
                let first = (cube(r + t) - cube(r)) / 3.0 * (1.0 - c);
                let second = w * a * (r * r - (r - t) * (r - t)) / 2.0
                    + (cube(r) - cube(r - t)) / 3.0 * (c - 1.0);
                swept + std::f64::consts::PI * (first + second)
            }
        }
    }
}

fn unit(v: P2) -> Option<P2> {
    super::unit(v)
}

fn line(id: u64, a: P2, b: P2) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start: a, end: b },
    }
}

fn arc(id: u64, center: P2, start: P2, end: P2) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Arc { center, start, end },
    }
}

fn at(p: P2, d: P2, s: f64, b: P2, t: f64) -> P2 {
    [p[0] + s * d[0] + t * b[0], p[1] + s * d[1] + t * b[1]]
}

/// A box round segments (lines by their ends, arcs and circles by their
/// circle's, splines by their control points): it holds them.
fn boxed(segs: &[Segment]) -> [f64; 4] {
    use crate::profile::Shape;
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut put = |p: P2, r: f64| {
        b = [
            b[0].min(p[0] - r),
            b[1].min(p[1] - r),
            b[2].max(p[0] + r),
            b[3].max(p[1] + r),
        ];
    };
    for s in segs {
        match &s.shape {
            Shape::Line => {
                put(s.start, 0.0);
                put(s.end, 0.0);
            }
            &Shape::Arc { center, radius } | &Shape::Circle { center, radius } => {
                put(center, radius);
            }
            Shape::Spline(sp) => {
                for &p in &sp.control {
                    put(p, 0.0);
                }
            }
        }
    }
    b
}

/// Places the formed louvres in the part's flats, in order: each must lie,
/// footprint and lance, wholly inside one flat, clear of its outline, its
/// holes, the louvres before it and every hole across a bend. Refused by
/// reason, naming the louvre by its place (`flange` in the error is the
/// louvre's place).
pub fn place(
    part: &Part,
    specs: &[FormedSpec],
    tolerance: &Tolerance,
) -> Result<Vec<Formed>, KernelError> {
    let tol = tolerance.linear;
    let floor = SLIVER_TOLERANCES * tol;
    let mut out: Vec<Formed> = Vec::with_capacity(specs.len());
    for (i, spec) in specs.iter().enumerate() {
        let no = |why: String| refuse(Some(i), format!("formed louvre {i}: {why}"));
        let [a, b] = spec.lance;
        let length = super::distance(a, b);
        let (w, h) = (spec.width, spec.height);
        if !(length.is_finite() && w.is_finite() && h.is_finite()) {
            return Err(no("its lance, width or height is not finite".into()));
        }
        if !(w > floor) {
            return Err(no(format!("its width {w} has to be more than {floor}")));
        }
        let t = part.thickness;
        if !(h > t + floor) {
            return Err(no(format!(
                "its height {h} has to be more than the thickness {t} (the hood's lip would lie in the sheet in front of it)"
            )));
        }
        if !(h < w) {
            return Err(no(format!(
                "its height {h} has to be less than its width {w} (the hood's S would lean over its back)"
            )));
        }
        let radius = (w * w + h * h) / (4.0 * h);
        let turn = 2.0 * h.patan2(w);
        if !(radius - t > floor) {
            return Err(no(format!(
                "its hood's radius {radius} leaves no top skin behind the inflection with the thickness {t}: make it wider or lower"
            )));
        }
        let Some(d) = unit(super::sub(b, a)) else {
            return Err(no("its lance has no length".into()));
        };
        let back = [-d[1], d[0]];
        let middle = match spec.ends {
            HoodEnds::Open => length,
            HoodEnds::Closed => length - 2.0 * w,
        };
        if !(middle > floor) {
            return Err(no(format!(
                "its lance {length} leaves no straight middle {middle} between its closed ends (each as long as the width {w}): make it longer than twice the width, or the ends open"
            )));
        }
        let curves = match spec.ends {
            HoodEnds::Open => vec![
                line(1, a, b),
                line(2, b, at(b, d, 0.0, back, w)),
                line(3, at(b, d, 0.0, back, w), at(a, d, 0.0, back, w)),
                line(4, at(a, d, 0.0, back, w), a),
            ],
            HoodEnds::Closed => {
                let (m0, m1) = (at(a, d, w, back, 0.0), at(a, d, w + middle, back, 0.0));
                vec![
                    line(1, a, b),
                    arc(2, m1, b, at(m1, d, 0.0, back, w)),
                    line(3, at(m1, d, 0.0, back, w), at(m0, d, 0.0, back, w)),
                    arc(4, m0, at(m0, d, 0.0, back, w), a),
                ]
            }
        };
        let mut found = detect_profiles(&curves, tolerance).map_err(|e| no(e.to_string()))?;
        let (Some(footprint), true) = (found.pop(), found.is_empty()) else {
            return Err(no("its footprint is not one region".into()));
        };
        let lance: Vec<Segment> = {
            let segs = &footprint.outer.segments;
            // The footprint's pieces that are cut: its front and, open, its
            // sides (by their curves).
            let cut: &[u64] = match spec.ends {
                HoodEnds::Open => &[1, 2, 4],
                HoodEnds::Closed => &[1],
            };
            segs.iter()
                .filter(|s| cut.contains(&s.source))
                .cloned()
                .collect()
        };
        let mine = &footprint.outer.segments;
        let fp_box = boxed(mine);
        if let Some(c) = part.cuts.iter().find(|c| {
            let cb = boxed(&c.segments);
            cb[0] <= fp_box[2] + tol
                && fp_box[0] <= cb[2] + tol
                && cb[1] <= fp_box[3] + tol
                && fp_box[1] <= cb[3] + tol
        }) {
            return Err(no(format!(
                "its footprint lies beside the hole across a bend {} (not supported): move it clear",
                c.named
            )));
        }
        let mut flat_of = None;
        for (f, flat) in part.flats.iter().enumerate() {
            let mut holes = flat.holes.clone();
            for o in out.iter().filter(|o| o.flat == f) {
                holes.push(o.footprint.outer.segments.clone());
            }
            if super::hole_fits(&flat.outer, &holes, mine, tolerance) {
                flat_of = Some(f);
                break;
            }
        }
        let Some(flat) = flat_of else {
            return Err(no(
                "its footprint (the lance and the hood's reach behind it, its left) lies in no one flat clear of its edges, holes and the louvres before it: move it, or make it smaller".into(),
            ));
        };
        out.push(Formed {
            spec: *spec,
            flat,
            radius,
            turn,
            footprint,
            lance,
        });
    }
    // A hood near a sharp bend's inner skins: proven clear or refused
    // (sheetmetal.md › *Sharp bends* › *Touching*).
    super::sharp::clear_hoods(part, &out, tol)?;
    Ok(out)
}

/// The section's closed profile in its own sketch axes (`x` back, `y` up).
fn section(f: &Formed, t: f64, tolerance: &Tolerance) -> Result<Profile, KernelError> {
    let (w, h, r) = (f.spec.width, f.spec.height, f.radius);
    let (c1, c2) = ([0.0, h - r], [w, r]);
    let mid = [0.5 * w, 0.5 * h];
    // The unit normal at the middle, up the first arc's way.
    let u = [(mid[0] - c1[0]) / r, (mid[1] - c1[1]) / r];
    let mid_top = [mid[0] + t * u[0], mid[1] + t * u[1]];
    let curves = vec![
        arc(1, c1, mid, [0.0, h]),
        line(2, [0.0, h], [0.0, h + t]),
        arc(3, c1, mid_top, [0.0, h + t]),
        arc(4, c2, mid_top, [w, t]),
        line(5, [w, t], [w, 0.0]),
        arc(6, c2, mid, [w, 0.0]),
    ];
    let mut found = detect_profiles(&curves, tolerance)?;
    match (found.pop(), found.is_empty()) {
        (Some(p), true) => Ok(p),
        _ => Err(refuse(None, "a formed louvre's section is not one region")),
    }
}

/// A body with the role of each of its faces ([`bodies`]).
pub type WithRoles = (Body, Vec<u64>);

/// The role a face of an extrusion or a revolution of a profile has: the
/// curve that swept it (its id: the section's 1 the first arc's bottom
/// skin, 2 the lip's rim, 3 the first arc's top skin, 4 the second arc's
/// top skin, 5 the back rim, 6 the second arc's bottom skin; the
/// footprint's 1 the lance, 2 to 4 the rest of its outline), 7 the cap
/// at the start of the sweep, 8 at its end; `swap` exchanges 7 and 8.
fn role_of(origin: &crate::extrude::Origin, swap: bool) -> Result<u64, KernelError> {
    use crate::extrude::Role;
    let bad = || refuse(None, "a formed louvre's face has no role");
    if origin.piece != 0 {
        return Err(bad());
    }
    match origin.role {
        Role::Side => origin
            .sources
            .first()
            .copied()
            .filter(|&c| (1..=6).contains(&c))
            .ok_or_else(bad),
        Role::StartCap => Ok(if swap { 8 } else { 7 }),
        Role::EndCap => Ok(if swap { 7 } else { 8 }),
        _ => Err(bad()),
    }
}

fn roles(made: &OpResult, swap: bool, piece: u64) -> Result<Vec<u64>, KernelError> {
    made.origins
        .faces
        .iter()
        .map(|o| role_of(o, swap).map(|r| piece * 16 + r))
        .collect()
}

/// The footprint's prism through the sheet and the hood, both in the
/// world (`world` the flat's motion then the part's plane), each with the
/// **role** of each face, what names it (docs/design/sheetmetal.md ›
/// *Formed louvres* › *Names*): `16 p + r`, `p` the piece (0 the swept
/// middle, 1 the end turned at the lance's start, 2 at its end; the prism
/// 0) and `r` the face's role in it ([`role_of`]: the curve that swept it,
/// 7 the cap on the lance's start side, 8 on its end side), the same
/// whatever the width, height or length.
pub fn bodies(
    part: &Part,
    f: &Formed,
    tolerance: &Tolerance,
) -> Result<(WithRoles, WithRoles), KernelError> {
    let t = part.thickness;
    let flat = part
        .flats
        .get(f.flat)
        .ok_or_else(|| refuse(None, "a formed louvre's flat is missing"))?;
    let world = super::world(&part.plane, flat);
    let frame_at = |origin: Vec3, z: Vec3, x: Vec3| -> Result<Frame, KernelError> {
        Frame::new(world.point(origin), world.vector(z), world.vector(x))
            .ok_or_else(|| refuse(None, "a formed louvre's frame is not finite"))
    };
    // The footprint through the thickness.
    let bottom = frame_at(Vec3::ZERO, Vec3::Z, Vec3::X)?;
    let prism_made = extrude(
        std::slice::from_ref(&f.footprint),
        &bottom,
        0.0,
        t,
        tolerance,
    )?;
    let prism = (prism_made.body.clone(), roles(&prism_made, false, 0)?);
    // The section's frame at the middle's start: x back, y up.
    let [a, b] = f.spec.lance;
    let d = unit(super::sub(b, a)).ok_or_else(|| refuse(None, "a lance has no length"))?;
    let back = Vec3::new(-d[1], d[0], 0.0);
    let (up, level) = if f.spec.up {
        (Vec3::Z, 0.0)
    } else {
        (-Vec3::Z, t)
    };
    let start = match f.spec.ends {
        HoodEnds::Open => 0.0,
        HoodEnds::Closed => f.spec.width,
    };
    let along = Vec3::new(d[0], d[1], 0.0);
    let origin = |s: f64| Vec3::new(a[0] + s * d[0], a[1] + s * d[1], level);
    let normal = back.cross(up);
    let sect = section(f, t, tolerance)?;
    let l = f.middle();
    let o0 = origin(start);
    let plane0 = frame_at(o0, normal, back)?;
    // `normal` is `d` for a hood up, `−d` down.
    let forward = normal.dot(along) > 0.0;
    let (from, to) = if forward { (0.0, l) } else { (-l, 0.0) };
    let middle = extrude(std::slice::from_ref(&sect), &plane0, from, to, tolerance)?;
    // Down, the sweep runs from the lance's end side to its start side.
    let mut hood_roles = roles(&middle, !forward, 0)?;
    let mut hood = middle.body;
    if f.spec.ends == HoodEnds::Closed {
        // Turning `back` a quarter about `up` by a positive angle gives
        // `up × back`; at the middle's start the end turns towards `−d`,
        // at its end towards `+d`.
        let toward_start = up.cross(back).dot(along) < 0.0;
        let quarter = std::f64::consts::FRAC_PI_2;
        let o1 = origin(start + l);
        let plane1 = frame_at(o1, normal, back)?;
        for (piece, plane, positive) in [(1u64, plane0, toward_start), (2, plane1, !toward_start)] {
            let (lo, hi) = if positive {
                (0.0, quarter)
            } else {
                (-quarter, 0.0)
            };
            let cap: OpResult = revolve(
                std::slice::from_ref(&sect),
                &plane,
                [0.0, 0.0],
                [0.0, 1.0],
                lo,
                hi,
                tolerance,
            )?;
            let joined = crate::boolean::boolean(
                &hood,
                &cap.body,
                crate::boolean::BooleanOp::Union,
                tolerance,
            )?;
            let cap_roles = roles(&cap, false, piece)?;
            hood_roles = joined
                .faces
                .iter()
                .map(|src| {
                    let list = match src.operand {
                        crate::boolean::Operand::A => &hood_roles,
                        crate::boolean::Operand::B => &cap_roles,
                    };
                    list.get(src.face.0 as usize)
                        .copied()
                        .ok_or_else(|| refuse(None, "a formed louvre's face has no role"))
                })
                .collect::<Result<_, _>>()?;
            hood = joined.body;
        }
    }
    Ok((prism, (hood, hood_roles)))
}
