//! Rims along tight arcs (docs/design/plates.md › *Corners*): a plate's
//! boundary edge is traced past each of its ends by where the tangent
//! planes put its corners and twice the thicknesses beyond, so an arc of
//! radius `r` and sweep `s` was traced over `s + 4t/r` (with square ends),
//! several turns of its circle once `r` is below about the thickness. Each
//! body here is held to its closed form, to `topo::check`, to the measure
//! and to its mesh: holes, discs, rounded corners, slots and segments of
//! arcs of one circle, square and bevelled, on every side, at the radii
//! where that run was a whole number of turns (even and odd) and a hair
//! beside them, r/t from 1e-3 to 10, sweeps from 3 degrees to a whole
//! turn; sheet-metal bends up and down at the inner radii where the run of
//! their arcs was a whole number of turns, and a hole across a bend; at
//! three scales, in the world and in frames turned and moved off it.
//!
//! Closed forms, worked out apart from the kernel: a flat plate with
//! square rims is its outline's area (Green's theorem on the exact arcs,
//! `hunt_sheet_common`) times the thickness; with bevelled rims, where the
//! rim at the level `h` is moved out of the material by `h tan α`, the
//! integral over the levels of the area at each: a disc or a hole of
//! arcs of one circle `π ∫ (r ± h tan α)² dh` in closed form, a segment
//! (an arc and its chord, moved apart at their corners as the rims meet)
//! the segment's area of the circle `r + h tan α` cut by the chord moved
//! out by `h tan α`, integrated by Gauss–Legendre on 64 pieces; a bend
//! `θ T (R + T/2)` per length of its bend line, the flats `A T`.

// `!(x <= y)` is deliberate: unlike `x > y` it is also true for NaN, so
// a volume that is not a number is a problem.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::neg_cmp_op_on_partial_ord
)]

mod boolean_common;
mod hunt_sheet_common;

use boolean_common::Rng;
use craft3d_geom::Curve3;
use craft3d_kernel::extrude::fill;
use craft3d_kernel::plate::{Joint, Side, plate};
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, Cutout, Direction, FlangeSpec, LengthFrom, On, Spec, build,
};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::{Body, EdgeId, FaceId, check};
use hunt_sheet_common::*;
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// The whole numbers of turns `n` at which an arc of sweep `s` was traced
/// over `n` turns (at the radius `4t / (2πn − s)`): every count to 8,
/// then about a factor 2 apart, to r/t about 1e-3.
const TURNS: [u32; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 11, 16, 23, 32, 64, 128, 256, 637];

/// The radii over the thickness tried besides those: 1e-3 to 10.
const GRID: [f64; 9] = [1e-3, 3e-3, 1e-2, 3e-2, 0.1, 0.3, 1.0, 3.0, 10.0];

/// A whole-turn radius exactly and hairs beside it: four up to 8 turns,
/// a billionth over beyond.
const HAIRS: [f64; 5] = [1.0, 1.0 - 1e-9, 1.0 + 1e-9, 1.0 - 1e-6, 1.0 + 1e-6];
const FEW_HAIRS: [f64; 2] = [1.0, 1.0 + 1e-9];

/// The radius at which an arc of sweep `s` was traced over `n` whole
/// turns with the thickness `t` (twice it past each end).
fn whole_turns(s: f64, t: f64, n: u32) -> f64 {
    4.0 * t / (TAU * f64::from(n) - s)
}

/// The hairs about `n` whole turns.
fn hairs(n: u32) -> &'static [f64] {
    if n <= 8 { &HAIRS } else { &FEW_HAIRS }
}

/// The radii of a sweep `s` over the thickness 1: the whole turns (each
/// with its hairs) and the grid, those in `(lo, hi)`.
fn radii(s: f64, lo: f64, hi: f64) -> Vec<f64> {
    let mut out = Vec::new();
    for n in TURNS {
        let r = whole_turns(s, 1.0, n);
        if r > lo && r < hi {
            out.extend(hairs(n).iter().map(|h| r * h));
        }
    }
    out.extend(GRID.iter().copied().filter(|r| *r > lo && *r < hi));
    out
}

/// What the rims of a flat plate's arcs are.
#[derive(Clone, Copy, Debug)]
enum Rims {
    Square,
    /// Every edge of the outline of arcs (or of the segment) bevelled.
    Bevel(f64),
}

