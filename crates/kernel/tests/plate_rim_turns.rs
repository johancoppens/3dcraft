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
use craft3d_geom::{Curve3, Surface};
use craft3d_kernel::extrude::fill;
use craft3d_kernel::plate::{Joint, Side, plate};
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, Cutout, Direction, FlangeSpec, LengthFrom, On, Spec, build,
};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
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
    let normal = face_normal(&sheet);
    let want = Want {
        volume: volume_of(c, &shape, (levels.0 / k, levels.1 / k)) * k * k * k,
        feature: shape.feature * k,
        vertices: Some(expected_vertices(c, &placing, &sheet, normal, levels)),
        // A segment's bevelled rims meet at its corners along a conic
        // (a cone against a plane), not a line.
        lines_across: match (c.rims, c.outline) {
            (Rims::Bevel(_), Outline::Segment { .. }) => None,
            _ => Some(normal),
        },
    };
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
        Ok(Ok(made)) => judge(tally, n, &what, &made.body, &want, &tol),
    }
}

/// Where a plate of `sheet` with square ends has its vertices: each
/// vertex `v` of the sheet at each level `h` along the outward normal
/// `n(v)` of a face it bounds (the faces of a folded sheet are tangent
/// where they meet), `v + h n(v)`.
fn square_vertices(sheet: &Body, (lo, hi): (f64, f64)) -> Vec<Vec3> {
    let mut out = Vec::new();
    for (i, v) in sheet.vertices.iter().enumerate() {
        let face = sheet
            .faces
            .iter()
            .find(|f| {
                f.loops.iter().filter_map(|l| sheet.loop_(*l)).any(|l| {
                    l.coedges
                        .iter()
                        .filter_map(|c| sheet.edge(c.edge))
                        .any(|e| e.start.0 as usize == i || e.end.0 as usize == i)
                })
            })
            .expect("a vertex of a face");
        let (u, w) = face.surface.project(v.point).unwrap();
        let n = face.surface.normal(u, w).unwrap() * if face.reversed { -1.0 } else { 1.0 };
        out.push(v.point + n * lo);
        out.push(v.point + n * hi);
    }
    out
}

/// The outward normal of the flat sheet's one face.
fn face_normal(sheet: &Body) -> Vec3 {
    let face = &sheet.faces[0];
    let Surface::Plane(p) = &face.surface else {
        panic!("a flat sheet");
    };
    p.frame().z() * if face.reversed { -1.0 } else { 1.0 }
}

/// Where the plate's vertices lie, worked out apart from the kernel: each
/// vertex `v` of the sheet at each level `h` along the face's outward
/// normal `n`, `v + h n` at a square end; on a bevelled arc of one circle
/// (a hole's, a disc's) moved along its radius out of the material by
/// `h tan α` (so on the circle `r ∓ h tan α`); at a segment's corner
/// (both its edges bevelled) where the circle `r + h tan α` meets the
/// chord moved out by `h tan α`.
fn expected_vertices(
    c: &PlateCase,
    placing: &Placing,
    sheet: &Body,
    normal: Vec3,
    (lo, hi): (f64, f64),
) -> Vec<Vec3> {
    let k = c.k;
    let world = |p: P2| c.frame.point(p[0], p[1], 0.0);
    let centre = world(placing.point([0.0, 0.0]));
    let r = c.r * k;
    let along = |d: P2| {
        let q = placing.vector(d);
        c.frame.vector(q[0], q[1], 0.0)
    };
    let mut out = Vec::new();
    for v in &sheet.vertices {
        let p = v.point;
        for h in [lo, hi] {
            let q = match (c.rims, c.outline) {
                (Rims::Bevel(alpha), Outline::Hole { .. } | Outline::Disc { .. })
                    if ((p - centre).length() - r).abs() <= 1e-9 * r.max(1.0) + 16.0 * 1e-6 * k =>
                {
                    let d = h * alpha.ptan();
                    let grown = match c.outline {
                        Outline::Hole { .. } => r - d,
                        _ => r + d,
                    };
                    centre + (p - centre) * (grown / r)
                }
                (Rims::Bevel(alpha), Outline::Segment { s }) => {
                    let d = h * alpha.ptan();
                    let x0 = r * (0.5 * s).pcos() - d;
                    let big = r + d;
                    let y = (big * big - x0 * x0).max(0.0).sqrt();
                    let (e, f) = (along([1.0, 0.0]), along([0.0, 1.0]));
                    let side = if (p - centre).dot(f) >= 0.0 {
                        1.0
                    } else {
                        -1.0
                    };
                    centre + e * x0 + f * (side * y)
                }
                _ => p,
            };
            out.push(q + normal * h);
        }
    }
    out
}

