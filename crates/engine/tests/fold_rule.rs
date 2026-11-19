//! The rule of a face folded over itself (`craft3d_topo::folds`;
//! docs/design/verification.md › *A face folded over itself*), as built
//! for its round as a warning: what it shows (the review's seven folded
//! fades, the fitted top pulled through itself at three sizes, the loft
//! through its axis), what it passes (the notches at three scales, the
//! fades the kernel builds on a concave edge, the ship propeller, the
//! overhang, the hull), that `topo::check` refuses none of them while
//! `FOLDS_REFUSED` is off and that `document.check` says the place; the
//! same answers cold and warm and from several threads; never a panic.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
mod fold_common;

use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, EdgeId, Fold, FoldKind, check, folds, forget_folds};
use fold_common::{narrow, notch_bodies, read_step, right_angle, roof_block, script, tolerance};
use serde_json::json;
use std::path::PathBuf;

const SEVEN: [&str; 7] = [
    "fade-3-6-0.33",
    "fade-3-6-0.5",
    "fade-4-8-0.33",
    "fade-4-8-0.5",
    "fade-5-10-1",
    "fade-3-3-2",
    "fade-4-8-1",
];

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fold_probe")
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn shown(found: &[Fold]) -> Vec<&Fold> {
    found.iter().filter(|f| f.is_fold()).collect()
}

/// The loft of two circles, radius 1 and 1.2 times `k`, the second
/// started half a turn round: every ruling through one point of the axis.
fn loft_through_its_axis(k: f64) -> Body {
    use craft3d_kernel::loft::{Options, Section, loft};
    use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
    let arcs = |first: u64, r: f64| -> Vec<SketchCurve> {
        let points = [[r, 0.0], [0.0, r], [-r, 0.0], [0.0, -r]];
        (0..4)
            .map(|i| SketchCurve {
                id: first + i as u64,
                curve: Curve2::Arc {
                    center: [0.0, 0.0],
                    start: points[i],
                    end: points[(i + 1) % 4],
                },
            })
            .collect()
    };
    let tol = tolerance(1e-6 * k);
    let low = detect_profiles(&arcs(1, k), &tol).unwrap();
    let high = detect_profiles(&arcs(11, 1.2 * k), &tol).unwrap();
    let sections = [
        Section {
            plane: Frame::WORLD,
            outline: low[0].outer.clone(),
            start: None,
        },
        Section {
            plane: Frame::new(Vec3::new(0.0, 0.0, 2.0 * k), Vec3::Z, Vec3::X).unwrap(),
            outline: high[0].outer.clone(),
            start: Some(13),
        },
    ];
    loft(
        &sections,
        &Options {
            ruled: true,
            closed: false,
        },
        &tol,
    )
    .unwrap()
    .body
}

#[test]
fn the_rule_shows_the_folds_and_check_refuses_none_of_them_yet() {
    const { assert!(!craft3d_topo::FOLDS_REFUSED) };
    // Each of the seven shows two layers facing each other through air,
    // one line for the face, at the least gap.
    for name in SEVEN {
        let body = read_step(&fixtures().join(format!("fades/{name}.step")), 0.02).unwrap();
        forget_folds();
        let before = craft3d_topo::fold_work();
        let found = folds(&body, &tolerance(0.02));
        let after = craft3d_topo::fold_work();
        println!(
            "FOLDRULE {name}: {found:?} ({} pairs of pieces)",
            after.1 - before.1
        );
        assert!(
            found
                .iter()
                .any(|f| matches!(f.kind, FoldKind::Layers { gap } if gap < 0.02)),
            "{name}: {found:?}"
        );
        assert!(check(&body, &tolerance(0.02)).is_empty(), "{name}");
    }
    for times in [3.0, 100.0, 10_000.0] {
        let body = roof_block(times * 6.0, 1.0, true, 3);
        let found = folds(&body, &tolerance(1e-3));
        println!("FOLDRULE loop x{times}: {} places", found.len());
        assert!(
            found
                .iter()
                .any(|f| matches!(f.kind, FoldKind::Layers { .. }) && f.face.0 == 5),
            "loop x{times}: {found:?}"
        );
        assert!(check(&body, &tolerance(1e-3)).is_empty());
    }
    for k in [1.0, 1e4] {
        let body = loft_through_its_axis(k);
        let found = folds(&body, &tolerance(1e-6 * k));
        println!("FOLDRULE loft x{k}: {found:?}");
        assert!(
            found.iter().any(|f| f.kind == FoldKind::NoNormal),
            "loft x{k}: {found:?}"
        );
    }
}

