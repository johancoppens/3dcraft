//! Bend reliefs cut into a closed corner's extension, and bend reliefs
//! that do not fit their flat (docs/design/sheetmetal.md › *Bend
//! reliefs*), held to closed forms at 1e-3, 1 and 1e6.
//!
//! - A tray `60 × 40`, walls 10 on every edge, `R = T = 1`, `K = ½`,
//!   round corner reliefs of 3, every corner closed by overlap with a gap
//!   of 0.25, and a return flange on the bottom wall's tip spanning the
//!   whole tip (the spot the sheet-metal enclosure Jig stopped its return
//!   flanges short of): the closed corners run the tip on past both ends
//!   of the return's bend, so both ends lie inside the extended edge.
//!   Until 2026-10-31 a bend relief there was refused ("not built,
//!   owed"); now each shape's notch is cut into the extension: the flat
//!   area loses exactly two notches (`s²`, `π s²/4`, `s² + 3π s²/4`), the
//!   volume is the flat area times `T`, both ends carry the relief, the
//!   part passes the checker, folds and unfolds, and every name of the
//!   part without the relief is kept (the new ones all `edge(11, …)`).
//! - A relief that leaves less than a sliver of the extension is refused
//!   naming the return flange.
//! - A tear relief whose circle (reaching `2s` along the edge and into
//!   the flat) runs out of a short wall's flat across its tangent line,
//!   or out of an extension across its end, is refused naming the flange
//!   before any body is made; round of the same size, reaching `s`, is
//!   built.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, CornerKind, CornerSpec, Direction, FlangeSpec, LengthFrom, On, Part, Relief,
    ReliefKind, SheetFace, Spec, build, developed_allowance, edge_numbers, flat_area,
};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{FaceId, check, mass_properties};
use std::collections::BTreeSet;
use std::f64::consts::{FRAC_PI_2, PI};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// The rectangle `0..w` × `0..d` scaled by `k`: curves 1 to 4 from the
/// bottom edge on.
fn rect(w: f64, d: f64, k: f64) -> Profile {
    let pts = [[0.0, 0.0], [w, 0.0], [w, d], [0.0, d]];
    let curves: Vec<SketchCurve> = (0..4)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            SketchCurve {
                id: i as u64 + 1,
                curve: Curve2::Line {
                    start: [a[0] * k, a[1] * k],
                    end: [b[0] * k, b[1] * k],
                },
            }
        })
        .collect();
    let mut found = detect_profiles(&curves, &tolerance(k)).unwrap();
    found.remove(0)
}

fn flange(on: On, length: f64, r: f64, inset: [f64; 2]) -> FlangeSpec {
    FlangeSpec {
        on,
        length,
        angle: FRAC_PI_2,
        direction: Direction::Up,
        radius: r,
        k: 0.5,
        from: LengthFrom::Tangent,
        inset,
        outline: None,
    }
}

/// The tray: walls `wall` long on the four edges (flanges 0 to 3, from the
/// bottom edge on), radius `r`, round corner reliefs of `corner`, every
/// corner closed by overlap with a gap of `0.25 k`, and a return `6 k`
/// long on the bottom wall's tip with the insets `inset` (flange 4).
fn tray(k: f64, wall: f64, r: f64, corner: f64, inset: [f64; 2], bend: Option<Relief>) -> Spec {
    let mut flanges: Vec<FlangeSpec> = (1..=4)
        .map(|c| flange(On::Base(c), wall, r, [0.0, 0.0]))
        .collect();
    flanges.push(flange(On::Tip(0), 6.0 * k, r, inset));
    Spec {
        base: rect(60.0, 40.0, k),
        plane: Frame::WORLD,
        thickness: k,
        flanges,
        relief: Some(Relief {
            kind: ReliefKind::Round,
            size: corner,
        }),
        bend_relief: bend,
        corner: Closing {
            kind: CornerKind::Overlap,
            gap: 0.25 * k,
            over: None,
        },
        corners: Vec::new(),
        cutouts: Vec::new(),
        jogs: Vec::new(),
        ids: Vec::new(),
    }
}

#[track_caller]
fn near(actual: f64, expected: f64, slack: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= slack,
        "{what}: {actual} against {expected} (off by {:e}, allowed {slack:e})",
        actual - expected
    );
}

fn volume(part: &Part) -> f64 {
    mass_properties(&part.solid.body).unwrap().volume
}

fn reason(spec: &Spec, tol: &Tolerance) -> (Option<usize>, String) {
    match build(spec, tol) {
        Ok(_) => panic!("built"),
        Err(KernelError::SheetMetal { flange, reason }) => (flange, reason),
        Err(other) => panic!("not a sheet-metal refusal: {other}"),
    }
}

