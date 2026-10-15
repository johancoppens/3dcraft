//! Booleans on seeded random pairs (docs/testing.md, oracle 3): every case
//! ends as a valid body with the identities holding, as an empty result,
//! or as `Unsupported`. Never a panic, an invalid body or a wrong volume.
//!
//! The identities compare the booleans with each other, so a kernel that
//! missed every intersection would satisfy them. Two independent checks
//! close that gap: the overlap of two axis-aligned boxes is known exactly,
//! and for every other pair the volume of the intersection is estimated by
//! sampling points against the operands' own definitions.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, BooleanOptions, boolean, boolean_with};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_math::{Frame, Vec3};
use craft3d_testkit::parallel::map_parallel;
use craft3d_topo::{Body, check, genus, mass_properties};
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

/// Which shapes a run draws from.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shapes {
    /// Blocks, cylinders, L-shaped extrusions and slots with a hole: what
    /// the first boolean kernel handled, with the seeds of its runs.
    Flat,
    /// Those and cones, balls and rings.
    All,
}

fn random_shape(
    rng: &mut Rng,
    placement: Placement,
    shared: &Frame,
    k: f64,
    shapes: Shapes,
) -> Shape {
    let frame = random_frame(rng, placement, shared, k);
    let tol = tolerance(k);
    let (a, b, c) = (
        size(rng, placement, k),
        size(rng, placement, k),
        size(rng, placement, k),
    );
    let kinds = if shapes == Shapes::All { 12 } else { 6 };
    match rng.below(kinds) {
        6 | 7 => {
            // A cone on the frame's plane.
            let (r, h) = (a * 0.5, c);
            let (lo, hi) = bounds(&frame, [-r, -r, 0.0], [r, r, h]);
            Shape {
                body: primitives::cone(&frame, r, h).unwrap(),
                contains: Box::new(move |p| {
                    let (x, y, z) = frame.local(p);
                    (0.0..=h).contains(&z) && x.hypot(y) <= r * (1.0 - z / h)
                }),
                lo,
                hi,
                aligned: None,
            }
        }
        8 | 9 => {
            let r = a * 0.5;
            let (lo, hi) = bounds(&frame, [-r, -r, -r], [r, r, r]);
            Shape {
                body: primitives::sphere(&frame, r).unwrap(),
                contains: Box::new(move |p| p.distance(frame.origin()) <= r),
                lo,
                hi,
                aligned: None,
            }
        }
        10 | 11 => {
            let (major, minor) = (0.5 * (a + b), 0.25 * a);
            let reach = major + minor;
            let (lo, hi) = bounds(&frame, [-reach, -reach, -minor], [reach, reach, minor]);
            Shape {
                body: primitives::torus(&frame, major, minor).unwrap(),
                contains: Box::new(move |p| {
                    let (x, y, z) = frame.local(p);
                    (x.hypot(y) - major).hypot(z) <= minor
                }),
                lo,
                hi,
                aligned: None,
            }
        }
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

/// The volume common to both shapes, by sampling: the estimate and five
/// standard deviations of it (three would fail a correct pair now and then
/// over the thousands of pairs of a long run).
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
    // Five standard deviations, by the interval of Agresti and Coull: the
    // deviation is taken at the fraction with z²/2 hits and z² samples
    // added (z = 5), not at the fraction counted. Counted, it is zero
    // when no sample hits, and an overlap that 6 of 3000 samples should
    // have hit goes unhit once in four hundred pairs: seed 2000331 of the
    // long run with every shape (two rings sharing 0.325 of a cell of
    // 164; a hundred times the samples give 0.314 ± 0.013).
    const Z: f64 = 5.0;
    let samples = SAMPLES as f64 + Z * Z;
    let middle = (inside as f64 + 0.5 * Z * Z) / samples;
    let deviation = (middle * (1.0 - middle) / samples).sqrt();
    (
        cell * fraction,
        cell * (Z * deviation + (middle - fraction).abs()),
    )
}

/// How far the volume and the area of a body may be off because its
/// spline edges are only within the tolerance of the curves they stand
/// for: the tolerance times the area of the faces they bound (here: of the
/// whole body) for the volume, and times the length of those edges for
/// the area. Nothing for a body without spline edges.
fn spline_slack(body: &Body, k: f64) -> (f64, f64) {
    let tol = 1e-6 * k;
    let mut length = 0.0;
    for edge in &body.edges {
        if matches!(edge.curve, craft3d_geom::Curve3::Nurbs(_)) {
            let (t0, t1) = edge.range;
            let mut previous = edge.curve.eval(t0);
            for i in 1..=64 {
                let p = edge.curve.eval(t0 + (t1 - t0) * f64::from(i) / 64.0);
                length += p.distance(previous);
                previous = p;
            }
        }
    }
    if length == 0.0 {
        return (0.0, 0.0);
    }
    let area = mass_properties(body).map_or(0.0, |m| m.area);
    (4.0 * tol * area, 4.0 * tol * length)
}

/// A hash of a result, for the digest of a run: see the corpus test. The
/// digest is the wrapping sum of the hashes of a run's results, kept in
/// that run's [`Tally`] (not in a static, which every test of the binary
/// would add to in whatever order the test threads run: 2026-10-16), so
/// it is the same whatever runs beside it and on any number of threads.
fn digest(result: &craft3d_kernel::boolean::BooleanResult) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{result:?}").hash(&mut hasher);
    hasher.finish()
}

