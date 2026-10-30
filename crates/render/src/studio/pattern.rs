//! Layer 3.5: procedural solid textures (docs/design/rendering.md, *Layer
//! 3.5: solid textures*). A [`Pattern`] is a function of the point in the
//! model's space, with no mapping of the surface: B-rep faces have no
//! tidy parametrisation, and a cut through a body shows the pattern
//! running on, as in a real material. It gives a face's base colour (a
//! ramp in linear light) and, optionally, its roughness at each point.
//!
//! Sources (clean-room, from the literature):
//!
//! - K. Perlin, *An Image Synthesizer*, SIGGRAPH 1985 (solid textures,
//!   turbulence, marble as `sin(x + turbulence)`), and *Improving Noise*,
//!   SIGGRAPH 2002 (the quintic fade `6t⁵ − 15t⁴ + 10t³` and the twelve
//!   edge gradients chosen by a hash's low four bits).
//! - S. Worley, *A Cellular Texture Basis Function*, SIGGRAPH 1996 (the
//!   nearest of one random feature point per cell, over the 27 cells
//!   around a point).
//! - D. Ebert, F. K. Musgrave, D. Peachey, K. Perlin, S. Worley,
//!   *Texturing & Modeling: A Procedural Approach*, 3rd ed., 2003: fBm
//!   (octaves of lacunarity 2 and gain ½), wood as rings about an axis
//!   wobbled by noise, early and late wood.
//! - M. Jarzynski and M. Olano, *Hash Functions for GPU Rendering*, JCGT
//!   2020: `pcg3d`, a hash of three `u32` made only of wrapping integer
//!   products, sums, xors and shifts, so the CPU and every GPU compute the
//!   same bits. No driver's noise or hash is used.
//!
//! The CPU evaluates in `f64` and is the definition; the GPU's
//! `studio.wgsl` repeats it in `f32` from the interpolated position. Where
//! the pattern is steep (a ring's edge, a cell's border, a tow's border)
//! the two may decide differently; [`Pattern::in_doubt`] marks those
//! samples.

use super::tonemap;
use craft3d_math::Vec3;

/// The most colours a ramp has.
pub const MAX_STOPS: usize = 4;
/// The most octaves of fBm.
pub const MAX_DETAIL: u32 = 6;
/// The pattern's local coordinates (in units of its scale) are clamped to
/// this, so that the lattice's cells fit an `i32` on both renderers. In
/// `f32` the coordinates resolve 1/8 of a feature there; the pattern is
/// sharp on the GPU to about 10⁴ features from its origin.
pub const Q_LIMIT: f64 = 1_048_576.0;
/// The smallest and largest scale a pattern keeps, in model units.
pub const MIN_SCALE: f32 = 1e-6;
pub const MAX_SCALE: f32 = 1e9;
/// How far, in `f32` epsilons of the magnitudes the GPU computes the point
/// from, the point may move before a sample is in doubt
/// ([`Pattern::in_doubt`]). Provisional; measured against the GPU in
/// `crates/gpu/tests/layers.rs` (docs/design/rendering.md, *Layer 3.5*).
pub const PATTERN_DOUBT: f64 = 8.0;
/// How far, in samples, the GPU's point may lie from the CPU's because its
/// rasteriser snaps vertices to a sub-pixel grid (8 bits on the adapters
/// measured; Vulkan requires at least 4): a 256th of a sample.
pub const SUBPIXEL_DOUBT: f64 = 1.0 / 256.0;

/// The kind of solid texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PatternKind {
    /// Concentric rings about the axis, wobbled by noise; early and late
    /// wood, fine grain along the axis, medullary rays.
    Wood,
    /// Veins from turbulence across the axis.
    Marble,
    /// Cellular noise: each cell one of the ramp's colours.
    Granite,
    /// Cellular noise: a dot of the ramp's last colour in each cell.
    Speckle,
    /// fBm: cast iron, stone, a variation of roughness.
    Noise,
    /// Streaks along the axis.
    Brushed,
    /// A 2×2 twill weave, around the axis or across it.
    Carbon,
}

