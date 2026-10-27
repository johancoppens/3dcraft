//! Booleans the showcase models found refused (2026-10-15), each from the
//! steps that made it, with what the result has to satisfy.
//!
//! - The lamp: a shell of revolution with thin walls less a union of ten
//!   lofted cells was refused, "an intersection curve could not be fitted
//!   within the tolerance", though every nine of them and each alone
//!   passed. Every face pair was intersected inside the box both bodies
//!   share, so the union decided the box; the box cut a branch of the
//!   curve of the shell and one cell, and the span to the new end was a
//!   new cubic nobody held to the tolerance. The marcher now refines the
//!   pieces a box cuts.
//! - The impeller: blades with a rounded leading edge, refused in a
//!   pattern ("tangent or nearly coincident over too large an area") and
//!   on a hub ("two curves leave a point in the same direction"): faces
//!   were intersected in the box of the bodies; now in that of the faces.
//! - The impeller: blades with a rounded trailing edge on a hub ("two
//!   points closer together than the tolerance can tell apart"): where
//!   two edges meet, the place found twice is one vertex.
//! - The lamp: a shade with many holes that could not be measured.
//! - The bike: a top tube less its inset core ("tangent or nearly
//!   coincident over too large an area": the tube's taper was walked whole,
//!   far beyond the box asked about) and two mirrored seat stays ("could
//!   not be fitted": a face on a torus had the box of its whole torus).
//! - The turbofan: blades rounded to 0.1 mm on a disc (a march step that
//!   went once around a loop smaller than itself).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_geom::intersect::{SurfaceMeeting, meet_surfaces};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_math::{Bbox3, Transform, Vec3};
use craft3d_topo::{Body, mass_properties};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

fn run(session: &mut Session, id: &str, params: &Value) -> Value {
    session
        .execute(id, params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A session after the steps of `file` (a JSON array of `[id, params]`).
fn steps(file: &str) -> Session {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/boolean_findings")
            .join(file),
    )
    .unwrap();
    let steps: Vec<(String, Value)> = serde_json::from_str(&text).unwrap();
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.new", &json!({"units": "mm"}));
    for (id, params) in &steps {
        run(&mut s, id, params);
    }
    s
}

fn status(session: &mut Session, name: &str) -> Value {
    let document = run(session, "document.inspect", &json!({}));
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}: {document}"))
        .clone()
}