/// The notch's area for a relief of size `r` at a square corner.
fn notch_area(kind: ReliefKind, r: f64) -> f64 {
    match kind {
        ReliefKind::Rect => r * r,
        ReliefKind::Round => PI * r * r / 4.0,
        ReliefKind::Tear => r * r + 3.0 * PI * r * r / 4.0,
    }
}

/// The solid passes the checker; fold then unfold: every folded flat's
/// vertices land back on a corner of its outline at z = 0; each bend's
/// neutral band develops to its strip's width.
fn healthy(part: &Part, tol: &Tolerance) {
    let defects = check(&part.solid.body, tol);
    assert!(
        defects.is_empty(),
        "the solid fails the checker: {defects:?}"
    );
    for (f, role) in part.sheet_faces.iter().enumerate() {
        let SheetFace::Flat(i) = role else { continue };
        let flat = &part.flats[*i];
        let back = flat.transform.inverse().unwrap();
        let face = part.sheet.face(FaceId(f as u32)).unwrap();
        for l in &face.loops {
            for c in &part.sheet.loop_(*l).unwrap().coedges {
                let v = part.sheet.coedge_ends(*c).unwrap().0;
                let (x, y, z) = part.plane.local(part.sheet.vertex(v).unwrap().point);
                let p = back.point(Vec3::new(x, y, z));
                assert!(p.z.abs() <= tol.linear, "off the plane by {}", p.z);
                let on = flat
                    .outer
                    .iter()
                    .chain(flat.holes.iter().flatten())
                    .map(|s| (s.start[0] - p.x).phypot(s.start[1] - p.y))
                    .fold(f64::INFINITY, f64::min);
                assert!(on <= tol.linear, "a vertex lands {on} from its corner");
            }
        }
    }
    for b in &part.bends {
        near(
            developed_allowance(b, tol).unwrap(),
            b.allowance,
            4.0 * tol.linear,
            "the developed bend allowance",
        );
    }
}

fn names(part: &Part) -> BTreeSet<Vec<u64>> {
    part.edge_keys
        .iter()
        .map(|k| edge_numbers(part, *k).unwrap())
        .collect()
}

#[test]
fn a_bend_relief_is_cut_into_a_closed_corners_extension() {
    for k in SCALES {
        let tol = tolerance(k);
        let (r, s) = (k, k);
        let plain = build(&tray(k, 10.0 * k, r, 3.0 * k, [0.0, 0.0], None), &tol).unwrap();
        healthy(&plain, &tol);
        // Both ends of the return's bend lie inside the extended tip,
        // without a relief: the return spans the tip, 60 − 2 · 3 long.
        let ends: Vec<_> = plain.bend_ends.iter().filter(|e| e.bend == 4).collect();
        assert_eq!(ends.len(), 2, "scale {k}");
        assert!(ends.iter().all(|e| e.relief.is_none()));
        near(ends[0].point[0], 3.0 * k, 1e-9 * k, "the return's start");
        near(
            ends[1].point[0],
            57.0 * k,
            1e-9 * k * 60.0,
            "the return's end",
        );
        let a0 = flat_area(&plain);
        let names0 = names(&plain);
        for kind in [ReliefKind::Rect, ReliefKind::Round, ReliefKind::Tear] {
            let relief = Relief { kind, size: s };
            let part = build(
                &tray(k, 10.0 * k, r, 3.0 * k, [0.0, 0.0], Some(relief)),
                &tol,
            )
            .unwrap_or_else(|e| panic!("scale {k} {kind:?}: {e}"));
            healthy(&part, &tol);
            let a = flat_area(&part);
            near(
                a0 - a,
                2.0 * notch_area(kind, s),
                1e-9 * a0,
                &format!("scale {k} {kind:?}: the notches' area"),
            );
            // At K = ½ with square walls the volume is the flat area
            // times T.
            let v = volume(&part);
            near(
                v,
                a * k,
                1e-8 * v,
                &format!("scale {k} {kind:?}: the volume"),
            );
            let ends: Vec<_> = part.bend_ends.iter().filter(|e| e.bend == 4).collect();
            assert_eq!(ends.len(), 2, "scale {k} {kind:?}");
            assert!(ends.iter().all(|e| e.relief == Some(relief)));
            // The notch's corners in the bottom wall's flat: `p ∓ s x̂`
            // along the tip and `p − s ŷ` into the wall (its tip runs
            // along x at y = −(BA + 10)).
            let flat = &part.flats[1];
            let tip_y = ends[0].point[1];
            for (e, along) in [(ends[0], -1.0), (ends[1], 1.0)] {
                for corner in [[e.point[0] + along * s, tip_y], [e.point[0], tip_y + s]] {
                    let on = flat
                        .outer
                        .iter()
                        .map(|g| (g.start[0] - corner[0]).phypot(g.start[1] - corner[1]))
                        .fold(f64::INFINITY, f64::min);
                    assert!(
                        on <= tol.linear,
                        "scale {k} {kind:?}: no corner at {corner:?}"
                    );
                }
            }
            // No name of the part without the relief is lost; the notches
            // add only `edge(11, 4, end, j)`.
            let with = names(&part);
            assert!(
                names0.is_subset(&with),
                "scale {k} {kind:?}: lost {:?}",
                names0.difference(&with).collect::<Vec<_>>()
            );
            for added in with.difference(&names0) {
                assert_eq!(&added[..2], &[11, 4], "scale {k} {kind:?}: {added:?}");
            }
            // The same part twice is the same.
            let again = build(
                &tray(k, 10.0 * k, r, 3.0 * k, [0.0, 0.0], Some(relief)),
                &tol,
            )
            .unwrap();
            assert_eq!(flat_area(&again).to_bits(), a.to_bits());
            assert_eq!(names(&again), with);
        }
    }
}

