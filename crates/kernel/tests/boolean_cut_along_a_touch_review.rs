//! The review of the cut along the line where a wall thins to nothing
//! (`section::junctions`, docs/design/booleans.md › *A cut along the line
//! where a wall thins to nothing*): where else two stretches of one face
//! lie along each other between different vertices, and whether the rule
//! cuts what it should and nothing else.
//!
//! A plate x 0..2, y 0..10, z 0..10 with a hole of radius 2.5 along x about
//! y = 2.5, z = 5, tangent to the side y = 0 along the line z = 5 (a wall
//! that thins to nothing, accepted 2026-10-18), cut by a plane through that
//! line: the side's stretch and the hole's run along each other on the
//! cutting face. The hole through the plate (both stretches end at the
//! plate's ends), blind to x = 1.5 (the hole's stretch ends inside the
//! side's), with its circles starting on the touch line (a stretch along a
//! seam), the side made of two faces of one plane (stretches on different
//! faces of the same surface, which overlap the hole's in part), and the
//! plane turned about the touch line or tilted by a small angle across it
//! (curves that cross at a small angle). And the seed's union, refused as
//! zero thickness, at a place on the touch line; the seed's reduction cut
//! by a block whose own edge crosses the touch line; the seed's block with
//! its side stepping out inside the groove (a partial overlap).
//!
//! Measured on the candidate 49310fba (Check 45746463963) and the branch
//! 4967e52e (Check 45746460198), 221 cases each: no case the candidate
//! builds or refuses in words is worse on the branch. The block whose edge
//! crosses the touch line, refused on the candidate (*"two curves leave a
//! point in the same direction and stay together"*), is built on the branch
//! to its closed form within 5e-13. Every cut of the plate with the hole
//! is refused on both (owed, as on the candidate, the tests ignored): the
//! hole through the plate as *"imprint lost"* on both; the blind hole, at
//! every angle, as *"two curves leave a point in the same direction"* on
//! the candidate and *"imprint lost"* on the branch (the words moved, still
//! refused); the plate whose side is two faces of one plane is refused as
//! zero thickness already when the hole is cut, so the partial overlap is
//! never reached there. The plane tilted across the line is refused in the
//! same words on both. The seed's block whose side steps out inside the
//! groove gives a partial overlap: both of its cuts were invalid bodies on
//! the candidate (Check 45750345753); on the branch (Check 45750343242) one
//! is built to its closed form and the other, whose side's quarter points
//! miss the cylinder's stretch, refused in words.
//!
//! Every volume is written without the kernel ([`oracle`]: the square and
//! the disc on one side of a line, a circular segment, Simpson's rule of a
//! smooth segment), the same to 1e-12 by membership in
//! `plan/seed-122490/review/oracle_plate.py` in the workshop.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

mod common;

use common::{polygon, rectangle};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::boolean::{BooleanOptions, VerifyOptions, boolean, verify};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::halfspace::{Kept, keep_below};
use craft3d_kernel::primitives::{block, cylinder};
use craft3d_kernel::profile::detect_profiles;
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Portable, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, check, mass_properties};
use std::f64::consts::TAU;

/// The scales the cases are built at; the tolerance is a millionth of
/// the scale.
const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn turn() -> Transform {
    Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 0.7).unwrap()
}

/// The volumes written without the kernel, at scale 1.
mod oracle {
    use craft3d_math::Portable;
    use std::f64::consts::PI;

    /// The hole's radius.
    pub const R: f64 = 2.5;

    /// Which side of the line through the touch point (y 0, z 5) at the
    /// angle `theta` from +y, in the plane yz, a point is on: positive
    /// "above" (the normal (−sin θ, cos θ)).
    fn side(theta: f64, (y, z): (f64, f64)) -> f64 {
        let (s, c) = theta.psin_cos();
        -s * y + c * (z - 5.0)
    }

