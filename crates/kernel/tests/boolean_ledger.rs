//! The ledger of an operation (`craft3d_geom::allowance::asked`: what it
//! counts, what its caches answer from memory and the arithmetic of
//! rational nets) is a function of what it asks alone: the same with the
//! caches cold and warm and with the section's helpers or without them.
//! That is what allows a bound on it at all (docs/design/booleans.md ›
//! *A boolean that runs for most of an hour*). The count itself is not
//! (helpers fill the trees' memos, whose hits it does not charge), which
//! is why the bound is on the ledger.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use craft3d_geom::{NurbsSurface, Surface, allowance};
use craft3d_kernel::boolean::{BooleanOp, boolean, release_unheld};
use craft3d_kernel::primitives;
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::Body;

/// As `boolean_weighted_chart.rs`.
fn block_with_weighted_sides(size: Vec3, seed: u64) -> Body {
    let mut body = primitives::block(&Frame::WORLD, size.x, size.y, size.z).unwrap();
    let mut state = seed;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0];
    for index in 0..body.faces.len() {
        let Some(face) = body.faces.get(index) else {
            continue;
        };
        let Surface::Plane(plane) = &face.surface else {
            continue;
        };
        let f = *plane.frame();
        if !(f.z().x > 0.99 || f.z().y > 0.99 || f.z().z > 0.99) {
            continue;
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for vertex in &body.vertices {
            let (x, y, z) = f.local(vertex.point);
            if z.abs() <= 1e-9 * size.length() {
                lo = [lo[0].min(x), lo[1].min(y)];
                hi = [hi[0].max(x), hi[1].max(y)];
            }
        }
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for i in 0..6 {
            for j in 0..6 {
                ctrl.push(f.point(
                    lo[0] + (hi[0] - lo[0]) * f64::from(i) / 5.0,
                    lo[1] + (hi[1] - lo[1]) * f64::from(j) / 5.0,
                    0.0,
                ));
                weights.push((6.0 * (next() - 0.5) * std::f64::consts::LN_10).pexp());
            }
        }
        if let Some(face) = body.faces.get_mut(index) {
            face.surface = Surface::Nurbs(
                NurbsSurface::new(3, 3, knots.clone(), knots.clone(), 6, 6, ctrl, weights).unwrap(),
            );
        }
    }
    body
}

/// A block of `size` in `frame` whose bottom is a ruled spline (no
/// weights) whose speed along one side rises from `slow` of its width to
/// twice it, as `classify_far.rs` makes it.
fn block_with_a_ruled_bottom(frame: &Frame, size: Vec3, slow: f64) -> Body {
    let mut body = primitives::block(frame, size.x, size.y, size.z).unwrap();
    let bottom = body
        .faces
        .iter()
        .position(|face| match &face.surface {
            Surface::Plane(plane) => {
                let (_, _, z) = frame.local(plane.frame().origin());
                plane.frame().z().dot(frame.z()).abs() > 0.99 && z.abs() <= 1e-9 * size.length()
            }
            _ => false,
        })
        .unwrap();
    let Surface::Plane(plane) = &body.faces[bottom].surface else {
        panic!("the bottom is not a plane")
    };
    let f = *plane.frame();
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for vertex in &body.vertices {
        let (x, y, z) = f.local(vertex.point);
        if z.abs() <= 1e-9 * size.length() {
            lo = [lo[0].min(x), lo[1].min(y)];
            hi = [hi[0].max(x), hi[1].max(y)];
        }
    }
    let at = |x: f64, y: f64| {
        f.point(
            lo[0] + x * (hi[0] - lo[0]),
            lo[1] + y * (hi[1] - lo[1]),
            0.0,
        )
    };
    let line = vec![0.0, 0.0, 1.0, 1.0];
    let surface = if slow > 0.0 {
        NurbsSurface::new(
            2,
            1,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            line,
            3,
            2,
            vec![
                at(0.0, 0.0),
                at(0.0, 1.0),
                at(slow, 0.0),
                at(slow, 1.0),
                at(1.0, 0.0),
                at(1.0, 1.0),
            ],
            vec![1.0; 6],
        )
    } else {
        NurbsSurface::new(
            1,
            1,
            line.clone(),
            line,
            2,
            2,
            vec![at(0.0, 0.0), at(0.0, 1.0), at(1.0, 0.0), at(1.0, 1.0)],
            vec![1.0; 4],
        )
    };
    body.faces[bottom].surface = Surface::Nurbs(surface.unwrap());
    body
}

