//! Three findings of the plate and sheet-metal hunt, each held by a sweep
//! of its family (docs/design/plates.md, sheetmetal.md, developable.md,
//! 2026-11-15):
//!
//! - **A plate of faces that close on themselves** (no boundary: a whole
//!   sphere or torus, a sphere or a ring turned from a profile with its
//!   seams, a capsule, a ring of rounded section, a ring turned from a
//!   closed spline) is a hollow: two closed shells, the outside and a
//!   cavity. Each is held to its closed form (Pappus on the band between
//!   the profile and its parallel curve: `2π ∫∫ x dA`, the band's moment in
//!   closed form or by Gauss–Legendre on the exact spline), the checker,
//!   one lump with one cavity, the genus of each shell, the measure's
//!   bound and a closed mesh of the volume; on every side, at thicknesses
//!   up to and past the one where a skin vanishes (refused, saying so), at
//!   three scales, in turned and moved frames.
//! - **The development of a cylinder's or a cone's side** in a frame
//!   turned off the world's axes, at scales from 1e-3 to 1e9: developed,
//!   its area the closed form.
//! - **Sheet-metal bends from 1e-6 to 179 degrees** at sizes from 1e-3 to
//!   1e7: the box `Body::bbox` gives against the box of the part's edges
//!   sampled, with the closed-form volume, the checker and the mesh.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod hunt_sheet_common;

use boolean_common::Rng;
use craft3d_geom::Surface;
use craft3d_kernel::lumps::{Lump, lumps};
use craft3d_kernel::plate::{Side, plate};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_kernel::sheetmetal::{Closing, Direction, FlangeSpec, LengthFrom, On, Spec, build};
use craft3d_kernel::unfold::develop;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{Body, FaceId, ShellId, check, mass_bound, mass_properties, shell_genera};
use hunt_sheet_common::{P2, bspline, case_seed, mesh_volume, tolerance};
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// A frame at scale `k`: the world, or turned and moved some way off.
fn turned(rng: &mut Rng, k: f64, far: f64) -> Frame {
    let origin = Vec3::new(
        rng.between(-far, far) * k,
        rng.between(-far, far) * k,
        rng.between(-far, far) * k,
    );
    let z = Vec3::new(
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
    );
    let x = Vec3::new(
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
    );
    Frame::new(origin, z, x).unwrap_or(Frame::WORLD)
}

fn frames(seed: u64, k: f64) -> Vec<Frame> {
    let mut rng = Rng(seed);
    vec![
        Frame::WORLD,
        turned(&mut rng, k, 0.0),
        turned(&mut rng, k, 200.0),
    ]
}

// ---------------------------------------------------------------------
// Faces that close on themselves.
// ---------------------------------------------------------------------

fn line(id: u64, start: P2, end: P2) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

fn arc(id: u64, center: P2, start: P2, end: P2) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Arc { center, start, end },
    }
}

/// The closed cubic of [`spline_ring`]: ten control points about
/// `(c, 0)`, clamped, its first and last the same point and the
/// neighbours of it in line, so the loop is tangent where it closes;
/// counter-clockwise.
fn spline_loop(c: f64, a: f64, b: f64, h: f64) -> (Vec<f64>, Vec<P2>) {
    let control = vec![
        [c + a, 0.0],
        [c + a, h],
        [c + h, b],
        [c - h, b],
        [c - a, h],
        [c - a, -h],
        [c - h, -b],
        [c + h, -b],
        [c + a, -h],
        [c + a, 0.0],
    ];
    let mut knots = vec![0.0; 4];
    for i in 1..=6 {
        knots.push(f64::from(i) / 7.0);
    }
    knots.extend([1.0; 4]);
    (knots, control)
}

/// The nodes and weights of the Gauss–Legendre rule of `n` points on
/// [0, 1] (Newton on the Legendre polynomial).
fn gauss(n: usize) -> Vec<(f64, f64)> {
    (0..n)
        .map(|i| {
            let mut x = (PI * (i as f64 + 0.75) / (n as f64 + 0.5)).pcos();
            let mut dp = 1.0;
            for _ in 0..100 {
                let (mut p0, mut p1) = (1.0, x);
                for k in 2..=n {
                    let p2 = ((2 * k - 1) as f64 * x * p1 - (k - 1) as f64 * p0) / k as f64;
                    p0 = p1;
                    p1 = p2;
                }
                dp = n as f64 * (x * p1 - p0) / (x * x - 1.0);
                let step = p1 / dp;
                x -= step;
                if step.abs() < 1e-16 {
                    break;
                }
            }
            (0.5 * (1.0 - x), 1.0 / ((1.0 - x * x) * dp * dp))
        })
        .collect()
}

