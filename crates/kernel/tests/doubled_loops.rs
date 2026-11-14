//! Faces whose loops go round their circle twice, or run back over
//! themselves, built by hand (no operation makes them since the plate's
//! corner rule of 2026-11-14, `docs/design/plates.md` › *Corners*): what
//! every part of the net says about them, the topological check, the
//! tessellator, the self-check and the measure (`docs/design/topology.md`
//! › *Loops that are simple*). The check must refuse every one of them,
//! naming the face and the loop; the rows print what the rest of the net
//! says, so the table in the design document can be read again.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::NurbsCurve;
use craft3d_math::{Frame, Portable, Tolerance, Transform};
use craft3d_testkit::rings::{
    band, degrees, disc, disc_with_spur, lid, plate_with_hole, spline_disc, spline_disc_back,
    spline_lid_back, spline_spike,
};
use craft3d_topo::{Body, Defect, check, mass_properties};
use std::f64::consts::PI;

struct Row {
    name: &'static str,
    body: Body,
    /// Whether the check must refuse it: a sheet may have a cut, whose two
    /// sides run over each other opposite ways.
    refused: bool,
    /// The closed form the body would have if its loops went round once:
    /// `(volume, area)`; a sheet's volume is 0.
    closed: (f64, f64),
}

fn rows() -> Vec<Row> {
    let f = Frame::WORLD;
    let once = degrees(&[0.0, 120.0, 240.0, 360.0]);
    let twice = degrees(&[0.0, 240.0, 480.0, 720.0]);
    let long = degrees(&[0.0, 120.0, 479.4, 720.0]);
    let back = degrees(&[0.0, 359.4, 300.0, 360.0]);
    let disc_closed = (PI, 4.0 * PI);
    let plate_closed = (16.0 - PI, 48.0);
    // The spike's face measures as the region without it.
    let spike_area = mass_properties(&spline_spike(&f, 1.0, false).unwrap())
        .unwrap()
        .area;
    vec![
        Row {
            name: "disc, both faces' loops twice round (3 arcs of 240 deg)",
            body: disc(&f, 1.0, 1.0, &twice, &twice).unwrap(),
            closed: disc_closed,
            refused: true,
        },
        Row {
            name: "disc, only the lid's loop twice round",
            body: disc(&f, 1.0, 1.0, &twice, &once).unwrap(),
            closed: disc_closed,
            refused: true,
        },
        Row {
            name: "disc, arcs of 120, 359.4 and 240.6 deg (twice round)",
            body: disc(&f, 1.0, 1.0, &long, &long).unwrap(),
            closed: disc_closed,
            refused: true,
        },
        Row {
            name: "disc, once round with an arc of 359.4 deg run back over",
            body: disc(&f, 1.0, 1.0, &back, &once).unwrap(),
            closed: disc_closed,
            refused: true,
        },
        Row {
            name: "plate with a hole wound twice (3 arcs of 240 deg)",
            body: plate_with_hole(&f, 4.0, 1.0, 1.0, &twice, &twice).unwrap(),
            closed: plate_closed,
            refused: true,
        },
        Row {
            name: "plane sheet, its loop twice round (3 arcs of 240 deg)",
            body: lid(&f, 1.0, &twice).unwrap(),
            closed: (0.0, PI),
            refused: true,
        },
        Row {
            name: "plane sheet, once round with an arc of 359.4 deg run back over",
            body: lid(&f, 1.0, &back).unwrap(),
            closed: (0.0, PI),
            refused: true,
        },
        Row {
            name: "cylinder band sheet, both loops twice round the axis",
            body: band(&f, 1.0, 1.0, &twice, &twice).unwrap(),
            closed: (0.0, 2.0 * PI),
            refused: true,
        },
        Row {
            name: "disc bounded by two whole spline circles (twice round)",
            body: spline_disc(&f, 1.0, 1.0, 2).unwrap(),
            closed: disc_closed,
            refused: true,
        },
        Row {
            name: "disc whose lid's spline circle goes nearly round, back and on (once round)",
            body: spline_disc_back(&f, 1.0, 1.0).unwrap(),
            closed: disc_closed,
            refused: true,
        },
        Row {
            name: "plane sheet, the same spline loop",
            body: spline_lid_back(&f, 1.0).unwrap(),
            closed: (0.0, PI),
            refused: true,
        },
        Row {
            // One edge used twice by one loop is a cut along an edge the
            // face holds on both sides, as sheet metal's bridged annuli
            // are at one place of their plane: right, and not judged.
            name: "disc whose lid runs out along a line and back (a spur: not judged)",
            body: disc_with_spur(&f, 1.0, 1.0).unwrap(),
            closed: disc_closed,
            refused: false,
        },
        Row {
            name: "sheet with a spline spike out and back (a slit: a sheet may have it)",
            body: spline_spike(&f, 1.0, true).unwrap(),
            closed: (0.0, spike_area),
            refused: false,
        },
    ]
}