    /// The area of the square y, z in 0..10 above the line: the square
    /// clipped by the half-plane (Sutherland–Hodgman), by the shoelace.
    pub fn square_above(theta: f64) -> f64 {
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let mut kept: Vec<(f64, f64)> = Vec::new();
        for (p, q) in square.iter().zip(square.iter().cycle().skip(1)) {
            let (fp, fq) = (side(theta, *p), side(theta, *q));
            if fp >= 0.0 {
                kept.push(*p);
            }
            if (fp >= 0.0) != (fq >= 0.0) {
                let t = fp / (fp - fq);
                kept.push((p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1)));
            }
        }
        let twice: f64 = kept
            .iter()
            .zip(kept.iter().cycle().skip(1))
            .map(|(p, q)| p.0 * q.1 - q.0 * p.1)
            .sum();
        0.5 * twice.abs()
    }

    /// The area of the hole's disc above the line: a chord from the touch
    /// point at the angle θ to the diameter there subtends 2 asin |cos θ|
    /// at the centre, which lies below for θ in [0, π) (its side is
    /// −2.5 sin θ), so above is the segment r²/2 (α − sin α).
    pub fn disc_above(theta: f64) -> f64 {
        let alpha = 2.0 * theta.pcos().abs().min(1.0).pasin();
        0.5 * R * R * (alpha - alpha.psin())
    }

    /// The plate's volume above and below the plane through the touch line
    /// at θ, the hole over `length` of the plate's 2.
    pub fn through_the_line(theta: f64, length: f64) -> (f64, f64) {
        let (square, disc) = (square_above(theta), disc_above(theta));
        (
            2.0 * square - length * disc,
            2.0 * (100.0 - square) - length * (PI * R * R - disc),
        )
    }

    /// The part of the hole's disc beyond a chord `h` from its centre.
    fn segment(h: f64) -> f64 {
        let h = h.clamp(-R, R);
        R * R * (h / R).pacos() - h * (R * R - h * h).max(0.0).sqrt()
    }

    /// The plate's volume above and below the plane z = 5 + ε (x − 1),
    /// the hole over `length`: the plate's half is 10 ∫ (5 − ε (x − 1)) dx
    /// over 0..2 = 100 either side; the hole's part above is the segment
    /// beyond ε (x − 1), smooth in x, by Simpson's rule (2000 pieces).
    pub fn tilted(eps: f64, length: f64) -> (f64, f64) {
        let n: i32 = 2000;
        let h = length / f64::from(n);
        let f = |x: f64| segment(eps * (x - 1.0));
        let mut sum = f(0.0) + f(length);
        for i in 1..n {
            let weight = if i % 2 == 1 { 4.0 } else { 2.0 };
            sum += weight * f(f64::from(i) * h);
        }
        let hole_above = sum * h / 3.0;
        (
            100.0 - hole_above,
            100.0 - (length * PI * R * R - hole_above),
        )
    }

    /// ∫ √(r² − z²) dz from 0 to `b` (|b| ≤ r).
    fn under(r: f64, b: f64) -> f64 {
        0.5 * (b * (r * r - b * b).max(0.0).sqrt() + r * r * (b / r).pasin())
    }

    /// The seed's groove (the region 2.5 ≤ √(x² + z²) ≤ 12.5 within the
    /// block's x −5..12.5) over z 0..5, per unit of y: the outer circle's
    /// half width plus 5 (the side x = −5 within it, where √(12.5² − z²)
    /// > 5) less the inner circle's width.
    pub fn groove_section() -> f64 {
        under(12.5, 5.0) + 25.0 - 2.0 * under(2.5, 2.5)
    }
}

/// The plate x 0..2, y 0..10, z 0..10 (times `k`), extruded from its
/// outline in the xy plane; with `split`, its side y = 0 is two faces of
/// one plane, x 0..split and split..2 (a vertex of the outline there).
fn plate(k: f64, split: Option<f64>) -> Body {
    let tol = tolerance(k);
    let mut points = vec![[0.0, 0.0]];
    if let Some(s) = split {
        points.push([s * k, 0.0]);
    }
    points.extend([[2.0 * k, 0.0], [2.0 * k, 10.0 * k], [0.0, 10.0 * k]]);
    let profiles = detect_profiles(&polygon(1, 1, &points), &tol).unwrap();
    extrude(&profiles, &Frame::WORLD, 0.0, 10.0 * k, &tol)
        .unwrap()
        .body
}

