//! The problem and its answer: pure data.

use craft3d_math::Vec3;

/// A 3×3 matrix stored as its three columns. As the axes of a frame or the
/// rotation of a pose, the columns are the x, y and z axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3 {
    pub cols: [Vec3; 3],
}

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3 {
        cols: [Vec3::X, Vec3::Y, Vec3::Z],
    };

    pub const fn from_cols(x: Vec3, y: Vec3, z: Vec3) -> Mat3 {
        Mat3 { cols: [x, y, z] }
    }

    /// The rotation by `angle` radians about the unit (or zero) `axis`,
    /// right-handed (Rodrigues). A zero axis or angle gives the identity.
    pub fn rotation(axis: Vec3, angle: f64) -> Mat3 {
        let Some(k) = axis.normalized() else {
            return Mat3::IDENTITY;
        };
        let (s, c) = angle.sin_cos();
        let col = |e: Vec3| e * c + k.cross(e) * s + k * (k.dot(e) * (1.0 - c));
        Mat3::from_cols(col(Vec3::X), col(Vec3::Y), col(Vec3::Z))
    }

    /// The rotation `exp([w]×)`: about `w` by `|w|` radians.
    pub fn exp(w: Vec3) -> Mat3 {
        Mat3::rotation(w, w.length())
    }

    pub fn x(&self) -> Vec3 {
        self.cols[0]
    }

    pub fn y(&self) -> Vec3 {
        self.cols[1]
    }

    pub fn z(&self) -> Vec3 {
        self.cols[2]
    }

    /// `M v`.
    pub fn apply(&self, v: Vec3) -> Vec3 {
        self.cols[0] * v.x + self.cols[1] * v.y + self.cols[2] * v.z
    }

    /// `Mᵀ v`.
    pub fn apply_transposed(&self, v: Vec3) -> Vec3 {
        Vec3::new(
            self.cols[0].dot(v),
            self.cols[1].dot(v),
            self.cols[2].dot(v),
        )
    }

    /// `M N`.
    pub fn mul(&self, n: &Mat3) -> Mat3 {
        Mat3::from_cols(
            self.apply(n.cols[0]),
            self.apply(n.cols[1]),
            self.apply(n.cols[2]),
        )
    }

    pub fn transpose(&self) -> Mat3 {
        let [a, b, c] = self.cols;
        Mat3::from_cols(
            Vec3::new(a.x, b.x, c.x),
            Vec3::new(a.y, b.y, c.y),
            Vec3::new(a.z, b.z, c.z),
        )
    }

    pub fn is_finite(&self) -> bool {
        self.cols.iter().all(|c| c.is_finite())
    }

    /// The nearest right-handed orthonormal matrix by Gram–Schmidt from z
    /// (z kept, x made square to it, y = z × x), or `None` when the columns
    /// are not finite, z is zero, x lies within `epsilon` (relative) of z's
    /// direction, or the given y points against z × x (a left-handed or
    /// degenerate set of axes).
    pub fn orthonormalized(&self, epsilon: f64) -> Option<Mat3> {
        if !self.is_finite() {
            return None;
        }
        let z = self.z().normalized()?;
        let x_raw = self.x();
        let x_len = x_raw.length();
        let x_perp = x_raw - z * x_raw.dot(z);
        if x_len <= 0.0 || x_perp.length() <= epsilon * x_len {
            return None;
        }
        let x = x_perp.normalized()?;
        let y = z.cross(x);
        let y_raw = self.y();
        if y_raw.dot(y) <= epsilon * y_raw.length() {
            return None;
        }
        Some(Mat3::from_cols(x, y, z))
    }
}

/// A frame on an instance, in the instance's own coordinates: an origin
/// and three right-handed orthonormal axes (the columns of `axes`). Its z
/// axis is the mate's axis or normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub origin: Vec3,
    pub axes: Mat3,
}

