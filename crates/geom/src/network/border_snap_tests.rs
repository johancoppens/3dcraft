//! Where `iso_track_along` cuts an edge's line (finding 7 of the class-A
//! tank, 2026-10-31): an end a rounding short of the face's border is cut
//! on the border; an end that really lies inside keeps its cut, and the
//! rows describe only that piece; an end within the tolerance and within
//! `BORDER_SNAP` of the range is put on the border (its rows run to it, the
//! difference being the position gap); an end within the tolerance in
//! space but far inside in parameter, where the line stands still near the
//! border, keeps its cut. At 1e-3, 1 and 1e6.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::{BORDER_SNAP, Raw, Track, iso_rows, iso_track_along, point};
use crate::NurbsSurface;
use crate::refine::piece_raw;
use craft3d_math::Vec3;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// A gently curved patch of size `k`, degree 3 along u on `knots_u` (its
/// control columns `columns(i)` in x) and degree 2 across v.
fn patch(k: f64, knots_u: Vec<f64>, columns: &[f64]) -> NurbsSurface {
    let nu = columns.len();
    let mut points = Vec::new();
    for &x in columns {
        for j in 0..3 {
            let y = f64::from(j) / 2.0;
            points.push(Vec3::new(x, y, 0.1 * x * y + 0.05 * x * x) * k);
        }
    }
    NurbsSurface::new(
        3,
        2,
        knots_u,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        nu,
        3,
        points,
        vec![1.0; nu * 3],
    )
    .unwrap()
}

fn regular(k: f64) -> NurbsSurface {
    patch(
        k,
        vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
        &[0.0, 0.25, 0.5, 0.75, 1.0],
    )
}

/// The line along u at `v` of `s`, cut to `[from, to]`, as an edge.
fn edge(s: &NurbsSurface, v: f64, from: f64, to: f64) -> crate::NurbsCurve {
    let (line, _) = iso_rows(s, false, v).unwrap();
    piece_raw(line, from, to).unwrap().curve().unwrap()
}

/// The rows' line and the line of the face at the same `at`, whole.
fn rows_and_line(s: &NurbsSurface, e: &crate::NurbsCurve, tol: f64) -> (Raw, Raw) {
    let track = Track::new(e, tol).unwrap();
    let (rows, along_v, at) = iso_track_along(s, &track, tol).unwrap().unwrap();
    assert!(!along_v, "the edge runs along u");
    let [x0, _, _] = rows;
    let (line, _) = iso_rows(s, along_v, at).unwrap();
    (x0, line)
}

fn last(r: &Raw) -> Vec3 {
    point(*r.points.last().unwrap()).unwrap()
}

/// Case 1: an end a rounding short of the border (3e-12 of the range,
/// beyond `SAME_KNOT` but far within `BORDER_SNAP`) is cut on the border:
/// the rows are the face's line, control point for control point.
#[test]
fn an_end_a_rounding_short_of_the_border_is_cut_on_it() {
    for k in SCALES {
        let s = regular(k);
        let tol = 1e-6 * k;
        let e = edge(&s, 0.4, 0.0, 1.0 - 3e-12);
        let (x0, line) = rows_and_line(&s, &e, tol);
        assert_eq!(x0.points, line.points, "at {k}");
        assert_eq!(x0.knots, line.knots, "at {k}");
    }
}

/// Case 2: an end far more than the tolerance inside the face keeps its
/// cut, and the rows describe that piece only: they end where the edge
/// does, not at the border.
#[test]
fn an_end_inside_the_face_keeps_its_cut() {
    for k in SCALES {
        let s = regular(k);
        let tol = 1e-6 * k;
        let e = edge(&s, 0.4, 0.0, 0.7);
        let (x0, line) = rows_and_line(&s, &e, tol);
        let end = e.control_points().last().copied().unwrap();
        assert!(last(&x0).distance(end) <= tol, "at {k}");
        assert!(last(&line).distance(end) > 100.0 * tol, "at {k}");
    }
}

/// Case 3: an end within the tolerance of the border and within
/// `BORDER_SNAP` of the range from it is put on the border: its rows run
/// to the border (the line whole), and the edge's end lies within the
/// tolerance of theirs (the position gap the measure bounds). One a
/// hundred times further in parameter, still within the tolerance in
/// space, keeps its cut.
#[test]
fn an_end_within_the_tolerance_of_the_border_is_put_on_it_within_the_snap() {
    for k in SCALES {
        let s = regular(k);
        let tol = 1e-6 * k;
        let gap = 0.1 * BORDER_SNAP;
        let e = edge(&s, 0.4, 0.0, 1.0 - gap);
        let (x0, line) = rows_and_line(&s, &e, tol);
        assert_eq!(x0.points, line.points, "at {k}");
        let end = e.control_points().last().copied().unwrap();
        let moved = last(&x0).distance(end);
        assert!(moved > 0.0 && moved <= tol, "at {k}: {moved:e}");
        // Beyond the snap in parameter: the cut stays.
        let e = edge(&s, 0.4, 0.0, 1.0 - 10.0 * BORDER_SNAP);
        let end = e.control_points().last().copied().unwrap();
        assert!(last(&line).distance(end) <= tol, "at {k}: still within tol");
        let (x0, line) = rows_and_line(&s, &e, tol);
        assert_ne!(x0.points, line.points, "at {k}");
        assert!(last(&x0).distance(end) <= 1e-3 * tol, "at {k}");
    }
}

/// A border where the line stands still: the last four control columns
/// are one, so along u the line is constant from 0.75 to the border. An
/// edge ending at 0.74 is within the tolerance of the border's point in
/// space, but a quarter of the range from it in parameter: it keeps its
/// cut (the snap by distance alone put it on the border and took the
/// standing piece into the rows).
#[test]
fn a_border_where_the_line_stands_still_does_not_pull_an_end() {
    for k in SCALES {
        let s = patch(
            k,
            vec![0.0, 0.0, 0.0, 0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 1.0],
            &[0.0, 0.25, 0.5, 0.75, 0.75, 0.75, 0.75],
        );
        let tol = 1e-6 * k;
        let e = edge(&s, 0.4, 0.0, 0.745);
        let end = e.control_points().last().copied().unwrap();
        let (line, _) = iso_rows(&s, false, 0.4).unwrap();
        assert!(
            last(&line).distance(end) <= tol,
            "at {k}: the end is within the tolerance of the border ({:e})",
            last(&line).distance(end)
        );
        let (x0, line) = rows_and_line(&s, &e, tol);
        assert!(
            x0.points.len() < line.points.len(),
            "at {k}: the rows run on to the border ({} of {})",
            x0.points.len(),
            line.points.len()
        );
    }
}