/// What a body is held to: its closed-form volume, its narrowest width
/// (the mesh's chord is at most a quarter of it), its vertices each at
/// its station's closed form (where known), and the plates' normal, along
/// which every edge across the plate (from one level to the other) at a
/// smooth or square corner is a line (where asked).
struct Want {
    volume: f64,
    feature: f64,
    vertices: Option<Vec<Vec3>>,
    lines_across: Option<Vec3>,
}

/// How far each vertex of `body` lies from the nearest of `want`, and the
/// other way round: the larger of the two worst, or infinity when the
/// counts differ.
fn vertices_off(body: &Body, want: &[Vec3]) -> f64 {
    if body.vertices.len() != want.len() {
        return f64::INFINITY;
    }
    let nearest = |p: Vec3, among: &mut dyn Iterator<Item = Vec3>| {
        among.map(|q| q.distance(p)).fold(f64::INFINITY, f64::min)
    };
    let got: Vec<Vec3> = body.vertices.iter().map(|v| v.point).collect();
    let a = got
        .iter()
        .map(|p| nearest(*p, &mut want.iter().copied()))
        .fold(0.0, f64::max);
    let b = want
        .iter()
        .map(|p| nearest(*p, &mut got.iter().copied()))
        .fold(0.0, f64::max);
    a.max(b)
}

