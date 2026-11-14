//! Tests of the tip's nets against closed forms: the half ball, the
//! half ellipsoid, the volume over any prism, the ball that fits a cone
//! and a zone of a ball; the three tangencies measured; the conditions
//! at their true limits; hostile input; determinism.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use super::*;
use craft3d_math::quadrature::Rule;
use craft3d_math::{Frame, Portable, bits_of};
use std::f64::consts::{FRAC_1_SQRT_2, PI, SQRT_2};

/// The fullness of quarter ellipses: `ω = √½`.
const ROUND: f64 = SQRT_2 - 1.0;
/// The fullness of parabolas: `ω = 1`.
const PARABOLA: f64 = 0.5;
const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// One Bézier piece of a section in its plane: degree, control points,
/// weights.
type Piece = (usize, Vec<[f64; 2]>, Vec<f64>);

fn frames(k: f64) -> [Frame; 2] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(3.0 * k, -2.0 * k, 5.0 * k),
            Vec3::new(1.0, 2.0, 2.0),
            Vec3::new(1.0, 0.0, 0.0),
        )
        .unwrap(),
    ]
}

/// A circle of radius `r` about the origin in four rational quarters,
/// counter-clockwise, scaled to an ellipse by `(a, b)`.
fn ellipse(a: f64, b: f64) -> Vec<Piece> {
    let quarters = [
        [(1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
        [(0.0, 1.0), (-1.0, 1.0), (-1.0, 0.0)],
        [(-1.0, 0.0), (-1.0, -1.0), (0.0, -1.0)],
        [(0.0, -1.0), (1.0, -1.0), (1.0, 0.0)],
    ];
    quarters
        .iter()
        .map(|q| {
            (
                2,
                q.iter().map(|&(x, y)| [a * x, b * y]).collect(),
                vec![1.0, FRAC_1_SQRT_2, 1.0],
            )
        })
        .collect()
}

/// Two cubic arcs, tangent where they meet: 12 by 1.5.
fn oval() -> Vec<Piece> {
    let (a, b) = (6.0, 1.0);
    vec![
        (3, vec![[a, 0.0], [a, b], [-a, b], [-a, 0.0]], vec![1.0; 4]),
        (
            3,
            vec![[-a, 0.0], [-a, -b], [a, -b], [a, 0.0]],
            vec![1.0; 4],
        ),
    ]
}

/// Two parabolas with two sharp corners: 12 by 2.
fn lens() -> Vec<Piece> {
    let (a, b) = (6.0, 1.0);
    vec![
        (2, vec![[a, 0.0], [0.0, 2.0 * b], [-a, 0.0]], vec![1.0; 3]),
        (2, vec![[-a, 0.0], [0.0, -2.0 * b], [a, 0.0]], vec![1.0; 3]),
    ]
}

/// A cambered deck over a U, sharp at the sheer: 16 by about 22.
fn hull() -> Vec<Piece> {
    let (w, d, camber) = (8.0, 28.0, 1.92);
    vec![
        (2, vec![[w, 0.0], [0.0, camber], [-w, 0.0]], vec![1.0; 3]),
        (
            3,
            vec![[-w, 0.0], [-w, -d], [w, -d], [w, 0.0]],
            vec![1.0; 4],
        ),
    ]
}

/// Straight sides through the corners given, counter-clockwise.
fn polygon(corners: &[[f64; 2]]) -> Vec<Piece> {
    (0..corners.len())
        .map(|i| {
            (
                1,
                vec![corners[i], corners[(i + 1) % corners.len()]],
                vec![1.0; 2],
            )
        })
        .collect()
}

fn square() -> Vec<Piece> {
    polygon(&[[1.0, -1.0], [1.0, 1.0], [-1.0, 1.0], [-1.0, -1.0]])
}

fn one_piece(degree: usize) -> Vec<f64> {
    one_span(degree)
}

/// The section's curves in the xy plane of `frame`, scaled by `k`.
fn rim_curves(frame: &Frame, k: f64, section: &[Piece]) -> Vec<NurbsCurve> {
    section
        .iter()
        .map(|(degree, points, weights)| {
            NurbsCurve::new(
                *degree,
                one_piece(*degree),
                points
                    .iter()
                    .map(|p| frame.point(p[0] * k, p[1] * k, 0.0))
                    .collect(),
                weights.clone(),
            )
            .unwrap()
        })
        .collect()
}

/// A side face that ends on `rim`: ruled from the row `back` gives for
/// every control point (index, point, weight) to the rim's own row.
fn side_to(rim: &NurbsCurve, back: impl Fn(usize, Vec3, f64) -> (Vec3, f64)) -> NurbsSurface {
    let mut points = Vec::new();
    let mut weights = Vec::new();
    for (i, (&c, &w)) in rim.control_points().iter().zip(rim.weights()).enumerate() {
        let (b, wb) = back(i, c, w);
        points.extend([b, c]);
        weights.extend([wb, w]);
    }
    NurbsSurface::new(
        rim.degree(),
        1,
        rim.knots().to_vec(),
        one_piece(1),
        rim.control_points().len(),
        2,
        points,
        weights,
    )
    .unwrap()
}

fn rims_of(sides: &[NurbsSurface]) -> Vec<TipRim> {
    sides
        .iter()
        .map(|s| TipRim::of_border(s, Border::VEnd).unwrap())
        .collect()
}

/// The sides of a prism on the section: straight back along the frame's
/// z axis.
fn prism(frame: &Frame, k: f64, section: &[Piece]) -> Vec<NurbsSurface> {
    rim_curves(frame, k, section)
        .iter()
        .map(|rim| side_to(rim, |_, c, w| (c - frame.z() * (2.0 * k), w)))
        .collect()
}

/// The sides of a cone on the section: straight back from `apex`.
fn cone(frame: &Frame, k: f64, section: &[Piece], apex: Vec3) -> Vec<NurbsSurface> {
    rim_curves(frame, k, section)
        .iter()
        .map(|rim| side_to(rim, |_, c, w| (apex + (c - apex) * 1.5, w)))
        .collect()
}

fn spec(tip: Vec3, normal: Vec3, fullness: f64, k: f64) -> TipSpec {
    TipSpec {
        tip,
        normal,
        fullness,
        tolerance: 1e-9 * k,
        angular: 1e-9,
    }
}

/// The distinct knot spans of a clamped knot vector.
fn spans(knots: &[f64]) -> Vec<(f64, f64)> {
    knots
        .windows(2)
        .filter(|w| w[1] > w[0])
        .map(|w| (w[0], w[1]))
        .collect()
}

/// The volume between the tip's faces and the plane through `on` square
/// to the normal the rim lies in: the divergence theorem about `on`, by
/// Gauss–Legendre on every span.
fn volume(tip: &Tip, on: Vec3) -> f64 {
    let rule = Rule::new(40);
    let mut sum = 0.0;
    for face in &tip.faces {
        for (u0, u1) in spans(face.knots_u()) {
            for (u, wu) in rule.points(u0, u1) {
                for (v, wv) in rule.points(0.0, 1.0) {
                    let (p, du, dv) = face.derivatives(u, v);
                    sum += wu * wv * (p - on).dot(du.cross(dv)) / 3.0;
                }
            }
        }
    }
    if tip.counter_clockwise { sum } else { -sum }
}

/// The area the rim curves enclose, seen against `normal`.
fn area(rims: &[NurbsCurve], on: Vec3, normal: Vec3) -> f64 {
    let rule = Rule::new(40);
    let mut sum = 0.0;
    for rim in rims {
        for (t0, t1) in spans(rim.knots()) {
            for (t, w) in rule.points(t0, t1) {
                let (p, d) = rim.derivatives(t);
                sum += w * 0.5 * normal.dot((p - on).cross(d));
            }
        }
    }
    sum
}

fn normal_of(face: &NurbsSurface, u: f64, v: f64) -> Vec3 {
    let (_, du, dv) = face.derivatives(u, v);
    du.cross(dv)
}

/// The largest angle between the side's normal and the tip's along every
/// rim curve, at 33 places of every span.
fn rim_angle(sides: &[NurbsSurface], tip: &Tip) -> f64 {
    let mut worst = 0.0_f64;
    for (side, face) in sides.iter().zip(&tip.faces) {
        let (_, (_, v_end)) = side.domain();
        for (u0, u1) in spans(face.knots_u()) {
            for i in 0..=32 {
                let u = u0 + (u1 - u0) * f64::from(i) / 32.0;
                worst = worst.max(normal_of(side, u, v_end).angle_to(normal_of(face, u, 0.0)));
            }
        }
    }
    worst
}

/// The angles between the faces' normals and the tip's plane's, on every
/// rail (seen from both of its faces) at `off` of the way before the tip.
fn tip_angle(tip: &Tip, normal: Vec3, off: f64) -> f64 {
    let mut worst = 0.0_f64;
    let sign = if tip.counter_clockwise { 1.0 } else { -1.0 };
    for face in &tip.faces {
        let ((u0, u1), _) = face.domain();
        for u in [u0, u1, 0.5 * (u0 + u1)] {
            worst = worst.max((normal_of(face, u, 1.0 - off) * sign).angle_to(normal));
        }
    }
    worst
}

/// The angle between the two faces that meet on rail `k`, at `v`.
fn rail_angle(tip: &Tip, k: usize, v: f64) -> f64 {
    let count = tip.faces.len();
    let (here, before) = (&tip.faces[k], &tip.faces[(k + count - 1) % count]);
    let ((u0, _), _) = here.domain();
    let ((_, u1), _) = before.domain();
    normal_of(here, u0, v).angle_to(normal_of(before, u1, v))
}

fn assert_near(got: f64, want: f64, relative: f64, what: &str) -> f64 {
    let off = ((got - want) / want).abs();
    assert!(
        (got - want).abs() <= relative * want.abs(),
        "{what}: got {got:e}, want {want:e} (off by {off:e})"
    );
    off
}

// ------------------------------------------------------- closed forms

#[test]
fn a_circle_on_a_cylinder_closes_as_the_half_ball() {
    let (mut off_surface, mut off_volume) = (0.0_f64, 0.0_f64);
    for k in SCALES {
        for frame in frames(k) {
            let r = 5.0 * k;
            let centre = frame.origin();
            let quarters = rims_of(&prism(&frame, k, &ellipse(5.0, 5.0)));
            // The same circle as one closed curve, which is cut in two.
            let circle = NurbsCurve::circle(&frame, r).unwrap();
            let whole = vec![TipRim {
                cross: circle
                    .weights()
                    .iter()
                    .map(|&w| {
                        let z = frame.z() * w;
                        [z.x, z.y, z.z, 0.0]
                    })
                    .collect(),
                curve: circle,
            }];
            for (rims, faces) in [(quarters, 4), (whole, 2)] {
                let tip =
                    tip_nets(&rims, &spec(centre + frame.z() * r, frame.z(), ROUND, k)).unwrap();
                assert_eq!(tip.form, TipForm::Level);
                assert_eq!(
                    (tip.faces.len(), tip.rails.len(), tip.rims.len()),
                    (faces, faces, faces)
                );
                assert!(tip.counter_clockwise);
                let mut worst = 0.0_f64;
                for face in &tip.faces {
                    let ((u0, u1), _) = face.domain();
                    for i in 0..=16 {
                        for j in 0..=16 {
                            let (u, v) =
                                (u0 + (u1 - u0) * f64::from(i) / 16.0, f64::from(j) / 16.0);
                            let p = face.eval(u, v);
                            worst = worst.max((p.distance(centre) - r).abs() / r);
                            assert!(frame.z().dot(p - centre) >= -1e-12 * r);
                        }
                    }
                }
                assert!(worst <= 1e-12, "off the ball by {worst:e} at {k}");
                off_surface = off_surface.max(worst);
                off_volume = off_volume.max(assert_near(
                    volume(&tip, centre),
                    2.0 * PI * r * r * r / 3.0,
                    1e-12,
                    "volume",
                ));
                // The rails are quarter circles to the pole.
                for rail in &tip.rails {
                    for i in 0..=8 {
                        let p = rail.eval(f64::from(i) / 8.0);
                        assert!((p.distance(centre) - r).abs() <= 1e-12 * r);
                    }
                    assert_eq!(rail.eval(1.0), tip.tip);
                }
            }
        }
    }
    println!("half ball: {off_surface:.1e} off the ball, volume {off_volume:.1e} off");
}

#[test]
fn an_elliptic_prism_closes_as_the_half_ellipsoid() {
    let (mut off_surface, mut off_volume) = (0.0_f64, 0.0_f64);
    for k in SCALES {
        for frame in frames(k) {
            let (a, b, c) = (7.0, 2.0, 3.0);
            let sides = prism(&frame, k, &ellipse(a, b));
            let tip = tip_nets(
                &rims_of(&sides),
                &spec(frame.point(0.0, 0.0, c * k), frame.z(), ROUND, k),
            )
            .unwrap();
            let mut worst = 0.0_f64;
            for face in &tip.faces {
                for i in 0..=16 {
                    for j in 0..=16 {
                        let (x, y, z) =
                            frame.local(face.eval(f64::from(i) / 16.0, f64::from(j) / 16.0));
                        let (x, y, z) = (x / (a * k), y / (b * k), z / (c * k));
                        worst = worst.max((x * x + y * y + z * z - 1.0).abs());
                    }
                }
            }
            assert!(worst <= 2e-12, "off the ellipsoid by {worst:e} at {k}");
            off_surface = off_surface.max(worst);
            let want = 2.0 * PI * a * b * c * k * k * k / 3.0;
            off_volume = off_volume.max(assert_near(
                volume(&tip, frame.origin()),
                want,
                1e-12,
                "volume",
            ));
            assert!(rim_angle(&sides, &tip) < 1e-13);
        }
    }
    println!("half ellipsoid: {off_surface:.1e} off its equation, volume {off_volume:.1e} off");
}

#[test]
fn over_any_prism_the_volume_is_a_fixed_share_of_base_times_length() {
    let mut off_volume = 0.0_f64;
    let sections: [(&str, Vec<Piece>); 5] = [
        ("oval", oval()),
        ("lens", lens()),
        ("hull", hull()),
        ("square", square()),
        ("ellipse", ellipse(3.0, 1.0)),
    ];
    for k in SCALES {
        for frame in frames(k) {
            for (name, section) in &sections {
                let sides = prism(&frame, k, section);
                let rims = rims_of(&sides);
                let curves = rim_curves(&frame, k, section);
                let base = area(&curves, frame.origin(), frame.z());
                assert!(base > 0.0);
                // Anywhere over the section: the slices are the section
                // scaled about the tip's foot, whatever the foot.
                let foot = if *name == "hull" {
                    [0.5, -9.0]
                } else {
                    [0.2, 0.1]
                };
                let length = 3.0 * k;
                for (fullness, share) in [(ROUND, 2.0 / 3.0), (PARABOLA, 11.0 / 15.0)] {
                    let at = frame.point(foot[0] * k, foot[1] * k, length);
                    let tip = tip_nets(&rims, &spec(at, frame.z(), fullness, k)).unwrap();
                    assert_eq!(tip.form, TipForm::Level, "{name}");
                    off_volume = off_volume.max(assert_near(
                        volume(&tip, frame.origin()),
                        share * base * length,
                        1e-12,
                        &format!("{name} at {k}, fullness {fullness}"),
                    ));
                }
            }
        }
    }
    println!("prisms: volume {off_volume:.1e} off its share of base times length");
}

/// The fullness whose conic weight is `omega`.
fn fullness_of(omega: f64) -> f64 {
    omega / (1.0 + omega)
}

/// The zone of a ball of radius `r` about the frame's origin from the
/// equator up to the latitude `to`: a rational surface of revolution
/// whose weights change across its upper border (the arc's times the
/// circle's).
fn ball_zone(frame: &Frame, r: f64, to: f64) -> NurbsSurface {
    // (The unit circle's own numbers: read back through a frame far from
    // the origin they would be rounded at its distance.)
    let circle = NurbsCurve::circle(&Frame::WORLD, 1.0).unwrap();
    let half = 0.5 * to;
    let arc = [
        (r, 0.0, 1.0),
        (r, r * half.ptan(), half.pcos()),
        (r * to.pcos(), r * to.psin(), 1.0),
    ];
    let mut points = Vec::new();
    let mut weights = Vec::new();
    for (c, wc) in circle.control_points().iter().zip(circle.weights()) {
        let (x, y) = (c.x, c.y);
        for (rho, z, wa) in arc {
            points.push(frame.point(rho * x, rho * y, z));
            weights.push(wc * wa);
        }
    }
    NurbsSurface::new(
        2,
        2,
        circle.knots().to_vec(),
        one_piece(2),
        9,
        3,
        points,
        weights,
    )
    .unwrap()
}

#[test]
fn a_zone_of_a_ball_closes_as_the_ball() {
    let (mut off_surface, mut off_volume) = (0.0_f64, 0.0_f64);
    // The side's weights change across the rim here, in one ratio with
    // the rim's: the tip must not see that, only the surface.
    for k in SCALES {
        for frame in frames(k) {
            for to in [0.3_f64, 0.9, 1.3] {
                let r = 4.0 * k;
                let zone = ball_zone(&frame, r, to);
                let rims = vec![TipRim::of_border(&zone, Border::VEnd).unwrap()];
                let omega = (0.5 * (0.5 * PI - to)).pcos();
                let pole = frame.point(0.0, 0.0, r);
                let tip = tip_nets(&rims, &spec(pole, frame.z(), fullness_of(omega), k)).unwrap();
                assert_eq!((tip.form, tip.faces.len()), (TipForm::Level, 2));
                let mut worst = 0.0_f64;
                for face in &tip.faces {
                    let ((u0, u1), _) = face.domain();
                    for i in 0..=16 {
                        for j in 0..=16 {
                            let u = u0 + (u1 - u0) * f64::from(i) / 16.0;
                            let p = face.eval(u, f64::from(j) / 16.0);
                            worst = worst.max((p.distance(frame.origin()) - r).abs() / r);
                        }
                    }
                }
                assert!(worst <= 1e-12, "off the ball by {worst:e} at {k}, {to}");
                off_surface = off_surface.max(worst);
                // The cap's volume, measured from the rim's plane.
                let h = r * (1.0 - to.psin());
                let cap = PI * h * h * (3.0 * r - h) / 3.0;
                let on = frame.point(0.0, 0.0, r * to.psin());
                off_volume =
                    off_volume.max(assert_near(volume(&tip, on), cap, 1e-11, "cap volume"));
            }
        }
    }
    println!("zone of a ball: {off_surface:.1e} off the ball, cap volume {off_volume:.1e} off");
}

#[test]
fn a_cone_closes_as_the_ball_that_fits_it() {
    let mut off_surface = 0.0_f64;
    for k in SCALES {
        for frame in frames(k) {
            let (r, height) = (5.0 * k, 12.0 * k);
            let apex = frame.point(0.0, 0.0, height);
            let sides = cone(&frame, k, &ellipse(5.0, 5.0), apex);
            // The side makes the angle alpha with the axis.
            let alpha = (r / height).patan();
            let ball = r / alpha.pcos();
            let length = r * alpha.pcos() / (1.0 + alpha.psin());
            let omega = (0.5 * (0.5 * PI - alpha)).pcos();
            let centre = frame.point(0.0, 0.0, -ball * alpha.psin());
            let tip = tip_nets(
                &rims_of(&sides),
                &spec(
                    frame.point(0.0, 0.0, length),
                    frame.z(),
                    fullness_of(omega),
                    k,
                ),
            )
            .unwrap();
            assert_eq!(tip.form, TipForm::Level);
            // How steeply the side leaves the end is proven on the control
            // points, and the middle one of every quarter arc lies √2 as
            // far from the axis as the circle: the bound is that much
            // below the truth here, and exact where it is the same vector
            // all round (`a_side_must_leave_the_end_toward_the_tip`).
            let truth = height / r.phypot(height);
            let bound = height / (SQRT_2 * r).phypot(height);
            assert!(bound < truth);
            let _ = assert_near(tip.onward, bound, 1e-12, "the proven sine");
            let mut worst = 0.0_f64;
            for face in &tip.faces {
                for i in 0..=16 {
                    for j in 0..=16 {
                        let p = face.eval(f64::from(i) / 16.0, f64::from(j) / 16.0);
                        worst = worst.max((p.distance(centre) - ball).abs() / ball);
                    }
                }
            }
            assert!(worst <= 1e-12, "off the ball by {worst:e} at {k}");
            off_surface = off_surface.max(worst);
            assert!(rim_angle(&sides, &tip) < 1e-13);
        }
    }
    println!("cone: {off_surface:.1e} off the ball that fits it");
}

// ----------------------------------------------------------- tangency

/// A section by name, with the foot of its tip and whether each of its
/// vertices is smooth.
type Case = (&'static str, Vec<Piece>, [f64; 2], Vec<bool>);

/// The sections the tangencies are measured on.
fn sections() -> Vec<Case> {
    vec![
        ("oval", oval(), [0.3, 0.1], vec![true, true]),
        ("lens", lens(), [0.3, 0.1], vec![false, false]),
        ("hull", hull(), [0.5, -9.0], vec![false, false]),
        ("square", square(), [0.2, -0.1], vec![false; 4]),
        ("ellipse", ellipse(3.0, 1.0), [0.2, 0.1], vec![true; 4]),
    ]
}

#[test]
fn the_tip_runs_on_from_the_sides_along_the_rim() {
    let mut worst = 0.0_f64;
    for k in SCALES {
        for frame in frames(k) {
            for (name, section, foot, _) in sections() {
                let apex = frame.point(foot[0] * k, foot[1] * k, 40.0 * k);
                for sides in [prism(&frame, k, &section), cone(&frame, k, &section, apex)] {
                    let at = frame.point(foot[0] * k, foot[1] * k, 3.0 * k);
                    let tip = tip_nets(&rims_of(&sides), &spec(at, frame.z(), ROUND, k)).unwrap();
                    let angle = rim_angle(&sides, &tip);
                    assert!(angle < 1e-13, "{name} at {k}: {angle:e} rad");
                    worst = worst.max(angle);
                }
            }
        }
    }
    println!("rim tangency, level: at most {worst:e} rad");
}

#[test]
fn the_faces_reach_the_tip_with_one_tangent_plane() {
    for k in SCALES {
        for frame in frames(k) {
            for (name, section, foot, _) in sections() {
                let sides = prism(&frame, k, &section);
                let at = frame.point(foot[0] * k, foot[1] * k, 3.0 * k);
                let tip = tip_nets(&rims_of(&sides), &spec(at, frame.z(), ROUND, k)).unwrap();
                let angles = [1e-2, 1e-3, 1e-4].map(|off| tip_angle(&tip, frame.z(), off));
                if k == 1.0 {
                    println!("tip tangency, {name}: {angles:.3?} rad");
                }
                // Linear convergence: a tenth as far, a tenth of the angle.
                for pair in angles.windows(2) {
                    let ratio = pair[0] / pair[1];
                    assert!((9.0..11.0).contains(&ratio), "{name} at {k}: {angles:.3?}");
                }
                assert!(angles[2] < 1e-3, "{name}: {angles:.3?}");
            }
        }
    }
}

#[test]
fn across_a_rail_the_faces_are_tangent_where_the_section_is_smooth() {
    let mut worst = 0.0_f64;
    for k in SCALES {
        for frame in frames(k) {
            for (name, section, foot, smooth) in sections() {
                let apex = frame.point(foot[0] * k, foot[1] * k, 40.0 * k);
                for sides in [prism(&frame, k, &section), cone(&frame, k, &section, apex)] {
                    let at = frame.point(foot[0] * k, foot[1] * k, 3.0 * k);
                    let tip = tip_nets(&rims_of(&sides), &spec(at, frame.z(), ROUND, k)).unwrap();
                    assert_eq!(tip.smooth, smooth, "{name}");
                    for (rail, &smooth) in smooth.iter().enumerate() {
                        if smooth {
                            for i in 0..=99 {
                                let angle = rail_angle(&tip, rail, f64::from(i) / 100.0);
                                assert!(angle < 1e-12, "{name} rail {rail}: {angle:e}");
                                worst = worst.max(angle);
                            }
                        } else {
                            // A corner of the section goes on as an edge
                            // whose angle falls all the way, and in
                            // proportion at the tip.
                            let angles: Vec<f64> = (0..=20)
                                .map(|i| rail_angle(&tip, rail, f64::from(i) / 20.0 * 0.999))
                                .collect();
                            assert!(angles[0] > 0.3, "{name}: {angles:.3?}");
                            for pair in angles.windows(2) {
                                assert!(pair[1] < pair[0], "{name}: {angles:.3?}");
                            }
                            let near =
                                [1e-2, 1e-3, 1e-4].map(|off| rail_angle(&tip, rail, 1.0 - off));
                            if k == 1.0 && frame == Frame::WORLD && rail == 0 {
                                println!(
                                    "corner rail, {name}: {:.3e} rad at the rim, {near:.3?} toward the tip",
                                    angles[0]
                                );
                            }
                            for pair in near.windows(2) {
                                let ratio = pair[0] / pair[1];
                                assert!((9.0..11.0).contains(&ratio), "{name}: {near:.3?}");
                            }
                        }
                    }
                }
            }
        }
    }
    println!("across smooth rails, level: at most {worst:e} rad");
}

// ------------------------------------------------------ the conditions

/// The rims of sides that lean: every cross vector the same, `lean` in
/// the section's plane per unit of height.
fn leaning(frame: &Frame, k: f64, section: &[Piece], lean: [f64; 2], rise: f64) -> Vec<TipRim> {
    let d = frame.vector(lean[0], lean[1], rise);
    rim_curves(frame, k, section)
        .into_iter()
        .map(|curve| TipRim {
            cross: curve
                .weights()
                .iter()
                .map(|&w| [d.x * w, d.y * w, d.z * w, 0.0])
                .collect(),
            curve,
        })
        .collect()
}

#[test]
fn a_side_must_leave_the_end_toward_the_tip() {
    for k in SCALES {
        for frame in frames(k) {
            // A sheared prism: the sine of every side's angle with the
            // end is the rise of its direction, so the limit is exact.
            let build = |sine: f64| {
                let run = (1.0 - sine * sine).sqrt();
                let rims = leaning(&frame, k, &square(), [run, 0.0], sine);
                let length = 1e-3 * k;
                // The tip over the middle of what the sides lean to.
                let at = frame.point(length * run / sine, 0.0, length);
                tip_nets(&rims, &spec(at, frame.z(), ROUND, k))
            };
            let tip = build(1.5 * ONWARD_MARGIN).unwrap();
            let _ = assert_near(tip.onward, 1.5 * ONWARD_MARGIN, 1e-9, "the proven sine");
            assert!(matches!(
                build(0.7 * ONWARD_MARGIN),
                Err(TipError::NotOnward { rim: 0, control: 0, sine }) if (sine / (0.7 * ONWARD_MARGIN) - 1.0).abs() < 1e-9
            ));
            // Along the end, and back into the body.
            for rise in [0.0, -0.5] {
                let rims = leaning(&frame, k, &square(), [1.0, 0.0], rise);
                let at = frame.point(0.0, 0.0, k);
                assert!(matches!(
                    tip_nets(&rims, &spec(at, frame.z(), ROUND, k)),
                    Err(TipError::NotOnward { .. })
                ));
            }
        }
    }
}

/// A prism's tip at `foot` (in the section's plane) and one unit long.
fn over(frame: &Frame, k: f64, section: &[Piece], foot: [f64; 2]) -> Result<Tip, TipError> {
    let rims = rims_of(&prism(frame, k, section));
    let at = frame.point(foot[0] * k, foot[1] * k, k);
    tip_nets(&rims, &spec(at, frame.z(), ROUND, k))
}

#[test]
fn the_tip_must_stand_inside_what_the_sides_are_carried_on_to() {
    let ell = polygon(&[
        [0.0, 0.0],
        [2.0, 0.0],
        [2.0, 1.0],
        [1.0, 1.0],
        [1.0, 2.0],
        [0.0, 2.0],
    ]);
    for k in SCALES {
        for frame in frames(k) {
            // A square of side 2: star-shaped about every point inside.
            assert!(over(&frame, k, &square(), [1.0 - 1e-6, 0.3]).is_ok());
            assert!(over(&frame, k, &square(), [-0.999, 0.999]).is_ok());
            for foot in [[1.0 + 1e-6, 0.3], [1.0, 0.3], [3.0, 0.0], [1.0, 1.0]] {
                assert!(
                    matches!(
                        over(&frame, k, &square(), foot),
                        Err(TipError::NotStarShaped { .. } | TipError::NotProven { .. })
                    ),
                    "{foot:?}"
                );
            }
            assert!(matches!(
                over(&frame, k, &square(), [1.0 + 1e-6, 0.3]),
                Err(TipError::NotStarShaped { rim: 0, .. })
            ));
            // An L: seen whole only from the square in its corner.
            assert!(over(&frame, k, &ell, [1.0 - 1e-6, 0.5]).is_ok());
            assert!(over(&frame, k, &ell, [0.5, 1.0 - 1e-6]).is_ok());
            assert!(matches!(
                over(&frame, k, &ell, [1.0 + 1e-6, 0.5]),
                Err(TipError::NotStarShaped { .. })
            ));
            assert!(matches!(
                over(&frame, k, &ell, [0.5, 1.0 + 1e-6]),
                Err(TipError::NotStarShaped { .. })
            ));
        }
    }
}

/// A blade-like end: an ellipse 12 by 2 whose sides close to a ridge,
/// across its thickness at the height `thin`, along its chord at `long`.
fn wedge(frame: &Frame, k: f64, thin: f64, long: f64) -> Vec<TipRim> {
    rim_curves(frame, k, &ellipse(6.0, 1.0))
        .into_iter()
        .map(|curve| TipRim {
            cross: curve
                .control_points()
                .iter()
                .zip(curve.weights())
                .map(|(c, &w)| {
                    let (x, y, _) = frame.local(*c);
                    let d = frame.vector(-x / (long * k), -y / (thin * k), 1.0);
                    [d.x * w, d.y * w, d.z * w, 0.0]
                })
                .collect(),
            curve,
        })
        .collect()
}

#[test]
fn the_sides_must_not_close_in_before_the_tip() {
    for k in SCALES {
        for frame in frames(k) {
            // A cone closes at its apex: the carried section shrinks to a
            // point there and comes out turned half round beyond, which
            // is star-shaped about the tip and runs the rim's way again.
            // Only the rim, seen from the tip, tells.
            let height = 10.0;
            let apex = frame.point(0.0, 0.0, height * k);
            let rims = rims_of(&cone(&frame, k, &ellipse(5.0, 5.0), apex));
            let build = |length: f64| {
                tip_nets(
                    &rims,
                    &spec(frame.point(0.0, 0.0, length * k), frame.z(), ROUND, k),
                )
            };
            assert!(build(height * (1.0 - 1e-6)).is_ok());
            for beyond in [height * (1.0 + 1e-6), 1.5 * height, 3.0 * height] {
                assert!(
                    matches!(build(beyond), Err(TipError::ClosesIn { rim: 0, across, .. }) if across < 1.0),
                    "{beyond}: {:?}",
                    build(beyond).map(|t| t.star)
                );
            }
            assert!(build(height).is_err());
            // A blade closes across its thickness first: the carried
            // section is a line there and inside out beyond.
            let rims = wedge(&frame, k, 4.0, 20.0);
            let build = |length: f64| {
                tip_nets(
                    &rims,
                    &spec(frame.point(0.0, 0.0, length * k), frame.z(), ROUND, k),
                )
            };
            assert!(build(4.0 * (1.0 - 1e-6)).is_ok());
            assert!(matches!(
                build(4.0 * (1.0 + 1e-6)),
                Err(TipError::NotStarShaped { .. })
            ));
            assert!(build(4.0).is_err());
        }
    }
}

#[test]
fn the_longest_length_is_bracketed_at_the_true_limit() {
    for k in SCALES {
        for frame in frames(k) {
            let tolerance = 1e-9 * k;
            let search = |rims: &[TipRim], foot: [f64; 2], upto: f64| {
                longest_tip_length(
                    rims,
                    frame.point(foot[0] * k, foot[1] * k, 0.0),
                    frame.z(),
                    ROUND,
                    tolerance,
                    upto * k,
                )
            };
            let report = |what: &str, found: TipLength, limit: f64| {
                // The limit lies between the two, which are a billionth
                // apart: the last length that is proven and the first
                // that is not.
                let fails = found.fails.unwrap();
                assert!(found.fits < limit * k, "{what}: {found:?}");
                assert!(fails > limit * k * (1.0 - 1e-9), "{what}: {found:?}");
                assert!(fails - found.fits <= LENGTH_RESOLUTION * fails);
                assert_eq!(found.again, None);
                let short = 1.0 - found.fits / (limit * k);
                assert!(short < 2e-9, "{what}: {short:e} short of the limit");
                if k == 1.0 && frame == Frame::WORLD {
                    println!("longest length, {what}: {short:.2e} short of the true limit");
                }
            };
            // A cone: its apex.
            let apex = frame.point(0.0, 0.0, 10.0 * k);
            let rims = rims_of(&cone(&frame, k, &ellipse(5.0, 5.0), apex));
            report("cone", search(&rims, [0.0, 0.0], 37.0).unwrap(), 10.0);
            // A cone over a hull-like section with corners.
            let apex = frame.point(0.5 * k, -9.0 * k, 25.0 * k);
            let rims = rims_of(&cone(&frame, k, &hull(), apex));
            report("hull cone", search(&rims, [0.5, -9.0], 80.0).unwrap(), 25.0);
            // A blade: where its thickness closes.
            let rims = wedge(&frame, k, 4.0, 20.0);
            report("blade", search(&rims, [0.0, 0.0], 9.0).unwrap(), 4.0);
            // A sheared rod with the tip straight over the middle of its
            // end: where the rod has leant away from under it.
            let rims = leaning(&frame, k, &ellipse(5.0, 5.0), [1.0, 0.0], 1.0);
            report("sheared rod", search(&rims, [0.0, 0.0], 13.0).unwrap(), 5.0);
            // A prism takes any length; over a foot outside it, none.
            let rims = rims_of(&prism(&frame, k, &lens()));
            assert_eq!(
                search(&rims, [1.0, 0.2], 50.0).unwrap(),
                TipLength {
                    fits: 50.0 * k,
                    fails: None,
                    again: None,
                }
            );
            assert_eq!(search(&rims, [7.0, 0.0], 50.0), Err(TipError::NoLengthFits));
        }
    }
}

#[test]
fn the_shoulder_is_what_the_sides_are_carried_on_to() {
    for k in SCALES {
        for frame in frames(k) {
            let apex = frame.point(0.5 * k, -9.0 * k, 25.0 * k);
            let rims = rims_of(&cone(&frame, k, &hull(), apex));
            let on = frame.point(100.0 * k, 0.0, 5.0 * k);
            let (form, shoulder) = tip_shoulder(&rims, on, frame.z(), 1e-9 * k).unwrap();
            assert_eq!((form, shoulder.len()), (TipForm::Level, 2));
            // A fifth of the way to the apex: the section at four fifths.
            for (curve, rim) in shoulder.iter().zip(&rims) {
                for i in 0..=8 {
                    let t = f64::from(i) / 8.0;
                    let want = apex + (rim.curve.eval(t) - apex) * 0.8;
                    assert!(curve.eval(t).distance(want) < 1e-13 * 30.0 * k);
                }
            }
        }
    }
}

// -------------------------------------------------- borders and turns

#[test]
fn a_rim_is_read_from_any_border_of_a_side_and_either_way_round() {
    let frame = frames(1.0)[1];
    let sides = prism(&frame, 1.0, &hull());
    let rims = rims_of(&sides);
    for (side, rim) in sides.iter().zip(&rims) {
        let (count_u, count_v) = side.counts();
        let (pu, pv) = side.degrees();
        let at = |i: usize, j: usize| {
            (
                side.control_points()[i * count_v + j],
                side.weights()[i * count_v + j],
            )
        };
        // The same face with its rows the other way: the rim at v's start.
        let turned: Vec<(Vec3, f64)> = (0..count_u)
            .flat_map(|i| (0..count_v).rev().map(move |j| (i, j)))
            .map(|(i, j)| at(i, j))
            .collect();
        let start = NurbsSurface::new(
            pu,
            pv,
            side.knots_u().to_vec(),
            side.knots_v().to_vec(),
            count_u,
            count_v,
            turned.iter().map(|x| x.0).collect(),
            turned.iter().map(|x| x.1).collect(),
        )
        .unwrap();
        assert_eq!(&TipRim::of_border(&start, Border::VStart).unwrap(), rim);
        // And with u and v changed places: the rim at u's end, and start.
        for (source, border) in [(side, Border::UEnd), (&start, Border::UStart)] {
            let (source_points, source_weights) = (source.control_points(), source.weights());
            let swapped: Vec<(Vec3, f64)> = (0..count_v)
                .flat_map(|j| (0..count_u).map(move |i| (i, j)))
                .map(|(i, j)| {
                    (
                        source_points[i * count_v + j],
                        source_weights[i * count_v + j],
                    )
                })
                .collect();
            let other = NurbsSurface::new(
                pv,
                pu,
                source.knots_v().to_vec(),
                source.knots_u().to_vec(),
                count_v,
                count_u,
                swapped.iter().map(|x| x.0).collect(),
                swapped.iter().map(|x| x.1).collect(),
            )
            .unwrap();
            assert_eq!(&TipRim::of_border(&other, border).unwrap(), rim);
        }
        assert_eq!(&rim.reversed().unwrap().reversed().unwrap(), rim);
    }
    // The loop the other way round: the same tip, its faces facing in.
    let at = frame.point(0.5, -9.0, 4.0);
    let tip = tip_nets(&rims, &spec(at, frame.z(), ROUND, 1.0)).unwrap();
    let back: Vec<TipRim> = rims.iter().rev().map(|r| r.reversed().unwrap()).collect();
    let turned = tip_nets(&back, &spec(at, frame.z(), ROUND, 1.0)).unwrap();
    assert!(tip.counter_clockwise && !turned.counter_clockwise);
    let _ = assert_near(
        volume(&turned, frame.origin()),
        volume(&tip, frame.origin()),
        1e-13,
        "volume",
    );
    let _ = assert_near(turned.star, tip.star, 1e-9, "star");
}

// -------------------------------------------------------- determinism

fn words(tip: &Tip) -> Vec<Vec<u64>> {
    let mut all: Vec<Vec<u64>> = tip.faces.iter().map(bits_of).collect();
    all.extend(tip.rails.iter().map(bits_of));
    all.extend(tip.rims.iter().map(bits_of));
    all.push(vec![tip.onward.to_bits(), tip.star.to_bits()]);
    all
}

#[test]
fn the_same_rim_gives_the_same_bits_from_any_curve_on() {
    let frame = frames(1.0)[1];
    let apex = frame.point(0.2, 0.1, 30.0);
    for (sides, form) in [
        (cone(&frame, 1.0, &ellipse(3.0, 1.0), apex), TipForm::Level),
        (
            leaning_loft(&frame, 1.0, &ellipse(3.0, 1.0)),
            TipForm::General,
        ),
    ] {
        let rims = rims_of(&sides);
        let the_spec = spec(frame.point(0.2, 0.1, 2.0), frame.z(), 0.45, 1.0);
        let tip = tip_nets(&rims, &the_spec).unwrap();
        assert_eq!(tip.form, form);
        assert_eq!(words(&tip), words(&tip_nets(&rims, &the_spec).unwrap()));
        // Started one curve on: the same faces, one place on.
        let mut turned = rims.clone();
        turned.rotate_left(1);
        let other = tip_nets(&turned, &the_spec).unwrap();
        for i in 0..4 {
            assert_eq!(bits_of(&other.faces[i]), bits_of(&tip.faces[(i + 1) % 4]));
            assert_eq!(bits_of(&other.rails[i]), bits_of(&tip.rails[(i + 1) % 4]));
        }
        assert_eq!(
            (other.onward.to_bits(), other.star.to_bits()),
            (tip.onward.to_bits(), tip.star.to_bits())
        );
        // A rail is its own face's first column and the last of the
        // face before, bit for bit.
        for i in 0..4 {
            let (here, before) = (&tip.faces[i], &tip.faces[(i + 3) % 4]);
            let (_, rows) = here.counts();
            let (count, _) = before.counts();
            let first = &here.control_points()[..rows];
            let last = &before.control_points()[(count - 1) * rows..];
            assert_eq!(
                bits_of(&first.to_vec()),
                bits_of(&tip.rails[i].control_points().to_vec())
            );
            assert_eq!(bits_of(&last.to_vec()), bits_of(&first.to_vec()));
        }
    }
}

// ------------------------------------------------------ hostile input

#[test]
fn hostile_input_is_refused_with_a_reason() {
    let frame = Frame::WORLD;
    let rims = rims_of(&prism(&frame, 1.0, &hull()));
    let good = spec(frame.point(0.5, -9.0, 4.0), frame.z(), ROUND, 1.0);
    assert!(tip_nets(&rims, &good).is_ok());
    let refused = |rims: &[TipRim], spec: TipSpec| -> String {
        let error = tip_nets(rims, &spec).unwrap_err();
        assert!(!error.to_string().is_empty());
        format!("{error:?}")
    };
    // The numbers of the specification.
    for fullness in [0.0, 1.0, -0.3, 2.0, 1e-4, 0.9999, f64::NAN, f64::INFINITY] {
        assert!(refused(&rims, TipSpec { fullness, ..good }).starts_with("Invalid"));
    }
    for tolerance in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(refused(&rims, TipSpec { tolerance, ..good }).starts_with("Invalid"));
    }
    for angular in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(refused(&rims, TipSpec { angular, ..good }).starts_with("Invalid"));
    }
    for normal in [
        Vec3::ZERO,
        Vec3::new(f64::NAN, 0.0, 1.0),
        Vec3::new(0.0, f64::INFINITY, 0.0),
    ] {
        assert!(refused(&rims, TipSpec { normal, ..good }).starts_with("Invalid"));
    }
    for tip in [
        Vec3::new(f64::NAN, 0.0, 4.0),
        Vec3::new(0.0, 0.0, f64::INFINITY),
    ] {
        assert!(refused(&rims, TipSpec { tip, ..good }).starts_with("Invalid"));
    }
    // A length of nothing, less than nothing, less than the tolerance.
    for height in [0.0, -4.0, 1e-10] {
        let tip = frame.point(0.5, -9.0, height);
        assert!(refused(&rims, TipSpec { tip, ..good }).starts_with("RimReachesPlane"));
    }
    // A normal that points back into the body.
    assert!(
        refused(
            &rims,
            TipSpec {
                normal: -frame.z(),
                ..good
            }
        )
        .starts_with("RimReachesPlane")
    );
    // A tip outside the shoulder.
    let tip = frame.point(30.0, 0.0, 4.0);
    assert!(refused(&rims, TipSpec { tip, ..good }).starts_with("NotStarShaped"));
    // A length beyond all reason is a result or a refusal, not a crash.
    for height in [1e30, 1e200, 1e300, f64::MAX] {
        let tip = frame.point(0.5, -9.0, height);
        let _ = tip_nets(&rims, &TipSpec { tip, ..good });
        let _ = longest_tip_length(
            &rims,
            frame.point(0.5, -9.0, 0.0),
            frame.z(),
            ROUND,
            1e-9,
            height,
        );
    }
    for upto in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            longest_tip_length(
                &rims,
                frame.point(0.5, -9.0, 0.0),
                frame.z(),
                ROUND,
                1e-9,
                upto
            ),
            Err(TipError::Invalid(_))
        ));
    }
    // The rim itself.
    assert!(refused(&[], good).starts_with("Invalid"));
    let mut short = rims.clone();
    short[1].cross.pop();
    assert!(refused(&short, good).starts_with("Invalid"));
    let mut long = rims.clone();
    long[0].cross.push([0.0; 4]);
    assert!(refused(&long, good).starts_with("Invalid"));
    for bad in [f64::NAN, f64::INFINITY] {
        let mut poisoned = rims.clone();
        poisoned[0].cross[1][2] = bad;
        assert!(refused(&poisoned, good).starts_with("Invalid"));
    }
    // One open curve, and curves that do not join end to end.
    assert!(refused(&rims[..1], good).starts_with("RimOpen"));
    let mut apart = rims.clone();
    apart[1] = TipRim::of_border(
        &prism(&frame, 1.0, &hull())
            .iter()
            .map(|s| {
                let moved: Vec<Vec3> = s
                    .control_points()
                    .iter()
                    .map(|p| *p + Vec3::new(1e-6, 0.0, 0.0))
                    .collect();
                NurbsSurface::new(
                    3,
                    1,
                    s.knots_u().to_vec(),
                    s.knots_v().to_vec(),
                    4,
                    2,
                    moved,
                    s.weights().to_vec(),
                )
            })
            .nth(1)
            .unwrap()
            .unwrap(),
        Border::VEnd,
    )
    .unwrap();
    assert!(refused(&apart, good).starts_with("RimOpen { rim: 0"));
    // The same rim curve twice: a loop of no area.
    let twice = vec![rims[0].clone(), rims[0].reversed().unwrap()];
    assert!(tip_nets(&twice, &good).is_err());
    // A rim curve whose knots are not clamped.
    let loose = NurbsCurve::new(
        2,
        vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
        vec![Vec3::X, Vec3::Y, -Vec3::X],
        vec![1.0; 3],
    )
    .unwrap();
    let rim = TipRim {
        cross: vec![[0.0, 0.0, 1.0, 0.0]; 3],
        curve: loose,
    };
    assert!(refused(&[rim], good).starts_with("Unsupported"));
    // Weights that are zero or negative never make a curve to begin with.
    for weight in [0.0, -1.0, f64::NAN] {
        assert!(
            NurbsCurve::new(1, one_piece(1), vec![Vec3::X, Vec3::Y], vec![1.0, weight]).is_err()
        );
    }
    // A cross row with a weight part of its own: in step with the rim's
    // weights it stands for another direction, which its neighbours do
    // not share; out of step it is refused as that.
    let mut sinking = rims.clone();
    for d in &mut sinking[0].cross {
        d[3] = -1.0;
    }
    assert!(refused(&sinking, good).starts_with("SidesDisagree"));
    sinking[0].cross[1][3] = 0.5;
    assert!(refused(&sinking, good).starts_with("Unsupported"));
    for bad in [f64::MAX, -f64::MAX] {
        sinking[0].cross[1][3] = bad;
        let _ = refused(&sinking, good);
    }
    // Sides that go different ways where they meet.
    let mut split = rims.clone();
    for d in &mut split[0].cross {
        d[0] += 0.5 * d[2];
    }
    assert!(refused(&split, good).starts_with("SidesDisagree"));
    // More curves and more control points than a tip is built over.
    let many = vec![rims[0].clone(); MAX_TIP_RIMS + 1];
    assert!(refused(&many, good).starts_with("Invalid"));
    let big = NurbsCurve::new(
        1,
        {
            let mut knots = vec![0.0];
            knots.extend((0..MAX_TIP_POINTS).map(|i| i as f64));
            knots.push((MAX_TIP_POINTS - 1) as f64);
            knots
        },
        (0..MAX_TIP_POINTS)
            .map(|i| Vec3::new(i as f64, 0.0, 0.0))
            .collect(),
        vec![1.0; MAX_TIP_POINTS],
    )
    .unwrap();
    let rim = TipRim {
        cross: vec![[0.0, 0.0, 1.0, 0.0]; MAX_TIP_POINTS],
        curve: big,
    };
    assert!(refused(&[rim.clone(), rim], good).starts_with("Invalid"));
    // A side face that is not clamped across.
    let side = NurbsSurface::new(
        1,
        2,
        one_piece(1),
        vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
        2,
        3,
        (0..6)
            .map(|i| Vec3::new(f64::from(i / 3), f64::from(i % 3), 0.0))
            .collect(),
        vec![1.0; 6],
    )
    .unwrap();
    assert!(matches!(
        TipRim::of_border(&side, Border::VEnd),
        Err(TipError::Unsupported(_))
    ));
    assert!(TipRim::of_border(&side, Border::UEnd).is_err());
}