/// One line of the table: what each part of the net says.
fn report(row: &Row) -> (Vec<Defect>, String) {
    let tol = Tolerance::default();
    let defects = check(&row.body, &tol);
    let checked = if defects.is_empty() {
        "passes".to_string()
    } else {
        format!(
            "{} defect(s): {}",
            defects.len(),
            defects
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | ")
        )
    };
    let mass = match mass_properties(&row.body) {
        Ok(m) => format!(
            "volume {:.6} ({:.4}x), area {:.6} ({:.4}x)",
            m.volume,
            if row.closed.0 == 0.0 {
                f64::NAN
            } else {
                m.volume / row.closed.0
            },
            m.area,
            m.area / row.closed.1
        ),
        Err(e) => format!("refused: {e}"),
    };
    let params = craft3d_tess::TessParams {
        chordal_tolerance: 1e-3,
        ..craft3d_tess::TessParams::default()
    };
    let mesh = match craft3d_tess::tessellate_body(&row.body, &params) {
        Ok(m) => {
            let area: f64 = m
                .triangles
                .iter()
                .map(|t| {
                    let [a, b, c] = t.map(|i| m.positions[i as usize]);
                    0.5 * (b - a).cross(c - a).length()
                })
                .sum();
            format!("meshed, {} triangles, area {area:.6}", m.triangles.len())
        }
        Err(e) => format!("refused: {e}"),
    };
    let selfcheck = match craft3d_tess::body_self_intersections(
        &row.body,
        &craft3d_tess::SelfCheckParams::new(tol.linear),
    ) {
        Ok(found) if found.is_clean() => format!("clean ({} marginal)", found.marginal_crossings),
        Ok(found) => format!("finds: {found}"),
        Err(e) => format!("refused: {e}"),
    };
    (
        defects,
        format!(
            "{}\n  check: {checked}\n  measure: {mass}\n  tessellator: {mesh}\n  self-check: {selfcheck}",
            row.name
        ),
    )
}

#[test]
fn every_part_of_the_net_on_loops_that_are_not_simple() {
    let mut wrong = Vec::new();
    for row in rows() {
        let (defects, line) = report(&row);
        println!("{line}");
        let caught = defects
            .iter()
            .any(|d| matches!(d, Defect::LoopNotSimple { .. }));
        if caught != row.refused {
            wrong.push(row.name);
        }
    }
    assert!(
        wrong.is_empty(),
        "refused where it must pass or passed where it must be refused: {wrong:?}"
    );
}

