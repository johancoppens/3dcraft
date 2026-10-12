//! Booleans on seeded random pairs (docs/testing.md, oracle 3): every case
//! ends as a valid body with the identities holding, as an empty result,
//! or as `Unsupported`. Never a panic, an invalid body or a wrong volume.
//!
//! The identities compare the booleans with each other, so a kernel that
//! missed every intersection would satisfy them. Two independent checks
//! close that gap: the overlap of two axis-aligned boxes is known exactly,
//! and for every other pair the volume of the intersection is estimated by
//! sampling points against the operands' own definitions.

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, check, mass_properties};
use std::collections::BTreeMap;

use BooleanOp::{Intersect, Subtract, Union};

/// How a pair is placed.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Placement {
    /// Each operand turned and moved at random.
    General,
    /// Both turned the same way, moved at random: parallel axes and faces,
    /// nothing coincident.
    Parallel,
    /// Along the world axes with every size and position a multiple of a
    /// half: flush faces, touching faces, shared edges, tangent cylinders.
    Grid,
}

/// An operand with its own definition of what is inside it.
struct Shape {
    body: Body,
    contains: Box<dyn Fn(Vec3) -> bool>,
    /// A box in world axes that contains it.
    lo: Vec3,
    hi: Vec3,
    /// For an axis-aligned box: its corners.
    aligned: Option<(Vec3, Vec3)>,
}

fn bounds(frame: &Frame, lo: [f64; 3], hi: [f64; 3]) -> (Vec3, Vec3) {
    let mut min = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut max = -min;
    for i in 0..8 {
        let pick = |axis: usize| {
            if i >> axis & 1 == 0 {
                lo[axis]
            } else {
                hi[axis]
            }
        };
        let p = frame.point(pick(0), pick(1), pick(2));
        min = min.min(p);
        max = max.max(p);
    }
    (min, max)
}

