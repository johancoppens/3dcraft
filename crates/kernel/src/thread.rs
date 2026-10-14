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
//! The ISO 261 coarse pitches are [`coarse_pitch`]'s table.

use crate::KernelError;
use crate::helix::{Helix, RunOut, sweep_helix};
use crate::profile::{Curve2, SketchCurve, detect_profiles};
use crate::sweep::Swept;
use craft3d_math::{Frame, Tolerance, Vec3};
use std::f64::consts::FRAC_PI_2;

/// ISO 261 coarse pitches by nominal diameter, both in millimetres.
const COARSE: [(f64, f64); 26] = [
    (1.0, 0.25),
    (1.2, 0.25),
    (1.4, 0.3),
    (1.6, 0.35),
    (1.8, 0.35),
    (2.0, 0.4),
    (2.5, 0.45),
    (3.0, 0.5),
    (3.5, 0.6),
    (4.0, 0.7),
    (5.0, 0.8),
    (6.0, 1.0),
    (8.0, 1.25),
    (10.0, 1.5),
    (12.0, 1.75),
    (14.0, 2.0),
    (16.0, 2.0),
    (20.0, 2.5),
    (24.0, 3.0),
    (30.0, 3.5),
    (36.0, 4.0),
    (42.0, 4.5),
    (48.0, 5.0),
    (56.0, 5.5),
    (64.0, 6.0),
    (68.0, 6.0),
];

/// The coarse pitch of ISO 261 for a nominal diameter in millimetres,
/// when the table has that size.
pub fn coarse_pitch(nominal: f64) -> Option<f64> {
    COARSE
        .iter()
        .find(|(d, _)| (d - nominal).abs() < 1e-9)
        .map(|(_, p)| *p)
}

/// The nominal diameter in millimetres of a designation such as `M10`,
/// `m10` or `M2.5`; `None` for anything else.
pub fn nominal_diameter(size: &str) -> Option<f64> {
    let digits = size.trim().strip_prefix(['M', 'm'])?;
    if digits.is_empty() || digits.len() > 8 || !digits.chars().all(|c| c.is_ascii_digit() || c == '.')
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
}

/// How far round the run-out goes.
pub const RUN_OUT: f64 = FRAC_PI_2;

/// The groove of `thread` at `place`: the solid to subtract from the
/// shank, and the helix it follows.
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
        thread.minor() / 2.0,
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
    let profiles = detect_profiles(&thread.groove(), tolerance)?;
    let [profile] = profiles.as_slice() else {
        return Err(KernelError::InvalidProfile {
            reason: "the groove of a thread is one closed profile",
        });
    };
    let run_out = RunOut {
        angle: RUN_OUT,
        // The root comes out to an eighth of H beyond the major diameter.
        rise: thread.depth() + thread.triangle() / 8.0,
    };
    let swept = sweep_helix(profile, &plane, &helix, Some(run_out), tolerance)?;
    Ok((swept, helix))
}
