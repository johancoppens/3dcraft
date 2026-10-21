//! The perturbed-body oracle of tolerant modelling (the decision of
//! 2026-10-18; `docs/design/booleans.md`, Method › *Tolerant operands*).
//!
//! Valid bodies (blocks, cylinders, spheres, tori, a freeform face, a
//! boolean of two of them) are perturbed the way STEP files are: a face's
//! surface moved off its edges, a vertex nudged off its edges' ends, an
//! edge's curve moved off its faces, a curve refitted coarser. Each gap is
//! measured and recorded as the entity's tolerance (`topo::record_edge`,
//! `topo::record_vertex`). Then:
//!
//! - (a) the perturbed body passes `topo::check` with the recorded
//!   tolerances, and fails without them and with a tenth of them, where
//!   the gap is one the check's own places see (a vertex or an edge's end
//!   moved);
//! - (b) a boolean between perturbed bodies is right (its volume within a
//!   bound derived from the tolerances of the boolean of the unperturbed
//!   bodies, and valid with its own table) or refused, never invalid, at
//!   1e-3, 1 and 1e6.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Curve3, NurbsCurve};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Transform, Vec3, bits_of};
use craft3d_testkit::parallel::map_parallel;
use craft3d_testkit::trimmed;
use craft3d_topo::{
    Body, EdgeId, FaceId, VertexId, check, check_capped, mass_properties, record_edge,
    record_vertex,
};

/// SplitMix64: seeded, the same numbers everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// In `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn direction(&mut self) -> Vec3 {
        loop {
            let v = Vec3::new(
                2.0 * self.unit() - 1.0,
                2.0 * self.unit() - 1.0,
                2.0 * self.unit() - 1.0,
            );
            let l = v.length();
            if l > 0.1 && l <= 1.0 {
                return v * (1.0 / l);
            }
        }
    }
}

fn tolerance(scale: f64) -> Tolerance {
    Tolerance {
        linear: 1e-6 * scale,
        angular: 1e-9,
    }
}

fn at(x: f64, y: f64, z: f64, scale: f64) -> Frame {
    Frame::WORLD.at(Vec3::new(x, y, z) * scale).unwrap()
}

/// The first operand: a 2 × 2 × 2 block, or the block less a cylinder
/// through it along x (a boolean of two bodies).
fn first(kind: usize, scale: f64) -> Body {
    let block = primitives::block(
        &at(0.0, 0.0, 0.0, scale),
        2.0 * scale,
        2.0 * scale,
        2.0 * scale,
    )
    .unwrap();
    if kind == 0 {
        return block;
    }
    let along_x = Frame::new(Vec3::new(-0.5, 1.0, 0.8) * scale, Vec3::X, Vec3::Y).unwrap();
    let hole = primitives::cylinder(&along_x, 0.4 * scale, 3.0 * scale).unwrap();
    boolean(&block, &hole, BooleanOp::Subtract, &tolerance(scale))
        .unwrap()
        .body
}

/// The second operand: a cylinder, a sphere, a torus, a freeform face (a
/// block with a NURBS roof) or a block, each through a face of the first.
fn second(kind: usize, scale: f64) -> Body {
    match kind {
        0 => primitives::cylinder(&at(1.1, 0.9, -0.5, scale), 0.6 * scale, 3.0 * scale).unwrap(),
        1 => primitives::sphere(&at(2.1, 1.05, 0.95, scale), 0.8 * scale).unwrap(),
        2 => primitives::torus(&at(0.95, 1.1, 2.05, scale), 0.7 * scale, 0.25 * scale).unwrap(),
        3 => {
            trimmed::domed_block(
                &at(0.6, 0.5, 0.4, scale),
                2.2 * scale,
                1.1 * scale,
                1.2 * scale,
                true,
            )
            .unwrap()
            .body
        }
        _ => primitives::block(
            &at(0.7, 0.6, 0.5, scale),
            1.9 * scale,
            1.8 * scale,
            1.7 * scale,
        )
        .unwrap(),
    }
}

const SECONDS: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Perturbation {
    /// A face's surface moved off its edges.
    FaceMoved,
    /// A vertex moved off the ends of its edges.
    VertexNudged,
    /// An edge's curve moved off its faces (and its ends off its vertices).
    EdgeMoved,
    /// An edge's curve refitted as a coarse cubic spline.
    Refitted,
}

