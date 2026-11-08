//! The second set of documents of `tests/step_freecad.rs` (oracle 8,
//! docs/testing.md): what was built on 2026-10-13 and had only been
//! checked by our own oracles. Extrusions that end on a face (an inclined
//! plane, a cylinder, a sphere from outside and from inside, a torus, a
//! freeform wall) and through all; sketches on datum planes (at a
//! distance, at an angle, through three points), revolved about a datum
//! axis, mirrored in a datum plane and patterned about a datum axis;
//! spline profiles with a hole, a spline wall cut off by a roller, nine
//! spline blades on a hub; booleans that leave slivers ten tolerances
//! thick; and bodies a hundred thousand times smaller than their distance
//! from the origin. All at the scales 1e-3, 1 and 1e6.
//!
//! Three things are compared per body, not per file (a small body next to
//! a large one would be measured against the large one's size):
//!
//! 1. ours (`document.inspect`) against a closed form, or an integral
//!    written out here, wherever there is one ([`Closed`]);
//! 2. FreeCAD's reading of the STEP file against ours (the harness);
//! 3. FreeCAD's reading of the STL and 3MF files against ours
//!    ([`check_meshes`]).
//!
//! Where FreeCAD cannot judge, the case is in [`BEYOND_THE_READER`] with
//! what was measured; nothing else may disagree.

// `!(x <= bound)` on purpose: a NaN is a problem too.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::harness::{self, Extent, MeshFile, RELATIVE};
use super::step;
use super::{APPROX, EXACT, Expect, FLAT, RING, SPLINE, SPLINE_QUADRATURE, Set};
use craft3d_engine::Session;
use craft3d_math::Portable;
use serde_json::{Value, json};
use std::f64::consts::{FRAC_1_SQRT_2, PI};
use std::fmt::Write as _;
use std::path::PathBuf;

/// A mesh file of a document and what we say is in it, in the file's
/// unit.
pub struct MeshCase {
    pub file: String,
    pub bytes: Vec<u8>,
    pub bodies: usize,
    /// How many of the bodies' meshes the exporter calls watertight.
    pub closed: usize,
    pub triangles: usize,
    /// The chordal tolerance the exporter used.
    pub tolerance: f64,
    /// Of the exact bodies.
    pub volume: f64,
    pub area: f64,
    pub bbox: Option<[f64; 6]>,
    /// The chordal tolerance of the mesh `bbox` is the box of.
    pub bbox_slack: f64,
}

/// What a body must measure by a formula that owes nothing to the kernel,
/// for lengths of 1 (scaled by the test).
struct Closed {
    body: &'static str,
    volume: f64,
    area: f64,
    centroid: Option<[f64; 3]>,
    /// Relative agreement asked of ours.
    relative: f64,
    /// `false` where only the volume has a formula here.
    with_area: bool,
}

fn closed(body: &'static str, volume: f64, area: f64) -> Closed {
    Closed {
        body,
        volume,
        area,
        centroid: None,
        relative: 1e-9,
        with_area: true,
    }
}

impl Closed {
    fn at(mut self, centroid: [f64; 3]) -> Self {
        self.centroid = Some(centroid);
        self
    }
    fn within(mut self, relative: f64) -> Self {
        self.relative = relative;
        self
    }
    fn volume_only(mut self) -> Self {
        self.with_area = false;
        self
    }
}

type Forms = Result<Vec<Closed>, String>;

fn sketch(s: &mut Session, name: &str, plane: Value, offset: f64) -> Result<(), String> {
    step(
        s,
        "sketch.create",
        json!({"name": name, "plane": plane, "offset": offset}),
    )
    .map(drop)
}

fn circle(s: &mut Session, sketch: &str, center: [f64; 2], radius: f64) -> Result<(), String> {
    step(
        s,
        "sketch.circle",
        json!({"sketch": sketch, "center": center, "radius": radius}),
    )
    .map(drop)
}

fn polygon(s: &mut Session, sketch: &str, k: f64, points: &[[f64; 2]]) -> Result<(), String> {
    let scaled: Vec<[f64; 2]> = points.iter().map(|p| [p[0] * k, p[1] * k]).collect();
    step(
        s,
        "sketch.polyline",
        json!({"sketch": sketch, "closed": true, "points": scaled}),
    )
    .map(drop)
}

fn rectangle(s: &mut Session, sketch: &str, k: f64, x: (f64, f64), y: (f64, f64)) -> Built {
    polygon(
        s,
        sketch,
        k,
        &[[x.0, y.0], [x.1, y.0], [x.1, y.1], [x.0, y.1]],
    )
}

type Built = Result<(), String>;

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: [f64; 3], f: f64) -> [f64; 3] {
    [a[0] * f, a[1] * f, a[2] * f]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: [f64; 3]) -> [f64; 3] {
    mul(a, 1.0 / dot(a, a).sqrt())
}

/// The integral of `f` over [0, 1] by five-point Gauss-Legendre on 64
/// pieces: for the smooth integrands here, to rounding.
fn integral(f: impl Fn(f64) -> f64) -> f64 {
    const GAUSS: [(f64, f64); 5] = [
        (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
        (-0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
        (0.0, 0.568_888_888_888_888_9),
        (0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
        (0.906_179_845_938_664, 0.236_926_885_056_189_1),
    ];
    let pieces = 64;
    let half = 0.5 / pieces as f64;
    (0..pieces)
        .map(|i| {
            let mid = (2 * i + 1) as f64 * half;
            GAUSS
                .iter()
                .map(|(x, w)| w * half * f(mid + half * x))
                .sum::<f64>()
        })
        .sum()
}

/// The integral of `f(x, y)` over the disc of radius `b` about `centre`,
/// in polar coordinates about its centre.
fn over_disc(centre: [f64; 2], b: f64, f: impl Fn(f64, f64) -> f64) -> f64 {
    integral(|u| {
        let (sin, cos) = (2.0 * PI * u).psin_cos();
        2.0 * PI
            * integral(|v| {
                let rho = b * v;
                f(centre[0] + rho * cos, centre[1] + rho * sin) * rho * b
            })
    })
}

/// A round peg of radius `b` about `centre` on the ground, up to a
/// surface at height `z(x, y)` over it whose area is `stretch(x, y)` per
/// unit of ground: volume and area by integrals over the disc and round
/// its rim.
fn peg_under(
    centre: [f64; 2],
    b: f64,
    z: impl Fn(f64, f64) -> f64,
    stretch: impl Fn(f64, f64) -> f64,
) -> (f64, f64) {
    let wall = integral(|u| {
        let (sin, cos) = (2.0 * PI * u).psin_cos();
        2.0 * PI * b * z(centre[0] + b * cos, centre[1] + b * sin)
    });
    (
        over_disc(centre, b, &z),
        PI * b * b + wall + over_disc(centre, b, stretch),
    )
}

// ---------------------------------------------------------------------
// Extrusions that end on a face, or go through all.

/// A round post from the ground up to the tilted floor of a disc: an
/// obliquely cut cylinder (the closed forms of `tests/extrude_ends.rs`).
fn up_to_an_inclined_plane(s: &mut Session, k: f64) -> Forms {
    let (a, h, tilt) = (2.0, 7.0, 0.4_f64);
    step(
        s,
        "feature.cylinder",
        json!({"radius": 20.0 * k, "height": 3.0 * k, "position": [0, 0, h * k],
               "axis": [tilt.psin(), 0, tilt.pcos()], "name": "Roof"}),
    )?;
    sketch(s, "Round", json!("xy"), 0.0)?;
    circle(s, "Round", [0.0, 0.0], a * k)?;
    // The floor of a cylinder is its face 1.
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Round", "upTo": "F1:face(1)", "name": "Post"}),
    )?;
    Ok(vec![
        closed("Roof", PI * 400.0 * 3.0, 2.0 * PI * 20.0 * 23.0),
        closed(
            "Post",
            PI * a * a * h,
            PI * a * a + 2.0 * PI * a * h + PI * a * a / tilt.pcos(),
        ),
    ])
}

/// A rectangle from the ground up to the wall of a lying pipe: a saddle.
fn up_to_a_cylinder(s: &mut Session, k: f64) -> Forms {
    let (a, l, r, h) = (1.5, 4.0, 2.5, 6.0);
    step(
        s,
        "feature.cylinder",
        json!({"radius": r * k, "height": 10.0 * k, "position": [0, -3.0 * k, h * k],
               "axis": [0, 1, 0], "name": "Pipe"}),
    )?;
    sketch(s, "Foot", json!("xy"), 0.0)?;
    rectangle(s, "Foot", k, (-a, a), (0.0, l))?;
    // The wall of a cylinder is its face 0.
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Foot", "upTo": "F1:face(0)", "name": "Saddle"}),
    )?;
    let drop = (r * r - a * a).sqrt();
    let front = 2.0 * a * h - a * drop - r * r * (a / r).pasin();
    Ok(vec![
        closed("Pipe", PI * r * r * 10.0, 2.0 * PI * r * (r + 10.0)),
        closed(
            "Saddle",
            l * front,
            2.0 * a * l + 2.0 * front + 2.0 * l * (h - drop) + 2.0 * r * (a / r).pasin() * l,
        ),
    ])
}

/// A round peg up to a ball, from a plane below it and from a plane
/// inside it. With b the peg's radius and R the ball's, the ball's skin
/// over the peg lies √(R² − ρ²) from the plane through its centre, which
/// over the disc sums to 2π/3 (R³ − (R² − b²)^(3/2)): taken off the
/// cylinder up to that plane from outside, added to it from inside.
fn up_to_a_sphere(s: &mut Session, k: f64) -> Forms {
    let (b, rr) = (1.5_f64, 4.0_f64);
    step(
        s,
        "feature.sphere",
        json!({"radius": rr * k, "position": [0, 0, 9.0 * k], "name": "Ball"}),
    )?;
    let fall = (rr * rr - b * b).sqrt();
    let under_skin = 2.0 * PI / 3.0 * (rr.ppowi(3) - fall.ppowi(3));
    let cap_area = 2.0 * PI * rr * (rr - fall);
    let mut forms = vec![closed(
        "Ball",
        4.0 / 3.0 * PI * rr.ppowi(3),
        4.0 * PI * rr * rr,
    )];
    for (name, peg, offset, inside) in [
        ("Below", "Below peg", 0.0, false),
        ("Within", "Within peg", 8.0, true),
    ] {
        sketch(s, name, json!("xy"), offset * k)?;
        circle(s, name, [0.0, 0.0], b * k)?;
        step(
            s,
            "feature.extrude",
            json!({"sketch": name, "upTo": "F1:face(0)", "name": peg}),
        )?;
        // `height`: of the peg's wall, to the circle where it meets the
        // ball.
        let h = 9.0 - offset;
        let (volume, height) = if inside {
            (PI * b * b * h + under_skin, h + fall)
        } else {
            (PI * b * b * h - under_skin, h - fall)
        };
        forms.push(closed(
            peg,
            volume,
            PI * b * b + 2.0 * PI * b * height + cap_area,
        ));
    }
    Ok(forms)
}

/// A peg beside the ball's axis: its end is bounded by a curve that is no
/// circle about the axis. The ball's underside is at
/// z = 9 − √(16 − x² − y²), with 4 / √(16 − x² − y²) of skin per unit of
/// ground: integrated over the peg's disc. Ours is held to 1e-7 of that,
/// as `tests/extrude_ends.rs` holds its freeform end: the rim of the end
/// is a spline within the modelling tolerance (1e-6 of the scale) of the
/// exact curve, not that curve (measured: 3.6e-9 and 5.4e-9, alike at
/// the three scales).
fn up_to_a_sphere_off_its_axis(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "feature.sphere",
        json!({"radius": 4.0 * k, "position": [0, 0, 9.0 * k], "name": "Ball"}),
    )?;
    sketch(s, "Aside", json!("xy"), 0.0)?;
    circle(s, "Aside", [1.75 * k, 0.5 * k], 1.5 * k)?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Aside", "upTo": "F1:face(0)", "name": "Peg"}),
    )?;
    let root = |x: f64, y: f64| (16.0 - x * x - y * y).sqrt();
    let (volume, area) = peg_under(
        [1.75, 0.5],
        1.5,
        |x, y| 9.0 - root(x, y),
        |x, y| 4.0 / root(x, y),
    );
    Ok(vec![closed("Peg", volume, area).within(1e-7)])
}

