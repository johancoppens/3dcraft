//! Image points: derived points with no unknowns of their own, the image
//! of a point under a shift, a chained run or a turn (one instance of a
//! pattern inside a sketch). They add no degrees of freedom; a constraint
//! on one acts on the points it is made from, and the solver finds the
//! step of a run from where its last instance has to end.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Tolerance;
use craft3d_solve::{
    Constraint as C, ConstraintStatus, Entity, EntityId, ImageMap, Sketch, SolveError,
    SolveOptions, solve,
};

fn options(scale: f64) -> SolveOptions {
    SolveOptions::new(Tolerance::new(1e-9 * scale, 1e-9).unwrap())
}

fn point(solution: &craft3d_solve::Solution, id: EntityId) -> [f64; 2] {
    solution.point(id).unwrap()
}

/// A run from A (fixed at the origin) to B, repeated n times along x: the
/// end of the last instance, `B + (n - 1)(B - A)`, coincident with a fixed
/// point C. The solver finds `B = A + (C - A) / n`.
#[test]
fn the_solver_finds_the_step_of_a_run_from_its_end() {
    for scale in [1e-3, 1.0, 1e3] {
        for n in [1_u32, 2, 3, 7, 40, 400] {
            let mut s = Sketch::new();
            let a = s.add_point(0.0, 0.0);
            let b = s.add_point(1.0 * scale, 0.3 * scale);
            let c = s.add_point(97.0 * scale, 0.0);
            let last = EntityId(u32::try_from(s.entities.len()).unwrap());
            s.entities.push(Entity::Image {
                of: b,
                map: ImageMap::Chain {
                    from: a,
                    to: b,
                    k: f64::from(n - 1),
                },
            });
            s.add_constraint(C::Fixed { point: a, at: None });
            s.add_constraint(C::Fixed { point: c, at: None });
            s.add_constraint(C::Coincident { a: c, b: last });
            let solved = solve(&s, &options(scale)).unwrap();
            assert!(solved.is_consistent());
            assert_eq!(solved.dof, 0, "the image adds no unknowns");
            assert_eq!(solved.unknowns, 6);
            let [bx, by] = point(&solved, b);
            let want = 97.0 * scale / f64::from(n);
            assert!(
                ((bx - want) / want).abs() <= 1e-12,
                "n {n} at {scale}: {bx} vs {want}"
            );
            assert!(by.abs() <= 1e-12 * scale, "{by}");
        }
    }
}

/// A turn about a point that is solved too: the image of A turned by the
/// angle has to be B, which fixes the centre on the bisector, and a
/// distance fixes it along that.
#[test]
fn a_turn_about_a_solved_centre_moves_the_centre() {
    let mut s = Sketch::new();
    let a = s.add_point(1.0, 0.0);
    let b = s.add_point(0.0, 1.0);
    let o = s.add_point(0.2, 0.1);
    let turned = EntityId(u32::try_from(s.entities.len()).unwrap());
    s.entities.push(Entity::Image {
        of: a,
        map: ImageMap::Turn {
            center: Some(o),
            at: [0.0, 0.0],
            angle: std::f64::consts::FRAC_PI_2,
        },
    });
    s.add_constraint(C::Fixed { point: a, at: None });
    s.add_constraint(C::Fixed { point: b, at: None });
    s.add_constraint(C::Coincident { a: turned, b });
    let solved = solve(&s, &options(1.0)).unwrap();
    assert!(solved.is_consistent());
    // A quarter turn takes (1, 0) to (0, 1) about the origin only.
    let [ox, oy] = point(&solved, o);
    assert!(ox.abs() < 1e-12 && oy.abs() < 1e-12, "{ox} {oy}");
    assert_eq!(solved.dof, 0);
}

/// A constraint the image repeats is redundant, one it cannot meet
/// conflicts: blame works through image points like any other.
#[test]
fn redundancy_and_conflict_through_images() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let shifted = EntityId(1);
    s.entities.push(Entity::Image {
        of: a,
        map: ImageMap::Shift { offset: [5.0, 0.0] },
    });
    let b = s.add_point(5.0, 0.0);
    s.add_constraint(C::Fixed { point: a, at: None });
    s.add_constraint(C::Coincident { a: shifted, b });
    s.add_constraint(C::Distance { a, b, value: 5.0 });
    s.add_constraint(C::HorizontalDistance { a, b, value: 4.0 });
    let solved = solve(&s, &options(1.0)).unwrap();
    let states: Vec<ConstraintStatus> = solved.constraints.iter().map(|c| c.status).collect();
    assert_eq!(
        states,
        [
            ConstraintStatus::Satisfied,
            ConstraintStatus::Satisfied,
            ConstraintStatus::Redundant,
            ConstraintStatus::Conflicting
        ]
    );
}

