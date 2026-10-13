//! The little geometry the case generator needs: vectors, rotations, the
//! five primitive solids and the domed block, and what can be said about
//! each without a kernel (where its faces, edges and corners are, a ball
//! inside it, a ball around it, whether a point is inside).
//!
//! `xtask` depends on no 3DCraft crate, on purpose: the corpus must not be
//! built with the code it is going to judge. The domed block's roof, a
//! bicubic Bézier patch, is evaluated here with the Bernstein polynomials
//! written out.

use std::f64::consts::{FRAC_1_SQRT_2, PI, TAU};
use std::ops::{Add, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct V(pub f64, pub f64, pub f64);

impl Add for V {
    type Output = V;
    fn add(self, o: V) -> V {
        V(self.0 + o.0, self.1 + o.1, self.2 + o.2)
    }
}

impl Sub for V {
    type Output = V;
    fn sub(self, o: V) -> V {
        V(self.0 - o.0, self.1 - o.1, self.2 - o.2)
    }
}

impl Mul<f64> for V {
    type Output = V;
    fn mul(self, k: f64) -> V {
        V(self.0 * k, self.1 * k, self.2 * k)
    }
}

impl Neg for V {
    type Output = V;
    fn neg(self) -> V {
        V(-self.0, -self.1, -self.2)
    }
}

impl V {
    pub const ZERO: V = V(0.0, 0.0, 0.0);
    pub const X: V = V(1.0, 0.0, 0.0);
    pub const Y: V = V(0.0, 1.0, 0.0);
    pub const Z: V = V(0.0, 0.0, 1.0);

    pub fn dot(self, o: V) -> f64 {
        self.0 * o.0 + self.1 * o.1 + self.2 * o.2
    }

    pub fn cross(self, o: V) -> V {
        V(
            self.1 * o.2 - self.2 * o.1,
            self.2 * o.0 - self.0 * o.2,
            self.0 * o.1 - self.1 * o.0,
        )
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    /// The direction of `self`; `self` unchanged when it has no length.
    pub fn unit(self) -> V {
        let n = self.length();
        if n > 0.0 { self * (1.0 / n) } else { self }
    }

    /// Component `i` modulo 3.
    pub fn component(self, i: usize) -> f64 {
        match i % 3 {
            0 => self.0,
            1 => self.1,
            _ => self.2,
        }
    }

    /// The unit vector along axis `i` modulo 3.
    pub fn axis(i: usize) -> V {
        match i % 3 {
            0 => V::X,
            1 => V::Y,
            _ => V::Z,
        }
    }

    /// Some unit vector perpendicular to `self`.
    pub fn perpendicular(self) -> V {
        let least = if self.0.abs() <= self.1.abs() && self.0.abs() <= self.2.abs() {
            V::X
        } else if self.1.abs() <= self.2.abs() {
            V::Y
        } else {
            V::Z
        };
        self.cross(least).unit()
    }
}

/// A rotation, as the images of the three axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rot {
    pub x: V,
    pub y: V,
    pub z: V,
}

impl Rot {
    pub const IDENTITY: Rot = Rot {
        x: V::X,
        y: V::Y,
        z: V::Z,
    };
    /// Half a turn about x: z and y point the other way.
    pub const FLIP: Rot = Rot {
        x: V::X,
        y: V(0.0, -1.0, 0.0),
        z: V(0.0, 0.0, -1.0),
    };
    /// A quarter turn about x: z goes to -y.
    pub const ROLL: Rot = Rot {
        x: V::X,
        y: V::Z,
        z: V(0.0, -1.0, 0.0),
    };

    pub fn apply(&self, v: V) -> V {
        self.x * v.0 + self.y * v.1 + self.z * v.2
    }

    /// `self` after `first`.
    pub fn after(&self, first: &Rot) -> Rot {
        Rot {
            x: self.apply(first.x),
            y: self.apply(first.y),
            z: self.apply(first.z),
        }
    }

    pub fn about_z(angle: f64) -> Rot {
        let (s, c) = angle.sin_cos();
        Rot {
            x: V(c, s, 0.0),
            y: V(-s, c, 0.0),
            z: V::Z,
        }
    }

    /// From a unit quaternion `(w, x, y, z)`.
    pub fn from_quaternion(w: f64, x: f64, y: f64, z: f64) -> Rot {
        Rot {
            x: V(
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y + w * z),
                2.0 * (x * z - w * y),
            ),
            y: V(
                2.0 * (x * y - w * z),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z + w * x),
            ),
            z: V(
                2.0 * (x * z + w * y),
                2.0 * (y * z - w * x),
                1.0 - 2.0 * (x * x + y * y),
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Block,
    Cylinder,
    Cone,
    Sphere,
    Torus,
    /// A block under a roof that is a bicubic Bézier patch.
    Dome,
    /// The same block, its roof the same patch as a B-spline surface.
    Spline,
}

pub const KINDS: [Kind; 7] = [
    Kind::Block,
    Kind::Cylinder,
    Kind::Cone,
    Kind::Sphere,
    Kind::Torus,
    Kind::Dome,
    Kind::Spline,
];

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Block => "block",
            Kind::Cylinder => "cylinder",
            Kind::Cone => "cone",
            Kind::Sphere => "sphere",
            Kind::Torus => "torus",
            Kind::Dome => "dome",
            Kind::Spline => "spline",
        }
    }

    /// Whether the kind is the domed block, in either representation.
    pub fn domed(self) -> bool {
        matches!(self, Kind::Dome | Kind::Spline)
    }
}