/// The outlines.
#[derive(Clone, Copy, Debug)]
enum Outline {
    /// A square with a hole of `n` arcs of one circle, the first of sweep
    /// `s` (the rest share what is left of the turn).
    Hole { s: f64, n: usize },
    /// A whole circle as one edge (a closed edge, traced once round).
    Round,
    /// A disc of two arcs of one circle, the first of sweep `s`.
    Disc { s: f64 },
    /// A square with its corners rounded.
    Rounded,
    /// A square with a slot whose half circles are `split` arcs each.
    Slot { split: usize },
    /// An arc of sweep `s` and its chord.
    Segment { s: f64 },
}

impl Outline {
    /// The sweep of the arc whose whole-turn radii are tried.
    fn sweep(self) -> f64 {
        match self {
            Outline::Hole { s, .. } | Outline::Disc { s } | Outline::Segment { s } => s,
            Outline::Round => TAU,
            Outline::Rounded => FRAC_PI_2,
            Outline::Slot { split } => PI / split as f64,
        }
    }
}

/// One flat plate.
#[derive(Clone, Debug)]
struct PlateCase {
    outline: Outline,
    /// The arcs' radius at thickness 1, scale 1.
    r: f64,
    side: Side,
    rims: Rims,
    k: f64,
    frame: Frame,
    turn: f64,
}

/// The shape of a case at thickness 1 and scale 1, and the radius of its
/// arcs.
fn shape_of(c: &PlateCase) -> Shape {
    let r = c.r;
    let a0 = 0.3;
    let square = |h: f64| polygon_loop(&[[-h, -h], [h, -h], [h, h], [-h, h]]);
    let mut shape = Shape {
        what: String::new(),
        outer: Vec::new(),
        holes: Vec::new(),
        inner: ([0.0, 0.0], 0.0),
        hole_discs: Vec::new(),
        feature: r,
    };
    match c.outline {
        Outline::Hole { s, n } => {
            let h = (2.0 * r).max(10.0);
            let mut cuts = vec![a0, a0 + s];
            for j in 1..n.saturating_sub(1) {
                cuts.push(a0 + s + (TAU - s) * j as f64 / (n - 1) as f64);
            }
            shape.outer = square(h);
            shape.holes = vec![reversed(&split_circle([0.0, 0.0], r, &cuts))];
            shape.inner = ([0.5 * (h + r), 0.0], 0.1 * (h - r));
        }
        Outline::Round => {
            let h = (2.0 * r).max(10.0);
            shape.outer = square(h);
            shape.holes = vec![vec![Seg::Circle {
                c: [0.0, 0.0],
                r,
                ccw: false,
            }]];
            shape.inner = ([0.5 * (h + r), 0.0], 0.1 * (h - r));
        }
        Outline::Disc { s } => {
            shape.outer = split_circle([0.0, 0.0], r, &[a0, a0 + s]);
            shape.inner = ([0.0, 0.0], 0.5 * r);
        }
        Outline::Rounded => {
            let w = 4.0 * r + 10.0;
            shape.outer = rounded_rect(w, w, r);
            shape.inner = ([0.0, 0.0], 0.25 * w);
        }
        Outline::Slot { split } => {
            let l = 2.0 * r;
            let h = (l + 2.0 * r).max(10.0);
            shape.outer = square(h);
            shape.holes = vec![reversed(&slot(l, r, split))];
            shape.inner = ([0.0, 0.5 * (h + r)], 0.1 * (h - r));
        }
        Outline::Segment { s } => {
            let (from, to) = (
                polar([0.0, 0.0], r, -0.5 * s),
                polar([0.0, 0.0], r, 0.5 * s),
            );
            shape.outer = vec![
                arc([0.0, 0.0], r, -0.5 * s, s, from, to),
                Seg::Line(to, from),
            ];
            shape.inner = ([0.5 * r * (1.0 + (0.5 * s).pcos()), 0.0], 0.0);
            shape.feature = r * (1.0 - (0.5 * s).pcos()).min(1.0);
        }
    }
    shape.what = format!("{:?} r {}", c.outline, r);
    shape
}

