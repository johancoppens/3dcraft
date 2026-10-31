//! Solids of several lumps and with cavities (the boolean checklist's item
//! 5): results that fall apart, leave a void, nest a lump in a cavity, and
//! operands that already have several shells, analytic and freeform, at
//! three scales. Each result is held to a closed-form volume, its lumps
//! and cavities as `lumps` sorts them, Euler–Poincaré per shell (the genus
//! of each shell), a watertight mesh enclosing the volume, the boolean's
//! own check in full, and the identities of sets with the other two
//! operations: vol A = vol (A ∖ B) + vol (A ∩ B) and
//! vol (A ∪ B) = vol A + vol B − vol (A ∩ B).
//!
//! `lumps` itself is held to seeded mistakes: a cavity turned inside out,
//! a cavity outside every lump, a lump inside the material of another.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod boolean_common;

use boolean_common::*;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::lumps::{Lump, lumps};
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::solids;
use craft3d_topo::{Body, Defect, ShellId, check, merge, shell_genera};
use std::f64::consts::PI;

fn cube(k: f64, lo: f64, hi: f64) -> Body {
    boxed(k, [lo, lo, lo], [hi, hi, hi])
}

/// A block of 10 with a cavity of 4 in its middle.
fn hollow(k: f64) -> Body {
    solve(
        &cube(k, 0.0, 10.0),
        &cube(k, 3.0, 7.0),
        BooleanOp::Subtract,
        k,
    )
    .body
}

/// The spline cone of the test kit (a freeform surface with a pole at its
/// apex), apex at `apex`, along +y, times `k`.
fn spline_cone(k: f64, apex: [f64; 3], radius: f64, height: f64) -> Body {
    let frame = Frame::WORLD
        .at(Vec3::new(apex[0] * k, apex[1] * k, apex[2] * k))
        .unwrap();
    solids::spline_cone(&frame, radius * k, height * k)
        .unwrap()
        .body
}

/// What a result is: its volume at scale 1, and per lump the number of
/// cavities, sorted.
struct Expect {
    volume: f64,
    lumps: Vec<usize>,
    /// Genus of each shell, sorted.
    genera: Vec<u32>,
}

fn shape(found: &[Lump]) -> Vec<usize> {
    let mut cavities: Vec<usize> = found.iter().map(|l| l.cavities.len()).collect();
    cavities.sort_unstable();
    cavities
}

/// The lumps of `body`, which must be a valid solid that meshes closed
/// around its own volume, held to `expect`.
#[track_caller]
fn holds(body: &Body, expect: &Expect, k: f64, what: &str) {
    let tol = tolerance(k);
    assert_eq!(check(body, &tol), [], "{what}");
    let volume = mass(body).volume;
    let target = expect.volume * k * k * k;
    assert!(
        (volume - target).abs() <= 1e-7 * target.abs(),
        "{what}: volume {volume} against {target}"
    );
    let found = lumps(body, &tol).unwrap_or_else(|e| panic!("{what}: lumps: {e}"));
    assert_eq!(shape(&found), expect.lumps, "{what}: lumps and cavities");
    // Every shell is in exactly one lump, once.
    let mut every: Vec<u32> = found
        .iter()
        .flat_map(|l| std::iter::once(l.outer).chain(l.cavities.iter().copied()))
        .map(|s| s.0)
        .collect();
    every.sort_unstable();
    assert_eq!(
        every,
        (0..body.shells.len() as u32).collect::<Vec<_>>(),
        "{what}"
    );
    // The volume is the signed sum over the shells: each lump's outside
    // less its cavities, each of them measured alone.
    let summed: f64 = (0..body.shells.len())
        .map(|s| {
            let alone = craft3d_topo::shell_alone(body, ShellId(s as u32)).unwrap();
            mass(&alone).volume
        })
        .sum();
    assert!(
        (summed - volume).abs() <= 1e-9 * volume.abs(),
        "{what}: the shells sum to {summed}, the body is {volume}"
    );
    let mut genera = shell_genera(body).unwrap_or_else(|| panic!("{what}: shell genera"));
    genera.sort_unstable();
    assert_eq!(genera, expect.genera, "{what}: genus per shell");
    // Watertight, and the mesh encloses the volume.
    let chord = 2e-3 * k;
    let params = TessParams {
        chordal_tolerance: chord,
        ..TessParams::default()
    };
    let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
    let welded = mesh.weld().unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert!(welded.is_closed(), "{what}: the mesh is not watertight");
    let area = mass(body).area;
    assert!(
        (welded.volume() - volume).abs() <= area * chord,
        "{what}: mesh volume {} against {volume}",
        welded.volume()
    );
}

