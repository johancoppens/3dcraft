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

use craft3d_math::{Frame, Tolerance};
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

/// A helical strip of ten turns on a cylinder (a thread's crest), one side
/// or both one spline edge: the loop is followed in eight steps per edge,
/// a half step of 225° on the single edge, which the chart cannot unwrap
/// (it read "goes round the cylinder 16 times"; review of 2026-11-14). A
/// loop whose half step moves more than a quarter period is not held to
/// how often it goes round: refused here only where the region rule
/// refuses it on `main` too (both sides single edges, the strip the right
/// way round: "every loop runs clockwise", owed in `topology.md`).
#[test]
fn a_long_helical_strip_is_not_taken_for_a_loop_that_goes_round_often() {
    use craft3d_geom::{Curve3, Cylinder, Line, Surface};
    use craft3d_kernel::helix::Helix;
    use craft3d_math::Vec3;
    use craft3d_topo::{BodyKind, Coedge, EdgeId};
    let tol = Tolerance::default();
    let (pitch, turns, w) = (1.0, 10.0, 0.3);
    let z = Vec3::new(0.0, 0.0, 1.0);
    let x = Vec3::new(1.0, 0.0, 0.0);
    let piece = |z0: f64, start: Vec3, t: f64| {
        let hx = Helix::new(z * z0, z, start, 1.0, pitch, t, false).unwrap();
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
    for (single_a, single_b) in [(false, false), (true, false), (true, true)] {
        for reversed in [false, true] {
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
                    for k in 0..(2.0 * turns) as usize {
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
            let low = side(&mut body, 0.0, single_a);
            let high = side(&mut body, w, single_b);
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
                Surface::Cylinder(Cylinder::new(Frame::WORLD, 1.0).unwrap()),
                reversed,
                vec![lp],
            );
            body.add_shell(vec![face]);
            let found: Vec<String> = check(&body, &tol)
                .iter()
                .filter(|d| matches!(d, Defect::LoopNotSimple { .. }))
                .map(ToString::to_string)
                .collect();
            assert!(
                found.is_empty(),
                "single {single_a} {single_b}, reversed {reversed}: {found:?}"
            );
        }
    }
}