/// The hole's cylinder, radius 2.5 about the line y = 2.5, z = 5, over x
/// `from..to` (times `k`): tangent to the plate's side y = 0 along z = 5.
/// Its circles start on the far side, or with `seam` on the touch line.
fn drill(k: f64, (from, to): (f64, f64), seam: bool) -> Body {
    let hint = if seam {
        Vec3::new(0.0, -1.0, 0.0)
    } else {
        Vec3::Y
    };
    let frame = Frame::new(Vec3::new(from, 2.5, 5.0) * k, Vec3::X, hint).unwrap();
    cylinder(&frame, 2.5 * k, (to - from) * k).unwrap()
}

/// A plate with a hole tangent to its side.
#[derive(Clone, Copy, Debug)]
struct Holed {
    split: Option<f64>,
    hole: (f64, f64),
    seam: bool,
}

const THROUGH: (f64, f64) = (-2.0, 4.0);
const BLIND: (f64, f64) = (-1.0, 1.5);

impl Holed {
    fn new(split: Option<f64>, hole: (f64, f64), seam: bool) -> Self {
        Holed { split, hole, seam }
    }

    /// How much of the plate's length the hole runs over.
    fn length(&self) -> f64 {
        self.hole.1.min(2.0) - self.hole.0.max(0.0)
    }

    fn body(&self, k: f64, turned: bool) -> Result<Body, String> {
        let (mut a, mut b) = (plate(k, self.split), drill(k, self.hole, self.seam));
        if turned {
            a = a.transformed(&turn()).unwrap();
            b = b.transformed(&turn()).unwrap();
        }
        boolean(&a, &b, Subtract, &tolerance(k))
            .map(|made| made.body)
            .map_err(|e| format!("the hole refused: {e}"))
    }
}

/// A plane through the touch line at an angle (radians from +y), or
/// through the point (1, 0, 5) of it tilted by `ε` about the y axis.
#[derive(Clone, Copy, Debug)]
enum Plane {
    Through(f64),
    Tilted(f64),
}

impl Plane {
    /// The plane at scale `k` keeping the side above it (or below), for
    /// `keep_below`, whose normal points to the side cut away.
    fn frame(self, k: f64, above: bool, turned: bool) -> Frame {
        let (origin, up) = match self {
            Plane::Through(theta) => {
                let (s, c) = theta.psin_cos();
                (Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, -s, c))
            }
            Plane::Tilted(eps) => (Vec3::new(1.0, 0.0, 5.0), Vec3::new(-eps, 0.0, 1.0)),
        };
        let normal = if above { -up } else { up };
        let frame = Frame::from_z(origin * k, normal).unwrap();
        if turned {
            turn().frame(&frame).unwrap()
        } else {
            frame
        }
    }

    fn expected(self, length: f64, above: bool) -> f64 {
        let (up, down) = match self {
            Plane::Through(theta) => oracle::through_the_line(theta, length),
            Plane::Tilted(eps) => oracle::tilted(eps, length),
        };
        if above { up } else { down }
    }
}

