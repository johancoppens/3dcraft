//! ISO metric screw threads: the basic profile of ISO 68-1 and the
//! groove that cuts it into a shank.
//!
//! **The basic profile** is built on the fundamental triangle of height
//! `H = √3/2 · P` (flanks at 30° to the radial direction, 60° between
//! them). Of an external thread the crest is a flat `P/8` wide at the
//! major diameter `D`, the root a flat `P/4` wide at the minor diameter
//! `D − 5H/4` (the basic minor diameter of the internal thread, `d1 = D −
//! 1.0825 P`): the tooth is `5H/8` deep. The groove is what lies
//! between two teeth: a trapezoid `7P/8` wide at the major diameter and
//! `P/4` at the root. It is carried on past the major diameter by `H/16`
//! so that it breaks through the shank's surface cleanly, which keeps it
//! narrower than the pitch (`15P/16`): one turn does not touch the next.
//!
//! **The groove** is swept along a helix at the minor radius by the
//! screw motion ([`crate::helix`]): it starts a pitch before the start
//! of the thread (outside the material, where the shank ends), runs the
//! thread's length at full depth, and lifts out of the material over a
//! quarter turn (the run-out). Subtracting it from the shank leaves the
//! thread.
//!
//! The ISO 261 coarse pitches are [`coarse_pitch`]'s table, the fine ones
//! [`fine_pitches`]'.

use crate::KernelError;
use crate::helix::{Helix, RunOut, sweep_helix};
use crate::profile::{Curve2, SketchCurve, detect_profiles};
use crate::sweep::Swept;
use craft3d_math::{Frame, Tolerance, Vec3};
use std::f64::consts::FRAC_PI_2;

/// ISO 261 coarse pitches by nominal diameter, both in millimetres: every
/// size from M1 to M68 that has one (first, second and third choice).
/// The sizes of ISO 261 that have fine pitches only (M5.5, M15, M17, …)
/// are in [`FINE`].
const COARSE: [(f64, f64); 40] = [
    (1.0, 0.25),
    (1.1, 0.25),
    (1.2, 0.25),
    (1.4, 0.3),
    (1.6, 0.35),
    (1.8, 0.35),
    (2.0, 0.4),
    (2.2, 0.45),
    (2.5, 0.45),
    (3.0, 0.5),
    (3.5, 0.6),
    (4.0, 0.7),
    (4.5, 0.75),
    (5.0, 0.8),
    (6.0, 1.0),
    (7.0, 1.0),
    (8.0, 1.25),
    (9.0, 1.25),
    (10.0, 1.5),
    (11.0, 1.5),
    (12.0, 1.75),
    (14.0, 2.0),
    (16.0, 2.0),
    (18.0, 2.5),
    (20.0, 2.5),
    (22.0, 2.5),
    (24.0, 3.0),
    (27.0, 3.0),
    (30.0, 3.5),
    (33.0, 3.5),
    (36.0, 4.0),
    (39.0, 4.0),
    (42.0, 4.5),
    (45.0, 4.5),
    (48.0, 5.0),
    (52.0, 5.0),
    (56.0, 5.5),
    (60.0, 5.5),
    (64.0, 6.0),
    (68.0, 6.0),
];

