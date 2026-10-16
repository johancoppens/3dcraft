//! Carrying a sketch along when its image points jump
//! (`Sketch::image_starts`): geometry that hangs on an image follows the
//! image to its new place instead of landing on another root, whatever the
//! size of the jump, and a sketch whose images did not move solves exactly
//! as it did.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Tolerance;
use craft3d_solve::{
    Constraint as C, ConstraintStatus, Entity, EntityId, ImageMap, Sketch, Solution, SolveOptions,
    solve,
};

fn options(scale: f64) -> SolveOptions {
    SolveOptions::new(Tolerance::new(1e-9 * scale, 1e-9).unwrap())
}

/// A square wave from the fixed origin (up 2, right 5, down 2, right 5,
/// all dimensioned), its last instance in a chained run of `n`, and after
/// it a point `q` one unit further right (a horizontal line from the image,
/// one unit long) and a point `r` five units below `q`. Returns the sketch,
/// the image and `q`, `r`.
struct Wave {
    sketch: Sketch,
    image: EntityId,
    q: EntityId,
    r: EntityId,
}

fn wave(scale: f64, n: u32) -> Wave {
    let mut s = Sketch::new();
    let p = |v: f64| v * scale;
    let o = s.add_point(0.0, 0.0);
    let a = s.add_point(0.0, p(2.0));
    let b = s.add_point(p(5.0), p(2.0));
    let c = s.add_point(p(5.0), 0.0);
    let d = s.add_point(p(10.0), 0.0);
    let image = EntityId(u32::try_from(s.entities.len()).unwrap());
    s.entities.push(Entity::Image {
        of: d,
        map: ImageMap::Chain {
            from: o,
            to: d,
            k: f64::from(n - 1),
        },
    });
    let end = p(10.0 * f64::from(n));
    let q = s.add_point(end + p(1.0), 0.0);
    let r = s.add_point(end + p(1.0), p(-5.0));
    s.add_constraint(C::Fixed { point: o, at: None });
    for (from, to, horizontal, length) in [
        (o, a, false, 2.0),
        (a, b, true, 5.0),
        (b, c, false, 2.0),
        (c, d, true, 5.0),
        (image, q, true, 1.0),
        (q, r, false, 5.0),
    ] {
        let line = s.add_line(from, to);
        s.add_constraint(if horizontal {
            C::Horizontal { line }
        } else {
            C::Vertical { line }
        });
        s.add_constraint(C::Distance {
            a: from,
            b: to,
            value: p(length),
        });
    }
    Wave {
        sketch: s,
        image,
        q,
        r,
    }
}

/// `solved` with the count of the run set to `n`, as the next starting
/// guess: the stored coordinates, and where the image was when they were a
/// solution (`carry`), or nothing (the solve as it was before).
fn recount(w: &Wave, solved: &Solution, n: u32, carry: bool) -> Sketch {
    let mut s = w.sketch.clone();
    s.entities = solved.entities.clone();
    let was = image_at(&s, w.image);
    if let Some(Entity::Image {
        map: ImageMap::Chain { k, .. },
        ..
    }) = s.entities.get_mut(w.image.0 as usize)
    {
        *k = f64::from(n - 1);
    }
    s.image_starts = if carry {
        vec![(w.image, was)]
    } else {
        Vec::new()
    };
    s
}

fn image_at(s: &Sketch, image: EntityId) -> [f64; 2] {
    let Some(Entity::Image { of, map }) = s.entities.get(image.0 as usize) else {
        panic!("not an image");
    };
    map.apply(s.point(*of).unwrap(), |id| s.point(id)).unwrap()
}

fn ok(solution: &Solution) {
    assert!(
        solution
            .constraints
            .iter()
            .all(|c| c.status == ConstraintStatus::Satisfied),
        "{:?}",
        solution.constraints
    );
}

