//! The Bézier spans of spline surfaces and the pieces of spline curves, as
//! digests of their bits (written by the review of 2026-11-17,
//! review-grid-scale), for surfaces of
//! every kind the extraction meets: polynomial and rational, degrees 1 to
//! 32, single spans, clamped and unclamped (periodic) knots, interior
//! knots of every multiplicity up to the degree and past it, nets closed
//! on themselves, rows collapsed to a pole, zeros of both signs,
//! subnormal numbers, and magnitudes up to the largest finite number
//! (where a homogeneous point or a blend of two overflows). The digest of
//! them all is held to what it was when spans were made one knot at a time
//! (main before 2026-11-17, and the branch that made them in one pass:
//! the review's runs on both); a change is told to the bit. To compare two
//! commits case by case, print the lines and `diff` them:
//!
//! `cargo test --release -p craft3d-geom --test spans_digest -- --nocapture`

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{NurbsCurve, NurbsSurface};
use craft3d_math::Vec3;
use std::collections::BTreeMap;

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn f(&mut self, x: f64) {
        self.bytes(&x.to_bits().to_le_bytes());
    }
    fn u(&mut self, x: u64) {
        self.bytes(&x.to_le_bytes());
    }
    fn v(&mut self, p: Vec3) {
        self.f(p.x);
        self.f(p.y);
        self.f(p.z);
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        self.next() as f64 / (1u64 << 53) as f64
    }
}

/// A knot vector for `count` points of `degree`, of one of several kinds.
fn knots(rng: &mut Rng, degree: usize, count: usize, kind: usize) -> Vec<f64> {
    let n = count + degree + 1;
    match kind {
        // Clamped, interior knots spread unevenly, each of a random
        // multiplicity (up to the degree, sometimes one more).
        0 | 1 => {
            let mut inner = Vec::new();
            let wanted = count - degree - 1;
            while inner.len() < wanted {
                let value = rng.unit();
                let most = if kind == 1 { degree + 1 } else { degree };
                let times = 1 + rng.below(most);
                for _ in 0..times.min(wanted - inner.len()) {
                    inner.push(value);
                }
            }
            inner.sort_by(f64::total_cmp);
            let mut k = vec![0.0; degree + 1];
            k.extend(inner);
            k.extend(std::iter::repeat_n(1.0, degree + 1));
            k
        }
        // Uniform, unclamped (a periodic B-spline's knots).
        2 => (0..n).map(|i| i as f64).collect(),
        // Unclamped and uneven, with runs.
        3 => {
            let mut k: Vec<f64> = (0..n).map(|_| (rng.below(7) as f64) * 0.25 - 0.5).collect();
            k.sort_by(f64::total_cmp);
            // An empty domain is refused: then spread the knots.
            if k[degree] >= k[count] {
                for (i, x) in k.iter_mut().enumerate() {
                    *x += i as f64 * 1e-3;
                }
            }
            k
        }
        // Clamped with the ends' multiplicity past degree + 1 on one side
        // (a run at the start longer than the order) and -0 as the start.
        4 if count >= degree + 2 => {
            let mut k = vec![-0.0; degree + 2];
            let inner = n - 2 * (degree + 1) - 1;
            for i in 0..inner {
                k.push((i + 1) as f64 / (inner + 1) as f64);
            }
            k.extend(std::iter::repeat_n(1.0, degree + 1));
            k
        }
        4 => knots(rng, degree, count, 0),
        // Clamped, every interior knot of multiplicity exactly the degree
        // (already Bézier), scaled to a large parameter range.
        _ => {
            let pieces = (count - 1) / degree;
            let mut k = vec![0.0; degree + 1];
            for i in 1..pieces {
                for _ in 0..degree {
                    k.push(i as f64 * 1e3);
                }
            }
            while k.len() < n - degree - 1 {
                let last = *k.last().unwrap();
                k.push(last);
            }
            let end = pieces.max(1) as f64 * 1e3;
            k.extend(std::iter::repeat_n(end, degree + 1));
            k.truncate(n);
            k
        }
    }
}

/// A coordinate of one of several kinds.
fn value(rng: &mut Rng, kind: usize) -> f64 {
    match kind {
        0 => rng.unit() * 20.0 - 10.0,
        1 => [0.0, -0.0, 1.0, -1.0, 0.5, -0.25][rng.below(6)],
        2 => [5e-324, -5e-324, 1e-310, -1e-310, 0.0, -0.0, 1e-300][rng.below(7)],
        3 => (rng.unit() - 0.5) * 2e15,
        4 => [f64::MAX, -f64::MAX, f64::MAX * 0.75, 0.0][rng.below(4)],
        _ => (rng.unit() - 0.5) * 2e300,
    }
}

#[derive(Default)]
struct Tally {
    cases: usize,
    refused: usize,
    spans: usize,
    rational: usize,
    with_zero_signs: usize,
    pieces: usize,
}

