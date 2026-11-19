//! Ignored in the gate (a probe: it prints and holds nothing; fold-check
//! 2026-11-19, after the review): run it with
//! `cargo test -p craft3d-engine --test fold_probe -- --ignored --nocapture`.
//!
//! fold-check, step 1: a probe, printed and not held to anything, of what
//! the checks a body gets say of a face folded over itself, whatever made
//! it (docs/design/verification.md › *A face folded over itself*). Each
//! case is built as small as it can be and on bodies of about 3, 100 and
//! 10 000 times its fold, and for each the lines `FOLD <case> …` say what
//! the operation that made it said, then `topo::check`, the deep check
//! (`tess::body_self_intersections` at the relative chord the deep check
//! uses, 1e-3 of the body's diagonal, and finer), the measure, the mesh
//! and each exporter, and the folded face's own regularity (its normal's
//! Bernstein net per span, a dense sample of the sine between its
//! parameter directions). Run with `-- --nocapture`.
//!
//! The fillets whose radius fades along the hull's stem that the review
//! of 2026-11-17 found folded are read from STEP fixtures
//! (`tests/fold_probe/fades/`): the fillet refuses them now.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::neg_cmp_op_on_partial_ord
)]

use craft3d_geom::{Curve3, Line, NurbsCurve, NurbsSurface, Plane, RationalBezierPatch, Surface};
use craft3d_io::{MeshFormat, StepBody, StepOptions, export_mesh, export_step};
use craft3d_kernel::fillet::{Fade, FilletLawEdge, RadiusLaw, fillet_by_law};
use craft3d_kernel::loft::{Options as LoftOptions, Section, loft};
use craft3d_kernel::offset_face::{FaceOffset, offset_faces};
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::sweep::sweep;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_meshio::Unit;
use craft3d_tess::{
    SelfCheck, SelfCheckParams, TessParams, body_self_intersections, tessellate_body,
};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId, check, mass_properties};
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn tolerance(linear: f64) -> Tolerance {
    Tolerance::new(linear, 1e-9).unwrap()
}

fn said(found: &SelfCheck) -> String {
    let first = found.crossings.first().map(|c| {
        format!(
            "faces {:?} {:?} depth {:.4e} margin {:.4e} at ({:.4}, {:.4}, {:.4})",
            c.faces, c.kind, c.depth, c.margin, c.at.x, c.at.y, c.at.z
        )
    });
    let deepest = found
        .crossings
        .iter()
        .map(|c| c.depth)
        .fold(0.0_f64, f64::max);
    format!(
        "certain {} marginal {} touching {} shells-certain {} chord {:.4e} margin {:.4e} passes {} triangles {} degenerate {} deepest {:.4e} first {:?}",
        found.certain_crossings,
        found.marginal_crossings,
        found.touching.len(),
        found.shells.iter().filter(|s| s.certain).count(),
        found.chordal_tolerance,
        found.margin,
        found.passes,
        found.triangles,
        found.degenerate_triangles,
        deepest,
        first
    )
}

/// Whether the Bernstein net of `w³ (S_u × S_v)` lies in an open
/// half-space: the patch's normal is then not zero anywhere on it. Tried
/// along the sum of the coefficients, the sum of their directions, and a
/// perceptron's steps from each (a probe's version of `geom::grid`'s,
/// without its rounding bounds: a coefficient must clear by 1e-12 of the
/// largest).
fn net_clear(patch: &RationalBezierPatch) -> bool {
    let Some(net) = patch.weighted_normal_net() else {
        return false;
    };
    let largest = net.iter().map(|c| c.length()).fold(0.0_f64, f64::max);
    if !(largest > 0.0 && largest.is_finite()) {
        return false;
    }
    let room = 1e-12 * largest;
    let blocked = |d: Vec3| net.iter().position(|c| !(c.dot(d) > room));
    let sum = net.iter().fold(Vec3::ZERO, |s, c| s + *c);
    let units = net
        .iter()
        .filter_map(|c| c.normalized())
        .fold(Vec3::ZERO, |s, c| s + c);
    for start in [sum, units] {
        let Some(mut d) = start.normalized() else {
            continue;
        };
        for _ in 0..32 {
            let Some(k) = blocked(d) else {
                return true;
            };
            let Some(next) = net[k].normalized().and_then(|c| (d + c).normalized()) else {
                break;
            };
            d = next;
        }
        if blocked(d).is_none() {
            return true;
        }
    }
    false
}