/// The welding table's finding W1: the count of a run jumps from 3 to 10
/// and back, from 1 to 400, from 400 to 2. Carried along, the point after
/// the run lands one unit beyond its end every time, at three scales;
/// without the carrying the jump up put it one unit *before* the end, the
/// profile folded back.
#[test]
fn geometry_after_a_run_follows_its_end_when_the_count_jumps() {
    for scale in [1e-3, 1.0, 1e3] {
        for (from, to) in [(3, 10), (10, 3), (1, 400), (400, 2), (3, 4)] {
            let w = wave(scale, from);
            let first = solve(&w.sketch, &options(scale)).unwrap();
            ok(&first);
            let [qx, _] = first.point(w.q).unwrap();
            let want = scale * (10.0 * f64::from(from) + 1.0);
            assert!((qx - want).abs() <= 1e-9 * want, "{qx} vs {want}");

            let next = solve(&recount(&w, &first, to, true), &options(scale)).unwrap();
            ok(&next);
            let want = scale * (10.0 * f64::from(to) + 1.0);
            let [qx, qy] = next.point(w.q).unwrap();
            let [rx, ry] = next.point(w.r).unwrap();
            assert!(
                (qx - want).abs() <= 1e-9 * want && qy.abs() <= 1e-9 * scale,
                "{from} -> {to} at {scale}: q at {qx}, {qy}, want {want}"
            );
            assert!(
                (rx - want).abs() <= 1e-9 * want && (ry + 5.0 * scale).abs() <= 1e-9 * scale,
                "{from} -> {to} at {scale}: r at {rx}, {ry}"
            );
        }
    }
    // What the finding saw: from the stored coordinates alone, the jump
    // up lands on the other root.
    let w = wave(1.0, 3);
    let first = solve(&w.sketch, &options(1.0)).unwrap();
    let flipped = solve(&recount(&w, &first, 10, false), &options(1.0)).unwrap();
    let [qx, _] = flipped.point(w.q).unwrap();
    assert!((qx - 99.0).abs() < 1e-6, "{qx}");
}

/// 3 -> 10 -> 3 comes back to the root it started on, within rounding.
#[test]
fn a_count_that_goes_and_comes_back_returns_to_the_same_root() {
    for scale in [1e-3, 1.0, 1e3] {
        let w = wave(scale, 3);
        let first = solve(&w.sketch, &options(scale)).unwrap();
        let up = solve(&recount(&w, &first, 10, true), &options(scale)).unwrap();
        let back = solve(&recount(&w, &up, 3, true), &options(scale)).unwrap();
        for (a, b) in first.entities.iter().zip(&back.entities) {
            if let (Entity::Point { x, y }, Entity::Point { x: x2, y: y2 }) = (a, b) {
                assert!(
                    (x - x2).abs() <= 1e-12 * 100.0 * scale && (y - y2).abs() <= 1e-12 * scale,
                    "{a:?} vs {b:?}"
                );
            }
        }
    }
}

/// A turn whose angle changes from 30 to 170 degrees: a point kept on the
/// ray from the centre through the image, one unit beyond the image, stays
/// beyond it (radius 4) rather than coming to rest before it (radius 2).
#[test]
fn a_turn_whose_angle_changes_carries_what_hangs_on_its_image() {
    for scale in [1e-3, 1.0, 1e3] {
        let p = |v: f64| v * scale;
        let build = |degrees: f64, start: [f64; 2]| {
            let mut s = Sketch::new();
            let centre = s.add_point(0.0, 0.0);
            let seed = s.add_point(p(3.0), 0.0);
            let image = EntityId(u32::try_from(s.entities.len()).unwrap());
            s.entities.push(Entity::Image {
                of: seed,
                map: ImageMap::Turn {
                    center: Some(centre),
                    at: [0.0, 0.0],
                    angle: degrees.to_radians(),
                },
            });
            let q = s.add_point(start[0], start[1]);
            let ray = s.add_line(centre, image);
            s.add_constraint(C::Fixed {
                point: centre,
                at: None,
            });
            s.add_constraint(C::Fixed {
                point: seed,
                at: None,
            });
            s.add_constraint(C::PointOnLine {
                point: q,
                line: ray,
            });
            s.add_constraint(C::Distance {
                a: image,
                b: q,
                value: p(1.0),
            });
            (s, image, q)
        };
        let a = 30f64.to_radians();
        let (s, image, q) = build(30.0, [p(4.0) * a.cos(), p(4.0) * a.sin()]);
        let first = solve(&s, &options(scale)).unwrap();
        ok(&first);
        let was = image_at(&s, image);
        let (mut next, ..) = build(170.0, [0.0, 0.0]);
        next.entities = first.entities.clone();
        if let Some(Entity::Image {
            map: ImageMap::Turn { angle, .. },
            ..
        }) = next.entities.get_mut(image.0 as usize)
        {
            *angle = 170f64.to_radians();
        }
        next.image_starts = vec![(image, was)];
        let solved = solve(&next, &options(scale)).unwrap();
        ok(&solved);
        let [x, y] = solved.point(q).unwrap();
        let b = 170f64.to_radians();
        assert!(
            (x - p(4.0) * b.cos()).abs() <= 1e-9 * scale
                && (y - p(4.0) * b.sin()).abs() <= 1e-9 * scale,
            "at {scale}: q at {x}, {y}"
        );
    }
}