/// Holds a built body to `want` by two means, the measure and the mesh
/// (at a thousandth of its size, or a quarter of its narrowest width
/// where that is less), after the checker; to its vertices' closed forms
/// (each within 16 tolerances) and its edges across the plate at smooth
/// or square corners being lines; and where all that holds, whether it
/// passes through itself: `I` invalid, `M` checked but not meshed, `W`
/// measured or meshed off the closed form, a vertex off, a corner's edge
/// curved (or not measured), `X` through itself, `B` right.
fn judge(tally: &mut Tally, n: usize, what: &str, body: &Body, want: &Want, tol: &Tolerance) {
    let mut found = Found::default();
    let defects = check(body, tol);
    if let Some(first) = defects.first() {
        found.invalid = true;
        found.problems.push(format!("not valid: {first:?}"));
        tally.record(n, what, &found, false);
        return;
    }
    if let Some(vs) = &want.vertices {
        let off = vertices_off(body, vs);
        if !(off <= 16.0 * tol.linear) {
            found.problems.push(format!(
                "vertices: {} for {}, the worst {:.3e} tolerances from its closed form",
                body.vertices.len(),
                vs.len(),
                off / tol.linear
            ));
        }
    }
    if let Some(normal) = want.lines_across {
        for (i, e) in body.edges.iter().enumerate() {
            let (Some(a), Some(b)) = (body.vertex(e.start), body.vertex(e.end)) else {
                continue;
            };
            let rise = (b.point - a.point).dot(normal).abs();
            if rise > 10.0 * tol.linear && !matches!(e.curve, Curve3::Line(_)) {
                found.problems.push(format!(
                    "edge {i} across the plate is a {}, not a line",
                    e.curve.kind()
                ));
                break;
            }
        }
    }
    let feature = want.feature;
    let want = want.volume;
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
        Ok(Ok(part)) => {
            let want = Want {
                volume: want,
                feature: f64::INFINITY,
                vertices: Some(square_vertices(&part.sheet, (0.0, t))),
                lines_across: None,
            };
            judge(tally, n, &what, &part.solid.body, &want, &tol)
        }
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
fn elliptic_sheet(frame: &Frame, half: f64, a: f64, b: f64, rational: bool) -> Body {
    use craft3d_geom::{Ellipse, Line, NurbsCurve, Plane, Surface};
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
    // The analytic ellipse over `0..π..2π`, or the rational quadratic
    // spline of the circle's net (four spans, as a file gives it) scaled
    // to the ellipse, over `0..½..1`.
    let (ellipse, half_way, whole) = if rational {
        let circle = NurbsCurve::circle(&Frame::WORLD, 1.0).unwrap();
        let ctrl = circle
            .control_points()
            .iter()
            .map(|p| frame.point(a * p.x, b * p.y, 0.0))
            .collect();
        let net =
            NurbsCurve::new(2, circle.knots().to_vec(), ctrl, circle.weights().to_vec()).unwrap();
        (Curve3::Nurbs(net), 0.5, 1.0)
    } else {
        (
            Curve3::Ellipse(Ellipse::new(*frame, a, b).unwrap()),
            PI,
            TAU,
        )
    };
    let va = body.add_vertex(ellipse.eval(0.0));
    let vb = body.add_vertex(ellipse.eval(half_way));
    let top = body.add_edge(ellipse.clone(), (0.0, half_way), va, vb);
    let bottom = body.add_edge(ellipse, (half_way, whole), vb, va);
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
        let sheet = elliptic_sheet(&frame, half * k, a * k, b * k, false);
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
            Ok(Ok(made)) => {
                let want = Want {
                    volume: want,
                    feature: b * k,
                    vertices: Some(square_vertices(&sheet, side.levels(k))),
                    lines_across: Some(face_normal(&sheet)),
                };
                judge(&mut tally, n, &what, &made.body, &want, &tol)
            }
        }
    }
    held(&tally, "elliptic holes", &[], &[]);
}

/// Holes and discs of two or three arcs of one circle (half circles,
/// thirds, and an arc of 0.02 rad with the rest of the turn), every arc
/// bevelled by ±0.3 rad, on every side, at r/t 0.5 to 5: the rims of two
/// arcs are one cone, so at each vertex the corner is where the rows
/// reach it (`plate::runs_on`). Held to the vertices' closed forms (on
/// the circle `r ∓ h tan α` at each cut), the edges across the corners
/// being the cone's rulings (lines), the volume and the mesh. Before
/// 2026-11-15 (round 2) the corners were solved on the two coinciding
/// cones and slid round the circle.
#[test]
fn bevelled_arcs_of_one_circle_have_their_corners_at_their_stations() {
    let mut tally = Tally::new();
    let mut n = 0;
    for outline in [
        Outline::Hole { s: PI, n: 2 },
        Outline::Hole { s: TAU / 3.0, n: 3 },
        Outline::Hole { s: 0.02, n: 2 },
        Outline::Disc { s: PI },
        Outline::Disc { s: 0.02 },
    ] {
        for r in [0.5, 1.0, 2.0, 5.0] {
            for alpha in [0.3, -0.3] {
                for side in [Side::Out, Side::In, Side::Mid] {
                    let mut rng = Rng(case_seed(0x9A7E_0006, n));
                    let k = SCALES[rng.below(3) as usize];
                    let c = PlateCase {
                        outline,
                        r,
                        side,
                        rims: Rims::Bevel(alpha),
                        k,
                        frame: frame(&mut rng, k),
                        turn: rng.between(0.0, TAU),
                    };
                    plate_case(&mut tally, n, &c);
                    n += 1;
                }
            }
        }
    }
    held(&tally, "bevelled arcs of one circle", &[], &[]);
}

