//! The review of the folds imprinted from themselves (2026-11-25,
//! `section::fold_edges`, `not_imprinted`, `Lying::Fold`; the repair's own
//! tests are `boolean_mirrored_tangency.rs`).
//!
//! Where a fold (an edge whose two faces meet at no angle, their outward
//! normals opposite) belongs to a face that lies on a face of the other
//! body, the repair imprints it from itself unless the stretches already on
//! the other face run along it. These tests put folds on faces that lie on
//! another body's faces in ways the repair's tests do not, and hold every
//! result to the deep check, the boolean's full check (volumes, seeded
//! points), and where one exists to a closed form or to the identities of
//! sets; every case prints one line (`CASE ...`), so that a run on the
//! branch and a run on `main` can be told apart line by line.
//!
//! - *A block with a hole tangent to its side* (a wall that thins to
//!   nothing along a line: the side and the hole meet there at no angle),
//!   and the hole a little into the side (two edges where they cross at a
//!   few thousandths of a radian, folds by the repair's sine, or at more
//!   than a hundredth, not): with a copy moved along the fold (their sides
//!   and holes on one surface each, their folds on one line, half of each
//!   shared), with its mirror image in the plane through the hole's axis
//!   (the same body, every curve the other way), and with both. Held to
//!   closed forms (a disc less the segment the side cuts off) and to the
//!   same sets made from plain blocks: `(P \ D) ∪ (Q \ D) = (P ∪ Q) \ D`.
//! - *The design of random design 114519, mixed*: its shell (floor edge
//!   2.4e-4 along the seam) against the shell of the solid built where the
//!   mirror puts it (floor edge on the seam) and their mirror images; and
//!   against the mirror image of a shell a quarter of a tolerance thinner
//!   (its edge where floor and rod cross, 1.9e-3 the other way).
//! - *The design with its rod's seam on the tangency line*: the rod's
//!   cylinder built with its seam at its lowest line, where both floors
//!   end, or at its top: the folds then lie beside the other face's seam.
//! - *The design with a copy of itself along the rod* (a linear pattern
//!   whose instances overlap): the folds on one line, partly shared.
//!
//! Measured (Checks 45663956210 on the branch and 45663962868 on `main`
//! ed93bafc, 333 cases each): 262 the same line for line on both (every
//! block with a tangent hole and its copies bit for bit: where the folds
//! are exact nothing changes); 65 an invalid body or a refusal on `main`
//! and on the branch built and held (62) or refused as zero thickness
//! where the shell whose floor ends on the seam is cut by the mirror image
//! of the other (3): the copies along the rod (32, on `main` invalid bodies
//! and *"an edge shared by faces that do not pair up"*), the shells mixed
//! with a quarter of a tolerance thinner (24), and the shell on the seam
//! with the other's mirror image (9; at 1e3 turned that shell is refused,
//! *"too far … below 1249.99"*, on both). Refused on both in the same words:
//! the design's shell with the shell on the seam on the same side (`X+Y`,
//! `X'+Y'`, *"an imprinted face could not be divided into regions"*,
//! owed); and with the rod's cylinder built with its seam at its top and
//! bottom, the shell itself at 1.25 and 1.25 less a quarter of a
//! tolerance, *"the edge shrinks to nothing … below 0.885611"*, at both
//! scales and turned (the shell's answer depends on where the cylinder's
//! parametric seam is; owed, not the boolean's).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;

use boolean_common::{boxed, rod};
use craft3d_geom::Surface;
use craft3d_kernel::boolean::{BooleanOp, VerifyOptions, boolean, verify};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Portable, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, FaceId, check, mass_properties};

const OPS: [BooleanOp; 3] = [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect];

/// What a body and its mirror image that is the same set are held to.
const UNION_COMMON: [BooleanOp; 2] = [BooleanOp::Union, BooleanOp::Intersect];

/// As built, and turned out of the axes.
fn turns() -> [Transform; 2] {
    [
        Transform::translation(Vec3::ZERO),
        Transform::rotation(Vec3::new(1.0, 1.0, 1.0), 0.3)
            .unwrap()
            .then(&Transform::rotation(Vec3::new(0.2, -1.0, 0.5), 1.1).unwrap()),
    ]
}