/// What a cut gave: `ok` (one shell, `topo::check` clean, the volume
/// within `slack` times the tolerance times the area of the closed form),
/// or what is wrong; a line of it printed (`CASE`) for the review's
/// records.
fn judge(label: &str, made: Result<Body, String>, k: f64, expected: f64, slack: f64) -> String {
    let tol = tolerance(k);
    let verdict = match made {
        Err(e) if e.contains("invalid body") => format!("invalid: {e}"),
        Err(e) => format!("refused: {e}"),
        Ok(body) => {
            let problems: Vec<String> =
                check(&body, &tol).iter().map(ToString::to_string).collect();
            let counts = (body.vertices.len(), body.edges.len(), body.faces.len());
            match mass_properties(&body) {
                Err(e) => format!("built, no volume: {e}; {problems:?}"),
                Ok(mass) => {
                    let volume = mass.volume / (k * k * k);
                    let allowed = slack * 1e-6 * mass.area / (k * k);
                    let off = volume - expected;
                    let shape = format!(
                        "{} shells, counts {counts:?}, volume {volume:.12} against {expected:.12} (off {off:.1e}, allowed {allowed:.1e})",
                        body.shells.len()
                    );
                    if problems.is_empty() && body.shells.len() == 1 && off.abs() <= allowed {
                        format!("ok: {shape}")
                    } else {
                        format!("built wrong: {shape}; {problems:?}")
                    }
                }
            }
        }
    };
    println!("CASE {label}: {verdict}");
    verdict
}

/// The plate with the hole cut by the plane, keeping the side above (or
/// below) it: what is wrong, if anything.
fn plane_cut(holed: Holed, plane: Plane, k: f64, above: bool, turned: bool, slack: f64) -> String {
    let label = format!("{holed:?} {plane:?} k={k} above={above} turned={turned}");
    let made = holed.body(k, turned).and_then(|body| {
        match keep_below(
            &body,
            &plane.frame(k, above, turned),
            &tolerance(k),
            &BooleanOptions::default(),
        ) {
            Ok(Kept::Cut(made)) => Ok(made.body),
            Ok(Kept::Whole) => Err("kept whole".to_string()),
            Err(e) => Err(e.to_string()),
        }
    });
    judge(
        &label,
        made,
        k,
        plane.expected(holed.length(), above),
        slack,
    )
}

/// The same cut at z = 5 as a boolean: the plate with the hole intersected
/// with a block standing on that plane, held to the boolean's full check
/// as well.
fn block_cut(holed: Holed, k: f64, turned: bool) -> String {
    let label = format!("{holed:?} block above z=5 k={k} turned={turned}");
    let tol = tolerance(k);
    let made = holed.body(k, turned).and_then(|body| {
        let mut upper = block(
            &Frame::WORLD.at(Vec3::new(-1.0, -1.0, 5.0) * k).unwrap(),
            4.0 * k,
            12.0 * k,
            6.0 * k,
        )
        .unwrap();
        if turned {
            upper = upper.transformed(&turn()).unwrap();
        }
        let made = boolean(&upper, &body, Intersect, &tol).map_err(|e| e.to_string())?;
        let verified = verify(
            &upper,
            &body,
            Intersect,
            &made.body,
            &tol,
            &VerifyOptions::default(),
        )
        .map_err(|e| format!("the boolean's check cannot run: {e}"))?;
        match verified.violations.first() {
            Some(first) => Err(format!("invalid body (the full check): {first}")),
            None => Ok(made.body),
        }
    });
    judge(
        &label,
        made,
        k,
        Plane::Through(0.0).expected(holed.length(), true),
        1e-2,
    )
}

fn failures(found: &[String]) -> Vec<&String> {
    found.iter().filter(|v| !v.starts_with("ok")).collect()
}

#[test]
fn the_oracles_agree_with_the_review_script() {
    let near = |a: f64, b: f64| (a - b).abs() < 1e-10 * b.abs().max(1.0);
    for (theta, length, above, below) in [
        (0.0, 2.0, 80.36504591506379, 80.36504591506379),
        (30.0, 2.0, 35.623959572917215, 125.10613225721036),
        (60.0, 1.5, 13.584512288746149, 156.96305658384952),
        (150.0, 1.5, 37.54328722699339, 133.00428164560228),
    ] {
        let (up, down) = oracle::through_the_line(f64::to_radians(theta), length);
        assert!(
            near(up, above) && near(down, below),
            "{theta}, {length}: {up}, {down}"
        );
    }
    for (eps, length, above, below) in [
        (1e-2, 1.1, 89.17602528661514, 89.22552521995503),
        (1e-3, 1.5, 85.2719094363291, 85.2756594362666),
        (1e-2, 2.0, 80.36504591506379, 80.36504591506379),
    ] {
        let (up, down) = oracle::tilted(eps, length);
        assert!(
            near(up, above) && near(down, below),
            "{eps}, {length}: {up}, {down}"
        );
    }
    // The seed's reduction, its volume as the branch wrote it.
    let reduced = 17.5 * 17.5 * 5.0 - 2.5 * oracle::groove_section();
    assert!(near(reduced, 1341.316563374929), "{reduced}");
}