/// A sleeve (a ring between two circles) from the ground up to the
/// underside of a torus lying above it, about the same axis. With R and r
/// the torus's radii, a the half width of the sleeve and h the height of
/// the torus's middle: the end is the zone of the torus within a of its
/// lowest circle,
/// V = 2π R (2 a h − a √(r² − a²) − r² asin(a / r)).
fn up_to_a_torus(s: &mut Session, k: f64) -> Forms {
    let (big, small, a, h) = (10.0_f64, 3.0_f64, 2.0_f64, 9.0_f64);
    step(
        s,
        "feature.torus",
        json!({"major": big * k, "minor": small * k, "position": [0, 0, h * k], "name": "Ring"}),
    )?;
    sketch(s, "Band", json!("xy"), 0.0)?;
    circle(s, "Band", [0.0, 0.0], (big + a) * k)?;
    circle(s, "Band", [0.0, 0.0], (big - a) * k)?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Band", "upTo": "F1:face(0)", "name": "Sleeve"}),
    )?;
    let drop = (small * small - a * a).sqrt();
    let turn = (a / small).pasin();
    Ok(vec![
        closed(
            "Ring",
            2.0 * PI * PI * big * small * small,
            4.0 * PI * PI * big * small,
        ),
        closed(
            "Sleeve",
            2.0 * PI * big * (2.0 * a * h - a * drop - small * small * turn),
            4.0 * PI * big * a + 4.0 * PI * big * (h - drop) + 4.0 * PI * big * small * turn,
        ),
    ])
}

/// A round peg up to the torus, off its lowest circle: the end is bounded
/// by a curve that is no circle of the torus. With s = √(x² + y²) − 10
/// the torus's underside is at z = 9 − √(9 − s²), with 3 / √(9 − s²) of
/// skin per unit of ground: integrated over the peg's disc, and held to
/// 1e-7 as for the ball (measured: 6.8e-10 and 7.6e-10).
fn up_to_a_torus_off_its_circle(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "feature.torus",
        json!({"major": 10.0 * k, "minor": 3.0 * k, "position": [0, 0, 9.0 * k], "name": "Ring"}),
    )?;
    sketch(s, "Round", json!("xy"), 0.0)?;
    circle(s, "Round", [10.5 * k, k], 1.5 * k)?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Round", "upTo": "F1:face(0)", "name": "Peg"}),
    )?;
    let root = |x: f64, y: f64| (9.0 - ((x * x + y * y).sqrt() - 10.0).ppowi(2)).sqrt();
    let (volume, area) = peg_under(
        [10.5, 1.0],
        1.5,
        |x, y| 9.0 - root(x, y),
        |x, y| 3.0 / root(x, y),
    );
    Ok(vec![closed("Peg", volume, area).within(1e-7)])
}

/// A bar up to the underside of an arch drawn as a parabola (a B-spline
/// of degree 2): the closed forms of `tests/extrude_ends.rs`, by the
/// primitive of the parabola and its arc length.
fn up_to_a_freeform_face(s: &mut Session, k: f64) -> Forms {
    // y(x) = 4 + 8 t (1 - t), t = (x + 6) / 12.
    sketch(s, "Wave", json!("xy"), 0.0)?;
    let spline = step(
        s,
        "sketch.spline",
        json!({"sketch": "Wave", "degree": 2,
               "control": [[-6.0 * k, 4.0 * k], [0.0, 8.0 * k], [6.0 * k, 4.0 * k]]}),
    )?;
    let curve = spline["entity"].as_str().unwrap().to_string();
    step(
        s,
        "sketch.polyline",
        json!({"sketch": "Wave",
               "points": [spline["end"], [6.0 * k, 12.0 * k], [-6.0 * k, 12.0 * k], spline["start"]]}),
    )?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Wave", "distance": 5.0 * k, "name": "Arch"}),
    )?;
    // On zx the sketch's x is the world's z and its y the world's x.
    let (x0, x1, z0, z1) = (-2.0, 3.0, 1.0, 4.0);
    sketch(s, "Bar", json!("zx"), 0.0)?;
    rectangle(s, "Bar", k, (z0, z1), (x0, x1))?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Bar", "upTo": format!("F2:side({curve})"), "name": "Prop"}),
    )?;
    let t = |x: f64| (x + 6.0) / 12.0;
    let y = |x: f64| 4.0 + 8.0 * t(x) * (1.0 - t(x));
    let primitive = |x: f64| 12.0 * (4.0 * t(x) + 4.0 * t(x).ppowi(2) - 8.0 / 3.0 * t(x).ppowi(3));
    let slope = |x: f64| (2.0 - 4.0 * t(x)) / 3.0;
    let arc = |s: f64| (s * (1.0 + s * s).sqrt() + s.pasinh()) / 2.0;
    let under = primitive(x1) - primitive(x0);
    let length = 9.0 * (arc(slope(x0)) - arc(slope(x1)));
    let whole_under = primitive(6.0) - primitive(-6.0);
    let whole_length = 9.0 * (arc(slope(-6.0)) - arc(slope(6.0)));
    let height = z1 - z0;
    Ok(vec![
        // The arch: 12 wide, from the parabola up to y = 12, 5 deep.
        closed(
            "Arch",
            (144.0 - whole_under) * 5.0,
            2.0 * (144.0 - whole_under) + 5.0 * (whole_length + 8.0 + 12.0 + 8.0),
        ),
        closed(
            "Prop",
            height * under,
            (x1 - x0) * height + height * length + 2.0 * under + height * (y(x0) + y(x1)),
        )
        .within(1e-7),
    ])
}

/// Two holes drilled through a plate with `throughAll` and `cut`.
fn through_all_cut(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "feature.block",
        json!({"size": [40.0 * k, 30.0 * k, 8.0 * k], "name": "Plate"}),
    )?;
    sketch(s, "Holes", json!("xy"), 8.0 * k)?;
    circle(s, "Holes", [10.0 * k, 10.0 * k], 3.0 * k)?;
    circle(s, "Holes", [25.0 * k, 15.0 * k], 4.0 * k)?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Holes", "throughAll": true, "reversed": true, "cut": "Plate",
               "name": "Drilled"}),
    )?;
    Ok(vec![
        closed(
            "Drilled",
            9600.0 - PI * 25.0 * 8.0,
            2.0 * (1200.0 + 320.0 + 240.0) - 2.0 * PI * 25.0 + 2.0 * PI * 7.0 * 8.0,
        )
        .at([
            (9600.0 * 20.0 - PI * 8.0 * (9.0 * 10.0 + 16.0 * 25.0)) / (9600.0 - PI * 200.0),
            (9600.0 * 15.0 - PI * 8.0 * (9.0 * 10.0 + 16.0 * 15.0)) / (9600.0 - PI * 200.0),
            4.0,
        ]),
    ])
}

/// A pin through a block above and a disc below the sketch plane, both
/// ways at once: it ends 1 % of the way past the far side of each (as
/// `tests/extrude_ends.rs` pins), from -9.09 to 10.1.
fn through_all_both_ways(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "feature.block",
        json!({"size": [20.0 * k, 20.0 * k, 4.0 * k], "position": [-10.0 * k, -10.0 * k, 6.0 * k],
               "name": "Upper"}),
    )?;
    step(
        s,
        "feature.cylinder",
        json!({"radius": 8.0 * k, "height": 3.0 * k, "position": [0, 0, -9.0 * k], "name": "Lower"}),
    )?;
    sketch(s, "Pin", json!("xy"), 0.0)?;
    circle(s, "Pin", [0.0, 0.0], k)?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Pin", "throughAll": true, "symmetric": true, "name": "Rod"}),
    )?;
    Ok(vec![
        closed("Rod", PI * 19.19, 2.0 * PI + 2.0 * PI * 19.19)
            .at([0.0, 0.0, (10.1 - 9.09) / 2.0])
            .within(1e-6),
    ])
}

// ---------------------------------------------------------------------
// Datums.

/// A ring extruded from a sketch on a datum plane 9 above the ground.
fn datum_plane_at_a_distance(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "feature.block",
        json!({"size": [30.0 * k, 20.0 * k, 5.0 * k], "name": "Base"}),
    )?;
    step(
        s,
        "datum.plane",
        json!({"from": "F1:face(1)", "distance": 1.5 * k, "name": "Shelf"}),
    )?;
    step(
        s,
        "datum.plane",
        json!({"from": "xy", "distance": 9.0 * k, "name": "Level"}),
    )?;
    sketch(s, "On level", json!("Level"), 0.0)?;
    circle(s, "On level", [12.0 * k, 9.0 * k], 5.0 * k)?;
    circle(s, "On level", [12.0 * k, 9.0 * k], 2.0 * k)?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "On level", "distance": 6.0 * k, "name": "Ring"}),
    )?;
    // On a plane placed by a face: only the height is known here.
    sketch(s, "On shelf", json!("Shelf"), 0.0)?;
    circle(s, "On shelf", [3.0 * k, 4.0 * k], 1.5 * k)?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "On shelf", "distance": 2.0 * k, "name": "Knob"}),
    )?;
    Ok(vec![
        closed("Base", 3000.0, 2.0 * (600.0 + 150.0 + 100.0)).at([15.0, 10.0, 2.5]),
        closed(
            "Ring",
            PI * 21.0 * 6.0,
            2.0 * PI * 21.0 + 2.0 * PI * 7.0 * 6.0,
        )
        .at([12.0, 9.0, 12.0]),
        closed(
            "Knob",
            PI * 2.25 * 2.0,
            2.0 * PI * 2.25 + 2.0 * PI * 1.5 * 2.0,
        ),
    ])
}

const TILT: f64 = 35.0;

