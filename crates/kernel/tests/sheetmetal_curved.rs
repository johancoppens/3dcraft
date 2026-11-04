//! Curved bends (docs/design/sheetmetal.md › *Curved bends*): a flange
//! bent along an arc or a circle of the base, its bend a piece of a torus
//! and its flat a piece of a cone (a cylinder at 90 degrees), its flat
//! pattern the radial development (the section's lengths kept along every
//! radial line), held at 1e-3, 1 and 1e6 to:
//!
//! - the volume by Pappus (the section turned about the arc's axis),
//!   written here on its own: `A_base T + Δ (a A_sec + ε M_x)`;
//! - the flat area: the base's plus the annular sectors `Δ/2 |r_t² − a²|`;
//! - the strain bound against a dense measurement of the strain on the
//!   built solid (its torus skins sampled along a meridian, its tip and far
//!   edges' radii): every measured strain within the bound, the bound
//!   within a millionth of the largest measured;
//! - the fold along each radial line: the side rims' arcs `θ R`, `θ (R +
//!   T)` and lines `L`, so the neutral fibre's `θ (R + K T)` is the strip;
//! - names: each curved flange's edges `F9:edge(17, id − 1, k)`;
//! - refusals by reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Curve3, Surface};
use craft3d_kernel::KernelError;
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::sheetmetal::curved::{CurvedSpec, strain_at, strain_bound};
use craft3d_kernel::sheetmetal::{
    Closing, Direction, EdgeKey, FlangeSpec, LengthFrom, On, Part, Spec, build_with, edge_numbers,
    flat_area, outline,
};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{check, mass_properties};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn line(id: u64, start: [f64; 2], end: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

fn arc(id: u64, center: [f64; 2], start: [f64; 2], end: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Arc { center, start, end },
    }
}

fn s(p: [f64; 2], k: f64) -> [f64; 2] {
    [p[0] * k, p[1] * k]
}

fn profile(curves: &[SketchCurve], k: f64) -> Profile {
    let mut found = detect_profiles(curves, &tolerance(k)).unwrap();
    assert_eq!(found.len(), 1);
    found.remove(0)
}

/// A plate 100 wide whose top is a half circle of radius 50 about (50,
/// 60): the arc (curve 3) runs counter-clockwise, the flange grows away
/// from its centre (a shrink flange).
fn dome(k: f64) -> Profile {
    profile(
        &[
            line(1, s([0.0, 0.0], k), s([100.0, 0.0], k)),
            line(2, s([100.0, 0.0], k), s([100.0, 60.0], k)),
            arc(
                3,
                s([50.0, 60.0], k),
                s([100.0, 60.0], k),
                s([0.0, 60.0], k),
            ),
            line(4, s([0.0, 60.0], k), s([0.0, 0.0], k)),
        ],
        k,
    )
}

/// The area of [`dome`].
fn dome_area(k: f64) -> f64 {
    (100.0 * 60.0 + 0.5 * PI * 50.0 * 50.0) * k * k
}

/// A plate 100 × 60 with a half-round notch of radius 25 about (50, 60)
/// in its top edge (curve 4): the loop runs the arc clockwise, the flange
/// grows towards its centre (a stretch flange).
fn notch(k: f64) -> Profile {
    profile(
        &[
            line(1, s([0.0, 0.0], k), s([100.0, 0.0], k)),
            line(2, s([100.0, 0.0], k), s([100.0, 60.0], k)),
            line(3, s([100.0, 60.0], k), s([75.0, 60.0], k)),
            arc(
                4,
                s([50.0, 60.0], k),
                s([25.0, 60.0], k),
                s([75.0, 60.0], k),
            ),
            line(5, s([25.0, 60.0], k), s([0.0, 60.0], k)),
            line(6, s([0.0, 60.0], k), s([0.0, 0.0], k)),
        ],
        k,
    )
}

fn notch_area(k: f64) -> f64 {
    (100.0 * 60.0 - 0.5 * PI * 25.0 * 25.0) * k * k
}

