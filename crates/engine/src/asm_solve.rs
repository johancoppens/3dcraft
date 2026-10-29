//! STAND-IN for the assembly solver of track B (plan/assemblies.md ›
//! *Contract between the tracks*): the same shape, `solve(&Problem) ->
//! Result<Solution, Diagnosis>`, so the engine is built and tested against
//! it until the real solver lands and replaces this module.
//!
//! What it does: forward kinematics over a spanning tree of the mates,
//! from the grounded instances. A mate whose two instances are both placed
//! already (a closed loop, or a second mate between the same two) is
//! checked: satisfied within the tolerance it is redundant and fine,
//! otherwise the loop is reported as one this stand-in cannot close.
//! Free motions take their driven value, or 0. Instances no mate reaches
//! stay at their start pose and keep their six degrees of freedom.
//! Couplings and limits are not applied (reported as unsupported when
//! present).
#![allow(dead_code)] // The fields the real solver reads.

use craft3d_math::{Transform, Vec3};

/// A frame in an instance's own coordinates: an origin and three axes
/// (the columns of `axes` are x, y and z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Frame {
    pub origin: Vec3,
    pub x: Vec3,
    pub y: Vec3,
    pub z: Vec3,
}

impl Frame {
    pub(crate) fn to_transform(self) -> Transform {
        let frame = craft3d_math::Frame::new(self.origin, self.z, self.x);
        frame.map_or(Transform::translation(self.origin), |f| {
            Transform::from_frame(&f)
        })
    }