/// The regularity of a freeform surface span by span: how many spans the
/// net proves at once, how many after quartering (up to six times), and
/// the pieces left unproven (with the sine between the parameter
/// directions at their middles, the least of them, and where).
fn regularity(surface: &Surface) -> String {
    let nurbs = match surface {
        Surface::Nurbs(n) => n.clone(),
        Surface::Bezier(b) => NurbsSurface::from_bezier(b).unwrap(),
        _ => return "analytic".to_string(),
    };
    let Ok(spans) = nurbs.bezier_spans() else {
        return "no spans".to_string();
    };
    let t = Instant::now();
    let (mut at_once, mut quartered, mut unproven_spans, mut leaves) = (0, 0, 0, 0_usize);
    let mut least: Option<(f64, f64, f64, Vec3)> = None;
    for span in &spans {
        let mut stack = vec![(
            span.patch.clone(),
            (0.0_f64, 1.0_f64),
            (0.0_f64, 1.0_f64),
            0_u32,
        )];
        let mut left = 0_usize;
        while let Some((patch, s, r, depth)) = stack.pop() {
            if net_clear(&patch) {
                continue;
            }
            if depth >= 6 {
                left += 1;
                let u = span.u.0 + (span.u.1 - span.u.0) * 0.5 * (s.0 + s.1);
                let v = span.v.0 + (span.v.1 - span.v.0) * 0.5 * (r.0 + r.1);
                let (p, du, dv) = nurbs.derivatives(u, v);
                let size = du.length() * dv.length();
                let sine = if size > 0.0 {
                    du.cross(dv).length() / size
                } else {
                    0.0
                };
                if least.is_none_or(|l| sine < l.0) {
                    least = Some((sine, u, v, p));
                }
                continue;
            }
            let (sm, rm) = (0.5 * (s.0 + s.1), 0.5 * (r.0 + r.1));
            let (a, b) = patch.split_u(0.5);
            let (aa, ab) = a.split_v(0.5);
            let (ba, bb) = b.split_v(0.5);
            stack.push((aa, (s.0, sm), (r.0, rm), depth + 1));
            stack.push((ab, (s.0, sm), (rm, r.1), depth + 1));
            stack.push((ba, (sm, s.1), (r.0, rm), depth + 1));
            stack.push((bb, (sm, s.1), (rm, r.1), depth + 1));
            if depth == 0 {
                quartered += 1;
            }
        }
        if left > 0 {
            unproven_spans += 1;
            leaves += left;
        }
    }
    at_once += spans.len() - quartered;
    format!(
        "spans {} proven at once {} quartered {} (of which unproven {}, {} pieces left) least sine left {:?} ({:.1} ms)",
        spans.len(),
        at_once,
        quartered,
        unproven_spans,
        leaves,
        least.map(|l| (l.0, l.1, l.2, (l.3.x, l.3.y, l.3.z))),
        ms(t)
    )
}

/// The sine between the parameter directions over a 240 × 240 grid of the
/// surface's domain (its least, where), and how many samples have a unit
/// normal against the mean of all of them.
fn sampled(surface: &Surface) -> String {
    let ((u0, u1), (v0, v1)) = surface.domain();
    let n = 240;
    let mut normals = Vec::with_capacity((n + 1) * (n + 1));
    let mut least = (f64::INFINITY, 0.0, 0.0, Vec3::ZERO);
    for i in 0..=n {
        for j in 0..=n {
            let u = u0 + (u1 - u0) * i as f64 / n as f64;
            let v = v0 + (v1 - v0) * j as f64 / n as f64;
            let (p, du, dv) = surface.derivatives(u, v);
            let size = du.length() * dv.length();
            let normal = du.cross(dv);
            let sine = if size > 0.0 {
                normal.length() / size
            } else {
                0.0
            };
            if sine < least.0 {
                least = (sine, u, v, p);
            }
            if let Some(unit) = normal.normalized() {
                normals.push(unit);
            }
        }
    }
    let mean = normals
        .iter()
        .fold(Vec3::ZERO, |s, c| s + *c)
        .normalized()
        .unwrap_or(Vec3::Z);
    let against = normals.iter().filter(|c| c.dot(mean) < 0.0).count();
    format!(
        "least sine {:.3e} at (u {:.5}, v {:.5}) ({:.4}, {:.4}, {:.4}); {} of {} unit normals against their mean",
        least.0,
        least.1,
        least.2,
        least.3.x,
        least.3.y,
        least.3.z,
        against,
        normals.len()
    )
}

/// One Bézier span as a surface of its own (clamped knots over [0, 1]²).
fn span_surface(patch: &RationalBezierPatch) -> Option<Surface> {
    let (m, n) = patch.degrees();
    let clamped = |d: usize| {
        let mut k = vec![0.0; d + 1];
        k.extend(vec![1.0; d + 1]);
        k
    };
    NurbsSurface::new(
        m,
        n,
        clamped(m),
        clamped(n),
        m + 1,
        n + 1,
        patch.control_points(),
        patch.weights(),
    )
    .ok()
    .map(Surface::Nurbs)
}