// ------------------------------------------------- the general net

/// The sides of a loft whose last two sections lean to one another: the
/// row before the rim lies in a plane that is not parallel to the end.
fn leaning_loft(frame: &Frame, k: f64, section: &[Piece]) -> Vec<NurbsSurface> {
    // How far the section reaches from the frame's origin, in its units.
    let reach = section
        .iter()
        .flat_map(|(_, points, _)| points.iter())
        .fold(0.0_f64, |m, p| m.max(p[0].abs()).max(p[1].abs()));
    rim_curves(frame, k, section)
        .iter()
        .map(|rim| {
            side_to(rim, |_, c, w| {
                let (x, y, _) = frame.local(c);
                let back = 2.0 * k + (0.3 * x + 0.15 * y) / reach;
                (frame.point(0.95 * x, 0.95 * y, -back), w)
            })
        })
        .collect()
}

#[test]
fn the_general_net_is_the_level_net_raised_a_degree_where_the_sides_are_level() {
    let mut worst = 0.0_f64;
    for k in SCALES {
        for frame in frames(k) {
            for (name, section, foot, _) in sections() {
                let apex = frame.point(foot[0] * k, foot[1] * k, 40.0 * k);
                for (fullness, sides) in [
                    (ROUND, prism(&frame, k, &section)),
                    (0.3, cone(&frame, k, &section, apex)),
                    (0.8, cone(&frame, k, &section, apex)),
                ] {
                    let at = frame.point(foot[0] * k, foot[1] * k, 3.0 * k);
                    let (normal, omega) = checked(frame.z(), fullness, at).unwrap();
                    let prepared = prepare(&rims_of(&sides), 1e-9 * k).unwrap();
                    let asked = Asked {
                        tip: at,
                        normal,
                        omega,
                        tolerance: 1e-9 * k,
                        angular: 1e-9,
                    };
                    let work = &mut Work(MAX_WORK);
                    let level = solve(&prepared, &asked, None, work).unwrap();
                    let general = solve(&prepared, &asked, Some(TipForm::General), work).unwrap();
                    assert_eq!(
                        (level.form, general.form),
                        (TipForm::Level, TipForm::General)
                    );
                    // Control point for control point: the conic's rows
                    // (a, b, c) raised are a, (a + 2b)/3, (2b + c)/3, c
                    // in homogeneous terms (The NURBS Book, eq. 5.36).
                    let size = 30.0 * k;
                    for (conic, cubic) in level.nets.iter().zip(&general.nets) {
                        for i in 0..conic[0].len() {
                            let h = |j: usize| h_of(conic[j][i].0, conic[j][i].1);
                            let raised = [
                                h(0),
                                h_sum(h(0), 1.0 / 3.0, h(1), 2.0 / 3.0),
                                h_sum(h(1), 2.0 / 3.0, h(2), 1.0 / 3.0),
                                h(2),
                            ];
                            for (j, want) in raised.iter().enumerate() {
                                let (point, weight) = cubic[j][i];
                                let want_weight = h_weight(*want);
                                let want_point = h_xyz(*want) * (1.0 / want_weight);
                                let off = point.distance(want_point) / size;
                                let off = off.max((weight / want_weight - 1.0).abs());
                                assert!(off < 1e-14, "{name} at {k}: row {j} off by {off:e}");
                                worst = worst.max(off);
                            }
                        }
                    }
                    // And so one surface.
                    let level = curves_and_surfaces(&prepared, level, at).unwrap();
                    let general = curves_and_surfaces(&prepared, general, at).unwrap();
                    for (a, b) in level.faces.iter().zip(&general.faces) {
                        for i in 0..=8 {
                            for j in 0..=8 {
                                let (u, v) = (f64::from(i) / 8.0, f64::from(j) / 8.0);
                                assert!(a.eval(u, v).distance(b.eval(u, v)) < 1e-14 * size);
                            }
                        }
                    }
                    let _ = assert_near(general.star, level.star, 1e-6, "star");
                }
            }
        }
    }
    println!("general against the raised conic: at most {worst:e} of the size");
}