fn line(id: u64, start: [f64; 2], end: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

fn polygon(points: &[[f64; 2]]) -> Vec<SketchCurve> {
    (0..points.len())
        .map(|i| line(i as u64 + 1, points[i], points[(i + 1) % points.len()]))
        .collect()
}

fn random_frame(rng: &mut Rng, placement: Placement, shared: &Frame, k: f64) -> Frame {
    match placement {
        Placement::General => {
            let v = |rng: &mut Rng| {
                Vec3::new(
                    rng.between(-1.0, 1.0),
                    rng.between(-1.0, 1.0),
                    rng.between(-1.0, 1.0),
                )
            };
            let origin = v(rng) * (1.5 * k);
            loop {
                if let Some(frame) = Frame::new(origin, v(rng), v(rng)) {
                    return frame;
                }
            }
        }
        Placement::Parallel => {
            let origin = shared.point(
                rng.between(-1.5, 1.5) * k,
                rng.between(-1.5, 1.5) * k,
                rng.between(-1.5, 1.5) * k,
            );
            shared.at(origin).unwrap()
        }
        Placement::Grid => {
            let half = |rng: &mut Rng| (rng.below(5) as f64 - 2.0) * 0.5 * k;
            let origin = Vec3::new(half(rng), half(rng), half(rng));
            // Upright mostly; sometimes lying along x or y.
            match rng.below(6) {
                0 => Frame::new(origin, Vec3::X, Vec3::Y).unwrap(),
                1 => Frame::new(origin, Vec3::Y, Vec3::Z).unwrap(),
                _ => Frame::WORLD.at(origin).unwrap(),
            }
        }
    }
}

/// A size: any in general, a multiple of a half on the grid.
fn size(rng: &mut Rng, placement: Placement, k: f64) -> f64 {
    if placement == Placement::Grid {
        (rng.below(5) as f64 + 1.0) * 0.5 * k
    } else {
        rng.between(0.5, 3.0) * k
    }
}

fn random_shape(rng: &mut Rng, placement: Placement, shared: &Frame, k: f64) -> Shape {
    let frame = random_frame(rng, placement, shared, k);
    let tol = tolerance(k);
    let (a, b, c) = (
        size(rng, placement, k),
        size(rng, placement, k),
        size(rng, placement, k),
    );
    match rng.below(6) {
        0 | 1 => {
            let (lo, hi) = bounds(&frame, [0.0; 3], [a, b, c]);
            let aligned = (frame.z() == Vec3::Z && frame.x() == Vec3::X).then_some((lo, hi));
            Shape {
                body: primitives::block(&frame, a, b, c).unwrap(),
                contains: Box::new(move |p| {
                    let (x, y, z) = frame.local(p);
                    (0.0..=a).contains(&x) && (0.0..=b).contains(&y) && (0.0..=c).contains(&z)
                }),
                lo,
                hi,
                aligned,
            }
        }
        2 | 3 => {
            let r = a * 0.5;
            let (lo, hi) = bounds(&frame, [-r, -r, 0.0], [r, r, c]);
            Shape {
                body: primitives::cylinder(&frame, r, c).unwrap(),
                contains: Box::new(move |p| {
                    let (x, y, z) = frame.local(p);
                    x * x + y * y <= r * r && (0.0..=c).contains(&z)
                }),
                lo,
                hi,
                aligned: None,
            }
        }
        4 => {
            // An L: a by b with the corner beyond (a/2, b/2) cut away.
            let (ha, hb) = (a * 0.5, b * 0.5);
            let curves = polygon(&[[0.0, 0.0], [a, 0.0], [a, hb], [ha, hb], [ha, b], [0.0, b]]);
            let profiles = detect_profiles(&curves, &tol).unwrap();
            let (lo, hi) = bounds(&frame, [0.0; 3], [a, b, c]);
            Shape {
                body: extrude(&profiles, &frame, 0.0, c, &tol).unwrap().body,
                contains: Box::new(move |p| {
                    let (x, y, z) = frame.local(p);
                    (0.0..=c).contains(&z)
                        && (0.0..=a).contains(&x)
                        && (0.0..=b).contains(&y)
                        && !(x > ha && y > hb)
                }),
                lo,
                hi,
                aligned: None,
            }
        }
        _ => {
            // A slot (two half circles joined by lines) with a round hole.
            let (r, hole) = (b * 0.5, b * 0.25);
            let curves = vec![
                line(1, [0.0, -r], [a, -r]),
                SketchCurve {
                    id: 2,
                    curve: Curve2::Arc {
                        center: [a, 0.0],
                        start: [a, -r],
                        end: [a, r],
                    },
                },
                line(3, [a, r], [0.0, r]),
                SketchCurve {
                    id: 4,
                    curve: Curve2::Arc {
                        center: [0.0, 0.0],
                        start: [0.0, r],
                        end: [0.0, -r],
                    },
                },
                SketchCurve {
                    id: 5,
                    curve: Curve2::Circle {
                        center: [a * 0.5, 0.0],
                        radius: hole,
                    },
                },
            ];
            let profiles = detect_profiles(&curves, &tol).unwrap();
            let (lo, hi) = bounds(&frame, [-r, -r, 0.0], [a + r, r, c]);
            Shape {
                body: extrude(&profiles, &frame, 0.0, c, &tol).unwrap().body,
                contains: Box::new(move |p| {
                    let (x, y, z) = frame.local(p);
                    let within = |cx: f64, radius: f64| (x - cx).hypot(y) <= radius;
                    let slot =
                        ((0.0..=a).contains(&x) && y.abs() <= r) || within(0.0, r) || within(a, r);
                    (0.0..=c).contains(&z) && slot && !within(a * 0.5, hole)
                }),
                lo,
                hi,
                aligned: None,
            }
        }
    }
}

/// The volume common to both shapes, by sampling: the estimate and three
/// standard deviations of it.
fn sampled_overlap(a: &Shape, b: &Shape, rng: &mut Rng) -> (f64, f64) {
    let lo = a.lo.max(b.lo);
    let hi = a.hi.min(b.hi);
    if lo.x >= hi.x || lo.y >= hi.y || lo.z >= hi.z {
        return (0.0, 0.0);
    }
    let cell = (hi.x - lo.x) * (hi.y - lo.y) * (hi.z - lo.z);
    const SAMPLES: usize = 3000;
    let mut inside = 0_usize;
    for _ in 0..SAMPLES {
        let p = Vec3::new(
            rng.between(lo.x, hi.x),
            rng.between(lo.y, hi.y),
            rng.between(lo.z, hi.z),
        );
        if (a.contains)(p) && (b.contains)(p) {
            inside += 1;
        }
    }
    let fraction = inside as f64 / SAMPLES as f64;
    let deviation = (fraction * (1.0 - fraction) / SAMPLES as f64).sqrt();
    // With no sample inside the overlap may still be a sliver.
    (
        cell * fraction,
        cell * (3.0 * deviation + 3.0 / SAMPLES as f64),
    )
}

/// The volume of a boolean; 0 when it is empty; `Err` with the reason when
/// it is not supported. Anything else fails the test.
fn volume_of(
    a: &Body,
    b: &Body,
    op: BooleanOp,
    k: f64,
    what: &str,
) -> Result<(f64, f64, Option<Body>), &'static str> {
    let tol = tolerance(k);
    match boolean(a, b, op, &tol) {
        Ok(result) => {
            assert_eq!(check(&result.body, &tol), [], "{what}: {op:?}");
            assert_eq!(result.faces.len(), result.body.faces.len(), "{what}");
            assert_eq!(result.edges.len(), result.body.edges.len(), "{what}");
            assert_eq!(result.vertices.len(), result.body.vertices.len(), "{what}");
            let m = match mass_properties(&result.body) {
                Ok(m) => m,
                Err(e) => panic!("{what}: {op:?}: mass properties: {e}"),
            };
            assert!(m.volume > 0.0, "{what}: {op:?}: volume {}", m.volume);
            Ok((m.volume, m.area, Some(result.body)))
        }
        Err(KernelError::EmptyResult { .. }) => Ok((0.0, 0.0, None)),
        Err(KernelError::Unsupported { reason, .. }) => Err(reason),
        Err(other) => panic!("{what}: {op:?}: {other}"),
    }
}

