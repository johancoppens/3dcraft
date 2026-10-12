//! Shared by the integration tests: the catalogue of sketches whose
//! solutions are known by hand, and small helpers.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::f64::consts::PI;

use craft3d_math::Tolerance;
use craft3d_solve::{
    Constraint as C, ConstraintStatus, EntityId, Sketch, Solution, SolveOptions, solve,
};

pub fn options() -> SolveOptions {
    SolveOptions::new(Tolerance::default())
}

pub fn solved(sketch: &Sketch) -> Solution {
    match solve(sketch, &options()) {
        Ok(s) => s,
        Err(e) => panic!("solve failed: {e}"),
    }
}

/// The sketch with the solution's coordinates: the next starting guess.
pub fn resolved(sketch: &Sketch, solution: &Solution) -> Sketch {
    Sketch {
        entities: solution.entities.clone(),
        constraints: sketch.constraints.clone(),
    }
}

pub fn statuses(solution: &Solution) -> Vec<ConstraintStatus> {
    solution.constraints.iter().map(|c| c.status).collect()
}

/// A sketch with the answer worked out by hand.
pub struct Case {
    pub name: &'static str,
    pub sketch: Sketch,
    /// Expected point coordinates, at scale 1 (multiply by the scale).
    pub points: Vec<(EntityId, [f64; 2])>,
    /// Expected radii of circles and arcs, at scale 1.
    pub radii: Vec<(EntityId, f64)>,
    /// Degrees of freedom, counted by hand in the comment of the case.
    pub dof: usize,
}

struct B {
    k: f64,
    s: Sketch,
}

impl B {
    fn pt(&mut self, x: f64, y: f64) -> EntityId {
        self.s.add_point(x * self.k, y * self.k)
    }
    fn line(&mut self, a: EntityId, b: EntityId) -> EntityId {
        self.s.add_line(a, b)
    }
    fn circle(&mut self, c: EntityId, r: f64) -> EntityId {
        self.s.add_circle(c, r * self.k)
    }
    fn arc(&mut self, c: EntityId, s: EntityId, e: EntityId) -> EntityId {
        self.s.add_arc(c, s, e)
    }
    fn add(&mut self, c: C) {
        self.s.add_constraint(c);
    }
    fn fix(&mut self, point: EntityId, x: f64, y: f64) {
        self.add(C::Fixed {
            point,
            at: Some([x * self.k, y * self.k]),
        });
    }
    fn dist(&mut self, a: EntityId, b: EntityId, value: f64) {
        self.add(C::Distance {
            a,
            b,
            value: value * self.k,
        });
    }
    fn hdist(&mut self, a: EntityId, b: EntityId, value: f64) {
        self.add(C::HorizontalDistance {
            a,
            b,
            value: value * self.k,
        });
    }
    fn vdist(&mut self, a: EntityId, b: EntityId, value: f64) {
        self.add(C::VerticalDistance {
            a,
            b,
            value: value * self.k,
        });
    }
    fn radius(&mut self, circle: EntityId, value: f64) {
        self.add(C::Radius {
            circle,
            value: value * self.k,
        });
    }
    fn diameter(&mut self, circle: EntityId, value: f64) {
        self.add(C::Diameter {
            circle,
            value: value * self.k,
        });
    }
}

/// The phase-1 acceptance shape: an L profile, 40 wide and 30 high with
/// legs 10 thick, drawn roughly and then dimensioned. Returns the sketch
/// and its six corners, counter-clockwise from the origin.
pub fn l_profile(k: f64) -> (Sketch, [EntityId; 6]) {
    let mut b = B {
        k,
        s: Sketch::new(),
    };
    let p = [
        b.pt(0.3, -0.4),
        b.pt(37.0, 1.0),
        b.pt(41.0, 8.0),
        b.pt(12.0, 11.0),
        b.pt(9.0, 27.0),
        b.pt(-1.0, 31.0),
    ];
    let l: Vec<EntityId> = (0..6).map(|i| b.line(p[i], p[(i + 1) % 6])).collect();
    b.fix(p[0], 0.0, 0.0);
    for (i, line) in l.iter().enumerate() {
        b.add(if i % 2 == 0 {
            C::Horizontal { line: *line }
        } else {
            C::Vertical { line: *line }
        });
    }
    b.hdist(p[0], p[1], 40.0);
    b.vdist(p[0], p[5], 30.0);
    b.dist(p[1], p[2], 10.0);
    b.dist(p[5], p[4], 10.0);
    (b.s, p)
}

