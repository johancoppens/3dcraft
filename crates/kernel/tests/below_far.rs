//! A section small against its distance from the origin (fault M, fuzz
//! seed 138822; 2026-11-22; docs/design/sections-and-levels.md).
//!
//! Five bodies with closed forms, a sphere (the spherical cap and its
//! circle), a block cut aslant (a rectangle), a cylinder cut aslant (an
//! ellipse), a torus cut square to its axis (an annulus, Pappus) and a
//! spline cone (freeform, rule 3), each turned in space and placed at 0,
//! 10, 1e3, 1e4, 1e5 and 1e7 times its size from the origin, at the scales
//! 1e-3, 1 and 1e6 with the tolerance scaled along, cut at three offsets:
//! every one is measured (but the spline cone 1e7 times its size away,
//! which the boolean refuses to cut, before the change as after it), to
//! its closed form within 1e-9 of its size to
//! the integral's power (the freeform: and 20 tolerances, as the module's
//! own tests hold it) and 64 times what its coordinates' rounding allows
//! (a unit in the last place of its distance from the origin along its
//! perimeter, over its surface).
//!
//! And today's answers kept: every row of that table measured before the
//! change, and the oracles of the module's own tests, the level search's
//! and the floating equilibrium's, pinned to the bit (a hash of every
//! number they give).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::below::{BelowPlane, CutPlane, Floating, Level, below, floating, level_for};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives::{block, cylinder, sphere, torus};
use craft3d_math::{Frame, Portable, Tolerance, Transform, Vec3};
use craft3d_topo::Body;
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// How far from the origin each body lies, in its own size.
const RATIOS: [f64; 6] = [0.0, 10.0, 1e3, 1e4, 1e5, 1e7];

