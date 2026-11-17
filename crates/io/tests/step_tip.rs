//! A fillet that ends in a point, through STEP and back: the fitted
//! rational face whose last row is one point (the fade's tip,
//! docs/design/blends.md) is written, read again (`craft3d-stepread`) and
//! is the same solid: its counts, its volume and its area, valid, at the
//! scales 1e-3, 1 and 1e6, the tip at a vertex of the block and short of
//! it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_io::{StepBody, StepOptions, export_step};
use craft3d_kernel::fillet::{Fade, FilletLawEdge, RadiusLaw, fillet_by_law};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_stepread::{Options, parse};
use craft3d_topo::{Body, EdgeId, check, mass_properties};

/// The block 40 × 30 × 40 on a tilted frame whose upright edge is rounded
/// 3, the radius running to nothing over 15, `back` short of its top.
fn tipped(k: f64, back: f64, tolerance: &Tolerance) -> Body {
    let frame = Frame::new(
        Vec3::new(3.0, -2.0, 5.0) * k,
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(1.0, 0.0, 0.2),
    )
    .unwrap();
    let body = primitives::block(&frame, 40.0 * k, 30.0 * k, 40.0 * k).unwrap();
    let middle = frame.point(0.0, 0.0, 20.0 * k);
    let (index, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            (
                i,
                e.curve.eval(0.5 * (e.range.0 + e.range.1)).distance(middle),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    let edge = EdgeId(index as u32);
    let data = body.edge(edge).unwrap();
    let up = (data.curve.eval(data.range.1) - data.curve.eval(data.range.0)).dot(frame.z()) > 0.0;
    let fade = Some(Fade::tip(15.0 * k, back * k));
    let law = RadiusLaw {
        radius: 3.0 * k,
        fades: if up { [None, fade] } else { [fade, None] },
    };
    fillet_by_law(&body, &[FilletLawEdge { edge, law }], tolerance)
        .unwrap()
        .body
}

#[test]
fn a_fillet_that_ends_in_a_point_reads_back_the_same_solid() {
    for k in [1e-3, 1.0, 1e6] {
        let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for back in [0.0, 5.0] {
            let what = format!("back {back} at {k}");
            let body = tipped(k, back, &tolerance);
            let options = StepOptions {
                tolerance,
                ..StepOptions::default()
            };
            let (bytes, _) = export_step(&[StepBody::new("tipped", &body)], &options)
                .unwrap_or_else(|e| panic!("{what}: export: {e}"));
            let parsed = parse(&bytes).unwrap_or_else(|e| panic!("{what}: parse: {e}"));
            assert_eq!(parsed.items().len(), 1, "{what}");
            let read = Options {
                metres_per_unit: 1e-3,
                tolerance,
                loosest: 1000.0 * tolerance.linear,
            };
            let done = parsed
                .build(0, &read)
                .unwrap_or_else(|e| panic!("{what}: read: {e}"));
            assert_eq!(done.report.unproven_edges, 0, "{what}");
            assert_eq!(check(&done.body, &tolerance), [], "{what}");
            assert_eq!(
                (
                    done.body.faces.len(),
                    done.body.edges.len(),
                    done.body.vertices.len()
                ),
                (body.faces.len(), body.edges.len(), body.vertices.len()),
                "{what}"
            );
            let (was, is) = (
                mass_properties(&body).unwrap(),
                mass_properties(&done.body).unwrap(),
            );
            let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * a.abs().max(b.abs());
            assert!(
                close(was.volume, is.volume) && close(was.area, is.area),
                "{what}: volume {} for {}, area {} for {}",
                is.volume,
                was.volume,
                is.area,
                was.area
            );
            println!(
                "{what}: {} bytes, volume off {:.1e}, area off {:.1e} of themselves",
                bytes.len(),
                (was.volume - is.volume).abs() / was.volume,
                (was.area - is.area).abs() / was.area
            );
        }
    }
}