/// A primitive in its own frame, as 3DCraft and FreeCAD both define it: a
/// block with a corner at the origin and edges along the axes; a cylinder
/// and a cone standing on the xy plane about z, the cone with its apex at
/// the height; a sphere about the origin; a torus about z.
///
/// The domed block is the one solid with a freeform face: a floor
/// `size.0 × size.1` at z = 0, four planar walls, and a roof that is a
/// bicubic Bézier patch over the regular 4 × 4 net of [`dome_net`], so
/// that the roof is the height field `z = h(x / dx, y / dy)` with `h` of
/// [`dome_height`] and its boundary curves are cubic splines in the
/// walls' planes. It is `craft3d_testkit::trimmed::domed_block`, built
/// again here from its definition. With `spline` the roof is the same
/// patch as a B-spline surface: the same points, another representation,
/// another code path in a kernel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Solid {
    Block { size: V },
    Cylinder { radius: f64, height: f64 },
    Cone { radius: f64, height: f64 },
    Sphere { radius: f64 },
    Torus { major: f64, minor: f64 },
    Dome { size: V, spline: bool },
}

/// The cubic Bernstein polynomials at `t`.
fn bernstein(t: f64) -> [f64; 4] {
    let s = 1.0 - t;
    [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t]
}

/// Their derivatives at `t`.
fn bernstein_slope(t: f64) -> [f64; 4] {
    let s = 1.0 - t;
    [
        -3.0 * s * s,
        3.0 * s * s - 6.0 * s * t,
        6.0 * s * t - 3.0 * t * t,
        3.0 * t * t,
    ]
}

/// The heights of the roof's control net, `[i][j]` for the control point
/// at `(dx i / 3, dy j / 3)`: `height × (1 + 0.15 × (((7 i + 3 j) mod 5)
/// − 2))`, between 0.7 and 1.3 times `height`, without symmetry.
pub fn dome_net(height: f64) -> [[f64; 4]; 4] {
    let mut net = [[0.0; 4]; 4];
    for (i, row) in net.iter_mut().enumerate() {
        for (j, z) in row.iter_mut().enumerate() {
            *z = height * (1.0 + 0.15 * (((i * 7 + j * 3) % 5) as f64 - 2.0));
        }
    }
    net
}

/// `Σ a_i b_j net[i][j]`.
fn tensor(a: &[f64; 4], b: &[f64; 4], net: &[[f64; 4]; 4]) -> f64 {
    a.iter()
        .zip(net)
        .map(|(a, row)| a * b.iter().zip(row).map(|(b, z)| b * z).sum::<f64>())
        .sum()
}

/// The roof's height above the floor at `(u, v)` in the unit square, for
/// the net of `height`: `h(u, v) = Σ B_i(u) B_j(v) z_ij`.
pub fn dome_height(height: f64, u: f64, v: f64) -> f64 {
    tensor(&bernstein(u), &bernstein(v), &dome_net(height))
}

/// Its partial derivatives with respect to `u` and `v`.
fn dome_slope(height: f64, u: f64, v: f64) -> (f64, f64) {
    let net = dome_net(height);
    (
        tensor(&bernstein_slope(u), &bernstein(v), &net),
        tensor(&bernstein(u), &bernstein_slope(v), &net),
    )
}

/// The mean of the net's heights: the roof's mean height, since every
/// Bernstein polynomial has the same integral over the unit interval.
fn dome_mean(height: f64) -> f64 {
    dome_net(height).iter().flatten().sum::<f64>() / 16.0
}

/// The steepest the roof gets along either parameter: `3 ×` the largest
/// difference between neighbouring control heights bounds `|∂h/∂u|` and
/// `|∂h/∂v|` (the derivative of a cubic Bézier curve is a quadratic one
/// over those differences, and a Bézier curve stays within the hull of
/// its control points).
fn dome_steepest(height: f64) -> f64 {
    let net = dome_net(height);
    let mut steepest: f64 = 0.0;
    for i in 0..4 {
        for j in 0..3 {
            let (Some(a), Some(b), Some(c), Some(d)) = (
                net.get(i).and_then(|r| r.get(j)),
                net.get(i).and_then(|r| r.get(j + 1)),
                net.get(j).and_then(|r| r.get(i)),
                net.get(j + 1).and_then(|r| r.get(i)),
            ) else {
                continue;
            };
            steepest = steepest.max((a - b).abs()).max((c - d).abs());
        }
    }
    3.0 * steepest
}

/// Gauss–Legendre nodes and weights on `[0, 1]`, `n` of them, by Newton's
/// method on the Legendre polynomial (each root bracketed by its usual
/// first guess; the iteration is capped, so a root that does not settle
/// is still returned, slightly off).
fn gauss_legendre(n: usize) -> Vec<(f64, f64)> {
    let mut rule = Vec::with_capacity(n);
    for k in 1..=n {
        let mut x = (PI * (k as f64 - 0.25) / (n as f64 + 0.5)).cos();
        let mut slope = 1.0;
        for _ in 0..100 {
            let (mut p0, mut p1) = (1.0, x);
            for m in 2..=n {
                let m = m as f64;
                (p0, p1) = (p1, ((2.0 * m - 1.0) * x * p1 - (m - 1.0) * p0) / m);
            }
            slope = n as f64 * (x * p1 - p0) / (x * x - 1.0);
            if slope == 0.0 || !slope.is_finite() {
                break;
            }
            let step = p1 / slope;
            x -= step;
            if step.abs() < 1e-16 {
                break;
            }
        }
        let weight = 1.0 / ((1.0 - x * x) * slope * slope);
        rule.push((0.5 * (1.0 - x), weight));
    }
    rule
}