#[derive(Default)]
struct Tally {
    pairs: usize,
    /// All six booleans gave a body or an empty result.
    solved: usize,
    /// Of those, pairs that overlap.
    overlapping: usize,
    unsupported: BTreeMap<&'static str, usize>,
    recombined: usize,
    recombine_unsupported: BTreeMap<&'static str, usize>,
}

fn run_pair(seed: u64, placement: Placement, k: f64, tally: &mut Tally) {
    let mut rng = Rng(seed);
    let shared = random_frame(&mut rng, Placement::General, &Frame::WORLD, k);
    let a = random_shape(&mut rng, placement, &shared, k);
    let b = random_shape(&mut rng, placement, &shared, k);
    let what = format!("seed {seed} {placement:?} scale {k}");
    tally.pairs += 1;

    let (va, aa) = {
        let m = mass_properties(&a.body).unwrap();
        (m.volume, m.area)
    };
    let (vb, ab) = {
        let m = mass_properties(&b.body).unwrap();
        (m.volume, m.area)
    };
    let mut results = Vec::new();
    for (x, y, op) in [
        (&a, &b, Union),
        (&a, &b, Intersect),
        (&a, &b, Subtract),
        (&b, &a, Union),
        (&b, &a, Intersect),
        (&b, &a, Subtract),
    ] {
        match volume_of(&x.body, &y.body, op, k, &what) {
            Ok(result) => results.push(result),
            Err(reason) => {
                if std::env::var_os("BOOLEAN_VERBOSE").is_some() {
                    println!("{what}: {op:?}: {reason}");
                }
                *tally.unsupported.entry(reason).or_default() += 1;
                return;
            }
        }
    }
    tally.solved += 1;
    let [union, common, a_less_b, union_ba, common_ba, b_less_a] = results.as_slice() else {
        panic!("{what}: six results");
    };
    let unit = va.max(vb);
    let close = |x: f64, y: f64, name: &str| {
        assert!(
            (x - y).abs() <= 1e-9 * unit,
            "{what}: {name}: {x} is not {y}"
        );
    };
    close(
        union.0,
        va + vb - common.0,
        "vol(A∪B) = vol A + vol B − vol(A∩B)",
    );
    close(a_less_b.0, va - common.0, "vol(A−B) = vol A − vol(A∩B)");
    close(b_less_a.0, vb - common.0, "vol(B−A) = vol B − vol(A∩B)");
    close(union_ba.0, union.0, "vol(B∪A) = vol(A∪B)");
    close(common_ba.0, common.0, "vol(B∩A) = vol(A∩B)");
    if placement != Placement::Grid {
        // Nothing is coincident, so the boundary of A∪B and of A∩B
        // together are the boundaries of A and B, and so are those of the
        // two differences.
        let unit = aa.max(ab);
        let close = |x: f64, y: f64, name: &str| {
            assert!(
                (x - y).abs() <= 1e-9 * unit,
                "{what}: {name}: {x} is not {y}"
            );
        };
        close(union.1 + common.1, aa + ab, "area(A∪B) + area(A∩B)");
        close(a_less_b.1 + b_less_a.1, aa + ab, "area(A−B) + area(B−A)");
    }

    // Against the operands' own definitions.
    if let (Some((alo, ahi)), Some((blo, bhi))) = (a.aligned, b.aligned) {
        let (lo, hi) = (alo.max(blo), ahi.min(bhi));
        let side = |l: f64, h: f64| (h - l).max(0.0);
        let exact = side(lo.x, hi.x) * side(lo.y, hi.y) * side(lo.z, hi.z);
        close(common.0, exact, "the overlap of two aligned boxes");
    }
    let (estimate, slack) = sampled_overlap(&a, &b, &mut rng);
    assert!(
        (common.0 - estimate).abs() <= slack,
        "{what}: vol(A∩B) = {} but sampling gives {estimate} ± {slack}",
        common.0
    );
    if common.0 > 0.0 {
        tally.overlapping += 1;
    }

    // (A − B) ∪ (A ∩ B) is A again: its pieces meet in coincident faces.
    if let (Some(less), Some(common)) = (&a_less_b.2, &common.2) {
        match volume_of(less, common, Union, k, &what) {
            Ok(whole) => {
                close(whole.0, va, "vol((A−B) ∪ (A∩B)) = vol A");
                tally.recombined += 1;
            }
            Err(reason) => {
                if std::env::var_os("BOOLEAN_VERBOSE").is_some() {
                    println!("{what}: recombining: {reason}");
                }
                *tally.recombine_unsupported.entry(reason).or_default() += 1;
            }
        }
    }
}

