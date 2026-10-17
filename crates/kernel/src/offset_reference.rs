//! The offset's cut step as it was before 2026-10-17: a sweep in x over
//! every pair of curves of the raw offset (its keep step, which tested
//! every curve of the input and every mitre wedge, is
//! [`Judge`] without trees). Kept as the oracle the indexed search
//! ([`super::BoxTree`]) must agree with, to the bit: the same pieces in
//! the same order, the same names, the same errors and the same largest
//! distances.

use super::*;

/// Step 3 by a sweep over the boxes' left sides.
pub(super) fn cut(raw: &mut [Candidate], boxes: &[Box2], adjacent: &[(usize, usize)], tol: f64) {
    let mut order: Vec<usize> = (0..raw.len()).collect();
    let left = |i: usize| boxes.get(i).map_or(f64::NAN, |b| b[0]);
    order.sort_by(|&i, &j| left(i).total_cmp(&left(j)).then(i.cmp(&j)));
    let mut cuts: Vec<(usize, f64, P2)> = Vec::new();
    for (position, &i) in order.iter().enumerate() {
        let (Some(bi), Some(ci)) = (boxes.get(i), raw.get(i)) else {
            continue;
        };
        for &j in order.iter().skip(position + 1) {
            let (Some(bj), Some(cj)) = (boxes.get(j), raw.get(j)) else {
                continue;
            };
            if bj[0] > bi[2] + tol {
                break;
            }
            if !boxes_overlap(*bi, *bj, tol) {
                continue;
            }
            let pair = if i < j { (i, j) } else { (j, i) };
            if adjacent.binary_search(&pair).is_ok() {
                continue;
            }
            for p in meet(&ci.geo, &cj.geo) {
                if !p.iter().all(|x| x.is_finite()) {
                    continue;
                }
                if let (Some(ti), Some(tj)) = (ci.geo.param(p, tol), cj.geo.param(p, tol)) {
                    cuts.push((i, ti, p));
                    cuts.push((j, tj, p));
                }
            }
        }
    }
    for (i, t, p) in cuts {
        if let Some(c) = raw.get_mut(i) {
            c.cuts.push((t, p));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{SketchCurve, detect_profiles};
    use std::collections::BTreeMap;

    /// SplitMix64.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }

        fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 / (1u64 << 53) as f64
        }

        /// A multiple of a half in `0..n`.
        fn grid(&mut self, n: u64) -> f64 {
            self.below(2 * n) as f64 / 2.0
        }
    }

    fn line(a: P2, b: P2) -> Curve2 {
        Curve2::Line { start: a, end: b }
    }

    fn polygon(points: &[P2]) -> Vec<Curve2> {
        (0..points.len())
            .map(|i| line(points[i], points[(i + 1) % points.len()]))
            .collect()
    }

    fn rectangle(x: (f64, f64), y: (f64, f64)) -> Vec<Curve2> {
        polygon(&[[x.0, y.0], [x.1, y.0], [x.1, y.1], [x.0, y.1]])
    }

    fn circle(c: P2, r: f64) -> Curve2 {
        Curve2::Circle {
            center: c,
            radius: r,
        }
    }

    /// A slot about `c`, `l` between the centres of its ends, radius `r`:
    /// lines and half circles that meet tangentially.
    fn slot(c: P2, l: f64, r: f64) -> Vec<Curve2> {
        let (a, b) = ([c[0] - l / 2.0, c[1]], [c[0] + l / 2.0, c[1]]);
        vec![
            line([a[0], c[1] - r], [b[0], c[1] - r]),
            Curve2::Arc {
                center: b,
                start: [b[0], c[1] - r],
                end: [b[0], c[1] + r],
            },
            line([b[0], c[1] + r], [a[0], c[1] + r]),
            Curve2::Arc {
                center: a,
                start: [a[0], c[1] + r],
                end: [a[0], c[1] - r],
            },
        ]
    }

    /// A rectangle with its corners rounded by `r` (tangent arcs).
    fn rounded(x: (f64, f64), y: (f64, f64), r: f64) -> Vec<Curve2> {
        let arc = |c: P2, a: P2, b: P2| Curve2::Arc {
            center: c,
            start: a,
            end: b,
        };
        vec![
            line([x.0 + r, y.0], [x.1 - r, y.0]),
            arc([x.1 - r, y.0 + r], [x.1 - r, y.0], [x.1, y.0 + r]),
            line([x.1, y.0 + r], [x.1, y.1 - r]),
            arc([x.1 - r, y.1 - r], [x.1, y.1 - r], [x.1 - r, y.1]),
            line([x.1 - r, y.1], [x.0 + r, y.1]),
            arc([x.0 + r, y.1 - r], [x.0 + r, y.1], [x.0, y.1 - r]),
            line([x.0, y.1 - r], [x.0, y.0 + r]),
            arc([x.0 + r, y.0 + r], [x.0, y.0 + r], [x.0 + r, y.0]),
        ]
    }

    /// A star of `n` points about `c` between the radii `r0` and `r1`.
    fn star(c: P2, n: usize, r0: f64, r1: f64, turn: f64) -> Vec<Curve2> {
        let points: Vec<P2> = (0..2 * n)
            .map(|k| {
                let a = turn + PI * k as f64 / n as f64;
                let r = if k.is_multiple_of(2) { r1 } else { r0 };
                [c[0] + r * a.cos(), c[1] + r * a.sin()]
            })
            .collect();
        polygon(&points)
    }

    /// A strip whose top is a saw of `n` teeth.
    fn saw(at: P2, n: usize, p: f64, height: f64, depth: f64) -> Vec<Curve2> {
        let l = n as f64 * p;
        let mut points = vec![at, [at[0] + l, at[1]], [at[0] + l, at[1] + height]];
        for k in (0..n).rev() {
            points.push([at[0] + (k as f64 + 0.5) * p, at[1] + height + depth]);
            points.push([at[0] + k as f64 * p, at[1] + height]);
        }
        polygon(&points)
    }

    /// A C: a square with a slot cut into it from the right.
    fn c_shape(at: P2, s: f64, arm: f64, back: f64) -> Vec<Curve2> {
        let [x, y] = at;
        polygon(&[
            [x, y],
            [x + s, y],
            [x + s, y + arm],
            [x + back, y + arm],
            [x + back, y + s - arm],
            [x + s, y + s - arm],
            [x + s, y + s],
            [x, y + s],
        ])
    }

    fn scaled(c: &Curve2, k: f64) -> Curve2 {
        let s = |p: P2| [p[0] * k, p[1] * k];
        match c {
            &Curve2::Line { start, end } => line(s(start), s(end)),
            &Curve2::Arc { center, start, end } => Curve2::Arc {
                center: s(center),
                start: s(start),
                end: s(end),
            },
            &Curve2::Circle { center, radius } => circle(s(center), radius * k),
            other => other.clone(),
        }
    }

    /// One random sketch: a few shapes of every kind on a small grid, so
    /// that they nest, lie side by side and, offset, cut each other,
    /// merge, split and vanish.
    fn sketch(rng: &mut Rng) -> Vec<Curve2> {
        let mut curves = Vec::new();
        for _ in 0..1 + rng.below(5) {
            let c = [rng.grid(20), rng.grid(20)];
            match rng.below(9) {
                0 => {
                    let n = 3 + rng.below(4) as usize;
                    let turn = rng.unit();
                    curves.extend(star(c, n, 0.5 + rng.grid(2), 3.0 + rng.grid(5), turn));
                }
                1 | 2 => {
                    let (w, h) = (1.0 + rng.grid(8), 1.0 + rng.grid(8));
                    curves.extend(rectangle((c[0], c[0] + w), (c[1], c[1] + h)));
                }
                3 => curves.push(circle(c, 0.5 + rng.grid(6))),
                4 => curves.extend(slot(c, rng.grid(6) + 1.0, 0.5 + rng.grid(3))),
                5 => {
                    let (w, h) = (2.0 + rng.grid(8), 2.0 + rng.grid(8));
                    let r = 0.25 + rng.unit() * (w.min(h) / 2.0 - 0.3);
                    curves.extend(rounded((c[0], c[0] + w), (c[1], c[1] + h), r));
                }
                6 => {
                    let n = 2 + rng.below(8) as usize;
                    curves.extend(saw(
                        c,
                        n,
                        0.5 + rng.grid(2),
                        1.0 + rng.grid(3),
                        0.5 + rng.grid(2),
                    ));
                }
                7 => curves.extend(c_shape(
                    c,
                    4.0 + rng.grid(4),
                    1.0 + rng.grid(1),
                    1.0 + rng.grid(2),
                )),
                _ => {
                    // A ring of holes in a disc: nested loops.
                    let r = 3.0 + rng.grid(4);
                    curves.push(circle(c, r));
                    let n = 1 + rng.below(6);
                    for k in 0..n {
                        let a = TAU * k as f64 / n as f64;
                        let at = [c[0] + 0.55 * r * a.cos(), c[1] + 0.55 * r * a.sin()];
                        curves.push(circle(at, 0.2 * r));
                    }
                }
            }
        }
        curves
    }

    /// The indexed search gives what the reference gives, to the bit.
    fn agree(profiles: &[Profile], d: f64, corners: Corners, tol: &Tolerance) -> &'static str {
        let indexed = offset_with(profiles, d, corners, tol, Search::Indexed(BOUNDS));
        let reference = offset_with(profiles, d, corners, tol, Search::AllPairs);
        // Debug prints every number to round trip, so equal text is equal
        // bits (and tells -0 from 0).
        assert_eq!(
            format!("{indexed:?}"),
            format!("{reference:?}"),
            "d {d}, {corners:?}: {profiles:?}"
        );
        match indexed {
            Ok(o) if o.is_whole() => "whole",
            Ok(_) => "split or joined",
            Err(KernelError::OffsetProfile {
                largest: Some(_), ..
            }) => "refused with a largest distance",
            Err(KernelError::OffsetProfile { .. }) => "refused",
            Err(_) => "other",
        }
    }

    fn with_ids(curves: &[Curve2], k: f64) -> Vec<SketchCurve> {
        curves
            .iter()
            .enumerate()
            .map(|(i, c)| SketchCurve {
                id: 2 * i as u64 + 3,
                curve: scaled(c, k),
            })
            .collect()
    }

    const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

    #[test]
    fn indexed_search_agrees_with_all_pairs_on_random_sketches() {
        let mut rng = Rng(20_261_017);
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        let mut sketches = 0;
        while sketches < 400 {
            let drawn = sketch(&mut rng);
            let k = SCALES[sketches % 4];
            let tol = Tolerance {
                linear: 1e-6 * k,
                ..Tolerance::default()
            };
            let Ok(profiles) = detect_profiles(&with_ids(&drawn, k), &tol) else {
                continue;
            };
            sketches += 1;
            for _ in 0..2 {
                let d = [-1.0, 1.0][rng.below(2) as usize]
                    * [0.05, 0.3, 0.8, 1.7, 3.5][rng.below(5) as usize]
                    * (0.5 + rng.unit())
                    * k;
                let corners = [Corners::Round, Corners::Sharp][rng.below(2) as usize];
                *seen.entry(agree(&profiles, d, corners, &tol)).or_default() += 1;
            }
        }
        for outcome in [
            "whole",
            "split or joined",
            "refused with a largest distance",
        ] {
            assert!(seen.get(outcome).copied().unwrap_or(0) >= 20, "{seen:?}");
        }
    }

    /// Shapes where the order of the cuts and the pieces matters, at four
    /// scales and at distances that keep, split, join and close loops.
    #[test]
    fn indexed_search_agrees_with_all_pairs_on_structured_sketches() {
        let mut shapes: Vec<(&str, Vec<Curve2>)> = Vec::new();
        // A sheet of holes (nested loops) that grow into each other.
        let mut sheet = rectangle((0.0, 60.0), (0.0, 60.0));
        for i in 0..25 {
            let c = [10.0 * (i % 5 + 1) as f64, 10.0 * (i / 5 + 1) as f64];
            sheet.push(circle(c, 3.0));
        }
        shapes.push(("sheet", sheet));
        // Holes tangent to each other's offsets at exactly half the gap.
        shapes.push((
            "two holes",
            [
                rectangle((0.0, 20.0), (0.0, 10.0)),
                vec![circle([6.0, 5.0], 2.0), circle([14.0, 5.0], 2.0)],
            ]
            .concat(),
        ));
        shapes.push(("saw", saw([0.0, 0.0], 12, 2.0, 6.0, 1.5)));
        shapes.push(("deep saw", saw([0.0, 0.0], 9, 1.0, 3.0, 6.0)));
        shapes.push(("star", star([0.0, 0.0], 9, 1.0, 10.0, 0.1)));
        shapes.push(("C", c_shape([0.0, 0.0], 10.0, 3.0, 2.0)));
        shapes.push(("slot", slot([0.0, 0.0], 6.0, 2.0)));
        shapes.push(("rounded", rounded((0.0, 10.0), (0.0, 6.0), 1.5)));
        let mut nested = Vec::new();
        for k in 0..6 {
            let s = 2.0 * k as f64;
            nested.extend(rectangle((s, 30.0 - s), (s, 30.0 - s)));
        }
        shapes.push(("nested squares", nested));
        let rings: Vec<Curve2> = (1..8).map(|k| circle([0.0, 0.0], 1.5 * k as f64)).collect();
        shapes.push(("concentric circles", rings));
        // Slots side by side, tangent where their offsets meet.
        let mut slots = rectangle((-2.0, 30.0), (-6.0, 6.0));
        for k in 0..4 {
            slots.extend(slot([4.0 + 7.0 * k as f64, 0.0], 3.0, 1.5));
        }
        shapes.push(("slots", slots));
        let distances = [0.1, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 6.0];
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for (name, curves) in &shapes {
            for k in SCALES {
                let tol = Tolerance {
                    linear: 1e-6 * k,
                    ..Tolerance::default()
                };
                let profiles = detect_profiles(&with_ids(curves, k), &tol)
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                for d in distances {
                    for sign in [-1.0, 1.0] {
                        for corners in [Corners::Round, Corners::Sharp] {
                            *seen
                                .entry(agree(&profiles, sign * d * k, corners, &tol))
                                .or_default() += 1;
                        }
                    }
                }
            }
        }
        for outcome in [
            "whole",
            "split or joined",
            "refused with a largest distance",
        ] {
            assert!(seen.get(outcome).copied().unwrap_or(0) >= 20, "{seen:?}");
        }
    }

    /// Mitres whose wedges are long and sharp, where the rounding of the
    /// wedge test is largest: the indexed search still agrees.
    #[test]
    fn sharp_mitres_agree() {
        let mut seen = 0;
        for teeth in [3, 7] {
            for spike in [20.0, 200.0, 2000.0] {
                let curves = star([5.0, -3.0], teeth, 1.0, spike, 0.3);
                for k in SCALES {
                    let tol = Tolerance {
                        linear: 1e-6 * k,
                        ..Tolerance::default()
                    };
                    let profiles = detect_profiles(&with_ids(&curves, k), &tol).unwrap();
                    for d in [-0.3, 0.05, 0.3, 2.0] {
                        agree(&profiles, d * k, Corners::Sharp, &tol);
                        seen += 1;
                    }
                }
            }
        }
        assert_eq!(seen, 96);
    }

    /// Every cap and bound on the work, met with a small input under
    /// small bounds: refused as such, naming curves of the input, without
    /// a search for a distance; and a bound met by that search ends it
    /// without a distance, the reason staying the outline's.
    #[test]
    fn bounds_are_refused_naming_the_curves() {
        let tol = Tolerance::default();
        let small = |f: &dyn Fn(&mut Bounds)| {
            let mut b = BOUNDS;
            f(&mut b);
            Search::Indexed(b)
        };
        let of = |curves: &[Curve2]| detect_profiles(&with_ids(curves, 1.0), &tol).unwrap();
        let ids = |curves: &[Curve2]| -> Vec<u64> {
            (0..curves.len()).map(|i| 2 * i as u64 + 3).collect()
        };
        let crowded_by = |result: Result<OffsetOutline, KernelError>, what: &str, input: &[u64]| {
            let Err(KernelError::CrowdedOffset {
                what: w, curves, ..
            }) = &result
            else {
                panic!("{result:?}");
            };
            assert!(w.contains(what), "{w}");
            assert!(!curves.is_empty() && curves.len() <= MAX_REPORTED);
            assert!(curves.iter().all(|c| input.contains(c)), "{curves:?}");
            assert!(curves.windows(2).all(|w| w[0] < w[1]), "{curves:?}");
            // The message names them.
            let text = result.as_ref().unwrap_err().to_string();
            assert!(text.contains(&curves[0].to_string()), "{text}");
        };

        let mut nested = Vec::new();
        for k in 0..6 {
            let s = 2.0 * k as f64;
            nested.extend(rectangle((s, 30.0 - s), (s, 30.0 - s)));
        }
        let input = of(&nested);
        let r = offset_with(&input, 0.1, Corners::Round, &tol, small(&|b| b.pairs = 10));
        crowded_by(r, "pairs of curves", &ids(&nested));

        let rays = star([0.0, 0.0], 9, 1.0, 10.0, 0.1);
        let input = of(&rays);
        let r = offset_with(
            &input,
            -0.3,
            Corners::Round,
            &tol,
            small(&|b| b.cut_points = 2),
        );
        crowded_by(r, "points where its curves cross", &ids(&rays));

        let mut sheet = rectangle((0.0, 60.0), (0.0, 60.0));
        for i in 0..25 {
            sheet.push(circle(
                [10.0 * (i % 5 + 1) as f64, 10.0 * (i / 5 + 1) as f64],
                3.0,
            ));
        }
        let input = of(&sheet);
        let r = offset_with(&input, 0.1, Corners::Round, &tol, small(&|b| b.near = 10));
        crowded_by(r, "near the middles", &ids(&sheet));
        // The same with sharp corners, where wedges are searched too: a
        // saw's valleys.
        let teeth = saw([0.0, 0.0], 12, 2.0, 6.0, 1.5);
        let input = of(&teeth);
        let r = offset_with(&input, -0.2, Corners::Sharp, &tol, small(&|b| b.near = 40));
        crowded_by(r, "near the middles", &ids(&teeth));

        // A C whose back parts: its long side leaves a piece in each
        // region, named by the curves of its loop.
        let c = c_shape([0.0, 0.0], 10.0, 3.0, 2.0);
        let input = of(&c);
        let whole =
            offset_with(&input, -1.2, Corners::Round, &tol, Search::Indexed(BOUNDS)).unwrap();
        assert!(!whole.is_whole());
        let r = offset_with(&input, -1.2, Corners::Round, &tol, small(&|b| b.named = 3));
        crowded_by(r, "names of its pieces", &ids(&c));

        // The caps on curves.
        let square = rectangle((0.0, 10.0), (0.0, 10.0));
        let input = of(&square);
        let r = offset_with(&input, 1.0, Corners::Round, &tol, small(&|b| b.pieces = 7));
        assert!(
            matches!(
                r,
                Err(KernelError::TooManyOffsetCurves { count: 8, limit: 7 })
            ),
            "{r:?}"
        );
        let r = offset_with(&input, 1.0, Corners::Round, &tol, small(&|b| b.raw = 5));
        assert!(
            matches!(r, Err(KernelError::TooManyOffsetCurves { limit: 5, .. })),
            "{r:?}"
        );

        // Inward past the outline's reach: the search for the largest
        // distance meets the bound and ends without a distance.
        let r = offset_with(&input, -1e10, Corners::Round, &tol, small(&|b| b.pairs = 1));
        assert!(
            matches!(
                &r,
                Err(KernelError::OffsetProfile {
                    reason: "leaves nothing of a region",
                    largest: None,
                    ..
                })
            ),
            "{r:?}"
        );
        let r = offset_with(&input, -1e10, Corners::Round, &tol, Search::Indexed(BOUNDS));
        let Err(KernelError::OffsetProfile {
            largest: Some(largest),
            ..
        }) = r
        else {
            panic!("{r:?}");
        };
        assert!((largest + 5.0).abs() <= 2.0 * tol.linear, "{largest}");
    }
}
