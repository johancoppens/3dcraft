//! Oracle 8 (docs/testing.md) for STEP: FreeCAD, an independent kernel,
//! reads the exact B-rep we export and measures it.
//!
//! Opt-in, because it needs FreeCAD (`cargo xtask freecad` runs it):
//!
//! ```sh
//! CRAFT3D_FREECAD=/path/to/FreeCAD.AppImage cargo test --release -p craft3d-io --test step_freecad -- --ignored --nocapture
//! ```
//!
//! The bodies here are built directly on the kernel and by hand, which
//! covers what the command layer cannot make yet (spline surfaces,
//! cavities of every kind, sheets); the bodies made by commands are in
//! `crates/engine/tests/step_freecad.rs`. The table is printed and left in
//! `target/tmp/step-io/report.md`.

// `!(x <= bound)` on purpose: a NaN is a problem too.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::neg_cmp_op_on_partial_ord
)]

use craft3d_math::Portable;
#[path = "common/bodies.rs"]
mod bodies;
#[path = "common/step_harness.rs"]
mod harness;

use craft3d_geom::Surface;
use craft3d_io::{AngleUnit, StepBody, StepOptions, export_step};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::{KernelError, primitives};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_meshio::Unit;
use craft3d_testkit::{solids, trimmed};
use craft3d_topo::{Body, BodyKind, mass_properties};
use harness::{APPROXIMATED, Case, Extent, RELATIVE};

/// Sheets made by `extrude::fill`.
mod sheets {
    use craft3d_kernel::extrude::fill;
    use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
    use craft3d_math::{Frame, Tolerance};
    use craft3d_topo::Body;

    /// The square of half side `half` about the origin of `frame`, in its
    /// plane, with a round hole of radius `hole` (none for 0).
    pub fn square(frame: &Frame, half: f64, hole: f64, tol: &Tolerance) -> Body {
        let c = [[-half, -half], [half, -half], [half, half], [-half, half]];
        let mut curves: Vec<SketchCurve> = (0..4)
            .map(|i| SketchCurve {
                id: i as u64 + 1,
                curve: Curve2::Line {
                    start: c[i],
                    end: c[(i + 1) % 4],
                },
            })
            .collect();
        if hole > 0.0 {
            curves.push(SketchCurve {
                id: 5,
                curve: Curve2::Circle {
                    center: [0.0, 0.0],
                    radius: hole,
                },
            });
        }
        let profiles = detect_profiles(&curves, tol).unwrap();
        fill(&profiles, frame, tol).unwrap().body
    }
}