/// The area at the level `h` of a segment of the circle `r` (its arc of
/// sweep `s`) whose rims are moved out of the material by `d = h tan α`:
/// the circle `r + d` cut by the chord moved out by `d`.
fn segment_area(r: f64, s: f64, d: f64) -> f64 {
    let big = r + d;
    let x0 = r * (0.5 * s).pcos() - d;
    let beta = 2.0 * (x0 / big).clamp(-1.0, 1.0).pacos();
    0.5 * big * big * (beta - beta.psin())
}

fn cube(x: f64) -> f64 {
    x * x * x
}

/// `∫ f` over `(a, b)` by Gauss–Legendre of 6 points on 64 pieces.
fn integral(a: f64, b: f64, f: impl Fn(f64) -> f64) -> f64 {
    const NODES: [(f64, f64); 6] = [
        (-0.932_469_514_203_152, 0.171_324_492_379_170),
        (-0.661_209_386_466_265, 0.360_761_573_048_139),
        (-0.238_619_186_083_197, 0.467_913_934_572_691),
        (0.238_619_186_083_197, 0.467_913_934_572_691),
        (0.661_209_386_466_265, 0.360_761_573_048_139),
        (0.932_469_514_203_152, 0.171_324_492_379_170),
    ];
    let pieces = 64;
    let w = (b - a) / f64::from(pieces);
    let mut sum = 0.0;
    for i in 0..pieces {
        let m = a + w * (f64::from(i) + 0.5);
        for (x, wt) in NODES {
            sum += wt * f(m + 0.5 * w * x);
        }
    }
    0.5 * w * sum
}

/// The closed-form volume of a case at thickness 1 and scale 1 (levels
/// `lo`, `hi`), or `None` where its bevel would close what it bounds.
fn volume_of(c: &PlateCase, shape: &Shape, (lo, hi): (f64, f64)) -> f64 {
    let t = hi - lo;
    let r = c.r;
    match (c.rims, c.outline) {
        (Rims::Square, _) => shape.area() * t,
        (Rims::Bevel(alpha), Outline::Hole { .. }) => {
            // The hole shrinks at the front: radius r − h tan α.
            let tau = alpha.ptan();
            let outer: f64 = shape.outer.iter().map(|s| s.green(shape.inner.0)).sum();
            outer * t
                - PI * (r * r * t - r * tau * (hi * hi - lo * lo)
                    + tau * tau * (cube(hi) - cube(lo)) / 3.0)
        }
        (Rims::Bevel(alpha), Outline::Disc { .. }) => {
            let tau = alpha.ptan();
            PI * (r * r * t
                + r * tau * (hi * hi - lo * lo)
                + tau * tau * (cube(hi) - cube(lo)) / 3.0)
        }
        (Rims::Bevel(alpha), Outline::Segment { s }) => {
            let tau = alpha.ptan();
            integral(lo, hi, |h| segment_area(r, s, h * tau))
        }
        (Rims::Bevel(_), _) => f64::NAN,
    }
}

/// Builds and judges one flat plate; its letter goes into `tally`.
fn plate_case(tally: &mut Tally, n: usize, c: &PlateCase) {
    let k = c.k;
    let tol = tolerance(k);
    let t = k;
    let shape = shape_of(c);
    let what = format!(
        "{} (r/t {:e}), {:?}, {:?}, k {k}, turn {}, frame {:?}",
        shape.what, c.r, c.side, c.rims, c.turn, c.frame
    );
    let placing = Placing {
        turn: c.turn,
        k,
        at: [0.0, 0.0],
        flip_lines: false,
    };
    let curves = sketch(&shape, &placing, 1);
    let profiles = match detect_profiles(&curves, &tol) {
        Ok(p) if p.len() == 1 && p[0].holes.len() == shape.holes.len() => p,
        other => {
            tally.notes.push(format!(
                "case {n}: {what}: no single profile: {:?}",
                other.map(|p| p.len())
            ));
            tally.outcomes.push('S');
            return;
        }
    };
    let sheet = match fill(&profiles, &c.frame, &tol) {
        Ok(s) => s.body,
        Err(e) => {
            tally.notes.push(format!("case {n}: {what}: no fill: {e}"));
            tally.outcomes.push('S');
            return;
        }
    };
    let mut joints = BTreeMap::new();
    if let Rims::Bevel(alpha) = c.rims {
        for (i, e) in sheet.edges.iter().enumerate() {
            let bevelled = match c.outline {
                Outline::Segment { .. } => true,
                _ => matches!(e.curve, Curve3::Circle(_)),
            };
            if bevelled {
                joints.insert(EdgeId(i as u32), Joint::Bevel(alpha));
            }
        }
    }
    let levels = c.side.levels(t);
    let want = volume_of(c, &shape, (levels.0 / k, levels.1 / k)) * k * k * k;
    let made = guarded(|| plate(&sheet, &[FaceId(0)], t, c.side, &joints, &tol));
    match made {
        Err(p) => {
            tally.wrong.push(format!("case {n} (P): {what}: {p}"));
            tally.outcomes.push('P');
        }
        Ok(Err(e)) => {
            tally.refused(&e.to_string());
            tally.notes.push(format!("case {n}: {what}: refused: {e}"));
        }
        Ok(Ok(made)) => judge(tally, n, &what, &made.body, want, shape.feature * k, &tol),
    }
}

