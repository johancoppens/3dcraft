//! The review of the curve of touch kept past a face's border (random
//! design 108069; docs/design/booleans.md › *A curve of touch kept past a
//! face's border*): the rule `section::clip` gained (a stretch along which
//! two faces only touch, inside both by its samples but with an end clear
//! outside one of them, is left out where the result does not keep both
//! faces there) met on other shapes than the patterned torus.
//!
//! The shapes, in the test's units (the tolerance a millionth of them):
//!
//! - A block, its floor z = 0: `CENTRED` (x from -5 to 5, y from -3 to 3,
//!   z from 0 to 4), whose floor's middle and the middles of its two short
//!   edges lie on the line y = 0, z = 0, and `ASIDE` (y from -2 to 4),
//!   whose floor's middle does not.
//! - The lens: the solid cylinder of radius 1 about the line y = 0,
//!   z = 1 (in the block) or z = -1 (under it), x from -4 to 4, within the
//!   ball of radius `R` about the origin. Its wall touches the floor along
//!   the line y = 0, z = 0 for |x| up to `R`, from within the block or
//!   from under it. The wall's border is where the ball meets the
//!   cylinder, a spline, which crosses the line at x = ±R and there only
//!   touches the floor: a spline edge's touch, which stands off the line
//!   where the result keeps one face or neither (`corner::touch`). The
//!   floor's line of touch is cut where the block's short edges cross it
//!   (closed form), at x = ±5, clear outside the wall; its samples, at
//!   x = 0 and ±2.5, are inside both faces. That is the stretch the rule
//!   is about.
//! - The rod: the same cylinder from x = -3 to 3, its ends circles that
//!   touch the floor in closed form (the corpus's tangent block and
//!   cylinder).
//! - The notch: the ball of radius 0.2 about (0.5, 0, 0), taken from the
//!   lens or the rod, so that the line of touch leaves the wall and comes
//!   back between the samples.
//!
//! The answers, without the kernel: the lens lies within the block (or
//! under it, touching it along the line only), so the union is the block
//! (or two parts that touch along the line), the common part the lens (or
//! nothing), and the differences what is left. The lens's volume is
//! integrated two ways (`lens_volume_by_rings`, about the cylinder's
//! axis; `lens_volume_by_slices`, across x, the area of two discs in
//! closed form), and so is the notch's; `the_volumes_two_ways` holds them
//! to each other.
//!
//! Each test gathers every case's failure before it fails, so that one run
//! says how each case fares.
//!
//! Measured (Check 45698408500, this file on `main` ed93bafc; Check
//! 45698388808 on the branch, 5c0b1a3c): all 130 outcomes the same on
//! both, case by case. Built right on both: everything with the lens under
//! the block, and the notched rod's union, common part and difference with
//! the block. Refused on both as at the edge of tolerance (*"a piece of a
//! face lies within tolerance of a face it is not coincident with"*, the
//! block's floor): every union, common part and difference of the lens (or
//! the notched lens) within the block that keeps one face or neither,
//! whatever the block's placing and wherever its short edges are; ignored
//! below as owed, as on `main`. The block less the lens, the notched lens
//! or the notched rod (the result keeps both faces) is refused as zero
//! thickness at an end of the line on both. Where the rule is read (Check
//! 45708746642, a throwaway print on the branch): in the 20 booleans of
//! `the_block_ending_a_few_tolerances_past_the_lens` alone, `keeps_both`
//! reading `Some(false)` each time, so the stretch was left out there and
//! the refusal is `main`'s all the same; with the block's short edges at
//! x = ±5 the stretch is left out before, by its ends beyond the wall's
//! box, on `main` too.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::boolean::{BooleanOp, VerifyOptions, boolean, verify};
use craft3d_kernel::primitives;
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, check, mass_properties};

/// The radius of the ball the lens is cut from.
const R: f64 = 3.2;
/// The radius of the notch.
const NOTCH: f64 = 0.2;
/// Where along x the notch is.
const NOTCH_AT: f64 = 0.5;

type Cuboid = ([f64; 3], [f64; 3]);
const CENTRED: Cuboid = ([-5.0, -3.0, 0.0], [5.0, 3.0, 4.0]);
const ASIDE: Cuboid = ([-5.0, -2.0, 0.0], [5.0, 4.0, 4.0]);
/// The volume of either block.
const BLOCK: f64 = 240.0;