impl Frame {
    pub const IDENTITY: Frame = Frame {
        origin: Vec3::ZERO,
        axes: Mat3::IDENTITY,
    };

    pub const fn new(origin: Vec3, axes: Mat3) -> Frame {
        Frame { origin, axes }
    }

    /// A frame at `origin` with the world's axes.
    pub const fn at(origin: Vec3) -> Frame {
        Frame {
            origin,
            axes: Mat3::IDENTITY,
        }
    }

    /// A frame at `origin` whose z axis is `z` and whose x axis is `x_hint`
    /// made square to it; `None` when they are degenerate.
    pub fn from_z_x(origin: Vec3, z: Vec3, x_hint: Vec3) -> Option<Frame> {
        let y = z.cross(x_hint);
        let axes = Mat3::from_cols(x_hint, y, z).orthonormalized(1e-12)?;
        origin.is_finite().then_some(Frame { origin, axes })
    }
}

/// Where an instance is: world = `rotation` · local + `translation`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub rotation: Mat3,
    pub translation: Vec3,
}

impl Pose {
    pub const IDENTITY: Pose = Pose {
        rotation: Mat3::IDENTITY,
        translation: Vec3::ZERO,
    };

    pub const fn new(rotation: Mat3, translation: Vec3) -> Pose {
        Pose {
            rotation,
            translation,
        }
    }

    pub const fn at(translation: Vec3) -> Pose {
        Pose {
            rotation: Mat3::IDENTITY,
            translation,
        }
    }

    /// A local point in the world.
    pub fn point(&self, p: Vec3) -> Vec3 {
        self.rotation.apply(p) + self.translation
    }

    /// A local direction in the world.
    pub fn vector(&self, v: Vec3) -> Vec3 {
        self.rotation.apply(v)
    }

    /// A local frame in the world.
    pub fn frame(&self, f: &Frame) -> Frame {
        Frame {
            origin: self.point(f.origin),
            axes: self.rotation.mul(&f.axes),
        }
    }
}

/// The kinds of mate. Every mate says that the frame on `b` is the frame on
/// `a` moved by the mate's motion `M(q) = T(x, y, offset + d) · Rz(angle + θ)`
/// (in `a`'s frame), where the variables a kind has are free and the others
/// are zero; [`MateKind::Ball`] keeps only the origins together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MateKind {
    /// No variables: 6 degrees of freedom removed.
    Fixed,
    /// θ about z: 5 removed.
    Revolute,
    /// d along z: 5 removed.
    Slider,
    /// θ about z and d along z (in that order): 4 removed.
    Cylindrical,
    /// x and y in the plane, θ about its normal (in that order): 3 removed.
    Planar,
    /// The origins coincide, any rotation: 3 removed, no variables.
    Ball,
}

/// What one variable of a mate measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarKind {
    /// The turn θ about z, radians.
    Angle,
    /// The shift d along z, model units.
    Distance,
    /// The shift x in the plane, model units.
    PlaneX,
    /// The shift y in the plane, model units.
    PlaneY,
}

impl VarKind {
    pub fn is_angle(self) -> bool {
        self == VarKind::Angle
    }

    pub fn label(self) -> &'static str {
        match self {
            VarKind::Angle => "angle",
            VarKind::Distance => "distance",
            VarKind::PlaneX => "x",
            VarKind::PlaneY => "y",
        }
    }
}

impl MateKind {
    /// The mate's variables, in order.
    pub fn variables(self) -> &'static [VarKind] {
        match self {
            MateKind::Fixed | MateKind::Ball => &[],
            MateKind::Revolute => &[VarKind::Angle],
            MateKind::Slider => &[VarKind::Distance],
            MateKind::Cylindrical => &[VarKind::Angle, VarKind::Distance],
            MateKind::Planar => &[VarKind::PlaneX, VarKind::PlaneY, VarKind::Angle],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MateKind::Fixed => "fixed",
            MateKind::Revolute => "revolute",
            MateKind::Slider => "slider",
            MateKind::Cylindrical => "cylindrical",
            MateKind::Planar => "planar",
            MateKind::Ball => "ball",
        }
    }

    /// Whether the mate keeps the two frames' axes together (all but a
    /// ball).
    pub fn holds_rotation(self) -> bool {
        self != MateKind::Ball
    }
}