/// ISO 261 fine pitches by nominal diameter, both in millimetres, from
/// the coarsest: every size from M1 to M68, those without a coarse pitch
/// included. (ISO 262's selection of sizes and fine pitches is a subset.)
const FINE: [(f64, &[f64]); 55] = [
    (1.0, &[0.2]),
    (1.1, &[0.2]),
    (1.2, &[0.2]),
    (1.4, &[0.2]),
    (1.6, &[0.2]),
    (1.8, &[0.2]),
    (2.0, &[0.25]),
    (2.2, &[0.25]),
    (2.5, &[0.35]),
    (3.0, &[0.35]),
    (3.5, &[0.35]),
    (4.0, &[0.5]),
    (4.5, &[0.5]),
    (5.0, &[0.5]),
    (5.5, &[0.5]),
    (6.0, &[0.75]),
    (7.0, &[0.75]),
    (8.0, &[1.0, 0.75]),
    (9.0, &[1.0, 0.75]),
    (10.0, &[1.25, 1.0, 0.75]),
    (11.0, &[1.0, 0.75]),
    (12.0, &[1.5, 1.25, 1.0]),
    (14.0, &[1.5, 1.25, 1.0]),
    (15.0, &[1.5, 1.0]),
    (16.0, &[1.5, 1.0]),
    (17.0, &[1.5, 1.0]),
    (18.0, &[2.0, 1.5, 1.0]),
    (20.0, &[2.0, 1.5, 1.0]),
    (22.0, &[2.0, 1.5, 1.0]),
    (24.0, &[2.0, 1.5, 1.0]),
    (25.0, &[2.0, 1.5, 1.0]),
    (26.0, &[1.5]),
    (27.0, &[2.0, 1.5, 1.0]),
    (28.0, &[2.0, 1.5, 1.0]),
    (30.0, &[3.0, 2.0, 1.5, 1.0]),
    (32.0, &[2.0, 1.5]),
    (33.0, &[3.0, 2.0, 1.5]),
    (35.0, &[1.5]),
    (36.0, &[3.0, 2.0, 1.5]),
    (38.0, &[1.5]),
    (39.0, &[3.0, 2.0, 1.5]),
    (40.0, &[3.0, 2.0, 1.5]),
    (42.0, &[4.0, 3.0, 2.0, 1.5]),
    (45.0, &[4.0, 3.0, 2.0, 1.5]),
    (48.0, &[4.0, 3.0, 2.0, 1.5]),
    (50.0, &[3.0, 2.0, 1.5]),
    (52.0, &[4.0, 3.0, 2.0, 1.5]),
    (55.0, &[4.0, 3.0, 2.0, 1.5]),
    (56.0, &[4.0, 3.0, 2.0, 1.5]),
    (58.0, &[4.0, 3.0, 2.0, 1.5]),
    (60.0, &[4.0, 3.0, 2.0, 1.5]),
    (62.0, &[4.0, 3.0, 2.0, 1.5]),
    (64.0, &[4.0, 3.0, 2.0, 1.5]),
    (65.0, &[4.0, 3.0, 2.0, 1.5]),
    (68.0, &[4.0, 3.0, 2.0, 1.5]),
];

/// The coarse pitch of ISO 261 for a nominal diameter in millimetres,
/// when the table has that size.
pub fn coarse_pitch(nominal: f64) -> Option<f64> {
    COARSE
        .iter()
        .find(|(d, _)| (d - nominal).abs() < 1e-9)
        .map(|(_, p)| *p)
}

/// The fine pitches of ISO 261 for a nominal diameter in millimetres,
/// from the coarsest, when it is a size of ISO 261 (M1 to M68).
pub fn fine_pitches(nominal: f64) -> Option<&'static [f64]> {
    FINE.iter()
        .find(|(d, _)| (d - nominal).abs() < 1e-9)
        .map(|(_, p)| *p)
}

/// The sizes of ISO 261 that [`coarse_pitch`] knows, as nominal
/// diameter and coarse pitch in millimetres, from the smallest.
pub fn iso_sizes() -> &'static [(f64, f64)] {
    &COARSE
}

/// Every size of ISO 261 from M1 to M68, as nominal diameter and fine
/// pitches in millimetres, from the smallest: those of [`iso_sizes`] and
/// those that have fine pitches only.
pub fn iso_261_sizes() -> &'static [(f64, &'static [f64])] {
    &FINE
}

/// What to say of a size that has no coarse pitch: its fine pitches, or
/// that it is no size of ISO 261.
pub fn no_coarse_pitch(designation: &str, nominal: f64) -> String {
    match fine_pitches(nominal) {
        Some(fine) => format!(
            "{designation} has no coarse pitch in ISO 261 (its fine pitches: {} mm); give `pitch`",
            fine.iter()
                .map(f64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        None => format!(
            "{designation} has no coarse pitch in ISO 261 (it is no size of its table, M1 to M68); give `pitch`"
        ),
    }
}

/// The designation of a nominal diameter in millimetres: `M10`, `M2.5`,
/// the number rounded to a millionth of a millimetre; `None` for a
/// diameter that is not a finite positive number.
pub fn designation(nominal: f64) -> Option<String> {
    if !(nominal.is_finite() && nominal > 0.0 && nominal < 1e9) {
        return None;
    }
    let rounded = (nominal * 1e6).round() / 1e6;
    (rounded > 0.0).then(|| format!("M{rounded}"))
}

/// The nominal diameter in millimetres of a designation such as `M10`,
/// `m10` or `M2.5`; `None` for anything else.
pub fn nominal_diameter(size: &str) -> Option<f64> {
    let digits = size.trim().strip_prefix(['M', 'm'])?;
    if digits.is_empty()
        || digits.len() > 8
        || !digits.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        return None;
    }
    let d: f64 = digits.parse().ok()?;
    (d.is_finite() && d > 0.0).then_some(d)
}

/// The basic profile of an external ISO metric thread, in the document's
/// length unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IsoThread {
    /// The major (nominal) diameter.
    pub major: f64,
    pub pitch: f64,
}