/// The rule used for the roof's area: `ROOF_POINTS` Gauss–Legendre
/// points on each of `ROOF_PANELS` equal panels each way. The integrand
/// is analytic, but its nearest complex singularity comes close to the
/// real square where the roof is steep (height 3 over a side of 1): one
/// 32-point rule is then still 3e-11 off, while this composite rule, one
/// with 16 panels of 8 points and a plain 128-point rule agree to 1e-14.
const ROOF_PANELS: usize = 8;
const ROOF_POINTS: usize = 16;

/// The area of the roof: `∫∫ √(1 + z_x² + z_y²) dx dy` over the floor, by
/// composite Gauss–Legendre quadrature. There is no closed form.
fn dome_roof_area(size: V, panels: usize, points: usize) -> f64 {
    let V(dx, dy, height) = size;
    let net = dome_net(height);
    let rule = gauss_legendre(points);
    let width = 1.0 / panels.max(1) as f64;
    let mut total = 0.0;
    for pu in 0..panels {
        for pv in 0..panels {
            for &(u, wu) in &rule {
                let u = (pu as f64 + u) * width;
                let (bu, su) = (bernstein(u), bernstein_slope(u));
                for &(v, wv) in &rule {
                    let v = (pv as f64 + v) * width;
                    let (bv, sv) = (bernstein(v), bernstein_slope(v));
                    let zx = tensor(&su, &bv, &net) / dx;
                    let zy = tensor(&bu, &sv, &net) / dy;
                    total += wu * wv * (1.0 + zx * zx + zy * zy).sqrt();
                }
            }
        }
    }
    total * dx * dy * width * width
}

/// The four walls of a domed block, `k` in `0..4`, each running from one
/// corner of the floor to the next: where it starts, the unit direction
/// it runs in, its length and its outward normal.
fn dome_wall(size: V, k: usize) -> (V, V, f64, V) {
    let V(dx, dy, _) = size;
    match k % 4 {
        0 => (V::ZERO, V::X, dx, -V::Y),
        1 => (V(dx, 0.0, 0.0), V::Y, dy, V::X),
        2 => (V(dx, dy, 0.0), -V::X, dx, V::Y),
        _ => (V(0.0, dy, 0.0), -V::Y, dy, -V::X),
    }
}

/// The roof's corner above floor corner `k` (where wall `k` starts).
fn dome_corner_height(height: f64, k: usize) -> f64 {
    let (i, j) = match k % 4 {
        0 => (0, 0),
        1 => (3, 0),
        2 => (3, 3),
        _ => (0, 3),
    };
    dome_net(height)
        .get(i)
        .and_then(|row| row.get(j))
        .copied()
        .unwrap_or(height)
}

/// What kind of place on a solid another solid can be put against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// A point in a planar face.
    Face,
    /// A point on a curved face.
    Smooth,
    /// A point on a sharp edge.
    Edge,
    /// A corner or an apex.
    Vertex,
}

impl Class {
    pub fn sharp(self) -> bool {
        matches!(self, Class::Edge | Class::Vertex)
    }

    pub fn name(self) -> &'static str {
        match self {
            Class::Face => "face",
            Class::Smooth => "curved",
            Class::Edge => "edge",
            Class::Vertex => "vertex",
        }
    }
}

/// A place on a solid, in its own frame: the point `p`, a unit normal `n`
/// of a plane through `p` that has the whole solid on its inner side (a
/// supporting plane), and a unit direction `t` in that plane (along the
/// edge, or along the ruling of a cylinder or a cone).
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub p: V,
    pub n: V,
    pub t: V,
}

/// The three random numbers a place is chosen with, each in `[0, 1)`, and
/// whether to prefer round positions (centres, angle 0).
#[derive(Clone, Copy, Debug)]
pub struct Pick {
    pub which: u64,
    pub u: f64,
    pub v: f64,
    pub round: bool,
}

/// To the nearest eighth.
pub fn eighths(x: f64) -> f64 {
    (x * 8.0).round() / 8.0
}

impl Solid {
    pub fn kind(&self) -> Kind {
        match self {
            Solid::Block { .. } => Kind::Block,
            Solid::Cylinder { .. } => Kind::Cylinder,
            Solid::Cone { .. } => Kind::Cone,
            Solid::Sphere { .. } => Kind::Sphere,
            Solid::Torus { .. } => Kind::Torus,
            Solid::Dome { spline: false, .. } => Kind::Dome,
            Solid::Dome { spline: true, .. } => Kind::Spline,
        }
    }

    pub fn scaled(&self, k: f64) -> Solid {
        match *self {
            Solid::Block { size } => Solid::Block { size: size * k },
            Solid::Dome { size, spline } => Solid::Dome {
                size: size * k,
                spline,
            },
            Solid::Cylinder { radius, height } => Solid::Cylinder {
                radius: radius * k,
                height: height * k,
            },
            Solid::Cone { radius, height } => Solid::Cone {
                radius: radius * k,
                height: height * k,
            },
            Solid::Sphere { radius } => Solid::Sphere { radius: radius * k },
            Solid::Torus { major, minor } => Solid::Torus {
                major: major * k,
                minor: minor * k,
            },
        }
    }