/// The surface regular at the tolerance, span by span: its offsets by
/// `+tol` and `−tol` proven not to fold (`geom::offset::regular`, which
/// proves `1 − d κ > 0` for both principal curvatures from Bernstein forms
/// of the span), so no radius of curvature below the tolerance on either
/// side. Per span: proven, a pole (a border drawn into a point: no normal
/// there), a fold proven at a place (with the radius there), not proven.
fn tolerance_regularity(surface: &Surface, tol: f64) -> String {
    let nurbs = match surface {
        Surface::Nurbs(n) => n.clone(),
        Surface::Bezier(b) => NurbsSurface::from_bezier(b).unwrap(),
        _ => return "analytic".to_string(),
    };
    let Ok(spans) = nurbs.bezier_spans() else {
        return "no spans".to_string();
    };
    let t = Instant::now();
    let (mut proven, mut poles, mut unproven, mut other) = (0_usize, 0_usize, 0_usize, 0_usize);
    let mut folds: Vec<String> = Vec::new();
    let mut unproven_at: Vec<String> = Vec::new();
    for span in &spans {
        let Some(own) = span_surface(&span.patch) else {
            other += 1;
            continue;
        };
        let global = |at: (f64, f64)| {
            (
                span.u.0 + (span.u.1 - span.u.0) * at.0,
                span.v.0 + (span.v.1 - span.v.0) * at.1,
            )
        };
        let mut verdict = Ok(());
        for d in [tol, -tol] {
            verdict = craft3d_geom::offset::regular(&own, d);
            if verdict.is_err() {
                break;
            }
        }
        match verdict {
            Ok(()) => proven += 1,
            Err(craft3d_geom::offset::OffsetError::NoNormal { .. }) => poles += 1,
            Err(craft3d_geom::offset::OffsetError::Folds { at, radius }) => {
                let (u, v) = global(at);
                let p = nurbs.eval(u, v);
                if folds.len() < 6 {
                    folds.push(format!(
                        "(u {u:.5} v {v:.5} at ({:.4}, {:.4}, {:.4}) radius {radius:.3e})",
                        p.x, p.y, p.z
                    ));
                } else {
                    folds.push(String::new());
                }
            }
            Err(craft3d_geom::offset::OffsetError::Unproven { at }) => {
                unproven += 1;
                let (u, v) = global(at);
                let p = nurbs.eval(u, v);
                if unproven_at.len() < 4 {
                    unproven_at.push(format!(
                        "(u {u:.5} v {v:.5} at ({:.4}, {:.4}, {:.4}))",
                        p.x, p.y, p.z
                    ));
                }
            }
            Err(_) => other += 1,
        }
    }
    let shown: Vec<&String> = folds.iter().filter(|f| !f.is_empty()).collect();
    format!(
        "spans {} proven {proven} poles {poles} folds {} {:?} unproven {unproven} {:?} other {other} ({:.1} ms)",
        spans.len(),
        folds.len(),
        shown,
        unproven_at,
        ms(t)
    )
}

/// [`tolerance_regularity`] side by side: the offset by the tolerance out
/// of the material (the face's outward side: where it folds, the face
/// bends towards the outside more tightly than the tolerance, a concave
/// bend: two layers with air between them closer than the tolerance) and
/// into it (a convex bend: an edge of material thinner than the
/// tolerance), each asked of every span; the folds of each, and the first
/// places.
fn by_side(surface: &Surface, tol: f64, reversed: bool) -> String {
    let nurbs = match surface {
        Surface::Nurbs(n) => n.clone(),
        Surface::Bezier(b) => NurbsSurface::from_bezier(b).unwrap(),
        _ => return "analytic".to_string(),
    };
    let Ok(spans) = nurbs.bezier_spans() else {
        return "no spans".to_string();
    };
    let outward = if reversed { -tol } else { tol };
    let mut said = String::new();
    for (side, d) in [
        ("out of the material (concave)", outward),
        ("into it (convex)", -outward),
        ("out by half the tolerance", 0.5 * outward),
        ("out by a quarter", 0.25 * outward),
    ] {
        let (mut folds, mut no_normal, mut unproven) = (0_usize, 0_usize, 0_usize);
        let mut places: Vec<String> = Vec::new();
        for span in &spans {
            let Some(own) = span_surface(&span.patch) else {
                continue;
            };
            match craft3d_geom::offset::regular(&own, d) {
                Err(craft3d_geom::offset::OffsetError::Folds { at, radius }) => {
                    folds += 1;
                    let u = span.u.0 + (span.u.1 - span.u.0) * at.0;
                    let v = span.v.0 + (span.v.1 - span.v.0) * at.1;
                    let p = nurbs.eval(u, v);
                    if places.len() < 3 {
                        places.push(format!(
                            "({:.4}, {:.4}, {:.4}) radius {radius:.3e}",
                            p.x, p.y, p.z
                        ));
                    }
                }
                Err(craft3d_geom::offset::OffsetError::NoNormal { .. }) => no_normal += 1,
                Err(craft3d_geom::offset::OffsetError::Unproven { .. }) => unproven += 1,
                _ => {}
            }
        }
        let _ = std::fmt::Write::write_fmt(
            &mut said,
            format_args!(
                "{side}: folds {folds} {places:?} no normal {no_normal} unproven {unproven}; "
            ),
        );
    }
    said
}