/// `∫∫ x dA` over the band between the closed counter-clockwise spline and
/// its parallel curves at `lo` and `hi` along its outward normal: per point
/// of the curve `x (hi − lo) + (n_x + x κ)(hi² − lo²)/2 + n_x κ (hi³ − lo³)/3`
/// times the speed (the band's area element is `(1 + h κ) ds dh`), by
/// Gauss–Legendre on each span of the exact spline.
fn spline_band_moment(knots: &[f64], control: &[P2], (lo, hi): (f64, f64)) -> f64 {
    let n = control.len();
    // The derivative's control points, a quadratic on the inner knots.
    let d1: Vec<P2> = (0..n - 1)
        .map(|i| {
            let s = 3.0 / (knots[i + 4] - knots[i + 1]);
            [
                s * (control[i + 1][0] - control[i][0]),
                s * (control[i + 1][1] - control[i][1]),
            ]
        })
        .collect();
    let k1 = &knots[1..knots.len() - 1];
    let rule = gauss(24);
    let mut sum = 0.0;
    for span in knots.windows(2) {
        let (a, b) = (span[0], span[1]);
        if b <= a {
            continue;
        }
        for piece in 0..8 {
            let (pa, pb) = (
                a + (b - a) * f64::from(piece) / 8.0,
                a + (b - a) * f64::from(piece + 1) / 8.0,
            );
            for &(x, w) in &rule {
                let u = pa + (pb - pa) * x;
                let (p, d) = bspline(3, knots, control, u);
                let (_, dd) = bspline(2, k1, &d1, u);
                let speed = d[0].phypot(d[1]);
                let kappa = (d[0] * dd[1] - d[1] * dd[0]) / (speed * speed * speed);
                let nx = d[1] / speed;
                let f = p[0] * (hi - lo)
                    + (nx + p[0] * kappa) * (hi * hi - lo * lo) / 2.0
                    + nx * kappa * (hi * hi * hi - lo * lo * lo) / 3.0;
                sum += w * (pb - pa) * f * speed;
            }
        }
    }
    sum
}

/// A family of faces that close on themselves, at scale `k` in `frame`.
struct Closed {
    name: &'static str,
    body: Body,
    /// Every face of the body is plated.
    faces: Vec<FaceId>,
    /// The thickness, by side, at which a skin vanishes (`None`: none).
    limit: fn(Side, f64) -> Option<f64>,
    /// The closed-form volume of the plate at the levels `(lo, hi)`.
    volume: Box<dyn Fn((f64, f64)) -> f64>,
    /// Genus of each shell.
    genus: u32,
    /// Freeform: its skins are fits within a quarter of the tolerance.
    freeform: bool,
    /// The length that sets its size.
    size: f64,
}

fn revolved(curves: &[SketchCurve], frame: &Frame, tol: &Tolerance) -> Body {
    let profiles = detect_profiles(curves, tol).unwrap();
    revolve(&profiles, frame, [0.0, 0.0], [0.0, 1.0], 0.0, TAU, tol)
        .unwrap()
        .body
}

fn all_faces(body: &Body) -> Vec<FaceId> {
    (0..body.faces.len() as u32).map(FaceId).collect()
}

