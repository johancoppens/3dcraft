//! The triangle budget (`TessParams::max_triangles`): a mesh within it is
//! the mesh made without one, bit for bit; a mesh over it is refused with
//! `TessError::OverBudget` before the rest of the body is tessellated; and
//! neither the verdict nor what it reports depends on the number of
//! threads.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Frame;
use craft3d_tess::{
    Mesh, TessError, TessParams, tessellate_body, tessellate_body_and_edges,
    tessellate_body_with_threads, tessellate_edges, tessellate_patches,
};
use craft3d_testkit::solids::Reference;
use craft3d_testkit::{solids, teapot, trimmed};

fn params(tolerance: f64, max_triangles: Option<usize>) -> TessParams {
    TessParams {
        chordal_tolerance: tolerance,
        max_triangles,
        ..TessParams::default()
    }
}

/// Analytic and freeform bodies, with one and with many faces.
fn fixtures() -> Vec<Reference> {
    let mut all = solids::all(&Frame::WORLD, 1.0).unwrap();
    all.extend(trimmed::all(&Frame::WORLD, 1.0).unwrap());
    all.push(trimmed::pipe_elbow(&Frame::WORLD, 3.0, 1.0, 1.9).unwrap());
    all
}

/// The triangles of each face of `mesh`, in face order.
fn per_face(mesh: &Mesh, faces: usize) -> Vec<usize> {
    let mut counts = vec![0; faces];
    for &face in &mesh.face_ids {
        counts[face as usize] += 1;
    }
    counts
}

const THREADS: [usize; 4] = [1, 2, 8, 16];

#[test]
fn a_mesh_within_the_budget_is_the_mesh_without_one() {
    for solid in fixtures() {
        for tolerance in [1e-2, 1e-3] {
            let free = tessellate_body(&solid.body, &params(tolerance, None)).unwrap();
            let count = free.triangles.len();
            // The count itself is within the budget; so is anything above.
            for budget in [count, count + 1, usize::MAX] {
                for threads in THREADS {
                    let bounded = tessellate_body_with_threads(
                        &solid.body,
                        &params(tolerance, Some(budget)),
                        threads,
                    )
                    .unwrap();
                    assert!(
                        bounded == free,
                        "{} at {tolerance}, budget {budget}, {threads} threads",
                        solid.name
                    );
                }
            }
            // With its edges too, and the edges alone take no budget.
            let both =
                tessellate_body_and_edges(&solid.body, &params(tolerance, Some(count))).unwrap();
            assert!(both.mesh.as_ref() == Ok(&free), "{}", solid.name);
            let edges = tessellate_edges(&solid.body, &params(tolerance, None)).unwrap();
            assert_eq!(both.edges, edges, "{}", solid.name);
            assert_eq!(
                tessellate_edges(&solid.body, &params(tolerance, Some(0))).unwrap(),
                edges,
                "{}",
                solid.name
            );
        }
    }
}

#[test]
fn a_mesh_over_the_budget_is_refused_the_same_on_any_number_of_threads() {
    let mut refused = 0;
    for solid in fixtures() {
        let tolerance = 1e-3;
        let free = tessellate_body(&solid.body, &params(tolerance, None)).unwrap();
        let count = free.triangles.len();
        let faces = per_face(&free, solid.body.faces.len());
        for budget in [0, 1, count / 7, count / 2, count - 1] {
            let single =
                tessellate_body_with_threads(&solid.body, &params(tolerance, Some(budget)), 1)
                    .unwrap_err();
            let TessError::OverBudget {
                triangles,
                faces: done,
                max_triangles,
            } = single
            else {
                panic!("{}: {single:?}", solid.name);
            };
            assert_eq!(max_triangles, budget);
            assert!(
                triangles > budget,
                "{}: {triangles} <= {budget}",
                solid.name
            );
            assert!(done >= 1 && done <= faces.len(), "{}: {done}", solid.name);
            // No more than what the whole mesh has: the rest was left out.
            assert!(triangles <= count, "{}: {triangles} > {count}", solid.name);
            for _ in 0..2 {
                for threads in THREADS {
                    let again = tessellate_body_with_threads(
                        &solid.body,
                        &params(tolerance, Some(budget)),
                        threads,
                    )
                    .unwrap_err();
                    assert_eq!(
                        again, single,
                        "{}, budget {budget}, {threads} threads",
                        solid.name
                    );
                }
            }
            // With the edges: the same refusal, and no half result.
            assert_eq!(
                tessellate_body_and_edges(&solid.body, &params(tolerance, Some(budget)))
                    .unwrap_err(),
                single,
                "{}",
                solid.name
            );
            refused += 1;
        }
    }
    assert!(refused >= 50, "{refused}");
}

