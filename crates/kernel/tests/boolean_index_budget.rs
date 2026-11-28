//! A freeform face whose Bézier spans and their index would take more than
//! one face may (`craft3d_kernel::boolean::MAX_INDEX_BYTES`, worked out from
//! its knots and degrees) is refused before any of that memory is taken
//! (docs/design/booleans.md › *A face of 2¹⁸ spans*): a sheet of degree
//! 15 × 15 just over the budget, cut by a block, with the process's peak
//! memory held to what the sheet itself takes. One test a process, so that
//! the peak is this test's. Just under the budget the same face is cut
//! (ignored: it takes a gigabyte; run in release).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{
    BooleanOp, MAX_INDEX_BYTES, TOO_LARGE_INDEX, boolean, index_bytes, index_bytes_of, span_count,
};
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge};

/// The process's peak resident memory so far, in MiB (Linux; not a number
/// elsewhere).
fn peak_mib() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kib| kib.parse::<f64>().ok())
        })
        .map_or(f64::NAN, |kib| kib / 1024.0)
}

/// A gently rolling sheet of degree `degree` both ways over `spans` × `spans`
/// uniform knot spans, 100 × 100 at height 5, as a sheet body of one face
/// bounded by its four borders.
fn sheet(degree: usize, spans: usize) -> Body {
    let n = spans + degree;
    let mut knots = vec![0.0; degree + 1];
    for k in 1..spans {
        knots.push(k as f64 / spans as f64);
    }
    knots.extend(std::iter::repeat_n(1.0, degree + 1));
    let mut ctrl = Vec::with_capacity(n * n);
    for i in 0..n {
        for j in 0..n {
            let (x, y) = (
                100.0 * i as f64 / (n - 1) as f64,
                100.0 * j as f64 / (n - 1) as f64,
            );
            ctrl.push(Vec3::new(
                x,
                y,
                5.0 + 0.5 * (0.1 * x).psin() * (0.07 * y).pcos(),
            ));
        }
    }
    let surface = NurbsSurface::new(
        degree,
        degree,
        knots.clone(),
        knots.clone(),
        n,
        n,
        ctrl.clone(),
        vec![1.0; n * n],
    )
    .unwrap();
    // The borders: control point (i, j) at i * n + j, i along u.
    let border = |along_u: bool, at: usize| {
        let points: Vec<Vec3> = (0..n)
            .map(|k| {
                if along_u {
                    ctrl[k * n + at]
                } else {
                    ctrl[at * n + k]
                }
            })
            .collect();
        NurbsCurve::polynomial(degree, knots.clone(), points).unwrap()
    };
    let corner = |i: usize, j: usize| ctrl[i * n + j];
    let mut body = Body::new(BodyKind::Sheet);
    let c00 = body.add_vertex(corner(0, 0));
    let c10 = body.add_vertex(corner(n - 1, 0));
    let c01 = body.add_vertex(corner(0, n - 1));
    let c11 = body.add_vertex(corner(n - 1, n - 1));
    let mut coedges = Vec::new();
    for (curve, start, end, back) in [
        (border(true, 0), c00, c10, false),
        (border(false, n - 1), c10, c11, false),
        (border(true, n - 1), c01, c11, true),
        (border(false, 0), c00, c01, true),
    ] {
        let range = curve.domain();
        let edge = body.add_edge(Curve3::Nurbs(curve), range, start, end);
        coedges.push(Coedge {
            edge,
            reversed: back,
        });
    }
    let lp = body.add_loop(coedges);
    let face = body.add_face(Surface::Nurbs(surface), false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// The most knot spans a side of a square degree-15 sheet may have for
/// its spans and index to fit the budget, and one more.
fn sides() -> (usize, usize) {
    let fits = |side: usize| index_bytes_of(side * side, 15, 15) <= MAX_INDEX_BYTES;
    let mut side = 1;
    while fits(side + 1) {
        side += 1;
    }
    (side, side + 1)
}

/// Just over the budget: refused, naming the face, before its spans (a
/// gigabyte) are made: the peak memory grows by less than a tenth of what
/// they would take.
#[test]
fn a_face_just_over_the_index_budget_is_refused_before_its_memory_is_taken() {
    let (under, over) = sides();
    assert!(under * under > craft3d_kernel::boolean::MAX_FREEFORM_SPANS / 8);
    let tool = sheet(15, over);
    let surface = &tool.faces[0].surface;
    let bytes = index_bytes(surface).unwrap();
    assert_eq!(span_count(surface), Some(over * over));
    assert!(bytes > MAX_INDEX_BYTES, "{bytes}");
    assert!(index_bytes_of(under * under, 15, 15) <= MAX_INDEX_BYTES);
    let block = craft3d_kernel::primitives::block(&Frame::WORLD, 100.0, 100.0, 10.0).unwrap();
    let tolerance = Tolerance::new(1e-3, 1e-6).unwrap();
    let before = peak_mib();
    let refused = boolean(&block, &tool, BooleanOp::Intersect, &tolerance);
    let after = peak_mib();
    match refused {
        Err(KernelError::Unsupported { reason, faces, .. }) => {
            assert_eq!(reason, TOO_LARGE_INDEX);
            assert_eq!(faces.len(), 1, "{faces:?}");
        }
        other => panic!("not refused for its index: {other:?}"),
    }
    let would = bytes as f64 / f64::from(1 << 20);
    println!(
        "degree 15, {over} × {over} spans: {would:.0} MiB would be taken; peak {before:.0} MiB before, {after:.0} after"
    );
    if before.is_finite() && after.is_finite() {
        assert!(after - before < 0.1 * would, "{before} -> {after} MiB");
    } else {
        // The peak is read from `/proc/self/status`, which only Linux
        // has: macOS and Windows gave `NaN -> NaN MiB` (public CI,
        // 2026-11-23 and 2026-11-27), and NaN fails every comparison.
        // There the refusal above is what is asserted; the memory it
        // spares is measured on Linux, where it must be.
        assert!(
            !cfg!(target_os = "linux"),
            "no peak memory on Linux: {before} -> {after} MiB"
        );
        println!(
            "the peak memory is not measured here (no /proc/self/status): the refusal is asserted"
        );
    }
}

/// Just under the budget the same sheet cuts the block; its peak is about
/// the budget (ignored: a gigabyte; release).
#[test]
#[ignore]
fn a_face_just_under_the_index_budget_is_cut() {
    let (under, _) = sides();
    let tool = sheet(15, under);
    let bytes = index_bytes(&tool.faces[0].surface).unwrap();
    assert!(bytes <= MAX_INDEX_BYTES);
    let block = craft3d_kernel::primitives::block(&Frame::WORLD, 100.0, 100.0, 10.0).unwrap();
    let tolerance = Tolerance::new(1e-3, 1e-6).unwrap();
    let before = peak_mib();
    let t = std::time::Instant::now();
    let cut = boolean(&block, &tool, BooleanOp::Intersect, &tolerance).unwrap();
    println!(
        "degree 15, {under} × {under} spans: {:.0} MiB counted; cut in {:.1} s to {} faces; peak {before:.0} MiB before, {:.0} after",
        bytes as f64 / f64::from(1 << 20),
        t.elapsed().as_secs_f64(),
        cut.body.faces.len(),
        peak_mib()
    );
}