/// Holds a built body to `want` (its closed-form volume) by two means,
/// the measure and the mesh (at a thousandth of its size, or a quarter
/// of its narrowest width `feature` where that is less), after the
/// checker, and where both hold whether it passes through itself: `I`
/// invalid, `M` checked but not meshed, `W` measured or meshed off the
/// closed form (or not measured), `X` through itself, `B` right.
#[allow(clippy::too_many_arguments)]
fn judge(
    tally: &mut Tally,
    n: usize,
    what: &str,
    body: &Body,
    want: f64,
    feature: f64,
    tol: &Tolerance,
) {
    let mut found = Found::default();
    let defects = check(body, tol);
    if let Some(first) = defects.first() {
        found.invalid = true;
        found.problems.push(format!("not valid: {first:?}"));
        tally.record(n, what, &found, false);
        return;
    }
    let chord = (1e-3 * size_of(body)).min(0.25 * feature);
    let measure = measured(body);
    let mesh = mesh_volume(body, chord);
    let slack = |m: &Measured| {
        m.bound
            + 1e-9 * want.abs()
            + if any_freeform(body) {
                0.5 * tol.linear * m.area
            } else {
                0.0
            }
    };
    match (&measure, &mesh) {
        (None, Err(e)) => {
            found.unmeshable = true;
            found
                .problems
                .push(format!("passes the checker but is neither measured nor meshed: {e}"));
        }
        (Some(_), Err(e)) => {
            found.unmeshable = true;
            found
                .problems
                .push(format!("passes the checker and is measured but not meshed: {e}"));
        }
        (None, Ok(v)) => found.problems.push(format!(
            "passes the checker and is meshed but not measured: the mesh's volume {v:.9e} for {want:.9e} ({:.2e} of it)",
            v / want - 1.0
        )),
        (Some(m), Ok(v)) => {
            if !((m.volume - want).abs() <= slack(m)) {
                // The second means: the mesh, within its chord's bound.
                let mesh_off = !((v - want).abs() <= 2.0 * chord * m.area + m.bound);
                found.problems.push(format!(
                    "volume {:.17e} against the closed form {want:.17e} ({:.3e} of it); the mesh's {v:.9e} {}",
                    m.volume,
                    m.volume / want - 1.0,
                    if mesh_off { "off too" } else { "within its bound" }
                ));
            } else if !((v - m.volume).abs() <= 2.0 * chord * m.area + m.bound) {
                found.problems.push(format!(
                    "the mesh's volume {v:.9e} against the measured {:.9e}",
                    m.volume
                ));
            }
        }
    }
    let crosses = found.problems.is_empty() && crosses_itself(body, tol);
    tally.record(n, what, &found, crosses);
}