/// The refusal of a corner that a run kept within a turn does not reach:
/// a segment of an ellipse (semi-axes 1.2 and 1) of 355° and its chord,
/// 100 thick, both edges bevelled by atan 6.23 (81°), so that at the
/// front the rims have moved out by 623 and meet about 0.1° from the
/// chord's middle: the corners past the arc's two ends leave less than an
/// eighth of the 5° between them, more than the run may cover, and an
/// ellipse's rows are splines, cut where the corners lie on them. (On a
/// circle the same corners are built: its rows are the circle, which
/// holds every angle, `plate_rim_turns` › the segments.)
#[test]
fn a_corner_further_round_than_a_kept_run_is_refused_saying_so() {
    use craft3d_geom::{Ellipse, Line, Plane};
    use craft3d_topo::{BodyKind, Coedge};
    let tol = tolerance(1.0);
    let s = 355f64.to_radians();
    let ellipse = Curve3::Ellipse(Ellipse::new(Frame::WORLD, 1.2, 1.0).unwrap());
    let mut sheet = Body::new(BodyKind::Sheet);
    let (p, q) = (ellipse.eval(-0.5 * s), ellipse.eval(0.5 * s));
    let (vp, vq) = (sheet.add_vertex(p), sheet.add_vertex(q));
    let arc = sheet.add_edge(ellipse, (-0.5 * s, 0.5 * s), vp, vq);
    let chord = sheet.add_edge(
        Curve3::Line(Line::through(q, p).unwrap()),
        (0.0, q.distance(p)),
        vq,
        vp,
    );
    let lp = sheet.add_loop(vec![
        Coedge {
            edge: arc,
            reversed: false,
        },
        Coedge {
            edge: chord,
            reversed: false,
        },
    ]);
    let face = sheet.add_face(Surface::Plane(Plane::new(Frame::WORLD)), false, vec![lp]);
    sheet.add_shell(vec![face]);
    assert!(check(&sheet, &tol).is_empty(), "the sheet");
    let bevel = Joint::Bevel(6.23f64.patan());
    let joints: BTreeMap<EdgeId, Joint<'_>> = [(arc, bevel), (chord, bevel)].into_iter().collect();
    let e = plate(&sheet, &[FaceId(0)], 100.0, Side::Out, &joints, &tol).expect_err("refused");
    assert!(
        e.to_string()
            .contains("seven eighths of what is left of its turn"),
        "another reason: {e}"
    );
}