// ---------------------------------------------------------------------
// The volumes, written without the kernel.

/// Simpson's rule for `f` on `[a, b]` with `n` (even) intervals.
fn simpson(f: impl Fn(f64) -> f64, a: f64, b: f64, n: u32) -> f64 {
    let h = (b - a) / f64::from(n);
    let mut sum = f(a) + f(b);
    for i in 1..n {
        let w = if i % 2 == 1 { 4.0 } else { 2.0 };
        sum += w * f(a + h * f64::from(i));
    }
    sum * h / 3.0
}

/// The area of the disc of radius `s` about the origin within the disc of
/// radius 1 about (0, 1) (whose border passes through the origin): the
/// lens of two circles whose centres are 1 apart.
fn lens_area(s: f64) -> f64 {
    if s <= 0.0 {
        0.0
    } else if s >= 2.0 {
        std::f64::consts::PI
    } else {
        (1.0 - 0.5 * s * s).pacos() + s * s * (0.5 * s).pacos() - 0.5 * s * (4.0 - s * s).sqrt()
    }
}

/// The volume of the lens, about the cylinder's axis: at the distance
/// `rho` from it and the angle `phi`, the lens is the stretch of x where
/// `x² + y² + z² <= R²`, and `y² + z² = 1 + rho² - 2 rho cos phi`.
fn lens_volume_by_rings() -> f64 {
    let rings = |rho: f64| {
        let m = 1024_u32;
        let mut sum = 0.0;
        for j in 0..m {
            let phi = std::f64::consts::TAU * f64::from(j) / f64::from(m);
            sum += 2.0 * (R * R - 1.0 - rho * rho + 2.0 * rho * phi.pcos()).sqrt();
        }
        rho * sum * std::f64::consts::TAU / f64::from(m)
    };
    simpson(rings, 0.0, 1.0, 2000)
}

/// The volume of the lens, across x: at x, the disc of radius
/// `sqrt(R² - x²)` about the axis x within the cylinder's disc.
fn lens_volume_by_slices() -> f64 {
    let full = (R * R - 4.0).sqrt();
    let edge = simpson(
        |x| lens_area((R * R - x * x).max(0.0).sqrt()),
        full,
        R,
        200_000,
    );
    2.0 * (std::f64::consts::PI * full + edge)
}

/// The notch's volume within the cylinder, in polar co-ordinates about
/// its centre in the plane x = 0.5: `y = r cos phi`, `z = r sin phi`; the
/// cylinder's disc is `r <= 2 sin phi`, the ball's stretch of x at r is
/// `2 sqrt(NOTCH² - r²)`, and its integral over r up to m is
/// `2/3 (NOTCH³ - (NOTCH² - m²)^(3/2))`.
fn notch_volume_by_rings() -> f64 {
    let n3 = NOTCH * NOTCH * NOTCH;
    let cap = |m: f64| {
        let rest = (NOTCH * NOTCH - m * m).max(0.0);
        2.0 / 3.0 * (n3 - rest * rest.sqrt())
    };
    let phi0 = (0.5 * NOTCH).pasin();
    let side = simpson(|phi| cap(2.0 * phi.psin()), 0.0, phi0, 100_000);
    2.0 * side + 2.0 / 3.0 * n3 * (std::f64::consts::PI - 2.0 * phi0)
}

/// The notch's volume within the cylinder, across x.
fn notch_volume_by_slices() -> f64 {
    simpson(
        |t| lens_area((NOTCH * NOTCH - t * t).max(0.0).sqrt()),
        -NOTCH,
        NOTCH,
        200_000,
    )
}

/// The rod's volume: a cylinder of radius 1, 6 long.
fn rod_volume() -> f64 {
    6.0 * std::f64::consts::PI
}

// ---------------------------------------------------------------------
// The bodies.

#[derive(Clone, Copy, Debug)]
struct Case {
    /// The scale of every length.
    k: f64,
    /// Turned out of the axes.
    turned: bool,
}

const PLAIN: Case = Case {
    k: 1.0,
    turned: false,
};

impl Case {
    fn several() -> [Case; 4] {
        [
            PLAIN,
            Case { k: 1e-3, ..PLAIN },
            Case { k: 1e6, ..PLAIN },
            Case {
                turned: true,
                ..PLAIN
            },
        ]
    }

