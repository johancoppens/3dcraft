//! Shared by the profile and extrusion tests: curve builders, a seeded
//! generator and the catalogue of profiles with their closed forms.

// The catalogue is a list of commented cases, pushed one by one.
#![allow(dead_code, clippy::vec_init_then_push)]

use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2};
use craft3d_math::{Frame, Vec3};
use std::f64::consts::PI;

pub type P = [f64; 2];

pub fn line(id: u64, start: P, end: P) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

/// Counter-clockwise from `start` to `end` about `center`.
pub fn arc(id: u64, center: P, start: P, end: P) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Arc { center, start, end },
    }
}

pub fn circle(id: u64, center: P, radius: f64) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Circle { center, radius },
    }
}

/// The closed polygon through `points`, with ids `first`, `first + step`, …
pub fn polygon(first: u64, step: u64, points: &[P]) -> Vec<SketchCurve> {
    (0..points.len())
        .map(|i| {
            line(
                first + step * i as u64,
                points[i],
                points[(i + 1) % points.len()],
            )
        })
        .collect()
}

/// The rectangle `x0..x1` by `y0..y1`, counter-clockwise from its lower
/// left corner.
pub fn rectangle(first: u64, step: u64, x: (f64, f64), y: (f64, f64)) -> Vec<SketchCurve> {
    polygon(
        first,
        step,
        &[[x.0, y.0], [x.1, y.0], [x.1, y.1], [x.0, y.1]],
    )
}

pub fn scale_point(p: P, k: f64) -> P {
    [p[0] * k, p[1] * k]
}

pub fn scale_curves(curves: &[SketchCurve], k: f64) -> Vec<SketchCurve> {
    curves
        .iter()
        .map(|c| SketchCurve {
            id: c.id,
            curve: match &c.curve {
                &Curve2::Line { start, end } => Curve2::Line {
                    start: scale_point(start, k),
                    end: scale_point(end, k),
                },
                &Curve2::Arc { center, start, end } => Curve2::Arc {
                    center: scale_point(center, k),
                    start: scale_point(start, k),
                    end: scale_point(end, k),
                },
                &Curve2::Circle { center, radius } => Curve2::Circle {
                    center: scale_point(center, k),
                    radius: radius * k,
                },
                Curve2::Spline(spline) => Curve2::Spline(Spline2 {
                    degree: spline.degree,
                    knots: spline.knots.clone(),
                    control: spline.control.iter().map(|p| scale_point(*p, k)).collect(),
                }),
            },
        })
        .collect()
}

/// Every line drawn the other way. (An arc is counter-clockwise by
/// definition and a circle has no direction: neither can be drawn the other
/// way and still be the same curve.)
pub fn lines_reversed(curves: &[SketchCurve]) -> Vec<SketchCurve> {
    curves
        .iter()
        .map(|c| SketchCurve {
            id: c.id,
            curve: match &c.curve {
                &Curve2::Line { start, end } => Curve2::Line {
                    start: end,
                    end: start,
                },
                other => other.clone(),
            },
        })
        .collect()
}

/// SplitMix64: small, seeded, the same on every platform.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i as u64 + 1) as usize);
        }
    }
}

/// The frames the catalogue is extruded on: the world, and a tilted plane
/// away from the origin.
pub fn frames(k: f64) -> Vec<Frame> {
    vec![
        Frame::WORLD,
        Frame::new(
            Vec3::new(3.0 * k, -2.0 * k, 5.0 * k),
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(1.0, 0.0, 0.2),
        )
        .unwrap(),
    ]
}

/// One region of a case, with its closed forms.
pub struct Part {
    pub area: f64,
    pub centroid: P,
    pub holes: usize,
}

pub struct Case {
    pub name: &'static str,
    pub curves: Vec<SketchCurve>,
    /// The profiles expected, in order.
    pub parts: Vec<Part>,
    /// The total length of all curves.
    pub perimeter: f64,
    /// Faces, edges and vertices of the extrusion, counted by hand.
    pub counts: (usize, usize, usize),
}

