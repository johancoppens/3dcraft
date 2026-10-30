//! Profile detection's search for pairs as it was before 2026-10-17:
//! sweeps in x over every pair of ends, of curves and of loops. Kept as
//! the oracle the indexed search ([`super::BoxTree`]) must agree with, to
//! the error and the order of the result.

use super::*;

/// How many times the loop goes around `p`, counter-clockwise: the turn of
/// every segment, summed.
fn winding(path: &Loop2, p: P2) -> f64 {
    let turn: f64 = path.segments.iter().map(|s| s.turn_about(p)).sum();
    (turn / TAU).round()
}

/// For every end the end it meets. Errors when an end meets none, or more
/// than one.
pub(super) fn join_ends(
    curves: &[SketchCurve],
    ends: &[End],
    tol: f64,
) -> Result<Vec<usize>, KernelError> {
    let mut order: Vec<usize> = (0..ends.len()).collect();
    let x = |i: usize| ends.get(i).map_or(f64::NAN, |e| e.point[0]);
    order.sort_by(|&i, &j| x(i).total_cmp(&x(j)).then(i.cmp(&j)));
    // The first few ends near each end, and how many there are in all.
    let mut near: Vec<(usize, [usize; 2])> = vec![(0, [usize::MAX; 2]); ends.len()];
    let mut record = |i: usize, j: usize| {
        if let Some((count, list)) = near.get_mut(i) {
            if let Some(slot) = list.get_mut(*count) {
                *slot = j;
            }
            *count = count.saturating_add(1);
        }
    };
    for (position, &i) in order.iter().enumerate() {
        let Some(a) = ends.get(i) else { continue };
        for &j in order.iter().skip(position + 1) {
            let Some(b) = ends.get(j) else { continue };
            if b.point[0] - a.point[0] > tol {
                break;
            }
            if distance(a.point, b.point) <= tol {
                record(i, j);
                record(j, i);
            }
        }
    }
    let id_of = |end: usize| {
        ends.get(end)
            .and_then(|e| curves.get(e.curve))
            .map(|c| c.id)
    };
    let report = |mut ids: Vec<u64>| {
        ids.sort_unstable();
        ids.dedup();
        ids.truncate(MAX_REPORTED);
        ids
    };
    let mut branching = Vec::new();
    let mut open = Vec::new();
    for (i, (count, list)) in near.iter().enumerate() {
        match count {
            0 => open.extend(id_of(i)),
            1 => {}
            _ => {
                branching.extend(id_of(i));
                branching.extend(list.iter().filter_map(|&j| id_of(j)));
            }
        }
    }
    if !branching.is_empty() {
        return Err(KernelError::BranchingProfile {
            curves: report(branching),
        });
    }
    if !open.is_empty() {
        return Err(KernelError::OpenProfile {
            curves: report(open),
        });
    }
    Ok(near.iter().map(|(_, list)| list[0]).collect())
}

/// Errors when two curves touch, cross or overlap away from a shared end.
/// `mate_curve[i][k]` is the curve joined to end `k` of curve `i`.
pub(super) fn check_clear(
    own: &[Segment],
    mate_curve: &[[Option<usize>; 2]],
    tol: f64,
) -> Result<(), KernelError> {
    // Boxes grown by the tolerance, swept in x: only pairs whose boxes
    // overlap are compared.
    let boxes: Vec<[f64; 4]> = own
        .iter()
        .map(|s| {
            let b = s.bbox();
            [b[0] - tol, b[1] - tol, b[2] + tol, b[3] + tol]
        })
        .collect();
    // A spline can cross itself.
    for segment in own {
        if matches!(segment.shape, Shape::Spline(_)) && !spline2::self_clear(&segment.parts(), tol)
        {
            return Err(KernelError::CrossingCurves {
                curves: [segment.source, segment.source],
            });
        }
    }
    let mut order: Vec<usize> = (0..own.len()).collect();
    let min_x = |i: usize| boxes.get(i).map_or(f64::NAN, |b| b[0]);
    order.sort_by(|&i, &j| min_x(i).total_cmp(&min_x(j)).then(i.cmp(&j)));
    for (position, &i) in order.iter().enumerate() {
        let (Some(a), Some(box_a), Some(mates_a)) = (own.get(i), boxes.get(i), mate_curve.get(i))
        else {
            return Err(internal());
        };
        for &j in order.iter().skip(position + 1) {
            let (Some(b), Some(box_b), Some(mates_b)) =
                (own.get(j), boxes.get(j), mate_curve.get(j))
            else {
                return Err(internal());
            };
            if box_b[0] > box_a[2] {
                break;
            }
            if box_b[1] > box_a[3] || box_a[1] > box_b[3] {
                continue;
            }
            let shared_a = mates_a.map(|m| m == Some(j));
            let shared_b = mates_b.map(|m| m == Some(i));
            if !(keeps_clear(a, b, shared_a, tol) && keeps_clear(b, a, shared_b, tol)) {
                return Err(KernelError::CrossingCurves {
                    curves: [a.source.min(b.source), a.source.max(b.source)],
                });
            }
        }
    }
    Ok(())
}