fn expect(volume: f64, lumps: &[usize], genera: &[u32]) -> Expect {
    Expect {
        volume,
        lumps: lumps.to_vec(),
        genera: genera.to_vec(),
    }
}

/// `a op b` held to `expect`, and the identities of sets with the other
/// two operations (where they are not empty).
#[track_caller]
fn case(a: &Body, b: &Body, op: BooleanOp, expect: &Expect, k: f64, what: &str) {
    let what = format!("{what} at scale {k}");
    let result = solve(a, b, op, k);
    holds(&result.body, expect, k, &what);
    let tol = tolerance(k);
    let volume = |op: BooleanOp| match boolean(a, b, op, &tol) {
        Ok(r) => {
            verified(a, b, op, &r.body, &tol, &what);
            mass(&r.body).volume
        }
        Err(KernelError::EmptyResult { .. }) => 0.0,
        Err(e) => panic!("{what}: {op:?}: {e}"),
    };
    let (va, vb) = (mass(a).volume, mass(b).volume);
    let (union, difference, common) = (
        volume(BooleanOp::Union),
        volume(BooleanOp::Subtract),
        volume(BooleanOp::Intersect),
    );
    let slack = 1e-9 * (va.abs() + vb.abs());
    assert!(
        (va - difference - common).abs() <= slack,
        "{what}: A = (A - B) + (A n B): {va} against {difference} + {common}"
    );
    assert!(
        (union - (va + vb - common)).abs() <= slack,
        "{what}: A u B = A + B - A n B: {union} against {va} + {vb} - {common}"
    );
}

#[test]
fn a_cut_through_the_middle_leaves_two_lumps() {
    for k in SCALES {
        let slab = boxed(k, [-1.0, -1.0, 4.0], [11.0, 11.0, 6.0]);
        case(
            &cube(k, 0.0, 10.0),
            &slab,
            BooleanOp::Subtract,
            &expect(800.0, &[0, 0], &[0, 0]),
            k,
            "block less a slab",
        );
        // Through a cylinder across its axis: two half-discs' prisms.
        let rod = rod(k, [0.0, 0.0, 0.0], 5.0, 10.0);
        let across = boxed(k, [-6.0, -1.0, -1.0], [6.0, 1.0, 11.0]);
        let strip = 2.0 * (24f64.sqrt() + 25.0 * (0.2f64).pasin());
        case(
            &rod,
            &across,
            BooleanOp::Subtract,
            &expect(10.0 * (25.0 * PI - strip), &[0, 0], &[0, 0]),
            k,
            "rod less a slab along it",
        );
    }
}

#[test]
fn a_body_strictly_inside_leaves_a_cavity() {
    for k in SCALES {
        case(
            &cube(k, 0.0, 10.0),
            &cube(k, 3.0, 7.0),
            BooleanOp::Subtract,
            &expect(936.0, &[1], &[0, 0]),
            k,
            "block less a block inside",
        );
        case(
            &cube(k, 0.0, 10.0),
            &ball(k, [5.0, 5.0, 5.0], 3.0),
            BooleanOp::Subtract,
            &expect(1000.0 - 36.0 * PI, &[1], &[0, 0]),
            k,
            "block less a ball inside",
        );
        // A void of genus one.
        case(
            &boxed(k, [0.0, 0.0, 0.0], [20.0, 20.0, 10.0]),
            &ring(k, [10.0, 10.0, 5.0], 5.0, 2.0),
            BooleanOp::Subtract,
            &expect(4000.0 - 2.0 * PI * PI * 5.0 * 4.0, &[1], &[0, 1]),
            k,
            "block less a ring inside",
        );
        // Shells of one face each, without vertices.
        case(
            &ball(k, [0.0, 0.0, 0.0], 5.0),
            &ball(k, [0.0, 0.0, 0.0], 3.0),
            BooleanOp::Subtract,
            &expect(4.0 / 3.0 * PI * (125.0 - 27.0), &[1], &[0, 0]),
            k,
            "ball less a ball inside",
        );
    }
}

