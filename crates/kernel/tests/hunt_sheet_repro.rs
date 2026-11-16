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
use craft3d_math::Portable;
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

// The rays' finding (case 123 of the seeded plates) was repaired on
// 2026-11-14 and is held in `hunt_sheet_plates.rs`, where the case is
// built bit for bit, and in `classify_far.rs`.

/// FIXED (was refused, the plate's trace): a bend **down** whose inner
/// radius is below about a fiftieth of the thickness. The bend's arcs on
/// the folded bottom skin (the inner skin of a bend down) have the radius
/// `R`, and the plate traced every arc twice the thickness past its ends,
/// `2T/R` radians each way, hundreds of turns here, which past 4096
/// points was refused: "the rim could not be traced within a quarter of
/// the tolerance". A run on a circle is now kept within a turn
/// (`docs/design/plates.md` › *Corners*), and the part is its closed form
/// as the same bend up (its bottom skin is the outer one, radius `R + T`)
/// and the same bend down with `R = 0` (a sharp bend, joined in apart)
/// were.
#[test]
fn a_bend_down_tighter_than_a_fiftieth_of_the_thickness_is_its_closed_form() {
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
        for (dir, rr) in [
            (Direction::Up, r),
            (Direction::Down, 0.0),
            (Direction::Down, r),
        ] {
            let p = part(dir, rr).unwrap_or_else(|e| panic!("{k} {dir:?} {rr}: {e}"));
            let b = &p.solid.body;
            assert!(check(b, &tol).is_empty());
            let got = mass_properties(b).unwrap().volume;
            assert!(
                (got / v(rr) - 1.0).abs() < 1e-9,
                "{k} {dir:?}: {got} for {}",
                v(rr)
            );
            assert!(mesh_volume(b, 1e-3 * size_of(b)).is_ok(), "{k} {dir:?}");
        }
    }
}

/// FIXED (was an invalid body, the plate in a sheet-metal part): a base
/// 50 × 40, 1 thick, with a flange 20 long on its right edge bent
/// **down** 140 degrees about the inner radius 0.1762914118095948 (K 0.4)
/// was a body that `topo::check` passed, whose volume measured 3213.85
/// where the closed form `2000 + 40 (θ (R + ½) + 20)` is 2866.10 (12 %
/// over), and which could not be meshed: its edges along the bend's outer
/// arc at both ends (17, 19) were splines over a parameter range of 15,
/// round the arc's circle 2.39 times, where every neighbour's were
/// circles over the bend's 2.44 rad. The bottom skin of a bend down has
/// the inner radius `R`, whose arcs were traced `2T/R` past each end; here
/// the run, `θ + 4T/R`, was 8π (four turns): the cause of
/// `a_hole_whose_rims_ran_an_even_number_of_turns_is_its_closed_form`. The
/// run is kept within a turn now (`docs/design/plates.md` › *Corners*).
/// Held here at that radius, a part in 1e12 and in 1e3 either side, and
/// at 90 degrees about the radius of two turns there, `4T / (3.5 π)`,
/// which was refused over a like band: each the closed form to 1e-10, its
/// mesh closed, edge 17 a circle.
#[test]
fn a_bend_down_at_one_radius_is_its_closed_form() {
    use craft3d_geom::Curve3;
    let tol = tolerance(1.0);
    for (th, r0) in [
        (140f64.to_radians(), 0.1762914118095948),
        (FRAC_PI_2, 4.0 / (3.5 * PI)),
    ] {
        let make = |r: f64| {
            build(
                &rect_spec(
                    50.0,
                    40.0,
                    1.0,
                    1.0,
                    vec![flange(2, th, Direction::Down, r, 0.4, 20.0)],
                ),
                &tol,
            )
            .unwrap_or_else(|e| panic!("{th} {r}: {e}"))
        };
        let want = |r: f64| 2000.0 + 40.0 * (th * (r + 0.5) + 20.0);
        for f in [1.0 - 1e-3, 1.0 - 1e-12, 1.0, 1.0 + 1e-12, 1.0 + 1e-3] {
            let r = r0 * f;
            let p = make(r);
            let b = &p.solid.body;
            assert!(check(b, &tol).is_empty(), "{th} {r}");
            let v = mass_properties(b).unwrap().volume;
            assert!((v / want(r) - 1.0).abs() < 1e-10, "{th} {r}: {v}");
            assert!(mesh_volume(b, 1e-3 * size_of(b)).is_ok(), "{th} {r}");
            assert!(matches!(b.edges[17].curve, Curve3::Circle(_)), "{th} {r}");
        }
    }
}

