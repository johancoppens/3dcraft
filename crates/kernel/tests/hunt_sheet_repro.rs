//! The findings of the plate and sheet-metal hunts (`hunt_sheet_*.rs`),
//! each reduced to its smallest case and pinned as it is today, so that a
//! fix has to change the pin (and take the case off the list). Each was
//! measured by two means at least before it was written down.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod hunt_sheet_common;

use craft3d_geom::Surface;
use craft3d_kernel::plate::{Side, plate};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{Closing, Direction, FlangeSpec, LengthFrom, On, Spec, build};
use craft3d_kernel::unfold::develop;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{FaceId, check, mass_properties};
use hunt_sheet_common::*;
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI};

fn line(id: u64, start: P2, end: P2) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

fn rect_spec(w: f64, d: f64, t: f64, k: f64, flanges: Vec<FlangeSpec>) -> Spec {
    let c = [[0.0, 0.0], [w, 0.0], [w, d], [0.0, d]];
    let curves: Vec<SketchCurve> = (0..4)
        .map(|i| line(i as u64 + 1, c[i], c[(i + 1) % 4]))
        .collect();
    let base = detect_profiles(&curves, &tolerance(k)).unwrap().remove(0);
    Spec {
        base,
        plane: Frame::WORLD,
        thickness: t,
        flanges,
        relief: None,
        bend_relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts: Vec::new(),
        jogs: Vec::new(),
        ids: Vec::new(),
    }
}

fn flange(on: u64, angle: f64, dir: Direction, r: f64, kk: f64, l: f64) -> FlangeSpec {
    FlangeSpec {
        on: On::Base(on),
        length: l,
        angle,
        direction: dir,
        radius: r,
        k: kk,
        from: LengthFrom::Tangent,
        inset: [0.0; 2],
        outline: None,
    }
}

// The rays' finding (case 123 of the seeded plates) is pinned in
// `hunt_sheet_plates.rs`, where the case is built bit for bit.

/// REFUSED, owed (the plate's trace): a bend **down** whose inner radius
/// is below about a fiftieth of the thickness. The bend's arcs on the
/// folded bottom skin (the inner skin of a bend down) have the radius `R`,
/// and the plate traces every arc twice the thickness past its ends,
/// `2T/R` radians each way (`docs/design/plates.md` › *Corners*), which
/// past 4096 points is refused: "the rim could not be traced within a
/// quarter of the tolerance". The same bend up (its bottom skin is the
/// outer one, radius `R + T`) and the same bend down with `R = 0` (a
/// sharp bend, joined in apart) build, so nothing real refuses it. The
/// sweep in `hunt_sheet_sizes.rs` puts the change at about `R = 0.02 T`
/// for a bend of 90 degrees.
#[test]
fn a_bend_down_tighter_than_a_fiftieth_of_the_thickness_is_refused() {
    for k in [1e-3, 1.0, 1e6] {
        let tol = tolerance(k);
        let t = 0.326 * k;
        let r = 1e-3 * k;
        let part = |dir: Direction, r: f64| {
            build(
                &rect_spec(
                    65.4 * k,
                    59.4 * k,
                    t,
                    k,
                    vec![flange(2, FRAC_PI_2, dir, r, 0.4, 24.9 * k)],
                ),
                &tol,
            )
        };
        let v = |r: f64| {
            65.4 * 59.4 * k * k * t + 59.4 * k * (FRAC_PI_2 * t * (r + 0.5 * t) + 24.9 * k * t)
        };
        for (dir, rr) in [(Direction::Up, r), (Direction::Down, 0.0)] {
            let p = part(dir, rr).unwrap_or_else(|e| panic!("{k} {dir:?} {rr}: {e}"));
            assert!(check(&p.solid.body, &tol).is_empty());
            let got = mass_properties(&p.solid.body).unwrap().volume;
            assert!(
                (got / v(rr) - 1.0).abs() < 1e-9,
                "{k} {dir:?}: {got} for {}",
                v(rr)
            );
        }
        let e = part(Direction::Down, r).expect_err("known refused: a bend down at R = T / 326");
        assert!(
            e.to_string().contains("could not be traced"),
            "{k}: refused for another reason: {e}"
        );
    }
}