impl PatternKind {
    /// Every kind, in order of [`Self::code`].
    pub const ALL: [PatternKind; 7] = [
        PatternKind::Wood,
        PatternKind::Marble,
        PatternKind::Granite,
        PatternKind::Speckle,
        PatternKind::Noise,
        PatternKind::Brushed,
        PatternKind::Carbon,
    ];

    /// The number the GPU's material table holds (1 to 7; 0 is none).
    pub fn code(self) -> u32 {
        match self {
            PatternKind::Wood => 1,
            PatternKind::Marble => 2,
            PatternKind::Granite => 3,
            PatternKind::Speckle => 4,
            PatternKind::Noise => 5,
            PatternKind::Brushed => 6,
            PatternKind::Carbon => 7,
        }
    }

    /// Its name in commands and files.
    pub fn name(self) -> &'static str {
        match self {
            PatternKind::Wood => "wood",
            PatternKind::Marble => "marble",
            PatternKind::Granite => "granite",
            PatternKind::Speckle => "speckle",
            PatternKind::Noise => "noise",
            PatternKind::Brushed => "brushed",
            PatternKind::Carbon => "carbon",
        }
    }

    /// The kind of a name.
    pub fn from_name(name: &str) -> Option<PatternKind> {
        PatternKind::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// A solid texture as both renderers evaluate it, in `f32` where the GPU
/// reads the same numbers. [`Pattern::sanitized`] brings any value into
/// range.
#[derive(Clone, Copy, Debug)]
pub struct Pattern {
    pub kind: PatternKind,
    /// The ramp's colours (sRGB), the first `stops` of them, spaced evenly
    /// from 0 to 1 and blended in linear light.
    pub ramp: [[u8; 3]; MAX_STOPS],
    pub stops: u8,
    /// The roughness at the ramp's two ends; `None` keeps the face's own.
    pub roughness: Option<[f32; 2]>,
    /// The size of one feature in model units: the ring spacing of wood,
    /// the veins' of marble, a cell of granite, a tow's width of carbon.
    pub scale: f32,
    /// The pattern's origin in the model's space (the pith of wood).
    pub origin: [f64; 3],
    /// Its axis (wood's pith, marble's layering, the brushing and the
    /// weave's direction); any length, made unit by [`Self::frame`].
    pub axis: [f32; 3],
    pub seed: u32,
    /// Wood's ring wobble and marble's turbulence, in features; noise's
    /// extra contrast.
    pub distortion: f32,
    /// Octaves of fBm, 1 to [`MAX_DETAIL`].
    pub detail: u32,
    /// The strength of fine streaks (wood's grain, brushing's scratches),
    /// 0 to 1.
    pub grain: f32,
    /// Wood's medullary rays, 0 to 1.
    pub rays: f32,
    /// Speckle's dot radius in cells, 0 to 1.
    pub spot: f32,
    /// Carbon: the weave wraps around the axis (a tube), else it lies in
    /// the plane across the axis (a plate whose normal is the axis).
    pub wrap: bool,
}

impl Default for Pattern {
    fn default() -> Self {
        Pattern {
            kind: PatternKind::Noise,
            ramp: [[0; 3], [255; 3], [255; 3], [255; 3]],
            stops: 2,
            roughness: None,
            scale: 1.0,
            origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            seed: 0,
            distortion: 0.0,
            detail: 4,
            grain: 0.0,
            rays: 0.0,
            spot: 0.5,
            wrap: true,
        }
    }
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        let bits = |p: &Pattern| {
            (
                p.kind,
                p.ramp,
                p.stops,
                p.roughness.map(|r| r.map(f32::to_bits)),
                p.scale.to_bits(),
                p.origin.map(f64::to_bits),
                p.axis.map(f32::to_bits),
                p.seed,
                [p.distortion, p.grain, p.rays, p.spot].map(f32::to_bits),
                p.detail,
                p.wrap,
            )
        };
        bits(self) == bits(other)
    }
}

