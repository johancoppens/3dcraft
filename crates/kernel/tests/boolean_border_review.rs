//! Around a meeting curve that leaves a face's border: the band of a few
//! tolerances and borders that curve (the review of the repair of
//! 2026-11-20, docs/design/booleans.md › *Contact refused where a curve
//! lies exactly on a face's border*; its own tests are
//! `boolean_border_tangent.rs`).
//!
//! That repair reads a stretch of a meeting curve next to a face's border
//! differently, and tells bodies that touch in a point with no vertex
//! there to touch. Its tests hold the place of tangency itself, and places
//! 40 tolerances off it. These hold what lies between and beside:
//!
//! - *The band.* The same plate and cylinder with the footprint a
//!   fraction of a tolerance to six tolerances short of the plate's edge
//!   or over it, and the cylinder as much above the plate's top or sunk
//!   into it. What the pair is follows from its sizes in closed form: how
//!   far apart, or how thick what the two share.
//! - *A border that curves.* A plate whose front is a bump, an arc of a
//!   circle or a parabola drawn as a spline, and a cylinder tangent to it
//!   at its lowest point: from inside with a radius well under the
//!   border's radius of curvature, next to it (the rim then stays within
//!   the tolerance of the border for a long way), equal to it (an arc: the
//!   rim lies along the border) and just over it (the rim leaves the face
//!   and comes back); and from outside.
//!
//! A refusal is always allowed here: these are places at the edge of
//! tolerance. What is held is that no answer is wrong: bodies that share a
//! point are never clear, bodies more than three tolerances apart never
//! meet, bodies that share no volume never overlap, a common volume three
//! tolerances thick or more is never missed, `contact` and `interference`
//! agree, and a boolean that is built is valid, passes the boolean's own
//! check in full and has its closed-form volume. And that the places of
//! tangency themselves, which the repair's own tests hold, are answered.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;
mod common;

use boolean_common::SCALES;
use common::{arc, line};
use craft3d_geom::{Curve3, NurbsCurve};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::boolean::{
    BooleanOptions, Contact, Interference, VerifyOptions, boolean_with, contact, interference,
    verify,
};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{Body, check, mass_properties};
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI};

/// The radius of the cylinder, times the scale.
const RADIUS: f64 = 4.0;

/// The tolerance at scale `k`: a millionth of the unit, or a coarse one
/// (0.01 at a radius of 4, as a Jig's millimetres).
fn tolerance(k: f64, coarse: bool) -> Tolerance {
    Tolerance::new(if coarse { 1e-2 * k } else { 1e-6 * k }, 1e-9).unwrap()
}

/// What a pair is by its sizes, in tolerances.
#[derive(Clone, Copy, Debug)]
struct Truth {
    /// How far apart the bodies are; 0 when they share a point.
    apart: f64,
    /// How thick what they share is; 0 when they share no volume.
    thick: f64,
    /// The volume they share.
    common: f64,
}

/// What a run saw.
#[derive(Default)]
struct Tally {
    places: usize,
    /// Wrong answers: any of them fails the test.
    faults: Vec<String>,
    /// Everything else, counted by kind.
    seen: BTreeMap<String, usize>,
}

impl Tally {
    fn saw(&mut self, kind: &str) {
        *self.seen.entry(kind.to_string()).or_insert(0) += 1;
    }

    /// Prints what was seen and fails on a wrong answer.
    fn hold(self, what: &str) {
        println!("{what}: {} places", self.places);
        for (kind, count) in &self.seen {
            println!("  {count:>6}  {kind}");
        }
        assert!(
            self.faults.is_empty(),
            "{} wrong answers at {} places of {what}:\n  {}",
            self.faults.len(),
            self.places,
            self.faults.join("\n  ")
        );
    }
}

/// Why something was refused, without the faces and the place it names.
fn why(e: &KernelError) -> String {
    let text = e.to_string();
    let cut = text
        .find(" (face")
        .or_else(|| text.find(" at ("))
        .unwrap_or(text.len());
    text[..cut].to_string()
}

fn told(found: &Interference) -> Contact {
    match found {
        Interference::Clear => Contact::Clear,
        Interference::Touch { .. } => Contact::Touch,
        Interference::Overlap(_) => Contact::Overlap,
    }
}