    /// The frame moved by `t`.
    pub(crate) fn moved(self, t: &Transform) -> Frame {
        Frame {
            origin: t.point(self.origin),
            x: t.vector(self.x),
            y: t.vector(self.y),
            z: t.vector(self.z),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MateKind {
    Fixed,
    Revolute,
    Slider,
    Cylindrical,
    Planar,
    Ball,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CouplingKind {
    Gear,
    Rack,
    Screw,
}

/// One mate between two instances (`usize::MAX` is the world), each with
/// its frame in that instance's coordinates.
#[derive(Clone, Debug)]
pub(crate) struct MateSpec {
    pub kind: MateKind,
    pub a: (usize, Frame),
    pub b: (usize, Frame),
    /// Along `a`'s z, in model units.
    pub offset: f64,
    /// About `a`'s z, in radians.
    pub angle: f64,
    pub limits: Option<(f64, f64)>,
    /// The value of the main free motion, when it is driven (radians or
    /// model units).
    pub driven: Option<f64>,
}

#[derive(Clone, Debug)]
pub(crate) struct CouplingSpec {
    pub kind: CouplingKind,
    pub mates: (usize, usize),
    pub value: f64,
}

/// A rigid pose: from the instance's coordinates to the assembly's.
pub(crate) type Pose = Transform;

/// The world, as an instance index in a [`MateSpec`].
pub(crate) const WORLD: usize = usize::MAX;

#[derive(Clone, Debug)]
pub(crate) struct Problem {
    pub instances: usize,
    pub grounded: Vec<usize>,
    pub mates: Vec<MateSpec>,
    pub couplings: Vec<CouplingSpec>,
    pub start: Vec<Pose>,
    pub tolerance: f64,
    pub scale: f64,
}

#[derive(Clone, Debug)]
pub(crate) struct Solution {
    pub poses: Vec<Pose>,
    /// Per mate, the value of its main free motion (none for a mate
    /// without one).
    pub free: Vec<Option<f64>>,
    /// Degrees of freedom left, per instance.
    pub dof: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Diagnosis {
    /// Mates contradict each other: the mates, by index, and the gap.
    Infeasible { mates: Vec<usize>, gap: f64 },
    /// What the solver cannot do (yet).
    Unsupported(String),
    /// The problem is malformed (an index out of range, a value not
    /// finite).
    Invalid(String),
}

/// The relative motion a mate allows, at its value: in `a`'s frame.
fn joint(spec: &MateSpec) -> Option<Transform> {
    let value = spec.driven.unwrap_or(0.0);
    let along = |d: f64| Transform::translation(Vec3::new(0.0, 0.0, d));
    let about = |a: f64| Transform::rotation(Vec3::Z, a);
    Some(match spec.kind {
        MateKind::Fixed | MateKind::Ball => about(spec.angle)?.then(&along(spec.offset)),
        MateKind::Revolute | MateKind::Cylindrical | MateKind::Planar => {
            about(spec.angle + value)?.then(&along(spec.offset))
        }
        MateKind::Slider => about(spec.angle)?.then(&along(spec.offset + value)),
    })
}

/// `b`'s pose from `a`'s: `b`'s frame turned over onto `a`'s frame moved
/// by the joint.
fn pose_of_b(pose_a: &Transform, spec: &MateSpec) -> Option<Transform> {
    let fa = spec.a.1.to_transform();
    let fb = spec.b.1.to_transform();
    let over = Transform::rotation(Vec3::X, std::f64::consts::PI)?;
    // local b-frame coordinates -> b frame turned -> joint -> a frame -> world
    Some(
        fb.inverse()?
            .then(&over)
            .then(&joint(spec)?)
            .then(&fa)
            .then(pose_a),
    )
}

fn pose_of_a(pose_b: &Transform, spec: &MateSpec) -> Option<Transform> {
    // pose_b = X . pose_a with X = fb^-1 over J fa: pose_a = X^-1 . pose_b
    let x = pose_of_b(&Transform::IDENTITY, spec)?;
    Some(x.inverse()?.then(pose_b))
}

/// How far two poses are apart on the frames of `spec`'s `b`: the
/// distance of its origin and of its axis tips at `scale`.
fn gap(p: &Transform, q: &Transform, frame: &Frame, scale: f64) -> f64 {
    let points = [
        frame.origin,
        frame.origin + frame.x * scale,
        frame.origin + frame.z * scale,
    ];
    points
        .iter()
        .map(|&v| p.point(v).distance(q.point(v)))
        .fold(0.0, f64::max)
}

pub(crate) fn solve(p: &Problem) -> Result<Solution, Diagnosis> {
    if p.start.len() != p.instances {
        return Err(Diagnosis::Invalid(
            "a start pose is needed per instance".to_string(),
        ));
    }
    let ok = |i: usize| i == WORLD || i < p.instances;
    if let Some(bad) = p.mates.iter().position(|m| !ok(m.a.0) || !ok(m.b.0)) {
        return Err(Diagnosis::Invalid(format!("mate {bad} names no instance")));
    }
    if !p.couplings.is_empty() {
        return Err(Diagnosis::Unsupported(
            "couplings need the assembly solver, which has not landed yet".to_string(),
        ));
    }
    let mut placed: Vec<Option<Transform>> = vec![None; p.instances];
    for &g in &p.grounded {
        if let Some(slot) = placed.get_mut(g) {
            *slot = Some(p.start.get(g).copied().unwrap_or(Transform::IDENTITY));
        }
    }
    let pose = |placed: &[Option<Transform>], i: usize| -> Option<Transform> {
        if i == WORLD {
            Some(Transform::IDENTITY)
        } else {
            placed.get(i).copied().flatten()
        }
    };
    let mut used = vec![false; p.mates.len()];
    // Spread from what is placed, mate by mate in order, until nothing
    // more can be placed: at most one pass per instance.
    for _ in 0..=p.instances {
        let mut progressed = false;
        for (k, spec) in p.mates.iter().enumerate() {
            if used.get(k).copied().unwrap_or(true) {
                continue;
            }
            let (pa, pb) = (pose(&placed, spec.a.0), pose(&placed, spec.b.0));
            let new = match (pa, pb) {
                (Some(pa), None) => pose_of_b(&pa, spec).map(|t| (spec.b.0, t)),
                (None, Some(pb)) => pose_of_a(&pb, spec).map(|t| (spec.a.0, t)),
                _ => continue,
            };
            let Some((at, t)) = new else {
                return Err(Diagnosis::Invalid(format!(
                    "mate {k} has a frame or a value that is not finite"
                )));
            };
            if let Some(slot) = placed.get_mut(at) {
                *slot = Some(t);
            }
            if let Some(u) = used.get_mut(k) {
                *u = true;
            }
            progressed = true;
        }
        if !progressed {
            break;
        }
    }
    // What is left joins two placed instances (or two unplaced ones):
    // a loop, checked.
    let mut worst: Option<(usize, f64)> = None;
    for (k, spec) in p.mates.iter().enumerate() {
        if used.get(k).copied().unwrap_or(true) {
            continue;
        }
        let (Some(pa), Some(pb)) = (pose(&placed, spec.a.0), pose(&placed, spec.b.0)) else {
            continue;
        };
        let Some(want) = pose_of_b(&pa, spec) else {
            continue;
        };
        let g = gap(&want, &pb, &spec.b.1, p.scale.max(p.tolerance));
        if g > p.tolerance && worst.is_none_or(|(_, w)| g > w) {
            worst = Some((k, g));
        }
    }
    if let Some((k, g)) = worst {
        return Err(Diagnosis::Infeasible {
            mates: vec![k],
            gap: g,
        });
    }
    let poses: Vec<Transform> = placed
        .iter()
        .zip(&p.start)
        .map(|(placed, start)| placed.unwrap_or(*start))
        .collect();
    let free = p
        .mates
        .iter()
        .map(|m| match m.kind {
            MateKind::Fixed | MateKind::Ball => None,
            _ => Some(m.driven.unwrap_or(0.0)),
        })
        .collect();
    // An instance not reached keeps its six; a reached one has what its
    // undriven mates leave (a rough count: the stand-in does not rank).
    let mut dof: Vec<usize> = placed
        .iter()
        .map(|p| if p.is_some() { 0 } else { 6 })
        .collect();
    for spec in &p.mates {
        let left = match (spec.kind, spec.driven.is_some()) {
            (MateKind::Fixed, _) => 0,
            (MateKind::Ball, _) => 3,
            (MateKind::Revolute | MateKind::Slider, true) => 0,
            (MateKind::Revolute | MateKind::Slider, false) => 1,
            (MateKind::Cylindrical, true) => 1,
            (MateKind::Cylindrical, false) => 2,
            (MateKind::Planar, true) => 2,
            (MateKind::Planar, false) => 3,
        };
        if let Some(d) = dof.get_mut(spec.b.0) {
            *d = d.saturating_add(left).min(6);
        }
    }
    Ok(Solution { poses, free, dof })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn frame(origin: Vec3, z: Vec3) -> Frame {
        let f = craft3d_math::Frame::from_z(origin, z).unwrap();
        Frame {
            origin: f.origin(),
            x: f.x(),
            y: f.y(),
            z: f.z(),
        }
    }

    #[test]
    fn a_fixed_mate_puts_face_against_face() {
        // A block's top (z = 10) against another's bottom (z = 0).
        let spec = MateSpec {
            kind: MateKind::Fixed,
            a: (0, frame(Vec3::new(5.0, 5.0, 10.0), Vec3::Z)),
            b: (1, frame(Vec3::new(5.0, 5.0, 0.0), -Vec3::Z)),
            offset: 0.0,
            angle: 0.0,
            limits: None,
            driven: None,
        };
        let p = Problem {
            instances: 2,
            grounded: vec![0],
            mates: vec![spec],
            couplings: Vec::new(),
            start: vec![Transform::IDENTITY; 2],
            tolerance: 1e-6,
            scale: 10.0,
        };
        let s = solve(&p).unwrap();
        let moved = s.poses[1].point(Vec3::new(5.0, 5.0, 0.0));
        assert!(
            moved.distance(Vec3::new(5.0, 5.0, 10.0)) < 1e-9,
            "{moved:?}"
        );
        // Its top went up by 10 too, not flipped.
        let top = s.poses[1].point(Vec3::new(0.0, 0.0, 10.0));
        assert!((top.z - 20.0).abs() < 1e-9, "{top:?}");
    }
}