/// The flat plates: every outline at its whole-turn radii with their
/// hairs and at the grid, each with a side, a scale, a frame, a turn and
/// (where its closed form is known) square or bevelled rims drawn from
/// the case's seed.
fn plate_cases(outlines: &[Outline], seed: u64) -> Vec<PlateCase> {
    let mut out = Vec::new();
    for outline in outlines {
        let (lo, hi) = match outline {
            Outline::Segment { s } if *s < 0.5 => (1e-3, 3.0),
            _ => (9e-4, 11.0),
        };
        for r in radii(outline.sweep(), lo, hi) {
            let mut rng = Rng(case_seed(seed, out.len()));
            let side = match rng.below(3) {
                0 => Side::Out,
                1 => Side::In,
                _ => Side::Mid,
            };
            let k = SCALES[rng.below(3) as usize];
            let frame = frame(&mut rng, k);
            let turn = rng.between(0.0, TAU);
            let bevel_known = matches!(
                outline,
                Outline::Hole { .. } | Outline::Disc { .. } | Outline::Segment { .. }
            );
            let rims = if bevel_known && rng.below(3) == 0 {
                let alpha = rng.between(0.1, 0.5) * if rng.below(2) == 0 { 1.0 } else { -1.0 };
                // Kept where the rims stay well clear of closing what they
                // bound: moved by at most a third of the narrowest width.
                let width = match outline {
                    Outline::Segment { s } => r * (1.0 - (0.5 * s).pcos()).min(1.0),
                    _ => r,
                };
                if alpha.ptan().abs() < width / 3.0 {
                    Rims::Bevel(alpha)
                } else {
                    Rims::Square
                }
            } else {
                Rims::Square
            };
            out.push(PlateCase {
                outline: *outline,
                r,
                side,
                rims,
                k,
                frame,
                turn,
            });
        }
    }
    out
}

const HOLES: [Outline; 7] = [
    Outline::Hole { s: PI, n: 2 },
    Outline::Hole {
        s: 3.0 * PI / 180.0,
        n: 2,
    },
    Outline::Hole {
        s: 10.0 * PI / 180.0,
        n: 2,
    },
    Outline::Hole { s: FRAC_PI_2, n: 2 },
    Outline::Hole {
        s: 330.0 * PI / 180.0,
        n: 2,
    },
    Outline::Hole { s: TAU / 3.0, n: 3 },
    Outline::Round,
];

const OTHERS: [Outline; 8] = [
    Outline::Disc { s: PI },
    Outline::Disc {
        s: 20.0 * PI / 180.0,
    },
    Outline::Rounded,
    Outline::Slot { split: 1 },
    Outline::Slot { split: 2 },
    Outline::Segment {
        s: 10.0 * PI / 180.0,
    },
    Outline::Segment { s: PI },
    Outline::Segment {
        s: 300.0 * PI / 180.0,
    },
];

/// One sheet-metal part: a base `50 × 40`, `T` 1, one flange 20 long on
/// its right edge (40 long) at `angle`, all at scale `k` in `plane`.
#[derive(Clone, Debug)]
struct BendCase {
    angle: f64,
    direction: Direction,
    /// The inner radius at `T` 1, scale 1.
    r: f64,
    kk: f64,
    k: f64,
    plane: Frame,
    /// A hole of this radius (at `T` 1, scale 1) over the middle of the
    /// bend's strip, if any.
    hole: Option<f64>,
}

fn line(id: u64, start: P2, end: P2) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