impl Eq for Pattern {}

fn within(v: f32, lo: f32, hi: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        fallback
    }
}

/// `smoothstep(a, b, x)`: 0 below `a`, 1 above `b`, the cubic between.
pub fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Jarzynski and Olano's `pcg3d`.
pub fn pcg3d(v: [u32; 3]) -> [u32; 3] {
    let [x, y, z] = v.map(|c| c.wrapping_mul(1_664_525).wrapping_add(1_013_904_223));
    let x = x.wrapping_add(y.wrapping_mul(z));
    let y = y.wrapping_add(z.wrapping_mul(x));
    let z = z.wrapping_add(x.wrapping_mul(y));
    let [x, y, z] = [x ^ (x >> 16), y ^ (y >> 16), z ^ (z >> 16)];
    let x = x.wrapping_add(y.wrapping_mul(z));
    let y = y.wrapping_add(z.wrapping_mul(x));
    let z = z.wrapping_add(x.wrapping_mul(y));
    [x, y, z]
}

/// The lattice offset of `seed`'s octave `octave`: the field of another
/// seed or octave is the same field far away (the hash has no period the
/// lattice reaches).
pub fn offset(seed: u32, octave: u32) -> [u32; 3] {
    pcg3d([seed, octave, 0x2545_f491])
}

/// A hash's top 24 bits as a number in `[0, 1)`, exact in `f32`.
fn unit(h: u32) -> f64 {
    f64::from(h >> 8) * (1.0 / 16_777_216.0)
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + t * (b - a)
}