/// The general net as the design first gave it, for comparison: its
/// third row from every control point carried along its own direction
/// to the tip's plane.
fn carried_shoulder_faces(sides: &[NurbsSurface], at: Vec3, normal: Vec3, omega: f64) -> Tip {
    let level = normal.dot(at);
    let rims = rims_of(sides);
    let mut least = f64::INFINITY;
    let reach = |c: Vec3, d: Vec3| (level - normal.dot(c)) / normal.dot(d);
    for rim in &rims {
        for ((c, d), w) in rim
            .curve
            .control_points()
            .iter()
            .zip(&rim.cross)
            .zip(rim.curve.weights())
        {
            least = least.min(reach(*c, h_xyz(*d) * (1.0 / w)));
        }
    }
    let toward = 2.0 * omega / (1.0 + 2.0 * omega);
    let mut faces = Vec::new();
    for rim in &rims {
        let mut points = Vec::new();
        let mut weights = Vec::new();
        for ((c, d), w) in rim
            .curve
            .control_points()
            .iter()
            .zip(&rim.cross)
            .zip(rim.curve.weights())
        {
            let d = h_xyz(*d) * (1.0 / w);
            let shoulder = *c + d * reach(*c, d);
            points.extend([
                *c,
                *c + d * (toward * least),
                at + (shoulder - at) * toward,
                at,
            ]);
            weights.extend(
                [
                    1.0,
                    (1.0 + 2.0 * omega) / 3.0,
                    (1.0 + 2.0 * omega) / 3.0,
                    1.0,
                ]
                .map(|x| x * w),
            );
        }
        let count = rim.curve.control_points().len();
        faces.push(
            NurbsSurface::new(
                rim.curve.degree(),
                3,
                rim.curve.knots().to_vec(),
                one_piece(3),
                count,
                4,
                points,
                weights,
            )
            .unwrap(),
        );
    }
    Tip {
        form: TipForm::General,
        rims: rims.iter().map(|r| r.curve.clone()).collect(),
        faces,
        rails: Vec::new(),
        tip: at,
        counter_clockwise: true,
        onward: 0.0,
        star: 0.0,
        smooth: Vec::new(),
        creases: Vec::new(),
        off_sides: Vec::new(),
    }
}