/// Builds and judges one sheet-metal part.
fn bend_case(tally: &mut Tally, n: usize, c: &BendCase) {
    let k = c.k;
    let tol = tolerance(k);
    let (w, d, t, l) = (50.0 * k, 40.0 * k, k, 20.0 * k);
    let r = c.r * k;
    let what = format!(
        "bend {:.3} deg {:?} R/T {:e} K {} hole {:?} k {k} plane {:?}",
        c.angle.to_degrees(),
        c.direction,
        c.r,
        c.kk,
        c.hole,
        c.plane
    );
    let corners = [[0.0, 0.0], [w, 0.0], [w, d], [0.0, d]];
    let curves: Vec<SketchCurve> = (0..4)
        .map(|i| line(i as u64 + 1, corners[i], corners[(i + 1) % 4]))
        .collect();
    let base = detect_profiles(&curves, &tol).unwrap().remove(0);
    let theta = c.angle;
    let ba = theta * (r + c.kk * t);
    let mut cutouts = Vec::new();
    let mut want = w * d * t + d * (theta * t * (r + 0.5 * t) + l * t);
    if let Some(rho) = c.hole {
        let rho = rho * k;
        // The hole's middle on the middle of the strip, which runs from
        // the base's right edge `ba` across in the flat pattern.
        cutouts.push(Cutout {
            profile: detect_profiles(
                &[SketchCurve {
                    id: 100,
                    curve: Curve2::Circle {
                        center: [w + 0.5 * ba, 0.5 * d],
                        radius: rho,
                    },
                }],
                &tol,
            )
            .unwrap()
            .remove(0),
            from: "hole".into(),
        });
        // The disc's part on the strip (a band across it) and on the flats.
        let g = |x: f64| {
            let x = x.clamp(-rho, rho);
            x * (rho * rho - x * x).max(0.0).sqrt() + rho * rho * (x / rho).pasin()
        };
        let band = g(0.5 * ba) - g(-0.5 * ba);
        let disc = PI * rho * rho;
        // A strip's area becomes `T (R + T/2) / (R + K T)` of volume.
        want -= (disc - band) * t + band * t * (r + 0.5 * t) / (r + c.kk * t);
    }
    let spec = Spec {
        base,
        plane: c.plane,
        thickness: t,
        flanges: vec![FlangeSpec {
            on: On::Base(2),
            length: l,
            angle: theta,
            direction: c.direction,
            radius: r,
            k: c.kk,
            from: LengthFrom::Tangent,
            inset: [0.0; 2],
            outline: None,
        }],
        relief: None,
        bend_relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts,
        jogs: Vec::new(),
        ids: Vec::new(),
    };
    match guarded(|| build(&spec, &tol)) {
        Err(p) => {
            tally.wrong.push(format!("case {n} (P): {what}: {p}"));
            tally.outcomes.push('P');
        }
        Ok(Err(e)) => {
            tally.refused(&e.to_string());
            tally.notes.push(format!("case {n}: {what}: refused: {e}"));
        }
        Ok(Ok(part)) => judge(tally, n, &what, &part.solid.body, want, f64::INFINITY, &tol),
    }
}

/// The bends: at angles from 3 to 175 degrees, down (whose bottom skin's
/// arcs have the inner radius `R`, traced over `θ + 4T/R`) at the
/// whole-turn radii with their hairs and the grid, up (radius `R + T`,
/// traced over `θ + 4T/(R + T)`: whole turns only past about 131 degrees)
/// likewise, and the sharp bend; each with a K-factor, a scale and a
/// plane drawn from the case's seed.
fn bend_cases(seed: u64) -> Vec<BendCase> {
    let mut out = Vec::new();
    let angles = [3.0, 30.0, 90.0, 140.0, 175.0].map(|a: f64| a.to_radians());
    for angle in angles {
        for direction in [Direction::Down, Direction::Up] {
            let mut rs: Vec<f64> = match direction {
                Direction::Down => radii(angle, 9e-4, 11.0),
                Direction::Up => {
                    let mut rs = Vec::new();
                    for n in TURNS {
                        let r = whole_turns(angle, 1.0, n) - 1.0;
                        if r > 1e-4 {
                            rs.extend(hairs(n).iter().map(|h| (r + 1.0) * h - 1.0));
                        }
                    }
                    rs.extend(GRID);
                    rs
                }
            };
            rs.push(0.0);
            for r in rs {
                let mut rng = Rng(case_seed(seed, out.len()));
                let k = SCALES[rng.below(3) as usize];
                let plane = frame(&mut rng, k);
                let kk = [0.3, 0.44, 0.5][rng.below(3) as usize];
                out.push(BendCase {
                    angle,
                    direction,
                    r,
                    kk,
                    k,
                    plane,
                    hole: None,
                });
            }
        }
    }
    out
}

/// A hole across a bend up or down of 90 or 140 degrees, of radius 0.4,
/// 0.89 or 1.3 of the thickness, at inner radii from 1e-3 to 1 of the
/// thickness, each with a K-factor, a scale and a plane drawn from the
/// case's seed.
fn hole_cases(seed: u64) -> Vec<BendCase> {
    let mut out = Vec::new();
    for r in [1e-3, 2e-3, 3e-3, 1e-2, 3e-2, 0.1, 0.3, 1.0] {
        for rho in [0.4, 0.89, 1.3] {
            for (angle, direction) in [
                (FRAC_PI_2, Direction::Up),
                (FRAC_PI_2, Direction::Down),
                (140f64.to_radians(), Direction::Up),
            ] {
                let mut rng = Rng(case_seed(seed, out.len()));
                let k = SCALES[rng.below(3) as usize];
                let plane = frame(&mut rng, k);
                let kk = [0.3, 0.44, 0.5][rng.below(3) as usize];
                out.push(BendCase {
                    angle,
                    direction,
                    r,
                    kk,
                    k,
                    plane,
                    hole: Some(rho),
                });
            }
        }
    }
    out
}

