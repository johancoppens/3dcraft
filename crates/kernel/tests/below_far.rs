//! A section small against its distance from the origin (fault M, fuzz
//! seed 138822; 2026-11-22; docs/design/sections-and-levels.md).
//!
//! Five bodies with closed forms, a sphere (the spherical cap and its
//! circle), a block cut aslant (a rectangle), a cylinder cut aslant (an
//! ellipse), a torus cut square to its axis (an annulus, Pappus) and a
//! spline cone (freeform, rule 3), each turned in space and placed at 0,
//! 10, 1e3, 1e4, 1e5 and 1e7 times its size from the origin, at the scales
//! 1e-3, 1 and 1e6 with the tolerance scaled along, cut at three offsets:
//! every one is measured, to its closed form within 1e-9 of its size to
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

#[test]
fn far_table() {
    let rows = far_rows();
    print_rows(&rows);
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

#[test]
fn todays_answers_table() {
    for (name, h) in todays_answers() {
        println!("PIN (\"{name}\", 0x{h:016x}),");
    }
}
