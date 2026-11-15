//! The searches of [`Surface::project`] that end short (they go on,
//! [`Surface::finish`]) against the closed forms of
//! [`super::project_sweep`], and those that do not end short against the
//! old path: these never go on ([`super::FINISHED`]), so their answer is
//! the one the descent always gave, bit for bit.

use super::project_sweep::*;
use crate::allowance;

fn finished() -> usize {
    super::FINISHED.with(std::cell::Cell::get)
}

/// On sheets drawn without weights, of every aspect from 1:1 to a
/// million to one, on knots even, stretched up to 1e9 either way or
/// crowded, of degrees one to five, and on the rational cylinders and
/// spheres (also on stretched knots, the cylinders' rows weighted from
/// 1e-3 to 1e3, which keeps the cylinder), at scales 1e-3, 1 and 1e6, in
/// the world, turned, and turned a thousand and a million sizes from the
/// origin: no search over the whole surface ends short, so every one takes
/// the old path, and every foot is the closed form's to what the
/// coordinates and the parameters resolve.
#[test]
fn searches_on_evenly_drawn_sheets_take_the_old_path() {
    let mut random = Random(0x0E1D);
    let mut whole = Tally::default();
    let before = finished();
    for (kind, how, _, sheet) in sheets("plane,cylinder,sphere", &[1e-3, 1.0, 1e6], &mut random) {
        if kind.starts_with("plane") && kind.contains("weighted") {
            continue;
        }
        let tally = feet(&sheet, 4, Search::Whole, &mut random);
        assert_eq!(tally.missed + tally.errors, 0, "{kind} {how}");
        whole.add(tally);
    }
    assert_eq!(
        finished(),
        before,
        "a search over the whole surface went on"
    );
    println!("{}", whole.row("project"));
}

/// On the sheets of the long block of `classify_far.rs` (bicubic, six by
/// six control points, weights from 1e-3 to 1e3), 30 by 900 and 900 by 20
/// in the world's frame, the feet of points along their four borders,
/// where the checker projects the points of the edges of a face: each is
/// the closed form's to what the coordinates and the parameters resolve.
/// Before the repair 56 of these 1024 feet on the sheets 30 by 900 were
/// off, by up to 14.6, and 125 of 1024 on those 900 by 20, by up to 8.5
/// (none on the squares); the checker read an edge of the block as 1.65
/// from its face.
#[test]
fn the_borders_of_the_long_blocks_sheets_are_found() {
    let mut state = 1_u64;
    for (length, width) in [(30.0, 900.0), (900.0, 20.0), (30.0, 30.0)] {
        let mut tally = Tally::default();
        for _ in 0..4 {
            let sheet = pin_sheet(PLACES[0], (length, width), &mut state);
            for k in 0..64 {
                let t = (k as f64 + 0.5) / 64.0;
                for (x, y) in [
                    (length * t, 0.0),
                    (length * t, width),
                    (0.0, width * t),
                    (length, width * t),
                ] {
                    let foot = sheet.frame.point(x, y, 0.0);
                    let lift = [0.0, 1e-7, 0.05][k % 3] * width.min(length);
                    let p = foot + sheet.frame.z() * lift;
                    let (found, units) = allowance::measured(|| sheet.surface.project(p));
                    let (u, v) = found.unwrap();
                    let off = sheet.surface.eval(u, v).distance(foot);
                    let allowed = allowed(&sheet, p) + parameter_rounding(&sheet.surface, (u, v));
                    tally.tried += 1;
                    tally.units += units;
                    tally.worst = tally.worst.max(off / sheet.size);
                    assert!(
                        off <= allowed,
                        "{length} x {width}: the foot of {p:?} is {off:e} off (allowed {allowed:e})"
                    );
                }
            }
        }
        println!("{}", tally.row(&format!("{length} x {width}")));
    }
}

/// Weighted plane sheets (weights from 1e-3 to 1e3 at every control point,
/// which race and shear the parametrisation a millionfold) of aspects 1:1
/// to a thousand to one, of degrees one to five and every
/// parametrisation, at three scales in the world's frame and a turned
/// one: where the search ends short it goes on, and every foot is within
/// the tolerance (a millionth of the sheet's smaller size) of the closed
/// form's. Before the repair 384 of these 1512 feet were beyond it, by up
/// to 1.8 % of the sheet's size.
#[test]
fn the_feet_of_weighted_sheets_are_found_within_the_tolerance() {
    let mut random = Random(0x3E16);
    let mut total = Tally::default();
    let before = finished();
    for (kind, how, place, sheet) in sheets("plane", &[1e-3, 1.0, 1e6], &mut random) {
        if !kind.contains("weighted") || kind.contains("1e6:1") || place.far > 0.0 {
            continue;
        }
        let tally = feet(&sheet, 4, Search::Whole, &mut random);
        assert_eq!(tally.beyond + tally.errors, 0, "{kind} {how}");
        total.add(tally);
    }
    println!("{}", total.row("weighted sheets"));
    println!("searches that went on: {}", finished() - before);
    assert!(finished() > before);
}