/// Runs `cases` with `run`, tallied.
fn tally_of<C>(cases: &[C], run: impl Fn(&mut Tally, usize, &C)) -> Tally {
    let mut tally = Tally::new();
    for (n, c) in cases.iter().enumerate() {
        run(&mut tally, n, c);
    }
    tally
}

/// Holds `tally`: no case wrong, invalid or a panic but `known`, no case
/// refused but `refused`, at least nine in ten built right.
fn held(tally: &Tally, name: &str, known: &[usize], refused: &[usize]) {
    tally.hold(name, 0, known, tally.outcomes.len() * 9 / 10);
    let got: Vec<usize> = tally
        .outcomes
        .chars()
        .enumerate()
        .filter_map(|(i, c)| (c == 'R').then_some(i))
        .collect();
    assert_eq!(got, refused, "{name}: refused {:#?}", tally.refusals);
}

/// Holes of arcs of one circle in a square, and a whole circle. Known,
/// the same before the run was kept within a turn (owed, the
/// tessellator): a hole of three arcs of radius 3 t bevelled by 0.41 rad
/// in a turned frame (case 388) measures as its closed form to the bit,
/// but one of its cones cannot be tessellated ("the boundary crosses
/// itself in parameter space"; its square twin, cylinders, is meshed).
#[test]
fn holes_of_arcs_at_whole_turns_are_their_closed_forms() {
    let cases = plate_cases(&HOLES, 0x9A7E_0001);
    held(&tally_of(&cases, plate_case), "holes", &[388], &[]);
}

/// Discs, rounded corners, slots and segments. Refused (owed, as before
/// the run was kept within a turn): a disc of two half circles bevelled
/// by 0.38 rad at `r/t` 1.27 about its middle (case 2; the edge across a
/// corner between two bevelled rims of one cone, traced where they meet,
/// does not settle: docs/design/plates.md › *Corners*, smooth vertices
/// between bevelled rims), and one of radius 10 bevelled by −0.31 rad
/// at 1e6 in a far frame (case 64; the rims there do not meet across the
/// plate). Known (owed, the tessellator): plates far thinner across than
/// they are thick, a disc of radius 3e-3 t (case 57) and segments of 10
/// degrees 1e-5 to 5e-4 t wide (cases 345, 364, 365, 375, 379), measure
/// as their closed forms (to 1e-9), but their meshes, at chords from 1e-2
/// to 1e-6 of their size, are a few triangles whose volumes are from half
/// to thousands of times the closed form's, or below zero (before, 57,
/// 364, 375 and 379 were refused, 365 the same and 345 meshed).
#[test]
fn discs_corners_slots_and_segments_at_whole_turns_are_their_closed_forms() {
    let cases = plate_cases(&OTHERS, 0x9A7E_0002);
    held(
        &tally_of(&cases, plate_case),
        "outlines",
        &[57, 345, 364, 365, 375, 379],
        &[2, 64],
    );
}

/// Bends up and down at whole turns.
#[test]
fn bends_at_whole_turns_are_their_closed_forms() {
    let cases = bend_cases(0x9A7E_0003);
    held(&tally_of(&cases, bend_case), "bends", &[], &[]);
}

/// Holes across bends. Known (owed, `hunt_sheet_repro.rs` ›
/// `a_hole_across_a_tight_bend_cannot_be_meshed`): a hole across a bend
/// down of a few thousandths of the thickness is meshed to its closed
/// form, but the measure does not settle on its rim (cases 1, 4, 7, 16,
/// 19, 22; such bends were refused until 2026-11-15); refused, as before:
/// a hole 0.4 of the thickness across a bend up of 1e-3, its diameter
/// just over the strip's width (case 0).
#[test]
fn holes_across_tight_bends_are_their_closed_forms() {
    let cases = hole_cases(0x9A7E_0004);
    held(
        &tally_of(&cases, bend_case),
        "holes across bends",
        &[1, 4, 7, 16, 19, 22],
        &[0],
    );
}

