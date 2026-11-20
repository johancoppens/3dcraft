//! A meeting curve that leaves a face's border tangentially, with a vertex
//! next to where it leaves (2026-11-20; the gear reducer Jig's keyed
//! shafts, docs/design/booleans.md › *Contact refused where a curve lies
//! exactly on a face's border*).
//!
//! Where the curve in which two surfaces meet touches the border of one
//! of the faces without crossing it, it stays within the tolerance of
//! that border for a while. A vertex of the other body on that stretch
//! (the seam of a closed curve an earlier boolean traced, the vertex of a
//! cylinder's rim) cuts a short piece off the curve: from the place of
//! tangency to the vertex. With the vertex between one and four
//! tolerances off the border that piece is within the tolerance of the
//! border at some of its samples and clear of it at others or at its
//! end, and was refused: "a curve that is partly on and partly off a face
//! boundary" or "an imprinted curve that does not fit the edges it should
//! coincide with".
//!
//! Two pairs, both made here:
//!
//! - *A plate and a cylinder at its edge.* The cylinder stands at the
//!   level of the plate's top face with its footprint tangent to the
//!   face's front edge, from inside (a boss on the plate) or from outside
//!   (beside the plate, touching its top front edge in one point), and
//!   the vertex of its rim `e` tolerances off that edge. Around it a grid:
//!   the footprint tangent, 40 tolerances further in and 40 further out;
//!   the cylinder resting, lifted (clear) and sunk in (overlap); in the
//!   world's axes and turned; closed forms for every answer (the area a
//!   disc and a half plane share).
//! - *A keyed shaft and a hub.* A shaft with a keyway whose rounded end
//!   was cut as two arcs (a vertex on the end's rim, `e` tolerances off
//!   the side of the hub's keyway), in a hub whose bore lies on the
//!   shaft's seat and whose keyway is as wide as the shaft's: the rim of
//!   the keyway's end is tangent to the border of the hub's bore, and the
//!   curve is a spline the marcher traces.
//!
//! At 1e-3, 1 and 1e6, with the tolerance a millionth of the unit and
//! with a coarse one (as the Jig's: a quarter of a percent of the
//! radius), where the piece is long.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;
mod common;

use boolean_common::{SCALES, mass};
use common::{arc, circle, line};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::boolean::{
    BooleanOp, BooleanOptions, Contact, ContactKind, Interference, VerifyOptions, boolean,
    boolean_with, contact, interference, verify,
};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::detect_profiles;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{Body, check};
use std::f64::consts::{FRAC_PI_2, PI};

/// The offsets of the vertex from the border, in tolerances: merged with
/// it, in the band where the three samples are on it, in the band where a
/// quarter point is clear of it, well clear, and (`None`) a quarter turn
/// away.
const OFFSETS: [Option<f64>; 5] = [Some(0.4), Some(1.4), Some(3.0), Some(8.0), None];

/// The tolerance at scale `k`: a millionth of the unit, or the coarse one
/// (0.01 at a radius of 4, as the Jig's millimetres).
fn tolerance(k: f64, coarse: bool) -> Tolerance {
    Tolerance::new(if coarse { 1e-2 * k } else { 1e-6 * k }, 1e-9).unwrap()
}

/// The booleans as the interference query runs them: parts that touch are
/// allowed.
fn unrefused() -> BooleanOptions {
    BooleanOptions {
        refuse_zero_thickness: false,
        ..BooleanOptions::default()
    }
}

/// What a boolean of the pair gave, held to what every boolean keeps:
/// `Ok(volume)` (0 for an empty result) or why not.
fn volume_of(a: &Body, b: &Body, op: BooleanOp, tol: &Tolerance) -> Result<f64, String> {
    match boolean_with(a, b, op, tol, &unrefused()) {
        Ok(result) => {
            let defects = check(&result.body, tol);
            if !defects.is_empty() {
                return Err(format!("invalid: {defects:?}"));
            }
            match verify(a, b, op, &result.body, tol, &VerifyOptions::default()) {
                Ok(found) if found.violations.is_empty() => {}
                Ok(found) => {
                    let all: Vec<String> =
                        found.violations.iter().map(ToString::to_string).collect();
                    return Err(format!("breaks the boolean's check: {}", all.join("; ")));
                }
                Err(e) => return Err(format!("the boolean's check cannot run: {e}")),
            }
            Ok(mass(&result.body).volume)
        }
        Err(KernelError::EmptyResult { .. }) => Ok(0.0),
        Err(e) => Err(format!("refused: {e}")),
    }
}