#[test]
fn where_the_sides_are_not_level_the_general_net_keeps_all_three_tangencies() {
    for k in SCALES {
        for frame in frames(k) {
            for (name, section, foot, smooth) in sections() {
                let sides = leaning_loft(&frame, k, &section);
                let rims = rims_of(&sides);
                let at = frame.point(foot[0] * k, foot[1] * k, 3.0 * k);
                let tip = tip_nets(&rims, &spec(at, frame.z(), ROUND, k)).unwrap();
                assert_eq!(tip.form, TipForm::General, "{name}");
                assert_eq!(tip.smooth, smooth, "{name}");
                // Along the rim: exact.
                let along = rim_angle(&sides, &tip);
                assert!(along < 1e-13, "{name} at {k}: {along:e}");
                // At the tip: one plane, reached in proportion.
                let angles = [1e-2, 1e-3, 1e-4].map(|off| tip_angle(&tip, frame.z(), off));
                for pair in angles.windows(2) {
                    let ratio = pair[0] / pair[1];
                    assert!((9.0..11.0).contains(&ratio), "{name} at {k}: {angles:.3?}");
                }
                // No point of the faces beyond the tip's plane.
                for face in &tip.faces {
                    for i in 0..=24 {
                        for j in 0..=24 {
                            let p = face.eval(f64::from(i) / 24.0, f64::from(j) / 24.0);
                            assert!(frame.z().dot(p - at) <= 1e-12 * k);
                        }
                    }
                }
                // Across the rails at smooth vertices: exact, where the
                // row carried point by point to the plane leaves a crease.
                let (_, omega) = checked(frame.z(), ROUND, at).unwrap();
                let carried = carried_shoulder_faces(&sides, at, frame.z(), omega);
                assert!(rim_angle(&sides, &carried) < 1e-13);
                let mut across = 0.0_f64;
                let mut crease = 0.0_f64;
                for (rail, &smooth) in smooth.iter().enumerate() {
                    if smooth {
                        for i in 0..=99 {
                            let v = f64::from(i) / 100.0;
                            across = across.max(rail_angle(&tip, rail, v));
                            crease = crease.max(rail_angle(&carried, rail, v));
                        }
                    }
                }
                assert!(across < 1e-12, "{name} at {k}: {across:e}");
                if smooth.iter().any(|s| *s) {
                    assert!(crease > 1e-6, "{name} at {k}: {crease:e}");
                }
                if k == 1.0 && frame == Frame::WORLD {
                    println!(
                        "general, {name}: rim {along:.1e}, tip {angles:.3?}, smooth rails {across:.1e} (carried point by point: {crease:.2e}), star {:.2e}",
                        tip.star
                    );
                }
            }
        }
    }
}