/// The edges a face's loops use.
fn edges_of(body: &Body, face: FaceId) -> Vec<EdgeId> {
    let f = body.face(face).unwrap();
    let mut edges: Vec<EdgeId> = f
        .loops
        .iter()
        .flat_map(|l| body.loop_(*l).unwrap().coedges.iter().map(|c| c.edge))
        .collect();
    edges.sort();
    edges.dedup();
    edges
}

/// `curve` over `range` as a cubic spline through `spans + 1` of its
/// points with its tangents there (piecewise cubic Hermite, C1), on the
/// same parameters: the ends are the curve's, the rest lies off it by the
/// interpolation error.
fn hermite(curve: &Curve3, range: (f64, f64), spans: usize) -> Curve3 {
    let (t0, t1) = range;
    let h = (t1 - t0) / spans as f64;
    let mut points = Vec::new();
    let mut knots = vec![t0; 4];
    for i in 0..spans {
        let (a, b) = (t0 + h * i as f64, t0 + h * (i + 1) as f64);
        let (pa, da) = curve.derivatives(a);
        let (pb, db) = curve.derivatives(b);
        if i == 0 {
            points.push(pa);
        }
        points.push(pa + da * (h / 3.0));
        points.push(pb - db * (h / 3.0));
        points.push(pb);
        if i + 1 < spans {
            knots.extend([b, b, b]);
        }
    }
    knots.extend([t1; 4]);
    Curve3::Nurbs(NurbsCurve::polynomial(3, knots, points).unwrap())
}

/// The farthest of `samples` places of `a` from `b`'s points at the same
/// parameters.
fn apart(a: &Curve3, b: &Curve3, range: (f64, f64)) -> f64 {
    (0..=64)
        .map(|k| {
            let t = range.0 + (range.1 - range.0) * f64::from(k) / 64.0;
            a.eval(t).distance(b.eval(t))
        })
        .fold(0.0, f64::max)
}

/// `body` perturbed twice by up to `gap`, each gap measured and recorded;
/// the perturbations made and whether one of them moved a vertex or an
/// edge's end, which the check's own places see.
fn perturbed(
    body: &Body,
    rng: &mut Rng,
    gap: f64,
    tol: &Tolerance,
    cap: f64,
) -> (Body, Vec<Perturbation>, bool) {
    let mut out = body.clone();
    let mut made = Vec::new();
    let mut seen = false;
    for _ in 0..2 {
        let delta = gap * (0.5 + 0.5 * rng.unit());
        let dir = rng.direction();
        match rng.below(4) {
            0 => {
                let with_loops: Vec<usize> = (0..out.faces.len())
                    .filter(|&f| !out.faces[f].loops.is_empty())
                    .collect();
                if with_loops.is_empty() {
                    continue;
                }
                let face = FaceId(with_loops[rng.below(with_loops.len())] as u32);
                let f = &mut out.faces[face.0 as usize];
                f.surface = f
                    .surface
                    .transformed(&Transform::translation(dir * delta))
                    .unwrap();
                for edge in edges_of(&out, face) {
                    record_edge(&mut out, edge, tol, cap).unwrap();
                }
                made.push(Perturbation::FaceMoved);
            }
            1 if !out.vertices.is_empty() => {
                let v = VertexId(rng.below(out.vertices.len()) as u32);
                out.vertices[v.0 as usize].point += dir * delta;
                record_vertex(&mut out, v, tol, cap).unwrap();
                made.push(Perturbation::VertexNudged);
                seen |= delta > tol.linear;
            }
            2 if !out.edges.is_empty() => {
                let e = EdgeId(rng.below(out.edges.len()) as u32);
                let edge = &mut out.edges[e.0 as usize];
                edge.curve = edge
                    .curve
                    .transformed(&Transform::translation(dir * delta))
                    .unwrap();
                let ends = [edge.start, edge.end];
                record_edge(&mut out, e, tol, cap).unwrap();
                for v in ends {
                    record_vertex(&mut out, v, tol, cap).unwrap();
                }
                made.push(Perturbation::EdgeMoved);
                seen |= delta > tol.linear;
            }
            1 | 2 => continue,
            _ => {
                let curved: Vec<usize> = (0..out.edges.len())
                    .filter(|&e| !matches!(out.edges[e].curve, Curve3::Line(_)))
                    .collect();
                if curved.is_empty() {
                    continue;
                }
                let e = EdgeId(curved[rng.below(curved.len())] as u32);
                let edge = out.edges[e.0 as usize].clone();
                // As coarse as stays within the gap of the curve.
                let mut spans = 1;
                let fit = loop {
                    let fit = hermite(&edge.curve, edge.range, spans);
                    if apart(&fit, &edge.curve, edge.range) <= 0.5 * delta || spans >= 256 {
                        break fit;
                    }
                    spans *= 2;
                };
                out.edges[e.0 as usize].curve = fit;
                record_edge(&mut out, e, tol, cap).unwrap();
                made.push(Perturbation::Refitted);
            }
        }
    }
    (out, made, seen)
}