/// The xy plane turned about x by `TILT` degrees, counter-clockwise seen
/// from the tip of x: its x axis, y axis and normal.
fn ramp() -> ([f64; 3], [f64; 3], [f64; 3]) {
    let (sin, cos) = TILT.to_radians().psin_cos();
    ([1.0, 0.0, 0.0], [0.0, cos, sin], [0.0, -sin, cos])
}

/// A box on a datum plane at an angle: flat faces only, so FreeCAD's
/// bounding box must be ours as well.
fn datum_plane_at_an_angle(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "datum.plane",
        json!({"from": "xy", "about": "x", "angle": format!("{TILT} deg"), "name": "Ramp"}),
    )?;
    sketch(s, "On ramp", json!("Ramp"), 0.0)?;
    rectangle(s, "On ramp", k, (1.0, 5.0), (2.0, 5.0))?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "On ramp", "distance": 5.0 * k, "name": "Wedge"}),
    )?;
    let (x, y, n) = ramp();
    Ok(vec![
        closed("Wedge", 60.0, 94.0).at(add(add(mul(x, 3.0), mul(y, 3.5)), mul(n, 2.5))),
    ])
}

/// A knob on a plane hinged on an edge of a block (the edge between its
/// top and its right side) at 45 degrees.
fn datum_plane_about_an_edge(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "feature.block",
        json!({"size": [8.0 * k, 6.0 * k, 3.0 * k], "name": "Base"}),
    )?;
    step(
        s,
        "datum.plane",
        json!({"from": "F1:face(1)", "about": ["F1:face(1)", "F1:face(5)"], "angle": "45 deg",
               "name": "Lid"}),
    )?;
    sketch(s, "On lid", json!("Lid"), 0.0)?;
    circle(s, "On lid", [k, k], 0.8 * k)?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "On lid", "distance": 2.0 * k, "name": "Knob"}),
    )?;
    Ok(vec![
        closed("Base", 144.0, 2.0 * (48.0 + 24.0 + 18.0)),
        closed(
            "Knob",
            PI * 0.64 * 2.0,
            2.0 * PI * 0.64 + 2.0 * PI * 0.8 * 2.0,
        ),
    ])
}

const SLANT: [[f64; 3]; 3] = [[6.0, 0.0, 0.0], [0.0, 4.0, 0.0], [0.0, 0.0, 3.0]];

/// The plane through the three points of `SLANT`: the origin is the first,
/// x points at the second and the three go counter-clockwise round the
/// normal.
fn slant() -> ([f64; 3], [f64; 3], [f64; 3]) {
    let x = unit(sub(SLANT[1], SLANT[0]));
    let n = unit(cross(sub(SLANT[1], SLANT[0]), sub(SLANT[2], SLANT[0])));
    (x, cross(n, x), n)
}

fn slant_plane(s: &mut Session, k: f64) -> Built {
    step(
        s,
        "datum.plane",
        json!({"through": SLANT.map(|p| mul(p, k)), "name": "Slant"}),
    )
    .map(drop)
}

/// A tube on a datum plane through three points.
fn datum_plane_through_three_points(s: &mut Session, k: f64) -> Forms {
    slant_plane(s, k)?;
    sketch(s, "On slant", json!("Slant"), 0.0)?;
    circle(s, "On slant", [2.0 * k, k], 3.0 * k)?;
    circle(s, "On slant", [2.0 * k, k], k)?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "On slant", "distance": 2.5 * k, "name": "Tube"}),
    )?;
    let (x, y, n) = slant();
    Ok(vec![
        closed(
            "Tube",
            PI * 8.0 * 2.5,
            2.0 * PI * 8.0 + 2.0 * PI * 4.0 * 2.5,
        )
        .at(add(SLANT[0], add(add(mul(x, 2.0), y), mul(n, 1.25)))),
    ])
}

/// A rectangle on the tilted plane turned about a datum axis that lies in
/// that plane (where it meets a plane at x = -2), by a third of a turn
/// or all the way round: Pappus.
fn revolve_about_a_datum_axis(whole: bool) -> impl Fn(&mut Session, f64) -> Forms {
    move |s, k| revolved_about_a_datum_axis(s, k, whole)
}

fn revolved_about_a_datum_axis(s: &mut Session, k: f64, whole: bool) -> Forms {
    step(
        s,
        "datum.plane",
        json!({"from": "xy", "about": "x", "angle": format!("{TILT} deg"), "name": "Ramp"}),
    )?;
    step(
        s,
        "datum.plane",
        json!({"from": "yz", "distance": -2.0 * k, "name": "Side"}),
    )?;
    step(
        s,
        "datum.axis",
        json!({"planes": ["Ramp", "Side"], "name": "Hinge"}),
    )?;
    sketch(s, "Profile", json!("Ramp"), 0.0)?;
    rectangle(s, "Profile", k, (1.0, 5.0), (2.0, 5.0))?;
    let angle = if whole { "360 deg" } else { "120 deg" };
    step(
        s,
        "feature.revolve",
        json!({"sketch": "Profile", "axis": "Hinge", "angle": angle, "name": "Turned"}),
    )?;
    // The rectangle: 4 by 3, its middle 5 from the axis; its sides at 3,
    // 5, 5 and 7 from it.
    let (area, swept) = (12.0, 3.0 * 3.0 + 3.0 * 7.0 + 2.0 * 4.0 * 5.0);
    let third = 2.0 * PI / 3.0;
    let (_, y, _) = ramp();
    Ok(vec![if whole {
        closed("Turned", 2.0 * PI * 5.0 * area, 2.0 * PI * swept)
            .at(add([-2.0, 0.0, 0.0], mul(y, 3.5)))
    } else {
        closed("Turned", third * 5.0 * area, third * swept + 2.0 * area)
    }])
}

/// A circle on the plane through three points turned about the line
/// through two of them: a torus about an axis askew to everything.
fn revolve_a_circle_about_a_datum_axis(s: &mut Session, k: f64) -> Forms {
    slant_plane(s, k)?;
    step(
        s,
        "datum.axis",
        json!({"through": [mul(SLANT[0], k), mul(SLANT[1], k)], "name": "Edge"}),
    )?;
    sketch(s, "Section", json!("Slant"), 0.0)?;
    circle(s, "Section", [3.0 * k, 4.0 * k], 1.5 * k)?;
    step(
        s,
        "feature.revolve",
        json!({"sketch": "Section", "axis": "Edge", "name": "Hoop"}),
    )?;
    let (x, _, _) = slant();
    Ok(vec![
        closed(
            "Hoop",
            2.0 * PI * PI * 4.0 * 2.25,
            4.0 * PI * PI * 4.0 * 1.5,
        )
        .at(add(SLANT[0], mul(x, 3.0))),
    ])
}

/// An L mirrored in a datum plane through the z axis, 20 degrees from
/// yz, the original kept: the mirror image of the centre of mass.
fn mirror_in_a_datum_plane(s: &mut Session, k: f64) -> Forms {
    sketch(s, "Ell", json!("xy"), 0.0)?;
    polygon(
        s,
        "Ell",
        k,
        &[
            [2.0, 1.0],
            [8.0, 1.0],
            [8.0, 3.0],
            [4.0, 3.0],
            [4.0, 7.0],
            [2.0, 7.0],
        ],
    )?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Ell", "distance": 4.0 * k, "name": "Half"}),
    )?;
    step(
        s,
        "datum.plane",
        json!({"from": "yz", "about": "z", "angle": "20 deg", "name": "Mid"}),
    )?;
    step(
        s,
        "feature.mirror",
        json!({"source": "Half", "plane": "Mid", "keep": true, "name": "Other"}),
    )?;
    // Two rectangles: 6 by 2 about (5, 2) and 2 by 4 about (3, 5).
    let centre = [
        (12.0 * 5.0 + 8.0 * 3.0) / 20.0,
        (12.0 * 2.0 + 8.0 * 5.0) / 20.0,
        2.0,
    ];
    let (sin, cos) = 20.0_f64.to_radians().psin_cos();
    let n = [cos, sin, 0.0];
    let image = sub(centre, mul(n, 2.0 * dot(centre, n)));
    let (volume, area) = (80.0, 2.0 * 20.0 + 24.0 * 4.0);
    Ok(vec![
        closed("Half", volume, area).at(centre),
        closed("Other", volume, area).at(image),
    ])
}

/// A block mirrored in a datum plane on its own left face and joined
/// with its image: one block twice as long.
fn mirror_in_a_face_and_join(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "feature.block",
        json!({"size": [10.0 * k, 6.0 * k, 4.0 * k], "name": "Half"}),
    )?;
    step(
        s,
        "datum.plane",
        json!({"from": "F1:face(4)", "distance": 0, "name": "Mid"}),
    )?;
    step(
        s,
        "feature.mirror",
        json!({"source": "Half", "plane": "Mid", "keep": true, "name": "Other"}),
    )?;
    step(
        s,
        "feature.boolean",
        json!({"op": "union", "target": "Half", "tool": "Other", "name": "Whole"}),
    )?;
    Ok(vec![
        closed("Whole", 480.0, 2.0 * (120.0 + 80.0 + 24.0)).at([0.0, 3.0, 2.0]),
    ])
}

/// Five blocks round the axis of a tilted cylinder (a datum axis taken
/// from its wall): their common centre of mass lies on that axis.
fn pattern_about_a_datum_axis(s: &mut Session, k: f64) -> Forms {
    let (at, along) = ([7.0, -3.0, 5.0], [0.3, -0.4, 1.0]);
    step(
        s,
        "feature.cylinder",
        json!({"radius": 6.0 * k, "height": 20.0 * k, "position": mul(at, k), "axis": along,
               "name": "Hub"}),
    )?;
    step(
        s,
        "datum.axis",
        json!({"face": "F1:face(0)", "name": "Spin"}),
    )?;
    step(
        s,
        "feature.block",
        json!({"size": [3.0 * k, 2.0 * k, 5.0 * k], "position": [20.0 * k, -k, 0], "name": "Pin"}),
    )?;
    step(
        s,
        "feature.pattern",
        json!({"source": "Pin", "count": 5, "angle": "72 deg", "axis": "Spin", "name": "Pins"}),
    )?;
    let d = unit(along);
    let centre = add(at, mul(d, dot(sub([21.5, 0.0, 2.5], at), d)));
    Ok(vec![
        closed("Hub", PI * 36.0 * 20.0, 2.0 * PI * 6.0 * 26.0),
        closed("Pins", 5.0 * 30.0, 5.0 * 62.0).at(centre),
    ])
}