#[test]
fn the_rule_passes_the_bodies_that_are_valid() {
    let mut bodies: Vec<(String, Body, f64)> = Vec::new();
    for e in [0.2, 0.05, 0.02] {
        for b in notch_bodies(&format!("notch-90-{e}"), 0.02, &right_angle(e), 1.0) {
            bodies.push((format!("notch 90° root {e}"), (*b).clone(), 0.02));
        }
    }
    for (k, t) in [(1e-3, 0.02e-3), (1e4, 0.02e4)] {
        for b in notch_bodies(&format!("notch-90-x{k}"), t, &right_angle(0.05), k) {
            bodies.push((format!("notch 90° x{k}"), (*b).clone(), t));
        }
    }
    for b in notch_bodies("notch-30", 0.02, &narrow(0.05), 1.0) {
        bodies.push(("notch 30°".into(), (*b).clone(), 0.02));
    }
    for times in [3.0, 100.0, 10_000.0] {
        bodies.push((
            format!("overhang x{times}"),
            roof_block(times * 6.0, 1.0, false, 3),
            1e-3,
        ));
    }
    // The fades the kernel builds on a concave edge (its own refusal on).
    let line =
        |a: [f64; 2], b: [f64; 2]| ("sketch.line", json!({"sketch": "L", "from": a, "to": b}));
    let ell = script(
        "fold-rule-ell",
        0.02,
        &[
            ("sketch.create", json!({"name": "L"})),
            line([0.0, 0.0], [40.0, 0.0]),
            line([40.0, 0.0], [40.0, 20.0]),
            line([40.0, 20.0], [20.0, 20.0]),
            line([20.0, 20.0], [20.0, 40.0]),
            line([20.0, 40.0], [0.0, 40.0]),
            line([0.0, 40.0], [0.0, 0.0]),
            (
                "feature.extrude",
                json!({"sketch": "L", "distance": 30, "name": "B"}),
            ),
        ],
    );
    let ell = ell.first().unwrap();
    let edge = (0..ell.edges.len())
        .map(|i| EdgeId(i as u32))
        .find(|e| {
            let edge = ell.edge(*e).unwrap();
            let (a, b) = (
                ell.vertex(edge.start).unwrap().point,
                ell.vertex(edge.end).unwrap().point,
            );
            let on = |p: Vec3| (p.x - 20.0).abs() < 1e-6 && (p.y - 20.0).abs() < 1e-6;
            on(a) && on(b) && (a.z - b.z).abs() > 1.0
        })
        .unwrap();
    for (radius, over, back) in [
        (3.0, 6.0, 0.33),
        (3.0, 6.0, 0.0),
        (2.0, 10.0, 0.5),
        (3.0, 3.0, 2.0),
        (3.0, 3.0, 0.0),
        (5.0, 5.0, 0.5),
    ] {
        use craft3d_kernel::fillet::{Fade, FilletLawEdge, RadiusLaw, fillet_by_law};
        let law = RadiusLaw {
            radius,
            fades: [Some(Fade::tip(over, back)), None],
        };
        if let Ok(made) = fillet_by_law(ell, &[FilletLawEdge { edge, law }], &tolerance(0.02)) {
            bodies.push((
                format!("concave fade {radius}/{over}/{back}"),
                made.body,
                0.02,
            ));
        }
    }
    if let Some(hull) = read_step(&fixtures().join("trimmed.step"), 0.02) {
        bodies.push(("the hull".into(), hull, 0.02));
    }
    for (name, file) in [
        (
            "the ship propeller",
            "docs/examples/published/ship-propeller.3dcraft",
        ),
        (
            "the hull fixture",
            "crates/engine/tests/fillet_fade/hull.3dcraft",
        ),
    ] {
        let doc = craft3d_format::from_bytes(&std::fs::read(root().join(file)).unwrap()).unwrap();
        let regen = craft3d_regen::regenerate(&doc);
        for (_, b) in regen.bodies() {
            bodies.push((name.to_string(), (**b).clone(), doc.tolerance.linear));
        }
    }
    assert!(bodies.len() >= 20, "{} bodies", bodies.len());
    for (name, body, t) in &bodies {
        let found = folds(body, &tolerance(*t));
        println!(
            "FOLDRULE {name}: {} places, {} folds",
            found.len(),
            shown(&found).len()
        );
        assert!(found.is_empty(), "{name}: {found:?}");
    }
}