    pub fn volume(&self) -> f64 {
        let pi = TAU / 2.0;
        match *self {
            Solid::Block { size } => size.0 * size.1 * size.2,
            Solid::Cylinder { radius, height } => pi * radius * radius * height,
            Solid::Cone { radius, height } => pi * radius * radius * height / 3.0,
            Solid::Sphere { radius } => 4.0 / 3.0 * pi * radius * radius * radius,
            Solid::Torus { major, minor } => 2.0 * pi * pi * major * minor * minor,
            // The floor times the roof's mean height.
            Solid::Dome { size, .. } => size.0 * size.1 * dome_mean(size.2),
        }
    }

    /// The area: a closed form, except for the roof of a domed block,
    /// which is a quadrature that three rules agree on to 1e-14.
    pub fn area(&self) -> f64 {
        let pi = TAU / 2.0;
        match *self {
            Solid::Block { size } => 2.0 * (size.0 * size.1 + size.1 * size.2 + size.2 * size.0),
            Solid::Cylinder { radius, height } => TAU * radius * (radius + height),
            Solid::Cone { radius, height } => pi * radius * (radius + radius.hypot(height)),
            Solid::Sphere { radius } => 4.0 * pi * radius * radius,
            Solid::Torus { major, minor } => 4.0 * pi * pi * major * minor,
            Solid::Dome { size, .. } => {
                let V(dx, dy, height) = size;
                // Each wall is the region under a cubic Bézier curve, the
                // border of the net in its plane: its length times the
                // mean of that border's heights.
                let net = dome_net(height);
                let border = |heights: [f64; 4]| heights.iter().sum::<f64>() / 4.0;
                let row = |j: usize| {
                    let mut h = [0.0; 4];
                    for (i, z) in h.iter_mut().enumerate() {
                        *z = net.get(i).and_then(|r| r.get(j)).copied().unwrap_or(0.0);
                    }
                    h
                };
                let column = |i: usize| net.get(i).copied().unwrap_or([0.0; 4]);
                let walls = dx * (border(row(0)) + border(row(3)))
                    + dy * (border(column(0)) + border(column(3)));
                dx * dy + walls + dome_roof_area(size, ROOF_PANELS, ROOF_POINTS)
            }
        }
    }