#[test]
fn digests_of_the_spans_of_surfaces_of_every_kind() {
    let mut rng = Rng(0x7265_7669_6577);
    let mut tally: BTreeMap<String, Tally> = BTreeMap::new();
    let mut all = Fnv::new();
    for case in 0..6000 {
        let degree_u = if case % 97 == 0 {
            20 + rng.below(13)
        } else {
            1 + rng.below(6)
        };
        let degree_v = if case % 89 == 0 {
            20 + rng.below(13)
        } else {
            1 + rng.below(5)
        };
        let big = case % 50 == 0 && degree_u <= 6 && degree_v <= 5;
        let extra = |rng: &mut Rng| {
            if big {
                40 + rng.below(160)
            } else if rng.below(5) == 0 {
                0
            } else {
                rng.below(12)
            }
        };
        let count_u = degree_u + 1 + extra(&mut rng);
        let count_v = degree_v + 1 + extra(&mut rng);
        let knot_kind = rng.below(6);
        let ku = knots(&mut rng, degree_u, count_u, knot_kind);
        let kind_v = rng.below(6);
        let kv = knots(&mut rng, degree_v, count_v, kind_v);
        let value_kind = match case % 10 {
            0..=4 => 0,
            5 | 6 => 1,
            7 => 2,
            8 => 3,
            _ => 4 + rng.below(2),
        };
        let rational = case % 3 != 0;
        let shape = rng.below(4); // 0 free, 1 closed in u, 2 pole row, 3 periodic wrap
        let mut ctrl = Vec::with_capacity(count_u * count_v);
        let mut weights = Vec::with_capacity(count_u * count_v);
        for i in 0..count_u {
            for j in 0..count_v {
                let p = match shape {
                    1 if i + 1 == count_u => ctrl[j],
                    2 if i == 0 => Vec3::new(0.0, -0.0, 1.0),
                    3 if i + degree_u >= count_u => ctrl[(i + degree_u - count_u) * count_v + j],
                    _ => Vec3::new(
                        value(&mut rng, value_kind),
                        value(&mut rng, value_kind),
                        value(&mut rng, value_kind),
                    ),
                };
                ctrl.push(p);
                weights.push(if rational {
                    match rng.below(4) {
                        0 => 1.0,
                        // 10 to the power -3 to 3 (as `powi` gives them: 1 / 10ⁿ below one).
                        1 => [1e-3, 1e-2, 1e-1, 1.0, 1e1, 1e2, 1e3][rng.below(7)],
                        _ => 0.25 + rng.unit() * 3.0,
                    }
                } else {
                    1.0
                });
            }
        }
        let zero_signs = ctrl.iter().any(|p| {
            [p.x, p.y, p.z]
                .iter()
                .any(|x| *x == 0.0 && x.is_sign_negative())
        });
        let class = format!(
            "knots{knot_kind}-values{value_kind}-shape{shape}-{}",
            if rational { "rational" } else { "polynomial" }
        );
        let t = tally.entry(class.clone()).or_default();
        t.cases += 1;
        let mut h = Fnv::new();
        let surface = NurbsSurface::new(
            degree_u,
            degree_v,
            ku.clone(),
            kv.clone(),
            count_u,
            count_v,
            ctrl.clone(),
            weights.clone(),
        );
        match surface {
            Err(e) => {
                t.refused += 1;
                h.bytes(format!("{e}").as_bytes());
            }
            Ok(surface) => {
                if rational {
                    t.rational += 1;
                }
                if zero_signs {
                    t.with_zero_signs += 1;
                }
                match surface.bezier_spans() {
                    Err(e) => h.bytes(format!("spans {e}").as_bytes()),
                    Ok(spans) => {
                        t.spans += spans.len();
                        h.u(spans.len() as u64);
                        for span in &spans {
                            h.f(span.u.0);
                            h.f(span.u.1);
                            h.f(span.v.0);
                            h.f(span.v.1);
                            let (a, b) = span.patch.degrees();
                            h.u(a as u64);
                            h.u(b as u64);
                            for p in span.patch.control_points() {
                                h.v(p);
                            }
                            for w in span.patch.weights() {
                                h.f(w);
                            }
                        }
                    }
                }
            }
        }
        // The first column as a curve: its pieces, one piece at a time,
        // and its derivative bounds.
        let column: Vec<Vec3> = (0..count_u).map(|i| ctrl[i * count_v]).collect();
        let column_w: Vec<f64> = (0..count_u).map(|i| weights[i * count_v]).collect();
        match NurbsCurve::new(degree_u, ku.clone(), column, column_w) {
            Err(e) => h.bytes(format!("curve {e}").as_bytes()),
            Ok(curve) => {
                let pieces = curve.bezier_pieces();
                t.pieces += pieces.len();
                h.u(pieces.len() as u64);
                for ((lo, hi), points) in &pieces {
                    h.f(*lo);
                    h.f(*hi);
                    for (p, w) in points {
                        h.v(*p);
                        h.f(*w);
                    }
                }
                for span in 0..ku.len() {
                    if let Some(((lo, hi), points)) = curve.bezier_piece(span) {
                        h.u(span as u64);
                        h.f(lo);
                        h.f(hi);
                        for (p, w) in points {
                            h.v(p);
                            h.f(w);
                        }
                    }
                }
                match curve.derivative_bounds() {
                    Some((a, b)) => {
                        h.f(a);
                        h.f(b);
                    }
                    None => h.u(u64::MAX),
                }
            }
        }
        println!(
            "S {case} {class} {degree_u}x{degree_v} {count_u}x{count_v} {:016x}",
            h.0
        );
        all.u(h.0);
    }
    for (class, t) in &tally {
        println!(
            "T {class}: {} cases, {} refused, {} rational, {} with -0, {} spans, {} curve pieces",
            t.cases, t.refused, t.rational, t.with_zero_signs, t.spans, t.pieces
        );
    }
    println!("ALL {:016x}", all.0);
    assert_eq!(format!("{:016x}", all.0), "f367892a0bb14266");
}