fn closed_shapes(k: f64, frame: &Frame, tol: &Tolerance) -> Vec<Closed> {
    let ball =
        |(lo, hi): (f64, f64), r: f64| 4.0 / 3.0 * PI * ((r + hi).ppowi(3) - (r + lo).ppowi(3));
    let ring = |(lo, hi): (f64, f64), big: f64, a: f64| {
        2.0 * PI * PI * big * ((a + hi).ppowi(2) - (a + lo).ppowi(2))
    };
    let (r, big, a) = (10.0 * k, 20.0 * k, 5.0 * k);
    let mut out = Vec::new();
    let sphere = primitives::sphere(frame, r).unwrap();
    out.push(Closed {
        name: "sphere",
        faces: all_faces(&sphere),
        body: sphere,
        limit: |side, r| match side {
            Side::In => Some(r),
            Side::Mid => Some(2.0 * r),
            _ => None,
        },
        volume: Box::new(move |l| ball(l, r)),
        genus: 0,
        freeform: false,
        size: r,
    });
    let torus = primitives::torus(frame, big, a).unwrap();
    out.push(Closed {
        name: "torus",
        faces: all_faces(&torus),
        body: torus,
        // The tube's radius 5 and what is left to the axis, 15.
        limit: |side, a| match side {
            Side::In => Some(a),
            Side::Out => Some(3.0 * a),
            Side::Mid => Some(2.0 * a),
            Side::Levels(..) => None,
        },
        volume: Box::new(move |l| ring(l, big, a)),
        genus: 1,
        freeform: false,
        size: a,
    });
    // A half disc turned about the axis it stands on: a sphere with a
    // seam and two poles.
    let half = revolved(
        &[
            arc(1, [0.0, 0.0], [0.0, -r], [0.0, r]),
            line(2, [0.0, r], [0.0, -r]),
        ],
        frame,
        tol,
    );
    out.push(Closed {
        name: "turned sphere",
        faces: half
            .faces
            .iter()
            .enumerate()
            .filter(|(_, f)| matches!(f.surface, Surface::Sphere(_)))
            .map(|(i, _)| FaceId(i as u32))
            .collect(),
        body: half,
        limit: |side, r| match side {
            Side::In => Some(r),
            Side::Mid => Some(2.0 * r),
            _ => None,
        },
        volume: Box::new(move |l| ball(l, r)),
        genus: 0,
        freeform: false,
        size: r,
    });
    // A circle turned about an axis beside it: a torus with two seams.
    let circle = revolved(
        &[SketchCurve {
            id: 1,
            curve: Curve2::Circle {
                center: [big, 0.0],
                radius: a,
            },
        }],
        frame,
        tol,
    );
    out.push(Closed {
        name: "turned torus",
        faces: all_faces(&circle),
        body: circle,
        limit: |side, a| match side {
            Side::In => Some(a),
            Side::Out => Some(3.0 * a),
            Side::Mid => Some(2.0 * a),
            Side::Levels(..) => None,
        },
        volume: Box::new(move |l| ring(l, big, a)),
        genus: 1,
        freeform: false,
        size: a,
    });
    // A capsule: two half spheres of radius r and a cylinder 2r long,
    // tangent along two circles; three faces.
    let half_l = r;
    let capsule = revolved(
        &[
            arc(1, [0.0, -half_l], [0.0, -half_l - r], [r, -half_l]),
            line(2, [r, -half_l], [r, half_l]),
            arc(3, [0.0, half_l], [r, half_l], [0.0, half_l + r]),
            line(4, [0.0, half_l + r], [0.0, -half_l - r]),
        ],
        frame,
        tol,
    );
    out.push(Closed {
        name: "capsule",
        faces: capsule
            .faces
            .iter()
            .enumerate()
            .filter(|(_, f)| !matches!(f.surface, Surface::Plane(_)))
            .map(|(i, _)| FaceId(i as u32))
            .collect(),
        body: capsule,
        limit: |side, r| match side {
            Side::In => Some(r),
            Side::Mid => Some(2.0 * r),
            _ => None,
        },
        volume: Box::new(move |(lo, hi)| {
            PI * ((r + hi).ppowi(2) - (r + lo).ppowi(2)) * 2.0 * half_l + ball((lo, hi), r)
        }),
        genus: 0,
        freeform: false,
        size: r,
    });
    // A ring of rounded rectangular section: the rectangle 2·6 × 2·4 about
    // (30, 0) with corners of radius 2, turned: annuli, cylinders and tori,
    // tangent along circles.
    let (c, ha, hb, rho) = (30.0 * k, 6.0 * k, 4.0 * k, 2.0 * k);
    let (x0, x1, y0, y1) = (c - ha, c + ha, -hb, hb);
    let rounded = revolved(
        &[
            line(1, [x0 + rho, y0], [x1 - rho, y0]),
            arc(2, [x1 - rho, y0 + rho], [x1 - rho, y0], [x1, y0 + rho]),
            line(3, [x1, y0 + rho], [x1, y1 - rho]),
            arc(4, [x1 - rho, y1 - rho], [x1, y1 - rho], [x1 - rho, y1]),
            line(5, [x1 - rho, y1], [x0 + rho, y1]),
            arc(6, [x0 + rho, y1 - rho], [x0 + rho, y1], [x0, y1 - rho]),
            line(7, [x0, y1 - rho], [x0, y0 + rho]),
            arc(8, [x0 + rho, y0 + rho], [x0, y0 + rho], [x0 + rho, y0]),
        ],
        frame,
        tol,
    );
    let area = move |h: f64| 4.0 * (ha + h) * (hb + h) - (4.0 - PI) * (rho + h).ppowi(2);
    out.push(Closed {
        name: "rounded ring",
        faces: all_faces(&rounded),
        body: rounded,
        // The corners' radius 2; outward the inner wall nears the axis at
        // 24.
        limit: |side, rho| match side {
            Side::In => Some(rho),
            Side::Mid => Some(2.0 * rho),
            Side::Out => Some(12.0 * rho),
            Side::Levels(..) => None,
        },
        volume: Box::new(move |(lo, hi)| 2.0 * PI * c * (area(hi) - area(lo))),
        genus: 1,
        freeform: false,
        size: rho,
    });
    // A closed cubic turned about an axis beside it: freeform faces closed
    // both ways.
    let (knots, control) = spline_loop(30.0 * k, 6.0 * k, 4.0 * k, 2.0 * k);
    let curve = Spline2 {
        degree: 3,
        knots: knots.clone(),
        control: control.clone(),
    };
    let spline = revolved(
        &[SketchCurve {
            id: 1,
            curve: Curve2::Spline(curve),
        }],
        frame,
        tol,
    );
    out.push(Closed {
        name: "spline ring",
        faces: all_faces(&spline),
        body: spline,
        // Kept well inside the least radius of curvature (about 2.3).
        limit: |side, s| match side {
            Side::In => Some(s),
            Side::Mid => Some(2.0 * s),
            Side::Out => Some(2.0 * s),
            Side::Levels(..) => None,
        },
        volume: Box::new(move |l| 2.0 * PI * spline_band_moment(&knots, &control, l)),
        genus: 1,
        freeform: true,
        size: k,
    });
    out
}