/// The least radius of curvature over a 600 × 120 grid of the surface's
/// domain, leaving out samples within four tolerances of a tip (a sample
/// where the parameter directions are parallel to 1e-6 or one vanishes):
/// where, how far from the nearest tip, and how many samples are below
/// the tolerance and below ten tolerances.
fn curvature_scan(surface: &Surface, tol: f64) -> String {
    let ((u0, u1), (v0, v1)) = surface.domain();
    let (nu, nv) = (600, 120);
    let mut tips: Vec<Vec3> = Vec::new();
    let mut samples: Vec<(f64, f64, f64, Vec3)> = Vec::with_capacity((nu + 1) * (nv + 1));
    for i in 0..=nu {
        for j in 0..=nv {
            let u = u0 + (u1 - u0) * i as f64 / nu as f64;
            let v = v0 + (v1 - v0) * j as f64 / nv as f64;
            let (p, du, dv) = surface.derivatives(u, v);
            let size = du.length() * dv.length();
            let sine = if size > 0.0 {
                du.cross(dv).length() / size
            } else {
                0.0
            };
            if !(sine > 1e-6) {
                if tips.iter().all(|t| t.distance(p) > tol) && tips.len() < 64 {
                    tips.push(p);
                }
                continue;
            }
            let Some((k, h)) = craft3d_geom::curvature::curvatures(surface, u, v) else {
                continue;
            };
            let most = h.abs() + (h * h - k).max(0.0).sqrt();
            let radius = if most > 0.0 {
                1.0 / most
            } else {
                f64::INFINITY
            };
            samples.push((radius, u, v, p));
        }
    }
    let near_tip = |p: Vec3| tips.iter().any(|t| t.distance(p) <= 4.0 * tol);
    let mut least = (f64::INFINITY, 0.0, 0.0, Vec3::ZERO);
    let (mut below, mut below_ten) = (0_usize, 0_usize);
    for &(radius, u, v, p) in &samples {
        if near_tip(p) {
            continue;
        }
        if radius < tol {
            below += 1;
        }
        if radius < 10.0 * tol {
            below_ten += 1;
        }
        if radius < least.0 {
            least = (radius, u, v, p);
        }
    }
    let to_tip = tips
        .iter()
        .map(|t| t.distance(least.3))
        .fold(f64::INFINITY, f64::min);
    format!(
        "least radius {:.3e} ({:.3e} tolerances) at (u {:.5} v {:.5}) ({:.4}, {:.4}, {:.4}), {:.3e} from the nearest of {} tips; samples below the tolerance {below}, below ten {below_ten}, of {}",
        least.0,
        least.0 / tol,
        least.1,
        least.2,
        least.3.x,
        least.3.y,
        least.3.z,
        to_tip,
        tips.len(),
        samples.len()
    )
}

/// Everything the probe says of one body: `faces` are the faces whose
/// regularity is printed (the folded ones; every freeform face when
/// empty).
fn report(case: &str, body: &Body, tol: &Tolerance, faces: &[usize], finest: f64) {
    let diagonal = body.bbox().map(|b| b.diagonal()).unwrap_or(f64::NAN);
    println!(
        "FOLD {case} body: {} faces, diagonal {diagonal:.6e}, tolerance {:e}",
        body.faces.len(),
        tol.linear
    );
    let t = Instant::now();
    let defects = check(body, tol);
    println!(
        "FOLD {case} check: {} defects, first {:?} ({:.1} ms)",
        defects.len(),
        defects.first(),
        ms(t)
    );
    let mut rel = 1e-3;
    while rel >= finest * 0.999 {
        let params = SelfCheckParams {
            relative_chord: rel,
            ..SelfCheckParams::new(tol.linear)
        };
        let t = Instant::now();
        match body_self_intersections(body, &params) {
            Ok(found) => println!(
                "FOLD {case} deep rel {rel:.0e}: {} ({:.0} ms)",
                said(&found),
                ms(t)
            ),
            Err(e) => println!(
                "FOLD {case} deep rel {rel:.0e}: error {e} ({:.0} ms)",
                ms(t)
            ),
        }
        rel *= 0.1;
    }
    let t = Instant::now();
    match mass_properties(body) {
        Ok(m) => println!(
            "FOLD {case} measure: volume {:.10e} area {:.10e} ({:.0} ms)",
            m.volume,
            m.area,
            ms(t)
        ),
        Err(e) => println!("FOLD {case} measure: error {e:?} ({:.0} ms)", ms(t)),
    }
    let chord = 1e-3 * diagonal;
    let t = Instant::now();
    match tessellate_body(
        body,
        &TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        },
    ) {
        Ok(mesh) => {
            let welded = mesh.weld();
            let (closed, volume) = match &welded {
                Ok(w) => (w.is_closed(), w.volume()),
                Err(_) => (false, f64::NAN),
            };
            println!(
                "FOLD {case} mesh at {chord:.3e}: {} triangles, welded {}, closed {closed}, volume {volume:.10e} ({:.0} ms)",
                mesh.triangles.len(),
                welded.is_ok(),
                ms(t)
            );
            for format in [MeshFormat::StlBinary, MeshFormat::ThreeMf, MeshFormat::Obj] {
                match export_mesh(&mesh, "probe", Unit::Millimeter, format) {
                    Ok(bytes) => println!("FOLD {case} export {format:?}: {} bytes", bytes.len()),
                    Err(e) => println!("FOLD {case} export {format:?}: error {e}"),
                }
            }
        }
        Err(e) => println!(
            "FOLD {case} mesh at {chord:.3e}: error {e} ({:.0} ms)",
            ms(t)
        ),
    }
    let options = StepOptions {
        tolerance: *tol,
        ..StepOptions::default()
    };
    match export_step(&[StepBody::new("probe", body)], &options) {
        Ok((bytes, _)) => {
            println!("FOLD {case} export STEP: {} bytes", bytes.len());
            imported(case, &bytes, tol);
        }
        Err(e) => println!("FOLD {case} export STEP: error {e}"),
    }
    for (index, face) in body.faces.iter().enumerate() {
        let freeform = matches!(face.surface, Surface::Nurbs(_) | Surface::Bezier(_));
        if !(faces.contains(&index) || (faces.is_empty() && freeform)) {
            continue;
        }
        println!(
            "FOLD {case} face {index} reversed {} regularity: {}",
            face.reversed,
            regularity(&face.surface)
        );
        if freeform {
            println!(
                "FOLD {case} face {index} sampled: {}",
                sampled(&face.surface)
            );
            println!(
                "FOLD {case} face {index} at the tolerance: {}",
                tolerance_regularity(&face.surface, tol.linear)
            );
            println!(
                "FOLD {case} face {index} curvature: {}",
                curvature_scan(&face.surface, tol.linear)
            );
            println!(
                "FOLD {case} face {index} by side: {}",
                by_side(&face.surface, tol.linear, face.reversed)
            );
        }
    }
}