#[test]
fn a_rod_cut_aslant_takes_a_tip_square_to_its_axis() {
    // A cone-like side with a slanted end: the rim does not lie in a
    // plane parallel to the tip's, the sides go straight up to it.
    for k in SCALES {
        for frame in frames(k) {
            let r = 5.0 * k;
            let sides: Vec<NurbsSurface> = rim_curves(&frame, k, &ellipse(5.0, 5.0))
                .iter()
                .map(|flat| {
                    let slanted: Vec<Vec3> = flat
                        .control_points()
                        .iter()
                        .map(|p| *p + frame.z() * (0.3 * frame.local(*p).0))
                        .collect();
                    let rim =
                        NurbsCurve::new(2, one_piece(2), slanted, flat.weights().to_vec()).unwrap();
                    side_to(&rim, |_, c, w| (c - frame.z() * (2.0 * k), w))
                })
                .collect();
            let at = frame.point(0.0, 0.0, 2.0 * r);
            let tip = tip_nets(&rims_of(&sides), &spec(at, frame.z(), ROUND, k)).unwrap();
            assert_eq!(tip.form, TipForm::General);
            assert_eq!(tip.smooth, [true; 4]);
            assert!(rim_angle(&sides, &tip) < 1e-13);
            let angles = [1e-2, 1e-3, 1e-4].map(|off| tip_angle(&tip, frame.z(), off));
            for pair in angles.windows(2) {
                assert!((9.0..11.0).contains(&(pair[0] / pair[1])), "{angles:.3?}");
            }
            for rail in 0..4 {
                for i in 0..=99 {
                    assert!(rail_angle(&tip, rail, f64::from(i) / 100.0) < 1e-12);
                }
            }
            // Every point stays over the rod's section: the sides go
            // straight on and close in from there.
            for face in &tip.faces {
                for i in 0..=16 {
                    for j in 0..=16 {
                        let (x, y, _) =
                            frame.local(face.eval(f64::from(i) / 16.0, f64::from(j) / 16.0));
                        assert!(x.phypot(y) <= r * (1.0 + 1e-12));
                    }
                }
            }
        }
    }
}