#[test]
fn operands_with_a_cavity_keep_it_open_it_or_fill_it() {
    for k in SCALES {
        let hollow = hollow(k);
        // A rod that misses the cavity: it stays.
        case(
            &hollow,
            &rod(k, [1.5, 1.5, -5.0], 1.0, 20.0),
            BooleanOp::Subtract,
            &expect(936.0 - 10.0 * PI, &[1], &[0, 1]),
            k,
            "hollow less a rod beside the cavity",
        );
        // A rod from inside the cavity out through the top: it opens.
        case(
            &hollow,
            &rod(k, [5.0, 5.0, 5.0], 1.0, 6.0),
            BooleanOp::Subtract,
            &expect(936.0 - 3.0 * PI, &[0], &[0]),
            k,
            "hollow less a rod out of the cavity",
        );
        // A block in the cavity, apart from its walls: a lump in a void.
        case(
            &hollow,
            &cube(k, 4.0, 6.0),
            BooleanOp::Union,
            &expect(944.0, &[0, 1], &[0, 0, 0]),
            k,
            "hollow and a block in its cavity",
        );
        // Half of it: the cavity is cut open.
        case(
            &hollow,
            &boxed(k, [-1.0, -1.0, -1.0], [9.0, 9.0, 4.0]),
            BooleanOp::Intersect,
            &expect(308.0, &[0], &[0]),
            k,
            "hollow and a block over its lower part",
        );
        // Its middle: the rind around the cavity.
        case(
            &hollow,
            &cube(k, 2.0, 8.0),
            BooleanOp::Intersect,
            &expect(152.0, &[1], &[0, 0]),
            k,
            "hollow and a block around the cavity",
        );
        // As the tool: what is left of a larger block is in two lumps,
        // the part the cavity spared among them.
        case(
            &boxed(k, [-1.0, -1.0, -6.0], [11.0, 11.0, 6.0]),
            &hollow,
            BooleanOp::Subtract,
            &expect(1176.0, &[0, 0], &[0, 0]),
            k,
            "block less a hollow over its top",
        );
        // Two hollows overlapping: each fills part of the other's cavity.
        let other = solve(
            &boxed(k, [5.0, 0.0, 0.0], [15.0, 10.0, 10.0]),
            &boxed(k, [8.0, 3.0, 3.0], [12.0, 7.0, 7.0]),
            BooleanOp::Subtract,
            k,
        )
        .body;
        case(
            &hollow,
            &other,
            BooleanOp::Union,
            &expect(1436.0, &[2], &[0, 0, 0]),
            k,
            "two hollows overlapping",
        );
    }
}

#[test]
fn operands_of_several_lumps_and_lumps_nested_in_cavities() {
    for k in SCALES {
        let halves = solve(
            &cube(k, 0.0, 10.0),
            &boxed(k, [-1.0, -1.0, 4.0], [11.0, 11.0, 6.0]),
            BooleanOp::Subtract,
            k,
        )
        .body;
        // A rod through both lumps.
        case(
            &halves,
            &rod(k, [5.0, 5.0, -1.0], 1.0, 12.0),
            BooleanOp::Subtract,
            &expect(800.0 - 8.0 * PI, &[0, 0], &[1, 1]),
            k,
            "two lumps less a rod through both",
        );
        // A post that joins them again.
        case(
            &halves,
            &boxed(k, [4.0, 4.0, 0.0], [6.0, 6.0, 10.0]),
            BooleanOp::Union,
            &expect(808.0, &[0], &[0]),
            k,
            "two lumps and a post between them",
        );
        // A hollow block in the cavity of a hollow block: two lumps with a
        // cavity each, the second nested in the first's void.
        let outer = solve(
            &cube(k, 0.0, 10.0),
            &cube(k, 2.0, 8.0),
            BooleanOp::Subtract,
            k,
        )
        .body;
        let inner = solve(
            &cube(k, 3.0, 7.0),
            &cube(k, 4.0, 6.0),
            BooleanOp::Subtract,
            k,
        )
        .body;
        case(
            &outer,
            &inner,
            BooleanOp::Union,
            &expect(1000.0 - 216.0 + 64.0 - 8.0, &[1, 1], &[0, 0, 0, 0]),
            k,
            "a hollow in the cavity of a hollow",
        );
        // A lump in a cavity, drilled through: the cavity opens and the
        // lump hangs in it on its own.
        let nested = solve(&hollow(k), &cube(k, 4.0, 6.0), BooleanOp::Union, k).body;
        case(
            &nested,
            &rod(k, [5.0, 5.0, -1.0], 0.5, 12.0),
            BooleanOp::Subtract,
            &expect(944.0 - 8.0 * 0.25 * PI, &[0, 0], &[1, 1]),
            k,
            "a lump in a cavity, drilled through",
        );
    }
}