/// The three booleans of a pair against their volumes: the union, the
/// first less the second, and what they share (`common`, 0 when they only
/// touch or are apart).
fn booleans(
    a: &Body,
    b: &Body,
    common: f64,
    tol: &Tolerance,
    faults: &mut Vec<String>,
    what: &str,
) {
    let (ma, mb) = (mass(a), mass(b));
    // A volume is known to the tolerance over the boundary.
    let slack = tol.linear * (ma.area + mb.area) + 1e-9 * (ma.volume + mb.volume);
    for (op, want) in [
        (Union, ma.volume + mb.volume - common),
        (Subtract, ma.volume - common),
        (Intersect, common),
    ] {
        match volume_of(a, b, op, tol) {
            Ok(found) if (found - want).abs() <= slack => {}
            Ok(found) => faults.push(format!("{what}: {op:?} has volume {found}, not {want}")),
            Err(why) => faults.push(format!("{what}: {op:?} {why}")),
        }
    }
}

// ---------------------------------------------------------------------
// A plate and a cylinder at its edge.

/// The radius of the cylinder, times the scale.
const RADIUS: f64 = 4.0;

/// From which side the cylinder's footprint is tangent to the plate's
/// front edge.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Side {
    /// On the plate: a boss.
    Inside,
    /// Beside the plate.
    Outside,
}

/// One place of the grid.
#[derive(Clone, Copy, Debug)]
struct Place {
    k: f64,
    coarse: bool,
    /// Both bodies turned by this about z.
    turn: f64,
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

    /// The plate `[-10, 10] × [0, 10] × [-5, 0]`, times the scale.
    fn plate(&self) -> Body {
        let k = self.k;
        let frame = Frame::new(
            self.turned(Vec3::new(-10.0 * k, 0.0, -5.0 * k)),
            Vec3::Z,
            self.turned(Vec3::X),
        )
        .unwrap();
        primitives::block(&frame, 20.0 * k, 10.0 * k, 5.0 * k).unwrap()
    }

    /// The cylinder of radius 4 and height 6, times the scale. The
    /// vertex of its rim is on its own x axis.
    fn cylinder(&self) -> Body {
        let (k, tol) = (self.k, self.tol().linear);
        let r = RADIUS * k;
        let away = match self.offset {
            Some(e) => (1.0 - e * tol / r).pacos(),
            None => FRAC_PI_2,
        };
        // The place of the rim nearest the edge is towards -y from inside
        // and towards +y from outside.
        let angle = match self.side {
            Side::Inside => -FRAC_PI_2 + away,
            Side::Outside => FRAC_PI_2 - away,
        };
        let frame = Frame::new(
            self.turned(Vec3::new(0.0, self.centre(), self.lift * tol)),
            Vec3::Z,
            self.turned(Vec3::new(angle.pcos(), angle.psin(), 0.0)),
        )
        .unwrap();
        primitives::cylinder(&frame, r, 6.0 * k).unwrap()
    }

    /// The area the footprint and the plate's top share: of the disc of
    /// radius `r` about `(0, c)`, the part with `y ≥ 0`.
    fn shared(&self) -> f64 {
        let (r, c) = (RADIUS * self.k, self.centre());
        if c >= r {
            PI * r * r
        } else if c <= -r {
            0.0
        } else {
            r * r * (PI - (c / r).pacos()) + c * (r * r - c * c).sqrt()
        }
    }

    fn want(&self) -> Want {
        let area = self.shared();
        let tangent_outside = self.side == Side::Outside && self.shift == 0.0;
        if self.lift > 2.0 {
            Want::Clear
        } else if self.lift == 0.0 {
            if area > 0.0 {
                Want::Face(area)
            } else if tangent_outside {
                Want::Point
            } else {
                Want::Clear
            }
        } else if area > 0.0 {
            Want::Overlap(area * -self.lift * self.tol().linear)
        } else if tangent_outside {
            // The wall lies against the plate's front face along a line.
            Want::Line
        } else {
            Want::Clear
        }
    }
}

/// What a place of the grid is, by its closed form.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Want {
    Clear,
    /// They touch in one point.
    Point,
    /// They touch along a line.
    Line,
    /// They share this area.
    Face(f64),
    /// They share this volume.
    Overlap(f64),
}