/// REFUSED, owed (the plate): a plate of a whole sphere or torus (a face
/// without boundary) is a hollow ball or ring, two shells; the plate puts
/// both skins in one shell and refuses its own result ("plate produced an
/// invalid body: ... the faces of shell 0 fall into 2 pieces that share
/// no edge"). The side of a cylinder (a face closing on itself across its
/// seam, with two boundaries) is plated to its closed form.
#[test]
fn plates_of_a_whole_sphere_and_torus_are_refused() {
    let k = 1.0;
    let tol = tolerance(k);
    let ball = primitives::sphere(&Frame::WORLD, 10.0).unwrap();
    let ring = primitives::torus(&Frame::WORLD, 20.0, 5.0).unwrap();
    for (name, body) in [("sphere", ball), ("torus", ring)] {
        let face = body
            .faces
            .iter()
            .position(|f| f.surface.kind() == name)
            .unwrap();
        let e = plate(
            &body,
            &[FaceId(face as u32)],
            1.0,
            Side::Out,
            &BTreeMap::new(),
            &tol,
        )
        .expect_err("known refused");
        assert!(
            e.to_string().contains("plate produced an invalid body"),
            "{name}: {e}"
        );
    }
    let can = primitives::cylinder(&Frame::WORLD, 10.0, 20.0).unwrap();
    let face = can
        .faces
        .iter()
        .position(|f| matches!(f.surface, Surface::Cylinder(_)))
        .unwrap();
    let made = plate(
        &can,
        &[FaceId(face as u32)],
        1.0,
        Side::Out,
        &BTreeMap::new(),
        &tol,
    )
    .unwrap();
    let v = PI * 20.0 * (11.0 * 11.0 - 100.0);
    let got = mass_properties(&made.body).unwrap().volume;
    assert!((got / v - 1.0).abs() < 1e-9, "{got} for {v}");
}

/// REFUSED, owed (the unfold): the side of a cylinder of radius 31.437
/// and height 12.025, times 1e6, whose axis is turned off the world's
/// axes, is refused by `develop` ("face 0 has a boundary that does not
/// close in the surface's parameters"), wherever it stands; the same
/// cylinder at scale 1 (its tolerance scaled with it), or with the
/// world's axes at 1e6, is developed. A refusal that changes with the
/// scale alone.
#[test]
fn a_turned_cylinder_unrolls_at_one_scale_and_not_at_another() {
    let x = Vec3::new(0.6897164377873604, 0.42849178879240324, -0.5836831523895482);
    let z = Vec3::new(
        -0.40679339275755244,
        -0.4375582154097686,
        -0.8019114313539981,
    );
    let mut seen = Vec::new();
    for (k, zz, xx) in [(1e6, z, x), (1.0, z, x), (1e6, Vec3::Z, Vec3::X)] {
        let f = Frame::new(Vec3::new(0.0, 0.0, 0.0), zz, xx).unwrap();
        let body = primitives::cylinder(&f, 31.43694550814996 * k, 12.025057614637177 * k).unwrap();
        let face = body
            .faces
            .iter()
            .position(|f| matches!(f.surface, Surface::Cylinder(_)))
            .unwrap();
        let made = develop(&body, &[FaceId(face as u32)], &tolerance(k));
        if let Ok(d) = &made {
            let flat = d.flat_body(&Frame::WORLD, &tolerance(k)).unwrap();
            let a = mass_properties(&flat.body).unwrap().area;
            let want = 2.0 * PI * 31.43694550814996 * 12.025057614637177 * k * k;
            assert!((a / want - 1.0).abs() < 1e-7, "{k}: {a} for {want}");
        }
        seen.push(made.is_ok());
    }
    assert_eq!(
        seen,
        vec![false, true, true],
        "known refused at 1e6 with turned axes only"
    );
}

/// A representation, not a wrong body: a flange bent down by 0.00324
/// degrees (5.65e-5 rad) about the inner radius 1.178 with the thickness
/// 2.811, times 1e6, has one freeform face (a rim) among its planes and
/// cylinders, where the design says a sheet-metal part's ends are planes
/// (docs/design/sheetmetal.md › *The part*); so `Body::bbox`, which adds a
/// freeform face's whole control net, reaches 5.17e6 below the part,
/// whose lowest point is 715 below the base. The volume is the closed
/// form.
#[test]
fn a_bend_of_a_tiny_angle_has_a_freeform_rim() {
    let k = 1e6;
    let tol = tolerance(k);
    let (w, d, t) = (
        5.2593701507052034e7,
        7.373017334004687e7,
        2.8106251915983027e6,
    );
    let (th, r, kk, l) = (
        0.003238984780f64.to_radians(),
        1.178231653133451e6,
        0.013811553625156625,
        1.2653619695385022e7,
    );
    let part = build(
        &rect_spec(w, d, t, k, vec![flange(2, th, Direction::Down, r, kk, l)]),
        &tol,
    )
    .unwrap();
    let body = &part.solid.body;
    let v = w * d * t + d * (th * t * (r + 0.5 * t) + l * t);
    let got = mass_properties(body).unwrap().volume;
    assert!((got / v - 1.0).abs() < 1e-8, "{got} for {v}");
    let freeform = body
        .faces
        .iter()
        .filter(|f| matches!(f.surface, Surface::Nurbs(_)))
        .count();
    assert_eq!(freeform, 1, "known: one freeform rim");
    let (lo, _) = body.bbox().unwrap().corners().unwrap();
    let (slo, _) = bounds(body);
    assert!(slo.z > -716.0 && slo.z < -714.0, "{slo:?}");
    assert!(lo.z < -5e6, "known: the box reaches {lo:?}");
}