#[test]
fn freeform_operands_leave_cavities_and_lumps() {
    for k in SCALES {
        // A cavity bounded by a spline surface with a pole.
        let cone = spline_cone(k, [0.0, 0.0, 0.0], 2.0, 4.0);
        case(
            &boxed(k, [-4.0, -1.0, -4.0], [4.0, 6.0, 4.0]),
            &cone,
            BooleanOp::Subtract,
            &expect(448.0 - PI * 4.0 * 4.0 / 3.0, &[1], &[0, 0]),
            k,
            "block less a spline cone inside",
        );
        // The spline cone cut across: its tip and a frustum.
        let slab = boxed(k, [-3.0, 1.5, -3.0], [3.0, 2.5, 3.0]);
        let tip = PI * 0.75 * 0.75 * 1.5 / 3.0;
        let frustum = PI * (4.0 * 4.0 - 1.25 * 1.25 * 2.5) / 3.0;
        case(
            &cone,
            &slab,
            BooleanOp::Subtract,
            &expect(tip + frustum, &[0, 0], &[0, 0]),
            k,
            "spline cone less a slab across it",
        );
    }
}

/// `body` with every face of shell `shell` turned inside out.
fn turned_inside_out(body: &Body, shell: usize) -> Body {
    let mut out = body.clone();
    for &face in &body.shells[shell].faces {
        let f = &mut out.faces[face.0 as usize];
        f.reversed = !f.reversed;
        for &l in &f.loops.clone() {
            let coedges = &mut out.loops[l.0 as usize].coedges;
            coedges.reverse();
            for c in coedges.iter_mut() {
                c.reversed = !c.reversed;
            }
        }
    }
    out
}

fn nesting_shell(result: Result<Vec<Lump>, KernelError>) -> u32 {
    match result {
        Err(KernelError::InvalidBody { defects, .. }) => match defects.as_slice() {
            [Defect::ShellNesting { shell, reason }] => {
                assert!(!reason.is_empty());
                shell.0
            }
            other => panic!("expected one nesting defect, got {other:?}"),
        },
        other => panic!("expected a nesting defect, got {other:?}"),
    }
}

#[test]
fn shells_that_do_not_nest_as_lumps_and_cavities_are_found() {
    for k in SCALES {
        let tol = tolerance(k);
        let hollow = hollow(k);
        assert_eq!(
            lumps(&hollow, &tol).unwrap(),
            vec![Lump {
                outer: ShellId(0),
                cavities: vec![ShellId(1)]
            }]
        );
        // The cavity turned inside out: a lump in the material of the
        // block. It passes the checker (every edge is used once each way,
        // the counts fit); only where it lies says it is wrong.
        let wrong = turned_inside_out(&hollow, 1);
        assert_eq!(check(&wrong, &tol), []);
        assert_eq!(nesting_shell(lumps(&wrong, &tol)), 1);
        // The outside turned inside out: a cavity outside every lump.
        assert_eq!(
            nesting_shell(lumps(&turned_inside_out(&hollow, 0), &tol)),
            0
        );
        // A cavity apart from the block.
        let far = turned_inside_out(&boxed(k, [20.0, 0.0, 0.0], [22.0, 2.0, 2.0]), 0);
        let apart = merge(&[&cube(k, 0.0, 10.0), &far]).unwrap().0;
        assert_eq!(nesting_shell(lumps(&apart, &tol)), 1);
        // Two lumps apart are two lumps, at no cost of rays.
        let two = merge(&[&cube(k, 0.0, 10.0), &cube(k, 20.0, 22.0)])
            .unwrap()
            .0;
        assert_eq!(shape(&lumps(&two, &tol).unwrap()), vec![0, 0]);
        // A block inside a block, both facing out (a lump in the material
        // of another).
        let stacked = merge(&[&cube(k, 0.0, 10.0), &cube(k, 3.0, 7.0)]).unwrap().0;
        assert_eq!(nesting_shell(lumps(&stacked, &tol)), 1);
    }
}