/// What is reported is checked against the whole mesh, which does not
/// know of budgets: the shortest run of faces, from the first, whose
/// triangles exceed the budget. For bodies whose faces are settled in one
/// round (no face asks for finer edges), which the fixtures with straight
/// and circular edges are.
#[test]
fn the_refusal_names_the_first_faces_that_exceed_the_budget() {
    let mut checked = 0;
    for solid in fixtures() {
        let tolerance = 1e-3;
        let free = tessellate_body(&solid.body, &params(tolerance, None)).unwrap();
        let faces = per_face(&free, solid.body.faces.len());
        let count = free.triangles.len();
        for budget in [0, count / 3, count - 1] {
            let (mut sum, mut expected) = (0, 0);
            for &n in &faces {
                sum += n;
                expected += 1;
                if sum > budget {
                    break;
                }
            }
            let error = tessellate_body(&solid.body, &params(tolerance, Some(budget))).unwrap_err();
            let TessError::OverBudget {
                triangles,
                faces: done,
                ..
            } = error
            else {
                panic!("{}: {error:?}", solid.name);
            };
            // A round before the last may count a face differently; then
            // the report is of that round, and only its excess is checked
            // above.
            if (triangles, done) == (sum, expected) {
                checked += 1;
            } else {
                println!(
                    "{}: budget {budget}: {triangles} after {done} faces in an earlier round, \
                     {sum} after {expected} in the mesh",
                    solid.name
                );
            }
        }
    }
    assert!(checked >= 30, "{checked}");
}

#[test]
fn patches_stop_at_the_first_that_exceeds_the_budget() {
    let patches = teapot::teapot().unwrap();
    let tolerance = 1e-2;
    let free = tessellate_patches(&patches, &params(tolerance, None)).unwrap();
    let count = free.triangles.len();
    let each = per_face(&free, patches.len());
    for budget in [count, usize::MAX] {
        let bounded = tessellate_patches(&patches, &params(tolerance, Some(budget))).unwrap();
        assert!(bounded == free);
    }
    let two = each[0] + each[1];
    for (budget, faces, triangles) in [
        (0, 1, each[0]),
        (each[0] - 1, 1, each[0]),
        (each[0], 2, two),
        (count - 1, patches.len(), count),
    ] {
        assert_eq!(
            tessellate_patches(&patches, &params(tolerance, Some(budget))),
            Err(TessError::OverBudget {
                triangles,
                faces,
                max_triangles: budget,
            })
        );
    }
    let said = tessellate_patches(&patches, &params(tolerance, Some(10)))
        .unwrap_err()
        .to_string();
    assert!(said.contains("over the budget of 10"), "{said}");
}

/// The work a mesh counts toward an operation's bound
/// (`craft3d_geom::allowance`) is the same whatever the number of threads:
/// each worker's is added to the calling thread's when it is joined. So
/// is the verdict under a bound: a bound too small for the mesh ends it
/// with `TessError::Spent`, on every number of threads alike.
#[test]
fn the_work_a_mesh_counts_and_its_bound_do_not_depend_on_the_threads() {
    use craft3d_geom::allowance;
    let mut freeform = 0;
    for solid in fixtures() {
        let p = params(1e-3, None);
        let mut counts = Vec::new();
        for threads in THREADS {
            let before = allowance::spent();
            tessellate_body_with_threads(&solid.body, &p, threads).unwrap();
            counts.push(allowance::spent() - before);
        }
        assert!(
            counts.windows(2).all(|w| w[0] == w[1]),
            "{}: {counts:?}",
            solid.name
        );
        let whole = counts[0];
        if whole == 0 {
            continue;
        }
        freeform += 1;
        for threads in THREADS {
            let got = allowance::within(whole / 2, || {
                tessellate_body_with_threads(&solid.body, &p, threads)
            });
            // Asked after each round: what a round leaves undone of the
            // bound is not refused, but no more than that.
            match got {
                Err(TessError::Spent) | Ok(_) => {}
                Err(other) => panic!("{}: {other}", solid.name),
            }
            let zero =
                allowance::within(0, || tessellate_body_with_threads(&solid.body, &p, threads));
            assert!(
                matches!(zero, Err(TessError::Spent)),
                "{}: {zero:?}",
                solid.name
            );
        }
    }
    assert!(freeform > 0, "no fixture evaluated a freeform surface");
}

/// The self-check is held to its own bound (`SELF_CHECK_WORK`): a body
/// whose check needs more ends with `TessError::Spent`, which a caller
/// treats as a body it could not look at. Here a smaller bound around it
/// stands for a body that is dear to mesh.
#[test]
fn a_self_check_past_its_bound_ends_with_spent() {
    use craft3d_geom::allowance;
    use craft3d_tess::{SELF_CHECK_WORK, SelfCheckParams, body_self_intersections};
    let solid = trimmed::pipe_elbow(&Frame::WORLD, 3.0, 1.0, 1.9).unwrap();
    let params = SelfCheckParams::new(1e-6);
    let before = allowance::spent();
    let check = body_self_intersections(&solid.body, &params).unwrap();
    assert!(check.crossings.is_empty());
    let whole = allowance::spent() - before;
    assert!(whole > 0 && whole < SELF_CHECK_WORK / 10, "{whole}");
    let got = allowance::within(0, || body_self_intersections(&solid.body, &params));
    assert!(matches!(got, Err(TessError::Spent)), "{got:?}");
}