/// Three balls round an axis through two datum points, one at
/// coordinates and one on a vertex of a block.
fn pattern_about_an_axis_through_datum_points(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "feature.block",
        json!({"size": [4.0 * k, 4.0 * k, 2.0 * k], "name": "Seat"}),
    )?;
    step(
        s,
        "datum.point",
        json!({"at": [0, 0, 10.0 * k], "name": "High"}),
    )?;
    // Vertex 7 of a block: high x, high y, high z.
    step(
        s,
        "datum.point",
        json!({"vertex": "F1:vertex(7)", "name": "Corner"}),
    )?;
    step(
        s,
        "datum.axis",
        json!({"through": ["High", "Corner"], "name": "Lean"}),
    )?;
    step(
        s,
        "feature.sphere",
        json!({"radius": 1.5 * k, "position": [9.0 * k, 0, 0], "name": "Ball"}),
    )?;
    step(
        s,
        "feature.pattern",
        json!({"source": "Ball", "count": 3, "angle": "120 deg", "axis": "Lean", "name": "Balls"}),
    )?;
    let (at, d) = ([0.0, 0.0, 10.0], unit([4.0, 4.0, -8.0]));
    let centre = add(at, mul(d, dot(sub([9.0, 0.0, 0.0], at), d)));
    Ok(vec![
        closed(
            "Balls",
            3.0 * 4.0 / 3.0 * PI * 1.5_f64.ppowi(3),
            3.0 * 4.0 * PI * 2.25,
        )
        .at(centre),
    ])
}

// ---------------------------------------------------------------------
// Spline profiles.

/// A D (a spline and a line) with a round hole, extruded: every curved
/// face is a whole patch, so the report script's quadrature measures what
/// FreeCAD imported.
fn spline_d_with_a_hole(s: &mut Session, k: f64) -> Forms {
    sketch(s, "Profile", json!("zx"), 0.0)?;
    let p = |x: f64, y: f64| json!([x * k, y * k]);
    step(
        s,
        "sketch.spline",
        json!({"sketch": "Profile",
               "control": [p(0.0, 0.0), p(5.0, -1.0), p(7.0, 3.0), p(5.0, 7.0), p(0.0, 6.0)]}),
    )?;
    step(
        s,
        "sketch.line",
        json!({"sketch": "Profile", "from": "e5", "to": "e1"}),
    )?;
    circle(s, "Profile", [2.5 * k, 3.0 * k], k)?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": 4.0 * k, "name": "Hull"}),
    )?;
    Ok(Vec::new())
}

/// The pillar of the showcase: a leaf (a cubic Bézier and its chord) from
/// the ground up to the wall of a roller lying across it. Its measures by
/// integrals along the Bézier, with the roller's underside written out:
/// z(y) = 45 − √(18² − (y − 25)²).
fn pillar(s: &mut Session, k: f64) -> Forms {
    sketch(s, "Base", json!("xy"), 0.0)?;
    rectangle(s, "Base", k, (0.0, 80.0), (0.0, 50.0))?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Base", "distance": 8.0 * k, "name": "Plate"}),
    )?;
    sketch(s, "RollerProfile", json!("yz"), 40.0 * k)?;
    circle(s, "RollerProfile", [25.0 * k, 45.0 * k], 18.0 * k)?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "RollerProfile", "distance": 80.0 * k, "symmetric": true,
               "name": "Roller"}),
    )?;
    sketch(s, "Leaf", json!("xy"), 0.0)?;
    let xs = [18.0, 30.0, 52.0, 64.0];
    step(
        s,
        "sketch.spline",
        json!({"sketch": "Leaf", "control": [[xs[0] * k, 25.0 * k], [xs[1] * k, 44.0 * k],
               [xs[2] * k, 44.0 * k], [xs[3] * k, 25.0 * k]]}),
    )?;
    step(
        s,
        "sketch.polyline",
        json!({"sketch": "Leaf", "points": ["e4", "e1"]}),
    )?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Leaf", "upTo": "F4:side(e2)", "name": "Pillar"}),
    )?;
    // The Bézier: x(t) by its Bernstein polynomials, and its height above
    // the chord w(t) = y(t) − 25 = 57 t (1 − t).
    let dx = |t: f64| {
        3.0 * ((xs[1] - xs[0]) * (1.0 - t).ppowi(2)
            + 2.0 * (xs[2] - xs[1]) * t * (1.0 - t)
            + (xs[3] - xs[2]) * t * t)
    };
    let w = |t: f64| 57.0 * t * (1.0 - t);
    let dw = |t: f64| 57.0 * (1.0 - 2.0 * t);
    let root = |w: f64| (324.0 - w * w).sqrt();
    // The volume over a strip of the leaf from the chord to w: the
    // integral of 45 − √(324 − w²).
    let column = |w: f64| 45.0 * w - 0.5 * (w * root(w) + 324.0 * (w / 18.0).pasin());
    let volume = integral(|t| column(w(t)) * dx(t));
    let floor = integral(|t| w(t) * dx(t));
    let front = (xs[3] - xs[0]) * (45.0 - 18.0);
    let wall = integral(|t| (45.0 - root(w(t))) * (dx(t).ppowi(2) + dw(t).ppowi(2)).sqrt());
    // The end, on the roller: 18 / √(324 − w²) per unit of floor.
    let end = integral(|t| 18.0 * (w(t) / 18.0).pasin() * dx(t));
    Ok(vec![
        closed("Plate", 32000.0, 2.0 * (4000.0 + 640.0 + 400.0)).at([40.0, 25.0, 4.0]),
        closed("Roller", PI * 324.0 * 80.0, 2.0 * PI * 18.0 * (18.0 + 80.0)).at([40.0, 25.0, 45.0]),
        closed("Pillar", volume, floor + front + wall + end),
    ])
}

/// The impeller of the showcase: nine blades between two cubic Béziers,
/// patterned about the axis and joined with a disc and a hub, bored.
fn impeller(s: &mut Session, k: f64) -> Forms {
    let p = |x: f64, y: f64| json!([x * k, y * k]);
    step(
        s,
        "feature.cylinder",
        json!({"radius": 60.0 * k, "height": 4.0 * k, "name": "Disc"}),
    )?;
    step(
        s,
        "feature.cylinder",
        json!({"radius": 14.0 * k, "height": 34.0 * k, "name": "Hub"}),
    )?;
    sketch(s, "BladeProfile", json!("xy"), 4.0 * k)?;
    step(
        s,
        "sketch.spline",
        json!({"sketch": "BladeProfile",
               "control": [p(11.0, -3.0), p(26.0, 16.0), p(46.0, 20.0), p(58.0, 4.0)]}),
    )?;
    step(
        s,
        "sketch.spline",
        json!({"sketch": "BladeProfile", "control": ["e4", p(45.0, 14.0), p(27.0, 10.0), "e1"]}),
    )?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "BladeProfile", "distance": 24.0 * k, "name": "Blade"}),
    )?;
    step(
        s,
        "feature.pattern",
        json!({"source": "Blade", "count": 9, "angle": "360 deg / 9", "name": "Blades"}),
    )?;
    step(
        s,
        "feature.boolean",
        json!({"op": "union", "target": "Disc", "tool": "Hub", "name": "Base"}),
    )?;
    step(
        s,
        "feature.boolean",
        json!({"op": "union", "target": "Base", "tool": "Blades", "name": "Wheel"}),
    )?;
    step(
        s,
        "feature.cylinder",
        json!({"radius": 5.0 * k, "height": 50.0 * k, "position": [0, 0, -8.0 * k], "name": "Bore"}),
    )?;
    step(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Wheel", "tool": "Bore", "name": "Impeller"}),
    )?;
    Ok(Vec::new())
}

/// A point of the B-spline of degree `degree` on `control` with the
/// clamped, evenly spaced knots `sketch.spline` gives its control points,
/// at `u` between 0 and the number of spans: de Boor's recurrence.
fn spline_point(control: &[[f64; 2]], degree: usize, u: f64) -> [f64; 2] {
    let spans = control.len() - degree;
    let knot = |i: usize| (i.saturating_sub(degree)).min(spans) as f64;
    let span = (u.floor() as usize).min(spans - 1) + degree;
    let mut d: Vec<[f64; 2]> = (0..=degree).map(|j| control[j + span - degree]).collect();
    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = j + span - degree;
            let a = (u - knot(i)) / (knot(i + degree - r + 1) - knot(i));
            d[j] = [
                (1.0 - a) * d[j - 1][0] + a * d[j][0],
                (1.0 - a) * d[j - 1][1] + a * d[j][1],
            ];
        }
    }
    d[degree]
}

/// The half outline of the housing's base: straight from (10, -35) by
/// the mirror plane's (0, ±35) to (10, 35), and back by a spline.
const BASE_SPLINE: [[f64; 2]; 7] = [
    [10.0, 35.0],
    [30.0, 35.0],
    [56.0, 40.0],
    [62.0, 0.0],
    [56.0, -40.0],
    [30.0, -35.0],
    [10.0, -35.0],
];

/// The area of the housing's whole base: twice the strip 10 wide by the
/// mirror plane and what the spline encloses beyond it, the integral of
/// (x − 10) dy along the spline, whose derivative is the spline of degree
/// 2 on the differences of its control points (over the knot spans they
/// bridge).
fn base_area() -> f64 {
    let spans = BASE_SPLINE.len() - 3;
    let knot = |i: usize| (i.saturating_sub(3)).min(spans) as f64;
    let hodograph: Vec<[f64; 2]> = (0..BASE_SPLINE.len() - 1)
        .map(|i| {
            let f = 3.0 / (knot(i + 4) - knot(i + 1));
            [
                f * (BASE_SPLINE[i + 1][0] - BASE_SPLINE[i][0]),
                f * (BASE_SPLINE[i + 1][1] - BASE_SPLINE[i][1]),
            ]
        })
        .collect();
    // Span by span: a polynomial on each. The spline runs from y = 35
    // down to y = -35, so dy is negative.
    let beyond: f64 = (0..spans)
        .map(|span| {
            -integral(|t| {
                let u = span as f64 + t;
                (spline_point(&BASE_SPLINE, 3, u)[0] - 10.0) * spline_point(&hodograph, 2, u)[1]
            })
        })
        .sum();
    2.0 * (10.0 * 70.0 + beyond)
}

fn housing_base(s: &mut Session, k: f64) -> Built {
    let p = |q: [f64; 2]| json!([q[0] * k, q[1] * k]);
    step(
        s,
        "datum.plane",
        json!({"from": "yz", "distance": 0, "name": "Mid"}),
    )?;
    sketch(s, "HalfOutline", json!("xy"), 0.0)?;
    step(
        s,
        "sketch.polyline",
        json!({"sketch": "HalfOutline",
               "points": [p([10.0, -35.0]), p([0.0, -35.0]), p([0.0, 35.0]), p([10.0, 35.0])]}),
    )?;
    let mut control: Vec<Value> = BASE_SPLINE.iter().map(|q| p(*q)).collect();
    control[0] = json!("e4");
    control[6] = json!("e1");
    step(
        s,
        "sketch.spline",
        json!({"sketch": "HalfOutline", "control": control}),
    )?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "HalfOutline", "distance": 8.0 * k, "name": "HalfBase"}),
    )?;
    step(
        s,
        "feature.mirror",
        json!({"source": "HalfBase", "plane": "Mid", "keep": true, "name": "OtherHalf"}),
    )?;
    step(
        s,
        "feature.boolean",
        json!({"op": "union", "target": "HalfBase", "tool": "OtherHalf", "name": "Base"}),
    )
    .map(drop)
}