fn tilted(scale: f64) -> Frame {
    Frame::new(
        Vec3::new(1.0, -2.0, 0.5) * scale,
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap()
}

fn case(file: &str, body: &Body, lumps: usize, options: &StepOptions, to_mm: f64) -> Case {
    let (bytes, _) = export_step(&[StepBody::new(file, body)], options)
        .unwrap_or_else(|e| panic!("{file}: {e}"));
    let mass = mass_properties(body).unwrap_or_else(|e| panic!("{file}: {e}"));
    let bbox = body
        .bbox()
        .ok()
        .and_then(|b| b.corners())
        .map(|(lo, hi)| [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z]);
    let solid = body.kind == BodyKind::Solid;
    Case {
        file: format!("{file}.step"),
        bytes,
        to_mm,
        solid,
        solids: if solid { lumps } else { 0 },
        volume: mass.volume,
        area: mass.area,
        centroid: mass.centroid.map(|c| [c.x, c.y, c.z]),
        bbox,
        counts: (
            body.faces.len(),
            body.edges.len(),
            body.vertices.len(),
            body.shells.len(),
        ),
        bbox_slack: 0.0,
        extent: extent(body),
        names: vec![file.to_string()],
        relative: RELATIVE,
        parts: Vec::new(),
    }
}

/// `Body::bbox` is tight for planes, cylinders and cones, and for whole
/// spheres and tori.
fn extent(body: &Body) -> Extent {
    if body
        .faces
        .iter()
        .any(|f| matches!(f.surface, Surface::Torus(_)))
    {
        Extent::Unchecked
    } else if body
        .faces
        .iter()
        .all(|f| matches!(f.surface, Surface::Plane(_)))
    {
        Extent::Exact
    } else if body.faces.iter().all(|f| {
        f.loops.is_empty()
            || matches!(
                f.surface,
                Surface::Plane(_) | Surface::Cylinder(_) | Surface::Cone(_)
            )
    }) {
        Extent::Tight
    } else {
        Extent::Loose
    }
}

fn options(scale: f64) -> StepOptions {
    StepOptions {
        // The tolerance follows the size, as a document's would be set.
        tolerance: Tolerance::new(1e-6 * scale.min(1.0), 1e-9).unwrap(),
        ..StepOptions::default()
    }
}

#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD"]
fn freecad_reads_every_kernel_body_as_the_solid_we_made() {
    let mut cases = Vec::new();
    for (scale, tag) in [(1e-3, "milli"), (1.0, "one"), (1e6, "mega")] {
        let frame = tilted(scale);
        let mut all = solids::all(&frame, scale).unwrap();
        all.extend(trimmed::all(&frame, scale).unwrap());
        all.push(trimmed::sphere_octant(&frame, 2.0 * scale).unwrap());
        all.push(trimmed::pipe_elbow(&frame, 3.0 * scale, 1.0 * scale, 1.2).unwrap());
        for (i, reference) in all.iter().enumerate() {
            let name = format!("{tag}-{i:02}-{}", reference.name.replace(' ', "_"));
            let mut c = case(&name, &reference.body, 1, &options(scale), 1.0);
            // The closed forms, not our integrals, where the kit has them.
            c.volume = reference.volume;
            c.area = reference.area;
            // The one body here with an edge that is not a line in the
            // parameter plane of its face: an ellipse on a cylinder.
            if reference.name == "oblique cylinder" {
                c.relative = APPROXIMATED;
            }
            cases.push(c);
        }
        for (name, body, lumps) in [
            (
                "cavity_sphere",
                bodies::block_with_spherical_cavity(scale),
                1,
            ),
            ("cavity_box_cone", bodies::block_with_two_cavities(scale), 1),
            ("two_lumps", bodies::two_lumps(scale), 2),
            ("nested", bodies::nested(scale), 3),
            (
                "quarter_round",
                bodies::quarter_round(&frame, 1.5 * scale, 2.0 * scale),
                1,
            ),
            ("open_box", bodies::open_box(scale), 0),
            (
                "washer",
                bodies::washer(&frame, 3.0 * scale, 1.0 * scale),
                0,
            ),
            (
                "ellipse_wide",
                bodies::elliptic_disc(&frame, 3.0 * scale, 1.0 * scale),
                0,
            ),
            (
                "ellipse_tall",
                bodies::elliptic_disc(&frame, 1.0 * scale, 3.0 * scale),
                0,
            ),
        ] {
            cases.push(case(
                &format!("{tag}-{name}"),
                &body,
                lumps,
                &options(scale),
                1.0,
            ));
        }
    }
    // Sheets made by the kernel's own operations (a fill, a sheet trimmed
    // by a solid) and a solid cut by a sheet, against closed forms: a
    // square of 20 with a hole of radius 1 (400 − π); the disc a ball of
    // radius 2 leaves of it, its centre 0.5 off the plane (π (4 − 0.25));
    // the disc of radius 0.75 the spline cone leaves of the plane a
    // quarter of the way up; the part of a block 4 × 3 × 2 above the
    // plane a quarter of the way down (volume 6, area 31).
    for (scale, tag) in [(1e-3, "milli"), (1.0, "one"), (1e6, "mega")] {
        let k = scale;
        // The tolerance follows the size, for the booleans and the file.
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let sized = StepOptions {
            tolerance: tol,
            ..StepOptions::default()
        };
        let up = Frame::WORLD.at(Vec3::new(0.0, 0.0, 1.5 * k)).unwrap();
        let plain = sheets::square(&up, 10.0 * k, 0.0, &tol);
        let holed = sheets::square(&up, 10.0 * k, 1.0 * k, &tol);
        let ball = primitives::sphere(
            &Frame::WORLD.at(Vec3::new(0.0, 0.0, 2.0 * k)).unwrap(),
            2.0 * k,
        )
        .unwrap();
        let block = primitives::block(
            &Frame::WORLD.at(Vec3::new(-2.0 * k, -k, 0.0)).unwrap(),
            4.0 * k,
            3.0 * k,
            2.0 * k,
        )
        .unwrap();
        let cone = solids::spline_cone(&Frame::WORLD, 3.0 * k, 4.0 * k)
            .unwrap()
            .body;
        let across = Frame::new(Vec3::new(0.0, k, 0.0), Vec3::Y, Vec3::X).unwrap();
        let wall = sheets::square(&across, 10.0 * k, 0.0, &tol);
        let trimmed_by_ball = boolean(&plain, &ball, BooleanOp::Intersect, &tol)
            .unwrap()
            .body;
        let trimmed_by_cone = boolean(&wall, &cone, BooleanOp::Intersect, &tol)
            .unwrap()
            .body;
        let cut = boolean(&block, &plain, BooleanOp::Subtract, &tol)
            .unwrap()
            .body;
        let pi = std::f64::consts::PI;
        for (name, body, lumps, area, volume) in [
            ("fill_with_hole", &holed, 0, (400.0 - pi) * k * k, 0.0),
            (
                "sheet_in_ball",
                &trimmed_by_ball,
                0,
                pi * (4.0 - 0.25) * k * k,
                0.0,
            ),
            (
                "sheet_in_spline_cone",
                &trimmed_by_cone,
                0,
                pi * 0.5625 * k * k,
                0.0,
            ),
            ("block_cut_by_sheet", &cut, 1, 31.0 * k * k, 6.0 * k * k * k),
        ] {
            assert_eq!(body.kind == BodyKind::Sheet, lumps == 0, "{name}");
            let mut c = case(&format!("{tag}-{name}"), body, lumps, &sized, 1.0);
            c.area = area;
            if lumps > 0 {
                c.volume = volume;
            }
            // The cone's section is bounded by two spline edges (within
            // 2e-10 of the circle, our area the closed form to 1e-9):
            // FreeCAD interpolates their parameter-space curves on import
            // ([`APPROXIMATED`]; it reads the area 5.9e-6 small), and our
            // box of a spline edge is that of its control points.
            if name == "sheet_in_spline_cone" {
                c.relative = APPROXIMATED;
                c.extent = Extent::Loose;
            }
            cases.push(c);
        }
    }
    // A cylinder whose wall and rims are rational splines of four spans
    // joined with full-multiplicity knots (the nine-point circle). Neither
    // integrator is good across such knots: FreeCAD's area of the flat
    // lid, bounded by a curve whose length it gets right to 1e-15, is off
    // by 1.8e-4, its volume by 8e-3 (and `mass_properties` by 2.4e-3:
    // reported). So this body is compared with the closed form, coarsely;
    // the rational one-span `quarter_round` above is the precise check.
    {
        let (r, h) = (1.5, 2.0);
        let body = bodies::nurbs_can(&tilted(1.0), r, h);
        let mut c = case("nurbs_can", &body, 1, &options(1.0), 1.0);
        c.volume = std::f64::consts::PI * r * r * h;
        c.area = std::f64::consts::TAU * r * (r + h);
        c.centroid = None;
        c.relative = 2e-2;
        cases.push(c);
    }
    // Units: the same 40 x 20 x 10 mm block and a cone, from documents in
    // each unit; FreeCAD must report millimetres. And angles in degrees.
    for (unit, name, to_mm) in [
        (Unit::Micron, "um", 1e-3),
        (Unit::Millimeter, "mm", 1.0),
        (Unit::Centimeter, "cm", 10.0),
        (Unit::Meter, "m", 1000.0),
        (Unit::Inch, "in", 25.4),
        (Unit::Foot, "ft", 304.8),
    ] {
        for angle_unit in [AngleUnit::Radian, AngleUnit::Degree] {
            let options = StepOptions {
                length_unit: unit,
                angle_unit,
                tolerance: Tolerance::new(1e-6 / to_mm, 1e-9).unwrap(),
                ..StepOptions::default()
            };
            let block =
                primitives::block(&Frame::WORLD, 40.0 / to_mm, 20.0 / to_mm, 10.0 / to_mm).unwrap();
            let cone = primitives::cone(&tilted(1.0 / to_mm), 20.0 / to_mm, 30.0 / to_mm).unwrap();
            let tag = if angle_unit == AngleUnit::Degree {
                "deg"
            } else {
                "rad"
            };
            let c = case(
                &format!("unit-{name}-{tag}-block"),
                &block,
                1,
                &options,
                to_mm,
            );
            // In millimetres whatever the unit: 40 x 20 x 10.
            assert!((c.volume * to_mm.ppowi(3) - 8000.0).abs() < 1e-6);
            cases.push(c);
            cases.push(case(
                &format!("unit-{name}-{tag}-cone"),
                &cone,
                1,
                &options,
                to_mm,
            ));
        }
    }
    let rows = harness::check("step-io", "STEP: kernel bodies read by FreeCAD", &cases);
    let bad: Vec<String> = rows
        .iter()
        .filter(|r| !r.problems.is_empty())
        .map(|r| format!("{}: {}", r.file, r.problems.join("; ")))
        .collect();
    assert!(
        bad.is_empty(),
        "{} files with problems:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// The volume between the floor and the roof of
/// `trimmed::domed_block(_, dx, dy, height, _)` over the rectangle
/// `x` × `y` of its floor: the roof is the bicubic Bézier patch over a
/// regular net whose heights are `height (1 + 0.15 (((7 i + 3 j) mod 5) −
/// 2))`, a polynomial of degree 3 each way, which three-point
/// Gauss-Legendre integrates exactly.
fn under_the_roof(dx: f64, dy: f64, height: f64, x: (f64, f64), y: (f64, f64)) -> f64 {
    let bernstein = |t: f64| {
        let s = 1.0 - t;
        [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t]
    };
    let roof = |px: f64, py: f64| {
        let (bu, bv) = (bernstein(px / dx), bernstein(py / dy));
        let mut z = 0.0;
        for (i, u) in bu.iter().enumerate() {
            for (j, v) in bv.iter().enumerate() {
                z += u * v * height * (1.0 + 0.15 * (((i * 7 + j * 3) % 5) as f64 - 2.0));
            }
        }
        z
    };
    let gauss = [
        (-(0.6_f64.sqrt()), 5.0 / 9.0),
        (0.0, 8.0 / 9.0),
        (0.6_f64.sqrt(), 5.0 / 9.0),
    ];
    let (mx, hx) = ((x.0 + x.1) / 2.0, (x.1 - x.0) / 2.0);
    let (my, hy) = ((y.0 + y.1) / 2.0, (y.1 - y.0) / 2.0);
    let mut total = 0.0;
    for (a, wa) in gauss {
        for (b, wb) in gauss {
            total += wa * wb * hx * hy * roof(mx + hx * a, my + hy * b);
        }
    }
    total
}

/// Booleans of the test kit's domed block (a block under a bicubic roof,
/// as a Bézier patch and as a NURBS surface), which no command builds
/// yet: a block through the roof, and the corpus case
/// `contact/dome-dome/003` of `crates/kernel/tests/boolean_freeform.rs`, a
/// corner of one roof resting on another roof, where the two touch in a
/// point: their union is refused as zero thickness (the developer's
/// decision of 2026-10-15; `a_corner_of_a_roof_resting_on_another_roof`
/// in the kernel's tests holds it at three scales without FreeCAD) and is
/// not exported, and the difference is the lower one as it was.
///
/// The bound is `APPROXIMATED`: the cut through the roof is a curve that
/// is no line in the roof's parameter plane, and FreeCAD's area of a
/// spline face is a numerical integral it does not take to convergence
/// (docs/testing.md, the boolean corpus: 6.7e-6 at worst there). The
/// volumes of the pieces are also held to each other here:
/// vol(A ∪ B) + vol(A ∩ B) = vol A + vol B, by FreeCAD's numbers alone.
#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD"]
fn freecad_reads_booleans_of_domed_blocks() {
    let mut cases = Vec::new();
    let mut refused = Vec::new();
    // Per scale and representation: the files of A, B, A ∪ B, A ∩ B.
    let mut identities = Vec::new();
    // Files and the volume each must have for lengths of 1.
    let mut closed: Vec<(String, f64)> = Vec::new();
    for (scale, tag) in [(1e-3, "milli"), (1.0, "one"), (1e6, "mega")] {
        let tolerance = Tolerance::new(1e-6 * scale, 1e-9).unwrap();
        let options = StepOptions {
            tolerance,
            ..StepOptions::default()
        };
        for (nurbs, kind) in [(false, "bezier"), (true, "nurbs")] {
            // The operands themselves, added after the results.
            let mut operands = Vec::new();
            let mut add = |name: &str, a: &Body, b: &Body, op: BooleanOp, lumps: usize| {
                let file = format!("{tag}-{kind}-{name}");
                match boolean(a, b, op, &tolerance) {
                    Ok(result) => {
                        let mut c = case(&file, &result.body, lumps, &options, 1.0);
                        c.relative = APPROXIMATED;
                        c.extent = Extent::Loose;
                        cases.push(c);
                    }
                    Err(e) => refused.push(format!("{file}: {e}")),
                }
                format!("{file}.step")
            };
            // A block standing in the dome and out through its roof.
            let dome =
                trimmed::domed_block(&tilted(scale), 2.0 * scale, 1.5 * scale, 2.5 * scale, nurbs)
                    .unwrap();
            let base = tilted(scale);
            let post = primitives::block(
                &Frame::new(
                    base.point(0.5 * scale, 0.4 * scale, 1.5 * scale),
                    base.z(),
                    base.x(),
                )
                .unwrap(),
                scale,
                0.7 * scale,
                2.0 * scale,
            )
            .unwrap();
            // What they share, by the roof's own definition (the heights of
            // `trimmed::domed_block`'s net, written out again): the part
            // of the post under the roof, from 1.5 up.
            let shared = under_the_roof(2.0, 1.5, 2.5, (0.5, 1.5), (0.4, 1.1)) - 0.7 * 1.5;
            let post_volume = 1.0 * 0.7 * 2.0;
            let both = add("dome-post-union", &dome.body, &post, BooleanOp::Union, 1);
            closed.push((
                both.clone(),
                dome.volume / scale.ppowi(3) + post_volume - shared,
            ));
            let less = add(
                "dome-post-subtract",
                &dome.body,
                &post,
                BooleanOp::Subtract,
                1,
            );
            closed.push((less, dome.volume / scale.ppowi(3) - shared));
            let common = add(
                "dome-post-intersect",
                &dome.body,
                &post,
                BooleanOp::Intersect,
                1,
            );
            closed.push((common.clone(), shared));
            let mut c = case(&format!("{tag}-{kind}-dome"), &dome.body, 1, &options, 1.0);
            c.relative = APPROXIMATED;
            let alone = c.file.clone();
            operands.push(c);
            let c = case(&format!("{tag}-{kind}-post"), &post, 1, &options, 1.0);
            identities.push([alone, c.file.clone(), both, common]);
            operands.push(c);

            // A corner of one roof resting on another roof.
            let lower = trimmed::domed_block(
                &Frame::new(
                    Vec3::new(-2.436569322600609, -2.194065104762556, 2.6567827309250918) * scale,
                    Vec3::new(0.6869183022974135, -0.7150994983998157, 0.12952201881213576),
                    Vec3::new(
                        -0.44103510150997627,
                        -0.5518503100792028,
                        -0.7077777013311259,
                    ),
                )
                .unwrap(),
                2.454469726218531 * scale,
                1.233351519002846 * scale,
                2.1578085031501213 * scale,
                nurbs,
            )
            .unwrap();
            let upper = trimmed::domed_block(
                &Frame::new(
                    Vec3::new(0.1340965815724604, -5.527588649130966, 5.27146697314718) * scale,
                    Vec3::new(0.533233144706825, 0.477304375339366, -0.698457548221771),
                    Vec3::new(
                        0.14799983577272827,
                        -0.8655317124360711,
                        -0.4784881434045441,
                    ),
                )
                .unwrap(),
                1.8356596835981147 * scale,
                2.9960054569223717 * scale,
                1.9891773926774199 * scale,
                nurbs,
            )
            .unwrap();
            add(
                "roof-corner-subtract",
                &lower.body,
                &upper.body,
                BooleanOp::Subtract,
                1,
            );
            // Two parts of one body touching in a point: refused, and
            // anything else (a body, or another refusal) is a problem.
            match boolean(&lower.body, &upper.body, BooleanOp::Union, &tolerance) {
                Err(KernelError::ZeroThickness { .. }) => {}
                Err(e) => refused.push(format!("{tag}-{kind}-roof-corner-union: {e}")),
                Ok(_) => refused.push(format!(
                    "{tag}-{kind}-roof-corner-union: made, where the parts touch in a point (zero thickness, to be refused)"
                )),
            }
            cases.append(&mut operands);
        }
    }
    let rows = harness::check(
        "step-io-domes",
        "STEP: booleans of domed blocks read by FreeCAD",
        &cases,
    );
    let mut bad: Vec<String> = rows
        .iter()
        .filter(|r| !r.problems.is_empty())
        .map(|r| format!("{}: {}", r.file, r.problems.join("; ")))
        .collect();
    // Ours against the roof's definition. The cut through the roof is a
    // spline within the modelling tolerance (1e-6 of the scale) of the
    // exact curve, so the body is the exact one give or take a skin one
    // tolerance thick: its volume may be off by the tolerance times its
    // area. (Measured: 4e-8 of the volume at most, where that bound is
    // 4e-6 to 9e-6; not alike at the three scales.)
    for (file, volume) in &closed {
        let Some(case) = cases.iter().find(|c| &c.file == file) else {
            continue;
        };
        let scale: f64 = if file.starts_with("milli") {
            1e-3
        } else if file.starts_with("mega") {
            1e6
        } else {
            1.0
        };
        let off = case.volume / (volume * scale.ppowi(3)) - 1.0;
        println!("{file}: our volume over the roof's own integral - 1 = {off:+.2e}");
        let skin = 1e-6 * scale * case.area / case.volume;
        if !(off.abs() <= skin) {
            bad.push(format!(
                "{file}: our volume is {off:e} off the integral of the roof (asked {skin:e})"
            ));
        }
    }
    // The identity among FreeCAD's own volumes of our four files.
    let theirs = |file: &str| {
        let (row, case) = rows.iter().zip(&cases).find(|(row, _)| row.file == file)?;
        Some(row.volume_ratio * case.volume)
    };
    for [a, b, both_file, common] in &identities {
        let (Some(a), Some(b), Some(both), Some(common)) =
            (theirs(a), theirs(b), theirs(both_file), theirs(common))
        else {
            continue;
        };
        let off = (both + common - a - b) / (a + b);
        println!(
            "{both_file}: vol(A ∪ B) + vol(A ∩ B) - vol A - vol B = {off:+.2e} of vol A + vol B, by FreeCAD's numbers"
        );
        if !(off.abs() <= 1e-7) {
            bad.push(format!(
                "FreeCAD's volumes of our union and intersection do not add up: {off:e}"
            ));
        }
    }
    if !refused.is_empty() {
        println!("\n{} booleans the kernel refused:", refused.len());
        for line in &refused {
            println!("- {line}");
        }
    }
    assert!(
        refused.is_empty(),
        "{} booleans refused:\n{}",
        refused.len(),
        refused.join("\n")
    );
    assert!(
        bad.is_empty(),
        "{} files with problems:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// An assembly (`step_assembly`): a block placed twice, the second turned
/// a quarter about z, and a smaller block turned a quarter about x, read
/// by FreeCAD as the three placed solids: their total volume, area and
/// centre, and the box around them, which the turns change (a placement
/// written wrong moves it).
#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD"]
fn freecad_reads_an_assembly_as_its_placed_parts() {
    use craft3d_io::{StepOccurrence, step_assembly};
    use craft3d_math::Transform;
    let quarter = std::f64::consts::FRAC_PI_2;
    let mut cases = Vec::new();
    for k in [1e-2, 1.0, 1e3] {
        let tol = options(k);
        let block = primitives::block(&Frame::WORLD, 20.0 * k, 10.0 * k, 6.0 * k).unwrap();
        let small = primitives::block(&Frame::WORLD, 8.0 * k, 4.0 * k, 2.0 * k).unwrap();
        let poses = [
            (0, Transform::IDENTITY, "A"),
            (
                0,
                Transform::rotation(Vec3::Z, quarter)
                    .unwrap()
                    .then(&Transform::translation(Vec3::new(0.0, 0.0, 30.0 * k))),
                "B",
            ),
            (
                1,
                Transform::rotation(Vec3::X, quarter)
                    .unwrap()
                    .then(&Transform::translation(Vec3::new(40.0 * k, 0.0, 0.0))),
                "C",
            ),
        ];
        let parts = [&block, &small];
        let occurrences: Vec<StepOccurrence> = poses
            .iter()
            .map(|(part, pose, name)| StepOccurrence {
                part: *part,
                pose: *pose,
                name: (*name).to_string(),
            })
            .collect();
        let file = format!("assembly-{k:e}");
        let (bytes, _) = step_assembly(
            &file,
            &[
                StepBody::new("Block", &block),
                StepBody::new("Small", &small),
            ],
            &occurrences,
            &tol,
        )
        .unwrap();
        // What FreeCAD should find: the bodies where they are placed.
        let placed: Vec<Body> = poses
            .iter()
            .map(|(part, pose, _)| parts[*part].transformed(pose).unwrap())
            .collect();
        let (mut volume, mut area, mut moment) = (0.0, 0.0, Vec3::ZERO);
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        let mut counts = (0, 0, 0, 0);
        for body in &placed {
            let m = mass_properties(body).unwrap();
            volume += m.volume;
            area += m.area;
            moment += m.centroid.unwrap() * m.volume;
            let (a, b) = body.bbox().unwrap().corners().unwrap();
            for (i, (a, b)) in [(a.x, b.x), (a.y, b.y), (a.z, b.z)].into_iter().enumerate() {
                lo[i] = lo[i].min(a);
                hi[i] = hi[i].max(b);
            }
            counts.0 += body.faces.len();
            counts.1 += body.edges.len();
            counts.2 += body.vertices.len();
            counts.3 += body.shells.len();
        }
        // The turns are in the box: B reaches x = -10, C z = 4.
        assert!((lo[0] + 10.0 * k).abs() < 1e-9 * k && (hi[2] - 36.0 * k).abs() < 1e-9 * k);
        let centre = moment * (1.0 / volume);
        cases.push(Case {
            file: format!("{file}.step"),
            bytes,
            to_mm: 1.0,
            solid: true,
            solids: 3,
            volume,
            area,
            centroid: Some([centre.x, centre.y, centre.z]),
            bbox: Some([lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]),
            bbox_slack: 0.0,
            extent: Extent::Exact,
            counts,
            names: vec![file.clone(), "Block".to_string(), "Small".to_string()],
            relative: RELATIVE,
            parts: Vec::new(),
        });
    }
    let rows = harness::check(
        "step-io-assembly",
        "STEP: an assembly read by FreeCAD",
        &cases,
    );
    let bad: Vec<String> = rows
        .iter()
        .filter(|r| !r.problems.is_empty())
        .map(|r| format!("{}: {}", r.file, r.problems.join("; ")))
        .collect();
    assert!(
        bad.is_empty(),
        "{} files with problems:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