impl IsoThread {
    pub fn new(major: f64, pitch: f64) -> Result<IsoThread, KernelError> {
        if !(major.is_finite() && major > 0.0) {
            return Err(KernelError::InvalidSize {
                name: "thread diameter",
                value: major,
            });
        }
        if !(pitch.is_finite() && pitch > 0.0 && pitch < major) {
            return Err(KernelError::InvalidSize {
                name: "thread pitch",
                value: pitch,
            });
        }
        Ok(IsoThread { major, pitch })
    }

    /// The height of the fundamental triangle, `√3/2 · P`.
    pub fn triangle(&self) -> f64 {
        3.0_f64.sqrt() / 2.0 * self.pitch
    }

    /// How deep the tooth is: `5H/8`.
    pub fn depth(&self) -> f64 {
        5.0 * self.triangle() / 8.0
    }

    /// The minor diameter of the basic profile: `D − 5H/4`.
    pub fn minor(&self) -> f64 {
        self.major - 2.0 * self.depth()
    }

    /// How far beyond the major diameter the groove reaches: `H/16`.
    pub fn overshoot(&self) -> f64 {
        self.triangle() / 16.0
    }

    /// The area of the groove inside the major diameter, in the plane
    /// through the axis: `(7P/8 + P/4)/2 · 5H/8`.
    pub fn groove_area(&self) -> f64 {
        (7.0 / 8.0 + 1.0 / 4.0) / 2.0 * self.pitch * self.depth()
    }

    /// The distance from the axis of the centre of that area.
    pub fn groove_centre(&self) -> f64 {
        // A trapezoid of widths a (at the root) and b (at the crest line)
        // over a depth h has its centre h (a + 2b) / (3 (a + b)) from the
        // side of width a.
        let (a, b, h) = (self.pitch / 4.0, 7.0 * self.pitch / 8.0, self.depth());
        self.minor() / 2.0 + h * (a + 2.0 * b) / (3.0 * (a + b))
    }

    /// The area of the groove of the internal thread (what a tap cuts
    /// into the wall of a hole of the minor diameter), in the plane
    /// through the axis: the external thread's tooth, `(P/8 + 3P/4)/2 ·
    /// 5H/8`.
    pub fn internal_groove_area(&self) -> f64 {
        (1.0 / 8.0 + 3.0 / 4.0) / 2.0 * self.pitch * self.depth()
    }

    /// The distance from the axis of the centre of that area.
    pub fn internal_groove_centre(&self) -> f64 {
        // Widths a at the minor diameter, b at the major, over the depth
        // h: the centre is h (a + 2b) / (3 (a + b)) out from the minor.
        let (a, b, h) = (3.0 * self.pitch / 4.0, self.pitch / 8.0, self.depth());
        self.minor() / 2.0 + h * (a + 2.0 * b) / (3.0 * (a + b))
    }

    /// The tap drill's diameter: `D − P`.
    pub fn tap_drill(&self) -> f64 {
        self.major - self.pitch
    }