#[test]
fn a_bend_relief_longer_than_the_extension_is_refused() {
    for k in SCALES {
        let tol = tolerance(k);
        // The extension runs on from x = 3 to about the covering wall's
        // inside face, less than 4: a relief of 5 leaves nothing of it.
        let relief = Relief {
            kind: ReliefKind::Rect,
            size: 5.0 * k,
        };
        let (flange, why) = reason(
            &tray(k, 10.0 * k, k, 3.0 * k, [0.0, 0.0], Some(relief)),
            &tol,
        );
        assert_eq!(flange, Some(4), "scale {k}: {why}");
        assert!(
            why.contains("of the extension beside it"),
            "scale {k}: {why}"
        );
    }
}

#[test]
fn a_tear_bend_relief_that_runs_out_of_its_flat_is_refused() {
    for k in SCALES {
        let tol = tolerance(k);
        // A short wall: flat 3 long beside a radius of 4, the return inset
        // 20 from both ends of the tip. A tear of 2 reaches 4 into the
        // wall, past its tangent line; round and rect reach 2 and fit.
        let short = |kind| {
            let mut spec = tray(
                k,
                3.0 * k,
                4.0 * k,
                6.0 * k,
                [20.0 * k, 20.0 * k],
                Some(Relief {
                    kind,
                    size: 2.0 * k,
                }),
            );
            spec.corner = Closing::OPEN;
            spec
        };
        let (flange, why) = reason(&short(ReliefKind::Tear), &tol);
        assert_eq!(flange, Some(4), "scale {k}: {why}");
        assert!(
            why.contains("cuts across another edge of flange 0's flat")
                && why.contains("tear relief's circle"),
            "scale {k}: {why}"
        );
        for kind in [ReliefKind::Round, ReliefKind::Rect] {
            let part = build(&short(kind), &tol).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
            healthy(&part, &tol);
        }
        // Beside a closed corner: the return spans the tip, so its notch
        // is cut into the extension, which runs on less than 4 past it.
        // A tear of 1.5 reaches 3 along it (fits), of 1.9 reaches 3.8,
        // past the extension's end.
        let tear = |size: f64| {
            tray(
                k,
                10.0 * k,
                k,
                3.0 * k,
                [0.0, 0.0],
                Some(Relief {
                    kind: ReliefKind::Tear,
                    size,
                }),
            )
        };
        let part = build(&tear(1.5 * k), &tol).unwrap();
        healthy(&part, &tol);
        let (flange, why) = reason(&tear(1.9 * k), &tol);
        assert_eq!(flange, Some(4), "scale {k}: {why}");
        assert!(
            why.contains("tear") && why.contains("of the extension beside it"),
            "scale {k}: {why}"
        );
    }
}