/// At most this many variables per mate ([`MateKind::Planar`]).
pub const MAX_MATE_VARIABLES: usize = 3;

/// One mate between two instances.
#[derive(Clone, Debug, PartialEq)]
pub struct MateSpec {
    /// For messages; may be empty (the mate is then called by its index).
    pub name: String,
    pub kind: MateKind,
    /// The instance and the frame on it, in its own coordinates.
    pub a: (usize, Frame),
    pub b: (usize, Frame),
    /// A fixed shift along `a`'s z, model units.
    pub offset: f64,
    /// A fixed turn about `a`'s z, radians.
    pub angle: f64,
    /// `b`'s frame turned half a turn about its x axis first (its z
    /// reversed): face to face instead of side by side.
    pub flip: bool,
    /// Per variable (in [`MateKind::variables`] order), the range it may
    /// take; `None` for no limit. Entries beyond the kind's variables must
    /// be `None`.
    pub limits: [Option<(f64, f64)>; MAX_MATE_VARIABLES],
    /// Per variable, the value it is driven to; `None` for a variable the
    /// solver finds. Entries beyond the kind's variables must be `None`.
    pub driven: [Option<f64>; MAX_MATE_VARIABLES],
}

impl MateSpec {
    /// A mate with no offset, angle, flip, limits or drivers.
    pub fn new(kind: MateKind, a: (usize, Frame), b: (usize, Frame)) -> MateSpec {
        MateSpec {
            name: String::new(),
            kind,
            a,
            b,
            offset: 0.0,
            angle: 0.0,
            flip: false,
            limits: [None; MAX_MATE_VARIABLES],
            driven: [None; MAX_MATE_VARIABLES],
        }
    }

    pub fn named(mut self, name: &str) -> MateSpec {
        self.name = name.to_string();
        self
    }

    /// Drives the mate's first variable.
    pub fn driven(mut self, value: f64) -> MateSpec {
        self.driven[0] = Some(value);
        self
    }
}

/// How a coupling ties mate variables together. Angles are radians,
/// distances model units, and each variable is measured from the mate's
/// zero (its two frames aligned up to `offset` and `angle`).
#[derive(Clone, Debug, PartialEq)]
pub enum CouplingKind {
    /// `θa = −value · θb`: two meshing gears, `value` the ratio of `b`'s
    /// pitch radius to `a`'s (negative for an internal gear).
    Gear,
    /// `da = value · θb`: a rack (`a`, a slider or a cylindrical mate's
    /// distance) on a pinion (`b`, an angle) of pitch radius `value`.
    Rack,
    /// `da = value · θb / 2π`: a screw of lead `value` per turn; `a` and
    /// `b` may be the same cylindrical mate.
    Screw,
    /// `Σ cᵢ qᵢ = value` over the terms `(mate, variable index, cᵢ)`, `qᵢ`
    /// in radians or model units: any linear tie, such as a planetary
    /// stage (sun and carrier against ground, a planet against the
    /// carrier). `mates` is not read.
    Linear(Vec<(usize, usize, f64)>),
}

/// A coupling between mate variables.
#[derive(Clone, Debug, PartialEq)]
pub struct CouplingSpec {
    pub kind: CouplingKind,
    /// The mates coupled: `(a, b)` as [`CouplingKind`] names them.
    pub mates: (usize, usize),
    pub value: f64,
}