fn tol(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

// ------------------------------------------------------------- hashing

/// FNV-1a over the bits of `values`: two lists hash alike only when every
/// number is the same to the bit (and in the same order).
fn hash(values: &[f64]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in values {
        for byte in v.to_bits().to_le_bytes() {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

fn push3(out: &mut Vec<f64>, v: Option<Vec3>) {
    match v {
        Some(v) => out.extend([v.x, v.y, v.z]),
        None => out.extend([f64::NAN; 3]),
    }
}

fn below_values(out: &mut Vec<f64>, b: &BelowPlane) {
    out.extend([b.offset, b.volume]);
    push3(out, b.centroid);
    out.push(b.surface);
    let s = b.section;
    out.push(s.area);
    push3(out, s.centroid);
    out.extend([s.i_x, s.i_y, s.product]);
}

fn below_hash(b: &BelowPlane) -> u64 {
    let mut out = Vec::new();
    below_values(&mut out, b);
    hash(&out)
}

fn level_hash(l: &Level) -> u64 {
    let mut out = Vec::new();
    below_values(&mut out, &l.below);
    out.extend([l.steps as f64, l.residual]);
    hash(&out)
}

fn floating_hash(f: &Floating) -> u64 {
    let mut out = Vec::new();
    for e in &f.equilibria {
        push3(&mut out, Some(e.plane.normal));
        push3(&mut out, Some(e.plane.x_axis));
        below_values(&mut out, &e.level.below);
        out.extend([e.level.steps as f64, e.level.residual, e.arm, e.energy]);
        for p in e.principal {
            out.push(p.gm);
            push3(&mut out, Some(p.about));
            out.push(p.stability as u8 as f64);
        }
        out.extend([e.iterations as f64, f64::from(u8::from(e.settled))]);
    }
    out.extend([f.unfinished.len() as f64, f.levels as f64, f.floor]);
    hash(&out)
}

// ------------------------------------------------- bodies far from the origin

#[derive(Clone, Copy, Debug)]
enum Kind {
    Sphere,
    Block,
    Cylinder,
    Torus,
    Cone,
}

const KINDS: [Kind; 5] = [
    Kind::Sphere,
    Kind::Block,
    Kind::Cylinder,
    Kind::Torus,
    Kind::Cone,
];

/// Where a body lies: its own origin at `centre`, its own axes turned by
/// `turn` (a turn about a slanted axis, so no face, edge or plane is along
/// a world axis).
struct Placed {
    centre: Vec3,
    turn: Transform,
}

impl Placed {
    fn new(k: f64, ratio: f64) -> Placed {
        Placed {
            centre: Vec3::new(0.6, -0.48, 0.64) * (ratio * k),
            turn: Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 1.1).unwrap(),
        }
    }

    fn point(&self, local: Vec3) -> Vec3 {
        self.centre + self.turn.vector(local)
    }

    fn vector(&self, local: Vec3) -> Vec3 {
        self.turn.vector(local)
    }

    fn frame(&self, origin: Vec3, z: Vec3, x: Vec3) -> Frame {
        Frame::new(self.point(origin), self.vector(z), self.vector(x)).unwrap()
    }

    /// A world point in the body's own axes.
    fn local(&self, world: Vec3) -> Vec3 {
        let d = world - self.centre;
        Vec3::new(
            d.dot(self.vector(Vec3::X)),
            d.dot(self.vector(Vec3::Y)),
            d.dot(self.vector(Vec3::Z)),
        )
    }
}

fn make(kind: Kind, k: f64, at: &Placed) -> Result<Body, String> {
    match kind {
        Kind::Sphere => {
            sphere(&at.frame(Vec3::ZERO, Vec3::Z, Vec3::X), k).map_err(|e| e.to_string())
        }
        Kind::Block => block(
            &at.frame(Vec3::new(-1.5 * k, -0.5 * k, -0.5 * k), Vec3::Z, Vec3::X),
            3.0 * k,
            k,
            k,
        )
        .map_err(|e| e.to_string()),
        Kind::Cylinder => cylinder(
            &at.frame(Vec3::new(0.0, 0.0, -2.0 * k), Vec3::Z, Vec3::X),
            k,
            4.0 * k,
        )
        .map_err(|e| e.to_string()),
        Kind::Torus => torus(&at.frame(Vec3::ZERO, Vec3::Z, Vec3::X), 2.0 * k, 0.5 * k)
            .map_err(|e| e.to_string()),
        // Apex at the origin, its axis along the frame's y: here the body's z.
        Kind::Cone => craft3d_testkit::solids::spline_cone(
            &at.frame(Vec3::ZERO, Vec3::X, Vec3::Y),
            0.5 * k,
            k,
        )
        .map(|r| r.body)
        .map_err(|e| e.to_string()),
    }
}

/// The cutting plane in the body's own axes: its normal (x along x).
fn local_normal(kind: Kind) -> Vec3 {
    let tilted = |angle: f64| {
        let (s, c) = angle.psin_cos();
        Vec3::new(0.0, -s, c)
    };
    match kind {
        Kind::Sphere | Kind::Torus | Kind::Cone => Vec3::Z,
        Kind::Block => tilted(0.3),
        Kind::Cylinder => tilted(0.5),
    }
}

/// The plane's offsets from the body's own origin along its normal, in
/// its size.
fn offsets(kind: Kind) -> [f64; 3] {
    match kind {
        Kind::Sphere => [-0.5, 0.0, 0.3],
        Kind::Block => [-0.2, 0.0, 0.2],
        Kind::Cylinder => [-0.3, 0.0, 0.3],
        Kind::Torus => [-0.3, 0.0, 0.25],
        Kind::Cone => [0.3, 0.6, 0.9],
    }
}

/// What lies below the plane at `h` from the body's origin, in closed
/// form, in the body's own axes.
#[derive(Clone, Copy, Debug)]
struct Exact {
    volume: f64,
    /// The centroid below.
    below: Vec3,
    area: f64,
    /// The section's centroid.
    centre: Vec3,
    i_x: f64,
    i_y: f64,
    product: f64,
    /// The section's perimeter, and how far it reaches from its centroid.
    perimeter: f64,
    reach: f64,
    /// The whole body's surface: no less than the surface below.
    surface: f64,
    /// A freeform body's section lies within the tolerance of its
    /// surface: tolerances (times the size to the integral's power less
    /// one) its closed forms are held to beyond the analytic ones'.
    slack: f64,
}

fn exact(kind: Kind, k: f64, h: f64) -> Exact {
    match kind {
        Kind::Sphere => {
            let r = k;
            let s = h + r;
            let d = 3.0 * (2.0 * r - s).ppowi(2) / (4.0 * (3.0 * r - s));
            let a2 = r * r - h * h;
            Exact {
                volume: PI * s * s * (3.0 * r - s) / 3.0,
                below: Vec3::new(0.0, 0.0, -d),
                area: PI * a2,
                centre: Vec3::new(0.0, 0.0, h),
                i_x: PI * a2 * a2 / 4.0,
                i_y: PI * a2 * a2 / 4.0,
                product: 0.0,
                perimeter: 2.0 * PI * a2.sqrt(),
                reach: a2.sqrt(),
                surface: 4.0 * PI * r * r,
                slack: 0.0,
            }
        }
        Kind::Block => {
            let (l, w, ht) = (3.0 * k, k, k);
            let (s, c) = 0.3_f64.psin_cos();
            let (alpha, beta, wide) = (h / c, s / c, w / c);
            let depth = alpha + 0.5 * ht;
            Exact {
                volume: l * w * depth,
                below: Vec3::new(
                    0.0,
                    beta * w * w / (12.0 * depth),
                    (alpha * alpha + beta * beta * w * w / 12.0 - 0.25 * ht * ht) / (2.0 * depth),
                ),
                area: l * wide,
                centre: Vec3::new(0.0, 0.0, alpha),
                i_x: l * wide.ppowi(3) / 12.0,
                i_y: wide * l.ppowi(3) / 12.0,
                product: 0.0,
                perimeter: 2.0 * (l + wide),
                reach: 0.5 * l.phypot(wide),
                surface: 2.0 * (l * w + l * ht + w * ht),
                slack: 0.0,
            }
        }
        Kind::Cylinder => {
            let r = k;
            let half = 2.0 * k;
            let (s, c) = 0.5_f64.psin_cos();
            let (alpha, beta) = (h / c, s / c);
            let disc = PI * r * r;
            let volume = disc * (half + alpha);
            let (a, b) = (r, r / c);
            Exact {
                volume,
                below: Vec3::new(
                    0.0,
                    beta * PI * r.ppowi(4) / (4.0 * volume),
                    (disc * alpha * alpha + beta * beta * PI * r.ppowi(4) / 4.0
                        - half * half * disc)
                        / (2.0 * volume),
                ),
                area: disc / c,
                centre: Vec3::new(0.0, 0.0, alpha),
                i_x: PI * a * b.ppowi(3) / 4.0,
                i_y: PI * a.ppowi(3) * b / 4.0,
                product: 0.0,
                // Ramanujan's: within 1e-5 of it, for a yardstick.
                perimeter: PI * (3.0 * (a + b) - ((3.0 * a + b) * (a + 3.0 * b)).sqrt()),
                reach: b,
                surface: 2.0 * PI * r * (r + 2.0 * half),
                slack: 0.0,
            }
        }
        Kind::Torus => {
            let (big, r) = (2.0 * k, 0.5 * k);
            let w = (r * r - h * h).sqrt();
            let segment = r * r * (-h / r).pacos() + h * w;
            Exact {
                volume: 2.0 * PI * big * segment,
                below: Vec3::new(0.0, 0.0, -2.0 / 3.0 * w.ppowi(3) / segment),
                area: 4.0 * PI * big * w,
                centre: Vec3::new(0.0, 0.0, h),
                i_x: PI * ((big + w).ppowi(4) - (big - w).ppowi(4)) / 4.0,
                i_y: PI * ((big + w).ppowi(4) - (big - w).ppowi(4)) / 4.0,
                product: 0.0,
                perimeter: 4.0 * PI * big,
                reach: big + w,
                surface: 4.0 * PI * PI * big * r,
                slack: 0.0,
            }
        }
        Kind::Cone => {
            let (top, height) = (0.5 * k, k);
            let rho = top * h / height;
            Exact {
                volume: PI * rho * rho * h / 3.0,
                below: Vec3::new(0.0, 0.0, 0.75 * h),
                area: PI * rho * rho,
                centre: Vec3::new(0.0, 0.0, h),
                i_x: PI * rho.ppowi(4) / 4.0,
                i_y: PI * rho.ppowi(4) / 4.0,
                product: 0.0,
                perimeter: 2.0 * PI * rho,
                reach: rho,
                surface: PI * top * (top + top.phypot(height)) + PI * top * top,
                slack: 20.0,
            }
        }
    }
}

/// One row of the table: a body at a distance, cut at an offset.
struct Row {
    name: String,
    k: f64,
    ratio: f64,
    kind: Kind,
    found: Result<(BelowPlane, Placed), String>,
    want: Exact,
}

/// Every body at every scale and distance, cut at its three offsets.
fn far_rows() -> Vec<Row> {
    let mut rows = Vec::new();
    for kind in KINDS {
        for k in SCALES {
            for ratio in RATIOS {
                let at = Placed::new(k, ratio);
                let made = make(kind, k, &at);
                let normal = at.vector(local_normal(kind));
                let plane = CutPlane::new(normal, at.vector(Vec3::X)).unwrap();
                for h in offsets(kind) {
                    let name = format!("{kind:?} k={k:e} D={ratio:e} h={h}");
                    let offset = at.centre.dot(normal) + h * k;
                    let found = match &made {
                        Ok(body) => below(&[body], &plane, offset, &tol(k))
                            .map(|b| (b, Placed::new(k, ratio)))
                            .map_err(|e| e.to_string()),
                        Err(e) => Err(format!("not built: {e}")),
                    };
                    rows.push(Row {
                        name,
                        k,
                        ratio,
                        kind,
                        found,
                        want: exact(kind, k, h * k),
                    });
                }
            }
        }
    }
    rows
}

/// How far each measured number is from its closed form, as a share of
/// what it is allowed (1e-9 of the size to its power, a freeform body's
/// tolerances, and 64 units of what the coordinates allow), and the error
/// itself in units of what the coordinates allow (`ε · distance` along
/// the perimeter, over the surface; for the centroids, of the point and
/// moved by the section's or the volume's first moment).
struct Errors {
    /// (what, error / allowed, error / rounding)
    each: Vec<(&'static str, f64, f64)>,
}

fn errors(row: &Row, found: &BelowPlane, at: &Placed) -> Errors {
    let (k, want) = (row.k, &row.want);
    let far = row.ratio * k;
    let eps = f64::EPSILON;
    let tolerance = tol(k).linear;
    let slack = |power: i32| want.slack * tolerance * k.ppowi(power - 1);
    let allowed = |value: f64, power: i32, rounding: f64| {
        1e-9 * value.abs().max(k.ppowi(power)) + slack(power) + 64.0 * rounding
    };
    let s = found.section;
    let mut each = Vec::new();
    let mut add = |what: &'static str, got: f64, value: f64, power: i32, rounding: f64| {
        let err = (got - value).abs();
        each.push((what, err / allowed(value, power, rounding), err / rounding));
    };
    let r_area = eps * far * want.perimeter;
    let r_volume = eps * far * want.surface;
    let r_moment = eps * far * want.perimeter * want.reach * want.reach;
    add("volume", found.volume, want.volume, 3, r_volume);
    add("area", s.area, want.area, 2, r_area);
    add("ix", s.i_x, want.i_x, 4, r_moment);
    add("iy", s.i_y, want.i_y, 4, r_moment);
    add("ixy", s.product, want.product, 4, r_moment);
    let r_centre = eps * far * (1.0 + want.reach * want.perimeter / want.area);
    let centre = s
        .centroid
        .map_or(f64::INFINITY, |c| (at.local(c) - want.centre).length());
    each.push((
        "centre",
        centre / allowed(0.0, 1, r_centre),
        centre / r_centre,
    ));
    let r_below = eps * far * (1.0 + 3.0 * k * want.surface / want.volume);
    let below = found
        .centroid
        .map_or(f64::INFINITY, |c| (at.local(c) - want.below).length());
    each.push(("below", below / allowed(0.0, 1, r_below), below / r_below));
    Errors { each }
}

fn print_rows(rows: &[Row]) {
    println!("far table: body, scale, distance / size, offset / size: what was found");
    println!(
        "  each number: error / allowed (pass at <= 1), [error / rounding of the coordinates]"
    );
    for row in rows {
        match &row.found {
            Ok((found, at)) => {
                let e = errors(row, found, at);
                let worst = e.each.iter().map(|x| x.1).fold(0.0, f64::max);
                let cells: Vec<String> = e
                    .each
                    .iter()
                    .map(|(what, share, units)| {
                        if row.ratio == 0.0 {
                            format!("{what} {share:.1e}")
                        } else {
                            format!("{what} {share:.1e} [{units:.2}]")
                        }
                    })
                    .collect();
                println!(
                    "ROW {} | answered, worst {worst:.1e} | {} | hash {:016x}",
                    row.name,
                    cells.join(", "),
                    below_hash(found)
                );
            }
            Err(e) => println!("ROW {} | REFUSED: {e}", row.name),
        }
    }
    let refused = rows.iter().filter(|r| r.found.is_err()).count();
    println!("far table: {} rows, {refused} refused", rows.len());
    for kind in KINDS {
        for ratio in RATIOS {
            let (n, bad) = rows
                .iter()
                .filter(|r| {
                    r.ratio == ratio
                        && std::mem::discriminant(&r.kind) == std::mem::discriminant(&kind)
                })
                .fold((0, 0), |(n, bad), r| {
                    (n + 1, bad + usize::from(r.found.is_err()))
                });
            println!("SUMMARY {kind:?} D={ratio:e}: {bad} of {n} refused");
        }
    }
}

/// What each row of the table that was answered before the change gave
/// (`main` 5305636e, printed by the Check run 45024256884 on the probe
/// commit 31d39767, which is that commit and these tests alone): the hash
/// of every number of it. A row not listed was refused there. The torus's
/// rows at 0 and 10 times its size from the origin since 2026-11-25 (the
/// trial Gate 45639379639 of `check/near-look-3`): the box of a whole
/// torus face reaches `R √(z_j² + z_k²) + r` along each world axis rather
/// than `R √(1 − z_i²) + r` (`craft3d_topo` `transform.rs` ›
/// `face_extent`), the same reach but for its last bits for this turned
/// torus, and the section reads the box, so the hashes moved; every one of
/// those rows is measured within its allowance as before.
const FAR_PINS: &[(&str, u64)] = &[
    ("Sphere k=1e-3 D=0e0 h=-0.5", 0xee47ab214d279442),
    ("Sphere k=1e-3 D=0e0 h=0", 0x8ae60270df768af2),
    ("Sphere k=1e-3 D=0e0 h=0.3", 0xbae6b8c2e11da283),
    ("Sphere k=1e-3 D=1e1 h=-0.5", 0xb26633ea3f7e49c7),
    ("Sphere k=1e-3 D=1e1 h=0", 0xb17aeb77b0b57bec),
    ("Sphere k=1e-3 D=1e1 h=0.3", 0x939d6ae241fc612d),
    ("Sphere k=1e-3 D=1e3 h=-0.5", 0xaeef174da07df359),
    ("Sphere k=1e-3 D=1e3 h=0", 0x05b6c82c9686b46e),
    ("Sphere k=1e-3 D=1e3 h=0.3", 0x09001467befca637),
    ("Sphere k=1e-3 D=1e4 h=-0.5", 0xb2fe4cb3fdfc7b35),
    ("Sphere k=1e-3 D=1e4 h=0", 0x04e273e233415e1e),
    ("Sphere k=1e-3 D=1e4 h=0.3", 0x0555e5b32888ac8b),
    ("Sphere k=1e0 D=0e0 h=-0.5", 0x06e546b2d944a930),
    ("Sphere k=1e0 D=0e0 h=0", 0x4302588332608285),
    ("Sphere k=1e0 D=0e0 h=0.3", 0x4d9d3692523bf123),
    ("Sphere k=1e0 D=1e1 h=-0.5", 0x369d2625c43f4e41),
    ("Sphere k=1e0 D=1e1 h=0", 0x1e63eb0b49dd2058),
    ("Sphere k=1e0 D=1e1 h=0.3", 0x1841160d75bc30cb),
    ("Sphere k=1e0 D=1e3 h=-0.5", 0x6b380b28b57fe433),
    ("Sphere k=1e0 D=1e3 h=0", 0xb2393c581954e800),
    ("Sphere k=1e0 D=1e3 h=0.3", 0xd56819d8be1f811e),
    ("Sphere k=1e0 D=1e4 h=-0.5", 0xa0f7cc761ff30941),
    ("Sphere k=1e0 D=1e4 h=0", 0x44ba7821855b4171),
    ("Sphere k=1e0 D=1e4 h=0.3", 0x867da8a651910fd7),
    ("Sphere k=1e6 D=0e0 h=-0.5", 0x973c024915523769),
    ("Sphere k=1e6 D=0e0 h=0", 0x811d1cd6b3c10a49),
    ("Sphere k=1e6 D=0e0 h=0.3", 0xd33985ddc4e03575),
    ("Sphere k=1e6 D=1e1 h=-0.5", 0x9dccdc19039c1b9e),
    ("Sphere k=1e6 D=1e1 h=0", 0xb06cde2cf2477409),
    ("Sphere k=1e6 D=1e1 h=0.3", 0x4b09cf3529359ca5),
    ("Sphere k=1e6 D=1e3 h=-0.5", 0x58169125cece24c0),
    ("Sphere k=1e6 D=1e3 h=0", 0x4bdf1c0edae37d66),
    ("Sphere k=1e6 D=1e3 h=0.3", 0x31f80e5d35515be4),
    ("Sphere k=1e6 D=1e4 h=-0.5", 0xb5ca7b7c27811a13),
    ("Sphere k=1e6 D=1e4 h=0", 0xab72e5ba00dc884b),
    ("Sphere k=1e6 D=1e4 h=0.3", 0xfe8c2a633c477398),
    ("Block k=1e-3 D=0e0 h=-0.2", 0x9fca75cd5a1d87c1),
    ("Block k=1e-3 D=0e0 h=0", 0x97606996e6e85642),
    ("Block k=1e-3 D=0e0 h=0.2", 0x475c182faee4dd77),
    ("Block k=1e-3 D=1e1 h=-0.2", 0x1ab72b2dc67996b3),
    ("Block k=1e-3 D=1e1 h=0", 0x74abd640468e2b23),
    ("Block k=1e-3 D=1e1 h=0.2", 0x12817de2898df5d0),
    ("Block k=1e-3 D=1e3 h=-0.2", 0x8940edf515187710),
    ("Block k=1e-3 D=1e3 h=0", 0x75cf8158c4be6f68),
    ("Block k=1e-3 D=1e3 h=0.2", 0x6579425685ac0e04),
    ("Block k=1e-3 D=1e4 h=-0.2", 0xa828245ae44d0014),
    ("Block k=1e-3 D=1e4 h=0", 0x1fbec9d7523856a9),
    ("Block k=1e-3 D=1e4 h=0.2", 0xe314af3fa867127f),
    ("Block k=1e0 D=0e0 h=-0.2", 0xb7141e0c2657e828),
    ("Block k=1e0 D=0e0 h=0", 0xbd47a8f8ff305eae),
    ("Block k=1e0 D=0e0 h=0.2", 0x15c912226610ee07),
    ("Block k=1e0 D=1e1 h=-0.2", 0x35807ca512881929),
    ("Block k=1e0 D=1e1 h=0", 0x17d3ef7531cd315e),
    ("Block k=1e0 D=1e1 h=0.2", 0xb7fa137e307395df),
    ("Block k=1e0 D=1e3 h=-0.2", 0xcb6029d6c80dc390),
    ("Block k=1e0 D=1e3 h=0", 0xe73f4d51d0eb9bea),
    ("Block k=1e0 D=1e3 h=0.2", 0x710d7e37313243d8),
    ("Block k=1e0 D=1e4 h=-0.2", 0xa93243eae7623953),
    ("Block k=1e0 D=1e4 h=0", 0x3650fb4d977ed8b3),
    ("Block k=1e0 D=1e4 h=0.2", 0xd88e6adc0c4e36a6),
    ("Block k=1e6 D=0e0 h=-0.2", 0xdd1c9c9f711193a7),
    ("Block k=1e6 D=0e0 h=0", 0x3e8e756d40d28a12),
    ("Block k=1e6 D=0e0 h=0.2", 0x259f73e3766bf563),
    ("Block k=1e6 D=1e1 h=-0.2", 0xe6628d0f1b864e76),
    ("Block k=1e6 D=1e1 h=0", 0xe8599a32f37f045e),
    ("Block k=1e6 D=1e1 h=0.2", 0xf08cf951b7a8c924),
    ("Block k=1e6 D=1e3 h=-0.2", 0x9e9d21d90278cb7d),
    ("Block k=1e6 D=1e3 h=0", 0xd679f7b5b30fe84c),
    ("Block k=1e6 D=1e3 h=0.2", 0x16ba42bcb1ff09c6),
    ("Block k=1e6 D=1e4 h=-0.2", 0xaa342951dcdd335b),
    ("Block k=1e6 D=1e4 h=0", 0xc1ec5e304acecc4e),
    ("Block k=1e6 D=1e4 h=0.2", 0xd9207199f7341138),
    ("Cylinder k=1e-3 D=0e0 h=-0.3", 0x3f43e59c467117e7),
    ("Cylinder k=1e-3 D=0e0 h=0", 0xca2c69584eb71168),
    ("Cylinder k=1e-3 D=0e0 h=0.3", 0x6eedf84db3d1ed78),
    ("Cylinder k=1e-3 D=1e1 h=-0.3", 0x6b37f666a5a1e6c0),
    ("Cylinder k=1e-3 D=1e1 h=0", 0xdb7deb4aa2e7d0a0),
    ("Cylinder k=1e-3 D=1e1 h=0.3", 0xce03b77c0cc87311),
    ("Cylinder k=1e-3 D=1e3 h=-0.3", 0x8110d86a71dffdcb),
    ("Cylinder k=1e-3 D=1e3 h=0", 0xc214476b870d4634),
    ("Cylinder k=1e-3 D=1e3 h=0.3", 0x47563b27bcd6ef50),
    ("Cylinder k=1e-3 D=1e4 h=-0.3", 0x93cc642aac7d7eff),
    ("Cylinder k=1e-3 D=1e4 h=0", 0xa43018d5bb53427a),
    ("Cylinder k=1e-3 D=1e4 h=0.3", 0x30459eba9042f60f),
    ("Cylinder k=1e0 D=0e0 h=-0.3", 0xaad7cf6ea54c3910),
    ("Cylinder k=1e0 D=0e0 h=0", 0x423d8eb587db01ee),
    ("Cylinder k=1e0 D=0e0 h=0.3", 0x29577a88c84dc12e),
    ("Cylinder k=1e0 D=1e1 h=-0.3", 0xde90a761d02f7078),
    ("Cylinder k=1e0 D=1e1 h=0", 0xed3ddd5460a4944c),
    ("Cylinder k=1e0 D=1e1 h=0.3", 0x7d02b57e4380b5f5),
    ("Cylinder k=1e0 D=1e3 h=-0.3", 0xbf2fb15b9ef5fd4b),
    ("Cylinder k=1e0 D=1e3 h=0", 0xb65c5794313f7993),
    ("Cylinder k=1e0 D=1e3 h=0.3", 0xc1f8a5756ac3f912),
    ("Cylinder k=1e0 D=1e4 h=-0.3", 0x651188355261ea64),
    ("Cylinder k=1e0 D=1e4 h=0", 0x54307ab6a36bb4b2),
    ("Cylinder k=1e0 D=1e4 h=0.3", 0x4f107caae50d2962),
    ("Cylinder k=1e6 D=0e0 h=-0.3", 0x867c44b62ec92bd5),
    ("Cylinder k=1e6 D=0e0 h=0", 0xec0eba31e9527776),
    ("Cylinder k=1e6 D=0e0 h=0.3", 0x7c92b46609e9e9b8),
    ("Cylinder k=1e6 D=1e1 h=-0.3", 0x604faba0b7f5f1d4),
    ("Cylinder k=1e6 D=1e1 h=0", 0x6e6558bda47ec875),
    ("Cylinder k=1e6 D=1e1 h=0.3", 0x365202c8ba88abed),
    ("Cylinder k=1e6 D=1e3 h=-0.3", 0x0c0882fe3b01d850),
    ("Cylinder k=1e6 D=1e3 h=0", 0x5d53df66e07d6955),
    ("Cylinder k=1e6 D=1e3 h=0.3", 0x3e13c32b7050e406),
    ("Cylinder k=1e6 D=1e4 h=-0.3", 0xe8cdd5284b038b70),
    ("Cylinder k=1e6 D=1e4 h=0", 0xdecba7f9762271d0),
    ("Cylinder k=1e6 D=1e4 h=0.3", 0x45c376f34d34e55d),
    ("Torus k=1e-3 D=0e0 h=-0.3", 0xefce9577b3bdffd3),
    ("Torus k=1e-3 D=0e0 h=0", 0x754a9618896691e7),
    ("Torus k=1e-3 D=0e0 h=0.25", 0x848a5785bf89c7d6),
    ("Torus k=1e-3 D=1e1 h=-0.3", 0x4329ea0e338dd1a0),
    ("Torus k=1e-3 D=1e1 h=0", 0x97fbf4c969acf561),
    ("Torus k=1e-3 D=1e1 h=0.25", 0x6a8372d4dd8731cc),
    ("Torus k=1e-3 D=1e3 h=-0.3", 0x996af0cc0c16e517),
    ("Torus k=1e-3 D=1e3 h=0", 0xfcf3b05b7e880d37),
    ("Torus k=1e-3 D=1e3 h=0.25", 0x918c7f583a541136),
    ("Torus k=1e-3 D=1e4 h=-0.3", 0x478bae3df5d86a6d),
    ("Torus k=1e-3 D=1e4 h=0", 0x1c522520733855d1),
    ("Torus k=1e-3 D=1e4 h=0.25", 0x724f58e09996e6db),
    ("Torus k=1e0 D=0e0 h=-0.3", 0x91bc65fdbe718066),
    ("Torus k=1e0 D=0e0 h=0", 0x69692ef7f08bec5f),
    ("Torus k=1e0 D=0e0 h=0.25", 0x5e9bc0880708d7dc),
    ("Torus k=1e0 D=1e1 h=-0.3", 0x8f30a8f9baf7516c),
    ("Torus k=1e0 D=1e1 h=0", 0xd523fe6bfe4429e6),
    ("Torus k=1e0 D=1e1 h=0.25", 0xec38a87bcf0f89c3),
    ("Torus k=1e0 D=1e3 h=-0.3", 0xf87d704a62f7d5f8),
    ("Torus k=1e0 D=1e3 h=0", 0x748bac26d1f663b7),
    ("Torus k=1e0 D=1e3 h=0.25", 0x342648cca363f678),
    ("Torus k=1e0 D=1e4 h=-0.3", 0x6bf528e0d39d08fa),
    ("Torus k=1e0 D=1e4 h=0", 0x2a1b76b9508c0dc6),
    ("Torus k=1e0 D=1e4 h=0.25", 0x5c743aba5fec0a58),
    ("Torus k=1e6 D=0e0 h=-0.3", 0x82a1caab09f8be10),
    ("Torus k=1e6 D=0e0 h=0", 0x6db1f16d230cb318),
    ("Torus k=1e6 D=0e0 h=0.25", 0xf9e4b27cdc009b2a),
    ("Torus k=1e6 D=1e1 h=-0.3", 0x4ed19f7086e08cfb),
    ("Torus k=1e6 D=1e1 h=0", 0xad590219a5257b74),
    ("Torus k=1e6 D=1e1 h=0.25", 0x3674e8bfddcfaa85),
    ("Torus k=1e6 D=1e3 h=-0.3", 0x4a6d49854193e486),
    ("Torus k=1e6 D=1e3 h=0", 0xea4df2b3ab54b9dd),
    ("Torus k=1e6 D=1e3 h=0.25", 0x8d72f3414fe1eb59),
    ("Torus k=1e6 D=1e4 h=-0.3", 0x16b41ffed6ba7476),
    ("Torus k=1e6 D=1e4 h=0", 0x43e65ec25683ee81),
    ("Torus k=1e6 D=1e4 h=0.25", 0xccd49883399662eb),
    ("Cone k=1e-3 D=0e0 h=0.3", 0xde58fa15b808d049),
    ("Cone k=1e-3 D=0e0 h=0.6", 0x3a845419fba0ccf0),
    ("Cone k=1e-3 D=0e0 h=0.9", 0x487bc8b2b498ca46),
    ("Cone k=1e-3 D=1e1 h=0.3", 0xb7194bd8a29066ed),
    ("Cone k=1e-3 D=1e1 h=0.6", 0xd462c615a0f85f62),
    ("Cone k=1e-3 D=1e1 h=0.9", 0x003909aad048b4f2),
    ("Cone k=1e-3 D=1e3 h=0.3", 0xcb3b75c670e9e638),
    ("Cone k=1e-3 D=1e3 h=0.6", 0x331f408574b815e2),
    ("Cone k=1e-3 D=1e3 h=0.9", 0xa843d7dc268415bd),
    ("Cone k=1e0 D=0e0 h=0.3", 0x0817a011cb7a8a62),
    ("Cone k=1e0 D=0e0 h=0.6", 0x4ab7609594d8004c),
    ("Cone k=1e0 D=0e0 h=0.9", 0x1f6715fa0ff962f4),
    ("Cone k=1e0 D=1e1 h=0.3", 0xbf76fadc60656917),
    ("Cone k=1e0 D=1e1 h=0.6", 0x61ab4943302592b9),
    ("Cone k=1e0 D=1e1 h=0.9", 0x36e240db3ed2922a),
    ("Cone k=1e0 D=1e3 h=0.3", 0x116e02dd02e4e620),
    ("Cone k=1e0 D=1e3 h=0.6", 0xb7d2096a227a1d8a),
    ("Cone k=1e0 D=1e3 h=0.9", 0xe251b3b835b47e36),
    ("Cone k=1e6 D=0e0 h=0.3", 0x2d929991f1b6b7e5),
    ("Cone k=1e6 D=0e0 h=0.6", 0xc35d84dc5bccb47c),
    ("Cone k=1e6 D=0e0 h=0.9", 0xe5aaf919812a79a2),
    ("Cone k=1e6 D=1e1 h=0.3", 0x2ca390fbe66f1446),
    ("Cone k=1e6 D=1e1 h=0.6", 0xe98cd2c25749c9a7),
    ("Cone k=1e6 D=1e1 h=0.9", 0xc126ad89f9c3c3a6),
    ("Cone k=1e6 D=1e3 h=0.3", 0x4e5e85d8f04cc494),
    ("Cone k=1e6 D=1e3 h=0.6", 0xa8a4673be19c7521),
    ("Cone k=1e6 D=1e3 h=0.9", 0x740cab63396ad7e2),
];

/// Whether the boolean that cuts off what lies below may refuse the row,
/// before the section is reached: the spline cone 1e7 times its size from
/// the origin, whose box the boolean refuses ("the box has no size, or the
/// tolerance is finer than rounding allows at its size"), before the
/// change as after it (the Check runs 45024256884 and 45025489872): not
/// the section's refusal, and not this test's to lift.
fn the_boolean_refuses(row: &Row, error: &str) -> bool {
    matches!(row.kind, Kind::Cone)
        && row.ratio == 1e7
        && error.contains("cannot be cut off")
        && error.contains("the tolerance is finer than rounding allows")
}

/// Every row measured, within its allowance (see [`errors`]), but where
/// the boolean refuses to cut it ([`the_boolean_refuses`]); every row
/// answered before the change the same to the bit.
#[test]
fn far_sections_are_measured_to_the_rounding_of_their_coordinates() {
    let rows = far_rows();
    print_rows(&rows);
    let mut wrong = Vec::new();
    for row in &rows {
        let (found, at) = match &row.found {
            Ok(found) => found,
            Err(e) if the_boolean_refuses(row, e) => continue,
            Err(e) => {
                wrong.push(format!("{}: refused: {e}", row.name));
                continue;
            }
        };
        for (what, share, _) in errors(row, found, at).each {
            if share.is_nan() || share > 1.0 {
                wrong.push(format!(
                    "{}: {what} off by {share:e} of what it is allowed",
                    row.name
                ));
            }
        }
        if let Some((_, pin)) = FAR_PINS.iter().find(|(name, _)| *name == row.name)
            && below_hash(found) != *pin
        {
            wrong.push(format!("{}: not what it was before the change", row.name));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

// ------------------------------------------------------ today's answers

fn upright() -> CutPlane {
    CutPlane::new(Vec3::Z, Vec3::X).unwrap()
}

/// The plane turned by `angle` about its x axis.
fn turned(plane: &CutPlane, angle: f64) -> CutPlane {
    let (s, c) = angle.psin_cos();
    CutPlane::new(
        plane.normal * c + plane.x_axis.cross(plane.normal) * s,
        plane.x_axis,
    )
    .unwrap()
}

fn up_at(x: f64, y: f64, z: f64) -> Frame {
    Frame::new(Vec3::new(x, y, z), Vec3::Z, Vec3::X).unwrap()
}

/// A box L × B × D centred on the z axis, its bottom at z = 0.
fn boxed(k: f64, (l, b, d): (f64, f64, f64)) -> Body {
    block(&up_at(-0.5 * l * k, -0.5 * b * k, 0.0), l * k, b * k, d * k).unwrap()
}

/// The oracles of `below/tests.rs` and `below/float/tests.rs`, measured
/// through the public functions: each a name and the hash of every number
/// it gives.
fn todays_answers() -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = Vec::new();
    let mut at = |name: String, h: u64| out.push((name, h));
    for k in SCALES {
        let t = tol(k);
        // A box 4 × 1 × 1: below 0.3, and the level for its volume.
        let body = boxed(k, (4.0, 1.0, 1.0));
        at(
            format!("box k={k:e}"),
            below_hash(&below(&[&body], &upright(), 0.3 * k, &t).unwrap()),
        );
        at(
            format!("box level k={k:e}"),
            level_hash(&level_for(&[&body], &upright(), 1.2 * k.ppowi(3), &t).unwrap()),
        );
        // Under turned planes, the level for the same volume.
        for degrees in [5.0_f64, 10.0, -10.0] {
            let plane = turned(&upright(), degrees.to_radians());
            at(
                format!("box turned {degrees} level k={k:e}"),
                level_hash(&level_for(&[&body], &plane, 1.6 * k.ppowi(3), &t).unwrap()),
            );
        }
        // The same box moved and turned in space.
        let turn = Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 1.1).unwrap();
        let frame = Frame::new(
            turn.point(Vec3::new(-2.0 * k, -0.5 * k, 0.0)),
            turn.vector(Vec3::Z),
            turn.vector(Vec3::X),
        )
        .unwrap();
        let moved = block(&frame, 4.0 * k, k, k).unwrap();
        let plane = CutPlane::new(turn.vector(Vec3::Z), turn.vector(Vec3::X)).unwrap();
        at(
            format!("box moved k={k:e}"),
            below_hash(&below(&[&moved], &plane, 0.3 * k, &t).unwrap()),
        );
        // A sphere at three levels, and along (1, 1, 1).
        let ball = sphere(&Frame::WORLD, k).unwrap();
        for h in [-0.5, 0.0, 0.3] {
            at(
                format!("sphere h={h} k={k:e}"),
                below_hash(&below(&[&ball], &upright(), h * k, &t).unwrap()),
            );
        }
        let oblique = CutPlane::new(Vec3::new(1.0, 1.0, 1.0), Vec3::new(1.0, -1.0, 0.0)).unwrap();
        at(
            format!("sphere oblique k={k:e}"),
            below_hash(&below(&[&ball], &oblique, -0.9 * k, &t).unwrap()),
        );
        // A horizontal cylinder at two levels.
        let lying = cylinder(
            &Frame::new(Vec3::new(-1.5 * k, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap(),
            k,
            3.0 * k,
        )
        .unwrap();
        for h in [-0.2, 0.4] {
            at(
                format!("lying cylinder h={h} k={k:e}"),
                below_hash(&below(&[&lying], &upright(), h * k, &t).unwrap()),
            );
        }
        // A V prism.
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let v = block(
            &Frame::new(Vec3::new(-k, 0.0, 0.0), Vec3::new(0.0, -s, s), Vec3::X).unwrap(),
            2.0 * k,
            k,
            k,
        )
        .unwrap();
        at(
            format!("v prism k={k:e}"),
            below_hash(&below(&[&v], &upright(), 0.4 * k, &t).unwrap()),
        );
        // A spline cone apex down, and the level for its volume.
        let cone = craft3d_testkit::solids::spline_cone(
            &Frame::new(Vec3::ZERO, Vec3::X, Vec3::Y).unwrap(),
            0.5 * k,
            k,
        )
        .unwrap()
        .body;
        let cut = below(&[&cone], &upright(), 0.6 * k, &t).unwrap();
        at(format!("spline cone k={k:e}"), below_hash(&cut));
        at(
            format!("spline cone level k={k:e}"),
            level_hash(&level_for(&[&cone], &upright(), cut.volume, &t).unwrap()),
        );
        // A rectangle turned in the plane.
        let (sa, ca) = 0.4_f64.psin_cos();
        let slab = block(
            &Frame::new(Vec3::new(0.0, 0.0, -k), Vec3::Z, Vec3::new(ca, sa, 0.0)).unwrap(),
            3.0 * k,
            k,
            2.0 * k,
        )
        .unwrap();
        at(
            format!("turned rectangle k={k:e}"),
            below_hash(&below(&[&slab], &upright(), 0.0, &t).unwrap()),
        );
        // A thin rod tilted and cut near its end.
        let alpha = 0.5_f64;
        let axis = Vec3::new(alpha.psin(), 0.0, alpha.pcos());
        let rod = cylinder(
            &Frame::new(Vec3::ZERO, axis, Vec3::Y).unwrap(),
            k,
            1000.0 * k,
        )
        .unwrap();
        at(
            format!("thin rod k={k:e}"),
            below_hash(&below(&[&rod], &upright(), 5.0 * k, &t).unwrap()),
        );
        // A tube: the annulus.
        let outer = cylinder(&Frame::WORLD, 2.0 * k, 3.0 * k).unwrap();
        let inner = cylinder(&up_at(0.0, 0.0, -k), k, 5.0 * k).unwrap();
        let tube = boolean(&outer, &inner, BooleanOp::Subtract, &t)
            .unwrap()
            .body;
        at(
            format!("tube k={k:e}"),
            below_hash(&below(&[&tube], &upright(), 1.5 * k, &t).unwrap()),
        );
        // A U: two section faces.
        let solid = block(&up_at(-2.0 * k, -1.5 * k, 0.0), 4.0 * k, 3.0 * k, 3.0 * k).unwrap();
        let slot = block(&up_at(-k, -3.0 * k, k), 2.0 * k, 6.0 * k, 3.0 * k).unwrap();
        let u = boolean(&solid, &slot, BooleanOp::Subtract, &t)
            .unwrap()
            .body;
        at(
            format!("u k={k:e}"),
            below_hash(&below(&[&u], &upright(), 2.0 * k, &t).unwrap()),
        );
        // A stepped block: the level on its shoulder.
        let base = block(&up_at(0.0, 0.0, 0.0), 2.0 * k, k, k).unwrap();
        let top = block(&up_at(0.0, 0.0, k), k, k, k).unwrap();
        let stepped = boolean(&base, &top, BooleanOp::Union, &t).unwrap().body;
        at(
            format!("stepped level k={k:e}"),
            level_hash(&level_for(&[&stepped], &upright(), 2.0 * k.ppowi(3), &t).unwrap()),
        );
        // A torus with its axis tilted, cut near its bottom.
        let tilted = Vec3::new(0.4_f64.psin(), 0.0, 0.4_f64.pcos());
        let ring = torus(
            &Frame::new(Vec3::ZERO, tilted, Vec3::Y).unwrap(),
            2.0 * k,
            0.5 * k,
        )
        .unwrap();
        let (lowest, _, _) = craft3d_kernel::below::extent(&[&ring], Vec3::Z).unwrap();
        for f in [1e-2, 0.5, 1.5] {
            at(
                format!("torus near bottom {f} k={k:e}"),
                below_hash(&below(&[&ring], &upright(), lowest + f * 0.5 * k, &t).unwrap()),
            );
        }
        // A prism on an edge 1e4 from the origin, cut by a plane tilted
        // 1e-8 (a far section that settles strictly).
        let theta: f64 = 1e-8;
        let (sine, cosine) = theta.psin_cos();
        let near_z = CutPlane::new(Vec3::new(0.0, sine, cosine), Vec3::X).unwrap();
        let prism = block(
            &Frame::new(Vec3::new(-k, -1e4 * k, 0.0), Vec3::new(0.0, -s, s), Vec3::X).unwrap(),
            2.0 * k,
            k,
            k,
        )
        .unwrap();
        let edge = Vec3::new(0.0, -1e4 * k, 0.0).dot(near_z.normal);
        at(
            format!("far prism k={k:e}"),
            below_hash(&below(&[&prism], &near_z, 0.5 * edge, &t).unwrap()),
        );
        // The floating equilibria.
        let barge = boxed(k, (10.0, 2.0, 2.0));
        for (kg, g) in [(0.5, 0.0), (0.9, 0.0), (0.5, 0.1)] {
            let centre = Vec3::new(0.0, g * k, kg * k);
            at(
                format!("floating box kg={kg} g={g} k={k:e}"),
                floating_hash(
                    &floating(&[&barge], &upright(), 20.0 * k.ppowi(3), centre, &t).unwrap(),
                ),
            );
        }
        let (e_off, roll) = (0.3 * k, 30f64.to_radians());
        let centre = Vec3::new(0.0, e_off * roll.psin(), -e_off * roll.pcos());
        let lying4 = cylinder(
            &Frame::new(Vec3::new(-2.0 * k, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap(),
            k,
            4.0 * k,
        )
        .unwrap();
        at(
            format!("floating cylinder k={k:e}"),
            floating_hash(
                &floating(&[&lying4], &upright(), 2.0 * PI * k.ppowi(3), centre, &t).unwrap(),
            ),
        );
        let whole = 4.0 / 3.0 * PI * k.ppowi(3);
        for centre in [Vec3::ZERO, Vec3::new(0.0, 0.2 * k, -0.3 * k)] {
            at(
                format!("floating sphere {:?} k={k:e}", centre * (1.0 / k)),
                floating_hash(&floating(&[&ball], &upright(), 0.3 * whole, centre, &t).unwrap()),
            );
        }
    }
    out
}

/// What each of [`todays_answers`] gave before the change (`main`
/// 5305636e, printed by the Check run 45024256884 on the probe commit
/// 31d39767): the hash of every number of it. The torus near its bottom at
/// 1 and 1e6 since 2026-11-25, as [`FAR_PINS`] says: its axis leans 0.4
/// rad from z, the lowest point of its box moved in its last bits with the
/// box's reach, and the cuts are made from it.
const PINS: &[(&str, u64)] = &[
    ("box k=1e-3", 0x7ca715e251251f39),
    ("box level k=1e-3", 0x741c327413effae8),
    ("box turned 5 level k=1e-3", 0x6b7d12235e2c0c6e),
    ("box turned 10 level k=1e-3", 0x573df09c92246bd4),
    ("box turned -10 level k=1e-3", 0x27affe7d367e0b40),
    ("box moved k=1e-3", 0xd565b08b681385bf),
    ("sphere h=-0.5 k=1e-3", 0x10d6f7575f32177c),
    ("sphere h=0 k=1e-3", 0x32091dcdb6ff38d4),
    ("sphere h=0.3 k=1e-3", 0x352b73979d0968c3),
    ("sphere oblique k=1e-3", 0x81990cfb7853fded),
    ("lying cylinder h=-0.2 k=1e-3", 0xcd451e33c09e259c),
    ("lying cylinder h=0.4 k=1e-3", 0xf4a7933655b2e394),
    ("v prism k=1e-3", 0xc775740d98a5971d),
    ("spline cone k=1e-3", 0x3575184e3c868e8c),
    ("spline cone level k=1e-3", 0x3a4ae4f76008f8b8),
    ("turned rectangle k=1e-3", 0xdc745891d734e45b),
    ("thin rod k=1e-3", 0xe06a64180024a2e9),
    ("tube k=1e-3", 0x85f4c91fa964d010),
    ("u k=1e-3", 0x24434a1f5a4aacb6),
    ("stepped level k=1e-3", 0x0df6f61808fe5ab1),
    ("torus near bottom 0.01 k=1e-3", 0xc7877df24c1571d7),
    ("torus near bottom 0.5 k=1e-3", 0xa6b9d60423526f99),
    ("torus near bottom 1.5 k=1e-3", 0xb5769ea10348f3ce),
    ("far prism k=1e-3", 0x500f690175d0d658),
    ("floating box kg=0.5 g=0 k=1e-3", 0x6a6d66870390447d),
    ("floating box kg=0.9 g=0 k=1e-3", 0x901f857574577479),
    ("floating box kg=0.5 g=0.1 k=1e-3", 0x26002d7397affdd4),
    ("floating cylinder k=1e-3", 0x79d170128e6d8566),
    (
        "floating sphere Vec3 { x: 0.0, y: 0.0, z: 0.0 } k=1e-3",
        0x623630fbde1212e4,
    ),
    (
        "floating sphere Vec3 { x: 0.0, y: 0.2, z: -0.3 } k=1e-3",
        0x1654599d1f7568b5,
    ),
    ("box k=1e0", 0xf1012040d146c6fc),
    ("box level k=1e0", 0xbab7b0d3a7b7b783),
    ("box turned 5 level k=1e0", 0x35a904f98e4a6f85),
    ("box turned 10 level k=1e0", 0x6c1b606a6bcaa7a9),
    ("box turned -10 level k=1e0", 0x7f5d62ec67206d20),
    ("box moved k=1e0", 0x932e32a6e5b81039),
    ("sphere h=-0.5 k=1e0", 0x6f93e0418be5325d),
    ("sphere h=0 k=1e0", 0x323649e13ed3e88b),
    ("sphere h=0.3 k=1e0", 0x4eb34c90d2aea556),
    ("sphere oblique k=1e0", 0x2d8877a18ba61ce8),
    ("lying cylinder h=-0.2 k=1e0", 0x591576cbafe08165),
    ("lying cylinder h=0.4 k=1e0", 0x24da5f2964f58c16),
    ("v prism k=1e0", 0xc69b886513a1220e),
    ("spline cone k=1e0", 0xa20a3577315b2602),
    ("spline cone level k=1e0", 0x0071e3d0c7eb17ec),
    ("turned rectangle k=1e0", 0x907714ac87226642),
    ("thin rod k=1e0", 0xb842520591c8ab45),
    ("tube k=1e0", 0xc347bfc2c4e5cb1a),
    ("u k=1e0", 0x7acf0e97627193f8),
    ("stepped level k=1e0", 0x0f9781cb76543ef5),
    ("torus near bottom 0.01 k=1e0", 0x2877307a980ee2ae),
    ("torus near bottom 0.5 k=1e0", 0x9036027448dd2128),
    ("torus near bottom 1.5 k=1e0", 0x2b85948129ba9f42),
    ("far prism k=1e0", 0x9f890d6f3e8b3bed),
    ("floating box kg=0.5 g=0 k=1e0", 0xd553971c1023c7b5),
    ("floating box kg=0.9 g=0 k=1e0", 0x48e9907afa925056),
    ("floating box kg=0.5 g=0.1 k=1e0", 0x2dfe4c75454f7995),
    ("floating cylinder k=1e0", 0xa58fadb6bcde443d),
    (
        "floating sphere Vec3 { x: 0.0, y: 0.0, z: 0.0 } k=1e0",
        0xa730d44a9f71f017,
    ),
    (
        "floating sphere Vec3 { x: 0.0, y: 0.2, z: -0.3 } k=1e0",
        0xf66c6e6c274944f7,
    ),
    ("box k=1e6", 0x48347f7af74f9be1),
    ("box level k=1e6", 0x8eb126b7a0f35d90),
    ("box turned 5 level k=1e6", 0x44a913b1f310c2ed),
    ("box turned 10 level k=1e6", 0x80ddd2c9dad3ca80),
    ("box turned -10 level k=1e6", 0x5dd147d8816f8461),
    ("box moved k=1e6", 0xc861a7a91fa5f8d6),
    ("sphere h=-0.5 k=1e6", 0x10e7bfa27ad6a2b5),
    ("sphere h=0 k=1e6", 0x67692cca66478d7a),
    ("sphere h=0.3 k=1e6", 0xe64fdd93b03e3ced),
    ("sphere oblique k=1e6", 0x60ab2b15f4a06e6c),
    ("lying cylinder h=-0.2 k=1e6", 0x3f93fd94176eb375),
    ("lying cylinder h=0.4 k=1e6", 0x360cc2eb2d00c3cf),
    ("v prism k=1e6", 0x7b6ae03b83d9650f),
    ("spline cone k=1e6", 0xd268020c9f00f2ac),
    ("spline cone level k=1e6", 0x2971e6a8a39ec7ed),
    ("turned rectangle k=1e6", 0x99ce15786cc3e633),
    ("thin rod k=1e6", 0xe2afc45891d93700),
    ("tube k=1e6", 0x66a715b93cd435aa),
    ("u k=1e6", 0x87f3e7973b752d53),
    ("stepped level k=1e6", 0xa9932015e603cf11),
    ("torus near bottom 0.01 k=1e6", 0x345f0001c034b8b0),
    ("torus near bottom 0.5 k=1e6", 0x0342b3a16385d06e),
    ("torus near bottom 1.5 k=1e6", 0x68423c8098d418c2),
    ("far prism k=1e6", 0x0ae1f8be78cb5836),
    ("floating box kg=0.5 g=0 k=1e6", 0xe0167398fb99e39c),
    ("floating box kg=0.9 g=0 k=1e6", 0xdda8cc0eb9ffef3e),
    ("floating box kg=0.5 g=0.1 k=1e6", 0xedbd87f3419c3a48),
    ("floating cylinder k=1e6", 0x04d8b2cfc1b8c98f),
    (
        "floating sphere Vec3 { x: 0.0, y: 0.0, z: 0.0 } k=1e6",
        0x3ea3396d7afb3d2c,
    ),
    (
        "floating sphere Vec3 { x: 0.0, y: 0.19999999999999998, z: -0.3 } k=1e6",
        0x118fcda9f026fccf,
    ),
];

#[test]
fn todays_answers_are_kept_to_the_bit() {
    let found = todays_answers();
    let names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
    let pinned: Vec<&str> = PINS.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, pinned, "the same cases");
    let moved: Vec<String> = found
        .iter()
        .zip(PINS)
        .filter(|((_, h), (_, pin))| h != pin)
        .map(|((name, h), (_, pin))| format!("{name}: 0x{h:016x}, before 0x{pin:016x}"))
        .collect();
    assert!(moved.is_empty(), "{moved:#?}");
}