/// What `file.import` does with the STEP file the body was written as:
/// read it, hold it to `topo::check` and the deep check of the whole body
/// (`regen::imported`).
fn imported(case: &str, bytes: &[u8], tol: &Tolerance) {
    let options = craft3d_stepread::Options {
        metres_per_unit: 1e-3,
        tolerance: *tol,
        loosest: 1000.0 * tol.linear,
    };
    let t = Instant::now();
    let read = craft3d_stepread::parse(bytes).and_then(|parsed| parsed.build(0, &options));
    match read {
        Ok(read) => {
            let defects = check(&read.body, tol);
            let deep = body_self_intersections(&read.body, &SelfCheckParams::new(tol.linear));
            println!(
                "FOLD {case} STEP read back: {} faces, check {} defects {:?}, deep {} ({:.0} ms)",
                read.body.faces.len(),
                defects.len(),
                defects.first(),
                match deep {
                    Ok(found) => said(&found),
                    Err(e) => format!("error {e}"),
                },
                ms(t)
            );
        }
        Err(e) => println!("FOLD {case} STEP read back: refused: {e} ({:.0} ms)", ms(t)),
    }
}

// ---------------------------------------------------------------------
// The fillet that fades along the hull's stem.

fn hull() -> (Body, Tolerance) {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fold_probe/trimmed.step");
    let bytes = std::fs::read(path).unwrap();
    let tol = tolerance(0.02);
    let options = craft3d_stepread::Options {
        metres_per_unit: 1e-3,
        tolerance: tol,
        loosest: 1000.0 * tol.linear,
    };
    (
        craft3d_stepread::parse(&bytes)
            .unwrap()
            .build(0, &options)
            .unwrap()
            .body,
        tol,
    )
}