/// A plate 100 × 60 with a round hole of radius 15 at (50, 30) (curve 5):
/// a collar bent from the hole's edge grows into it.
fn holed(k: f64) -> Profile {
    profile(
        &[
            line(1, s([0.0, 0.0], k), s([100.0, 0.0], k)),
            line(2, s([100.0, 0.0], k), s([100.0, 60.0], k)),
            line(3, s([100.0, 60.0], k), s([0.0, 60.0], k)),
            line(4, s([0.0, 60.0], k), s([0.0, 0.0], k)),
            SketchCurve {
                id: 5,
                curve: Curve2::Circle {
                    center: s([50.0, 30.0], k),
                    radius: 15.0 * k,
                },
            },
        ],
        k,
    )
}

fn holed_area(k: f64) -> f64 {
    (100.0 * 60.0 - PI * 15.0 * 15.0) * k * k
}

/// A disc of radius 40 (curve 1): a pan's rim.
fn disc(k: f64) -> Profile {
    profile(
        &[SketchCurve {
            id: 1,
            curve: Curve2::Circle {
                center: [0.0, 0.0],
                radius: 40.0 * k,
            },
        }],
        k,
    )
}

fn spec(base: Profile, t: f64) -> Spec {
    Spec {
        base,
        plane: Frame::WORLD,
        thickness: t,
        flanges: Vec::new(),
        relief: None,
        bend_relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts: Vec::new(),
        jogs: Vec::new(),
        ids: Vec::new(),
    }
}

fn curved(edge: u64, length: f64, angle: f64, up: bool, radius: f64, k: f64) -> CurvedSpec {
    CurvedSpec {
        edge,
        length,
        angle,
        direction: if up { Direction::Up } else { Direction::Down },
        radius,
        k,
        from: LengthFrom::Tangent,
        id: 1,
    }
}

#[track_caller]
fn near(actual: f64, expected: f64, slack: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= slack,
        "{what}: {actual} against {expected} (off by {:e}, allowed {slack:e})",
        actual - expected
    );
}

/// The section's `∫ ρ dA` about the arc's axis, written out on its own:
/// the bend's ring sector (inner radius R, outer R + T, over θ) at
/// `x = r sin φ`, and the flat `L × T` at `x = r sin θ + d cos θ`.
fn pappus(a: f64, eps: f64, theta: f64, r: f64, t: f64, l: f64) -> f64 {
    let ro = r + t;
    let bend_area = theta * (ro * ro - r * r) / 2.0;
    let bend_mx = (1.0 - theta.pcos()) * (ro.ppowi(3) - r.ppowi(3)) / 3.0;
    let flat_area = l * t;
    let flat_mx = theta.psin() * l * (ro * ro - r * r) / 2.0 + theta.pcos() * t * l * l / 2.0;
    a * (bend_area + flat_area) + eps * (bend_mx + flat_mx)
}