/// The base of the showcase's bearing housing alone: a half outline with
/// a spline, extruded, mirrored in a datum plane and joined with its
/// image. Every curved face is a whole patch, so the report script's
/// quadrature measures what FreeCAD imported.
fn spline_housing_base(s: &mut Session, k: f64) -> Forms {
    housing_base(s, k)?;
    Ok(vec![closed("Base", 8.0 * base_area(), 0.0).volume_only()])
}

/// The bearing housing of the showcase (`docs/showcase/housing`): the
/// base above; a sleeve lying over it; a pedestal between two Béziers
/// from the base up to the sleeve's wall; the three joined and bored; two
/// oil holes drilled from a datum plane turned 30 degrees about the
/// sleeve's axis (a datum axis); three bolt holes in a row, mirrored in
/// the datum plane the base was mirrored in.
///
/// Its volume, piece by piece from the definition: the base; the pedestal
/// as the integral of 32 − √(18² − x²) over its footprint (from the base's
/// top at 8 to the sleeve's underside, 40 − √(18² − x²)); the sleeve's
/// wall, π (18² − 11²) 60; less six bolt holes through the base and two
/// oil holes of radius a through the wall along a radius, each
/// ∫ 2 √(a² − u²) (√(18² − u²) − √(11² − u²)) du.
fn housing(s: &mut Session, k: f64) -> Forms {
    housing_base(s, k)?;
    let p = |x: f64, y: f64| json!([x * k, y * k]);
    step(
        s,
        "feature.cylinder",
        json!({"radius": 18.0 * k, "height": 60.0 * k, "position": [0, -30.0 * k, 40.0 * k],
               "axis": [0, 1, 0], "name": "Sleeve"}),
    )?;
    sketch(s, "Foot", json!("xy"), 8.0 * k)?;
    step(
        s,
        "sketch.spline",
        json!({"sketch": "Foot",
               "control": [p(0.0, -24.0), p(14.0, -24.0), p(14.0, 24.0), p(0.0, 24.0)]}),
    )?;
    step(
        s,
        "sketch.spline",
        json!({"sketch": "Foot", "control": ["e4", p(-14.0, 24.0), p(-14.0, -24.0), "e1"]}),
    )?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Foot", "upTo": "F6:face(0)", "name": "Pedestal"}),
    )?;
    step(
        s,
        "feature.boolean",
        json!({"op": "union", "target": "Base", "tool": "Pedestal", "name": "Stand"}),
    )?;
    step(
        s,
        "feature.boolean",
        json!({"op": "union", "target": "Stand", "tool": "Sleeve", "name": "Solid"}),
    )?;
    step(
        s,
        "feature.cylinder",
        json!({"radius": 11.0 * k, "height": 70.0 * k, "position": [0, -35.0 * k, 40.0 * k],
               "axis": [0, 1, 0], "name": "Bore"}),
    )?;
    step(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Solid", "tool": "Bore", "name": "Bored"}),
    )?;
    step(
        s,
        "datum.axis",
        json!({"face": "F6:face(0)", "name": "SleeveAxis"}),
    )?;
    step(
        s,
        "datum.plane",
        json!({"from": "xy", "distance": 40.0 * k, "name": "AxisLevel"}),
    )?;
    step(
        s,
        "datum.plane",
        json!({"from": "AxisLevel", "about": "SleeveAxis", "angle": "30 deg", "name": "Slope"}),
    )?;
    sketch(s, "OilHoles", json!("Slope"), 24.0 * k)?;
    circle(s, "OilHoles", [0.0, -14.0 * k], 2.5 * k)?;
    circle(s, "OilHoles", [0.0, 14.0 * k], 2.5 * k)?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "OilHoles", "distance": 24.0 * k, "reversed": true, "cut": "Bored",
               "name": "Oiled"}),
    )?;
    step(
        s,
        "feature.cylinder",
        json!({"radius": 3.5 * k, "height": 20.0 * k, "position": [44.0 * k, -22.0 * k, -6.0 * k],
               "name": "Hole"}),
    )?;
    step(
        s,
        "feature.pattern",
        json!({"source": "Hole", "count": 3, "step": [0, 22.0 * k, 0], "name": "HolesRight"}),
    )?;
    step(
        s,
        "feature.mirror",
        json!({"source": "HolesRight", "plane": "Mid", "keep": true, "name": "HolesLeft"}),
    )?;
    step(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Oiled", "tool": "HolesRight", "name": "DrilledRight"}),
    )?;
    step(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "DrilledRight", "tool": "HolesLeft", "name": "Housing"}),
    )?;

    // The pedestal's footprint: x = ±42 t (1 − t), y from -24 to 24 with
    // dy/dt = 288 t (1 − t).
    let under = |x: f64| 32.0 * x - 0.5 * (x * (324.0 - x * x).sqrt() + 324.0 * (x / 18.0).pasin());
    let pedestal = 2.0 * integral(|t| under(42.0 * t * (1.0 - t)) * 288.0 * t * (1.0 - t));
    let sleeve = PI * (324.0 - 121.0) * 60.0;
    let bolts = 6.0 * PI * 3.5 * 3.5 * 8.0;
    let a = 2.5_f64;
    // u = a sin φ takes the root out of the rim.
    let oil = 2.0
        * integral(|t| {
            let phi = PI * (t - 0.5);
            let u = a * phi.psin();
            PI * 2.0
                * a
                * a
                * phi.pcos().ppowi(2)
                * ((324.0 - u * u).sqrt() - (121.0 - u * u).sqrt())
        });
    Ok(vec![
        closed(
            "Housing",
            8.0 * base_area() + pedestal + sleeve - bolts - oil,
            0.0,
        )
        .volume_only(),
    ])
}

/// A bored sleeve (a tube of radii 18 and 11, 60 long, lying along y)
/// with one oil hole of radius `b` drilled along a radius: from 24 off the
/// axis, on a datum plane turned by `degrees` about the sleeve's axis,
/// `depth` deep, `along` from the middle of the sleeve.
///
/// The rim of such a hole on the sleeve is a closed spline edge with one
/// vertex, and its knots in the body are lengths. These are the bodies
/// FreeCAD called invalid at the scale 1e6 while the knots were written
/// as they are ("self-intersecting wire" on the sleeve's wall; 57 of 144
/// such sleeves at 1e6, 1 of 144 at 1e3, none at 1), and calls valid now
/// that a curve's knots are written from 0 to 1 (`craft3d_io::step`,
/// B-spline curve knots). The four here were among the 57.
///
/// The closed form, across the hole (x) and along it (z, from the axis):
/// the tube's wall is where 11² ≤ x² + z² ≤ 18², the hole takes z from
/// 24 − depth up, so over a point of its disc it removes the length
/// `gone(x)` below, and of the tube's skin the patches it passes
/// through, each larger than the disc under it by radius / height; its
/// own wall is that length round its rim, and it has a flat bottom where
/// it ends in the wall.
fn oil_hole(degrees: f64, b: f64, depth: f64, along: f64) -> impl Fn(&mut Session, f64) -> Forms {
    move |s, k| {
        step(
            s,
            "feature.cylinder",
            json!({"radius": 18.0 * k, "height": 60.0 * k, "position": [0, -30.0 * k, 40.0 * k],
                   "axis": [0, 1, 0], "name": "Sleeve"}),
        )?;
        step(
            s,
            "feature.cylinder",
            json!({"radius": 11.0 * k, "height": 70.0 * k, "position": [0, -35.0 * k, 40.0 * k],
                   "axis": [0, 1, 0], "name": "Bore"}),
        )?;
        step(
            s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Sleeve", "tool": "Bore", "name": "Bored"}),
        )?;
        step(
            s,
            "datum.axis",
            json!({"face": "F1:face(0)", "name": "SleeveAxis"}),
        )?;
        step(
            s,
            "datum.plane",
            json!({"from": "xy", "distance": 40.0 * k, "name": "AxisLevel"}),
        )?;
        step(
            s,
            "datum.plane",
            json!({"from": "AxisLevel", "about": "SleeveAxis",
                   "angle": format!("{degrees} deg"), "name": "Slope"}),
        )?;
        sketch(s, "OilHole", json!("Slope"), 24.0 * k)?;
        circle(s, "OilHole", [0.0, along * k], b * k)?;
        step(
            s,
            "feature.extrude",
            json!({"sketch": "OilHole", "distance": depth * k, "reversed": true, "cut": "Bored",
                   "name": "Oiled"}),
        )?;

        let (outer, inner, length) = (18.0_f64, 11.0_f64, 60.0);
        let end = 24.0 - depth;
        // The stretches of z the wall has over x: above and below the bore.
        let wall = move |x: f64| {
            let (o, i) = (
                (outer * outer - x * x).sqrt(),
                (inner * inner - x * x).sqrt(),
            );
            [(i, o), (-o, -i)]
        };
        let gone = move |x: f64| -> f64 {
            wall(x)
                .iter()
                .map(|&(low, high)| (high.min(24.0) - low.max(end)).max(0.0))
                .sum()
        };
        // The skin the hole passes through: every end of a stretch above
        // the hole's own end, as radius over height.
        let skin = move |x: f64| -> f64 {
            wall(x)
                .iter()
                .flat_map(|&(low, high)| [(low, low.abs()), (high, high.abs())])
                .filter(|&(z, _)| z > end)
                .map(|(_, height)| {
                    let radius = (height * height + x * x).sqrt();
                    radius / height
                })
                .sum()
        };
        // A flat bottom where the hole ends inside the wall (asked of the
        // whole disc: the cases here end well inside it or well outside).
        let ends_in_the_wall = wall(0.0).iter().any(|&(low, high)| low < end && end < high);
        let bottom = if ends_in_the_wall { PI * b * b } else { 0.0 };
        let rim = integral(|u| 2.0 * PI * b * gone(b * (2.0 * PI * u).pcos()));
        let tube = PI * (outer * outer - inner * inner);
        Ok(vec![
            closed(
                "Oiled",
                tube * length - over_disc([0.0, 0.0], b, |x, _| gone(x)),
                2.0 * PI * (outer + inner) * length + 2.0 * tube
                    - over_disc([0.0, 0.0], b, |x, _| skin(x))
                    + rim
                    + bottom,
            )
            .within(1e-7),
        ])
    }
}

// ---------------------------------------------------------------------
// Slivers: overlaps ten tolerances deep (the document's tolerance is
// 1e-6 of the scale), with the closed forms of
// `crates/kernel/tests/boolean.rs`.

const DEEP: f64 = 10e-6;

fn combine(s: &mut Session, op: &str, target: &str, tool: &str) -> Built {
    step(
        s,
        "feature.boolean",
        json!({"op": op, "target": target, "tool": tool, "name": "Result"}),
    )
    .map(drop)
}