// ------------------------------------------- what the rim must be

/// Straight rim curves from point to point, the side's step at every
/// point given.
fn polygon_rims(points: &[Vec3], steps: &[Vec3]) -> Vec<TipRim> {
    let count = points.len();
    (0..count)
        .map(|i| {
            let j = (i + 1) % count;
            TipRim {
                curve: NurbsCurve::polynomial(1, one_piece(1), vec![points[i], points[j]]).unwrap(),
                cross: [steps[i], steps[j]]
                    .iter()
                    .map(|s| [s.x, s.y, s.z, 0.0])
                    .collect(),
            }
        })
        .collect()
}

fn flat(corners: &[[f64; 2]]) -> Vec<Vec3> {
    corners.iter().map(|c| Vec3::new(c[0], c[1], 0.0)).collect()
}

#[test]
fn a_rim_that_is_not_one_simple_curve_in_a_plane_is_refused() {
    let the_spec = spec(Vec3::new(0.0, 0.0, 4.0), Vec3::Z, ROUND, 1.0);
    let over = |points: &[Vec3]| {
        tip_nets(
            &polygon_rims(points, &vec![Vec3::Z; points.len()]),
            &the_spec,
        )
    };
    // A simple closed curve in space that goes twice round the axis:
    // seen from the tip it crosses itself, and so would the faces (the
    // review found hundreds of crossing triangles in them).
    let twice: Vec<Vec3> = (0..48)
        .map(|i| {
            let t = 4.0 * PI * f64::from(i) / 48.0;
            let r = 5.0 + 1.5 * (0.5 * t).pcos();
            Vec3::new(r * t.pcos(), r * t.psin(), -1.0 + 0.8 * (0.5 * t).psin())
        })
        .collect();
    assert!(matches!(over(&twice), Err(TipError::Unsupported(text)) if text.contains("one plane")));
    // The same pressed flat: in a plane, and crossing itself.
    let pressed: Vec<Vec3> = twice.iter().map(|p| Vec3::new(p.x, p.y, 0.0)).collect();
    assert!(matches!(over(&pressed), Err(TipError::RimCrosses { .. })));
    // An octagon gone round twice lies on itself.
    let octagon: Vec<Vec3> = (0..16)
        .map(|i| {
            let t = PI * f64::from(i) / 4.0;
            Vec3::new(5.0 * t.pcos(), 5.0 * t.psin(), 0.0)
        })
        .collect();
    assert!(matches!(over(&octagon), Err(TipError::RimCrosses { .. })));
    assert!(over(&octagon[..8]).is_ok());
    // A bow that passes through one point twice, and a loop that crosses.
    let touching = flat(&[
        [-2.0, -1.0],
        [0.0, 0.0],
        [2.0, -1.0],
        [2.0, 1.0],
        [0.0, 0.0],
        [-2.0, 1.0],
    ]);
    assert!(matches!(over(&touching), Err(TipError::RimCrosses { .. })));
    let crossing = flat(&[
        [3.0, -3.0],
        [3.0, 3.0],
        [-3.0, 3.0],
        [1.0, -1.0],
        [-1.0, -3.0],
        [0.0, 2.0],
    ]);
    assert!(over(&crossing).is_err());
    // A corner lifted out of the plane: by less than the tolerance it is
    // the plane's, by more the rim is not planar. (A plane that is not
    // square to the normal is one too: `a_rod_cut_aslant_…`.)
    for (lift, planar) in [(1e-10, true), (1e-6, false)] {
        let mut square = flat(&[[3.0, -3.0], [3.0, 3.0], [-3.0, 3.0], [-3.0, -3.0]]);
        square[1].z = lift;
        match over(&square) {
            Ok(tip) => assert!(planar && tip.form == TipForm::Level),
            Err(error) => assert!(
                !planar && matches!(error, TipError::Unsupported(_)),
                "{error}"
            ),
        }
    }
}

#[test]
fn a_rim_curve_that_jumps_is_refused() {
    // A knot as often as the degree plus one inside a curve: its two
    // sides are two curves, here half a unit and then one unit apart.
    let the_spec = spec(Vec3::new(0.0, 0.0, 3.0), Vec3::Z, ROUND, 1.0);
    let up = [0.0, 0.0, 1.0, 0.0];
    let rest = polygon_rims(
        &flat(&[[5.0, 5.0], [-5.0, 5.0], [-5.0, -5.0], [5.0, -5.0]]),
        &[Vec3::Z; 4],
    );
    for gap in [0.0, 0.5] {
        let first = TipRim {
            curve: NurbsCurve::polynomial(
                1,
                vec![0.0, 0.0, 1.0, 1.0, 2.0, 2.0],
                flat(&[[5.0, -5.0], [5.0, -gap], [5.0, gap], [5.0, 5.0]]),
            )
            .unwrap(),
            cross: vec![up; 4],
        };
        let mut rims = vec![first];
        rims.extend(rest.iter().take(3).cloned());
        assert!(
            matches!(tip_nets(&rims, &the_spec), Err(TipError::Invalid(text)) if text.contains("more often than its degree"))
        );
    }
    // The same edge with its knot once is a curve with a corner in it.
    let first = TipRim {
        curve: NurbsCurve::polynomial(
            1,
            vec![0.0, 0.0, 1.0, 2.0, 2.0],
            flat(&[[5.0, -5.0], [5.0, 0.0], [5.0, 5.0]]),
        )
        .unwrap(),
        cross: vec![up; 3],
    };
    let mut rims = vec![first];
    rims.extend(rest.iter().take(3).cloned());
    assert!(tip_nets(&rims, &the_spec).is_ok());
}

// --------------------------------------------------- the floor

#[test]
fn a_right_tip_is_accepted_however_thin_full_or_weighted() {
    let frame = Frame::WORLD;
    // A blade ten thousand times longer than thick, the tip over its
    // middle: convex, so every such tip is right, at any fullness and
    // length.
    for t in [1e-2, 1e-4] {
        let section = polygon(&[
            [-0.5, -0.5 * t],
            [0.5, -0.5 * t],
            [0.5, 0.5 * t],
            [-0.5, 0.5 * t],
        ]);
        let rims = rims_of(&prism(&frame, 1.0, &section));
        for length in [1e-4, 1e-3, 1e-2, 1.0, 10.0] {
            for fullness in [1e-3, 0.05, ROUND, 0.95, 0.999] {
                let the_spec = TipSpec {
                    tip: frame.point(0.0, 0.0, length),
                    normal: frame.z(),
                    fullness,
                    tolerance: 1e-12,
                    angular: 1e-9,
                };
                let tip = tip_nets(&rims, &the_spec);
                assert!(
                    tip.is_ok(),
                    "{t:e} thick, {length:e} long, fullness {fullness}: {tip:?}"
                );
            }
        }
    }
    // Nor do the weights' size or the model's count: a square with one
    // edge written as a rational quadratic with a heavy middle point, and
    // every weight of another edge a million times the rest.
    for k in SCALES {
        let written = |middle: f64| {
            let mut section = square();
            section[0] = (
                2,
                vec![[1.0, -1.0], [1.0, 0.0], [1.0, 1.0]],
                vec![1.0, middle, 1.0],
            );
            section[2].2 = vec![1e6, 1e6];
            over(&frame, k, &section, [0.3, -0.2])
        };
        let plain = over(&frame, k, &square(), [0.3, -0.2]).unwrap();
        let heavy = written(1e4).unwrap();
        // (The volume by quadrature needs a milder parametrisation.)
        let mild = written(4.0).unwrap();
        let _ = assert_near(
            volume(&mild, frame.origin()),
            volume(&plain, frame.origin()),
            1e-9,
            "volume",
        );
        // The bound on the shoulder's sine is of the shape alone: the
        // same at every fullness, scale and weight (within what the
        // heavier hulls give away).
        for fullness in [1e-3, 0.5, 0.999] {
            let rims = rims_of(&prism(&frame, k, &square()));
            let at = frame.point(0.3 * k, -0.2 * k, k);
            let other = tip_nets(&rims, &spec(at, frame.z(), fullness, k)).unwrap();
            let _ = assert_near(other.star, plain.star, 1e-9, "the sine's bound");
        }
        assert!(plain.star > 0.3 && plain.star < 1.0, "{}", plain.star);
        assert!(heavy.star > 0.0);
    }
}

#[test]
fn what_rounding_could_decide_either_way_is_told_as_not_proven() {
    for k in SCALES {
        for frame in frames(k) {
            // The tip exactly over an edge of the square: the shoulder
            // passes through it.
            assert!(matches!(
                over(&frame, k, &square(), [1.0, 0.3]),
                Err(TipError::NotProven { .. })
            ));
            // A hair inside it is right and a hair outside wrong, and
            // each is told as what it is.
            assert!(over(&frame, k, &square(), [1.0 - 1e-9, 0.3]).is_ok());
            assert!(matches!(
                over(&frame, k, &square(), [1.0 + 1e-9, 0.3]),
                Err(TipError::NotStarShaped { rim: 0, .. })
            ));
            // A cone carried exactly to its apex: everything in one point.
            let apex = frame.point(0.0, 0.0, 10.0 * k);
            let rims = rims_of(&cone(&frame, k, &ellipse(5.0, 5.0), apex));
            assert!(matches!(
                tip_nets(&rims, &spec(apex, frame.z(), ROUND, k)),
                Err(TipError::NotProven { .. })
            ));
        }
    }
    // A rim curve that stands still at one end (a doubled control
    // point) is the same shape, and its faces' determinant is nothing
    // there: not proven, and said so.
    let mut section = oval();
    section[0].1[1] = section[0].1[0];
    assert!(matches!(
        over(&Frame::WORLD, 1.0, &section, [0.3, 0.1]),
        Err(TipError::NotProven { rim: 0, .. })
    ));
}