/// Builds and checks the solid, its volume and flat area against the
/// closed forms, the strain bound against the solid, the fold along the
/// radial lines and the names.
#[allow(clippy::too_many_arguments)]
fn holds(
    base: Profile,
    base_area: f64,
    c: CurvedSpec,
    a: f64,
    eps: f64,
    sweep: f64,
    t: f64,
    k: f64,
) -> Part {
    let tol = tolerance(k);
    let theta = c.angle;
    let (r, kk, l) = (c.radius, c.k, c.length);
    let part = build_with(&spec(base, t), std::slice::from_ref(&c), None, &tol)
        .unwrap_or_else(|e| panic!("the part is refused: {e}"));
    let defects = check(&part.solid.body, &tol);
    assert!(
        defects.is_empty(),
        "the solid fails the checker: {defects:?}"
    );
    // Volume by Pappus.
    let v = mass_properties(&part.solid.body).unwrap().volume;
    let expected = base_area * t + sweep * pappus(a, eps, theta, r, t, l);
    near(v, expected, 1e-7 * expected, "volume");
    // The flat area.
    let rn = r + kk * t;
    let ba = theta * rn;
    let tip = a + eps * (ba + l);
    let area = base_area + 0.5 * sweep * (tip * tip - a * a).abs();
    near(flat_area(&part), area, 1e-9 * area, "flat area");
    // The outline holds it too (every curve of it, profiles detected).
    let chains = outline(&part, &tol);
    let curves: Vec<SketchCurve> = chains
        .iter()
        .flatten()
        .enumerate()
        .map(|(i, seg)| SketchCurve {
            id: i as u64 + 1,
            curve: match seg.shape {
                craft3d_kernel::profile::Shape::Line => Curve2::Line {
                    start: seg.start,
                    end: seg.end,
                },
                craft3d_kernel::profile::Shape::Arc { center, .. } => {
                    let (a0, a1) = if seg.reversed {
                        (seg.end, seg.start)
                    } else {
                        (seg.start, seg.end)
                    };
                    Curve2::Arc {
                        center,
                        start: a0,
                        end: a1,
                    }
                }
                craft3d_kernel::profile::Shape::Circle { center, radius } => {
                    Curve2::Circle { center, radius }
                }
                _ => panic!("no spline here"),
            },
        })
        .collect();
    let found = detect_profiles(&curves, &tol).unwrap();
    assert_eq!(found.len(), 1);
    let measured: f64 = found[0].area();
    near(measured, area, 1e-9 * area, "outline's area");

    // The strain bound, against the solid.
    let cv = &part.curved[0];
    let bound = cv.strain;
    let mut worst = (0.0f64, 0.0f64);
    let mut seen = |e: f64| {
        worst = (worst.0.min(e), worst.1.max(e));
    };
    let up = c.direction == Direction::Up;
    let zc = if up { t + r } else { -r };
    let sigma = if up { 1.0 } else { -1.0 };
    let center = Vec3::new(cv.center[0], cv.center[1], 0.0);
    let (ms, mc) = (cv.begin + 0.5 * cv.sweep).psin_cos();
    let radial = Vec3::new(mc, ms, 0.0);
    let rho_of = |p: Vec3| (p - center).cross(Vec3::Z).length();
    let tori: Vec<_> = part
        .solid
        .body
        .faces
        .iter()
        .filter_map(|f| match &f.surface {
            Surface::Torus(tor) => Some(*tor),
            _ => None,
        })
        .collect();
    assert_eq!(tori.len(), 2, "the bend's two skins");
    for rr in [r, r + t] {
        let torus = tori
            .iter()
            .find(|tor| (tor.radii().1 - rr).abs() <= tol.linear)
            .expect("a skin of radius r");
        let surf = Surface::Torus(*torus);
        let n = 4000;
        for j in 0..=n {
            let phi = theta * j as f64 / n as f64;
            // The section's point at φ on this skin, put on the solid's
            // torus: its distance from the arc's axis is measured there.
            let guess = center
                + radial * (a + eps * rr * phi.psin())
                + Vec3::Z * (zc - sigma * rr * phi.pcos());
            let (u, w) = surf.project(guess).unwrap();
            let p = surf.eval(u, w);
            assert!(
                p.distance(guess) <= tol.linear,
                "the skin is where the section is"
            );
            let s_flat = rn * phi;
            seen(rho_of(p) / (a + eps * s_flat) - 1.0);
        }
    }
    // The far and tip edges: circles about the arc's axis.
    for e in &part.solid.body.edges {
        let Curve3::Circle(circle) = &e.curve else {
            continue;
        };
        let axis = circle.frame().z();
        if axis.cross(Vec3::Z).length() > 1e-9 {
            continue;
        }
        let o = circle.frame().origin();
        if (Vec3::new(o.x, o.y, 0.0) - center).length() > tol.linear {
            continue;
        }
        // Which fibre: its height says the depth and where it is.
        let rho = circle.radius();
        for (s_flat, rr) in [(ba, r), (ba, r + t), (ba + l, r), (ba + l, r + t)] {
            let x = rr * theta.psin() + (s_flat - ba) * theta.pcos();
            let z = zc - sigma * rr * theta.pcos() + sigma * (s_flat - ba) * theta.psin();
            if (o.z - z).abs() <= tol.linear && (rho - (a + eps * x)).abs() <= tol.linear {
                seen(rho / (a + eps * s_flat) - 1.0);
            }
        }
    }
    // And the closed form densely, as a check of the branch and bound.
    for j in 0..=20_000 {
        let sv = (ba + l) * j as f64 / 20_000.0;
        for rr in [r, r + 0.5 * t, r + t] {
            seen(strain_at(a, eps, theta, rn, l, sv, rr));
        }
    }
    assert!(
        worst.0 >= bound.least && worst.1 <= bound.most,
        "measured strain {worst:?} outside the bound {bound:?}"
    );
    let spread = 1e-7 * bound.largest() + 1e-11;
    assert!(
        bound.most - worst.1 <= spread && worst.0 - bound.least <= spread,
        "the bound {bound:?} is not tight to the measured {worst:?}"
    );

    // Along every radial line the section is laid out as a straight bend's:
    // the side rims' arcs θ R and θ (R + T) and the flat's lines L.
    if sweep < TAU {
        let mut arcs = 0;
        let mut lines = 0;
        for e in &part.solid.body.edges {
            match &e.curve {
                Curve3::Circle(cl) if cl.frame().z().cross(Vec3::Z).length() > 0.5 => {
                    let len = (e.range.1 - e.range.0) * cl.radius();
                    let rr = cl.radius();
                    assert!(
                        (rr - r).abs() <= tol.linear || (rr - r - t).abs() <= tol.linear,
                        "a side arc of radius {rr}"
                    );
                    near(len, theta * rr, 1e-9 * len.max(tol.linear), "side arc");
                    arcs += 1;
                }
                Curve3::Line(_) => {
                    let len = e.range.1 - e.range.0;
                    if (len - l).abs() <= 1e-6 * l {
                        lines += 1;
                    }
                }
                _ => {}
            }
        }
        assert_eq!(arcs, 4, "two skins at two ends");
        assert!(lines >= 4, "two skins' lines at two ends, {lines}");
        // The neutral fibre between them is the strip.
        near(ba, theta * (r + kk * t), 1e-12 * ba, "allowance");
    }
    // Names.
    let keys: Vec<Vec<u64>> = part
        .edge_keys
        .iter()
        .filter(|k| matches!(k, EdgeKey::Curved { .. }))
        .map(|k| edge_numbers(&part, *k).unwrap())
        .collect();
    let wanted = if sweep < TAU { 6 } else { 2 };
    assert_eq!(keys.len(), wanted);
    assert!(keys.iter().all(|n| n[0] == 17 && n[1] == c.id - 1));
    part
}