/// Holds `contact` and `interference` of a place to its closed form, and
/// the three booleans to their volumes.
fn held(place: &Place, faults: &mut Vec<String>) {
    let (plate, cylinder, tol) = (place.plate(), place.cylinder(), place.tol());
    let want = place.want();
    let what = format!(
        "scale {} {} turn {} {:?} shift {} lift {} offset {:?} (want {want:?})",
        place.k,
        if place.coarse { "coarse" } else { "fine" },
        place.turn,
        place.side,
        place.shift,
        place.lift,
        place.offset
    );
    let r = RADIUS * place.k;
    // An area is known to the tolerance along the footprint's outline.
    let slack = |area: f64| tol.linear * 2.0 * PI * r + 1e-7 * area;
    let simple = match want {
        Want::Clear => Contact::Clear,
        Want::Point | Want::Line | Want::Face(_) => Contact::Touch,
        Want::Overlap(_) => Contact::Overlap,
    };
    match contact(&plate, &cylinder, &tol) {
        Ok(found) if found == simple => {}
        Ok(found) => faults.push(format!("{what}: contact says {found:?}")),
        Err(e) => faults.push(format!("{what}: contact refused: {e}")),
    }
    match (interference(&plate, &cylinder, &tol), want) {
        (Ok(Interference::Clear), Want::Clear) => {}
        // A contact in a point, or along a line: no area. (With the
        // vertex within the tolerance of the edge the point is read as a
        // short stretch of the edge: a touch all the same.)
        (Ok(Interference::Touch { kind, area, .. }), Want::Point | Want::Line)
            if kind != ContactKind::Face && area == 0.0 => {}
        (Ok(Interference::Touch { kind, area, .. }), Want::Face(shared))
            if kind == ContactKind::Face && (area - shared).abs() <= slack(shared) => {}
        (Ok(Interference::Overlap(common)), Want::Overlap(volume))
            if (mass(&common.body).volume - volume).abs()
                <= slack(place.shared()) * -place.lift * tol.linear + 1e-9 * volume => {}
        (Ok(found), _) => {
            let found = match found {
                Interference::Overlap(common) => {
                    format!("an overlap of {}", mass(&common.body).volume)
                }
                other => format!("{other:?}"),
            };
            faults.push(format!("{what}: interference says {found}"));
        }
        (Err(e), _) => faults.push(format!("{what}: interference refused: {e}")),
    }
    let common = match want {
        Want::Overlap(volume) => volume,
        _ => 0.0,
    };
    booleans(&plate, &cylinder, common, &tol, faults, &what);
}

/// The grid at one scale and tolerance.
fn grid(k: f64, coarse: bool) {
    let mut faults = Vec::new();
    let mut places = 0;
    for turn in [0.0, 0.5] {
        for side in [Side::Inside, Side::Outside] {
            for shift in [0.0, 40.0, -40.0] {
                for lift in [0.0, 5.0, -50.0] {
                    for offset in OFFSETS {
                        let place = Place {
                            k,
                            coarse,
                            turn,
                            side,
                            shift,
                            lift,
                            offset,
                        };
                        held(&place, &mut faults);
                        places += 1;
                    }
                }
            }
        }
    }
    assert!(
        faults.is_empty(),
        "{} faults at {places} places:\n  {}",
        faults.len(),
        faults.join("\n  ")
    );
}

#[test]
fn a_cylinder_at_a_plates_edge_at_a_thousandth() {
    grid(SCALES[0], false);
    grid(SCALES[0], true);
}

#[test]
fn a_cylinder_at_a_plates_edge_at_one() {
    grid(SCALES[1], false);
    grid(SCALES[1], true);
}

#[test]
fn a_cylinder_at_a_plates_edge_at_a_million() {
    grid(SCALES[2], false);
    grid(SCALES[2], true);
}

// ---------------------------------------------------------------------
// A keyed shaft and a hub.

/// The seat's radius and half the key's width (the radius of the
/// keyway's ends), times the scale.
const SEAT: f64 = 12.5;
const HALF_KEY: f64 = 4.0;

