//! Spindle and horn tori read from hand-written files: half an apple and
//! half a lemon (the part of a sheet below its equator, closed by a disc)
//! against the closed forms of their volume and area, at three scales; the
//! sheet chosen by the edges, by `select_outer`, through an offset and a
//! surface of revolution; hostile radii refused by name.

use super::*;
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance};
use std::f64::consts::{PI, TAU};

const HEAD: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('','',(''),(''),'','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n";
const TAIL: &str = "ENDSEC;\nEND-ISO-10303-21;\n";

/// A solid in millimetres: the part below `z = 0` of a face on surface
/// `#40` (which `surface` defines, with ids from 40 up; `same_sense` its
/// flag), bounded by the circle of `radius` about the z axis in `z = 0`,
/// and the disc that closes it on top.
fn half(surface: &str, same_sense: bool, radius: f64) -> String {
    let sense = if same_sense { ".T." } else { ".F." };
    format!(
        "{HEAD}#1=( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) );\n\
         #2=( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) );\n\
         #4=( GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#2)) REPRESENTATION_CONTEXT('','') );\n\
         #5=ADVANCED_BREP_SHAPE_REPRESENTATION('part',(#10),#4);\n\
         #10=MANIFOLD_SOLID_BREP('solid',#11);\n\
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
         {surface}{TAIL}"
    )
}

fn torus(keyword: &str, major: f64, minor: f64, extra: &str) -> String {
    format!("#40={keyword}('',#27,{major:.17E},{minor:.17E}{extra});\n")
}

fn options(linear: f64) -> Options {
    let tolerance = Tolerance {
        linear,
        ..Tolerance::default()
    };
    Options {
        metres_per_unit: 1e-3,
        tolerance,
        loosest: 1000.0 * linear,
    }
}

fn read(text: &str, scale: f64) -> Result<Imported, ImportError> {
    parse(text.as_bytes())?.build(0, &options(1e-7 * scale))
}

/// `(volume, area)` of the whole apple and the whole lemon of the torus
/// `(R, r)`, `R ≤ r`: what the tube's disc on either side of the axis
/// sweeps (Pappus: its area times the way its centroid goes round), and
/// what its arc sweeps. `s` is the circular segment beyond the axis.
fn apple(major: f64, minor: f64) -> (f64, f64) {
    let (big, small) = (minor, major);
    let rise = (big * big - small * small).sqrt();
    let segment = big * big * (small / big).pacos() - small * rise;
    let volume = TAU * (PI * big * big * small - segment * small + 2.0 / 3.0 * rise.ppowi(3));
    let reach = (-small / big).pacos();
    let area = TAU * big * (2.0 * small * reach + 2.0 * big * reach.psin());
    (volume, area)
}

fn lemon(major: f64, minor: f64) -> (f64, f64) {
    let (big, small) = (minor, major);
    let rise = (big * big - small * small).sqrt();
    let segment = big * big * (small / big).pacos() - small * rise;
    let volume = TAU * (2.0 / 3.0 * rise.ppowi(3) - small * segment);
    let reach = (small / big).pacos();
    let area = TAU * big * (2.0 * big * reach.psin() - 2.0 * small * reach);
    (volume, area)
}

/// `(volume, area)`, and that the solid is the part below its rim (its
/// centroid below `z = 0`), not the part of the sheet above it that the
/// same edges bound: the face read with the file's normal.
fn measured(imported: &Imported, scale: f64) -> (f64, f64) {
    let (volume, area, z) = measured_z(imported, scale);
    assert!(z < 0.0, "the part above the rim: centroid at z = {z:e}");
    (volume, area)
}

fn measured_z(imported: &Imported, scale: f64) -> (f64, f64, f64) {
    let defects = craft3d_topo::check(&imported.body, &options(1e-7 * scale).tolerance);
    assert!(defects.is_empty(), "{defects:?}");
    let mass = craft3d_topo::mass_properties(&imported.body).expect("measured");
    let z = mass.centroid.map_or(f64::NAN, |c| c.z);
    (mass.volume, mass.area, z)
}

fn close(what: &str, got: f64, want: f64) {
    assert!(
        (got - want).abs() <= 1e-9 * want.abs(),
        "{what}: {got:e}, closed form {want:e} (relative {:e})",
        (got - want).abs() / want.abs()
    );
}

#[test]
fn half_an_apple_and_half_a_lemon_match_their_closed_forms_at_three_scales() {
    for scale in [1e-3, 1.0, 1e3] {
        for (major, minor) in [(1.0, 2.0), (0.25, 3.0), (1.9, 2.0)] {
            let (major, minor) = (major * scale, minor * scale);
            // The apple: its normal away from the tube's centre circle,
            // the file's as the face uses it.
            let rim = major + minor;
            let file = half(&torus("TOROIDAL_SURFACE", major, minor, ""), true, rim);
            let got = measured(&read(&file, scale).expect("apple read"), scale);
            let (volume, area) = apple(major, minor);
            close("apple volume", got.0, volume / 2.0);
            close("apple area", got.1, area / 2.0 + PI * rim * rim);
            // The same, named the outer part.
            let file = half(
                &torus("DEGENERATE_TOROIDAL_SURFACE", major, minor, ",.T."),
                true,
                rim,
            );
            let got = measured(&read(&file, scale).expect("degenerate apple"), scale);
            close("degenerate apple volume", got.0, volume / 2.0);
            // The lemon: `∂u × ∂v` points towards the tube's centre on it,
            // into the lemon, so the face uses it turned over.
            let rim = minor - major;
            let (volume, area) = lemon(major, minor);
            for surface in [
                torus("TOROIDAL_SURFACE", major, minor, ""),
                torus("DEGENERATE_TOROIDAL_SURFACE", major, minor, ",.F."),
            ] {
                let file = half(&surface, false, rim);
                let got = measured(&read(&file, scale).expect("lemon read"), scale);
                close("lemon volume", got.0, volume / 2.0);
                close("lemon area", got.1, area / 2.0 + PI * rim * rim);
            }
        }
    }
}

#[test]
fn a_horn_torus_has_an_apple_and_no_lemon() {
    let (major, minor) = (1.5, 1.5);
    let file = half(&torus("TOROIDAL_SURFACE", major, minor, ""), true, 3.0);
    let got = measured(&read(&file, 1.0).expect("horn"), 1.0);
    // The apple of a horn torus: the whole tube, `2π² R r²`, and its
    // whole area, `4π² R r`.
    close("horn volume", got.0, PI * PI * major * minor * minor);
    close("horn area", got.1, 2.0 * PI * PI * major * minor + 9.0 * PI);
    let file = half(
        &torus("DEGENERATE_TOROIDAL_SURFACE", major, minor, ",.F."),
        false,
        0.5,
    );
    let message = read(&file, 1.0).expect_err("refused").to_string();
    assert!(
        message.contains("#40") && message.contains("empty"),
        "{message}"
    );
}

#[test]
fn an_offset_of_a_spindle_moves_each_part_its_own_way() {
    // The basis (1, 1.5) offset by 0.5 along `∂u × ∂v`: the apple grows to
    // the torus (1, 2), the lemon (whose normal points to the tube's
    // centre) shrinks to (1, 1): a horn, which has none.
    let basis = "#41=TOROIDAL_SURFACE('',#27,1.,1.5);\n";
    let apple_file = half(
        &format!("#40=OFFSET_SURFACE('',#41,0.5,.F.);\n{basis}"),
        true,
        3.0,
    );
    let got = measured(&read(&apple_file, 1.0).expect("offset apple"), 1.0);
    close("offset apple volume", got.0, apple(1.0, 2.0).0 / 2.0);
    // Offset by -0.5: the lemon grows to (1, 2), the apple shrinks to the
    // horn (1, 1).
    let lemon_file = half(
        &format!("#40=OFFSET_SURFACE('',#41,-0.5,.F.);\n{basis}"),
        false,
        1.0,
    );
    let got = measured(&read(&lemon_file, 1.0).expect("offset lemon"), 1.0);
    close("offset lemon volume", got.0, lemon(1.0, 2.0).0 / 2.0);
    // A ring torus (1.5, 0.5) offset outward by 0.5: the apple of (1.5, 1),
    // a horn: the whole tube.
    let ring = "#41=TOROIDAL_SURFACE('',#27,1.5,0.5);\n";
    let file = half(
        &format!("#40=OFFSET_SURFACE('',#41,1.,.F.);\n{ring}"),
        true,
        3.0,
    );
    let got = measured(&read(&file, 1.0).expect("ring offset to a horn"), 1.0);
    close("ring offset volume", got.0, PI * PI * 1.5 * 1.5 * 1.5);
    // A spindle offset back into a ring: its apple a ring torus again
    // (whose face needs both its rims), its lemon grown.
    let spindle = spindle::Spindle::toroidal(Frame::WORLD, 1.0, 1.5, None, false).expect("spindle");
    let Ok(geometry::Read::Spindle(moved)) = spindle.offset(-0.75) else {
        panic!("not a spindle");
    };
    assert_eq!(moved.apple().map(|s| s.minor), Some(0.75));
    assert_eq!(moved.lemon().map(|s| s.minor), Some(2.25));
    let (surface, _) = moved.surface(true).expect("the apple");
    assert!(matches!(surface, craft3d_geom::Surface::Torus(t) if t.radii() == (1.0, 0.75)));
    // Only the apple kept, shrunk into a ring: the whole ring torus.
    let apple = spindle::Spindle {
        select: Some(true),
        ..spindle
    };
    assert!(matches!(
        apple.offset(-0.75),
        Ok(geometry::Read::Done(craft3d_geom::Surface::Torus(_), false))
    ));
    // Both shrunk away: refused.
    let lemon = spindle::Spindle {
        select: Some(false),
        ..spindle
    };
    assert!(lemon.offset(1.0).is_err());
}

#[test]
fn a_circle_turned_across_its_axis_is_a_spindle() {
    // The circle of radius 2 about (1, 0, 0) in the xz plane turned about
    // z: the torus (1, 2). With its axis -y the circle runs from x towards
    // z, as the standard torus's v does, and the face uses the surface's
    // normal as it is; with +y it runs the other way, so the normal is
    // turned over and the face uses it so.
    for (circle_axis, sense) in [("0.,-1.,0.", true), ("0.,1.,0.", false)] {
        let surface = format!(
            "#40=SURFACE_OF_REVOLUTION('',#41,#44);\n\
             #41=CIRCLE('',#42,2.);\n\
             #42=AXIS2_PLACEMENT_3D('',#43,#45,#21);\n\
             #43=CARTESIAN_POINT('',(1.,0.,0.));\n\
             #44=AXIS1_PLACEMENT('',#19,#20);\n\
             #45=DIRECTION('',({circle_axis}));\n"
        );
        let file = half(&surface, sense, 3.0);
        let got = measured(&read(&file, 1.0).expect("revolution read"), 1.0);
        close("revolved apple volume", got.0, apple(1.0, 2.0).0 / 2.0);
        // The other sense reads the face turned over: the same edges then
        // bound the half above the rim.
        let turned = half(&surface, !sense, 3.0);
        let got = measured_z(&read(&turned, 1.0).expect("revolution read"), 1.0);
        close("turned over", got.0, apple(1.0, 2.0).0 / 2.0);
        assert!(got.2 > 0.0, "turned over: centroid at z = {:e}", got.2);
    }
}

#[test]
fn edges_on_the_wrong_part_are_refused_by_name() {
    // The rim of the lemon on a surface that is only the apple.
    let file = half(
        &torus("DEGENERATE_TOROIDAL_SURFACE", 1.0, 2.0, ",.T."),
        false,
        1.0,
    );
    let message = read(&file, 1.0).expect_err("refused").to_string();
    assert!(
        message.contains("#40") && message.contains("lemon"),
        "{message}"
    );
    // A select_outer that is not a boolean.
    let file = half(
        &torus("DEGENERATE_TOROIDAL_SURFACE", 1.0, 2.0, ",$"),
        true,
        3.0,
    );
    let message = read(&file, 1.0).expect_err("refused").to_string();
    assert!(message.contains("select_outer"), "{message}");
}

#[test]
fn hostile_radii_are_refused_not_panicked() {
    // No torus at all: refused, naming the surface.
    for (major, minor) in [
        ("0.", "2."),
        ("-1.", "2."),
        ("1.", "0."),
        ("1.", "-2."),
        ("-2.", "-1."),
        ("0.", "0."),
        ("0.", "-0."),
    ] {
        for (keyword, extra) in [
            ("TOROIDAL_SURFACE", ""),
            ("DEGENERATE_TOROIDAL_SURFACE", ",.T."),
            ("DEGENERATE_TOROIDAL_SURFACE", ",.F."),
        ] {
            let surface = format!("#40={keyword}('',#27,{major},{minor}{extra});\n");
            let message = read(&half(&surface, true, 3.0), 1.0)
                .err()
                .unwrap_or_else(|| panic!("{keyword} ({major}, {minor}) read"))
                .to_string();
            assert!(message.contains("#40"), "{message}");
        }
    }
    // Radii that are a torus, or too large to be numbers, whose face does
    // not fit: an error of some kind, never a panic.
    for (major, minor) in [
        ("1.E400", "2."),
        ("1.", "1.E400"),
        ("1.E-320", "2."),
        ("2.", "2."),
        ("1.E300", "1.E300"),
    ] {
        for (keyword, extra) in [
            ("TOROIDAL_SURFACE", ""),
            ("DEGENERATE_TOROIDAL_SURFACE", ",.F."),
        ] {
            let surface = format!("#40={keyword}('',#27,{major},{minor}{extra});\n");
            assert!(read(&half(&surface, true, 3.0), 1.0).is_err());
        }
    }
    // Radii as `Spindle` sees them: NaN and infinity.
    for (major, minor) in [(f64::NAN, 1.0), (1.0, f64::NAN), (f64::INFINITY, 1.0)] {
        assert!(spindle::Spindle::toroidal(Frame::WORLD, major, minor, None, false).is_err());
    }
}