#[derive(Default, Debug)]
struct Tally {
    right: usize,
    refused_past: usize,
    slivers: Vec<String>,
    wrong: Vec<String>,
}

/// Plates of faces that close on themselves on every side, at thicknesses
/// up to and past the one where a skin vanishes.
fn whole(names: &[&str]) -> Tally {
    let mut tally = Tally::default();
    let mut lines = Vec::new();
    for (si, &k) in SCALES.iter().enumerate() {
        let tol = tolerance(k);
        for (fi, frame) in frames(case_seed(0x5eed_0b0d, si), k).iter().enumerate() {
            for shape in closed_shapes(k, frame, &tol) {
                if !names.contains(&shape.name) {
                    continue;
                }
                for side in [Side::Out, Side::In, Side::Mid] {
                    let limit = (shape.limit)(side, shape.size);
                    let reference = limit.unwrap_or(shape.size);
                    // The spline ring is plated thin only (its offsets are
                    // fits): a twentieth of its size and less.
                    let shares: &[f64] = if shape.freeform {
                        &[0.01, 0.05]
                    } else {
                        &[0.01, 0.3, 0.9, 0.999, 1.0, 1.2]
                    };
                    // How far a skin moves per thickness (half of it on
                    // each side for `Mid`), so that the skin left at the
                    // limit is `(l − t) · per`.
                    let per = if matches!(side, Side::Mid) { 0.5 } else { 1.0 };
                    let mut thicknesses: Vec<(String, f64)> = shares
                        .iter()
                        .map(|s| (format!("{s} of {reference}"), reference * s))
                        .collect();
                    // Beside the limit: a skin 3 tolerances in size left,
                    // and one of half the tolerance (refused).
                    if let (Some(l), false) = (limit, shape.freeform) {
                        for left in [3.0, 0.5] {
                            thicknesses.push((
                                format!("{left} tolerances short of {l}"),
                                l - left * tol.linear / per,
                            ));
                        }
                    }
                    for (said, t) in thicknesses {
                        let what = format!("{} k {k} frame {fi} {side:?} t {said}", shape.name);
                        let made =
                            plate(&shape.body, &shape.faces, t, side, &BTreeMap::new(), &tol);
                        // Past the limit, or within the tolerance of it.
                        let past = limit.is_some_and(|l| (l - t) * per <= tol.linear);
                        // A skin a thousandth of its size or less from
                        // vanishing: a sliver (a torus whose tube is a few
                        // tolerances thick), valid, but which the
                        // tessellator, the measure or the sorting into lumps
                        // may not take (owed to them: docs/design/plates.md
                        // › *Limits*).
                        let sliver = limit.is_some_and(|l| (l - t) <= 2e-3 * l);
                        match made {
                            Err(e) if past => {
                                let said = e.to_string();
                                if said.contains("vanishes") {
                                    tally.refused_past += 1;
                                } else {
                                    tally.wrong.push(format!(
                                        "{what}: refused, but not saying why: {said}"
                                    ));
                                }
                            }
                            Err(e) => tally.wrong.push(format!("{what}: refused: {e}")),
                            Ok(_) if past => {
                                tally.wrong.push(format!("{what}: built past the limit"))
                            }
                            Ok(p) => match held(&p.body, &shape, side.levels(t), &tol) {
                                Ok(line) => {
                                    tally.right += 1;
                                    lines.push((what, line));
                                }
                                Err(Held::CouldNot(why)) if sliver => {
                                    tally.slivers.push(format!("{what}: {why}"));
                                }
                                Err(Held::CouldNot(why) | Held::Wrong(why)) => {
                                    tally.wrong.push(format!("{what}: {why}"));
                                }
                            },
                        }
                    }
                }
            }
        }
    }
    let worst = lines.iter().map(|(_, off)| *off).fold(0.0_f64, f64::max);
    for line in &tally.slivers {
        println!("sliver {line}");
    }
    for line in &tally.wrong {
        println!("WRONG {line}");
    }
    println!(
        "{names:?}: right {} (volumes off at most {worst:.2e}), refused past the limit {}, slivers not taken further {}, wrong {}",
        tally.right,
        tally.refused_past,
        tally.slivers.len(),
        tally.wrong.len()
    );
    tally
}