/// A box set ten tolerances deep into the side of a plate.
fn box_into_a_plate(op: &'static str) -> impl Fn(&mut Session, f64) -> Forms {
    move |s, k| {
        step(
            s,
            "feature.block",
            json!({"size": [4.0 * k, 3.0 * k, 2.0 * k], "name": "Plate"}),
        )?;
        let long = 2.0 + DEEP;
        step(
            s,
            "feature.block",
            json!({"size": [long * k, 2.0 * k, k], "position": [(4.0 - DEEP) * k, 0.5 * k, 0.5 * k],
                   "name": "Beside"}),
        )?;
        combine(s, op, "Plate", "Beside")?;
        let beside_area = 2.0 * (long * 2.0 + long + 2.0);
        let (volume, area) = match op {
            // The sliver: 2 by 1 and `DEEP` thick.
            "intersect" => (2.0 * DEEP, 4.0 + 6.0 * DEEP),
            "union" => (28.0, 52.0 - 2.0 + beside_area - 2.0 - 6.0 * DEEP),
            // The plate with a pocket ten tolerances deep.
            _ => (24.0 - 2.0 * DEEP, 52.0 + 6.0 * DEEP),
        };
        Ok(vec![closed("Result", volume, area)])
    }
}

/// A square bar of side 1 along x, lying on one of its edges: that edge is
/// at height `lowest` over the x axis, and its faces are at 45 degrees.
fn bar_on_its_edge(s: &mut Session, k: f64, lowest: f64, name: &str) -> Built {
    let h = FRAC_1_SQRT_2;
    sketch(s, "Diamond", json!("yz"), -0.5 * k)?;
    polygon(
        s,
        "Diamond",
        k,
        &[
            [0.0, lowest],
            [h, lowest + h],
            [0.0, lowest + 2.0 * h],
            [-h, lowest + h],
        ],
    )?;
    step(
        s,
        "feature.extrude",
        json!({"sketch": "Diamond", "distance": k, "name": name}),
    )
    .map(drop)
}

/// A cube on its edge pushed ten tolerances into the top of a plate: the
/// overlap is a prism with triangles for ends, twenty tolerances wide and
/// ten high.
fn edge_into_a_face(op: &'static str) -> impl Fn(&mut Session, f64) -> Forms {
    move |s, k| {
        step(
            s,
            "feature.block",
            json!({"size": [4.0 * k, 3.0 * k, 2.0 * k], "position": [-2.0 * k, -1.5 * k, 0],
                   "name": "Plate"}),
        )?;
        bar_on_its_edge(s, k, 2.0 - DEEP, "Cube")?;
        combine(s, op, "Plate", "Cube")?;
        let root = 2.0_f64.sqrt();
        let (volume, area) = match op {
            "intersect" => (
                DEEP * DEEP,
                2.0 * DEEP * DEEP + 2.0 * DEEP + 2.0 * root * DEEP,
            ),
            "union" => (
                25.0 - DEEP * DEEP,
                52.0 + 6.0 - 2.0 * DEEP - 2.0 * root * DEEP - 2.0 * DEEP * DEEP,
            ),
            _ => (
                24.0 - DEEP * DEEP,
                52.0 + 2.0 * root * DEEP - 2.0 * DEEP + 2.0 * DEEP * DEEP,
            ),
        };
        Ok(vec![closed("Result", volume, area)])
    }
}

/// A rod standing on an edge of a bar that passes under the middle of its
/// base: they touch along a stretch of that edge and nowhere else. Their
/// union would be two parts of one body touching along a line, which is
/// refused as zero thickness (the developer's decision of 2026-10-15;
/// `zero_thickness.rs` holds that at three scales without FreeCAD): for
/// "union" the refusal is required, undone, and the two bodies, which may
/// touch, are what is exported. The rod less the bar is the rod.
fn rod_on_an_edge(op: &'static str) -> impl Fn(&mut Session, f64) -> Forms {
    move |s, k| {
        step(
            s,
            "feature.cylinder",
            json!({"radius": k, "height": 2.0 * k, "name": "Rod"}),
        )?;
        bar_on_its_edge(s, k, -2.0 * FRAC_1_SQRT_2, "Wedge")?;
        if op != "union" {
            combine(s, op, "Rod", "Wedge")?;
            return Ok(vec![closed("Result", 2.0 * PI, 6.0 * PI)]);
        }
        let params = json!({"op": op, "target": "Rod", "tool": "Wedge", "name": "Result"});
        let made = s
            .execute("feature.boolean", &params)
            .map_err(|e| format!("feature.boolean {params}: {e}"))?;
        let why = made["error"].as_str().unwrap_or("");
        if made["status"] != "failed" || !why.contains("zero thickness") {
            return Err(format!(
                "feature.boolean {params}: the parts touch along a line, to be refused as zero thickness, but: {made}"
            ));
        }
        step(s, "edit.undo", json!({}))?;
        Ok(vec![
            closed("Rod", 2.0 * PI, 6.0 * PI),
            closed("Wedge", 1.0, 6.0),
        ])
    }
}

// ---------------------------------------------------------------------
// Small bodies far from the origin.

/// A block ten tolerances long, 2, 3 and 5 from the origin: a hundred
/// thousand times its own size away. The coordinates are rounded to
/// 2⁻⁵² of 5 before anything is computed, which is 1e-10 of the block's
/// size; a volume is right to three times that.
fn small_block_far_away(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "feature.block",
        json!({"size": [1e-5 * k, 1e-5 * k, 1e-5 * k], "position": [2.0 * k, 3.0 * k, 5.0 * k],
               "name": "Grain"}),
    )?;
    Ok(vec![
        closed("Grain", 1e-15, 6e-10)
            .at([2.000_005, 3.000_005, 5.000_005])
            .within(1e-9),
    ])
}

fn small_cylinder_far_away(s: &mut Session, k: f64) -> Forms {
    step(
        s,
        "feature.cylinder",
        json!({"radius": 1e-5 * k, "height": 2e-5 * k, "position": [2.0 * k, 3.0 * k, 5.0 * k],
               "name": "Grain"}),
    )?;
    Ok(vec![
        closed("Grain", PI * 2e-15, 2.0 * PI * 1e-5 * 3e-5)
            .at([2.0, 3.0, 5.000_01])
            .within(1e-9),
    ])
}

// ---------------------------------------------------------------------

/// Edges that are no line in the parameter plane of their face, and no
/// torus in the box: see `harness::APPROXIMATED`.
const APPROX_RING: Expect = Expect {
    relative: harness::APPROXIMATED,
    extent: Extent::Unchecked,
};
/// A torus, or a part of a sphere or a spline surface, whose box of ours
/// is that of a mesh: one box must contain the other.
const LOOSE: Expect = Expect {
    relative: RELATIVE,
    extent: Extent::Loose,
};

/// The housing: plane faces bounded by a spline of four spans, among
/// trimmed curved faces. FreeCAD's own volume of it is 1.04e-3 short and
/// its area 4.2e-5, at every scale, and its centre of mass lies 6.7e-5 mm
/// off the plane the body is symmetric about (ours: 3e-10). That is its
/// integration of such plane faces (see `SPLINE`: 1e-4 to 3e-4 for one
/// and two spans), not its reading of the file: of the base alone
/// (`spline-housing-base`) its own volume is 1.8e-3 short while the
/// quadrature over what it imported gives ours to 1e-14, and ours of the
/// whole housing is the volume worked out from the definition in
/// [`housing`] to 1e-9. So of FreeCAD's two measures only that they are
/// near; that it reads a valid, closed solid of our 20 faces is asked as
/// everywhere.
const ROUGH: Expect = Expect {
    relative: 5e-3,
    extent: Extent::Tight,
};

/// Cases FreeCAD cannot judge, by a part of the file name, each with what
/// was measured. A case named here is reported, not failed; what we say of
/// it is still held to its closed form.
///
/// All of them are one thing. FreeCAD takes points closer together than
/// 1e-7 mm for one point, whatever the file's uncertainty says (its
/// kernel's own precision, fixed in millimetres), and at the scale 1e-3
/// a sliver ten tolerances thick is 1e-8 mm thick and a grain ten
/// tolerances long 1e-8 mm long: it reads the sliver's 8 vertices as 4
/// and the grain's 8 as 1. That it is the reader's millimetres and not
/// the file is shown by the same documents with the same numbers in
/// metres (`METRES`: 1e-8 m is 1e-5 mm), which it reads to 1e-11, as it
/// does those at the scales 1 and 1e6.
const BEYOND_THE_READER: &[(&str, &str)] = &[
    (
        "milli-sliver-box-in-plate-subtract.step",
        SMALLER_THAN_ITS_PRECISION,
    ),
    (
        "milli-sliver-box-in-plate-intersect.step",
        SMALLER_THAN_ITS_PRECISION,
    ),
    (
        "milli-sliver-edge-in-face-union.step",
        SMALLER_THAN_ITS_PRECISION,
    ),
    (
        "milli-sliver-edge-in-face-subtract.step",
        SMALLER_THAN_ITS_PRECISION,
    ),
    (
        "milli-sliver-edge-in-face-intersect.step",
        SMALLER_THAN_ITS_PRECISION,
    ),
    ("milli-far-small-block.step", SMALLER_THAN_ITS_PRECISION),
    ("milli-far-small-cylinder.step", SMALLER_THAN_ITS_PRECISION),
];
const SMALLER_THAN_ITS_PRECISION: &str = "a feature of 1e-8 mm, under FreeCAD's fixed precision of 1e-7 mm; \
     the same document in metres is read right";
/// The documents of [`BEYOND_THE_READER`], built once more with the same
/// numbers in a document whose unit is the metre.
const METRES: &[&str] = &[
    "sliver-box-in-plate-subtract",
    "sliver-box-in-plate-intersect",
    "sliver-edge-in-face-union",
    "sliver-edge-in-face-subtract",
    "sliver-edge-in-face-intersect",
    "far-small-block",
    "far-small-cylinder",
];

/// `∫ ρ dA` of a curved flange's section about its arc's axis (Pappus,
/// docs/design/sheetmetal.md › *Curved bends*), written out here: the
/// bend's ring sector between `r` and `r + t` over `theta` at `x = ρ sin φ`,
/// and the flat `l × t` at `x = ρ sin θ + d cos θ`; `eps` +1 when the
/// flange grows away from the arc's centre.
fn pappus(a: f64, eps: f64, theta: f64, r: f64, t: f64, l: f64) -> f64 {
    let ro = r + t;
    let area = theta * (ro * ro - r * r) / 2.0 + l * t;
    let mx = (1.0 - theta.pcos()) * (ro.ppowi(3) - r.ppowi(3)) / 3.0
        + theta.psin() * l * (ro * ro - r * r) / 2.0
        + theta.pcos() * t * l * l / 2.0;
    a * area + eps * mx
}