// ------------------------------------------ lengths in two stretches

#[test]
fn the_longest_length_is_the_end_of_the_first_stretch_and_says_when_more_fit() {
    // A square with a vertex in the middle of its lower edge that is
    // carried far to the right, the corner beside it less: seen from the
    // tip the first overtakes the second and is caught up again. The
    // shoulder is star-shaped below (3 − √5)/2 and above (3 + √5)/2; with
    // the upper corners leaning in, up to 5.
    let points = flat(&[
        [-1.0, -1.0],
        [0.0, -1.0],
        [1.0, -1.0],
        [1.0, 1.0],
        [-1.0, 1.0],
    ]);
    let steps = [
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(5.0, -1.0, 1.0),
        Vec3::new(1.0, 0.0, 1.0),
        Vec3::new(0.0, -0.2, 1.0),
        Vec3::new(0.0, -0.2, 1.0),
    ];
    let rims = polygon_rims(&points, &steps);
    let first = 0.5 * (3.0 - 5.0_f64.sqrt());
    let second = 0.5 * (3.0 + 5.0_f64.sqrt());
    let fits = |length: f64| {
        tip_nets(
            &rims,
            &spec(Vec3::new(0.0, 0.0, length), Vec3::Z, ROUND, 1.0),
        )
        .is_ok()
    };
    assert!(fits(0.38) && !fits(0.39) && !fits(2.6) && fits(2.63) && fits(4.9) && !fits(5.1));
    for (upto, again) in [
        (4.0, Some(4.0)),
        (6.0, Some(3.0)),
        (8.0, Some(4.0)),
        (100.0, Some(3.125)),
    ] {
        let found = longest_tip_length(&rims, Vec3::ZERO, Vec3::Z, ROUND, 1e-9, upto).unwrap();
        let _ = assert_near(found.fits, first, 1e-8, "the first stretch's end");
        assert!(
            found.fits < first
                && found
                    .fails
                    .is_some_and(|l| l - found.fits <= LENGTH_RESOLUTION * l)
        );
        assert_eq!(found.again, again, "upto {upto}");
        assert!(found.again.is_some_and(|l| l > second && fits(l)));
    }
    // A stretch narrower than a factor of two can fall between the
    // lengths tried: nothing is said of it then, and what is said holds.
    let found = longest_tip_length(&rims, Vec3::ZERO, Vec3::Z, ROUND, 1e-9, 20.0).unwrap();
    let _ = assert_near(found.fits, first, 1e-8, "the first stretch's end");
    assert_eq!(found.again, None);
    // Within the first stretch the length asked for is the answer.
    let found = longest_tip_length(&rims, Vec3::ZERO, Vec3::Z, ROUND, 1e-9, 0.3).unwrap();
    assert_eq!(
        found,
        TipLength {
            fits: 0.3,
            fails: None,
            again: None
        }
    );
}

// --------------------------------------- how a side is written

#[test]
fn the_tip_does_not_depend_on_how_a_sides_weights_are_written() {
    for k in SCALES {
        for frame in frames(k) {
            let at = frame.point(0.3 * k, 0.2 * k, 3.0 * k);
            let rims = rim_curves(&frame, k, &ellipse(5.0, 5.0));
            // The rod's sides with the row before the rim weighted by
            // another number on every quarter, and one quarter's row
            // twice as far back: the same surfaces, the same tip.
            let plain: Vec<NurbsSurface> = rims
                .iter()
                .map(|rim| side_to(rim, |_, c, w| (c - frame.z() * (2.0 * k), w)))
                .collect();
            let written: Vec<NurbsSurface> = rims
                .iter()
                .enumerate()
                .map(|(q, rim)| {
                    let (weight, back) = [(1.0, 2.0), (0.5, 2.0), (3.0, 4.0), (0.25, 0.5)][q];
                    side_to(rim, |_, c, w| (c - frame.z() * (back * k), w * weight))
                })
                .collect();
            let want = tip_nets(&rims_of(&plain), &spec(at, frame.z(), 0.6, k)).unwrap();
            let got = tip_nets(&rims_of(&written), &spec(at, frame.z(), 0.6, k)).unwrap();
            assert_eq!(got.form, TipForm::Level);
            for (a, b) in want.faces.iter().zip(&got.faces) {
                for (p, q) in a.control_points().iter().zip(b.control_points()) {
                    assert!(p.distance(*q) < 1e-12 * 10.0 * k, "{}", p.distance(*q));
                }
                assert_eq!(a.weights(), b.weights());
            }
            assert_eq!(got.smooth, [true; 4]);
        }
    }
    // Where the weights across the rim are out of step with the rim's,
    // the side is one writing among others with other directions: a
    // plane side of a square prism whose row before the rim is weighted
    // 1, 2, 1, and a rod's quarter with that row polynomial.
    let frame = Frame::WORLD;
    let edge = (2, vec![[1.0, -1.0], [1.0, 0.0], [1.0, 1.0]], vec![1.0; 3]);
    let mut section = square();
    section[0] = edge;
    let mut sides = prism(&frame, 1.0, &section);
    let rim = rim_curves(&frame, 1.0, &section).remove(0);
    sides[0] = side_to(&rim, |i, c, w| {
        (c - frame.z() * 2.0, if i == 1 { 2.0 * w } else { w })
    });
    let at = frame.point(0.0, 0.0, 1.0);
    assert!(matches!(
        tip_nets(&rims_of(&sides), &spec(at, frame.z(), ROUND, 1.0)),
        Err(TipError::Unsupported(text)) if text.contains("out of step")
    ));
    let sides: Vec<NurbsSurface> = rim_curves(&frame, 1.0, &ellipse(5.0, 5.0))
        .iter()
        .map(|rim| side_to(rim, |_, c, _| (c - frame.z() * 2.0, 1.0)))
        .collect();
    assert!(matches!(
        tip_nets(&rims_of(&sides), &spec(frame.point(0.0, 0.0, 3.0), frame.z(), ROUND, 1.0)),
        Err(TipError::Unsupported(text)) if text.contains("out of step")
    ));
}

// ------------------------------- sides that agree within the tolerance

#[test]
fn sides_that_agree_within_the_tolerance_are_built_on_one_row() {
    let frame = Frame::WORLD;
    let tolerance = 1e-7;
    let section = polygon(&[[5.0, -5.0], [5.0, 5.0], [-5.0, 5.0], [-5.0, -5.0]]);
    let curves = rim_curves(&frame, 1.0, &section);
    // One side's row before the rim moved sideways at the shared edge,
    // as two sewn or imported faces differ; then the sides lean, so that
    // the net is the general one.
    for lean in [0.0, 0.2] {
        let sides = |off: f64| -> Vec<NurbsSurface> {
            curves
                .iter()
                .enumerate()
                .map(|(q, rim)| {
                    side_to(rim, |i, c, w| {
                        let (x, _, _) = frame.local(c);
                        let mut back = c - frame.z() * (2.0 + lean * x / 5.0);
                        if q == 1 && i == 0 {
                            back.x += off;
                        }
                        (back, w)
                    })
                })
                .collect()
        };
        for length in [1.0, 10.0, 300.0] {
            let the_spec = TipSpec {
                tip: frame.point(0.3, 0.2, length),
                normal: frame.z(),
                fullness: ROUND,
                tolerance,
                angular: 1e-9,
            };
            let exact = tip_nets(&rims_of(&sides(0.0)), &the_spec).unwrap();
            let near = tip_nets(&rims_of(&sides(0.5 * tolerance)), &the_spec).unwrap();
            assert_eq!(near.form, exact.form);
            assert_eq!(
                near.form,
                if lean == 0.0 {
                    TipForm::Level
                } else {
                    TipForm::General
                }
            );
            // The earlier face leaves its own side by the angle between
            // the two steps about the rim's direction, and says so: half
            // a tolerance over rows two apart.
            assert_eq!(exact.off_sides, [0.0; 4]);
            let off = near.off_sides[0];
            assert!((off / (0.25 * tolerance) - 1.0).abs() < 0.2, "{off:e}");
            assert_eq!(near.off_sides[1..], [0.0; 3]);
            // Built on the later side's row: the rail is that side's own.
            let (_, rows) = near.faces[1].counts();
            let (count, _) = near.faces[0].counts();
            let column = &near.faces[1].control_points()[..rows];
            assert_eq!(
                bits_of(&column.to_vec()),
                bits_of(&near.rails[1].control_points().to_vec())
            );
            let last = &near.faces[0].control_points()[(count - 1) * rows..];
            assert_eq!(bits_of(&last[1..].to_vec()), bits_of(&column[1..].to_vec()));
            let _ = assert_near(
                volume(&near, frame.origin()),
                volume(&exact, frame.origin()),
                1e-6,
                "volume",
            );
            // Three tolerances off is another direction.
            assert!(matches!(
                tip_nets(&rims_of(&sides(3.0 * tolerance)), &the_spec),
                Err(TipError::SidesDisagree { rim: 1, gap }) if gap > 2.9 * tolerance && gap < 3.1 * tolerance
            ));
        }
    }
}

// --------------------------------------------------- creases

#[test]
fn every_rail_tells_the_angle_between_its_faces() {
    for k in SCALES {
        for frame in frames(k) {
            for (name, section, foot, smooth) in sections() {
                for sides in [
                    prism(&frame, k, &section),
                    leaning_loft(&frame, k, &section),
                ] {
                    let at = frame.point(foot[0] * k, foot[1] * k, 3.0 * k);
                    let tip = tip_nets(&rims_of(&sides), &spec(at, frame.z(), ROUND, k)).unwrap();
                    assert_eq!(tip.creases.len(), tip.rails.len());
                    for (rail, crease) in tip.creases.iter().enumerate() {
                        // What the faces' own evaluation gives.
                        let at_rim = rail_angle(&tip, rail, 0.0);
                        let largest = (0..64)
                            .map(|i| rail_angle(&tip, rail, f64::from(i) / 64.0))
                            .fold(0.0, f64::max);
                        assert!(
                            (crease.at_rim - at_rim).abs() < 1e-11,
                            "{name}: {crease:?} {at_rim}"
                        );
                        assert!(
                            (crease.largest - largest).abs() < 1e-11,
                            "{name}: {crease:?} {largest}"
                        );
                        if smooth[rail] {
                            assert!(
                                tip.smooth[rail] && crease.largest < 1e-12,
                                "{name}: {crease:?}"
                            );
                        } else {
                            assert!(!tip.smooth[rail] && crease.at_rim > 0.3);
                            assert!(crease.largest >= crease.at_rim, "{name}");
                        }
                    }
                }
            }
        }
    }
    // A square's corners are square.
    let tip = over(&Frame::WORLD, 1.0, &square(), [0.2, -0.1]).unwrap();
    for crease in &tip.creases {
        assert!((crease.at_rim - 0.5 * PI).abs() < 1e-14);
    }
}

