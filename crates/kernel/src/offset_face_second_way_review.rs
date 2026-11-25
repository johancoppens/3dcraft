//! The review of the second way tried whole (`check/fuzz-timeouts`
//! 963416fb): [`offset_for_known`] held to itself as it was on `main`
//! ed93bafc, kept here word for word as the reference
//! (`reference_offset_for_known`); the only thing added to it is that it
//! says whether its second search ran and whether that built.
//!
//! A search builds only from its whole distance (`bounded_fitting` returns
//! a body from `attempt(1.0)` alone, and only where the bound was not
//! reached during it), and `main`'s second search is said only where it
//! builds, so trying the whole alone must give the same answer, body or
//! refusal, word for word, with what the refusal says fits. On the
//! random design 20802370's body (a ball and a ring united, filleted where
//! they meet), every face moved in, out, and all but the ring, with the
//! corners split (as the shell's hollow) and kept (as an offset and a
//! thickening): among them the ring's tube moved in as far as its radius
//! and further, where the rebuilt fillets make no body and the second way
//! is tried.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_arguments
)]

use super::*;
use crate::fillet::{FilletEdge, fillet};
use craft3d_math::{Frame, Vec3};
use std::cell::Cell;

/// `offset_for_known` of `main`, word for word, with `second` set to
/// whether its second search built, where it ran.
fn reference_offset_for_known(
    body: &Body,
    offsets: &[FaceOffset],
    blends: &Blends,
    known: Option<BTreeMap<FaceId, Fillet>>,
    corners: tweak::Corners,
    operation: &'static str,
    tolerance: &Tolerance,
    second: &Cell<Option<bool>>,
) -> Result<Body, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0 && tolerance.angular.is_finite() && tolerance.angular > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if body.kind != BodyKind::Solid {
        return Err(refuse(
            operation,
            Vec::new(),
            None,
            "the body is a sheet: offsetting, shelling or thickening a sheet is not built yet (only the faces of a solid can be moved)",
        ));
    }
    if offsets.len() > MAX_FACES {
        return Err(refuse(
            operation,
            Vec::new(),
            None,
            "more faces than one operation takes",
        ));
    }
    let mut largest = 0.0_f64;
    for (index, offset) in offsets.iter().enumerate() {
        if !offset.distance.is_finite() {
            return Err(KernelError::InvalidSize {
                name: "distance",
                value: offset.distance,
            });
        }
        if offsets
            .iter()
            .take(index)
            .any(|earlier| earlier.face == offset.face)
        {
            return Err(refuse(
                operation,
                vec![offset.face],
                None,
                "the face is given twice",
            ));
        }
        largest = largest.max(offset.distance.abs());
    }
    if offsets.is_empty() || largest <= tol {
        return Err(refuse(
            operation,
            Vec::new(),
            None,
            "there is nothing to move: no face, or a distance that cannot be told from none at this tolerance",
        ));
    }
    let defects = craft3d_topo::check(body, tolerance);
    if !defects.is_empty() {
        return Err(KernelError::InvalidBody {
            operation,
            defects: defects.into_iter().take(crate::MAX_REPORTED).collect(),
        });
    }
    let moving: BTreeMap<FaceId, f64> = offsets.iter().map(|o| (o.face, o.distance)).collect();
    let keys: BTreeSet<FaceId> = moving.keys().copied().collect();
    let fillets = match known {
        Some(known) => known,
        None => crate::offset_blend::fillets(body, &keys, &blends.radii, tolerance),
    };
    if let Some(faces) = crate::offset_blend::stays(&fillets, &moving) {
        return Err(refuse(operation, faces, None, crate::offset_blend::STAYS));
    }
    if let Some((part, face)) = crate::offset_blend::vanishing(&fillets, &moving, tolerance)
        && part < 1.0
    {
        let fits = part * largest;
        if fits.is_nan() || fits <= tol {
            return Err(refuse(
                operation,
                vec![face],
                None,
                crate::offset_blend::VANISHES,
            ));
        }
        return Err(KernelError::FaceOpTooFar {
            operation,
            faces: vec![face],
            edge: None,
            reason: crate::offset_blend::VANISHES,
            value: largest,
            largest: fits,
        });
    }
    let moved: Vec<FaceId> = offsets.iter().map(|o| o.face).collect();
    let search = |fillets: &BTreeMap<FaceId, Fillet>| {
        let unfolded = |fraction: f64| {
            offsets.iter().all(|offset| {
                if fillets.contains_key(&offset.face) {
                    return true;
                }
                let Some(face) = body.face(offset.face) else {
                    return true;
                };
                if !matches!(face.surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
                    return true;
                }
                let along = offset.distance * fraction * if face.reversed { -1.0 } else { 1.0 };
                craft3d_geom::offset::regular(&face.surface, along).is_ok()
            })
        };
        fitting_with(
            |fraction| {
                at(
                    body, offsets, fillets, fraction, corners, operation, tolerance,
                )
            },
            Some(&unfolded),
            largest,
            operation,
            tolerance,
            face_op_work(body, &moved),
        )
    };
    craft3d_geom::allowance::within(face_op_work(body, &moved), || {
        let first = search(&fillets);
        if first.is_ok() || fillets.is_empty() {
            return first;
        }
        match search(&BTreeMap::new()) {
            Ok(made) => {
                second.set(Some(true));
                Ok(made)
            }
            Err(_) => {
                second.set(Some(false));
                first
            }
        }
    })
}