fn block_with_a_slow_bottom(slow: f64) -> Body {
    block_with_a_ruled_bottom(&Frame::WORLD, Vec3::new(30.0, 30.0, 20.0), slow)
}

/// The caches the kernel and geometry keep between operations, let go of.
fn cold() {
    let _ = release_unheld(&std::collections::BTreeSet::new());
    let _ = craft3d_geom::intersect::forget_kept();
    craft3d_geom::forget_projections();
}

/// The boolean of `make()` with `tool`: what it gave (as text) and what
/// it added to the count and to the ledger.
fn ledger_of(make: &(dyn Fn() -> Body + Sync), tool: &Body, op: BooleanOp) -> (String, u64, u64) {
    let tol = Tolerance::new(1e-6, 1e-9).unwrap();
    let body = make();
    let (spent, asked) = (allowance::spent(), allowance::asked());
    let made = boolean(&body, tool, op, &tol);
    (
        match made {
            Ok(m) => format!("built, {} faces", m.body.faces.len()),
            Err(e) => format!("refused: {e}"),
        },
        allowance::spent() - spent,
        allowance::asked() - asked,
    )
}

/// The ledger of the same booleans cold and warm, with the helpers the
/// pool has, with one at most and alone: the same number every time.
#[test]
fn the_ledger_of_a_boolean_does_not_depend_on_caches_or_threads() {
    let weighted = || block_with_weighted_sides(Vec3::new(30.0, 30.0, 20.0), 1);
    let slow = || block_with_a_slow_bottom(1e-4);
    let over = primitives::block(
        &Frame::WORLD.at(Vec3::new(12.0, 10.5, 9.0)).unwrap(),
        27.0,
        25.5,
        16.0,
    )
    .unwrap();
    let under = primitives::block(
        &Frame::WORLD.at(Vec3::new(12.0, 10.5, -4.0)).unwrap(),
        27.0,
        25.5,
        16.0,
    )
    .unwrap();
    let cases: [(&str, &(dyn Fn() -> Body + Sync), &Body, BooleanOp); 3] = [
        ("weighted sides", &weighted, &over, BooleanOp::Intersect),
        ("a slow bottom", &slow, &under, BooleanOp::Intersect),
        ("a slow bottom", &slow, &under, BooleanOp::Subtract),
    ];
    for (name, make, tool, op) in cases {
        let mut seen = Vec::new();
        // With the pool's helpers, cold and then warm.
        cold();
        seen.push(("all, cold", ledger_of(make, tool, op)));
        seen.push(("all, warm", ledger_of(make, tool, op)));
        // With one helper at most: the rest of the pool held here.
        {
            let _held = craft3d_math::pool::lease(craft3d_math::pool::SHARED_THREADS - 1);
            cold();
            seen.push(("one helper, cold", ledger_of(make, tool, op)));
            seen.push(("one helper, warm", ledger_of(make, tool, op)));
        }
        // Alone, on a thread of its own that takes none.
        let alone = std::thread::scope(|s| {
            s.spawn(|| {
                craft3d_math::pool::work_alone();
                cold();
                let first = ledger_of(make, tool, op);
                (first, ledger_of(make, tool, op))
            })
            .join()
            .unwrap()
        });
        seen.push(("alone, cold", alone.0));
        seen.push(("alone, warm", alone.1));
        for (how, (made, spent, asked)) in &seen {
            println!("{name} {op:?}, {how}: {made}; counted {spent}, ledger {asked}");
        }
        let (first_made, _, first_asked) = &seen[0].1;
        for (how, (made, _, asked)) in &seen {
            assert_eq!(made, first_made, "{name} {op:?}, {how}");
            assert_eq!(asked, first_asked, "{name} {op:?}, {how}: the ledger moved");
        }
    }
}