/// Where the image is now, or within the tolerance of it: the solve is the
/// one without `image_starts`, bit for bit. Hostile entries (not an image,
/// no such entity, not finite) are left alone.
#[test]
fn an_image_that_did_not_move_changes_nothing() {
    let w = wave(1.0, 3);
    let first = solve(&w.sketch, &options(1.0)).unwrap();
    let plain = solve(&recount(&w, &first, 10, false), &options(1.0)).unwrap();
    let mut s = recount(&w, &first, 10, false);
    let here = image_at(&s, w.image);
    for starts in [
        vec![(w.image, here)],
        vec![(w.image, [here[0] + 1e-10, here[1]])],
        vec![(EntityId(0), [55.0, 1.0])],
        vec![(EntityId(9999), [55.0, 1.0])],
        vec![(w.image, [f64::NAN, 0.0])],
        vec![(w.image, [f64::INFINITY, 0.0])],
    ] {
        s.image_starts = starts.clone();
        assert_eq!(solve(&s, &options(1.0)), Ok(plain.clone()), "{starts:?}");
    }
    // Carrying is deterministic too.
    let carried = recount(&w, &first, 10, true);
    assert_eq!(
        solve(&carried, &options(1.0)),
        solve(&carried, &options(1.0))
    );
}

/// The carried solve keeps the solver's guarantees: its solution solves
/// again to itself with the same verdict, and a conflict that comes with
/// the jump is still diagnosed (q pinned where the old end of the run
/// was cannot be one unit after the new one).
#[test]
fn a_carried_solve_is_a_solve() {
    let w = wave(1.0, 3);
    let first = solve(&w.sketch, &options(1.0)).unwrap();
    let carried = solve(&recount(&w, &first, 10, true), &options(1.0)).unwrap();
    let mut again = w.sketch.clone();
    again.entities = carried.entities.clone();
    if let Some(Entity::Image {
        map: ImageMap::Chain { k, .. },
        ..
    }) = again.entities.get_mut(w.image.0 as usize)
    {
        *k = 9.0;
    }
    let second = solve(&again, &options(1.0)).unwrap();
    assert_eq!(second.entities, carried.entities);
    assert_eq!(second.constraints, carried.constraints);

    let mut pinned = recount(&w, &first, 10, true);
    pinned.add_constraint(C::Fixed {
        point: w.q,
        at: Some([31.0, 0.0]),
    });
    let mut earlier = w.sketch.clone();
    earlier.add_constraint(C::Fixed {
        point: w.q,
        at: Some([31.0, 0.0]),
    });
    let solved = solve(&pinned, &options(1.0)).unwrap();
    let last = solved.constraints.last().unwrap();
    assert_eq!(last.status, ConstraintStatus::Conflicting, "{solved:?}");
    assert!(solve(&earlier, &options(1.0)).unwrap().is_consistent());
}