#[test]
fn an_image_of_what_is_not_a_point_is_refused() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(1.0, 0.0);
    let line = s.add_line(a, b);
    s.entities.push(Entity::Image {
        of: line,
        map: ImageMap::Shift { offset: [1.0, 0.0] },
    });
    assert!(matches!(
        solve(&s, &options(1.0)),
        Err(SolveError::WrongKind { .. })
    ));
    // An image of an image (nested patterns) is the image of where the
    // first one is; an image of one after it, of itself, or more than
    // eight deep is refused. One of a missing point is refused too.
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    s.entities.push(Entity::Image {
        of: a,
        map: ImageMap::Shift { offset: [1.0, 0.0] },
    });
    s.entities.push(Entity::Image {
        of: EntityId(1),
        map: ImageMap::Shift { offset: [1.0, 0.5] },
    });
    let solved = solve(&s, &options(1.0)).unwrap();
    assert_eq!(solved.point(EntityId(2)), None);
    for (of, at) in [(1, 1), (2, 1), (2, 2)] {
        let mut wrong = Sketch::new();
        wrong.add_point(0.0, 0.0);
        wrong.entities.push(Entity::Image {
            of: EntityId(of),
            map: ImageMap::Shift { offset: [1.0, 0.0] },
        });
        wrong.entities.push(Entity::Image {
            of: EntityId(at),
            map: ImageMap::Shift { offset: [1.0, 0.0] },
        });
        assert!(
            matches!(
                solve(&wrong, &options(1.0)),
                Err(SolveError::WrongKind { .. })
            ),
            "{of} {at}"
        );
    }
    let mut deep = Sketch::new();
    deep.add_point(0.0, 0.0);
    for i in 0..9 {
        deep.entities.push(Entity::Image {
            of: EntityId(i),
            map: ImageMap::Shift { offset: [1.0, 0.0] },
        });
    }
    assert!(matches!(
        solve(&deep, &options(1.0)),
        Err(SolveError::InvalidEntity { entity: 9, .. })
    ));
    deep.entities.pop();
    assert!(solve(&deep, &options(1.0)).is_ok());
    let mut s = Sketch::new();
    s.entities.push(Entity::Image {
        of: EntityId(5),
        map: ImageMap::Shift {
            offset: [f64::NAN, 0.0],
        },
    });
    assert!(solve(&s, &options(1.0)).is_err());
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    s.entities.push(Entity::Image {
        of: a,
        map: ImageMap::Turn {
            center: None,
            at: [0.0, 0.0],
            angle: f64::INFINITY,
        },
    });
    assert!(matches!(
        solve(&s, &options(1.0)),
        Err(SolveError::NonFinite { .. })
    ));
}

/// Solving again from a solution moves nothing, with image points in it.
#[test]
fn solving_a_solved_sketch_with_images_moves_nothing() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(1.1, 0.2);
    let c = s.add_point(33.3, 4.4);
    let last = EntityId(3);
    s.entities.push(Entity::Image {
        of: b,
        map: ImageMap::Chain {
            from: a,
            to: b,
            k: 10.0,
        },
    });
    s.add_constraint(C::Fixed { point: a, at: None });
    s.add_constraint(C::Fixed { point: c, at: None });
    s.add_constraint(C::Coincident { a: c, b: last });
    let first = solve(&s, &options(1.0)).unwrap();
    let again = Sketch {
        entities: first.entities.clone(),
        ..s.clone()
    };
    let second = solve(&again, &options(1.0)).unwrap();
    assert_eq!(second.entities, first.entities);
    assert_eq!(second.iterations, 0);
}

/// Nested chained runs: a run from A (fixed) to B repeated `k` times, its
/// end `Y = A + k (B - A)` (an image of B), and that whole run repeated `n`
/// times from A to Y: the end of the last cell, an image of the image,
/// `A + n k (B - A)`, coincident with a fixed point C. The solver finds
/// `B = A + (C - A) / (n k)`, with exact gradients through both levels.
#[test]
fn the_solver_finds_the_step_of_a_run_of_runs_from_its_end() {
    for scale in [1e-3, 1.0, 1e6] {
        for (k, n) in [(1_u32, 1_u32), (2, 3), (3, 7), (6, 10), (1, 40)] {
            let mut s = Sketch::new();
            let a = s.add_point(0.0, 0.0);
            let b = s.add_point(1.0 * scale, 0.3 * scale);
            let c = s.add_point(97.0 * scale, 0.0);
            let y = EntityId(u32::try_from(s.entities.len()).unwrap());
            s.entities.push(Entity::Image {
                of: b,
                map: ImageMap::Chain {
                    from: a,
                    to: b,
                    k: f64::from(k - 1),
                },
            });
            let end = EntityId(u32::try_from(s.entities.len()).unwrap());
            s.entities.push(Entity::Image {
                of: y,
                map: ImageMap::Chain {
                    from: a,
                    to: y,
                    k: f64::from(n - 1),
                },
            });
            s.add_constraint(C::Fixed { point: a, at: None });
            s.add_constraint(C::Fixed { point: c, at: None });
            s.add_constraint(C::Coincident { a: c, b: end });
            let solved = solve(&s, &options(scale)).unwrap();
            assert!(solved.is_consistent());
            assert_eq!(solved.dof, 0, "images add no unknowns");
            let [bx, by] = point(&solved, b);
            let want = 97.0 * scale / f64::from(n * k);
            assert!(
                ((bx - want) / want).abs() <= 1e-12 && by.abs() <= 1e-12 * scale,
                "k {k} n {n} at {scale}: {bx} {by} vs {want}"
            );
        }
    }
}