/// An assembly to solve.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    /// The number of instances, numbered from 0.
    pub instances: usize,
    /// Optional names of the instances, for messages (index order; may be
    /// shorter than `instances`).
    pub names: Vec<String>,
    /// Instances that stay where `start` has them.
    pub grounded: Vec<usize>,
    pub mates: Vec<MateSpec>,
    pub couplings: Vec<CouplingSpec>,
    /// One pose per instance: where the solve starts.
    pub start: Vec<Pose>,
    /// Model units: residuals within this count as satisfied.
    pub tolerance: f64,
    /// Model units: the assembly's size (the length a turn of one radian
    /// is compared with). Variables and residuals are measured in it.
    pub scale: f64,
}

impl Problem {
    /// An empty problem over `instances` instances at the identity pose.
    pub fn new(instances: usize, tolerance: f64, scale: f64) -> Problem {
        Problem {
            instances,
            names: Vec::new(),
            grounded: Vec::new(),
            mates: Vec::new(),
            couplings: Vec::new(),
            start: vec![Pose::IDENTITY; instances],
            tolerance,
            scale,
        }
    }

    /// The name of an instance for messages.
    pub fn instance_name(&self, i: usize) -> String {
        match self.names.get(i) {
            Some(n) if !n.is_empty() => n.clone(),
            _ => format!("instance {i}"),
        }
    }

    /// The name of a mate for messages.
    pub fn mate_name(&self, m: usize) -> String {
        match self.mates.get(m) {
            Some(spec) if !spec.name.is_empty() => spec.name.clone(),
            Some(spec) => format!("{} mate {m}", spec.kind.label()),
            None => format!("mate {m}"),
        }
    }

    /// The driven variables in order (mate order, then variable order):
    /// `(mate, variable index)`. [`crate::Solver::advance`] takes new
    /// values in this order.
    pub fn driven_slots(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (m, spec) in self.mates.iter().enumerate() {
            for (v, d) in spec.driven.iter().enumerate() {
                if d.is_some() {
                    out.push((m, v));
                }
            }
        }
        out
    }
}

/// One rigid motion of an instance that the mates leave free, per unit of
/// the free direction: its points move by `linear + angular × (p − point)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    /// A shift along a unit direction (world).
    Slide { direction: Vec3 },
    /// A turn about a unit axis (world) through a point, with `pitch` model
    /// units of shift along the axis per radian (0 for a pure turn).
    Turn { axis: Vec3, point: Vec3, pitch: f64 },
}

/// How one instance moves along one free direction.
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceMotion {
    pub instance: usize,
    pub motion: Motion,
    /// The same motion as a twist of the instance's reference point (its
    /// pose's translation): `(velocity, angular velocity)`, world, per unit
    /// of the free direction (model units, radians).
    pub twist: (Vec3, Vec3),
}

/// One direction of motion the mates leave free (a basis vector of the
/// Jacobian's null space).
#[derive(Clone, Debug, PartialEq)]
pub struct FreeMotion {
    /// The instances it moves, in index order.
    pub instances: Vec<InstanceMotion>,
    /// The mate variables it changes: `(mate, variable index, rate)`.
    pub variables: Vec<(usize, usize, f64)>,
}

/// Mates (and couplings, drivers) whose equations repeat what earlier ones
/// already say, and hold anyway. The last is the one that added nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct Redundancy {
    pub groups: Vec<Group>,
}

/// One source of equations, for diagnosis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Mate(usize),
    Coupling(usize),
    /// The driver of a mate's variable.
    Driver {
        mate: usize,
        variable: usize,
    },
    /// A limit of a mate's variable, held at its bound.
    Limit {
        mate: usize,
        variable: usize,
    },
}