/// The volume of a boolean; 0 when it is empty; `Err` with the reason when
/// it is not supported. Anything else fails the test.
fn volume_of(
    a: &Body,
    b: &Body,
    op: BooleanOp,
    k: f64,
    what: &str,
    digests: &mut u64,
) -> Result<(f64, f64, Option<Body>), &'static str> {
    let tol = tolerance(k);
    match boolean(a, b, op, &tol) {
        Ok(result) => {
            *digests = digests.wrapping_add(digest(&result));
            assert_eq!(check(&result.body, &tol), [], "{what}: {op:?}");
            assert_eq!(result.faces.len(), result.body.faces.len(), "{what}");
            assert_eq!(result.edges.len(), result.body.edges.len(), "{what}");
            assert_eq!(result.vertices.len(), result.body.vertices.len(), "{what}");
            let m = match mass_properties(&result.body) {
                Ok(m) => m,
                Err(e) => panic!("{what}: {op:?}: mass properties: {e}"),
            };
            assert!(m.volume > 0.0, "{what}: {op:?}: volume {}", m.volume);
            // Merging changes how the solid is divided into faces, not
            // the solid: without it the same volume, area and genus, and
            // never fewer faces, edges or vertices.
            let plain = BooleanOptions {
                merge: false,
                ..BooleanOptions::default()
            };
            match boolean_with(a, b, op, &tol, &plain) {
                Ok(unmerged) => {
                    assert_eq!(
                        check(&unmerged.body, &tol),
                        [],
                        "{what}: {op:?}, not merged"
                    );
                    let u = match mass_properties(&unmerged.body) {
                        Ok(u) => u,
                        Err(e) => panic!("{what}: {op:?}, not merged: mass properties: {e}"),
                    };
                    // `mass_properties` integrates a piece of a cone or a
                    // sphere numerically, to about 1e-9 of the body: a
                    // cone of radius 1 and height 2 (seed 33 on the grid,
                    // every shape, scale 0.001, which a ring touches
                    // along a circle) has π r² h / 3 to 5e-16 as one
                    // face and to 1.003e-9 as the two faces that circle
                    // divides it into before merging, although the
                    // circle lies on the cone to rounding. So 1e-8 where
                    // the unmerged body has such a face; 1e-9 everywhere
                    // else.
                    let integrated = unmerged.body.faces.iter().any(|f| {
                        matches!(
                            f.surface,
                            craft3d_geom::Surface::Sphere(_) | craft3d_geom::Surface::Cone(_)
                        )
                    });
                    let exact = if integrated { 1e-8 } else { 1e-9 };
                    assert!(
                        // (The splines may be among what merging removes.)
                        (u.volume - m.volume).abs()
                            <= exact * m.volume + spline_slack(&unmerged.body, k).0
                            && (u.area - m.area).abs()
                                <= exact * m.area + spline_slack(&unmerged.body, k).1,
                        "{what}: {op:?}: merging changed volume {} to {} or area {} to {}",
                        u.volume,
                        m.volume,
                        u.area,
                        m.area
                    );
                    assert_eq!(genus(&unmerged.body), genus(&result.body), "{what}: {op:?}");
                    assert_eq!(unmerged.body.shells.len(), result.body.shells.len());
                    assert!(unmerged.body.faces.len() >= result.body.faces.len());
                    assert!(unmerged.body.edges.len() >= result.body.edges.len());
                    assert!(unmerged.body.vertices.len() >= result.body.vertices.len());
                }
                Err(e) => panic!("{what}: {op:?} works merged but not unmerged: {e}"),
            }
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
    /// The wrapping sum of the hashes of every result ([`digest`]).
    digest: u64,
}

impl Tally {
    /// Adds the counts of another run to this one.
    fn absorb(&mut self, other: Tally) {
        self.pairs += other.pairs;
        self.solved += other.solved;
        self.overlapping += other.overlapping;
        for (reason, count) in other.unsupported {
            *self.unsupported.entry(reason).or_default() += count;
        }
        self.recombined += other.recombined;
        self.digest = self.digest.wrapping_add(other.digest);
        for (reason, count) in other.recombine_unsupported {
            *self.recombine_unsupported.entry(reason).or_default() += count;
        }
    }
}

fn run_pair(seed: u64, placement: Placement, k: f64, shapes: Shapes, tally: &mut Tally) {
    let mut rng = Rng(seed);
    let shared = random_frame(&mut rng, Placement::General, &Frame::WORLD, k);
    let a = random_shape(&mut rng, placement, &shared, k, shapes);
    let b = random_shape(&mut rng, placement, &shared, k, shapes);
    let what = format!("seed {seed} {placement:?} {shapes:?} scale {k}");
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
        match volume_of(&x.body, &y.body, op, k, &what, &mut tally.digest) {
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
    // Where an edge is a spline (the surfaces meet in a curve without a
    // closed form), it is within the tolerance of the true curve, and the
    // identities hold to that: see `spline_slack`.
    let slack = results
        .iter()
        .filter_map(|r| r.2.as_ref())
        .map(|body| spline_slack(body, k))
        .fold((0.0_f64, 0.0_f64), |a, b| (a.0.max(b.0), a.1.max(b.1)));
    let close = |x: f64, y: f64, name: &str| {
        assert!(
            (x - y).abs() <= 1e-9 * unit + slack.0,
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
                (x - y).abs() <= 1e-9 * unit + slack.1,
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
        match volume_of(less, common, Union, k, &what, &mut tally.digest) {
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

fn hunt(pairs_per_cell: u64, first_seed: u64, shapes: Shapes) {
    // Every pair has its own seed, so the cases are the same whatever the
    // thread count; all cells run side by side in one pool, so no cell's
    // tail leaves cores idle, and the counts are added up per cell.
    let mut cells = Vec::new();
    for k in SCALES {
        for placement in [Placement::General, Placement::Parallel, Placement::Grid] {
            cells.push((k, placement));
        }
    }
    let mut items = Vec::new();
    for c in 0..cells.len() {
        for i in 0..pairs_per_cell {
            items.push((c, first_seed + i));
        }
    }
    let parts = map_parallel(items, |(c, seed)| {
        let (k, placement) = cells[c];
        let mut tally = Tally::default();
        run_pair(seed, placement, k, shapes, &mut tally);
        (c, tally)
    });
    let mut tallies: Vec<Tally> = cells.iter().map(|_| Tally::default()).collect();
    for (c, part) in parts {
        tallies[c].absorb(part);
    }
    for (&(k, placement), tally) in cells.iter().zip(&tallies) {
        let refused: usize = tally.unsupported.values().sum();
        println!(
            "{shapes:?} scale {k:e} {placement:?}: {} pairs, {} solved ({} overlapping), {} unsupported; \
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
        // (With every shape most general pairs wait for the general
        // surface intersection; a fifth is asked of them until then.)
        let least = if shapes == Shapes::All { 5 } else { 2 };
        assert!(
            tally.solved * least >= tally.pairs,
            "{placement:?} at {k}: only {} of {} solved",
            tally.solved,
            tally.pairs
        );
        assert!(
            tally.overlapping * 8 >= tally.solved,
            "{placement:?} at {k}"
        );
    }
    let digest = tallies
        .iter()
        .fold(0_u64, |sum, tally| sum.wrapping_add(tally.digest));
    println!("digest of all results: {digest:016x}");
}

#[test]
fn random_pairs_satisfy_the_identities() {
    hunt(120, 1, Shapes::Flat);
}

#[test]
fn random_pairs_with_cones_balls_and_rings_satisfy_the_identities() {
    hunt(120, 1, Shapes::All);
}

/// One pair, to look at: `BOOLEAN_PAIR="seed placement scale"` with the
/// placement as 0 (general), 1 (parallel) or 2 (grid); with
/// `BOOLEAN_ALL_SHAPES` set, a pair of the runs with every shape.
#[test]
#[ignore = "a tool for reproducing one pair of a run"]
fn one_pair() {
    // Not asked for (the long runs are started with `--ignored`, which
    // starts this too): nothing to look at, and nothing wrong.
    let Ok(spec) = std::env::var("BOOLEAN_PAIR") else {
        println!("one_pair: BOOLEAN_PAIR is not set, nothing to do");
        return;
    };
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
    let shapes = if std::env::var_os("BOOLEAN_ALL_SHAPES").is_some() {
        Shapes::All
    } else {
        Shapes::Flat
    };
    let a = random_shape(&mut rng, placement, &shared, *k, shapes);
    let b = random_shape(&mut rng, placement, &shared, *k, shapes);
    let tol = tolerance(*k);
    let describe =
        |name: &str, r: &Result<craft3d_kernel::boolean::BooleanResult, KernelError>| match r {
            Ok(r) => println!(
                "{name}: {:?}, {} faces, {} shells, {} edges, {} vertices, {} loops, genus {:?}",
                mass_properties(&r.body),
                r.body.faces.len(),
                r.body.shells.len(),
                r.body.edges.len(),
                r.body.vertices.len(),
                r.body.faces.iter().map(|f| f.loops.len()).sum::<usize>(),
                genus(&r.body)
            ),
            Err(e) => println!("{name}: {e}"),
        };
    println!(
        "A: {} faces, box {:?} .. {:?}",
        a.body.faces.len(),
        a.lo,
        a.hi
    );
    println!(
        "B: {} faces, box {:?} .. {:?}",
        b.body.faces.len(),
        b.lo,
        b.hi
    );
    if std::env::var_os("BOOLEAN_VERBOSE").is_some() {
        for (name, body) in [("A", &a.body), ("B", &b.body)] {
            for (i, face) in body.faces.iter().enumerate() {
                println!("{name} face {i}: {:?}", face.surface);
            }
        }
    }
    println!("A: {:?}", mass_properties(&a.body));
    println!("B: {:?}", mass_properties(&b.body));
    // A closer look than the run takes: a hundred times the samples.
    let mut total = 0.0;
    for _ in 0..100 {
        total += sampled_overlap(&a, &b, &mut rng).0;
    }
    println!("A ∩ B by sampling: {}", total / 100.0);
    let less = boolean(&a.body, &b.body, Subtract, &tol);
    let common = boolean(&a.body, &b.body, Intersect, &tol);
    let plain = craft3d_kernel::boolean::BooleanOptions {
        merge: false,
        ..BooleanOptions::default()
    };
    describe(
        "A ∪ B, not merged",
        &craft3d_kernel::boolean::boolean_with(&a.body, &b.body, Union, &tol, &plain),
    );
    describe(
        "A ∩ B, not merged",
        &craft3d_kernel::boolean::boolean_with(&a.body, &b.body, Intersect, &tol, &plain),
    );
    describe(
        "B ∪ A, not merged",
        &craft3d_kernel::boolean::boolean_with(&b.body, &a.body, Union, &tol, &plain),
    );
    describe("B ∪ A", &boolean(&b.body, &a.body, Union, &tol));
    describe("A ∪ B", &boolean(&a.body, &b.body, Union, &tol));
    // `BOOLEAN_FACE=n`: the boundary of face n of the union, edge by edge.
    if let (Some(face), Ok(union)) = (
        std::env::var("BOOLEAN_FACE")
            .ok()
            .and_then(|f| f.parse::<usize>().ok()),
        boolean(&a.body, &b.body, Union, &tol),
    ) && let Some(f) = union.body.faces.get(face)
    {
        println!("face {face} of A ∪ B: {:?}", f.surface);
        for &id in &f.loops {
            println!("  loop");
            for coedge in &union.body.loop_(id).unwrap().coedges {
                let edge = union.body.edge(coedge.edge).unwrap();
                let (from, to) = (
                    union.body.vertex(edge.start).unwrap().point,
                    union.body.vertex(edge.end).unwrap().point,
                );
                let off = |p: Vec3| {
                    let (u, v) = f.surface.project(p).unwrap();
                    f.surface.eval(u, v).distance(p)
                };
                println!(
                    "    edge {} {} {} {:?} from {:?} ({:?}) to {:?} ({:?}), ends off the curve by {:e} and {:e}, off the surface by {:e} and {:e}; {:?}",
                    coedge.edge.0,
                    if coedge.reversed {
                        "reversed"
                    } else {
                        "forward"
                    },
                    edge.curve.kind(),
                    edge.range,
                    edge.start,
                    from,
                    edge.end,
                    to,
                    edge.curve.eval(edge.range.0).distance(from),
                    edge.curve.eval(edge.range.1).distance(to),
                    off(from),
                    off(to),
                    union.edges[coedge.edge.0 as usize]
                );
            }
        }
    }
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
    hunt(1500, 1_000_000, Shapes::Flat);
}

/// The long run with every shape.
#[test]
#[ignore = "thousands of pairs; run in release"]
fn random_pairs_with_every_shape_long_run() {
    hunt(1500, 2_000_000, Shapes::All);
}

/// Pairs of the runs that once went wrong, each with what it was.
///
/// - Seed 32 on the grid: two slots with holes whose round ends cross at
///   right angles. A rim met a wall exactly at the seam of its circle and
///   the crossing was lost: an invalid body.
/// - Seeds 1000568, 1000735, 1000810, 1001320 and 1001488 on the grid,
///   and 87 with every shape: a cylinder that lies against the plane of
///   another cylinder's rim, so that the curve in which the two walls meet
///   touches that rim at its highest point. The curve is a spline within
///   the tolerance of the true one, and at its very end its direction was
///   off to the wrong side of the rim by a few millionths of a radian:
///   "an imprinted face could not be divided into regions".
/// - Seed 2000516 with every shape: a cone whose apex lies on a ball. The
///   curve in which they meet is a loop that begins and ends at the apex;
///   its two ends were taken for one cut, no stretch was left, and the
///   bodies were said to have nothing in common (the sampled overlap
///   caught it).
/// - Seed 2000613 on the grid with every shape: a hole along the tangent
///   wall of a slot, lying on the slot's floor. Its curve on the slot's
///   round end touches the floor's rim where the rim meets that wall,
///   with contact of the fourth order: the place found for it on the rim
///   is 1.4e-4 from the true one, and for a twentieth of the rim the
///   curve and the rim are within the tolerance of each other. Their
///   order about that point was read where they are a hundredth of a
///   tolerance apart, came out by the curve's approximation, and the
///   face "could not be divided into regions; this is a bug". It is read
///   now where the two are more than four tolerances apart.
/// - Seed 2001169 on the grid with every shape: a cylinder (the round end
///   of a slot) inside the tube of a torus, its axis square to the
///   torus', touching the top of the tube: contact of the fourth order,
///   a tacnode of the intersection. The march passed through it and
///   stalled: refused until 2026-10-15.
#[test]
fn pairs_that_once_went_wrong() {
    for k in SCALES {
        for (seed, shapes) in [
            (32, Shapes::Flat),
            (1_000_568, Shapes::Flat),
            (1_000_735, Shapes::Flat),
            (1_000_810, Shapes::Flat),
            (1_001_320, Shapes::Flat),
            (1_001_488, Shapes::Flat),
            (87, Shapes::All),
            (2_000_516, Shapes::All),
            (2_000_613, Shapes::All),
            (2_001_169, Shapes::All),
        ] {
            let mut tally = Tally::default();
            run_pair(seed, Placement::Grid, k, shapes, &mut tally);
            assert_eq!(
                (tally.solved, tally.unsupported.len()),
                (1, 0),
                "seed {seed} at scale {k}: {:?}",
                tally.unsupported
            );
            assert_eq!(tally.overlapping, 1, "seed {seed} at scale {k}");
        }
    }
}