#[test]
fn a_sheet_is_not_an_operand_and_has_no_lumps() {
    let k = 1.0;
    let tol = tolerance(k);
    let mut sheet = cube(k, 0.0, 1.0);
    sheet.kind = craft3d_topo::BodyKind::Sheet;
    match boolean(&sheet, &cube(k, 0.5, 2.0), BooleanOp::Union, &tol) {
        Err(KernelError::Unsupported { reason, .. }) => assert!(reason.contains("sheet")),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(matches!(
        lumps(&sheet, &tol),
        Err(KernelError::InvalidBody { .. })
    ));
}

/// A plate of `n` × `n` cells of 10 with a cube cavity of 4 in each, put
/// together by hand (the plate and the cavities turned inside out), and
/// its shells' volumes.
fn plate_with_cavities(k: f64, n: usize) -> Body {
    let side = 10.0 * n as f64;
    let mut parts = vec![boxed(k, [0.0, 0.0, 0.0], [side, side, 10.0])];
    for i in 0..n {
        for j in 0..n {
            let (x, y) = (10.0 * i as f64 + 3.0, 10.0 * j as f64 + 3.0);
            parts.push(turned_inside_out(
                &boxed(k, [x, y, 3.0], [x + 4.0, y + 4.0, 7.0]),
                0,
            ));
        }
    }
    let all: Vec<&Body> = parts.iter().collect();
    merge(&all).unwrap().0
}

#[test]
fn shell_volumes_are_the_shells_measured_alone_and_sum_to_the_body() {
    for k in SCALES {
        let body = solve(&hollow(k), &cube(k, 4.0, 6.0), BooleanOp::Union, k).body;
        let (whole, shells) = craft3d_topo::mass_with_shells(&body);
        // The body's measure is the very value mass_properties gives.
        assert_eq!(whole, craft3d_topo::mass_properties(&body));
        assert_eq!(shells.len(), 3);
        for (s, measured) in shells.iter().enumerate() {
            let measured = measured.as_ref().unwrap();
            let alone = mass(&craft3d_topo::shell_alone(&body, ShellId(s as u32)).unwrap()).volume;
            assert!(
                (measured.volume - alone).abs() <= 1e-9 * alone.abs(),
                "shell {s}: {} against {alone}",
                measured.volume
            );
            assert!(measured.bound >= 0.0 && measured.bound < 1e-6 * measured.volume.abs());
        }
        let total: f64 = shells.iter().map(|s| s.as_ref().unwrap().volume).sum();
        let volume = whole.unwrap().volume;
        assert!(
            (total - volume).abs() <= 1e-9 * volume,
            "{total} against {volume}"
        );
    }
}

#[test]
fn a_cavity_that_meets_the_outside_is_refused_as_shells_that_meet() {
    for k in SCALES {
        let tol = tolerance(k);
        // A cavity in the corner of the block: its corner is the block's.
        let corner = turned_inside_out(&cube(k, 0.0, 2.0), 0);
        let body = merge(&[&cube(k, 0.0, 10.0), &corner]).unwrap().0;
        match lumps(&body, &tol) {
            Err(KernelError::InvalidBody { defects, .. }) => match defects.as_slice() {
                [Defect::ShellNesting { reason, .. }] => {
                    assert!(reason.contains("meets"), "{reason}")
                }
                other => panic!("{other:?}"),
            },
            other => panic!("expected shells that meet, got {other:?}"),
        }
    }
}

#[test]
fn many_cavities_are_sorted_with_each_shell_prepared_once() {
    let k = 1.0;
    let tol = tolerance(k);
    let bodies: Vec<(usize, Body)> = [5, 10]
        .into_iter()
        .map(|n| (n * n, plate_with_cavities(k, n)))
        .collect();
    let volumes: Vec<_> = bodies
        .iter()
        .map(|(_, b)| craft3d_topo::shell_volumes(b))
        .collect();
    for ((count, body), measured) in bodies.iter().zip(&volumes) {
        let found = craft3d_kernel::lumps::lumps_measured(body, &tol, measured).unwrap();
        assert_eq!(shape(&found), vec![*count]);
    }
    // Four times the cavities, about four times the work: classifying
    // them against a plate prepared once per cavity would be sixteen.
    craft3d_testkit::cost::assert_growth_below(
        "lumps of a plate with cavities",
        (0, 1),
        8.0,
        std::time::Duration::ZERO,
        |i| {
            let (_, body) = &bodies[i];
            craft3d_kernel::lumps::lumps_measured(body, &tol, &volumes[i]).unwrap();
        },
    );
}

/// The time of `lumps` on a plate with 400 cube cavities (the review's
/// case), in release:
/// `cargo test --release -p craft3d-kernel --test lumps -- --ignored --nocapture`
#[test]
#[ignore = "timing, information only"]
fn time_of_lumps_on_a_plate_with_400_cavities() {
    let k = 1.0;
    let body = plate_with_cavities(k, 20);
    let start = std::time::Instant::now();
    let found = lumps(&body, &tolerance(k)).unwrap();
    println!(
        "{} faces, {} shells: lumps in {:?}",
        body.faces.len(),
        body.shells.len(),
        start.elapsed()
    );
    assert_eq!(shape(&found), vec![400]);
    // And the look regeneration takes for shells that touch.
    let start = std::time::Instant::now();
    let touch = craft3d_kernel::boolean::parts_touch(&body, &tolerance(k)).unwrap();
    println!("parts_touch in {:?}", start.elapsed());
    assert_eq!(touch, None);
}

/// A small shell in a large body keeps its sign check: what its volume
/// can be off by scales with the shell, not with the body around it (the
/// review of 2026-10-21 found a cube of 0.05 or 0.01 in a block of 1000,
/// facing the wrong way, accepted once the sign was held to a bound that
/// grew with the whole body's diagonal). In the middle of the block and
/// near two of its corners (far from the block's middle), at three scales.
#[test]
fn a_small_shell_in_a_large_body_is_held_to_its_sign() {
    for k in SCALES {
        let tol = tolerance(k);
        let block = cube(k, 0.0, 1000.0);
        for side in [0.05, 0.01] {
            for at in [500.0, 3.0, 996.0] {
                let what = format!("a cube of {side} at {at} in a block of 1000 at scale {k}");
                let small = cube(k, at, at + side);
                // A cavity as it should be: one lump with one cavity.
                let right = merge(&[&block, &turned_inside_out(&small, 0)]).unwrap().0;
                assert_eq!(check(&right, &tol), [], "{what}");
                assert_eq!(
                    lumps(&right, &tol).unwrap(),
                    vec![Lump {
                        outer: ShellId(0),
                        cavities: vec![ShellId(1)]
                    }],
                    "{what}"
                );
                // The cavity with its faces inside out, which is a small
                // block facing out in the material of the large one: the
                // checker passes it, its sign does not.
                let wrong = merge(&[&block, &small]).unwrap().0;
                assert_eq!(check(&wrong, &tol), [], "{what}");
                assert_eq!(nesting_shell(lumps(&wrong, &tol)), 1, "{what}");
                // A small cavity outside every lump.
                let far = cube(k, 1000.0 + at, 1000.0 + at + side);
                let apart = merge(&[&block, &turned_inside_out(&far, 0)]).unwrap().0;
                assert_eq!(nesting_shell(lumps(&apart, &tol)), 1, "{what}, apart");
            }
        }
    }
}