/// Why a built plate is not held: another module could not take it (the
/// sorting into lumps, the measure, the tessellator), or it is wrong.
enum Held {
    CouldNot(String),
    Wrong(String),
}

/// A hollow: valid, two shells (an outside and a cavity), each of the
/// shape's genus, its volume the closed form (by the measure, within its
/// bound and what fits within a quarter of the tolerance can leave), and
/// its mesh closed and of that volume within its chord.
fn held(body: &Body, shape: &Closed, levels: (f64, f64), tol: &Tolerance) -> Result<f64, Held> {
    let defects = check(body, tol);
    if !defects.is_empty() {
        return Err(Held::Wrong(format!("not valid: {:?}", defects.first())));
    }
    if body.shells.len() != 2 {
        return Err(Held::Wrong(format!("{} shells", body.shells.len())));
    }
    let found = lumps(body, tol).map_err(|e| Held::CouldNot(format!("lumps: {e}")))?;
    let one = |outer: u32, cavity: u32| Lump {
        outer: ShellId(outer),
        cavities: vec![ShellId(cavity)],
    };
    if found != vec![one(0, 1)] && found != vec![one(1, 0)] {
        return Err(Held::Wrong(format!("lumps {found:?}")));
    }
    let genera = shell_genera(body).ok_or(Held::Wrong("no genera".into()))?;
    if genera != vec![shape.genus; 2] {
        return Err(Held::Wrong(format!("genera {genera:?}")));
    }
    let (m, bound) = mass_bound(body).map_err(|e| Held::CouldNot(format!("not measured: {e}")))?;
    let want = (shape.volume)(levels);
    // Each fitted skin within a quarter of the tolerance of its exact
    // offset: the volume within its area times that.
    let fits = if shape.freeform {
        0.25 * tol.linear * m.area
    } else {
        0.0
    };
    let off = (m.volume - want).abs();
    if off > 1e-9 * want.abs() + bound.volume + fits {
        return Err(Held::Wrong(format!(
            "volume {} for {want} (off {:.3e}, bound {:.3e})",
            m.volume,
            off / want,
            bound.volume
        )));
    }
    let chord = 2e-3 * shape.size.max(levels.1 - levels.0);
    let mesh = mesh_volume(body, chord).map_err(Held::CouldNot)?;
    if (mesh - want).abs() > 2.0 * chord * m.area {
        return Err(Held::Wrong(format!("mesh volume {mesh} for {want}")));
    }
    Ok(off / want)
}

#[test]
fn plates_of_whole_spheres_and_tori_are_hollows() {
    let tally = whole(&["sphere", "torus", "turned sphere", "turned torus"]);
    assert!(tally.wrong.is_empty() && tally.slivers.is_empty());
    assert_eq!((tally.right, tally.refused_past), (558, 270));
}