fn part(area: f64, centroid: P, holes: usize) -> Part {
    Part {
        area,
        centroid,
        holes,
    }
}

/// The catalogue at unit scale. Every expected value is a closed form
/// worked out in the comment above it.
fn unit_catalogue() -> Vec<Case> {
    let mut cases = Vec::new();

    // Rectangle 4 × 3 with its lower left corner at (1, 2).
    // Area 12, perimeter 14, centre (3, 3.5). A box: 6 faces, 12 edges,
    // 8 vertices.
    cases.push(Case {
        name: "rectangle",
        curves: rectangle(10, 10, (1.0, 5.0), (2.0, 5.0)),
        parts: vec![part(12.0, [3.0, 3.5], 0)],
        perimeter: 14.0,
        counts: (6, 12, 8),
    });

    // The L profile 40 × 30 with legs 5: a 40 × 5 foot (area 200, centre
    // (20, 2.5)) and a 5 × 25 upright on it (area 125, centre (2.5, 17.5)).
    // Area 325; centroid (200·20 + 125·2.5, 200·2.5 + 125·17.5) / 325.
    // Perimeter 40 + 5 + 35 + 25 + 5 + 30 = 140. Six sides and two caps:
    // 8 faces, 3·6 edges, 2·6 vertices.
    cases.push(Case {
        name: "L profile",
        curves: polygon(
            7,
            3,
            &[
                [0.0, 0.0],
                [40.0, 0.0],
                [40.0, 5.0],
                [5.0, 5.0],
                [5.0, 30.0],
                [0.0, 30.0],
            ],
        ),
        parts: vec![part(325.0, [4312.5 / 325.0, 2687.5 / 325.0], 0)],
        perimeter: 140.0,
        counts: (8, 18, 12),
    });

    // Right triangle with legs 6 and 8: area 24, hypotenuse 10, perimeter
    // 24, centroid a third of the way along each leg.
    cases.push(Case {
        name: "triangle",
        curves: polygon(1, 1, &[[0.0, 0.0], [6.0, 0.0], [0.0, 8.0]]),
        parts: vec![part(24.0, [2.0, 8.0 / 3.0], 0)],
        perimeter: 24.0,
        counts: (5, 9, 6),
    });

    // Circle of radius 3 about (2, −1): area 9π, perimeter 6π. A cylinder:
    // wall and two discs, two circles with a vertex each.
    cases.push(Case {
        name: "circle",
        curves: vec![circle(42, [2.0, -1.0], 3.0)],
        parts: vec![part(9.0 * PI, [2.0, -1.0], 0)],
        perimeter: 6.0 * PI,
        counts: (3, 2, 2),
    });

    // Plate 10 × 6 with a round hole of radius 2 about (4, 3).
    // Area 60 − 4π, perimeter 32 + 4π, centroid by subtraction:
    // (60·(5, 3) − 4π·(4, 3)) / (60 − 4π). A box (6, 12, 8) plus the hole's
    // wall, two circles and two vertices.
    let a = 60.0 - 4.0 * PI;
    let mut curves = rectangle(1, 1, (0.0, 10.0), (0.0, 6.0));
    curves.push(circle(9, [4.0, 3.0], 2.0));
    cases.push(Case {
        name: "plate with a round hole",
        curves,
        parts: vec![part(a, [(300.0 - 16.0 * PI) / a, 3.0], 1)],
        perimeter: 32.0 + 4.0 * PI,
        counts: (7, 14, 10),
    });

    // Plate 10 × 6 with the square hole 2..5 by 1..4 (area 9, centre
    // (3.5, 2.5)). Area 51, perimeter 32 + 12, centroid
    // (60·(5, 3) − 9·(3.5, 2.5)) / 51. Two boxes' worth of sides, edges
    // and vertices, two caps.
    let mut curves = rectangle(1, 1, (0.0, 10.0), (0.0, 6.0));
    curves.extend(rectangle(5, 1, (2.0, 5.0), (1.0, 4.0)));
    cases.push(Case {
        name: "plate with a rectangular hole",
        curves,
        parts: vec![part(51.0, [268.5 / 51.0, 157.5 / 51.0], 1)],
        perimeter: 44.0,
        counts: (10, 24, 16),
    });

    // Slot: two half circles of radius r = 2 about (0, 0) and (6, 0),
    // joined by lines of length L = 6. Area 2rL + πr² = 24 + 4π, perimeter
    // 2L + 2πr = 12 + 4π, centre (3, 0). Four sides.
    cases.push(Case {
        name: "slot",
        curves: vec![
            line(1, [0.0, -2.0], [6.0, -2.0]),
            arc(2, [6.0, 0.0], [6.0, -2.0], [6.0, 2.0]),
            line(3, [6.0, 2.0], [0.0, 2.0]),
            arc(4, [0.0, 0.0], [0.0, 2.0], [0.0, -2.0]),
        ],
        parts: vec![part(24.0 + 4.0 * PI, [3.0, 0.0], 0)],
        perimeter: 12.0 + 4.0 * PI,
        counts: (6, 12, 8),
    });

    // D shape: the upper half of the disc of radius 5 about the origin.
    // Area 25π/2, perimeter 5π + 10, centroid 4r/(3π) above the straight
    // side.
    cases.push(Case {
        name: "D shape",
        curves: vec![
            arc(3, [0.0, 0.0], [5.0, 0.0], [-5.0, 0.0]),
            line(8, [-5.0, 0.0], [5.0, 0.0]),
        ],
        parts: vec![part(12.5 * PI, [0.0, 20.0 / (3.0 * PI)], 0)],
        perimeter: 5.0 * PI + 10.0,
        counts: (4, 6, 4),
    });

    // Lens: what the discs of radius 5 about (−3, 0) and (3, 0) share.
    // They meet at (0, ±4); each arc spans 2α with cos α = 3/5. The lens is
    // two circular segments of area r²(2α − sin 2α)/2 = 25α − 12 each:
    // area 50α − 24, perimeter 2·r·2α = 20α, centre the origin.
    let alpha = 0.6_f64.acos();
    cases.push(Case {
        name: "lens",
        curves: vec![
            arc(5, [-3.0, 0.0], [0.0, -4.0], [0.0, 4.0]),
            arc(2, [3.0, 0.0], [0.0, 4.0], [0.0, -4.0]),
        ],
        parts: vec![part(50.0 * alpha - 24.0, [0.0, 0.0], 0)],
        perimeter: 20.0 * alpha,
        counts: (4, 6, 4),
    });

    // Rounded rectangle a × b = 10 × 6 with fillets r = 1.
    // Area ab − (4 − π)r² = 56 + π, perimeter 2(a − 2r) + 2(b − 2r) + 2πr
    // = 24 + 2π, centre (5, 3). Eight sides.
    cases.push(Case {
        name: "rounded rectangle",
        curves: vec![
            line(1, [1.0, 0.0], [9.0, 0.0]),
            arc(2, [9.0, 1.0], [9.0, 0.0], [10.0, 1.0]),
            line(3, [10.0, 1.0], [10.0, 5.0]),
            arc(4, [9.0, 5.0], [10.0, 5.0], [9.0, 6.0]),
            line(5, [9.0, 6.0], [1.0, 6.0]),
            arc(6, [1.0, 5.0], [1.0, 6.0], [0.0, 5.0]),
            line(7, [0.0, 5.0], [0.0, 1.0]),
            arc(8, [1.0, 1.0], [0.0, 1.0], [1.0, 0.0]),
        ],
        parts: vec![part(56.0 + PI, [5.0, 3.0], 0)],
        perimeter: 24.0 + 2.0 * PI,
        counts: (10, 24, 16),
    });

    // Ring: circles of radius 5 and 3 about (1, 1). Area 16π, perimeter
    // 16π. Two walls, two caps; four circles with a vertex each.
    cases.push(Case {
        name: "ring",
        curves: vec![circle(2, [1.0, 1.0], 3.0), circle(6, [1.0, 1.0], 5.0)],
        parts: vec![part(16.0 * PI, [1.0, 1.0], 1)],
        perimeter: 16.0 * PI,
        counts: (4, 4, 4),
    });

    // Island: a plate 20 × 20, a square hole 4..16 in it and a square
    // 8..12 inside the hole. Two regions about (10, 10): 400 − 144 = 256
    // and 16. Perimeter 80 + 48 + 16. A holed box (10, 24, 16) and a box.
    let mut curves = rectangle(20, 1, (0.0, 20.0), (0.0, 20.0));
    curves.extend(rectangle(10, 1, (4.0, 16.0), (4.0, 16.0)));
    curves.extend(rectangle(30, 1, (8.0, 12.0), (8.0, 12.0)));
    cases.push(Case {
        name: "island",
        curves,
        // The plate's ids start at 20, the island's at 30.
        parts: vec![part(256.0, [10.0, 10.0], 1), part(16.0, [10.0, 10.0], 0)],
        perimeter: 144.0,
        counts: (16, 36, 24),
    });

    // Two separate rectangles: 2 × 1 at the origin (centre (1, 0.5)) and
    // 3 × 2 at (5, 0) (centre (6.5, 1)). Perimeter 6 + 10. Two boxes.
    let mut curves = rectangle(1, 1, (0.0, 2.0), (0.0, 1.0));
    curves.extend(rectangle(11, 1, (5.0, 8.0), (0.0, 2.0)));
    cases.push(Case {
        name: "two rectangles",
        curves,
        parts: vec![part(2.0, [1.0, 0.5], 0), part(6.0, [6.5, 1.0], 0)],
        perimeter: 16.0,
        counts: (12, 24, 16),
    });

    // Bite: the square 0..10 less the half disc of radius 2 about (10, 5)
    // on its right side: a concave arc in an outer loop. Area 100 − 2π;
    // perimeter 10 + 3 + 2π + 3 + 10 + 10; the half disc's centroid is
    // 4r/(3π) = 8/(3π) left of x = 10, so the centroid's x is
    // (100·5 − 2π·(10 − 8/(3π))) / (100 − 2π) = (500 − 20π + 16/3) / (100 − 2π).
    // Six sides.
    let a = 100.0 - 2.0 * PI;
    cases.push(Case {
        name: "bite",
        curves: vec![
            line(1, [0.0, 0.0], [10.0, 0.0]),
            line(2, [10.0, 0.0], [10.0, 3.0]),
            arc(3, [10.0, 5.0], [10.0, 7.0], [10.0, 3.0]),
            line(4, [10.0, 7.0], [10.0, 10.0]),
            line(5, [10.0, 10.0], [0.0, 10.0]),
            line(6, [0.0, 10.0], [0.0, 0.0]),
        ],
        parts: vec![part(a, [(500.0 - 20.0 * PI + 16.0 / 3.0) / a, 5.0], 0)],
        perimeter: 36.0 + 2.0 * PI,
        counts: (8, 18, 12),
    });

    // Plate 20 × 10 with a slot through it: half circles of radius 1 about
    // (5, 5) and (9, 5). The slot's area is 2·1·4 + π about (7, 5).
    // Area 192 − π, perimeter 60 + 8 + 2π, centroid
    // (200·10 − (8 + π)·7) / (192 − π) in x. A box plus four walls.
    let a = 192.0 - PI;
    let mut curves = rectangle(1, 1, (0.0, 20.0), (0.0, 10.0));
    curves.extend([
        line(11, [5.0, 4.0], [9.0, 4.0]),
        arc(12, [9.0, 5.0], [9.0, 4.0], [9.0, 6.0]),
        line(13, [9.0, 6.0], [5.0, 6.0]),
        arc(14, [5.0, 5.0], [5.0, 6.0], [5.0, 4.0]),
    ]);
    cases.push(Case {
        name: "plate with a slot",
        curves,
        parts: vec![part(a, [(2000.0 - 7.0 * (8.0 + PI)) / a, 5.0], 1)],
        perimeter: 68.0 + 2.0 * PI,
        counts: (10, 24, 16),
    });

    // Island with a hole of its own: the plate, hole and island above, and
    // a circle of radius 1 about (10, 10) in the island. Nesting depth 3:
    // the circle is a hole of the island, not of the plate. Regions
    // 400 − 144 = 256 and 16 − π, both about (10, 10). A holed box
    // (10, 24, 16) and a box with a round hole (7, 14, 10).
    let mut curves = rectangle(20, 1, (0.0, 20.0), (0.0, 20.0));
    curves.extend(rectangle(10, 1, (4.0, 16.0), (4.0, 16.0)));
    curves.extend(rectangle(30, 1, (8.0, 12.0), (8.0, 12.0)));
    curves.push(circle(5, [10.0, 10.0], 1.0));
    cases.push(Case {
        name: "island with a hole",
        curves,
        parts: vec![
            part(256.0, [10.0, 10.0], 1),
            part(16.0 - PI, [10.0, 10.0], 1),
        ],
        perimeter: 144.0 + 2.0 * PI,
        counts: (17, 38, 26),
    });

    // The lens above with a round hole of radius 1/2 about its centre: a
    // loop inside a loop made of arcs. The hole's first point (0.5, 0) is
    // between the chord and the arc of one arc of the lens and beyond the
    // chord of the other. Area 50α − 24 − π/4, perimeter 20α + π. The lens
    // (4, 6, 4) plus a wall, two circles and two vertices.
    cases.push(Case {
        name: "lens with a hole",
        curves: vec![
            arc(5, [-3.0, 0.0], [0.0, -4.0], [0.0, 4.0]),
            arc(2, [3.0, 0.0], [0.0, 4.0], [0.0, -4.0]),
            circle(9, [0.0, 0.0], 0.5),
        ],
        parts: vec![part(50.0 * alpha - 24.0 - PI / 4.0, [0.0, 0.0], 1)],
        perimeter: 20.0 * alpha + PI,
        counts: (5, 8, 6),
    });

    // The D above and a disc of radius 1/2 about (0, −2): under the flat
    // side, so outside the D, but inside the circle its arc is part of.
    // Two regions: 25π/2 and π/4. Perimeter 5π + 10 + π. The D (4, 6, 4)
    // and a cylinder (3, 2, 2).
    cases.push(Case {
        name: "D and a disc under it",
        curves: vec![
            arc(3, [0.0, 0.0], [5.0, 0.0], [-5.0, 0.0]),
            line(8, [-5.0, 0.0], [5.0, 0.0]),
            circle(9, [0.0, -2.0], 0.5),
        ],
        parts: vec![
            part(12.5 * PI, [0.0, 20.0 / (3.0 * PI)], 0),
            part(PI / 4.0, [0.0, -2.0], 0),
        ],
        perimeter: 6.0 * PI + 10.0,
        counts: (7, 8, 6),
    });

    // The bite above with a disc of radius 0.4 about (9.2, 5) lying in the
    // bitten-out half disc: inside the square's box and inside the circle
    // of the concave arc, yet outside the region. Its first point is
    // (9.6, 5). Two regions: 100 − 2π and 0.16π. Perimeter 36 + 2π + 0.8π.
    // The bite (8, 18, 12) and a cylinder (3, 2, 2).
    let a = 100.0 - 2.0 * PI;
    cases.push(Case {
        name: "bite with a disc in it",
        curves: vec![
            line(1, [0.0, 0.0], [10.0, 0.0]),
            line(2, [10.0, 0.0], [10.0, 3.0]),
            arc(3, [10.0, 5.0], [10.0, 7.0], [10.0, 3.0]),
            line(4, [10.0, 7.0], [10.0, 10.0]),
            line(5, [10.0, 10.0], [0.0, 10.0]),
            line(6, [0.0, 10.0], [0.0, 0.0]),
            circle(9, [9.2, 5.0], 0.4),
        ],
        parts: vec![
            part(a, [(500.0 - 20.0 * PI + 16.0 / 3.0) / a, 5.0], 0),
            part(0.16 * PI, [9.2, 5.0], 0),
        ],
        perimeter: 36.0 + 2.8 * PI,
        counts: (11, 20, 14),
    });

    cases
}