/// Nesting by every pair of loops.
pub(super) fn nest(rings: &[Ring]) -> Vec<(usize, Option<usize>)> {
    let mut nesting = Vec::with_capacity(rings.len());
    for (i, ring) in rings.iter().enumerate() {
        let p = ring.path.origin();
        let (mut depth, mut parent): (usize, Option<(f64, usize)>) = (0, None);
        for (j, around) in rings.iter().enumerate() {
            let b = around.bbox;
            let outside_box = p[0] < b[0] || p[1] < b[1] || p[0] > b[2] || p[1] > b[3];
            if i == j || outside_box || winding(&around.path, p) == 0.0 {
                continue;
            }
            depth += 1;
            if parent.is_none_or(|(area, _)| around.area < area) {
                parent = Some((around.area, j));
            }
        }
        nesting.push((depth, parent.map(|(_, j)| j)));
    }
    nesting
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    /// SplitMix64: seeded, the same on every platform.
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
            self.next() % n.max(1)
        }

        /// A whole number in `0..n`, as a coordinate.
        fn grid(&mut self, n: u64) -> f64 {
            self.below(n) as f64
        }

        fn shuffle<T>(&mut self, items: &mut [T]) {
            for i in (1..items.len()).rev() {
                items.swap(i, self.below(i as u64 + 1) as usize);
            }
        }
    }

    fn line(start: P2, end: P2) -> Curve2 {
        Curve2::Line { start, end }
    }

    fn polygon(points: &[P2]) -> Vec<Curve2> {
        (0..points.len())
            .map(|i| line(points[i], points[(i + 1) % points.len()]))
            .collect()
    }

    fn rectangle(x: (f64, f64), y: (f64, f64)) -> Vec<Curve2> {
        polygon(&[[x.0, y.0], [x.1, y.0], [x.1, y.1], [x.0, y.1]])
    }

    /// A slot: two lines and two half circles, `l` between the centres.
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

    /// A closed cubic spline through eight control points about `c`.
    fn blob(c: P2, r: f64) -> Curve2 {
        let mut control: Vec<P2> = (0..8)
            .map(|i| {
                let a = TAU * f64::from(i) / 8.0;
                let s = if i % 2 == 0 { r } else { 0.7 * r };
                [c[0] + s * a.cos(), c[1] + s * a.sin()]
            })
            .collect();
        control.push(control[0]);
        let n = control.len();
        let mut knots = vec![0.0; 4];
        knots.extend((1..n - 3).map(|k| k as f64));
        knots.extend([(n - 3) as f64; 4]);
        Curve2::Spline(Spline2 {
            degree: 3,
            knots,
            control,
        })
    }

    fn scaled_curve(c: &Curve2, k: f64) -> Curve2 {
        let s = |p: P2| [p[0] * k, p[1] * k];
        match c {
            &Curve2::Line { start, end } => line(s(start), s(end)),
            &Curve2::Arc { center, start, end } => Curve2::Arc {
                center: s(center),
                start: s(start),
                end: s(end),
            },
            &Curve2::Circle { center, radius } => Curve2::Circle {
                center: s(center),
                radius: radius * k,
            },
            Curve2::Spline(spline) => Curve2::Spline(Spline2 {
                degree: spline.degree,
                knots: spline.knots.clone(),
                control: spline.control.iter().map(|&p| s(p)).collect(),
            }),
        }
    }

    /// One random sketch: a few shapes of every kind on a small grid, so
    /// that they nest, touch, share sides, cross and meet tangentially.
    fn sketch(rng: &mut Rng) -> Vec<Curve2> {
        let mut curves = Vec::new();
        for _ in 0..1 + rng.below(6) {
            match rng.below(7) {
                0 => {
                    let n = 3 + rng.below(4);
                    let points: Vec<P2> = (0..n).map(|_| [rng.grid(24), rng.grid(24)]).collect();
                    curves.extend(polygon(&points));
                }
                1 | 2 => {
                    let (x, y) = (rng.grid(20), rng.grid(20));
                    let (w, h) = (1.0 + rng.grid(8), 1.0 + rng.grid(8));
                    curves.extend(rectangle((x, x + w), (y, y + h)));
                }
                3 | 4 => curves.push(Curve2::Circle {
                    center: [rng.grid(24), rng.grid(24)],
                    radius: 1.0 + rng.grid(6),
                }),
                5 => curves.extend(slot(
                    [rng.grid(20), rng.grid(20)],
                    rng.grid(6) + 1.0,
                    1.0 + rng.grid(3),
                )),
                _ => curves.push(blob([rng.grid(24), rng.grid(24)], 1.0 + rng.grid(5))),
            }
        }
        // Now and then a spoiled end: moved by less or more than the
        // tolerance (with the scale applied later, 1e-6 per unit), or a
        // stray line that branches or dangles.
        match rng.below(6) {
            0 => {
                if let Some(Curve2::Line { start, .. }) = curves.first_mut() {
                    start[0] += 4e-7;
                }
            }
            1 => {
                if let Some(Curve2::Line { start, .. }) = curves.first_mut() {
                    start[1] += 3e-6;
                }
            }
            2 => curves.push(line(
                [rng.grid(24), rng.grid(24)],
                [rng.grid(24), rng.grid(24)],
            )),
            _ => {}
        }
        curves
    }

    /// The indexed search gives what the sweeps over every pair give: the
    /// same profiles in the same order, or the same error.
    fn agree(curves: &[SketchCurve], tol: &Tolerance) -> Result<Vec<Profile>, KernelError> {
        let indexed = detect(curves, tol, Search::Indexed, &BTreeSet::new());
        let reference = detect(curves, tol, Search::AllPairs, &BTreeSet::new());
        assert_eq!(indexed, reference, "{curves:?}");
        indexed
    }

    fn kind(r: &Result<Vec<Profile>, KernelError>) -> &'static str {
        match r {
            Ok(_) => "ok",
            Err(KernelError::OpenProfile { .. }) => "open",
            Err(KernelError::BranchingProfile { .. }) => "branching",
            Err(KernelError::CrossingCurves { .. }) => "crossing",
            Err(KernelError::ZeroAreaLoop { .. }) => "zero area",
            Err(KernelError::DegenerateCurve { .. }) => "degenerate",
            Err(_) => "other",
        }
    }

    #[test]
    fn indexed_search_agrees_with_all_pairs_on_random_sketches() {
        let mut rng = Rng(20_261_017);
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for round in 0..1500 {
            let drawn = sketch(&mut rng);
            let k = [1e-3, 1.0, 1e3, 1e6][round % 4];
            let mut curves: Vec<SketchCurve> = drawn
                .iter()
                .enumerate()
                .map(|(i, c)| SketchCurve {
                    id: 3 * i as u64 + 1,
                    curve: scaled_curve(c, k),
                })
                .collect();
            rng.shuffle(&mut curves);
            let tol = Tolerance {
                linear: 1e-6 * k,
                ..Tolerance::default()
            };
            *seen.entry(kind(&agree(&curves, &tol))).or_default() += 1;
            // The small sketch at the default tolerance too, which is then
            // coarse: more ends meet and more curves touch. (At large
            // scales it is the tolerance of a much finer drawing, which
            // splines are refined to at length.)
            if k < 1.0 {
                let _ = agree(&curves, &Tolerance::default());
            }
        }
        // Every outcome was met, many times.
        for outcome in ["ok", "open", "branching", "crossing"] {
            assert!(seen.get(outcome).copied().unwrap_or(0) >= 20, "{seen:?}");
        }
    }

    /// Larger inputs where the order of pairs matters: many shapes, many
    /// of them in error, the first failing pair and the reported ids
    /// must be the reference's.
    #[test]
    fn indexed_search_agrees_with_all_pairs_on_crowded_sketches() {
        let mut rng = Rng(17);
        for round in 0..40 {
            let mut drawn = Vec::new();
            for _ in 0..30 + rng.below(60) {
                drawn.extend(sketch(&mut rng).into_iter().map(|c| {
                    // Spread the shapes out on a larger grid.
                    let shift = [rng.grid(8) * 30.0, rng.grid(8) * 30.0];
                    shifted(&c, shift)
                }));
            }
            let k = [1e-3, 1.0, 1e6][round % 3];
            let mut curves: Vec<SketchCurve> = drawn
                .iter()
                .enumerate()
                .map(|(i, c)| SketchCurve {
                    id: i as u64,
                    curve: scaled_curve(c, k),
                })
                .collect();
            rng.shuffle(&mut curves);
            agree(
                &curves,
                &Tolerance {
                    linear: 1e-6 * k,
                    ..Tolerance::default()
                },
            )
            .ok();
        }
    }

    fn shifted(c: &Curve2, d: P2) -> Curve2 {
        let s = |p: P2| [p[0] + d[0], p[1] + d[1]];
        match c {
            &Curve2::Line { start, end } => line(s(start), s(end)),
            &Curve2::Arc { center, start, end } => Curve2::Arc {
                center: s(center),
                start: s(start),
                end: s(end),
            },
            &Curve2::Circle { center, radius } => Curve2::Circle {
                center: s(center),
                radius,
            },
            Curve2::Spline(spline) => Curve2::Spline(Spline2 {
                degree: spline.degree,
                knots: spline.knots.clone(),
                control: spline.control.iter().map(|&p| s(p)).collect(),
            }),
        }
    }

    fn with_ids(curves: Vec<Curve2>) -> Vec<SketchCurve> {
        curves
            .into_iter()
            .enumerate()
            .map(|(i, curve)| SketchCurve {
                id: i as u64,
                curve,
            })
            .collect()
    }

    /// Structured layouts: sheets of holes and slots, deep nesting,
    /// tangent circles in a row, a column of squares, circles in a ring
    /// that all touch their neighbours, and many ends at one point.
    #[test]
    fn indexed_search_agrees_with_all_pairs_on_structured_sketches() {
        let tol = Tolerance::default();
        let mut cases: Vec<(&str, Vec<Curve2>)> = Vec::new();
        let mut sheet = rectangle((0.0, 110.0), (0.0, 110.0));
        for i in 0..100 {
            let c = [(i % 10 + 1) as f64 * 10.0, (i / 10 + 1) as f64 * 10.0];
            if i % 3 == 0 {
                sheet.extend(slot(c, 3.0, 2.0));
            } else if i % 3 == 1 {
                sheet.push(Curve2::Circle {
                    center: c,
                    radius: 3.0,
                });
            } else {
                sheet.push(blob(c, 3.0));
            }
        }
        cases.push(("sheet", sheet.clone()));
        // A hole moved onto its neighbour: tangent circles.
        let mut tangent = sheet.clone();
        tangent.push(Curve2::Circle {
            center: [15.0, 10.0],
            radius: 2.0,
        });
        cases.push(("tangent hole", tangent));
        cases.push((
            "nested squares",
            (0..60)
                .flat_map(|i| {
                    let d = f64::from(i);
                    rectangle((d, 200.0 - d), (d, 200.0 - d))
                })
                .collect(),
        ));
        cases.push((
            "concentric circles and slots",
            (0..60)
                .flat_map(|i| {
                    let r = 2.0 + f64::from(i);
                    if i % 2 == 0 {
                        vec![Curve2::Circle {
                            center: [0.0, 0.0],
                            radius: r,
                        }]
                    } else {
                        slot([0.0, 0.0], 1e-3, r)
                    }
                })
                .collect(),
        ));
        cases.push((
            "tangent circles in a row",
            (0..50)
                .map(|i| Curve2::Circle {
                    center: [2.0 * f64::from(i), 0.0],
                    radius: 1.0,
                })
                .collect(),
        ));
        let mut column = rectangle((0.0, 3.0), (0.0, 201.0));
        for i in 0..100 {
            let y = 2.0 * f64::from(i) + 1.0;
            column.extend(rectangle((1.0, 2.0), (y, y + 1.0)));
        }
        cases.push(("column", column.clone()));
        // Two squares of the column that share a side.
        let mut shared = column;
        shared.extend(rectangle((1.0, 2.0), (2.0, 3.0)));
        cases.push(("squares sharing a side", shared));
        cases.push((
            "a star of lines",
            (0..40)
                .map(|i| {
                    let a = TAU * f64::from(i) / 40.0;
                    line([0.0, 0.0], [a.cos(), a.sin()])
                })
                .collect(),
        ));
        cases.push((
            "the same square many times",
            (0..30)
                .flat_map(|_| rectangle((0.0, 1.0), (0.0, 1.0)))
                .collect(),
        ));
        cases.push((
            "squares crossing in a chain",
            (0..40)
                .flat_map(|i| {
                    let x = f64::from(i) * 0.5;
                    rectangle((x, x + 1.0), (0.0, 1.0))
                })
                .collect(),
        ));
        for (name, curves) in cases {
            for k in [1e-3, 1.0, 1e3, 1e6] {
                let scaled: Vec<Curve2> = curves.iter().map(|c| scaled_curve(c, k)).collect();
                let tol_k = Tolerance {
                    linear: tol.linear * k,
                    ..tol
                };
                let result = agree(&with_ids(scaled), &tol_k);
                if name == "sheet" || name == "column" || name == "nested squares" {
                    assert!(result.is_ok(), "{name} at {k}: {result:?}");
                }
            }
        }
    }
    /// Big star outlines (many segments, so winding numbers go through the
    /// tree of stretches) with circles and squares scattered in and out of
    /// them and sometimes a smaller star inside: the same as the reference.
    #[test]
    fn indexed_search_agrees_with_all_pairs_on_big_loops_with_holes() {
        let mut rng = Rng(99);
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for round in 0..400 {
            let n = 2 * (5 + rng.below(60) as usize);
            let (r0, r1) = (50.0, 55.0 + rng.grid(40));
            let star = |n: usize, r0: f64, r1: f64, turn: f64| -> Vec<P2> {
                (0..n)
                    .map(|i| {
                        let a = TAU * i as f64 / n as f64 + turn;
                        let r = if i % 2 == 0 { r0 } else { r1 };
                        [r * a.cos(), r * a.sin()]
                    })
                    .collect()
            };
            let mut curves = polygon(&star(n, r0, r1, 0.01 * round as f64));
            for _ in 0..rng.below(40) {
                curves.push(Curve2::Circle {
                    center: [rng.grid(200) - 100.0, rng.grid(200) - 100.0],
                    radius: 0.5 + rng.grid(2),
                });
            }
            for _ in 0..rng.below(10) {
                let c = [rng.grid(200) - 100.0, rng.grid(200) - 100.0];
                curves.extend(rectangle((c[0], c[0] + 1.0), (c[1], c[1] + 1.0)));
            }
            if rng.below(2) == 0 {
                let m = 2 * (5 + rng.below(30) as usize);
                curves.extend(polygon(&star(m, 20.0, 35.0, 0.0)));
            }
            let k = [1e-3, 1.0, 1e3, 1e6][round % 4];
            let mut scaled: Vec<SketchCurve> = curves
                .iter()
                .enumerate()
                .map(|(i, c)| SketchCurve {
                    id: i as u64,
                    curve: scaled_curve(c, k),
                })
                .collect();
            rng.shuffle(&mut scaled);
            let tol = Tolerance {
                linear: 1e-6 * k,
                ..Tolerance::default()
            };
            *seen.entry(kind(&agree(&scaled, &tol))).or_default() += 1;
        }
        assert!(seen.get("ok").copied().unwrap_or(0) > 50, "{seen:?}");
    }
}