/// Every sketch at scale `k` (all lengths and coordinates times `k`).
pub fn catalogue(k: f64) -> Vec<Case> {
    let mut cases = Vec::new();
    let new = || B {
        k,
        s: Sketch::new(),
    };

    // 1. Rectangle 10 x 5 with a corner at the origin.
    // Unknowns 8. Fixed 2 + two horizontals 2 + two verticals 2 + two
    // dimensions 2 = 8 equations: DOF 0.
    {
        let mut b = new();
        let p = [
            b.pt(0.1, -0.2),
            b.pt(9.0, 0.5),
            b.pt(11.0, 6.0),
            b.pt(-1.0, 4.5),
        ];
        let l: Vec<EntityId> = (0..4).map(|i| b.line(p[i], p[(i + 1) % 4])).collect();
        b.fix(p[0], 0.0, 0.0);
        b.add(C::Horizontal { line: l[0] });
        b.add(C::Vertical { line: l[1] });
        b.add(C::Horizontal { line: l[2] });
        b.add(C::Vertical { line: l[3] });
        b.dist(p[0], p[1], 10.0);
        b.dist(p[1], p[2], 5.0);
        cases.push(Case {
            name: "rectangle",
            sketch: b.s,
            points: vec![
                (p[0], [0.0, 0.0]),
                (p[1], [10.0, 0.0]),
                (p[2], [10.0, 5.0]),
                (p[3], [0.0, 5.0]),
            ],
            radii: vec![],
            dof: 0,
        });
    }

    // 2. L profile. Unknowns 12. Fixed 2 + six axis-aligned sides 6 + four
    // dimensions 4 = 12: DOF 0. Walking round from the origin: 40 along x,
    // 10 up, back to x = 10 (the upper leg is 10 thick), up to y = 30,
    // back to x = 0.
    {
        let (sketch, p) = l_profile(k);
        cases.push(Case {
            name: "l_profile",
            sketch,
            points: vec![
                (p[0], [0.0, 0.0]),
                (p[1], [40.0, 0.0]),
                (p[2], [40.0, 10.0]),
                (p[3], [10.0, 10.0]),
                (p[4], [10.0, 30.0]),
                (p[5], [0.0, 30.0]),
            ],
            radii: vec![],
            dof: 0,
        });
    }

    // 3. Triangle from three lengths, 4-3-5: right-angled at b because
    // 4^2 + 3^2 = 5^2, so with a at the origin and ab along +x, c = (4, 3)
    // (above, where it was drawn). Unknowns 6. Fixed 2 + level 1 + three
    // lengths 3 = 6: DOF 0.
    {
        let mut b = new();
        let pa = b.pt(0.0, 0.0);
        let pb = b.pt(4.5, 0.3);
        let pc = b.pt(3.5, 2.5);
        b.fix(pa, 0.0, 0.0);
        b.add(C::HorizontalPoints { a: pa, b: pb });
        b.dist(pa, pb, 4.0);
        b.dist(pb, pc, 3.0);
        b.dist(pc, pa, 5.0);
        cases.push(Case {
            name: "triangle_three_lengths",
            sketch: b.s,
            points: vec![(pb, [4.0, 0.0]), (pc, [4.0, 3.0])],
            radii: vec![],
            dof: 0,
        });
    }

    // 4. Circle of radius 2 resting on a horizontal line, its centre 5 to
    // the right of the line's start: centre (5, 2). Unknowns 7 (two line
    // ends, centre, radius). Fixed 2 + horizontal 1 + length 1 + radius 1 +
    // offset 1 + tangent 1 = 7: DOF 0.
    {
        let mut b = new();
        let p0 = b.pt(0.0, 0.0);
        let p1 = b.pt(9.0, 0.4);
        let line = b.line(p0, p1);
        let c = b.pt(4.5, 2.5);
        let circle = b.circle(c, 2.2);
        b.fix(p0, 0.0, 0.0);
        b.add(C::Horizontal { line });
        b.hdist(p0, p1, 10.0);
        b.radius(circle, 2.0);
        b.hdist(p0, c, 5.0);
        b.add(C::Tangent { a: line, b: circle });
        cases.push(Case {
            name: "line_tangent_to_circle",
            sketch: b.s,
            points: vec![(p1, [10.0, 0.0]), (c, [5.0, 2.0])],
            radii: vec![(circle, 2.0)],
            dof: 0,
        });
    }

    // 5. Two circles touching from outside, radii 3 and 2, centres level:
    // the second centre is at 3 + 2 = 5. Unknowns 6. Fixed 2 + two radii 2 +
    // level 1 + tangent 1 = 6: DOF 0.
    {
        let mut b = new();
        let c1 = b.pt(0.0, 0.0);
        let k1 = b.circle(c1, 2.6);
        let c2 = b.pt(7.5, 0.4);
        let k2 = b.circle(c2, 1.7);
        b.fix(c1, 0.0, 0.0);
        b.radius(k1, 3.0);
        b.radius(k2, 2.0);
        b.add(C::HorizontalPoints { a: c1, b: c2 });
        b.add(C::Tangent { a: k1, b: k2 });
        cases.push(Case {
            name: "tangent_circles_outside",
            sketch: b.s,
            points: vec![(c2, [5.0, 0.0])],
            radii: vec![(k1, 3.0), (k2, 2.0)],
            dof: 0,
        });
    }

    // 6. A circle of radius 2 touching a circle of radius 5 from inside,
    // centres level: the centres are 5 - 2 = 3 apart. Same count as 5.
    {
        let mut b = new();
        let c1 = b.pt(0.0, 0.0);
        let k1 = b.circle(c1, 5.5);
        let c2 = b.pt(2.5, 0.3);
        let k2 = b.circle(c2, 1.6);
        b.fix(c1, 0.0, 0.0);
        b.radius(k1, 5.0);
        b.radius(k2, 2.0);
        b.add(C::HorizontalPoints { a: c1, b: c2 });
        b.add(C::Tangent { a: k1, b: k2 });
        cases.push(Case {
            name: "tangent_circles_inside",
            sketch: b.s,
            points: vec![(c2, [3.0, 0.0])],
            radii: vec![(k1, 5.0), (k2, 2.0)],
            dof: 0,
        });
    }

    // 7. Rounded corner: the x axis and the y axis joined by a fillet of
    // radius 2. Its centre is 2 from both axes, (2, 2), and it meets them
    // at the feet of the centre, (2, 0) and (0, 2). The arc runs counter-
    // clockwise from (0, 2) (west of the centre) to (2, 0) (south).
    // Unknowns 10 (a, th, c, tv, b). Arc 1 + fixed 2 + horizontal 1 +
    // vertical 1 + two offsets 2 + two tangents 2 + radius 1 = 10: DOF 0.
    {
        let mut b = new();
        let a = b.pt(10.0, 0.0);
        let th = b.pt(2.4, 0.3);
        let c = b.pt(2.5, 1.7);
        let tv = b.pt(-0.2, 2.3);
        let far = b.pt(0.3, 9.0);
        let l1 = b.line(th, a);
        let l2 = b.line(far, tv);
        let arc = b.arc(c, tv, th);
        b.fix(a, 10.0, 0.0);
        b.add(C::Horizontal { line: l1 });
        b.add(C::Vertical { line: l2 });
        b.hdist(far, a, 10.0);
        b.vdist(a, far, 10.0);
        b.add(C::Tangent { a: l1, b: arc });
        b.add(C::Tangent { a: arc, b: l2 });
        b.radius(arc, 2.0);
        cases.push(Case {
            name: "rounded_corner",
            sketch: b.s,
            points: vec![
                (th, [2.0, 0.0]),
                (c, [2.0, 2.0]),
                (tv, [0.0, 2.0]),
                (far, [0.0, 10.0]),
            ],
            radii: vec![(arc, 2.0)],
            dof: 0,
        });
    }

    // 8. Slot: two half circles of radius 5 around (0, 0) and (20, 0),
    // joined by the lines y = 5 and y = -5. Unknowns 12 (two centres, four
    // corners). Two arcs 2 + fixed 2 + level 1 + spacing 1 + radius 1 +
    // equal radii 1 + four tangents 4 = 12: DOF 0.
    {
        let mut b = new();
        let c1 = b.pt(0.2, -0.1);
        let c2 = b.pt(19.0, 0.6);
        let tl = b.pt(0.4, 4.6);
        let bl = b.pt(-0.3, -5.5);
        let tr = b.pt(20.5, 5.3);
        let br = b.pt(19.6, -4.4);
        let top = b.line(tl, tr);
        let bottom = b.line(bl, br);
        let left = b.arc(c1, tl, bl);
        let right = b.arc(c2, br, tr);
        b.fix(c1, 0.0, 0.0);
        b.add(C::HorizontalPoints { a: c1, b: c2 });
        b.dist(c1, c2, 20.0);
        b.radius(left, 5.0);
        b.add(C::Equal { a: left, b: right });
        b.add(C::Tangent { a: top, b: left });
        b.add(C::Tangent { a: top, b: right });
        b.add(C::Tangent { a: bottom, b: left });
        b.add(C::Tangent {
            a: bottom,
            b: right,
        });
        cases.push(Case {
            name: "slot",
            sketch: b.s,
            points: vec![
                (c2, [20.0, 0.0]),
                (tl, [0.0, 5.0]),
                (bl, [0.0, -5.0]),
                (tr, [20.0, 5.0]),
                (br, [20.0, -5.0]),
            ],
            radii: vec![(left, 5.0), (right, 5.0)],
            dof: 0,
        });
    }

    // 9. Mirror image in the line y = x: (3, 1) becomes (1, 3).
    // Unknowns 8. Three fixed points 6 + symmetric 2 = 8: DOF 0.
    {
        let mut b = new();
        let o = b.pt(0.0, 0.0);
        let q = b.pt(10.0, 10.0);
        let axis = b.line(o, q);
        let pa = b.pt(3.0, 1.0);
        let pb = b.pt(0.2, 2.1);
        b.fix(o, 0.0, 0.0);
        b.fix(q, 10.0, 10.0);
        b.fix(pa, 3.0, 1.0);
        b.add(C::Symmetric { a: pa, b: pb, axis });
        cases.push(Case {
            name: "symmetric_points",
            sketch: b.s,
            points: vec![(pb, [1.0, 3.0])],
            radii: vec![],
            dof: 0,
        });
    }

    // 10. Middle of the segment (0, 0)-(6, 8): (3, 4).
    // Unknowns 6. Two fixed points 4 + midpoint 2 = 6: DOF 0.
    {
        let mut b = new();
        let p0 = b.pt(0.0, 0.0);
        let p1 = b.pt(6.0, 8.0);
        let line = b.line(p0, p1);
        let m = b.pt(1.0, 5.0);
        b.fix(p0, 0.0, 0.0);
        b.fix(p1, 6.0, 8.0);
        b.add(C::Midpoint { point: m, line });
        cases.push(Case {
            name: "midpoint",
            sketch: b.s,
            points: vec![(m, [3.0, 4.0])],
            radii: vec![],
            dof: 0,
        });
    }

    // 11. A line of length 10 at 30 degrees to the x axis ends at
    // (10 cos 30, 10 sin 30) = (5 sqrt 3, 5). Unknowns 6. Two fixed points
    // 4 + length 1 + angle 1 = 6: DOF 0.
    {
        let mut b = new();
        let o = b.pt(0.0, 0.0);
        let a = b.pt(10.0, 0.0);
        let p = b.pt(7.0, 6.0);
        let base = b.line(o, a);
        let arm = b.line(o, p);
        b.fix(o, 0.0, 0.0);
        b.fix(a, 10.0, 0.0);
        b.dist(o, p, 10.0);
        b.add(C::Angle {
            a: base,
            b: arm,
            value: PI / 6.0,
        });
        cases.push(Case {
            name: "angle_between_lines",
            sketch: b.s,
            points: vec![(p, [5.0 * 3f64.sqrt(), 5.0])],
            radii: vec![],
            dof: 0,
        });
    }

    // 12. Concentric circles around (2, 3): radius 4 and diameter 3.
    // Unknowns 6. Fixed 2 + radius 1 + concentric 2 + diameter 1: DOF 0.
    {
        let mut b = new();
        let c1 = b.pt(2.0, 3.0);
        let k1 = b.circle(c1, 3.0);
        let c2 = b.pt(2.5, 2.2);
        let k2 = b.circle(c2, 1.3);
        b.fix(c1, 2.0, 3.0);
        b.radius(k1, 4.0);
        b.add(C::Concentric { a: k1, b: k2 });
        b.diameter(k2, 3.0);
        cases.push(Case {
            name: "concentric_circles",
            sketch: b.s,
            points: vec![(c2, [2.0, 3.0])],
            radii: vec![(k1, 4.0), (k2, 1.5)],
            dof: 0,
        });
    }

    // 13. Equal radii: the second circle takes radius 2.5 and, having no
    // reason to move, stays where it is. Unknowns 6. Fixed 2 + radius 1 +
    // equal 1 = 4: DOF 2 (the second centre).
    {
        let mut b = new();
        let c1 = b.pt(0.0, 0.0);
        let k1 = b.circle(c1, 2.0);
        let c2 = b.pt(7.0, 1.0);
        let k2 = b.circle(c2, 4.0);
        b.fix(c1, 0.0, 0.0);
        b.radius(k1, 2.5);
        b.add(C::Equal { a: k1, b: k2 });
        cases.push(Case {
            name: "equal_radii",
            sketch: b.s,
            points: vec![(c2, [7.0, 1.0])],
            radii: vec![(k1, 2.5), (k2, 2.5)],
            dof: 2,
        });
    }

    // 14. A point at (6, 8), 10 from the centre, pulled onto the circle of
    // radius 5 around the origin: the shortest way is along the radius, to
    // half the distance, (3, 4). Unknowns 5. Fixed 2 + radius 1 + on the
    // circle 1 = 4: DOF 1 (the point slides round).
    {
        let mut b = new();
        let c = b.pt(0.0, 0.0);
        let circle = b.circle(c, 5.0);
        let p = b.pt(6.0, 8.0);
        b.fix(c, 0.0, 0.0);
        b.radius(circle, 5.0);
        b.add(C::PointOnCircle { point: p, circle });
        cases.push(Case {
            name: "point_on_circle",
            sketch: b.s,
            points: vec![(p, [3.0, 4.0])],
            radii: vec![(circle, 5.0)],
            dof: 1,
        });
    }

    // 15. Bolt circle: four holes of diameter 2 on a pitch circle of radius
    // 10, a quarter turn apart, the first one on the +x axis: east, north,
    // west, south. Unknowns 15 (centre 2, pitch radius 1, four hole centres
    // 8, four hole radii 4). Fixed 2 + pitch radius 1 + four on the pitch
    // circle 4 + three angles 3 + level 1 + three equal 3 + diameter 1 = 15:
    // DOF 0.
    {
        let mut b = new();
        let o = b.pt(0.0, 0.0);
        let pitch = b.circle(o, 9.0);
        let h = [
            b.pt(9.0, 1.0),
            b.pt(1.0, 9.0),
            b.pt(-9.0, 1.0),
            b.pt(0.5, -11.0),
        ];
        let spokes: Vec<EntityId> = h.iter().map(|p| b.line(o, *p)).collect();
        let holes: Vec<EntityId> = h
            .iter()
            .enumerate()
            .map(|(i, p)| b.circle(*p, 0.8 + 0.1 * i as f64))
            .collect();
        b.fix(o, 0.0, 0.0);
        b.radius(pitch, 10.0);
        for p in h {
            b.add(C::PointOnCircle {
                point: p,
                circle: pitch,
            });
        }
        for i in 0..3 {
            b.add(C::Angle {
                a: spokes[i],
                b: spokes[i + 1],
                value: PI / 2.0,
            });
        }
        b.add(C::HorizontalPoints { a: o, b: h[0] });
        for i in 0..3 {
            b.add(C::Equal {
                a: holes[i],
                b: holes[i + 1],
            });
        }
        b.diameter(holes[0], 2.0);
        cases.push(Case {
            name: "bolt_circle",
            sketch: b.s,
            points: vec![
                (h[0], [10.0, 0.0]),
                (h[1], [0.0, 10.0]),
                (h[2], [-10.0, 0.0]),
                (h[3], [0.0, -10.0]),
            ],
            radii: holes.iter().map(|c| (*c, 1.0)).collect(),
            dof: 0,
        });
    }

    // 16. A free line: nothing moves, 4 degrees of freedom.
    {
        let mut b = new();
        let p0 = b.pt(1.0, 2.0);
        let p1 = b.pt(4.0, -3.0);
        b.line(p0, p1);
        cases.push(Case {
            name: "free_line",
            sketch: b.s,
            points: vec![(p0, [1.0, 2.0]), (p1, [4.0, -3.0])],
            radii: vec![],
            dof: 4,
        });
    }

    // 17. A rectangle without dimensions, one corner fixed. Unknowns 8.
    // Fixed 2 + four axis-aligned sides 4 = 6: DOF 2 (width and height).
    // All equations are linear and touch separate coordinates, so the
    // nearest solution is found per equation: p1.y joins the fixed 0; the
    // right side takes the mean x of its ends, (10 + 11) / 2 = 10.5; the
    // top takes the mean y, (6 + 5) / 2 = 5.5; p3.x joins the fixed 0.
    {
        let mut b = new();
        let p = [
            b.pt(0.0, 0.0),
            b.pt(10.0, 1.0),
            b.pt(11.0, 6.0),
            b.pt(0.5, 5.0),
        ];
        let l: Vec<EntityId> = (0..4).map(|i| b.line(p[i], p[(i + 1) % 4])).collect();
        b.add(C::Fixed {
            point: p[0],
            at: None,
        });
        b.add(C::Horizontal { line: l[0] });
        b.add(C::Vertical { line: l[1] });
        b.add(C::Horizontal { line: l[2] });
        b.add(C::Vertical { line: l[3] });
        cases.push(Case {
            name: "rectangle_without_dimensions",
            sketch: b.s,
            points: vec![
                (p[0], [0.0, 0.0]),
                (p[1], [10.5, 0.0]),
                (p[2], [10.5, 5.5]),
                (p[3], [0.0, 5.5]),
            ],
            radii: vec![],
            dof: 2,
        });
    }

    // 18. A free circle: centre and radius, 3 degrees of freedom.
    {
        let mut b = new();
        let c = b.pt(3.0, -2.0);
        let circle = b.circle(c, 1.5);
        cases.push(Case {
            name: "free_circle",
            sketch: b.s,
            points: vec![(c, [3.0, -2.0])],
            radii: vec![(circle, 1.5)],
            dof: 3,
        });
    }

    // 19. A free arc that already is one (both ends 5 from the centre):
    // three points less the arc equation, 5 degrees of freedom.
    {
        let mut b = new();
        let c = b.pt(0.0, 0.0);
        let s = b.pt(5.0, 0.0);
        let e = b.pt(-3.0, 4.0);
        let arc = b.arc(c, s, e);
        cases.push(Case {
            name: "free_arc",
            sketch: b.s,
            points: vec![(c, [0.0, 0.0]), (s, [5.0, 0.0]), (e, [-3.0, 4.0])],
            radii: vec![(arc, 5.0)],
            dof: 5,
        });
    }

    // 20. Two legs of equal length 7 at a right angle from the origin, the
    // first along +x: the second ends at (0, 7) (above, as drawn).
    // Unknowns 6. Fixed 2 + horizontal 1 + length 1 + perpendicular 1 +
    // equal 1 = 6: DOF 0.
    {
        let mut b = new();
        let o = b.pt(0.0, 0.0);
        let a = b.pt(6.0, 0.5);
        let p = b.pt(1.0, 6.0);
        let first = b.line(o, a);
        let second = b.line(o, p);
        b.fix(o, 0.0, 0.0);
        b.add(C::Horizontal { line: first });
        b.dist(o, a, 7.0);
        b.add(C::Perpendicular {
            a: first,
            b: second,
        });
        b.add(C::Equal {
            a: first,
            b: second,
        });
        cases.push(Case {
            name: "equal_perpendicular_legs",
            sketch: b.s,
            points: vec![(a, [7.0, 0.0]), (p, [0.0, 7.0])],
            radii: vec![],
            dof: 0,
        });
    }

    // 21. A line parallel to the x axis, 4 above it, starting above the
    // origin and 10 long: (0, 4) to (10, 4). Unknowns 8. Two fixed points
    // 4 + parallel 1 + point-line distance 1 + above the origin 1 + length
    // along x 1 = 8: DOF 0.
    {
        let mut b = new();
        let o = b.pt(0.0, 0.0);
        let a = b.pt(10.0, 0.0);
        let base = b.line(o, a);
        let p = b.pt(0.5, 3.5);
        let q = b.pt(9.0, 5.0);
        let other = b.line(p, q);
        b.fix(o, 0.0, 0.0);
        b.fix(a, 10.0, 0.0);
        b.add(C::Parallel { a: base, b: other });
        b.dist(p, base, 4.0);
        b.add(C::VerticalPoints { a: o, b: p });
        b.hdist(p, q, 10.0);
        cases.push(Case {
            name: "parallel_at_distance",
            sketch: b.s,
            points: vec![(p, [0.0, 4.0]), (q, [10.0, 4.0])],
            radii: vec![],
            dof: 0,
        });
    }

    // 22. The point 5 along the line from the origin to (6, 8): the line
    // has direction (0.6, 0.8), so (3, 4). Unknowns 6. Two fixed points 4 +
    // on the line 1 + distance 1 = 6: DOF 0.
    {
        let mut b = new();
        let o = b.pt(0.0, 0.0);
        let a = b.pt(6.0, 8.0);
        let line = b.line(o, a);
        let p = b.pt(2.0, 3.5);
        b.fix(o, 0.0, 0.0);
        b.fix(a, 6.0, 8.0);
        b.add(C::PointOnLine { point: p, line });
        b.dist(o, p, 5.0);
        cases.push(Case {
            name: "point_on_line_at_distance",
            sketch: b.s,
            points: vec![(p, [3.0, 4.0])],
            radii: vec![],
            dof: 0,
        });
    }

    // 23. A circle of radius 3 in the corner between the +x and +y axes
    // touches both: centre (3, 3). Unknowns 9. Three fixed points 6 + two
    // tangents 2 + radius 1 = 9: DOF 0.
    {
        let mut b = new();
        let o = b.pt(0.0, 0.0);
        let a = b.pt(10.0, 0.0);
        let u = b.pt(0.0, 10.0);
        let floor = b.line(o, a);
        let wall = b.line(o, u);
        let c = b.pt(2.5, 3.5);
        let circle = b.circle(c, 2.0);
        b.fix(o, 0.0, 0.0);
        b.fix(a, 10.0, 0.0);
        b.fix(u, 0.0, 10.0);
        b.add(C::Tangent {
            a: floor,
            b: circle,
        });
        b.add(C::Tangent { a: circle, b: wall });
        b.radius(circle, 3.0);
        cases.push(Case {
            name: "circle_in_corner",
            sketch: b.s,
            points: vec![(c, [3.0, 3.0])],
            radii: vec![(circle, 3.0)],
            dof: 0,
        });
    }

    // 24. A hole of diameter 6, 12 across and 7 up from the origin, drawn
    // on the left: the offsets keep their side, so the centre is (-12, 7).
    // Unknowns 5. Fixed 2 + two offsets 2 + diameter 1 = 5: DOF 0.
    {
        let mut b = new();
        let o = b.pt(0.0, 0.0);
        let c = b.pt(-11.0, 8.0);
        let hole = b.circle(c, 2.0);
        b.fix(o, 0.0, 0.0);
        b.hdist(o, c, 12.0);
        b.vdist(o, c, 7.0);
        b.diameter(hole, 6.0);
        cases.push(Case {
            name: "hole_position_keeps_side",
            sketch: b.s,
            points: vec![(c, [-12.0, 7.0])],
            radii: vec![(hole, 3.0)],
            dof: 0,
        });
    }

    // 25. A quarter arc of radius 5 around the origin from the +x axis to
    // the +y axis: (5, 0) to (0, 5). Unknowns 6. Arc 1 + fixed 2 + start
    // level with the centre 1 + radius 1 + end above the centre 1 = 6:
    // DOF 0.
    {
        let mut b = new();
        let c = b.pt(0.0, 0.0);
        let s = b.pt(4.0, 0.5);
        let e = b.pt(0.3, 4.0);
        let arc = b.arc(c, s, e);
        b.fix(c, 0.0, 0.0);
        b.add(C::HorizontalPoints { a: c, b: s });
        b.radius(arc, 5.0);
        b.add(C::VerticalPoints { a: c, b: e });
        cases.push(Case {
            name: "quarter_arc",
            sketch: b.s,
            points: vec![(s, [5.0, 0.0]), (e, [0.0, 5.0])],
            radii: vec![(arc, 5.0)],
            dof: 0,
        });
    }

    // 26. Two arcs joined tangentially at j. The first, radius 5 around
    // the origin, runs from (0, -5) to j = (5, 0). The second, radius 3,
    // starts at j and curves the other way, so its centre is on the far
    // side of j on the line through the first centre: (8, 0); it ends
    // straight below its centre at (8, -3). Unknowns 10 (c1, s1, j, c2,
    // e2). Two arcs 2 + fixed 2 + j level with c1 1 + radius 1 + s1 below
    // c1 1 + tangent 1 + radius 1 + e2 below c2 1 = 10: DOF 0.
    {
        let mut b = new();
        let c1 = b.pt(0.0, 0.0);
        let s1 = b.pt(0.2, -4.6);
        let j = b.pt(4.7, 0.3);
        let c2 = b.pt(7.5, 0.3);
        let e2 = b.pt(8.3, -2.5);
        let first = b.arc(c1, s1, j);
        let second = b.arc(c2, j, e2);
        b.fix(c1, 0.0, 0.0);
        b.add(C::HorizontalPoints { a: c1, b: j });
        b.radius(first, 5.0);
        b.add(C::VerticalPoints { a: c1, b: s1 });
        b.add(C::Tangent {
            a: first,
            b: second,
        });
        b.radius(second, 3.0);
        b.add(C::VerticalPoints { a: c2, b: e2 });
        cases.push(Case {
            name: "tangent_arcs_joined",
            sketch: b.s,
            points: vec![
                (s1, [0.0, -5.0]),
                (j, [5.0, 0.0]),
                (c2, [8.0, 0.0]),
                (e2, [8.0, -3.0]),
            ],
            radii: vec![(first, 5.0), (second, 3.0)],
            dof: 0,
        });
    }

    cases
}