/// A solved assembly.
#[derive(Clone, Debug, PartialEq)]
pub struct Solution {
    /// One pose per instance.
    pub poses: Vec<Pose>,
    /// Per mate, its variables' values (in [`MateKind::variables`] order;
    /// radians and model units), driven ones included. Angles are not
    /// wrapped: they follow the motion continuously.
    pub values: Vec<Vec<f64>>,
    /// The degrees of freedom left: unknowns minus the Jacobian's rank.
    pub dof: usize,
    /// The Jacobian's rank at the solution.
    pub rank: usize,
    /// The number of scalar equations.
    pub equations: usize,
    /// A basis of the free motions (`dof` of them).
    pub free: Vec<FreeMotion>,
    /// Equations that repeat earlier ones and hold anyway.
    pub redundant: Vec<Redundancy>,
    /// Limits the solve holds at their bound: `(mate, variable index)`.
    pub at_limit: Vec<(usize, usize)>,
    /// The driven values solved for, in [`Problem::driven_slots`] order.
    pub drives: Vec<f64>,
    /// No instance is grounded: the whole assembly may move as one.
    pub floating: bool,
    /// The largest residual: model units for positions, and model units at
    /// `scale` for angles.
    pub residual: f64,
    /// Newton iterations spent (all passes).
    pub iterations: usize,
}

impl Solution {
    /// A driven or free variable's value.
    pub fn value(&self, mate: usize, variable: usize) -> Option<f64> {
        self.values.get(mate)?.get(variable).copied()
    }
}

/// How far a blamed group is from holding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gap {
    /// The distance by which positions miss, model units.
    pub length: f64,
    /// The angle by which axes miss, radians.
    pub angle: f64,
}

/// Why there is no solution.
#[derive(Clone, Debug, PartialEq)]
pub enum Diagnosis {
    /// The problem itself is malformed (a NaN frame, a zero axis, an
    /// index out of range, no instances).
    Invalid { reason: String },
    /// The groups' equations depend on each other and contradict: the
    /// last of `groups` was added after the others already decided what it
    /// asks for.
    OverConstrained { groups: Vec<Group>, gap: Gap },
    /// A loop that cannot close at any configuration the motion reaches:
    /// the solve came to rest where the loop is stretched or folded (its
    /// Jacobian lost rank there), `loop_mates` are the mates of the loop,
    /// `blamed` is the last group of the contradiction in problem order,
    /// and the loop misses by `gap` where it comes closest.
    Infeasible {
        blamed: Group,
        loop_mates: Vec<usize>,
        gap: Gap,
    },
    /// A driven value outside its variable's limits.
    OutOfLimits {
        mate: usize,
        variable: usize,
        value: f64,
        limits: (f64, f64),
    },
    /// The iteration stopped at its cap still moving.
    DidNotConverge { iterations: usize, residual: f64 },
    /// Along a sequence, the solution kept jumping further than the motion
    /// allows however the step was cut: a branch point near `drives`.
    BranchLost { drives: Vec<f64> },
    /// The problem needs more arithmetic than the solver allows.
    TooLarge { reason: String },
    /// Along a sequence, the drive cannot go past a dead point (a limit
    /// position of the mechanism) at `drives`; `cause` is why the next
    /// value has no solution.
    DeadPoint {
        drives: Vec<f64>,
        /// The mates whose equations come to depend on the driver's there
        /// (the loop at its limit position); empty when the last value
        /// reached is not close enough to tell.
        loop_mates: Vec<usize>,
        cause: Box<Diagnosis>,
    },
}

/// A dead point met along a sequence: the rank dropped.
#[derive(Clone, Debug, PartialEq)]
pub struct DeadPoint {
    /// The frame it was met at.
    pub frame: usize,
    /// The driven values there.
    pub drives: Vec<f64>,
    /// How many more degrees of freedom the solution has there than at the
    /// sequence's start.
    pub extra_dof: usize,
    /// The mates whose equations depend on a driver's there (the loop).
    pub loop_mates: Vec<usize>,
    /// The motions free there.
    pub free: Vec<FreeMotion>,
}

/// The answer over a sequence of driven values.
#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    /// One solution per frame solved, in order.
    pub frames: Vec<Solution>,
    pub dead_points: Vec<DeadPoint>,
    /// The first frame that could not be solved, and why; the frames before
    /// it are in `frames`.
    pub stopped: Option<(usize, Diagnosis)>,
}