#[test]
fn the_valid_twins_pass_the_whole_net() {
    let f = Frame::WORLD;
    let tol = Tolerance::default();
    let once = degrees(&[0.0, 120.0, 240.0, 360.0]);
    for body in [
        disc(&f, 1.0, 1.0, &once, &once).unwrap(),
        plate_with_hole(&f, 4.0, 1.0, 1.0, &once, &once).unwrap(),
        band(&f, 1.0, 1.0, &once, &once).unwrap(),
        spline_disc(&f, 1.0, 1.0, 1).unwrap(),
        spline_spike(&f, 1.0, false).unwrap(),
    ] {
        assert_eq!(check(&body, &tol), vec![]);
        mass_properties(&body).unwrap();
        let found = craft3d_tess::body_self_intersections(
            &body,
            &craft3d_tess::SelfCheckParams::new(tol.linear),
        )
        .unwrap();
        assert!(found.is_clean(), "{found}");
    }
}

/// A helical strip of `turns` turns on a cylinder of radius `scale` (a
/// thread's crest), pitch `scale` and `0.3 scale` wide: its low and high
/// sides each one spline edge (`single`) or one per half turn, joined by
/// two lines, the whole moved by `place`; the face is the strip the right
/// way round (its loop counter-clockwise about the outward normal) unless
/// `reversed`, which flips the face and leaves the loop: a face that would
/// be the whole cylinder outside its loop, the wrong way round. With the
/// tolerance of a document at that scale (the helices fitted within a tenth
/// of it).
fn helical_strip(
    scale: f64,
    place: &Transform,
    turns: f64,
    (single_low, single_high): (bool, bool),
    reversed: bool,
) -> (Body, Tolerance) {
    use craft3d_geom::{Curve3, Cylinder, Line, Surface};
    use craft3d_kernel::helix::Helix;
    use craft3d_math::Vec3;
    use craft3d_topo::{BodyKind, Coedge, EdgeId};
    let tol = Tolerance::new(Tolerance::default().linear * scale, 1e-9).unwrap();
    let (pitch, w) = (scale, 0.3 * scale);
    let z = Vec3::new(0.0, 0.0, 1.0);
    let x = Vec3::new(1.0, 0.0, 0.0);
    let piece = |z0: f64, start: Vec3, t: f64| {
        let hx = Helix::new(z * z0, z, start * scale, scale, pitch, t, false).unwrap();
        let c = hx.curve(tol.linear * 0.1).unwrap().curve;
        let range = c.domain();
        (Curve3::Nurbs(c), range)
    };
    let line = |body: &mut Body, a, b| {
        let (pa, pb): (Vec3, Vec3) = (
            body.vertices[a as usize].point,
            body.vertices[b as usize].point,
        );
        body.add_edge(
            Curve3::Line(Line::through(pa, pb).unwrap()),
            (0.0, pa.distance(pb)),
            craft3d_topo::VertexId(a),
            craft3d_topo::VertexId(b),
        )
    };
    let mut body = Body::new(BodyKind::Sheet);
    let side = |body: &mut Body, z0: f64, single: bool| -> Vec<EdgeId> {
        if single {
            let (c, r) = piece(z0, x, turns);
            let a = body.add_vertex(c.eval(r.0));
            let b = body.add_vertex(c.eval(r.1));
            vec![body.add_edge(c, r, a, b)]
        } else {
            let mut out = Vec::new();
            let mut prev = None;
            for k in 0..(2.0 * turns).round() as usize {
                let start = if k % 2 == 0 { x } else { x * -1.0 };
                let (c, r) = piece(z0 + 0.5 * pitch * k as f64, start, 0.5);
                let a = prev.unwrap_or_else(|| body.add_vertex(c.eval(r.0)));
                let b = body.add_vertex(c.eval(r.1));
                out.push(body.add_edge(c, r, a, b));
                prev = Some(b);
            }
            out
        }
    };
    let low = side(&mut body, 0.0, single_low);
    let high = side(&mut body, w, single_high);
    let start_of = |body: &Body, e: EdgeId| body.edges[e.0 as usize].start.0;
    let end_of = |body: &Body, e: EdgeId| body.edges[e.0 as usize].end.0;
    let (l0, l1) = (start_of(&body, low[0]), end_of(&body, *low.last().unwrap()));
    let (h0, h1) = (
        start_of(&body, high[0]),
        end_of(&body, *high.last().unwrap()),
    );
    let up = line(&mut body, l1, h1);
    let down = line(&mut body, h0, l0);
    let forward = |edge| Coedge {
        edge,
        reversed: false,
    };
    let mut coedges: Vec<Coedge> = low.iter().map(|e| forward(*e)).collect();
    coedges.push(forward(up));
    coedges.extend(high.iter().rev().map(|e| Coedge {
        edge: *e,
        reversed: true,
    }));
    coedges.push(forward(down));
    let lp = body.add_loop(coedges);
    let face = body.add_face(
        Surface::Cylinder(Cylinder::new(Frame::WORLD, scale).unwrap()),
        reversed,
        vec![lp],
    );
    body.add_shell(vec![face]);
    (body.transformed(place).unwrap(), tol)
}