/// FIXED (was an invalid body or refused, the plate): a plate 1 thick of
/// a square 20 × 20 with a round hole drawn as two half circles of radius
/// `r = 4t/(7π)` (0.18189136353359467) was a body that `topo::check`
/// passed but that could not be measured or meshed ("face … cannot be
/// tessellated: its loops do not bound a region"); at `r = 4t/(3π)`
/// (0.4244131815783876) the plate was refused; a billionth larger or
/// smaller, both built right. Each arc's rim was traced past both ends by
/// twice the thickness, `2t/r` radians, so the traced run was the arc's
/// sweep plus `4t/r`; at these radii an even number of whole turns (4 and
/// 2), whose first and last points meet, and `plate::simplest` took the
/// run as closed and fitted the circle through its first point, the point
/// a quarter along and the middle one by index, which then lie on top of
/// each other; the circle was not found and each hole edge was cut from
/// the spline through the whole run, round its circle 2.5 times. Every
/// station along a circle is a function of the circle's point, so a run
/// past a turn traced its own points again: it is kept within a turn now
/// (`docs/design/plates.md` › *Corners*). Held at both radii and a
/// billionth either side: the closed form `(400 − π r²) t` to 1e-12, its
/// mesh closed, the hole's edges circles.
#[test]
fn a_hole_whose_rims_ran_an_even_number_of_turns_is_its_closed_form() {
    use craft3d_kernel::extrude::fill;
    use craft3d_kernel::plate::{Side, plate};
    use std::collections::BTreeMap;
    let tol = tolerance(1.0);
    let plated = |r: f64| {
        let hole = reversed(&split_circle([0.0, 0.0], r, &[0.3, 0.3 + PI]));
        let shape = Shape {
            what: String::new(),
            outer: polygon_loop(&[[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0]]),
            holes: vec![hole],
            inner: ([5.0, 5.0], 3.0),
            hole_discs: vec![([0.0, 0.0], r)],
            feature: r,
        };
        let placing = Placing {
            turn: 0.0,
            k: 1.0,
            at: [0.0, 0.0],
            flip_lines: false,
        };
        let curves = sketch(&shape, &placing, 1);
        let profiles = detect_profiles(&curves, &tol).unwrap();
        let sheet = fill(&profiles, &Frame::WORLD, &tol).unwrap().body;
        (
            plate(&sheet, &[FaceId(0)], 1.0, Side::Out, &BTreeMap::new(), &tol),
            shape.area(),
        )
    };
    for r0 in [4.0 / (7.0 * PI), 4.0 / (3.0 * PI)] {
        for r in [r0 * (1.0 - 1e-9), r0, r0 * (1.0 + 1e-9)] {
            let (made, want) = plated(r);
            let b = made.unwrap_or_else(|e| panic!("{r}: {e}")).body;
            assert!(check(&b, &tol).is_empty(), "{r}");
            let v = mass_properties(&b).unwrap().volume;
            assert!((v / want - 1.0).abs() < 1e-12, "{r}: {v} for {want}");
            assert!(mesh_volume(&b, 1e-3 * size_of(&b)).is_ok(), "{r}");
            let circles = b
                .edges
                .iter()
                .filter(|e| matches!(e.curve, craft3d_geom::Curve3::Circle(_)))
                .count();
            let splines = b
                .edges
                .iter()
                .filter(|e| matches!(e.curve, craft3d_geom::Curve3::Nurbs(_)))
                .count();
            assert_eq!((circles, splines), (4, 0), "{r}");
        }
    }
}