/// Rims that end differently where the boundary runs on smoothly are
/// refused, saying so: a slot whose half circles are bevelled by ±0.3 rad
/// and whose straight sides end square, on every side, at three scales.
/// At a level where the bevel has grown the circle past the sides' lines
/// the rims meet past the vertex along a side (where the circle crosses
/// the line), elsewhere nowhere; the plate does not trace that, and before
/// (round 2) it said "the rims … do not meet in one point", which was not
/// true where they do. The same slot with every edge square, or every
/// edge bevelled alike, is built (`bevelled_arcs_of_one_circle…`, the
/// outlines' slots).
#[test]
fn rims_that_end_differently_at_a_smooth_corner_are_refused_saying_so() {
    for (n, (alpha, side, k)) in [
        (0.3, Side::Out, 1.0),
        (-0.3, Side::Out, 1e-3),
        (0.3, Side::In, 1e6),
        (-0.3, Side::Mid, 1.0),
    ]
    .into_iter()
    .enumerate()
    {
        let tol = tolerance(k);
        let c = PlateCase {
            outline: Outline::Slot { split: 1 },
            r: 1.0,
            side,
            rims: Rims::Square,
            k,
            frame: Frame::WORLD,
            turn: 0.3,
        };
        let shape = shape_of(&c);
        let placing = Placing {
            turn: c.turn,
            k,
            at: [0.0, 0.0],
            flip_lines: false,
        };
        let profiles = detect_profiles(&sketch(&shape, &placing, 1), &tol).unwrap();
        let sheet = fill(&profiles, &Frame::WORLD, &tol).unwrap().body;
        let joints: BTreeMap<EdgeId, Joint<'_>> = (0..sheet.edges.len())
            .filter(|i| matches!(sheet.edges[*i].curve, Curve3::Circle(_)))
            .map(|i| (EdgeId(i as u32), Joint::Bevel(alpha)))
            .collect();
        assert_eq!(joints.len(), 2, "{n}: the slot's two arcs");
        let e = plate(&sheet, &[FaceId(0)], k, side, &joints, &tol).expect_err("refused");
        assert!(
            e.to_string().contains(
                "the boundary runs on smoothly, but the rims of its two edges (one bevelled by"
            ),
            "{n}: another reason: {e}"
        );
        // Every edge of the slot bevelled alike: its hole at the level `h`
        // is the slot of radius `r − h tan α` about the same centres, its
        // vertices `(±l, ±(r − h tan α))`, its area `4 l ρ + π ρ²`.
        let (r, l) = (k, 2.0 * k);
        let joints: BTreeMap<EdgeId, Joint<'_>> = (0..sheet.edges.len())
            .filter(|i| {
                let e = &sheet.edges[*i];
                e.curve.eval(0.5 * (e.range.0 + e.range.1)).length() < 4.0 * k
            })
            .map(|i| (EdgeId(i as u32), Joint::Bevel(alpha)))
            .collect();
        assert_eq!(joints.len(), 4, "{n}: the slot's four edges");
        let made = plate(&sheet, &[FaceId(0)], k, side, &joints, &tol)
            .unwrap_or_else(|e| panic!("{n}: {e}"));
        let (lo, hi) = side.levels(k);
        let tau = alpha.ptan();
        let rho = |h: f64| r - h * tau;
        let half = 10.0 * k;
        let normal = face_normal(&sheet);
        let mut vs: Vec<Vec3> = square_vertices(&sheet, (lo, hi))
            .into_iter()
            .filter(|p| p.x.abs() > 5.0 * k)
            .collect();
        let (sn, cs) = c.turn.psin_cos();
        for h in [lo, hi] {
            for (x, y) in [(l, rho(h)), (-l, rho(h)), (l, -rho(h)), (-l, -rho(h))] {
                vs.push(Vec3::new(cs * x - sn * y, sn * x + cs * y, 0.0) + normal * h);
            }
        }
        let want = Want {
            volume: 4.0 * half * half * k
                - integral(lo, hi, |h| 4.0 * l * rho(h) + PI * rho(h) * rho(h)),
            feature: r,
            vertices: Some(vs),
            lines_across: Some(normal),
        };
        let mut tally = Tally::new();
        judge(
            &mut tally,
            n,
            "a slot bevelled alike",
            &made.body,
            &want,
            &tol,
        );
        assert_eq!(tally.outcomes, "B", "{n}: {:?}", tally.wrong);
    }
}

/// KNOWN, owed (docs/design/plates.md › *Limits*, *Runs that come back
/// round*): an elliptic hole drawn as two halves of a rational spline
/// ellipse (the circle's net of four quadratic spans scaled, as a file
/// gives an ellipse), b/a 0.5 or 0.1, `b` from 0.05 to 1 of the plate's
/// thickness, is refused (for reasons that are not the cause) or not
/// meshed, the same before and after a
/// run on a circle or an ellipse was kept within a turn. A spline edge has
/// no period, so its run is not kept: each half is traced past its ends
/// round the rest of its closed curve and on along the end span's rational
/// continuation, which goes on round the ellipse (read, not shown to be
/// the cause). The same ellipse as an ellipse is built
/// (`elliptic_holes_at_whole_turns_are_their_closed_forms`). When it is
/// fixed this test fails: take it off.
#[test]
fn a_rational_spline_ellipse_hole_is_refused_or_not_meshed() {
    for (n, (ratio, b)) in [(0.5, 0.05), (0.1, 0.18), (0.1, 1.0)]
        .into_iter()
        .enumerate()
    {
        let tol = tolerance(1.0);
        let sheet = elliptic_sheet(&Frame::WORLD, 10.0, b / ratio, b, true);
        assert!(check(&sheet, &tol).is_empty(), "{n}: the sheet");
        let got = match plate(&sheet, &[FaceId(0)], 1.0, Side::Out, &BTreeMap::new(), &tol) {
            Err(_) => 'R',
            Ok(made) => {
                if mesh_volume(&made.body, 1e-3 * size_of(&made.body)).is_err()
                    || measured(&made.body).is_none()
                {
                    'M'
                } else {
                    let want = 400.0 - PI * b * b / ratio;
                    let m = measured(&made.body).unwrap();
                    if (m.volume / want - 1.0).abs() < 1e-6 {
                        'B'
                    } else {
                        'W'
                    }
                }
            }
        };
        assert!(
            got == 'R' || got == 'M',
            "{n} (b/a {ratio}, b/t {b}): known refused or not meshed, now {got}: take this off"
        );
    }
}