/// The total length of the body's edges (64 chords each, a little short
/// of it), and its area.
fn sizes(body: &Body) -> (f64, f64) {
    let length = body
        .edges
        .iter()
        .map(|e| {
            let (t0, t1) = e.range;
            (0..64)
                .map(|k| {
                    let t = |k: f64| t0 + (t1 - t0) * k / 64.0;
                    e.curve
                        .eval(t(f64::from(k)))
                        .distance(e.curve.eval(t(f64::from(k + 1))))
                })
                .sum::<f64>()
        })
        .sum::<f64>();
    (1.1 * length, mass_properties(body).unwrap().area)
}

#[derive(Debug, Default)]
struct Tally {
    bodies: usize,
    perturbations: std::collections::BTreeMap<String, usize>,
    fail_without: usize,
    fail_tenth: usize,
    seen: usize,
    booleans: usize,
    right: usize,
    /// Results that carry a table of their own.
    tolerant: usize,
    refused: std::collections::BTreeMap<String, usize>,
    reference_refused: usize,
    worst_ratio: f64,
}

fn refusal(error: &KernelError) -> String {
    match error {
        KernelError::Unsupported { reason, .. } => (*reason).to_string(),
        KernelError::InvalidBody { .. } => "invalid body (refused)".to_string(),
        other => format!("{other}").chars().take(60).collect(),
    }
}

fn run_case(scale: f64, k: f64, a_kind: usize, b_kind: usize, seed: u64) -> Tally {
    let tol = tolerance(scale);
    let cap = 1000.0 * tol.linear;
    let gap = k * tol.linear;
    let mut rng = Rng(seed);
    let mut tally = Tally::default();
    let a0 = first(a_kind, scale);
    let b0 = second(b_kind, scale);
    let mut operands = Vec::new();
    for body in [&a0, &b0] {
        let (p, made, seen) = perturbed(body, &mut rng, gap, &tol, cap);
        tally.bodies += 1;
        for m in &made {
            *tally.perturbations.entry(format!("{m:?}")).or_default() += 1;
        }
        // (a) With the recorded tolerances it passes.
        assert_eq!(
            check(&p, &tol),
            Vec::new(),
            "scale {scale}, k {k}, kinds {a_kind}/{b_kind}, seed {seed}: {made:?}"
        );
        let mut bare = p.clone();
        bare.tolerances = Default::default();
        let mut tenth = p.clone();
        for (e, t) in p.tolerances.edges() {
            tenth.set_edge_tolerance(e, t / 10.0).unwrap();
        }
        for (v, t) in p.tolerances.vertices() {
            tenth.set_vertex_tolerance(v, t / 10.0).unwrap();
        }
        let without = !check(&bare, &tol).is_empty();
        let with_tenth = !check(&tenth, &tol).is_empty();
        if seen {
            tally.seen += 1;
            assert!(
                without,
                "scale {scale}, k {k}, seed {seed}: {made:?} passes without"
            );
            assert!(
                with_tenth,
                "scale {scale}, k {k}, seed {seed}: {made:?} passes a tenth"
            );
        }
        tally.fail_without += usize::from(without);
        tally.fail_tenth += usize::from(with_tenth);
        operands.push(p);
    }
    let (a, b) = (&operands[0], &operands[1]);
    let working = a
        .working_tolerance(tol.linear)
        .max(b.working_tolerance(tol.linear));
    let (la, aa) = sizes(&a0);
    let (lb, ab) = sizes(&b0);
    let diagonal = {
        let (bb_a, bb_b) = (a0.bbox().unwrap(), b0.bbox().unwrap());
        let (a_lo, a_hi) = bb_a.corners().unwrap();
        let (b_lo, b_hi) = bb_b.corners().unwrap();
        craft3d_math::Bbox3::from_points([a_lo, a_hi, b_lo, b_hi]).diagonal()
    };
    for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
        let Ok(reference) = boolean(&a0, &b0, op, &tol) else {
            tally.reference_refused += 1;
            continue;
        };
        let v0 = mass_properties(&reference.body).unwrap().volume;
        tally.booleans += 1;
        match boolean(a, b, op, &tol) {
            Ok(result) => {
                // (b) Never invalid: it passes with its own table...
                assert_eq!(
                    check_capped(&result.body, &tol, cap),
                    Vec::new(),
                    "scale {scale}, k {k}, kinds {a_kind}/{b_kind}, seed {seed}, {op:?}"
                );
                // ... and its tolerances are those it was built at, at most.
                assert!(result.body.working_tolerance(tol.linear) <= working);
                let v = mass_properties(&result.body).unwrap().volume;
                // Each operand's faces within its working tolerance of the
                // unperturbed ones, its boundaries within it along every
                // edge, and the boolean's merging within it: the volume
                // moves by at most a slab of that thickness over the
                // faces and a strip of it along the edges, as far out as
                // the bodies reach.
                let bound = working * (aa + ab + (la + lb) * diagonal) + 1e-9 * v0.abs();
                let ratio = (v - v0).abs() / bound;
                tally.worst_ratio = tally.worst_ratio.max(ratio);
                assert!(
                    ratio <= 1.0,
                    "scale {scale}, k {k}, kinds {a_kind}/{b_kind}, seed {seed}, {op:?}: volume {v} against {v0}, bound {bound:e}"
                );
                tally.right += 1;
                tally.tolerant += usize::from(result.body.is_tolerant());
                // The same operands give the same body, bit for bit.
                let again = boolean(a, b, op, &tol).unwrap();
                assert_eq!(bits_of(&again.body), bits_of(&result.body));
            }
            Err(error) => {
                *tally.refused.entry(refusal(&error)).or_default() += 1;
            }
        }
    }
    tally
}

