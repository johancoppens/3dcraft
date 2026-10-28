//! Thick panels on a membrane (docs/design/origami.md › *Thick panels*):
//! one prism per facet, set back from its creases by the clearance; two
//! panels beside a crease meet when it folds towards them by more than
//! 2 atan(c / T), away from them never below 180 degrees.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::origami::{
    Angle, Arrangement, BoundarySegment, Crease, CreaseKind, FoldError, PanelSide, Panels, arrange,
    fold, thick_panels,
};
use craft3d_math::{Frame, Tolerance, Vec3};

const ANGULAR: f64 = 1e-9;

fn polygon(points: &[[f64; 2]]) -> Vec<BoundarySegment> {
    let n = points.len();
    (0..n)
        .map(|i| BoundarySegment {
            id: i as u64 + 1,
            start: points[i],
            end: points[(i + 1) % n],
            reversed: false,
        })
        .collect()
}

fn crease(id: u64, start: [f64; 2], end: [f64; 2], kind: CreaseKind) -> Crease {
    Crease {
        id,
        start,
        end,
        kind,
    }
}

fn angles(arr: &Arrangement, given: &[(u64, f64)]) -> Vec<Angle> {
    arr.hinges
        .iter()
        .map(|h| {
            let c = &arr.creases[h.crease];
            match given.iter().find(|(id, _)| *id == c.id) {
                Some((_, a)) => Angle::Given(c.kind.sign() * a),
                None => Angle::Free {
                    sign: c.kind.sign(),
                },
            }
        })
        .collect()
}

fn frame() -> Frame {
    Frame::new(Vec3::ZERO, Vec3::Z, Vec3::X).unwrap()
}

#[test]
fn panels_beside_a_crease_meet_past_twice_the_angle_of_clearance_over_thickness() {
    for scale in [1e-3, 1.0, 1e6] {
        let s = 2.0 * scale;
        let tol = 1e-9 * scale;
        let tolerance = Tolerance::new(tol, ANGULAR).unwrap();
        let (t, c) = (0.1 * scale, 0.05 * scale);
        let limit = 2.0 * (c / t).atan();
        for (kind, side, towards) in [
            (CreaseKind::Mountain, PanelSide::Back, true),
            (CreaseKind::Valley, PanelSide::Back, false),
            (CreaseKind::Valley, PanelSide::Front, true),
            (CreaseKind::Mountain, PanelSide::Front, false),
        ] {
            let square = polygon(&[[0.0, 0.0], [s, 0.0], [s, s], [0.0, s]]);
            let arr = arrange(
                &square,
                &[crease(10, [s / 2.0, 0.0], [s / 2.0, s], kind)],
                tol,
            )
            .unwrap();
            let panels = Panels {
                thickness: t,
                clearance: c,
                side,
            };
            let at = |rho: f64| {
                let folded = fold(&arr, &angles(&arr, &[(10, rho)]), 0, tol, ANGULAR).unwrap();
                thick_panels(&arr, &folded, &frame(), &panels, &tolerance)
            };
            // Two prisms of (s/2 − c) × s × T.
            let made = at(limit - 0.5f64.to_radians()).unwrap();
            assert_eq!(made.body.shells.len(), 2);
            assert_eq!(made.body.faces.len(), 12);
            let volume = craft3d_topo::mass_properties(&made.body).unwrap().volume;
            let want = 2.0 * (s / 2.0 - c) * s * t;
            assert!((volume - want).abs() <= 1e-9 * want, "{volume} vs {want}");
            let past = at(limit + 0.5f64.to_radians());
            if towards {
                assert!(
                    matches!(past, Err(FoldError::PanelsMeet(ref m)) if m.facets == [0, 1]),
                    "{kind:?} {side:?}: {past:?}"
                );
            } else {
                assert!(past.is_ok(), "{kind:?} {side:?}: {past:?}");
                assert!(at(170f64.to_radians()).is_ok());
            }
        }
    }
}