/// A millionth of the unit.
fn tolerance(s: f64) -> Tolerance {
    Tolerance {
        linear: 1e-6 * s,
        ..Tolerance::default()
    }
}

/// One boolean, as one line: `built` with its volume and counts, or the
/// refusal's words (`INVALID` where they are the boolean's own check
/// refusing an invalid body), or `WRONG` where a body was built and fails
/// the deep check, the full check or `want` (a volume, within the
/// tolerance times the result's area). The volume, for the identities.
fn outcome(
    a: &Body,
    b: &Body,
    op: BooleanOp,
    tol: &Tolerance,
    want: Option<f64>,
) -> (String, Option<f64>) {
    let made = match boolean(a, b, op, tol) {
        Ok(made) => made,
        Err(e) => {
            let words = e.to_string();
            let short: String = words.chars().take(220).collect();
            let kind = if words.contains("invalid body") {
                "INVALID"
            } else {
                "refused"
            };
            return (format!("{kind}: {short}"), None);
        }
    };
    let defects = check(&made.body, tol);
    if let Some(first) = defects.first() {
        return (format!("WRONG: {} defects, {first:?}", defects.len()), None);
    }
    match verify(a, b, op, &made.body, tol, &VerifyOptions::default()) {
        Err(e) => return (format!("WRONG: the full check cannot run: {e}"), None),
        Ok(found) => {
            if let Some(first) = found.violations.first() {
                return (format!("WRONG: the full check: {first}"), None);
            }
        }
    }
    let props = match mass_properties(&made.body) {
        Ok(props) => props,
        Err(e) => return (format!("WRONG: no mass properties: {e}"), None),
    };
    let counts = format!(
        "V {:.12e} A {:.9e} F {} E {} V {}",
        props.volume,
        props.area,
        made.body.faces.len(),
        made.body.edges.len(),
        made.body.vertices.len()
    );
    if let Some(want) = want {
        let bound = tol.linear * props.area.max(1e-300);
        let off = (props.volume - want).abs();
        if off.is_nan() || off > bound {
            return (
                format!(
                    "WRONG: volume {} against {want} ({:.3e} of the bound); {counts}",
                    props.volume,
                    off / bound
                ),
                Some(props.volume),
            );
        }
    }
    (format!("built: {counts}"), Some(props.volume))
}

/// Prints the case and keeps a line for the failures at the end.
fn record(case: &str, line: &str, bad: &mut Vec<String>) {
    println!("CASE {case}: {line}");
    if line.starts_with("WRONG") || line.starts_with("INVALID") {
        bad.push(format!("{case}: {line}"));
    }
}

fn verdict(bad: &[String]) {
    assert!(
        bad.is_empty(),
        "{} cases built a wrong body or an invalid one:\n  {}",
        bad.len(),
        bad.join("\n  ")
    );
}

// ---------------------------------------------------------------------
// A block with a hole tangent to its side.
// ---------------------------------------------------------------------

/// The area of the unit disc about (1 + d, 0) on the side x >= 0 of the
/// plane x = 0 (d > -1): the disc less the segment the plane cuts off.
fn disc_inside(d: f64) -> f64 {
    let pi = std::f64::consts::PI;
    if d >= 0.0 {
        return pi;
    }
    let h = -d;
    let c = 1.0 - h;
    let segment = c.pacos() - c * (1.0 - c * c).sqrt();
    pi - segment
}

/// The drill: a rod of radius 1 along z about x = 1 + d, y = 1.5, from
/// z = -1 to 4 (through both blocks), times `k`.
fn drill(k: f64, d: f64) -> Body {
    rod(k, [1.0 + d, 1.5, -1.0], 1.0, 5.0)
}

/// The block x 0..4, y 0..3, z `z0`..`z0 + 2`, times `k`.
fn plain(k: f64, z0: f64) -> Body {
    boxed(k, [0.0, 0.0, z0], [4.0, 3.0, z0 + 2.0])
}