#[test]
fn a_shrink_flange_on_a_half_circle() {
    for k in SCALES {
        for (kk, up, angle) in [
            (0.33, true, FRAC_PI_2),
            (0.5, false, FRAC_PI_2),
            (0.4, true, PI / 3.0),
            (0.5, true, 2.0 * PI / 3.0),
        ] {
            let c = curved(3, 10.0 * k, angle, up, 2.0 * k, kk);
            holds(dome(k), dome_area(k), c, 50.0 * k, 1.0, PI, 1.0 * k, k);
        }
    }
}

#[test]
fn a_stretch_flange_in_a_notch() {
    for k in SCALES {
        for (kk, up, angle) in [(0.33, true, FRAC_PI_2), (0.5, false, PI / 4.0)] {
            let c = curved(4, 8.0 * k, angle, up, 2.0 * k, kk);
            holds(notch(k), notch_area(k), c, 25.0 * k, -1.0, PI, 1.0 * k, k);
        }
    }
}

#[test]
fn a_collar_round_a_hole_and_a_pans_rim() {
    for k in SCALES {
        let c = curved(5, 4.0 * k, FRAC_PI_2, false, 1.5 * k, 0.4);
        holds(holed(k), holed_area(k), c, 15.0 * k, -1.0, TAU, 1.0 * k, k);
        let c = curved(1, 12.0 * k, FRAC_PI_2, true, 3.0 * k, 0.5);
        holds(
            disc(k),
            PI * 1600.0 * k * k,
            c,
            40.0 * k,
            1.0,
            TAU,
            1.0 * k,
            k,
        );
    }
}

/// The bound in closed form where its extremes are known: at 90 degrees a
/// shrink flange's largest compression is the tip's inner fibre, `(R − BA
/// − L) / (a + BA + L)`; a stretch flange's largest stretch the same fibre
/// mirrored, `−(R − BA − L) / (a − BA − L)`.
#[test]
fn the_bound_at_ninety_degrees_is_the_tips_inner_fibre() {
    for k in SCALES {
        let (a, r, t, kk, l) = (50.0 * k, 2.0 * k, 1.0 * k, 0.33, 10.0 * k);
        let ba = FRAC_PI_2 * (r + kk * t);
        let b = strain_bound(a, 1.0, FRAC_PI_2, r, kk, t, l).unwrap();
        let tip = (r - ba - l) / (a + ba + l);
        near(b.least, tip, 1e-11, "shrink");
        let b = strain_bound(a, -1.0, FRAC_PI_2, r, kk, t, l).unwrap();
        let tip = -(r - ba - l) / (a - ba - l);
        near(b.most, tip, 1e-11, "stretch");
        assert!(b.least <= 0.0 && b.most >= 0.0);
    }
}