#[test]
fn a_rail_is_called_smooth_only_where_its_crease_is_below_the_angular_tolerance() {
    // An oval whose two arcs meet at a small angle: the second arc's
    // first leg turned by `turn`.
    let frame = Frame::WORLD;
    for (angular, turn, smooth) in [
        (1e-6, 1e-7, true),
        (1e-6, 4e-7, false),
        (1e-6, 1e-5, false),
        (1e-9, 1e-7, false),
    ] {
        let mut section = oval();
        section[1].1[1] = [-6.0 + turn, -1.0];
        let rims = rims_of(&prism(&frame, 1.0, &section));
        let the_spec = TipSpec {
            angular,
            ..spec(frame.point(0.3, 0.1, 3.0), frame.z(), ROUND, 1.0)
        };
        let tip = tip_nets(&rims, &the_spec).unwrap();
        assert_eq!(
            tip.smooth[1], smooth,
            "{turn:e} against {angular:e}: {:?}",
            tip.creases[1]
        );
        assert!((tip.creases[1].at_rim - turn).abs() < 1e-3 * turn);
        if smooth {
            assert!(tip.creases[1].largest < angular);
        }
    }
}

// --------------------------------------------------- far and large

#[test]
fn a_tip_far_from_the_origin_is_the_same_tip() {
    // A cone a million sizes away, with a tolerance its coordinates can
    // hold: level as it is at the origin, and the same faces moved.
    let frame = frames(1.0)[1];
    let away = Vec3::new(1.0e7, -2.0e7, 1.5e7);
    let far = frame.at(frame.origin() + away).unwrap();
    let tips: Vec<Tip> = [frame, far]
        .iter()
        .map(|f| {
            let apex = f.point(0.0, 0.0, 12.0);
            let rims = rims_of(&cone(f, 1.0, &ellipse(5.0, 3.0), apex));
            let the_spec = TipSpec {
                tolerance: 1e-6,
                angular: 1e-6,
                ..spec(f.point(0.2, 0.1, 3.0), f.z(), ROUND, 1.0)
            };
            tip_nets(&rims, &the_spec).unwrap()
        })
        .collect();
    assert_eq!(
        (tips[0].form, tips[1].form),
        (TipForm::Level, TipForm::Level)
    );
    for (near, moved) in tips[0].faces.iter().zip(&tips[1].faces) {
        for (p, q) in near.control_points().iter().zip(moved.control_points()) {
            assert!((*p + away).distance(*q) < 1e-7);
        }
    }
    assert_eq!(tips[0].smooth, tips[1].smooth);
}

#[test]
fn the_work_of_a_call_is_bounded() {
    // A rim of the most control points allowed, of high degree, and a
    // tip that is nowhere near right: the answer comes, whatever it is.
    let degree = 16;
    let spans = (MAX_TIP_POINTS / 2 - 1) / degree;
    let half = |turned: f64| {
        let count = spans * degree + 1;
        let mut knots = vec![0.0; degree + 1];
        for s in 1..spans {
            knots.extend(std::iter::repeat_n(s as f64, degree));
        }
        knots.extend(std::iter::repeat_n(spans as f64, degree + 1));
        let points: Vec<Vec3> = (0..count)
            .map(|i| {
                let t = turned + PI * i as f64 / (count - 1) as f64;
                let r = 5.0 + 0.3 * (40.0 * t).psin();
                Vec3::new(r * t.pcos(), r * t.psin(), 0.0)
            })
            .collect();
        TipRim {
            cross: vec![[0.0, 0.0, 1.0, 0.0]; count],
            curve: NurbsCurve::polynomial(degree, knots, points).unwrap(),
        }
    };
    let rims = vec![half(0.0), half(PI)];
    let started = std::time::Instant::now();
    let tip = tip_nets(&rims, &spec(Vec3::new(0.2, 0.1, 3.0), Vec3::Z, ROUND, 1.0));
    let length = longest_tip_length(&rims, Vec3::new(4.9, 0.0, 0.0), Vec3::Z, ROUND, 1e-9, 50.0);
    println!(
        "{} control points of degree {degree}: {} and {} in {:.1?}",
        2 * (spans * degree + 1),
        tip.map(|t| format!("{:?}", t.form))
            .unwrap_or_else(|e| format!("{e:?}")),
        length
            .map(|l| format!("{l:?}"))
            .unwrap_or_else(|e| format!("{e:?}")),
        started.elapsed()
    );
}

// ------------------------------------------ however the rim is cut

fn side_of(rim: &TipRim) -> Side {
    Side {
        degree: rim.curve.degree(),
        knots: rim.curve.knots().to_vec(),
        points: rim.curve.control_points().to_vec(),
        weights: rim.curve.weights().to_vec(),
        cross: rim.cross.clone(),
    }
}

fn rim_of(side: &Side) -> TipRim {
    TipRim {
        curve: NurbsCurve::new(
            side.degree,
            side.knots.clone(),
            side.points.clone(),
            side.weights.clone(),
        )
        .unwrap(),
        cross: side.cross.clone(),
    }
}

/// Every rim curve cut in two at the middle of its range: the same
/// curves and the same sides, twice as many.
fn cut(rims: &[TipRim]) -> Vec<TipRim> {
    rims.iter()
        .flat_map(|rim| cut_in_two(&side_of(rim)).unwrap())
        .map(|side| rim_of(&side))
        .collect()
}

/// Every rim curve with knots put in at three places: the same curves
/// with more control points.
fn refined(rims: &[TipRim]) -> Vec<TipRim> {
    rims.iter()
        .map(|rim| {
            let side = side_of(rim);
            let (a, b) = rim.curve.domain();
            let mut knots = side.knots.clone();
            let mut rows = vec![side.homogeneous().collect::<Vec<H>>(), side.cross.clone()];
            for share in [0.3, 0.55, 0.8] {
                insert_knot(&mut knots, side.degree, &mut rows, a + share * (b - a));
            }
            rim_of(&Side {
                degree: side.degree,
                knots,
                points: rows[0]
                    .iter()
                    .map(|&h| h_xyz(h) * (1.0 / h_weight(h)))
                    .collect(),
                weights: rows[0].iter().map(|&h| h_weight(h)).collect(),
                cross: rows[1].clone(),
            })
        })
        .collect()
}

/// The rim curves joined two and two into one curve each (a knot as
/// often as the degree where they met): the same curves, half as many.
fn joined(rims: &[TipRim]) -> Vec<TipRim> {
    rims.chunks(2)
        .map(|pair| {
            let (a, b) = (side_of(&pair[0]), side_of(&pair[1]));
            let degree = a.degree;
            let mut knots = vec![0.0; degree + 1];
            knots.extend(std::iter::repeat_n(1.0, degree));
            knots.extend(std::iter::repeat_n(2.0, degree + 1));
            rim_of(&Side {
                degree,
                knots,
                points: a.points.iter().chain(&b.points[1..]).copied().collect(),
                weights: a.weights.iter().chain(&b.weights[1..]).copied().collect(),
                cross: a.cross.iter().chain(&b.cross[1..]).copied().collect(),
            })
        })
        .collect()
}

/// How far the faces of `fine` lie from those of `coarse` at the same
/// places: `place` gives, for a face of `fine` and a parameter along it,
/// the face of `coarse` and its parameter.
fn apart(fine: &Tip, coarse: &Tip, place: impl Fn(usize, f64) -> (usize, f64)) -> f64 {
    let mut worst = 0.0_f64;
    for (index, face) in fine.faces.iter().enumerate() {
        let ((u0, u1), _) = face.domain();
        for i in 0..=12 {
            let u = u0 + (u1 - u0) * f64::from(i) / 12.0;
            let (other, at) = place(index, u);
            for j in 0..=12 {
                let v = f64::from(j) / 12.0;
                worst = worst.max(face.eval(u, v).distance(coarse.faces[other].eval(at, v)));
            }
        }
    }
    worst
}

#[test]
fn the_same_rim_cut_another_way_gives_the_same_tip() {
    let mut worst = 0.0_f64;
    for k in SCALES {
        for frame in frames(k) {
            for (name, section, foot, _) in sections() {
                // Sides that are not level, and sides that are.
                for (sides, form) in [
                    (leaning_loft(&frame, k, &section), TipForm::General),
                    (prism(&frame, k, &section), TipForm::Level),
                ] {
                    let rims = rims_of(&sides);
                    let the_spec = spec(
                        frame.point(foot[0] * k, foot[1] * k, 3.0 * k),
                        frame.z(),
                        0.6,
                        k,
                    );
                    let whole = tip_nets(&rims, &the_spec).unwrap();
                    assert_eq!(whole.form, form);
                    let size = if name == "hull" { 30.0 * k } else { 12.0 * k };
                    let mut hold = |what: &str, off: f64| {
                        assert!(off < 1e-11 * size, "{name} {form:?} {what} at {k}: {off:e}");
                        worst = worst.max(off / size);
                    };
                    // Cut in two, and in four.
                    let halves = tip_nets(&cut(&rims), &the_spec).unwrap();
                    hold("cut in two", apart(&halves, &whole, |f, u| (f / 2, u)));
                    let quarters = tip_nets(&cut(&cut(&rims)), &the_spec).unwrap();
                    hold("cut in four", apart(&quarters, &whole, |f, u| (f / 4, u)));
                    // Knots put in.
                    let fine = tip_nets(&refined(&rims), &the_spec).unwrap();
                    hold("refined", apart(&fine, &whole, |f, u| (f, u)));
                    // Joined two and two (where there are four curves; two
                    // joined would be one closed curve, cut where it was).
                    if rims.len() == 4 {
                        let fewer = tip_nets(&joined(&rims), &the_spec).unwrap();
                        assert_eq!(fewer.faces.len(), 2);
                        hold(
                            "joined",
                            apart(&whole, &fewer, |f, u| (f / 2, u + (f % 2) as f64)),
                        );
                    }
                }
            }
        }
    }
    println!("the same rim cut another way: tips at most {worst:.1e} of the size apart");
}

#[test]
fn the_least_quotient_is_of_the_functions_not_of_their_spans() {
    // a(t) = 1 + (t − 0.3)², b(t) = 2 − t on [0, 1], as Bézier
    // coefficients: the least of a / b is at t = 2 − √3.89.
    let a = [1.09, 0.79, 1.49];
    let b = [2.0, 1.5, 1.0];
    let pairs = |a: &[f64], b: &[f64]| -> Vec<(f64, f64)> {
        a.iter().copied().zip(b.iter().copied()).collect()
    };
    let at = 2.0 - 3.89_f64.sqrt();
    let truth = (1.0 + (at - 0.3) * (at - 0.3)) / (2.0 - at);
    let whole = least_quotient(vec![pairs(&a, &b)]).unwrap();
    assert!(
        (whole - truth).abs() < 2.0 * FACTOR_ACCURACY * truth,
        "{whole} {truth}"
    );
    // The least over the coefficients is far below it.
    assert!(0.79 / 1.5 < 0.99 * truth);
    // The same functions cut at 0.5 (de Casteljau by hand).
    let halves = |c: [f64; 3]| {
        let (m0, m1) = (0.5 * (c[0] + c[1]), 0.5 * (c[1] + c[2]));
        let m = 0.5 * (m0 + m1);
        ([c[0], m0, m], [m, m1, c[2]])
    };
    let ((a0, a1), (b0, b1)) = (halves(a), halves(b));
    let cut = least_quotient(vec![pairs(&a0, &b0), pairs(&a1, &b1)]).unwrap();
    assert!((cut - whole).abs() < 2.0 * FACTOR_ACCURACY * truth);
    // A constant quotient is itself.
    let level = least_quotient(vec![pairs(&[2.0, 4.0, 6.0], &[1.0, 2.0, 3.0])]).unwrap();
    assert_eq!(level, 2.0);
    assert_eq!(least_quotient(vec![pairs(&[1.0, 2.0], &[1.0, 0.0])]), None);
    assert_eq!(
        least_quotient(vec![pairs(&[1.0, f64::NAN], &[1.0, 1.0])]),
        None
    );
}