/// REFUSED by the regeneration, owed (a hole across a tight bend): a base
/// 25.867 × 74.256, 1.1204 thick, with a flange 15.663 long on its top
/// edge bent up 90 degrees about the inner radius 0.001 (K 0.5), and a
/// round hole of radius 1 drawn over the middle of the bend's strip, is
/// a body that `topo::check` passes and whose volume is the closed form
/// (the flats less the hole's part on them times `T`, the bend less the
/// hole's part on the strip times `T (R + T/2) / (R + K T)`), but that
/// cannot be meshed ("face … cannot be tessellated: its loops do not
/// bound a region"), so the regeneration's self-intersection check
/// refuses the part. The same hole with the radius 0.1, or a hole on the
/// base's tangent line with the radius 0.001, is meshed. Found by the
/// wide sweep of `hunt_sheet_metal.rs` (cases 3150 and 3348, at 1e6 and
/// 1; case 525, at 1e-3, could not be measured, though meshed). Not the
/// traced run of
/// `a_hole_whose_rims_ran_an_even_number_of_turns_is_its_closed_form`:
/// the hole's edges on the bend are splines, traced as before. The face
/// the tessellator refuses is the hole's rim on the bend, whose rulings
/// at the inner skin (radius `R`) come in from both sides of the hole at
/// one height, so that its two sides lie about `R √2` apart there; meshed
/// at a chord of 2e-6 of the part's size it is its closed form, at 1e-5
/// and coarser (the self-intersection check's chords) the tessellator
/// finds its boundary crossing itself in parameter space, where its
/// edges' points, projected onto it, make the rectangle `[0, 1]²` of its
/// parameters. Owed: the tessellator (and, in other such parts, the
/// measure) on a face whose sides lie closer than its chord. When it is
/// fixed this test fails: take it off.
#[test]
fn a_hole_across_a_tight_bend_cannot_be_meshed() {
    use craft3d_kernel::sheetmetal::Cutout;
    let tol = tolerance(1.0);
    let (w, d, t) = (25.866747247304552, 74.25551480120751, 1.120411010680168);
    let l = 15.663052181070924;
    let part = |r: f64| {
        let mut spec = rect_spec(
            w,
            d,
            t,
            1.0,
            vec![flange(3, FRAC_PI_2, Direction::Up, r, 0.5, l)],
        );
        let ba = FRAC_PI_2 * (r + 0.5 * t);
        spec.cutouts.push(Cutout {
            profile: detect_profiles(
                &[SketchCurve {
                    id: 100,
                    curve: Curve2::Circle {
                        center: [0.5 * w, d + 0.5 * ba],
                        radius: 1.0,
                    },
                }],
                &tol,
            )
            .unwrap()
            .remove(0),
            from: "hole".into(),
        });
        // The hole lies wholly on the strip when its diameter is less
        // than the strip's width; else its part beyond lies on the flats.
        let rho: f64 = 1.0;
        let g = |x: f64| {
            let x = x.clamp(-rho, rho);
            x * (rho * rho - x * x).max(0.0).sqrt() + rho * rho * (x / rho).pasin()
        };
        let band = g(0.5 * ba) - g(-0.5 * ba);
        let disc = PI * rho * rho;
        let want = t * (w * d + w * l - (disc - band)) + w * FRAC_PI_2 * t * (r + 0.5 * t)
            - band * t * (r + 0.5 * t) / (r + 0.5 * t);
        (build(&spec, &tol).unwrap(), want)
    };
    let (p, want) = part(0.1);
    let b = &p.solid.body;
    assert!(check(b, &tol).is_empty());
    assert!((mass_properties(b).unwrap().volume / want - 1.0).abs() < 1e-9);
    assert!(mesh_volume(b, 1e-3 * size_of(b)).is_ok());
    let (p, want) = part(1e-3);
    let b = &p.solid.body;
    assert!(check(b, &tol).is_empty(), "known: the checker passes it");
    let v = mass_properties(b).unwrap().volume;
    assert!(
        (v / want - 1.0).abs() < 1e-9,
        "its volume is right: {v} for {want}"
    );
    assert!(
        mesh_volume(b, 1e-3 * size_of(b)).is_err(),
        "known: it cannot be meshed"
    );
}