/// The rounded ring's slivers (its corners' tori a few tolerances or a
/// thousandth of their radius thick, its inner wall as near its axis) are
/// valid but not always sorted into lumps, measured or meshed: owed to
/// those (docs/design/plates.md › *Limits*).
#[test]
fn plates_of_closed_sheets_of_several_faces_are_hollows() {
    let tally = whole(&["capsule", "rounded ring"]);
    assert!(tally.wrong.is_empty(), "{:?}", tally.wrong);
    assert_eq!(
        (tally.right, tally.refused_past, tally.slivers.len()),
        (248, 135, 31)
    );
}

#[test]
fn a_plate_of_a_ring_turned_from_a_closed_spline_is_a_hollow() {
    let tally = whole(&["spline ring"]);
    assert!(tally.wrong.is_empty(), "{:?}", tally.wrong);
    assert_eq!(tally.right, 54);
}

#[test]
fn hostile_thicknesses_of_a_whole_sphere_are_refused() {
    let tol = tolerance(1.0);
    let ball = primitives::sphere(&Frame::WORLD, 10.0).unwrap();
    for t in [f64::NAN, f64::INFINITY, -1.0, 0.0, 1e-7, 1e300, f64::MAX] {
        for side in [Side::Out, Side::In, Side::Mid] {
            let made = plate(&ball, &[FaceId(0)], t, side, &BTreeMap::new(), &tol);
            if let Ok(p) = made {
                // Only a thickness the sphere can take on that side.
                assert!(t.is_finite() && t > tol.linear, "{t} {side:?}");
                assert!(check(&p.body, &tol).is_empty(), "{t} {side:?}");
            }
        }
    }
}

// ---------------------------------------------------------------------
// Developments of turned cylinders and cones.
// ---------------------------------------------------------------------

/// The side of a cylinder (radii equal) or a cone's frustum, turned from a
/// rectangle or a trapezoid standing on the axis, developed: its area
/// `π (r₀ + r₁) s`, `s` the slant. Before 2026-11-15 the turned cylinder
/// of the hunt was refused at 1e6, and every one at 1e8 and 1e9 (a
/// cylinder of radius 1e9 passed for collapsed everywhere).
#[test]
fn turned_cylinders_and_cones_unroll_at_every_scale() {
    let (count, wrong) = developments();
    assert_eq!(count, 196);
    assert!(wrong.is_empty(), "{wrong:?}");
}

fn developments() -> (usize, Vec<String>) {
    let mut count = 0;
    let mut wrong = Vec::new();
    for (si, k) in [1e-3, 1.0, 1e3, 1e6, 1e7, 1e8, 1e9].into_iter().enumerate() {
        let tol = tolerance(k);
        let mut rng = Rng(case_seed(0xc0_11e5, si));
        let mut list = vec![Frame::WORLD];
        for _ in 0..6 {
            let far = if rng.below(2) == 0 { 0.0 } else { 1e3 };
            list.push(turned(&mut rng, k, far));
        }
        for (fi, frame) in list.iter().enumerate() {
            for (r0, r1, h) in [
                (31.43694550814996, 31.43694550814996, 12.025057614637177),
                (10.0, 10.0, 40.0),
                (10.0, 4.0, 7.0),
                (3.0, 9.0, 20.0),
            ] {
                let (r0, r1, h) = (r0 * k, r1 * k, h * k);
                let curves = [
                    line(1, [0.0, 0.0], [r0, 0.0]),
                    line(2, [r0, 0.0], [r1, h]),
                    line(3, [r1, h], [0.0, h]),
                    line(4, [0.0, h], [0.0, 0.0]),
                ];
                let body = revolved(&curves, frame, &tol);
                let Some(side) = body
                    .faces
                    .iter()
                    .position(|f| !matches!(f.surface, Surface::Plane(_)))
                else {
                    panic!("no side");
                };
                let what = format!("k {k} frame {fi} r {r0} {r1} h {h}");
                count += 1;
                let made = develop(&body, &[FaceId(side as u32)], &tol)
                    .and_then(|d| d.flat_body(&Frame::WORLD, &tol));
                let want = PI * (r0 + r1) * (r1 - r0).phypot(h);
                match made {
                    Err(e) => wrong.push(format!("{what}: refused: {e}")),
                    Ok(flat) => {
                        let a = mass_properties(&flat.body).unwrap().area;
                        let close = (a / want - 1.0).abs() < 1e-7;
                        if !close {
                            wrong.push(format!("{what}: area {a} for {want}"));
                        }
                    }
                }
            }
        }
    }
    for w in &wrong {
        println!("WRONG {w}");
    }
    println!("developments: {count}, wrong {}", wrong.len());
    (count, wrong)
}