    /// The groove of the internal thread as curves of a plane through the
    /// axis, as [`IsoThread::groove`]: the basic profile of ISO 68-1 for
    /// the nut, its root a flat `P/8` wide at the major diameter, its
    /// flanks reaching `3P/4` apart at the minor diameter, and carried on
    /// `H/16` inside it so that it breaks through the wall of the hole.
    /// Ids 1 to 4: the root, the flank facing −y, the line inside the
    /// minor diameter, the flank facing +y.
    pub fn internal_groove(&self) -> Vec<SketchCurve> {
        let p = self.pitch;
        let root = self.major / 2.0;
        let inner = self.minor() / 2.0 - self.overshoot();
        let half = 3.0 * p / 8.0 + self.overshoot() / 3.0_f64.sqrt();
        // The root first, as for the external groove.
        let corners = [
            [root, p / 16.0],
            [root, -p / 16.0],
            [inner, -half],
            [inner, half],
        ];
        corners
            .iter()
            .zip(corners.iter().cycle().skip(1))
            .zip(1..)
            .map(|((start, end), id)| SketchCurve {
                id,
                curve: Curve2::Line {
                    start: *start,
                    end: *end,
                },
            })
            .collect()
    }

    /// The groove as curves of a plane through the axis: x is the distance
    /// from the axis, y the way along it, centred on y = 0. Ids 1 to 4:
    /// the root, the flank facing −y, the line outside the major
    /// diameter, the flank facing +y.
    pub fn groove(&self) -> Vec<SketchCurve> {
        let p = self.pitch;
        let root = self.minor() / 2.0;
        let out = self.major / 2.0 + self.overshoot();
        // 7P/16 at the major diameter, wider by tan 30° per unit further
        // out.
        let half = 7.0 * p / 16.0 + self.overshoot() / 3.0_f64.sqrt();
        let corners = [[root, p / 8.0], [root, -p / 8.0], [out, -half], [out, half]];
        corners
            .iter()
            .zip(corners.iter().cycle().skip(1))
            .zip(1..)
            .map(|((start, end), id)| SketchCurve {
                id,
                curve: Curve2::Line {
                    start: *start,
                    end: *end,
                },
            })
            .collect()
    }
}

/// Where a thread is cut and how far it goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThreadPlace {
    /// The point of the axis where the thread starts (on the end face).
    pub start: Vec3,
    /// The unit direction along the axis into the material.
    pub along: Vec3,
    /// A unit direction square to the axis: where the helix starts.
    pub across: Vec3,
    /// How far the thread goes at full depth from `start`.
    pub length: f64,
    pub left_handed: bool,
    /// An internal thread (a tapped hole): the groove is cut outwards
    /// into the wall from a hole of the minor diameter.
    pub internal: bool,
}

/// How far round the run-out goes.
pub const RUN_OUT: f64 = FRAC_PI_2;

/// The groove of `thread` at `place`: the solid to subtract from the
/// shank (or from the wall round a hole, for an internal thread), and the
/// helix it follows. The groove of an internal thread runs out towards
/// the axis, into the hole.
pub fn groove(
    thread: &IsoThread,
    place: &ThreadPlace,
    tolerance: &Tolerance,
) -> Result<(Swept, Helix), KernelError> {
    let p = thread.pitch;
    if !(place.length.is_finite() && place.length > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "thread length",
            value: place.length,
        });
    }
    let along = place.along.normalized().ok_or(KernelError::InvalidSize {
        name: "thread axis",
        value: 0.0,
    })?;
    let lead = place.start - along * p;
    let helix = Helix::new(
        lead,
        along,
        place.across,
        if place.internal {
            thread.major / 2.0
        } else {
            thread.minor() / 2.0
        },
        p,
        (place.length + p) / p,
        place.left_handed,
    )?;
    // The plane through the axis and the start of the helix, its x away
    // from the axis and its y along it.
    let plane = Frame::new(lead, helix.start.cross(along), helix.start).ok_or(
        KernelError::InvalidSize {
            name: "thread axis",
            value: 0.0,
        },
    )?;
    let curves = if place.internal {
        thread.internal_groove()
    } else {
        thread.groove()
    };
    let profiles = detect_profiles(&curves, tolerance)?;
    let [profile] = profiles.as_slice() else {
        return Err(KernelError::InvalidProfile {
            reason: "the groove of a thread is one closed profile",
        });
    };
    // The root comes out to an eighth of H beyond the major diameter, or
    // for an internal thread in to an eighth of H inside the minor one.
    let rise = thread.depth() + thread.triangle() / 8.0;
    let run_out = RunOut {
        angle: RUN_OUT,
        rise: if place.internal { -rise } else { rise },
    };
    let swept = sweep_helix(profile, &plane, &helix, Some(run_out), tolerance)?;
    Ok((swept, helix))
}