/// How an answer is wrong, if it is.
fn wrong(found: Contact, truth: Truth) -> Option<&'static str> {
    match found {
        Contact::Clear if truth.apart == 0.0 => Some("clear, where they share a point"),
        Contact::Touch | Contact::Overlap if truth.apart > 3.0 => {
            Some("they meet, where they are more than three tolerances apart")
        }
        Contact::Overlap if truth.thick == 0.0 => Some("an overlap, where they share no volume"),
        Contact::Clear | Contact::Touch if truth.thick >= 3.0 => {
            Some("no overlap, where what they share is three tolerances thick or more")
        }
        _ => None,
    }
}

/// The booleans as the interference query runs them: parts that touch are
/// allowed.
fn unrefused() -> BooleanOptions {
    BooleanOptions {
        refuse_zero_thickness: false,
        ..BooleanOptions::default()
    }
}

/// Holds `contact` and `interference` of a pair to what it is, and with
/// `booleans` its union, difference and intersection to their validity,
/// the boolean's own check in full and their volumes; with `meshed` each
/// result is meshed as well, and one that cannot be is counted. `answered`
/// says the pair must not be refused by either query.
fn judge(
    a: &Body,
    b: &Body,
    truth: Truth,
    tol: &Tolerance,
    (answered, booleans, meshed): (bool, bool, bool),
    what: &str,
    tally: &mut Tally,
) {
    tally.places += 1;
    let simple = contact(a, b, tol);
    let full = interference(a, b, tol);
    let answers = [
        ("contact", simple.as_ref().copied().map_err(why)),
        ("interference", full.as_ref().map(told).map_err(why)),
    ];
    for (name, answer) in &answers {
        match answer {
            Ok(found) => {
                if let Some(how) = wrong(*found, truth) {
                    tally
                        .faults
                        .push(format!("{what}: {name} says {found:?}: {how}"));
                } else if *found == Contact::Clear && truth.apart <= 0.5 {
                    tally.saw(&format!("{name}: clear within half the tolerance"));
                } else {
                    tally.saw(&format!("{name}: {found:?}"));
                }
            }
            Err(reason) if answered => {
                tally
                    .faults
                    .push(format!("{what}: {name} refused: {reason}"));
            }
            Err(reason) => tally.saw(&format!("{name} refused: {reason}")),
        }
    }
    if let [(_, Ok(simple)), (_, Ok(full))] = &answers
        && simple != full
    {
        tally.faults.push(format!(
            "{what}: contact says {simple:?} and interference {full:?}"
        ));
    }
    if !booleans {
        return;
    }
    let (Ok(ma), Ok(mb)) = (mass_properties(a), mass_properties(b)) else {
        tally
            .faults
            .push(format!("{what}: an operand cannot be measured"));
        return;
    };
    // A volume is known to the tolerance over the boundary.
    let slack = tol.linear * (ma.area + mb.area) + 1e-9 * (ma.volume + mb.volume);
    for (op, want) in [
        (Union, ma.volume + mb.volume - truth.common),
        (Subtract, ma.volume - truth.common),
        (Intersect, truth.common),
    ] {
        match boolean_with(a, b, op, tol, &unrefused()) {
            Ok(result) => {
                let defects = check(&result.body, tol);
                if !defects.is_empty() {
                    tally
                        .faults
                        .push(format!("{what}: {op:?} is invalid: {defects:?}"));
                    continue;
                }
                match verify(a, b, op, &result.body, tol, &VerifyOptions::default()) {
                    Ok(found) if found.violations.is_empty() => {}
                    Ok(found) => {
                        let all: Vec<String> =
                            found.violations.iter().map(ToString::to_string).collect();
                        tally.faults.push(format!(
                            "{what}: {op:?} breaks the boolean's check: {}",
                            all.join("; ")
                        ));
                        continue;
                    }
                    Err(_) => tally.saw("a result the boolean's check cannot run on"),
                }
                match mass_properties(&result.body) {
                    Ok(found) if (found.volume - want).abs() <= slack => {
                        tally.saw("a boolean built, at its volume");
                    }
                    Ok(found) => tally.faults.push(format!(
                        "{what}: {op:?} has volume {}, not {want}",
                        found.volume
                    )),
                    Err(_) => tally.saw("a result that cannot be measured"),
                }
                if meshed {
                    let params = craft3d_tess::SelfCheckParams::new(tol.linear);
                    match craft3d_tess::body_self_intersections(&result.body, &params) {
                        Ok(deep) if deep.certain_crossings == 0 => {}
                        Ok(_) => tally.saw(&format!("{op:?}: a result whose mesh crosses itself")),
                        Err(_) => tally.saw(&format!("{op:?}: a result that cannot be meshed")),
                    }
                }
            }
            Err(KernelError::EmptyResult { .. }) => {
                if want.abs() <= slack {
                    tally.saw("a boolean empty, as it is");
                } else {
                    tally
                        .faults
                        .push(format!("{what}: {op:?} is empty, not {want}"));
                }
            }
            Err(e) => tally.saw(&format!("a boolean refused: {}", why(&e))),
        }
    }
}