/// The catalogue with every length multiplied by `k`.
pub fn catalogue(k: f64) -> Vec<Case> {
    unit_catalogue()
        .into_iter()
        .map(|case| Case {
            curves: scale_curves(&case.curves, k),
            parts: case
                .parts
                .into_iter()
                .map(|p| part(p.area * k * k, scale_point(p.centroid, k), p.holes))
                .collect(),
            perimeter: case.perimeter * k,
            ..case
        })
        .collect()
}

/// A polygon (counter-clockwise) whose corner `i` is rounded by a
/// tangent arc of radius `radii[i]` (0: left sharp): the sketch, its area,
/// the length of its straight pieces, and the turn of each corner (left
/// positive).
pub fn rounded_polygon(points: &[[f64; 2]], radii: &[f64]) -> (Vec<SketchCurve>, f64, f64, Vec<f64>) {
    let n = points.len();
    let mut corners = Vec::new();
    let mut polygon_area = 0.0;
    for i in 0..n {
        let (p, q) = (points[i], points[(i + 1) % n]);
        polygon_area += 0.5 * (p[0] * q[1] - p[1] * q[0]);
    }
    let unit = |a: [f64; 2], b: [f64; 2]| {
        let l = (b[0] - a[0]).hypot(b[1] - a[1]);
        [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
    };
    // Per corner: the tangent points on the incoming and outgoing side,
    // the centre, the turn, the radius.
    for i in 0..n {
        let (prev, here, next) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n]);
        let (a, b) = (unit(prev, here), unit(here, next));
        let turn = (a[0] * b[1] - a[1] * b[0]).atan2(a[0] * b[0] + a[1] * b[1]);
        let big = radii[i];
        // A corner that turns right (an inside corner) is rounded the
        // other way round: its arc's centre lies to the right.
        let side = turn.signum();
        let back = big * (turn.abs() / 2.0).tan();
        let t_in = [here[0] - a[0] * back, here[1] - a[1] * back];
        let t_out = [here[0] + b[0] * back, here[1] + b[1] * back];
        let centre = [t_in[0] - a[1] * big * side, t_in[1] + a[0] * big * side];
        corners.push((t_in, t_out, centre, turn, big));
    }
    let mut curves = Vec::new();
    let mut id = 1;
    let mut lines = 0.0;
    let mut cut = 0.0;
    for i in 0..n {
        let (t_in, t_out, centre, turn, big) = corners[i];
        if big > 0.0 {
            if turn > 0.0 {
                curves.push(arc(id, centre, t_in, t_out));
            } else {
                curves.push(arc(id, centre, t_out, t_in));
            }
            id += 1;
        }
        let next_in = corners[(i + 1) % n].0;
        curves.push(line(id, t_out, next_in));
        id += 1;
        lines += (next_in[0] - t_out[0]).hypot(next_in[1] - t_out[1]);
        // What the arc cuts off an outside corner, or fills in an inside
        // one.
        cut += turn.signum() * big * big * ((turn.abs() / 2.0).tan() - turn.abs() / 2.0);
    }
    let turns = corners.iter().map(|c| c.3).collect();
    (curves, polygon_area - cut, lines, turns)
}

/// The corners of the plate of finding E1 at a tenth.
pub const PLATE_E1: [[f64; 2]; 8] = [
    [-12.0, -7.0],
    [12.0, -7.0],
    [15.0, 0.0],
    [15.0, 9.0],
    [9.5, 17.5],
    [-9.5, 17.5],
    [-15.0, 9.0],
    [-15.0, 0.0],
];

/// The outline of the plate of finding E1 at a tenth, with its closed
/// forms, and which corners are sharp.
pub fn plate_e1() -> (
    Vec<SketchCurve>,
    f64,
    f64,
    Vec<f64>,
    [usize; 2],
) {
    let radii = [0.0, 0.0, 1.4, 1.4, 1.4, 1.4, 1.4, 1.4];
    let (curves, area, lines, turns) = rounded_polygon(&PLATE_E1, &radii);
    (curves, area, lines, turns, [0, 1])
}