/// The faces of `body` that use `edge`.
fn users(body: &Body, edge: EdgeId) -> Vec<usize> {
    (0..body.faces.len())
        .filter(|&f| {
            body.faces[f]
                .loops
                .iter()
                .filter_map(|&l| body.loop_(l))
                .flat_map(|l| &l.coedges)
                .any(|c| c.edge == edge)
        })
        .collect()
}

/// The body of the random design 20802370 before its shell: a ball of
/// radius 10 and a ring of radii 12.5 and 2.5 through it, united, and
/// filleted with 1.25 along every edge between them.
fn ball_and_ring(tolerance: &Tolerance) -> Option<Body> {
    let ball = crate::primitives::sphere(
        &Frame::new(Vec3::new(-9.9927, -7.5, 12.5), Vec3::Z, Vec3::X)?,
        10.0,
    )
    .ok()?;
    let ring = crate::primitives::torus(
        &Frame::new(Vec3::new(-18.7237, -1.2547, -2.5), Vec3::Y, Vec3::Z)?,
        12.5,
        2.5,
    )
    .ok()?;
    let united = crate::boolean::boolean(&ball, &ring, crate::boolean::BooleanOp::Union, tolerance)
        .ok()?
        .body;
    let edges: Vec<FilletEdge> = (0..united.edges.len())
        .filter_map(|e| {
            let edge = EdgeId(u32::try_from(e).ok()?);
            let faces = users(&united, edge);
            let on_ball = faces
                .iter()
                .any(|&f| matches!(united.faces[f].surface, Surface::Sphere(_)));
            let on_ring = faces
                .iter()
                .any(|&f| matches!(united.faces[f].surface, Surface::Torus(_)));
            (on_ball && on_ring).then_some(FilletEdge { edge, radius: 1.25 })
        })
        .collect();
    if edges.is_empty() {
        return None;
    }
    Some(fillet(&united, &edges, tolerance).ok()?.body)
}