/// The ledger of a boolean after a different boolean on the same
/// surfaces, and of the two interleaved: the same as each cold. The
/// review's case (2026-11-16): the long block with weighted sides, whose
/// intersection asks so many questions of its trees that their shared
/// memos let go of answers the operation asks again; searched again where
/// the ledger does not grow, those were kept with nothing on it, and the
/// intersection after the subtraction counted 971 883 155 against
/// 1 103 978 774 cold (the subtraction 543 256 296 and 480 578 418
/// against 546 165 204).
#[test]
fn the_ledger_of_a_boolean_does_not_depend_on_the_operations_before_it() {
    let long = || block_with_weighted_sides(Vec3::new(30.0, 900.0, 20.0), 1);
    let block_at = |at: Vec3, size: Vec3| {
        primitives::block(&Frame::WORLD.at(at).unwrap(), size.x, size.y, size.z).unwrap()
    };
    let through = block_at(Vec3::new(12.0, 315.0, 9.0), Vec3::new(27.0, 765.0, 16.0));
    let across = block_at(Vec3::new(5.0, 100.0, 12.0), Vec3::new(30.0, 500.0, 10.0));
    let one = (&through, BooleanOp::Intersect);
    let other = (&across, BooleanOp::Subtract);
    let run = |(tool, op): (&Body, BooleanOp)| ledger_of(&long, tool, op);
    cold();
    let one_cold = run(one);
    cold();
    let other_cold = run(other);
    let mut seen = vec![("intersection", "cold", one_cold.clone())];
    seen.push(("subtraction", "cold", other_cold.clone()));
    cold();
    let _ = run(other);
    seen.push(("intersection", "after the subtraction", run(one)));
    cold();
    let _ = run(one);
    seen.push(("subtraction", "after the intersection", run(other)));
    cold();
    for round in ["interleaved, first", "interleaved, second"] {
        seen.push(("intersection", round, run(one)));
        seen.push(("subtraction", round, run(other)));
    }
    for (name, how, (made, spent, asked)) in &seen {
        println!("long block, {name}, {how}: {made}; counted {spent}, ledger {asked}");
    }
    assert!(one_cold.2 > 0 && other_cold.2 > 0);
    for (name, how, (made, _, asked)) in &seen {
        let (cold_made, _, cold_asked) = if *name == "intersection" {
            &one_cold
        } else {
            &other_cold
        };
        assert_eq!(made, cold_made, "{name}, {how}");
        assert_eq!(asked, cold_asked, "{name}, {how}: the ledger moved");
    }
}

/// The block whose bottom is a ruled spline whose speed rises a
/// hundred thousandfold or a millionfold (no weights), intersected with a
/// block through the bottom: what it gives, in how long, and its ledger.
/// By hand, in release: it runs for a minute or more.
#[test]
#[ignore = "a minute or more: by hand, in release"]
fn the_slowed_bottom_at_the_bound() {
    let tol = Tolerance::new(1e-6, 1e-9).unwrap();
    let under = primitives::block(
        &Frame::WORLD.at(Vec3::new(12.0, 10.5, -4.0)).unwrap(),
        27.0,
        25.5,
        16.0,
    )
    .unwrap();
    for slow in [1e-5, 1e-6] {
        let body = block_with_a_slow_bottom(slow);
        let started = std::time::Instant::now();
        let ((made, spent), asked) = allowance::measured_asked(|| {
            allowance::measured(|| boolean(&body, &under, BooleanOp::Intersect, &tol))
        });
        println!(
            "the bottom slowed to {slow}: {} in {:.1} s; counted {spent}, ledger {asked}",
            match made {
                Ok(m) => format!("built, {} faces", m.body.faces.len()),
                Err(e) => format!("refused: {e}"),
            },
            started.elapsed().as_secs_f64()
        );
    }
}