// ---------------------------------------------------------------------
// Bends of any angle: the box against the part.
// ---------------------------------------------------------------------

fn rect_spec(w: f64, d: f64, t: f64, k: f64, plane: Frame, flange: FlangeSpec) -> Spec {
    let c = [[0.0, 0.0], [w, 0.0], [w, d], [0.0, d]];
    let curves: Vec<SketchCurve> = (0..4)
        .map(|i| line(i as u64 + 1, c[i], c[(i + 1) % 4]))
        .collect();
    let base = detect_profiles(&curves, &tolerance(k)).unwrap().remove(0);
    Spec {
        base,
        plane,
        thickness: t,
        flanges: vec![flange],
        relief: None,
        bend_relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts: Vec::new(),
        jogs: Vec::new(),
        ids: Vec::new(),
    }
}

/// The box of the vertices and of 1024 points along each edge.
fn sampled(body: &Body) -> (Vec3, Vec3) {
    let mut lo = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = -lo;
    let mut add = |p: Vec3| {
        lo = lo.min(p);
        hi = hi.max(p);
    };
    for v in &body.vertices {
        add(v.point);
    }
    for e in &body.edges {
        for i in 0..=1024 {
            add(e
                .curve
                .eval(e.range.0 + (e.range.1 - e.range.0) * f64::from(i) / 1024.0));
        }
    }
    (lo, hi)
}

/// A base 52.6 × 73.7, 2.81 thick, with a flange 12.65 long bent by
/// `angle` about the inner radius `ratio` times the thickness, every
/// length times `k`: built right (valid, its volume `w d T + d (θ T (R +
/// T/2) + L T)`, meshed), and its `Body::bbox` the box of its edges (within
/// what 1024 points along an arc can miss and the tolerance), or within a
/// thousandth of the part's size; else counted wild. Before 2026-11-15, 60
/// of the 2,080 were wild (bends of 1e-5 to 0.0032 degrees, each with one
/// freeform side whose net spanned most of the bend's circle).
#[test]
fn bends_of_any_angle_have_the_box_of_their_edges() {
    let bent = bends();
    assert!(bent.wild.is_empty() && bent.wrong.is_empty());
    // The refused: bends of 1e-6 to 1e-3 degrees whose strip or inner arc
    // is about the tolerance or shorter (docs/design/sheetmetal.md ›
    // *Limits*), the same parts and words as before 2026-11-15.
    assert_eq!((bent.right, bent.refused.len()), (1720, 360));
}

struct Bends {
    right: usize,
    wild: Vec<String>,
    refused: Vec<String>,
    wrong: Vec<String>,
}

