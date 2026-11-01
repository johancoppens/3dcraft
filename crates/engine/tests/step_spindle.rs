//! A STEP solid on a spindle torus (major radius smaller than its minor
//! radius) through the command layer: imported, checked deep (meshed and
//! searched for self-crossings), measured against the closed form, and cut
//! by a boolean whose halves add up (docs/design/step-import.md, Spindle
//! and horn tori). The file is written here, by hand.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::plain::run;
use common::{session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::{PI, TAU};

const HEAD: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('','',(''),(''),'','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n";
const TAIL: &str = "ENDSEC;\nEND-ISO-10303-21;\n";

/// The part below `z = 0` of the sheet of the torus `(major, minor)` about
/// z (the apple, `outer`, or the lemon), closed by a disc: in millimetres.
fn half(major: f64, minor: f64, outer: bool) -> String {
    // ISO 10303-42's normal points away from the tube's centre circle on
    // both parts: out of the apple and out of the lemon.
    let (radius, sense) = if outer {
        (major + minor, ".T.")
    } else {
        (minor - major, ".T.")
    };
    format!(
        "{HEAD}#1=( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) );\n\
         #2=( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) );\n\
         #4=( GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#2)) REPRESENTATION_CONTEXT('','') );\n\
         #5=ADVANCED_BREP_SHAPE_REPRESENTATION('part',(#10),#4);\n\
         #10=MANIFOLD_SOLID_BREP('half',#11);\n\
         #11=CLOSED_SHELL('',(#12,#22));\n\
         #12=ADVANCED_FACE('',(#13),#40,{sense});\n\
         #13=FACE_OUTER_BOUND('',#14,.T.);\n\
         #14=EDGE_LOOP('',(#15));\n\
         #15=ORIENTED_EDGE('',*,*,#16,.F.);\n\
         #16=EDGE_CURVE('',#17,#17,#30,.T.);\n\
         #17=VERTEX_POINT('',#18);\n\
         #18=CARTESIAN_POINT('',({radius:.17E},0.,0.));\n\
         #19=CARTESIAN_POINT('',(0.,0.,0.));\n\
         #20=DIRECTION('',(0.,0.,1.));\n\
         #21=DIRECTION('',(1.,0.,0.));\n\
         #22=ADVANCED_FACE('',(#23),#26,.T.);\n\
         #23=FACE_OUTER_BOUND('',#24,.T.);\n\
         #24=EDGE_LOOP('',(#25));\n\
         #25=ORIENTED_EDGE('',*,*,#16,.T.);\n\
         #26=PLANE('',#27);\n\
         #27=AXIS2_PLACEMENT_3D('',#19,#20,#21);\n\
         #30=CIRCLE('',#27,{radius:.17E});\n\
         #40=TOROIDAL_SURFACE('',#27,{major:.17E},{minor:.17E});\n\
         {TAIL}"
    )
}

/// Half the volume of the apple or the lemon of `(R, r)`: Pappus on the
/// tube's disc on either side of the axis (`s` the segment beyond it).
fn half_volume(major: f64, minor: f64, outer: bool) -> f64 {
    let (r, big) = (major, minor);
    let rise = (big * big - r * r).sqrt();
    let segment = big * big * (r / big).pacos() - r * rise;
    let whole = if outer {
        TAU * (PI * big * big * r - segment * r + 2.0 / 3.0 * rise.ppowi(3))
    } else {
        TAU * (2.0 / 3.0 * rise.ppowi(3) - r * segment)
    };
    whole / 2.0
}

fn volume(s: &mut Session) -> f64 {
    let inspected = run(s, "document.inspect", json!({}));
    let bodies = inspected["bodies"].as_array().unwrap();
    bodies.iter().map(|b| b["volume"].as_f64().unwrap()).sum()
}

/// The centroid's height: below the rim for the half the file meant (the
/// halves above and below are mirror images, of one volume).
fn centroid_z(s: &mut Session) -> f64 {
    run(s, "document.measure", json!({}))["centroid"][2]
        .as_f64()
        .unwrap()
}

fn imported(dir: &std::path::Path, file: &str) -> Session {
    let mut s = session_in(dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    let result = run(&mut s, "file.import", json!({"path": file}));
    assert_eq!(
        result["bodies"][0]["import"]["turnedShells"], 0,
        "{file}: {result}"
    );
    s
}

fn deep_ok(s: &mut Session, what: &str) {
    let check: Value = run(s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{what}: {check}");
}

#[test]
fn a_spindle_torus_solid_imports_measures_and_takes_booleans() {
    let dir = workdir("step-spindle", "booleans");
    for (outer, file) in [(true, "apple.step"), (false, "lemon.step")] {
        let (major, minor) = (10.0, 20.0);
        std::fs::write(dir.join(file), half(major, minor, outer)).unwrap();
        let want = half_volume(major, minor, outer);
        let mut s = imported(&dir, file);
        deep_ok(&mut s, file);
        let z = centroid_z(&mut s);
        assert!(z < 0.0, "{file}: the half above the rim (centroid z {z})");
        let whole = volume(&mut s);
        assert!(
            (whole - want).abs() <= 1e-7 * want,
            "{file}: {whole} against the closed form {want}"
        );
        // A block over the half where x > 0, through the pole: subtracted
        // and intersected, each is half (by symmetry), and they add up.
        let reach = 2.0 * (major + minor);
        let mut halves = 0.0;
        for op in ["subtract", "intersect"] {
            let mut s = imported(&dir, file);
            run(
                &mut s,
                "feature.block",
                json!({"size": [reach, 2.0 * reach, 2.0 * reach], "position": [0, -reach, -reach], "name": "Cutter"}),
            );
            run(
                &mut s,
                "feature.boolean",
                json!({"target": "F1", "tool": "Cutter", "op": op}),
            );
            deep_ok(&mut s, op);
            let z = centroid_z(&mut s);
            assert!(z < 0.0, "{file} {op}: centroid z {z}");
            let part = volume(&mut s);
            assert!(
                (part - want / 2.0).abs() <= 1e-7 * want,
                "{file} {op}: {part} against {}",
                want / 2.0
            );
            halves += part;
        }
        assert!(
            (halves - whole).abs() <= 1e-7 * want,
            "{file}: {halves} and {whole}"
        );
        // A rod along the axis through the pole, united: what sticks out
        // of the solid is added (the rod of radius 1 from z = -40 to 10).
        let mut s = imported(&dir, file);
        run(
            &mut s,
            "feature.cylinder",
            json!({"radius": 1, "height": 5, "position": [0, 0, 0], "name": "Rod"}),
        );
        run(
            &mut s,
            "feature.boolean",
            json!({"target": "F1", "tool": "Rod", "op": "union"}),
        );
        deep_ok(&mut s, "union");
        let united = volume(&mut s);
        assert!(
            (united - whole - 5.0 * PI).abs() <= 1e-7 * want,
            "{file} union: {united} against {}",
            whole + 5.0 * PI
        );
    }
}
