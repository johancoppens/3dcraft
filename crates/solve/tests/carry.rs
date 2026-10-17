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
    Carried, Constraint as C, ConstraintStatus, Entity, EntityId, ImageMap, ImageStart, Sketch,
    Solution, SolveOptions, solve,
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

/// How the next solve is told where the images were.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Start {
    /// Not at all: the solve as it was before carrying.
    None,
    /// The place only: carried in a straight line.
    Place,
    /// The place and the map: carried through the pattern's family.
    Map,
}

fn start(s: &Sketch, image: EntityId, how: Start) -> Vec<ImageStart> {
    let Some(Entity::Image { map, .. }) = s.entities.get(image.0 as usize) else {
        panic!("not an image");
    };
    match how {
        Start::None => Vec::new(),
        Start::Place => vec![ImageStart {
            image,
            at: image_at(s, image),
            map: None,
        }],
        Start::Map => vec![ImageStart {
            image,
            at: image_at(s, image),
            map: Some(*map),
        }],
    }
}

/// `solved` with the count of the run set to `n`, as the next starting
/// guess: the stored coordinates, and where the image was when they were a
/// solution, as `how` says.
fn recount(w: &Wave, solved: &Solution, n: u32, how: Start) -> Sketch {
    let mut s = w.sketch.clone();
    s.entities = solved.entities.clone();
    let starts = start(&s, w.image, how);
    if let Some(Entity::Image {
        map: ImageMap::Chain { k, .. },
        ..
    }) = s.entities.get_mut(w.image.0 as usize)
    {
        *k = f64::from(n - 1);
    }
    s.image_starts = starts;
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
    for (scale, how) in [1e-3, 1.0, 1e3]
        .into_iter()
        .flat_map(|s| [(s, Start::Map), (s, Start::Place)])
    {
        for (from, to) in [(3, 10), (10, 3), (1, 400), (400, 2), (3, 4)] {
            let w = wave(scale, from);
            let first = solve(&w.sketch, &options(scale)).unwrap();
            ok(&first);
            let [qx, _] = first.point(w.q).unwrap();
            let want = scale * (10.0 * f64::from(from) + 1.0);
            assert!((qx - want).abs() <= 1e-9 * want, "{qx} vs {want}");

            let next = solve(&recount(&w, &first, to, how), &options(scale)).unwrap();
            ok(&next);
            let want = scale * (10.0 * f64::from(to) + 1.0);
            let [qx, qy] = next.point(w.q).unwrap();
            let [rx, ry] = next.point(w.r).unwrap();
            assert!(
                (qx - want).abs() <= 1e-9 * want && qy.abs() <= 1e-9 * scale,
                "{from} -> {to} at {scale} ({how:?}): q at {qx}, {qy}, want {want}"
            );
            assert_eq!(next.carry, Some(Carried::Whole), "{from} -> {to}");
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
    let flipped = solve(&recount(&w, &first, 10, Start::None), &options(1.0)).unwrap();
    let [qx, _] = flipped.point(w.q).unwrap();
    assert!((qx - 99.0).abs() < 1e-6, "{qx}");
}

/// 3 -> 10 -> 3 comes back to the root it started on, within rounding,
/// by way of the right root at 10 (without the carrying the way up lands
/// on the other root, and this fails there).
#[test]
fn a_count_that_goes_and_comes_back_returns_to_the_same_root() {
    for scale in [1e-3, 1.0, 1e3] {
        let w = wave(scale, 3);
        let first = solve(&w.sketch, &options(scale)).unwrap();
        let up = solve(&recount(&w, &first, 10, Start::Map), &options(scale)).unwrap();
        let [qx, _] = up.point(w.q).unwrap();
        assert!(
            (qx - 101.0 * scale).abs() <= 1e-9 * scale * 101.0,
            "up at {scale}: q at {qx}"
        );
        let back = solve(&recount(&w, &up, 3, Start::Map), &options(scale)).unwrap();
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
        next.image_starts = vec![ImageStart {
            image,
            at: was,
            map: None,
        }];
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
/// no such entity, not finite, a map of another family that puts it where
/// it is) are left alone.
#[test]
fn an_image_that_did_not_move_changes_nothing() {
    let w = wave(1.0, 3);
    let first = solve(&w.sketch, &options(1.0)).unwrap();
    let plain = solve(&recount(&w, &first, 10, Start::None), &options(1.0)).unwrap();
    assert_eq!(plain.carry, None);
    let mut s = recount(&w, &first, 10, Start::None);
    let here = image_at(&s, w.image);
    let Some(Entity::Image { map: now, .. }) = s.entities.get(w.image.0 as usize).copied() else {
        panic!()
    };
    let place = |image, at| ImageStart {
        image,
        at,
        map: None,
    };
    for starts in [
        vec![place(w.image, here)],
        vec![place(w.image, [here[0] + 1e-10, here[1]])],
        // Its map now, and where that puts it.
        vec![ImageStart {
            image: w.image,
            at: here,
            map: Some(now),
        }],
        vec![place(EntityId(0), [55.0, 1.0])],
        vec![place(EntityId(9999), [55.0, 1.0])],
        vec![place(w.image, [f64::NAN, 0.0])],
        vec![place(w.image, [f64::INFINITY, 0.0])],
        vec![ImageStart {
            image: w.image,
            at: here,
            map: Some(ImageMap::Turn {
                center: None,
                at: [0.0, 0.0],
                angle: f64::NAN,
            }),
        }],
    ] {
        s.image_starts = starts.clone();
        assert_eq!(solve(&s, &options(1.0)), Ok(plain.clone()), "{starts:?}");
    }
    // Carrying is deterministic too.
    let carried = recount(&w, &first, 10, Start::Map);
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
    let carried = solve(&recount(&w, &first, 10, Start::Map), &options(1.0)).unwrap();
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

    let mut pinned = recount(&w, &first, 10, Start::Map);
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

/// A turn about a fixed centre point: a seed point at radius 3, its image
/// in instance `k` turned by `k * step`, and a point `q` on the ray from
/// the centre through the image, one unit beyond it. Returns the sketch,
/// the image and `q`.
fn turn(scale: f64, angle: f64, q_at: [f64; 2]) -> (Sketch, EntityId, EntityId) {
    let mut s = Sketch::new();
    let centre = s.add_point(0.0, 0.0);
    let seed = s.add_point(3.0 * scale, 0.0);
    let image = EntityId(u32::try_from(s.entities.len()).unwrap());
    s.entities.push(Entity::Image {
        of: seed,
        map: ImageMap::Turn {
            center: Some(centre),
            at: [0.0, 0.0],
            angle,
        },
    });
    let q = s.add_point(q_at[0], q_at[1]);
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
        value: scale,
    });
    (s, image, q)
}

/// From the solution at angle `from`, the turn set to `to` and solved
/// with the image's start told as `how` says.
fn returned(scale: f64, from: f64, to: f64, how: Start) -> (Solution, EntityId) {
    let (s, image, q) = turn(
        scale,
        from,
        [4.0 * scale * from.cos(), 4.0 * scale * from.sin()],
    );
    let first = solve(&s, &options(scale)).unwrap();
    ok(&first);
    let mut next = s.clone();
    next.entities = first.entities.clone();
    next.image_starts = start(&next, image, how);
    if let Some(Entity::Image {
        map: ImageMap::Turn { angle, .. },
        ..
    }) = next.entities.get_mut(image.0 as usize)
    {
        *angle = to;
    }
    (solve(&next, &options(scale)).unwrap(), q)
}

/// The image of a ring with a step of 30 degrees whose count goes from 3
/// to 10 swings from 60 to 270 degrees, and a turn from 1 to 179 degrees:
/// carried through the turn (its angle going from the old value to the new
/// one), the point one unit beyond the image stays beyond it, at three
/// scales. A straight path from the old place to the new one passes near
/// the centre, where the ray has nearly no direction.
#[test]
fn a_ring_whose_count_jumps_carries_what_hangs_on_its_image() {
    for scale in [1e-3, 1.0, 1e3] {
        for (from, to) in [
            (60.0f64, 270.0f64),
            (1.0, 179.0),
            (270.0, 60.0),
            (0.0, 180.0),
        ] {
            let (from, to) = (from.to_radians(), to.to_radians());
            let (solved, q) = returned(scale, from, to, Start::Map);
            ok(&solved);
            assert_eq!(solved.carry, Some(Carried::Whole));
            let [x, y] = solved.point(q).unwrap();
            let (wx, wy) = (4.0 * scale * to.cos(), 4.0 * scale * to.sin());
            assert!(
                (x - wx).abs() <= 1e-9 * scale && (y - wy).abs() <= 1e-9 * scale,
                "{} -> {} deg at {scale}: q at {x}, {y}, want {wx}, {wy}",
                from.to_degrees(),
                to.to_degrees()
            );
        }
    }
}

/// When the path cannot be followed to its end the solution says how far
/// it got: `q` pinned at 31 cannot stay one unit after an image that goes
/// from 30 to 90 (the run's count 3 -> 10) past the moment it is at 32.
#[test]
fn a_path_that_stops_says_how_far_it_got() {
    let w = wave(1.0, 3);
    let first = solve(&w.sketch, &options(1.0)).unwrap();
    let mut pinned = recount(&w, &first, 10, Start::Map);
    pinned.add_constraint(C::Fixed {
        point: w.q,
        at: Some([31.0, 0.0]),
    });
    let solved = solve(&pinned, &options(1.0)).unwrap();
    match solved.carry {
        Some(Carried::Stopped {
            reached,
            moved,
            out_of_work,
        }) => {
            assert!((0.0..1.0).contains(&reached), "{reached}");
            assert_eq!(moved, vec![w.image]);
            assert!(!out_of_work);
        }
        other => panic!("{other:?}"),
    }
}

/// An equation that holds as many unknowns as one can (18: four image
/// points made from nine points) cannot take the path parameter as well:
/// nothing is carried, the solution says so, and the solve is the one
/// from the stored coordinates.
#[test]
fn an_equation_too_full_for_the_path_is_not_carried() {
    let mut s = Sketch::new();
    let point = |s: &mut Sketch, x: f64, y: f64| s.add_point(x, y);
    let (a1, a2) = (point(&mut s, 0.0, 0.0), point(&mut s, 1.0, 0.0));
    let (b1, b2) = (point(&mut s, 0.0, 5.0), point(&mut s, 0.0, 6.0));
    let c = point(&mut s, 9.0, 9.0);
    let p: Vec<EntityId> = (0..4)
        .map(|i| point(&mut s, f64::from(i), 2.0 + f64::from(i * i)))
        .collect();
    let image = |s: &mut Sketch, of, map| {
        let id = EntityId(u32::try_from(s.entities.len()).unwrap());
        s.entities.push(Entity::Image { of, map });
        id
    };
    let chain = |from, to, k| ImageMap::Chain { from, to, k };
    let i1 = image(&mut s, p[0], chain(a1, a2, 2.0));
    let i2 = image(&mut s, p[1], chain(a1, a2, 2.0));
    let i3 = image(&mut s, p[2], chain(b1, b2, 1.0));
    let i4 = image(
        &mut s,
        p[3],
        ImageMap::Turn {
            center: Some(c),
            at: [0.0, 0.0],
            angle: 0.3,
        },
    );
    let l1 = s.add_line(i1, i2);
    let l2 = s.add_line(i3, i4);
    s.add_constraint(C::Parallel { a: l1, b: l2 });
    let first = solve(&s, &options(1.0)).unwrap();
    let mut next = s.clone();
    next.entities = first.entities.clone();
    next.image_starts = start(&next, i1, Start::Map);
    if let Some(Entity::Image {
        map: ImageMap::Chain { k, .. },
        ..
    }) = next.entities.get_mut(i1.0 as usize)
    {
        *k = 7.0;
    }
    let carried = solve(&next, &options(1.0)).unwrap();
    match &carried.carry {
        Some(Carried::Skipped { moved, .. }) => assert_eq!(moved, &vec![i1]),
        other => panic!("{other:?}"),
    }
    let mut plain = next.clone();
    plain.image_starts.clear();
    let plain = solve(&plain, &options(1.0)).unwrap();
    assert_eq!(carried.entities, plain.entities);
    assert_eq!(carried.constraints, plain.constraints);
}

/// Carrying never makes a sketch fail for want of work that solved
/// without it: at the smallest work limit at which the sketch solves from
/// its stored coordinates, it solves with the carrying too (which takes
/// half of that budget of its own and leaves the solve all of it).
#[test]
fn carrying_never_takes_the_work_the_solve_needs() {
    let w = wave(1.0, 3);
    let first = solve(&w.sketch, &options(1.0)).unwrap();
    // A path that halves its steps until it stops costs the most: `q`
    // pinned where it was cannot follow the run's end (as in
    // `a_path_that_stops_says_how_far_it_got`).
    let pin = |mut s: Sketch| {
        s.add_constraint(C::Fixed {
            point: w.q,
            at: Some([31.0, 0.0]),
        });
        s
    };
    for (to, pinned) in [(10, false), (400, false), (10, true), (400, true)] {
        let mut plain = recount(&w, &first, to, Start::None);
        let mut carried = recount(&w, &first, to, Start::Map);
        if pinned {
            plain = pin(plain);
            carried = pin(carried);
        }
        let with = |limit: u64| {
            let mut o = options(1.0);
            o.work_limit = limit;
            o
        };
        // The smallest limit at which the plain solve succeeds.
        let (mut lo, mut hi) = (0u64, 1u64 << 40);
        assert!(solve(&plain, &with(hi)).is_ok());
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if solve(&plain, &with(mid)).is_ok() {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        assert!(solve(&plain, &with(lo)).is_err());
        // Every limit from there to eight times it at which the plain
        // solve succeeds.
        for step in 0..=280u64 {
            let limit = hi + hi * step / 40;
            if solve(&plain, &with(limit)).is_err() {
                continue;
            }
            let solved = solve(&carried, &with(limit));
            assert!(
                solved.is_ok(),
                "3 -> {to} (pinned {pinned}) at limit {limit}: {solved:?}"
            );
        }
        // With room for the carrying it lands on the right root.
        if !pinned {
            let roomy = solve(&carried, &with(4 * hi + 1_000_000)).unwrap();
            assert_eq!(roomy.carry, Some(Carried::Whole), "3 -> {to}");
        }
    }
}

/// A carrying that runs out of its share of the work says so: at a work
/// limit just above what the plain solve needs, the carrying's half of it
/// does not reach the end of the jump 3 -> 400, while the solve after it
/// succeeds.
#[test]
fn a_path_that_runs_out_of_work_says_so() {
    let w = wave(1.0, 3);
    let first = solve(&w.sketch, &options(1.0)).unwrap();
    let carried = recount(&w, &first, 400, Start::Map);
    let with = |limit: u64| {
        let mut o = options(1.0);
        o.work_limit = limit;
        o
    };
    let found = (0..120u32)
        .map(|i| 2f64.powf(f64::from(i) / 3.0) as u64)
        .any(|limit| {
            matches!(
                solve(&carried, &with(limit)),
                Ok(Solution {
                    carry: Some(Carried::Stopped {
                        out_of_work: true,
                        ..
                    }),
                    ..
                })
            )
        });
    assert!(
        found,
        "no work limit at which the carrying runs out and the solve succeeds"
    );
}

/// A stored map whose numbers on today's points do not put the image
/// where it was (an edit moved the ends of the run as well) is not a path
/// from there: the image is carried in a straight line from its place,
/// exactly as with the place alone.
#[test]
fn a_map_that_does_not_fit_the_place_is_not_followed() {
    let w = wave(1.0, 3);
    let first = solve(&w.sketch, &options(1.0)).unwrap();
    let straight = recount(&w, &first, 10, Start::Place);
    let mut wrong = recount(&w, &first, 10, Start::Map);
    for start in &mut wrong.image_starts {
        // As if the run had had instance 5 there: 50 from the start.
        if let Some(ImageMap::Chain { k, .. }) = &mut start.map {
            *k = 5.0;
        }
    }
    assert_eq!(
        solve(&wrong, &options(1.0)),
        solve(&straight, &options(1.0))
    );
}

/// What the carrying costs: the run's count jumps 3 -> 400 and goes
/// 400 -> 401, solved with and without the image's old place (without it
/// the jump up lands on the other root, so only the time compares). Run
/// with `--ignored --nocapture` in an optimised build; the numbers are in
/// the dev log.
#[test]
#[ignore = "a timing, for optimised builds"]
fn carrying_a_jump_of_400_is_quick() {
    use std::time::Instant;
    let o = options(1.0);
    let w = wave(1.0, 3);
    let first = solve(&w.sketch, &o).unwrap();
    for carry in [Start::None, Start::Map] {
        let up = recount(&w, &first, 400, carry);
        let t = Instant::now();
        let mut solved = solve(&up, &o).unwrap();
        for _ in 1..100 {
            solved = solve(&up, &o).unwrap();
        }
        let jump = t.elapsed() / 100;
        let next = recount(&w, &solved, 401, carry);
        let t = Instant::now();
        for _ in 0..100 {
            solved = solve(&next, &o).unwrap();
        }
        let step = t.elapsed() / 100;
        let still = recount(&w, &solved, 401, carry);
        let t = Instant::now();
        for _ in 0..100 {
            solve(&still, &o).unwrap();
        }
        let none = t.elapsed() / 100;
        println!(
            "{carry:?}: 3 -> 400 {jump:?}, 400 -> 401 {step:?}, unmoved {none:?}, q at {:?}",
            solved.point(w.q)
        );
    }
}