#[test]
fn document_check_says_where_a_face_folds_and_nothing_fails() {
    let dir = common::workdir("fold-rule", "check");
    std::fs::copy(
        fixtures().join("fades/fade-4-8-0.5.step"),
        dir.join("fold.step"),
    )
    .unwrap();
    let mut s = common::session_in(&dir);
    s.execute("file.new", &json!({"units": "mm", "tolerance": 0.02}))
        .unwrap();
    let imported = s
        .execute("file.import", &json!({"path": "fold.step"}))
        .unwrap();
    assert_ne!(imported["status"], "failed", "{imported}");
    let checked = s.execute("document.check", &json!({})).unwrap();
    println!("FOLDRULE document.check: {checked}");
    assert_eq!(checked["ok"], true, "{checked}");
    let warnings = checked["warnings"].as_array().unwrap();
    let fold = warnings
        .iter()
        .find(|w| w["kind"] == "fold")
        .unwrap_or_else(|| panic!("{checked}"));
    let text = fold["warning"].as_str().unwrap();
    assert!(
        text.contains("folds over itself at (")
            && text.contains("two of its layers face each other")
            && text.contains("within the tolerance 2e-2"),
        "{text}"
    );
}

#[test]
fn the_same_answers_cold_warm_and_from_several_threads() {
    let body = read_step(&fixtures().join("fades/fade-3-6-0.5.step"), 0.02).unwrap();
    let tol = tolerance(0.02);
    forget_folds();
    let cold = folds(&body, &tol);
    let warm = folds(&body, &tol);
    assert_eq!(cold, warm);
    assert!(forget_folds() > 0);
    let threads: Vec<Vec<Fold>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4).map(|_| scope.spawn(|| folds(&body, &tol))).collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for found in threads {
        assert_eq!(found, cold);
    }
    // Another document's tolerance is another answer, not a remembered one.
    let coarser = folds(&body, &tolerance(0.04));
    assert!(coarser.iter().all(|f| f.tolerance == 0.04));
}

#[test]
fn hostile_faces_are_answered_without_a_panic() {
    use craft3d_geom::{NurbsSurface, Surface};
    // An empty body and a body whose face has no loops.
    assert!(folds(&Body::new(craft3d_topo::BodyKind::Solid), &tolerance(1e-3)).is_empty());
    let mut lonely = roof_block(18.0, 1.0, true, 3);
    for face in &mut lonely.faces {
        face.loops.clear();
    }
    let _ = folds(&lonely, &tolerance(1e-3));
    // A face of degree 16 each way: the forms are not built for it; the
    // rule says it could not prove it, nothing more.
    let n = 17;
    let mut net = Vec::new();
    for i in 0..n {
        for j in 0..n {
            net.push(Vec3::new(
                i as f64,
                j as f64,
                ((i * 7 + j * 3) % 5) as f64 * 0.1,
            ));
        }
    }
    let knots = |d: usize| {
        let mut k = vec![0.0; d + 1];
        k.extend(vec![1.0; d + 1]);
        k
    };
    let mut high = roof_block(18.0, 1.0, false, 3);
    if let Some(face) = high.faces.get_mut(5) {
        face.surface = Surface::Nurbs(
            NurbsSurface::new(16, 16, knots(16), knots(16), n, n, net, vec![1.0; n * n]).unwrap(),
        );
    }
    let found = folds(&high, &tolerance(1e-3));
    println!("FOLDRULE degree 16: {found:?}");
    assert!(found.iter().all(|f| !f.is_fold()));
    // Tolerances that are no tolerance.
    for t in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let tol = craft3d_math::Tolerance {
            linear: t,
            angular: 1e-9,
        };
        assert!(folds(&high, &tol).is_empty());
    }
}