/// A mitred corner's end is slanted across the thickness: with square
/// walls bent up the extension runs on `5 − g/√2` past the return's end
/// on the bottom skin (the outer, to the plane through the outer skins'
/// meeting line moved `g/2`) and `T` less on the top skin. A relief that
/// fits the bottom skin's run-on but not the top's would leave the
/// notch's side through the end face (the plate cannot make it): refused
/// naming the return; one that fits both is built, its two notches off
/// the flat area.
#[test]
fn a_bend_relief_beside_a_mitred_end_fits_both_skins() {
    for k in SCALES {
        let tol = tolerance(k);
        let g = 0.5 * k;
        let mitred = |bend: Option<Relief>| {
            let mut spec = tray(k, 10.0 * k, k, 3.0 * k, [0.0, 0.0], bend);
            spec.corner = Closing {
                kind: CornerKind::Miter,
                gap: g,
                over: None,
            };
            spec
        };
        let plain = build(&mitred(None), &tol).unwrap();
        let a0 = flat_area(&plain);
        // Bottom run-on 4.646, top 3.646: a relief of 4 fits only the
        // bottom.
        let too_big = Relief {
            kind: ReliefKind::Rect,
            size: 4.0 * k,
        };
        let (flange, why) = reason(&mitred(Some(too_big)), &tol);
        assert_eq!(flange, Some(4), "scale {k}: {why}");
        assert!(why.contains("the least on either skin"), "scale {k}: {why}");
        let run_on: f64 = why
            .split("runs on past by ")
            .nth(1)
            .and_then(|r| r.split(' ').next())
            .unwrap()
            .parse()
            .unwrap();
        near(
            run_on,
            5.0 * k - g / 2.0_f64.sqrt() - k,
            1e-9 * k * 10.0,
            &format!("scale {k}: the top skin's run-on"),
        );
        let fits = Relief {
            kind: ReliefKind::Rect,
            size: 3.0 * k,
        };
        let part = build(&mitred(Some(fits)), &tol).unwrap();
        healthy(&part, &tol);
        near(
            a0 - flat_area(&part),
            2.0 * notch_area(ReliefKind::Rect, 3.0 * k),
            1e-9 * a0,
            &format!("scale {k}: the notches' area"),
        );
    }
}

/// An extension whose end slants across the flat: the left wall (flange 3)
/// bent 60 degrees covers the corner at the origin, so the bottom wall's
/// extension runs to that wall's inside face, a plane that meets the
/// bottom wall's flat along a line not square to its tip. The run-on past
/// the return's start is linear in the depth `d` into the wall, `f(d) =
/// f₀ + σ d` (read off the plain part's end line). A rect notch fits when
/// `min(f₀, f₀ + σ r) − r` keeps a sliver: one that fits is built, its notch
/// off the flat area (the corner at the return's end left open); where the slant takes the end line nearer
/// at depth (σ < 0), a notch that fits the tip line (`r < f₀`) but not its
/// own depth is refused, naming the return, before any body is made.
/// The slant is the closed form `σ = −tan 30°`.
#[test]
fn a_bend_relief_beside_an_end_that_slants_across_the_flat() {
    for k in SCALES {
        let tol = tolerance(k);
        let slanted = |bend: Option<Relief>| {
            let mut spec = tray(k, 10.0 * k, k, 3.0 * k, [0.0, 0.0], bend);
            spec.flanges[3].angle = PI / 3.0;
            // The corner at the return's end open: its bend reaches the
            // tip's end there, so only the slanted end is looked at.
            spec.corners = vec![CornerSpec {
                flanges: [0, 1],
                closing: Closing::OPEN,
            }];
            spec
        };
        let plain = build(&slanted(None), &tol).unwrap();
        let a0 = flat_area(&plain);
        let start = plain
            .bend_ends
            .iter()
            .find(|e| e.bend == 4 && e.end == 0)
            .unwrap()
            .point;
        // The bottom wall's extension end at its start: the segment the
        // first `End` names, from the tangent line's side to the tip's.
        let wall = &plain.flats[1];
        let end = wall
            .ends
            .iter()
            .map(|e| &wall.outer[e.segment])
            .find(|g| g.start[0] < 0.5 * 60.0 * k)
            .unwrap();
        let x_at = |y: f64| {
            let (a, b) = (end.start, end.end);
            a[0] + (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1])
        };
        // Into the wall from its tip is +y (the tip lies at the lowest y).
        let f = |d: f64| start[0] - x_at(start[1] + d);
        let (f0, sigma) = (f(0.0), f(k) - f(0.0));
        let sigma = sigma / k;
        // The covering wall leans out 30 degrees from upright, so its
        // inside face comes nearer by tan 30° per unit of depth.
        near(
            sigma,
            -(PI / 6.0).ptan(),
            1e-9,
            &format!("scale {k}: the end's slant"),
        );
        // A rect relief that fits with a margin of a tenth of the run-on.
        let fits = 0.9 * f0.min(f0 / (1.0 - sigma.min(0.0)));
        let part = build(
            &slanted(Some(Relief {
                kind: ReliefKind::Rect,
                size: fits,
            })),
            &tol,
        )
        .unwrap_or_else(|e| panic!("scale {k}: {e}"));
        healthy(&part, &tol);
        near(
            a0 - flat_area(&part),
            fits * fits,
            1e-9 * a0,
            &format!("scale {k}: the notches' area"),
        );
        {
            // Fits the tip line, not its depth.
            let r = 0.5 * (f0 + f0 / (1.0 - sigma));
            let (flange, why) = reason(
                &slanted(Some(Relief {
                    kind: ReliefKind::Rect,
                    size: r,
                })),
                &tol,
            );
            assert_eq!(flange, Some(4), "scale {k}: {why}");
            assert!(
                why.contains("cuts across") || why.contains("of the extension beside it"),
                "scale {k}: {why}"
            );
        }
    }
}