    fn tolerance(self) -> Tolerance {
        Tolerance::new(1e-6 * self.k, 1e-9).unwrap()
    }

    fn turn(self) -> Transform {
        Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
            .unwrap()
            .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * self.k))
    }

    /// `body`, made in the test's axes, where this case puts it.
    fn place(self, body: Body) -> Body {
        if self.turned {
            body.transformed(&self.turn()).unwrap()
        } else {
            body
        }
    }

    /// A point of the result back in the test's axes and units.
    fn local(self, p: Vec3) -> Vec3 {
        let p = if self.turned {
            self.turn().inverse().unwrap().point(p)
        } else {
            p
        };
        p * (1.0 / self.k)
    }

    fn cuboid(self, (lo, hi): Cuboid) -> Body {
        let k = self.k;
        let frame = Frame::new(Vec3::new(lo[0], lo[1], lo[2]) * k, Vec3::Z, Vec3::X).unwrap();
        let body = primitives::block(
            &frame,
            (hi[0] - lo[0]) * k,
            (hi[1] - lo[1]) * k,
            (hi[2] - lo[2]) * k,
        )
        .unwrap();
        self.place(body)
    }

    /// The solid cylinder of radius 1 about the line y = 0, z = `z`, from
    /// x = `from` to `to`; its circles' vertices at the top, away from the
    /// floor.
    fn cylinder(self, from: f64, to: f64, z: f64) -> Body {
        let k = self.k;
        let frame = Frame::new(Vec3::new(from, 0.0, z) * k, Vec3::X, Vec3::Z).unwrap();
        primitives::cylinder(&frame, k, (to - from) * k).unwrap()
    }

    /// A ball about `centre`, its poles along y and its seam below its
    /// centre in the plane x = centre.x, away from the lens's caps and from
    /// the cylinder.
    fn ball(self, centre: Vec3, radius: f64) -> Body {
        let frame = Frame::new(centre * self.k, Vec3::Y, -Vec3::Z).unwrap();
        primitives::sphere(&frame, radius * self.k).unwrap()
    }

    /// The lens, in the block (`below` false) or under it, made in the
    /// test's axes.
    fn lens_here(self, below: bool) -> Body {
        let tol = self.tolerance();
        let z = if below { -1.0 } else { 1.0 };
        let cylinder = self.cylinder(-4.0, 4.0, z);
        let ball = self.ball(Vec3::ZERO, R);
        boolean(&cylinder, &ball, Intersect, &tol).unwrap().body
    }

    fn lens(self, below: bool) -> Body {
        self.place(self.lens_here(below))
    }

    /// `body`, made in the test's axes, less the notch.
    fn notched(self, body: &Body) -> Body {
        let tol = self.tolerance();
        let notch = self.ball(Vec3::new(NOTCH_AT, 0.0, 0.0), NOTCH);
        self.place(boolean(body, &notch, Subtract, &tol).unwrap().body)
    }
}

/// What a boolean should come to.
#[derive(Clone, Copy, Debug)]
enum Want {
    /// A body of this volume, in the test's units.
    Body(f64),
    /// Nothing.
    Empty,
    /// Two parts that touch along the line y = 0, z = 0, refused.
    ZeroThickness,
}