/// From which side the cylinder's footprint is tangent to the border.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Side {
    /// On the plate: a boss.
    Inside,
    /// Beside the plate.
    Outside,
}

// ---------------------------------------------------------------------
// The band: a plate and a cylinder at its top face's front edge.

/// Where along the plate's front edge the footprint is nearest to it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Along {
    /// At the middle of the edge.
    Middle,
    /// At the edge's end, a corner of the top face.
    Corner,
}

/// One place of the band: the plate `[-10, 10] × [0, 10] × [-5, 0]` and
/// a cylinder of radius 4 and height 6, times the scale.
#[derive(Clone, Copy, Debug)]
struct Place {
    k: f64,
    coarse: bool,
    /// The plate's edges are splines (the same straight lines, as cubic
    /// B-splines): a freeform border.
    splines: bool,
    /// Both bodies turned by this about z.
    turn: f64,
    along: Along,
    side: Side,
    /// How far the footprint is moved into the plate from tangency, in
    /// tolerances.
    shift: f64,
    /// How far the cylinder's base is above the plate's top, in
    /// tolerances.
    lift: f64,
    /// The offset of the rim's vertex from the tangent at the place
    /// nearest the edge, in tolerances; `None` a quarter turn from it.
    offset: Option<f64>,
}

impl Place {
    fn tol(&self) -> Tolerance {
        tolerance(self.k, self.coarse)
    }

    /// Where the footprint's centre is across the edge (the edge is
    /// `y = 0`, the plate `y ≥ 0`, before the turn).
    fn centre(&self) -> f64 {
        let r = RADIUS * self.k;
        let shift = self.shift * self.tol().linear;
        match self.side {
            Side::Inside => r + shift,
            Side::Outside => -r + shift,
        }
    }

    fn turned(&self, p: Vec3) -> Vec3 {
        let (s, c) = self.turn.psin_cos();
        Vec3::new(c * p.x - s * p.y, s * p.x + c * p.y, p.z)
    }

    fn plate(&self) -> Body {
        let k = self.k;
        let frame = Frame::new(
            self.turned(Vec3::new(-10.0 * k, 0.0, -5.0 * k)),
            Vec3::Z,
            self.turned(Vec3::X),
        )
        .unwrap();
        let mut plate = primitives::block(&frame, 20.0 * k, 10.0 * k, 5.0 * k).unwrap();
        if self.splines {
            let points: Vec<Vec3> = plate.vertices.iter().map(|v| v.point).collect();
            for edge in &mut plate.edges {
                let (a, b) = (points[edge.start.0 as usize], points[edge.end.0 as usize]);
                let length = a.distance(b);
                let ctrl = vec![a, a + (b - a) * (1.0 / 3.0), a + (b - a) * (2.0 / 3.0), b];
                let knots = vec![0.0, 0.0, 0.0, 0.0, length, length, length, length];
                edge.curve = Curve3::Nurbs(NurbsCurve::new(3, knots, ctrl, vec![1.0; 4]).unwrap());
                edge.range = (0.0, length);
            }
        }
        plate
    }

    fn cylinder(&self) -> Body {
        let (k, tol) = (self.k, self.tol().linear);
        let r = RADIUS * k;
        let away = match self.offset {
            Some(e) => (1.0 - e * tol / r).pacos(),
            None => FRAC_PI_2,
        };
        let angle = match self.side {
            Side::Inside => -FRAC_PI_2 + away,
            Side::Outside => FRAC_PI_2 - away,
        };
        let x = if self.along == Along::Middle {
            0.0
        } else {
            -10.0 * k
        };
        let frame = Frame::new(
            self.turned(Vec3::new(x, self.centre(), self.lift * tol)),
            Vec3::Z,
            self.turned(Vec3::new(angle.pcos(), angle.psin(), 0.0)),
        )
        .unwrap();
        primitives::cylinder(&frame, r, 6.0 * k).unwrap()
    }

