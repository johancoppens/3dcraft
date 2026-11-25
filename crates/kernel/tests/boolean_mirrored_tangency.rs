//! A shell united with its mirror image, where the hollow's floor comes
//! tangent to its hollowed rod (random design 114519;
//! docs/design/booleans.md › *Two operands' edges placed apart on one
//! tangency line*).
//!
//! The solid: a ball of radius 5 about (5, 10, 12.5) with the block
//! x 0..2.5, y 5..20, z 10..17.5 taken out of it, joined to a rod of
//! radius 5 along x about the line y = 7.5, z = 12.5 (x -10..2.5); as the
//! design makes it, built below z = 5 and mirrored in it. Shelled by t,
//! open at the ball and at the block's cut face x = 2.5, the hollow's floor
//! (the plane z = 10 - t) and its rod (radius 5 - t) meet where
//! (y - 7.5)² = 18.75 - 15 t, and the shell keeps the line at +: at
//! t = 1.25 the plane is tangent to the cylinder along its lowest line
//! y = 7.5, z = 8.75 (the seam), and the floor and the rod's inside meet
//! there at no angle; within a band √(2 r tol) = 2.7e-3 wide about the
//! seam they are within the tolerance of each other. The shell is mirrored
//! in the plane y = 7.5, through the rod's axis (the same cylinder, the
//! same floor plane), and combined with itself.
//!
//! The design's shell at 1.25 puts the floor's edge 2.4e-4 along the band
//! (Newton's steps stop where the plane is within the tolerance), and its
//! mirror image puts its own as far on the other side; 0.1 to 0.5 of a
//! tolerance thinner, the two cross for real 1.2e-3 to 2.7e-3 from the
//! seam. Either way each operand's floor edge lies on the other's rod face
//! within the tolerance and divides it there, and on `main` nothing
//! imprinted it: the floor and the other's rod meet in the closed form's
//! tangent line on the seam, outside the floor's face. The boolean built
//! an invalid body for the union, the difference and the common part, at
//! every scale. Now such an edge, a fold (the floor and the rod meet there
//! at no angle) of a face that lies on the other body's face, is imprinted
//! there from itself (`section::fold_edges`).
//!
//! Held to, at scales 1e-3, 1, 1e3 and 1e6, as built and turned out of the
//! axes: the deep check of every body built and the boolean's full check
//! (its volume rules and seeded points); the identities of sets; and the
//! volumes against the same operations on shells 400 and 1600 tolerances
//! thinner, where the floor and the rod cross at more than a hundredth of
//! a radian and no rule of the band reaches, carried on along the
//! thickness: the volume is smooth in it but for the cusps' sections, of
//! the order of (k tol)^(3/2) √r, a billionth of the bound. Where the two
//! edges lie on the seam itself (the solid built where the mirror puts
//! it, whose shell finds the vertex on the seam to rounding), the two
//! cusps between floor and rod touch along it: refused as zero thickness,
//! as on `main`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, VerifyOptions, boolean, verify};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, FaceId, check, mass_properties};

const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

/// As built, and turned out of the axes.
fn turns() -> [Transform; 2] {
    [
        Transform::translation(Vec3::ZERO),
        Transform::rotation(Vec3::new(1.0, 1.0, 1.0), 0.3)
            .unwrap()
            .then(&Transform::rotation(Vec3::new(0.2, -1.0, 0.5), 1.1).unwrap()),
    ]
}

/// The design's tolerance, 1e-6 at scale 1.
fn tolerance(s: f64) -> Tolerance {
    Tolerance {
        linear: 1e-6 * s,
        ..Tolerance::default()
    }
}

fn at(x: f64, y: f64, z: f64) -> Frame {
    Frame::WORLD.at(Vec3::new(x, y, z)).unwrap()
}