/// The boolean of `a` and `b`, judged against `want`: a body passes
/// `topo::check` and the boolean's full check and has the volume wanted to
/// within the tolerance times its area; a refusal as zero thickness is at
/// a point of the line of touch, within twenty tolerances of it. Every
/// other outcome is a failure, added to `failures`.
fn judge(
    case: Case,
    label: &str,
    (a, b): (&Body, &Body),
    op: BooleanOp,
    want: Want,
    failures: &mut Vec<String>,
) {
    let tol = case.tolerance();
    let k3 = case.k * case.k * case.k;
    let mut fail = |why: String| {
        println!("{label} at {case:?}: FAILS: {why}");
        failures.push(format!("{label} at {case:?}: {why}"));
    };
    match boolean(a, b, op, &tol) {
        Ok(result) => {
            let body = result.body;
            let defects = check(&body, &tol);
            if !defects.is_empty() {
                return fail(format!("built, with defects {defects:?}"));
            }
            match verify(a, b, op, &body, &tol, &VerifyOptions::default()) {
                Ok(found) if found.violations.is_empty() => {}
                Ok(found) => return fail(format!("built, violating {:?}", found.violations)),
                Err(e) => return fail(format!("built, not verified: {e}")),
            }
            let mass = match mass_properties(&body) {
                Ok(mass) => mass,
                Err(e) => return fail(format!("built, not measured: {e:?}")),
            };
            let volume = mass.volume / k3;
            println!(
                "{label} at {case:?}: built, {} faces, {} shells, volume {volume:.9}, \
                 wanted {want:?}",
                body.faces.len(),
                body.shells.len(),
            );
            match want {
                Want::Body(expected) => {
                    let allowed = tol.linear * mass.area / k3;
                    if (volume - expected).abs() > allowed {
                        fail(format!(
                            "volume {volume:.12} against {expected:.12}, allowed {allowed:e}"
                        ));
                    }
                }
                Want::Empty | Want::ZeroThickness => fail(format!("built where {want:?}")),
            }
        }
        Err(KernelError::EmptyResult { .. }) => {
            println!("{label} at {case:?}: empty, wanted {want:?}");
            if !matches!(want, Want::Empty) {
                fail(format!("empty where {want:?}"));
            }
        }
        Err(KernelError::ZeroThickness { at, .. }) => {
            let p = case.local(at);
            let off = p.y.abs().max(p.z.abs());
            println!(
                "{label} at {case:?}: zero thickness at {p:?}, {off:e} off the line, \
                 wanted {want:?}"
            );
            if !matches!(want, Want::ZeroThickness) {
                fail(format!("zero thickness at {p:?} where {want:?}"));
            } else if off > 2e-5 || p.x.abs() > R + 2e-5 {
                fail(format!("zero thickness at {p:?}, not on the line of touch"));
            }
        }
        Err(e) => fail(format!("refused where {want:?}: {e}")),
    }
}