/// Perlin's gradient for the hash `h` at the offset `(x, y, z)`: one of
/// twelve edge directions (four repeated), by the low four bits. The
/// selection is the one of the reference implementation published with
/// *Improving Noise* (K. Perlin, `ImprovedNoise`, 2002): bits 3 and 2 pick
/// the two axes `u, v` of the edge (`x, y`; `x, z`; `y, z`, and 12 and 14
/// repeating `y, x`), bits 0 and 1 their signs, so the dot product of the
/// offset with an edge of the cube `(±1, ±1, 0)` and its turns is a sum of
/// two of its coordinates; 16 cases for 12 edges, the 4 extra ones a
/// tetrahedron of them, as the paper chooses to avoid a division by 12.
fn grad(h: u32, x: f64, y: f64, z: f64) -> f64 {
    let h = h & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

/// The cell of a coordinate, clamped to [`Q_LIMIT`] first.
fn cell(c: f64) -> (i32, f64) {
    let c = if c.is_finite() {
        c.clamp(-Q_LIMIT, Q_LIMIT)
    } else {
        0.0
    };
    let f = c.floor();
    // Within ±2^20 before the cast.
    (f as i32, c - f)
}

fn hash_cell(i: [i32; 3], off: [u32; 3]) -> [u32; 3] {
    pcg3d([
        (i[0] as u32).wrapping_add(off[0]),
        (i[1] as u32).wrapping_add(off[1]),
        (i[2] as u32).wrapping_add(off[2]),
    ])
}

/// Seeded gradient noise at `q`, about −1 to 1, 0 at every lattice point.
pub fn noise(q: [f64; 3], off: [u32; 3]) -> f64 {
    let (ix, fx) = cell(q[0]);
    let (iy, fy) = cell(q[1]);
    let (iz, fz) = cell(q[2]);
    let corner = |dx: i32, dy: i32, dz: i32| {
        let h = hash_cell(
            [
                ix.wrapping_add(dx),
                iy.wrapping_add(dy),
                iz.wrapping_add(dz),
            ],
            off,
        );
        grad(
            h[0],
            fx - f64::from(dx),
            fy - f64::from(dy),
            fz - f64::from(dz),
        )
    };
    let (u, v, w) = (fade(fx), fade(fy), fade(fz));
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// fBm: `octaves` octaves of [`noise`], lacunarity 2 and gain ½, the
/// octaves' seeds from `base` on.
pub fn fbm(q: [f64; 3], seed: u32, octaves: u32, base: u32) -> f64 {
    let mut sum = 0.0;
    let (mut f, mut a) = (1.0, 1.0);
    for k in 0..octaves.min(MAX_DETAIL) {
        sum += a * noise(q.map(|c| c * f), offset(seed, base + k));
        f *= 2.0;
        a *= 0.5;
    }
    sum
}

/// Perlin's turbulence: fBm of `|noise|`.
pub fn turbulence(q: [f64; 3], seed: u32, octaves: u32, base: u32) -> f64 {
    let mut sum = 0.0;
    let (mut f, mut a) = (1.0, 1.0);
    for k in 0..octaves.min(MAX_DETAIL) {
        sum += a * noise(q.map(|c| c * f), offset(seed, base + k)).abs();
        f *= 2.0;
        a *= 0.5;
    }
    sum
}

/// Worley's cellular noise: the distance to the nearest feature point (one
/// per cell) and a number in `[0, 1)` of that point's cell.
pub fn cellular(q: [f64; 3], off: [u32; 3]) -> (f64, f64) {
    let c = cellular_pair(q, off);
    (c.f1, c.v1)
}

/// The nearest two feature points of [`cellular`]: their distances and
/// their cells' numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cells {
    pub f1: f64,
    pub v1: f64,
    pub f2: f64,
    pub v2: f64,
}

/// [`cellular`] with the second nearest feature point (Worley's `F2`).
pub fn cellular_pair(q: [f64; 3], off: [u32; 3]) -> Cells {
    let (ix, fx) = cell(q[0]);
    let (iy, fy) = cell(q[1]);
    let (iz, fz) = cell(q[2]);
    let (mut best, mut second) = (f64::INFINITY, f64::INFINITY);
    let (mut value, mut value2) = (0.0, 0.0);
    for dz in -1..=1_i32 {
        for dy in -1..=1_i32 {
            for dx in -1..=1_i32 {
                let h = hash_cell(
                    [
                        ix.wrapping_add(dx),
                        iy.wrapping_add(dy),
                        iz.wrapping_add(dz),
                    ],
                    off,
                );
                let px = f64::from(dx) + unit(h[0]) - fx;
                let py = f64::from(dy) + unit(h[1]) - fy;
                let pz = f64::from(dz) + unit(h[2]) - fz;
                let d2 = px * px + py * py + pz * pz;
                if d2 < best {
                    second = best;
                    value2 = value;
                    best = d2;
                    value = unit(pcg3d(h)[0]);
                } else if d2 < second {
                    second = d2;
                    value2 = unit(pcg3d(h)[0]);
                }
            }
        }
    }
    Cells {
        f1: best.sqrt(),
        v1: value,
        f2: second.sqrt(),
        v2: value2,
    }
}

/// `sin(π x)` for `x` in −1 to 1 (clamped), by a polynomial of `+` and `×`
/// only, so that it gives the same bits on every platform (the standard
/// library's `sin` is the platform's and may differ in the last bit): the
/// argument folded into ±½ by `sin(π x) = sin(π (±1 − x))`, then the Taylor
/// series of `sin y`, `|y| ≤ π/2`, to `y²¹` (a remainder below 10⁻¹⁸).
pub fn sin_pi(x: f64) -> f64 {
    let x = if x.is_finite() {
        x.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let x = if x > 0.5 {
        1.0 - x
    } else if x < -0.5 {
        -1.0 - x
    } else {
        x
    };
    let y = std::f64::consts::PI * x;
    let y2 = y * y;
    // Horner over y (1 − y²/(2·3) (1 − y²/(4·5) (1 − …))).
    let mut sum = 1.0;
    for k in (1..=10_u32).rev() {
        let n = f64::from(2 * k) * f64::from(2 * k + 1);
        sum = 1.0 - y2 / n * sum;
    }
    y * sum
}

/// `atan(z)` for `z` in 0 to 1 by halving the angle twice
/// (`atan z = 2 atan(z / (1 + √(1 + z²)))`, `√` exact in IEEE) and the
/// series to `z²⁷`: the same bits everywhere, as [`sin_pi`].
fn atan_unit(z: f64) -> f64 {
    let mut z = z;
    for _ in 0..2 {
        z /= 1.0 + (1.0 + z * z).sqrt();
    }
    let z2 = z * z;
    let mut sum = 0.0;
    for k in (0..=13_u32).rev() {
        let term = 1.0 / f64::from(2 * k + 1);
        sum = term - z2 * sum;
    }
    4.0 * z * sum
}

/// `atan2(y, x)` in −π to π, from [`atan_unit`] by the octant: the same
/// bits everywhere. 0 for `(0, 0)` and for numbers that are not finite.
pub fn atan2_exact(y: f64, x: f64) -> f64 {
    use std::f64::consts::{FRAC_PI_2, PI};
    if !(x.is_finite() && y.is_finite()) || (x == 0.0 && y == 0.0) {
        return 0.0;
    }
    let (ax, ay) = (x.abs(), y.abs());
    let a = if ay <= ax {
        atan_unit(ay / ax)
    } else {
        FRAC_PI_2 - atan_unit(ax / ay)
    };
    let a = if x < 0.0 { PI - a } else { a };
    if y < 0.0 { -a } else { a }
}

/// Wood's profile across a ring, `f` the place in it (0 to 1): early wood
/// (0) widening into late wood (1), back to early wood at the ring's
/// border; 0 at both ends.
pub fn ring_profile(f: f64) -> f64 {
    smoothstep(0.35, 0.85, f) * (1.0 - smoothstep(0.9, 1.0, f))
}

fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::new(f64::from(a[0]), f64::from(a[1]), f64::from(a[2]))
}

impl Pattern {
    /// Every value brought into its range; one that is not a number takes
    /// the default's.
    pub fn sanitized(self) -> Pattern {
        let d = Pattern::default();
        let axis = if self.axis.iter().all(|c| c.is_finite())
            && v3(self.axis).length() > f64::from(f32::MIN_POSITIVE)
        {
            self.axis
        } else {
            d.axis
        };
        Pattern {
            kind: self.kind,
            ramp: self.ramp,
            stops: self.stops.clamp(1, MAX_STOPS as u8),
            roughness: self.roughness.map(|r| r.map(|v| within(v, 0.0, 1.0, 0.5))),
            scale: within(self.scale, MIN_SCALE, MAX_SCALE, 1.0),
            origin: if self.origin.iter().all(|c| c.is_finite()) {
                self.origin
            } else {
                [0.0; 3]
            },
            axis,
            seed: self.seed,
            distortion: within(self.distortion, 0.0, 8.0, 0.0),
            detail: self.detail.clamp(1, MAX_DETAIL),
            grain: within(self.grain, 0.0, 1.0, 0.0),
            rays: within(self.rays, 0.0, 1.0, 0.0),
            spot: within(self.spot, 0.0, 1.0, d.spot),
            wrap: self.wrap,
        }
    }

    /// The pattern's axes in the model's: two across the axis and the axis
    /// itself, unit, in `f32` (both renderers read these numbers).
    pub fn frame(&self) -> [[f32; 3]; 3] {
        let z = v3(self.axis).normalized().unwrap_or(Vec3::Z);
        let helper = if z.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
        let x = z.cross(helper).normalized().unwrap_or(Vec3::X);
        let y = z.cross(x);
        // Deliberate narrowing: the GPU reads these.
        let f = |v: Vec3| [v.x as f32, v.y as f32, v.z as f32];
        [f(x), f(y), f(z)]
    }

    /// `p`'s local coordinates in units of the scale: across, across,
    /// along the axis.
    pub fn local(&self, p: Vec3) -> [f64; 3] {
        let rel = p - Vec3::new(self.origin[0], self.origin[1], self.origin[2]);
        let s = f64::from(self.scale);
        self.frame().map(|a| v3(a).dot(rel) / s)
    }

    /// The pattern's value (0 to 1, a place on the ramp) at local
    /// coordinates `q`.
    pub fn value(&self, q: [f64; 3]) -> f64 {
        let [x, y, z] = q;
        let seed = self.seed;
        let detail = self.detail;
        let t = match self.kind {
            PatternKind::Wood => {
                let wobble = if self.distortion > 0.0 {
                    // At most two octaves: where a face cuts the rings at
                    // a grazing angle a fine wobble is stretched into teeth.
                    f64::from(self.distortion)
                        * fbm([x * 0.35, y * 0.35, z * 0.03], seed, detail.min(2), 0)
                } else {
                    0.0
                };
                let radius = (x * x + y * y).sqrt();
                let r = radius + wobble;
                let mut t = ring_profile(r - r.floor());
                if self.grain > 0.0 {
                    t += f64::from(self.grain)
                        * 0.18
                        * noise([x * 12.0, y * 12.0, z * 0.15], offset(seed, 24));
                }
                if self.rays > 0.0 && radius > 1e-9 {
                    let n = noise(
                        [x / radius * 48.0, y / radius * 48.0, z * 0.6 + radius * 0.3],
                        offset(seed, 32),
                    );
                    t -= f64::from(self.rays) * 0.7 * smoothstep(0.25, 0.45, n);
                }
                t
            }
            PatternKind::Marble => {
                let turb = turbulence([x * 0.5, y * 0.5, z * 0.5], seed, detail, 0);
                // `sin(2π u)` from `u`'s place in its period, so that the
                // GPU's `sin` takes an argument within ±π.
                let u = z + f64::from(self.distortion) * turb;
                let s = -sin_pi(2.0 * (u - u.floor()) - 1.0);
                let v = 1.0 - s.abs();
                let v2 = v * v;
                // Faint clouds between the veins.
                v2 * v2 + 0.1 * (1.0 + fbm([x * 2.0, y * 2.0, z * 2.0], seed, 3, 8))
            }
            PatternKind::Granite => {
                // Each crystal one of the ramp's colours, blended with its
                // neighbour's within a thin band along their border (so the
                // pattern has no jump for the renderers to decide apart).
                let c = cellular_pair(q, offset(seed, 0));
                let stops = f64::from(self.stops.max(2));
                let quantised = |v: f64| ((v * stops).floor() / (stops - 1.0)).min(1.0);
                let (t1, t2) = (quantised(c.v1), quantised(c.v2));
                let w = smoothstep(0.0, 0.08, c.f2 - c.f1);
                t2 + (t1 - t2) * (0.5 + 0.5 * w)
            }
            PatternKind::Speckle => {
                let (d, v) = cellular(q, offset(seed, 0));
                let radius = f64::from(self.spot) * (0.5 + 0.5 * v);
                // A dot of no size is no dot (smoothstep of an empty
                // interval is not defined in WGSL).
                if radius > 0.0 {
                    1.0 - smoothstep(radius * 0.8, radius, d)
                } else {
                    0.0
                }
            }
            PatternKind::Noise => {
                0.5 + 0.6 * (1.0 + f64::from(self.distortion)) * fbm(q, seed, detail, 0)
            }
            PatternKind::Brushed => {
                let mut t = 0.5 + 0.9 * fbm([x, y, z * 0.02], seed, detail.min(3), 0);
                if self.grain > 0.0 {
                    t += f64::from(self.grain)
                        * 0.5
                        * noise([x * 3.0, y * 3.0, z * 0.05], offset(seed, 24));
                }
                t
            }
            PatternKind::Carbon => {
                let (a, b) = if self.wrap {
                    let r = (x * x + y * y).sqrt();
                    let around = (r * std::f64::consts::TAU / 4.0 + 0.5).floor().max(1.0) * 4.0;
                    let turn = atan2_exact(y, x) / std::f64::consts::TAU + 0.5;
                    (z, turn * around)
                } else {
                    (x, y)
                };
                // A tow floats over two cells: the warp (along `a`) where
                // `(i + j) mod 4` is 0 or 1, the weft (along `b`) where it is
                // 2 or 3. Its brightness is round across it and along its
                // float, 0 at every border, so the weave has no jump.
                let (i, fa) = cell(a);
                let (j, fb) = cell(b);
                let k = i.wrapping_add(j) & 3;
                let (across, along, amp) = if k < 2 {
                    (fb, f64::from(k) + fa, 1.0)
                } else {
                    (fa, f64::from(k - 2) + fb, 0.55)
                };
                amp * sin_pi(across) * sin_pi(0.5 * along)
            }
        };
        if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The ramp's colour at `t`, in linear light.
    pub fn color(&self, t: f64) -> [f64; 3] {
        let stops = usize::from(self.stops.clamp(1, MAX_STOPS as u8));
        let decode = |k: usize| {
            self.ramp
                .get(k)
                .copied()
                .unwrap_or([0; 3])
                .map(|b| f64::from(stops_linear(b)))
        };
        if stops == 1 {
            return decode(0);
        }
        let x = t.clamp(0.0, 1.0) * (stops - 1) as f64;
        let k = (x.floor() as usize).min(stops - 2);
        let f = x - k as f64;
        let (a, b) = (decode(k), decode(k + 1));
        [0, 1, 2].map(|c| {
            let (a, b) = (
                a.get(c).copied().unwrap_or(0.0),
                b.get(c).copied().unwrap_or(0.0),
            );
            a + f * (b - a)
        })
    }

    /// The roughness at `t`, or `fallback` without a ramp of its own.
    pub fn roughness_at(&self, t: f64, fallback: f64) -> f64 {
        match self.roughness {
            Some([a, b]) => {
                let (a, b) = (f64::from(a), f64::from(b));
                a + t.clamp(0.0, 1.0) * (b - a)
            }
            None => fallback,
        }
    }

    /// The base colour (linear) and roughness at the point `p`, the face's
    /// own roughness `roughness` kept without a ramp of roughness.
    pub fn at(&self, p: Vec3, roughness: f64) -> ([f64; 3], f64) {
        let t = self.value(self.local(p));
        (self.color(t), self.roughness_at(t, roughness))
    }

    /// The mean colour (sRGB) over a cube of 12 features about the origin,
    /// sampled at 24³ cell centres and averaged in linear light: what a
    /// format that cannot hold the pattern exports.
    pub fn mean_color(&self) -> [u8; 3] {
        const N: usize = 24;
        let mut sum = [0.0; 3];
        for k in 0..N {
            for j in 0..N {
                for i in 0..N {
                    let c = |n: usize| (n as f64 + 0.5) / N as f64 * 12.0 - 6.0;
                    let col = self.color(self.value([c(i), c(j), c(k)]));
                    for (s, v) in sum.iter_mut().zip(col) {
                        *s += v;
                    }
                }
            }
        }
        let count = (N * N * N) as f64;
        sum.map(|s| tonemap::to_byte(tonemap::srgb_encode(s / count)))
    }

    /// True when moving `p` by `delta` (model units) along any of the six
    /// axis directions moves a channel of its colour, in sRGB levels, by
    /// more than one level, or its roughness by as much: where the GPU's
    /// `f32` point alone may use up the comparison's one level.
    pub fn in_doubt(&self, p: Vec3, delta: f64) -> bool {
        let level = |p: Vec3| {
            let (c, r) = self.at(p, 0.0);
            let [a, b, d] = c.map(|v| tonemap::srgb_encode(v) * 255.0);
            [a, b, d, r * 255.0]
        };
        let here = level(p);
        [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::X, -Vec3::Y, -Vec3::Z]
            .into_iter()
            .any(|d| {
                let there = level(p + d * delta);
                here.iter().zip(there).any(|(a, b)| (a - b).abs() > 1.0)
            })
    }
}

/// A ramp byte in linear light, in `f32` as the GPU's table holds it.
pub fn stops_linear(b: u8) -> f32 {
    // Deliberate narrowing: the GPU reads this.
    tonemap::byte_to_linear(b) as f32
}