fn refused(base: Profile, c: CurvedSpec, max: Option<f64>, k: f64, what: &str) {
    let tol = tolerance(k);
    match build_with(&spec(base, k), &[c], max, &tol) {
        Err(KernelError::SheetMetal { reason, .. }) => {
            assert!(reason.contains(what), "`{reason}` does not say `{what}`");
        }
        Err(other) => panic!("refused for another reason: {other}"),
        Ok(_) => panic!("built, but should be refused for {what}"),
    }
}

#[test]
fn refusals_by_reason() {
    for k in SCALES {
        // The bend reaches its own arc's axis.
        refused(
            notch(k),
            curved(4, 3.0 * k, FRAC_PI_2, true, 24.5 * k, 0.5),
            None,
            k,
            "the bend would reach the axis",
        );
        // The stretch flange's flat reaches the notch's centre.
        refused(
            notch(k),
            curved(4, 30.0 * k, FRAC_PI_2, true, 2.0 * k, 0.5),
            None,
            k,
            "reaches the centre",
        );
        // Above the material's limit.
        refused(
            dome(k),
            curved(3, 10.0 * k, FRAC_PI_2, true, 2.0 * k, 0.5),
            Some(0.05),
            k,
            "more than maxStrain",
        );
        // A line is a flange of `flanges`.
        refused(
            dome(k),
            curved(1, 10.0 * k, FRAC_PI_2, true, 2.0 * k, 0.5),
            None,
            k,
            "straight line",
        );
        refused(
            dome(k),
            curved(9, 10.0 * k, FRAC_PI_2, true, 2.0 * k, 0.5),
            None,
            k,
            "no edge of the base",
        );
        refused(
            dome(k),
            curved(3, 10.0 * k, FRAC_PI_2, true, 0.0, 0.5),
            None,
            k,
            "inner radius above the tolerance",
        );
        refused(
            dome(k),
            curved(3, 10.0 * k, 4.0, true, 2.0 * k, 0.5),
            None,
            k,
            "at most 180",
        );
    }
}

#[test]
fn the_limit_passes_a_flange_within_it() {
    let k = 1.0;
    let tol = tolerance(k);
    let c = curved(3, 10.0, FRAC_PI_2, true, 2.0, 0.5);
    let part = build_with(
        &spec(dome(k), 1.0),
        std::slice::from_ref(&c),
        Some(0.3),
        &tol,
    )
    .unwrap();
    let largest = part.curved[0].strain.largest();
    assert!(largest > 0.05 && largest < 0.3, "{largest}");
    // A limit just under it refuses.
    assert!(build_with(&spec(dome(k), 1.0), &[c], Some(largest * 0.999), &tol).is_err());
}

/// A spline edge is refused with the method's reason (a canal surface).
#[test]
fn a_spline_edge_is_refused() {
    let k = 1.0;
    let spline = Spline2 {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        control: vec![[100.0, 60.0], [80.0, 100.0], [20.0, 100.0], [0.0, 60.0]],
    };
    let base = profile(
        &[
            line(1, [0.0, 0.0], [100.0, 0.0]),
            line(2, [100.0, 0.0], [100.0, 60.0]),
            SketchCurve {
                id: 3,
                curve: Curve2::Spline(spline),
            },
            line(4, [0.0, 60.0], [0.0, 0.0]),
        ],
        k,
    );
    refused(
        base,
        curved(3, 10.0, FRAC_PI_2, true, 2.0, 0.5),
        None,
        k,
        "canal surface",
    );
}

/// A rectangle 100 × 60 with its corners rounded to radius `rc`: lines
/// 1 (bottom), 2 (right), 3 (top), 4 (left), arcs 5 to 8 at the corners
/// from the bottom right counter-clockwise.
fn rounded(rc: f64, k: f64) -> Profile {
    let (w, h) = (100.0, 60.0);
    profile(
        &[
            line(1, s([rc, 0.0], k), s([w - rc, 0.0], k)),
            line(2, s([w, rc], k), s([w, h - rc], k)),
            line(3, s([w - rc, h], k), s([rc, h], k)),
            line(4, s([0.0, h - rc], k), s([0.0, rc], k)),
            arc(5, s([w - rc, rc], k), s([w - rc, 0.0], k), s([w, rc], k)),
            arc(
                6,
                s([w - rc, h - rc], k),
                s([w, h - rc], k),
                s([w - rc, h], k),
            ),
            arc(7, s([rc, h - rc], k), s([rc, h], k), s([0.0, h - rc], k)),
            arc(8, s([rc, rc], k), s([0.0, rc], k), s([rc, 0.0], k)),
        ],
        k,
    )
}

