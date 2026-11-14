//! The plate's corner where two square rims run on smoothly into each
//! other (docs/design/plates.md › *Corners*; found 2026-11-03): a hole of a
//! quarter, a half and a quarter circle in a sheet-metal part's flat,
//! plain and on a jog's far side (a flat folded away from the pattern's
//! plane), keeps the volume of the plate less the disc at 1e-3, 1 and 1e6.
//! Before the fix the corner between two rims on one cylinder was solved
//! on coinciding surfaces, drifted along their common line, and the edges
//! went round the wrong side of the circle: a body the checker passes,
//! 2 % short of its volume.
//!
//! Found again 2026-11-14 (A-019, a flange with a gear sector): whether
//! the boundary runs on at a vertex was read from the dot of the two
//! tangents against the cosine of the angular tolerance, which is 1 in
//! floating point, so the joint ran on only when the dot rounded to −1 or
//! below; one unit in the last place above it made the joint a crease and
//! the corner drifted as before. Held at sixty places a circle is split at
//! (its centre, radius and three cuts moved together) at three scales: a
//! disc of three arcs plated on its own (`plate`, the case reduced to one
//! face and its loop: 30 of 180 were valid bodies of twice the volume, the
//! loop going round its circle twice), and a hole of three arcs in a
//! sheet-metal plate (24 of 180 valid and 4 % short, the hole taken out
//! twice), each now to its closed form.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::extrude::fill;
use craft3d_kernel::plate::{Side, plate as plate_faces};
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{Closing, Direction, JogSpec, Spec, build, web_length};
use craft3d_math::{Frame, Portable, Tolerance};
use craft3d_topo::{FaceId, check, mass_properties};
use std::collections::BTreeMap;
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn plate(k: f64, extra: Vec<SketchCurve>) -> Profile {
    let c = [[0.0, 0.0], [100.0, 0.0], [100.0, 40.0], [0.0, 40.0]];
    let mut curves: Vec<SketchCurve> = (0..4)
        .map(|i| SketchCurve {
            id: i as u64 + 1,
            curve: Curve2::Line {
                start: [c[i][0] * k, c[i][1] * k],
                end: [c[(i + 1) % 4][0] * k, c[(i + 1) % 4][1] * k],
            },
        })
        .collect();
    curves.extend(extra);
    detect_profiles(&curves, &tolerance(k)).unwrap().remove(0)
}

fn spec(base: Profile, k: f64, jog: bool) -> Spec {
    Spec {
        base,
        plane: Frame::WORLD,
        thickness: 1.0 * k,
        flanges: Vec::new(),
        relief: None,
        bend_relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts: Vec::new(),
        jogs: if jog {
            vec![JogSpec {
                line: [[50.0 * k, -5.0 * k], [50.0 * k, 65.0 * k]],
                offset: 8.0 * k,
                angle: PI / 3.0,
                direction: Direction::Up,
                radius: 1.0 * k,
                k: 0.5,
                ids: [1, 2],
            }]
        } else {
            Vec::new()
        },
        ids: Vec::new(),
    }
}

/// A hole of three arcs of one circle about `c`, radius `r`, split at the
/// angles `cuts` (degrees, increasing, less than a turn apart).
fn split_circle(c: [f64; 2], r: f64, cuts: [f64; 3]) -> Vec<SketchCurve> {
    let p = |deg: f64| {
        let a = deg.to_radians();
        [c[0] + r * a.pcos(), c[1] + r * a.psin()]
    };
    let [a, b, d] = cuts;
    [(a, b), (b, d), (d, a + 360.0)]
        .iter()
        .enumerate()
        .map(|(i, &(from, to))| SketchCurve {
            id: 20 + i as u64,
            curve: Curve2::Arc {
                center: c,
                start: p(from),
                end: p(to),
            },
        })
        .collect()
}