    /// The area the footprint and the plate's top share: of the disc of
    /// radius `r` about `(0, c)`, the part with `y ≥ 0`; half of it with
    /// the disc's centre over the plate's side.
    fn shared(&self) -> f64 {
        let (r, c) = (RADIUS * self.k, self.centre());
        let above = if c >= r {
            PI * r * r
        } else if c <= -r {
            0.0
        } else {
            r * r * (PI - (c / r).pacos()) + c * (r * r - c * c).sqrt()
        };
        if self.along == Along::Middle {
            above
        } else {
            0.5 * above
        }
    }

    /// What the pair is. Beside the plate and short of its edge, the
    /// bodies are apart by that much across and by the lift above. Else
    /// the footprint reaches the plate's top: lifted, they are apart by
    /// the lift; resting, they touch; sunk, they share the footprint's
    /// part over the plate times the depth, as thick as the lesser of the
    /// two (from inside the footprint is as wide as the cylinder).
    fn truth(&self) -> Truth {
        let (shift, lift) = (self.shift, self.lift);
        let none = |apart: f64| Truth {
            apart,
            thick: 0.0,
            common: 0.0,
        };
        if self.side == Side::Outside && shift < 0.0 {
            return none((-shift).phypot(lift.max(0.0)));
        }
        let reach = match self.side {
            Side::Outside => shift,
            Side::Inside => f64::INFINITY,
        };
        if lift > 0.0 {
            none(lift)
        } else if lift == 0.0 || reach == 0.0 {
            none(0.0)
        } else {
            Truth {
                apart: 0.0,
                thick: reach.min(-lift),
                common: self.shared() * -lift * self.tol().linear,
            }
        }
    }
}