fn straight(edge: u64, length: f64, up: bool, radius: f64, k: f64) -> FlangeSpec {
    FlangeSpec {
        on: On::Base(edge),
        length,
        angle: FRAC_PI_2,
        direction: if up { Direction::Up } else { Direction::Down },
        radius,
        k,
        from: LengthFrom::Tangent,
        inset: [0.0; 2],
        outline: None,
    }
}

/// A tray with rounded corners: a wall on every line and a curved flange
/// on every corner's arc, alike, so each corner continues the walls beside
/// it into one wall round the tray (no side between them in the part or in
/// the pattern). The volume is the base's slab, the straight walls' (their
/// bends' ring sectors and flats) and the corners' by Pappus; the flat
/// area the base's, the walls' rectangles and the corners' annular
/// sectors; the outline one closed chain.
#[test]
fn a_tray_with_rounded_corners_is_one_wall() {
    for k in SCALES {
        for (up, kk) in [(true, 0.4), (false, 0.5)] {
            let tol = tolerance(k);
            let (rc, t, r, l) = (15.0 * k, 1.0 * k, 2.0 * k, 10.0 * k);
            let mut sp = spec(rounded(15.0, k), t);
            sp.flanges = (1..=4).map(|e| straight(e, l, up, r, kk)).collect();
            let corners: Vec<CurvedSpec> = (5..=8)
                .map(|e| CurvedSpec {
                    id: e,
                    ..curved(e, l, FRAC_PI_2, up, r, kk)
                })
                .collect();
            let part = build_with(&sp, &corners, None, &tol)
                .unwrap_or_else(|e| panic!("scale {k}: refused: {e}"));
            let defects = check(&part.solid.body, &tol);
            assert!(defects.is_empty(), "scale {k}: {defects:?}");
            for c in &part.curved {
                assert!(
                    c.joined.iter().all(Option::is_some),
                    "scale {k}: not joined"
                );
            }
            let theta = FRAC_PI_2;
            let ba = theta * (r + kk * t);
            let base = (100.0 * 60.0 - (4.0 - PI) * 15.0 * 15.0) * k * k;
            let walls = 2.0 * (70.0 + 30.0) * k;
            let wall_volume = walls * (theta * t * (r + 0.5 * t) + l * t);
            let corner_volume = 4.0 * FRAC_PI_2 * pappus(rc, 1.0, theta, r, t, l);
            let want = base * t + wall_volume + corner_volume;
            let v = mass_properties(&part.solid.body).unwrap().volume;
            near(v, want, 1e-7 * want, "tray volume");
            let tip = rc + ba + l;
            let area = base + walls * (ba + l) + PI * (tip * tip - rc * rc);
            near(flat_area(&part), area, 1e-9 * area, "tray flat area");
            let chains = outline(&part, &tol);
            assert_eq!(chains.len(), 1, "scale {k}: one outline");
            // The outline is the base grown by BA + L all round: four lines
            // and four arcs of radius rc + BA + L.
            assert_eq!(chains[0].len(), 8, "scale {k}: {:?}", chains[0].len());
        }
    }
}

/// A wall beside a corner's arc that is not alike (another length) is not
/// continued: their strips lie along each other in the pattern, refused.
#[test]
fn a_wall_unlike_its_corner_is_refused() {
    let k = 1.0;
    let tol = tolerance(k);
    let mut sp = spec(rounded(15.0, k), 1.0);
    sp.flanges = vec![straight(1, 12.0, true, 2.0, 0.4)];
    let corner = curved(5, 10.0, FRAC_PI_2, true, 2.0, 0.4);
    match build_with(&sp, &[corner], None, &tol) {
        Err(KernelError::SheetMetal { reason, .. }) => {
            assert!(reason.contains("overlaps itself"), "{reason}")
        }
        other => panic!("{other:?}"),
    }
}