/// The entity a sketch command made.
fn made(s: &mut Session, id: &str, params: Value) -> Result<String, String> {
    let out = step(s, id, params)?;
    out["entity"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{id} made no entity"))
}

/// A sheet-metal part of thickness 1, radius 2 and K-factor 0.4 (scaled).
fn sheet_part(s: &mut Session, k: f64, name: &str, rest: Value) -> Built {
    let mut params = json!({"sketch": "Base", "name": name, "thickness": k,
                            "radius": 2.0 * k, "kFactor": 0.4});
    if let (Some(map), Value::Object(more)) = (params.as_object_mut(), rest) {
        map.extend(more);
    }
    step(s, "feature.sheetMetal", params).map(drop)
}

/// A round pan: a disc of radius 40 with its rim bent up 90 degrees, 10
/// long (a whole circle: a torus and a cylinder about the disc's axis).
fn curved_pan(s: &mut Session, k: f64) -> Forms {
    sketch(s, "Base", json!("xy"), 0.0)?;
    let rim = made(
        s,
        "sketch.circle",
        json!({"sketch": "Base", "center": [0, 0], "radius": 40.0 * k}),
    )?;
    sheet_part(
        s,
        k,
        "Pan",
        json!({"curvedFlanges": [{"edge": rim, "length": 10.0 * k, "angle": "90 deg"}]}),
    )?;
    let v = PI * 1600.0 + 2.0 * PI * pappus(40.0, 1.0, PI / 2.0, 2.0, 1.0, 10.0);
    Ok(vec![closed("Pan", v, 0.0).volume_only().within(1e-7)])
}

/// A collar bent down 90 degrees round a hole of radius 15 in a plate 100
/// × 60, 4 long, its own radius 1.5 and K-factor ½ (a stretch flange).
fn curved_collar(s: &mut Session, k: f64) -> Forms {
    sketch(s, "Base", json!("xy"), 0.0)?;
    rectangle(s, "Base", k, (0.0, 100.0), (0.0, 60.0))?;
    let hole = made(
        s,
        "sketch.circle",
        json!({"sketch": "Base", "center": [50.0 * k, 30.0 * k], "radius": 15.0 * k}),
    )?;
    sheet_part(
        s,
        k,
        "Collar",
        json!({"curvedFlanges": [{"edge": hole, "length": 4.0 * k, "angle": "90 deg",
                                  "direction": "down", "radius": 1.5 * k, "kFactor": 0.5}]}),
    )?;
    let v = 6000.0 - PI * 225.0 + 2.0 * PI * pappus(15.0, -1.0, PI / 2.0, 1.5, 1.0, 4.0);
    Ok(vec![closed("Collar", v, 0.0).volume_only().within(1e-7)])
}

/// A plate 100 wide whose top is a half circle of radius 50, its arc bent
/// up by `degrees`, 10 long: the flat a cone (60: widening, 120: closing).
fn curved_cone(degrees: f64) -> impl Fn(&mut Session, f64) -> Forms {
    move |s: &mut Session, k: f64| {
        sketch(s, "Base", json!("xy"), 0.0)?;
        for (a, b) in [
            ([0.0, 0.0], [100.0, 0.0]),
            ([100.0, 0.0], [100.0, 60.0]),
            ([0.0, 60.0], [0.0, 0.0]),
        ] {
            made(
                s,
                "sketch.line",
                json!({"sketch": "Base", "from": [a[0] * k, a[1] * k], "to": [b[0] * k, b[1] * k]}),
            )?;
        }
        let arc = made(
            s,
            "sketch.arc",
            json!({"sketch": "Base", "center": [50.0 * k, 60.0 * k],
                   "start": [100.0 * k, 60.0 * k], "end": [0.0, 60.0 * k]}),
        )?;
        sheet_part(
            s,
            k,
            "Dome",
            json!({"curvedFlanges": [{"edge": arc, "length": 10.0 * k,
                                      "angle": format!("{degrees} deg")}]}),
        )?;
        let v = 6000.0 + PI * 1250.0 + PI * pappus(50.0, 1.0, degrees.to_radians(), 2.0, 1.0, 10.0);
        Ok(vec![closed("Dome", v, 0.0).volume_only().within(1e-7)])
    }
}

/// A tray 100 × 60 with its corners rounded to 25: four walls 8 long on the
/// lines and four curved corners, one wall all round.
fn curved_tray(s: &mut Session, k: f64) -> Forms {
    sketch(s, "Base", json!("xy"), 0.0)?;
    let (rc, w, h) = (25.0, 100.0, 60.0);
    let sc = |p: [f64; 2]| [p[0] * k, p[1] * k];
    let mut lines = Vec::new();
    for (a, b) in [
        ([rc, 0.0], [w - rc, 0.0]),
        ([w, rc], [w, h - rc]),
        ([w - rc, h], [rc, h]),
        ([0.0, h - rc], [0.0, rc]),
    ] {
        lines.push(made(
            s,
            "sketch.line",
            json!({"sketch": "Base", "from": sc(a), "to": sc(b)}),
        )?);
    }
    let mut arcs = Vec::new();
    for (c, a, b) in [
        ([w - rc, rc], [w - rc, 0.0], [w, rc]),
        ([w - rc, h - rc], [w, h - rc], [w - rc, h]),
        ([rc, h - rc], [rc, h], [0.0, h - rc]),
        ([rc, rc], [0.0, rc], [rc, 0.0]),
    ] {
        arcs.push(made(
            s,
            "sketch.arc",
            json!({"sketch": "Base", "center": sc(c), "start": sc(a), "end": sc(b)}),
        )?);
    }
    let wall = |e: &String| json!({"edge": e, "length": 8.0 * k, "angle": "90 deg"});
    sheet_part(
        s,
        k,
        "Tray",
        json!({"flanges": lines.iter().map(wall).collect::<Vec<_>>(),
               "curvedFlanges": arcs.iter().map(wall).collect::<Vec<_>>()}),
    )?;
    let theta = PI / 2.0;
    let walls = 2.0 * (50.0 + 10.0);
    let v = (6000.0 - (4.0 - PI) * rc * rc)
        + walls * (theta * (2.0 + 0.5) + 8.0)
        + 4.0 * theta * pappus(rc, 1.0, theta, 2.0, 1.0, 8.0);
    Ok(vec![closed("Tray", v, 0.0).volume_only().within(1e-7)])
}

/// How the meshes of the documents are held to the exact bodies.
///
/// FreeCAD's mesh kernel stores 32-bit floats, so a coordinate is rounded
/// by up to 6e-8 of its own size (2⁻²⁴), whatever the format; the surface
/// of the mesh is within the chordal tolerance of the body. So FreeCAD's
/// volume must be ours within the area times (the tolerance plus that
/// rounding of the largest coordinate), 2e-5 of the volume as
/// `crates/io/tests/freecad.rs` asks, and what its sum loses: it adds
/// per triangle the triple product of three corners as they are, six
/// products of the size of the largest coordinate cubed, each rounded to
/// 6e-8 of itself. For a body far smaller than its distance from the
/// origin that last term is larger than the body (measured: a block of
/// 1e-15 at (2, 3, 5) has a volume of 1.9e-6 in FreeCAD), and FreeCAD's
/// volume then says nothing; that the mesh is closed, its triangle count
/// and its box are still compared.
pub fn check_meshes(dir: &std::path::Path, meshes: &[MeshCase]) -> (String, Vec<String>) {
    let files: Vec<MeshFile> = meshes
        .iter()
        .map(|m| MeshFile {
            file: m.file.clone(),
            bytes: m.bytes.clone(),
        })
        .collect();
    let records = harness::run_freecad_meshes(dir, &files);
    let mut text = String::from(
        "\n## Meshes of the same documents, read by FreeCAD's mesh module\n\n\
         | file | solid | triangles | volume: FreeCAD's - ours, over the bound | bbox off, over the bound |\n|---|---|---|---|---|\n",
    );
    let mut problems = Vec::new();
    for mesh in meshes {
        let Some(record) = records.get(&mesh.file) else {
            problems.push(format!("{}: no record", mesh.file));
            continue;
        };
        if let Some(error) = record["error"].as_str() {
            problems.push(format!("{}: FreeCAD could not read it: {error}", mesh.file));
            continue;
        }
        let mut bad = Vec::new();
        if mesh.closed != mesh.bodies {
            bad.push(format!(
                "we call {} of {} meshes closed",
                mesh.closed, mesh.bodies
            ));
        }
        if record["solid"] != true {
            bad.push("FreeCAD does not see a closed solid".to_string());
        }
        let facets = record["facets"].as_u64().unwrap_or(0) as usize;
        if facets != mesh.triangles {
            bad.push(format!(
                "{facets} facets in FreeCAD, {} written",
                mesh.triangles
            ));
        }
        let theirs: Vec<f64> = record["bbox"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default();
        let reach = theirs.iter().fold(0.0_f64, |m, x| m.max(x.abs()));
        let rounding = reach * 1.2e-7;
        let volume = record["volume"].as_f64().unwrap_or(f64::NAN);
        let sum = mesh.triangles as f64 * reach.ppowi(3) * 6e-8;
        let bound = mesh.area * (mesh.tolerance + rounding) + 2e-5 * mesh.volume + sum;
        let volume_off = (volume - mesh.volume).abs() / bound;
        if !(volume_off <= 1.0) {
            bad.push(format!(
                "volume {volume:e}, ours {:e}, bound {bound:e}",
                mesh.volume
            ));
        }
        // Both boxes are of a mesh within its tolerance of the body.
        let mut box_off = f64::NAN;
        if let (Some(ours), 6) = (mesh.bbox, theirs.len()) {
            let size = (0..3).map(|i| ours[i + 3] - ours[i]).fold(0.0, f64::max);
            let bound = mesh.tolerance.max(mesh.bbox_slack) + rounding + 1e-6 * size;
            box_off = (0..6)
                .map(|i| (theirs[i] - ours[i]).abs())
                .fold(0.0, f64::max)
                / bound;
            if !(box_off <= 1.0) {
                bad.push(format!("bounding box {theirs:?}, ours {ours:?}"));
            }
        }
        writeln!(
            text,
            "| {} | {} | {} | {:.2} | {:.2} |",
            mesh.file, record["solid"], facets, volume_off, box_off
        )
        .unwrap();
        if !bad.is_empty() {
            problems.push(format!("{}: {}", mesh.file, bad.join("; ")));
        }
    }
    (text, problems)
}

#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD"]
fn freecad_reads_extrusion_ends_datums_splines_and_slivers() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("step-features");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut set = Set {
        dir: dir.clone(),
        cases: Vec::new(),
        refused: Vec::new(),
        unmeasured: Vec::new(),
        per_body: true,
        meshes: Some(Vec::new()),
    };
    // Ours against the closed forms: the lines of the report, and what
    // disagrees.
    let mut forms_text = String::from(
        "\n## Ours against the closed forms\n\n| file | body | volume - 1 | area - 1 | centre off |\n|---|---|---|---|---|\n",
    );
    let mut forms_bad = Vec::new();

    type Build<'a> = Box<dyn Fn(&mut Session, f64) -> Forms + 'a>;
    let cases: Vec<(&str, Expect, Build)> = vec![
        // An ellipse on the post's wall.
        (
            "upto-inclined-plane",
            APPROX,
            Box::new(up_to_an_inclined_plane),
        ),
        ("upto-cylinder", EXACT, Box::new(up_to_a_cylinder)),
        ("upto-sphere", EXACT, Box::new(up_to_a_sphere)),
        (
            "upto-sphere-off-axis",
            APPROX,
            Box::new(up_to_a_sphere_off_its_axis),
        ),
        ("upto-torus", RING, Box::new(up_to_a_torus)),
        (
            "upto-torus-off-circle",
            APPROX_RING,
            Box::new(up_to_a_torus_off_its_circle),
        ),
        ("upto-freeform", EXACT, Box::new(up_to_a_freeform_face)),
        ("through-all-cut", EXACT, Box::new(through_all_cut)),
        (
            "through-all-both-ways",
            EXACT,
            Box::new(through_all_both_ways),
        ),
        ("datum-distance", EXACT, Box::new(datum_plane_at_a_distance)),
        ("datum-angle", FLAT, Box::new(datum_plane_at_an_angle)),
        (
            "datum-about-edge",
            EXACT,
            Box::new(datum_plane_about_an_edge),
        ),
        (
            "datum-three-points",
            EXACT,
            Box::new(datum_plane_through_three_points),
        ),
        (
            "datum-revolve-third",
            EXACT,
            Box::new(revolve_about_a_datum_axis(false)),
        ),
        (
            "datum-revolve-whole",
            EXACT,
            Box::new(revolve_about_a_datum_axis(true)),
        ),
        (
            "datum-revolve-circle",
            RING,
            Box::new(revolve_a_circle_about_a_datum_axis),
        ),
        ("datum-mirror", FLAT, Box::new(mirror_in_a_datum_plane)),
        (
            "datum-mirror-joined",
            FLAT,
            Box::new(mirror_in_a_face_and_join),
        ),
        ("datum-pattern", EXACT, Box::new(pattern_about_a_datum_axis)),
        (
            "datum-pattern-points",
            LOOSE,
            Box::new(pattern_about_an_axis_through_datum_points),
        ),
        ("spline-d-hole", SPLINE, Box::new(spline_d_with_a_hole)),
        ("pillar", APPROX, Box::new(pillar)),
        ("impeller", EXACT, Box::new(impeller)),
        ("spline-housing-base", ROUGH, Box::new(spline_housing_base)),
        ("housing", ROUGH, Box::new(housing)),
        // Through the wall, along a radius.
        (
            "oil-hole-radial",
            APPROX,
            Box::new(oil_hole(0.0, 2.5, 24.0, -14.0)),
        ),
        // Blind: ends in the wall.
        (
            "oil-hole-blind",
            APPROX,
            Box::new(oil_hole(60.0, 1.5, 10.0, 3.0)),
        ),
        // Through both walls.
        (
            "oil-hole-across",
            APPROX,
            Box::new(oil_hole(45.0, 6.0, 60.0, 3.0)),
        ),
        (
            "oil-hole-wide",
            APPROX,
            Box::new(oil_hole(135.0, 6.0, 24.0, -14.0)),
        ),
        (
            "sliver-box-in-plate-union",
            FLAT,
            Box::new(box_into_a_plate("union")),
        ),
        (
            "sliver-box-in-plate-subtract",
            FLAT,
            Box::new(box_into_a_plate("subtract")),
        ),
        (
            "sliver-box-in-plate-intersect",
            FLAT,
            Box::new(box_into_a_plate("intersect")),
        ),
        (
            "sliver-edge-in-face-union",
            FLAT,
            Box::new(edge_into_a_face("union")),
        ),
        (
            "sliver-edge-in-face-subtract",
            FLAT,
            Box::new(edge_into_a_face("subtract")),
        ),
        (
            "sliver-edge-in-face-intersect",
            FLAT,
            Box::new(edge_into_a_face("intersect")),
        ),
        (
            "rod-on-edge-union-refused",
            EXACT,
            Box::new(rod_on_an_edge("union")),
        ),
        (
            "rod-on-edge-subtract",
            EXACT,
            Box::new(rod_on_an_edge("subtract")),
        ),
        ("far-small-block", FLAT, Box::new(small_block_far_away)),
        (
            "far-small-cylinder",
            EXACT,
            Box::new(small_cylinder_far_away),
        ),
        // Curved bends (sheet metal): tori and cones about an arc's axis.
        ("curved-pan", RING, Box::new(curved_pan)),
        ("curved-collar", RING, Box::new(curved_collar)),
        ("curved-cone-60", RING, Box::new(curved_cone(60.0))),
        ("curved-cone-120", RING, Box::new(curved_cone(120.0))),
        ("curved-tray", RING, Box::new(curved_tray)),
    ];

    let mut meshes = None;
    for (k, tag, unit) in [
        (1e-3, "milli", "mm"),
        (1.0, "one", "mm"),
        (1e6, "mega", "mm"),
        (1e-3, "metres", "m"),
    ] {
        if unit == "m" {
            // No meshes of these: FreeCAD has read the same numbers at
            // the scale 1e-3 already, and a mesh has no unit it heeds.
            meshes = set.meshes.take();
        }
        for (name, expect, build) in &cases {
            if unit == "m" && !METRES.contains(name) {
                continue;
            }
            let file = format!("{tag}-{name}");
            let mut forms = Vec::new();
            let before = set.cases.len();
            set.add(&file, unit, k, *expect, |s, k| {
                forms = build(s, k)?;
                Ok(())
            });
            if set.cases.len() == before {
                continue;
            }
            let case = set.cases.last().unwrap();
            for form in &forms {
                let Some(part) = case.parts.iter().find(|p| p.name == form.body) else {
                    forms_bad.push(format!("{file}: no body {}", form.body));
                    continue;
                };
                let volume = part.volume / (form.volume * k.ppowi(3)) - 1.0;
                let area = if form.with_area {
                    part.area / (form.area * k * k) - 1.0
                } else {
                    0.0
                };
                let size = part.area.sqrt();
                let centre = form.centroid.map(|c| {
                    (0..3)
                        .map(|i| (part.centroid[i] - c[i] * k).ppowi(2))
                        .sum::<f64>()
                        .sqrt()
                        / size
                });
                writeln!(
                    forms_text,
                    "| {file} | {} | {volume:+.1e} | {} | {} |",
                    form.body,
                    if form.with_area {
                        format!("{area:+.1e}")
                    } else {
                        "-".to_string()
                    },
                    centre.map_or("-".to_string(), |c| format!("{c:.1e}"))
                )
                .unwrap();
                if !(volume.abs() <= form.relative && area.abs() <= form.relative)
                    || centre.is_some_and(|c| !(c <= 10.0 * form.relative))
                {
                    forms_bad.push(format!(
                        "{file}: {} is off its closed form: volume {volume:+e}, area {area:+e}, centre {centre:?} (asked {:e})",
                        form.body, form.relative
                    ));
                }
            }
        }
    }

    let rows = harness::check(
        "step-features-files",
        "STEP: extrusion ends, datums, splines and slivers, read by FreeCAD",
        &set.cases,
    );
    let files = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("step-features-files");
    let mut report = std::fs::read_to_string(files.join("report.md")).unwrap();

    // Body by body.
    report.push_str(
        "\n## Body by body: FreeCAD's solid over ours\n\n| file | body | volume - 1 | area - 1 | centre off |\n|---|---|---|---|---|\n",
    );
    for row in &rows {
        for part in &row.parts {
            writeln!(
                report,
                "| {} | {} | {:+.1e} | {:+.1e} | {:.1e} |",
                row.file,
                part.name,
                part.volume_ratio - 1.0,
                part.area_ratio - 1.0,
                part.centre_off
            )
            .unwrap();
        }
    }
    report.push_str(&forms_text);

    // The spline solids whose curved faces are whole patches, by a
    // quadrature over what FreeCAD imported (see `SPLINE`).
    let mut bad: Vec<String> = Vec::new();
    report.push_str("\n## By quadrature over FreeCAD's geometry\n\n");
    for row in rows.iter().filter(|r| r.file.contains("spline")) {
        match row.quadrature_volume_ratio {
            Some(ratio) => {
                writeln!(
                    report,
                    "- {}: the volume of FreeCAD's geometry over ours - 1 = {:+.2e}",
                    row.file,
                    ratio - 1.0
                )
                .unwrap();
                if !((ratio - 1.0).abs() <= SPLINE_QUADRATURE) {
                    bad.push(format!(
                        "{}: FreeCAD's geometry has {ratio} times our volume",
                        row.file
                    ));
                }
            }
            None => bad.push(format!("{}: no volume by quadrature", row.file)),
        }
    }

    // FreeCAD measures the same housing at every scale, and calls it
    // valid at every scale (it called the one at 1e6 "self-intersecting"
    // until 2026-10-14, while the knots of its spline edges were written
    // as the lengths they are in the body: see `oil_hole`). Validity is
    // asked of every row below, the housing's among them; this asks that
    // what it measures does not depend on the scale either.
    let housings: Vec<&harness::Row> = rows
        .iter()
        .filter(|r| r.file.ends_with("-housing.step"))
        .collect();
    assert_eq!(housings.len(), 3, "the housing at three scales");
    for row in &housings {
        for (what, a, b) in [
            ("volume", row.volume_ratio, housings[0].volume_ratio),
            ("area", row.area_ratio, housings[0].area_ratio),
            ("centre", row.centre_off, housings[0].centre_off),
        ] {
            if !((a - b).abs() <= 1e-9) {
                bad.push(format!(
                    "{}: FreeCAD's {what} over ours is {a}, and {b} at the first scale",
                    row.file
                ));
            }
        }
    }

    let (mesh_text, mesh_bad) = check_meshes(&files, meshes.as_deref().unwrap());
    report.push_str(&mesh_text);

    let mut listed = Vec::new();
    for row in rows.iter().filter(|r| !r.problems.is_empty()) {
        let line = format!("{}: {}", row.file, row.problems.join("; "));
        match BEYOND_THE_READER
            .iter()
            .find(|(part, _)| row.file.contains(part))
        {
            Some((_, why)) => listed.push(format!("{line}\n  ({why})")),
            None => bad.push(line),
        }
    }
    for line in mesh_bad {
        match BEYOND_THE_READER
            .iter()
            .find(|(part, _)| line.contains(part))
        {
            Some((_, why)) => listed.push(format!("{line}\n  ({why})")),
            None => bad.push(line),
        }
    }
    bad.extend(forms_bad);
    for (title, lines) in [
        ("refused by a command (not exported)", &set.refused),
        ("where our own measurement is missing", &set.unmeasured),
        ("beyond what FreeCAD can judge", &listed),
        ("PROBLEMS", &bad),
    ] {
        if !lines.is_empty() {
            writeln!(report, "\n## {} {title}\n", lines.len()).unwrap();
            for line in lines {
                writeln!(report, "- {line}").unwrap();
            }
        }
    }
    std::fs::write(files.join("report.md"), &report).unwrap();
    println!("{report}\nreport: {}", files.join("report.md").display());
    assert!(
        set.refused.is_empty(),
        "{} documents were refused:\n{}",
        set.refused.len(),
        set.refused.join("\n")
    );
    assert!(set.unmeasured.is_empty(), "{}", set.unmeasured.join("\n"));
    assert!(
        bad.is_empty(),
        "{} problems:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