/// The hole through the plate, cut by the plane z = 5 through the touch
/// line: the side's stretch and the hole's run between the same two
/// vertices (the plate's ends), one arc as before the repair. Also with
/// the hole's circles starting on the touch line (a stretch along its
/// seam), and as the boolean with a block standing on the plane.
#[test]
#[ignore = "owed, as on the candidate: refused as imprint lost on the candidate and the branch \
            (Check 45746463963 on 49310fba, Check 45746460198 on 4967e52e)"]
fn a_hole_through_a_plate_cut_along_its_touch() {
    let mut found = Vec::new();
    for k in SCALES {
        for seam in [false, true] {
            let holed = Holed::new(None, THROUGH, seam);
            for above in [true, false] {
                found.push(plane_cut(holed, Plane::Through(0.0), k, above, false, 1e-2));
            }
            found.push(block_cut(holed, k, false));
        }
    }
    let holed = Holed::new(None, THROUGH, false);
    found.push(plane_cut(holed, Plane::Through(0.0), 1.0, true, true, 1e-2));
    let wrong = failures(&found);
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The hole blind at x = 1.5, cut by the plane through the touch line at
/// angles from 0 to 150 degrees, either side: the hole's stretch ends at
/// x = 1.5, inside the side's, as the seed's groove's does; with the
/// circles starting on the touch line; turned out of the axes.
#[test]
#[ignore = "owed, as on the candidate: refused on the candidate as two curves that leave a point \
            in the same direction (Check 45746463963 on 49310fba) and on the branch as imprint lost \
            (Check 45746460198 on 4967e52e), at every angle and scale"]
fn a_blind_hole_cut_along_its_touch_at_angles() {
    let mut found = Vec::new();
    for k in SCALES {
        for degrees in [0.0, 30.0, 60.0, 80.0, 100.0, 120.0, 150.0] {
            let plane = Plane::Through(f64::to_radians(degrees));
            for above in [true, false] {
                found.push(plane_cut(
                    Holed::new(None, BLIND, false),
                    plane,
                    k,
                    above,
                    false,
                    1e-2,
                ));
            }
        }
        for above in [true, false] {
            found.push(plane_cut(
                Holed::new(None, BLIND, true),
                Plane::Through(0.0),
                k,
                above,
                false,
                1e-2,
            ));
        }
        found.push(block_cut(Holed::new(None, BLIND, false), k, false));
    }
    for degrees in [0.0, 30.0] {
        let plane = Plane::Through(f64::to_radians(degrees));
        found.push(plane_cut(
            Holed::new(None, BLIND, false),
            plane,
            1.0,
            true,
            true,
            1e-2,
        ));
        found.push(plane_cut(
            Holed::new(None, BLIND, false),
            plane,
            1e3,
            false,
            true,
            1e-2,
        ));
    }
    let wrong = failures(&found);
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The side made of two faces of one plane, split at x = `s`: the hole's
/// stretch on the cutting face runs along both side stretches, and with
/// the blind hole overlaps the second in part (each running past the
/// other's end). With the split at 1 a quarter point of each lies on the
/// other; with the split at 0.2 the first lies within the hole's.
#[test]
#[ignore = "owed, as on the candidate: with the blind hole the plate is refused as zero thickness \
            at the split already when the hole is cut; the through hole's cuts are refused (two \
            curves that leave a point in the same direction on the candidate, imprint lost on the \
            branch; Check 45746463963 on 49310fba, Check 45746460198 on 4967e52e)"]
fn a_side_of_two_faces_along_the_hole() {
    let mut found = Vec::new();
    for k in SCALES {
        for (split, hole) in [(1.0, THROUGH), (1.0, BLIND), (0.2, BLIND)] {
            let holed = Holed::new(Some(split), hole, false);
            for above in [true, false] {
                found.push(plane_cut(holed, Plane::Through(0.0), k, above, false, 1e-2));
            }
            found.push(block_cut(holed, k, false));
        }
        let plane = Plane::Through(f64::to_radians(30.0));
        found.push(plane_cut(
            Holed::new(Some(1.0), THROUGH, false),
            plane,
            k,
            true,
            false,
            1e-2,
        ));
    }
    let wrong = failures(&found);
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The partial overlap the rule does not see: the side split at 1.4, the
/// blind hole to 1.5. The second side stretch (1.4..2) and the hole's
/// (0..1.5) overlap over 1.4..1.5, less than a quarter of either, so no
/// quarter point of one lies on the other and neither is cut at the
/// other's end: the two arcs between different vertices that the repair
/// is about.
#[test]
#[ignore = "owed, as on the candidate: the plate with its side of two faces is refused as zero \
            thickness at the split (1.4, 0, 5) when the hole is cut, so the overlap is never reached \
            (Check 45746463963 on 49310fba, Check 45746460198 on 4967e52e)"]
fn a_short_partial_overlap_along_the_hole() {
    let mut found = Vec::new();
    for k in SCALES {
        let holed = Holed::new(Some(1.4), BLIND, false);
        for above in [true, false] {
            found.push(plane_cut(holed, Plane::Through(0.0), k, above, false, 1e-2));
        }
        found.push(block_cut(holed, k, false));
    }
    let wrong = failures(&found);
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The plane through the point (1, 0, 5) of the touch line tilted by a
/// small angle about y: on it the hole's curve leaves the side's line
/// quadratically, within the tolerance of it near x = 1 only (curves that
/// cross, or touch, at a small angle). Built to the volume, or refused in
/// words: never a body the boolean's own check refuses.
#[test]
fn a_plane_tilted_across_the_touch_line() {
    let mut found = Vec::new();
    for k in SCALES {
        for eps in [1e-2, 1e-3] {
            for hole in [THROUGH, BLIND, (-1.0, 1.1)] {
                for above in [true, false] {
                    found.push(plane_cut(
                        Holed::new(None, hole, false),
                        Plane::Tilted(eps),
                        k,
                        above,
                        false,
                        1.0,
                    ));
                }
            }
        }
    }
    let wrong: Vec<&String> = found
        .iter()
        .filter(|v| !(v.starts_with("ok") || v.starts_with("refused")))
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The seed's grooved block (`boolean_cut_along_a_touch.rs`: a block
/// x −5..12.5, y −1.25..16.25, z −5..5 less the rectangle x 2.5..12.5,
/// y 7.5..10 turned once about y, whose outer cylinder touches the side
/// x = 12.5 along z = 0, y 7.5..10).
fn grooved(k: f64) -> Body {
    let tol = tolerance(k);
    let blank = block(
        &Frame::WORLD.at(Vec3::new(-5.0, -1.25, -5.0) * k).unwrap(),
        17.5 * k,
        17.5 * k,
        10.0 * k,
    )
    .unwrap();
    boolean(&blank, &groove(k), Subtract, &tol).unwrap().body
}

/// The seed's groove: the rectangle x 2.5..12.5, y 7.5..10 of the xy
/// sketch turned once about y (times `k`).
fn groove(k: f64) -> Body {
    let tol = tolerance(k);
    let profiles = detect_profiles(
        &rectangle(1, 1, (2.5 * k, 12.5 * k), (7.5 * k, 10.0 * k)),
        &tol,
    )
    .unwrap();
    revolve(
        &profiles,
        &Frame::WORLD,
        [0.0, 0.0],
        [0.0, 1.0],
        0.0,
        TAU,
        &tol,
    )
    .unwrap()
    .body
}

/// The seed's block with its side stepping out inside the groove: the
/// outline x −5..12.5 over y `a..c`, x −5..14 over y `c..16.25`, z −5..5
/// (times `k`), less the groove. The side x = 12.5 ends at y = `c`, inside
/// the groove's 7.5..10, while the groove's outer cylinder runs on to 10
/// inside the wider part: on the plane z = 0 the side's stretch (a..c) and
/// the cylinder's (7.5..10) overlap in part, each running past the other's
/// end.
fn stepped(k: f64, a: f64, c: f64) -> Result<Body, String> {
    let tol = tolerance(k);
    let points: Vec<[f64; 2]> = [
        [-5.0, a],
        [12.5, a],
        [12.5, c],
        [14.0, c],
        [14.0, 16.25],
        [-5.0, 16.25],
    ]
    .iter()
    .map(|p| [p[0] * k, p[1] * k])
    .collect();
    let profiles = detect_profiles(&polygon(1, 1, &points), &tol).unwrap();
    let plane = Frame::WORLD.at(Vec3::new(0.0, 0.0, -5.0) * k).unwrap();
    let blank = extrude(&profiles, &plane, 0.0, 10.0 * k, &tol)
        .unwrap()
        .body;
    boolean(&blank, &groove(k), Subtract, &tol)
        .map(|made| made.body)
        .map_err(|e| format!("the stepped block refused: {e}"))
}

/// The stepped block above z = 0 (intersected with the block standing on
/// that plane): one shell to the closed form, or refused in words, never a
/// body the boolean's own check refuses. The side from y = 7 to 8.75: a
/// quarter point of each stretch lies on the other, both are cut, and the
/// body is built (one shell, 24 vertices, 37 edges, 13 faces, to 5e-13 of
/// the closed form). From y = −1.25 to 8.75: only the cylinder's quarter
/// point lies on the side's stretch, which is cut at 7.5; the cylinder's is
/// not cut at 8.75, and the boolean is refused (*"two curves leave a point
/// in the same direction"*): built or not by where the quarter points of an
/// unrelated length fall (owed). From −1.25 to 8: the stepped block itself
/// is refused as zero thickness at (12.5, 7.5, 0), as on the candidate.
/// On the candidate (Check 45750345753) both cuts were invalid bodies
/// (*"loop 0 of face 0 … runs over loop 1 of the face"*); on the branch
/// (Check 45750343242) as above. Volume: the outline's area times 5 less
/// the groove's section over its 2.5 (the groove stays within x ≤ 12.5).
#[test]
fn a_side_that_steps_out_inside_the_groove() {
    let mut found = Vec::new();
    for k in SCALES {
        for (a, c, builds) in [(7.0, 8.75, true), (-1.25, 8.75, false), (-1.25, 8.0, false)] {
            let expected =
                5.0 * (17.5 * (16.25 - a) + 1.5 * (16.25 - c)) - 2.5 * oracle::groove_section();
            let label = format!("stepped side y {a}..{c} k={k}");
            let tol = tolerance(k);
            let upper = standing(k, (-1.25, 18.75));
            let made = stepped(k, a, c).and_then(|body| {
                let made = boolean(&upper, &body, Intersect, &tol).map_err(|e| e.to_string())?;
                let verified = verify(
                    &upper,
                    &body,
                    Intersect,
                    &made.body,
                    &tol,
                    &VerifyOptions::default(),
                )
                .map_err(|e| format!("the boolean's check cannot run: {e}"))?;
                match verified.violations.first() {
                    Some(first) => Err(format!("invalid body (the full check): {first}")),
                    None => Ok(made.body),
                }
            });
            let verdict = judge(&label, made, k, expected, 1e-2);
            if builds && !verdict.starts_with("ok") {
                found.push(format!("not built: {verdict}"));
            } else {
                found.push(verdict);
            }
        }
    }
    let wrong: Vec<&String> = found
        .iter()
        .filter(|v| !(v.starts_with("ok") || v.starts_with("refused")))
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A block standing on z = 0, x −5..15, z 0..10, over y `y0..y1` (times
/// `k`).
fn standing(k: f64, (y0, y1): (f64, f64)) -> Body {
    let frame = Frame::WORLD.at(Vec3::new(-5.0, y0, 0.0) * k).unwrap();
    block(&frame, 20.0 * k, (y1 - y0) * k, 10.0 * k).unwrap()
}

/// The seed's union, the grooved block with the block standing on z = 0:
/// refused as zero thickness (the crescent below the plane meets the
/// standing block's bottom along the touch line, two parts touching), the
/// place named on the touch line (x 12.5, z 0, y 7.5..10), within twice
/// the tolerance: its end before the repair, its middle after; neither
/// is pinned elsewhere. Either way round, at four scales and turned.
#[test]
fn the_seeds_union_is_refused_on_the_touch_line() {
    let mut found = Vec::new();
    for k in SCALES {
        for turned in [false, true] {
            let (mut a, mut b) = (grooved(k), standing(k, (-1.25, 18.75)));
            if turned {
                a = a.transformed(&turn()).unwrap();
                b = b.transformed(&turn()).unwrap();
            }
            for (x, y, order) in [(&a, &b, "grooved first"), (&b, &a, "standing first")] {
                let label = format!("union k={k} turned={turned} {order}");
                let verdict = match boolean(x, y, Union, &tolerance(k)) {
                    Err(KernelError::ZeroThickness { at, .. }) => {
                        let at = if turned {
                            turn().inverse().unwrap().point(at)
                        } else {
                            at
                        };
                        let near = 2e-6 * k;
                        let on = (at.x - 12.5 * k).abs() <= near
                            && at.z.abs() <= near
                            && at.y >= 7.5 * k - near
                            && at.y <= 10.0 * k + near;
                        let at = at * (1.0 / k);
                        if on {
                            format!("ok: zero thickness at {at:?} (scale 1)")
                        } else {
                            format!("zero thickness off the touch line: {at:?} (scale 1)")
                        }
                    }
                    Err(e) => format!("refused otherwise: {e}"),
                    Ok(_) => "built".to_string(),
                };
                println!("CASE {label}: {verdict}");
                found.push(verdict);
            }
        }
    }
    let wrong = failures(&found);
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The seed's reduction cut by a block whose own edge crosses the touch
/// line in the middle (its y from −1.25 to 8.75, or from 8.75 to 18.75):
/// the side's stretch and the groove's end together at that edge, the
/// groove's other end inside the side's. Volume: the block's part above
/// z = 0 over that y less the groove's section over its 1.25 of it.
#[test]
fn a_cutting_face_that_ends_across_the_touch_line() {
    let mut found = Vec::new();
    for k in SCALES {
        for (y0, y1, block_y) in [(-1.25, 8.75, 10.0), (8.75, 18.75, 7.5)] {
            let expected = 17.5 * block_y * 5.0 - 1.25 * oracle::groove_section();
            let label = format!("grooved ∩ standing y {y0}..{y1} k={k}");
            let tol = tolerance(k);
            let (a, b) = (grooved(k), standing(k, (y0, y1)));
            let made = boolean(&b, &a, Intersect, &tol)
                .map_err(|e| e.to_string())
                .and_then(|made| {
                    let verified = verify(
                        &b,
                        &a,
                        Intersect,
                        &made.body,
                        &tol,
                        &VerifyOptions::default(),
                    )
                    .map_err(|e| format!("the boolean's check cannot run: {e}"))?;
                    match verified.violations.first() {
                        Some(first) => Err(format!("invalid body (the full check): {first}")),
                        None => Ok(made.body),
                    }
                });
            found.push(judge(&label, made, k, expected, 1e-2));
        }
    }
    let wrong = failures(&found);
    assert!(wrong.is_empty(), "{wrong:#?}");
}