/// Where the strips are built: in the world, and turned and moved off it.
fn places(scale: f64) -> [Transform; 2] {
    use craft3d_math::Vec3;
    [
        Transform::from_frame(&Frame::WORLD),
        Transform::from_frame(
            &Frame::new(
                Vec3::new(0.3, -1.7, 2.1) * scale,
                Vec3::new(0.2, -0.5, 0.84),
                Vec3::new(0.9, 0.3, 0.1),
            )
            .unwrap(),
        ),
    ]
}

const SIDES: [(bool, bool); 4] = [(false, false), (true, false), (false, true), (true, true)];

/// A helical strip is read the way it goes round, whatever its sides are
/// made of (`docs/design/topology.md` › *Loops that bound a region*): the
/// right way round it passes the whole check, the wrong way round (the face
/// flipped, its loop kept) the region rule refuses it, by the strip's own
/// area (its width times the length of its sides in the cylinder's
/// parameters, `0.3 · 2π · turns`) with the sign that says so. A side that
/// is one spline edge of many turns was followed in eight steps, each more
/// than a turn, which the chart read as going a few turns back: until
/// 2026-11-14 the right way round with both sides single edges was refused
/// at 10 and 12 turns ("every loop runs clockwise", areas -1.131e1 and
/// -7.540e0), and at 15 with one side single ("goes round the cylinder 16
/// times"), and the wrong way round passed at 8 to 20 turns with one side
/// or both single. Now each spline edge is followed in as many steps as its
/// control points show it needs to move less than a quarter turn from one
/// point to the next. Three scales, in the world and turned and moved.
#[test]
fn a_helical_strip_is_read_the_way_it_goes_round() {
    let mut wrong = Vec::new();
    for scale in [1e-3, 1.0, 1e6] {
        for place in places(scale) {
            for turns in [1.0, 3.0, 8.0, 10.0, 12.0, 15.0, 20.0, 64.0] {
                for sides in SIDES {
                    let (right, tol) = helical_strip(scale, &place, turns, sides, false);
                    let found = check(&right, &tol);
                    if !found.is_empty() {
                        wrong.push(format!(
                            "right way, scale {scale}, {turns} turns, single {sides:?}: {found:?}"
                        ));
                    }
                    let (flipped, tol) = helical_strip(scale, &place, turns, sides, true);
                    let found = check(&flipped, &tol);
                    if !refused_by_its_area(&found, 0.3 * 2.0 * PI * turns * scale) {
                        wrong.push(format!(
                            "wrong way, scale {scale}, {turns} turns, single {sides:?}: {found:?}"
                        ));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Whether `found` holds the region rule's refusal of a face whose one loop
/// runs clockwise about its outward normal, enclosing `area` in the
/// surface's parameters (as the rule prints it, to four figures).
fn refused_by_its_area(found: &[Defect], area: f64) -> bool {
    found.iter().any(|d| {
        let text = d.to_string();
        matches!(d, Defect::LoopsBoundNoRegion { .. })
            && text.contains("every loop runs clockwise")
            && text
                .split("areas -")
                .nth(1)
                .and_then(|rest| rest.split(')').next())
                .and_then(|a| a.parse::<f64>().ok())
                .is_some_and(|a| (a - area).abs() <= 1e-3 * area)
    })
}

/// A helical strip of ten turns on a cylinder (a thread's crest), one side
/// or both one spline edge, is not taken for a loop that goes round often
/// (review of 2026-11-14: followed in eight steps per edge, a half step of
/// 225° on the single edge, it read "goes round the cylinder 16 times").
/// Since each spline edge on a surface that closes about an axis is
/// followed in steps shown to move less than a quarter turn, the walk goes
/// round with it; `a_helical_strip_is_read_the_way_it_goes_round` holds
/// both orientations to the whole check.
#[test]
fn a_long_helical_strip_is_not_taken_for_a_loop_that_goes_round_often() {
    let world = Transform::from_frame(&Frame::WORLD);
    for sides in [(false, false), (true, false), (true, true)] {
        for reversed in [false, true] {
            let (body, tol) = helical_strip(1.0, &world, 10.0, sides, reversed);
            let found: Vec<String> = check(&body, &tol)
                .iter()
                .filter(|d| matches!(d, Defect::LoopNotSimple { .. }))
                .map(ToString::to_string)
                .collect();
            assert!(
                found.is_empty(),
                "single {sides:?}, reversed {reversed}: {found:?}"
            );
        }
    }
}

/// A spiral on the cone of radius `r0` at height 0 widening by `slope` a
/// unit of height about the world's z axis, going `quarters` quarter turns
/// from the height `z0`, rising `pitch` a turn: each quarter a rational
/// cubic, the rational quadratic quarter circle times a radius linear in
/// its parameter, at the height that radius has on the cone, so the curve
/// lies on the cone exactly.
fn cone_spiral(r0: f64, slope: f64, z0: f64, pitch: f64, quarters: usize) -> NurbsCurve {
    use craft3d_math::Vec3;
    use std::f64::consts::FRAC_PI_2;
    let mut ctrl: Vec<Vec3> = Vec::new();
    let mut weights: Vec<f64> = Vec::new();
    let mut knots = vec![0.0; 4];
    let radius_at = |angle: f64| r0 + slope * (z0 + pitch * angle / (2.0 * PI));
    for k in 0..quarters {
        let (t0, t1) = (k as f64 * FRAC_PI_2, (k + 1) as f64 * FRAC_PI_2);
        let mid = 0.5 * (t0 + t1);
        // The quarter circle, homogeneous: numerators and weights.
        let a = [t0.psin_cos(), mid.psin_cos(), t1.psin_cos()];
        let w = [1.0, 0.5 * 2.0_f64.sqrt(), 1.0];
        let b = [radius_at(t0), radius_at(t1)];
        let z = [(b[0] - r0) / slope, (b[1] - r0) / slope];
        // A quadratic Bernstein polynomial times a linear one.
        let product = |f: [f64; 3], g: [f64; 2]| {
            [
                f[0] * g[0],
                (2.0 * f[1] * g[0] + f[0] * g[1]) / 3.0,
                (f[2] * g[0] + 2.0 * f[1] * g[1]) / 3.0,
                f[2] * g[1],
            ]
        };
        let xs = product([a[0].1, a[1].1, a[2].1], b);
        let ys = product([a[0].0, a[1].0, a[2].0], b);
        let zs = product(w, z);
        let ws = product(w, [1.0, 1.0]);
        for j in usize::from(k > 0)..4 {
            ctrl.push(Vec3::new(xs[j] / ws[j], ys[j] / ws[j], zs[j] / ws[j]));
            weights.push(ws[j]);
        }
        let knot = (k + 1) as f64;
        knots.extend(if k + 1 < quarters {
            vec![knot; 3]
        } else {
            vec![knot; 4]
        });
    }
    NurbsCurve::new(3, knots, ctrl, weights).unwrap()
}

/// The helical strip of `a_helical_strip_is_read_the_way_it_goes_round` on
/// a cone (a tapered thread's crest: a pipe thread's taper, 1 in 32 on the
/// radius, and a steep one), each side one spline edge of `turns` turns
/// lying on the cone exactly, joined by lines along the cone: the right way
/// round it passes the whole check, the wrong way round the region rule
/// refuses it by the strip's area in the cone's parameters (`0.3 · 2π ·
/// turns`, the high side the low one raised 0.3). At the scale 1e-3 the
/// flipped strip may pass instead: these sides' parameter runs unevenly in
/// angle (a rational quarter circle's), and the rule takes the middle of a
/// step's distance from its chord in `(u, v)`, along the chord included, as
/// its polyline's error, beside which an area of radians times a length that
/// small cannot be told from nothing (it passes such a face, as it did before
/// 2026-11-14; `docs/design/topology.md` › *A helix of many turns as one
/// edge*). A refusal there is still by the strip's area, and nothing else.
#[test]
fn a_tapered_helical_strip_is_read_the_way_it_goes_round() {
    use craft3d_geom::{Cone, Curve3, Line, Surface};
    use craft3d_topo::{BodyKind, Coedge, VertexId};
    let mut wrong = Vec::new();
    for scale in [1e-3, 1.0, 1e6] {
        let tol = Tolerance::new(Tolerance::default().linear * scale, 1e-9).unwrap();
        for slope in [1.0 / 32.0, 0.5] {
            for turns in [1_usize, 10, 15, 64] {
                for reversed in [false, true] {
                    let (r0, pitch, w) = (scale, scale, 0.3 * scale);
                    let low = cone_spiral(r0, slope, 0.0, pitch, 4 * turns);
                    let high = cone_spiral(r0, slope, w, pitch, 4 * turns);
                    let mut body = Body::new(BodyKind::Sheet);
                    let mut side = |c: NurbsCurve| {
                        let r = c.domain();
                        let a = body.add_vertex(c.eval(r.0));
                        let b = body.add_vertex(c.eval(r.1));
                        (body.add_edge(Curve3::Nurbs(c), r, a, b), a, b)
                    };
                    let (lo, l0, l1) = side(low);
                    let (hi, h0, h1) = side(high);
                    let mut line = |a: VertexId, b: VertexId| {
                        let (pa, pb) = (
                            body.vertices[a.0 as usize].point,
                            body.vertices[b.0 as usize].point,
                        );
                        body.add_edge(
                            Curve3::Line(Line::through(pa, pb).unwrap()),
                            (0.0, pa.distance(pb)),
                            a,
                            b,
                        )
                    };
                    let up = line(l1, h1);
                    let down = line(h0, l0);
                    let coedge = |edge, reversed| Coedge { edge, reversed };
                    let lp = body.add_loop(vec![
                        coedge(lo, false),
                        coedge(up, false),
                        coedge(hi, true),
                        coedge(down, false),
                    ]);
                    let cone = Cone::new(Frame::WORLD, r0, slope.patan()).unwrap();
                    let face = body.add_face(Surface::Cone(cone), reversed, vec![lp]);
                    body.add_shell(vec![face]);
                    let found = check(&body, &tol);
                    let what = format!("scale {scale}, slope {slope}, {turns} turns");
                    if reversed {
                        let area = 0.3 * 2.0 * PI * turns as f64 * scale;
                        let small = scale < 1e-2 && found.is_empty();
                        if !(refused_by_its_area(&found, area) || small) {
                            wrong.push(format!("wrong way, {what}: {found:?}"));
                        }
                    } else if !found.is_empty() {
                        wrong.push(format!("right way, {what}: {found:?}"));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