/// The solid the design shells, at scale `s`: built below z = 5 and
/// mirrored in it, as the design does (`mirrored`), or built where the
/// mirror puts it.
fn solid(s: f64, tol: &Tolerance, mirrored: bool) -> Body {
    let (zb, zc) = if mirrored { (-7.5, -2.5) } else { (10.0, 12.5) };
    let block = primitives::block(&at(0.0, 5.0 * s, zb * s), 2.5 * s, 15.0 * s, 7.5 * s).unwrap();
    let ball = primitives::sphere(&at(5.0 * s, 10.0 * s, zc * s), 5.0 * s).unwrap();
    let cut = boolean(&ball, &block, BooleanOp::Subtract, tol).unwrap();
    let axis = Frame::new(
        Vec3::new(2.5 * s, 7.5 * s, zc * s),
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    )
    .unwrap();
    let rod = primitives::cylinder(&axis, 5.0 * s, 12.5 * s).unwrap();
    let joined = boolean(&rod, &cut.body, BooleanOp::Union, tol)
        .unwrap()
        .body;
    if !mirrored {
        return joined;
    }
    let mirror = Transform::mirror(Vec3::new(0.0, 0.0, 5.0 * s), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    joined.transformed(&mirror).unwrap()
}

/// The faces the design leaves open: the ball, and the block's cut face
/// x = 2.5 (facing -x).
fn open_faces(body: &Body, s: f64) -> Vec<FaceId> {
    let open: Vec<FaceId> = body
        .faces
        .iter()
        .enumerate()
        .filter(|(_, f)| match &f.surface {
            Surface::Sphere(_) => true,
            Surface::Plane(p) => {
                let outward = p.frame().z() * if f.reversed { -1.0 } else { 1.0 };
                outward.x < -0.99 && (p.frame().origin().x - 2.5 * s).abs() < 1e-9 * s
            }
            _ => false,
        })
        .map(|(i, _)| FaceId(u32::try_from(i).unwrap()))
        .collect();
    assert_eq!(open.len(), 2, "the ball and the cut face");
    open
}

/// The solid shelled by `t` (at scale 1; `k` tolerances less than 1.25),
/// turned by `turn`.
fn shelled(s: f64, k: f64, mirrored: bool, turn: &Transform) -> Body {
    let tol = tolerance(s);
    let local = solid(s, &tol, mirrored);
    let open = open_faces(&local, s);
    let body = local.transformed(turn).unwrap();
    let t = 1.25 - k * 1e-6;
    craft3d_kernel::shell::shell(&body, t * s, &open, &tol)
        .unwrap_or_else(|e| panic!("scale {s:e}, k {k}, mirrored {mirrored}: {e}"))
        .body
}

/// `body`'s mirror image in the plane y = 7.5 (turned with it).
fn mirror_image(body: &Body, s: f64, turn: &Transform) -> Body {
    let point = turn.point(Vec3::new(0.0, 7.5 * s, 0.0));
    let normal = turn.vector(Vec3::new(0.0, 1.0, 0.0));
    body.transformed(&Transform::mirror(point, normal).unwrap())
        .unwrap()
}

/// The vertices of `body` within 0.01 s of the seam (y = 7.5 s,
/// z = 8.75 s), as (x, y - 7.5 s, z - 8.75 s) in the solid's own axes.
fn near_the_seam(body: &Body, s: f64, turn: &Transform) -> Vec<Vec3> {
    let back = turn.inverse().unwrap();
    body.vertices
        .iter()
        .map(|v| back.point(v.point))
        .map(|p| Vec3::new(p.x, p.y - 7.5 * s, p.z - 8.75 * s))
        .filter(|d| d.y.abs() < 0.01 * s && d.z.abs() < 0.01 * s)
        .collect()
}

/// Where the floor and the rod cross `k` tolerances below the tangency,
/// from the seam, in units of the scale: √(15 k 1e-6) (above).
fn crossing(k: f64) -> f64 {
    (15.0 * k * 1e-6).sqrt()
}

/// The half width of the band about the seam where floor and rod are
/// within the tolerance of each other, in units of the scale: √(2 r tol).
fn band() -> f64 {
    (2.0 * 3.75 * 1e-6_f64).sqrt()
}

/// Thicknesses below the tangency, in tolerances, at which the floor and
/// the rod cross within the band: the union, the difference and the common
/// part of the shell and its mirror image were invalid bodies on `main`.
const IN_THE_BAND: [f64; 3] = [0.1, 0.25, 0.5];

/// Thicknesses below the tangency, in tolerances, at which the floor and
/// the rod cross at more than a hundredth of a radian (√(15 k 1e-6) / 3.75
/// over 0.02): the oracle's.
const FAR: [f64; 2] = [400.0, 1600.0];

/// The volumes of the union, the difference and the common part of the
/// mirror image and the shell `k` tolerances below the tangency, each
/// built and held to the deep check and the boolean's full check; and the
/// shell's volume and area.
fn three(s: f64, k: f64, turn: &Transform) -> ([f64; 3], f64, f64) {
    let tol = tolerance(s);
    let shell = shelled(s, k, true, turn);
    let image = mirror_image(&shell, s, turn);
    let mut volumes = [0.0; 3];
    for (slot, op) in
        volumes
            .iter_mut()
            .zip([BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect])
    {
        let case = format!("scale {s:e}, k {k}, {op:?}, {turn:?}");
        let made = boolean(&image, &shell, op, &tol).unwrap_or_else(|e| panic!("{case}: {e}"));
        let defects = check(&made.body, &tol);
        assert!(defects.is_empty(), "{case}: {defects:?}");
        let checked = verify(
            &image,
            &shell,
            op,
            &made.body,
            &tol,
            &VerifyOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{case}: {e}"));
        assert!(
            checked.violations.is_empty(),
            "{case}: {:?}",
            checked.violations
        );
        *slot = mass_properties(&made.body).unwrap().volume;
    }
    let props = mass_properties(&shell).unwrap();
    (volumes, props.volume, props.area)
}

/// The three operations `k` tolerances below the tangency against the
/// identities of sets and against the same operations far from it,
/// carried on along the thickness.
fn held(s: f64, k: f64, turn: &Transform, far: &[[f64; 3]]) {
    let tol = tolerance(s).linear;
    let case = format!("scale {s:e}, k {k}, {turn:?}");
    let ([union, difference, common], shell, area) = three(s, k, turn);
    let bound = tol * area;
    // The mirror image has the shell's volume.
    assert!(
        (union + common - 2.0 * shell).abs() <= 2.0 * bound,
        "{case}: union and common part"
    );
    assert!(
        (difference + common - shell).abs() <= bound,
        "{case}: difference and common part"
    );
    for (index, got) in [union, difference, common].into_iter().enumerate() {
        let (near, farther) = (far[0][index], far[1][index]);
        let slope = (farther - near) / (FAR[1] - FAR[0]);
        let carried = near + slope * (k - FAR[0]);
        assert!(
            (got - carried).abs() <= bound,
            "{case}: operation {index}: {got} against {carried} carried on ({:e} of the bound)",
            (got - carried).abs() / bound
        );
    }
}

/// The far operations' volumes, at a scale and a turn.
fn far(s: f64, turn: &Transform) -> Vec<[f64; 3]> {
    FAR.iter().map(|&k| three(s, k, turn).0).collect()
}

/// The design's own shell (1.25, its floor's edge 2.4e-4 along the band)
/// with its mirror image: the union, the difference and the common part
/// are built (on `main` each an invalid body), and held as above.
#[test]
fn the_designs_shell_and_its_mirror_image_are_combined() {
    for s in SCALES {
        for turn in &turns() {
            let shell = shelled(s, 0.0, true, turn);
            let near = near_the_seam(&shell, s, turn);
            assert!(near.len() >= 2, "scale {s:e}: {near:?}");
            for d in &near {
                assert!(
                    d.y.abs() <= band() * s,
                    "scale {s:e}: {d:?} outside the band"
                );
            }
            held(s, 0.0, turn, &far(s, turn));
        }
    }
}

/// Shelled a fraction of a tolerance thinner, the floor and the rod cross
/// within the band, the shell's edge where they cross (in closed form)
/// and its mirror image's on the seam's other side: the union, the
/// difference and the common part are built (on `main` each an invalid
/// body), and held as above.
#[test]
fn a_shell_and_its_mirror_image_with_their_edges_apart_in_the_band() {
    for s in SCALES {
        let tol = tolerance(s).linear;
        for turn in &turns() {
            let far = far(s, turn);
            for k in IN_THE_BAND {
                let shell = shelled(s, k, true, turn);
                let near = near_the_seam(&shell, s, turn);
                assert!(near.len() >= 2, "scale {s:e} k {k}: {near:?}");
                for d in &near {
                    assert!(
                        (d.y - crossing(k) * s).abs() <= 1e-2 * tol,
                        "scale {s:e} k {k}: a vertex at {:e}, the crossing at {:e}",
                        d.y / s,
                        crossing(k)
                    );
                }
                held(s, k, turn, &far);
            }
        }
    }
}

/// The solid built where the mirror puts it: its shell at 1.25 finds the
/// floor's edge on the seam (to rounding), and so does its mirror image,
/// so the two cusps between floor and rod touch along the seam: the union
/// and the difference are refused as zero thickness at a point of it (as
/// on `main`), the common part is built and passes the checks.
#[test]
fn on_the_seam_the_two_cusps_touch() {
    let (s, turn) = (1.0, Transform::translation(Vec3::ZERO));
    let tol = tolerance(s);
    let shell = shelled(s, 0.0, false, &turn);
    let near = near_the_seam(&shell, s, &turn);
    assert!(near.len() >= 2, "{near:?}");
    for d in &near {
        assert!(
            d.y.abs() <= 1e-2 * tol.linear && d.z.abs() <= tol.linear,
            "{d:?}"
        );
    }
    let image = mirror_image(&shell, s, &turn);
    for op in [BooleanOp::Union, BooleanOp::Subtract] {
        match boolean(&image, &shell, op, &tol) {
            Err(KernelError::ZeroThickness { at, .. }) => assert!(
                (at.y - 7.5).abs() <= tol.linear && (at.z - 8.75).abs() <= tol.linear,
                "{op:?}: zero thickness at {at:?}, off the seam"
            ),
            Err(other) => panic!("{op:?}: {other}"),
            Ok(made) => panic!("{op:?}: built, {} faces", made.body.faces.len()),
        }
    }
    let common = boolean(&image, &shell, BooleanOp::Intersect, &tol).unwrap();
    assert!(check(&common.body, &tol).is_empty());
    let checked = verify(
        &image,
        &shell,
        BooleanOp::Intersect,
        &common.body,
        &tol,
        &VerifyOptions::default(),
    )
    .unwrap();
    assert!(checked.violations.is_empty(), "{:?}", checked.violations);
}