/// The hole's offsets from tangent to the side, in units: tangent, within
/// half a tolerance, two and twenty tolerances into the side (the side and
/// the hole cross at 2e-3 and 6.3e-3 of a radian: folds by the repair's
/// sine) and sixty (1.1e-2: not).
const OFFSETS: [f64; 5] = [0.0, -0.5e-6, -2e-6, -20e-6, -60e-6];

/// A block with a hole tangent to its side (or `d` off), with a copy moved
/// one unit along the fold, with its mirror image in the plane through the
/// hole's axis, and with the mirror image of the copy; at three scales and
/// turned. Each result held to its closed form and to the same set made
/// from plain blocks less the drill.
#[test]
fn a_block_with_a_hole_tangent_to_its_side_and_its_copies() {
    let mut bad = Vec::new();
    for k in [1e-3, 1.0, 1e6] {
        let tol = tolerance(k);
        for (turned, turn) in turns().iter().enumerate() {
            let place = |body: Body| body.transformed(turn).unwrap();
            for d in OFFSETS {
                let hole = drill(k, d);
                let made = |z0: f64| -> Option<Body> {
                    match boolean(&plain(k, z0), &hole, BooleanOp::Subtract, &tol) {
                        Ok(made) => Some(made.body),
                        Err(e) => {
                            println!(
                                "CASE holed k {k:e} d {d:e} z0 {z0}: the operand refused: {e}"
                            );
                            None
                        }
                    }
                };
                let (Some(lower), Some(upper)) = (made(0.0), made(1.0)) else {
                    continue;
                };
                let mirror =
                    Transform::mirror(Vec3::new(0.0, 1.5 * k, 0.0), Vec3::new(0.0, 1.0, 0.0))
                        .unwrap();
                let lower_image = lower.transformed(&mirror).unwrap();
                let upper_image = upper.transformed(&mirror).unwrap();
                let section = (12.0 - disc_inside(d)) * k * k;
                // The same sets from plain blocks: (P op Q) \ D.
                let composed = |op: BooleanOp| -> Option<f64> {
                    let pq = boolean(&plain(k, 0.0), &plain(k, 1.0), op, &tol).ok()?;
                    let r = boolean(&pq.body, &hole, BooleanOp::Subtract, &tol).ok()?;
                    mass_properties(&r.body).ok().map(|p| p.volume)
                };
                let a = place(lower);
                for (name, b, ops) in [
                    ("copy", place(upper), &OPS[..]),
                    ("image", place(lower_image), &UNION_COMMON[..]),
                    ("image-copy", place(upper_image), &OPS[..]),
                ] {
                    for &op in ops {
                        let want = match (name, op) {
                            ("image", BooleanOp::Union | BooleanOp::Intersect) => {
                                Some(2.0 * section * k)
                            }
                            (_, BooleanOp::Union) => Some(3.0 * section * k),
                            (_, _) => Some(section * k),
                        };
                        let case = format!("holed k {k:e} turned {turned} d {d:e} {name} {op:?}");
                        let (line, volume) = outcome(&a, &b, op, &tol, want);
                        record(&case, &line, &mut bad);
                        if name == "copy"
                            && turned == 0
                            && let (Some(got), Some(set)) = (volume, composed(op))
                        {
                            let bound = 1e-6 * k * 60.0 * k * k;
                            if (got - set).abs() > bound {
                                record(
                                    &format!("{case} against (P op Q) \\ D"),
                                    &format!("WRONG: {got} against {set}"),
                                    &mut bad,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    verdict(&bad);
}

// ---------------------------------------------------------------------
// The design of random design 114519 (as `boolean_mirrored_tangency.rs`
// builds it), with the rod's seam where asked.
// ---------------------------------------------------------------------

/// Where the rod's cylinder has its seam, in the solid's final place.
#[derive(Clone, Copy, Debug)]
enum Seam {
    /// As the design: toward +y.
    Side,
    /// At its lowest line, where the floors end.
    Bottom,
    /// At its top.
    Top,
}

fn at(x: f64, y: f64, z: f64) -> Frame {
    Frame::WORLD.at(Vec3::new(x, y, z)).unwrap()
}

/// The solid the design shells, at scale `s` (see the repair's test).
fn solid(s: f64, tol: &Tolerance, mirrored: bool, seam: Seam) -> Body {
    let (zb, zc) = if mirrored { (-7.5, -2.5) } else { (10.0, 12.5) };
    let block = primitives::block(&at(0.0, 5.0 * s, zb * s), 2.5 * s, 15.0 * s, 7.5 * s).unwrap();
    let ball = primitives::sphere(&at(5.0 * s, 10.0 * s, zc * s), 5.0 * s).unwrap();
    let cut = boolean(&ball, &block, BooleanOp::Subtract, tol).unwrap();
    // The mirror in z = 5 turns a seam toward -z into one toward +z.
    let flip = if mirrored { -1.0 } else { 1.0 };
    let hint = match seam {
        Seam::Side => Vec3::new(0.0, 1.0, 0.0),
        Seam::Bottom => Vec3::new(0.0, 0.0, -flip),
        Seam::Top => Vec3::new(0.0, 0.0, flip),
    };
    let axis = Frame::new(
        Vec3::new(2.5 * s, 7.5 * s, zc * s),
        Vec3::new(-1.0, 0.0, 0.0),
        hint,
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

/// The ball and the block's cut face x = 2.5 (facing -x).
fn open_faces(body: &Body, s: f64) -> Vec<FaceId> {
    body.faces
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
        .collect()
}

/// The solid shelled by 1.25 less `k` tolerances, turned by `turn`; the
/// shell's refusal in its words.
fn shelled(s: f64, k: f64, mirrored: bool, seam: Seam, turn: &Transform) -> Result<Body, String> {
    let tol = tolerance(s);
    let local = solid(s, &tol, mirrored, seam);
    let open = open_faces(&local, s);
    if open.len() != 2 {
        return Err(format!("{} open faces", open.len()));
    }
    let body = local.transformed(turn).unwrap();
    let t = 1.25 - k * 1e-6;
    craft3d_kernel::shell::shell(&body, t * s, &open, &tol)
        .map(|made| made.body)
        .map_err(|e| e.to_string())
}

/// `body`'s mirror image in the plane y = 7.5 (turned with it).
fn mirror_image(body: &Body, s: f64, turn: &Transform) -> Body {
    let point = turn.point(Vec3::new(0.0, 7.5 * s, 0.0));
    let normal = turn.vector(Vec3::new(0.0, 1.0, 0.0));
    body.transformed(&Transform::mirror(point, normal).unwrap())
        .unwrap()
}

/// The three operations of `a` and `b`, each a line; the identities of
/// sets where all three were built.
fn three(case: &str, a: &Body, b: &Body, tol: &Tolerance, bad: &mut Vec<String>) {
    let mut volumes = [None; 3];
    for (slot, op) in volumes.iter_mut().zip(OPS) {
        let (line, volume) = outcome(a, b, op, tol, None);
        record(&format!("{case} {op:?}"), &line, bad);
        *slot = volume;
    }
    let (Ok(pa), Ok(pb)) = (mass_properties(a), mass_properties(b)) else {
        return;
    };
    if let [Some(union), Some(difference), Some(common)] = volumes {
        let bound = tol.linear * (pa.area + pb.area);
        let rules = [
            (
                "union + common = A + B",
                union + common - pa.volume - pb.volume,
            ),
            ("difference + common = A", difference + common - pa.volume),
        ];
        for (rule, off) in rules {
            if off.is_nan() || off.abs() > bound {
                record(
                    &format!("{case} identity"),
                    &format!(
                        "WRONG: {rule} off by {off:e} ({:.3e} of the bound)",
                        off.abs() / bound
                    ),
                    bad,
                );
            }
        }
    }
}

/// The body of that name.
fn find<'a>(bodies: &'a [(String, Body)], name: &str) -> Option<&'a Body> {
    bodies.iter().find(|(n, _)| n == name).map(|(_, b)| b)
}

/// The design's shell (its floor edge 2.4e-4 along the seam), the shell of
/// the solid built where the mirror puts it (its floor edge on the seam to
/// rounding), the mirror images of both, and the mirror image of a shell a
/// quarter of a tolerance thinner (its edge 1.9e-3 the other way), each
/// pair on one rod and one floor plane: the three operations.
#[test]
fn the_designs_shells_on_and_off_the_seam_mixed() {
    let mut bad = Vec::new();
    for s in [1.0, 1e3] {
        let tol = tolerance(s);
        for (turned, turn) in turns().iter().enumerate() {
            let shells = [
                ("X", shelled(s, 0.0, true, Seam::Side, turn)),
                ("Y", shelled(s, 0.0, false, Seam::Side, turn)),
                ("Q", shelled(s, 0.25, true, Seam::Side, turn)),
            ];
            let mut bodies: Vec<(String, Body)> = Vec::new();
            for (name, made) in shells {
                match made {
                    Ok(body) => {
                        bodies.push((format!("{name}'"), mirror_image(&body, s, turn)));
                        bodies.push((name.to_string(), body));
                    }
                    Err(e) => println!("CASE shell {name} s {s:e} turned {turned}: refused: {e}"),
                }
            }
            for (first, second) in [
                ("X", "Y'"),
                ("Y", "X'"),
                ("X", "Y"),
                ("X'", "Y'"),
                ("X", "Q'"),
                ("Q", "X'"),
            ] {
                let (Some(a), Some(b)) = (find(&bodies, first), find(&bodies, second)) else {
                    println!("CASE mixed s {s:e} turned {turned} {first}+{second}: no operands");
                    continue;
                };
                three(
                    &format!("mixed s {s:e} turned {turned} {first}+{second}"),
                    a,
                    b,
                    &tol,
                    &mut bad,
                );
            }
        }
    }
    verdict(&bad);
}

/// The design with the rod's cylinder built with its seam at its lowest
/// line (where both floors end, between them) or at its top: the shell and
/// its mirror image, the three operations.
#[test]
fn the_design_with_its_rods_seam_on_the_tangency_line() {
    let mut bad = Vec::new();
    for s in [1.0, 1e3] {
        let tol = tolerance(s);
        for (turned, turn) in turns().iter().enumerate() {
            for seam in [Seam::Bottom, Seam::Top] {
                for k in [0.0, 0.25] {
                    let case = format!("seam {seam:?} s {s:e} turned {turned} k {k}");
                    let shell = match shelled(s, k, true, seam, turn) {
                        Ok(shell) => shell,
                        Err(e) => {
                            println!("CASE {case}: the shell refused: {e}");
                            continue;
                        }
                    };
                    let image = mirror_image(&shell, s, turn);
                    three(&case, &image, &shell, &tol, &mut bad);
                }
            }
        }
    }
    verdict(&bad);
}

/// The design's shell with a copy of itself moved along the rod (a linear
/// pattern whose instances overlap: the floors' folds on one line, partly
/// shared), and with its mirror image so moved.
#[test]
fn the_designs_shell_with_a_copy_along_its_rod() {
    let mut bad = Vec::new();
    for s in [1.0, 1e3] {
        let tol = tolerance(s);
        for (turned, turn) in turns().iter().enumerate() {
            let shell = match shelled(s, 0.0, true, Seam::Side, turn) {
                Ok(shell) => shell,
                Err(e) => {
                    println!("CASE along s {s:e} turned {turned}: the shell refused: {e}");
                    continue;
                }
            };
            let image = mirror_image(&shell, s, turn);
            for d in [0.1, 0.3] {
                let step = Transform::translation(turn.vector(Vec3::new(d * s, 0.0, 0.0)));
                let copy = shell.transformed(&step).unwrap();
                let moved_image = image.transformed(&step).unwrap();
                three(
                    &format!("along s {s:e} turned {turned} d {d} copy"),
                    &copy,
                    &shell,
                    &tol,
                    &mut bad,
                );
                three(
                    &format!("along s {s:e} turned {turned} d {d} image"),
                    &moved_image,
                    &shell,
                    &tol,
                    &mut bad,
                );
            }
        }
    }
    verdict(&bad);
}