/// A square with an elliptic hole of two half ellipses of semi-axes `a`
/// and `b`, built as a sheet by hand (a sketch has no ellipse), the
/// hole's loop clockwise.
fn elliptic_sheet(frame: &Frame, half: f64, a: f64, b: f64) -> Body {
    use craft3d_geom::{Ellipse, Line, Plane, Surface};
    use craft3d_topo::{BodyKind, Coedge};
    let mut body = Body::new(BodyKind::Sheet);
    let corners = [(-half, -half), (half, -half), (half, half), (-half, half)];
    let vs: Vec<_> = corners
        .iter()
        .map(|(x, y)| body.add_vertex(frame.point(*x, *y, 0.0)))
        .collect();
    let mut outer = Vec::new();
    for i in 0..4 {
        let (p, q) = (corners[i], corners[(i + 1) % 4]);
        let (p, q) = (frame.point(p.0, p.1, 0.0), frame.point(q.0, q.1, 0.0));
        let line = Line::through(p, q).unwrap();
        let e = body.add_edge(
            Curve3::Line(line),
            (0.0, p.distance(q)),
            vs[i],
            vs[(i + 1) % 4],
        );
        outer.push(Coedge {
            edge: e,
            reversed: false,
        });
    }
    let ellipse = Curve3::Ellipse(Ellipse::new(*frame, a, b).unwrap());
    let va = body.add_vertex(ellipse.eval(0.0));
    let vb = body.add_vertex(ellipse.eval(PI));
    let top = body.add_edge(ellipse.clone(), (0.0, PI), va, vb);
    let bottom = body.add_edge(ellipse, (PI, TAU), vb, va);
    let hole = vec![
        Coedge {
            edge: bottom,
            reversed: true,
        },
        Coedge {
            edge: top,
            reversed: true,
        },
    ];
    let loops = vec![body.add_loop(outer), body.add_loop(hole)];
    let face = body.add_face(Surface::Plane(Plane::new(*frame)), false, loops);
    body.add_shell(vec![face]);
    body
}

/// An elliptic hole (`a = 1.7 b`) whose half ellipses were traced over a
/// whole number of turns (`π + 4t/b`, their speed at their ends being
/// `b`) and a hair beside, and at three other sizes: its rows are
/// ellipses, which are never taken for circles, so before a run was kept
/// within a turn its edges were cut from splines wound round the
/// ellipse. The closed form `(4 h² − π a b) t`; a side, a scale and a
/// frame from the case's seed.
#[test]
fn elliptic_holes_at_whole_turns_are_their_closed_forms() {
    let mut tally = Tally::new();
    let mut bs = Vec::new();
    for n in [1u32, 2, 3, 4, 5, 6, 8, 16, 64] {
        let b = 4.0 / (TAU * f64::from(n) - PI);
        bs.extend([b, b * (1.0 - 1e-9), b * (1.0 + 1e-9)]);
    }
    bs.extend([1e-2, 0.1, 1.0]);
    for (n, b) in bs.into_iter().enumerate() {
        let mut rng = Rng(case_seed(0x9A7E_0005, n));
        let side = match rng.below(3) {
            0 => Side::Out,
            1 => Side::In,
            _ => Side::Mid,
        };
        let k = SCALES[rng.below(3) as usize];
        let frame = frame(&mut rng, k);
        let tol = tolerance(k);
        let (a, half) = (1.7 * b, (5.1 * b).max(10.0));
        let sheet = elliptic_sheet(&frame, half * k, a * k, b * k);
        assert!(check(&sheet, &tol).is_empty(), "{n}: the sheet");
        let want = (4.0 * half * half - PI * a * b) * k * k * k;
        let what = format!("ellipse b/t {b:e}, {side:?}, k {k}, frame {frame:?}");
        match guarded(|| plate(&sheet, &[FaceId(0)], k, side, &BTreeMap::new(), &tol)) {
            Err(p) => {
                tally.wrong.push(format!("case {n} (P): {what}: {p}"));
                tally.outcomes.push('P');
            }
            Ok(Err(e)) => {
                tally.refused(&e.to_string());
                tally.notes.push(format!("case {n}: {what}: refused: {e}"));
            }
            Ok(Ok(made)) => judge(&mut tally, n, &what, &made.body, want, b * k, &tol),
        }
    }
    held(&tally, "elliptic holes", &[], &[]);
}