/// Hostile numbers about a tight arc: a thickness that is not a number,
/// zero, negative, infinite or within the tolerance is refused; a
/// thickness of 1e300 or a hundred million times the hole's radius, and a
/// bevel near square, are built or refused, never a panic; a bend's
/// inner radius that is not a number, negative or infinite is refused,
/// and one of 1e-300 or 1e300 is built or refused, never a panic.
#[test]
fn hostile_numbers_about_a_tight_arc_are_refused_or_built_never_a_panic() {
    let tol = tolerance(1.0);
    let hole = reversed(&split_circle([0.0, 0.0], 1e-3, &[0.3, 0.3 + PI]));
    let shape = Shape {
        what: String::new(),
        outer: polygon_loop(&[[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0]]),
        holes: vec![hole],
        inner: ([5.0, 5.0], 3.0),
        hole_discs: vec![([0.0, 0.0], 1e-3)],
        feature: 1e-3,
    };
    let placing = Placing {
        turn: 0.0,
        k: 1.0,
        at: [0.0, 0.0],
        flip_lines: false,
    };
    let profiles = detect_profiles(&sketch(&shape, &placing, 1), &tol).unwrap();
    let sheet = fill(&profiles, &Frame::WORLD, &tol).unwrap().body;
    for t in [f64::NAN, 0.0, -1.0, f64::INFINITY, 1e-7] {
        let made = guarded(|| plate(&sheet, &[FaceId(0)], t, Side::Out, &BTreeMap::new(), &tol));
        assert!(matches!(made, Ok(Err(_))), "{t}: {made:?}");
    }
    let arcs: Vec<EdgeId> = (0..sheet.edges.len())
        .filter(|i| matches!(sheet.edges[*i].curve, Curve3::Circle(_)))
        .map(|i| EdgeId(i as u32))
        .collect();
    for (t, joint) in [
        (1e300, Joint::Square),
        (1e5, Joint::Square),
        (1.0, Joint::Bevel(FRAC_PI_2 - 1e-4)),
        (1.0, Joint::Bevel(1e-4 - FRAC_PI_2)),
    ] {
        let joints: BTreeMap<EdgeId, Joint<'_>> = arcs.iter().map(|e| (*e, joint)).collect();
        let made = guarded(|| plate(&sheet, &[FaceId(0)], t, Side::Mid, &joints, &tol));
        assert!(made.is_ok(), "{t} {joint:?}: a panic: {made:?}");
    }
    for r in [f64::NAN, -1.0, f64::INFINITY, 1e-300, 1e300] {
        let c = BendCase {
            angle: FRAC_PI_2,
            direction: Direction::Down,
            r,
            kk: 0.4,
            k: 1.0,
            plane: Frame::WORLD,
            hole: None,
        };
        let mut tally = Tally::new();
        bend_case(&mut tally, 0, &c);
        let got = tally.outcomes.chars().next().unwrap();
        assert!(got != 'P', "R {r}: a panic: {:?}", tally.wrong);
        if !(r >= 0.0 && r.is_finite()) {
            assert_eq!(got, 'R', "R {r}: {:?}", tally.wrong);
        }
    }
}
