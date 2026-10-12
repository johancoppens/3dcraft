//! The cases of the boolean corpus: pairs of primitive solids in the
//! configurations a boolean has to get right, generated from a seed.
//!
//! Everything is generated at scale 1 (sizes of about one, tolerance 1e-6);
//! a case's `scale` says by how much both sides multiply its lengths.

use super::geometry::{
    Class, KINDS, Kind, Pick, Place, Placed, Rot, Solid, Spin, V, against, eighths,
};
use std::f64::consts::TAU;

/// The linear tolerance at scale 1; gaps of the `near` category are
/// multiples of it.
pub const TOLERANCE: f64 = 1e-6;

/// What the construction itself says about the pair, whatever a kernel
/// answers. Used to catch a wrong reference, never as the reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    /// No common volume, and no contact either.
    Apart,
    /// No common volume; the solids may touch.
    Disjoint,
    /// B lies in A; it may touch A's boundary from inside.
    BInA,
    AInB,
    /// The same set of points.
    Same,
}

impl Expect {
    pub fn name(self) -> &'static str {
        match self {
            Expect::Apart => "apart",
            Expect::Disjoint => "disjoint",
            Expect::BInA => "b_in_a",
            Expect::AInB => "a_in_b",
            Expect::Same => "same",
        }
    }

    fn swapped(self) -> Expect {
        match self {
            Expect::BInA => Expect::AInB,
            Expect::AInB => Expect::BInA,
            other => other,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Case {
    pub id: String,
    pub category: &'static str,
    pub variant: String,
    pub scale: f64,
    /// How far the solids were moved apart from an exact contact, in
    /// tolerances; negative when moved into each other.
    pub gap: Option<f64>,
    /// The case with the same solids in exact contact.
    pub snap: Option<String>,
    pub expect: Option<Expect>,
    pub a: Placed,
    pub b: Placed,
}

/// SplitMix64: small, seeded, the same on every platform.
struct Rng(u64);

impl Rng {
    /// Seeded by the corpus seed and a name, so that adding cases to one
    /// category leaves every other case as it was.
    fn named(seed: u64, name: &str) -> Rng {
        // FNV-1a.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in name.bytes() {
            h = (h ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        let mut rng = Rng(seed ^ h);
        rng.next();
        rng
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn between(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    fn direction(&mut self) -> V {
        let z = self.between(-1.0, 1.0);
        let (s, c) = (TAU * self.unit()).sin_cos();
        let r = (1.0 - z * z).max(0.0).sqrt();
        V(r * c, r * s, z)
    }

    /// A rotation, uniform over all of them.
    fn rotation(&mut self) -> Rot {
        let (u1, u2, u3) = (self.unit(), self.unit(), self.unit());
        let (a, b) = ((1.0 - u1).sqrt(), u1.sqrt());
        let (s2, c2) = (TAU * u2).sin_cos();
        let (s3, c3) = (TAU * u3).sin_cos();
        Rot::from_quaternion(b * c3, a * s2, a * c2, b * s3)
    }

    fn pick(&mut self, round: bool) -> Pick {
        Pick {
            which: self.next() >> 20,
            u: self.unit(),
            v: self.unit(),
            round,
        }
    }

    /// A length between `lo` and `hi`; a multiple of an eighth when
    /// `round`.
    fn length(&mut self, lo: f64, hi: f64, round: bool) -> f64 {
        let x = self.between(lo, hi);
        if round { eighths(x).max(0.125) } else { x }
    }

    fn solid(&mut self, kind: Kind, round: bool) -> Solid {
        match kind {
            Kind::Block => Solid::Block {
                size: V(
                    self.length(0.6, 3.0, round),
                    self.length(0.6, 3.0, round),
                    self.length(0.6, 3.0, round),
                ),
            },
            Kind::Cylinder => Solid::Cylinder {
                radius: self.length(0.4, 1.5, round),
                height: self.length(0.8, 3.0, round),
            },
            Kind::Cone => Solid::Cone {
                radius: self.length(0.5, 1.6, round),
                height: self.length(0.8, 3.0, round),
            },
            Kind::Sphere => Solid::Sphere {
                radius: self.length(0.5, 1.6, round),
            },
            Kind::Torus => {
                let major = self.length(1.2, 2.5, round);
                Solid::Torus {
                    major,
                    minor: self.length(0.25, 0.45 * major, round),
                }
            }
        }
    }

    /// A frame for the first solid: the world's axes at a round origin
    /// when `round`, anything otherwise.
    fn frame(&mut self, round: bool) -> (V, Rot) {
        if round {
            let mut c = || eighths(self.between(-2.0, 2.0));
            (V(c(), c(), c()), Rot::IDENTITY)
        } else {
            let o = V(
                self.between(-4.0, 4.0),
                self.between(-4.0, 4.0),
                self.between(-4.0, 4.0),
            );
            (o, self.rotation())
        }
    }

    fn spin(&mut self, round: bool, turn: u64) -> Spin {
        match (turn % 3, round) {
            (0, _) => Spin::Aligned,
            (1, _) => Spin::Quarter,
            (_, true) => Spin::Aligned,
            (_, false) => Spin::Angle(TAU * self.unit()),
        }
    }
}

const SCALES: [f64; 5] = [1.0, 1.0, 1.0, 1e-3, 1e6];

fn pair_name(k1: Kind, k2: Kind) -> String {
    format!("{}-{}", k1.name(), k2.name())
}

/// The cases of one category and kind pair, numbered as they are added.
struct Group<'a> {
    seed: u64,
    category: &'static str,
    pair: String,
    out: &'a mut Vec<Case>,
    count: usize,
}

impl Group<'_> {
    fn index(&self) -> usize {
        self.count
    }

    fn id(&self, index: usize) -> String {
        format!("{}/{}/{:03}", self.category, self.pair, index)
    }

    /// The generator of the next case.
    fn rng(&self) -> Rng {
        Rng::named(self.seed, &self.id(self.count))
    }

    fn push(&mut self, variant: String, expect: Option<Expect>, a: Placed, b: Placed) -> String {
        self.push_near(variant, expect, a, b, None, None)
    }

    fn push_near(
        &mut self,
        variant: String,
        expect: Option<Expect>,
        a: Placed,
        b: Placed,
        gap: Option<f64>,
        snap: Option<String>,
    ) -> String {
        let id = self.id(self.count);
        let scale = SCALES
            .get(self.count % SCALES.len())
            .copied()
            .unwrap_or(1.0);
        self.out.push(Case {
            id: id.clone(),
            category: self.category,
            variant,
            scale,
            gap,
            snap,
            expect,
            a,
            b,
        });
        self.count += 1;
        id
    }
}

/// The two kinds in the order the case uses them, alternating.
fn ordered(k1: Kind, k2: Kind, swap: bool) -> (Kind, Kind) {
    if swap { (k2, k1) } else { (k1, k2) }
}

/// `b` placed so that a ball inside it overlaps a ball inside `a`: the
/// solids share volume, whatever their kinds.
fn overlapping(rng: &mut Rng, a: &Placed, solid: Solid, r: Rot) -> Placed {
    let (ca, ra) = a.solid.inner_ball(rng.unit());
    let (cb, rb) = solid.inner_ball(rng.unit());
    let target = a.world(ca) + rng.direction() * (rng.between(0.15, 0.85) * (ra + rb));
    Placed {
        solid,
        o: target - r.apply(cb),
        r,
    }
}

fn general(g: &mut Group, k1: Kind, k2: Kind, n: usize) {
    for i in 0..n {
        let mut rng = g.rng();
        let (ka, kb) = ordered(k1, k2, i % 2 == 1);
        let (o, r) = rng.frame(false);
        let a = Placed {
            solid: rng.solid(ka, false),
            o,
            r,
        };
        let solid = rng.solid(kb, false);
        let turn = rng.rotation();
        let b = overlapping(&mut rng, &a, solid, turn);
        g.push("overlap".to_string(), None, a, b);
    }
}

/// Axes parallel, nothing coincident.
fn parallel(g: &mut Group, k1: Kind, k2: Kind, n: usize) {
    for i in 0..n {
        let mut rng = g.rng();
        let (ka, kb) = ordered(k1, k2, i % 2 == 1);
        // The world's axes for half of them; positions are never round.
        let (o, _) = rng.frame(false);
        let r = if i % 4 < 2 {
            Rot::IDENTITY
        } else {
            rng.rotation()
        };
        let a = Placed {
            solid: rng.solid(ka, false),
            o,
            r,
        };
        let solid = rng.solid(kb, false);
        let (turn, variant) = if i % 8 < 4 {
            (r, "same-axes")
        } else {
            (r.after(&Rot::about_z(TAU * rng.unit())), "parallel-z")
        };
        let b = overlapping(&mut rng, &a, solid, turn);
        g.push(variant.to_string(), None, a, b);
    }
}

/// On one axis, with different radii, overlapping along it.
fn coaxial(g: &mut Group, k1: Kind, k2: Kind, n: usize) {
    for i in 0..n {
        let mut rng = g.rng();
        let (ka, kb) = ordered(k1, k2, i % 2 == 1);
        let (o, r) = rng.frame(i % 4 < 2);
        let a = Placed {
            solid: rng.solid(ka, false),
            o,
            r,
        };
        let solid = rng.solid(kb, false);
        let opposite = i % 3 == 2;
        let turn = if opposite { r.after(&Rot::FLIP) } else { r };
        let (la, ha) = a.solid.axial_extent();
        let (lb, hb) = solid.axial_extent();
        // Where on A's axis the middle of B goes.
        let at = rng.between(la, ha);
        let middle = 0.5 * (lb + hb) * if opposite { -1.0 } else { 1.0 };
        let on_axis = a.world(a.solid.axis_point() + V::Z * (at - middle));
        let b = Placed {
            solid,
            o: on_axis - turn.apply(solid.axis_point()),
            r: turn,
        };
        let variant = if opposite { "opposite" } else { "same-way" };
        g.push(variant.to_string(), None, a, b);
    }
}

type Kinds = (Kind, Kind);
type Classes = (Class, Class);
/// Whether a category wants the solids to meet at places of these classes.
type Wanted = fn(Class, Class) -> bool;

/// The classes of place two kinds can meet in, for a category.
fn class_pairs(ka: Kind, kb: Kind, wanted: fn(Class, Class) -> bool) -> Vec<(Class, Class)> {
    let mut pairs = Vec::new();
    for &ca in Solid::classes(ka) {
        for &cb in Solid::classes(kb) {
            if wanted(ca, cb) {
                pairs.push((ca, cb));
            }
        }
    }
    pairs
}

fn is_flush(a: Class, b: Class) -> bool {
    a == Class::Face && b == Class::Face
}

fn is_tangent(a: Class, b: Class) -> bool {
    !a.sharp() && !b.sharp() && !is_flush(a, b)
}

fn is_sharp(a: Class, b: Class) -> bool {
    a.sharp() || b.sharp()
}

/// Two solids against each other at a place of each.
struct Contact {
    a: Placed,
    solid: Solid,
    fa: Place,
    fb: Place,
    same_side: bool,
    spin: Spin,
    variant: String,
}

impl Contact {
    fn b(&self, gap: f64) -> Placed {
        against(
            &self.a,
            self.fa,
            self.solid,
            self.fb,
            self.same_side,
            self.spin,
            gap,
        )
    }
}

fn contact(
    rng: &mut Rng,
    (ka, kb): (Kind, Kind),
    (ca, cb): (Class, Class),
    same_side: bool,
    round: bool,
    turn: u64,
) -> Option<Contact> {
    let (o, r) = rng.frame(round);
    let a = Placed {
        solid: rng.solid(ka, round),
        o,
        r,
    };
    let solid = rng.solid(kb, round);
    let fa = a.solid.place(ca, rng.pick(round))?;
    let fb = solid.place(cb, rng.pick(round))?;
    let spin = rng.spin(round, turn);
    let side = if same_side { "same-side" } else { "against" };
    let spin_name = match spin {
        Spin::Aligned => "aligned",
        Spin::Quarter => "quarter",
        Spin::Angle(_) => "turned",
    };
    Some(Contact {
        a,
        solid,
        fa,
        fb,
        same_side,
        spin,
        variant: format!("{}-{}-{side}-{spin_name}", ca.name(), cb.name()),
    })
}

/// Cases where the solids meet at a place of each, cycling through the
/// class pairs `wanted` allows; both orders of the kinds.
fn contacts(
    g: &mut Group,
    k1: Kind,
    k2: Kind,
    n: usize,
    wanted: fn(Class, Class) -> bool,
    both_sides: bool,
) {
    let orders = [(k1, k2), (k2, k1)];
    let mut combos = Vec::new();
    for (order, &(ka, kb)) in orders.iter().enumerate() {
        if order == 1 && k1 == k2 {
            break;
        }
        for classes in class_pairs(ka, kb, wanted) {
            combos.push(((ka, kb), classes));
        }
    }
    if combos.is_empty() {
        return;
    }
    for i in 0..n {
        let Some(&(kinds, classes)) = combos.get(i % combos.len()) else {
            return;
        };
        let mut rng = g.rng();
        let lap = (i / combos.len()) as u64;
        let same_side = both_sides && lap % 2 == 1;
        let round = (i + i / combos.len()) % 2 == 0;
        let Some(c) = contact(
            &mut rng,
            kinds,
            classes,
            same_side,
            round,
            lap / 2 + i as u64,
        ) else {
            continue;
        };
        let expect = (!same_side).then_some(Expect::Disjoint);
        g.push(c.variant.clone(), expect, c.a, c.b(0.0));
    }
}

/// A pair described in a frame of its own: each solid with the origin and
/// axes of its frame there. `expect` is about (first, second).
struct Special {
    variant: &'static str,
    first: (Solid, V, Rot),
    second: (Solid, V, Rot),
    expect: Option<Expect>,
}

fn special(
    variant: &'static str,
    first: Solid,
    second: Solid,
    at: V,
    turn: Rot,
    expect: Option<Expect>,
) -> Option<Special> {
    Some(Special {
        variant,
        first: (first, V::ZERO, Rot::IDENTITY),
        second: (second, at, turn),
        expect,
    })
}

fn block(x: f64, y: f64, z: f64) -> Solid {
    Solid::Block { size: V(x, y, z) }
}

fn cylinder(radius: f64, height: f64) -> Solid {
    Solid::Cylinder { radius, height }
}

fn cone(radius: f64, height: f64) -> Solid {
    Solid::Cone { radius, height }
}

fn sphere(radius: f64) -> Solid {
    Solid::Sphere { radius }
}

fn torus(major: f64, minor: f64) -> Solid {
    Solid::Torus { major, minor }
}

/// The hand-made configurations of a category for a kind pair (`k1` not
/// after `k2`): number `j`, or `None` after the last one.
fn special_case(
    category: &str,
    k1: Kind,
    k2: Kind,
    j: usize,
    rng: &mut Rng,
    round: bool,
) -> Option<Special> {
    use Expect::{Apart, BInA, Disjoint, Same};
    use Kind::{Block, Cone, Cylinder, Sphere, Torus};
    let id = Rot::IDENTITY;
    let mut len = |lo: f64, hi: f64| rng.length(lo, hi, round);
    match (category, k1, k2, j) {
        // ---- flush: faces in one plane, on one cylinder, on one cone ----
        ("flush", Block, Block, 0) => {
            let (x, y, z) = (len(1.0, 3.0), len(1.0, 3.0), len(1.0, 3.0));
            let b = block(x * 0.5, y * 0.25, z * 0.75);
            special("shared-corner", block(x, y, z), b, V::ZERO, id, Some(BInA))
        }
        ("flush", Block, Block, 1) => {
            let (x, y, z) = (len(1.0, 3.0), len(1.0, 3.0), len(1.0, 3.0));
            let b = block(x, y, len(0.5, 2.0));
            special(
                "stacked-same-footprint",
                block(x, y, z),
                b,
                V(0.0, 0.0, z),
                id,
                Some(Disjoint),
            )
        }
        ("flush", Block, Block, 2) => {
            let (x, y, z) = (len(1.0, 3.0), len(1.0, 3.0), len(1.0, 3.0));
            let b = block(x * 0.5, y, z * 0.5);
            special(
                "notch-flush-with-three-faces",
                block(x, y, z),
                b,
                V(x * 0.25, 0.0, z * 0.5),
                id,
                Some(BInA),
            )
        }
        ("flush", Block, Block, 3) => {
            let (x, y, z) = (len(1.0, 3.0), len(1.0, 3.0), len(1.0, 3.0));
            special(
                "identical",
                block(x, y, z),
                block(x, y, z),
                V::ZERO,
                id,
                Some(Same),
            )
        }
        ("flush", Block, Cylinder, 0) => {
            let (x, y, z) = (len(2.0, 3.0), len(2.0, 3.0), len(0.5, 2.0));
            let r = len(0.25, 0.75);
            special(
                "cutter-flush-with-both-faces",
                block(x, y, z),
                cylinder(r, z),
                V(x * 0.5, y * 0.5, 0.0),
                id,
                Some(BInA),
            )
        }
        ("flush", Block, Cone, 0) => {
            let (x, y, z) = (len(2.0, 3.0), len(2.0, 3.0), len(1.0, 2.0));
            let r = len(0.25, 0.75);
            special(
                "cone-on-the-floor-inside",
                block(x, y, z),
                cone(r, z * 0.5),
                V(x * 0.5, y * 0.5, 0.0),
                id,
                Some(BInA),
            )
        }
        ("flush", Block, Cone, 1) => {
            let (x, y, z) = (len(2.0, 3.0), len(2.0, 3.0), len(1.0, 2.0));
            let r = len(0.25, 0.75);
            special(
                "cone-floor-to-ceiling",
                block(x, y, z),
                cone(r, z),
                V(x * 0.5, y * 0.5, 0.0),
                id,
                Some(BInA),
            )
        }
        ("flush", Cylinder, Cylinder, 0) => {
            let (r, h) = (len(0.5, 1.5), len(1.0, 2.0));
            special(
                "equal-radius-overlap",
                cylinder(r, h),
                cylinder(r, h),
                V(0.0, 0.0, h * 0.5),
                id,
                None,
            )
        }
        ("flush", Cylinder, Cylinder, 1) => {
            let (r, h) = (len(0.5, 1.5), len(1.0, 2.0));
            special(
                "equal-radius-stacked",
                cylinder(r, h),
                cylinder(r, len(0.5, 2.0)),
                V(0.0, 0.0, h),
                id,
                Some(Disjoint),
            )
        }
        ("flush", Cylinder, Cylinder, 2) => {
            let (r, h) = (len(0.5, 1.5), len(1.0, 2.0));
            special(
                "equal-radius-contained",
                cylinder(r, h),
                cylinder(r, h * 0.5),
                V(0.0, 0.0, h * 0.25),
                id,
                Some(BInA),
            )
        }
        ("flush", Cylinder, Cylinder, 3) => {
            let (r, h) = (len(0.5, 1.5), len(1.0, 2.0));
            special(
                "equal-radius-same-floor",
                cylinder(r, h),
                cylinder(r, h * 0.5),
                V::ZERO,
                id,
                Some(BInA),
            )
        }
        ("flush", Cylinder, Cylinder, 4) => {
            let (r, h) = (len(0.5, 1.5), len(1.0, 2.0));
            let turn = if round { id } else { Rot::about_z(1.0) };
            special(
                "identical",
                cylinder(r, h),
                cylinder(r, h),
                V::ZERO,
                turn,
                Some(Same),
            )
        }
        ("flush", Cylinder, Cone, 0) => {
            let (r, h) = (len(0.5, 1.5), len(1.0, 2.0));
            special(
                "cone-on-cylinder-equal-radius",
                cylinder(r, h),
                cone(r, len(0.5, 2.0)),
                V(0.0, 0.0, h),
                id,
                Some(Disjoint),
            )
        }
        ("flush", Cylinder, Cone, 1) => {
            let (r, h) = (len(0.5, 1.5), len(1.0, 2.0));
            special(
                "cone-in-cylinder-same-floor",
                cylinder(r, h),
                cone(r, h * 0.5),
                V::ZERO,
                id,
                Some(BInA),
            )
        }
        ("flush", Cylinder, Cone, 2) => {
            let (r, h) = (len(0.5, 1.5), len(1.0, 2.0));
            special(
                "cone-through-cylinder-same-floor",
                cylinder(r, h),
                cone(r, h * 2.0),
                V::ZERO,
                id,
                None,
            )
        }
        ("flush", Cylinder, Cone, 3) => {
            let (r, h) = (len(0.5, 1.5), len(1.0, 2.0));
            special(
                "cone-hanging-from-the-lid",
                cylinder(r, h),
                cone(r, h * 0.5),
                V(0.0, 0.0, h),
                Rot::FLIP,
                Some(BInA),
            )
        }
        ("flush", Cone, Cone, 0) => {
            let (r, h) = (len(0.5, 1.5), len(1.0, 2.0));
            special(
                "base-to-base",
                cone(r, h),
                cone(r, len(0.5, 2.0)),
                V::ZERO,
                Rot::FLIP,
                Some(Disjoint),
            )
        }
        ("flush", Cone, Cone, 1) => {
            let (r, h) = (len(0.5, 1.5), len(1.0, 2.0));
            special(
                "nested-same-base",
                cone(r, h),
                cone(r, h * 0.5),
                V::ZERO,
                id,
                Some(BInA),
            )
        }
        ("flush", Cone, Cone, 2) => {
            let (r, h) = (len(0.5, 1.5), len(1.0, 2.0));
            special(
                "same-conical-surface",
                cone(r, h),
                cone(r * 0.5, h * 0.5),
                V(0.0, 0.0, h * 0.5),
                id,
                Some(BInA),
            )
        }
        ("flush", Cone, Cone, 3) => {
            let (r, h) = (len(0.5, 1.5), len(1.0, 2.0));
            let turn = if round { id } else { Rot::about_z(1.0) };
            special(
                "identical",
                cone(r, h),
                cone(r, h),
                V::ZERO,
                turn,
                Some(Same),
            )
        }
        ("flush", Sphere, Sphere, 0) => {
            let r = len(0.5, 1.5);
            let turn = if round { id } else { rng.rotation() };
            special("identical", sphere(r), sphere(r), V::ZERO, turn, Some(Same))
        }
        ("flush", Torus, Torus, 0) => {
            let (major, minor) = (len(1.5, 2.5), len(0.25, 0.75));
            let turn = if round { id } else { Rot::about_z(1.0) };
            special(
                "identical",
                torus(major, minor),
                torus(major, minor),
                V::ZERO,
                turn,
                Some(Same),
            )
        }

        // ---- tangent: curved faces that touch without crossing ----
        ("tangent", Block, Cylinder, 0) => {
            let (r, z) = (len(0.5, 1.5), len(2.0, 3.0));
            special(
                "inscribed",
                block(2.0 * r, 2.0 * r, z),
                cylinder(r, z * 0.5),
                V(r, r, z * 0.25),
                id,
                Some(BInA),
            )
        }
        ("tangent", Block, Cylinder, 1) => {
            let (r, z) = (len(0.5, 1.5), len(1.0, 2.0));
            special(
                "inscribed-through",
                block(2.0 * r, 2.0 * r, z),
                cylinder(r, z * 2.0),
                V(r, r, -z * 0.5),
                id,
                None,
            )
        }
        ("tangent", Block, Sphere, 0) => {
            let r = len(0.5, 1.5);
            special(
                "inscribed",
                block(2.0 * r, 2.0 * r, 2.0 * r),
                sphere(r),
                V(r, r, r),
                id,
                Some(BInA),
            )
        }
        ("tangent", Block, Sphere, 1) => {
            let r = len(0.5, 1.0);
            special(
                "between-two-faces",
                block(4.0 * r, 2.0 * r, 3.0 * r),
                sphere(r),
                V(2.0 * r, r, 1.5 * r),
                id,
                Some(BInA),
            )
        }
        ("tangent", Block, Torus, 0) => {
            let (major, minor) = (len(1.0, 2.0), len(0.25, 0.5));
            let side = 2.0 * (major + minor);
            special(
                "ring-in-a-box",
                block(side, side, 2.0 * minor),
                torus(major, minor),
                V(major + minor, major + minor, minor),
                id,
                Some(BInA),
            )
        }
        ("tangent", Cylinder, Cylinder, 0) => {
            let (r, h) = (len(1.0, 1.5), len(2.0, 3.0));
            let small = r * 0.5;
            special(
                "inside-along-a-line",
                cylinder(r, h),
                cylinder(small, h * 0.5),
                V(r - small, 0.0, h * 0.25),
                id,
                Some(BInA),
            )
        }
        ("tangent", Cylinder, Cylinder, 1) => {
            let (r, h) = (len(1.0, 1.5), len(1.0, 2.0));
            let small = r * 0.5;
            special(
                "inside-along-a-line-through",
                cylinder(r, h),
                cylinder(small, h * 2.0),
                V(r - small, 0.0, -h * 0.5),
                id,
                None,
            )
        }
        ("tangent", Cylinder, Sphere, 0) => {
            let r = len(0.5, 1.0);
            special(
                "ball-in-a-hole-of-its-radius",
                cylinder(r, 4.0 * r),
                sphere(r),
                V(0.0, 0.0, 2.0 * r),
                id,
                Some(BInA),
            )
        }
        ("tangent", Cylinder, Sphere, 1) => {
            let r = len(0.5, 1.0);
            special(
                "ball-half-out-of-its-hole",
                cylinder(r, 4.0 * r),
                sphere(r),
                V(0.0, 0.0, 4.0 * r),
                id,
                None,
            )
        }
        ("tangent", Cylinder, Sphere, 2) => {
            let r = len(0.5, 1.0);
            special(
                "ball-on-the-floor-of-its-hole",
                cylinder(r, 4.0 * r),
                sphere(r),
                V(0.0, 0.0, r),
                id,
                Some(BInA),
            )
        }
        ("tangent", Cylinder, Torus, 0) => {
            let (major, minor) = (len(1.5, 2.5), len(0.25, 0.75));
            special(
                "plug-in-the-hole",
                cylinder(major - minor, 4.0 * minor),
                torus(major, minor),
                V(0.0, 0.0, 2.0 * minor),
                id,
                Some(Disjoint),
            )
        }
        ("tangent", Cylinder, Torus, 1) => {
            let (major, minor) = (len(1.5, 2.5), len(0.25, 0.75));
            special(
                "ring-in-a-sleeve",
                cylinder(major + minor, 4.0 * minor),
                torus(major, minor),
                V(0.0, 0.0, 2.0 * minor),
                id,
                Some(Expect::BInA),
            )
        }
        ("tangent", Cone, Sphere, 0) => {
            let (r, h) = (len(0.75, 1.5), len(1.0, 2.5));
            let rho = r * h / (r + r.hypot(h));
            special(
                "inscribed",
                cone(r, h),
                sphere(rho),
                V(0.0, 0.0, rho),
                id,
                Some(BInA),
            )
        }
        ("tangent", Cone, Sphere, 1) => {
            let (r, h) = (len(0.75, 1.5), len(1.0, 2.5));
            let rho = 0.5 * r * h / (r + r.hypot(h));
            // On the axis, `rho` from the side: `rho / sin(half angle)`
            // below the apex.
            let at = h - rho * r.hypot(h) / r;
            special(
                "ball-against-the-side",
                cone(r, h),
                sphere(rho),
                V(0.0, 0.0, at),
                id,
                Some(BInA),
            )
        }
        ("tangent", Cone, Torus, 0) => {
            let (r, h) = (len(1.0, 1.5), len(1.5, 2.5));
            let minor = len(0.25, 0.5);
            let slant = r.hypot(h);
            // The tube touches the side halfway up, from outside.
            let major = 0.5 * r + minor * h / slant;
            special(
                "ring-around-a-cone",
                cone(r, h),
                torus(major, minor),
                V(0.0, 0.0, 0.5 * h + minor * r / slant),
                id,
                Some(Disjoint),
            )
        }
        ("tangent", Sphere, Sphere, 0) => {
            let r = len(1.0, 1.5);
            let small = r * 0.5;
            let towards = if round { V::X } else { rng.direction() };
            special(
                "inside",
                sphere(r),
                sphere(small),
                towards * (r - small),
                id,
                Some(BInA),
            )
        }
        ("tangent", Sphere, Torus, 0) => {
            let (major, minor) = (len(1.5, 2.5), len(0.25, 0.75));
            special(
                "ball-in-the-tube",
                torus(major, minor),
                sphere(minor),
                V(major, 0.0, 0.0),
                id,
                Some(BInA),
            )
        }
        ("tangent", Sphere, Torus, 1) => {
            let (major, minor) = (len(1.5, 2.5), len(0.25, 0.75));
            let up = len(0.5, 1.5);
            special(
                "ball-on-a-ring",
                torus(major, minor),
                sphere(major.hypot(up) - minor),
                V(0.0, 0.0, up),
                id,
                Some(Disjoint),
            )
        }
        ("tangent", Sphere, Torus, 2) => {
            let (major, minor) = (len(1.5, 2.5), len(0.25, 0.75));
            special(
                "ball-filling-the-hole",
                torus(major, minor),
                sphere(major - minor),
                V::ZERO,
                id,
                Some(Disjoint),
            )
        }
        ("tangent", Torus, Torus, 0) => {
            let (major, minor) = (len(1.5, 2.5), len(0.25, 0.75));
            let other = len(0.25, 0.75);
            special(
                "stacked",
                torus(major, minor),
                torus(major, other),
                V(0.0, 0.0, minor + other),
                id,
                Some(Disjoint),
            )
        }
        ("tangent", Torus, Torus, 1) => {
            let (major, minor) = (len(1.5, 2.5), len(0.25, 0.5));
            let other = len(0.25, 0.5);
            special(
                "one-around-the-other",
                torus(major, minor),
                torus(major + minor + other, other),
                V::ZERO,
                id,
                Some(Disjoint),
            )
        }
        ("tangent", Torus, Torus, 2) => {
            let (major, minor) = (len(1.5, 2.5), len(0.5, 0.75));
            let other = minor * 0.5;
            special(
                "tube-in-tube",
                torus(major, minor),
                torus(major + minor - other, other),
                V::ZERO,
                id,
                Some(BInA),
            )
        }

        // ---- inside ----
        ("inside", Torus, Torus, 0) => {
            let (major, minor) = (len(1.5, 2.5), len(0.5, 0.75));
            special(
                "tube-in-tube",
                torus(major, minor),
                torus(major, minor * 0.5),
                V::ZERO,
                id,
                Some(BInA),
            )
        }
        ("inside", Cylinder, Cylinder, 0) => {
            let (r, h) = (len(1.0, 1.5), len(2.0, 3.0));
            special(
                "coaxial",
                cylinder(r, h),
                cylinder(r * 0.5, h * 0.5),
                V(0.0, 0.0, h * 0.25),
                id,
                Some(BInA),
            )
        }
        ("inside", Sphere, Sphere, 0) => {
            let r = len(1.0, 1.5);
            special(
                "concentric",
                sphere(r),
                sphere(r * 0.5),
                V::ZERO,
                id,
                Some(BInA),
            )
        }

        // ---- apart, yet inside each other's box ----
        ("apart", Block, Torus, 0) => {
            let (major, minor) = (len(1.5, 2.5), len(0.25, 0.5));
            let side = 0.5 * (major - minor);
            special(
                "bar-through-the-hole",
                block(side, side, 3.0),
                torus(major, minor),
                V(0.5 * side, 0.5 * side, 1.5),
                id,
                Some(Apart),
            )
        }
        ("apart", Cylinder, Torus, 0) => {
            let (major, minor) = (len(1.5, 2.5), len(0.25, 0.5));
            special(
                "rod-through-the-hole",
                cylinder(0.5 * (major - minor), 3.0),
                torus(major, minor),
                V(0.0, 0.0, 1.5),
                id,
                Some(Apart),
            )
        }
        ("apart", Cone, Torus, 0) => {
            let (major, minor) = (len(1.5, 2.5), len(0.25, 0.5));
            special(
                "cone-through-the-hole",
                cone(0.5 * (major - minor), 3.0),
                torus(major, minor),
                V(0.0, 0.0, 1.5),
                id,
                Some(Apart),
            )
        }
        ("apart", Sphere, Torus, 0) => {
            let (major, minor) = (len(1.5, 2.5), len(0.25, 0.5));
            special(
                "ball-in-the-hole",
                torus(major, minor),
                sphere(0.5 * (major - minor)),
                V::ZERO,
                id,
                Some(Apart),
            )
        }
        ("apart", Torus, Torus, 0) => {
            let (major, minor) = (len(1.5, 2.5), len(0.25, 0.5));
            let other = len(0.25, 0.5);
            special(
                "linked",
                torus(major, minor),
                torus(minor + other + 0.5, other),
                V(major, 0.0, 0.0),
                Rot::ROLL,
                Some(Apart),
            )
        }
        ("apart", Sphere, Sphere, 0) => {
            // Two balls whose boxes overlap.
            let r = len(0.5, 1.5);
            let d = (2.0 * r + 0.25) / 3.0_f64.sqrt();
            special(
                "boxes-overlap",
                sphere(r),
                sphere(r),
                V(d, d, d),
                id,
                Some(Apart),
            )
        }
        _ => None,
    }
}

/// The hand-made configurations of a category, `copies` of each with
/// other sizes, in a frame of the world or anywhere, in both orders.
fn specials(g: &mut Group, k1: Kind, k2: Kind, copies: usize) {
    for j in 0.. {
        for copy in 0..copies {
            let mut rng = g.rng();
            let round = g.index().is_multiple_of(2);
            let Some(s) = special_case(g.category, k1, k2, j, &mut rng, round) else {
                return;
            };
            let (o, r) = rng.frame(round);
            let place = |(solid, at, turn): (Solid, V, Rot)| Placed {
                solid,
                o: o + r.apply(at),
                r: r.after(&turn),
            };
            let (first, second) = (place(s.first), place(s.second));
            // The kinds of a special are not always in the order of the
            // pair's name; the order of the operands alternates anyway.
            if copy % 2 == 0 {
                g.push(s.variant.to_string(), s.expect, first, second);
            } else {
                let expect = s.expect.map(Expect::swapped);
                g.push(s.variant.to_string(), expect, second, first);
            }
        }
    }
}

/// One wholly inside the other, touching nothing.
fn inside(g: &mut Group, k1: Kind, k2: Kind, n: usize) {
    for i in 0..n {
        let mut rng = g.rng();
        // (outer, inner), both ways round.
        let (ko, ki) = ordered(k1, k2, i % 2 == 1);
        let (o, r) = rng.frame(i % 3 == 0);
        let outer = Placed {
            solid: rng.solid(ko, false),
            o,
            r,
        };
        let (centre, room) = outer.solid.inner_ball(rng.unit());
        let raw = rng.solid(ki, false);
        let fill = rng.between(0.3, 0.9);
        let solid = raw.scaled(fill * room / raw.outer_ball().1);
        let turn = if i % 3 == 0 { r } else { rng.rotation() };
        let slack = 0.9 * (1.0 - fill) * room;
        let target = outer.world(centre) + rng.direction() * (slack * rng.unit());
        let inner = Placed {
            solid,
            o: target - turn.apply(solid.outer_ball().0),
            r: turn,
        };
        // Which of the two is the first operand alternates as well.
        if i % 4 < 2 {
            g.push("b-in-a".to_string(), Some(Expect::BInA), outer, inner);
        } else {
            g.push("a-in-b".to_string(), Some(Expect::AInB), inner, outer);
        }
    }
}

fn apart(g: &mut Group, k1: Kind, k2: Kind, n: usize) {
    for i in 0..n {
        let mut rng = g.rng();
        let (ka, kb) = ordered(k1, k2, i % 2 == 1);
        if i % 2 == 0 {
            // Far: the balls around them do not meet.
            let (o, r) = rng.frame(i % 4 == 0);
            let a = Placed {
                solid: rng.solid(ka, false),
                o,
                r,
            };
            let solid = rng.solid(kb, false);
            let turn = if i % 4 == 0 { r } else { rng.rotation() };
            let (ca, ra) = a.solid.outer_ball();
            let (cb, rb) = solid.outer_ball();
            let target = a.world(ca) + rng.direction() * (rng.between(1.05, 2.0) * (ra + rb));
            let b = Placed {
                solid,
                o: target - turn.apply(cb),
                r: turn,
            };
            g.push("far".to_string(), Some(Expect::Apart), a, b);
        } else {
            // Close: a contact pulled apart, so the boxes mostly overlap.
            let all = class_pairs(ka, kb, |_, _| true);
            let Some(&classes) = all.get((i / 2) % all.len().max(1)) else {
                continue;
            };
            let Some(c) = contact(&mut rng, (ka, kb), classes, false, false, i as u64) else {
                continue;
            };
            let gap = rng.between(0.02, 0.5);
            g.push(
                format!("close-{}", c.variant),
                Some(Expect::Apart),
                c.a,
                c.b(gap),
            );
        }
    }
}

/// Contacts that are off by 10, 1 and 0.1 tolerances, either way, each
/// next to the exact contact it was made from.
fn near(g: &mut Group, k1: Kind, k2: Kind, bases: usize) {
    // One of each kind of contact the pair has, in turn.
    let mut kinds: Vec<(Kinds, Classes, bool)> = Vec::new();
    let tests: [(Wanted, bool); 4] = [
        (is_flush, false),
        (is_tangent, false),
        (is_sharp, false),
        (is_flush, true),
    ];
    for lap in 0..3 {
        for (wanted, same_side) in tests {
            let order = if lap % 2 == 0 { (k1, k2) } else { (k2, k1) };
            let pairs = class_pairs(order.0, order.1, wanted);
            if let Some(&classes) = pairs.get((lap * 2) % pairs.len().max(1)) {
                kinds.push((order, classes, same_side));
            }
        }
    }
    for base in 0..bases {
        let Some(&(order, classes, same_side)) = kinds.get(base % kinds.len().max(1)) else {
            return;
        };
        let mut rng = g.rng();
        let round = base % 2 == 0;
        let Some(c) = contact(&mut rng, order, classes, same_side, round, base as u64) else {
            continue;
        };
        let touching = (!same_side).then_some(Expect::Disjoint);
        let exact = g.push_near(
            format!("exact-{}", c.variant),
            touching,
            c.a,
            c.b(0.0),
            Some(0.0),
            None,
        );
        for tolerances in [10.0_f64, -10.0, 1.0, -1.0, 0.1, -0.1] {
            // Beyond the tolerance the answer is that of the gap; within
            // it, that of the exact contact is as good.
            let expect = match (same_side, tolerances) {
                (false, t) if t > 1.0 => Some(Expect::Apart),
                (false, t) if t > 0.0 => Some(Expect::Disjoint),
                _ => None,
            };
            let snap = (tolerances.abs() <= 1.0).then(|| exact.clone());
            g.push_near(
                format!("off-{}", c.variant),
                expect,
                c.a,
                c.b(tolerances * TOLERANCE),
                Some(tolerances),
                snap,
            );
        }
    }
}

/// Every case, in the order of the file.
pub fn cases(seed: u64) -> Vec<Case> {
    type Maker = fn(&mut Group, Kind, Kind);
    let categories: [(&'static str, Maker); 9] = [
        ("general", |g, a, b| general(g, a, b, 20)),
        ("parallel", |g, a, b| parallel(g, a, b, 16)),
        ("coaxial", |g, a, b| coaxial(g, a, b, 16)),
        ("flush", |g, a, b| {
            contacts(g, a, b, 24, is_flush, true);
            specials(g, a, b, 4);
        }),
        ("tangent", |g, a, b| {
            contacts(g, a, b, 10, is_tangent, false);
            specials(g, a, b, 4);
        }),
        ("contact", |g, a, b| contacts(g, a, b, 16, is_sharp, false)),
        ("inside", |g, a, b| {
            inside(g, a, b, 12);
            specials(g, a, b, 2);
        }),
        ("apart", |g, a, b| {
            apart(g, a, b, 12);
            specials(g, a, b, 2);
        }),
        ("near", |g, a, b| near(g, a, b, 3)),
    ];
    let mut out = Vec::new();
    for (category, make) in categories {
        for (i, &k1) in KINDS.iter().enumerate() {
            for &k2 in KINDS.iter().skip(i) {
                let mut group = Group {
                    seed,
                    category,
                    pair: pair_name(k1, k2),
                    out: &mut out,
                    count: 0,
                };
                make(&mut group, k1, k2);
            }
        }
    }
    out
}