fn assert_ok(session: &mut Session, name: &str) {
    let feature = status(session, name);
    assert_eq!(feature["status"], "ok", "{feature}");
    let check = run(session, "document.check", &json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

/// The bodies of the session's features by name (those taken by later
/// features too), regenerated from the
/// document saved to a file of its own, and the document's tolerance.
fn bodies(
    session: &mut Session,
    file: &str,
) -> (Vec<(String, Arc<Body>)>, craft3d_math::Tolerance) {
    let dir = std::env::temp_dir().join(format!("craft3d-boolean-findings-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(file);
    run(
        session,
        "file.save",
        &json!({"path": path.to_str().unwrap()}),
    );
    let doc = craft3d_format::load(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let regen = craft3d_regen::regenerate(&doc);
    let bodies = doc
        .features
        .iter()
        .filter_map(|f| {
            // Operands too: a body taken by a later feature stays here.
            let body = regen.feature(f.id)?.body.as_ref()?;
            Some((f.name.clone(), body.clone()))
        })
        .collect();
    let tolerance = craft3d_math::Tolerance {
        linear: doc.tolerance.linear,
        angular: doc.tolerance.angular,
    };
    (bodies, tolerance)
}

fn body<'a>(bodies: &'a [(String, Arc<Body>)], name: &str) -> &'a Body {
    &bodies.iter().find(|(n, _)| n == name).unwrap().1
}

fn volume(body: &Body) -> f64 {
    mass_properties(body).unwrap().volume
}

#[test]
fn the_lamp_shell_less_a_lofted_cell() {
    let mut s = steps("lamp_cell.steps.json");
    let cut = run(
        &mut s,
        "feature.boolean",
        &json!({"op": "subtract", "target": "Shell", "tool": "Cell66", "keepTool": true, "name": "Cut"}),
    );
    assert_ne!(cut["status"], "failed", "{cut}");
    assert_ok(&mut s, "Cut");
    let (bodies, tol) = bodies(&mut s, "lamp.3dcraft");
    let (shell, cell) = (body(&bodies, "Shell"), body(&bodies, "Cell66"));

    // The intersection of the shell's outer wall and the end cap of the cell
    // in the box the ten cells of the lamp gave it: the box cuts a branch
    // of their curve, and every curve is within the tolerance of both
    // surfaces (it was "could not be fitted").
    let within = Bbox3::from_points([
        Vec3::new(-168.04316601773542, -168.04316601773542, 104.38877107830017),
        Vec3::new(167.06945264388725, 168.0431660177354, 198.75929163002164),
    ]);
    let (wall, cap) = (&shell.faces[1].surface, &cell.faces[2].surface);
    let met = meet_surfaces(wall, cap, &within, &tol).unwrap();
    let SurfaceMeeting::Meets(meeting) = met else {
        panic!("the wall and the cap meet: {met:?}");
    };
    assert!(!meeting.curves.is_empty());
    for curve in &meeting.curves {
        assert!(curve.deviation <= tol.linear, "{}", curve.deviation);
        let (t0, t1) = curve.range;
        for k in 0..=400 {
            let p = curve.curve.eval(t0 + (t1 - t0) * f64::from(k) / 400.0);
            for surface in [wall, cap] {
                let (u, v) = surface.project(p).unwrap();
                let off = surface.eval(u, v).distance(p);
                assert!(off <= tol.linear, "{off} off a surface at {p:?}");
            }
        }
    }

    // The identities of the three booleans, in place and turned.
    let turn =
        Transform::rotation_about(Vec3::new(3.0, -7.0, 11.0), Vec3::new(0.3, -0.5, 0.8), 0.7)
            .unwrap();
    for t in [None, Some(turn)] {
        let place = |b: &Body| t.map_or_else(|| b.clone(), |t| b.transformed(&t).unwrap());
        let (a, b) = (place(shell), place(cell));
        let (va, vb) = (volume(&a), volume(&b));
        let of = |op| volume(&boolean(&a, &b, op, &tol).unwrap().body);
        let (rest, common, all) = (
            of(BooleanOp::Subtract),
            of(BooleanOp::Intersect),
            of(BooleanOp::Union),
        );
        assert!(
            common > 1e-3 * vb,
            "the cell goes through the wall: {common}"
        );
        assert!(
            (rest + common - va).abs() <= 1e-8 * va,
            "{rest} + {common} != {va}"
        );
        assert!(
            (all + common - va - vb).abs() <= 1e-8 * va,
            "{all} + {common} != {va} + {vb}"
        );
    }
}

/// The lamp's shade with six cells cut through it (the lamp's finding M1):
/// a face of the shell has eight loops, and its boundary integral, all
/// loops refined together, ran out of the face's work at the second sum
/// though each loop had settled. Measured now, and the identity of the
/// cut: what is left and what was taken make the shell.
#[test]
fn the_lamp_shade_with_many_holes_is_measured() {
    let mut s = steps("lamp_shade.steps.json");
    assert_ok(&mut s, "Shade6");
    let (bodies, tol) = bodies(&mut s, "shade.3dcraft");
    let (shell, cells) = (body(&bodies, "Shell"), body(&bodies, "B6U3_0"));
    let common = boolean(shell, cells, BooleanOp::Intersect, &tol)
        .unwrap()
        .body;
    let (left, taken, whole) = (
        volume(body(&bodies, "Shade6")),
        volume(&common),
        volume(shell),
    );
    assert!(taken > 0.0, "the cells cut the shell");
    assert!(
        (left + taken - whole).abs() <= 1e-8 * whole,
        "{left} + {taken} != {whole}"
    );
}

/// Two blades with a rounded trailing edge on a hub of revolution (the
/// impeller's finding F8, cut down to two blades): the edge of a blade's
/// fillet runs onto the hub's cylinder at a shallow angle right at the
/// hub's rim, and the place where it meets the rim's circle was found
/// twice, 6.6e-5 mm apart at a tolerance of 1e-4 mm ("two points closer
/// together than the tolerance can tell apart"). Each place lies within
/// the tolerance of both edges: it is one vertex. The union, and the
/// identities of the three booleans, in place and turned, to the
/// tolerance over the area (the fillet is a face approximated within it).
#[test]
fn blades_with_a_rounded_trailing_edge_on_a_hub() {
    let mut s = steps("impeller_te_hub.steps.json");
    assert_ok(&mut s, "WheelA");
    let (bodies, tol) = bodies(&mut s, "te_hub.3dcraft");
    let (hub, blades) = (body(&bodies, "HubRaw"), body(&bodies, "Blades"));
    let slack =
        tol.linear * (mass_properties(hub).unwrap().area + mass_properties(blades).unwrap().area);
    let turn =
        Transform::rotation_about(Vec3::new(2.0, -1.0, 3.0), Vec3::new(-0.2, 0.6, 0.77), 1.1)
            .unwrap();
    for t in [None, Some(turn)] {
        let place = |b: &Body| t.map_or_else(|| b.clone(), |t| b.transformed(&t).unwrap());
        let (a, b) = (place(hub), place(blades));
        let (va, vb) = (volume(&a), volume(&b));
        let of = |op| volume(&boolean(&a, &b, op, &tol).unwrap().body);
        let (all, common, rest) = (
            of(BooleanOp::Union),
            of(BooleanOp::Intersect),
            of(BooleanOp::Subtract),
        );
        assert!(common > 0.0, "the blades stand in the hub");
        assert!(
            (all + common - va - vb).abs() <= slack,
            "{all} + {common} != {va} + {vb}"
        );
        assert!(
            (rest + common - va).abs() <= slack,
            "{rest} + {common} != {va}"
        );
    }
}

/// How far a volume of bodies whose faces were approximated within the
/// tolerance can be off: the tolerance over their area. (The blade's
/// fillet is such a face, at the impeller's tolerance of 1e-4 mm: the
/// blade turned by 30 degrees about the hub's axis measures 6e-5 mm³
/// more, of 511 mm³.)
fn slack(tol: &craft3d_math::Tolerance, bodies: &[&Body]) -> f64 {
    tol.linear
        * bodies
            .iter()
            .map(|b| mass_properties(b).unwrap().area)
            .sum::<f64>()
}

/// A pattern of two blades with a rounded leading edge, apart from each
/// other (the impeller's finding F7): refused as "tangent or nearly
/// coincident over too large an area", the fillet's surface of one blade
/// and the side of the other running tangent far from both faces, inside
/// the box the bodies share. A pair of faces is intersected in the box
/// the two faces share.
#[test]
fn a_pattern_of_blades_with_a_rounded_leading_edge() {
    let mut s = steps("impeller_pattern.steps.json");
    assert_ok(&mut s, "Blades");
    let (bodies, tol) = bodies(&mut s, "pattern.3dcraft");
    // The instances do not touch: the pattern is twice the blade, with
    // every face of both.
    let (blade, blades) = (body(&bodies, "Blade"), body(&bodies, "Blades"));
    assert_eq!(blades.faces.len(), 2 * blade.faces.len());
    let (one, two) = (
        mass_properties(blade).unwrap(),
        mass_properties(blades).unwrap(),
    );
    assert!(
        (two.area - 2.0 * one.area).abs() <= 1e-9 * two.area,
        "{two:?} {one:?}"
    );
    let off = (two.volume - 2.0 * one.volume).abs();
    assert!(off <= slack(&tol, &[blades]), "{two:?} {one:?}");
}

/// A blade with a rounded leading edge on a hub of revolution (the
/// impeller's finding F5), refused as "two curves leave a point in the
/// same direction" with the box of the bodies; it builds with the box of
/// the faces.
#[test]
fn a_blade_with_a_rounded_leading_edge_on_a_spline_hub() {
    let mut s = steps("impeller_le_hub.steps.json");
    assert_ok(&mut s, "U1");
    let (bodies, tol) = bodies(&mut s, "hub.3dcraft");
    let (hub, blade, united) = (
        body(&bodies, "HubRaw"),
        body(&bodies, "Blade"),
        body(&bodies, "U1"),
    );
    let common = boolean(hub, blade, BooleanOp::Intersect, &tol)
        .unwrap()
        .body;
    let rest = boolean(hub, blade, BooleanOp::Subtract, &tol).unwrap().body;
    let (vc, vr) = (volume(&common), volume(&rest));
    let (vh, vb, all) = (volume(hub), volume(blade), volume(united));
    assert!(vc > 1e-3 * vb, "the blade sits in the hub: {vc}");
    let slack = slack(&tol, &[hub, blade]);
    assert!(
        (all + vc - vh - vb).abs() <= slack,
        "{all} + {vc} != {vh} + {vb}"
    );
    assert!((vr + vc - vh).abs() <= slack, "{vr} + {vc} != {vh}");
}

/// The bike's top tube less its core, the same loft 1.2 mm inside it (the
/// bike's finding B9): refused as "tangent or nearly coincident over too
/// large an area", the tube's tapered face walked whole where the core's
/// cylinder crosses it 300 mm from the box asked about. Only the part of
/// a chart that can be inside the box is walked. The core lies inside the
/// tube: what is left is the tube less the core, and the identities of
/// the three booleans hold, in place and turned, to the tolerance over
/// the area.
#[test]
fn the_top_tube_less_its_core() {
    let mut s = steps("bike_core.steps.json");
    assert_ok(&mut s, "Hollow");
    let (bodies, tol) = bodies(&mut s, "core.3dcraft");
    let (tube, core) = (body(&bodies, "TopTube"), body(&bodies, "TopTubeCore"));
    let slack = slack(&tol, &[tube, core]);
    let (vt, vc) = (volume(tube), volume(core));
    let hollow = volume(body(&bodies, "Hollow"));
    assert!(
        (hollow - (vt - vc)).abs() <= slack,
        "{hollow} != {vt} - {vc}"
    );
    let turn =
        Transform::rotation_about(Vec3::new(2.0, -1.0, 3.0), Vec3::new(-0.2, 0.6, 0.77), 1.1)
            .unwrap();
    for t in [None, Some(turn)] {
        let place = |b: &Body| t.map_or_else(|| b.clone(), |t| b.transformed(&t).unwrap());
        let (a, b) = (place(tube), place(core));
        let of = |op| volume(&boolean(&a, &b, op, &tol).unwrap().body);
        let (all, common, rest) = (
            of(BooleanOp::Union),
            of(BooleanOp::Intersect),
            of(BooleanOp::Subtract),
        );
        assert!((all - vt).abs() <= slack, "{all} != {vt}");
        assert!((common - vc).abs() <= slack, "{common} != {vc}");
        assert!((rest - (vt - vc)).abs() <= slack, "{rest} != {vt} - {vc}");
    }
}

/// The bike's two seat stays, each a tube swept along an arc of a circle
/// of 2.8 m and the second the mirror image of the first (the bike's
/// finding B6): refused as one, "an intersection curve could not be
/// fitted", a face on a torus having the box of its whole torus. The
/// stays are apart: their union is both.
#[test]
fn two_mirrored_seat_stays_are_one_body_of_two() {
    let mut s = steps("bike_stays.steps.json");
    assert_ok(&mut s, "Both");
    let (bodies, tol) = bodies(&mut s, "stays.3dcraft");
    let (left, right, both) = (
        body(&bodies, "RearL2"),
        body(&bodies, "Right"),
        body(&bodies, "Both"),
    );
    assert_eq!(both.shells.len(), left.shells.len() + right.shells.len());
    let (vl, vr, vb) = (volume(left), volume(right), volume(both));
    assert!(
        (vb - vl - vr).abs() <= slack(&tol, &[left, right]),
        "{vb} != {vl} + {vr}"
    );
}

/// A row of compressor blades whose leading and trailing edges are rounded
/// to 0.1 mm, united with their disc (the turbofan's finding F10): refused
/// as "a curve partly on and partly off a face boundary" at exactly 0.1
/// mm. The round's cylinder and the disc's rim meet in a loop 0.63 mm
/// round, and in the box of the bodies the march's first step went once
/// around it, so the curve was the loop twice. The union holds the disc
/// and the row with what they share once.
#[test]
fn a_row_of_blades_with_rounds_of_a_tenth_on_its_disc() {
    let mut s = steps("turbofan_c3_row.steps.json");
    assert_ok(&mut s, "C3DiscU");
    let (bodies, tol) = bodies(&mut s, "c3.3dcraft");
    let (disc, row, united) = (
        body(&bodies, "C3DiscRims"),
        body(&bodies, "C3Rotor"),
        body(&bodies, "C3DiscU"),
    );
    let common = volume(&boolean(disc, row, BooleanOp::Intersect, &tol).unwrap().body);
    let (vd, vr, vu) = (volume(disc), volume(row), volume(united));
    assert!(common > 0.0, "the blades stand in the disc");
    assert!(
        (vu + common - vd - vr).abs() <= slack(&tol, &[disc, row]),
        "{vu} + {common} != {vd} + {vr}"
    );
}

/// The union of `a` and `b` as a feature, which must build and pass the
/// deep check; then the identities of the three booleans of their bodies, in place and
/// turned, to the tolerance over the area (the difference made without
/// the refusal of zero thickness: the parts it leaves may touch).
fn united_and_identities(file: &str, a: &str, b: &str) {
    use craft3d_kernel::boolean::{BooleanOptions, boolean_with};
    let mut s = steps(file);
    let united = run(
        &mut s,
        "feature.boolean",
        &json!({"op": "union", "target": a, "tool": b, "keepTool": true, "name": "U"}),
    );
    assert_ne!(united["status"], "failed", "{united}");
    assert_ok(&mut s, "U");
    let (bodies, tol) = bodies(&mut s, file);
    let (first, second) = (body(&bodies, a), body(&bodies, b));
    let slack = slack(&tol, &[first, second]);
    let turn =
        Transform::rotation_about(Vec3::new(3.0, -7.0, 11.0), Vec3::new(0.3, -0.5, 0.8), 0.7)
            .unwrap();
    let unrefused = BooleanOptions {
        refuse_zero_thickness: false,
        ..BooleanOptions::default()
    };
    for t in [None, Some(turn)] {
        let place = |b: &Body| t.map_or_else(|| b.clone(), |t| b.transformed(&t).unwrap());
        let (x, y) = (place(first), place(second));
        let (vx, vy) = (volume(&x), volume(&y));
        let of = |op| volume(&boolean_with(&x, &y, op, &tol, &unrefused).unwrap().body);
        let (all, common, rest) = (
            of(BooleanOp::Union),
            of(BooleanOp::Intersect),
            of(BooleanOp::Subtract),
        );
        assert!(common > 0.0, "{file}: the bodies overlap");
        assert!(
            (all + common - vx - vy).abs() <= slack,
            "{file}: {all} + {common} != {vx} + {vy}"
        );
        assert!(
            (rest + common - vx).abs() <= slack,
            "{file}: {rest} + {common} != {vx}"
        );
    }
}

/// The Perseverance's drill tip (2026-10-21): a cone whose base circle
/// lies 3.9e-4 mm off the wall of a drill along a tilted axis (the
/// positions given to a thousandth), refused "two curves leave a point
/// in the same direction and stay together": the curve of cone and wall
/// crosses the base circle at 2.6e-5 rad, 1.9e-4 mm from the cone's seam.
/// An imprinted curve lies in the face, so it leaves a vertex of the
/// boundary on the face's side of it however shallow the angle
/// (`regions::order_run`). The union is right. The self-check's mesh of
/// it refused it next (the crescents between the base circle and the
/// curves, 3.9e-4 and 7.8e-4 mm across with tips at 2.6e-5 rad, needed
/// more rounds of halved chords than the tessellator gave); meshed since
/// 2026-10-21 (`docs/design/tessellator.md`), the self-check found the
/// seam's vertex 5.6e-9 from a crescent's side that is not its own and
/// refused it as touching itself. Measured (2026-10-22): the seam vertex
/// is 1.94e-4 along the base circle from the crossing, whose edges leave
/// it 1.3e-5 and 2.9e-5 rad apart, and 1.94e-4 times those is the 2.5e-9
/// and 5.6e-9 found: a corner sharper than the tolerance resolves, not
/// the shell meeting itself, which the touch rule reads as such since
/// the Garabit crack (`craft3d_topo::touches`, 2026-10-21). The feature
/// builds and passes the deep check.
#[test]
fn the_rover_drill_tip_on_a_tilted_drill() {
    united_and_identities("rover_drill_tip.steps.json", "Drill", "Tip");
}

/// The Garabit pier's stair (2026-10-21, union 127 of 307): a tread
/// whose end touches the inside of the helical stringer (a sweep along a
/// helix) and whose corners go into it, refused "two curves leave a point
/// in the same direction and stay together" on the tread's top and
/// bottom faces, where the stringer's curve leaves the end edge along it.
#[test]
fn the_pier_stair_tread_on_its_stringer() {
    united_and_identities("pier_stair_tread.steps.json", "Stringer0", "Tr0_0");
}

/// The winding number of a closed triangle mesh about `p` (the sum of
/// the solid angles of its triangles over 4π): an oracle for whether `p`
/// is inside that shares nothing with the kernel's ray casting.
fn winding(mesh: &craft3d_tess::Mesh, p: Vec3) -> f64 {
    let mut sum = 0.0;
    for t in &mesh.triangles {
        let [a, b, c] = t.map(|i| mesh.positions[i as usize] - p);
        let (la, lb, lc) = (a.length(), b.length(), c.length());
        let num = a.dot(b.cross(c));
        let den = la * lb * lc + a.dot(b) * lc + a.dot(c) * lb + b.dot(c) * la;
        sum += 2.0 * num.atan2(den);
    }
    sum / (4.0 * std::f64::consts::PI)
}

/// The sea kayak's tips (2026-10-27): `TipHalfD`, the tips' stock cut by
/// the deck's cylinder (each operand an extrusion of a formula curve, a
/// ruled spline face beside flat ends and sides, 3 m thick), passed every
/// rule of the boolean but the deep check called it wrong: the point
/// (-1864.42, -1469.54, -1233.82) outside the target, inside the tool and
/// inside the result. The result was right (a mesh's winding number
/// agrees with it everywhere); the ray casting was not: the ray from that
/// point crosses the deck's spline face 2432 mm on, and the whole ray was
/// proven clear of it at once, because the distance from the ray to a
/// flat leaf of the face read as 1.5e-5 mm (the rounding of a quadratic
/// expanded about the ray's start 2600 mm off) instead of zero. Five per
/// cent of the points in the box of the deck's cylinder were classified
/// wrongly, so a boolean that classified a region by such a ray would
/// have been wrong too. Held to the deep check, the identity of
/// intersection and difference, seeded points against the ray casting
/// and against the meshes' winding numbers, at three scales.
#[test]
fn the_sea_kayak_tips_against_the_deck() {
    use craft3d_kernel::boolean::{Place, VerifyOptions, classify_points, verify};
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/boolean_findings/sea_kayak_tips.steps.json"),
    )
    .unwrap();
    // Every length of the steps is in millimetres: the same model in
    // micrometres and in centimetres is the hull at a thousandth and at
    // ten times its size. (At a million times the 5 m hull is 5000 km
    // against a tolerance of 1e-6: no geometry is held to 1e-15 of its
    // size.)
    for (scale, unit) in [(1e-3, "um"), (1.0, "mm"), (10.0, "cm")] {
        let steps: Vec<(String, Value)> =
            serde_json::from_str(&text.replace(" mm\"", &format!(" {unit}\""))).unwrap();
        let mut s = Session::new(Workspace::unrestricted());
        run(&mut s, "file.new", &json!({"units": "mm"}));
        for (id, params) in &steps {
            run(&mut s, id, params);
        }
        for name in ["TipHalf1", "TipHalf2", "TipHalf3"] {
            assert_eq!(status(&mut s, name)["status"], "ok", "{scale}: {name}");
        }
        assert_ok(&mut s, "TipHalfD");
        let (bodies, tol) = bodies(&mut s, "sea_kayak_tips.3dcraft");
        let (a, b) = (
            body(&bodies, "TipHalf3").clone(),
            body(&bodies, "CylD").clone(),
        );
        if unit == "mm" {
            let found = Vec3::new(
                -1864.4193771268945,
                -1469.5401187305245,
                -1233.8242606133363,
            );
            assert_eq!(
                classify_points(&b, &[found], &tol).unwrap(),
                vec![Place::Outside]
            );
        }
        let common = boolean(&a, &b, BooleanOp::Intersect, &tol).unwrap().body;
        let rest = boolean(&a, &b, BooleanOp::Subtract, &tol).unwrap().body;
        let slack = slack(&tol, &[&a, &b]);
        let (va, vc, vr) = (volume(&a), volume(&common), volume(&rest));
        assert!(vc > 0.0 && vr > 0.0, "{scale}: {vc} {vr}");
        assert!(
            (vc + vr - va).abs() <= slack,
            "{scale}: {vc} + {vr} != {va} (slack {slack})"
        );
        let options = VerifyOptions {
            volumes: true,
            points: 2000,
        };
        for (op, result) in [
            (BooleanOp::Intersect, &common),
            (BooleanOp::Subtract, &rest),
        ] {
            let checked = verify(&a, &b, op, result, &tol, &options).unwrap();
            assert!(
                checked.is_ok(),
                "{scale} {op:?}: {:?}",
                checked
                    .violations
                    .iter()
                    .take(4)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            );
            assert!(checked.points >= 1500, "{scale}: {}", checked.points);
        }
        // The ray casting against the meshes' winding numbers, at points
        // seeded in the box of both.
        let params = craft3d_tess::TessParams {
            chordal_tolerance: 0.05 * scale,
            ..craft3d_tess::TessParams::default()
        };
        let mesh = |x: &Body| craft3d_tess::tessellate_body(x, &params).unwrap();
        let (lo, hi) = {
            let (ba, bb) = (a.bbox().unwrap(), b.bbox().unwrap());
            let ((a0, a1), (b0, b1)) = (ba.corners().unwrap(), bb.corners().unwrap());
            (a0.min(b0), a1.max(b1))
        };
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let points: Vec<Vec3> = (0..400)
            .map(|_| {
                Vec3::new(
                    lo.x + (hi.x - lo.x) * next(),
                    lo.y + (hi.y - lo.y) * next(),
                    lo.z + (hi.z - lo.z) * next(),
                )
            })
            .collect();
        let mut compared = 0;
        for x in [&a, &b, &common, &rest] {
            let m = mesh(x);
            let rays = classify_points(x, &points, &tol).unwrap();
            for (&p, ray) in points.iter().zip(rays) {
                let w = winding(&m, p);
                // (Within the mesh's chord of the surface the mesh and the
                // body may disagree.)
                if (w - w.round()).abs() > 0.01 {
                    continue;
                }
                let by_mesh = w > 0.5;
                match ray {
                    Place::Inside | Place::Outside => {
                        compared += 1;
                        assert_eq!(ray == Place::Inside, by_mesh, "{scale}: {p:?} {w}");
                    }
                    Place::OnBoundary | Place::Unknown => {}
                }
            }
        }
        assert!(compared >= 1500, "{scale}: {compared}");
    }
}