#[test]
fn perturbed_bodies_pass_with_their_tolerances_and_booleans_on_them_are_right_or_refused() {
    let mut cases = Vec::new();
    for scale in [1e-3_f64, 1.0, 1e6] {
        for k in [3.0_f64, 30.0, 300.0] {
            for a_kind in 0..2 {
                for b_kind in 0..SECONDS {
                    for s in 0..2u64 {
                        let seed = (scale.to_bits() ^ k.to_bits())
                            .wrapping_mul(31)
                            .wrapping_add((a_kind * 10 + b_kind) as u64 * 1000 + s);
                        cases.push((scale, k, a_kind, b_kind, seed));
                    }
                }
            }
        }
    }
    let tallies = map_parallel(cases, |(scale, k, a, b, seed)| {
        run_case(scale, k, a, b, seed)
    });
    let mut total = Tally::default();
    for t in tallies {
        total.bodies += t.bodies;
        for (m, n) in t.perturbations {
            *total.perturbations.entry(m).or_default() += n;
        }
        total.fail_without += t.fail_without;
        total.fail_tenth += t.fail_tenth;
        total.seen += t.seen;
        total.booleans += t.booleans;
        total.right += t.right;
        total.tolerant += t.tolerant;
        for (r, n) in t.refused {
            *total.refused.entry(r).or_default() += n;
        }
        total.reference_refused += t.reference_refused;
        total.worst_ratio = total.worst_ratio.max(t.worst_ratio);
    }
    println!("{total:#?}");
    // Most of them are results, not refusals.
    assert!(total.right * 2 > total.booleans, "{total:#?}");
}

#[test]
fn a_boolean_on_an_entity_above_the_cap_is_refused() {
    let tol = tolerance(1.0);
    let a = first(0, 1.0);
    let mut b = second(0, 1.0);
    b.set_edge_tolerance(EdgeId(0), 2e-3).unwrap();
    match boolean(&a, &b, BooleanOp::Union, &tol) {
        Err(KernelError::ToleranceAboveCap { tolerance, cap, .. }) => {
            assert_eq!((tolerance, cap), (2e-3, 1e-3));
        }
        other => panic!("{other:?}"),
    }
    let options = craft3d_kernel::boolean::BooleanOptions {
        loosest: Some(1e-2),
        ..Default::default()
    };
    let made =
        craft3d_kernel::boolean::boolean_with(&a, &b, BooleanOp::Union, &tol, &options).unwrap();
    assert!(made.body.is_tolerant());
}

#[test]
fn exact_operands_give_an_exact_result() {
    let tol = tolerance(1.0);
    let made = boolean(&first(0, 1.0), &second(0, 1.0), BooleanOp::Subtract, &tol).unwrap();
    assert!(!made.body.is_tolerant());
}