/// The band at one scale: both tolerances; the plate's edges as lines in
/// the world's axes and as splines turned by half a radian.
fn band(k: f64) {
    let mut tally = Tally::default();
    for coarse in [false, true] {
        for (splines, turn) in [(false, 0.0), (true, 0.5)] {
            for along in [Along::Middle, Along::Corner] {
                for side in [Side::Inside, Side::Outside] {
                    for shift in [0.0, 0.3, -0.3, 0.8, -0.8, 1.9, -1.9, 6.0, -6.0] {
                        for lift in [0.0, 0.4, 1.5, -0.4, -50.0] {
                            for offset in [Some(1.4), Some(3.0), None] {
                                let place = Place {
                                    k,
                                    coarse,
                                    splines,
                                    turn,
                                    along,
                                    side,
                                    shift,
                                    lift,
                                    offset,
                                };
                                // The place of tangency itself is held by
                                // the repair's own tests: answered.
                                let answered = shift == 0.0 && lift == 0.0;
                                let booleans =
                                    (lift == 0.0 || lift == -50.0) && offset == Some(3.0);
                                judge(
                                    &place.plate(),
                                    &place.cylinder(),
                                    place.truth(),
                                    &place.tol(),
                                    (answered, booleans, false),
                                    &format!("{place:?}"),
                                    &mut tally,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    tally.hold(&format!("the band at {k}"));
}

#[test]
fn a_few_tolerances_short_of_a_plates_edge_or_over_it_at_a_thousandth() {
    band(SCALES[0]);
}

#[test]
fn a_few_tolerances_short_of_a_plates_edge_or_over_it_at_one() {
    band(SCALES[1]);
}

#[test]
fn a_few_tolerances_short_of_a_plates_edge_or_over_it_at_a_million() {
    band(SCALES[2]);
}

// ---------------------------------------------------------------------
// A border that curves.

/// What the bump in the plate's front is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Border {
    /// An arc of a circle.
    Arc,
    /// A parabola, drawn as a cubic spline of one span.
    Parabola,
}

/// The plate `[-10, 10] × [0, 10] × [-5, 0]` with a bump in its front
/// about `x = 0`, and a cylinder of radius 4 tangent to the bump at its
/// lowest point, times the scale.
#[derive(Clone, Copy, Debug)]
struct Bump {
    k: f64,
    coarse: bool,
    border: Border,
    /// The cylinder's radius over the border's radius of curvature at
    /// its lowest point.
    ratio: f64,
    side: Side,
    /// How far the footprint is moved into the plate from tangency, in
    /// tolerances.
    shift: f64,
    /// How far the cylinder's base is above the plate's top, in
    /// tolerances.
    lift: f64,
    /// Where the rim's vertex is: the angle round the rim from the place
    /// of tangency at which a rim one hundredth off the border's radius
    /// would be that many tolerances off it; `None` a quarter turn.
    offset: Option<f64>,
}

impl Bump {
    fn tol(&self) -> Tolerance {
        tolerance(self.k, self.coarse)
    }

    /// The border's radius of curvature at its lowest point.
    fn rho(&self) -> f64 {
        RADIUS * self.k / self.ratio
    }

    /// Half the bump's width and how far below `y = 0` its lowest point
    /// is: an arc of 70° either way (50° of a large circle), or the
    /// parabola `y = -low (1 - x² / a²)`, whose radius of curvature at
    /// its lowest point is `a² / (2 low)`.
    fn bump(&self) -> (f64, f64) {
        let rho = self.rho();
        match self.border {
            Border::Arc => {
                let half: f64 = if rho < 8.0 * self.k { 70.0 } else { 50.0 };
                let half = half.to_radians();
                (rho * half.psin(), rho * (1.0 - half.pcos()))
            }
            Border::Parabola => {
                let a = (0.85 * rho).min(7.0 * self.k);
                (a, a * a / (2.0 * rho))
            }
        }
    }

    fn plate(&self) -> Result<Body, String> {
        let (k, tol) = (self.k, self.tol());
        let (a, low) = self.bump();
        let mut curves = vec![
            line(1, [a, 0.0], [10.0 * k, 0.0]),
            line(2, [10.0 * k, 0.0], [10.0 * k, 10.0 * k]),
            line(3, [10.0 * k, 10.0 * k], [-10.0 * k, 10.0 * k]),
            line(4, [-10.0 * k, 10.0 * k], [-10.0 * k, 0.0]),
            line(5, [-10.0 * k, 0.0], [-a, 0.0]),
        ];
        match self.border {
            Border::Arc => curves.push(arc(6, [0.0, self.rho() - low], [-a, 0.0], [a, 0.0])),
            Border::Parabola => {
                // A cubic of one span whose control points are evenly
                // spaced across: x runs evenly along it, and y is
                // `-3 h t (1 - t)`, the parabola with `low = 3 h / 4`.
                let h = low / 0.75;
                curves.push(SketchCurve {
                    id: 6,
                    curve: Curve2::Spline(Spline2 {
                        degree: 3,
                        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                        control: vec![[-a, 0.0], [-a / 3.0, -h], [a / 3.0, -h], [a, 0.0]],
                    }),
                });
            }
        }
        let profiles =
            detect_profiles(&curves, &tol).map_err(|e| format!("the plate's outline: {e}"))?;
        let plane = Frame::WORLD
            .at(Vec3::new(0.0, 0.0, -5.0 * k))
            .ok_or("the plate's plane")?;
        extrude(&profiles, &plane, 0.0, 5.0 * k, &tol)
            .map(|made| made.body)
            .map_err(|e| format!("the plate: {e}"))
    }

    /// Where the footprint's centre is across the plate's front.
    fn centre(&self) -> f64 {
        let r = RADIUS * self.k;
        let (_, low) = self.bump();
        let shift = self.shift * self.tol().linear;
        match self.side {
            Side::Inside => -low + r + shift,
            Side::Outside => -low - r + shift,
        }
    }

    fn cylinder(&self) -> Result<Body, String> {
        let (k, tol) = (self.k, self.tol().linear);
        let r = RADIUS * k;
        let away = match self.offset {
            Some(e) => {
                let mismatch = match self.side {
                    Side::Inside => (1.0 - self.ratio).abs().max(0.01),
                    Side::Outside => 1.0 + self.ratio,
                };
                (1.0 - e * tol / (r * mismatch)).clamp(-1.0, 1.0).pacos()
            }
            None => FRAC_PI_2,
        };
        let angle = match self.side {
            Side::Inside => -FRAC_PI_2 + away,
            Side::Outside => FRAC_PI_2 - away,
        };
        let frame = Frame::new(
            Vec3::new(0.0, self.centre(), self.lift * tol),
            Vec3::Z,
            Vec3::new(angle.pcos(), angle.psin(), 0.0),
        )
        .ok_or("the cylinder's frame")?;
        primitives::cylinder(&frame, r, 6.0 * k).map_err(|e| format!("the cylinder: {e}"))
    }

    /// What the pair is. From inside the footprint lies over the plate:
    /// all of it where the cylinder's radius is at most the border's
    /// radius of curvature (a circle inside a circle it is tangent to, or
    /// inside a parabola it is tangent to at its lowest point with no
    /// more than its curvature there); with a larger radius on an arc,
    /// the bump lies inside the footprint, and what the two share is the
    /// bump and the footprint's part behind the front line. From outside
    /// they touch in a point, are that far apart, or share a lens whose
    /// volume is far below what a volume is known to.
    fn truth(&self) -> Option<Truth> {
        let (shift, lift) = (self.shift, self.lift);
        let tol = self.tol().linear;
        let none = |apart: f64| Truth {
            apart,
            thick: 0.0,
            common: 0.0,
        };
        Some(match self.side {
            Side::Outside if shift < 0.0 => none((-shift).phypot(lift.max(0.0))),
            _ if lift > 0.0 => none(lift),
            _ if lift == 0.0 => none(0.0),
            Side::Outside if shift == 0.0 => none(0.0),
            Side::Outside => Truth {
                apart: 0.0,
                thick: shift.min(-lift),
                common: 0.0,
            },
            Side::Inside => {
                let r = RADIUS * self.k;
                let area = if self.ratio <= 1.0 {
                    PI * r * r
                } else if self.border == Border::Arc {
                    let (rho, c) = (self.rho(), self.centre());
                    let half: f64 = if rho < 8.0 * self.k { 70.0 } else { 50.0 };
                    let half = half.to_radians();
                    let bump = rho * rho * (half - half.psin() * half.pcos());
                    r * r * (PI - (c / r).pacos()) + c * (r * r - c * c).sqrt() + bump
                } else {
                    // A radius over a parabola's: no closed form here.
                    return None;
                };
                Truth {
                    apart: 0.0,
                    thick: -lift,
                    common: area * -lift * tol,
                }
            }
        })
    }
}

/// The curved borders at one scale, both tolerances.
fn curved(k: f64) {
    let mut tally = Tally::default();
    let cases = [
        (Border::Arc, Side::Inside, 0.4),
        (Border::Arc, Side::Inside, 0.99),
        (Border::Arc, Side::Inside, 1.0),
        (Border::Arc, Side::Inside, 1.01),
        (Border::Arc, Side::Outside, 0.4),
        (Border::Parabola, Side::Inside, 0.4),
        (Border::Parabola, Side::Inside, 0.99),
        (Border::Parabola, Side::Inside, 1.0),
        (Border::Parabola, Side::Outside, 0.4),
    ];
    for coarse in [false, true] {
        for (border, side, ratio) in cases {
            for shift in [0.0, -0.4] {
                for lift in [0.0, -50.0] {
                    for offset in [Some(1.4), None] {
                        let place = Bump {
                            k,
                            coarse,
                            border,
                            ratio,
                            side,
                            shift,
                            lift,
                            offset,
                        };
                        let what = format!("{place:?}");
                        let (plate, cylinder) = match (place.plate(), place.cylinder()) {
                            (Ok(plate), Ok(cylinder)) => (plate, cylinder),
                            (Err(reason), _) | (_, Err(reason)) => {
                                tally.faults.push(format!("{what}: not made: {reason}"));
                                continue;
                            }
                        };
                        let Some(truth) = place.truth() else {
                            continue;
                        };
                        // The place of tangency itself, with a radius
                        // well under the border's: answered, as on a
                        // straight border.
                        let answered = ratio == 0.4 && shift == 0.0 && lift == 0.0;
                        judge(
                            &plate,
                            &cylinder,
                            truth,
                            &place.tol(),
                            (answered, true, true),
                            &what,
                            &mut tally,
                        );
                    }
                }
            }
        }
    }
    tally.hold(&format!("the curved borders at {k}"));
}

#[test]
fn a_cylinder_tangent_to_a_border_that_curves_at_a_thousandth() {
    curved(SCALES[0]);
}

#[test]
fn a_cylinder_tangent_to_a_border_that_curves_at_one() {
    curved(SCALES[1]);
}

#[test]
fn a_cylinder_tangent_to_a_border_that_curves_at_a_million() {
    curved(SCALES[2]);
}