    /// The classes of place this kind of solid has.
    pub fn classes(kind: Kind) -> &'static [Class] {
        match kind {
            Kind::Block => &[Class::Face, Class::Edge, Class::Vertex],
            Kind::Cylinder => &[Class::Face, Class::Smooth, Class::Edge],
            Kind::Cone | Kind::Dome | Kind::Spline => {
                &[Class::Face, Class::Smooth, Class::Edge, Class::Vertex]
            }
            Kind::Sphere | Kind::Torus => &[Class::Smooth],
        }
    }

    /// Whether a place of this class on this kind has a supporting plane
    /// (the whole solid on its inner side), so that another solid put
    /// against it touches it there and nowhere else. True everywhere but
    /// on the roof of a domed block: the roof has no point whose tangent
    /// plane clears the rest of it (checked on a grid: its bumps rise
    /// above every tangent plane), so a contact there is tangent at the
    /// point and says nothing about the rest.
    pub fn supported(kind: Kind, class: Class) -> bool {
        !(kind.domed() && class == Class::Smooth)
    }

    /// A place of the class, or `None` when the solid has none.
    pub fn place(&self, class: Class, pick: Pick) -> Option<Place> {
        let angle = if pick.round { 0.0 } else { TAU * pick.u };
        let (sin, cos) = angle.sin_cos();
        let radial = V(cos, sin, 0.0);
        let around = V(-sin, cos, 0.0);
        // A fraction of a half-size to move off a centre by.
        let off = |t: f64, half: f64| {
            if pick.round {
                eighths(0.3 * (2.0 * t - 1.0) * half)
            } else {
                0.3 * (2.0 * t - 1.0) * half
            }
        };
        let sign = |bit: u64| {
            if pick.which >> bit & 1 == 0 {
                1.0
            } else {
                -1.0
            }
        };
        match (*self, class) {
            (Solid::Block { size }, Class::Face) => {
                let a = (pick.which % 3) as usize;
                let half = size * 0.5;
                let s = sign(4);
                let p = half
                    + V::axis(a) * (s * half.component(a))
                    + V::axis(a + 1) * off(pick.u, half.component(a + 1))
                    + V::axis(a + 2) * off(pick.v, half.component(a + 2));
                Some(Place {
                    p,
                    n: V::axis(a) * s,
                    t: V::axis(a + 1),
                })
            }
            (Solid::Block { size }, Class::Edge) => {
                let a = (pick.which % 3) as usize;
                let half = size * 0.5;
                let (s1, s2) = (sign(4), sign(5));
                let p = half
                    + V::axis(a + 1) * (s1 * half.component(a + 1))
                    + V::axis(a + 2) * (s2 * half.component(a + 2))
                    + V::axis(a) * off(pick.u, half.component(a));
                Some(Place {
                    p,
                    n: (V::axis(a + 1) * s1 + V::axis(a + 2) * s2) * FRAC_1_SQRT_2,
                    t: V::axis(a),
                })
            }
            (Solid::Block { size }, Class::Vertex) => {
                let s = V(sign(4), sign(5), sign(6));
                let half = size * 0.5;
                let n = s.unit();
                Some(Place {
                    p: half + V(s.0 * half.0, s.1 * half.1, s.2 * half.2),
                    n,
                    t: n.perpendicular(),
                })
            }
            (Solid::Cylinder { radius, height }, Class::Face) => {
                let top = pick.which & 1 == 0;
                let rho = if pick.round {
                    0.0
                } else {
                    0.5 * radius * pick.v
                };
                Some(Place {
                    p: radial * rho + V::Z * if top { height } else { 0.0 },
                    n: if top { V::Z } else { -V::Z },
                    t: V::X,
                })
            }
            (Solid::Cylinder { radius, height }, Class::Smooth) => {
                let f = if pick.round { 0.5 } else { 0.25 + 0.5 * pick.v };
                Some(Place {
                    p: radial * radius + V::Z * (height * f),
                    n: radial,
                    t: V::Z,
                })
            }
            (Solid::Cylinder { radius, height }, Class::Edge) => {
                let top = pick.which & 1 == 0;
                let up = if top { V::Z } else { -V::Z };
                Some(Place {
                    p: radial * radius + V::Z * if top { height } else { 0.0 },
                    n: (radial + up) * FRAC_1_SQRT_2,
                    t: around,
                })
            }
            (Solid::Cone { radius, .. }, Class::Face) => {
                let rho = if pick.round {
                    0.0
                } else {
                    0.5 * radius * pick.v
                };
                Some(Place {
                    p: radial * rho,
                    n: -V::Z,
                    t: V::X,
                })
            }
            (Solid::Cone { radius, height }, Class::Smooth) => {
                let f = if pick.round { 0.5 } else { 0.2 + 0.4 * pick.v };
                Some(Place {
                    p: radial * (radius * (1.0 - f)) + V::Z * (height * f),
                    n: (radial * height + V::Z * radius).unit(),
                    t: (V::Z * height - radial * radius).unit(),
                })
            }
            (Solid::Cone { radius, .. }, Class::Edge) => Some(Place {
                p: radial * radius,
                n: (radial - V::Z) * FRAC_1_SQRT_2,
                t: around,
            }),
            (Solid::Cone { height, .. }, Class::Vertex) => Some(Place {
                p: V::Z * height,
                n: V::Z,
                t: V::X,
            }),
            (Solid::Sphere { radius }, Class::Smooth) => {
                let n = if pick.round {
                    V::axis((pick.which % 3) as usize) * sign(4)
                } else {
                    let z = 2.0 * pick.v - 1.0;
                    let r = (1.0 - z * z).max(0.0).sqrt();
                    V(r * cos, r * sin, z)
                };
                Some(Place {
                    p: n * radius,
                    n,
                    t: n.perpendicular(),
                })
            }
            (Solid::Torus { major, minor }, Class::Smooth) => Some(match pick.which % 3 {
                // The outer equator.
                0 => Place {
                    p: radial * (major + minor),
                    n: radial,
                    t: V::Z,
                },
                // The circles it lies on when laid flat.
                1 => Place {
                    p: radial * major + V::Z * minor,
                    n: V::Z,
                    t: around,
                },
                _ => Place {
                    p: radial * major - V::Z * minor,
                    n: -V::Z,
                    t: around,
                },
            }),
            // The domed block: its floor and walls are planes, its floor
            // edges and uprights straight and sharp, its eight corners
            // sharp; the roof is curved. The roof's own edges, splines,
            // are no place: a sphere or a torus has none like them.
            (Solid::Dome { size, .. }, Class::Face) => {
                let V(dx, dy, height) = size;
                // A wall reaches at least 0.7 height everywhere: a point
                // about 0.35 height up is well inside it.
                let up = 0.35 * height;
                Some(match (pick.which % 5) as usize {
                    0 => Place {
                        p: V(
                            0.5 * dx + off(pick.u, 0.5 * dx),
                            0.5 * dy + off(pick.v, 0.5 * dy),
                            0.0,
                        ),
                        n: -V::Z,
                        t: V::X,
                    },
                    k => {
                        let (start, along, length, n) = dome_wall(size, k - 1);
                        Place {
                            p: start
                                + along * (0.5 * length + off(pick.u, 0.5 * length))
                                + V::Z * (up + off(pick.v, up)),
                            n,
                            t: along,
                        }
                    }
                })
            }
            (Solid::Dome { size, .. }, Class::Edge) => {
                let up = 0.35 * size.2;
                let k = (pick.which % 8) as usize;
                let (start, along, length, n) = dome_wall(size, k);
                Some(if k < 4 {
                    // A floor edge.
                    Place {
                        p: start + along * (0.5 * length + off(pick.u, 0.5 * length)),
                        n: (n - V::Z) * FRAC_1_SQRT_2,
                        t: along,
                    }
                } else {
                    // The upright where wall `k - 1` meets wall `k`.
                    let (_, _, _, before) = dome_wall(size, k + 3);
                    Place {
                        p: start + V::Z * (up + off(pick.u, up)),
                        n: (before + n) * FRAC_1_SQRT_2,
                        t: V::Z,
                    }
                })
            }
            (Solid::Dome { size, .. }, Class::Vertex) => {
                let V(dx, dy, height) = size;
                let k = (pick.which % 8) as usize;
                let (corner, _, _, n) = dome_wall(size, k);
                let (_, _, _, before) = dome_wall(size, k + 3);
                let sideways = before + n;
                let n = if k < 4 {
                    // A floor corner: the plane x + y + z = 0 there, with
                    // the signs of the corner, has the solid on one side.
                    (sideways - V::Z).unit()
                } else {
                    // A roof corner. The plane with normal (±1, ±1, b)
                    // through it has the solid below it when the roof
                    // climbs away from the corner more slowly than the
                    // plane: b |∂z/∂x| ≤ 1 and b |∂z/∂y| ≤ 1 everywhere,
                    // which `dome_steepest` bounds. Half that b, so that
                    // the plane touches the solid at the corner (and
                    // along the upright below it, where it only has the
                    // solid's points strictly below the corner) alone.
                    let b = 0.5 * dx.min(dy) / dome_steepest(height);
                    (sideways + V::Z * b).unit()
                };
                let p = if k < 4 {
                    corner
                } else {
                    corner + V::Z * dome_corner_height(height, k)
                };
                Some(Place {
                    p,
                    n,
                    t: n.perpendicular(),
                })
            }
            (Solid::Dome { size, .. }, Class::Smooth) => {
                // A point of the roof away from its edges, with the
                // roof's tangent plane: not a supporting plane, see
                // `supported`.
                let V(dx, dy, height) = size;
                let (u, v) = if pick.round {
                    (0.5, 0.5)
                } else {
                    (0.25 + 0.5 * pick.u, 0.25 + 0.5 * pick.v)
                };
                let (hu, hv) = dome_slope(height, u, v);
                Some(Place {
                    p: V(u * dx, v * dy, dome_height(height, u, v)),
                    // ∂u × ∂v for ∂u = (dx, 0, h_u) and ∂v = (0, dy, h_v):
                    // upward.
                    n: V(-hu * dy, -dx * hv, dx * dy).unit(),
                    t: V(dx, 0.0, hu).unit(),
                })
            }
            _ => None,
        }
    }

    /// A ball inside the solid: centre and radius. `turn` in `[0, 1)`
    /// chooses where in a torus.
    pub fn inner_ball(&self, turn: f64) -> (V, f64) {
        match *self {
            Solid::Block { size } => (size * 0.5, 0.5 * size.0.min(size.1).min(size.2)),
            Solid::Cylinder { radius, height } => (V::Z * (0.5 * height), radius.min(0.5 * height)),
            Solid::Cone { radius, height } => {
                let rho = radius * height / (radius + radius.hypot(height));
                (V::Z * rho, rho)
            }
            Solid::Sphere { radius } => (V::ZERO, radius),
            Solid::Torus { major, minor } => {
                let (s, c) = (TAU * turn).sin_cos();
                (V(major * c, major * s, 0.0), minor)
            }
            // Under the lowest the roof gets (0.7 height).
            Solid::Dome { size, .. } => {
                let up = 0.35 * size.2;
                (
                    V(0.5 * size.0, 0.5 * size.1, up),
                    (0.5 * size.0).min(0.5 * size.1).min(up),
                )
            }
        }
    }

    /// A ball around the solid: centre and radius.
    pub fn outer_ball(&self) -> (V, f64) {
        match *self {
            Solid::Block { size } => (size * 0.5, 0.5 * size.length()),
            Solid::Cylinder { radius, height } | Solid::Cone { radius, height } => {
                (V::Z * (0.5 * height), radius.hypot(0.5 * height))
            }
            Solid::Sphere { radius } => (V::ZERO, radius),
            Solid::Torus { major, minor } => (V::ZERO, major + minor),
            Solid::Dome { .. } => {
                let (lo, hi) = self.local_box();
                ((lo + hi) * 0.5, 0.5 * (hi - lo).length())
            }
        }
    }

    /// A point of the solid's own axis: the z axis of its frame, moved to
    /// the middle of a block.
    pub fn axis_point(&self) -> V {
        match *self {
            Solid::Block { size } | Solid::Dome { size, .. } => V(0.5 * size.0, 0.5 * size.1, 0.0),
            _ => V::ZERO,
        }
    }

    /// From where to where the solid reaches along its own z.
    pub fn axial_extent(&self) -> (f64, f64) {
        match *self {
            // The roof of a dome is about `height` up.
            Solid::Block { size } | Solid::Dome { size, .. } => (0.0, size.2),
            Solid::Cylinder { height, .. } | Solid::Cone { height, .. } => (0.0, height),
            Solid::Sphere { radius } => (-radius, radius),
            Solid::Torus { minor, .. } => (-minor, minor),
        }
    }

    /// The box of the solid in its own frame.
    pub fn local_box(&self) -> (V, V) {
        match *self {
            Solid::Block { size } => (V::ZERO, size),
            Solid::Cylinder { radius, height } | Solid::Cone { radius, height } => {
                (V(-radius, -radius, 0.0), V(radius, radius, height))
            }
            Solid::Sphere { radius } => (V(-radius, -radius, -radius), V(radius, radius, radius)),
            Solid::Torus { major, minor } => {
                let r = major + minor;
                (V(-r, -r, -minor), V(r, r, minor))
            }
            // The roof stays within the hull of its net: below 1.3 height.
            Solid::Dome { size, .. } => (V::ZERO, V(size.0, size.1, 1.3 * size.2)),
        }
    }

    /// Whether the point, in the solid's own frame, is inside it.
    pub fn contains(&self, p: V) -> bool {
        let rho = p.0.hypot(p.1);
        match *self {
            Solid::Block { size } => {
                (0.0..=size.0).contains(&p.0)
                    && (0.0..=size.1).contains(&p.1)
                    && (0.0..=size.2).contains(&p.2)
            }
            Solid::Cylinder { radius, height } => rho <= radius && (0.0..=height).contains(&p.2),
            Solid::Cone { radius, height } => {
                (0.0..=height).contains(&p.2) && rho <= radius * (1.0 - p.2 / height)
            }
            Solid::Sphere { radius } => p.length() <= radius,
            Solid::Torus { major, minor } => (rho - major).hypot(p.2) <= minor,
            Solid::Dome { size, .. } => {
                (0.0..=size.0).contains(&p.0)
                    && (0.0..=size.1).contains(&p.1)
                    && p.2 >= 0.0
                    && p.2 <= dome_height(size.2, p.0 / size.0, p.1 / size.1)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dome(dx: f64, dy: f64, height: f64) -> Solid {
        Solid::Dome {
            size: V(dx, dy, height),
            spline: false,
        }
    }

    /// Points of the solid: a grid over the floor, up to the roof.
    fn dome_points(size: V) -> Vec<V> {
        let mut points = Vec::new();
        let n = 60;
        for a in 0..=n {
            for b in 0..=n {
                let (u, v) = (a as f64 / n as f64, b as f64 / n as f64);
                let top = dome_height(size.2, u, v);
                // Up to just under the roof: `contains` evaluates the
                // roof again at `x / dx`, which rounds.
                for c in 0..=6 {
                    let f = if c == 6 { 1.0 - 1e-12 } else { c as f64 / 6.0 };
                    points.push(V(u * size.0, v * size.1, top * f));
                }
            }
        }
        points
    }

    #[test]
    fn the_net_is_the_test_kits() {
        let net = dome_net(2.0);
        assert_eq!(net[0], [1.4, 2.3, 1.7, 2.6]);
        assert_eq!(net[3], [1.7, 2.6, 2.0, 1.4]);
        assert!((dome_mean(1.0) - 0.98125).abs() < 1e-15);
        assert!((dome_steepest(1.0) - 1.35).abs() < 1e-15);
        // The corners of the patch are the corners of the net.
        assert!((dome_height(1.0, 0.0, 0.0) - 0.7).abs() < 1e-15);
        assert!((dome_height(1.0, 1.0, 1.0) - 0.7).abs() < 1e-15);
        assert!((dome_height(1.0, 0.0, 1.0) - 1.3).abs() < 1e-15);
        assert!((dome_height(1.0, 1.0, 0.0) - 0.85).abs() < 1e-15);
        for k in 0..4 {
            let (corner, ..) = dome_wall(V(2.0, 3.0, 1.0), k);
            assert!(
                (dome_corner_height(1.0, k) - dome_height(1.0, corner.0 / 2.0, corner.1 / 3.0))
                    .abs()
                    < 1e-15
            );
        }
    }

    #[test]
    fn the_roof_area_is_settled() {
        // A gentle roof, and the steepest ones the corpus makes.
        for size in [
            V(2.0, 1.5, 2.5),
            V(1.0, 3.0, 0.8),
            V(3.0, 1.0, 3.0),
            V(1.0, 1.0, 3.0),
        ] {
            let settled = dome_roof_area(size, ROOF_PANELS, ROOF_POINTS);
            for (panels, points) in [(4, 16), (16, 8), (1, 128)] {
                let other = dome_roof_area(size, panels, points);
                assert!(
                    (other - settled).abs() < 1e-13 * settled,
                    "{size:?} {panels}x{points}: {other} vs {settled}"
                );
            }
        }
        // Against the figure of the scratch check with FreeCAD's dome of
        // these sizes (whose own roof area was 9.5e-7 off this).
        assert!((dome(2.0, 1.5, 2.5).area() - 23.67869696607736).abs() < 1e-12);
        let rule = gauss_legendre(ROOF_POINTS);
        let weights: f64 = rule.iter().map(|(_, w)| w).sum();
        assert!((weights - 1.0).abs() < 1e-15);
        let mean: f64 = rule.iter().map(|(x, w)| x * w).sum();
        assert!((mean - 0.5).abs() < 1e-15);
    }

    #[test]
    fn the_roof_is_the_height_field() {
        let solid = dome(2.0, 1.5, 2.5);
        let V(dx, dy, height) = V(2.0, 1.5, 2.5);
        for &(u, v) in &[(0.3, 0.7), (0.5, 0.5), (0.05, 0.95), (0.8, 0.2)] {
            let top = dome_height(height, u, v);
            assert!(solid.contains(V(u * dx, v * dy, top - 1e-9)));
            assert!(!solid.contains(V(u * dx, v * dy, top + 1e-9)));
            // The slope by finite differences.
            let (hu, hv) = dome_slope(height, u, v);
            let e = 1e-6;
            let du = (dome_height(height, u + e, v) - dome_height(height, u - e, v)) / (2.0 * e);
            let dv = (dome_height(height, u, v + e) - dome_height(height, u, v - e)) / (2.0 * e);
            assert!((hu - du).abs() < 1e-7 && (hv - dv).abs() < 1e-7);
        }
        let (lo, hi) = solid.local_box();
        for p in dome_points(V(dx, dy, height)) {
            assert!(solid.contains(p));
            assert!(p.0 >= lo.0 && p.1 >= lo.1 && p.2 >= lo.2);
            assert!(p.0 <= hi.0 && p.1 <= hi.1 && p.2 <= hi.2);
        }
        let (centre, radius) = solid.inner_ball(0.0);
        for d in [V::X, -V::X, V::Y, -V::Y, V::Z, -V::Z] {
            assert!(solid.contains(centre + d * (radius * 0.999)));
        }
    }

    /// Every place of a domed block lies on it and has the solid on the
    /// inner side of its plane, except the roof's, which is tangent and
    /// says so.
    #[test]
    fn places_of_a_dome_support_it() {
        for size in [V(2.0, 1.5, 2.5), V(1.0, 3.0, 0.8), V(3.0, 1.0, 3.0)] {
            let solid = Solid::Dome {
                size,
                spline: false,
            };
            let points = dome_points(size);
            for class in Solid::classes(Kind::Dome) {
                for which in 0..8 {
                    for (u, v, round) in [(0.3, 0.8, false), (0.9, 0.1, false), (0.5, 0.5, true)] {
                        let pick = Pick { which, u, v, round };
                        let place = solid.place(*class, pick).unwrap();
                        assert!((place.n.length() - 1.0).abs() < 1e-12);
                        assert!((place.t.length() - 1.0).abs() < 1e-12);
                        assert!(place.n.dot(place.t).abs() < 1e-12, "{class:?} {which}");
                        // On the boundary: outside just above the normal
                        // and inside just below it, except at a corner,
                        // where the plane need not point into the solid
                        // (a roof corner's does not: the roof may fall
                        // away from it faster than the plane).
                        assert!(
                            !solid.contains(place.p + place.n * 1e-7),
                            "{class:?} {which}"
                        );
                        if *class == Class::Vertex {
                            assert!(solid.contains(place.p), "{class:?} {which}");
                        } else {
                            assert!(
                                solid.contains(place.p - place.n * 1e-7),
                                "{class:?} {which}"
                            );
                        }
                        if !Solid::supported(Kind::Dome, *class) {
                            continue;
                        }
                        let worst = points
                            .iter()
                            .map(|&q| place.n.dot(q - place.p))
                            .fold(f64::MIN, f64::max);
                        assert!(worst <= 1e-12, "{class:?} {which} {size:?}: {worst}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_roof_has_no_supporting_tangent_plane() {
        let size = V(2.0, 1.5, 2.5);
        let solid = Solid::Dome {
            size,
            spline: false,
        };
        let points = dome_points(size);
        let n = 20;
        for a in 1..n {
            for b in 1..n {
                let pick = Pick {
                    which: 0,
                    u: a as f64 / n as f64,
                    v: b as f64 / n as f64,
                    round: false,
                };
                let place = solid.place(Class::Smooth, pick).unwrap();
                let above = points
                    .iter()
                    .map(|&q| place.n.dot(q - place.p))
                    .fold(f64::MIN, f64::max);
                assert!(above > 1e-6, "a supporting tangent plane at {pick:?}");
            }
        }
    }
}

/// A solid somewhere: its frame has origin `o` and axes `r`.
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub solid: Solid,
    pub o: V,
    pub r: Rot,
}

impl Placed {
    pub fn world(&self, local: V) -> V {
        self.o + self.r.apply(local)
    }

    pub fn local(&self, world: V) -> V {
        let d = world - self.o;
        V(d.dot(self.r.x), d.dot(self.r.y), d.dot(self.r.z))
    }
}

/// How the second solid is turned about the common normal of a contact.
#[derive(Clone, Copy, Debug)]
pub enum Spin {
    /// Directions aligned: an edge along an edge, a ruling along a ruling.
    Aligned,
    /// A quarter turn, exactly.
    Quarter,
    Angle(f64),
}

/// Puts `solid` against `a`: its place `fb` at `a`'s place `fa`, moved
/// `gap` along `a`'s outward normal there. The normals are opposite (the
/// solids on either side of the common plane, touching only in it) unless
/// `same_side`, which lays both on one side of the plane with the places
/// coincident.
pub fn against(
    a: &Placed,
    fa: Place,
    solid: Solid,
    fb: Place,
    same_side: bool,
    spin: Spin,
    gap: f64,
) -> Placed {
    let normal = a.r.apply(fa.n);
    let along = a.r.apply(fa.t);
    let across = normal.cross(along);
    let m = if same_side { normal } else { -normal };
    let s = match spin {
        Spin::Aligned => along,
        Spin::Quarter => across,
        Spin::Angle(angle) => along * angle.cos() + across * angle.sin(),
    };
    let w = m.cross(s);
    let l = fb.n.cross(fb.t);
    // The rotation that takes (fb.n, fb.t, l) to (m, s, w).
    let turn = |v: V| m * fb.n.dot(v) + s * fb.t.dot(v) + w * l.dot(v);
    let r = Rot {
        x: turn(V::X),
        y: turn(V::Y),
        z: turn(V::Z),
    };
    let o = a.world(fa.p) + normal * gap - r.apply(fb.p);
    Placed { solid, o, r }
}