#[test]
fn what_thick_panels_refuse() {
    let s = 2.0;
    let tol = 1e-9;
    let tolerance = Tolerance::new(tol, ANGULAR).unwrap();
    let square = polygon(&[[0.0, 0.0], [s, 0.0], [s, s], [0.0, s]]);
    let arr = arrange(
        &square,
        &[crease(10, [1.0, 0.0], [1.0, s], CreaseKind::Valley)],
        tol,
    )
    .unwrap();
    let folded = fold(&arr, &angles(&arr, &[(10, 0.3)]), 0, tol, ANGULAR).unwrap();
    let with = |thickness: f64, clearance: f64| {
        thick_panels(
            &arr,
            &folded,
            &frame(),
            &Panels {
                thickness,
                clearance,
                side: PanelSide::Back,
            },
            &tolerance,
        )
    };
    for (t, c) in [
        (0.0, 0.1),
        (-1.0, 0.1),
        (f64::NAN, 0.1),
        (0.1, 0.0),
        (0.1, f64::INFINITY),
    ] {
        assert!(
            matches!(with(t, c), Err(FoldError::Panels { .. })),
            "{t} {c}"
        );
    }
    // A clearance as wide as a facet.
    assert!(matches!(with(0.1, 1.5), Err(FoldError::Clearance { .. })));
    // A slit: no width for the panel there.
    let slit = arrange(
        &square,
        &[
            crease(10, [1.0, 0.0], [1.0, s], CreaseKind::Valley),
            crease(11, [0.5, 0.0], [0.5, 1.0], CreaseKind::Cut),
        ],
        tol,
    )
    .unwrap();
    let folded = fold(&slit, &angles(&slit, &[(10, 0.3)]), 0, tol, ANGULAR).unwrap();
    let e = thick_panels(
        &slit,
        &folded,
        &frame(),
        &Panels {
            thickness: 0.1,
            clearance: 0.05,
            side: PanelSide::Back,
        },
        &tolerance,
    );
    assert!(matches!(e, Err(FoldError::Clearance { .. })), "{e:?}");
}

/// A degree-4 vertex and a Miura sheet with thick panels: the panels
/// round each vertex are checked as solids; a Miura sheet folds while its
/// fastest crease (the zigzag, 2 atan(tan(ρ/2) / cos γ)) stays within
/// the clearance's angle and is refused past it.
#[test]
fn thick_panels_round_vertices() {
    use CreaseKind::{Mountain as M, Valley as V};
    let tol = 1e-9;
    let tolerance = Tolerance::new(tol, ANGULAR).unwrap();
    // Miura 4 × 4 (as in origami.rs).
    let (l, d, h): (f64, f64, f64) = (1.0, 0.5, 1.0);
    let (cols, rows) = (4usize, 4usize);
    let at = |i: usize, j: usize| [i as f64 * l + (j % 2) as f64 * d, j as f64 * h];
    let mut outline = Vec::new();
    for i in 0..cols {
        outline.push(at(i, 0));
    }
    for j in 0..rows {
        outline.push(at(cols, j));
    }
    for i in (1..=cols).rev() {
        outline.push(at(i, rows));
    }
    for j in (1..=rows).rev() {
        outline.push(at(0, j));
    }
    let mut creases = Vec::new();
    let mut id = 100;
    let zig = |i: usize| if i % 2 == 0 { 1.0 } else { -1.0 };
    let kind = |sign: f64| if sign > 0.0 { V } else { M };
    for i in 1..cols {
        for j in 0..rows {
            creases.push(crease(id, at(i, j), at(i, j + 1), kind(zig(i))));
            id += 1;
        }
    }
    for j in 1..rows {
        for i in 0..cols {
            let sign = if j % 2 == 0 { -zig(i) } else { zig(i) };
            creases.push(crease(id, at(i, j), at(i + 1, j), kind(sign)));
            id += 1;
        }
    }
    let arr = arrange(&polygon(&outline), &creases, tol).unwrap();
    let driver = creases
        .iter()
        .find(|c| c.start[1] > 0.0 && c.start[1] == c.end[1])
        .unwrap()
        .id;
    let gamma = (h / d).atan();
    let (t, c): (f64, f64) = (0.05, 0.02);
    let limit = 2.0 * (c / t).atan();
    let panels = Panels {
        thickness: t,
        clearance: c,
        side: PanelSide::Back,
    };
    let fastest = |rho: f64| 2.0 * ((rho / 2.0).tan() / gamma.cos()).atan();
    // Well within: folds, one shell per facet.
    let rho = 5f64.to_radians();
    assert!(fastest(rho) < limit);
    let folded = fold(&arr, &angles(&arr, &[(driver, rho)]), 0, tol, ANGULAR).unwrap();
    let made = thick_panels(&arr, &folded, &frame(), &panels, &tolerance).unwrap();
    assert_eq!(made.body.shells.len(), 16);
    // Every zigzag past the clearance's angle: refused.
    let rho = 50f64.to_radians();
    assert!(fastest(rho) > limit);
    let folded = fold(&arr, &angles(&arr, &[(driver, rho)]), 0, tol, ANGULAR).unwrap();
    let e = thick_panels(&arr, &folded, &frame(), &panels, &tolerance);
    assert!(matches!(e, Err(FoldError::PanelsMeet(_))), "{e:?}");
}
