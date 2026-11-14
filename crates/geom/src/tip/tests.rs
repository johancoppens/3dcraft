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
                // A tip is refused a margin's worth before the limit, where
                // the faces are seen from it all but edge-on.
                let fails = found.fails.unwrap();
                assert!(
                    found.fits < fails && found.fits < limit * k,
                    "{what}: {found:?}"
                );
                assert!(fails - found.fits <= LENGTH_RESOLUTION * fails);
                assert!(fails <= limit * k * (1.0 + 1e-12), "{what}: {found:?}");
                let short = 1.0 - found.fits / (limit * k);
                assert!(short < 2e-8, "{what}: {short:e} short of the limit");
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
                    fails: None
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
    // A cross row whose weights take the side's to zero on the way.
    let mut sinking = rims.clone();
    for d in &mut sinking[0].cross {
        d[3] = -1.0;
    }
    let error = refused(&sinking, good);
    assert!(
        error.starts_with("Unsupported")
            || error.starts_with("WeightsRunOut")
            || error.starts_with("NotOnward"),
        "{error}"
    );
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
                    let level = solve(&prepared, at, normal, omega, 1e-9 * k, 1e-9, None).unwrap();
                    let general = solve(
                        &prepared,
                        at,
                        normal,
                        omega,
                        1e-9 * k,
                        1e-9,
                        Some(TipForm::General),
                    )
                    .unwrap();
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
                // Nothing beyond the tip's plane.
                for face in &tip.faces {
                    for p in face.control_points() {
                        assert!(frame.z().dot(*p - at) <= 1e-12 * k);
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

#[test]
fn sides_whose_rows_are_out_of_step_get_rails_that_are_edges_and_are_told_so() {
    // A circle's quarters with the row before them polynomial: across
    // the rim the weights change at the middle control point of each
    // quarter and not at its ends. The two sides of a rail are tangent
    // along their common edge (it is straight), but their rows do not
    // leave it at one ratio: no rows in the rim's basis are tangent to
    // both sides and to one another. The tip still runs on from each
    // side and has its one plane at the point; the rails are edges, and
    // `smooth` says so.
    for k in SCALES {
        for frame in frames(k) {
            let sides: Vec<NurbsSurface> = rim_curves(&frame, k, &ellipse(5.0, 5.0))
                .iter()
                .map(|rim| side_to(rim, |_, c, _| (c - frame.z() * (2.0 * k), 1.0)))
                .collect();
            let at = frame.point(0.0, 0.0, 3.0 * k);
            let tip = tip_nets(&rims_of(&sides), &spec(at, frame.z(), ROUND, k)).unwrap();
            assert_eq!(tip.form, TipForm::General);
            assert_eq!(tip.smooth, [false; 4]);
            assert!(rim_angle(&sides, &tip) < 1e-13);
            let angles = [1e-2, 1e-3, 1e-4].map(|off| tip_angle(&tip, frame.z(), off));
            for pair in angles.windows(2) {
                assert!((9.0..11.0).contains(&(pair[0] / pair[1])), "{angles:?}");
            }
            let mut across = 0.0_f64;
            for rail in 0..4 {
                assert!(rail_angle(&tip, rail, 0.0) < 1e-13);
                for i in 0..=99 {
                    across = across.max(rail_angle(&tip, rail, f64::from(i) / 100.0));
                }
            }
            assert!((0.1..0.2).contains(&across), "{across:e}");
            if k == 1.0 && frame == Frame::WORLD {
                println!("rows out of step: rails creased by up to {across:.3e} rad");
            }
        }
    }
}
