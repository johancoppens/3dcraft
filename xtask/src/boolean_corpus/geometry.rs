//! The little geometry the case generator needs: vectors, rotations, the
//! five primitive solids and what can be said about each without a kernel
//! (where its faces, edges and corners are, a ball inside it, a ball around
//! it, whether a point is inside).
//!
//! `xtask` depends on no 3DCraft crate, on purpose: the corpus must not be
//! built with the code it is going to judge.

use std::f64::consts::{FRAC_1_SQRT_2, TAU};
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
}

pub const KINDS: [Kind; 5] = [
    Kind::Block,
    Kind::Cylinder,
    Kind::Cone,
    Kind::Sphere,
    Kind::Torus,
];

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Block => "block",
            Kind::Cylinder => "cylinder",
            Kind::Cone => "cone",
            Kind::Sphere => "sphere",
            Kind::Torus => "torus",
        }
    }
}

/// A primitive in its own frame, as 3DCraft and FreeCAD both define it: a
/// block with a corner at the origin and edges along the axes; a cylinder
/// and a cone standing on the xy plane about z, the cone with its apex at
/// the height; a sphere about the origin; a torus about z.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Solid {
    Block { size: V },
    Cylinder { radius: f64, height: f64 },
    Cone { radius: f64, height: f64 },
    Sphere { radius: f64 },
    Torus { major: f64, minor: f64 },
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
        }
    }

    pub fn scaled(&self, k: f64) -> Solid {
        match *self {
            Solid::Block { size } => Solid::Block { size: size * k },
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
        }
    }

    pub fn area(&self) -> f64 {
        let pi = TAU / 2.0;
        match *self {
            Solid::Block { size } => 2.0 * (size.0 * size.1 + size.1 * size.2 + size.2 * size.0),
            Solid::Cylinder { radius, height } => TAU * radius * (radius + height),
            Solid::Cone { radius, height } => pi * radius * (radius + radius.hypot(height)),
            Solid::Sphere { radius } => 4.0 * pi * radius * radius,
            Solid::Torus { major, minor } => 4.0 * pi * pi * major * minor,
        }
    }

    /// The classes of place this kind of solid has.
    pub fn classes(kind: Kind) -> &'static [Class] {
        match kind {
            Kind::Block => &[Class::Face, Class::Edge, Class::Vertex],
            Kind::Cylinder => &[Class::Face, Class::Smooth, Class::Edge],
            Kind::Cone => &[Class::Face, Class::Smooth, Class::Edge, Class::Vertex],
            Kind::Sphere | Kind::Torus => &[Class::Smooth],
        }
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
        }
    }

    /// A point of the solid's own axis: the z axis of its frame, moved to
    /// the middle of a block.
    pub fn axis_point(&self) -> V {
        match *self {
            Solid::Block { size } => V(0.5 * size.0, 0.5 * size.1, 0.0),
            _ => V::ZERO,
        }
    }

    /// From where to where the solid reaches along its own z.
    pub fn axial_extent(&self) -> (f64, f64) {
        match *self {
            Solid::Block { size } => (0.0, size.2),
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