fn hunt(pairs_per_cell: u64, first_seed: u64) {
    for k in SCALES {
        for placement in [Placement::General, Placement::Parallel, Placement::Grid] {
            let mut tally = Tally::default();
            for i in 0..pairs_per_cell {
                run_pair(first_seed + i, placement, k, &mut tally);
            }
            let refused: usize = tally.unsupported.values().sum();
            println!(
                "scale {k:e} {placement:?}: {} pairs, {} solved ({} overlapping), {} unsupported; \
                 recombined {} of them, {} refused",
                tally.pairs,
                tally.solved,
                tally.overlapping,
                refused,
                tally.recombined,
                tally.recombine_unsupported.values().sum::<usize>(),
            );
            for (reason, count) in &tally.unsupported {
                println!("    {count:5}  {reason}");
            }
            for (reason, count) in &tally.recombine_unsupported {
                println!("    {count:5}  (recombining) {reason}");
            }
            // The run means something only when most pairs are solved and
            // a fair share of them overlap.
            assert!(
                tally.solved * 2 >= tally.pairs,
                "{placement:?} at {k}: only {} of {} solved",
                tally.solved,
                tally.pairs
            );
            assert!(
                tally.overlapping * 8 >= tally.solved,
                "{placement:?} at {k}"
            );
        }
    }
}

#[test]
fn random_pairs_satisfy_the_identities() {
    hunt(120, 1);
}

/// One pair, to look at: `BOOLEAN_PAIR="seed placement scale"` with the
/// placement as 0 (general), 1 (parallel) or 2 (grid).
#[test]
#[ignore = "a tool for reproducing one pair of a run"]
fn one_pair() {
    let spec = std::env::var("BOOLEAN_PAIR").unwrap_or_default();
    let fields: Vec<f64> = spec
        .split_whitespace()
        .filter_map(|f| f.parse().ok())
        .collect();
    let [seed, placement, k] = fields.as_slice() else {
        panic!("BOOLEAN_PAIR needs a seed, a placement and a scale");
    };
    let placement = [Placement::General, Placement::Parallel, Placement::Grid][*placement as usize];
    let mut rng = Rng(*seed as u64);
    let shared = random_frame(&mut rng, Placement::General, &Frame::WORLD, *k);
    let a = random_shape(&mut rng, placement, &shared, *k);
    let b = random_shape(&mut rng, placement, &shared, *k);
    let tol = tolerance(*k);
    let describe =
        |name: &str, r: &Result<craft3d_kernel::boolean::BooleanResult, KernelError>| match r {
            Ok(r) => println!(
                "{name}: {:?}, {} faces, {} shells",
                mass_properties(&r.body),
                r.body.faces.len(),
                r.body.shells.len()
            ),
            Err(e) => println!("{name}: {e}"),
        };
    println!("A: {:?}", mass_properties(&a.body));
    println!("B: {:?}", mass_properties(&b.body));
    let less = boolean(&a.body, &b.body, Subtract, &tol);
    let common = boolean(&a.body, &b.body, Intersect, &tol);
    describe("A ∪ B", &boolean(&a.body, &b.body, Union, &tol));
    describe("A − B", &less);
    describe("A ∩ B", &common);
    describe("B − A", &boolean(&b.body, &a.body, Subtract, &tol));
    if let (Ok(less), Ok(common)) = (less, common) {
        for (i, f) in less.body.faces.iter().enumerate() {
            println!(
                "  A−B face {i}: {} from {:?}",
                f.surface.kind(),
                less.faces[i]
            );
        }
        for (i, f) in common.body.faces.iter().enumerate() {
            println!(
                "  A∩B face {i}: {} from {:?}",
                f.surface.kind(),
                common.faces[i]
            );
        }
        describe(
            "(A − B) ∪ (A ∩ B)",
            &boolean(&less.body, &common.body, Union, &tol),
        );
    }
}

/// The long run: `cargo test --release -p craft3d-kernel --test
/// boolean_random -- --ignored --nocapture`.
#[test]
#[ignore = "thousands of pairs; run in release"]
fn random_pairs_long_run() {
    hunt(1500, 1_000_000);
}