/// Every face operation here answers as `main`'s did, body or refusal,
/// word for word, and where the second way is tried it asks no more.
#[test]
#[ignore = "the review's comparison with main's offset_for_known, minutes in release: cargo test --release -p craft3d-kernel --lib -- --ignored second_way_review --nocapture"]
fn the_second_way_tried_whole_answers_as_mains_halving_did() {
    let tolerance = Tolerance::new(1e-6, 1e-9).unwrap();
    let body = ball_and_ring(&tolerance).expect("the ball and the ring, united and filleted");
    let faces = body.faces.len();
    let ring: Vec<usize> = (0..faces)
        .filter(|&f| matches!(body.faces[f].surface, Surface::Torus(_)))
        .collect();
    let fillets = (0..faces)
        .filter(|&f| matches!(body.faces[f].surface, Surface::Nurbs(_)))
        .count();
    eprintln!(
        "{faces} faces, the ring's {ring:?}, {fillets} on splines: {:?}",
        body.faces
            .iter()
            .map(|f| match f.surface {
                Surface::Plane(_) => "plane",
                Surface::Cylinder(_) => "cylinder",
                Surface::Cone(_) => "cone",
                Surface::Sphere(_) => "sphere",
                Surface::Torus(_) => "torus",
                Surface::Bezier(_) | Surface::Nurbs(_) => "spline",
            })
            .collect::<Vec<_>>()
    );
    let moved = |distance: f64, but_ring: bool| -> Vec<FaceOffset> {
        (0..faces)
            .filter(|f| !(but_ring && ring.contains(f)))
            .map(|f| FaceOffset {
                face: FaceId(u32::try_from(f).unwrap()),
                distance,
            })
            .collect()
    };
    let cases = [
        (
            "every face in by 1, corners split",
            moved(-1.0, false),
            tweak::Corners::Split,
        ),
        (
            "every face in by 2.5 (the ring's tube), corners split",
            moved(-2.5, false),
            tweak::Corners::Split,
        ),
        (
            "every face in by 2.5, corners kept",
            moved(-2.5, false),
            tweak::Corners::Kept,
        ),
        (
            "every face in by 3, corners kept",
            moved(-3.0, false),
            tweak::Corners::Kept,
        ),
        (
            "every face out by 0.5, corners kept",
            moved(0.5, false),
            tweak::Corners::Kept,
        ),
        (
            "every face out by 2 (past the fillet), corners kept",
            moved(2.0, false),
            tweak::Corners::Kept,
        ),
        (
            "all but the ring in by 2.5, corners split",
            moved(-2.5, true),
            tweak::Corners::Split,
        ),
    ];
    let said = |result: &Result<Body, KernelError>| match result {
        Ok(made) => format!("a body of {} faces", made.faces.len()),
        Err(error) => error.to_string(),
    };
    let mut differ = Vec::new();
    let mut tried = 0_usize;
    for (name, offsets, corners) in &cases {
        let (new, new_units) = craft3d_geom::allowance::measured(|| {
            offset_for_known(
                &body,
                offsets,
                &Blends::default(),
                None,
                *corners,
                OPERATION,
                &tolerance,
            )
        });
        let second = Cell::new(None);
        let (old, old_units) = craft3d_geom::allowance::measured(|| {
            reference_offset_for_known(
                &body,
                offsets,
                &Blends::default(),
                None,
                *corners,
                OPERATION,
                &tolerance,
                &second,
            )
        });
        // The change once more, after `main`'s way warmed the caches: an
        // answer that depends on what ran before is no difference of the
        // change's.
        let again = offset_for_known(
            &body,
            offsets,
            &Blends::default(),
            None,
            *corners,
            OPERATION,
            &tolerance,
        );
        eprintln!(
            "{name}: {} in {new_units} units; main: {} in {old_units} units, its second search {:?}; again the same: {}",
            said(&new),
            said(&old),
            second.get(),
            again == new
        );
        if new != old && again != old {
            differ.push(format!(
                "{name}: {}, where main says {}",
                said(&new),
                said(&old)
            ));
        }
        if second.get().is_some() {
            tried += 1;
            if new_units > old_units {
                differ.push(format!(
                    "{name}: {new_units} units, where main asked {old_units}"
                ));
            }
        }
    }
    assert!(differ.is_empty(), "{differ:?}");
    assert!(tried > 0, "no case where the second way is tried");
}