fn bends() -> Bends {
    let (mut right, mut wild, mut refused, mut wrong) = (0, Vec::new(), Vec::new(), Vec::new());
    for (si, k) in [1e-3, 1.0, 1e3, 1e6, 1e7].into_iter().enumerate() {
        let tol = tolerance(k);
        let mut rng = Rng(case_seed(0xbe_4d, si));
        let planes = [Frame::WORLD, turned(&mut rng, k, 100.0)];
        let (w, d, t, l) = (52.59 * k, 73.73 * k, 2.81 * k, 12.65 * k);
        for (pi, plane) in planes.iter().enumerate() {
            for degrees in [
                1e-6,
                1e-5,
                1e-4,
                1e-3,
                3.238984780e-3,
                1e-2,
                0.1,
                1.0,
                10.0,
                45.0,
                90.0,
                135.0,
                179.0,
            ] {
                for dir in [Direction::Up, Direction::Down] {
                    for ratio in [0.01, 0.4192, 1.0, 5.0] {
                        for kk in [0.0138, 0.4] {
                            let th = f64::to_radians(degrees);
                            let r = ratio * t;
                            let what =
                                format!("k {k} plane {pi} {degrees}° {dir:?} R {ratio} T K {kk}");
                            let flange = FlangeSpec {
                                on: On::Base(2),
                                length: l,
                                angle: th,
                                direction: dir,
                                radius: r,
                                k: kk,
                                from: LengthFrom::Tangent,
                                inset: [0.0; 2],
                                outline: None,
                            };
                            let part = match build(&rect_spec(w, d, t, k, *plane, flange), &tol) {
                                Ok(p) => p,
                                Err(e) => {
                                    refused.push(format!("{what}: {e}"));
                                    continue;
                                }
                            };
                            let body = &part.solid.body;
                            if !check(body, &tol).is_empty() {
                                wrong.push(format!("{what}: not valid"));
                                continue;
                            }
                            let want = w * d * t + d * (th * t * (r + 0.5 * t) + l * t);
                            let got = mass_properties(body).map(|m| m.volume);
                            if !got.as_ref().is_ok_and(|v| (v / want - 1.0).abs() < 1e-8) {
                                wrong.push(format!("{what}: volume {got:?} for {want}"));
                                continue;
                            }
                            let (slo, shi) = sampled(body);
                            let size = (shi - slo).length();
                            if mesh_volume(body, 1e-3 * size).is_err() {
                                wrong.push(format!("{what}: not meshed"));
                                continue;
                            }
                            let (lo, hi) = body.bbox().unwrap().corners().unwrap();
                            let over = (slo - lo).max(hi - shi);
                            let most = over.x.max(over.y).max(over.z);
                            if most > 1e-3 * size {
                                let freeform = body
                                    .faces
                                    .iter()
                                    .filter(|f| matches!(f.surface, Surface::Nurbs(_)))
                                    .count();
                                wild.push(format!(
                                    "{what}: the box reaches {:.3e} of the size past the edges ({freeform} freeform faces)",
                                    most / size
                                ));
                            } else {
                                right += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    for line in &refused {
        println!("refused {line}");
    }
    for line in &wild {
        println!("WILD {line}");
    }
    for line in &wrong {
        println!("WRONG {line}");
    }
    println!(
        "bends: right {right}, wild box {}, refused {}, wrong {}",
        wild.len(),
        refused.len(),
        wrong.len()
    );
    Bends {
        right,
        wild,
        refused,
        wrong,
    }
}

/// Does another feature write a turn of a tiny angle with a far control
/// point? A cubic spline profile turned about an axis beside it by 1e-6 to
/// 179 degrees (a rational surface of revolution: per quarter turn or less
/// a quadratic arc, its middle control point `r / cos(θ/2)` from the axis,
/// beside the arc for a small `θ`), at three scales: `Body::bbox` (the
/// spline faces' whole nets) is the box of the edges sampled within a
/// thousandth of the size, or within the arc's own bulge past its chord.
#[test]
fn a_spline_turned_by_any_angle_has_a_net_beside_it() {
    let mut wild = Vec::new();
    let mut count = 0;
    for k in SCALES {
        let tol = tolerance(k);
        let control = [
            [20.0 * k, 0.0],
            [26.0 * k, 3.0 * k],
            [22.0 * k, 7.0 * k],
            [27.0 * k, 10.0 * k],
        ];
        let curves = [
            SketchCurve {
                id: 1,
                curve: Curve2::Spline(Spline2 {
                    degree: 3,
                    knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                    control: control.to_vec(),
                }),
            },
            line(2, [27.0 * k, 10.0 * k], [20.0 * k, 10.0 * k]),
            line(3, [20.0 * k, 10.0 * k], [20.0 * k, 0.0]),
        ];
        let profiles = detect_profiles(&curves, &tol).unwrap();
        for degrees in [1e-6, 1e-4, 1e-2, 1.0, 45.0, 90.0, 179.0] {
            let th = f64::to_radians(degrees);
            let body = match revolve(
                &profiles,
                &Frame::WORLD,
                [0.0, 0.0],
                [0.0, 1.0],
                0.0,
                th,
                &tol,
            ) {
                Ok(made) => made.body,
                Err(e) => {
                    println!("{k} {degrees}°: refused: {e}");
                    continue;
                }
            };
            count += 1;
            let (slo, shi) = sampled(&body);
            let size = (shi - slo).length();
            let (lo, hi) = body.bbox().unwrap().corners().unwrap();
            let over = (slo - lo).max(hi - shi);
            let most = over.x.max(over.y).max(over.z);
            // The arc's middle control point past the arc: `r (sec(φ/2) −
            // 1)` for pieces of `φ`, a quarter turn or less, at the largest
            // radius.
            let pieces = (degrees / 90.0).ceil();
            let bulge = 27.0 * k * (1.0 / (0.5 * th / pieces).pcos() - 1.0);
            if most > bulge + 1e-3 * size {
                wild.push(format!("{k} {degrees}°: {:.3e} of the size", most / size));
            }
        }
    }
    println!("turned splines: {count}, wild {}", wild.len());
    // The smallest turn is refused: it moves the profile by less than the
    // tolerance.
    assert_eq!(count, 18);
    assert!(wild.is_empty(), "{wild:?}");
}