/// Gathered failures, as the test's own.
fn none(failures: &[String]) {
    assert!(
        failures.is_empty(),
        "{} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------

/// The oracles: the lens's and the notch's volumes two ways, and the lens
/// the kernel makes against them.
#[test]
fn the_volumes_two_ways() {
    let (rings, slices) = (lens_volume_by_rings(), lens_volume_by_slices());
    println!("the lens: {rings:.12} by rings, {slices:.12} by slices");
    assert!((rings - slices).abs() < 1e-9, "{rings} {slices}");
    let (rings_n, slices_n) = (notch_volume_by_rings(), notch_volume_by_slices());
    println!("the notch: {rings_n:.12} by rings, {slices_n:.12} by slices");
    assert!((rings_n - slices_n).abs() < 1e-11, "{rings_n} {slices_n}");
    // Less than half the ball (the cylinder curves away from the plane
    // tangent to it at the ball's centre), more than the half ball over
    // the angles where the cylinder's disc reaches past the ball's.
    let half = 2.0 / 3.0 * std::f64::consts::PI * NOTCH * NOTCH * NOTCH;
    let phi0 = (0.5 * NOTCH).pasin();
    let least = half * (1.0 - 2.0 * phi0 / std::f64::consts::PI);
    assert!(
        rings_n < half && rings_n > least,
        "{rings_n} {least} {half}"
    );
    for below in [false, true] {
        let lens = PLAIN.lens_here(below);
        let mass = mass_properties(&lens).unwrap();
        println!("the lens made, below {below}: volume {:.12}", mass.volume);
        assert!(
            (mass.volume - rings).abs() <= 1e-6 * mass.area,
            "{} against {rings}",
            mass.volume
        );
    }
}

/// One boolean to judge: its label, its operands, the operation and what
/// it should come to.
type Job<'b> = (&'static str, &'b Body, &'b Body, BooleanOp, Want);

/// The lens within the block, touching its floor from inside along the
/// line: the union is the block, the common part the lens, the lens less
/// the block nothing. Each keeps one of the two faces that touch, or
/// neither, so the rule leaves the floor's stretch out where it ends clear
/// outside the wall.
#[test]
#[ignore = "owed, as on main: refused as at the edge of tolerance, a piece of the block's floor \
            within the tolerance of the lens's wall (Check 45698408500 on main, 45698388808 on \
            the branch, every case the same)"]
fn a_lens_within_a_block_united_and_in_common() {
    let lens_v = lens_volume_by_rings();
    let mut failures = Vec::new();
    for case in Case::several() {
        let lens = case.lens(false);
        for (name, cuboid) in [("centred", CENTRED), ("aside", ASIDE)] {
            let block = case.cuboid(cuboid);
            let jobs: [Job; 5] = [
                ("block + lens", &block, &lens, Union, Want::Body(BLOCK)),
                ("lens + block", &lens, &block, Union, Want::Body(BLOCK)),
                ("block * lens", &block, &lens, Intersect, Want::Body(lens_v)),
                ("lens * block", &lens, &block, Intersect, Want::Body(lens_v)),
                ("lens - block", &lens, &block, Subtract, Want::Empty),
            ];
            for (what, x, y, op, want) in jobs {
                let label = format!("{what}, the block {name}");
                judge(case, &label, (x, y), op, want, &mut failures);
            }
        }
    }
    none(&failures);
}

/// The lens (and the notched lens) cut from the block: the result keeps
/// both faces, so the rule does not read it. The cavity's wall touches the
/// floor along the line from within, and its cap meets the floor in the
/// point at either end of it; refused as zero thickness at such an end,
/// on `main` too (Check 45698408500). Which of the decisions of 2026-10-15
/// and 2026-10-18 holds there is not this review's (D-open-15): this pins
/// what `main` answers, it proves nothing.
#[test]
fn a_lens_cut_from_a_block() {
    let mut failures = Vec::new();
    for case in Case::several() {
        let lens = case.lens(false);
        for (name, cuboid) in [("centred", CENTRED), ("aside", ASIDE)] {
            let block = case.cuboid(cuboid);
            let label = format!("block - lens, the block {name}");
            let want = Want::ZeroThickness;
            judge(case, &label, (&block, &lens), Subtract, want, &mut failures);
        }
    }
    for case in [
        PLAIN,
        Case {
            turned: true,
            ..PLAIN
        },
    ] {
        let lens = case.notched(&case.lens_here(false));
        let block = case.cuboid(CENTRED);
        let label = "block - notched lens";
        let want = Want::ZeroThickness;
        judge(case, label, (&block, &lens), Subtract, want, &mut failures);
    }
    none(&failures);
}

/// The lens under the block, touching its floor from below along the
/// line: the block less the lens is the block, the lens less the block the
/// lens, the common part nothing. Each keeps one of the two faces or
/// neither.
#[test]
fn a_lens_under_a_block_cut_and_in_common() {
    let lens_v = lens_volume_by_rings();
    let mut failures = Vec::new();
    for case in Case::several() {
        let lens = case.lens(true);
        for (name, cuboid) in [("centred", CENTRED), ("aside", ASIDE)] {
            let block = case.cuboid(cuboid);
            let jobs: [Job; 4] = [
                ("block - lens", &block, &lens, Subtract, Want::Body(BLOCK)),
                ("lens - block", &lens, &block, Subtract, Want::Body(lens_v)),
                ("block * lens", &block, &lens, Intersect, Want::Empty),
                ("lens * block", &lens, &block, Intersect, Want::Empty),
            ];
            for (what, x, y, op, want) in jobs {
                let label = format!("{what}, the block {name}");
                judge(case, &label, (x, y), op, want, &mut failures);
            }
        }
    }
    none(&failures);
}

/// The lens under the block united with it: two parts that rest on each
/// other along the line, refused as zero thickness (2026-10-15). The
/// result keeps both faces; the rule does not read this one.
#[test]
fn a_lens_under_a_block_united() {
    let mut failures = Vec::new();
    for case in Case::several() {
        let lens = case.lens(true);
        for (name, cuboid) in [("centred", CENTRED), ("aside", ASIDE)] {
            let block = case.cuboid(cuboid);
            let jobs: [Job; 2] = [
                ("block + lens", &block, &lens, Union, Want::ZeroThickness),
                ("lens + block", &lens, &block, Union, Want::ZeroThickness),
            ];
            for (what, x, y, op, want) in jobs {
                let label = format!("{what}, the block {name}");
                judge(case, &label, (x, y), op, want, &mut failures);
            }
        }
    }
    none(&failures);
}

/// The block ending a few tolerances past the lens, so that the floor's
/// stretch ends near the wall's border: at 2.5 and 3 tolerances about the
/// distance at which an end is clear outside the wall (twice the
/// tolerance from its border), further at 5, 10 and 100.
#[test]
#[ignore = "owed, as on main: refused as at the edge of tolerance, a piece of the block's floor \
            within the tolerance of the lens's wall (Check 45698408500 on main, 45698388808 on \
            the branch, every case the same)"]
fn the_block_ending_a_few_tolerances_past_the_lens() {
    let lens_v = lens_volume_by_rings();
    let mut failures = Vec::new();
    let case = PLAIN;
    let lens = case.lens(false);
    for d in [2.5, 3.0, 5.0, 10.0, 100.0] {
        let end = R + d * 1e-6;
        let block = case.cuboid(([-end, -3.0, 0.0], [end, 3.0, 4.0]));
        let volume = 2.0 * end * 6.0 * 4.0;
        let jobs: [Job; 4] = [
            ("block + lens", &block, &lens, Union, Want::Body(volume)),
            ("lens + block", &lens, &block, Union, Want::Body(volume)),
            ("block * lens", &block, &lens, Intersect, Want::Body(lens_v)),
            ("lens - block", &lens, &block, Subtract, Want::Empty),
        ];
        for (what, x, y, op, want) in jobs {
            let label = format!("{what}, the block ending {d} tolerances past the lens");
            judge(case, &label, (x, y), op, want, &mut failures);
        }
    }
    none(&failures);
}

/// The lens less the notch within the block: the floor's stretch ends
/// clear outside the wall and leaves it and comes back on the way, at the
/// notch, between the samples (x = 0.3 to 0.7; the samples at 0 and
/// ±2.5). Where the result keeps one face or neither the rule leaves it
/// out by its ends. (The block less the notched lens, which keeps both, is
/// in `a_lens_cut_from_a_block`.)
#[test]
#[ignore = "owed, as on main: refused as at the edge of tolerance, a piece of the block's floor \
            within the tolerance of the lens's wall (Check 45698408500 on main, 45698388808 on \
            the branch, every case the same)"]
fn a_notched_lens_within_a_block() {
    let part = lens_volume_by_rings() - notch_volume_by_rings();
    let mut failures = Vec::new();
    for case in [
        PLAIN,
        Case {
            turned: true,
            ..PLAIN
        },
    ] {
        let lens = case.notched(&case.lens_here(false));
        let block = case.cuboid(CENTRED);
        let jobs: [Job; 3] = [
            (
                "block + notched lens",
                &block,
                &lens,
                Union,
                Want::Body(BLOCK),
            ),
            (
                "block * notched lens",
                &block,
                &lens,
                Intersect,
                Want::Body(part),
            ),
            ("notched lens - block", &lens, &block, Subtract, Want::Empty),
        ];
        for (what, x, y, op, want) in jobs {
            judge(case, what, (x, y), op, want, &mut failures);
        }
    }
    none(&failures);
}

/// The rod less the notch within the block: the floor's stretch along the
/// rod ends on the rod's circles (cut in closed form, on the wall's
/// border), and leaves the wall and comes back between its samples (x =
/// 0.3 to 0.7; the samples at 0 and ±1.5), where the notch's spline edge
/// only touches the floor. The rule does not see it by its ends (the
/// design note's *Owed*); built right all the same, on `main` too. The
/// block less the notched rod keeps both faces and is refused as zero
/// thickness at an end of the line, as the block less the lens is (on
/// `main` too; a pin, not a proof).
#[test]
fn a_notched_rod_within_a_block() {
    let part = rod_volume() - notch_volume_by_rings();
    let mut failures = Vec::new();
    for case in [
        PLAIN,
        Case {
            turned: true,
            ..PLAIN
        },
    ] {
        let rod = case.notched(&case.cylinder(-3.0, 3.0, 1.0));
        let block = case.cuboid(CENTRED);
        let jobs: [Job; 4] = [
            (
                "block + notched rod",
                &block,
                &rod,
                Union,
                Want::Body(BLOCK),
            ),
            (
                "block * notched rod",
                &block,
                &rod,
                Intersect,
                Want::Body(part),
            ),
            ("notched rod - block", &rod, &block, Subtract, Want::Empty),
            (
                "block - notched rod",
                &block,
                &rod,
                Subtract,
                Want::ZeroThickness,
            ),
        ];
        for (what, x, y, op, want) in jobs {
            judge(case, what, (x, y), op, want, &mut failures);
        }
    }
    none(&failures);
}