/// The shaft: a cylinder of radius 12.5 from `z = -4` to `56` with a
/// keyway 8 wide and 4 deep cut from the side of +x, its ends rounded
/// about `z = 1` and `z = 44`. With an `offset` the end at `z = 1` is cut
/// as two arcs that meet that many tolerances off the keyway's side.
fn shaft(k: f64, tol: &Tolerance, offset: Option<f64>) -> Body {
    let (seat, w) = (SEAT * k, HALF_KEY * k);
    let (z0, z1) = (1.0 * k, 44.0 * k);
    let blank = primitives::cylinder(
        &Frame::WORLD.at(Vec3::new(0.0, 0.0, -4.0 * k)).unwrap(),
        seat,
        60.0 * k,
    )
    .unwrap();
    let mut curves = vec![
        line(1, [w, z0], [w, z1]),
        arc(2, [0.0, z1], [w, z1], [-w, z1]),
        line(3, [-w, z1], [-w, z0]),
    ];
    match offset {
        Some(e) => {
            let away = (1.0 - e * tol.linear / w).pacos();
            let split = [-w * away.pcos(), z0 - w * away.psin()];
            curves.push(arc(4, [0.0, z0], [-w, z0], split));
            curves.push(arc(5, [0.0, z0], split, [w, z0]));
        }
        None => curves.push(arc(4, [0.0, z0], [-w, z0], [w, z0])),
    }
    let profiles = detect_profiles(&curves, tol).unwrap();
    // The sketch's x is the world's y and its y the world's z, on the
    // plane 4 below the top of the seat.
    let plane = Frame::new(Vec3::new(seat - 4.0 * k, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap();
    let cutter = extrude(&profiles, &plane, 0.0, 5.0 * k, tol).unwrap().body;
    boolean(&blank, &cutter, Subtract, tol).unwrap().body
}

/// The hub: a ring of outer radius 20 from `z = -3` to `48`, its bore on
/// the shaft's seat, with a keyway as wide as the shaft's.
fn hub(k: f64, tol: &Tolerance) -> Body {
    let (seat, w) = (SEAT * k, HALF_KEY * k);
    let x = (seat * seat - w * w).sqrt();
    let deep = seat + 2.5 * k;
    let curves = vec![
        circle(1, [0.0, 0.0], 20.0 * k),
        line(2, [x, -w], [deep, -w]),
        line(3, [deep, -w], [deep, w]),
        line(4, [deep, w], [x, w]),
        arc(5, [0.0, 0.0], [x, w], [x, -w]),
    ];
    let profiles = detect_profiles(&curves, tol).unwrap();
    let plane = Frame::WORLD.at(Vec3::new(0.0, 0.0, -3.0 * k)).unwrap();
    extrude(&profiles, &plane, 0.0, 51.0 * k, tol).unwrap().body
}

/// The hub on the shaft: the bore lies on the seat (a touch, the area of
/// the bore), whatever vertex the keyway's end has.
fn keyed(k: f64, coarse: bool) {
    let tol = tolerance(k, coarse);
    let hub = hub(k, &tol);
    assert_eq!(check(&hub, &tol), [], "the hub at {k}");
    // The bore: the seat's cylinder less the keyway's width, over the
    // hub's length.
    let (seat, w) = (SEAT * k, HALF_KEY * k);
    let bore = seat * (2.0 * PI - 2.0 * (w / seat).pasin()) * 51.0 * k;
    let mut faults = Vec::new();
    for offset in [None, Some(0.4), Some(1.4), Some(3.0), Some(8.0)] {
        let shaft = shaft(k, &tol, offset);
        assert_eq!(check(&shaft, &tol), [], "the shaft at {k}, {offset:?}");
        let what = format!(
            "scale {k} {} offset {offset:?}",
            if coarse { "coarse" } else { "fine" }
        );
        // Either way round: the hub's bore is then the first body's face
        // or the second's.
        for (a, b, way) in [
            (&hub, &shaft, "hub on shaft"),
            (&shaft, &hub, "shaft in hub"),
        ] {
            match contact(a, b, &tol) {
                Ok(Contact::Touch) => {}
                Ok(found) => faults.push(format!("{what}, {way}: contact says {found:?}")),
                Err(e) => faults.push(format!("{what}, {way}: contact refused: {e}")),
            }
            match interference(a, b, &tol) {
                Ok(Interference::Touch { kind, area, .. })
                    if kind == ContactKind::Face
                        && (area - bore).abs() <= tol.linear * 200.0 * k + 1e-6 * bore => {}
                Ok(Interference::Overlap(common)) => faults.push(format!(
                    "{what}, {way}: interference says an overlap of {}",
                    mass(&common.body).volume
                )),
                Ok(found) => faults.push(format!(
                    "{what}, {way}: interference says {found:?}, not a face of {bore}"
                )),
                Err(e) => faults.push(format!("{what}, {way}: interference refused: {e}")),
            }
            booleans(a, b, 0.0, &tol, &mut faults, &format!("{what}, {way}"));
        }
    }
    assert!(
        faults.is_empty(),
        "{} faults:\n  {}",
        faults.len(),
        faults.join("\n  ")
    );
}

#[test]
fn a_hub_on_a_keyed_shaft_at_the_jigs_tolerance() {
    for k in SCALES {
        keyed(k, true);
    }
}

#[test]
fn a_hub_on_a_keyed_shaft_at_a_millionth() {
    for k in SCALES {
        keyed(k, false);
    }
}