/// The stem: of the edges that end at the foot (5, 0, −8.9), the one that
/// rises; and whether the foot is its start.
fn stem(body: &Body) -> (EdgeId, bool) {
    let foot = Vec3::new(5.0, 0.0, -8.9);
    let mut found = None;
    for (i, e) in body.edges.iter().enumerate() {
        let (a, b) = (
            body.vertices[e.start.0 as usize].point,
            body.vertices[e.end.0 as usize].point,
        );
        for (here, there, at_start) in [(a, b, true), (b, a, false)] {
            if here.distance(foot) < 0.5 && there.z > here.z + 20.0 {
                found = Some((EdgeId(i as u32), at_start));
            }
        }
    }
    found.unwrap()
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn the_hulls_sides_at_the_forefoot() {
    // How the radius of curvature of the hull's lofted sides grows with
    // the distance from the forefoot (5, 0, −8.9), the vertex where the
    // probe's rule claimed a fold on them: for every freeform face that
    // has that vertex, a fine grid over the corner of its domain nearest
    // it, binned by distance in tolerances.
    let (body, tol) = hull();
    let foot = Vec3::new(5.0, 0.0, -8.9);
    let t = tol.linear;
    for (index, face) in body.faces.iter().enumerate() {
        if !matches!(face.surface, Surface::Nurbs(_) | Surface::Bezier(_)) {
            continue;
        }
        let ((u0, u1), (v0, v1)) = face.surface.domain();
        let Ok((fu, fv)) = face.surface.project(foot) else {
            continue;
        };
        if face.surface.eval(fu, fv).distance(foot) > 1e-3 {
            continue;
        }
        // Bins: below 1, 2, 4, …, 256 tolerances from the vertex.
        let mut least = [f64::INFINITY; 10];
        let mut sine_least = [f64::INFINITY; 10];
        let n = 800;
        for i in 0..=n {
            for j in 0..=n {
                let u = (fu + (u1 - u0) * 0.05 * (2.0 * i as f64 / n as f64 - 1.0)).clamp(u0, u1);
                let v = (fv + (v1 - v0) * 0.05 * (2.0 * j as f64 / n as f64 - 1.0)).clamp(v0, v1);
                let (p, du, dv) = face.surface.derivatives(u, v);
                let d = p.distance(foot) / t;
                let bin = if d < 1.0 {
                    0
                } else {
                    ((d.plog2().floor() as usize) + 1).min(9)
                };
                let size = du.length() * dv.length();
                if size > 0.0 {
                    sine_least[bin] = sine_least[bin].min(du.cross(dv).length() / size);
                }
                let Some((k, h)) = craft3d_geom::curvature::curvatures(&face.surface, u, v) else {
                    continue;
                };
                let most = h.abs() + (h * h - k).max(0.0).sqrt();
                if most > 0.0 {
                    least[bin] = least[bin].min(1.0 / most / t);
                }
            }
        }
        println!(
            "FOLD forefoot face {index} at (u {fu:.6}, v {fv:.6}): least radius in tolerances by distance (<1, 1-2, 2-4, …, ≥256 t): {:?}; least sine: {:?}",
            least.map(|r| (r * 1000.0).round() / 1000.0),
            sine_least.map(|s| (s * 1e6).round() / 1e6)
        );
    }
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn the_fade_on_the_hulls_stem() {
    // The review's seven folded fades, as the fillet built them with its
    // own refusal of a face that turns back switched off (STEP fixtures,
    // written on `check/fold-check` 15954a14, Check 44328620802).
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fold_probe/fades");
    for name in [
        "fade-3-6-0.33",
        "fade-3-6-0.5",
        "fade-4-8-0.33",
        "fade-4-8-0.5",
        "fade-5-10-1",
        "fade-3-3-2",
        "fade-4-8-1",
    ] {
        let tol = tolerance(0.02);
        let options = craft3d_stepread::Options {
            metres_per_unit: 1e-3,
            tolerance: tol,
            loosest: 1000.0 * tol.linear,
        };
        let bytes = std::fs::read(dir.join(format!("{name}.step"))).unwrap();
        let body = craft3d_stepread::parse(&bytes)
            .unwrap()
            .build(0, &options)
            .unwrap()
            .body;
        report(name, &body, &tol, &[], 1e-5);
    }
    // The sound law beside them, built with the refusal on.
    let (body, tol) = hull();
    let (edge, foot_at_start) = stem(&body);
    let fade = Some(Fade::tip(15.0, 0.33));
    let law = RadiusLaw {
        radius: 3.0,
        fades: if foot_at_start {
            [fade, None]
        } else {
            [None, fade]
        },
    };
    match fillet_by_law(&body, &[FilletLawEdge { edge, law }], &tol) {
        Ok(made) => report("fade r3 over15 back0.33", &made.body, &tol, &[], 1e-5),
        Err(e) => println!("FOLD fade r3 over15 back0.33 op: refused: {e}"),
    }
}

// ---------------------------------------------------------------------
// A block whose top is a bicubic spline over a net of 13 × 13 points, the
// middle of which is pulled about: a fitted face, as a file brings one.

const N: usize = 13;

/// Clamped uniform knots for `N` control points of degree 3.
fn knots() -> Vec<f64> {
    let inner = N - 4;
    let mut k = vec![0.0; 4];
    for i in 1..=inner {
        k.push(i as f64 / (inner + 1) as f64);
    }
    k.extend([1.0; 4]);
    k
}

/// The places of the net's points along one side of length `l`: seven
/// `s` apart about the middle, the others spread out to the ends.
fn axis(l: f64, s: f64) -> [f64; N] {
    let c = 0.5 * l;
    let a = c - 4.0 * s;
    [
        0.0,
        0.5 * a,
        a,
        c - 3.0 * s,
        c - 2.0 * s,
        c - s,
        c,
        c + s,
        c + 2.0 * s,
        c + 3.0 * s,
        c + 4.0 * s,
        c + 4.0 * s + 0.5 * a,
        l,
    ]
}

/// How much of the pull each row of the net takes: none at the borders.
const ROWS: [f64; N] = [
    0.0, 0.0, 0.0, 0.3, 0.7, 1.0, 1.0, 1.0, 0.7, 0.3, 0.0, 0.0, 0.0,
];

#[derive(Clone, Copy, Debug)]
enum Pull {
    /// The middle of a row pulled back over itself and up: each row in
    /// the middle crosses itself in a loop (plan/fold-check/scripts/
    /// roof_design.py), a row at the loop's birth comes to a cusp.
    Loop,
    /// The middle of a row pulled back while it rises: an overhang, each
    /// row a simple curve running back and up, regular everywhere. Not a
    /// fold: a valid solid whose face's normal turns against its
    /// neighbours'.
    Overhang,
    /// A smooth bump, for an offset past its curvature.
    Bump,
}

/// The block `l` square and `l / 3` high, its top pulled about over a
/// middle of `6 s`.
fn roof_block(l: f64, s: f64, pull: Pull) -> Body {
    let xs = axis(l, s);
    let h = l / 3.0;
    let (mut dx, mut dz) = ([0.0; N], [0.0; N]);
    match pull {
        Pull::Loop => {
            dx[5] = 2.2 * s;
            dx[7] = -2.2 * s;
            dz[5] = 1.2 * s;
            dz[6] = 2.0 * s;
            dz[7] = 1.2 * s;
        }
        Pull::Overhang => {
            dx[5] = 1.6 * s;
            dx[7] = -1.6 * s;
            let rise = [0.0, 0.3, 0.7, 1.1, 1.5, 1.8, 2.0, 2.0, 2.0, 2.0];
            for (k, r) in rise.iter().enumerate() {
                dz[3 + k] = r * s;
            }
        }
        Pull::Bump => {
            dz[5] = 0.5 * s;
            dz[6] = 1.0 * s;
            dz[7] = 0.5 * s;
        }
    }
    let mut net = Vec::with_capacity(N * N);
    for i in 0..N {
        for j in 0..N {
            let a = ROWS[j];
            net.push(Vec3::new(xs[i] + a * dx[i], xs[j], h + a * dz[i]));
        }
    }
    let surface =
        NurbsSurface::new(3, 3, knots(), knots(), N, N, net.clone(), vec![1.0; N * N]).unwrap();
    let at = |i: usize, j: usize| net[i * N + j];
    let mut body = Body::new(BodyKind::Solid);
    let ground = [(0.0, 0.0), (l, 0.0), (l, l), (0.0, l)];
    let low: Vec<VertexId> = ground
        .iter()
        .map(|&(x, y)| body.add_vertex(Vec3::new(x, y, 0.0)))
        .collect();
    let high: Vec<VertexId> = [(0, 0), (N - 1, 0), (N - 1, N - 1), (0, N - 1)]
        .iter()
        .map(|&(i, j)| body.add_vertex(at(i, j)))
        .collect();
    let cubic =
        |points: Vec<Vec3>| Curve3::Nurbs(NurbsCurve::polynomial(3, knots(), points).unwrap());
    let along_u = |j: usize| cubic((0..N).map(|i| at(i, j)).collect());
    let along_v = |i: usize| cubic((0..N).map(|j| at(i, j)).collect());
    let roof = [
        (
            body.add_edge(along_u(0), (0.0, 1.0), high[0], high[1]),
            true,
        ),
        (
            body.add_edge(along_v(N - 1), (0.0, 1.0), high[1], high[2]),
            true,
        ),
        (
            body.add_edge(along_u(N - 1), (0.0, 1.0), high[3], high[2]),
            false,
        ),
        (
            body.add_edge(along_v(0), (0.0, 1.0), high[0], high[3]),
            false,
        ),
    ];
    let straight = |body: &mut Body, from: VertexId, to: VertexId| -> EdgeId {
        let (a, b) = (
            body.vertex(from).unwrap().point,
            body.vertex(to).unwrap().point,
        );
        body.add_edge(
            Curve3::Line(Line::through(a, b).unwrap()),
            (0.0, a.distance(b)),
            from,
            to,
        )
    };
    let floor = [
        straight(&mut body, low[0], low[1]),
        straight(&mut body, low[1], low[2]),
        straight(&mut body, low[2], low[3]),
        straight(&mut body, low[3], low[0]),
    ];
    let up = [
        straight(&mut body, low[0], high[0]),
        straight(&mut body, low[1], high[1]),
        straight(&mut body, low[2], high[2]),
        straight(&mut body, low[3], high[3]),
    ];
    let coedge = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
    let mut faces: Vec<FaceId> = Vec::new();
    for k in 0..4 {
        let next = (k + 1) % 4;
        let lp = body.add_loop(vec![
            coedge(floor[k], false),
            coedge(up[next], false),
            coedge(roof[k].0, roof[k].1),
            coedge(up[k], true),
        ]);
        let start = Vec3::new(ground[k].0, ground[k].1, 0.0);
        let direction = Vec3::new(ground[next].0, ground[next].1, 0.0) - start;
        let frame = Frame::new(start, direction.cross(Vec3::Z), direction).unwrap();
        faces.push(body.add_face(Surface::Plane(Plane::new(frame)), false, vec![lp]));
    }
    let floor_loop = body.add_loop(floor.iter().rev().map(|&e| coedge(e, true)).collect());
    let floor_frame = Frame::new(Vec3::ZERO, -Vec3::Z, Vec3::X).unwrap();
    faces.push(body.add_face(
        Surface::Plane(Plane::new(floor_frame)),
        false,
        vec![floor_loop],
    ));
    let roof_loop = body.add_loop(roof.iter().map(|&(e, along)| coedge(e, !along)).collect());
    faces.push(body.add_face(Surface::Nurbs(surface), false, vec![roof_loop]));
    body.add_shell(faces);
    body
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn a_fitted_top_pulled_over_itself() {
    // The fold is the middle, 6 s wide; the bodies are 18 s (3 times the
    // fold), 600 s (100 times) and 60 000 s (10 000 times) square; one
    // tolerance (1e-3 s) throughout, as one document has.
    let s = 1.0;
    let tol = tolerance(1e-3 * s);
    for pull in [Pull::Loop, Pull::Overhang] {
        for times in [3.0, 100.0, 10_000.0] {
            let l = times * 6.0 * s;
            let body = roof_block(l, s, pull);
            report(&format!("roof {pull:?} x{times}"), &body, &tol, &[5], 1e-5);
        }
    }
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn an_offset_past_the_curvature_of_a_bump() {
    let s = 1.0;
    let tol = tolerance(1e-3 * s);
    for times in [3.0, 100.0] {
        let body = roof_block(times * 6.0 * s, s, Pull::Bump);
        for distance in [0.5 * s, 10.0 * s] {
            let case = format!("offset bump x{times} by {distance}");
            let t = Instant::now();
            match offset_faces(
                &body,
                &[FaceOffset {
                    face: FaceId(5),
                    distance,
                }],
                &tol,
            ) {
                Ok(made) => {
                    println!("FOLD {case} op: built ({:.0} ms)", ms(t));
                    report(&case, &made.body, &tol, &[], 1e-4);
                }
                Err(e) => println!("FOLD {case} op: refused: {e} ({:.0} ms)", ms(t)),
            }
        }
    }
}

// ---------------------------------------------------------------------
// A loft of two circles, the second turned half a turn against the first:
// every ruling passes through the axis at one height between them, so the
// side passes through a point inside each face and turns inside out
// beyond it (its normal vanishes along a row of its parameters that the
// loft's own sampling of nine rows does not meet).

fn quarter_arcs(first: u64, radius: f64) -> Vec<SketchCurve> {
    let points = [[radius, 0.0], [0.0, radius], [-radius, 0.0], [0.0, -radius]];
    (0..4)
        .map(|k| SketchCurve {
            id: first + k as u64,
            curve: Curve2::Arc {
                center: [0.0, 0.0],
                start: points[k],
                end: points[(k + 1) % 4],
            },
        })
        .collect()
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn a_loft_through_its_axis() {
    for k in [1.0, 100.0, 10_000.0] {
        let tol = tolerance(1e-6 * k);
        let (a, b, h) = (1.0 * k, 1.2 * k, 2.0 * k);
        let low = detect_profiles(&quarter_arcs(1, a), &tol).unwrap();
        let high = detect_profiles(&quarter_arcs(11, b), &tol).unwrap();
        let sections = [
            Section {
                plane: Frame::WORLD,
                outline: low[0].outer.clone(),
                start: None,
            },
            Section {
                plane: Frame::new(Vec3::new(0.0, 0.0, h), Vec3::Z, Vec3::X).unwrap(),
                outline: high[0].outer.clone(),
                start: Some(13),
            },
        ];
        let case = format!("loft through its axis x{k}");
        let t = Instant::now();
        match loft(
            &sections,
            &LoftOptions {
                ruled: true,
                closed: false,
            },
            &tol,
        ) {
            Ok(made) => {
                println!("FOLD {case} op: built ({:.0} ms)", ms(t));
                report(&case, &made.body, &tol, &[], 1e-4);
            }
            Err(e) => println!("FOLD {case} op: refused: {e} ({:.0} ms)", ms(t)),
        }
    }
}

// ---------------------------------------------------------------------
// Sweeps of a circle along a cubic that crosses itself (plan/fold-check/
// scripts/sweep_path.py: length 2.1958 at scale 1, least radius of
// curvature 0.10541): a profile wider than that radius folds (the sweep's
// own refusal); a thin one passes through itself where the path crosses.

const LOOP_PATH: [[f64; 2]; 4] = [
    [0.0, 0.0],
    [0.0, 1.886_796],
    [-1.165_998, 0.0212],
    [0.529_999, 0.847_998],
];

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn sweeps_along_a_path_that_crosses_itself() {
    let length = 2.195_798_812_420_925;
    for (scale, times) in [
        (1.0, 18.0),
        (1.0, 30.0),
        (1.0, 100.0),
        (100.0, 100.0),
        (1.0, 10_000.0),
    ] {
        let tol = tolerance(1e-6 * scale);
        let rho = scale * length / times;
        let profile = detect_profiles(
            &[SketchCurve {
                id: 1,
                curve: Curve2::Circle {
                    center: [0.0, 0.0],
                    radius: rho,
                },
            }],
            &tol,
        )
        .unwrap();
        let path = [SketchCurve {
            id: 2,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: LOOP_PATH
                    .iter()
                    .map(|p| [p[0] * scale, p[1] * scale])
                    .collect(),
            }),
        }];
        // The path's plane: x along the world's x, its y along the world's
        // z, so the path leaves the profile's plane (z = 0) square to it.
        let path_plane = Frame::new(Vec3::ZERO, Vec3::new(0.0, -1.0, 0.0), Vec3::X).unwrap();
        let case = format!("sweep loop length/profile {times} scale {scale}");
        let t = Instant::now();
        match sweep(&profile[0], &Frame::WORLD, &path, &path_plane, &tol) {
            Ok(made) => {
                println!("FOLD {case} op: built ({:.0} ms)", ms(t));
                report(&case, &made.body, &tol, &[], 1e-4);
            }
            Err(e) => println!("FOLD {case} op: refused: {e} ({:.0} ms)", ms(t)),
        }
    }
}