/// FIXED (was refused, the plate): a plate of a whole sphere or torus (a
/// face without boundary) is a hollow ball or ring: two closed shells, one
/// facing out of the material and a cavity inside it. The plate put both
/// skins in one shell and refused its own result ("plate produced an
/// invalid body: ... the faces of shell 0 fall into 2 pieces that share
/// no edge"); with no rim to join them each skin is a shell of its own now
/// (`docs/design/plates.md` › *The plate*). Held: the closed forms
/// `4/3 π ((r + t)³ − r³)` and `2π² R ((a + t)² − a²)` to 1e-12, the
/// checker, one lump with one cavity, the mesh closed and of the volume.
/// The side of a cylinder (a face closing on itself across its seam, with
/// two boundaries) is plated to its closed form, one shell, as before.
#[test]
fn plates_of_a_whole_sphere_and_torus_are_hollows() {
    use craft3d_kernel::lumps::{Lump, lumps};
    use craft3d_topo::ShellId;
    let k = 1.0;
    let tol = tolerance(k);
    let ball = primitives::sphere(&Frame::WORLD, 10.0).unwrap();
    let ring = primitives::torus(&Frame::WORLD, 20.0, 5.0).unwrap();
    let ball_volume = 4.0 / 3.0 * PI * (1331.0 - 1000.0);
    let ring_volume = 2.0 * PI * PI * 20.0 * (36.0 - 25.0);
    for (name, body, want) in [("sphere", ball, ball_volume), ("torus", ring, ring_volume)] {
        let face = body
            .faces
            .iter()
            .position(|f| f.surface.kind() == name)
            .unwrap();
        let made = plate(
            &body,
            &[FaceId(face as u32)],
            1.0,
            Side::Out,
            &BTreeMap::new(),
            &tol,
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        let b = &made.body;
        assert!(check(b, &tol).is_empty(), "{name}");
        assert_eq!(b.shells.len(), 2, "{name}");
        let got = mass_properties(b).unwrap().volume;
        assert!((got / want - 1.0).abs() < 1e-12, "{name}: {got} for {want}");
        assert_eq!(
            lumps(b, &tol).unwrap(),
            vec![Lump {
                outer: ShellId(0),
                cavities: vec![ShellId(1)]
            }],
            "{name}"
        );
        let mesh = mesh_volume(b, 0.05).unwrap();
        assert!((mesh / want - 1.0).abs() < 1e-2, "{name}: mesh {mesh}");
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
    assert_eq!(made.body.shells.len(), 1);
    let v = PI * 20.0 * (11.0 * 11.0 - 100.0);
    let got = mass_properties(&made.body).unwrap().volume;
    assert!((got / v - 1.0).abs() < 1e-9, "{got} for {v}");
}

/// FIXED (was refused, the unfold): the side of a cylinder of radius
/// 31.437 and height 12.025, times 1e6, whose axis is turned off the
/// world's axes, was refused by `develop` ("face 0 has a boundary that
/// does not close in the surface's parameters"), wherever it stood; the
/// same cylinder at scale 1 (its tolerance scaled with it), or with the
/// world's axes at 1e6, was developed. A loop's ends were held to close in
/// the parameters within `1e-9 (1 + |u| + |v|)`, and `v` of a cylinder is
/// a length along its axis: here the ends of its loops were apart in `v`
/// by the rounding of coordinates of 3e7 (about 1e-8), against a
/// tolerance of 1. A gap in the parameters is nothing now also where its
/// length on the part is within the tolerance
/// (`docs/design/developable.md` › *The region*). All three are
/// developed, each its area `2π r h` to 1e-7.
#[test]
fn a_turned_cylinder_unrolls_at_every_scale() {
    let x = Vec3::new(0.6897164377873604, 0.42849178879240324, -0.5836831523895482);
    let z = Vec3::new(
        -0.40679339275755244,
        -0.4375582154097686,
        -0.8019114313539981,
    );
    for (k, zz, xx) in [(1e6, z, x), (1.0, z, x), (1e6, Vec3::Z, Vec3::X)] {
        let f = Frame::new(Vec3::new(0.0, 0.0, 0.0), zz, xx).unwrap();
        let body = primitives::cylinder(&f, 31.43694550814996 * k, 12.025057614637177 * k).unwrap();
        let face = body
            .faces
            .iter()
            .position(|f| matches!(f.surface, Surface::Cylinder(_)))
            .unwrap();
        let d = develop(&body, &[FaceId(face as u32)], &tolerance(k))
            .unwrap_or_else(|e| panic!("{k}: {e}"));
        let flat = d.flat_body(&Frame::WORLD, &tolerance(k)).unwrap();
        let a = mass_properties(&flat.body).unwrap().area;
        let want = 2.0 * PI * 31.43694550814996 * 12.025057614637177 * k * k;
        assert!((a / want - 1.0).abs() < 1e-7, "{k}: {a} for {want}");
    }
}

/// FIXED (a representation, not a wrong body): a flange bent down by
/// 0.00324 degrees (5.65e-5 rad) about the inner radius 1.178 with the
/// thickness 2.811, times 1e6, had one freeform face among its planes and
/// cylinders, where the design says a sheet-metal part's ends are planes
/// (docs/design/sheetmetal.md › *The part*): the rim along one of the
/// bend's arcs, the ruled spline over the whole traced run (most of the
/// circle about the bend's axis, from radius `R` to `R + T`), so
/// `Body::bbox`, which adds a freeform face's whole control net, reached
/// 4.86e6 below the part, whose lowest point is 715 below the base. No
/// arc was written with a far control point: the net is the run's. The
/// rim's traced corners lie in the side plane to the rounding of their
/// coordinates (1.5e-8), but its splines' control points, a solve through
/// stations 3.5e-6 rad apart over the arc and 0.05 to 0.1 rad past it,
/// lay up to 0.38 off it (the tolerance 1, the share 0.25), and the plane
/// was read from the control points alone. It is read from the traced
/// corners too now (`docs/design/plates.md` › *Simplest curves and
/// surfaces*): the rim is the side plane, the part is planes and
/// cylinders, its box the sampled one, its volume the closed form as
/// before. Until the run on a circle was kept within a turn (2026-11-15)
/// the run went 1.52 turns and the box reached 5.17e6 below.
#[test]
fn a_bend_of_a_tiny_angle_is_planes_and_cylinders() {
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
    assert!(check(body, &tol).is_empty());
    let v = w * d * t + d * (th * t * (r + 0.5 * t) + l * t);
    let got = mass_properties(body).unwrap().volume;
    assert!((got / v - 1.0).abs() < 1e-8, "{got} for {v}");
    assert!(
        body.faces
            .iter()
            .all(|f| matches!(f.surface, Surface::Plane(_) | Surface::Cylinder(_))),
        "{:?}",
        body.faces
            .iter()
            .map(|f| f.surface.kind())
            .collect::<Vec<_>>()
    );
    let (lo, hi) = body.bbox().unwrap().corners().unwrap();
    let (slo, shi) = bounds(body);
    assert!(slo.z > -716.0 && slo.z < -714.0, "{slo:?}");
    for (a, b) in [(lo, slo), (hi, shi)] {
        assert!((a - b).length() <= tol.linear, "{a:?} against {b:?}");
    }
    assert!(mesh_volume(body, 1e-3 * size_of(body)).is_ok());
}

/// KNOWN, owed (docs/design/sheetmetal.md › *Limits*): a bend of
/// 0.00115 degrees down about the inner radius 0.004, 2.26 thick (the
/// metal hunt's seeded case 55, its other flanges left out): the bend's
/// inner arc `θ R` is 8e-8, shorter than the tolerance, so its inner skin
/// has no width, and the plate's own check refuses the part with "loop …
/// runs over itself", a true reason that names no bend. Until 2026-11-15
/// the plate's trace of the arcs refused it earlier ("could not be traced
/// … in 4096 points"). Owed: a refusal of such a bend by name. When it is
/// fixed this test fails: take it off.
#[test]
fn a_bend_whose_inner_arc_is_shorter_than_the_tolerance_is_refused_unnamed() {
    let tol = tolerance(1.0);
    let made = build(
        &rect_spec(
            27.839120860428434,
            54.6251641612266,
            2.257940608326146,
            1.0,
            vec![flange(
                3,
                0.001151984499f64.to_radians(),
                Direction::Down,
                3.961547451586655e-3,
                1.0,
                27.38840619926645,
            )],
        ),
        &tol,
    );
    let e = made.expect_err("known refused");
    assert!(
        e.to_string().contains("runs over itself"),
        "known: the plate's own check; now: {e}"
    );
}