/// The case reduced to the plate of one face: a disc whose boundary is
/// three arcs of one circle, each an arc of its own as a sketch's are,
/// plated 1 thick (at scale `k`). Its volume is `π r² T` and its area
/// `2π r² + 2π r T`.
#[test]
fn a_disc_of_three_arcs_of_one_circle_plates_to_its_closed_form() {
    let mut failed = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        for i in 0..60 {
            let shift = f64::from(i) * 1e-7;
            let c = [(3.1 + shift) * k, (-2.3 + shift) * k];
            let r = (7.0 - shift) * k;
            let cuts = [
                29.0 + 1.7 * f64::from(i),
                151.0 + 0.9 * f64::from(i),
                263.0 - 0.6 * f64::from(i),
            ];
            let curves = split_circle(c, r, cuts);
            let profiles = detect_profiles(&curves, &tol).unwrap();
            let sheet = fill(&profiles, &Frame::WORLD, &tol).unwrap().body;
            assert_eq!(sheet.edges.len(), 3, "the three arcs stay three edges");
            let faces: Vec<FaceId> = (0..sheet.faces.len() as u32).map(FaceId).collect();
            let t = 1.0 * k;
            let (v, a) = (PI * r * r * t, 2.0 * PI * r * r + 2.0 * PI * r * t);
            match plate_faces(&sheet, &faces, t, Side::Out, &BTreeMap::new(), &tol) {
                Ok(made) => {
                    let defects = check(&made.body, &tol);
                    match mass_properties(&made.body) {
                        Ok(m)
                            if defects.is_empty()
                                && (m.volume - v).abs() <= 1e-9 * v
                                && (m.area - a).abs() <= 1e-9 * a => {}
                        Ok(m) => failed.push(format!(
                            "{k} #{i}: volume {} against {v}, area {} against {a}, {} defects",
                            m.volume,
                            m.area,
                            defects.len()
                        )),
                        Err(e) => failed.push(format!(
                            "{k} #{i}: not measured ({e:?}), {} defects",
                            defects.len()
                        )),
                    }
                }
                Err(e) => failed.push(format!("{k} #{i}: {e}")),
            }
        }
    }
    assert!(failed.is_empty(), "{} failed: {failed:#?}", failed.len());
}

#[test]
fn a_hole_of_arcs_of_one_circle_split_anywhere_keeps_its_volume() {
    let mut failed = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        for i in 0..60 {
            let shift = f64::from(i) * 1e-7;
            let c = [(41.3 + shift) * k, (19.7 - shift) * k];
            let r = (7.0 + shift) * k;
            let cuts = [
                13.0 + 2.9 * f64::from(i),
                131.0 + 1.3 * f64::from(i),
                257.0 - 0.7 * f64::from(i),
            ];
            let holed = plate(k, split_circle(c, r, cuts));
            assert_eq!(holed.holes.len(), 1);
            let v = (4000.0 - PI * (r / k) * (r / k)) * k * k * k;
            match build(&spec(holed, k, false), &tol) {
                Ok(part) => {
                    let defects = check(&part.solid.body, &tol);
                    match mass_properties(&part.solid.body) {
                        Ok(m) if defects.is_empty() && (m.volume - v).abs() <= 1e-9 * v => {}
                        Ok(m) => failed.push(format!(
                            "{k} #{i}: {} against {v}, {} defects",
                            m.volume,
                            defects.len()
                        )),
                        Err(e) => failed.push(format!(
                            "{k} #{i}: not measured ({e:?}), {} defects",
                            defects.len()
                        )),
                    }
                }
                Err(e) => failed.push(format!("{k} #{i}: {e}")),
            }
        }
    }
    assert!(failed.is_empty(), "{} failed: {failed:#?}", failed.len());
}

#[test]
fn a_hole_of_arcs_of_one_circle_keeps_its_volume() {
    for k in SCALES {
        let tol = tolerance(k);
        for jog in [false, true] {
            for cx in [30.0, 75.0] {
                let c = [cx * k, 20.0 * k];
                let p = |deg: f64| {
                    let a = deg.to_radians();
                    [c[0] + 5.0 * k * a.pcos(), c[1] + 5.0 * k * a.psin()]
                };
                let arc = |id: u64, from: f64, to: f64| SketchCurve {
                    id,
                    curve: Curve2::Arc {
                        center: c,
                        start: p(from),
                        end: p(to),
                    },
                };
                let holed = plate(
                    k,
                    vec![
                        arc(9, 0.0, 90.0),
                        arc(10, 90.0, 270.0),
                        arc(11, 270.0, 360.0),
                    ],
                );
                assert_eq!(holed.holes.len(), 1);
                let part = build(&spec(holed, k, jog), &tol).unwrap();
                let defects = check(&part.solid.body, &tol);
                assert!(defects.is_empty(), "{defects:?}");
                // The plate in closed form: 100 × 40 × T, jogged the near
                // side, the web and the far side plus the two bends'
                // ring sectors `α 40 T (R + T/2)` (R = T = 1).
                let alpha = PI / 3.0;
                let plain = if jog {
                    let w = web_length(8.0, alpha, 1.0, 1.0).unwrap();
                    40.0 * (100.0 + w) + 2.0 * alpha * 40.0 * 1.5
                } else {
                    4000.0
                } * k
                    * k
                    * k;
                let v = plain - PI * 25.0 * k * k * k;
                let got = mass_properties(&part.solid.body).unwrap().volume;
                assert!(
                    (got - v).abs() <= 1e-9 * v,
                    "jog {jog}, centre {cx} at {k}: {got} against {v} ({:e})",
                    (got - v) / v
                );
            }
        }
    }
}
